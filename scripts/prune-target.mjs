#!/usr/bin/env node
/**
 * scripts/prune-target.mjs
 * 
 * Kokoro Storage Sentinel (KSS) - Full-Lifecycle Closed-Loop Storage Management
 * 
 * Designed for Kokoro-Engine (Tauri v2 + React 19 + Rust on Windows/macOS/Linux).
 * Provides end-to-end, fully automated, and resilient storage management across:
 *   1. Onboarding & First Setup (git hooks setup, sccache detection, symbol slimming)
 *   2. Branch Switching (instant non-blocking git tag, prioritizes orphan eviction)
 *   3. Test Suite Lifecycle (ephemeral retention for cargo test harnesses & PDBs)
 *   4. Daily Dev & Emergency Circuit Breaker (12GB watermark + drive-level low-disk shield)
 *   5. Observability Dashboard (--doctor storage health check)
 * 
 * Modes:
 *   - Automatic Watchdog: node scripts/prune-target.mjs --auto
 *   - Setup Git Hooks   : node scripts/prune-target.mjs --setup-hooks
 *   - Storage Doctor    : node scripts/prune-target.mjs --doctor
 *   - Manual Prune      : node scripts/prune-target.mjs [--dry-run] [--verbose] [--threshold-gb=N]
 */

import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";
import { pathToFileURL } from "node:url";

// =============================================================================
// Security: Canonical Containment & Link / Reparse Point Protection
// =============================================================================

function stripUncPrefix(p) {
  if (typeof p !== "string") return p;
  if (p.startsWith("\\\\?\\UNC\\")) {
    return "\\\\" + p.slice(8);
  }
  if (p.startsWith("\\\\?\\")) {
    return p.slice(4);
  }
  return p;
}

function normalizeCanonicalPath(p) {
  if (!p) return "";
  const stripped = stripUncPrefix(path.normalize(path.resolve(p)));
  return process.platform === "win32" ? stripped.toLowerCase() : stripped;
}

function getSafeRealPath(p) {
  try {
    const raw = fs.realpathSync.native ? fs.realpathSync.native(p) : fs.realpathSync(p);
    return stripUncPrefix(raw);
  } catch (_) {
    return null;
  }
}

function isCanonicallyContained(childPath, parentPath) {
  if (!childPath || !parentPath) return false;
  const normChild = normalizeCanonicalPath(childPath);
  const normParent = normalizeCanonicalPath(parentPath);
  if (normChild === normParent) return false;
  const rel = path.relative(normParent, normChild);
  return rel !== "" && !rel.startsWith("..") && !path.isAbsolute(rel);
}

function verifyPathChain(repoRoot, targetPath, debugPath) {
  const canonicalRoot = getSafeRealPath(repoRoot);
  if (!canonicalRoot) {
    return { valid: false, reason: `Cannot resolve canonical root: ${repoRoot}` };
  }

  const chain = [
    { name: "src-tauri", expectedDir: path.join(repoRoot, "src-tauri") },
    { name: "target", expectedDir: targetPath },
    { name: "debug", expectedDir: debugPath },
  ];

  let currentParent = canonicalRoot;
  let canonicalTarget = path.join(canonicalRoot, "src-tauri", "target");
  let canonicalDebug = path.join(canonicalTarget, "debug");

  for (const step of chain) {
    let lstat;
    try {
      lstat = fs.lstatSync(step.expectedDir);
    } catch (err) {
      if (err?.code === "ENOENT") {
        // Normal state: directory does not exist yet (clean slate)
        break;
      }
      return { valid: false, reason: `Failed to inspect '${step.name}': ${err.message}` };
    }

    // Security Gate: Reject any symlinks, junctions, or reparse redirects
    if (lstat.isSymbolicLink()) {
      return {
        valid: false,
        reason: `Path component '${step.name}' is a symbolic link or junction: ${step.expectedDir}`,
      };
    }

    if (!lstat.isDirectory()) {
      return {
        valid: false,
        reason: `Path component '${step.name}' is not a directory: ${step.expectedDir}`,
      };
    }

    const realDir = getSafeRealPath(step.expectedDir);
    if (!realDir) {
      return {
        valid: false,
        reason: `Failed to resolve realpath for '${step.name}': ${step.expectedDir}`,
      };
    }

    const expectedCanonicalDir = path.join(currentParent, step.name);
    if (normalizeCanonicalPath(realDir) !== normalizeCanonicalPath(expectedCanonicalDir)) {
      return {
        valid: false,
        reason: `Canonical divergence detected at '${step.name}': realpath (${realDir}) != expected (${expectedCanonicalDir})`,
      };
    }

    if (!isCanonicallyContained(realDir, canonicalRoot)) {
      return {
        valid: false,
        reason: `Path component '${step.name}' escapes root: ${realDir} not in ${canonicalRoot}`,
      };
    }

    currentParent = realDir;
    if (step.name === "target") {
      canonicalTarget = realDir;
    } else if (step.name === "debug") {
      canonicalDebug = realDir;
    }
  }

  return {
    valid: true,
    canonicalRoot,
    canonicalTarget,
    canonicalDebug,
  };
}

// =============================================================================
// Helper Functions: Formatting, Sizing & Safe Deletion
// =============================================================================

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function getDirSize(d, canonicalBoundary = null) {
  let size = 0;
  try {
    const dirLstat = fs.lstatSync(d);
    if (dirLstat.isSymbolicLink() || !dirLstat.isDirectory()) {
      return 0;
    }

    if (canonicalBoundary) {
      const realD = getSafeRealPath(d);
      if (
        !realD ||
        (!isCanonicallyContained(realD, canonicalBoundary) &&
          normalizeCanonicalPath(realD) !== normalizeCanonicalPath(canonicalBoundary))
      ) {
        return 0;
      }
    }

    const entries = fs.readdirSync(d, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.isSymbolicLink()) continue;

      const full = path.join(d, entry.name);
      if (entry.isDirectory()) {
        size += getDirSize(full, canonicalBoundary);
      } else if (entry.isFile()) {
        try {
          const fileLstat = fs.lstatSync(full);
          if (!fileLstat.isSymbolicLink()) {
            size += fileLstat.size;
          }
        } catch (_) {}
      }
    }
  } catch (_) {}
  return size;
}

