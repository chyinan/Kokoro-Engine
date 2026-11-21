// @vitest-environment jsdom
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { deferred } from "@/lib/chat/__tests__/harness";
import { emptyChatView } from "@/lib/chat/application";
import { editConversationMessage, type ChatViewSnapshot, type Conversation, type EditConversationMessageRequest } from "@/lib/kokoro-bridge";
import { ChatMessage } from "../ChatMessage";
import ChatPanel from "../ChatPanel";

vi.mock("@tauri-apps/api/core", async original => ({ ...await original<typeof import("@tauri-apps/api/core")>(), invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => vi.mocked(invoke).mockReset());

const beginning = "雨后的庭院花瓣落满青石路面。\n".repeat(108);
const ending = "窗边的茶水仍冒着白色的热气。\n".repeat(108) + "【全文末尾：归航】";
const rawText = (middle: string) => `\n  ${beginning}${middle}\n${ending}\n\t`;
const withMixedLineEndings = (text: string) => {
  let line = 0;
  return text.replace(/\n/g, () => line++ % 2 === 0 ? "\r\n" : "\n");
};
const textareaText = (text: string) => text.replace(/\r\n/g, "\n");

it("keeps the saved CRLF after two native-style edits and retries a failed save with the same bytes", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const save = vi.fn<(text: string) => Promise<boolean>>().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  const action = (name: string) => container.querySelector<HTMLButtonElement>(`button[title="chat.actions.${name}"]`)!;
  try {
    await act(async () => root.render(<ChatMessage message={{ role: "kokoro", text: "A\nB", editText: "A\r\nB" }}
      index={0} isStreaming={false} isTranslationExpanded={false} editScope="message" onEdit={save} onContinueFrom={() => {}}
      onToggleTranslation={() => {}} onApproveTool={() => {}} onRejectTool={() => {}} />));
    await act(async () => action("edit").click());
    const editor = container.querySelector("textarea")!;
    const insert = async (at: number, text: string, inputType: string) => {
      await act(async () => {
        const data = inputType === "insertFromPaste" ? null : text;
        editor.setSelectionRange(at, at);
        editor.dispatchEvent(new InputEvent("beforeinput", { bubbles: true, cancelable: true, inputType, data }));
        const value = editor.value.slice(0, at) + text + editor.value.slice(at);
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(editor, value);
        editor.setSelectionRange(at + text.length, at + text.length);
        editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType, data }));
      });
    };
    const first = "x".repeat(97);
    const second = "y".repeat(97);
    await insert(1, first, "insertFromPaste");
    await insert(editor.value.length, second, "insertText");
    const expected = `A${first}\r\nB${second}`;
    expect(editor.value).toBe(textareaText(expected));

    await act(async () => action("save").click());
    expect(editor.readOnly).toBe(false);
    expect(editor.value).toBe(textareaText(expected));
    await act(async () => action("save").click());

    expect(save.mock.calls).toEqual([[expected], [expected]]);
    expect(container.querySelector("textarea")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

it.each([
  { role: "user", addedLine: "", separatedEdits: false, pasteSize: 0 },
  { role: "assistant", addedLine: "", separatedEdits: false, pasteSize: 0 },
  { role: "assistant", addedLine: "\n新插入的行", separatedEdits: false, pasteSize: 0 },
  { role: "assistant", addedLine: "", separatedEdits: true, pasteSize: 0 },
  { role: "assistant", addedLine: "", separatedEdits: false, pasteSize: 97 },
])("edits and reopens a long saved $role message (added line: '$addedLine', separated edits: $separatedEdits, paste size: $pasteSize)", async ({ role, addedLine, separatedEdits, pasteSize }) => {
  const text = (middle: string) => role === "assistant" ? withMixedLineEndings(rawText(middle)) : rawText(middle);
  const sourceLabel = role === "assistant" ? "**中段甲**" : "中段甲";
  const original = text(sourceLabel);
  let edited = original.replace(sourceLabel, sourceLabel.replace("甲", "乙") + addedLine);
  if (separatedEdits) {
    edited = "开头新增\n" + edited.replace("雨后", "雨停").replace("归航", "归家");
  }
  if (pasteSize) {
    const line = beginning.slice(0, beginning.indexOf("\n"));
    edited = edited.replace(`${line}\r\n${line}`, `${"x".repeat(pasteSize)}${line}\r\n${line}${"y".repeat(pasteSize)}`);
  }
  // Assistant bubbles display clean text, while their editors open the original source.
  const displayed = (body: string) => role === "assistant" ? body.replace(/\*\*/g, "").trim() : body;
  const conversation = (id: string, title: string): Conversation => ({
    id, title, character_id: "character-a", topic: "", pinned_state: "{}",
    created_at: "2026-11-18T00:00:00Z", updated_at: "2026-11-18T00:00:00Z",
  });
  const archive = conversation("archive", "长文档案");
  const other = conversation("other", "另一条会话");
  const saved = { id: 101, role, content: original, created_at: archive.created_at, metadata: null };
  const otherRow = { ...saved, role: "user", id: 201, content: "会话切换验证" };
  let selected = archive.id;
  let generation = 3;
  let historyRevision = 1;
  const limitError = { code: "Validation", message: "Chat input exceeds the 262144-byte limit" };
  let holdEditedRead = false;
  const editedRead = deferred<void>();
  const rows = () => selected === archive.id ? [saved] : [otherRow];
  const snapshot = (): ChatViewSnapshot => ({
    ...emptyChatView, character_id: archive.character_id, conversation_id: selected, generation,
    history_revision: historyRevision, conversation_history_revision: historyRevision,
    messages: structuredClone(rows()),
  });
  const host = vi.mocked(invoke).mockImplementation(async (command, args) => {
    switch (command) {
      case "get_chat_view":
        if (holdEditedRead) await editedRead.promise;
        return snapshot();
      case "edit_conversation_message": {
        const { request } = args as { request: EditConversationMessageRequest };
        if (role === "user" && new TextEncoder().encode(request.new_content).length > 262144) throw limitError;
        saved.content = request.new_content;
        historyRevision++;
        holdEditedRead = true;
        return { message_id: saved.id, updated_content: saved.content };
      }
      case "list_conversations":
        return [archive, other];
      case "load_conversation":
        selected = (args as { request: { id: string } }).request.id;
        generation++;
        return { topic: "", pinned_state: "{}", messages: structuredClone(rows()) };
    }
  });

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const row = () => container.querySelector(`[data-chat-message-id="${saved.id}"]`);
  const bubble = () => row()?.querySelector(".whitespace-pre-wrap")?.textContent;
  const action = (name: string) => row()?.querySelector<HTMLButtonElement>(`button[title="chat.actions.${name}"]`);
  const editor = () => row()!.querySelector("textarea")!;
  const typeText = (text: string) => {
    const input = editor();
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const select = async ({ title }: Conversation) => {
    await act(async () => container.querySelector<HTMLButtonElement>("[data-chat-history-toggle]")!.click());
    const entry = [...container.querySelectorAll<HTMLDivElement>("div")].find(div => div.textContent === title);
    expect(entry).toBeDefined();
    await act(async () => entry!.click());
  };

  try {
    await act(async () => root.render(<ChatPanel />));
    expect(bubble()).toBe(displayed(original));
    await act(async () => action("edit")!.click());
    expect(editor().value).toBe(textareaText(original));

    await act(async () => typeText(" \n\t "));
    await act(async () => action("save")!.click());
    expect(host.mock.calls.some(([command]) => command === "edit_conversation_message")).toBe(false);
    expect(editor().value).toBe(" \n\t ");

    if (role === "user") {
      const oversized = "界🙂".repeat(37449) + "ab";
      expect(new TextEncoder().encode(oversized).length).toBe(262145);
      await act(async () => typeText(oversized));
      await act(async () => action("save")!.click());
      expect(saved.content).toBe(original);
      expect(container.querySelector('[role="alert"]')?.textContent).toBe(limitError.message);
      expect(editor().value).toBe(oversized);
      expect(editor().readOnly).toBe(false);
    }

    const beforeSave = host.mock.calls.length;
    await act(async () => typeText(textareaText(edited)));
    await act(async () => action("save")!.click());
    expect(host.mock.calls.slice(beforeSave).filter(([command]) => command === "edit_conversation_message")).toEqual([
      ["edit_conversation_message", { request: {
        conversation_id: archive.id, conversation_generation: 3, message_id: saved.id, new_content: edited,
      } }, undefined],
    ]);
    expect(editor().value).toBe(textareaText(edited));
    expect(editor().readOnly).toBe(true);
    await act(async () => { holdEditedRead = false; editedRead.resolve(); });
    expect(bubble()).toBe(displayed(edited));

    await select(other);
    expect(container.querySelector(`[data-chat-message-id="${otherRow.id}"]`)?.textContent).toContain(otherRow.content);
    await act(async () => root.render(null));
    await act(async () => root.render(<ChatPanel />));
    expect(row()).toBeNull();

    await select(archive);
    expect(host.mock.calls.filter(([command]) => command === "load_conversation").map(([, args]) => args)).toEqual([
      { request: { id: other.id } }, { request: { id: archive.id } },
    ]);
    expect(bubble()).toBe(displayed(edited));
    await act(async () => action("edit")!.click());
    expect(editor().value).toBe(textareaText(edited));
  } finally {
    editedRead.resolve();
    await act(async () => root.unmount());
    container.remove();
  }
});

it("passes the complete body and visible index through the compatibility IPC entry", async () => {
  const request = { conversation_id: "archive", visible_index: 1, new_content: rawText("中段乙") };
  const result = { message_id: 101, updated_content: request.new_content };
  vi.mocked(invoke).mockResolvedValueOnce(result);

  await expect(editConversationMessage(request)).resolves.toEqual(result);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("edit_conversation_message", { request }, undefined);
});
