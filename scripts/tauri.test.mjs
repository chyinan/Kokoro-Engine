import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { getTauriEnvironment } from "./tauri.mjs";

describe("Tauri Cargo discovery", () => {
  let root;
  let userHome;
  const pathKey = process.platform === "win32" ? "Path" : "PATH";
  const cargoName = process.platform === "win32" ? "cargo.exe" : "cargo";

  function installCargo(cargoHome) {
    const bin = path.join(cargoHome, "bin");
    mkdirSync(bin, { recursive: true });
    writeFileSync(path.join(bin, cargoName), "fixture");
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

    const result = getTauriEnvironment(env, userHome);

    expect(result[pathKey]).toBe(`${cargoBin}${path.delimiter}${env[pathKey]}`);
    expect(result.KEEP_ME).toBe("unchanged");
    expect(env[pathKey]).toBe(path.join(root, "node"));
    expect(getTauriEnvironment(result, userHome)).toEqual(result);
  });

  it("uses CARGO_HOME, including paths containing spaces, before the default installation", () => {
    installCargo(path.join(userHome, ".cargo"));
    const cargoHome = path.join(root, "custom Rust home");
    const cargoBin = installCargo(cargoHome);

    const result = getTauriEnvironment({ CARGO_HOME: cargoHome, [pathKey]: "" }, userHome);

    expect(result[pathKey]).toBe(cargoBin);
  });

  it("preserves an existing Cargo on PATH", () => {
    installCargo(path.join(userHome, ".cargo"));
    const selectedBin = installCargo(path.join(root, "selected toolchain"));
    const env = { [pathKey]: selectedBin };

    expect(getTauriEnvironment(env, userHome)).toEqual(env);
  });

  it("falls back to the default installation if CARGO_HOME has no Cargo", () => {
    const cargoBin = installCargo(path.join(userHome, ".cargo"));
    const result = getTauriEnvironment({ CARGO_HOME: path.join(root, "missing") }, userHome);

    expect(result[pathKey]).toBe(cargoBin);
  });

  it("keeps PATH unchanged when Rust is not installed", () => {
    const env = { [pathKey]: path.join(root, "node") };

    expect(getTauriEnvironment(env, userHome)).toEqual(env);
  });

  it.skipIf(process.platform !== "win32")("normalizes conflicting Windows PATH casing", () => {
    const cargoBin = installCargo(path.join(userHome, ".cargo"));
    const result = getTauriEnvironment({ PATH: "first", Path: "second" }, userHome);

    expect(result.PATH).toBe(`${cargoBin};first`);
    expect(result).not.toHaveProperty("Path");
  });
});