function getDriveFreeSpace(dirPath, rootFallback = process.cwd()) {
  try {
    let checkPath = rootFallback;
    try {
      if (fs.existsSync(dirPath)) {
        const lstat = fs.lstatSync(dirPath);
        if (!lstat.isSymbolicLink() && lstat.isDirectory()) {
          checkPath = dirPath;
        }
      }
    } catch (_) {}

    const st = fs.statfsSync(checkPath);
    const freeBytes = Number(st.bavail) * Number(st.bsize);
    const totalBytes = Number(st.blocks) * Number(st.bsize);
    return {
      freeBytes,
      totalBytes,
      freeGb: freeBytes / (1024 * 1024 * 1024),
      totalGb: totalBytes / (1024 * 1024 * 1024),
    };
  } catch (_) {
    return null;
  }
}

function safeRemove(itemPath, canonicalBoundary, isDryRun = false, isVerbose = false) {
  try {
    // 1. Never follow or remove through links or reparse points
    const lstat = fs.lstatSync(itemPath);
    if (lstat.isSymbolicLink()) {
      if (isVerbose) {
        console.warn(`[WARN] Refusing to remove symbolic link or reparse point: ${itemPath}`);
      }
      return { success: false, size: 0, reason: "symlink" };
    }

    // 2. Canonical containment verification: must be strictly inside expected boundary
    const realItem = getSafeRealPath(itemPath);
    if (!realItem) {
      return { success: false, size: 0, reason: "unresolvable" };
    }

    if (canonicalBoundary && !isCanonicallyContained(realItem, canonicalBoundary)) {
      if (isVerbose) {
        console.warn(`[WARN] Refusing to delete item outside boundary: ${realItem} not in ${canonicalBoundary}`);
      }
      return { success: false, size: 0, reason: "boundary_violation" };
    }

    const size = lstat.isDirectory() ? getDirSize(itemPath, canonicalBoundary) : lstat.size;

    if (!isDryRun) {
      if (lstat.isDirectory()) {
        // Atomic staging isolation: rename to staging first to avoid half-deleted states if locked
        const stagingPath = `${itemPath}.pruning-${Date.now()}`;
        try {
          fs.renameSync(itemPath, stagingPath);
          fs.rmSync(stagingPath, { recursive: true, force: true });
        } catch (renameErr) {
          if (["EBUSY", "EPERM", "EACCES"].includes(renameErr.code)) {
            if (isVerbose) {
              console.warn(`[WARN] Skipping locked/inaccessible item: ${path.basename(itemPath)} (${renameErr.code})`);
            }
            return { success: false, size: 0, reason: "locked", error: renameErr };
          }
          // Direct fallback if filesystem does not permit rename
          fs.rmSync(itemPath, { recursive: true, force: true });
        }
      } else {
        fs.unlinkSync(itemPath);
      }
    }
    return { success: true, size };
  } catch (err) {
    if (isVerbose) {
      console.warn(`[WARN] Skipping locked/inaccessible item: ${path.basename(itemPath)} (${err.code || err.message})`);
    }
    return { success: false, size: 0, error: err };
  }
}

// =============================================================================
// Active Build & Incremental Session Lock Detection
// =============================================================================

function isLockFileActive(lockFilePath, canonicalBoundary = null) {
  try {
    const lstat = fs.lstatSync(lockFilePath);
    if (lstat.isSymbolicLink() || !lstat.isFile()) return false;
    if (canonicalBoundary) {
      const real = getSafeRealPath(lockFilePath);
      if (!real || !isCanonicallyContained(real, canonicalBoundary)) return false;
    }
    // Probe lock: attempt read/write access.
    // On Windows, rustc locks s-*.lock via LockFileEx; openSync('r+') throws EBUSY/EPERM if locked.
    const fd = fs.openSync(lockFilePath, "r+");
    fs.closeSync(fd);
    return false;
  } catch (err) {
    if (
      err &&
      (["EBUSY", "EPERM", "EACCES"].includes(err.code) ||
        err.code === "ERR_FS_EISDIR" ||
        String(err.message).toLowerCase().includes("busy") ||
        String(err.message).toLowerCase().includes("locked"))
    ) {
      return true;
    }
    return false;
  }
}

function isSessionDirectoryLocked(sessionPath, canonicalBoundary = null, maxDepth = 2) {
  try {
    const lstat = fs.lstatSync(sessionPath);
    if (lstat.isSymbolicLink() || !lstat.isDirectory()) return false;
    const realSession = getSafeRealPath(sessionPath);
    if (!realSession || (canonicalBoundary && !isCanonicallyContained(realSession, canonicalBoundary))) {
      return false;
    }

    const subEntries = fs.readdirSync(sessionPath, { withFileTypes: true });
    for (const sub of subEntries) {
      if (sub.isSymbolicLink()) continue;
      const full = path.join(sessionPath, sub.name);
      if (sub.isFile() && sub.name.endsWith(".lock")) {
        if (isLockFileActive(full, canonicalBoundary)) {
          return true;
        }
      } else if (sub.isDirectory() && maxDepth > 1) {
        // Inspect nested session subdirectories (e.g. s-<hash>/) for locks
        if (isSessionDirectoryLocked(full, canonicalBoundary, maxDepth - 1)) {
          return true;
        }
      }
    }
  } catch (_) {}
  return false;
}

function hasActiveCompilerProcesses() {
  try {
    if (process.platform === "win32") {
      // On Windows, isLockFileActive catches LockFileEx kernel-level locks directly.
      return false;
    }
    // On POSIX (Linux/macOS), flock is advisory, so supplement with compiler process check
    const out = execSync("pgrep -x 'cargo|rustc' || true", {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      timeout: 1000,
    }).trim();
    return out.length > 0;
  } catch (_) {
    return false;
  }
}

function hasActiveIncrementalLocks(incrementalDir, canonicalDebugDir) {
  try {
    if (hasActiveCompilerProcesses()) {
      return true;
    }
    // Check for target/.cargo-lock if target directory is parent of debug
    const targetDir = path.dirname(path.resolve(incrementalDir, ".."));
    const cargoLock = path.join(targetDir, ".cargo-lock");
    if (fs.existsSync(cargoLock) && isLockFileActive(cargoLock, canonicalDebugDir ? path.dirname(canonicalDebugDir) : null)) {
      return true;
    }

    const entries = fs.readdirSync(incrementalDir, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.isSymbolicLink() || !entry.isDirectory()) continue;
      const crateDir = path.join(incrementalDir, entry.name);
      if (isSessionDirectoryLocked(crateDir, canonicalDebugDir)) {
        return true;
      }
    }
  } catch (_) {}
  return false;
}

// =============================================================================
// Core Pruning Algorithms: Incremental Sessions & Ephemeral Test Harnesses
// =============================================================================

