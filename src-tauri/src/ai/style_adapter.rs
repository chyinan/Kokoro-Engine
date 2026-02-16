//! Conversation Style Adapter — adjusts character speaking style by relationship depth.
//!
//! As conversation count grows, the character transitions through formality levels:
//! Stranger → Acquaintance → Friend → Intimate.
//! Emotion also modulates style (happy = more playful, sad = more withdrawn).

use serde::Serialize;

/// Style parameters that influence how the character communicates.
#[derive(Debug, Clone, Serialize)]
pub struct StyleDirective {
    /// Relationship tier name.
    pub tier: RelationshipTier,
    /// Formality level: 0.0 = very casual, 1.0 = very formal.
    pub formality: f32,
    /// Verbosity: 0.0 = terse, 1.0 = elaborate.
    pub verbosity: f32,
    /// Affection level: 0.0 = cold/distant, 1.0 = very warm/intimate.
    pub affection: f32,
    /// Humor level: 0.0 = serious, 1.0 = playful.
    pub humor: f32,
    /// Generated prompt instruction for the LLM.
    pub prompt_instruction: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum RelationshipTier {
    Stranger,     // 0-10 messages
    Acquaintance, // 11-50 messages
    Friend,       // 51-200 messages
    Intimate,     // 200+ messages
}

impl RelationshipTier {
    pub fn from_count(count: u64) -> Self {
        match count {
            0..=10 => Self::Stranger,
            11..=50 => Self::Acquaintance,
            51..=200 => Self::Friend,
            _ => Self::Intimate,
        }
    }
}

/// Compute the style directive based on relationship depth and emotional state.
pub fn compute_style(conversation_count: u64, mood: f32, emotion: &str) -> StyleDirective {
    let tier = RelationshipTier::from_count(conversation_count);

    // Base values from relationship tier
    let (base_formality, base_verbosity, base_affection, base_humor) = match tier {
        RelationshipTier::Stranger => (0.8, 0.4, 0.2, 0.2),
        RelationshipTier::Acquaintance => (0.5, 0.5, 0.4, 0.4),
        RelationshipTier::Friend => (0.3, 0.6, 0.6, 0.6),
        RelationshipTier::Intimate => (0.1, 0.7, 0.9, 0.7),
    };

    // Emotion modifiers
    let (formality_mod, verbosity_mod, affection_mod, humor_mod) = match emotion {
        "happy" | "excited" => (-0.1, 0.1, 0.1, 0.15),
        "sad" => (0.05, -0.1, 0.05, -0.2),
        "angry" => (-0.1, 0.1, -0.15, -0.2),
        "shy" => (0.15, -0.15, 0.05, -0.1),
        "smug" => (-0.15, 0.1, 0.0, 0.2),
        "worried" => (0.1, 0.1, 0.1, -0.15),
        "thinking" => (0.1, 0.15, 0.0, -0.1),
        "surprised" => (-0.1, 0.05, 0.05, 0.1),
        _ => (0.0, 0.0, 0.0, 0.0),
    };

    // Mood modifier — high mood = warmer, low mood = more reserved
    let mood_factor = (mood - 0.5) * 0.2; // -0.1 to +0.1

    let formality = (base_formality + formality_mod - mood_factor).clamp(0.0, 1.0);
    let verbosity = (base_verbosity + verbosity_mod + mood_factor * 0.5).clamp(0.0, 1.0);
    let affection = (base_affection + affection_mod + mood_factor).clamp(0.0, 1.0);
    let humor = (base_humor + humor_mod + mood_factor).clamp(0.0, 1.0);

    // Generate prompt instruction
    let prompt_instruction = generate_prompt(tier, formality, affection, humor);

    StyleDirective {
        tier,
        formality,
        verbosity,
        affection,
        humor,
        prompt_instruction,
    }
}

fn generate_prompt(tier: RelationshipTier, formality: f32, affection: f32, humor: f32) -> String {
    let mut parts = Vec::new();

    // Relationship context
    match tier {
        RelationshipTier::Stranger => {
            parts.push("你和用户刚认识，保持适当的礼貌和距离感。".to_string());
        }
        RelationshipTier::Acquaintance => {
            parts.push("你和用户已经有些熟悉了，说话可以自然一些。".to_string());
        }
        RelationshipTier::Friend => {
            parts.push("你和用户已经是好朋友了，说话亲近自然，可以开玩笑。".to_string());
        }
        RelationshipTier::Intimate => {
            parts
                .push("你和用户非常亲密，说话可以很随意、亲密，可以撒娇或者使用昵称。".to_string());
        }
    }

    // Formality hint
    if formality < 0.3 {
        parts.push("用轻松随意的语气说话。".to_string());
    } else if formality > 0.7 {
        parts.push("用较为正式礼貌的语气说话。".to_string());
    }

    // Affection hint
    if affection > 0.7 {
        parts.push("表达出对用户的关心和在意。".to_string());
    }

    // Humor hint
    if humor > 0.6 {
        parts.push("适当加入俏皮或幽默的元素。".to_string());
    } else if humor < 0.2 {
        parts.push("保持认真的语气。".to_string());
    }

    parts.join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stranger_is_formal() {
        let style = compute_style(5, 0.5, "neutral");
        assert_eq!(style.tier, RelationshipTier::Stranger);
        assert!(
            style.formality > 0.6,
            "Stranger should be formal, got {}",
            style.formality
        );
        assert!(
            style.affection < 0.4,
            "Stranger should have low affection, got {}",
            style.affection
        );
    }

    #[test]
    fn intimate_is_casual() {
        let style = compute_style(500, 0.7, "happy");
        assert_eq!(style.tier, RelationshipTier::Intimate);
        assert!(
            style.formality < 0.3,
            "Intimate should be casual, got {}",
            style.formality
        );
        assert!(
            style.affection > 0.7,
            "Intimate should be affectionate, got {}",
            style.affection
        );
    }

    #[test]
    fn happy_mood_increases_humor() {
        let neutral = compute_style(100, 0.5, "neutral");
        let happy = compute_style(100, 0.5, "happy");
        assert!(
            happy.humor > neutral.humor,
            "Happy should increase humor: {} vs {}",
            happy.humor,
            neutral.humor
        );
    }

    #[test]
    fn sad_decreases_humor() {
        let neutral = compute_style(100, 0.5, "neutral");
        let sad = compute_style(100, 0.5, "sad");
        assert!(
            sad.humor < neutral.humor,
            "Sad should decrease humor: {} vs {}",
            sad.humor,
            neutral.humor
        );
    }

    #[test]
    fn tier_progression() {
        assert_eq!(RelationshipTier::from_count(0), RelationshipTier::Stranger);
        assert_eq!(
            RelationshipTier::from_count(30),
            RelationshipTier::Acquaintance
        );
        assert_eq!(RelationshipTier::from_count(100), RelationshipTier::Friend);
        assert_eq!(
            RelationshipTier::from_count(300),
            RelationshipTier::Intimate
        );
    }

    #[test]
    fn prompt_contains_relationship_context() {
        let style = compute_style(5, 0.5, "neutral");
        assert!(
            style.prompt_instruction.contains("刚认识"),
            "Stranger prompt should mention new relationship"
        );

        let style2 = compute_style(300, 0.8, "happy");
        assert!(
            style2.prompt_instruction.contains("亲密"),
            "Intimate prompt should mention closeness"
        );
    }

    #[test]
    fn values_clamped() {
        // Extreme values shouldn't break
        let style = compute_style(1000, 1.0, "excited");
        assert!(style.formality >= 0.0 && style.formality <= 1.0);
        assert!(style.affection >= 0.0 && style.affection <= 1.0);
        assert!(style.humor >= 0.0 && style.humor <= 1.0);

        let style2 = compute_style(0, 0.0, "angry");
        assert!(style2.formality >= 0.0 && style2.formality <= 1.0);
        assert!(style2.affection >= 0.0 && style2.affection <= 1.0);
    }
}
