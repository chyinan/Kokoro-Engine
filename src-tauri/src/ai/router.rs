use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelType {
    Fast,  // e.g. GPT-3.5-Turbo, Haiku, Local 7B
    Smart, // e.g. GPT-4, Opus, Local 70B
    Cheap, // e.g. Local Quantized 7B
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub context_window: usize,
    pub model_type: ModelType,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            id: "gpt-3.5-turbo".to_string(),
            context_window: 16000,
            model_type: ModelType::Fast,
        }
    }
}

pub struct ModelRouter {
    // strict rules or heuristics could be added here
}

impl ModelRouter {
    pub fn new() -> Self {
        Self {}
    }

    pub fn route(&self, query: &str) -> ModelType {
        let q_lower = query.to_lowercase();

        // Heuristics for "Smart" model
        if q_lower.contains("code")
            || q_lower.contains("function")
            || q_lower.contains("analyze")
            || query.len() > 500
        {
            return ModelType::Smart;
        }

        // Default to Fast
        ModelType::Fast
    }
}
