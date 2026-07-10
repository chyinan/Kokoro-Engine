# OpenAI Responses API

Kokoro Engine supports the OpenAI Responses API as an optional LLM provider. Existing `openai` providers continue to use Chat Completions and are not migrated automatically.

## Configure

1. Open Settings, then API.
2. Add **OpenAI Responses**.
3. Enter an API key or configure `OPENAI_API_KEY` in the environment.
4. Keep the endpoint at `https://api.openai.com/v1` unless you use another Responses-compatible service.
5. Select a model and test the connection.

The provider sends Kokoro's local conversation history with `store: false`. Kokoro remains responsible for conversation storage and tool execution.

## Roll back

Select the existing **OpenAI-Compatible** provider to return to Chat Completions. The two provider entries can coexist, and switching does not rewrite conversation data or existing provider configuration.

Before installing a Kokoro Engine version that predates Responses support, remove or disable the `openai_responses` provider entry.
