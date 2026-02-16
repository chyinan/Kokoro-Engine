//! Emotion State Machine with per-character personality.
//!
//! Provides smooth emotion transitions with inertia, preventing abrupt mood
//! jumps. Personality traits (inertia, expressiveness, default mood) are
//! parsed from the character's persona text so different characters feel
//! fundamentally different.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

// ── Personality ────────────────────────────────────────────

/// Per-character emotional personality, derived from the persona card text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionPersonality {
    /// How resistant the character is to mood changes (0.0 = volatile, 1.0 = stoic).
    pub inertia: f32,
    /// How strongly the character expresses emotions (0.0 = reserved, 1.0 = dramatic).
    pub expressiveness: f32,
    /// Resting mood when no stimulus is present (0.0 = gloomy, 1.0 = cheerful).
    pub default_mood: f32,
}

impl Default for EmotionPersonality {
    fn default() -> Self {
        Self {
            inertia: 0.4,
            expressiveness: 0.6,
            default_mood: 0.5,
        }
    }
}

impl EmotionPersonality {
    /// Parse personality hints from persona text using keyword detection.
    ///
    /// Supports both Chinese and English keywords. Falls back to defaults
    /// when no keywords match.
    pub fn parse_from_persona(text: &str) -> Self {
        let lower = text.to_lowercase();
        let mut p = Self::default();

        // ── Inertia (resistance to mood change) ──
        // Low inertia = mood changes easily (lively characters)
        let low_inertia_kw = [
            "活泼",
            "元气",
            "开朗",
            "热情",
            "天真",
            "话多",
            "lively",
            "energetic",
            "cheerful",
            "bubbly",
            "hyper",
        ];
        // High inertia = mood is stable (calm characters)
        let high_inertia_kw = [
            "冷静", "沉稳", "冷淡", "高冷", "寡言", "冷酷", "成熟", "理性", "calm", "stoic",
            "reserved", "composed", "serious", "cold",
        ];

        if low_inertia_kw.iter().any(|kw| lower.contains(kw)) {
            p.inertia = 0.2;
        } else if high_inertia_kw.iter().any(|kw| lower.contains(kw)) {
            p.inertia = 0.7;
        }

        // ── Expressiveness (how strongly emotions show) ──
        let high_expr_kw = [
            "表情丰富",
            "夸张",
            "感性",
            "情绪化",
            "大大咧咧",
            "expressive",
            "dramatic",
            "emotional",
            "passionate",
        ];
        let low_expr_kw = [
            "面瘫",
            "扑克脸",
            "不善表达",
            "内敛",
            "害羞",
            "腼腆",
            "expressionless",
            "poker face",
            "introverted",
            "shy",
            "timid",
            "cold",
            "reserved",
        ];

        if high_expr_kw.iter().any(|kw| lower.contains(kw)) {
            p.expressiveness = 0.9;
        } else if low_expr_kw.iter().any(|kw| lower.contains(kw)) {
            p.expressiveness = 0.3;
        }

        // ── Default Mood ──
        let positive_kw = [
            "乐观",
            "阳光",
            "快乐",
            "幸福",
            "optimistic",
            "sunny",
            "happy",
            "upbeat",
        ];
        let negative_kw = [
            "忧郁",
            "悲观",
            "阴沉",
            "孤僻",
            "melancholic",
            "pessimistic",
            "gloomy",
            "brooding",
        ];

        if positive_kw.iter().any(|kw| lower.contains(kw)) {
            p.default_mood = 0.7;
        } else if negative_kw.iter().any(|kw| lower.contains(kw)) {
            p.default_mood = 0.3;
        }

        p
    }
}

// ── Emotion Entry ──────────────────────────────────────────

/// A single emotion record in the history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionEntry {
    pub emotion: String,
    pub mood: f32,
    pub timestamp: i64,
}

// ── Emotion State ──────────────────────────────────────────

/// Tracks the character's emotional state with smooth transitions.
#[derive(Debug, Clone)]
pub struct EmotionState {
    current_emotion: String,
    mood: f32,
    /// Accumulated inertia — increases when the same emotion persists.
    accumulated_inertia: f32,
    personality: EmotionPersonality,
    history: VecDeque<EmotionEntry>,
    max_history: usize,
}

impl EmotionState {
    pub fn new(personality: EmotionPersonality) -> Self {
        let default_mood = personality.default_mood;
        Self {
            current_emotion: "neutral".to_string(),
            mood: default_mood,
            accumulated_inertia: 0.0,
            personality,
            history: VecDeque::new(),
            max_history: 20,
        }
    }

