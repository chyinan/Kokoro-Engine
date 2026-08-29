# Quick start

This guide gets Kokoro Engine to a first reply through the focused onboarding flow: choose a language, activate a character, connect a provider, run a test, and chat.

## 1. Install Kokoro Engine

Download the installer for your platform from [GitHub Releases](https://github.com/chyinan/Kokoro-Engine/releases). Start the app after installation.

For a source build, install Node.js 18 or newer and stable Rust, then run:

```bash
npm install
npm run tauri dev
```

## 2. Choose a language

On first launch, select the language you want Kokoro to use for the setup and chat experience.

## 3. Choose or import a character

Pick one of the built-in characters from the main-surface catalog, or import a SillyTavern JSON/PNG card. You can edit, duplicate, or restore defaults from the same catalog. Activation applies the character's persona, greeting, visual assets, and voice settings together.

## 4. Connect an LLM

1. In the onboarding provider step, choose an existing provider or add an OpenAI-compatible provider.
2. Enter the endpoint, model, and API key required by that provider.
3. For Ollama, start Ollama first and use its local endpoint.
4. Use **Settings > API** later for advanced provider options.

Provider credentials stay in your local Kokoro configuration. Character cards and MODs don't receive or replace them.

## 5. Test the connection

Run the connection test in onboarding after entering the endpoint, model, and key. If it fails, use **Retry** or **Edit provider**; your configured values remain in the draft.

## 6. Send the first message

Send a short message in the chat step. On a later launch, a dismissed setup stays dismissed; use the **Resume setup** chip on the main surface to continue where you left off. The memory embedding model downloads in the background and never blocks a basic text reply; semantic retrieval becomes available when it is ready.

Voice, vision, MCP, MOD, and bot integrations are optional. Configure them only after basic text chat works.

## Next steps

- [Troubleshooting](troubleshooting.md)
- [Character ecosystem design](design-plans/2026-07-12-user-activation-character-ecosystem.md)
- [Build and architecture notes](architecture.md)
