#!/usr/bin/env node

import { accessSync, constants, statSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

/** Recover an installed Cargo when a terminal inherited an outdated PATH. */
export function getTauriEnvironment(environment = process.env, userHome = homedir()) {
  const env = { ...environment };
  const isWindows = process.platform === "win32";
  // Windows treats environment names as case-insensitive. Match Node's choice
  // and avoid passing conflicting PATH/Path entries to native child processes.
  const pathKeys = isWindows
    ? Object.keys(env).filter((key) => key.toLowerCase() === "path").sort()
    : ["PATH"];
  const pathKey = pathKeys[0] ?? (isWindows ? "Path" : "PATH");
  const currentPath = env[pathKey] ?? "";
  for (const duplicateKey of pathKeys.slice(1)) delete env[duplicateKey];

  const cargoName = isWindows ? "cargo.exe" : "cargo";
  const hasCargo = (directory) => {
    if (!directory) return false;
    const cargo = path.join(directory, cargoName);
    try {
      if (!statSync(cargo).isFile()) return false;
      if (!isWindows) accessSync(cargo, constants.X_OK);
      return true;
    } catch {
      return false;
    }
  };

  // Keep an explicitly selected toolchain ahead of any fallback installation.
  if (currentPath.split(path.delimiter).some(hasCargo)) return env;

  const cargoHomes = [environment.CARGO_HOME, path.join(userHome, ".cargo")];
  const cargoBin = cargoHomes
    .filter(Boolean)
    .map((cargoHome) => path.join(cargoHome, "bin"))
    .find(hasCargo);
  if (cargoBin) {
    env[pathKey] = currentPath ? `${cargoBin}${path.delimiter}${currentPath}` : cargoBin;
  }
  return env;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  Object.assign(process.env, getTauriEnvironment());
  // Run the official CLI in this process so argument parsing, exit codes and
  // Ctrl+C handling remain owned by Tauri; its children inherit the repaired PATH.
  await import("@tauri-apps/cli/tauri.js");
}
