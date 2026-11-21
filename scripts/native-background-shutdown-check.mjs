import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";

// Run from the checkout with a default-config desktop debug build and free
// loopback ports 1420, 19334 and 19335.
const [binary, playwright, outputDirectory] = process.argv.slice(2);
assert(
  binary && playwright && outputDirectory,
  "Usage: node scripts/native-background-shutdown-check.mjs <binary> <Playwright module directory> <new output directory>"
);
const cwd = process.cwd();
const executable = path.resolve(binary);
const { chromium } = createRequire(import.meta.url)(path.resolve(playwright));
const output = path.resolve(outputDirectory);
const python = process.env.KOKORO_FIXTURE_PYTHON ?? "python";
const endpoints = { vite: "http://localhost:1420", cdp: "http://127.0.0.1:19335", fixture: "http://127.0.0.1:19334" };
fs.mkdirSync(output, { recursive: true });
assert.equal(fs.readdirSync(output).length, 0, "Use a new empty output directory");

async function portIsOpen(host, port) {
  return new Promise((resolve) => {
    const connection = net.connect({ host, port });
    const finish = (open) => { connection.destroy(); resolve(open); };
    connection.once("connect", () => finish(true));
    connection.once("error", () => finish(false));
    connection.setTimeout(400, () => finish(false));
  });
}
for (const [host, port] of [["127.0.0.1", 1420], ["::1", 1420], ["127.0.0.1", 19334], ["127.0.0.1", 19335]]) {
  assert(!(await portIsOpen(host, port)), `Port ${host}:${port} is already in use`);
}

const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(read, description, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const result = await read();
    if (result) return result;
    await delay(35);
  }
  throw new Error("Timed out waiting for " + description);
}
const reachable = (url) => fetch(url, { signal: AbortSignal.timeout(450) }).then((r) => r.ok).catch(() => false);
const report = {
  executable,
  sha256: createHash("sha256").update(fs.readFileSync(executable)).digest("hex"),
  scenarios: []
};
function saveReport() {
  fs.writeFileSync(path.join(output, "native-shutdown-report.json"), JSON.stringify(report, null, 2));
}
function launch(program, args, label, directory, env = process.env) {
  fs.mkdirSync(directory, { recursive: true });
  const log = fs.openSync(path.join(directory, label + ".log"), "w");
  const error = fs.openSync(path.join(directory, label + ".stderr.log"), "w");
  const child = spawn(program, args, { cwd, env, windowsHide: true, stdio: ["pipe", log, error] });
  fs.closeSync(log);
  fs.closeSync(error);
  let exit;
  const done = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => { exit = { code, signal }; resolve(exit); });
  });
  return { child, done, running: () => !exit };
}
function stop(proc) {
  if (proc?.running()) proc.child.kill();
}
function getRows(db, query, ...values) {
  return JSON.parse(execFileSync(python, [
    "-c",
    "import json, sqlite3, sys\nwith sqlite3.connect(sys.argv[1], timeout=5) as con:\n con.row_factory=sqlite3.Row\n print(json.dumps([dict(row) for row in con.execute(sys.argv[2],json.loads(sys.argv[3]))]))",
    db, query, JSON.stringify(values)
  ], { windowsHide: true, encoding: "utf8" }));
}
async function holdSqliteWriter(db, directory) {
  const lock = launch(python, [
    "-u", "-c",
    "import sys, sqlite3\nwith sqlite3.connect(sys.argv[1], timeout=5) as db:\n db.execute('BEGIN IMMEDIATE')\n print('READY', flush=True)\n sys.stdin.readline()\n db.rollback()",
    db
  ], "sqlite-writer", directory);
  await until(() => fs.readFileSync(path.join(directory, "sqlite-writer.log"), "utf8").includes("READY"), "SQLite lock");
  return lock;
}
async function startApp(name) {
  const directory = path.join(output, name);
  const data = path.join(directory, "data");
  const appData = path.join(data, "com.chyin.kokoro");
  fs.mkdirSync(appData, { recursive: true });
  fs.writeFileSync(path.join(appData, "llm_config.json"), JSON.stringify({
    active_provider: "fixture",
    system_provider: "fixture",
    providers: [{
      id: "fixture", provider_type: "openai", enabled: true, supports_native_tools: true,
      base_url: endpoints.fixture + "/v1", api_key: "controlled-fixture", model: "controlled-sse", extra: {}
    }]
  }));
  fs.writeFileSync(path.join(appData, "proactive_enabled.json"), '{"enabled":false}');
  fs.writeFileSync(path.join(appData, "memory_system_config.json"), '{"enabled":false}');
  const app = launch(executable, [], "app", directory, {
    ...process.env,
    KOKORO_TEST_DATA_ROOT: data,
    WEBVIEW2_USER_DATA_FOLDER: path.join(directory, "webview"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=19335",
    RUST_LOG: "info"
  });
  const db = path.join(appData, "kokoro.db");
  let browser;
  try {
    await until(() => reachable(endpoints.cdp + "/json/version"), "WebView2 debugging interface", 25000);
    browser = await chromium.connectOverCDP(endpoints.cdp);
    const context = browser.contexts()[0];
    const main = await until(
      () => context.pages().find((page) => page.url().includes("1420") && !page.url().includes("pet.html") && !page.url().includes("bubble.html")),
      "main WebView2 content"
    );
    await main.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__));
    return { app, browser, context, main, directory, db };
  } catch (error) {
    if (browser) await browser.close().catch(() => {});
    stop(app);
    throw error;
  }
}
const view = (main) => main.evaluate(() => window.__TAURI_INTERNALS__.invoke("get_chat_view"));
const counts = async () => (await fetch(endpoints.fixture + "/counts")).json();