function pruneIncremental(
  incrementalDir,
  keepSessionsCount,
  canonicalDebugDir,
  isDryRun = false,
  isVerbose = false,
  isAuto = false,
  isEmergency = false
) {
  if (!fs.existsSync(incrementalDir)) return { count: 0, bytesFreed: 0, hasActiveLock: false, incomplete: false };

  try {
    const incLstat = fs.lstatSync(incrementalDir);
    if (incLstat.isSymbolicLink() || !incLstat.isDirectory()) {
      if (isVerbose) console.warn("[WARN] incremental directory is a link or non-directory; skipping.");
      return { count: 0, bytesFreed: 0, hasActiveLock: false, incomplete: false };
    }
  } catch (_) {
    return { count: 0, bytesFreed: 0, hasActiveLock: false, incomplete: false };
  }

  const realIncremental = getSafeRealPath(incrementalDir);
  if (!realIncremental || (canonicalDebugDir && !isCanonicallyContained(realIncremental, canonicalDebugDir))) {
    if (isVerbose) console.warn("[WARN] incremental directory escapes debug directory; skipping.");
    return { count: 0, bytesFreed: 0, hasActiveLock: false, incomplete: false };
  }

  let count = 0;
  let bytesFreed = 0;
  let hasActiveLock = false;
  let incomplete = false;

  // Active-build lock check: inspect candidate session directories for active s-*.lock
  if (hasActiveIncrementalLocks(incrementalDir, canonicalDebugDir)) {
    hasActiveLock = true;
    if (isAuto && !isEmergency) {
      if (isVerbose) {
        console.log("[INFO] Detected active compilation lock in candidate sessions; yielding auto prune.");
      }
      return { count: 0, bytesFreed: 0, hasActiveLock: true, incomplete: false };
    }
    if (isVerbose) {
      console.log("[INFO] Detected active lock in incremental folder; exercising extra caution.");
    }
  }

  let entries = [];
  try {
    entries = fs.readdirSync(incrementalDir, { withFileTypes: true });
  } catch (_) {
    return { count, bytesFreed, hasActiveLock, incomplete };
  }

  const groups = new Map();

  for (const entry of entries) {
    if (entry.isSymbolicLink() || !entry.isDirectory()) continue;
    const fullPath = path.join(incrementalDir, entry.name);
    let stat;
    try {
      stat = fs.lstatSync(fullPath);
      if (stat.isSymbolicLink() || !stat.isDirectory()) continue;
    } catch (_) {
      continue;
    }

    const realSession = getSafeRealPath(fullPath);
    if (!realSession || !isCanonicallyContained(realSession, realIncremental)) {
      continue;
    }

    const match = entry.name.match(/^([a-zA-Z0-9_-]+)-([0-9a-z]{10,20})$/);
    const prefix = match ? match[1] : entry.name;

    if (!groups.has(prefix)) {
      groups.set(prefix, []);
    }
    groups.get(prefix).push({
      name: entry.name,
      path: fullPath,
      mtime: stat.mtimeMs,
    });
  }

  for (const [prefix, sessions] of groups.entries()) {
    sessions.sort((a, b) => b.mtime - a.mtime);

    let keep = prefix.includes("kokoro") ? (keepSessionsCount ?? 2) : 1;
    const staleSessions = sessions.slice(keep);

    for (const session of staleSessions) {
      if (isSessionDirectoryLocked(session.path, canonicalDebugDir)) {
        hasActiveLock = true;
        incomplete = true;
        if (isVerbose) {
          console.warn(`[WARN] Candidate session ${session.name} is actively locked; skipping.`);
        }
        continue;
      }

      if (isVerbose) {
        console.log(`[PRUNE] Stale incremental session: ${session.name}`);
      }
      const res = safeRemove(session.path, canonicalDebugDir, isDryRun, isVerbose);
      if (res.success) {
        count++;
        bytesFreed += res.size;
      } else {
        incomplete = true;
      }
    }
  }

  return { count, bytesFreed, hasActiveLock, incomplete };
}

// Known Kokoro Engine test suites and domain modules that generate standalone test harnesses.
const KNOWN_TEST_HARNESSES = new Set([
  "characters",
  "vision",
  "commands",
  "llm",
  "tts",
  "stt",
  "mcp",
  "registry",
  "mods",
  "actions",
  "chat",
  "bot",
  "imagegen",
  "telegram",
  "qqbot",
  "config",
]);

/**
 * Validates whether an artifact in deps/ belongs to Kokoro Engine or a test harness.
 * Enforces a strict dual-layer safety fence to NEVER prune third-party dependencies.
 */
function isEligibleDepsArtifact(baseName, ext, exeBaseNames = new Set()) {
  const lowerBase = baseName.toLowerCase();
  const isKokoro = lowerBase.includes("kokoro");

  if (isKokoro) {
    // Project workspace crates: .exe, "", .pdb, .rlib, .d, .dsym are all eligible for lifecycle management
    return true;
  }

  // SAFETY: For non-kokoro names, ONLY test harness artifacts (.exe, "", .pdb, .d, .dsym) are eligible.
  // NEVER prune third-party library (.rlib) or proc-macro (.dll, .so, .dylib) artifacts!
  const isTestArtifactExt = ext === ".exe" || ext === "" || ext === ".pdb" || ext === ".d" || ext === ".dsym";
  if (!isTestArtifactExt) {
    return false;
  }

  // Must be in the known module allowlist OR have an executable target discovered in deps
  if (KNOWN_TEST_HARNESSES.has(lowerBase) || exeBaseNames.has(lowerBase)) {
    return true;
  }

  return false;
}

