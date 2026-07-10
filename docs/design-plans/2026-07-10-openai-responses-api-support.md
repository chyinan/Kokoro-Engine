# OpenAI Responses API Support Design

## Summary
The design adds OpenAI Responses as a separate, opt-in `openai_responses` provider while preserving the existing `openai` provider's Chat Completions behavior. A provider-local adapter implements the current `LlmProvider` contract, translates Kokoro's existing messages, parameters, images, and function tools into Responses requests, and maps typed stream events back into `LlmStreamEvent`. Kokoro continues sending its own assembled history with `store: false`, without adopting OpenAI-managed conversation state.

Native tools continue through the existing chat orchestration loop. A narrow internal extension carries opaque continuation items, such as reasoning output required by a follow-up tool request, only between rounds of the active in-memory tool loop. Delivery is phased across provider registration, request and response translation, streaming, tool continuation, settings and localization, then compatibility checks and rollback documentation.

## Definition of Done
Kokoro Engine has an implementation-ready, non-destructive design for adding OpenAI Responses API support as an opt-in protocol. Existing Chat Completions behavior, provider interfaces, IPC commands, saved configuration, and non-OpenAI providers remain usable with unchanged defaults. The design identifies exact integration boundaries, compatibility and rollback rules, test coverage, phased delivery, and acceptance criteria based on the current repository and official OpenAI documentation.

## Glossary
- **Chat Completions**: OpenAI's older chat protocol, exposed through `/chat/completions` and retained by Kokoro's existing `openai` provider.
- **Responses API**: OpenAI's newer response-generation protocol, exposed through `/responses` and added as the `openai_responses` provider.
- **`LlmProvider`**: Kokoro's Rust trait that defines the common interface implemented by each model provider.
- **`LlmStreamEvent`**: Kokoro's provider-neutral event type for successful streamed text, reasoning, tool calls, and the proposed continuation data. Stream errors remain `Result::Err` values.
- **Semantic SSE events**: Named Server-Sent Events that describe response lifecycle and content changes instead of an untyped text stream.
- **Native tool continuation**: The follow-up model request made after Kokoro executes a function call and returns its result.
- **Opaque continuation items**: Provider-specific response items preserved without interpretation so they can be included in the next tool round.
- **`call_id`**: The identifier linking a model-issued function call to its corresponding function output.
- **`previous_response_id`**: A Responses field that links a request to stored server-side response state; this design does not use it.
- **OpenAI Conversations API**: OpenAI-managed persistent conversation state, deferred because Kokoro's local database remains authoritative.
- **Tauri IPC**: The command and event boundary used for communication between Kokoro's React frontend and Rust backend.
- **`wiremock`**: The Rust HTTP mocking library used to verify request paths, payloads, and error handling in provider tests.

## Scope convergence

### Goal and boundary

Add the Responses API to the main LLM provider system without changing the meaning or default behavior of existing providers. The first release covers text, image input already represented by Kokoro messages, streaming, custom function tools, connection testing, and manual conversation history.

This design does not migrate independent STT, TTS, image generation, or direct VLM integrations. It does not replace Kokoro's local conversation database with OpenAI Conversations, and it does not expose OpenAI built-in tools through the product UI.

### Complexity check

The main overengineering risk is turning a protocol adapter into a repository-wide neutral message rewrite. The existing `LlmProvider` and `LlmStreamEvent` boundaries already isolate most protocol differences. Replacing the Chat Completions-shaped internal message representation would touch Anthropic, Ollama, llama.cpp, memory, vision, and chat orchestration without being required for Responses support.

The minimum design keeps that representation and translates it only inside the new provider. One narrow internal extension carries opaque Responses continuation items during a native-tool round. Existing providers ignore it.

## Architecture options

### Option A: change the existing `openai` provider to Responses

Rejected. Existing OpenAI-compatible servers may only implement `/chat/completions`. Changing the provider's endpoint would break saved configurations and local services.

### Option B: add an API protocol field to every OpenAI provider

Viable, but not preferred for the first release. A field such as `api_protocol` keeps one provider type, but it adds conditional behavior to the current implementation and makes accidental protocol changes easier. It also requires migration-aware handling across provider normalization, presets, and UI editing.

### Option C: add `openai_responses` as a separate provider type

