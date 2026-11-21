// Started by the native runner after relaunching the isolated app on the same database.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const output = path.resolve(process.argv[2]);
const read = name => JSON.parse(fs.readFileSync(path.join(output, name), "utf8"));
const run = read("run-info.json");
const native = read("native-report.json");
const fixture = native.checks.find(check => check.evidence?.restartFixture)?.evidence.restartFixture;
const report = { run: run.run, appPid: run.appPid, restartedAppPid: run.restartedAppPid, fixture };
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const db = path.join(run.dataRoot, "com.chyin.kokoro", "kokoro.db");
const digest = text => createHash("sha256").update(text).digest("hex");
const content = id => JSON.parse(execFileSync("python", ["-c", `
import json, sqlite3, sys
with sqlite3.connect(sys.argv[1], timeout=10) as db:
    row = db.execute("SELECT content FROM conversation_messages WHERE id = ?", (int(sys.argv[2]),)).fetchone()
    print(json.dumps(row[0] if row else None))
`, db, String(id)], { encoding: "utf8" }));

let browser;
try {
  assert(fixture);
  assert(Number.isSafeInteger(run.restartedAppPid));
  assert.notEqual(run.appPid, run.restartedAppPid);
  assert.equal(run.dataRoot, process.env.KOKORO_TEST_DATA_ROOT);
  const original = content(fixture.userId);
  assert.equal(Buffer.byteLength(original), fixture.bytes);
  assert.equal(digest(original), fixture.sha256);
  assert.equal(fixture.bytes, 262145);

  browser = await chromium.connectOverCDP("http://127.0.0.1:19333");
  const context = browser.contexts()[0];
  const waitPage = async (predicate) => {
    const deadline = Date.now() + 20000;
    while (Date.now() < deadline) {
      const page = context.pages().find(predicate);
      if (page) return page;
      await new Promise(resolve => setTimeout(resolve, 80));
    }
    throw new Error("Timed out waiting for the restarted app's WebView");
  };
  const pet = await waitPage(page => page.url().includes("pet.html"));
  await pet.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__));
  await pet.evaluate(async characterId => {
    const bridge = await import("/src/lib/kokoro-bridge.ts");
    const snapshot = await bridge.getChatView();
    if (snapshot.character_id !== characterId) {
      await bridge.commitCharacterActivation(await bridge.prepareCharacterActivation(characterId));
    }
    localStorage.setItem("kokoro_active_character_id", characterId);
    localStorage.setItem("kokoro_onboarding_status", "completed");
    const { WebviewWindow } = await import("/node_modules/@tauri-apps/api/webviewWindow.js");
    new WebviewWindow("main", { url: "index.html", title: "Kokoro edit restart check", width: 1200, height: 850 });
  }, native.setup.characterId);
  const main = await waitPage(page => page.url().endsWith("/index.html") || page.url().endsWith(":1420/"));
  await main.locator("textarea[data-onboarding-id='chat-input']").waitFor({ state: "visible" });
  await main.locator("[data-chat-history-toggle]").click();
  await main.getByText(fixture.title, { exact: true }).click();
  const user = main.locator(`[data-chat-message-id="${fixture.userId}"]`);
  const bubble = user.locator("div.whitespace-pre-wrap");
  assert(await bubble.textContent() === original, "restarted app must display the complete SQLite row");
  await user.getByRole("button", { name: /^(编辑|Edit)$/ }).click();
  const editor = user.locator("textarea");
  assert(await editor.inputValue() === original, "restarted app must load the complete editor draft");
  const selected = await main.evaluate(() => window.__TAURI_INTERNALS__.invoke("get_chat_view"));
  assert.equal(selected.conversation_id, fixture.conversationId);
  assert(selected.messages.find(row => row.id === fixture.userId)?.content === original);

  const shortened = "[case:edit-shortened-restart] 重启后缩短并重放🙂";
  const watchIpc = command => main.waitForResponse(response => response.url().endsWith(`/${command}`) && response.request().method() === "POST");
  await editor.fill(shortened);
  const pendingSave = watchIpc("edit_conversation_message");
  await user.getByRole("button", { name: /^(保存|Save) \(Ctrl\+Enter\)$/ }).click();
  const save = await pendingSave;
  assert.equal(save.headers()["tauri-response"], "ok");
  assert.deepEqual(await save.json(), { message_id: fixture.userId, updated_content: shortened });
  await editor.waitFor({ state: "detached" });
  assert.equal(content(fixture.userId), shortened);

  const pendingReplay = watchIpc("stream_chat");
  await main.locator(`[data-chat-message-id="${fixture.assistantId}"]`).getByRole("button", { name: /^(重新生成|Regenerate)$/ }).click();
  const replay = await pendingReplay;
  assert.equal(replay.headers()["tauri-response"], "ok");
  const replayRequest = replay.request().postDataJSON().request;
  assert.equal(replayRequest.message, shortened);
  assert.equal(replayRequest.regenerate_reply.assistant_message_id, fixture.assistantId);
  const result = await replay.json();
  assert.equal(result.status, "completed");
  assert.equal(result.conversation_id, fixture.conversationId);
  assert.equal(result.user_message_id, fixture.userId);
  assert.notEqual(result.assistant_message_id, fixture.assistantId);
  assert.equal(content(fixture.userId), shortened);
  const calls = (await (await fetch("http://127.0.0.1:19334/counts")).json()).filter(call => call.name === "edit-shortened-restart" && call.streaming);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].messages.findLast(message => message.role === "user")?.content, shortened);
  Object.assign(report, { status: "passed", reopenedBytes: Buffer.byteLength(original), reopenedSha256: digest(original),
    shortenedBytes: Buffer.byteLength(shortened), modelCall: calls[0].index, assistantMessageId: result.assistant_message_id });
  console.log("PASS native app restart preserves a 262145-byte user history row through display, edit, and replay");
} catch (error) {
  Object.assign(report, { status: "failed", error: error.stack ?? String(error) });
  throw error;
} finally {
  fs.writeFileSync(path.join(output, "edit-restart-report.json"), JSON.stringify(report, null, 2));
  await browser?.close();
}