function pruneDeps(
  depsDir,
  isEmergencyMode,
  canonicalDebugDir,
  isDryRun = false,
  isVerbose = false,
  hasBranchSwitched = false
) {
  if (!fs.existsSync(depsDir)) return { count: 0, bytesFreed: 0, incomplete: false };

  try {
    const depsLstat = fs.lstatSync(depsDir);
    if (depsLstat.isSymbolicLink() || !depsLstat.isDirectory()) {
      if (isVerbose) console.warn("[WARN] deps directory is a link or non-directory; skipping.");
      return { count: 0, bytesFreed: 0, incomplete: false };
    }
  } catch (_) {
    return { count: 0, bytesFreed: 0, incomplete: false };
  }

  const realDeps = getSafeRealPath(depsDir);
  if (!realDeps || (canonicalDebugDir && !isCanonicallyContained(realDeps, canonicalDebugDir))) {
    if (isVerbose) console.warn("[WARN] deps directory escapes debug directory; skipping.");
    return { count: 0, bytesFreed: 0, incomplete: false };
  }

  let count = 0;
  let bytesFreed = 0;
  let incomplete = false;

  let entries = [];
  try {
    entries = fs.readdirSync(depsDir, { withFileTypes: true });
  } catch (_) {
    return { count, bytesFreed, incomplete: false };
  }

  // Pass 1: discover executable target base names in deps
  // Windows generates .exe; Linux/macOS generates extensionless binaries
  const exeBaseNames = new Set();
  for (const entry of entries) {
    if (entry.isSymbolicLink()) continue;
    if (!entry.isFile() && !entry.isDirectory()) continue;
    const match = entry.name.match(/^(lib[a-zA-Z0-9_]+|[a-zA-Z0-9_]+)-([0-9a-f]{16})(?:(\.[a-zA-Z0-9_.]+))?$/i);
    if (match) {
      const ext = match[3] ? match[3].toLowerCase() : "";
      if (ext === ".exe" || ext === "") {
        exeBaseNames.add(match[1].toLowerCase());
      }
    }
  }

  const groups = new Map();
  const now = Date.now();
  const testProtectionWindowMs = 30 * 60 * 1000; // 30-minute shield for active test runs

  for (const entry of entries) {
    if (entry.isSymbolicLink()) continue;
    if (!entry.isFile() && !entry.isDirectory()) continue;
    const name = entry.name;
    const fullPath = path.join(depsDir, name);
    let stat;
    try {
      stat = fs.lstatSync(fullPath);
      if (stat.isSymbolicLink()) continue;
      if (!stat.isFile() && !stat.isDirectory()) continue;
      // If directory, only .dSYM bundles are permitted
      if (stat.isDirectory() && !name.toLowerCase().endsWith(".dsym")) continue;
    } catch (_) {
      continue;
    }

    const realFile = getSafeRealPath(fullPath);
    if (!realFile || !isCanonicallyContained(realFile, realDeps)) {
      continue;
    }

    const match = name.match(/^(lib[a-zA-Z0-9_]+|[a-zA-Z0-9_]+)-([0-9a-f]{16})(?:(\.[a-zA-Z0-9_.]+))?$/i);
    if (!match) continue;

    const baseName = match[1];
    const ext = match[3] ? match[3].toLowerCase() : "";

    // SAFETY: Only prune project's own artifacts and test harnesses.
    // NEVER prune third-party crates (.rlib, .dll, .so, .dylib) in deps.
    if (!isEligibleDepsArtifact(baseName, ext, exeBaseNames)) {
      continue;
    }

    const key = `${baseName}|${ext}`;

    if (!groups.has(key)) {
      groups.set(key, []);
    }
    groups.get(key).push({
      name,
      path: fullPath,
      mtime: stat.mtimeMs,
      size: stat.size,
    });
  }

  for (const [key, items] of groups.entries()) {
    if (items.length <= 1) continue;

    items.sort((a, b) => b.mtime - a.mtime);

    const [baseName, ext] = key.split("|");
    const isMainApp = (baseName === "tauri_appkokoro_engine");
    const isCoreLib = (baseName.toLowerCase().includes("kokoro_engine_lib") && ext === ".rlib");
    const isTestHarness = !isMainApp && !isCoreLib && (ext === ".exe" || ext === "" || ext === ".pdb" || ext === ".d" || ext === ".dsym");

    // Retention policy:
    // - Main app (.exe, extensionless, .pdb): keep latest 2 in normal dev mode (unless emergency or branch switched)
    // - Core library (.rlib): keep latest 1
    // - Test harnesses (.exe, extensionless, .pdb, .d, .dsym): keep latest 1 active generation
    let keepCount = 1;
    if (!isEmergencyMode && !hasBranchSwitched && isMainApp) {
      keepCount = 2;
    }

    const staleItems = items.slice(keepCount);

    for (const item of staleItems) {
      // If it's a test harness, apply the 30-minute shield unless emergency or branch switched
      if (isTestHarness && !isEmergencyMode && !hasBranchSwitched) {
        if (now - item.mtime < testProtectionWindowMs) {
          continue;
        }
      }

      if (isVerbose) {
        console.log(`[PRUNE] Stale artifact in deps: ${item.name} (${formatBytes(item.size)})`);
      }
      const res = safeRemove(item.path, canonicalDebugDir, isDryRun, isVerbose);
      if (res.success) {
        count++;
        bytesFreed += res.size;
      } else {
        incomplete = true;
      }
    }
  }

  return { count, bytesFreed, incomplete };
}

function consumeBranchSwitchMarker(targetDir, branchSwitchFile, canonicalRoot, isDryRun = false) {
  if (isDryRun) return false;
  try {
    if (!fs.existsSync(branchSwitchFile)) return false;
    const bsLstat = fs.lstatSync(branchSwitchFile);
    if (bsLstat.isSymbolicLink() || !bsLstat.isFile()) return false;

    const realMarker = getSafeRealPath(branchSwitchFile);
    if (!realMarker || (canonicalRoot && !isCanonicallyContained(realMarker, canonicalRoot))) {
      return false;
    }

    fs.unlinkSync(branchSwitchFile);
    return true;
  } catch (_) {
    return false;
  }
}

function writeCooldown(targetDir, cooldownFile, canonicalRoot, isDryRun = false) {
  if (isDryRun) return false;
  try {
    let exists = false;
    try {
      const lstat = fs.lstatSync(targetDir);
      if (lstat.isSymbolicLink() || !lstat.isDirectory()) return;
      exists = true;
    } catch (err) {
      if (err?.code !== "ENOENT") return;
    }

    if (!exists) {
      const parentDir = path.dirname(targetDir);
      try {
        const pLstat = fs.lstatSync(parentDir);
        if (pLstat.isSymbolicLink() || !pLstat.isDirectory()) return;
      } catch (_) {
        return;
      }
      fs.mkdirSync(targetDir, { recursive: true });
    }

    const realTarget = getSafeRealPath(targetDir);
    if (!realTarget || (canonicalRoot && !isCanonicallyContained(realTarget, canonicalRoot))) {
      return;
    }

    fs.writeFileSync(cooldownFile, Date.now().toString(), "utf8");
    return true;
  } catch (_) {
    return false;
  }
}

