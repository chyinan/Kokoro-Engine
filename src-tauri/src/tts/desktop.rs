use crate::chat::runtime::ChatRuntimeState;
use crate::ipc::wire::emit_to;
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Manager};

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
                let request_id = event.request_id.clone();
                let result = emit_to(&self.app, window, event);
                if result.is_err() {
                    if let Some(runtime) = &self.runtime {
                        runtime.finish_playback(&request_id, window, None);
                    }
                }
                result
            }
            TtsPlaybackEvent::Audio(event) => emit_to(&self.app, window, event),
            TtsPlaybackEvent::End(event) => emit_to(&self.app, window, event),
            TtsPlaybackEvent::Error(event) => emit_to(&self.app, window, event),
            TtsPlaybackEvent::BrowserDelegate(event) => emit_to(&self.app, window, event),
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