    /// Update emotion with smooth blending based on personality.
    ///
    /// Returns the *smoothed* (emotion, mood) after applying inertia.
    pub fn update(&mut self, raw_emotion: &str, raw_mood: f32) -> (String, f32) {
        let raw_mood = raw_mood.clamp(0.0, 1.0);

        // 1. Calculate effective inertia (base + accumulated)
        let effective_inertia =
            (self.personality.inertia + self.accumulated_inertia * 0.1).clamp(0.0, 0.85);

        // 2. Blend mood: new_mood = old_mood * inertia + raw_mood * (1 - inertia)
        let blended_mood = self.mood * effective_inertia + raw_mood * (1.0 - effective_inertia);

        // 3. Determine final emotion
        //    If the same emotion persists, increase accumulated inertia (harder to change).
        //    If different, decay accumulated inertia.
        let final_emotion = if raw_emotion == self.current_emotion {
            // Same emotion — increase inertia (max 3.0)
            self.accumulated_inertia = (self.accumulated_inertia + 0.5).min(3.0);
            raw_emotion.to_string()
        } else {
            // Different emotion — only switch if mood delta is large enough
            // to overcome the character's resistance
            let mood_delta = (raw_mood - self.mood).abs();
            let switch_threshold = effective_inertia * 0.3;

            if mood_delta > switch_threshold || self.accumulated_inertia < 0.5 {
                // Emotion changes — reset accumulated inertia
                self.accumulated_inertia = 0.0;
                raw_emotion.to_string()
            } else {
                // Resist the change — keep current emotion
                self.accumulated_inertia = (self.accumulated_inertia - 0.3).max(0.0);
                self.current_emotion.clone()
            }
        };

        // 4. Apply expressiveness to mood deviation from default
        let mood_deviation = blended_mood - self.personality.default_mood;
        let expressed_mood =
            self.personality.default_mood + mood_deviation * self.personality.expressiveness;
        let final_mood = expressed_mood.clamp(0.0, 1.0);

        // 5. Update state
        self.current_emotion = final_emotion.clone();
        self.mood = blended_mood; // Store un-expressed mood internally
        self.history.push_back(EmotionEntry {
            emotion: final_emotion.clone(),
            mood: final_mood,
            timestamp: chrono::Utc::now().timestamp(),
        });
        if self.history.len() > self.max_history {
            self.history.pop_front();
        }

        (final_emotion, final_mood)
    }

    /// Generate a natural-language description for system prompt injection.
    pub fn describe(&self) -> String {
        let mood_desc = match self.mood {
            m if m >= 0.8 => "非常好",
            m if m >= 0.6 => "不错",
            m if m >= 0.4 => "一般",
            m if m >= 0.2 => "有些低落",
            _ => "很低落",
        };

        let trend = self.detect_trend();
        let trend_desc = match trend {
            EmotionTrend::Rising => "，而且在好转",
            EmotionTrend::Falling => "，而且在走低",
            EmotionTrend::Stable => "",
        };

        format!(
            "你现在的心情{}{}。当前情绪状态：{}。",
            mood_desc, trend_desc, self.current_emotion
        )
    }

    /// Detect the mood trend from recent history.
    pub fn detect_trend(&self) -> EmotionTrend {
        if self.history.len() < 3 {
            return EmotionTrend::Stable;
        }
        let recent: Vec<f32> = self.history.iter().rev().take(5).map(|e| e.mood).collect();
        let first_half_avg: f32 =
            recent[recent.len() / 2..].iter().sum::<f32>() / (recent.len() / 2) as f32;
        let second_half_avg: f32 =
            recent[..recent.len() / 2].iter().sum::<f32>() / (recent.len() / 2) as f32;

        let delta = second_half_avg - first_half_avg;
        if delta > 0.1 {
            EmotionTrend::Rising
        } else if delta < -0.1 {
            EmotionTrend::Falling
        } else {
            EmotionTrend::Stable
        }
    }

    /// Get the current emotion name.
    pub fn current_emotion(&self) -> &str {
        &self.current_emotion
    }

    /// Get the current (internal, un-expressed) mood value.
    pub fn mood(&self) -> f32 {
        self.mood
    }

    /// Get the personality configuration.
    pub fn personality(&self) -> &EmotionPersonality {
        &self.personality
    }

    /// Get recent mood values from history (newest first) for trend detection.
    pub fn mood_history(&self) -> Vec<f32> {
        self.history.iter().rev().map(|e| e.mood).collect()
    }

    /// Replace personality (when switching characters).
    pub fn set_personality(&mut self, personality: EmotionPersonality) {
        self.personality = personality;
        // Reset to new character's default mood
        self.mood = self.personality.default_mood;
        self.current_emotion = "neutral".to_string();
        self.accumulated_inertia = 0.0;
        self.history.clear();
    }

