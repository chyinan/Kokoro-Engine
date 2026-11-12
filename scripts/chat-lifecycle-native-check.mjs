// Real Tauri/WebView2 integration with a controlled local SSE model, never a real provider.
// Start the isolated debug app + scripts/chat-lifecycle-model-fixture.mjs first.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const output = path.resolve(process.argv[2] ?? "output/playwright/chat-lifecycle");
const browser = await chromium.connectOverCDP("http://127.0.0.1:19333");
const context = browser.contexts()[0];
const pet = await waitFor(() => context.pages().find(page => page.url().includes("pet.html")));
const report = { startedAt: new Date().toISOString(), model: "controlled localhost SSE fixture", checks: [] };
const save = () => fs.writeFileSync(path.join(output, "native-report.json"), JSON.stringify(report, null, 2));
const view = page => page.evaluate(() => window.__TAURI_INTERNALS__.invoke("get_chat_view"));
const windows = page => page.evaluate(async () => (await (await import("/node_modules/@tauri-apps/api/webviewWindow.js")).getAllWebviewWindows()).map(window => window.label));
async function waitFor(test, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { const value = await test(); if (value) return value; await new Promise(resolve => setTimeout(resolve, 80)); }
  throw new Error("native assertion timed out");
}
async function check(name, operation) {
  const started = Date.now();
  try { const evidence = await operation(); report.checks.push({ name, status: "passed", milliseconds: Date.now() - started, evidence }); save(); console.log(`PASS ${name}`); }
  catch (error) { report.checks.push({ name, status: "failed", error: String(error) }); save(); throw error; }
}
async function submit(page, request, key) {
  await page.evaluate(async ({ request, key }) => {
    const { chatApplication } = await import("/src/lib/chat/application.ts");
    const { getKokoroErrorMessage } = await import("/src/lib/kokoro-bridge.ts");
    window[key] = { pending: true };
    chatApplication.send(request).then(result => { window[key] = { result }; }, error => { window[key] = { error: getKokoroErrorMessage(error), rawError: error }; });
  }, { request, key });
}
const outcome = (page, key) => waitFor(() => page.evaluate(key => window[key]?.pending ? null : window[key], key));
async function petSend(name) {
  await pet.evaluate(async () => { const event = await import("/node_modules/@tauri-apps/api/event.js"); await event.emitTo("pet", "toggle-chat-input"); });
  const input = pet.getByPlaceholder("说点什么..."); await input.waitFor({ state: "visible" });
  await input.fill(`[case:${name}]`); await input.press("Enter");
  return waitFor(async () => { const state = await view(pet); return state.turns.find(turn => turn.message === `[case:${name}]` && ["completed", "cancelled", "error", "rejected"].includes(turn.status)); });
}
try {
  await pet.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__));
  // Model an already configured installation without ever creating the main
  // WebView. Serialized setup avoids first-run StrictMode template provisioning
  // races and gives both views the same committed character from the outset.
  report.setup = await pet.evaluate(async () => {
    const bridge = await import("/src/lib/kokoro-bridge.ts");
    const templates = await bridge.listCharacterTemplates();
    for (const template of templates) await bridge.instantiateCharacterTemplate({ template_id: template.id, template_version: template.version, instance_id: template.id, user_nickname: "Tester", created_at: 1, updated_at: 1 });
    localStorage.setItem("kokoro_provisioned_character_template_ids_v2", JSON.stringify(templates.map(template => template.id)));
    const id = "chat-validation-character";
    await bridge.createCharacter({ id, name: "Chat Validation", persona: "Controlled integration test character", user_nickname: "Tester", source_format: "manual", created_at: 1, updated_at: 1, runtime_profile_json: JSON.stringify({ proactive_enabled: false }) });
    const committed = await bridge.commitCharacterActivation(await bridge.prepareCharacterActivation(id));
    localStorage.setItem("kokoro_active_character_id", id);
    return { characterId: id, templateIds: templates.map(template => template.id), committed };
  }); save();
  await check("pet UI sends before any main WebView or ChatPanel has existed", async () => {
    assert(!(await windows(pet)).includes("main"));
    const turn = await petSend("pet-only"); assert.equal(turn.status, "completed");
    const state = await view(pet); assert(state.messages.some(message => message.role === "assistant" && message.content.includes("pet-only")));
    assert(!(await windows(pet)).includes("main"));
    await pet.screenshot({ path: path.join(output, "01-pet-only.png") });
    return { windows: await windows(pet), turn, persistedMessageIds: state.messages.map(message => message.id) };
  });
  await pet.evaluate(async () => {
    localStorage.setItem("kokoro_onboarding_status", "completed");
    const { WebviewWindow } = await import("/node_modules/@tauri-apps/api/webviewWindow.js");
    new WebviewWindow("main", { url: "index.html", title: "Kokoro Chat Validation Main", width: 1200, height: 850 });
  });
  const main = await waitFor(() => context.pages().find(page => !page.url().includes("pet.html") && !page.url().includes("bubble.html") && page.url().includes("1420")));
  await main.locator("textarea[data-onboarding-id='chat-input']").waitFor({ state: "visible", timeout: 20000 });
  for (const page of [main, pet]) await page.evaluate(async () => {
    window.__nativeTts = [];
    const { listen } = await import("/node_modules/@tauri-apps/api/event.js");
    const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js");
    for (const event of ["tts:start", "tts:end"]) await listen(event, ({ payload }) => window.__nativeTts.push({ event, payload }), { target: getCurrentWindow().label });
  });
  await check("main restores the pet's completed history without another model call", async () => {
    await main.getByText("Controlled reply for [case:pet-only] completed.", { exact: false }).first().waitFor();
    const state = await view(main); assert.equal(state.busy, false);
    await main.screenshot({ path: path.join(output, "02-main-restored.png") });
    return { messageIds: state.messages.map(message => message.id) };
  });
  await check("generating turn survives ChatPanel replacement and remount", async () => {
    const input = main.locator("textarea[data-onboarding-id='chat-input']");
    await input.fill("[case:remount-slow]"); await input.press("Enter");
    await waitFor(async () => (await view(pet)).turns.some(turn => turn.message.includes("remount-slow") && turn.text.length > 0));
    await main.evaluate(async () => {
      const { registry } = await import("/src/ui/registry/ComponentRegistry.ts");
      window.__originalChatPanel = registry.get("ChatPanel"); registry.register("ChatPanel", () => null);
    });
    await waitFor(async () => await main.locator("textarea[data-onboarding-id='chat-input']").count() === 0);
    const detached = await view(pet); assert(detached.busy);
    await main.evaluate(async () => { const { registry } = await import("/src/ui/registry/ComponentRegistry.ts"); registry.register("ChatPanel", window.__originalChatPanel); });
    await main.locator("textarea[data-onboarding-id='chat-input']").waitFor();
    const recovered = await view(main); assert(recovered.turns.some(turn => turn.message.includes("remount-slow") && turn.text.length > 0));
    const terminal = await waitFor(async () => { const state = await view(pet); return !state.busy && state.turns.find(turn => turn.message.includes("remount-slow") && turn.status === "completed"); });
    await main.screenshot({ path: path.join(output, "03-remounted.png") }); return { detachedBusy: detached.busy, terminal };
  });
  await check("simultaneous main/pet intents retain atomic backend rejection", async () => {
    await Promise.all([
      submit(main, { message: "[case:concurrent-slow]", client_request_id: "native_concurrent_main" }, "__concurrent"),
      submit(pet, { message: "[case:concurrent-slow]", client_request_id: "native_concurrent_pet" }, "__concurrent"),
    ]);
    const results = await Promise.all([outcome(main, "__concurrent"), outcome(pet, "__concurrent")]);
    report.concurrentResults = results; save();
    assert.equal(results.filter(result => result.result?.status === "completed").length, 1);
    assert.equal(results.filter(result => result.error?.includes("chat_turn_busy")).length, 1);
    return results;
  });
  await check("same request across two WebViews executes once and returns the same result", async () => {
    const request = { message: "[case:duplicate-slow]", client_request_id: "native_duplicate" };
    await Promise.all([submit(main, request, "__duplicate"), submit(pet, request, "__duplicate")]);
    const results = await Promise.all([outcome(main, "__duplicate"), outcome(pet, "__duplicate")]);
    assert.deepEqual(results[0], results[1]); assert.equal(results[0].result.status, "completed"); return results;
  });
  await check("cancellation before admission uses logical request id and leaves no busy state", async () => {
    await pet.evaluate(async () => { const { chatApplication } = await import("/src/lib/chat/application.ts"); await chatApplication.cancel("native_before_cancel"); });
    await submit(pet, { message: "[case:before-cancel]", client_request_id: "native_before_cancel" }, "__cancelBefore");
    const result = await outcome(pet, "__cancelBefore"); assert(result.error?.includes("cancelled") || result.result?.status === "cancelled");
    assert.equal((await view(pet)).busy, false); return result;
  });
  await check("stream cancellation commits a terminal outcome and the next request succeeds", async () => {
    await submit(pet, { message: "[case:stream-cancel]", client_request_id: "native_stream_cancel" }, "__cancelStream");
    await waitFor(async () => (await view(pet)).turns.some(turn => turn.client_request_id === "native_stream_cancel" && turn.text.length > 0));
    await pet.evaluate(async () => { const { chatApplication } = await import("/src/lib/chat/application.ts"); await chatApplication.cancel("native_stream_cancel"); });
    const result = await outcome(pet, "__cancelStream"); assert(result.error?.includes("cancelled") || result.result?.status === "cancelled");
    assert.equal((await view(pet)).busy, false);
    const retry = await petSend("after-cancel"); assert.equal(retry.status, "completed"); return { result, retry };
  });
  await check("provider failure is terminal and retry remains available", async () => {
    await submit(pet, { message: "[case:provider-failure]", client_request_id: "native_failure" }, "__failure");
    const result = await outcome(pet, "__failure"); assert(result.error || result.result?.status === "error");
    const state = await view(pet); assert.equal(state.busy, false);
    const retry = await petSend("after-failure"); assert.equal(retry.status, "completed"); return { result, retry };
  });
  await check("a legacy Mod send shows its model connection failure in the selected chat", async () => {
    const message = "[case:legacy-mod-connect-failure]";
    const scope = await view(main);
    await main.evaluate(message => {
      // Same host entry used by legacy Mod actions; bypass the chat composer.
      document.dispatchEvent(new CustomEvent("kokoro:mod-action", {
        detail: { componentId: "legacy-mod-fixture", action: "send_message", data: { message } },
      }));
    }, message);
    const state = await waitFor(async () => {
      const current = await view(main);
      return current.turns.some(turn => turn.message === message && turn.status === "error") ? current : null;
    });
    const turn = state.turns.find(turn => turn.message === message && turn.status === "error");
    assert.match(turn.client_request_id, /^mod_/);
    assert.equal(turn.character_id, scope.character_id);
    assert.equal(turn.conversation_id, scope.conversation_id);
    const failure = state.messages.map(row => JSON.parse(row.metadata ?? "null"))
      .find(metadata => metadata?.type === "failure_event" && metadata.event.turn_id === turn.turn_id)?.event;
    assert.equal(failure?.stage, "stream_connect");
    assert.equal(failure.character_id, scope.character_id);
    assert.equal(failure.conversation_id, scope.conversation_id);
    const alert = main.getByRole("alert").filter({ hasText: failure.message });
    const savedReason = main.getByText(failure.message, { exact: true });
    await alert.and(savedReason).waitFor({ state: "visible" });
    assert.equal(await alert.count(), 1);
    assert.equal(await savedReason.count(), 1);
    assert.equal(state.messages.filter(row => row.role === "user" && row.content === message).length, 1);
    const prompt = main.getByText(message, { exact: true });
    await waitFor(async () => await prompt.count() === 1 && await prompt.isVisible());
    return { turn, failure, displayedReason: await alert.textContent() };
  });
  await check("composer failures announce once and retry removes a saved partial reply without duplicating the prompt", async () => {
    const message = "[case:composer-retry-once]";
    const input = main.locator("textarea[data-onboarding-id='chat-input']");
    await input.fill(message);
    await input.press("Enter");
    const failed = await waitFor(async () => (await view(main)).turns.find(turn => turn.message === message && turn.status === "error"));
    const failedHistory = (await view(main)).messages;
    const partial = failedHistory.find(row => row.id === failed.assistant_message_id && row.content.includes("Partial reply before failure."));
    assert(partial, "the first tool round must leave a saved partial reply");
    const diagnostic = failedHistory.find(row => {
      const metadata = JSON.parse(row.metadata ?? "null");
      return metadata?.type === "failure_event" && metadata.event.turn_id === failed.turn_id;
    });
    assert(diagnostic?.content.includes("controlled composer failure"));
    assert.equal(JSON.parse(diagnostic.metadata).user_message_id, failed.user_message_id);
    await submit(pet, { message, regenerate: true, regenerate_user_message_id: failed.user_message_id }, "__staleRetry");
    assert.match((await outcome(pet, "__staleRetry")).error, /Conversation history changed before retry/);
    assert.deepEqual((await view(main)).messages, failedHistory);
    const alert = main.getByRole("alert").filter({ hasText: "controlled composer failure" });
    const savedReason = main.getByText(diagnostic.content, { exact: true });
    await savedReason.waitFor({ state: "visible" });
    await alert.waitFor({ state: "visible" });
    assert.equal(await alert.count(), 1);
    await savedReason.locator("..").getByRole("button").click();
    const recovered = await waitFor(async () => (await view(main)).turns.find(turn =>
      turn.message === message && turn.client_request_id !== failed.client_request_id && turn.status === "completed"));
    const history = (await view(main)).messages;
    assert.equal(recovered.user_message_id, failed.user_message_id);
    assert.equal(history.filter(row => row.role === "user" && row.content === message).length, 1);
    assert(!history.some(row => row.id === diagnostic.id || row.id === partial.id));
    assert(history.some(row => row.id === recovered.assistant_message_id && row.content.includes(message)));
    return { failed, recovered };
  });
  await check("a hidden failure cannot retry the preceding visible prompt", async () => {
    const before = await view(main);
    const requestId = "native_hidden_failure";
    assert(before.messages.some(row => row.role === "user"));
    await submit(pet, { message: "[case:hidden-chat-failure]", client_request_id: requestId, hidden: true }, "__hiddenFailure");
    assert((await outcome(pet, "__hiddenFailure")).error);
    const state = await view(main);
    const failed = state.turns.find(turn => turn.client_request_id === requestId);
    assert.equal(failed.user_message_id, null);
    const diagnostic = state.messages.find(row => JSON.parse(row.metadata ?? "null")?.event?.turn_id === failed.turn_id);
    assert(diagnostic?.content.includes("controlled hidden-chat-failure"));
    assert.equal(JSON.parse(diagnostic.metadata).user_message_id, null);
    const reason = main.getByText(diagnostic.content, { exact: true });
    await reason.waitFor({ state: "visible" });
    assert.equal(await reason.locator("..").getByRole("button").count(), 0);
    assert.deepEqual(state.messages.filter(row => row.role === "user").map(row => row.id), before.messages.filter(row => row.role === "user").map(row => row.id));
    return { turn: failed.turn_id, diagnostic: diagnostic.id };
  });
  await check("an ordinary composer failure during panel unmount is announced once after reopening", async () => {
    const name = "detached-composer-failure";
    const message = `[case:${name}]`;
    const input = main.locator("textarea[data-onboarding-id='chat-input']");
    await input.fill(message);
    await input.press("Enter");
    await waitFor(async () => (await (await fetch("http://127.0.0.1:19334/counts")).json()).some(call => call.name === name && call.streaming));
    const setPanelMounted = mounted => main.evaluate(async mounted => {
      const { registry } = await import("/src/ui/registry/ComponentRegistry.ts");
      if (!mounted) window.__originalChatPanel = registry.get("ChatPanel");
      registry.register("ChatPanel", mounted ? window.__originalChatPanel : () => null);
    }, mounted);
    await setPanelMounted(false);
    await input.waitFor({ state: "detached" });
    try {
      assert.equal((await fetch("http://127.0.0.1:19334/release-detached-failure")).status, 204);
      const state = await waitFor(async () => {
        const current = await view(pet);
        return current.turns.some(turn => turn.message === message && turn.status === "error") ? current : null;
      });
      const failed = state.turns.find(turn => turn.message === message && turn.status === "error");
      const diagnostic = state.messages.find(row => JSON.parse(row.metadata ?? "null")?.event?.turn_id === failed.turn_id);
      assert(diagnostic?.content.includes(`controlled ${name}`));
      assert.equal(await input.count(), 0);
      await setPanelMounted(true);
      const reason = main.getByText(diagnostic.content, { exact: true });
      const alert = main.getByRole("alert").filter({ hasText: `controlled ${name}` });
      await alert.and(reason).waitFor({ state: "visible" });
      assert.equal(await alert.count(), 1);
      assert.equal(await reason.count(), 1);

      await setPanelMounted(false);
      await input.waitFor({ state: "detached" });
      await setPanelMounted(true);
      await reason.waitFor({ state: "visible" });
      assert.equal(await alert.count(), 0);
      return { turn: failed.turn_id, diagnostic: diagnostic.id };
    } finally {
      if (!await input.count()) await setPanelMounted(true);
    }
  });
  await check("builtin tool execution and its in-progress assistant draft are shown once", async () => {
    const requestId = "native_tool_rounds";
    await submit(pet, { message: "[case:tool]", client_request_id: requestId }, "__toolRounds");
    const during = await waitFor(() => main.evaluate(async requestId => {
      const { chatApplication } = await import("/src/lib/chat/application.ts");
      const { projectMessages } = await import("/src/lib/chat/projection.ts");
      const state = await chatApplication.refresh();
      const turn = state.turns.find(turn => turn.client_request_id === requestId && turn.status === "streaming" && turn.tools.length);
      const draft = state.messages.find(message => message.role === "assistant" && message.content.trim() === "Draft before get_time.");
      if (!turn || !draft) return null;
      const replies = projectMessages(state).filter(message => message.role === "kokoro" && message.text.includes(draft.content.trim()));
      return { draft, turn, replies };
    }, requestId));
    assert.equal(during.replies.length, 1);
    assert.equal(during.replies[0].turnId, during.turn.turn_id);
    assert.equal(during.replies[0].tools.length, 1);
    const result = await outcome(pet, "__toolRounds");
    assert.equal(result.result?.status, "completed");
    assert.equal(result.result.assistant_message_id, during.draft.id);
    const state = await view(pet);
    const turn = state.turns.find(turn => turn.client_request_id === requestId);
    const tools = state.messages.filter(message => message.role === "tool" && message.metadata?.includes(turn.turn_id));
    assert.equal(tools.length, 1); return { during, turn, tools };
  });
  await check("text-mode tool result remains visible after the saved history takes over", async () => {
    const turn = await petSend("text-tool");
    assert.equal(turn.status, "completed");
    assert.equal(turn.tools.length, 1);
    const evidence = await main.evaluate(async turn => {
      const { chatApplication } = await import("/src/lib/chat/application.ts");
      const { projectMessages } = await import("/src/lib/chat/projection.ts");
      const state = await chatApplication.refresh();
      return {
        revision: state.revision,
        savedToolCount: state.messages.filter(message => message.role === "tool" && message.metadata?.includes(turn.turn_id)).length,
        reply: projectMessages(state).find(message => message.id === turn.assistant_message_id),
      };
    }, turn);
    assert(evidence.revision >= turn.revision);
    assert.equal(evidence.savedToolCount, 0);
    assert.equal(evidence.reply?.tools?.length, 1);
    assert.match(evidence.reply.tools[0].text, /Current time:/);
    return evidence;
  });
  await check("text-mode tool-only reply stays visible without a saved assistant or tool row", async () => {
    const turn = await petSend("text-tool-only");
    assert.equal(turn.status, "completed");
    assert.equal(turn.assistant_message_id, null);
    assert.equal(turn.text, "");
    const evidence = await main.evaluate(async turn => {
      const { chatApplication } = await import("/src/lib/chat/application.ts");
      const { projectMessages } = await import("/src/lib/chat/projection.ts");
      const state = await chatApplication.refresh();
      return {
        revision: state.revision,
        savedUser: state.messages.find(message => message.id === turn.user_message_id && message.role === "user"),
        savedReplies: state.messages.filter(message => ["assistant", "tool"].includes(message.role) && message.metadata?.includes(turn.turn_id)),
        replies: projectMessages(state).filter(message => message.turnId === turn.turn_id && message.role === "kokoro"),
      };
    }, turn);
    assert(evidence.revision >= turn.revision);
    assert(evidence.savedUser);
    assert.deepEqual(evidence.savedReplies, []);
    assert.equal(evidence.replies.length, 1);
    assert.equal(evidence.replies[0].text, "");
    assert.equal(evidence.replies[0].tools?.length, 1);
    assert.match(evidence.replies[0].tools[0].text, /Current time:/);
    return evidence;
  });
  await check("history edits reject during generation, switching cancels without cross-writing, regeneration and clear preserve ownership", async () => {
    const source = (await view(pet)).conversation_id;
    await pet.evaluate(async () => (await import("/src/lib/chat/application.ts")).chatApplication.clear());
    const targetTurn = await petSend("history-target"); assert.equal(targetTurn.status, "completed");
    const target = targetTurn.conversation_id; assert.notEqual(target, source);
    await pet.evaluate(async source => (await import("/src/lib/chat/application.ts")).chatApplication.selectConversation(source), source);
    await submit(pet, { message: "[case:history-switch-slow]", client_request_id: "native_history_switch" }, "__historySwitch");
    await waitFor(async () => (await view(pet)).turns.some(turn => turn.client_request_id === "native_history_switch" && turn.text.length > 0));
    const rejections = await pet.evaluate(async () => {
      const bridge = await import("/src/lib/kokoro-bridge.ts"); const current = await bridge.getChatView();
      const operations = [
        () => bridge.editConversationMessage({ conversation_id: current.conversation_id, conversation_generation: current.generation, message_id: current.messages.find(message => message.role === "user").id, new_content: "must not persist" }),
        () => bridge.deleteLastMessages(1, current.conversation_id),
        () => bridge.createConversation(),
      ];
      const results = [];
      for (const operation of operations) { try { await operation(); results.push("unexpected success"); } catch (error) { results.push(bridge.getKokoroErrorMessage(error)); } }
      return results;
    }); assert(rejections.every(error => error.includes("chat_turn_busy")));
    await pet.evaluate(async target => (await import("/src/lib/chat/application.ts")).chatApplication.selectConversation(target), target);
    const cancelled = await outcome(pet, "__historySwitch"); assert(cancelled.error?.includes("cancelled") || cancelled.result?.status === "cancelled");
    const switched = await view(pet); assert.equal(switched.conversation_id, target);
    assert(!switched.messages.some(message => message.content.includes("history-switch")));
    const staleEdit = await pet.evaluate(async ({ source, target, generation, messageId }) => {
      const bridge = await import("/src/lib/kokoro-bridge.ts");
      await bridge.loadConversation(source);
      await bridge.loadConversation(target);
      try {
        await bridge.editConversationMessage({ conversation_id: target, conversation_generation: generation, message_id: messageId, new_content: "must not persist" });
      } catch (error) {
        return bridge.getKokoroErrorMessage(error);
      }
    }, { source, target, generation: switched.generation, messageId: switched.messages.find(message => message.role === "user").id });
    assert.match(staleEdit, /Conversation changed/);
    assert.deepEqual((await view(pet)).messages, switched.messages);
    const regenerated = await pet.evaluate(async () => {
      const { chatApplication } = await import("/src/lib/chat/application.ts");
      const { projectMessages } = await import("/src/lib/chat/projection.ts"); await chatApplication.refresh();
      const snapshot = chatApplication.getSnapshot();
      const user = projectMessages(snapshot).find(message => message.role === "user");
      await chatApplication.edit({ conversationId: snapshot.conversation_id, generation: snapshot.generation, messageId: user.id }, "[case:history-regenerated]");
      const updated = chatApplication.getSnapshot();
      const answer = projectMessages(updated).find(message => message.role === "kokoro");
      return chatApplication.continueFrom({ conversationId: updated.conversation_id, generation: updated.generation, messageId: answer.id }, true, { allow_image_gen: false });
    }); assert.equal(regenerated.status, "completed"); assert.equal(regenerated.conversation_id, target);
    const revised = await view(pet); assert.equal(revised.messages.filter(message => message.role === "user").length, 1);
    assert.equal(revised.messages.filter(message => message.role === "assistant").length, 1);
    await pet.evaluate(async source => (await import("/src/lib/chat/application.ts")).chatApplication.deleteConversation(source), source);
    assert.equal((await view(pet)).conversation_id, target);
    await pet.evaluate(async () => (await import("/src/lib/chat/application.ts")).chatApplication.clear());
    const cleared = await view(pet); assert.equal(cleared.conversation_id, null); assert.deepEqual(cleared.messages, []);
    return { source, target, rejections, cancelled, staleEdit, regenerated, revisedMessageIds: revised.messages.map(message => message.id), clearedGeneration: cleared.generation };
  });
  await check("hidden main window does not block pet generation", async () => {
    await main.evaluate(async () => (await import("/node_modules/@tauri-apps/api/window.js")).getCurrentWindow().hide());
    const turn = await petSend("main-hidden"); assert.equal(turn.status, "completed"); return { turn };
  });
  await check("serialized character selection cancels its old turn without any ChatPanel owner", async () => {
    await pet.evaluate(async () => {
      const bridge = await import("/src/lib/kokoro-bridge.ts");
      await bridge.createCharacter({ id: "chat-validation-other", name: "Other", persona: "Other controlled test character", user_nickname: "Tester", source_format: "manual", created_at: 1, updated_at: 1, runtime_profile_json: JSON.stringify({ proactive_enabled: false }) });
    });
    await submit(pet, { message: "[case:character-switch-slow]", client_request_id: "native_character_switch" }, "__characterSwitch");
    await waitFor(async () => (await view(pet)).turns.some(turn => turn.client_request_id === "native_character_switch" && turn.text.length > 0));
    const selection = await pet.evaluate(async () => {
      const bridge = await import("/src/lib/kokoro-bridge.ts");
      const { chatApplication } = await import("/src/lib/chat/application.ts");
      const { createCharacterActivationService } = await import("/src/features/characters/character-activation.ts");
      const service = createCharacterActivationService({
        beforeCharacterActivation: id => chatApplication.prepareCharacterSwitch(id),
        prepareCharacterActivation: bridge.prepareCharacterActivation, commitCharacterActivation: bridge.commitCharacterActivation,
        getCommittedCharacterRuntime: bridge.getCommittedCharacterRuntime,
        readFrontendRuntime: () => ({ activeCharacterId: "chat-validation-character", live2dModel: null, background: null, cueProfile: null, tts: { enabled: true, mode: "browser", providerId: "browser", voice: null, speed: 1, pitch: 1 } }),
        applyFrontendRuntime: async () => {}, restoreFrontendRuntime: async () => {}, writeRuntimeCache: () => {}, dispatchRuntimeChanged: () => {},
        probeLocalTtsPreset: async () => false, presentLocalTtsPresetProbe: () => {}, confirmLocalTtsPresetSave: async () => false, saveConfirmedLocalTtsPreset: async () => {},
      });
      const committed = await service.activateCharacter("chat-validation-other");
      await chatApplication.refresh();
      const switched = chatApplication.getSnapshot();
      const result = await chatApplication.send({ message: "[case:other-character]", client_request_id: "native_other_character", character_id: "chat-validation-other", conversation_id: switched.conversation_id, conversation_generation: switched.generation });
      await service.activateCharacter("chat-validation-character");
      return { committed, switched, result };
    });
    const cancelled = await outcome(pet, "__characterSwitch"); assert(cancelled.error?.includes("cancelled") || cancelled.result?.status === "cancelled");
    assert.equal(selection.switched.character_id, "chat-validation-other");
    assert(!selection.switched.messages.some(message => message.content.includes("character-switch")));
    assert.equal(selection.result.status, "completed"); return { cancelled, ...selection };
  });
  await check("remounting the chat panel recovers a real speech failure and the saved text", async () => {
    const originalConfig = await main.evaluate(async () => (await import("/src/lib/kokoro-bridge.ts")).getTtsConfig());
    try {
      await main.evaluate(async originalConfig => {
        const bridge = await import("/src/lib/kokoro-bridge.ts");
        // Replace the provider already bound to this character in the live TTS service.
        const id = (await bridge.getCommittedCharacterRuntime())?.runtime.tts.provider_id;
        if (!id) throw new Error("the fixture character must have an automatic speech provider");
        await bridge.saveTtsConfig({
          ...originalConfig,
          providers: [...originalConfig.providers.filter(provider => provider.id !== id), {
            id, provider_type: "openai", enabled: true, api_key: "controlled-fixture",
            base_url: "http://127.0.0.1:19334/v1", model: "tts-1", extra: {},
          }],
        });
      }, originalConfig);
      const input = main.locator("textarea[data-onboarding-id='chat-input']");
      await main.evaluate(async () => {
        const { registry } = await import("/src/ui/registry/ComponentRegistry.ts");
        window.__originalChatPanel = registry.get("ChatPanel");
        registry.register("ChatPanel", () => null);
      });
      await input.waitFor({ state: "detached" });
      await submit(pet, { message: "[case:speech-failure]", client_request_id: "native_speech_failure" }, "__speechFailure");
      const result = await outcome(pet, "__speechFailure");
      assert.equal(result.result?.status, "completed");
      const failedTurn = await waitFor(async () => (await view(pet)).turns.find(turn =>
        turn.client_request_id === "native_speech_failure" && turn.speech_error?.includes("controlled speech failure")));
      assert.equal(failedTurn.status, "completed");
      assert.equal(await input.count(), 0);
      await main.evaluate(async () => {
        const { registry } = await import("/src/ui/registry/ComponentRegistry.ts");
        registry.register("ChatPanel", window.__originalChatPanel);
      });
      await input.waitFor({ state: "visible" });
      const alert = main.getByRole("alert").filter({ hasText: "controlled speech failure" });
      await alert.waitFor({ state: "visible", timeout: 10000 });
      const warning = await alert.textContent();
      await main.getByText("Controlled reply for [case:speech-failure] completed.", { exact: false }).first().waitFor();
      return { result, warning };
    } finally {
      await main.evaluate(async config => (await import("/src/lib/kokoro-bridge.ts")).saveTtsConfig(config), originalConfig);
    }
  });
  await check("destroyed main WebView leaves pet/backend chat fully operational", async () => {
    await submit(main, { message: "[case:sender-destroyed-slow]", client_request_id: "native_sender_destroyed" }, "__destroyedSender");
    await waitFor(async () => (await view(pet)).turns.some(turn => turn.client_request_id === "native_sender_destroyed" && turn.text.length > 0));
    report.ttsMain = await main.evaluate(() => window.__nativeTts);
    await main.evaluate(async () => (await import("/node_modules/@tauri-apps/api/window.js")).getCurrentWindow().destroy()).catch(() => {});
    await waitFor(async () => !(await windows(pet)).includes("main"));
    const senderTurn = await waitFor(async () => (await view(pet)).turns.find(turn => turn.client_request_id === "native_sender_destroyed" && turn.status === "completed"));
    assert((await view(pet)).messages.some(message => message.role === "assistant" && message.content.includes("sender-destroyed-slow")));
    const turn = await petSend("main-destroyed"); assert.equal(turn.status, "completed");
    const state = await view(pet); assert(state.messages.some(message => message.role === "assistant" && message.content.includes("main-destroyed")));
    await pet.screenshot({ path: path.join(output, "04-no-main.png") }); return { windows: await windows(pet), senderTurn, turn };
  });
  const calls = await (await fetch("http://127.0.0.1:19334/counts")).json();
  report.ttsPet = await pet.evaluate(() => window.__nativeTts);
  await check("only one WebView receives each automatic speech request", async () => {
    const starts = [...report.ttsMain, ...report.ttsPet].filter(item => item.event === "tts:start");
    assert(starts.length >= 5); assert.equal(new Set(starts.map(item => item.payload.request_id)).size, starts.length);
    assert(report.ttsPet.some(item => item.event === "tts:start")); return starts;
  });
  await check("controlled model call counters prove no replay or parallel model generation", async () => {
    for (const name of ["pet-only", "remount-slow", "concurrent-slow", "duplicate-slow", "main-hidden", "sender-destroyed-slow", "main-destroyed"]) assert.equal(calls.filter(call => call.streaming && call.name === name).length, 1, name);
    assert.equal(calls.filter(call => call.streaming && call.name === "before-cancel").length, 0);
    assert.equal(calls.filter(call => call.streaming && call.name === "tool").length, 2); // tool round + one follow-up
    assert.equal(calls.filter(call => call.streaming && call.name === "text-tool").length, 2);
    assert.equal(calls.filter(call => call.streaming && call.name === "text-tool-only").length, 2);
    return calls;
  });
  report.completedAt = new Date().toISOString(); save();
} finally { await browser.close(); }
