import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const generated = "src/lib/generated/ipc.ts";
const errors = [];

for (const filename of ts.sys.readDirectory("src", [".ts", ".tsx"])) {
  const relative = path.relative(".", filename).replaceAll("\\", "/");
  if (relative === generated || /\.test\.[cm]?[jt]sx?$/.test(relative)) continue;

  const source = ts.createSourceFile(filename, fs.readFileSync(filename, "utf8"), ts.ScriptTarget.Latest, true);
  const report = (node, message) => {
    const { line } = source.getLineAndCharacterOfPosition(node.getStart(source));
    errors.push(`${relative}:${line + 1} ${message}`);
  };

  function visit(node) {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier)) {
      const moduleName = node.moduleSpecifier.text;
      const clause = node.importClause;
      const bindings = clause?.namedBindings;
      if (clause && !clause.isTypeOnly) {
        if (moduleName === "@tauri-apps/api/event" && (
          clause.name || (bindings && ts.isNamespaceImport(bindings))
          || bindings?.elements.some(entry => !entry.isTypeOnly)
        )) {
          report(node, "Use generated project event bindings.");
        }
        if (moduleName === "@tauri-apps/api/core" && (
          clause.name || (bindings && ts.isNamespaceImport(bindings))
          || bindings?.elements.some(entry => !entry.isTypeOnly && (entry.propertyName ?? entry.name).text === "invoke")
        )) {
          report(node, "Use generated project command bindings.");
        }
      }
    }
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
      const specifier = node.arguments[0];
      if (specifier && ts.isStringLiteral(specifier)
        && ["@tauri-apps/api/core", "@tauri-apps/api/event"].includes(specifier.text)) {
        report(node, "Use generated IPC bindings for project commands and events.");
      }
    }
    if (ts.isPropertyAccessExpression(node) && node.name.text === "invoke"
      && ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === "__TAURI_INTERNALS__") {
      report(node, "Use generated project command bindings.");
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
}

if (errors.length) {
  console.error(errors.join("\n"));
  process.exitCode = 1;
} else {
  console.log("Frontend IPC transport usage check passed.");
}