    /// Decay mood toward the character's default resting mood.
    ///
    /// Called periodically (e.g., every heartbeat tick). The decay rate
    /// controls how fast emotions fade — higher expressiveness = slower decay
    /// (expressive characters hold onto feelings longer).
    pub fn decay_toward_default(&mut self) {
        let decay_rate = 0.05 * (1.0 - self.personality.expressiveness * 0.5);
        let target = self.personality.default_mood;
        let delta = target - self.mood;

        if delta.abs() < 0.01 {
            // Close enough — snap to default
            self.mood = target;
            if self.accumulated_inertia > 0.0 {
                self.accumulated_inertia = (self.accumulated_inertia - 0.1).max(0.0);
            }
            return;
        }

        self.mood += delta * decay_rate;

        // Also decay accumulated inertia
        if self.accumulated_inertia > 0.0 {
            self.accumulated_inertia = (self.accumulated_inertia - 0.05).max(0.0);
        }

        // If mood is very close to default, gradually shift emotion to neutral
        if (self.mood - target).abs() < 0.05 && self.current_emotion != "neutral" {
            self.current_emotion = "neutral".to_string();
        }
    }

    /// Absorb the user's detected sentiment as environmental influence.
    ///
    /// This creates "emotion contagion" — the character's mood is slightly
    /// influenced by the user's emotional state. The effect is scaled by
    /// the character's expressiveness (empathetic characters are more affected).
    pub fn absorb_user_sentiment(&mut self, user_mood: f32, confidence: f32) {
        let user_mood = user_mood.clamp(0.0, 1.0);
        let confidence = confidence.clamp(0.0, 1.0);

        // Influence strength: expressiveness × confidence × 0.15
        // Max influence ≈ 0.135 (very expressive + very confident)
        let influence = self.personality.expressiveness * confidence * 0.15;

        // Pull mood toward user's mood
        self.mood = self.mood * (1.0 - influence) + user_mood * influence;
        self.mood = self.mood.clamp(0.0, 1.0);
    }

    /// Serialize state for persistence across app restarts.
    pub fn snapshot(&self) -> EmotionSnapshot {
        EmotionSnapshot {
            emotion: self.current_emotion.clone(),
            mood: self.mood,
            accumulated_inertia: self.accumulated_inertia,
        }
    }

    /// Restore from a persisted snapshot.
    pub fn restore_from_snapshot(&mut self, snap: &EmotionSnapshot) {
        self.current_emotion = snap.emotion.clone();
        self.mood = snap.mood;
        self.accumulated_inertia = snap.accumulated_inertia;
    }
}

/// Serializable snapshot of emotion state for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionSnapshot {
    pub emotion: String,
    pub mood: f32,
    pub accumulated_inertia: f32,
}

#[derive(Debug, PartialEq)]
pub enum EmotionTrend {
    Rising,
    Falling,
    Stable,
}

// ── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn lively_personality() -> EmotionPersonality {
        EmotionPersonality {
            inertia: 0.2,
            expressiveness: 0.9,
            default_mood: 0.6,
        }
    }

    fn stoic_personality() -> EmotionPersonality {
        EmotionPersonality {
            inertia: 0.7,
            expressiveness: 0.3,
            default_mood: 0.5,
        }
    }

    #[test]
    fn smooth_transition_prevents_mood_jump() {
        let mut state = EmotionState::new(stoic_personality());
        // Start neutral (mood ≈ 0.5), then suddenly get "happy" with mood 1.0
        let (_, mood1) = state.update("happy", 1.0);
        // Stoic character should NOT jump to 1.0
        assert!(
            mood1 < 0.8,
            "Stoic character should resist sudden mood jump, got {}",
            mood1
        );
    }

    #[test]
    fn lively_character_changes_faster() {
        let mut lively = EmotionState::new(lively_personality());
        let mut stoic = EmotionState::new(stoic_personality());

        let (_, lively_mood) = lively.update("happy", 1.0);
        let (_, stoic_mood) = stoic.update("happy", 1.0);

        assert!(
            lively_mood > stoic_mood,
            "Lively character should reach higher mood faster: lively={}, stoic={}",
            lively_mood,
            stoic_mood
        );
    }

    #[test]
    fn accumulated_inertia_resists_change() {
        let mut state = EmotionState::new(EmotionPersonality::default());

        // Build up inertia with 5 happy updates
        for _ in 0..5 {
            state.update("happy", 0.8);
        }

        // Now try a small mood shift to "sad" — close enough to resist
        let (emotion, _) = state.update("sad", 0.65);
        // Should resist the change because inertia is built up
        // and mood delta is small
        assert_eq!(
            emotion, "happy",
            "Should resist emotion change with high accumulated inertia"
        );
    }

    #[test]
    fn strong_stimulus_overcomes_inertia() {
        let mut state = EmotionState::new(EmotionPersonality::default());

        // Build up inertia
        for _ in 0..3 {
            state.update("happy", 0.8);
        }

        // Very strong negative stimulus should overcome inertia
        let (emotion, _) = state.update("angry", 0.1);
        assert_eq!(
            emotion, "angry",
            "Strong stimulus should overcome accumulated inertia"
        );
    }

    #[test]
    fn personality_parse_lively_character() {
        let persona = "她是一个活泼开朗的女孩，总是充满元气，性格乐观阳光。";
        let p = EmotionPersonality::parse_from_persona(persona);
        assert!(
            p.inertia < 0.3,
            "Lively character should have low inertia, got {}",
            p.inertia
        );
        assert!(
            p.default_mood > 0.6,
            "Optimistic character should have high default mood, got {}",
            p.default_mood
        );
    }

    #[test]
    fn personality_parse_stoic_character() {
        let persona = "A calm, composed warrior who rarely shows emotion. Cold and reserved.";
        let p = EmotionPersonality::parse_from_persona(persona);
        assert!(
            p.inertia > 0.5,
            "Stoic character should have high inertia, got {}",
            p.inertia
        );
        assert!(
            p.expressiveness < 0.5,
            "Reserved character should have low expressiveness, got {}",
            p.expressiveness
        );
    }

    #[test]
    fn personality_parse_no_keywords_returns_default() {
        let persona = "A character who lives in a small village.";
        let p = EmotionPersonality::parse_from_persona(persona);
        let d = EmotionPersonality::default();
        assert_eq!(p.inertia, d.inertia);
        assert_eq!(p.expressiveness, d.expressiveness);
        assert_eq!(p.default_mood, d.default_mood);
    }

    #[test]
    fn describe_produces_readable_text() {
        let mut state = EmotionState::new(EmotionPersonality::default());
        state.update("happy", 0.8);
        let desc = state.describe();
        assert!(!desc.is_empty(), "describe() should produce non-empty text");
        assert!(desc.contains("心情"), "describe() should mention mood");
    }

    #[test]
    fn set_personality_resets_state() {
        let mut state = EmotionState::new(lively_personality());
        state.update("happy", 0.9);
        state.update("happy", 0.9);

        // Switch to stoic character
        state.set_personality(stoic_personality());
        assert_eq!(state.current_emotion(), "neutral");
        assert!((state.mood() - 0.5).abs() < 0.01);
    }

    #[test]
    fn decay_moves_toward_default() {
        let mut state = EmotionState::new(EmotionPersonality {
            default_mood: 0.5,
            ..EmotionPersonality::default()
        });
        state.update("happy", 0.9);
        let initial_mood = state.mood();

        // Decay multiple times
        for _ in 0..10 {
            state.decay_toward_default();
        }
        assert!(
            (state.mood() - 0.5).abs() < (initial_mood - 0.5).abs(),
            "Mood should be closer to default after decay: {} vs initial {}",
            state.mood(),
            initial_mood,
        );
    }

    #[test]
    fn decay_eventually_reaches_default() {
        let mut state = EmotionState::new(EmotionPersonality::default());
        state.update("sad", 0.1);

        // Many decay cycles
        for _ in 0..200 {
            state.decay_toward_default();
        }
        assert!(
            (state.mood() - 0.5).abs() < 0.02,
            "Mood should reach default after many decays, got {}",
            state.mood()
        );
    }

    #[test]
    fn absorb_user_sentiment_shifts_mood() {
        let mut state = EmotionState::new(lively_personality());
        // Start neutral-ish
        let initial_mood = state.mood();

        // User is very sad with high confidence
        state.absorb_user_sentiment(0.1, 1.0);

        assert!(
            state.mood() < initial_mood,
            "Mood should decrease from user sadness: {} vs {}",
            state.mood(),
            initial_mood
        );
    }

    #[test]
    fn stoic_character_less_affected_by_user_sentiment() {
        let mut lively = EmotionState::new(lively_personality());
        let mut stoic = EmotionState::new(stoic_personality());

        lively.absorb_user_sentiment(0.1, 1.0);
        stoic.absorb_user_sentiment(0.1, 1.0);

        let lively_shift = (lively.mood() - 0.6).abs();
        let stoic_shift = (stoic.mood() - 0.5).abs();

        assert!(
            lively_shift > stoic_shift,
            "Lively should be more affected: lively_shift={}, stoic_shift={}",
            lively_shift,
            stoic_shift
        );
    }

    #[test]
    fn snapshot_and_restore_preserves_state() {
        let mut state = EmotionState::new(EmotionPersonality::default());
        state.update("happy", 0.85);
        state.update("happy", 0.85);

        let snap = state.snapshot();
        assert_eq!(snap.emotion, "happy");

        // Create new state and restore
        let mut state2 = EmotionState::new(EmotionPersonality::default());
        state2.restore_from_snapshot(&snap);
        assert_eq!(state2.current_emotion(), "happy");
        assert!((state2.mood() - state.mood()).abs() < 0.01);
    }
}
