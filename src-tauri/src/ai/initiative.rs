//! Initiative System — decides when and how the AI should proactively engage.
//!
//! Uses curiosity queue + emotion state + relationship depth + time context
//! to determine if the AI should speak up when idle.

use super::curiosity::CuriosityModule;
use super::emotion::EmotionState;

#[derive(Debug, Clone)]
pub enum InitiativeDecision {
    AskQuestion { topic: String },
    ShareThought { topic: String },
    VideoShare { keyword: String }, // For future expansion
    StayQuiet,
}

pub struct InitiativeSystem {
    last_action_ts: std::time::Instant,
}

impl InitiativeSystem {
    pub fn new() -> Self {
        Self {
            last_action_ts: std::time::Instant::now(),
        }
    }

    pub fn decide(
        &mut self,
        curiosity: &mut CuriosityModule,
        emotion: &EmotionState,
        conversation_count: u64,
        idle_seconds: u64,
    ) -> InitiativeDecision {
        // Cooldown check (default 5 minutes)
        if self.last_action_ts.elapsed().as_secs() < 300 {
            return InitiativeDecision::StayQuiet;
        }

        // Base probability based on relationship
        let mut prob = match conversation_count {
            0..=10 => 0.1,   // Shy/Polite
            11..=50 => 0.2,  // Friendly
            51..=200 => 0.4, // Close
            _ => 0.6,        // Intimate
        };

        // Modulate by emotion
        let mood = emotion.mood(); // 0.0 - 1.0
        prob *= 0.5 + mood; // Happy (1.0) -> 1.5x, Sad (0.0) -> 0.5x

        // Expressiveness factor
        prob *= 0.5 + (emotion.personality().expressiveness * 0.5);

        // Idle time factor (longer idle = higher chance, up to a point)
        if idle_seconds > 600 {
            prob *= 1.2;
        } else if idle_seconds < 60 {
            return InitiativeDecision::StayQuiet; // Too soon
        }

        // Roll dice
        let roll: f32 = rand::random();
        if roll > prob {
            return InitiativeDecision::StayQuiet;
        }

        // Success! Decide WHAT to do
        self.last_action_ts = std::time::Instant::now();

        // 1. Check curiosity queue
        if let Some(item) = curiosity.pick_topic() {
            if item.source == "memory" {
                return InitiativeDecision::AskQuestion { topic: item.topic };
            } else {
                return InitiativeDecision::ShareThought { topic: item.topic };
            }
        }

        // 2. Fallback: Generic topic based on context (handled by caller if StayQuiet)
        // Actually, let's just return ShareThought with "random" to let LLM decide
        InitiativeDecision::ShareThought {
            topic: "random".to_string(),
        }
    }
}
