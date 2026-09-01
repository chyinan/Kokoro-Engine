# Codex Runtime Provider (Experimental)

Kokoro Engine can optionally use a locally installed Codex CLI as a runtime provider:

```text
Kokoro LlmProvider
  → CodexRuntimeProvider
  → codex app-server --listen stdio://
  → the user's existing Codex configuration, provider, and authentication
```

This is an experimental runtime bridge, not an OpenAI Platform API provider. Kokoro does not
read, copy, or store Codex credentials, and it does not modify `~/.codex/config.toml`. Codex
continues to own ChatGPT/Codex subscription login, API keys, refresh, custom `model_provider`,
custom `base_url`, relays, and local backends.

## Setup

1. Install and sign in to Codex using the normal Codex CLI workflow.
2. In Kokoro, open **Settings → API** and add **Codex Runtime — Experimental**.
3. Leave **Model override** empty to use the model selected by Codex. Entering a model is a
   per-thread override and does not rewrite Codex configuration.
4. Use **Fetch Models** to query Codex's `model/list` RPC, or use **Test connection**.

The runtime status only detects the Codex executable. A connection test starts the app-server,
performs the app-server handshake, creates an ephemeral thread, and sends a real turn.

## Context and permissions

Kokoro sends its existing conversation context and, when enabled by Kokoro's normal tool loop,
its tool definitions through the app-server dynamic-tool protocol. Codex receives deterministic
`kokoro_tool_<index>_<safe-name>` aliases rather than Kokoro/MCP names such as `mcp__...`; return
calls are mapped back to their original names before Kokoro executes them. Kokoro remains
authoritative for Memory, RAG, MCP, and action execution.

Each request uses an ephemeral app-server thread and asks Codex to:

- use empty base/developer instruction overrides;
- disable permission, app, and collaboration instruction blocks;
- disable host skill discovery, plugins, connectors, web/search, shell, code-mode, memory, and
  multi-agent features for the thread;
- use no execution environments (`environments: []`);
- use `approvalPolicy: "never"` and `sandbox: "read-only"`;
- accept Kokoro tools only as dynamic tools, then return the call to Kokoro for execution.

The app-server is still an agent runtime and may retain fixed engine behavior that cannot be
removed through the public app-server request fields. Therefore this provider is “near-inference”
and not a claim that Codex is a bare model endpoint. Kokoro does not select Codex skills, plugins,
or MCP servers; in the currently tested Codex version, the thread-scoped `mcp_servers: {}`
override and disabled feature flags keep configured Codex MCP from being used by this bridge.
These are request-scoped, experimental assumptions rather than a guarantee for every future
Codex release. If a future Codex version rejects these security overrides, Kokoro fails the
request instead of falling back to a less restricted mode.

## Protocol and compatibility notes

The bridge currently uses the app-server JSONL subset documented by Codex:

- `initialize` then `initialized` once per child process;
- `thread/start` with `ephemeral: true`;
- `thread/inject_items` for Kokoro's prior Responses-compatible history;
- `turn/start` for the latest user input or a Kokoro tool result;
- `item/agentMessage/delta`, reasoning deltas, `item/tool/call`, and `turn/completed` events;
- `turn/interrupt` when the consumer drops an in-flight stream;
- `model/list` for runtime model discovery.

The bridge inherits Codex's current provider and authentication configuration by omitting
`modelProvider` and by launching the process without a replacement `CODEX_HOME`. Model overrides
are sent only in the ephemeral `thread/start` request. The child is restarted once after a setup
failure; a missing binary, incompatible app-server, process crash, RPC error, or malformed event
is surfaced as a provider error without changing the other Kokoro providers.

Timeouts are phase-specific. Ordinary JSON-RPC requests use a 30-second IPC timeout; the
`turn/start` acknowledgement has a five-minute timeout so Codex can complete its own backend
transport retry/fallback. After the acknowledgement, Kokoro allows five minutes for first output,
two minutes of streaming idle time, and fifteen minutes total for the turn. Transport/status
messages are recorded as bounded, non-sensitive classifications (for example websocket fallback,
reconnect, HTTPS, timeout, or authentication); raw stderr/status text is not logged.

Tool calls are intentionally bridged through `dynamicTools`, which is an experimental
app-server capability. Codex's own filesystem/shell/tool loop is not enabled for the Kokoro
thread. Quota and usage display are deferred; Kokoro does not scrape Codex account endpoints.

## Stability and licensing

The Codex app-server is documented and published as an experimental interface. Its exact method
fields and event set can change with the installed Codex version, so Kokoro uses a narrow parser
and reports incompatibility rather than guessing.

Kokoro independently implements the runtime bridge and keeps its lifecycle, capability gates,
and event parsers isolated from the rest of the provider stack.