async function readyCharacter(main) {
  // Visit a fixture document on the application's own dev origin so first-run
  // React effects cannot concurrently provision or activate a character.
  await main.route("http://localhost:1420/?shutdown-fixture", (route) => route.fulfill({
    contentType: "text/html",
    body: "<!doctype html><title>Kokoro lifecycle fixture</title>"
  }));
  await main.goto("http://localhost:1420/?shutdown-fixture");
  await main.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__));
  let lastError;
  try {
    await until(async () => main.evaluate(async () => {
    const bridge = await import("/src/lib/kokoro-bridge.ts");
    const templates = await bridge.listCharacterTemplates();
    localStorage.setItem("kokoro_provisioned_character_template_ids_v2", JSON.stringify(templates.map((entry) => entry.id)));
    for (const template of templates) {
      try {
        await bridge.instantiateCharacterTemplate({ template_id: template.id, template_version: template.version, instance_id: template.id, user_nickname: "Tester", created_at: 1, updated_at: 1 });
      } catch {}
    }
    const id = "shutdown-validation";
    try {
      if (!(await bridge.listCharacters()).some((entry) => entry.id === id)) {
        await bridge.createCharacter({
          id, name: "Shutdown Validation", persona: "Isolated integration fixture", user_nickname: "Tester",
          source_format: "manual", created_at: 1, updated_at: 1, runtime_profile_json: '{"proactive_enabled":false}'
        });
      }
      await bridge.commitCharacterActivation(await bridge.prepareCharacterActivation(id));
    } catch (error) {
      return { error: bridge.getKokoroErrorMessage(error) };
    }
    localStorage.setItem("kokoro_active_character_id", id);
    return { ready: true };
  }).then((result) => {
    lastError = result.error;
    return result.ready;
  }), "isolated character activation", 8000);
  } catch (error) {
    throw new Error(String(error) + (lastError ? ". " + lastError : ""));
  }
  assert.equal((await view(main)).character_id, "shutdown-validation");
}
async function submit(main, id, message, hidden = false) {
  const result = await view(main);
  await main.evaluate(async ({ id, message, hidden, snapshot }) => {
    const { listen } = await import("/node_modules/@tauri-apps/api/event.js");
    window.shutdownFixture = { complete: false, error: null };
    await listen("chat-turn-text-complete", () => { window.shutdownFixture.complete = true; });
    window.__TAURI_INTERNALS__.invoke("stream_chat", {
      request: {
        message, hidden, client_request_id: id, character_id: snapshot.character_id,
        conversation_id: snapshot.conversation_id, conversation_generation: snapshot.generation
      }
    }).then((response) => { window.shutdownFixture.response = response; }, (error) => {
      window.shutdownFixture.error = error;
    });
  }, { id, message, hidden, snapshot: result });
}
async function completed(main) {
  return until(async () => {
    const value = await main.evaluate(() => window.shutdownFixture);
    if (value?.error) throw new Error(JSON.stringify(value.error));
    return value?.response;
  }, "chat response");
}
async function assertSubmissionPending(main) {
  const error = await main.evaluate(() => window.shutdownFixture?.error);
  if (error) throw new Error("chat submission failed: " + JSON.stringify(error));
}
function closeMain(main, repeats = 1) {
  return main.evaluate(async (count) => {
    const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js");
    const window = getCurrentWindow();
    return Promise.allSettled(Array.from({ length: count }, () => window.close()));
  }, repeats);
}
async function waitExit(app) {
  return until(async () => app.running() ? null : app.done, "app process exit", 14500);
}
function lifecycleLog(directory) {
  return ["app.log", "app.stderr.log"]
    .map((file) => fs.readFileSync(path.join(directory, file), "utf8").replace(/\x1b\[[0-9;]*m/g, ""))
    .join("\n")
    .split(/\r?\n/).filter((line) => /background shutdown|application shutdown forced|watchdog deadline/i.test(line));
}
async function caseRun(name, operation) {
  const run = await startApp(name);
  try {
    const evidence = await operation(run);
    report.scenarios.push({ name, status: "passed", pid: run.app.child.pid, ...evidence });
    console.log("PASS", name);
  } catch (error) {
    report.scenarios.push({ name, status: "failed", error: String(error), lifecycle: lifecycleLog(run.directory) });
    throw error;
  } finally {
    if (run.app.running()) {
      await closeMain(run.main).catch(() => {});
      await waitExit(run.app).catch(() => stop(run.app));
    }
    await run.browser.close().catch(() => {});
    await until(async () => !(await reachable(endpoints.cdp + "/json/version")), "old WebView2 profile shutting down");
    saveReport();
  }
}

const vite = launch(process.execPath, ["node_modules/vite/bin/vite.js", "--strictPort"], "vite", output);
const fixture = launch(process.execPath, ["scripts/chat-lifecycle-model-fixture.mjs", output], "model", output);
try {
  await until(() => reachable(endpoints.vite), "Vite");
  await until(() => reachable(endpoints.fixture + "/counts"), "isolated model fixture");
  await caseRun("window-events", async ({ app, context, main, directory }) => {
    await main.evaluate(() => window.__TAURI_INTERNALS__.invoke("show_pet_window"));
    await until(() => context.pages().find((page) => page.url().includes("pet.html")), "pet WebView2");
    const requested = await main.evaluate(async () => {
      const { Window } = await import("/node_modules/@tauri-apps/api/window.js");
      const pet = await Window.getByLabel("pet");
      if (!pet) return false;
      await pet.close();
      return true;
    });
    assert(requested);
    await until(() => !context.pages().find((page) => page.url().includes("pet.html")), "pet closes without quitting");
    assert(app.running());
    await main.evaluate(() => window.__TAURI_INTERNALS__.invoke("show_pet_window"));
    await until(() => context.pages().find((page) => page.url().includes("pet.html")), "pet can reopen");
    const started = performance.now();
    const requests = closeMain(main, 3).catch(() => {});
    const exit = await waitExit(app);
    await requests;
    assert.equal(exit.code, 0);
    const lifecycle = lifecycleLog(directory);
    assert.equal(lifecycle.filter((line) => line.includes("background shutdown complete")).length, 1);
    return { exit, milliseconds: Math.round(performance.now() - started), lifecycle, petClosedAndReopened: true, closeRequests: 3 };
  });
  await caseRun("shutdown-during-generation", async ({ app, main, db, directory }) => {
    await readyCharacter(main);
    await submit(main, "native-stop-generation", "[case:pet-restore]");
    await until(async () => {
      await assertSubmissionPending(main);
      return (await view(main)).turns.find((turn) => turn.client_request_id === "native-stop-generation" && turn.text.includes("Controlled reply"));
    }, "active model stream");
    const turn = (await view(main)).turns.find((item) => item.client_request_id === "native-stop-generation");
    const latestCall = (await counts()).filter((call) => call.name === "pet-restore" && call.streaming).at(-1);
    const started = performance.now();
    void closeMain(main).catch(() => {});
    const exit = await waitExit(app);
    assert.equal(exit.code, 0);
    const cancelled = await until(async () => (await counts()).find((call) => call.index === latestCall.index && call.closedAt), "model transport drop");
    const rows = getRows(db, "SELECT role,content,metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id", turn.conversation_id);
    assert(rows.some((row) => row.role === "user" && row.content === "[case:pet-restore]"));
    assert(!rows.some((row) => row.role === "assistant"));
    const lifecycle = lifecycleLog(directory);
    assert(lifecycle.some((line) => line.includes("background shutdown complete")));
    return { exit, milliseconds: Math.round(performance.now() - started), protectedUserRow: true, assistantRows: 0, modelCompleted: cancelled.completed ?? false, lifecycle };
  });
  for (const forced of [false, true]) {
    const name = forced ? "shutdown-forces-held-save" : "shutdown-drains-held-save";
    await caseRun(name, async ({ app, main, db, directory }) => {
      await readyCharacter(main);
      await submit(main, "seed", "[case:seed]");
      const seed = await completed(main);
      assert.equal(seed.status, "completed");
      const turnCount = (await counts()).length;
      await submit(main, name, "[case:pet-restore]", true);
      await until(async () => {
        await assertSubmissionPending(main);
        return (await counts()).slice(turnCount).some((call) => call.name === "pet-restore" && call.streaming);
      }, "model stream reached controlled pause");
      const lock = await holdSqliteWriter(db, directory);
      try {
        await until(async () => (await fetch(endpoints.fixture + "/release-pet-restore", { method: "POST" })).status === 204, "release first fixture barrier");
        await until(async () => (await fetch(endpoints.fixture + "/finish-pet-restore", { method: "POST" })).status === 204, "finish model fixture");
        await until(() => main.evaluate(() => window.shutdownFixture.complete), "turn reaches assistant finalization");
        // The final SQLite transaction now waits on the independent writer.
        await delay(350);
        const started = performance.now();
        void closeMain(main, 2).catch(() => {});
        if (!forced) {
          await delay(400);
          assert(app.running());
          lock.child.stdin.end("\n");
          await lock.done;
        }
        const exit = await waitExit(app);
        const milliseconds = Math.round(performance.now() - started);
        if (forced) {
          lock.child.stdin.end("\n");
          await lock.done;
        }
        assert.equal(exit.code, forced ? 1 : 0);
        const outputs = getRows(db, "SELECT content FROM conversation_messages WHERE role='assistant' ORDER BY id").map((row) => row.content);
        assert.equal(outputs.length, forced ? 1 : 2);
        assert(outputs[0].includes("seed"));
        if (!forced) assert(outputs[1].includes("pet-restore"));
        assert.equal(getRows(db, "PRAGMA integrity_check")[0].integrity_check, "ok");
        const lifecycle = lifecycleLog(directory);
        assert(lifecycle.some((line) =>
          forced ? line.includes("application shutdown forced") && line.includes("chat/turn") : line.includes("background shutdown complete")
        ));
        return { exit, milliseconds, assistantRows: outputs, sqliteIntegrity: "ok", lifecycle };
      } finally {
        if (lock.running()) {
          lock.child.stdin.end("\n");
          await lock.done;
        }
      }
    });
  }
} finally {
  stop(fixture);
  stop(vite);
  saveReport();
}
