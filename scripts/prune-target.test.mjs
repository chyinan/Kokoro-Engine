import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import crypto from "node:crypto";
import * as childProcess from "node:child_process";
import {
  stripUncPrefix,
  normalizeCanonicalPath,
  isCanonicallyContained,
  verifyPathChain,
  getDirSize,
  safeRemove,
  isLockFileActive,
  isSessionDirectoryLocked,
  hasActiveIncrementalLocks,
  consumeBranchSwitchMarker,
  writeCooldown,
  updateCooldown,
  pruneIncremental,
  pruneDeps,
  runPruneTarget,
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
} from "./prune-target.mjs";

describe("scripts/prune-target.mjs security & canonical containment", () => {
  let tempRoot;
  let savedEnv = {};

  beforeEach(() => {
    savedEnv = {
      CI: process.env.CI,
      GITHUB_ACTIONS: process.env.GITHUB_ACTIONS,
      CONTINUOUS_INTEGRATION: process.env.CONTINUOUS_INTEGRATION,
      NODE_ENV: process.env.NODE_ENV,
      KOKORO_SKIP_HOOKS: process.env.KOKORO_SKIP_HOOKS,
      KOKORO_FORCE_CI_CHECK: process.env.KOKORO_FORCE_CI_CHECK,
    };
    delete process.env.CI;
    delete process.env.GITHUB_ACTIONS;
    delete process.env.CONTINUOUS_INTEGRATION;
    delete process.env.NODE_ENV;
    delete process.env.KOKORO_SKIP_HOOKS;
    delete process.env.KOKORO_FORCE_CI_CHECK;

    tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "kokoro-prune-test-"));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    for (const [key, val] of Object.entries(savedEnv)) {
      if (val === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = val;
      }
    }
    if (tempRoot && fs.existsSync(tempRoot)) {
      try {
        fs.rmSync(tempRoot, { recursive: true, force: true });
      } catch (_) {}
    }
  });

  // Helper to create directory junction on Windows or symlink on Unix
  function createDirLink(targetDir, linkPath) {
    const type = process.platform === "win32" ? "junction" : "dir";
    fs.symlinkSync(targetDir, linkPath, type);
  }

  // Helper to safely attempt creating file link (handles Windows unprivileged environments)
  function tryCreateFileLink(targetFile, linkPath) {
    try {
      fs.symlinkSync(targetFile, linkPath, "file");
      return true;
    } catch (err) {
      if (err?.code === "EPERM" || err?.code === "EACCES") {
        return false;
      }
      throw err;
    }
  }

  describe("Path normalization and canonical containment helpers", () => {
    it("stripUncPrefix removes Windows extended path prefixes", () => {
      expect(stripUncPrefix("\\\\?\\D:\\repo")).toBe("D:\\repo");
      expect(stripUncPrefix("\\\\?\\UNC\\server\\share")).toBe("\\\\server\\share");
      expect(stripUncPrefix("C:\\regular\\path")).toBe("C:\\regular\\path");
      expect(stripUncPrefix("/unix/path")).toBe("/unix/path");
    });

    it("isCanonicallyContained validates strict descendancy", () => {
      const parent = path.join(tempRoot, "parent");
      const child = path.join(parent, "child");
      const deepChild = path.join(parent, "sub", "deep");
      const outside = path.join(tempRoot, "outside");

      expect(isCanonicallyContained(child, parent)).toBe(true);
      expect(isCanonicallyContained(deepChild, parent)).toBe(true);

      // Self containment is false (must be strictly contained)
      expect(isCanonicallyContained(parent, parent)).toBe(false);

      // Outside paths are rejected
      expect(isCanonicallyContained(outside, parent)).toBe(false);
      expect(isCanonicallyContained(parent, child)).toBe(false);
    });

    it("isCanonicallyContained handles case folding on Windows", () => {
      if (process.platform === "win32") {
        const parentLower = "d:\\kokoro-engine\\src-tauri\\target\\debug";
        const childUpper = "D:\\KOKORO-ENGINE\\SRC-TAURI\\TARGET\\DEBUG\\incremental\\session-1";
        expect(isCanonicallyContained(childUpper, parentLower)).toBe(true);
      }
    });
  });

  describe("verifyPathChain defense gates", () => {
    it("approves valid directory hierarchy", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const target = path.join(srcTauri, "target");
      const debug = path.join(target, "debug");

      fs.mkdirSync(debug, { recursive: true });

      const check = verifyPathChain(tempRoot, target, debug);
      expect(check.valid).toBe(true);
      expect(check.canonicalRoot).toBeDefined();
      expect(check.canonicalTarget).toBeDefined();
      expect(check.canonicalDebug).toBeDefined();
    });

    it("approves clean slate when target/debug do not exist yet", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      fs.mkdirSync(srcTauri, { recursive: true });

      const target = path.join(srcTauri, "target");
      const debug = path.join(target, "debug");

      const check = verifyPathChain(tempRoot, target, debug);
      expect(check.valid).toBe(true);
    });

    it("rejects if src-tauri is a link or junction", () => {
      const externalDir = path.join(tempRoot, "external-src-tauri");
      fs.mkdirSync(externalDir, { recursive: true });

      const srcTauriLink = path.join(tempRoot, "src-tauri");
      createDirLink(externalDir, srcTauriLink);

      const target = path.join(srcTauriLink, "target");
      const debug = path.join(target, "debug");

      const check = verifyPathChain(tempRoot, target, debug);
      expect(check.valid).toBe(false);
      expect(check.reason).toMatch(/symbolic link or junction/i);
    });

    it("rejects if target is a link or junction to external directory", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      fs.mkdirSync(srcTauri, { recursive: true });

      const externalTarget = path.join(tempRoot, "external-target");
      fs.mkdirSync(externalTarget, { recursive: true });

      const targetLink = path.join(srcTauri, "target");
      createDirLink(externalTarget, targetLink);

      const debug = path.join(targetLink, "debug");

      const check = verifyPathChain(tempRoot, targetLink, debug);
      expect(check.valid).toBe(false);
      expect(check.reason).toMatch(/symbolic link or junction|canonical divergence/i);
    });

    it("rejects if debug is a link or junction", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const target = path.join(srcTauri, "target");
      fs.mkdirSync(target, { recursive: true });

      const externalDebug = path.join(tempRoot, "external-debug");
      fs.mkdirSync(externalDebug, { recursive: true });

      const debugLink = path.join(target, "debug");
      createDirLink(externalDebug, debugLink);

      const check = verifyPathChain(tempRoot, target, debugLink);
      expect(check.valid).toBe(false);
      expect(check.reason).toMatch(/symbolic link or junction|canonical divergence/i);
    });
  });

  describe("getDirSize link defense", () => {
    it("calculates regular files and skips linked subdirectories", () => {
      const testDir = path.join(tempRoot, "calc-test");
      fs.mkdirSync(testDir, { recursive: true });

      const file1 = path.join(testDir, "file1.bin");
      fs.writeFileSync(file1, Buffer.alloc(1000));

      const externalDir = path.join(tempRoot, "external-data");
      fs.mkdirSync(externalDir, { recursive: true });
      fs.writeFileSync(path.join(externalDir, "secret.bin"), Buffer.alloc(50000));

      // Link external data into testDir
      const linkInside = path.join(testDir, "linked-sub");
      createDirLink(externalDir, linkInside);

      const size = getDirSize(testDir, testDir);
      // Size must only be 1000 bytes, ignoring the 50000 bytes external folder
      expect(size).toBe(1000);
    });

    it("returns 0 if the directory itself is a link", () => {
      const externalDir = path.join(tempRoot, "external-dir");
      fs.mkdirSync(externalDir, { recursive: true });
      fs.writeFileSync(path.join(externalDir, "file.bin"), Buffer.alloc(5000));

      const linkDir = path.join(tempRoot, "linked-target");
      createDirLink(externalDir, linkDir);

      expect(getDirSize(linkDir)).toBe(0);
    });
  });

  describe("safeRemove boundary protection", () => {
    it("removes regular files and directories within boundary", () => {
      const boundary = path.join(tempRoot, "safe-boundary");
      fs.mkdirSync(boundary, { recursive: true });

      const regularFile = path.join(boundary, "artifact.bin");
      fs.writeFileSync(regularFile, "data");

      const res = safeRemove(regularFile, boundary);
      expect(res.success).toBe(true);
      expect(fs.existsSync(regularFile)).toBe(false);

      const regularSub = path.join(boundary, "session-dir");
      fs.mkdirSync(regularSub);
      fs.writeFileSync(path.join(regularSub, "subfile.txt"), "hello");

      const resDir = safeRemove(regularSub, boundary);
      expect(resDir.success).toBe(true);
      expect(fs.existsSync(regularSub)).toBe(false);
    });

    it("refuses to delete symbolic links or junctions, leaving target intact", () => {
      const boundary = path.join(tempRoot, "boundary");
      fs.mkdirSync(boundary, { recursive: true });

      const externalVitalDir = path.join(tempRoot, "external-vital-dir");
      fs.mkdirSync(externalVitalDir);
      fs.writeFileSync(path.join(externalVitalDir, "vital.txt"), "important system data");

      const linkedDir = path.join(boundary, "link-to-vital-dir");
      createDirLink(externalVitalDir, linkedDir);

      const res = safeRemove(linkedDir, boundary);
      expect(res.success).toBe(false);
      expect(res.reason).toBe("symlink");

      // External directory & file must not be modified or deleted!
      expect(fs.existsSync(externalVitalDir)).toBe(true);
      expect(fs.existsSync(path.join(externalVitalDir, "vital.txt"))).toBe(true);
      expect(fs.readFileSync(path.join(externalVitalDir, "vital.txt"), "utf8")).toBe("important system data");

      // Also test file link if permitted
      const externalFile = path.join(tempRoot, "external-vital-file.txt");
      fs.writeFileSync(externalFile, "file data");
      const linkedFile = path.join(boundary, "link-to-vital.txt");
      if (tryCreateFileLink(externalFile, linkedFile)) {
        const resFile = safeRemove(linkedFile, boundary);
        expect(resFile.success).toBe(false);
        expect(resFile.reason).toBe("symlink");
        expect(fs.existsSync(externalFile)).toBe(true);
      }
    });

    it("refuses to delete items outside boundary", () => {
      const boundary = path.join(tempRoot, "boundary");
      fs.mkdirSync(boundary, { recursive: true });

      const outsideFile = path.join(tempRoot, "outside.txt");
      fs.writeFileSync(outsideFile, "outside content");

      const res = safeRemove(outsideFile, boundary);
      expect(res.success).toBe(false);
      expect(res.reason).toBe("boundary_violation");
      expect(fs.existsSync(outsideFile)).toBe(true);
    });
  });

  describe("pruneIncremental and pruneDeps link rejection", () => {
    it("prunes stale incremental sessions but skips linked sessions", () => {
      const boundary = path.join(tempRoot, "debug");
      const incDir = path.join(boundary, "incremental");
      fs.mkdirSync(incDir, { recursive: true });

      // Stale session 1: regular directory
      const sess1 = path.join(incDir, "kokoro-0123456789");
      fs.mkdirSync(sess1);
      fs.writeFileSync(path.join(sess1, "data.bin"), "test1");

      // Stale session 2: regular directory (newer)
      const sess2 = path.join(incDir, "kokoro-012345678a");
      fs.mkdirSync(sess2);
      fs.writeFileSync(path.join(sess2, "data.bin"), "test2");

      // External folder disguised as session 3
      const externalVitalDir = path.join(tempRoot, "vital-user-data");
      fs.mkdirSync(externalVitalDir);
      fs.writeFileSync(path.join(externalVitalDir, "documents.txt"), "vital docs");

      const sessLink = path.join(incDir, "kokoro-012345678b");
      createDirLink(externalVitalDir, sessLink);

      // Keep 1 session -> sess1 will be pruned, sessLink should be rejected/skipped
      const res = pruneIncremental(incDir, 1, boundary);
      expect(res.count).toBe(1);

      // Verify external folder is untouched
      expect(fs.existsSync(externalVitalDir)).toBe(true);
      expect(fs.existsSync(path.join(externalVitalDir, "documents.txt"))).toBe(true);
      expect(fs.readFileSync(path.join(externalVitalDir, "documents.txt"), "utf8")).toBe("vital docs");
    });

    it("prunes stale deps files but skips linked files", () => {
      const boundary = path.join(tempRoot, "debug");
      const depsDir = path.join(boundary, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      // Create regular older kokoro artifact
      const artifactOld = path.join(depsDir, "libkokoro_app-1111222233334444.rlib");
      fs.writeFileSync(artifactOld, "old rlib");
      fs.utimesSync(artifactOld, new Date(Date.now() - 100000), new Date(Date.now() - 100000));

      // Create regular newer kokoro artifact
      const artifactNew = path.join(depsDir, "libkokoro_app-5555666677778888.rlib");
      fs.writeFileSync(artifactNew, "new rlib");

      // Create external vital file linked into deps
      const externalVitalFile = path.join(tempRoot, "my-external-config.json");
      fs.writeFileSync(externalVitalFile, "{\"vital\": true}");

      const artifactLinked = path.join(depsDir, "libkokoro_app-9999000011112222.rlib");
      const linkCreated = tryCreateFileLink(externalVitalFile, artifactLinked);
      if (linkCreated) {
        fs.utimesSync(artifactLinked, new Date(Date.now() - 200000), new Date(Date.now() - 200000));
      }

      const res = pruneDeps(depsDir, true, boundary);
      expect(res.count).toBe(1); // Only artifactOld is pruned
      expect(fs.existsSync(artifactOld)).toBe(false);
      expect(fs.existsSync(artifactNew)).toBe(true);

      if (linkCreated) {
        expect(fs.existsSync(externalVitalFile)).toBe(true);
        expect(fs.readFileSync(externalVitalFile, "utf8")).toBe("{\"vital\": true}");
      }
    });
  });

  describe("runPruneTarget full lifecycle defense", () => {
    it("in auto mode: skips cleanly when target is an external junction without deleting external files", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      fs.mkdirSync(srcTauri, { recursive: true });

      // Create external target with dummy incremental data
      const externalTarget = path.join(tempRoot, "external-target-location");
      const extDebug = path.join(externalTarget, "debug");
      const extInc = path.join(extDebug, "incremental");
      fs.mkdirSync(extInc, { recursive: true });

      const dummyFile = path.join(extInc, "external-session-data.bin");
      fs.writeFileSync(dummyFile, "critical external build artifacts");

      // Junction src-tauri/target -> externalTarget
      const targetLink = path.join(srcTauri, "target");
      createDirLink(externalTarget, targetLink);

      const result = runPruneTarget(["--auto"], tempRoot);
      expect(result.skipped).toBe(true);
      expect(result.reason).toMatch(/symbolic link or junction|canonical divergence/i);

      // The external files MUST be completely untouched!
      expect(fs.existsSync(dummyFile)).toBe(true);
      expect(fs.readFileSync(dummyFile, "utf8")).toBe("critical external build artifacts");
    });

    it("in manual mode: rejects target link and exits with error without touching external files", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      fs.mkdirSync(srcTauri, { recursive: true });

      const externalTarget = path.join(tempRoot, "external-target-manual");
      fs.mkdirSync(externalTarget, { recursive: true });
      const sensitiveFile = path.join(externalTarget, "sensitive.txt");
      fs.writeFileSync(sensitiveFile, "do not touch");

      const targetLink = path.join(srcTauri, "target");
      createDirLink(externalTarget, targetLink);

      const prevExitCode = process.exitCode;
      try {
        const result = runPruneTarget([], tempRoot);
        expect(result.skipped).toBe(true);
        expect(process.exitCode).toBe(1);
        expect(fs.existsSync(sensitiveFile)).toBe(true);
      } finally {
        process.exitCode = prevExitCode;
      }
    });
  });

  describe("Candidate session lock checking & active-build detection", () => {
    it("isLockFileActive distinguishes idle 0-byte locks from actively held locks", () => {
      const lockFile = path.join(tempRoot, "s-idle.lock");
      fs.writeFileSync(lockFile, "");

      // Idle file should return false (unlocked)
      expect(isLockFileActive(lockFile, tempRoot)).toBe(false);

      // When openSync throws EBUSY / EPERM (simulating exclusive lock from rustc)
      const origOpen = fs.openSync.bind(fs);
      const openSpy = vi.spyOn(fs, "openSync").mockImplementation((p, flags, mode) => {
        if (typeof p === "string" && p.includes("s-idle.lock")) {
          const err = new Error("resource busy or locked");
          err.code = "EBUSY";
          throw err;
        }
        return origOpen(p, flags, mode);
      });

      expect(isLockFileActive(lockFile, tempRoot)).toBe(true);
      openSpy.mockRestore();
    });

    it("isSessionDirectoryLocked detects active s-*.lock inside crate directories", () => {
      const debugDir = path.join(tempRoot, "debug");
      const crateDir = path.join(debugDir, "incremental", "tauri_app-18r2ewd68ake1");
      fs.mkdirSync(crateDir, { recursive: true });

      const lockFile = path.join(crateDir, "s-hm4irhwvbk-0uwx0fv.lock");
      fs.writeFileSync(lockFile, "");

      // When idle
      expect(isSessionDirectoryLocked(crateDir, debugDir)).toBe(false);

      // When lock file is actively locked
      const origOpen = fs.openSync.bind(fs);
      const openSpy = vi.spyOn(fs, "openSync").mockImplementation((p, flags, mode) => {
        if (typeof p === "string" && p.includes("s-hm4irhwvbk-0uwx0fv.lock")) {
          const err = new Error("operation not permitted");
          err.code = "EPERM";
          throw err;
        }
        return origOpen(p, flags, mode);
      });

      expect(isSessionDirectoryLocked(crateDir, debugDir)).toBe(true);
      openSpy.mockRestore();
    });

    it("pruneIncremental in auto mode yields when candidate session has an active lock", () => {
      const debugDir = path.join(tempRoot, "debug");
      const incDir = path.join(debugDir, "incremental");
      const crateDir1 = path.join(incDir, "kokoro_app-1111111111111111");
      const crateDir2 = path.join(incDir, "kokoro_app-2222222222222222");
      fs.mkdirSync(crateDir1, { recursive: true });
      fs.mkdirSync(crateDir2, { recursive: true });

      fs.writeFileSync(path.join(crateDir1, "s-session1.lock"), "");
      fs.writeFileSync(path.join(crateDir2, "s-session2.lock"), "");

      // Simulate lock on crateDir2
      const origOpen = fs.openSync.bind(fs);
      const openSpy = vi.spyOn(fs, "openSync").mockImplementation((p, flags, mode) => {
        if (typeof p === "string" && p.includes("s-session2.lock")) {
          const err = new Error("busy");
          err.code = "EBUSY";
          throw err;
        }
        return origOpen(p, flags, mode);
      });

      const res = pruneIncremental(incDir, 1, debugDir, false, false, true, false);
      expect(res.hasActiveLock).toBe(true);
      expect(res.count).toBe(0);
      expect(res.bytesFreed).toBe(0);

      // Both session directories must remain completely intact
      expect(fs.existsSync(crateDir1)).toBe(true);
      expect(fs.existsSync(crateDir2)).toBe(true);

      openSpy.mockRestore();
    });
  });

  describe("Branch switch marker lifecycle & safe consumption", () => {
    it("preserves .branch-switched when active locks prevent safe cleanup", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const incDir = path.join(debugDir, "incremental");
      const crateDir = path.join(incDir, "kokoro_lib-1234567890123456");

      fs.mkdirSync(crateDir, { recursive: true });
      fs.writeFileSync(path.join(crateDir, "s-active.lock"), "");

      const branchMarker = path.join(targetDir, ".branch-switched");
      fs.writeFileSync(branchMarker, Date.now().toString());

      const cooldownFile = path.join(targetDir, ".prune-cooldown");

      // Simulate active compiler lock on s-active.lock
      const origOpen = fs.openSync.bind(fs);
      const openSpy = vi.spyOn(fs, "openSync").mockImplementation((p, flags, mode) => {
        if (typeof p === "string" && p.endsWith(".lock")) {
          const err = new Error("locked");
          err.code = "EBUSY";
          throw err;
        }
        return origOpen(p, flags, mode);
      });

      const result = runPruneTarget(["--auto"], tempRoot);

      // Prune should have yielded on lock
      expect(result.count).toBe(0);

      // CRITICAL: Branch marker must NOT be consumed!
      expect(fs.existsSync(branchMarker)).toBe(true);

      // Full cooldown must NOT be recorded so next run retries
      expect(fs.existsSync(cooldownFile)).toBe(false);

      openSpy.mockRestore();
    });

    it("consumes .branch-switched and updates cooldown when cleanup completes safely", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const incDir = path.join(debugDir, "incremental");

      // Create two sessions (one newer, one older)
      const crateOld = path.join(incDir, "kokoro_lib-1111111111111111");
      const crateNew = path.join(incDir, "kokoro_lib-2222222222222222");
      fs.mkdirSync(crateOld, { recursive: true });
      fs.mkdirSync(crateNew, { recursive: true });

      fs.writeFileSync(path.join(crateOld, "dummy.bin"), "data1");
      fs.writeFileSync(path.join(crateNew, "dummy.bin"), "data2");
      fs.utimesSync(crateOld, new Date(Date.now() - 100000), new Date(Date.now() - 100000));

      const branchMarker = path.join(targetDir, ".branch-switched");
      fs.writeFileSync(branchMarker, Date.now().toString());

      const cooldownFile = path.join(targetDir, ".prune-cooldown");

      const result = runPruneTarget(["--auto"], tempRoot);

      // Older session was pruned
      expect(result.count).toBe(1);
      expect(fs.existsSync(crateOld)).toBe(false);
      expect(fs.existsSync(crateNew)).toBe(true);

      // Marker is safely consumed upon clean completion
      expect(fs.existsSync(branchMarker)).toBe(false);
      expect(fs.existsSync(cooldownFile)).toBe(true);
    });

    it("consumes stale .branch-switched if timestamp exceeds 24 hours TTL", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      fs.mkdirSync(debugDir, { recursive: true });

      const branchMarker = path.join(targetDir, ".branch-switched");
      // Timestamp from 30 hours ago (seconds format as written by post-checkout)
      const staleTimestampSec = Math.floor((Date.now() - 30 * 60 * 60 * 1000) / 1000);
      fs.writeFileSync(branchMarker, staleTimestampSec.toString());

      runPruneTarget(["--auto"], tempRoot);

      // Expired marker was cleaned up
      expect(fs.existsSync(branchMarker)).toBe(false);
    });

    it("ensures --dry-run is strictly read-only: does not delete marker, does not write cooldown, does not delete files", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const incDir = path.join(debugDir, "incremental");

      const crateOld = path.join(incDir, "kokoro_lib-1111111111111111");
      const crateNew = path.join(incDir, "kokoro_lib-2222222222222222");
      fs.mkdirSync(crateOld, { recursive: true });
      fs.mkdirSync(crateNew, { recursive: true });

      fs.writeFileSync(path.join(crateOld, "dummy.bin"), "data1");
      fs.writeFileSync(path.join(crateNew, "dummy.bin"), "data2");
      fs.utimesSync(crateOld, new Date(Date.now() - 100000), new Date(Date.now() - 100000));

      const branchMarker = path.join(targetDir, ".branch-switched");
      fs.writeFileSync(branchMarker, Date.now().toString());

      const cooldownFile = path.join(targetDir, ".prune-cooldown");

      const result = runPruneTarget(["--dry-run"], tempRoot);

      // Dry run indicates what would be freed
      expect(result.count).toBe(1);

      // CRITICAL: ZERO filesystem mutations
      expect(fs.existsSync(crateOld)).toBe(true);
      expect(fs.existsSync(crateNew)).toBe(true);
      expect(fs.existsSync(branchMarker)).toBe(true);
      expect(fs.existsSync(cooldownFile)).toBe(false);
    });

    it("ensures --dry-run preserves stale .branch-switched even when exceeding 24 hours TTL", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      fs.mkdirSync(debugDir, { recursive: true });

      const branchMarker = path.join(targetDir, ".branch-switched");
      const staleTimestampSec = Math.floor((Date.now() - 30 * 60 * 60 * 1000) / 1000);
      fs.writeFileSync(branchMarker, staleTimestampSec.toString());

      const cooldownFile = path.join(targetDir, ".prune-cooldown");

      runPruneTarget(["--dry-run"], tempRoot);

      // In dry-run, stale marker must NOT be unlinked
      expect(fs.existsSync(branchMarker)).toBe(true);
      expect(fs.existsSync(cooldownFile)).toBe(false);
    });

    it("ensures --auto --dry-run below threshold does not write .prune-cooldown", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      fs.mkdirSync(debugDir, { recursive: true });

      const cooldownFile = path.join(targetDir, ".prune-cooldown");

      const res = runPruneTarget(["--auto", "--dry-run"], tempRoot);
      expect(res.skipped).toBe(true);
      expect(res.reason).toBe("below_threshold");

      // Must not mutate cooldown file in dry run
      expect(fs.existsSync(cooldownFile)).toBe(false);
    });

    it("in auto mode: handles CI environment gating correctly", () => {
      // 1. When running on custom test root and CI=true, does not skip by default
      process.env.CI = "true";
      const resNormalTest = runPruneTarget(["--auto"], tempRoot);
      expect(resNormalTest.reason).not.toBe("ci_environment");

      // 2. When KOKORO_FORCE_CI_CHECK=1 is set, auto mode yields with ci_environment even on customRoot
      process.env.KOKORO_FORCE_CI_CHECK = "1";
      const resForced = runPruneTarget(["--auto"], tempRoot);
      expect(resForced.skipped).toBe(true);
      expect(resForced.reason).toBe("ci_environment");

      // 3. When customRoot is process.cwd() and CI=true, auto mode yields with ci_environment
      delete process.env.KOKORO_FORCE_CI_CHECK;
      const resCwd = runPruneTarget(["--auto"], process.cwd());
      expect(resCwd.skipped).toBe(true);
      expect(resCwd.reason).toBe("ci_environment");
    });

    it("ensures updateCooldown, writeCooldown, and consumeBranchSwitchMarker respect isDryRun guard", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const cooldownFile = path.join(targetDir, ".prune-cooldown");
      const branchMarker = path.join(targetDir, ".branch-switched");

      fs.mkdirSync(targetDir, { recursive: true });
      fs.writeFileSync(branchMarker, Date.now().toString());

      const writeRes = writeCooldown(targetDir, cooldownFile, tempRoot, true);
      expect(writeRes).toBe(false);
      expect(fs.existsSync(cooldownFile)).toBe(false);

      const consumeRes = consumeBranchSwitchMarker(targetDir, branchMarker, tempRoot, true);
      expect(consumeRes).toBe(false);
      expect(fs.existsSync(branchMarker)).toBe(true);

      const updateRes = updateCooldown(targetDir, cooldownFile, branchMarker, true, tempRoot, true);
      expect(updateRes).toBe(false);
      expect(fs.existsSync(cooldownFile)).toBe(false);
      expect(fs.existsSync(branchMarker)).toBe(true);
    });

    it("preserves .branch-switched when deps pruning is locked / incomplete", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const depsDir = path.join(debugDir, "deps");

      fs.mkdirSync(depsDir, { recursive: true });

      const file1 = path.join(depsDir, "kokoro_lib-1234567890abcdef.rlib");
      const file2 = path.join(depsDir, "kokoro_lib-fedcba0987654321.rlib");
      fs.writeFileSync(file1, "data1");
      fs.writeFileSync(file2, "data2");
      fs.utimesSync(file1, new Date(Date.now() - 100000), new Date(Date.now() - 100000));

      const branchMarker = path.join(targetDir, ".branch-switched");
      fs.writeFileSync(branchMarker, Date.now().toString());
      const cooldownFile = path.join(targetDir, ".prune-cooldown");

      // Mock unlinkSync to fail on the candidate item (simulating locked file / EBUSY)
      const origUnlink = fs.unlinkSync;
      const unlinkSpy = vi.spyOn(fs, "unlinkSync").mockImplementation((p) => {
        if (typeof p === "string" && p.includes("kokoro_lib")) {
          const err = new Error("locked");
          err.code = "EBUSY";
          throw err;
        }
        return origUnlink(p);
      });

      const result = runPruneTarget(["--auto"], tempRoot);

      // Marker must be preserved because cleanup was incomplete
      expect(fs.existsSync(branchMarker)).toBe(true);
      expect(fs.existsSync(cooldownFile)).toBe(false);

      unlinkSpy.mockRestore();
    });
  });

  describe("Git hook format & POSIX executable permissions", () => {
    const hooksDir = path.resolve(process.cwd(), ".githooks");

    it("verifies hook scripts exist, use LF line endings, and have POSIX shebang", () => {
      const hooks = ["post-checkout", "post-merge"];
      for (const hookName of hooks) {
        const hookPath = path.join(hooksDir, hookName);
        expect(fs.existsSync(hookPath)).toBe(true);

        const buffer = fs.readFileSync(hookPath);
        // Ensure no Windows CRLF (\r\n)
        expect(buffer.includes(Buffer.from("\r\n"))).toBe(false);

        const content = buffer.toString("utf8");
        expect(content.startsWith("#!/bin/sh\n")).toBe(true);
      }
    });

    it("ensures --setup-hooks enforces chmod 0o755 on hook scripts when on POSIX", () => {
      const fakeGitDir = path.join(tempRoot, ".git");
      const fakeHooksDir = path.join(tempRoot, ".githooks");
      fs.mkdirSync(fakeGitDir, { recursive: true });
      fs.mkdirSync(fakeHooksDir, { recursive: true });

      const checkoutHook = path.join(fakeHooksDir, "post-checkout");
      const mergeHook = path.join(fakeHooksDir, "post-merge");
      fs.writeFileSync(checkoutHook, "#!/bin/sh\nexit 0\n");
      fs.writeFileSync(mergeHook, "#!/bin/sh\nexit 0\n");

      const chmodSpy = vi.spyOn(fs, "chmodSync").mockImplementation(() => {});

      const originalPlatform = process.platform;
      Object.defineProperty(process, "platform", { value: "linux", configurable: true });

      try {
        const result = runPruneTarget(["--setup-hooks"], tempRoot);
        expect(result.success).toBe(true);
        expect(chmodSpy).toHaveBeenCalledWith(checkoutHook, 0o755);
        expect(chmodSpy).toHaveBeenCalledWith(mergeHook, 0o755);
      } finally {
        Object.defineProperty(process, "platform", { value: originalPlatform, configurable: true });
        chmodSpy.mockRestore();
      }
    });

    it("isPosixShellScript correctly identifies POSIX shell scripts and rejects non-shell scripts", () => {
      expect(isPosixShellScript("#!/bin/sh\necho test")).toBe(true);
      expect(isPosixShellScript("#!/bin/bash\necho test")).toBe(true);
      expect(isPosixShellScript("#!/usr/bin/env sh\necho test")).toBe(true);
      expect(isPosixShellScript("#!/usr/bin/env bash\necho test")).toBe(true);
      expect(isPosixShellScript("#!/bin/sh -e\necho test")).toBe(true);

      expect(isPosixShellScript("#!/usr/bin/env node\nconsole.log('hi')")).toBe(false);
      expect(isPosixShellScript("#!/usr/bin/env python3\nprint('hi')")).toBe(false);
      expect(isPosixShellScript("#!/usr/bin/python\nprint('hi')")).toBe(false);
      expect(isPosixShellScript("echo no shebang")).toBe(false);
      expect(isPosixShellScript("")).toBe(false);
      expect(isPosixShellScript(null)).toBe(false);
    });

    it("injectSentinelBlockIntoShellScript inserts Sentinel block after shebang before user body", () => {
      const userHook = "#!/bin/sh\n\n# User custom logic\nexit 0\n";
      const injected = injectSentinelBlockIntoShellScript(userHook, "post-checkout");

      expect(injected.startsWith("#!/bin/sh\n")).toBe(true);
      expect(injected.includes(SENTINEL_BLOCK_START)).toBe(true);
      expect(injected.includes(SENTINEL_BLOCK_END)).toBe(true);
      expect(injected.includes('if [ "$3" = "1" ]; then')).toBe(true);

      // Verify user logic appears AFTER the sentinel block
      const sentinelEndIdx = injected.indexOf(SENTINEL_BLOCK_END);
      const userLogicIdx = injected.indexOf("# User custom logic");
      expect(userLogicIdx).toBeGreaterThan(sentinelEndIdx);
    });

    it("getActiveHooks filters out .sample files and returns active hooks", () => {
      const hooksDir = path.join(tempRoot, "test-hooks");
      fs.mkdirSync(hooksDir, { recursive: true });

      fs.writeFileSync(path.join(hooksDir, "pre-commit.sample"), "#!/bin/sh\n");
      fs.writeFileSync(path.join(hooksDir, "post-update.sample"), "#!/bin/sh\n");
      fs.writeFileSync(path.join(hooksDir, "pre-commit"), "#!/bin/sh\n");
      fs.writeFileSync(path.join(hooksDir, "commit-msg"), "#!/bin/sh\n");

      const active = getActiveHooks(hooksDir);
      expect(active.sort()).toEqual(["commit-msg", "pre-commit"]);
    });

    it("--setup-hooks skips when KOKORO_SKIP_HOOKS=1", () => {
      const original = process.env.KOKORO_SKIP_HOOKS;
      process.env.KOKORO_SKIP_HOOKS = "1";
      try {
        const result = runPruneTarget(["--setup-hooks"], tempRoot);
        expect(result.skipped).toBe(true);
        expect(result.reason).toBe("kokoro_skip_hooks");
      } finally {
        if (original === undefined) {
          delete process.env.KOKORO_SKIP_HOOKS;
        } else {
          process.env.KOKORO_SKIP_HOOKS = original;
        }
      }
    });

    it("--setup-hooks skips when npm_config_kokoro_skip_hooks=1", () => {
      const original = process.env.npm_config_kokoro_skip_hooks;
      process.env.npm_config_kokoro_skip_hooks = "1";
      try {
        const result = runPruneTarget(["--setup-hooks", "--safe"], tempRoot);
        expect(result.skipped).toBe(true);
        expect(result.reason).toBe("kokoro_skip_hooks");
      } finally {
        if (original === undefined) {
          delete process.env.npm_config_kokoro_skip_hooks;
        } else {
          process.env.npm_config_kokoro_skip_hooks = original;
        }
      }
    });

    it("--setup-hooks --safe runs in CI environments unless KOKORO_SKIP_HOOKS=1", () => {
      const originalCI = process.env.CI;
      process.env.CI = "true";
      const fakeGitDir = path.join(tempRoot, ".git");
      const fakeGithooks = path.join(tempRoot, ".githooks");
      fs.mkdirSync(fakeGitDir, { recursive: true });
      fs.mkdirSync(fakeGithooks, { recursive: true });
      fs.writeFileSync(path.join(fakeGithooks, "post-checkout"), "#!/bin/sh\nexit 0\n");

      let configured = false;
      setGitExecutorForTesting((cmd) => {
        if (typeof cmd === "string" && cmd.includes("git config core.hooksPath .githooks")) {
          configured = true;
          return "";
        }
        return "";
      });

      try {
        const result = runPruneTarget(["--setup-hooks", "--safe"], tempRoot);
        expect(result.success).toBe(true);
        expect(configured).toBe(true);
      } finally {
        setGitExecutorForTesting(null);
        if (originalCI === undefined) {
          delete process.env.CI;
        } else {
          process.env.CI = originalCI;
        }
      }
    });

    it("--setup-hooks preserves existing custom core.hooksPath and refuses to overwrite without --force", () => {
      const fakeGitDir = path.join(tempRoot, ".git");
      fs.mkdirSync(fakeGitDir, { recursive: true });

      setGitExecutorForTesting((cmd) => {
        if (typeof cmd === "string" && cmd.includes("git config --get core.hooksPath")) {
          return ".husky\n";
        }
        if (typeof cmd === "string" && cmd.includes("git rev-parse --git-path hooks")) {
          return ".git/hooks\n";
        }
        return "";
      });

      try {
        // Without --force: preserves custom hooks
        const result = runPruneTarget(["--setup-hooks"], tempRoot);
        expect(result.success).toBe(true);
        expect(result.status).toBe("preserved_custom_hooks");
        expect(result.hooksPath).toBe(".husky");

        // With --force: allows explicit override
        const forceResult = runPruneTarget(["--setup-hooks", "--force"], tempRoot);
        expect(forceResult.success).toBe(true);
        expect(forceResult.status).toBe("forced_githooks");
      } finally {
        setGitExecutorForTesting(null);
      }
    });

    it("--setup-hooks safely chains into .git/hooks when active hooks exist without modifying core.hooksPath", () => {
      const fakeGitDir = path.join(tempRoot, ".git");
      const fakeHooksDir = path.join(fakeGitDir, "hooks");
      fs.mkdirSync(fakeHooksDir, { recursive: true });

      // User has existing pre-commit and post-checkout
      fs.writeFileSync(path.join(fakeHooksDir, "pre-commit"), "#!/bin/sh\necho 'running lint'\nexit 0\n");
      fs.writeFileSync(path.join(fakeHooksDir, "post-checkout"), "#!/bin/sh\n# user checkout hook\nexit 0\n");

      setGitExecutorForTesting((cmd) => {
        if (typeof cmd === "string" && cmd.includes("git config --get core.hooksPath")) {
          throw new Error("not set");
        }
        if (typeof cmd === "string" && cmd.includes("git rev-parse --git-path hooks")) {
          return path.join(tempRoot, ".git", "hooks") + "\n";
        }
        return "";
      });

      try {
        const result = runPruneTarget(["--setup-hooks"], tempRoot);
        expect(result.success).toBe(true);
        expect(result.status).toBe("chained_git_hooks");

        // Verify pre-commit was NOT touched
        const preCommitContent = fs.readFileSync(path.join(fakeHooksDir, "pre-commit"), "utf8");
        expect(preCommitContent).toBe("#!/bin/sh\necho 'running lint'\nexit 0\n");

        // Verify post-checkout had sentinel injected
        const postCheckoutContent = fs.readFileSync(path.join(fakeHooksDir, "post-checkout"), "utf8");
        expect(postCheckoutContent.includes(SENTINEL_BLOCK_START)).toBe(true);
        expect(postCheckoutContent.includes("# user checkout hook")).toBe(true);

        // Verify idempotency: running a second time does not duplicate block
        const secondResult = runPruneTarget(["--setup-hooks"], tempRoot);
        expect(secondResult.success).toBe(true);
        const secondContent = fs.readFileSync(path.join(fakeHooksDir, "post-checkout"), "utf8");
        const occurrences = secondContent.split(SENTINEL_BLOCK_START).length - 1;
        expect(occurrences).toBe(1);
      } finally {
        setGitExecutorForTesting(null);
      }
    });

    it("--setup-hooks does not inject shell syntax into non-shell existing hook (e.g. Python)", () => {
      const fakeGitDir = path.join(tempRoot, ".git");
      const fakeHooksDir = path.join(fakeGitDir, "hooks");
      fs.mkdirSync(fakeHooksDir, { recursive: true });

      // User has existing Python post-checkout hook
      const pythonHook = "#!/usr/bin/env python3\nimport sys\nprint('python hook')\nsys.exit(0)\n";
      fs.writeFileSync(path.join(fakeHooksDir, "post-checkout"), pythonHook);
      fs.writeFileSync(path.join(fakeHooksDir, "pre-commit"), "#!/bin/sh\nexit 0\n");

      setGitExecutorForTesting((cmd) => {
        if (typeof cmd === "string" && cmd.includes("git config --get core.hooksPath")) {
          throw new Error("not set");
        }
        if (typeof cmd === "string" && cmd.includes("git rev-parse --git-path hooks")) {
          return path.join(tempRoot, ".git", "hooks") + "\n";
        }
        return "";
      });

      try {
        const result = runPruneTarget(["--setup-hooks"], tempRoot);
        expect(result.success).toBe(true);
        expect(result.status).toBe("chained_git_hooks");

        // Verify python hook was completely preserved and not corrupted
        const postCheckoutContent = fs.readFileSync(path.join(fakeHooksDir, "post-checkout"), "utf8");
        expect(postCheckoutContent).toBe(pythonHook);
        expect(postCheckoutContent.includes(SENTINEL_BLOCK_START)).toBe(false);
      } finally {
        setGitExecutorForTesting(null);
      }
    });
  });

  describe("Target pruner deps/ test harness matching and retention rules", () => {
    it("isEligibleDepsArtifact identifies Kokoro workspace crates across all extensions", () => {
      expect(isEligibleDepsArtifact("tauri_appkokoro_engine", ".exe")).toBe(true);
      expect(isEligibleDepsArtifact("tauri_appkokoro_engine", ".pdb")).toBe(true);
      expect(isEligibleDepsArtifact("tauri_appkokoro_engine", ".d")).toBe(true);
      expect(isEligibleDepsArtifact("libtauri_appkokoro_engine_lib", ".rlib")).toBe(true);
      expect(isEligibleDepsArtifact("tauri_appkokoro_engine_lib", ".exe")).toBe(true);
      expect(isEligibleDepsArtifact("tauri_appkokoro_engine_lib", ".pdb")).toBe(true);
    });

    it("isEligibleDepsArtifact identifies documented integration test harnesses (characters, vision)", () => {
      expect(isEligibleDepsArtifact("characters", ".exe")).toBe(true);
      expect(isEligibleDepsArtifact("characters", ".pdb")).toBe(true);
      expect(isEligibleDepsArtifact("characters", ".d")).toBe(true);
      expect(isEligibleDepsArtifact("vision", ".exe")).toBe(true);
      expect(isEligibleDepsArtifact("vision", ".pdb")).toBe(true);
      expect(isEligibleDepsArtifact("vision", ".d")).toBe(true);
    });

    it("isEligibleDepsArtifact identifies executable targets dynamically discovered in deps", () => {
      const exeBaseNames = new Set(["custom_suite", "integration_runner"]);
      expect(isEligibleDepsArtifact("custom_suite", ".exe", exeBaseNames)).toBe(true);
      expect(isEligibleDepsArtifact("custom_suite", ".pdb", exeBaseNames)).toBe(true);
      expect(isEligibleDepsArtifact("integration_runner", ".exe", exeBaseNames)).toBe(true);

      // Unknown crate without executable in deps is rejected
      expect(isEligibleDepsArtifact("unknown_crate", ".pdb", exeBaseNames)).toBe(false);
    });

    it("isEligibleDepsArtifact strictly rejects third-party dependencies and proc-macros", () => {
      // Third-party .rlib
      expect(isEligibleDepsArtifact("libserde", ".rlib")).toBe(false);
      expect(isEligibleDepsArtifact("syn", ".rlib")).toBe(false);
      expect(isEligibleDepsArtifact("tokio", ".rlib")).toBe(false);

      // Even if third-party crate shares name with known module (e.g. vision), .rlib is never eligible
      expect(isEligibleDepsArtifact("libvision", ".rlib")).toBe(false);
      expect(isEligibleDepsArtifact("vision", ".rlib")).toBe(false);

      // Third-party proc-macros (.dll, .pdb)
      expect(isEligibleDepsArtifact("serde_derive", ".dll")).toBe(false);
      expect(isEligibleDepsArtifact("serde_derive", ".pdb")).toBe(false);
      expect(isEligibleDepsArtifact("tokio_macros", ".pdb")).toBe(false);
    });

    it("pruneDeps groups and prunes stale characters and vision test harnesses while keeping latest", () => {
      const debugDir = path.join(tempRoot, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      const now = Date.now();
      const past35m = new Date(now - 35 * 60 * 1000);
      const past10m = new Date(now - 10 * 60 * 1000);

      // Create 2 generations of characters test harness (.exe and .pdb)
      const charExeOld = path.join(depsDir, "characters-1111111111111111.exe");
      const charPdbOld = path.join(depsDir, "characters-1111111111111111.pdb");
      const charExeNew = path.join(depsDir, "characters-2222222222222222.exe");
      const charPdbNew = path.join(depsDir, "characters-2222222222222222.pdb");

      fs.writeFileSync(charExeOld, "old-exe");
      fs.writeFileSync(charPdbOld, "old-pdb");
      fs.writeFileSync(charExeNew, "new-exe");
      fs.writeFileSync(charPdbNew, "new-pdb");

      fs.utimesSync(charExeOld, past35m, past35m);
      fs.utimesSync(charPdbOld, past35m, past35m);
      fs.utimesSync(charExeNew, past10m, past10m);
      fs.utimesSync(charPdbNew, past10m, past10m);

      // Create 2 generations of vision test harness
      const visionExeOld = path.join(depsDir, "vision-1111111111111111.exe");
      const visionPdbOld = path.join(depsDir, "vision-1111111111111111.pdb");
      const visionExeNew = path.join(depsDir, "vision-2222222222222222.exe");
      const visionPdbNew = path.join(depsDir, "vision-2222222222222222.pdb");

      fs.writeFileSync(visionExeOld, "old-vision-exe");
      fs.writeFileSync(visionPdbOld, "old-vision-pdb");
      fs.writeFileSync(visionExeNew, "new-vision-exe");
      fs.writeFileSync(visionPdbNew, "new-vision-pdb");

      fs.utimesSync(visionExeOld, past35m, past35m);
      fs.utimesSync(visionPdbOld, past35m, past35m);
      fs.utimesSync(visionExeNew, past10m, past10m);
      fs.utimesSync(visionPdbNew, past10m, past10m);

      // Third-party files (must remain untouched!)
      const serdeRlibOld = path.join(depsDir, "libserde-1111111111111111.rlib");
      const serdeRlibNew = path.join(depsDir, "libserde-2222222222222222.rlib");
      const serdePdbOld = path.join(depsDir, "serde_derive-1111111111111111.pdb");
      const serdePdbNew = path.join(depsDir, "serde_derive-2222222222222222.pdb");
      fs.writeFileSync(serdeRlibOld, "serde-old");
      fs.writeFileSync(serdeRlibNew, "serde-new");
      fs.writeFileSync(serdePdbOld, "macro-old");
      fs.writeFileSync(serdePdbNew, "macro-new");
      fs.utimesSync(serdeRlibOld, past35m, past35m);
      fs.utimesSync(serdePdbOld, past35m, past35m);

      const res = pruneDeps(depsDir, false, debugDir, false, false, false);

      // 4 files pruned: charExeOld, charPdbOld, visionExeOld, visionPdbOld
      expect(res.count).toBe(4);
      expect(fs.existsSync(charExeOld)).toBe(false);
      expect(fs.existsSync(charPdbOld)).toBe(false);
      expect(fs.existsSync(charExeNew)).toBe(true);
      expect(fs.existsSync(charPdbNew)).toBe(true);

      expect(fs.existsSync(visionExeOld)).toBe(false);
      expect(fs.existsSync(visionPdbOld)).toBe(false);
      expect(fs.existsSync(visionExeNew)).toBe(true);
      expect(fs.existsSync(visionPdbNew)).toBe(true);

      // Third party dependencies are 100% untouched
      expect(fs.existsSync(serdeRlibOld)).toBe(true);
      expect(fs.existsSync(serdeRlibNew)).toBe(true);
      expect(fs.existsSync(serdePdbOld)).toBe(true);
      expect(fs.existsSync(serdePdbNew)).toBe(true);
    });

    it("pruneDeps protects recent (<30m) test harness items under active testing shield", () => {
      const debugDir = path.join(tempRoot, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      const now = Date.now();
      const past10m = new Date(now - 10 * 60 * 1000);
      const past5m = new Date(now - 5 * 60 * 1000);

      // Both generations are within 30 minutes
      const charExe1 = path.join(depsDir, "characters-1111111111111111.exe");
      const charExe2 = path.join(depsDir, "characters-2222222222222222.exe");
      fs.writeFileSync(charExe1, "char1");
      fs.writeFileSync(charExe2, "char2");
      fs.utimesSync(charExe1, past10m, past10m);
      fs.utimesSync(charExe2, past5m, past5m);

      const res = pruneDeps(depsDir, false, debugDir, false, false, false);

      // Protected by 30-minute shield: 0 files pruned
      expect(res.count).toBe(0);
      expect(fs.existsSync(charExe1)).toBe(true);
      expect(fs.existsSync(charExe2)).toBe(true);
    });

    it("pruneDeps treats tauri_appkokoro_engine_lib-*.exe as test harness (keep 1) and tauri_appkokoro_engine-*.exe as main app (keep 2)", () => {
      const debugDir = path.join(tempRoot, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      const now = Date.now();
      const past40m = new Date(now - 40 * 60 * 1000);
      const past20m = new Date(now - 20 * 60 * 1000);
      const past1m = new Date(now - 1 * 60 * 1000);

      // Main app: 3 generations. Normal mode keeps latest 2, prunes oldest (past40m)
      const main1 = path.join(depsDir, "tauri_appkokoro_engine-1111111111111111.exe");
      const main2 = path.join(depsDir, "tauri_appkokoro_engine-2222222222222222.exe");
      const main3 = path.join(depsDir, "tauri_appkokoro_engine-3333333333333333.exe");
      fs.writeFileSync(main1, "main1");
      fs.writeFileSync(main2, "main2");
      fs.writeFileSync(main3, "main3");
      fs.utimesSync(main1, past40m, past40m);
      fs.utimesSync(main2, past20m, past20m);
      fs.utimesSync(main3, past1m, past1m);

      // Lib unit test harness: 2 generations. Keeps 1, prunes oldest (past40m)
      const libTest1 = path.join(depsDir, "tauri_appkokoro_engine_lib-1111111111111111.exe");
      const libTest2 = path.join(depsDir, "tauri_appkokoro_engine_lib-2222222222222222.exe");
      fs.writeFileSync(libTest1, "libtest1");
      fs.writeFileSync(libTest2, "libtest2");
      fs.utimesSync(libTest1, past40m, past40m);
      fs.utimesSync(libTest2, past1m, past1m);

      const res = pruneDeps(depsDir, false, debugDir, false, false, false);

      // Pruned main1 (oldest of 3) and libTest1 (oldest of 2)
      expect(res.count).toBe(2);
      expect(fs.existsSync(main1)).toBe(false);
      expect(fs.existsSync(main2)).toBe(true);
      expect(fs.existsSync(main3)).toBe(true);

      expect(fs.existsSync(libTest1)).toBe(false);
      expect(fs.existsSync(libTest2)).toBe(true);
    });

    it("pruneDeps in emergency or branch-switched mode bypasses 30m shield and tightens main app to 1", () => {
      const debugDir = path.join(tempRoot, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      const now = Date.now();
      const past5m = new Date(now - 5 * 60 * 1000);
      const past1m = new Date(now - 1 * 60 * 1000);

      // Recent test harness (<30 min)
      const charExe1 = path.join(depsDir, "characters-1111111111111111.exe");
      const charExe2 = path.join(depsDir, "characters-2222222222222222.exe");
      fs.writeFileSync(charExe1, "char1");
      fs.writeFileSync(charExe2, "char2");
      fs.utimesSync(charExe1, past5m, past5m);
      fs.utimesSync(charExe2, past1m, past1m);

      // Recent main app (2 generations)
      const main1 = path.join(depsDir, "tauri_appkokoro_engine-1111111111111111.exe");
      const main2 = path.join(depsDir, "tauri_appkokoro_engine-2222222222222222.exe");
      fs.writeFileSync(main1, "main1");
      fs.writeFileSync(main2, "main2");
      fs.utimesSync(main1, past5m, past5m);
      fs.utimesSync(main2, past1m, past1m);

      // Run with hasBranchSwitched = true
      const res = pruneDeps(depsDir, false, debugDir, false, false, true);

      // Both older items are pruned despite being <30 min
      expect(res.count).toBe(2);
      expect(fs.existsSync(charExe1)).toBe(false);
      expect(fs.existsSync(charExe2)).toBe(true);
      expect(fs.existsSync(main1)).toBe(false);
      expect(fs.existsSync(main2)).toBe(true);
    });
  });

  describe("Whole-target watermark coverage & safe deletion scoping", () => {
    it("triggers pruning when total target footprint exceeds threshold even if debug alone is below threshold", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const releaseDir = path.join(targetDir, "release");
      const sherpaDir = path.join(targetDir, "sherpa-onnx-prebuilt");
      const debugDir = path.join(targetDir, "debug");
      const incDir = path.join(debugDir, "incremental");

      fs.mkdirSync(releaseDir, { recursive: true });
      fs.mkdirSync(sherpaDir, { recursive: true });
      fs.mkdirSync(incDir, { recursive: true });

      // Non-debug contents: 40,000 bytes each
      const releaseBin = path.join(releaseDir, "kokoro.exe");
      const sherpaLib = path.join(sherpaDir, "libsherpa.dll");
      fs.writeFileSync(releaseBin, Buffer.alloc(40000, 1));
      fs.writeFileSync(sherpaLib, Buffer.alloc(40000, 2));

      // Debug contents: 3 sessions of 5,000 bytes each (15,000 bytes total)
      const sess1 = path.join(incDir, "kokoro_lib-1111111111111111");
      const sess2 = path.join(incDir, "kokoro_lib-2222222222222222");
      const sess3 = path.join(incDir, "kokoro_lib-3333333333333333");
      fs.mkdirSync(sess1, { recursive: true });
      fs.mkdirSync(sess2, { recursive: true });
      fs.mkdirSync(sess3, { recursive: true });
      fs.writeFileSync(path.join(sess1, "data.bin"), Buffer.alloc(5000, 3));
      fs.writeFileSync(path.join(sess2, "data.bin"), Buffer.alloc(5000, 4));
      fs.writeFileSync(path.join(sess3, "data.bin"), Buffer.alloc(5000, 5));

      const now = Date.now();
      fs.utimesSync(sess1, new Date(now - 300000), new Date(now - 300000));
      fs.utimesSync(sess2, new Date(now - 200000), new Date(now - 200000));
      fs.utimesSync(sess3, new Date(now - 100000), new Date(now - 100000));

      // Total target size = 40,000 + 40,000 + 15,000 = ~95,000 bytes (~0.000088 GB).
      // Debug size = 15,000 bytes (~0.000014 GB).
      // If we set threshold to ~0.00004 GB (approx 42,950 bytes):
      // Debug ALONE (15 KB) is BELOW threshold (42.9 KB).
      // Total target (95 KB) is ABOVE threshold (42.9 KB).
      const thresholdGb = (42950 / (1024 * 1024 * 1024)).toFixed(8);

      const res = runPruneTarget([`--threshold-gb=${thresholdGb}`], tempRoot);

      // Oldest session (sess1) was pruned because default keep for kokoro is 2
      expect(res.count).toBe(1);
      expect(res.bytesFreed).toBe(5000);
      expect(fs.existsSync(sess1)).toBe(false);
      expect(fs.existsSync(sess2)).toBe(true);
      expect(fs.existsSync(sess3)).toBe(true);

      // CRITICAL: Non-debug files MUST remain 100% intact!
      expect(fs.existsSync(releaseBin)).toBe(true);
      expect(fs.readFileSync(releaseBin).length).toBe(40000);
      expect(fs.existsSync(sherpaLib)).toBe(true);
      expect(fs.readFileSync(sherpaLib).length).toBe(40000);
    });

    it("strictly scopes deletion to debugDir: never deletes release or vendor directories", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const releaseDir = path.join(targetDir, "release");
      const packageDir = path.join(targetDir, "package");
      const sherpaDir = path.join(targetDir, "sherpa-onnx-prebuilt");
      const debugDir = path.join(targetDir, "debug");

      fs.mkdirSync(releaseDir, { recursive: true });
      fs.mkdirSync(packageDir, { recursive: true });
      fs.mkdirSync(sherpaDir, { recursive: true });
      fs.mkdirSync(debugDir, { recursive: true });

      const releaseExe = path.join(releaseDir, "app.exe");
      const packageMsi = path.join(packageDir, "setup.msi");
      const sherpaDll = path.join(sherpaDir, "onnx.dll");

      fs.writeFileSync(releaseExe, "release-executable-critical");
      fs.writeFileSync(packageMsi, "installer-msi-critical");
      fs.writeFileSync(sherpaDll, "vendor-onnx-dll");

      // Execute manual prune
      const res = runPruneTarget([], tempRoot);

      expect(res.count).toBe(0);
      expect(fs.existsSync(releaseExe)).toBe(true);
      expect(fs.readFileSync(releaseExe, "utf8")).toBe("release-executable-critical");
      expect(fs.existsSync(packageMsi)).toBe(true);
      expect(fs.readFileSync(packageMsi, "utf8")).toBe("installer-msi-critical");
      expect(fs.existsSync(sherpaDll)).toBe(true);
      expect(fs.readFileSync(sherpaDll, "utf8")).toBe("vendor-onnx-dll");
    });

    it("safely handles missing debugDir when targetDir is above threshold", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const releaseDir = path.join(targetDir, "release");
      fs.mkdirSync(releaseDir, { recursive: true });

      const releaseExe = path.join(releaseDir, "app.exe");
      fs.writeFileSync(releaseExe, Buffer.alloc(50000, 1));

      const cooldownFile = path.join(targetDir, ".prune-cooldown");
      const thresholdGb = (1000 / (1024 * 1024 * 1024)).toFixed(8);

      // In auto mode: skips with no_debug_cache and writes cooldown
      const autoRes = runPruneTarget(["--auto", `--threshold-gb=${thresholdGb}`], tempRoot);
      expect(autoRes.skipped).toBe(true);
      expect(autoRes.reason).toBe("no_debug_cache");
      expect(fs.existsSync(cooldownFile)).toBe(true);

      // In manual mode: returns count 0, doesn't throw
      const manualRes = runPruneTarget([`--threshold-gb=${thresholdGb}`], tempRoot);
      expect(manualRes.count).toBe(0);
      expect(manualRes.bytesFreed).toBe(0);
    });

    it("skips with below_threshold when total target footprint is under threshold", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      fs.mkdirSync(debugDir, { recursive: true });

      const dummyFile = path.join(debugDir, "dummy.txt");
      fs.writeFileSync(dummyFile, "small");

      const cooldownFile = path.join(targetDir, ".prune-cooldown");
      // Set high threshold (10 GB)
      const res = runPruneTarget(["--auto", "--threshold-gb=10"], tempRoot);

      expect(res.skipped).toBe(true);
      expect(res.reason).toBe("below_threshold");
      expect(fs.existsSync(cooldownFile)).toBe(true);
    });
  });

  // =============================================================================
  // TARGETED FIXTURE-BASED TEST SUITES FOR DESTRUCTIVE LOGIC
  // =============================================================================

  // Helper: Computes a deterministic cryptographic snapshot of an entire directory tree
  function computeTreeSnapshot(baseDir) {
    if (!fs.existsSync(baseDir)) return { exists: false };
    const entries = [];

    function walk(currentDir) {
      const dirEntries = fs.readdirSync(currentDir, { withFileTypes: true });
      for (const entry of dirEntries) {
        const fullPath = path.join(currentDir, entry.name);
        const relPath = path.relative(baseDir, fullPath).replace(/\\/g, "/");
        const isSymlink = entry.isSymbolicLink();

        if (isSymlink) {
          entries.push({ relPath, type: "symlink" });
          continue;
        }

        if (entry.isDirectory()) {
          entries.push({ relPath, type: "dir" });
          walk(fullPath);
        } else if (entry.isFile()) {
          const content = fs.readFileSync(fullPath);
          const hash = crypto.createHash("sha256").update(content).digest("hex");
          const stat = fs.statSync(fullPath);
          entries.push({ relPath, type: "file", size: stat.size, hash });
        }
      }
    }

    walk(baseDir);
    entries.sort((a, b) => a.relPath.localeCompare(b.relPath));
    return { exists: true, entries };
  }

  // Helper: Creates a comprehensive target fixture with incremental, deps, release, markers
  function createComplexTargetFixture(root) {
    const srcTauri = path.join(root, "src-tauri");
    const targetDir = path.join(srcTauri, "target");
    const debugDir = path.join(targetDir, "debug");
    const incDir = path.join(debugDir, "incremental");
    const depsDir = path.join(debugDir, "deps");
    const releaseDir = path.join(targetDir, "release");

    fs.mkdirSync(incDir, { recursive: true });
    fs.mkdirSync(depsDir, { recursive: true });
    fs.mkdirSync(releaseDir, { recursive: true });

    const now = Date.now();
    const t40m = new Date(now - 40 * 60 * 1000);
    const t20m = new Date(now - 20 * 60 * 1000);
    const t5m = new Date(now - 5 * 60 * 1000);

    // Incremental sessions (3 generations of kokoro_lib)
    const inc1 = path.join(incDir, "kokoro_lib-1111111111111111");
    const inc2 = path.join(incDir, "kokoro_lib-2222222222222222");
    const inc3 = path.join(incDir, "kokoro_lib-3333333333333333");
    fs.mkdirSync(inc1);
    fs.mkdirSync(inc2);
    fs.mkdirSync(inc3);
    fs.writeFileSync(path.join(inc1, "dep-graph.bin"), Buffer.alloc(1000, 1));
    fs.writeFileSync(path.join(inc2, "dep-graph.bin"), Buffer.alloc(1000, 2));
    fs.writeFileSync(path.join(inc3, "dep-graph.bin"), Buffer.alloc(1000, 3));
    fs.utimesSync(inc1, t40m, t40m);
    fs.utimesSync(inc2, t20m, t20m);
    fs.utimesSync(inc3, t5m, t5m);

    // Deps artifacts
    // 3 generations of main app
    const main1 = path.join(depsDir, "tauri_appkokoro_engine-1111111111111111.exe");
    const main2 = path.join(depsDir, "tauri_appkokoro_engine-2222222222222222.exe");
    const main3 = path.join(depsDir, "tauri_appkokoro_engine-3333333333333333.exe");
    fs.writeFileSync(main1, Buffer.alloc(5000, 4));
    fs.writeFileSync(main2, Buffer.alloc(5000, 5));
    fs.writeFileSync(main3, Buffer.alloc(5000, 6));
    fs.utimesSync(main1, t40m, t40m);
    fs.utimesSync(main2, t20m, t20m);
    fs.utimesSync(main3, t5m, t5m);

    // 2 generations of characters test harness
    const char1 = path.join(depsDir, "characters-1111111111111111.exe");
    const char2 = path.join(depsDir, "characters-2222222222222222.exe");
    fs.writeFileSync(char1, Buffer.alloc(3000, 7));
    fs.writeFileSync(char2, Buffer.alloc(3000, 8));
    fs.utimesSync(char1, t40m, t40m);
    fs.utimesSync(char2, t5m, t5m);

    // Third-party crates (must never be touched)
    const serde = path.join(depsDir, "libserde-1111111111111111.rlib");
    const serdeMacro = path.join(depsDir, "serde_derive-1111111111111111.dll");
    fs.writeFileSync(serde, Buffer.alloc(8000, 9));
    fs.writeFileSync(serdeMacro, Buffer.alloc(8000, 10));
    fs.utimesSync(serde, t40m, t40m);
    fs.utimesSync(serdeMacro, t40m, t40m);

    // Release binary
    const relBin = path.join(releaseDir, "kokoro.exe");
    fs.writeFileSync(relBin, Buffer.alloc(20000, 11));

    // Markers
    const branchMarker = path.join(targetDir, ".branch-switched");
    const cooldownMarker = path.join(targetDir, ".prune-cooldown");
    fs.writeFileSync(branchMarker, Math.floor(now / 1000).toString());
    fs.writeFileSync(cooldownMarker, now.toString());

    return { targetDir, debugDir, incDir, depsDir, releaseDir, branchMarker, cooldownMarker };
  }

  describe("Targeted Fixture: Dry-Run State & Cryptographic Tree Immutability", () => {
    it("guarantees 100% byte-for-byte tree identity under manual --dry-run while reporting projected freed space", () => {
      const fixture = createComplexTargetFixture(tempRoot);
      const snapshotBefore = computeTreeSnapshot(fixture.targetDir);

      const result = runPruneTarget(["--dry-run"], tempRoot);

      const snapshotAfter = computeTreeSnapshot(fixture.targetDir);
      // CRYPTOGRAPHIC IMMUTABILITY: Entire directory tree must be identical
      expect(snapshotAfter).toEqual(snapshotBefore);

      // Projected numbers must reflect the work that WOULD have been done
      expect(result.count).toBeGreaterThan(0);
      expect(result.bytesFreed).toBeGreaterThan(0);
      expect(result.finalTargetSize).toBe(result.initialTargetSize - result.bytesFreed);

      // Markers remain 100% untouched
      expect(fs.existsSync(fixture.branchMarker)).toBe(true);
      expect(fs.existsSync(fixture.cooldownMarker)).toBe(true);
    });

    it("guarantees 100% byte-for-byte tree identity under --auto --dry-run without mutating cooldown", () => {
      const fixture = createComplexTargetFixture(tempRoot);
      const snapshotBefore = computeTreeSnapshot(fixture.targetDir);

      const result = runPruneTarget(["--auto", "--dry-run"], tempRoot);

      const snapshotAfter = computeTreeSnapshot(fixture.targetDir);
      expect(snapshotAfter).toEqual(snapshotBefore);
      expect(result.count).toBeGreaterThan(0);
    });

    it("guarantees zero temporary staging artifacts (.pruning-*) are created during dry-run", () => {
      createComplexTargetFixture(tempRoot);
      runPruneTarget(["--dry-run"], tempRoot);

      // Search entire tempRoot for any *.pruning-* files or folders
      function findStaging(d) {
        let found = [];
        const entries = fs.readdirSync(d, { withFileTypes: true });
        for (const e of entries) {
          const full = path.join(d, e.name);
          if (e.name.includes(".pruning-")) found.push(full);
          if (e.isDirectory()) found = found.concat(findStaging(full));
        }
        return found;
      }

      const stagingFiles = findStaging(tempRoot);
      expect(stagingFiles).toEqual([]);
    });

    it("preserves expired (>24h TTL) branch marker in dry-run mode without unlinking", () => {
      const fixture = createComplexTargetFixture(tempRoot);
      // Write timestamp from 3 days ago
      const oldTimeSec = Math.floor((Date.now() - 72 * 60 * 60 * 1000) / 1000);
      fs.writeFileSync(fixture.branchMarker, oldTimeSec.toString());

      const snapshotBefore = computeTreeSnapshot(fixture.targetDir);
      runPruneTarget(["--dry-run"], tempRoot);
      const snapshotAfter = computeTreeSnapshot(fixture.targetDir);

      expect(snapshotAfter).toEqual(snapshotBefore);
      expect(fs.existsSync(fixture.branchMarker)).toBe(true);
    });
  });

  describe("Targeted Fixture: Nested Locks & Concurrency Conflict Handling", () => {
    it("detects lock files nested 2 levels deep inside session subdirectories", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const crateDir = path.join(debugDir, "incremental", "kokoro_app-1234567890abcdef");
      const subSession = path.join(crateDir, "s-subsession-1234");
      fs.mkdirSync(subSession, { recursive: true });

      const nestedLock = path.join(subSession, "nested.lock");
      fs.writeFileSync(nestedLock, "");

      // Idle lock
      expect(isSessionDirectoryLocked(crateDir, debugDir)).toBe(false);

      // Actively held nested lock
      const origOpen = fs.openSync.bind(fs);
      const openSpy = vi.spyOn(fs, "openSync").mockImplementation((p, flags, mode) => {
        if (typeof p === "string" && p.includes("nested.lock")) {
          const err = new Error("locked");
          err.code = "EBUSY";
          throw err;
        }
        return origOpen(p, flags, mode);
      });

      try {
        expect(isSessionDirectoryLocked(crateDir, debugDir)).toBe(true);
      } finally {
        openSpy.mockRestore();
      }
    });

    it("detects target/.cargo-lock global build lock and yields auto prune", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const incDir = path.join(debugDir, "incremental");
      fs.mkdirSync(incDir, { recursive: true });

      const cargoLock = path.join(targetDir, ".cargo-lock");
      fs.writeFileSync(cargoLock, "");

      const branchMarker = path.join(targetDir, ".branch-switched");
      fs.writeFileSync(branchMarker, Date.now().toString());

      // Mock cargo-lock actively locked by running cargo
      const origOpen = fs.openSync.bind(fs);
      const openSpy = vi.spyOn(fs, "openSync").mockImplementation((p, flags, mode) => {
        if (typeof p === "string" && p.includes(".cargo-lock")) {
          const err = new Error("cargo build in progress");
          err.code = "EBUSY";
          throw err;
        }
        return origOpen(p, flags, mode);
      });

      try {
        expect(hasActiveIncrementalLocks(incDir, debugDir)).toBe(true);

        const res = runPruneTarget(["--auto"], tempRoot);
        // Clean yield: 0 items pruned, marker preserved
        expect(res.count).toBe(0);
        expect(fs.existsSync(branchMarker)).toBe(true);
      } finally {
        openSpy.mockRestore();
      }
    });

    it("handles concurrency split: auto mode yields entirely; manual mode skips locked crate, prunes unlocked crate, and retains branch marker", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const incDir = path.join(debugDir, "incremental");

      // Crate A: locked
      const crateA = path.join(incDir, "crate_locked-1111111111111111");
      fs.mkdirSync(crateA, { recursive: true });
      fs.writeFileSync(path.join(crateA, "s-lock.lock"), "");
      fs.writeFileSync(path.join(crateA, "data.bin"), "locked");

      // Crate B: unlocked with 2 sessions (one older, one newer)
      const crateB1 = path.join(incDir, "crate_free-1111111111111111");
      const crateB2 = path.join(incDir, "crate_free-2222222222222222");
      fs.mkdirSync(crateB1, { recursive: true });
      fs.mkdirSync(crateB2, { recursive: true });
      fs.writeFileSync(path.join(crateB1, "data.bin"), "stale");
      fs.writeFileSync(path.join(crateB2, "data.bin"), "fresh");
      fs.utimesSync(crateB1, new Date(Date.now() - 100000), new Date(Date.now() - 100000));

      const branchMarker = path.join(targetDir, ".branch-switched");
      fs.writeFileSync(branchMarker, Date.now().toString());

      // Mock lock on Crate A
      const origOpen = fs.openSync.bind(fs);
      const openSpy = vi.spyOn(fs, "openSync").mockImplementation((p, flags, mode) => {
        if (typeof p === "string" && p.includes("crate_locked")) {
          const err = new Error("busy");
          err.code = "EBUSY";
          throw err;
        }
        return origOpen(p, flags, mode);
      });

      try {
        // 1. Auto mode: must yield completely
        const autoRes = runPruneTarget(["--auto"], tempRoot);
        expect(autoRes.count).toBe(0);
        expect(fs.existsSync(crateB1)).toBe(true);
        expect(fs.existsSync(branchMarker)).toBe(true);

        // 2. Manual mode: prunes crateB1, skips crateA, but preserves branchMarker because cleanup was incomplete
        const manualRes = runPruneTarget([], tempRoot);
        expect(manualRes.count).toBe(1);
        expect(fs.existsSync(crateB1)).toBe(false);
        expect(fs.existsSync(crateB2)).toBe(true);
        expect(fs.existsSync(crateA)).toBe(true);

        // CRITICAL: .branch-switched MUST be retained for retry because crateA was skipped
        expect(fs.existsSync(branchMarker)).toBe(true);
      } finally {
        openSpy.mockRestore();
      }
    });

    it("lock release lifecycle: subsequent run consumes marker after compiler lock is released", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const incDir = path.join(debugDir, "incremental");

      const crateOld = path.join(incDir, "crate_test-1111111111111111");
      const crateNew = path.join(incDir, "crate_test-2222222222222222");
      fs.mkdirSync(crateOld, { recursive: true });
      fs.mkdirSync(crateNew, { recursive: true });
      fs.writeFileSync(path.join(crateOld, "s-test.lock"), "");
      fs.utimesSync(crateOld, new Date(Date.now() - 100000), new Date(Date.now() - 100000));

      const branchMarker = path.join(targetDir, ".branch-switched");
      fs.writeFileSync(branchMarker, Date.now().toString());

      let isLocked = true;
      const origOpen = fs.openSync.bind(fs);
      const openSpy = vi.spyOn(fs, "openSync").mockImplementation((p, flags, mode) => {
        if (isLocked && typeof p === "string" && p.includes("s-test.lock")) {
          const err = new Error("locked");
          err.code = "EBUSY";
          throw err;
        }
        return origOpen(p, flags, mode);
      });

      try {
        // Step 1: Locked -> yields, marker preserved
        const res1 = runPruneTarget(["--auto"], tempRoot);
        expect(res1.count).toBe(0);
        expect(fs.existsSync(branchMarker)).toBe(true);

        // Step 2: Lock released -> prune completes, marker consumed!
        isLocked = false;
        const res2 = runPruneTarget(["--auto"], tempRoot);
        expect(res2.count).toBe(1);
        expect(fs.existsSync(crateOld)).toBe(false);
        expect(fs.existsSync(crateNew)).toBe(true);
        expect(fs.existsSync(branchMarker)).toBe(false);
      } finally {
        openSpy.mockRestore();
      }
    });
  });

  describe("Targeted Fixture: Reparse Points, Directory Junctions & Symlink Escape Defense", () => {
    it("safely handles dangling / broken symlinks without unhandled ENOENT errors", () => {
      const boundary = path.join(tempRoot, "safe-boundary");
      fs.mkdirSync(boundary, { recursive: true });

      const nonExistentTarget = path.join(tempRoot, "does-not-exist");
      const brokenLink = path.join(boundary, "broken-link");

      // Create broken link
      try {
        fs.symlinkSync(nonExistentTarget, brokenLink, process.platform === "win32" ? "junction" : "dir");
      } catch (err) {
        if (err?.code === "EPERM" || err?.code === "EACCES") return; // Skip if OS denies symlink
      }

      if (fs.existsSync(brokenLink) || (fs.lstatSync(brokenLink).isSymbolicLink())) {
        const res = safeRemove(brokenLink, boundary);
        // Must reject symlink safely without throwing
        expect(res.success).toBe(false);
        expect(res.reason).toBe("symlink");
      }
    });

    it("verifies safeRemove never deletes the symlink or junction itself", () => {
      const boundary = path.join(tempRoot, "boundary");
      fs.mkdirSync(boundary, { recursive: true });

      const externalDir = path.join(tempRoot, "external-target");
      fs.mkdirSync(externalDir);
      fs.writeFileSync(path.join(externalDir, "data.txt"), "data");

      const linkPath = path.join(boundary, "my-link");
      createDirLink(externalDir, linkPath);

      const res = safeRemove(linkPath, boundary);
      expect(res.success).toBe(false);
      expect(res.reason).toBe("symlink");

      // CRITICAL: The link node itself in boundary must NOT have been unlinked!
      const lstat = fs.lstatSync(linkPath);
      expect(lstat.isSymbolicLink()).toBe(true);
      expect(fs.existsSync(externalDir)).toBe(true);
    });

    it("prevents symlink in deps pointing to repository source files from being unlinked", () => {
      const srcTauri = path.join(tempRoot, "src-tauri");
      const srcDir = path.join(srcTauri, "src");
      const targetDir = path.join(srcTauri, "target");
      const debugDir = path.join(targetDir, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(srcDir, { recursive: true });
      fs.mkdirSync(depsDir, { recursive: true });

      const mainSource = path.join(srcDir, "main.rs");
      fs.writeFileSync(mainSource, "fn main() { println!(\"Kokoro\"); }");

      const linkInDeps = path.join(depsDir, "characters-1111111111111111.exe");
      if (tryCreateFileLink(mainSource, linkInDeps)) {
        fs.utimesSync(linkInDeps, new Date(Date.now() - 100000), new Date(Date.now() - 100000));

        const res = pruneDeps(depsDir, true, debugDir);
        expect(res.count).toBe(0);

        // Source file must be completely untouched!
        expect(fs.existsSync(mainSource)).toBe(true);
        expect(fs.readFileSync(mainSource, "utf8")).toBe("fn main() { println!(\"Kokoro\"); }");
      }
    });
  });

  describe("Targeted Fixture: Cross-Platform Filename Pattern Matching & Artifact Classification", () => {
    it("Linux POSIX matrix: prunes extensionless test binaries while keeping main app and third-party .so/.rlib untouched", () => {
      const debugDir = path.join(tempRoot, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      const now = Date.now();
      const past40m = new Date(now - 40 * 60 * 1000);
      const past5m = new Date(now - 5 * 60 * 1000);

      // Linux test harness binaries (NO extension!)
      const charOld = path.join(depsDir, "characters-1111111111111111");
      const charNew = path.join(depsDir, "characters-2222222222222222");
      const charOldD = path.join(depsDir, "characters-1111111111111111.d");
      const charNewD = path.join(depsDir, "characters-2222222222222222.d");
      fs.writeFileSync(charOld, "old-binary");
      fs.writeFileSync(charNew, "new-binary");
      fs.writeFileSync(charOldD, "old-d");
      fs.writeFileSync(charNewD, "new-d");
      fs.utimesSync(charOld, past40m, past40m);
      fs.utimesSync(charOldD, past40m, past40m);
      fs.utimesSync(charNew, past5m, past5m);
      fs.utimesSync(charNewD, past5m, past5m);

      // Third-party shared libraries on Linux (.so) and libraries (.rlib)
      const synSo = path.join(depsDir, "libsyn-1111111111111111.so");
      const serdeRlib = path.join(depsDir, "libserde-1111111111111111.rlib");
      fs.writeFileSync(synSo, "macro-so");
      fs.writeFileSync(serdeRlib, "serde-rlib");
      fs.utimesSync(synSo, past40m, past40m);
      fs.utimesSync(serdeRlib, past40m, past40m);

      // Run pruneDeps in emergency mode to bypass 30m shield
      const res = pruneDeps(depsDir, true, debugDir);

      // Pruned charOld and charOldD
      expect(res.count).toBe(2);
      expect(fs.existsSync(charOld)).toBe(false);
      expect(fs.existsSync(charOldD)).toBe(false);
      expect(fs.existsSync(charNew)).toBe(true);
      expect(fs.existsSync(charNewD)).toBe(true);

      // Third-party libraries are 100% untouched
      expect(fs.existsSync(synSo)).toBe(true);
      expect(fs.existsSync(serdeRlib)).toBe(true);
    });

    it("macOS POSIX matrix: prunes extensionless test binaries and associated .dSYM bundles while preserving .dylib", () => {
      const debugDir = path.join(tempRoot, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      const now = Date.now();
      const past40m = new Date(now - 40 * 60 * 1000);
      const past5m = new Date(now - 5 * 60 * 1000);

      // macOS test harness binaries + .dSYM directory bundle
      const visionOld = path.join(depsDir, "vision-1111111111111111");
      const visionNew = path.join(depsDir, "vision-2222222222222222");
      const visionOldDsym = path.join(depsDir, "vision-1111111111111111.dsym");
      const visionNewDsym = path.join(depsDir, "vision-2222222222222222.dsym");

      fs.writeFileSync(visionOld, "old-mach-o");
      fs.writeFileSync(visionNew, "new-mach-o");
      fs.mkdirSync(visionOldDsym);
      fs.writeFileSync(path.join(visionOldDsym, "dwarf"), "dwarf-symbols");
      fs.mkdirSync(visionNewDsym);
      fs.writeFileSync(path.join(visionNewDsym, "dwarf"), "dwarf-symbols");

      fs.utimesSync(visionOld, past40m, past40m);
      fs.utimesSync(visionOldDsym, past40m, past40m);
      fs.utimesSync(visionNew, past5m, past5m);
      fs.utimesSync(visionNewDsym, past5m, past5m);

      // Third-party macOS dynamic library (.dylib)
      const dylib = path.join(depsDir, "libserde_derive-1111111111111111.dylib");
      fs.writeFileSync(dylib, "macro-dylib");
      fs.utimesSync(dylib, past40m, past40m);

      const res = pruneDeps(depsDir, true, debugDir);

      expect(res.count).toBe(2);
      expect(fs.existsSync(visionOld)).toBe(false);
      expect(fs.existsSync(visionOldDsym)).toBe(false);
      expect(fs.existsSync(visionNew)).toBe(true);
      expect(fs.existsSync(visionNewDsym)).toBe(true);

      expect(fs.existsSync(dylib)).toBe(true);
    });

    it("third-party name collision: preserves external .rlib even if name matches test harness keyword (e.g. libcharacters.rlib)", () => {
      const debugDir = path.join(tempRoot, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      const past50m = new Date(Date.now() - 50 * 60 * 1000);
      const fakeCrateRlib = path.join(depsDir, "libcharacters-1111111111111111.rlib");
      const fakeVisionRlib = path.join(depsDir, "vision-1111111111111111.rlib");

      fs.writeFileSync(fakeCrateRlib, "not-a-test-harness");
      fs.writeFileSync(fakeVisionRlib, "not-a-test-harness");
      fs.utimesSync(fakeCrateRlib, past50m, past50m);
      fs.utimesSync(fakeVisionRlib, past50m, past50m);

      const res = pruneDeps(depsDir, true, debugDir);
      expect(res.count).toBe(0);
      expect(fs.existsSync(fakeCrateRlib)).toBe(true);
      expect(fs.existsSync(fakeVisionRlib)).toBe(true);
    });

    it("dynamically discovers unlisted executable targets and manages their lifecycles", () => {
      const debugDir = path.join(tempRoot, "debug");
      const depsDir = path.join(debugDir, "deps");
      fs.mkdirSync(depsDir, { recursive: true });

      const past40m = new Date(Date.now() - 40 * 60 * 1000);
      const past5m = new Date(Date.now() - 5 * 60 * 1000);

      // Custom suite discovered as an executable
      const customExeOld = path.join(depsDir, "benchmarks_runner-1111111111111111.exe");
      const customExeNew = path.join(depsDir, "benchmarks_runner-2222222222222222.exe");
      fs.writeFileSync(customExeOld, "old-bench");
      fs.writeFileSync(customExeNew, "new-bench");
      fs.utimesSync(customExeOld, past40m, past40m);
      fs.utimesSync(customExeNew, past5m, past5m);

      const res = pruneDeps(depsDir, true, debugDir);
      expect(res.count).toBe(1);
      expect(fs.existsSync(customExeOld)).toBe(false);
      expect(fs.existsSync(customExeNew)).toBe(true);
    });
  });

  describe("Targeted Fixture: Git Hook Setup, Chaining & Shebang Integrity", () => {
    it("handles clean repository: initializes core.hooksPath to .githooks and verifies permissions", () => {
      const fakeGit = path.join(tempRoot, ".git");
      const fakeGithooks = path.join(tempRoot, ".githooks");
      fs.mkdirSync(fakeGit, { recursive: true });
      fs.mkdirSync(fakeGithooks, { recursive: true });

      fs.writeFileSync(path.join(fakeGithooks, "post-checkout"), "#!/bin/sh\nexit 0\n");
      fs.writeFileSync(path.join(fakeGithooks, "post-merge"), "#!/bin/sh\nexit 0\n");

      let configuredPath = null;
      setGitExecutorForTesting((cmd) => {
        if (typeof cmd === "string" && cmd.includes("git config core.hooksPath .githooks")) {
          configuredPath = ".githooks";
          return "";
        }
        return "";
      });

      try {
        const res = runPruneTarget(["--setup-hooks"], tempRoot);
        expect(res.success).toBe(true);
        expect(res.status).toBe("registered_githooks");
        expect(configuredPath).toBe(".githooks");
      } finally {
        setGitExecutorForTesting(null);
      }
    });

    it("chains idempotently: running setup 3 times in a repository with existing hooks creates exactly 1 Sentinel block", () => {
      const fakeGit = path.join(tempRoot, ".git");
      const fakeHooks = path.join(fakeGit, "hooks");
      fs.mkdirSync(fakeHooks, { recursive: true });

      const userScript = "#!/bin/sh\necho 'running lint'\nexit 0\n";
      fs.writeFileSync(path.join(fakeHooks, "pre-commit"), userScript);
      fs.writeFileSync(path.join(fakeHooks, "post-checkout"), "#!/bin/sh\n# custom post-checkout\nexit 0\n");

      setGitExecutorForTesting((cmd) => {
        if (typeof cmd === "string" && cmd.includes("git config --get core.hooksPath")) {
          throw new Error("not set");
        }
        if (typeof cmd === "string" && cmd.includes("git rev-parse --git-path hooks")) {
          return fakeHooks + "\n";
        }
        return "";
      });

      try {
        runPruneTarget(["--setup-hooks"], tempRoot);
        runPruneTarget(["--setup-hooks"], tempRoot);
        runPruneTarget(["--setup-hooks"], tempRoot);

        const content = fs.readFileSync(path.join(fakeHooks, "post-checkout"), "utf8");
        const count = content.split(SENTINEL_BLOCK_START).length - 1;
        expect(count).toBe(1);

        // User logic preserved
        expect(content.includes("# custom post-checkout")).toBe(true);
      } finally {
        setGitExecutorForTesting(null);
      }
    });
  });

  describe("Targeted Fixture: Windows / macOS / Linux Behavioral Differences", () => {
    it("handles case-folding on Windows and case-sensitivity on POSIX in canonical descendancy checks", () => {
      const parent = path.join(tempRoot, "parent");
      const childUpper = path.join(tempRoot, "PARENT", "child");
      fs.mkdirSync(parent, { recursive: true });

      if (process.platform === "win32") {
        expect(isCanonicallyContained(childUpper, parent)).toBe(true);
      } else {
        expect(isCanonicallyContained(childUpper, parent)).toBe(false);
      }
    });

    it("handles drive space and watermark calculation correctly when host storage has release and vendor binaries", () => {
      const fixture = createComplexTargetFixture(tempRoot);
      const initialSize = getDirSize(fixture.targetDir);
      expect(initialSize).toBeGreaterThan(0);

      // Verify release binaries are accounted for in total footprint but excluded from deletion
      const res = runPruneTarget([], tempRoot);
      expect(res.count).toBeGreaterThan(0);
      expect(fs.existsSync(path.join(fixture.releaseDir, "kokoro.exe"))).toBe(true);
    });
  });
});
