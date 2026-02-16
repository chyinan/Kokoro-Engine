//! Idle Behaviors — random animations when character is bored.

use super::emotion::EmotionState;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "params")]
pub enum IdleBehavior {
    #[serde(rename = "look_around")]
    LookAround { direction: f32, duration_ms: u64 },
    #[serde(rename = "stretch")]
    Stretch,
    #[serde(rename = "hum")]
    Hum { melody_seed: u32 },
    #[serde(rename = "sigh")]
    Sigh,
    #[serde(rename = "fidget")]
    Fidget,
}

pub struct IdleBehaviorSystem {
    last_behavior_ts: std::time::Instant,
}

impl IdleBehaviorSystem {
    pub fn new() -> Self {
        Self {
            last_behavior_ts: std::time::Instant::now(),
        }
    }

    /// Decide if an idle behavior should trigger.
    pub fn decide(&mut self, emotion: &EmotionState, idle_seconds: u64) -> Option<IdleBehavior> {
        // Minimum 10 seconds between behaviors
        if self.last_behavior_ts.elapsed().as_secs() < 10 {
            return None;
        }

        // Only trigger if truly idle (> 15s)
        if idle_seconds < 15 {
            return None;
        }

        // Probability increases with idle time up to a cap
        let base_chance = 0.05; // 5% per tick (assumes 5s tick? No, heartbeat is 30s. So 5% is low.)
                                // Actually heartbeat is 30s. So 0.05 means once every 10 minutes roughly.
                                // Let's make it higher for testing? Or dynamic.

        let mood = emotion.mood();
        let chance = base_chance + (idle_seconds as f32 / 3600.0).min(0.2);

        if rand::random::<f32>() > chance {
            return None;
        }

        self.last_behavior_ts = std::time::Instant::now();

        // Pick behavior based on emotion
        let roll = rand::random::<f32>();

        if mood < 0.3 {
            // Sad/Low energy
            if roll < 0.4 {
                return Some(IdleBehavior::Sigh);
            }
            return Some(IdleBehavior::LookAround {
                direction: 0.0,
                duration_ms: 2000,
            });
        } else if mood > 0.7 {
            // Happy/High energy
            if roll < 0.3 {
                return Some(IdleBehavior::Hum {
                    melody_seed: rand::random(),
                });
            }
            if roll < 0.6 {
                return Some(IdleBehavior::Stretch);
            }
        }

        // Neutral
        if roll < 0.5 {
            return Some(IdleBehavior::LookAround {
                direction: (rand::random::<f32>() - 0.5) * 2.0,
                duration_ms: 1000 + (rand::random::<u64>() % 2000),
            });
        }

        Some(IdleBehavior::Fidget)
    }
}
