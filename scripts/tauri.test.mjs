import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
const launcherSource = new URL("./tauri.mjs", import.meta.url);

function resolveTauriEnvironment(env, userHome) {
  // Load the same hashbang entry point as the package scripts, even on a
  // CRLF checkout where Vitest's SSR transform can't evaluate the hashbang.
  const stdout = execFileSync(process.execPath, [
    "--input-type=module",
    "--eval",
    [
      "const [launcher, env, userHome] = process.argv.slice(1);",
      "const { getTauriEnvironment } = await import(launcher);",
      "console.log(JSON.stringify(getTauriEnvironment(Object.freeze(JSON.parse(env)), userHome)));",
    ].join("\n"),
    launcherSource.href,
    JSON.stringify(env),
    userHome,
  ], { encoding: "utf8" });
  return JSON.parse(stdout);
}

describe("Tauri Cargo discovery", () => {
  let root;
  let userHome;
  const pathKey = process.platform === "win32" ? "Path" : "PATH";
  const cargoName = process.platform === "win32" ? "cargo.exe" : "cargo";

  function installCargo(cargoHome) {
    const bin = path.join(cargoHome, "bin");
    mkdirSync(bin, { recursive: true });
    writeFileSync(path.join(bin, cargoName), "fixture", { mode: 0o755 });
    return bin;
  }

  beforeEach(() => {
    root = mkdtempSync(path.join(tmpdir(), "kokoro tauri env-"));
    userHome = path.join(root, "user home");
  });

  afterEach(() => {
    // The only removal target is the directory created by this test's mkdtemp.
    rmSync(root, { recursive: true, force: true });
  });

  it("recovers default Cargo from a stale PATH without changing the caller's environment", () => {
    const cargoBin = installCargo(path.join(userHome, ".cargo"));
    const env = { [pathKey]: path.join(root, "node"), KEEP_ME: "unchanged" };

    const result = resolveTauriEnvironment(env, userHome);

    expect(result[pathKey]).toBe(`${cargoBin}${path.delimiter}${env[pathKey]}`);
    expect(result.KEEP_ME).toBe("unchanged");
    expect(env[pathKey]).toBe(path.join(root, "node"));
    expect(resolveTauriEnvironment(result, userHome)).toEqual(result);
  });

  it("uses CARGO_HOME, including paths containing spaces, before the default installation", () => {
    installCargo(path.join(userHome, ".cargo"));
    const cargoHome = path.join(root, "custom Rust home");
    const cargoBin = installCargo(cargoHome);

    const result = resolveTauriEnvironment({ CARGO_HOME: cargoHome, [pathKey]: "" }, userHome);

    expect(result[pathKey]).toBe(cargoBin);
  });

  it("preserves an existing Cargo on PATH", () => {
    installCargo(path.join(userHome, ".cargo"));
    const selectedBin = installCargo(path.join(root, "selected toolchain"));
    const env = { [pathKey]: selectedBin };

    expect(resolveTauriEnvironment(env, userHome)).toEqual(env);
  });

  it("falls back to the default installation if CARGO_HOME has no Cargo", () => {
    const cargoBin = installCargo(path.join(userHome, ".cargo"));
    const result = resolveTauriEnvironment({ CARGO_HOME: path.join(root, "missing") }, userHome);

    expect(result[pathKey]).toBe(cargoBin);
  });

  it("keeps PATH unchanged when Rust is not installed", () => {
    const env = { [pathKey]: path.join(root, "node") };

    expect(resolveTauriEnvironment(env, userHome)).toEqual(env);
  });

  it.skipIf(process.platform !== "win32")("normalizes conflicting Windows PATH casing", () => {
    const cargoBin = installCargo(path.join(userHome, ".cargo"));
    const result = resolveTauriEnvironment({ PATH: "first", Path: "second" }, userHome);

    expect(result.PATH).toBe(`${cargoBin};first`);
    expect(result).not.toHaveProperty("Path");
  });

  describe.skipIf(process.platform === "win32")("launcher on POSIX", () => {
    let launcher;

    beforeEach(() => {
      launcher = path.join(root, "tauri.mjs");
      copyFileSync(launcherSource, launcher);

      // Exercise the real launcher entry point and a real Cargo subprocess with
      // an isolated CLI entry point that needs no Rust or native Tauri libraries.
      const cli = path.join(root, "node_modules", "@tauri-apps", "cli");
      mkdirSync(cli, { recursive: true });
      writeFileSync(path.join(cli, "tauri.js"), [
        'const { execFileSync } = require("node:child_process");',
        'const cargo = execFileSync("cargo", [], { encoding: "utf8" });',
        'console.log(JSON.stringify({ cargo, path: process.env.PATH, args: process.argv.slice(2) }));',
      ].join("\n"));
    });

    function makeCargo(bin, output, mode = 0o755) {
      mkdirSync(bin, { recursive: true });
      writeFileSync(path.join(bin, "cargo"), "#!/bin/sh\nprintf '%s' " + output + "\n", { mode });
      return bin;
    }

    function launch(pathValue, extraEnv = {}) {
      const result = spawnSync(process.execPath, [launcher, "dev", "--fixture"], {
        cwd: root,
        env: { HOME: userHome, PATH: pathValue, ...extraEnv },
        encoding: "utf8",
      });
      expect(result.error).toBeUndefined();
      expect(result.status, result.stderr).toBe(0);
      return JSON.parse(result.stdout);
    }

    it.each(["non-executable file", "directory"])(
      "falls back to executable home Cargo when PATH contains a %s named cargo",
      (trap) => {
        const pathBin = path.join(root, "stale PATH");
        if (trap === "directory") {
          mkdirSync(path.join(pathBin, "cargo"), { recursive: true });
        } else {
          makeCargo(pathBin, "unusable-cargo", 0o644);
        }
        const homeBin = makeCargo(path.join(userHome, ".cargo", "bin"), "home-cargo");

        expect(launch(pathBin)).toEqual({
          cargo: "home-cargo",
          path: homeBin + path.delimiter + pathBin,
          args: ["dev", "--fixture"],
        });
      },
    );

    it("keeps a working PATH even when an earlier Cargo is not executable", () => {
      const unusableBin = makeCargo(path.join(root, "stale PATH"), "unusable-cargo", 0o644);
      const selectedBin = makeCargo(path.join(root, "chosen PATH"), "selected-cargo");
      makeCargo(path.join(userHome, ".cargo", "bin"), "home-cargo");
      const pathValue = [unusableBin, selectedBin].join(path.delimiter);

      expect(launch(pathValue)).toEqual({
        cargo: "selected-cargo",
        path: pathValue,
        args: ["dev", "--fixture"],
      });
    });

    it("ignores a non-executable custom Cargo home", () => {
      const unusableHome = path.join(root, "unusable Cargo home");
      makeCargo(path.join(unusableHome, "bin"), "unusable-cargo", 0o644);
      const homeBin = makeCargo(path.join(userHome, ".cargo", "bin"), "home-cargo");

      expect(launch("", { CARGO_HOME: unusableHome })).toEqual({
        cargo: "home-cargo",
        path: homeBin,
        args: ["dev", "--fixture"],
      });
    });
  });
});
