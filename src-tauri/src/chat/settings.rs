use crate::ai::context::AIOrchestrator;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub struct ContextSettings {
    pub strategy: String,
    pub max_message_chars: usize,
}

impl ContextSettings {
    pub(crate) async fn current(state: &AIOrchestrator) -> Self {
        let (strategy, max_message_chars) = state.get_context_settings().await;
        Self {
            strategy,
            max_message_chars,
        }
    }

    pub(crate) async fn save(self, state: &AIOrchestrator, path: &Path) {
        let settings = Self {
            strategy: if self.strategy == "summary" {
                "summary"
            } else {
                "window"
            }
            .into(),
            max_message_chars: self.max_message_chars.clamp(100, 50_000),
        };
        state
            .set_context_settings(settings.strategy.clone(), settings.max_message_chars)
            .await;
        if let Err(error) = crate::config::save_json_config(path, &settings, "CONTEXT") {
            tracing::error!(target: "context", "[Context] Failed to persist context_settings: {error}");
        }
    }

    pub(crate) async fn restore(state: &AIOrchestrator, path: &Path) {
        let settings: Option<serde_json::Value> = crate::config::load_json_config(path, "CONTEXT");
        if let Some(settings) = settings {
            let strategy = settings
                .get("strategy")
                .and_then(|value| value.as_str())
                .unwrap_or("window");
            let max = settings
                .get("max_message_chars")
                .and_then(|value| value.as_u64())
                .unwrap_or(2000) as usize;
            state.set_context_settings(strategy.to_owned(), max).await;
            tracing::info!(target: "ai", "Restored context_settings: strategy={strategy}, max_chars={max}");
        }
    }
}
