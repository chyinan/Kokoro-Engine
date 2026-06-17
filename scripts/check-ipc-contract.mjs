import fs from "node:fs";
import path from "node:path";

const root = process.cwd();
const rustLibPath = path.join(root, "src-tauri", "src", "lib.rs");
const srcDir = path.join(root, "src");

function read(filePath) {
  return fs.readFileSync(filePath, "utf8");
}

function walkFiles(dir, result = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walkFiles(fullPath, result);
      continue;
    }
    if (/\.(ts|tsx)$/.test(entry.name)) {
      result.push(fullPath);
    }
  }
  return result;
}

function registeredCommands() {
  const rust = read(rustLibPath);
  const match = rust.match(/generate_handler!\s*\[\s*([\s\S]*?)\s*\]\)/);
  if (!match) {
    throw new Error("Could not find tauri::generate_handler! block in src-tauri/src/lib.rs");
  }

  const block = match[1];
  const commands = new Set();
  const commandPattern = /(?:commands::[a-zA-Z0-9_]+::|stt::stream::)([a-zA-Z0-9_]+)/g;
  for (const item of block.matchAll(commandPattern)) {
    commands.add(item[1]);
  }
  return commands;
}

function invokedCommands() {
  const commands = new Map();
  const invokePattern = /\binvoke(?:<[^>]+>)?\(\s*["'`]([a-zA-Z0-9_]+)["'`]/g;
  const safeInvokePattern = /\bsafeInvoke(?:<[^>]+>)?\(\s*["'`]([a-zA-Z0-9_]+)["'`]/g;

  for (const filePath of walkFiles(srcDir)) {
    const text = read(filePath);
    const relative = path.relative(root, filePath).replaceAll("\\", "/");
    for (const pattern of [invokePattern, safeInvokePattern]) {
      for (const item of text.matchAll(pattern)) {
        const command = item[1];
        if (!commands.has(command)) {
          commands.set(command, []);
        }
        commands.get(command).push(relative);
      }
    }
  }
  return commands;
}

const registered = registeredCommands();
const invoked = invokedCommands();
const missing = [...invoked.keys()].filter((command) => !registered.has(command)).sort();

if (missing.length > 0) {
  console.error("TypeScript invokes commands that are not registered in src-tauri/src/lib.rs:");
  for (const command of missing) {
    console.error(`- ${command}: ${invoked.get(command).join(", ")}`);
  }
  process.exit(1);
}

console.log(`IPC contract check passed: ${invoked.size} invoked command names are registered.`);