function updateCooldown(targetDir, cooldownFile, branchSwitchFile, hasBranchSwitched, canonicalRoot, isDryRun = false) {
  if (isDryRun) return false;
  const wrote = writeCooldown(targetDir, cooldownFile, canonicalRoot, isDryRun);
  if (hasBranchSwitched && branchSwitchFile) {
    consumeBranchSwitchMarker(targetDir, branchSwitchFile, canonicalRoot, isDryRun);
  }
  return wrote;
}

// =============================================================================
// Git Hooks Management (Preservation & Safe Chaining)
// =============================================================================

const SENTINEL_BLOCK_START = "# >>> Kokoro Storage Sentinel (DO NOT EDIT) >>>";
const SENTINEL_BLOCK_END = "# <<< Kokoro Storage Sentinel <<<";

function getSentinelSnippet(hookName) {
  if (hookName === "post-checkout") {
    return `${SENTINEL_BLOCK_START}
if [ "$3" = "1" ]; then
  mkdir -p src-tauri/target 2>/dev/null || true
  date +%s > src-tauri/target/.branch-switched 2>/dev/null || true
fi
${SENTINEL_BLOCK_END}\n`;
  }
  return `${SENTINEL_BLOCK_START}
mkdir -p src-tauri/target 2>/dev/null || true
date +%s > src-tauri/target/.branch-switched 2>/dev/null || true
${SENTINEL_BLOCK_END}\n`;
}

function isPosixShellScript(content) {
  if (typeof content !== "string") return false;
  const firstLine = content.split(/\r?\n/)[0].trim();
  return /^#!\s*\/.*(?:sh|bash)(?:\s.*)?$/.test(firstLine);
}

function injectSentinelBlockIntoShellScript(originalContent, hookName) {
  const normalized = originalContent.replace(/\r\n/g, "\n");
  const firstNewlineIdx = normalized.indexOf("\n");
  const snippet = getSentinelSnippet(hookName);
  if (firstNewlineIdx === -1) {
    return `${normalized}\n\n${snippet}`;
  }
  const shebang = normalized.slice(0, firstNewlineIdx);
  const rest = normalized.slice(firstNewlineIdx + 1);
  return `${shebang}\n\n${snippet}\n${rest}`;
}

let gitExec = (cmd, opts) => execSync(cmd, opts);
function setGitExecutorForTesting(fn) {
  gitExec = fn || ((cmd, opts) => execSync(cmd, opts));
}

function getGitEnvironment(root) {
  try {
    let hooksDir = null;
    try {
      const raw = gitExec("git rev-parse --git-path hooks", {
        cwd: root,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      }).trim();
      if (raw) {
        hooksDir = path.resolve(root, raw);
      }
    } catch (_) {}

    // Fallback if git rev-parse failed (e.g. test environment or mocked .git)
    if (!hooksDir) {
      const gitPath = path.join(root, ".git");
      if (fs.existsSync(gitPath)) {
        try {
          const stat = fs.statSync(gitPath);
          if (stat.isDirectory()) {
            hooksDir = path.join(gitPath, "hooks");
          } else if (stat.isFile()) {
            const content = fs.readFileSync(gitPath, "utf8").trim();
            const match = content.match(/^gitdir:\s*(.+)$/m);
            if (match) {
              const gitdir = path.resolve(root, match[1]);
              hooksDir = path.join(gitdir, "hooks");
            }
          }
        } catch (_) {}
      }
    }

    let currentHooksPath = null;
    try {
      const raw = gitExec("git config --get core.hooksPath", {
        cwd: root,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      }).trim();
      if (raw) {
        currentHooksPath = raw;
      }
    } catch (_) {}

    const inGitRepo = !!hooksDir || fs.existsSync(path.join(root, ".git"));
    return {
      inGitRepo,
      hooksDir,
      currentHooksPath,
    };
  } catch (_) {
    return { inGitRepo: false, hooksDir: null, currentHooksPath: null };
  }
}

function getActiveHooks(hooksDir) {
  if (!hooksDir || !fs.existsSync(hooksDir)) return [];
  try {
    const entries = fs.readdirSync(hooksDir);
    return entries.filter((e) => {
      if (e.endsWith(".sample")) return false;
      try {
        const p = path.join(hooksDir, e);
        const st = fs.statSync(p);
        return st.isFile();
      } catch (_) {
        return false;
      }
    });
  } catch (_) {
    return [];
  }
}

// =============================================================================
// Main Execution Pipeline
// =============================================================================

