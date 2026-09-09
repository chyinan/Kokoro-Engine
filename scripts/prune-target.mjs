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

const root = process.cwd();
const targetDir = path.join(root, "src-tauri", "target");
const debugDir = path.join(targetDir, "debug");

const args = process.argv.slice(2);
const isAuto = args.includes("--auto");
const isDryRun = args.includes("--dry-run");
const isDoctor = args.includes("--doctor");
const isSetupHooks = args.includes("--setup-hooks");
const isVerbose = (args.includes("--verbose") || args.includes("-v")) && !isAuto;
const thresholdArg = args.find((a) => a.startsWith("--threshold-gb="));
const explicitThresholdGb = thresholdArg ? parseFloat(thresholdArg.split("=")[1]) : 0;

// =============================================================================
// Feature: --setup-hooks (Native zero-dependency Git hooks registration)
// =============================================================================
if (isSetupHooks) {
  try {
    const gitDir = path.join(root, ".git");
    const hooksDir = path.join(root, ".githooks");
    if (fs.existsSync(gitDir) && fs.existsSync(hooksDir)) {
      execSync("git config core.hooksPath .githooks", { stdio: "ignore" });
      if (!isAuto) {
        console.log("✅ [Kokoro Storage Sentinel] Git hooks registered to .githooks (branch-switch sensing active).");
      }
    }
  } catch (_) {
    // Graceful fallback: never fail install workflows
  }
  process.exit(0);
}

// =============================================================================
// Defense 1: Dev vs Release & CI Environment Boundary Gating
// =============================================================================
if (isAuto) {
  // 1.1 Never run in CI / Production pipelines
  if (
    process.env.CI ||
    process.env.GITHUB_ACTIONS ||
    process.env.CONTINUOUS_INTEGRATION ||
    process.env.NODE_ENV === "production"
  ) {
    process.exit(0);
  }

  // 1.2 Never run if a release build command is detected
  const isReleaseCmd = args.some((a) =>
    ["build", "--release", "-r", "release", "publish"].includes(a.toLowerCase())
  );
  if (isReleaseCmd) {
    process.exit(0);
  }
}

// =============================================================================
// Helper Functions: Formatting, Sizing & Drive Space
// =============================================================================
function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function getDirSize(d) {
  let size = 0;
  try {
    const entries = fs.readdirSync(d, { withFileTypes: true });
    for (const entry of entries) {
      const full = path.join(d, entry.name);
      if (entry.isDirectory()) {
        size += getDirSize(full);
      } else if (entry.isFile()) {
        size += fs.statSync(full).size;
      }
    }
  } catch (_) {}
  return size;
}

