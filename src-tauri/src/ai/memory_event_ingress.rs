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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryIngressRoute {
    pub focus_event: Option<MemoryEventType>,
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

pub fn build_cooldown_key(
    character_id: &str,
    conversation_id: &str,
    event_type: MemoryEventType,
) -> String {
    format!("{}:{}:{}", character_id.trim(), conversation_id.trim(), event_type.as_str())
}

pub fn build_routed_cooldown_key(
    character_id: &str,
    conversation_id: &str,
    route: MemoryIngressRoute,
) -> String {
    match route.focus_event {
        Some(event_type) => build_cooldown_key(character_id, conversation_id, event_type),
        None => format!(
            "{}:{}:event_detected",
            character_id.trim(),
            conversation_id.trim()
        ),
    }
}

pub fn memory_ingress_trigger_label(route: MemoryIngressRoute) -> &'static str {
    match route.focus_event {
        Some(MemoryEventType::Preference) => "event_preference",
        Some(MemoryEventType::Correction) => "event_correction",
        Some(MemoryEventType::Plan) => "event_plan",
        Some(MemoryEventType::Profile) => "event_profile",
        None => "event_detected",
    }
}

pub fn route_memory_ingress_event(
    detected_events: &[MemoryIngressEvent],
    intent_routing_enabled: bool,
) -> Option<MemoryIngressRoute> {
    let first = detected_events.first()?;
    Some(MemoryIngressRoute {
        focus_event: intent_routing_enabled.then_some(first.event_type),
        cooldown_secs: first.cooldown_secs,
    })
}

pub fn detect_memory_events(input: &str, options: &MemoryEventIngressOptions) -> Vec<MemoryIngressEvent> {
    let normalized = normalize(input);
    if normalized.is_empty() {
        return vec![];
    }

    let mut events = Vec::new();

    if contains_any(
        &normalized,
        &["??", "??", "??", "??", "??", "???", "actually", "not exactly"],
    ) {
        events.push(MemoryIngressEvent {
            event_type: MemoryEventType::Correction,
            cooldown_secs: options.event_cooldown_secs,
        });
    }

    if contains_any(
        &normalized,
        &[
            "???",
            "????",
            "????",
            "???",
            "??",
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
            "??",
            "??",
            "???",
            "??",
            "??",
            "???",
            "??",
            "??",
            "??",
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
            "??",
            "???",
            "??",
            "??",
            "???",
            "??",
            "??",
            "???",
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
            "???????????????",
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
            "???????????????",
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
            "????????????????",
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
        let result = detect_memory_events("????", &MemoryEventIngressOptions::default());
        assert!(result.is_empty());
    }

    #[test]
    fn builds_cooldown_key_with_event_type() {
        let key = build_cooldown_key("char-1", "conv-1", MemoryEventType::Preference);
        assert_eq!(key, "char-1:conv-1:preference");
    }

    #[test]
    fn routes_detected_event_when_intent_routing_enabled() {
        let detected = vec![MemoryIngressEvent {
            event_type: MemoryEventType::Plan,
            cooldown_secs: 45,
        }];

        let route = route_memory_ingress_event(&detected, true).expect("route should exist");

        assert_eq!(route.focus_event, Some(MemoryEventType::Plan));
        assert_eq!(memory_ingress_trigger_label(route), "event_plan");
        assert_eq!(
            build_routed_cooldown_key("char-1", "conv-1", route),
            "char-1:conv-1:plan"
        );
    }

    #[test]
    fn falls_back_to_generic_trigger_when_intent_routing_disabled() {
        let detected = vec![MemoryIngressEvent {
            event_type: MemoryEventType::Correction,
            cooldown_secs: 90,
        }];

        let route = route_memory_ingress_event(&detected, false).expect("route should exist");

        assert_eq!(route.focus_event, None);
        assert_eq!(memory_ingress_trigger_label(route), "event_detected");
        assert_eq!(
            build_routed_cooldown_key("char-1", "conv-1", route),
            "char-1:conv-1:event_detected"
        );
    }
}
