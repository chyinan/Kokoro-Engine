use crate::error::KokoroError;
use serde::Serialize;
use tauri::Emitter;

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

/// Returns the current character state for Live2D sync.
#[tauri::command]
pub fn get_character_state() -> CharacterState {
    // TODO: Read from actual state manager
    CharacterState {
        name: "Kokoro".to_string(),
        current_cue: "neutral".to_string(),
        mood: 0.5,
        is_speaking: false,
    }
}

#[tauri::command]
pub fn play_cue(app: tauri::AppHandle, cue: String) -> CharacterState {
    let trimmed = cue.trim();
    if !trimmed.is_empty() {
        let _ = app.emit(
            "chat-cue",
            serde_json::json!({
                "cue": trimmed,
                "source": "manual",
            }),
        );
    }

    CharacterState {
        name: "Kokoro".to_string(),
        current_cue: cue,
        mood: 0.5,
        is_speaking: false,
    }
}

/// Sends a user message and returns a placeholder AI response.
/// In Phase 2, this will integrate with the LLM adapter.
#[tauri::command]
pub async fn send_message(message: String) -> Result<ChatResponse, KokoroError> {
    if message.trim().is_empty() {
        return Err(KokoroError::Validation(
            "Message cannot be empty".to_string(),
        ));
    }
    Ok(ChatResponse {
        text: format!("Echo from Kokoro Engine: {}", message),
        cue: "joy".to_string(),
        mood_delta: 0.1,
    })
}
