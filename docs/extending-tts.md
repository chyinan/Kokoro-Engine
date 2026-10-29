# Extending TTS — Adding a New Backend

> **Version:** 1.1
> **Last Updated:** 2026-10-29

This guide explains how to add a new TTS provider to Kokoro Engine. The system is designed around the `TtsProvider` trait — implement it, register in config, and the engine handles routing, caching, and playback automatically.

## OpenAI Realtime speech output

The existing `provider_type: "openai"` continues to use OpenAI's Speech API for old configurations and compatible gateways. The separate `provider_type: "openai_realtime"` uses the GA Realtime WebSocket protocol with `gpt-realtime-2.1-mini` by default. It does not call `/v1/audio/speech`, listen to a microphone, use tools, or generate a replacement chat answer.

OpenAI's current text-to-speech guide says `tts-1`, `tts-1-hd`, and `gpt-4o-mini-tts` are deprecated with shutdown scheduled for 2027-01-06; it explicitly names `gpt-realtime-2.1-mini` as a Realtime migration target, not a Speech API model.

The Realtime provider uses one short-lived server-side WebSocket per sentence, sends a `response.create` with `conversation: "none"` and only that sentence as `input_text`, and streams `response.output_audio.delta` as 24 kHz mono PCM16 little-endian. Do not share a Realtime connection between independent TTS requests. Keep the instructions in the control field and the text to be read in the user input item; the model is generative, so exact verbatim output is not guaranteed.

The desktop `speak()` path sends those PCM deltas directly to the audio clock/analyser. The separate non-streaming `synthesize_text()` helper wraps PCM as WAV for file-oriented integrations that need a recognizable container; this is not used by desktop playback and is not the streaming path.

The example entry in `config/tts_config.json` is disabled and does not change the default provider. In the Settings → TTS tab, enable the provider, configure `OPENAI_API_KEY` (or enter a key), select a built-in Realtime voice, and save. The API key stays in the Rust backend. Realtime PCM chunks must include the `TtsAudioFormat` metadata; never pass these bytes through MP3 detection or `decodeAudioData`.

---

## Architecture Overview

```
┌─────────────────┐    ┌──────────┐    ┌────────────┐
│  TtsService     │───▶│ TtsRouter│───▶│  Provider  │──▶ Audio
│  (manager.rs)   │    │          │    │            │
│                 │    │ Scoring  │    ├────────────┤
│  Cache ◀──────┐ │    │ Fallback │    │ GPT-SoVITS │
│  Queue        │ │    └──────────┘    │ VITS       │
│               │ │                    │ OpenAI     │
│  speak()──────┘ │                    │ Azure      │
└─────────────────┘                    │ ElevenLabs │
                                       │ Browser    │
                                       │ YourNew    │
                                       └────────────┘
```

---

## Step 1: Create Your Provider Module

Create a new file `src-tauri/src/tts/my_provider.rs`:

```rust
use super::config::ProviderConfig;
use super::interface::{
    Gender, ProviderCapabilities, TtsEngine, TtsError,
    TtsParams, TtsProvider, VoiceProfile,
};
use async_trait::async_trait;

pub struct MyTTSProvider {
    // your fields
}

impl MyTTSProvider {
    pub fn from_config(config: &ProviderConfig) -> Option<Self> {
        // Build your provider from config.
        // Return None if required config is missing (e.g., no API key).
        Some(Self { /* ... */ })
    }
}

#[async_trait]
impl TtsProvider for MyTTSProvider {
    fn id(&self) -> String {
        "my_provider".to_string()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_streaming: false,
            supports_emotions: true,
            supports_speed: true,
            supports_pitch: false,
            supports_cloning: false,
            supports_ssml: false,
        }
    }

    fn voices(&self) -> Vec<VoiceProfile> {
        vec![VoiceProfile {
            voice_id: "my_voice_1".to_string(),
            name: "My Voice".to_string(),
            gender: Gender::Neutral,
            language: "en".to_string(),
            engine: TtsEngine::Cloud, // or Vits, Native
            provider_id: "my_provider".to_string(),
            extra_params: Default::default(),
        }]
    }

    async fn is_available(&self) -> bool {
        // Health check — ping server, check API key, etc.
        true
    }

    async fn synthesize(
        &self,
        text: &str,
        params: TtsParams,
    ) -> Result<Vec<u8>, TtsError> {
        // Your synthesis logic here.
        // Return MP3/WAV/PCM audio bytes on success.
        todo!()
    }
}
```

---

## Step 2: Register the Module

Update `src-tauri/src/tts/mod.rs`:

```rust
pub mod my_provider;
```

---

## Step 3: Register in the Provider Factory

Update `manager.rs` → `build_provider()`:

```rust
fn build_provider(config: &ProviderConfig) -> Option<Arc<dyn TtsProvider>> {
    match config.provider_type.as_str() {
        // ... existing providers ...
        "my_provider" => {
            my_provider::MyTTSProvider::from_config(config)
                .map(|p| Arc::new(p) as Arc<dyn TtsProvider>)
        }
        _ => None,
    }
}
```

---

## Step 4: Add Config Entry

Add to `tts_config.json`:

```json
{
    "id": "my_tts",
    "provider_type": "my_provider",
    "enabled": true,
    "api_key_env": "MY_TTS_API_KEY",
    "endpoint": "http://localhost:8080"
}
```

---

## Key Design Rules

| Rule | Rationale |
|---|---|
| Always implement `is_available()` | Enables smart routing fallback |
| Declare accurate `capabilities()` | Router uses these to match requests |
| Return `TtsError` variants, not strings | Consistent error handling |
| Use `from_config()` constructor | Config-driven loading |
| Return `None` from `from_config()` if config is invalid | Prevents broken providers from registering |
| Declare PCM `AudioFormat` explicitly | Bare PCM must not be guessed as MP3/WAV or passed to `decodeAudioData` |
| Handle `synthesize_stream_for_request()` cancellation when using a remote session | Stop must cancel or close that request's server work |

The OpenAI Realtime provider is intentionally an exception to the missing-key rule: it remains registered with `available: false`, so an explicit selection reports the missing key instead of silently routing to a different provider.

---

## Error Handling

Use the `TtsError` enum:

- `SynthesisFailed(msg)` — Synthesis HTTP/processing error
- `Unavailable(msg)` — Provider is down
- `Timeout(msg)` — Request exceeded time limit
- `ConfigError(msg)` — Invalid configuration

The `TtsService` will catch errors and attempt fallback to the next provider.

---

## Streaming Support (Optional)

Override `synthesize_stream()` if your provider supports chunked audio:

```rust
async fn synthesize_stream(
    &self,
    text: &str,
    params: TtsParams,
) -> Result<Vec<Vec<u8>>, TtsError> {
    // Return multiple audio chunks for incremental playback
    todo!()
}
```

The default implementation wraps `synthesize()` into a single-chunk response.

---

## Testing Your Provider

1. Add config entry with `"enabled": true`
2. Run `cargo check` to verify compilation
3. Start the app: `npm run tauri dev`
4. Open devtools console and call:
   ```js
   await window.__TAURI_INVOKE__("list_tts_providers")
   ```
5. Verify your provider appears and shows `available: true`
