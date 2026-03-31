//! Idle Behaviors — random animations when character is bored.

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

impl Default for IdleBehaviorSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl IdleBehaviorSystem {
    pub fn new() -> Self {
        Self {
            last_behavior_ts: std::time::Instant::now(),
        }
    }

    /// Decide if an idle behavior should trigger.
    pub fn decide(&mut self, idle_seconds: u64) -> Option<IdleBehavior> {
        // Minimum 10 seconds between behaviors
        if self.last_behavior_ts.elapsed().as_secs() < 10 {
            return None;
        }

        // Only trigger if truly idle (> 15s)
        if idle_seconds < 15 {
            return None;
        }

        let chance = 0.05 + (idle_seconds as f32 / 3600.0).min(0.2);

        if rand::random::<f32>() > chance {
            return None;
        }

        self.last_behavior_ts = std::time::Instant::now();

        let roll = rand::random::<f32>();

        if roll < 0.5 {
            return Some(IdleBehavior::LookAround {
                direction: (rand::random::<f32>() - 0.5) * 2.0,
                duration_ms: 1000 + (rand::random::<u64>() % 2000),
            });
        }

        Some(IdleBehavior::Fidget)
    }
}
