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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryIngressDecision {
    pub event: MemoryIngressEvent,
    pub trigger_label: &'static str,
}

#[derive(Debug, Clone)]
pub struct MemoryEventIngressOptions {
    pub enabled: bool,
    pub event_cooldown_secs: u64,
    pub intent_routing_enabled: bool,
}

impl Default for MemoryEventIngressOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            event_cooldown_secs: 120,
            intent_routing_enabled: true,
        }
    }
}

pub fn event_trigger_label(event_type: MemoryEventType) -> &'static str {
    match event_type {
        MemoryEventType::Preference => "event_preference",
        MemoryEventType::Correction => "event_correction",
        MemoryEventType::Plan => "event_plan",
        MemoryEventType::Profile => "event_profile",
    }
}

pub fn select_memory_ingress_decision(
    input: &str,
    options: &MemoryEventIngressOptions,
) -> Option<MemoryIngressDecision> {
    if !options.enabled {
        return None;
    }

    let events = detect_memory_events(input, options);
    let event = if options.intent_routing_enabled {
        prioritized_event(events)
    } else {
        events.into_iter().next()
    }?;

    Some(MemoryIngressDecision {
        trigger_label: event_trigger_label(event.event_type),
        event,
    })
}

pub fn memory_extraction_structured_enabled(options: &MemoryEventIngressOptions) -> bool {
    options.enabled && options.intent_routing_enabled
}

pub fn should_use_structured_extraction(
    upgrade_enabled: bool,
    options: &MemoryEventIngressOptions,
) -> bool {
    upgrade_enabled && memory_extraction_structured_enabled(options)
}

pub fn build_memory_ingress_decision_for_test(
    input: &str,
    event_trigger_enabled: bool,
    intent_routing_enabled: bool,
    cooldown_secs: u64,
) -> Option<MemoryIngressDecision> {
    select_memory_ingress_decision(
        input,
        &MemoryEventIngressOptions {
            enabled: event_trigger_enabled,
            event_cooldown_secs: cooldown_secs,
            intent_routing_enabled,
        },
    )
}

pub fn build_memory_extraction_options_for_test(
    structured_memory_enabled: bool,
    event_trigger_enabled: bool,
    intent_routing_enabled: bool,
) -> bool {
    should_use_structured_extraction(
        structured_memory_enabled,
        &MemoryEventIngressOptions {
            enabled: event_trigger_enabled,
            event_cooldown_secs: 120,
            intent_routing_enabled,
        },
    )
}

fn prioritized_event(events: Vec<MemoryIngressEvent>) -> Option<MemoryIngressEvent> {
    for event_type in [
        MemoryEventType::Preference,
        MemoryEventType::Correction,
        MemoryEventType::Plan,
        MemoryEventType::Profile,
    ] {
        if let Some(event) = events.iter().find(|event| event.event_type == event_type) {
            return Some(event.clone());
        }
    }
    None
}

pub fn build_cooldown_key(
    character_id: &str,
    conversation_id: &str,
    event_type: MemoryEventType,
) -> String {
    format!(
        "{}:{}:{}",
        character_id.trim(),
        conversation_id.trim(),
        event_type.as_str()
    )
}

pub fn detect_memory_events(
    input: &str,
    options: &MemoryEventIngressOptions,
) -> Vec<MemoryIngressEvent> {
    let normalized = normalize(input);
    if normalized.is_empty() {
        return vec![];
    }

    let mut events = Vec::new();

    if contains_any(
        &normalized,
        &["不是", "而是", "纠正", "更正", "说错", "并不是"],
    ) {
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
    ) && !is_explicit_correction_statement(&normalized)
    {
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

fn is_explicit_correction_statement(content: &str) -> bool {
    let has_direct_correction_keyword = contains_any(
        content,
        &[
            "\u{7ea0}\u{6b63}",
            "\u{66f4}\u{6b63}",
            "\u{8bf4}\u{9519}",
            "actually",
            "not exactly",
        ],
    );
    let has_negated_restatement =
        contains_any(content, &["\u{4e0d}\u{662f}", "\u{5e76}\u{4e0d}\u{662f}"])
            && contains_any(
                content,
                &["\u{800c}\u{662f}", "\u{ff0c}\u{662f}", ",\u{662f}", " is "],
            );

    has_direct_correction_keyword || has_negated_restatement
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
        assert!(result
            .iter()
            .any(|event| event.event_type == MemoryEventType::Correction));
    }

    #[test]
    fn explicit_correction_statement_does_not_emit_preference_event() {
        let result = detect_memory_events(
            "\u{4e0d}\u{662f}\u{6211}\u{559c}\u{6b22}\u{732b}\u{ff0c}\u{662f}\u{6211}\u{4ee5}\u{524d}\u{517b}\u{8fc7}\u{732b}",
            &MemoryEventIngressOptions::default(),
        );
        assert!(!result
            .iter()
            .any(|event| event.event_type == MemoryEventType::Preference));
    }

    #[test]
    fn detects_plan_event() {
        let result = detect_memory_events(
            "下周我要继续做这个记忆系统架构",
            &MemoryEventIngressOptions::default(),
        );
        assert!(result
            .iter()
            .any(|event| event.event_type == MemoryEventType::Plan));
    }

    #[test]
    fn detects_profile_event() {
        let result = detect_memory_events(
            "我是第一次接触这个项目的前端部分",
            &MemoryEventIngressOptions::default(),
        );
        assert!(result
            .iter()
            .any(|event| event.event_type == MemoryEventType::Profile));
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

    #[test]
    fn ingress_decision_respects_disabled_flag() {
        let decision = build_memory_ingress_decision_for_test(
            "我是第一次接触这个项目的前端部分",
            false,
            true,
            120,
        );
        assert!(decision.is_none());
    }

    #[test]
    fn ingress_decision_uses_priority_when_intent_routing_enabled() {
        let decision = build_memory_ingress_decision_for_test(
            "不是我喜欢猫，下周我要继续做前端",
            true,
            true,
            120,
        )
        .expect("decision");
        assert_eq!(decision.trigger_label, "event_preference");
    }

    #[test]
    fn structured_extraction_requires_both_flags() {
        assert!(build_memory_extraction_options_for_test(true, true, true));
        assert!(!build_memory_extraction_options_for_test(true, true, false));
        assert!(!build_memory_extraction_options_for_test(true, false, true));
        assert!(!build_memory_extraction_options_for_test(false, true, true));
    }
}