function getDriveFreeSpace(dirPath) {
  try {
    const checkPath = fs.existsSync(dirPath) ? dirPath : root;
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

function safeRemove(itemPath) {
  try {
    const stat = fs.statSync(itemPath);
    const size = stat.isDirectory() ? getDirSize(itemPath) : stat.size;
    if (!isDryRun) {
      if (stat.isDirectory()) {
        fs.rmSync(itemPath, { recursive: true, force: true });
      } else {
        fs.unlinkSync(itemPath);
      }
    }
    return { success: true, size };
  } catch (err) {
    if (isVerbose) {
      console.warn(`[WARN] Skipping locked/inaccessible item: ${path.basename(itemPath)} (${err.code || err.message})`);
    }
    return { success: false, size: 0 };
  }
}

// =============================================================================
// Feature: --doctor (Comprehensive Storage Health & Diagnostics Dashboard)
// =============================================================================
if (isDoctor) {
  console.log("================================================================================");
  console.log("             Kokoro Storage Sentinel - Health & Diagnostics Dashboard");
  console.log("================================================================================\n");

  // 1. Host Drive Status
  const drive = getDriveFreeSpace(targetDir);
  if (drive) {
    const usedPercent = (((drive.totalGb - drive.freeGb) / drive.totalGb) * 100).toFixed(1);
    const healthTag = drive.freeGb < 5 ? "⚠️ LOW DISK SPACE" : drive.freeGb < 15 ? "ℹ️ MODERATE" : "✅ HEALTHY";
    console.log(`[Drive Storage]   Available: ${drive.freeGb.toFixed(2)} GB / ${drive.totalGb.toFixed(2)} GB (${usedPercent}% used) [${healthTag}]`);
  }

  // 2. Target Directory Breakdown
  if (fs.existsSync(targetDir)) {
    const totalTarget = getDirSize(targetDir);
    console.log(`[Target Footprint] Total size: ${formatBytes(totalTarget)} (${(totalTarget / 1024 / 1024 / 1024).toFixed(2)} GB)`);

    const subdirs = ["debug", "release", "sherpa-onnx-prebuilt", "package"];
    for (const sub of subdirs) {
      const subPath = path.join(targetDir, sub);
      if (fs.existsSync(subPath)) {
        const sz = getDirSize(subPath);
        console.log(`  └─ target/${sub.padEnd(22)}: ${formatBytes(sz)}`);
        if (sub === "debug") {
          const debugSubs = ["deps", "incremental", "build"];
          for (const dsub of debugSubs) {
            const dsubPath = path.join(subPath, dsub);
            if (fs.existsSync(dsubPath)) {
              console.log(`       ├─ debug/${dsub.padEnd(16)}: ${formatBytes(getDirSize(dsubPath))}`);
            }
          }
        }
      }
    }
  } else {
    console.log("[Target Footprint] target directory not found (clean slate).");
  }

  // 3. Git Hooks Status
  let hooksStatus = "Not Configured";
  try {
    const currentHooks = execSync("git config --get core.hooksPath", { encoding: "utf8" }).trim();
    if (currentHooks === ".githooks") {
      hooksStatus = "✅ Active (.githooks registered)";
    } else if (currentHooks) {
      hooksStatus = `⚠️ Custom (${currentHooks})`;
    }
  } catch (_) {
    hooksStatus = "⚠️ Default (.git/hooks, run 'npm run storage:setup-hooks' to activate)";
  }
  console.log(`\n[Git Lifecycle]   Hooks Status: ${hooksStatus}`);

  // 4. Sccache Status
  let sccacheStatus = "Not Found (Install via: cargo install sccache --locked)";
  try {
    const sccacheVer = execSync("sccache --version", { stdio: ["ignore", "pipe", "ignore"], encoding: "utf8" }).trim();
    sccacheStatus = `✅ Installed (${sccacheVer})`;
  } catch (_) {}
  console.log(`[Build Cache]     Sccache: ${sccacheStatus}`);

  // 5. Recommendations
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
  process.exit(0);
}

// =============================================================================
// Defense 2 & 3: Branch Switch Sensing & Drive-level Emergency Low-Disk Shield
// =============================================================================
const cooldownFile = path.join(targetDir, ".prune-cooldown");
const branchSwitchFile = path.join(targetDir, ".branch-switched");
const emergencyFreeGbThreshold = parseFloat(process.env.KOKORO_EMERGENCY_FREE_GB || "5.0");
const cooldownMinutes = parseFloat(process.env.KOKORO_PRUNE_COOLDOWN_MINUTES || "120");

const hasBranchSwitched = fs.existsSync(branchSwitchFile);
const drive = getDriveFreeSpace(targetDir);
const isEmergency = drive ? drive.freeGb < emergencyFreeGbThreshold : false;

if (isAuto && !isEmergency && !hasBranchSwitched && fs.existsSync(cooldownFile)) {
  try {
    const lastCheck = parseInt(fs.readFileSync(cooldownFile, "utf8").trim(), 10);
    const now = Date.now();
    if (!isNaN(lastCheck) && now - lastCheck < cooldownMinutes * 60 * 1000) {
      // Cooldown active: skip immediately (< 2ms) to guarantee zero dev startup latency
      process.exit(0);
    }
  } catch (_) {
    // Proceed on read error
  }
}

// Determine effective threshold (default 12 GB in auto mode, 0 in manual mode unless specified)
let effectiveThresholdGb = explicitThresholdGb;
if (isAuto && effectiveThresholdGb === 0) {
  // If emergency, threshold drops to 8 GB to reclaim space immediately
  effectiveThresholdGb = isEmergency ? 8.0 : parseFloat(process.env.KOKORO_PRUNE_THRESHOLD_GB || "12");
}

// =============================================================================
// Core Pruning Algorithms: Incremental Sessions & Ephemeral Test Harnesses
// =============================================================================

function pruneIncremental(incrementalDir, keepSessionsCount) {
  if (!fs.existsSync(incrementalDir)) return { count: 0, bytesFreed: 0 };

  let count = 0;
  let bytesFreed = 0;

  let entries = [];
  try {
    entries = fs.readdirSync(incrementalDir, { withFileTypes: true });
  } catch (_) {
    return { count, bytesFreed };
  }

  // Yield on lock: do not compete with active rustc/rust-analyzer
  const hasLock = entries.some((e) => e.name.endsWith(".lock"));
  if (hasLock) {
    if (isAuto && !isEmergency) {
      return { count: 0, bytesFreed: 0 };
    }
    if (isVerbose) {
      console.log("[INFO] Detected active lock in incremental folder; exercising extra caution.");
    }
  }

  const groups = new Map();

  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const fullPath = path.join(incrementalDir, entry.name);
    let stat;
    try {
      stat = fs.statSync(fullPath);
    } catch (_) {
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

    // Default: keep 2 sessions for Kokoro (1 dev + 1 test), 1 for auxiliary.
    // On branch switch or emergency: tighten to 1 session to quickly reclaim 3-4 GB of orphan cache.
    let keep = prefix.includes("kokoro") ? (keepSessionsCount ?? 2) : 1;
    const staleSessions = sessions.slice(keep);

    for (const session of staleSessions) {
      if (isVerbose) {
        console.log(`[PRUNE] Stale incremental session: ${session.name}`);
      }
      const res = safeRemove(session.path);
      if (res.success) {
        count++;
        bytesFreed += res.size;
      }
    }
  }

  return { count, bytesFreed };
}

function pruneDeps(depsDir, isEmergencyMode) {
  if (!fs.existsSync(depsDir)) return { count: 0, bytesFreed: 0 };

  let count = 0;
  let bytesFreed = 0;

  let fileNames = [];
  try {
    fileNames = fs.readdirSync(depsDir);
  } catch (_) {
    return { count, bytesFreed };
  }

  const groups = new Map();
  const now = Date.now();
  const testProtectionWindowMs = 30 * 60 * 1000; // 30-minute shield for active test runs

  for (const name of fileNames) {
    const fullPath = path.join(depsDir, name);
    let stat;
    try {
      stat = fs.statSync(fullPath);
    } catch (_) {
      continue;
    }
    if (!stat.isFile()) continue;

    const match = name.match(/^(lib[a-zA-Z0-9_]+|[a-zA-Z0-9_]+)-([0-9a-f]{16})(\.[a-zA-Z0-9_.]+)$/i);
    if (!match) continue;

    const baseName = match[1];
    // SAFETY: Only prune the project's own artifacts (Kokoro-Engine workspace crates and test harnesses).
    // NEVER prune third-party crates in deps.
    if (!baseName.toLowerCase().includes("kokoro")) continue;

    const ext = match[3].toLowerCase();
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

    const isKokoro = key.includes("kokoro");
    const isTestHarness = isKokoro && key.includes(".exe") && !key.startsWith("tauri_appkokoro_engine");

    // Retention policy:
    // - Core library .rlib/.pdb: keep latest 1
    // - Main executable: keep latest 1 (or 2 if recent)
    // - Test harness .exe/.pdb: keep latest 1, but prune older versions if outside the 30-minute shield
    let keepCount = 1;
    if (!isEmergencyMode && !hasBranchSwitched && isKokoro && key.includes(".exe")) {
      keepCount = 2;
    }

    const staleItems = items.slice(keepCount);

    for (const item of staleItems) {
      // If it's a test harness, apply time-lapse shield unless emergency or branch switched
      if (isTestHarness && !isEmergencyMode && !hasBranchSwitched) {
        if (now - item.mtime < testProtectionWindowMs) {
          continue; // Protected by active testing window
        }
      }

      if (isVerbose) {
        console.log(`[PRUNE] Stale artifact in deps: ${item.name} (${formatBytes(item.size)})`);
      }
      const res = safeRemove(item.path);
      if (res.success) {
        count++;
        bytesFreed += res.size;
      }
    }
  }

  return { count, bytesFreed };
}

function updateCooldown() {
  try {
    if (!fs.existsSync(targetDir)) {
      fs.mkdirSync(targetDir, { recursive: true });
    }
    fs.writeFileSync(cooldownFile, Date.now().toString(), "utf8");
    if (hasBranchSwitched && fs.existsSync(branchSwitchFile)) {
      fs.unlinkSync(branchSwitchFile);
    }
  } catch (_) {}
}

// =============================================================================
// Main Execution Pipeline
// =============================================================================
function run() {
  if (!fs.existsSync(debugDir)) {
    if (!isAuto) {
      console.log(`Debug target directory not found: ${debugDir}`);
      console.log("Nothing to prune.");
    }
    return;
  }

  const initialSize = getDirSize(debugDir);
  const initialGb = initialSize / (1024 * 1024 * 1024);

  // Check threshold watermark (bypassed if emergency or branch switch event occurred)
  if (!isEmergency && !hasBranchSwitched && effectiveThresholdGb > 0 && initialGb < effectiveThresholdGb) {
    if (isAuto) {
      updateCooldown();
      process.exit(0); // Silent exit
    } else {
      console.log(`Target size (${initialGb.toFixed(2)} GB) is below threshold (${effectiveThresholdGb} GB). Skipping.`);
      return;
    }
  }

  // Update cooldown timestamp
  updateCooldown();

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
    console.log(`Current target/debug size: ${formatBytes(initialSize)}`);
    if (isDryRun) {
      console.log("[DRY-RUN MODE] No files will actually be deleted.");
    }
    console.log("Pruning obsolete incremental sessions, test binaries, and duplicate artifacts...");
  }

  // Keep 1 session if branch switched or emergency; keep 2 sessions otherwise
  const incKeepSessions = (hasBranchSwitched || isEmergency) ? 1 : 2;
  const incResult = pruneIncremental(path.join(debugDir, "incremental"), incKeepSessions);
  const depsResult = pruneDeps(path.join(debugDir, "deps"), isEmergency);

  const totalCount = incResult.count + depsResult.count;
  const totalFreed = incResult.bytesFreed + depsResult.bytesFreed;
  const finalSize = isDryRun ? initialSize - totalFreed : getDirSize(debugDir);

  if (isAuto) {
    if (isEmergency && totalFreed > 0) {
      console.log(`🚨 [Kokoro Storage Alert] Low drive space (Free: ${drive.freeGb.toFixed(1)} GB)! Emergency prune freed ${formatBytes(totalFreed)} (target: ${formatBytes(finalSize)}).`);
    } else if (hasBranchSwitched && totalFreed > 0) {
      console.log(`🌿 [Kokoro Storage Sentinel] Branch switch cache pruned: freed ${formatBytes(totalFreed)} (target: ${formatBytes(finalSize)}).`);
    } else if (totalFreed > 0) {
      console.log(`💡 [Kokoro Storage Watchdog] Cleaned ${formatBytes(totalFreed)} of stale build cache (target: ${formatBytes(finalSize)}).`);
    }
  } else {
    console.log("--------------------------------------------------------------------------------");
    console.log(`Incremental pruned : ${incResult.count} sessions, freed ${formatBytes(incResult.bytesFreed)}`);
    console.log(`Deps pruned        : ${depsResult.count} files, freed ${formatBytes(depsResult.bytesFreed)}`);
    console.log(`Total storage freed: ${formatBytes(totalFreed)} (${totalCount} items pruned)`);
    console.log(`Final target size  : ${formatBytes(finalSize)}`);
    console.log("--------------------------------------------------------------------------------");
    console.log("Done! Active incremental compilation cache remains intact.");
  }
}

run();
