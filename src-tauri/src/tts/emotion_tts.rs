//! Emotion-driven TTS parameter modulation.
//!
//! Maps emotion states to speech parameter adjustments (speed, pitch)
//! so the character's voice sounds expressive and matches their mood.

use serde::{Deserialize, Serialize};

/// Modifiers applied to base TTS parameters based on emotion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionTtsModifiers {
    /// Multiplier for speech speed (1.0 = no change).
    pub speed_factor: f32,
    /// Offset for pitch (0.0 = no change, positive = higher).
    pub pitch_offset: f32,
}

impl Default for EmotionTtsModifiers {
    fn default() -> Self {
        Self {
            speed_factor: 1.0,
            pitch_offset: 0.0,
        }
    }
}

/// Get TTS modifiers for a given emotion and mood value.
///
/// Mood (0.0-1.0) further scales the intensity:
/// high mood amplifies positive adjustments, low mood amplifies negative ones.
pub fn get_modifiers(emotion: &str, mood: f32) -> EmotionTtsModifiers {
    let mood = mood.clamp(0.0, 1.0);

    // Base modifiers per emotion
    let (speed, pitch) = match emotion {
        "joy" => (1.10, 0.05),
        "love" => (0.95, 0.06),
        "sadness" => (0.85, -0.10),
        "anger" => (1.15, -0.05),
        "surprise" => (1.05, 0.10),
        "fear" => (0.93, 0.03),
        _ => (1.0, 0.0), // No modulation
    };

    // Scale intensity by how extreme the mood is (distance from 0.5)
    let intensity = (mood - 0.5).abs() * 2.0; // 0.0 at neutral, 1.0 at extremes
    let scale = 0.5 + intensity * 0.5; // Range: 0.5 to 1.0

    EmotionTtsModifiers {
        speed_factor: 1.0 + (speed - 1.0) * scale,
        pitch_offset: pitch * scale,
    }
}

/// Apply emotion modifiers to base speed and pitch values.
pub fn apply_modifiers(
    base_speed: f32,
    base_pitch: f32,
    modifiers: &EmotionTtsModifiers,
) -> (f32, f32) {
    let final_speed = (base_speed * modifiers.speed_factor).clamp(0.5, 2.0);
    let final_pitch = (base_pitch + modifiers.pitch_offset).clamp(-1.0, 1.0);
    (final_speed, final_pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_returns_no_change() {
        let m = get_modifiers("neutral", 0.5);
        assert!((m.speed_factor - 1.0).abs() < 0.01);
        assert!(m.pitch_offset.abs() < 0.01);
    }

    #[test]
    fn joy_increases_speed_and_pitch() {
        let m = get_modifiers("joy", 0.85);
        assert!(m.speed_factor > 1.0, "Joy should increase speed");
        assert!(m.pitch_offset > 0.0, "Joy should raise pitch");
    }

    #[test]
    fn sadness_decreases_speed_and_pitch() {
        let m = get_modifiers("sadness", 0.2);
        assert!(m.speed_factor < 1.0, "Sadness should decrease speed");
        assert!(m.pitch_offset < 0.0, "Sadness should lower pitch");
    }

    #[test]
    fn extreme_mood_amplifies_effect() {
        let mild = get_modifiers("joy", 0.55);
        let extreme = get_modifiers("joy", 0.95);
        assert!(
            extreme.speed_factor > mild.speed_factor,
            "Extreme mood should amplify speed: extreme={}, mild={}",
            extreme.speed_factor,
            mild.speed_factor
        );
    }

    #[test]
    fn apply_modifiers_clamps_values() {
        let m = EmotionTtsModifiers {
            speed_factor: 5.0,
            pitch_offset: 3.0,
        };
        let (speed, pitch) = apply_modifiers(1.0, 0.0, &m);
        assert!(speed <= 2.0, "Speed should be clamped at 2.0");
        assert!(pitch <= 1.0, "Pitch should be clamped at 1.0");
    }

    #[test]
    fn unknown_emotion_returns_neutral() {
        let m = get_modifiers("confused", 0.5);
        assert!((m.speed_factor - 1.0).abs() < 0.01);
        assert!(m.pitch_offset.abs() < 0.01);
    }

    #[test]
    fn apply_modifiers_lower_bound_clamping() {
        let m = EmotionTtsModifiers {
            speed_factor: 0.1,  // Very low
            pitch_offset: -5.0, // Very negative
        };
        let (speed, pitch) = apply_modifiers(1.0, 0.0, &m);
        assert!(
            speed >= 0.5,
            "Speed should be clamped at 0.5 minimum, got {}",
            speed
        );
        assert!(
            pitch >= -1.0,
            "Pitch should be clamped at -1.0 minimum, got {}",
            pitch
        );
    }

    #[test]
    fn get_modifiers_with_out_of_range_mood_high() {
        let m = get_modifiers("joy", 1.5); // mood > 1.0
        assert!(
            m.speed_factor > 1.0,
            "Should handle mood > 1.0 without panic"
        );
        assert!(
            m.speed_factor <= 2.0,
            "Should still be within reasonable bounds"
        );
    }

    #[test]
    fn get_modifiers_with_out_of_range_mood_low() {
        let m = get_modifiers("sadness", -0.5); // mood < 0.0
        assert!(
            m.speed_factor < 1.0,
            "Should handle mood < 0.0 without panic"
        );
        assert!(
            m.speed_factor >= 0.5,
            "Should still be within reasonable bounds"
        );
    }

    #[test]
    fn get_modifiers_mood_symmetry() {
        let low_mood = get_modifiers("joy", 0.1); // 0.4 away from 0.5
        let high_mood = get_modifiers("joy", 0.9); // 0.4 away from 0.5
        assert!(
            (low_mood.speed_factor - high_mood.speed_factor).abs() < 0.01,
            "Symmetric moods should produce equal intensity"
        );
        assert!(
            (low_mood.pitch_offset - high_mood.pitch_offset).abs() < 0.01,
            "Symmetric moods should produce equal pitch offset"
        );
    }

    #[test]
    fn joy_emotion_spot_check() {
        let m = get_modifiers("joy", 0.8);
        assert!(m.speed_factor > 1.0, "Joy should increase speed");
        assert!(
            m.pitch_offset > 0.0,
            "Joy should have positive pitch offset"
        );
    }

    #[test]
    fn anger_emotion_spot_check() {
        let m = get_modifiers("anger", 0.7);
        assert!(m.speed_factor > 1.0, "Anger should increase speed");
        assert!(
            m.pitch_offset < 0.0,
            "Anger should have negative pitch offset"
        );
    }

    #[test]
    fn surprise_emotion_spot_check() {
        let m = get_modifiers("surprise", 0.75);
        assert!(m.speed_factor > 1.0, "Surprise should increase speed");
        assert!(m.pitch_offset > 0.0, "Surprise should raise pitch");
    }

    #[test]
    fn love_emotion_spot_check() {
        let m = get_modifiers("love", 0.5);
        assert!(m.speed_factor < 1.0, "Love should soften speed");
        assert!(
            m.pitch_offset > 0.0,
            "Love should have a slightly higher pitch"
        );
    }

    #[test]
    fn fear_emotion_spot_check() {
        let m = get_modifiers("fear", 0.4);
        assert!(m.speed_factor < 1.0, "Fear should decrease speed");
        assert!(m.pitch_offset > 0.0, "Fear should raise pitch slightly");
    }
}
