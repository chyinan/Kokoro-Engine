# Quick start

This guide gets the current Kokoro Engine release to its first reply. The character-first onboarding described in the design plan is still being implemented.

## 1. Install Kokoro Engine

Download the installer for your platform from [GitHub Releases](https://github.com/chyinan/Kokoro-Engine/releases). Start the app after installation.

For a source build, install Node.js 18 or newer and stable Rust, then run:

```bash
npm install
npm run tauri dev
```

## 2. Connect an LLM

1. Open **Settings > API**.
2. Choose an existing provider or add an OpenAI-compatible provider.
3. Enter the endpoint, model, and API key required by that provider.
4. For Ollama, start Ollama first and use its local endpoint.
5. Run the connection test before leaving the page.

Provider credentials stay in your local Kokoro configuration. Character cards and MODs don't receive or replace them.

## 3. Choose or import a character

Open **Settings > Persona**. Select the existing character, create one, or import a SillyTavern JSON/PNG card. Imported cards are editable copies; conversations and memories are stored under the character ID.

## 4. Send the first message

Return to the main screen, open chat, and send a short message. The current release may ask you to install the memory embedding model before semantic memory is available.

Voice, vision, MCP, MOD, and bot integrations are optional. Configure them only after basic text chat works.

## Next steps

- [Troubleshooting](troubleshooting.md)
- [Character ecosystem design](design-plans/2026-07-12-user-activation-character-ecosystem.md)
- [Build and architecture notes](architecture.md)
