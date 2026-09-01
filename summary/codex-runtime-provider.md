# Codex Runtime Provider Summary

## Local boundary

`src-tauri/src/llm/provider.rs` is the provider-neutral contract consumed by `commands/chat.rs`.
`src-tauri/src/llm/service.rs` builds enabled providers from `provider_type`; chat only receives
text/reasoning/tool stream events and remains the owner of Kokoro tool execution.

`src/ui/widgets/settings/ApiTab.tsx` edits `LlmConfig` records. Provider-specific defaults and
normalization belong in `src/features/onboarding/provider-setup-core.ts`; Tauri calls are wrapped
by `src/features/onboarding/provider-setup.ts` and `src/lib/kokoro-bridge.ts`.

## Runtime design

`codex_runtime` is a separate provider type. It starts `codex app-server --listen stdio://` lazily,
performs `initialize`/`initialized`, and multiplexes numeric JSONL request IDs with broadcast
notifications. Each provider call uses an ephemeral `thread/start`, injects Kokoro's converted
history with `thread/inject_items`, then calls `turn/start`.

Thread-scoped safety defaults clear overrideable base/developer instructions, disable Codex
permissions/apps/plugins/search/shell/memory/multi-agent features, set `mcp_servers: {}`, select
no environments, and use read-only/never approval settings. Kokoro tool definitions are passed
as app-server experimental `dynamicTools`; Codex receives deterministic `kokoro_tool_<index>_<safe-name>` aliases
instead of reserved/internal names such as `mcp__...`. `item/tool/call` is surfaced to `chat.rs`, mapped back to
the original name, answered as host-handled, and the Codex turn is interrupted so Kokoro executes the real tool
in its existing loop. Tool results are injected into the next ephemeral turn as `toolOutput`.

`model/list` is the source for runtime model discovery. `model` is an optional per-thread override;
`modelProvider` is intentionally omitted so Codex inherits the user's configured provider and
base URL. Codex credentials and `CODEX_HOME` are never read by Kokoro or returned through IPC.

Timeouts are phase-specific: ordinary JSON-RPC calls use a 30-second IPC timeout; `turn/start`
acknowledgement allows five minutes for Codex backend transport fallback; first output allows five
minutes, streaming idle allows two minutes, and total turn duration allows fifteen minutes.
App-server stderr and transport/status notifications are logged only as bounded non-sensitive
classifications. A current-machine smoke turn completed after roughly 126 seconds with the longer
window, confirming the earlier 30-second result was not evidence of a JSON-RPC deadlock.

## External protocol facts

The official Codex app-server protocol is bidirectional JSON-RPC without the `jsonrpc` field,
newline-delimited over stdio. Stable integration calls are `initialize`, `thread/start`,
`thread/inject_items`, `turn/start`, `turn/interrupt`, and `model/list`; streaming notifications
include `item/agentMessage/delta`, reasoning deltas, `item/tool/call`, and `turn/completed`.
Dynamic tools and several permission-related fields require `experimentalApi` negotiation. The
protocol is versioned with the installed CLI and must be treated as experimental.
