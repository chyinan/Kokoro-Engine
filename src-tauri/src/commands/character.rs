use crate::ai::context::AIOrchestrator;
use crate::commands::live2d::load_active_live2d_profile;
use crate::error::KokoroError;
use serde::Serialize;
use tauri::{Emitter, State};

#[derive(Serialize)]
pub struct CharacterState {
    pub name: String,
    pub current_cue: String,
    pub mood: f32,
    pub is_speaking: bool,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub text: String,
    pub cue: String,
    pub mood_delta: f32,
}

fn resolve_default_cue() -> String {
    if let Some(profile) = load_active_live2d_profile() {
        if profile.cue_map.contains_key("neutral") {
            return "neutral".to_string();
        }
        if let Some(first) = profile.cue_map.keys().next() {
            return first.clone();
        }
    }
    "neutral".to_string()
}

/// Returns the current character state for Live2D sync.
#[tauri::command]
pub async fn get_character_state(
    state: State<'_, AIOrchestrator>,
) -> Result<CharacterState, KokoroError> {
    let name = state.get_character_id().await;

    Ok(CharacterState {
        name,
        current_cue: resolve_default_cue(),
        mood: 0.5,
        is_speaking: false,
    })
}

#[tauri::command]
pub async fn play_cue(
    app: tauri::AppHandle,
    state: State<'_, AIOrchestrator>,
    cue: String,
) -> Result<CharacterState, KokoroError> {
    let trimmed = cue.trim();
    if trimmed.is_empty() {
        return Err(KokoroError::Validation("Cue cannot be empty".to_string()));
    }

    let profile = load_active_live2d_profile().ok_or_else(|| {
        KokoroError::Validation("No active Live2D model profile loaded".to_string())
    })?;

    if !profile.cue_map.contains_key(trimmed) {
        let available_cues = profile
            .cue_map
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(KokoroError::Validation(format!(
            "Unknown cue '{}'. Available configured cues: {}",
            trimmed,
            if available_cues.is_empty() {
                "(none)"
            } else {
                &available_cues
            }
        )));
    }

    let _ = app.emit(
        "chat-cue",
        serde_json::json!({
            "cue": trimmed,
            "source": "manual",
        }),
    );

    let name = state.get_character_id().await;
    Ok(CharacterState {
        name,
        current_cue: trimmed.to_string(),
        mood: 0.5,
        is_speaking: false,
    })
}

/// Legacy command kept for compatibility.
/// Real chat flow must use `stream_chat`.
#[tauri::command]
pub async fn send_message(message: String) -> Result<ChatResponse, KokoroError> {
    if message.trim().is_empty() {
        return Err(KokoroError::Validation(
            "Message cannot be empty".to_string(),
        ));
    }

    Err(KokoroError::Validation(
        "send_message is deprecated. Use stream_chat for real responses.".to_string(),
    ))
}
