//! User Sentiment Analysis — detect emotional tone from user messages.
//!
//! Uses keyword-based detection (fast, no LLM call) to estimate user's
//! emotional state. This feeds into the character's EmotionState as
//! "environmental influence" — the character is affected by the user's mood.

use serde::{Deserialize, Serialize};

/// Detected sentiment from a user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSentiment {
    /// Estimated mood (0.0 = very negative, 1.0 = very positive).
    pub mood: f32,
    /// Detected emotional tone.
    pub tone: SentimentTone,
    /// Confidence in the detection (0.0 = guessing, 1.0 = very confident).
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SentimentTone {
    Positive,
    Negative,
    Neutral,
    Questioning,
    Excited,
    Frustrated,
}

impl Default for UserSentiment {
    fn default() -> Self {
        Self {
            mood: 0.5,
            tone: SentimentTone::Neutral,
            confidence: 0.0,
        }
    }
}

// ── Keyword sets ───────────────────────────────────────────

const POSITIVE_KW: &[&str] = &[
    // Chinese
    "开心",
    "高兴",
    "快乐",
    "好棒",
    "太好了",
    "哈哈",
    "嘻嘻",
    "喜欢",
    "爱",
    "谢谢",
    "感谢",
    "好的",
    "可以",
    "没问题",
    "太棒了",
    "赞",
    "厉害",
    "有趣",
    "好玩",
    "期待",
    "幸福",
    "满意",
    "完美",
    "优秀",
    // English
    "happy",
    "glad",
    "great",
    "awesome",
    "amazing",
    "love",
    "thanks",
    "wonderful",
    "excellent",
    "nice",
    "cool",
    "perfect",
    "haha",
    "lol",
    "yay",
    "good",
    "beautiful",
    "incredible",
    // Emoji-like
    "😊",
    "😄",
    "😁",
    "❤",
    "💕",
    "👍",
    "🎉",
    "✨",
];

const NEGATIVE_KW: &[&str] = &[
    // Chinese
    "难过",
    "伤心",
    "不开心",
    "讨厌",
    "烦",
    "累了",
    "无聊",
    "生气",
    "害怕",
    "担心",
    "焦虑",
    "失望",
    "痛苦",
    "郁闷",
    "烦躁",
    "不行",
    "不好",
    "差",
    "算了",
    "唉",
    "呜呜",
    "呜",
    "哭",
    // English
    "sad",
    "angry",
    "annoyed",
    "frustrated",
    "tired",
    "bored",
    "hate",
    "terrible",
    "awful",
    "bad",
    "disappointed",
    "worried",
    "anxious",
    "stressed",
    "upset",
    "horrible",
    "sigh",
    // Emoji-like
    "😢",
    "😭",
    "😡",
    "😤",
    "💔",
    "😞",
    "😔",
];

const QUESTION_KW: &[&str] = &[
    "?",
    "？",
    "吗",
    "呢",
    "什么",
    "怎么",
    "为什么",
    "哪",
    "谁",
    "how",
    "what",
    "why",
    "when",
    "where",
    "who",
];

const EXCITEMENT_KW: &[&str] = &[
    "!",
    "！",
    "哇",
    "天哪",
    "不会吧",
    "真的吗",
    "太强了",
    "omg",
    "wow",
    "damn",
    "holy",
    "insane",
    "incredible",
];

const FRUSTRATION_KW: &[&str] = &[
    "不懂",
    "不会",
    "搞不定",
    "失败",
    "报错",
    "出错",
    "bug",
    "error",
    "broken",
    "crash",
    "stuck",
    "confused",
    "wrong",
    "doesn't work",
    "不对",
    "错了",
    "怎么回事",
];

/// Analyze a user message and detect sentiment.
pub fn analyze(text: &str) -> UserSentiment {
    let lower = text.to_lowercase();
    let char_count = text.chars().count();

    // Count keyword matches in each category
    let pos_count = POSITIVE_KW.iter().filter(|kw| lower.contains(*kw)).count();
    let neg_count = NEGATIVE_KW.iter().filter(|kw| lower.contains(*kw)).count();
    let q_count = QUESTION_KW.iter().filter(|kw| lower.contains(*kw)).count();
    let exc_count = EXCITEMENT_KW
        .iter()
        .filter(|kw| lower.contains(*kw))
        .count();
    let frus_count = FRUSTRATION_KW
        .iter()
        .filter(|kw| lower.contains(*kw))
        .count();

    let total_signals = pos_count + neg_count + q_count + exc_count + frus_count;

    // No signals detected → neutral
    if total_signals == 0 {
        return UserSentiment::default();
    }

    // Determine dominant tone
    let max_count = *[pos_count, neg_count, q_count, exc_count, frus_count]
        .iter()
        .max()
        .unwrap();

    let (tone, mood) = if frus_count == max_count && frus_count > 0 {
        (SentimentTone::Frustrated, 0.25)
    } else if neg_count == max_count && neg_count > 0 {
        (SentimentTone::Negative, 0.2)
    } else if exc_count == max_count && exc_count > 0 {
        (SentimentTone::Excited, 0.85)
    } else if pos_count == max_count && pos_count > 0 {
        (SentimentTone::Positive, 0.8)
    } else if q_count == max_count && q_count > 0 {
        (SentimentTone::Questioning, 0.5)
    } else {
        (SentimentTone::Neutral, 0.5)
    };

    // Confidence based on signal density (more keywords = more confident)
    let density = total_signals as f32 / (char_count.max(1) as f32 / 10.0);
    let confidence = density.clamp(0.1, 1.0);

    // Adjust mood by secondary signals
    let mood_adjusted = if pos_count > 0 && neg_count > 0 {
        // Mixed signals — pull toward neutral
        0.5 + (mood - 0.5) * 0.5
    } else {
        mood
    };

    UserSentiment {
        mood: mood_adjusted,
        tone,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_message_detected() {
        let s = analyze("哈哈太好了！我好开心");
        assert_eq!(s.tone, SentimentTone::Positive);
        assert!(s.mood > 0.6);
    }

    #[test]
    fn negative_message_detected() {
        let s = analyze("唉，好烦啊，真的很不开心");
        assert_eq!(s.tone, SentimentTone::Negative);
        assert!(s.mood < 0.4);
    }

    #[test]
    fn frustrated_message_detected() {
        let s = analyze("这个bug搞不定了，一直报错");
        assert_eq!(s.tone, SentimentTone::Frustrated);
        assert!(s.mood < 0.4);
    }

    #[test]
    fn question_detected() {
        let s = analyze("这个怎么用？");
        assert_eq!(s.tone, SentimentTone::Questioning);
    }

    #[test]
    fn neutral_for_plain_text() {
        let s = analyze("明天三点开会");
        assert_eq!(s.tone, SentimentTone::Neutral);
        assert!(s.confidence < 0.2);
    }

    #[test]
    fn english_positive() {
        let s = analyze("This is awesome! I love it, thanks!");
        assert!(
            s.mood > 0.6,
            "English positive should detect high mood, got {}",
            s.mood
        );
    }

    #[test]
    fn english_negative() {
        let s = analyze("I'm so frustrated and disappointed");
        assert!(
            s.mood < 0.4,
            "English negative should detect low mood, got {}",
            s.mood
        );
    }

    #[test]
    fn mixed_signals_pull_toward_neutral() {
        let s = analyze("我很开心但也有点担心");
        // Mixed positive + negative → mood closer to 0.5 than pure positive (0.8) or negative (0.2)
        assert!(
            s.mood > 0.3 && s.mood < 0.7,
            "Mixed should be near neutral, got {}",
            s.mood
        );
    }
}
