import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

export async function checkBotConfigTransactions({ page, check }) {
  const root = process.env.KOKORO_TEST_DATA_ROOT;
  assert(root && path.isAbsolute(root), "Bot saves require the isolated native fixture data root");
  const appData = path.join(root, "com.chyin.kokoro");
  const configPath = path.join(appData, "bot_config.json");
  await fs.access(path.join(appData, "kokoro.db"));

  async function invoke(name, ...args) {
    return page.evaluate(async ({ name, args }) => {
      const { commands } = await import("/src/lib/generated/ipc.ts");
      try {
        return { ok: true, value: await commands[name](...args) };
      } catch (error) {
        return { ok: false, error };
      }
    }, { name, args });
  }

  async function call(name, ...args) {
    const reply = await invoke(name, ...args);
    assert(reply.ok, `${name} failed: ${JSON.stringify(reply.error)}`);
    return reply.value;
  }

  async function assertConflict(name, ...args) {
    const reply = await invoke(name, ...args);
    assert(!reply.ok, `${name} accepted an outdated configuration`);
    assert.equal(reply.error?.code, "BotConfigConflict");
    return reply.error.code;
  }

  async function readSnapshots() {
    const bot = await call("getBotConfig");
    const telegram = await call("getTelegramConfig");
    assert.deepEqual(telegram, { revision: bot.revision, config: bot.telegram });
    return { bot, telegram };
  }

  async function assertCommitted(bot) {
    assert.deepEqual((await readSnapshots()).bot, bot);
    const bytes = await fs.readFile(configPath);
    assert.deepEqual(JSON.parse(bytes), bot);
    return bytes;
  }

  // Confirm the app attached to this CDP session is configured for the localhost
  // model fixture before performing any write.
  const llm = await call("getLlmConfig");
  assert.equal(llm.active_provider, "fixture");
  assert(llm.providers.some(provider => provider.id === "fixture"
    && provider.base_url === "http://127.0.0.1:19334/v1"));
  const baseline = (await readSnapshots()).bot;

  try {
    for (const telegramFirst of [true, false]) {
      const order = telegramFirst ? "Telegram then Bot" : "Bot then Telegram";
      await check(`native Bot config saves preserve both edits after retry, ${order}`, async () => {
        const initial = await readSnapshots();
        const voice = !initial.telegram.config.send_voice_reply;
        const character = initial.bot.telegram.character_id === "chat-validation-character"
          ? null
          : "chat-validation-character";
        const voiceEdit = { ...initial.telegram.config, send_voice_reply: voice };
        const characterEdit = {
          ...initial.bot,
          telegram: { ...initial.bot.telegram, character_id: character },
        };

        const first = telegramFirst
          ? await call("saveTelegramConfig", initial.telegram.revision, voiceEdit)
          : await call("saveBotConfig", characterEdit);
        assert.deepEqual(first, {
          ...initial.bot,
          revision: initial.bot.revision + 1,
          telegram: telegramFirst ? voiceEdit : characterEdit.telegram,
        });
        const bytes = await assertCommitted(first);
        const conflict = telegramFirst
          ? await assertConflict("saveBotConfig", characterEdit)
          : await assertConflict("saveTelegramConfig", initial.telegram.revision, voiceEdit);
        assert.deepEqual(await assertCommitted(first), bytes);

        const refreshed = await readSnapshots();
        const saved = telegramFirst
          ? await call("saveBotConfig", {
            ...refreshed.bot,
            telegram: { ...refreshed.bot.telegram, character_id: character },
          })
          : await call("saveTelegramConfig", refreshed.telegram.revision, {
            ...refreshed.telegram.config, send_voice_reply: voice,
          });
        assert.deepEqual(saved, {
          ...initial.bot,
          revision: initial.bot.revision + 2,
          telegram: { ...voiceEdit, character_id: character },
        });
        await assertCommitted(saved);
        return { initialRevision: initial.bot.revision, revisionAfterFirstSave: first.revision, finalRevision: saved.revision, conflict };
      });
    }

    await check("native Telegram conflict leaves a revoked chat out of runtime and disk", async () => {
      const retained = -1001234567890;
      const revoked = 123;
      const beforeSeed = (await readSnapshots()).bot;
      const seeded = await call("saveBotConfig", {
        ...beforeSeed,
        telegram: { ...beforeSeed.telegram, allowed_chat_ids: [revoked, retained] },
      });
      const initial = await readSnapshots();
      assert.deepEqual(initial.bot, seeded);
      const revocation = await call("saveBotConfig", {
        ...initial.bot,
        telegram: { ...initial.bot.telegram, allowed_chat_ids: [retained] },
      });
      const bytes = await assertCommitted(revocation);
      const voice = !initial.telegram.config.send_voice_reply;
      const conflict = await assertConflict("saveTelegramConfig", initial.telegram.revision, {
        ...initial.telegram.config, send_voice_reply: voice,
      });
      assert.deepEqual(await assertCommitted(revocation), bytes);

      const refreshed = await readSnapshots();
      const saved = await call("saveTelegramConfig", refreshed.telegram.revision, {
        ...refreshed.telegram.config, send_voice_reply: voice,
      });
      assert.deepEqual(saved, {
        ...revocation,
        revision: revocation.revision + 1,
        telegram: { ...revocation.telegram, send_voice_reply: voice, allowed_chat_ids: [retained] },
      });
      await assertCommitted(saved);
      return { revoked, retained, revocationRevision: revocation.revision, finalRevision: saved.revision, conflict };
    });
  } finally {
    const { bot } = await readSnapshots();
    const restored = await call("saveBotConfig", {
      ...bot,
      telegram: {
        ...bot.telegram,
        send_voice_reply: baseline.telegram.send_voice_reply,
        character_id: baseline.telegram.character_id,
        allowed_chat_ids: baseline.telegram.allowed_chat_ids,
      },
    });
    assert.deepEqual(restored, { ...baseline, revision: bot.revision + 1 });
    await assertCommitted(restored);
  }
}