function runPruneTarget(args = process.argv.slice(2), customRoot = process.cwd()) {
  const root = customRoot;
  const targetDir = path.join(root, "src-tauri", "target");
  const debugDir = path.join(targetDir, "debug");

  const isAuto = args.includes("--auto");
  const isDryRun = args.includes("--dry-run");
  const isDoctor = args.includes("--doctor");
  const isSetupHooks = args.includes("--setup-hooks");
  const isVerbose = (args.includes("--verbose") || args.includes("-v")) && !isAuto;
  const thresholdArg = args.find((a) => a.startsWith("--threshold-gb="));
  const explicitThresholdGb = thresholdArg ? parseFloat(thresholdArg.split("=")[1]) : 0;

  // Feature: --setup-hooks (Native zero-dependency Git hooks registration with safe preservation & chaining)
  if (isSetupHooks) {
    const isSafe = args.includes("--safe");
    const isForce = args.includes("--force");

    // Skip in CI or if explicitly opted out
    if (process.env.KOKORO_SKIP_HOOKS === "1" || process.env.npm_config_kokoro_skip_hooks === "1") {
      return { skipped: true, reason: "kokoro_skip_hooks", mode: "setup-hooks" };
    }
    if (
      (isAuto || isSafe) &&
      (process.env.CI || process.env.GITHUB_ACTIONS || process.env.CONTINUOUS_INTEGRATION)
    ) {
      return { skipped: true, reason: "ci_environment", mode: "setup-hooks" };
    }

    try {
      const gitEnv = getGitEnvironment(root);
      const bundledHooksDir = path.join(root, ".githooks");

      if (!gitEnv.inGitRepo) {
        return { skipped: true, reason: "not_a_git_repository", mode: "setup-hooks" };
      }

      // Case 1: Custom core.hooksPath already configured (e.g. .husky, enterprise hooks)
      if (gitEnv.currentHooksPath && gitEnv.currentHooksPath !== ".githooks") {
        if (!isForce) {
          if (!isAuto && !isSafe) {
            console.log(
              `ℹ️ [Kokoro Storage Sentinel] Existing core.hooksPath detected ('${gitEnv.currentHooksPath}'). Preserving existing hooks configuration.`
            );
          }
          return {
            success: true,
            mode: "setup-hooks",
            status: "preserved_custom_hooks",
            hooksPath: gitEnv.currentHooksPath,
          };
        }
      }

      // Case 2: core.hooksPath is already .githooks (or --force requested)
      if (gitEnv.currentHooksPath === ".githooks" || isForce) {
        try {
          gitExec("git config core.hooksPath .githooks", { cwd: root, stdio: "ignore" });
        } catch (_) {}

        if (fs.existsSync(bundledHooksDir) && process.platform !== "win32") {
          for (const hookName of ["post-checkout", "post-merge"]) {
            const hookPath = path.join(bundledHooksDir, hookName);
            if (fs.existsSync(hookPath)) {
              try {
                fs.chmodSync(hookPath, 0o755);
              } catch (_) {}
            }
          }
        }
        if (!isAuto && !isSafe) {
          console.log("✅ [Kokoro Storage Sentinel] Git hooks registered to .githooks (branch-switch sensing active).");
        }
        return { success: true, mode: "setup-hooks", status: isForce ? "forced_githooks" : "active_githooks" };
      }

      // Case 3: core.hooksPath is NOT set -> check if active hooks exist in hooksDir
      const activeHooks = getActiveHooks(gitEnv.hooksDir);
      if (activeHooks.length > 0 && gitEnv.hooksDir) {
        // Active hooks exist (e.g. pre-commit, custom post-checkout). DO NOT hijack core.hooksPath!
        // Instead, safely inline/chain sentinel snippet directly into gitEnv.hooksDir.
        try {
          fs.mkdirSync(gitEnv.hooksDir, { recursive: true });
        } catch (_) {}

        for (const hookName of ["post-checkout", "post-merge"]) {
          const hookPath = path.join(gitEnv.hooksDir, hookName);
          if (fs.existsSync(hookPath)) {
            try {
              const content = fs.readFileSync(hookPath, "utf8");
              if (!content.includes(SENTINEL_BLOCK_START) && isPosixShellScript(content)) {
                const injected = injectSentinelBlockIntoShellScript(content, hookName);
                fs.writeFileSync(hookPath, injected, "utf8");
                if (process.platform !== "win32") {
                  fs.chmodSync(hookPath, 0o755);
                }
              }
            } catch (_) {}
          } else {
            try {
              const newHook = `#!/bin/sh\n\n${getSentinelSnippet(hookName)}exit 0\n`;
              fs.writeFileSync(hookPath, newHook, "utf8");
              if (process.platform !== "win32") {
                fs.chmodSync(hookPath, 0o755);
              }
            } catch (_) {}
          }
        }

        // Also ensure bundled .githooks permissions if present
        if (fs.existsSync(bundledHooksDir) && process.platform !== "win32") {
          for (const hookName of ["post-checkout", "post-merge"]) {
            const hookPath = path.join(bundledHooksDir, hookName);
            if (fs.existsSync(hookPath)) {
              try {
                fs.chmodSync(hookPath, 0o755);
              } catch (_) {}
            }
          }
        }

        if (!isAuto && !isSafe) {
          console.log("✅ [Kokoro Storage Sentinel] Git hooks safely chained in .git/hooks (preserving existing hooks).");
        }
        return { success: true, mode: "setup-hooks", status: "chained_git_hooks" };
      }

      // Case 4: Clean slate (no custom hooks, no core.hooksPath).
      // Safe to point core.hooksPath to .githooks if .githooks exists.
      if (fs.existsSync(bundledHooksDir)) {
        try {
          gitExec("git config core.hooksPath .githooks", { cwd: root, stdio: "ignore" });
        } catch (_) {}

        if (process.platform !== "win32") {
          for (const hookName of ["post-checkout", "post-merge"]) {
            const hookPath = path.join(bundledHooksDir, hookName);
            if (fs.existsSync(hookPath)) {
              try {
                fs.chmodSync(hookPath, 0o755);
              } catch (_) {}
            }
          }
        }
        if (!isAuto && !isSafe) {
          console.log("✅ [Kokoro Storage Sentinel] Git hooks registered to .githooks (branch-switch sensing active).");
        }
        return { success: true, mode: "setup-hooks", status: "registered_githooks" };
      }
    } catch (_) {}
    return { success: true, mode: "setup-hooks" };
  }

  // Defense 1: Dev vs Release & CI Environment Boundary Gating
  if (isAuto) {
    const isCustomTestRoot = customRoot !== process.cwd();
    const shouldCheckCI = !isCustomTestRoot || process.env.KOKORO_FORCE_CI_CHECK === "1";
    if (
      shouldCheckCI &&
      (process.env.CI ||
        process.env.GITHUB_ACTIONS ||
        process.env.CONTINUOUS_INTEGRATION ||
        process.env.NODE_ENV === "production")
    ) {
      return { skipped: true, reason: "ci_environment" };
    }

    const isReleaseCmd = args.some((a) =>
      ["build", "--release", "-r", "release", "publish"].includes(a.toLowerCase())
    );
    if (isReleaseCmd) {
      return { skipped: true, reason: "release_command" };
    }
  }

  // Defense 0: Security Gate — Verify Path Chain & Canonical Containment
  const chainCheck = verifyPathChain(root, targetDir, debugDir);
  if (!chainCheck.valid) {
    if (isAuto) {
      if (isVerbose) {
        console.warn(`[WARN] Kokoro Storage Sentinel: target hierarchy check bypassed: ${chainCheck.reason}`);
      }
      return { skipped: true, reason: chainCheck.reason };
    }
    console.error(`❌ [Kokoro Storage Sentinel Security Alert] Refusing to traverse target directory: ${chainCheck.reason}`);
    process.exitCode = 1;
    return { skipped: true, reason: chainCheck.reason };
  }

  const canonicalRoot = chainCheck.canonicalRoot;
  const canonicalTarget = chainCheck.canonicalTarget;
  const canonicalDebug = chainCheck.canonicalDebug;

  // Feature: --doctor (Comprehensive Storage Health & Diagnostics Dashboard)
  if (isDoctor) {
    console.log("================================================================================");
    console.log("             Kokoro Storage Sentinel - Health & Diagnostics Dashboard");
    console.log("================================================================================\n");

    const drive = getDriveFreeSpace(targetDir, root);
    if (drive) {
      const usedPercent = (((drive.totalGb - drive.freeGb) / drive.totalGb) * 100).toFixed(1);
      const healthTag = drive.freeGb < 5 ? "⚠️ LOW DISK SPACE" : drive.freeGb < 15 ? "ℹ️ MODERATE" : "✅ HEALTHY";
      console.log(`[Drive Storage]   Available: ${drive.freeGb.toFixed(2)} GB / ${drive.totalGb.toFixed(2)} GB (${usedPercent}% used) [${healthTag}]`);
    }

    if (fs.existsSync(targetDir)) {
      const totalTarget = getDirSize(targetDir, canonicalTarget);
      console.log(`[Target Footprint] Total size: ${formatBytes(totalTarget)} (${(totalTarget / 1024 / 1024 / 1024).toFixed(2)} GB)`);

      const subdirs = ["debug", "release", "sherpa-onnx-prebuilt", "package"];
      for (const sub of subdirs) {
        const subPath = path.join(targetDir, sub);
        if (fs.existsSync(subPath)) {
          const sz = getDirSize(subPath, canonicalTarget);
          console.log(`  └─ target/${sub.padEnd(22)}: ${formatBytes(sz)}`);
          if (sub === "debug") {
            const debugSubs = ["deps", "incremental", "build"];
            for (const dsub of debugSubs) {
              const dsubPath = path.join(subPath, dsub);
              if (fs.existsSync(dsubPath)) {
                console.log(`       ├─ debug/${dsub.padEnd(16)}: ${formatBytes(getDirSize(dsubPath, canonicalTarget))}`);
              }
            }
          }
        }
      }
    } else {
      console.log("[Target Footprint] target directory not found (clean slate).");
    }

    let hooksStatus = "Not Configured";
    try {
      const gitEnv = getGitEnvironment(root);
      if (gitEnv.currentHooksPath === ".githooks") {
        hooksStatus = "✅ Active (.githooks registered)";
      } else if (gitEnv.currentHooksPath) {
        hooksStatus = `ℹ️ Preserved Custom (${gitEnv.currentHooksPath}) - Storage Sentinel fallback active`;
      } else {
        const postCheckout = gitEnv.hooksDir ? path.join(gitEnv.hooksDir, "post-checkout") : null;
        if (postCheckout && fs.existsSync(postCheckout)) {
          const content = fs.readFileSync(postCheckout, "utf8");
          if (content.includes(SENTINEL_BLOCK_START)) {
            hooksStatus = "✅ Active (chained in .git/hooks)";
          } else {
            hooksStatus = "ℹ️ Custom (.git/hooks has custom post-checkout)";
          }
        } else {
          hooksStatus = "⚠️ Default (.git/hooks, run 'npm run storage:setup-hooks' to activate)";
        }
      }
    } catch (_) {
      hooksStatus = "⚠️ Default (.git/hooks, run 'npm run storage:setup-hooks' to activate)";
    }
    console.log(`\n[Git Lifecycle]   Hooks Status: ${hooksStatus}`);

    let sccacheStatus = "Not Found (Install via: cargo install sccache --locked)";
    try {
      const sccacheVer = execSync("sccache --version", { stdio: ["ignore", "pipe", "ignore"], encoding: "utf8" }).trim();
      sccacheStatus = `✅ Installed (${sccacheVer})`;
    } catch (_) {}
    console.log(`[Build Cache]     Sccache: ${sccacheStatus}`);

    console.log("\n--------------------------------------------------------------------------------");
    console.log("Recommendations:");
    if (drive && drive.freeGb < 10) {
      console.log("  - [URGENT] Drive free space is under 10 GB. Run 'npm run prune:target' or free disk space.");
    }
    if (!sccacheStatus.includes("✅")) {
      console.log("  - [SPEEDUP] Enable sccache for instant branch switches and clean builds.");
    }
    console.log("  - [DAILY]  Watchdog automatically maintains target under 12 GB during 'npm run dev'.");
    console.log("================================================================================\n");
    return { success: true, mode: "doctor" };
  }

  // Defense 2 & 3: Branch Switch Sensing & Drive-level Emergency Low-Disk Shield
  const cooldownFile = path.join(targetDir, ".prune-cooldown");
  const branchSwitchFile = path.join(targetDir, ".branch-switched");
  const emergencyFreeGbThreshold = parseFloat(process.env.KOKORO_EMERGENCY_FREE_GB || "5.0");
  const cooldownMinutes = parseFloat(process.env.KOKORO_PRUNE_COOLDOWN_MINUTES || "120");

  let hasBranchSwitched = fs.existsSync(branchSwitchFile);
  if (hasBranchSwitched) {
    try {
      const bsLstat = fs.lstatSync(branchSwitchFile);
      if (!bsLstat.isSymbolicLink() && bsLstat.isFile()) {
        const content = fs.readFileSync(branchSwitchFile, "utf8").trim();
        const ts = parseInt(content, 10);
        // post-checkout writes seconds timestamp `date +%s`
        const timeMs = !isNaN(ts) && ts < 1e11 ? ts * 1000 : bsLstat.mtimeMs;
        const maxAgeMs = 24 * 60 * 60 * 1000;
        if (Date.now() - timeMs > maxAgeMs) {
          if (!isDryRun) {
            consumeBranchSwitchMarker(targetDir, branchSwitchFile, canonicalRoot, isDryRun);
          }
          hasBranchSwitched = false;
        }
      }
    } catch (_) {}
  }

  const drive = getDriveFreeSpace(targetDir, root);
  const isEmergency = drive ? drive.freeGb < emergencyFreeGbThreshold : false;

  if (isAuto && !isEmergency && !hasBranchSwitched && fs.existsSync(cooldownFile)) {
    try {
      const lastCheck = parseInt(fs.readFileSync(cooldownFile, "utf8").trim(), 10);
      const now = Date.now();
      if (!isNaN(lastCheck) && now - lastCheck < cooldownMinutes * 60 * 1000) {
        return { skipped: true, reason: "cooldown_active" };
      }
    } catch (_) {}
  }

  let effectiveThresholdGb = explicitThresholdGb;
  if (isAuto && effectiveThresholdGb === 0) {
    effectiveThresholdGb = isEmergency ? 8.0 : parseFloat(process.env.KOKORO_PRUNE_THRESHOLD_GB || "12");
  }

  if (!fs.existsSync(targetDir)) {
    if (!isAuto) {
      console.log(`Target directory not found: ${targetDir}`);
      console.log("Nothing to prune.");
    }
    return { count: 0, bytesFreed: 0 };
  }

  const initialTargetSize = getDirSize(targetDir, canonicalTarget);
  const initialTargetGb = initialTargetSize / (1024 * 1024 * 1024);
  const hasDebugDir = fs.existsSync(debugDir);
  const initialDebugSize = hasDebugDir ? getDirSize(debugDir, canonicalDebug) : 0;

  if (!isEmergency && !hasBranchSwitched && effectiveThresholdGb > 0 && initialTargetGb < effectiveThresholdGb) {
    if (isAuto) {
      if (!isDryRun) {
        writeCooldown(targetDir, cooldownFile, canonicalRoot, isDryRun);
      }
      return { skipped: true, reason: "below_threshold" };
    } else {
      console.log(`Target footprint (${initialTargetGb.toFixed(2)} GB) is below threshold (${effectiveThresholdGb} GB). Skipping.`);
      return { skipped: true, reason: "below_threshold" };
    }
  }

  if (!hasDebugDir) {
    if (isAuto) {
      if (!isDryRun) {
        writeCooldown(targetDir, cooldownFile, canonicalRoot, isDryRun);
      }
      return { count: 0, bytesFreed: 0, skipped: true, reason: "no_debug_cache" };
    } else {
      console.log(`Debug target directory not found: ${debugDir}`);
      console.log(`Non-debug target footprint (${initialTargetGb.toFixed(2)} GB) cannot be automatically pruned.`);
      console.log("Run 'npm run storage:doctor' for diagnostics or 'npm run clean:target' if a full reset is needed.");
      return { count: 0, bytesFreed: 0 };
    }
  }

  // NOTE: We deliberately do NOT consume the branch switch marker here.
  // It is only consumed after pruning has completed without active lock conflicts.

  if (!isAuto) {
    console.log("================================================================================");
    console.log("         Kokoro Storage Sentinel - Target Pruner & Watchdog");
    console.log("================================================================================");
    if (hasBranchSwitched) {
      console.log("[EVENT] Branch switch detected: running deep orphan eviction.");
    }
    if (isEmergency) {
      console.log(`[ALERT] Low host drive space (< ${emergencyFreeGbThreshold} GB free): emergency pruning activated!`);
    }
    console.log(`Current target footprint: ${formatBytes(initialTargetSize)} (target/debug: ${formatBytes(initialDebugSize)})`);
    if (isDryRun) {
      console.log("[DRY-RUN MODE] No files will actually be deleted.");
    }
    console.log("Pruning obsolete incremental sessions, test binaries, and duplicate artifacts...");
  }

  const incKeepSessions = (hasBranchSwitched || isEmergency) ? 1 : 2;
  const incResult = pruneIncremental(
    path.join(debugDir, "incremental"),
    incKeepSessions,
    canonicalDebug,
    isDryRun,
    isVerbose,
    isAuto,
    isEmergency
  );
  const depsResult = pruneDeps(
    path.join(debugDir, "deps"),
    isEmergency,
    canonicalDebug,
    isDryRun,
    isVerbose,
    hasBranchSwitched
  );

  const totalCount = incResult.count + depsResult.count;
  const totalFreed = incResult.bytesFreed + depsResult.bytesFreed;
  const finalTargetSize = isDryRun ? initialTargetSize - totalFreed : getDirSize(targetDir, canonicalTarget);

  // Safe Marker Consumption & Cooldown Lifecycle:
  // Only consume .branch-switched and set full cooldown when cleanup was safe (no active locks and not incomplete)
  const isCleanupSafe = !incResult.hasActiveLock && !incResult.incomplete && !depsResult.incomplete;

  if (!isDryRun) {
    if (isCleanupSafe) {
      updateCooldown(targetDir, cooldownFile, branchSwitchFile, hasBranchSwitched, canonicalRoot, isDryRun);
    } else {
      if (hasBranchSwitched && (isVerbose || !isAuto)) {
        console.warn("[WARN] Active compiler locks or incomplete cleanup detected; branch switch marker retained.");
      }
    }
  }

  if (isAuto) {
    if (isEmergency && totalFreed > 0) {
      console.log(`🚨 [Kokoro Storage Alert] Low drive space (Free: ${drive.freeGb.toFixed(1)} GB)! Emergency prune freed ${formatBytes(totalFreed)} (target: ${formatBytes(finalTargetSize)}).`);
    } else if (hasBranchSwitched && totalFreed > 0) {
      console.log(`🌿 [Kokoro Storage Sentinel] Branch switch cache pruned: freed ${formatBytes(totalFreed)} (target: ${formatBytes(finalTargetSize)}).`);
    } else if (totalFreed > 0) {
      console.log(`💡 [Kokoro Storage Watchdog] Cleaned ${formatBytes(totalFreed)} of stale build cache (target: ${formatBytes(finalTargetSize)}).`);
    }
  } else {
    console.log("--------------------------------------------------------------------------------");
    console.log(`Incremental pruned : ${incResult.count} sessions, freed ${formatBytes(incResult.bytesFreed)}`);
    console.log(`Deps pruned        : ${depsResult.count} files, freed ${formatBytes(depsResult.bytesFreed)}`);
    console.log(`Total storage freed: ${formatBytes(totalFreed)} (${totalCount} items pruned)`);
    console.log(`Final target size  : ${formatBytes(finalTargetSize)}`);
    console.log("--------------------------------------------------------------------------------");
    console.log("Done! Active incremental compilation cache remains intact.");
  }

  return {
    count: totalCount,
    bytesFreed: totalFreed,
    initialSize: initialTargetSize,
    finalSize: finalTargetSize,
    initialTargetSize,
    finalTargetSize,
    initialDebugSize,
  };
}

// =============================================================================
// Module Exports & Direct CLI Entrypoint
// =============================================================================

export {
  stripUncPrefix,
  normalizeCanonicalPath,
  getSafeRealPath,
  isCanonicallyContained,
  verifyPathChain,
  getDirSize,
  getDriveFreeSpace,
  safeRemove,
  isLockFileActive,
  isSessionDirectoryLocked,
  hasActiveIncrementalLocks,
  consumeBranchSwitchMarker,
  writeCooldown,
  pruneIncremental,
  pruneDeps,
  updateCooldown,
  getGitEnvironment,
  getActiveHooks,
  isPosixShellScript,
  injectSentinelBlockIntoShellScript,
  getSentinelSnippet,
  SENTINEL_BLOCK_START,
  SENTINEL_BLOCK_END,
  KNOWN_TEST_HARNESSES,
  isEligibleDepsArtifact,
  setGitExecutorForTesting,
  runPruneTarget,
};

const invokedPath = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : null;
if (invokedPath === import.meta.url) {
  runPruneTarget(process.argv.slice(2));
}
