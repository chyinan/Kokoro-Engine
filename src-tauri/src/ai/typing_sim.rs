//! Typing Simulation — variable pre-response delays for realism.
//!
//! Before the first `chat-delta` arrives, emit a `chat-typing` event
//! with a duration that varies based on character personality, emotion,
//! and estimated response complexity. Makes the character feel like
//! they're actually thinking before speaking.

use serde::Serialize;

/// Typing simulation parameters.
#[derive(Debug, Clone, Serialize)]
pub struct TypingParams {
    /// How long the typing indicator should show (milliseconds).
    pub duration_ms: u64,
    /// Typing speed description for frontend animation.
    pub speed: TypingSpeed,
}

#[derive(Debug, Clone, Serialize)]
pub enum TypingSpeed {
    Instant,  // < 300ms
    Fast,     // 300-800ms
    Normal,   // 800-2000ms
    Slow,     // 2000-4000ms
    Thinking, // 4000ms+ (complex questions)
}

/// Calculate typing simulation parameters.
///
/// Factors:
/// - `emotion`: current character emotion
/// - `mood`: current mood value (0.0-1.0)
/// - `expressiveness`: character expressiveness (0.0-1.0)
/// - `user_message_len`: length of user's message in chars
/// - `is_question`: whether the user asked a question
pub fn calculate_typing_delay(
    emotion: &str,
    _mood: f32,
    expressiveness: f32,
    user_message_len: usize,
    is_question: bool,
) -> TypingParams {
    // Base delay from message length (longer message = longer thinking)
    let length_factor = (user_message_len as f64 / 50.0).clamp(0.5, 3.0);

    // Emotion modifier
    let emotion_factor: f64 = match emotion {
        "excited" => 0.4,
        "happy" => 0.6,
        "angry" => 0.5,
        "thinking" => 2.0,
        "shy" => 1.5,
        "worried" => 1.3,
        "neutral" => 1.0,
        "sad" => 1.4,
        "smug" => 0.7,
        "surprised" => 0.5,
        _ => 1.0,
    };

    // Question modifier (questions require "more thought")
    let question_factor: f64 = if is_question { 1.3 } else { 1.0 };

    // Expressiveness modifier (expressive characters respond faster)
    let expressiveness_factor: f64 = 1.0 - (expressiveness as f64 * 0.3);

    // Calculate final delay
    let base_ms: f64 = 800.0;
    let total_ms =
        base_ms * length_factor * emotion_factor * question_factor * expressiveness_factor;
    let duration_ms = total_ms.clamp(200.0, 5000.0) as u64;

    let speed = match duration_ms {
        0..=299 => TypingSpeed::Instant,
        300..=799 => TypingSpeed::Fast,
        800..=1999 => TypingSpeed::Normal,
        2000..=3999 => TypingSpeed::Slow,
        _ => TypingSpeed::Thinking,
    };

    TypingParams { duration_ms, speed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excited_character_responds_faster() {
        let excited = calculate_typing_delay("excited", 0.9, 0.7, 20, false);
        let neutral = calculate_typing_delay("neutral", 0.5, 0.7, 20, false);
        assert!(
            excited.duration_ms < neutral.duration_ms,
            "Excited should be faster: {}ms vs {}ms",
            excited.duration_ms,
            neutral.duration_ms
        );
    }

    #[test]
    fn thinking_takes_longer() {
        let thinking = calculate_typing_delay("thinking", 0.5, 0.5, 30, false);
        let neutral = calculate_typing_delay("neutral", 0.5, 0.5, 30, false);
        assert!(
            thinking.duration_ms > neutral.duration_ms,
            "Thinking should be slower: {}ms vs {}ms",
            thinking.duration_ms,
            neutral.duration_ms
        );
    }

    #[test]
    fn question_increases_delay() {
        let question = calculate_typing_delay("neutral", 0.5, 0.5, 30, true);
        let statement = calculate_typing_delay("neutral", 0.5, 0.5, 30, false);
        assert!(
            question.duration_ms > statement.duration_ms,
            "Questions should take longer: {}ms vs {}ms",
            question.duration_ms,
            statement.duration_ms
        );
    }

    #[test]
    fn long_message_increases_delay() {
        let short = calculate_typing_delay("neutral", 0.5, 0.5, 10, false);
        let long = calculate_typing_delay("neutral", 0.5, 0.5, 200, false);
        assert!(
            long.duration_ms > short.duration_ms,
            "Long messages should need more thinking: {}ms vs {}ms",
            long.duration_ms,
            short.duration_ms
        );
    }

    #[test]
    fn delay_clamped_within_bounds() {
        let fast = calculate_typing_delay("excited", 0.9, 1.0, 5, false);
        let slow = calculate_typing_delay("thinking", 0.1, 0.0, 500, true);
        assert!(fast.duration_ms >= 200, "Min should be 200ms");
        assert!(slow.duration_ms <= 5000, "Max should be 5000ms");
    }
}
