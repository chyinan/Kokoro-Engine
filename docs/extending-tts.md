# Extending TTS — Adding a New Backend

> **Version:** 1.1
> **Last Updated:** 2026-03-01

This guide explains how to add a new TTS provider to Kokoro Engine. The system is designed around the `TtsProvider` trait — implement it, register in config, and the engine handles routing, caching, and playback automatically.

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
                                       │ RVC        │
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
            engine: TtsEngine::Cloud, // or Vits, Rvc, Native
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
fn build_provider(config: &ProviderConfig) -> Option<Box<dyn TtsProvider>> {
    match config.provider_type.as_str() {
        // ... existing providers ...
        "my_provider" => {
            my_provider::MyTTSProvider::from_config(config)
                .map(|p| Box::new(p) as Box<dyn TtsProvider>)
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