Recommended. Existing `openai` entries keep Chat Completions semantics. A Responses entry can coexist with the old entry, use the same API key and base URL, and be selected or removed independently. Rollback is a provider selection change rather than a configuration migration.

## Architecture

### Provider selection and configuration

Add `openai_responses` to the provider types supported by `src-tauri/src/llm/service.rs` and `src/ui/widgets/settings/ApiTab.tsx`. Do not change `default_providers()` in `src-tauri/src/llm/llm_config.rs`; new and existing installations continue to default to `provider_type: "openai"`.

The new provider uses the existing fields:

- `api_key` and `api_key_env` for authentication.
- `base_url`, defaulting to `https://api.openai.com/v1`.
- `model` for the selected model ID.
- `supports_native_tools` for Kokoro's current tool-loop switch.
- `extra` only for Responses-specific optional settings introduced later.

No config migration is required because `provider_type` is already a string in Rust and TypeScript. Presets continue to store complete provider records.

### Responses adapter

Add `src-tauri/src/llm/responses.rs` with `OpenAIResponsesProvider`. Enable the existing `async-openai 0.34.0` dependency's `responses` feature in `src-tauri/Cargo.toml`; a dependency version upgrade is not required for the initial implementation.

The adapter implements the existing `LlmProvider` trait and owns four translations:

1. `ChatCompletionRequestMessage` and `LlmChatMessage` to Responses `input` items.
2. `LlmParams` to supported Responses request fields. `max_tokens` maps to `max_output_tokens`; supported sampling fields map directly. Explicitly supplied options that Responses doesn't support must return a clear provider error instead of being silently ignored.
3. `LlmToolDefinition` to Responses function tools with JSON Schema parameters and `parallel_tool_calls: false`, matching Kokoro's sequential execution loop.
4. Responses output and semantic SSE events to `LlmStreamEvent`.

Use `store: false` and send Kokoro's assembled history on every request. Kokoro's database remains the source of truth for deletion, memory, character separation, and history restoration. Do not send `previous_response_id` or a Conversations API identifier in the first release.

### Streaming event mapping

The provider listens only to events required by Kokoro:

| Responses event | Kokoro result |
| --- | --- |
| `response.output_text.delta` | `LlmStreamEvent::Text` |
| `response.reasoning_summary_text.delta` or supported reasoning text delta | `LlmStreamEvent::ReasoningContent` |
| `response.output_item.added` for `function_call` | Initialize pending tool call by output index/item ID |
| `response.function_call_arguments.delta` | Append encoded JSON arguments |
| `response.function_call_arguments.done` or completed output item | Parse and emit `LlmStreamEvent::ToolCall` once |
| `response.completed` | Finish the stream |
| `response.failed`, `response.incomplete`, or `error` | Emit a normalized provider error |
| refusal events | Emit refusal text as visible text, then finish normally |

Unknown events are ignored and logged at debug level. The adapter must not treat `[DONE]` as the primary terminator because Responses uses typed lifecycle events.

### Native tool continuation

Kokoro currently appends an assistant tool-call message and one tool-result message per call before starting the next provider round. The Responses adapter translates these into `function_call` and `function_call_output` input items using the existing tool call ID as `call_id`.

Reasoning models may require reasoning output items from the tool-call response to be passed back with tool outputs. Extend the internal rich-message path with opaque provider continuation items:

- `src-tauri/src/llm/provider.rs` defines a provider-neutral container for continuation data and a stream event that delivers it.
- `src-tauri/src/commands/chat.rs` collects that data during a round and attaches it only to the next in-memory tool continuation.
- `src-tauri/src/llm/messages.rs` preserves the attached data while sanitizing the assistant/tool message sequence.
- Existing providers emit no continuation data and behave exactly as before.

This data is not sent through Tauri IPC and is not required after a completed tool loop. Persistent conversation history remains in Kokoro's current format.

### Public interface compatibility

Keep these contracts unchanged:

- Tauri commands: `get_llm_config`, `save_llm_config`, `test_llm_connection`, and `stream_chat`.
- Frontend chat events such as `chat-turn-delta`, `chat-turn-tool`, and `chat-error`.
- `LlmProvider::chat`, `chat_stream_rich`, and `chat_stream_with_tools_rich` call sites.
- Existing `openai`, `anthropic`, `ollama`, and `llama_cpp` provider behavior.

