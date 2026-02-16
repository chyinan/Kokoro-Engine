use serde::Serialize;

#[derive(Serialize)]
pub struct CharacterState {
    pub name: String,
    pub current_expression: String,
    pub mood: f32,
    pub is_speaking: bool,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub text: String,
    pub expression: String,
    pub mood_delta: f32,
}

/// Returns the current character state for Live2D sync.
#[tauri::command]
pub fn get_character_state() -> CharacterState {
    // TODO: Read from actual state manager
    CharacterState {
        name: "Kokoro".to_string(),
        current_expression: "neutral".to_string(),
        mood: 0.5,
        is_speaking: false,
    }
}

/// Sets the character's expression (triggered by UI interaction).
#[tauri::command]
pub fn set_expression(expression: String) -> CharacterState {
    // TODO: Update actual state and trigger Live2D animation
    CharacterState {
        name: "Kokoro".to_string(),
        current_expression: expression,
        mood: 0.5,
        is_speaking: false,
    }
}

/// Sends a user message and returns a placeholder AI response.
/// In Phase 2, this will integrate with the LLM adapter.
#[tauri::command]
pub async fn send_message(message: String) -> Result<ChatResponse, String> {
    if message.trim().is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    // TODO: Phase 2 — route through Context Manager → LLM Adapter
    Ok(ChatResponse {
        text: format!("Echo from Kokoro Engine: {}", message),
        expression: "happy".to_string(),
        mood_delta: 0.1,
    })
}
