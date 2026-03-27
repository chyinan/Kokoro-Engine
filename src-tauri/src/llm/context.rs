use crate::llm::messages::{assistant_text_message, system_message};
use async_openai::types::chat::ChatCompletionRequestMessage;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct ContextManager {
    pub system_prompt: String,
    pub history: VecDeque<ChatCompletionRequestMessage>, // Rolling window
    pub max_history_len: usize,
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextManager {
    pub fn new() -> Self {
        Self {
            system_prompt: "You are Kokoro, a helpful and friendly virtual assistant.".to_string(),
            history: VecDeque::new(),
            max_history_len: 20, // Default 10 pairs
        }
    }

    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    pub fn add_message(&mut self, role: String, content: String) {
        let message = match role.as_str() {
            "system" | "developer" => system_message(content),
            "assistant" => assistant_text_message(content),
            _ => crate::llm::messages::user_text_message(content),
        };
        self.history.push_back(message);
        while self.history.len() > self.max_history_len {
            self.history.pop_front();
        }
    }

    pub fn get_messages(&self) -> Vec<ChatCompletionRequestMessage> {
        let mut messages = vec![system_message(self.system_prompt.clone())];
        messages.extend(self.history.clone());
        messages
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }
}
