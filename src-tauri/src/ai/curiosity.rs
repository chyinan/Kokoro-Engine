//! Curiosity Module — generates interest topics from context.
//!
//! Scans recent memories and conversation for unresolved questions or topics.
//! Maintains a queue of "curiosity items" that the AI wants to explore.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuriosityItem {
    pub topic: String,
    pub relevance: f32, // 0.0 - 1.0
    pub source: String, // "memory", "conversation", "random"
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuriosityModule {
    queue: VecDeque<CuriosityItem>,
}

impl CuriosityModule {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// Add a new topic to the curiosity queue.
    pub fn add_topic(&mut self, topic: &str, relevance: f32, source: &str) {
        // Avoid duplicates
        if self.queue.iter().any(|i| i.topic == topic) {
            return;
        }

        let item = CuriosityItem {
            topic: topic.to_string(),
            relevance,
            source: source.to_string(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        self.queue.push_back(item);

        // Keep queue size manageable
        if self.queue.len() > 10 {
            self.queue.pop_front();
        }
    }

    /// Pick the most relevant topic to talk about.
    pub fn pick_topic(&mut self) -> Option<CuriosityItem> {
        if self.queue.is_empty() {
            return None;
        }

        // Simple strategy: pick highest relevance, remove it
        // In future: probabilistic pick based on relevance + age

        // Find index of max relevance
        let mut max_idx = 0;
        let mut max_rel = -1.0;

        for (i, item) in self.queue.iter().enumerate() {
            if item.relevance > max_rel {
                max_rel = item.relevance;
                max_idx = i;
            }
        }

        self.queue.remove(max_idx)
    }

    /// Decay relevance over time (call periodically).
    pub fn decay(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Remove old items (> 24 hours)
        self.queue.retain(|i| now - i.created_at < 86400);

        // Decay relevance
        for item in &mut self.queue {
            item.relevance *= 0.95;
        }
    }
}
