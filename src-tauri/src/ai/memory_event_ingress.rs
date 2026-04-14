use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryEventType {
    Preference,
    Correction,
    Plan,
    Profile,
}

impl MemoryEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Correction => "correction",
            Self::Plan => "plan",
            Self::Profile => "profile",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryIngressEvent {
    pub event_type: MemoryEventType,
    pub cooldown_secs: u64,
}

#[derive(Debug, Clone)]
pub struct MemoryEventIngressOptions {
    pub enabled: bool,
    pub event_cooldown_secs: u64,
}

impl Default for MemoryEventIngressOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            event_cooldown_secs: 120,
        }
    }
}

pub fn build_cooldown_key(character_id: &str, conversation_id: &str, event_type: MemoryEventType) -> String {
    format!("{}:{}:{}", character_id.trim(), conversation_id.trim(), event_type.as_str())
}

pub fn detect_memory_events(input: &str, options: &MemoryEventIngressOptions) -> Vec<MemoryIngressEvent> {
    let normalized = normalize(input);
    if normalized.is_empty() {
        return vec![];
    }

    let mut events = Vec::new();

    if contains_any(&normalized, &["不是", "而是", "纠正", "更正", "说错", "并不是"]) {
        events.push(MemoryIngressEvent {
            event_type: MemoryEventType::Correction,
            cooldown_secs: options.event_cooldown_secs,
        });
    }

    if contains_any(
        &normalized,
        &[
            "我喜欢",
            "我不喜欢",
            "我更喜欢",
            "我讨厌",
            "偏好",
            "prefer",
            "i like",
            "i dislike",
        ],
    ) {
        events.push(MemoryIngressEvent {
            event_type: MemoryEventType::Preference,
            cooldown_secs: options.event_cooldown_secs,
        });
    }

    if contains_any(
        &normalized,
        &[
            "我要",
            "我会",
            "我计划",
            "下周",
            "明天",
            "接下来",
            "打算",
            "承诺",
            "约定",
            "i will",
            "plan to",
        ],
    ) {
        events.push(MemoryIngressEvent {
            event_type: MemoryEventType::Plan,
            cooldown_secs: options.event_cooldown_secs,
        });
    }

    if contains_any(
        &normalized,
        &[
            "我是",
            "我来自",
            "我做",
            "我在",
            "第一次",
            "背景",
            "职业",
            "工作是",
            "my background",
            "i am",
        ],
    ) {
        events.push(MemoryIngressEvent {
            event_type: MemoryEventType::Profile,
            cooldown_secs: options.event_cooldown_secs,
        });
    }

    dedup_events(events)
}

fn normalize(input: &str) -> String {
    input.trim().to_lowercase()
}

fn contains_any(content: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| content.contains(keyword))
}

fn dedup_events(events: Vec<MemoryIngressEvent>) -> Vec<MemoryIngressEvent> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for event in events {
        if seen.insert(event.event_type) {
            result.push(event);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_preference_correction_event() {
        let result = detect_memory_events(
            "不是我喜欢猫，是我以前养过猫",
            &MemoryEventIngressOptions::default(),
        );
        assert!(
            result
                .iter()
                .any(|event| event.event_type == MemoryEventType::Correction)
        );
    }

    #[test]
    fn detects_plan_event() {
        let result = detect_memory_events(
            "下周我要继续做这个记忆系统架构",
            &MemoryEventIngressOptions::default(),
        );
        assert!(
            result
                .iter()
                .any(|event| event.event_type == MemoryEventType::Plan)
        );
    }

    #[test]
    fn detects_profile_event() {
        let result = detect_memory_events(
            "我是第一次接触这个项目的前端部分",
            &MemoryEventIngressOptions::default(),
        );
        assert!(
            result
                .iter()
                .any(|event| event.event_type == MemoryEventType::Profile)
        );
    }

    #[test]
    fn ignores_low_value_small_talk() {
        let result = detect_memory_events("哈哈好的", &MemoryEventIngressOptions::default());
        assert!(result.is_empty());
    }

    #[test]
    fn builds_cooldown_key_with_event_type() {
        let key = build_cooldown_key("char-1", "conv-1", MemoryEventType::Preference);
        assert_eq!(key, "char-1:conv-1:preference");
    }
}
