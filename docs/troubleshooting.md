# Troubleshooting

## The app opens but chat doesn't reply

Open **Settings > API** and run the connection test. Check the endpoint, model name, API key, and whether the provider is reachable from the same machine.

For Ollama, confirm the Ollama service is running and the selected model has been downloaded.

## The memory model download fails

Retry from the memory model dialog and check free disk space, proxy settings, and network access. In the current release, semantic memory may be unavailable until this model is installed. Non-blocking first chat is part of the in-progress activation work.

## A character import fails

Kokoro accepts SillyTavern JSON cards and PNG cards with embedded `chara` metadata. A normal PNG portrait without character metadata isn't a character card.

Don't import files from untrusted sources if they contain content you don't want included in prompts.

## Live2D doesn't appear

Switch back to the built-in model in **Settings > Model**. Imported models must include a valid Cubism `model3.json` file and all referenced assets.

## Voice output is silent

Check that TTS is enabled, the selected provider is available, and the voice exists for that provider. Browser TTS also depends on voices installed by the operating system.

## A bot or webhook can't reach Kokoro

Confirm the bot runtime is enabled and the bind host, port, endpoint path, and Bearer token match the client. `127.0.0.1` is reachable only from the same host; containers need an address that resolves back to the host.

## Report a problem

Use the [support form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=support.yml) for setup help or the [bug form](https://github.com/chyinan/Kokoro-Engine/issues/new?template=bug_report.yml) for reproducible defects. Remove API keys and other secrets from logs before posting.
