use crate::chat::runtime::ChatRuntimeState;
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter, Manager};

use super::{TtsPlayback, TtsPlaybackEvent};

pub(crate) struct TauriTtsPlayback {
    app: AppHandle,
    window: OnceLock<&'static str>,
    runtime: Option<Arc<ChatRuntimeState>>,
}

impl TauriTtsPlayback {
    pub(crate) fn new(app: &AppHandle, runtime: Option<Arc<ChatRuntimeState>>) -> Self {
        Self {
            app: app.clone(),
            window: OnceLock::new(),
            runtime,
        }
    }
}

impl TtsPlayback for TauriTtsPlayback {
    fn emit(&self, event: TtsPlaybackEvent) -> Result<(), String> {
        // The service emits Start first, after resolving selection and request ownership.
        let window = playback_window(&self.window, || {
            self.app.get_webview_window("main").is_some()
        });
        let result = match event {
            TtsPlaybackEvent::Start(event) => {
                if let Some(runtime) = &self.runtime {
                    runtime.begin_playback(&event.request_id, window);
                }
                let result = self.app.emit_to(window, "tts:start", &event);
                if result.is_err() {
                    if let Some(runtime) = &self.runtime {
                        runtime.finish_playback(&event.request_id, window, None);
                    }
                }
                result
            }
            TtsPlaybackEvent::Audio(event) => self.app.emit_to(window, "tts:audio", event),
            TtsPlaybackEvent::End(event) => self.app.emit_to(window, "tts:end", event),
            TtsPlaybackEvent::Error(event) => self.app.emit_to(window, "tts:error", event),
            TtsPlaybackEvent::BrowserDelegate(event) => {
                self.app.emit_to(window, "tts:browser-delegate", event)
            }
        };
        result.map_err(|error| error.to_string())
    }
}

fn playback_window(
    target: &OnceLock<&'static str>,
    main_is_open: impl FnOnce() -> bool,
) -> &'static str {
    target.get_or_init(|| if main_is_open() { "main" } else { "pet" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_chooses_its_window_at_start_and_retains_it() {
        let target = OnceLock::new();
        assert!(target.get().is_none());
        assert_eq!(playback_window(&target, || false), "pet");
        assert_eq!(
            playback_window(&target, || panic!("the request already owns its target")),
            "pet"
        );
        assert_eq!(playback_window(&OnceLock::new(), || true), "main");
    }
}