`src/lib/kokoro-bridge.ts` needs no chat API change. Its `provider_type: string` field already accepts the additive provider value.

### Settings behavior

Add an "OpenAI Responses" provider option in `ApiTab.tsx`. It uses the same API key, environment fallback, base URL, model list, enable/disable, active provider, system provider, and preset controls as OpenAI-compatible entries.

The UI must identify the protocol clearly. Do not label the existing provider simply as "OpenAI"; retain "OpenAI-Compatible" for Chat Completions and use "OpenAI Responses" for the new provider. Add user-facing strings to every locale in `src/ui/locales/*.json`.

Connection testing remains unchanged at the IPC level. `test_config_connection()` builds the selected provider and calls `chat`, so the new provider is tested against `/responses` automatically.

## Existing patterns

- Provider implementations live under `src-tauri/src/llm/` and implement `LlmProvider`.
- `src-tauri/src/llm/service.rs` dispatches implementations by `provider_type`.
- `src-tauri/src/llm/anthropic.rs` and `src-tauri/src/llm/llama_cpp.rs` use provider-local request translation and `wiremock` tests.
- `src-tauri/src/commands/chat.rs` owns tool execution and UI event emission; providers only emit neutral model events.
- `src/ui/widgets/settings/ApiTab.tsx` owns provider creation, normalization, model fetching, presets, and editing.
- Provider-specific optional values already use `LlmProviderConfig.extra`.

The design follows these boundaries. It does not introduce another IPC command family or a second chat orchestration loop.

## Implementation phases

<!-- START_PHASE_1 -->
### Phase 1: Additive provider registration

**Goal:** Make `openai_responses` a valid opt-in provider without changing existing defaults.

**Components:**

- `src-tauri/Cargo.toml` enables the `async-openai` `responses` feature.
- `src-tauri/src/llm/mod.rs` registers the new provider module.
- `src-tauri/src/llm/service.rs` constructs `OpenAIResponsesProvider` for `provider_type: "openai_responses"`.
- Service tests prove existing `openai` records still build the Chat Completions provider, unknown providers still fail, and the new provider can coexist with an old provider.

**Dependencies:** None.

**Done when:** Existing config tests pass without changing `default_providers()`, and a config containing both provider types loads, saves, and selects each provider independently.
<!-- END_PHASE_1 -->

<!-- START_PHASE_2 -->
### Phase 2: Request and response translation

**Goal:** Support non-streaming text and existing multimodal input through `/responses`.

**Components:**

- New `src-tauri/src/llm/responses.rs` implements authentication, base URL handling, message conversion, parameter validation, `store: false`, and output text extraction.
- Mapping tests cover system/developer/user/assistant messages, image content, empty output, refusal output, supported parameters, and unsupported explicit parameters.
- `wiremock` tests verify the request path is `/v1/responses`, existing OpenAI providers still use `/v1/chat/completions`, and API errors retain status and response body context.

**Dependencies:** Phase 1.

**Done when:** `LlmProvider::chat` returns text through Responses and the existing connection test succeeds against a mocked Responses endpoint.
<!-- END_PHASE_2 -->

<!-- START_PHASE_3 -->
### Phase 3: Semantic streaming

**Goal:** Convert Responses SSE into the existing Kokoro text and reasoning stream contract.

**Components:**

- `src-tauri/src/llm/responses.rs` parses typed lifecycle, text, reasoning, refusal, incomplete, failed, and error events.
- Streaming tests use recorded minimal SSE fixtures for text-only completion, refusal, incomplete output, API failure, malformed event JSON, and a trailing unknown event.
- `src-tauri/src/commands/chat.rs` requires no public event changes; regression tests confirm existing `chat-turn-delta` and `chat-error` behavior.

**Dependencies:** Phase 2.

**Done when:** A Responses stream produces the same visible frontend deltas and error behavior as the current Chat Completions provider.
<!-- END_PHASE_3 -->

<!-- START_PHASE_4 -->
### Phase 4: Native function tools

**Goal:** Preserve Kokoro's existing multi-round tool execution with Responses function calls.

**Components:**

