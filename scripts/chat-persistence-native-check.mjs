import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE);
const output = path.resolve(process.argv[2] ?? "output/playwright/chat-lifecycle/persistence");
fs.mkdirSync(output, { recursive:true });
const db = path.join(process.env.KOKORO_TEST_DATA_ROOT, "com.chyin.kokoro", "kokoro.db");
const browser = await chromium.connectOverCDP("http://127.0.0.1:19333");
const report = [];
async function waitFor(predicate) {
  const end = Date.now() + 12000;
  do {
    const value = await Promise.resolve().then(predicate).catch(() => null);
    if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 80));
  } while (Date.now() < end);
  throw new Error("fixture condition timed out");
}
function dbScript(sql) {
  execFileSync("python", ["-c", "import sqlite3,sys\nwith sqlite3.connect(sys.argv[1], timeout=10) as db:\n db.executescript(sys.argv[2])", db, sql]);
}
function rows(sql, ...params) {
  return JSON.parse(execFileSync("python", ["-c", "import json,sqlite3,sys\nwith sqlite3.connect(sys.argv[1], timeout=10) as db:\n db.row_factory = sqlite3.Row\n print(json.dumps([dict(row) for row in db.execute(sys.argv[2], json.loads(sys.argv[3]))],ensure_ascii=True))", db, sql, JSON.stringify(params)], { encoding: "utf8" }));
}
const modelCalls = async name => (await (await fetch("http://127.0.0.1:19334/counts")).json()).filter(call => call.name === name && call.streaming);
const character = "chat-persistence-validation-character";
try {
  const page = await waitFor(() => browser.contexts()[0].pages().find(p => p.url().includes("pet.html")));
  await page.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__));
  await page.evaluate(async id => {
    const bridge = await import("/src/lib/kokoro-bridge.ts");
    const templates = await bridge.listCharacterTemplates();
    for (const t of templates) {
      await bridge.instantiateCharacterTemplate({ template_id: t.id, template_version: t.version, instance_id: t.id, user_nickname: "Tester", created_at: 1, updated_at: 1 });
    }
    localStorage.setItem("kokoro_provisioned_character_template_ids_v2", JSON.stringify(templates.map(t => t.id)));
    await bridge.createCharacter({ id, name: "Persistence Validation", persona: "Isolated persistence fixture", user_nickname: "Tester", source_format: "manual", created_at: 1, updated_at: 1, runtime_profile_json: JSON.stringify({ proactive_enabled: false }) });
    await bridge.commitCharacterActivation(await bridge.prepareCharacterActivation(id));
    localStorage.setItem("kokoro_active_character_id", id);
    window.persistenceFinishes = [];
    await bridge.onChatTurnFinish(payload => window.persistenceFinishes.push(payload));
    const config = await bridge.getBotConfig();
    config.webhook = { ...config.webhook, enabled: true, bind_host: "127.0.0.1", port: 19335, bearer_token: "fixture-chat-token", character_id: id };
    await bridge.saveBotConfig(config);
    await bridge.startBotPlatform("webhook");
  }, character);
  const invoke = (name, params) => page.evaluate(({ name, params }) => window.__TAURI_INTERNALS__.invoke(name, params), { name, params });
  const view = () => invoke("get_chat_view");
  const submit = (id, message, extra = {}) => page.evaluate(async request => {
    try { return { result: await window.__TAURI_INTERNALS__.invoke("stream_chat", { request }) }; }
    catch (error) { return { error }; }
  }, { client_request_id:id, message, character_id:character, ...extra });
  const retry = id => page.evaluate(async clientRequestId => {
    try { return { result:await window.__TAURI_INTERNALS__.invoke("retry_chat_persistence", { clientRequestId }) }; }
    catch (error) { return { error }; }
  }, id);
  async function check(name, fn) {
    try { report.push({ name, status:"passed", evidence:await fn() }); console.log("PASS", name); }
    catch (error) { report.push({ name, status:"failed", error:error.stack }); console.error("FAIL", name, error.stack); }
    fs.writeFileSync(path.join(output, "persistence-report.json"), JSON.stringify(report, null, 2));
  }
  const conversationCount = () => rows("SELECT id FROM conversations WHERE character_id = ?", character).length;
  async function expectDesktopFailure(id, response) {
    assert.equal(response.result, undefined, JSON.stringify(response));
    assert.equal(response.error?.code, "ChatPersistence", JSON.stringify(response));
    const turn = (await view()).turns.find(turn => turn.client_request_id === id);
    assert.equal(turn.status, "error", JSON.stringify(turn));
    assert.equal(turn.error_code, "CHAT_PERSISTENCE_ERROR", JSON.stringify(turn));
    const events = await page.evaluate(id => window.persistenceFinishes.filter(event => event.client_request_id === id), id);
    assert(!events.some(event => event.status === "completed"));
    return { response, turn, events };
  }
  const databaseMessage = text => rows("SELECT id, conversation_id, role, metadata, content FROM conversation_messages WHERE content = ?", text);
  await waitFor(async () => (await fetch("http://127.0.0.1:19335/webhook/message", { method:"POST", body:"{}" })).status === 401);
  await check("desktop user INSERT fails before model and rolls back new conversation", async () => {
    const id = "persistence_user_insert", text = "[case:db-user-insert]";
    const before = conversationCount(), selected = (await view()).conversation_id;
    dbScript("CREATE TRIGGER reject_user BEFORE INSERT ON conversation_messages WHEN NEW.role = 'user' AND NEW.content = '[case:db-user-insert]' BEGIN SELECT RAISE(ABORT,'fixture user insert'); END;");
    let response;
    try { response = await submit(id, text); }
    finally { dbScript("DROP TRIGGER reject_user;"); }
    assert.equal(databaseMessage(text).length, 0);
    assert.deepEqual(await modelCalls("db-user-insert"), []);
    assert.equal((await view()).conversation_id, selected);
    assert.equal(conversationCount(), before);
    const failure = await expectDesktopFailure(id, response);
    assert.equal(failure.turn.user_save_failed, true);
    assert.equal(await invoke("recover_chat_draft_submission", { characterId:character, clientRequestId:id }), "unsent");
    return failure;
  });
  await check("desktop SQLite COMMIT rolls back conversation and user message on deferred FK error", async () => {
    const id = "persistence_commit", text = "[case:db-commit]";
    const before = conversationCount();
    dbScript("CREATE TABLE fixture_parent(id INTEGER PRIMARY KEY); CREATE TABLE fixture_child(parent_id INTEGER REFERENCES fixture_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER reject_commit AFTER INSERT ON conversation_messages WHEN NEW.role = 'user' AND NEW.content = '[case:db-commit]' BEGIN INSERT INTO fixture_child VALUES (1234); END;");
    let response, child;
    try { response = await submit(id, text); child = rows("SELECT * FROM fixture_child"); }
    finally { dbScript("DROP TRIGGER reject_commit; DROP TABLE fixture_child; DROP TABLE fixture_parent;"); }
    assert.equal(databaseMessage(text).length, 0);
    assert.deepEqual(await modelCalls("db-commit"), []);
    assert.deepEqual(child, []);
    assert.equal(conversationCount(), before);
    return expectDesktopFailure(id, response);
  });
  await check("desktop assistant finalization failure leaves any partial row marked as draft", async () => {
    const id = "persistence_final", text = "[case:db-final]";
    const finalRow = "NEW.role = 'assistant' AND instr(NEW.content, '[case:db-final]') > 0 AND coalesce(json_extract(NEW.metadata, '$.streaming_draft'),0) != 1";
    dbScript("CREATE TRIGGER reject_assistant_insert BEFORE INSERT ON conversation_messages WHEN " + finalRow + " BEGIN SELECT RAISE(ABORT,'fixture final assistant insert'); END; CREATE TRIGGER reject_assistant_update BEFORE UPDATE ON conversation_messages WHEN " + finalRow + " BEGIN SELECT RAISE(ABORT,'fixture final assistant update'); END;");
    let response, repeat;
    try { response = await submit(id, text); repeat = await retry(id); }
    finally { dbScript("DROP TRIGGER reject_assistant_insert; DROP TRIGGER reject_assistant_update;"); }
    assert.equal((await modelCalls("db-final")).length, 1);
    const user = databaseMessage(text);
    assert.equal(user.length, 1);
    const assistants = rows("SELECT id, content, metadata FROM conversation_messages WHERE role = 'assistant' AND instr(content, ?) > 0", text);
    assert.equal(assistants.length, 1, "fixture must exercise an existing streaming draft");
    assert(assistants.every(row => JSON.parse(row.metadata).streaming_draft === true));
    const failure = await expectDesktopFailure(id, response);
    assert.equal(repeat.error?.code, "ChatPersistence", JSON.stringify(repeat));
    assert.equal(failure.turn.persistence_retryable, true);
    const [a, b] = await Promise.all([retry(id), retry(id)]);
    assert.equal(a.result?.status, "completed", JSON.stringify(a));
    assert.deepEqual(a, b);
    assert.equal(a.result.user_message_id, user[0].id);
    assert.equal(a.result.assistant_message_id, assistants[0].id);
    assert.equal((await modelCalls("db-final")).length, 1);
    const finalized = rows("SELECT id, metadata FROM conversation_messages WHERE role = 'assistant' AND instr(content, ?) > 0", text);
    assert.deepEqual(finalized.map(row => row.id), [assistants[0].id]);
    assert.notEqual(JSON.parse(finalized[0].metadata).streaming_draft, true);
    const completions = await waitFor(async () => {
      const events = await page.evaluate(id => window.persistenceFinishes.filter(event => event.client_request_id === id && event.status === "completed"), id);
      return events.length ? events : null;
    });
    assert.equal(completions.length, 1);
    assert.equal(completions[0].assistant_message_id, finalized[0].id);
    return { ...failure, user, assistants, repeat, recovered:a, finalized };
  });
  await check("desktop hidden assistant INSERT fails, then retries to its original conversation", async () => {
    const id = "persistence_hidden_final", text = "[case:db-hidden-insert]";
    const before = (await view()).conversation_id;
    dbScript("CREATE TRIGGER reject_hidden_assistant BEFORE INSERT ON conversation_messages WHEN NEW.role = 'assistant' AND instr(NEW.content, '[case:db-hidden-insert]') > 0 BEGIN SELECT RAISE(ABORT,'fixture hidden assistant insert'); END;");
    let response;
    try { response = await submit(id, text, { hidden:true }); }
    finally { dbScript("DROP TRIGGER reject_hidden_assistant;"); }
    assert.equal((await modelCalls("db-hidden-insert")).length, 1);
    assert.equal((await view()).conversation_id, before);
    assert.equal(databaseMessage(text).length, 0);
    assert.equal(rows("SELECT id FROM conversation_messages WHERE role = 'assistant' AND instr(content,?) > 0", text).length, 0);
    const failure = await expectDesktopFailure(id, response);
    const recovered = await retry(id);
    assert.equal(recovered.result?.status, "completed", JSON.stringify(recovered));
    assert.equal(recovered.result.user_message_id, null);
    assert.equal(recovered.result.conversation_id, before);
    assert.equal((await modelCalls("db-hidden-insert")).length, 1);
    return { failure, recovered };
  });
  await check("desktop final save retry commits tool exchange and reply without another model round", async () => {
    const id = "persistence_tool_retry", text = "[case:db-tool-save]";
    dbScript("CREATE TRIGGER reject_tool_final BEFORE INSERT ON conversation_messages WHEN NEW.role = 'assistant' AND instr(NEW.content, '[case:db-tool-save]') > 0 AND coalesce(json_extract(NEW.metadata, '$.streaming_draft'),0) != 1 BEGIN SELECT RAISE(ABORT,'fixture tool reply write'); END;");
    let failure;
    try {
      failure = await expectDesktopFailure(id, await submit(id,text));
      assert.equal(failure.turn.tools.length, 1);
      assert.equal(failure.turn.persistence_retryable, true);
      assert.equal(rows("SELECT id FROM conversation_messages WHERE role = 'tool' AND json_extract(metadata, '$.turn_id') = ?", failure.turn.turn_id).length, 0);
    } finally { dbScript("DROP TRIGGER reject_tool_final;"); }
    const response = await retry(id);
    assert.equal(response.result?.status, "completed", JSON.stringify(response));
    const calls = await modelCalls("db-tool-save");
    assert.equal(calls.length, 2);
    assert.deepEqual(calls.map(call => call.toolResult), [false,true]);
    const [tool] = rows("SELECT id, conversation_id FROM conversation_messages WHERE role = 'tool' AND json_extract(metadata, '$.turn_id') = ?", failure.turn.turn_id);
    assert(tool.id < response.result.assistant_message_id);
    assert.equal(tool.conversation_id, response.result.conversation_id);
    return { failure, response, tool };
  });
  await check("desktop existing conversation metadata error rolls back user write", async () => {
    const id = "persistence_metadata", text = "[case:db-meta]";
    const cid = (await view()).conversation_id;
    const metadata = () => rows("SELECT title, updated_at, message_revision FROM conversations WHERE id = ?", cid)[0];
    const before = metadata();
    assert(before);
    dbScript("CREATE TRIGGER reject_chat_metadata BEFORE UPDATE OF updated_at ON conversations WHEN EXISTS (SELECT 1 FROM conversation_messages WHERE conversation_id = NEW.id AND content = '[case:db-meta]') BEGIN SELECT RAISE(ABORT,'fixture metadata update'); END;");
    let response;
    try { response = await submit(id, text); }
    finally { dbScript("DROP TRIGGER reject_chat_metadata;"); }
    assert.equal(databaseMessage(text).length, 0);
    assert.deepEqual(await modelCalls("db-meta"), []);
    assert.deepEqual(metadata(), before);
    return expectDesktopFailure(id, response);
  });
  await check("desktop initial save retry reuses its request and generates only after user commit", async () => {
    const id = "persistence_initial_retry", text = "[case:db-initial-retry]";
    dbScript("CREATE TRIGGER reject_initial_retry BEFORE INSERT ON conversation_messages WHEN NEW.role = 'user' AND NEW.content = '[case:db-initial-retry]' BEGIN SELECT RAISE(ABORT,'fixture initial write error'); END;");
    let failure;
    try {
      failure = await expectDesktopFailure(id, await submit(id, text));
      assert.equal(failure.turn.user_save_failed, true);
      assert.equal(databaseMessage(text).length, 0);
      assert.deepEqual(await modelCalls("db-initial-retry"), []);
    } finally { dbScript("DROP TRIGGER reject_initial_retry;"); }
    const [a,b] = await Promise.all([retry(id), retry(id)]);
    assert.equal(a.result?.status, "completed", JSON.stringify(a));
    assert.deepEqual(a,b);
    const [prompt] = databaseMessage(text);
    assert.equal(prompt.id, a.result.user_message_id);
    assert.equal(JSON.parse(prompt.metadata).client_request_id, id);
    assert.equal((await modelCalls("db-initial-retry")).length, 1);
    return { failure, result:a.result };
  });
  await check("same-ID user retry remains cancellable after the first worker exits", async () => {
    const id = "persistence_cancel_retry", text = "[case:db-cancel-retry-slow]";
    dbScript("CREATE TRIGGER reject_cancel_retry BEFORE INSERT ON conversation_messages WHEN NEW.role = 'user' AND NEW.content = '[case:db-cancel-retry-slow]' BEGIN SELECT RAISE(ABORT,'fixture initial write error'); END;");
    try { await expectDesktopFailure(id, await submit(id,text)); }
    finally { dbScript("DROP TRIGGER reject_cancel_retry;"); }
    const pending = retry(id);
    await waitFor(async () => (await modelCalls("db-cancel-retry-slow")).length === 1);
    await invoke("cancel_chat_turn", { turnId:id, reason:"stopped_by_user" });
    const cancelled = await pending;
    assert.equal(cancelled.result?.status, "cancelled", JSON.stringify(cancelled));
    assert.equal(databaseMessage(text).length, 1);
    const events = await page.evaluate(id => window.persistenceFinishes.filter(event => event.client_request_id === id), id);
    assert(!events.some(event => event.status === "completed"));
    return { cancelled, events };
  });
  await check("duplicate desktop delivery reuses row IDs and keeps original full prompt", async () => {
    const id = "persistence_duplicate";
    const text = "  [case:db-duplicate] " + "界".repeat(2200) + " full-width tail  ";
    const state = await view();
    const extra = { conversation_id:state.conversation_id, conversation_generation:state.generation };
    const [a,b] = await Promise.all([submit(id, text, extra), submit(id, text, extra)]);
    assert.equal(a.result?.status, "completed", JSON.stringify(a));
    assert.deepEqual(a, b);
    assert.equal((await modelCalls("db-duplicate")).length, 1);
    assert.deepEqual(databaseMessage(text).map(row => row.id), [a.result.user_message_id]);
    assert.equal(rows("SELECT role FROM conversation_messages WHERE id = ?", a.result.assistant_message_id)[0]?.role, "assistant");
    return { userId:a.result.user_message_id, assistantId:a.result.assistant_message_id, promptChars:text.length };
  });
  await check("desktop oversized input is rejected before any insert or generation", async () => {
    const id = "persistence_size_limit";
    const request = "[case:db-oversized] " + "界".repeat(90_000);
    const count = rows("SELECT COUNT(*) AS count FROM conversation_messages")[0].count;
    const response = await submit(id, request);
    assert.equal(response.error?.code, "Validation", JSON.stringify(response));
    assert.deepEqual(await modelCalls("db-oversized"), []);
    assert.equal(rows("SELECT COUNT(*) AS count FROM conversation_messages")[0].count, count);
    const events = await page.evaluate(id => window.persistenceFinishes.filter(event => event.client_request_id === id), id);
    assert(!events.some(event => event.status === "completed"));
    return { bytes:Buffer.byteLength(request), response };
  });
  await check("desktop and webhook retain full assistant text independently of channel formatting", async () => {
    const text = "[case:db-canonical-output]";
    const canonical = "**Formatted answer**\n\n" + "界".repeat(2300) + "\n\nComplete tail.";
    const desktop = await submit("persistence_canonical_output", text);
    assert.equal(desktop.result?.status, "completed", JSON.stringify(desktop));
    assert.equal(rows("SELECT content FROM conversation_messages WHERE id = ?", desktop.result.assistant_message_id)[0]?.content, canonical);
    const remote = await webhook(text, "persistence_canonical_output");
    assert.equal(remote.status, 200, JSON.stringify(remote));
    assert(remote.body.reply.endsWith("Complete tail."));
    const saved = databaseMessage(canonical).filter(row => row.role === "assistant");
    assert.equal(saved.length, 2);
    assert.notEqual(saved[0].conversation_id, saved[1].conversation_id);
    return { desktopId:desktop.result.assistant_message_id, savedIds:saved.map(row => row.id), chars:canonical.length };
  });
  await check("a model failure remains distinct from a storage failure", async () => {
    const id = "persistence_model_error", text = "[case:provider-failure]";
    const response = await submit(id, text);
    assert.equal(response.result, undefined, JSON.stringify(response));
    assert(response.error && response.error.code !== "ChatPersistence");
    assert.equal(databaseMessage(text).length, 1);
    assert.equal((await modelCalls("provider-failure")).length, 1);
    const turn = (await view()).turns.find(turn => turn.client_request_id === id);
    assert.equal(turn.status, "error");
    assert.notEqual(turn.error_code, "CHAT_PERSISTENCE_ERROR");
    assert(!turn.persistence_retryable);
    return { response, turn };
  });
  async function webhook(text, requestId) {
    const response = await fetch("http://127.0.0.1:19335/webhook/message", {
      method:"POST", headers:{ Authorization:"Bearer fixture-chat-token", "Content-Type":"application/json" },
      body:JSON.stringify({ text, request_id:requestId, character_id:character, conversation_id:"persistence-fixture", source:"fixture" }),
    });
    return { status:response.status, body:await response.json() };
  }
  await check("webhook user INSERT error is typed and skips model", async () => {
    const text = "[case:db-webhook-user]", before = conversationCount();
    dbScript("CREATE TRIGGER reject_webhook_user BEFORE INSERT ON conversation_messages WHEN NEW.role = 'user' AND NEW.content = '[case:db-webhook-user]' BEGIN SELECT RAISE(ABORT,'fixture webhook user insert'); END;");
    let response;
    try { response = await webhook(text); }
    finally { dbScript("DROP TRIGGER reject_webhook_user;"); }
    assert.deepEqual(await modelCalls("db-webhook-user"), []);
    assert.equal(databaseMessage(text).length, 0);
    assert.equal(conversationCount(), before);
    assert(response.status >= 400 && !response.body.reply, JSON.stringify(response));
    assert.equal(response.body.code, "CHAT_PERSISTENCE_ERROR");
    return response;
  });
  await check("webhook assistant INSERT error is typed and leaves only saved user", async () => {
    const text = "[case:db-webhook-final]";
    dbScript("CREATE TRIGGER reject_webhook_assistant BEFORE INSERT ON conversation_messages WHEN NEW.role = 'assistant' AND instr(NEW.content,'[case:db-webhook-final]') > 0 BEGIN SELECT RAISE(ABORT,'fixture webhook assistant insert'); END;");
    let response;
    try {
      response = await webhook(text, "persistence_webhook_final");
      const repeat = await webhook(text, "persistence_webhook_final");
      assert.equal(repeat.body.code, "CHAT_PERSISTENCE_ERROR", JSON.stringify(repeat));
    }
    finally { dbScript("DROP TRIGGER reject_webhook_assistant;"); }
    assert.equal((await modelCalls("db-webhook-final")).length, 1);
    assert.equal(databaseMessage(text).length, 1);
    assert.equal(rows("SELECT id FROM conversation_messages WHERE role = 'assistant' AND instr(content,?) > 0", text).length, 0);
    assert(response.status >= 400 && !response.body.reply, JSON.stringify(response));
    assert.equal(response.body.code, "CHAT_PERSISTENCE_ERROR");
    const recovered = await webhook(text, "persistence_webhook_final");
    assert.equal(recovered.status, 200, JSON.stringify(recovered));
    assert.deepEqual(await webhook(text, "persistence_webhook_final"), recovered);
    assert.equal((await modelCalls("db-webhook-final")).length, 1);
    assert.equal(databaseMessage(text).length, 1);
    const [reply] = databaseMessage(recovered.body.reply);
    assert.equal(reply.conversation_id, databaseMessage(text)[0].conversation_id);
    return { response, recovered, reply };
  });
  await check("switching desktop selection during webhook generation does not move either saved message", async () => {
    const text = "[case:db-webhook-switch-slow]";
    const request = webhook(text);
    await waitFor(async () => (await modelCalls("db-webhook-switch-slow")).length === 1);
    await invoke("clear_history");
    assert.equal((await view()).conversation_id, null);
    const response = await request;
    assert.equal(response.status, 200, JSON.stringify(response));
    const [prompt] = databaseMessage(text);
    const [reply] = databaseMessage(response.body.reply);
    assert.equal(prompt.role, "user");
    assert.equal(reply.role, "assistant");
    assert.equal(reply.conversation_id, prompt.conversation_id);
    assert.equal(rows("SELECT character_id FROM conversations WHERE id = ?", reply.conversation_id)[0].character_id, character);
    assert.equal((await view()).conversation_id, null);
    return { prompt, reply };
  });
  await check("hidden retry cannot create a conversation after its empty selection has changed", async () => {
    const id = "persistence_hidden_stale", text = "[case:db-hidden-stale]";
    assert.equal((await view()).conversation_id, null);
    const count = conversationCount();
    dbScript("CREATE TRIGGER reject_hidden_new BEFORE INSERT ON conversation_messages WHEN NEW.role = 'assistant' AND instr(NEW.content, '[case:db-hidden-stale]') > 0 BEGIN SELECT RAISE(ABORT,'fixture hidden new conversation write'); END;");
    let failure;
    try { failure = await expectDesktopFailure(id, await submit(id, text, { hidden:true })); }
    finally { dbScript("DROP TRIGGER reject_hidden_new;"); }
    assert.equal(conversationCount(), count);
    await invoke("clear_history");
    const stale = await retry(id);
    assert.equal(stale.error?.code, "Validation", JSON.stringify(stale));
    assert.equal(conversationCount(), count);
    assert.equal((await view()).conversation_id, null);
    assert.equal((await modelCalls("db-hidden-stale")).length, 1);
    return { failure, stale };
  });
  await check("desktop synthesis failure does not change committed assistant result", async () => {
    const id = "persistence_failed_speech", message = "[case:db-tts-failure]";
    await page.evaluate(async () => {
      const bridge = await import("/src/lib/kokoro-bridge.ts");
      const config = await bridge.getTtsConfig();
      const id = (await bridge.getCommittedCharacterRuntime())?.runtime.tts.provider_id;
      if (!id) throw new Error("fixture requires an automatic speech provider");
      await bridge.saveTtsConfig({
        ...config,
        providers:[...config.providers.filter(provider => provider.id !== id), {
          id, provider_type:"openai", enabled:true, api_key:"controlled-fixture",
          base_url:"http://127.0.0.1:19334/v1", model:"tts-1", extra:{},
        }],
      });
    });
    const response = await submit(id, message);
    assert.equal(response.result?.status, "completed", JSON.stringify(response));
    const turn = await waitFor(async () => (await view()).turns.find(t => t.client_request_id === id && t.speech_error));
    assert.equal(turn.status, "completed");
    assert.equal(turn.error_code, undefined);
    assert.equal(turn.assistant_message_id, response.result.assistant_message_id);
    assert.equal(rows("SELECT content FROM conversation_messages WHERE id = ?", turn.assistant_message_id)[0]?.content, response.result.text);
    return { response, turn };
  });
} finally { await browser.close(); }
if (report.some(check => check.status === "failed")) process.exitCode = 1;
