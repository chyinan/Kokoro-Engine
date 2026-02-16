use crate::llm::openai::{Message, MessageContent};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct ContextManager {
    pub system_prompt: String,
    pub history: VecDeque<Message>, // Rolling window
    pub max_history_len: usize,
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
        self.history.push_back(Message {
            role,
            content: MessageContent::Text(content),
        });
        while self.history.len() > self.max_history_len {
            self.history.pop_front();
        }
    }

    pub fn get_messages(&self) -> Vec<Message> {
        let mut messages = vec![Message {
            role: "system".to_string(),
            content: MessageContent::Text(self.system_prompt.clone()),
        }];
        messages.extend(self.history.clone());
        messages
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }
}