- `src-tauri/src/llm/responses.rs` maps tool definitions, aggregates function argument deltas, emits one `LlmStreamEvent::ToolCall` per completed call, and maps tool outputs back to `function_call_output` items.
- `src-tauri/src/llm/provider.rs`, `src-tauri/src/llm/messages.rs`, and `src-tauri/src/commands/chat.rs` carry opaque continuation items needed by reasoning models during the current tool loop.
- Tests cover one tool call, multiple sequential calls, fragmented JSON arguments, tool errors, call ID preservation, reasoning items followed by tool output, and reaching `max_tool_rounds`.

**Dependencies:** Phase 3.

**Done when:** Built-in and MCP tools execute through the unchanged Kokoro tool loop, and the follow-up Responses request contains the matching function call, required reasoning items, and function output.
<!-- END_PHASE_4 -->

<!-- START_PHASE_5 -->
### Phase 5: Settings and localization

**Goal:** Let users add, select, test, save, duplicate through presets, and remove a Responses provider without confusing it with Chat Completions.

**Components:**

- `src/ui/widgets/settings/ApiTab.tsx` adds the provider type, defaults, labels, API-key handling, model listing, and normalization rules.
- `src/ui/locales/*.json` adds the visible protocol name and short description.
- Focused Vitest coverage is added for provider creation, type switching, default preservation, preset round-trip, and model-list behavior.

**Dependencies:** Phases 1-4.

**Done when:** Saving an existing config produces no protocol changes, while a new Responses entry round-trips through IPC and remains independently selectable.
<!-- END_PHASE_5 -->

<!-- START_PHASE_6 -->
### Phase 6: Compatibility and release gate

**Goal:** Verify the additive provider across the supported chat workflows and document rollback.

**Components:**

- Rust provider and service suites run with both OpenAI protocols.
- Frontend unit tests and production build verify bridge and settings compatibility.
- Manual Tauri checks cover normal chat, streaming cancellation, image input, system-provider tasks, one built-in tool, one MCP tool, preset switching, restart/config reload, and switching back to the old provider.
- User documentation identifies `openai` as Chat Completions and `openai_responses` as Responses.

**Dependencies:** Phases 1-5.

**Done when:** Existing provider regression suites pass, all new protocol tests pass, and rollback requires only selecting the previous provider or removing the new entry.
<!-- END_PHASE_6 -->

## Rollout and rollback

Ship the provider behind explicit configuration rather than automatic endpoint detection. Do not migrate saved `openai` entries. During the first release, log the selected provider ID, provider type, endpoint path, terminal Responses event, and normalized failure category without logging API keys, message bodies, or tool arguments.

Rollback has no data migration:

1. Select the previous `openai` provider in settings.
2. Disable or remove the `openai_responses` entry.
3. If the implementation itself must be reverted, old config files remain readable because unknown inactive provider records can be removed from the UI before downgrade; release notes should recommend removing the entry before installing an older build.

## Acceptance criteria

- Existing configurations deserialize and retain their current provider type and endpoint behavior.
- Existing Chat Completions tests and user workflows continue to pass.
- Responses text, image input, streaming, refusal, errors, and custom function calls map to existing Kokoro behavior.
- No new Tauri chat command or frontend chat event is required.
- `store: false` is present in Responses requests and no remote conversation identifier is persisted.
- A user can configure both protocols at once and switch back without editing JSON.
- Unsupported request options fail explicitly for the Responses provider rather than being silently dropped.
- API keys, prompts, tool arguments, and response bodies are not added to normal logs.

## Deferred work

- OpenAI Conversations API and `previous_response_id` state management.
- Responses WebSocket mode.
- OpenAI built-in web search, file search, computer use, code interpreter, and remote MCP tools.
- Structured Outputs-specific UI.
- Migration of direct VLM, image generation, STT, or TTS endpoints.
- A repository-wide replacement of Chat Completions message types with a new neutral message model.

These items require separate product decisions and are not needed to add the protocol safely.

## References

- [Responses create reference](https://developers.openai.com/api/reference/resources/responses/methods/create)
- [Streaming API responses](https://developers.openai.com/api/docs/guides/streaming-responses)
- [Function calling](https://developers.openai.com/api/docs/guides/function-calling)
- [Conversation state](https://developers.openai.com/api/docs/guides/conversation-state)
