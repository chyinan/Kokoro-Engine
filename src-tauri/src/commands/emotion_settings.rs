use crate::ai::context::AIOrchestrator;
use crate::ai::emotion_settings::{self, EmotionSettings};
use crate::error::KokoroError;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

fn emotion_settings_path() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("emotion_settings.json")
}

#[tauri::command]
pub async fn get_emotion_settings(
    state: State<'_, Arc<RwLock<EmotionSettings>>>,
) -> Result<EmotionSettings, KokoroError> {
    Ok(state.read().await.clone())
}

#[tauri::command]
pub async fn save_emotion_settings(
    settings: EmotionSettings,
    settings_state: State<'_, Arc<RwLock<EmotionSettings>>>,
    orchestrator: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    {
        let mut guard = settings_state.write().await;
        *guard = settings.clone();
    }

    if !settings.enabled {
        let mut emotion = orchestrator.emotion_state.lock().await;
        let personality = emotion.personality().clone();
        emotion.set_personality_with_reset(personality, true);
        drop(emotion);

        if let Err(e) = orchestrator.save_emotion_state().await {
            eprintln!("[Emotion] Failed to persist reset state while disabling emotion: {}", e);
        }
    }

    emotion_settings::save_config(&emotion_settings_path(), &settings)
}
