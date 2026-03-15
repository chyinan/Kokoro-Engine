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

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn router() -> ModelRouter {
        ModelRouter::new()
    }

    #[test]
    fn test_route_short_casual_returns_fast() {
        assert_eq!(router().route("你好呀"), ModelType::Fast);
    }

    #[test]
    fn test_route_contains_code_returns_smart() {
        assert_eq!(router().route("write some code for me"), ModelType::Smart);
    }

    #[test]
    fn test_route_contains_function_returns_smart() {
        assert_eq!(router().route("explain this function"), ModelType::Smart);
    }

    #[test]
    fn test_route_contains_analyze_returns_smart() {
        assert_eq!(router().route("please analyze this data"), ModelType::Smart);
    }

    #[test]
    fn test_route_long_query_over_500_chars_returns_smart() {
        let long = "a".repeat(501);
        assert_eq!(router().route(&long), ModelType::Smart);
    }

    #[test]
    fn test_route_exactly_500_chars_returns_fast() {
        let boundary = "a".repeat(500);
        assert_eq!(router().route(&boundary), ModelType::Fast);
    }

    #[test]
    fn test_route_keyword_case_insensitive() {
        assert_eq!(router().route("show me some CODE"), ModelType::Smart);
        assert_eq!(router().route("ANALYZE this"), ModelType::Smart);
        assert_eq!(router().route("FUNCTION call"), ModelType::Smart);
    }

    #[test]
    fn test_route_empty_string_returns_fast() {
        assert_eq!(router().route(""), ModelType::Fast);
    }
}
