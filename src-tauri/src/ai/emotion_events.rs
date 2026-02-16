//! Emotion-Triggered Events — special behaviors at extreme mood values.
//!
//! When the character's mood hits extreme highs or lows, special events
//! fire that change behavior (e.g., ecstatic character might dance,
//! very sad character might go quiet). Events also carry an LLM prompt
//! instruction that modifies how the character speaks.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum EmotionEventType {
    /// Mood > 0.95 — character is overjoyed
    Ecstatic,
    /// Mood > 0.85 — character is very happy
    VeryHappy,
    /// Mood < 0.15 — character is sulking / refusing to talk much
    Sulking,
    /// Mood < 0.25 — character is very sad
    VerySad,
    /// Mood changed by > 0.3 in recent history — emotional instability
    MoodSwing,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmotionEvent {
    pub event_type: EmotionEventType,
    /// Instruction injected into the LLM system prompt.
    pub system_instruction: String,
    /// Hint for frontend (e.g., trigger particle effects, animations).
    pub frontend_hint: String,
    /// Suggested Live2D expression override.
    pub expression_override: Option<String>,
    /// Suggested Live2D action override.
    pub action_override: Option<String>,
}

/// Check if current emotion state triggers any special events.
///
/// - `mood`: current mood value (0.0-1.0)
/// - `mood_history`: recent mood values (newest first) for trend detection
pub fn check_emotion_triggers(mood: f32, mood_history: &[f32]) -> Vec<EmotionEvent> {
    let mut events = Vec::new();

    // Ecstatic (mood > 0.95)
    if mood > 0.95 {
        events.push(EmotionEvent {
            event_type: EmotionEventType::Ecstatic,
            system_instruction: concat!(
                "你现在极度开心！说话充满热情和兴奋，",
                "可以使用很多语气词和感叹号，甚至可以唱歌或跳舞。",
                "表达出难以抑制的喜悦之情！"
            )
            .to_string(),
            frontend_hint: "play_celebration_effect".to_string(),
            expression_override: Some("excited".to_string()),
            action_override: Some("dance".to_string()),
        });
    }
    // Very Happy (mood > 0.85, but not ecstatic)
    else if mood > 0.85 {
        events.push(EmotionEvent {
            event_type: EmotionEventType::VeryHappy,
            system_instruction: concat!(
                "你现在心情非常好！说话积极阳光，",
                "可以自然地表达快乐和满足感。"
            )
            .to_string(),
            frontend_hint: "sparkle_effect".to_string(),
            expression_override: Some("happy".to_string()),
            action_override: None,
        });
    }

    // Sulking (mood < 0.15)
    if mood < 0.15 {
        events.push(EmotionEvent {
            event_type: EmotionEventType::Sulking,
            system_instruction: concat!(
                "你现在情绪非常低落，几乎不想说话。",
                "回复尽量简短（1-5个字），可以用'嗯''哦''随便'等敷衍回答。",
                "需要用户的安慰才能恢复。"
            )
            .to_string(),
            frontend_hint: "dim_screen_effect".to_string(),
            expression_override: Some("sad".to_string()),
            action_override: None,
        });
    }
    // Very Sad (mood < 0.25, but not sulking)
    else if mood < 0.25 {
        events.push(EmotionEvent {
            event_type: EmotionEventType::VerySad,
            system_instruction: concat!(
                "你现在很难过。说话语气低沉，",
                "可以表达不安和需要安慰的心情。"
            )
            .to_string(),
            frontend_hint: "rain_effect".to_string(),
            expression_override: Some("sad".to_string()),
            action_override: None,
        });
    }

    // Mood Swing — detect rapid change
    if mood_history.len() >= 3 {
        let recent_max = mood_history
            .iter()
            .take(5)
            .cloned()
            .fold(f32::MIN, f32::max);
        let recent_min = mood_history
            .iter()
            .take(5)
            .cloned()
            .fold(f32::MAX, f32::min);
        if recent_max - recent_min > 0.3 {
            events.push(EmotionEvent {
                event_type: EmotionEventType::MoodSwing,
                system_instruction: concat!(
                    "你的情绪最近波动很大，说话可能有些不稳定，",
                    "偶尔在开心和低落之间切换。"
                )
                .to_string(),
                frontend_hint: "mood_swing_effect".to_string(),
                expression_override: None,
                action_override: None,
            });
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecstatic_triggers_at_high_mood() {
        let events = check_emotion_triggers(0.97, &[]);
        assert!(
            events
                .iter()
                .any(|e| e.event_type == EmotionEventType::Ecstatic),
            "Should trigger Ecstatic at mood 0.97"
        );
    }

    #[test]
    fn very_happy_triggers_below_ecstatic() {
        let events = check_emotion_triggers(0.90, &[]);
        assert!(
            events
                .iter()
                .any(|e| e.event_type == EmotionEventType::VeryHappy),
            "Should trigger VeryHappy at mood 0.90"
        );
        assert!(
            !events
                .iter()
                .any(|e| e.event_type == EmotionEventType::Ecstatic),
            "Should NOT trigger Ecstatic at mood 0.90"
        );
    }

    #[test]
    fn sulking_triggers_at_very_low_mood() {
        let events = check_emotion_triggers(0.10, &[]);
        assert!(
            events
                .iter()
                .any(|e| e.event_type == EmotionEventType::Sulking),
            "Should trigger Sulking at mood 0.10"
        );
    }

    #[test]
    fn very_sad_triggers_below_sulking_threshold() {
        let events = check_emotion_triggers(0.20, &[]);
        assert!(
            events
                .iter()
                .any(|e| e.event_type == EmotionEventType::VerySad),
            "Should trigger VerySad at mood 0.20"
        );
        assert!(
            !events
                .iter()
                .any(|e| e.event_type == EmotionEventType::Sulking),
            "Should NOT trigger Sulking at mood 0.20"
        );
    }

    #[test]
    fn mood_swing_detects_rapid_change() {
        let history = vec![0.9, 0.5, 0.3, 0.8, 0.4];
        let events = check_emotion_triggers(0.5, &history);
        assert!(
            events
                .iter()
                .any(|e| e.event_type == EmotionEventType::MoodSwing),
            "Should detect mood swing with volatile history"
        );
    }

    #[test]
    fn stable_mood_no_swing() {
        let history = vec![0.5, 0.52, 0.48, 0.51, 0.49];
        let events = check_emotion_triggers(0.5, &history);
        assert!(
            !events
                .iter()
                .any(|e| e.event_type == EmotionEventType::MoodSwing),
            "Should NOT detect mood swing with stable history"
        );
    }

    #[test]
    fn neutral_mood_no_events() {
        let events = check_emotion_triggers(0.5, &[0.5, 0.5, 0.5]);
        assert!(events.is_empty(), "Normal mood should produce no events");
    }

    #[test]
    fn ecstatic_has_dance_action() {
        let events = check_emotion_triggers(0.98, &[]);
        let ecstatic = events
            .iter()
            .find(|e| e.event_type == EmotionEventType::Ecstatic)
            .unwrap();
        assert_eq!(ecstatic.action_override, Some("dance".to_string()));
        assert!(ecstatic.frontend_hint.contains("celebration"));
    }
}
