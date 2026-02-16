//! Dynamic Expression Driver — generates real-time Live2D parameters.
//!
//! Converts the character's emotional state into granular animation
//! parameters that can drive Live2D model expressions. Emitted every
//! heartbeat tick for smooth, continuous expression animation.

use serde::Serialize;

/// A single frame of expression parameters for Live2D.
#[derive(Debug, Clone, Serialize)]
pub struct ExpressionFrame {
    /// Current primary emotion name.
    pub emotion: String,
    /// Overall mood value (0.0-1.0).
    pub mood: f32,
    /// Mood trend direction.
    pub trend: String,
    /// Expression intensity (0.0 = subtle, 1.0 = exaggerated).
    pub intensity: f32,
    /// Micro-expression parameters for fine-grained animation.
    pub micro: MicroExpressions,
}

/// Fine-grained facial parameters for Live2D model control.
#[derive(Debug, Clone, Serialize)]
pub struct MicroExpressions {
    /// Blink frequency modifier (0.0 = rarely, 1.0 = frequently).
    pub blink_rate: f32,
    /// Mouth curve: -1.0 = frown, 0.0 = neutral, 1.0 = smile.
    pub mouth_curve: f32,
    /// Eyebrow position: -1.0 = furrowed, 0.0 = neutral, 1.0 = raised.
    pub eyebrow_position: f32,
    /// Eye openness: 0.0 = squinting, 0.5 = normal, 1.0 = wide.
    pub eye_openness: f32,
    /// Head tilt: -1.0 = left, 0.0 = center, 1.0 = right.
    pub head_tilt: f32,
}

/// Compute an expression frame from the current emotional state.
pub fn compute_expression_frame(
    emotion: &str,
    mood: f32,
    trend: &str,
    expressiveness: f32,
) -> ExpressionFrame {
    // Base micro-expressions from emotion
    let (blink, mouth, eyebrow, eye_open, tilt) = match emotion {
        "happy" => (0.4, 0.7, 0.3, 0.6, 0.1),
        "excited" => (0.6, 0.9, 0.5, 0.8, 0.2),
        "sad" => (0.3, -0.5, -0.3, 0.3, -0.1),
        "angry" => (0.2, -0.3, -0.7, 0.7, 0.0),
        "surprised" => (0.1, 0.3, 0.8, 1.0, 0.0),
        "thinking" => (0.3, 0.0, 0.4, 0.4, 0.3),
        "shy" => (0.7, 0.2, -0.1, 0.3, -0.2),
        "smug" => (0.3, 0.4, 0.2, 0.5, 0.15),
        "worried" => (0.5, -0.2, 0.5, 0.5, -0.1),
        "neutral" => (0.3, 0.0, 0.0, 0.5, 0.0),
        _ => (0.3, 0.0, 0.0, 0.5, 0.0),
    };

    // Mood modulation — pushes micro-expressions toward mood extremes
    let mood_mod = (mood - 0.5) * 0.3;
    let mouth_final = (mouth + mood_mod).clamp(-1.0, 1.0);
    let eyebrow_final = (eyebrow + mood_mod * 0.5).clamp(-1.0, 1.0);

    // Intensity from expressiveness + mood distance from neutral
    let mood_distance = (mood - 0.5).abs() * 2.0;
    let intensity = (expressiveness * 0.6 + mood_distance * 0.4).clamp(0.0, 1.0);

    // Scale all parameters by intensity
    let scale = 0.5 + intensity * 0.5; // Range: 0.5–1.0

    ExpressionFrame {
        emotion: emotion.to_string(),
        mood,
        trend: trend.to_string(),
        intensity,
        micro: MicroExpressions {
            blink_rate: (blink * scale).clamp(0.0, 1.0),
            mouth_curve: (mouth_final * scale).clamp(-1.0, 1.0),
            eyebrow_position: (eyebrow_final * scale).clamp(-1.0, 1.0),
            eye_openness: (eye_open * scale).clamp(0.0, 1.0),
            head_tilt: (tilt * scale).clamp(-1.0, 1.0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_character_smiles() {
        let frame = compute_expression_frame("happy", 0.8, "rising", 0.7);
        assert!(
            frame.micro.mouth_curve > 0.3,
            "Happy should smile, got {}",
            frame.micro.mouth_curve
        );
    }

    #[test]
    fn sad_character_frowns() {
        let frame = compute_expression_frame("sad", 0.2, "falling", 0.7);
        assert!(
            frame.micro.mouth_curve < 0.0,
            "Sad should frown, got {}",
            frame.micro.mouth_curve
        );
    }

    #[test]
    fn surprised_has_wide_eyes() {
        let frame = compute_expression_frame("surprised", 0.6, "stable", 0.8);
        assert!(
            frame.micro.eye_openness > 0.7,
            "Surprised should have wide eyes, got {}",
            frame.micro.eye_openness
        );
    }

    #[test]
    fn shy_blinks_more() {
        let shy = compute_expression_frame("shy", 0.5, "stable", 0.7);
        let neutral = compute_expression_frame("neutral", 0.5, "stable", 0.7);
        assert!(
            shy.micro.blink_rate > neutral.micro.blink_rate,
            "Shy should blink more: {} vs {}",
            shy.micro.blink_rate,
            neutral.micro.blink_rate
        );
    }

    #[test]
    fn expressive_has_higher_intensity() {
        let low = compute_expression_frame("happy", 0.8, "stable", 0.2);
        let high = compute_expression_frame("happy", 0.8, "stable", 0.9);
        assert!(
            high.intensity > low.intensity,
            "Higher expressiveness = higher intensity: {} vs {}",
            high.intensity,
            low.intensity
        );
    }

    #[test]
    fn all_values_in_range() {
        // Test a variety of inputs
        for emotion in &["happy", "sad", "angry", "excited", "shy", "neutral"] {
            for mood in &[0.0, 0.5, 1.0] {
                let frame = compute_expression_frame(emotion, *mood, "stable", 0.5);
                assert!(frame.micro.blink_rate >= 0.0 && frame.micro.blink_rate <= 1.0);
                assert!(frame.micro.mouth_curve >= -1.0 && frame.micro.mouth_curve <= 1.0);
                assert!(
                    frame.micro.eyebrow_position >= -1.0 && frame.micro.eyebrow_position <= 1.0
                );
                assert!(frame.micro.eye_openness >= 0.0 && frame.micro.eye_openness <= 1.0);
                assert!(frame.micro.head_tilt >= -1.0 && frame.micro.head_tilt <= 1.0);
            }
        }
    }
}
