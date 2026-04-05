use crate::actions::registry::{ActionInfo, ActionPermissionLevel, ActionRiskTag};
use crate::actions::tool_settings::ToolSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    DenyPolicy { reason: String },
    DenyPendingApproval { reason: String },
    DenyFailClosed { reason: String },
}

pub fn risk_tag_label(tag: &ActionRiskTag) -> &'static str {
    match tag {
        ActionRiskTag::Read => "read",
        ActionRiskTag::Write => "write",
        ActionRiskTag::External => "external",
        ActionRiskTag::Sensitive => "sensitive",
    }
}

pub fn exceeds_safe_permission_ceiling(action: &ActionInfo, settings: &ToolSettings) -> bool {
    matches!(
        (action.permission_level, settings.max_permission_level),
        (ActionPermissionLevel::Elevated, ActionPermissionLevel::Safe)
    )
}

fn has_risk_tag(action: &ActionInfo, expected: ActionRiskTag) -> bool {
    action.risk_tags.contains(&expected)
}

pub fn evaluate_permission_decision(
    action: &ActionInfo,
    settings: &ToolSettings,
) -> PermissionDecision {
    if exceeds_safe_permission_ceiling(action, settings) && has_risk_tag(action, ActionRiskTag::Sensitive) {
        return PermissionDecision::DenyFailClosed {
            reason: "Denied by fail-closed policy: permission level 'elevated' exceeds max allowed 'safe'".to_string(),
        };
    }

    if settings.blocked_risk_tags.contains(&ActionRiskTag::Sensitive)
        && action.risk_tags.contains(&ActionRiskTag::Sensitive)
    {
        return PermissionDecision::DenyFailClosed {
            reason: "Denied by fail-closed policy: blocked risk tag 'sensitive'".to_string(),
        };
    }

    if exceeds_safe_permission_ceiling(action, settings) {
        return PermissionDecision::DenyPendingApproval {
            reason: "Denied pending approval: permission level 'elevated' requires approval".to_string(),
        };
    }

    for tag in &action.risk_tags {
        if !settings.blocked_risk_tags.contains(tag) {
            continue;
        }

        if matches!(tag, ActionRiskTag::Write)
            || (*tag == ActionRiskTag::Sensitive
                && action.permission_level == ActionPermissionLevel::Safe)
        {
            return PermissionDecision::DenyPendingApproval {
                reason: format!("Denied pending approval: risk tag '{}' requires approval", risk_tag_label(tag)),
            };
        }

        return PermissionDecision::DenyPolicy {
            reason: format!("Denied by policy: blocked risk tag '{}'", risk_tag_label(tag)),
        };
    }

    PermissionDecision::Allow
}

pub fn decision_reason(decision: &PermissionDecision) -> Option<&str> {
    match decision {
        PermissionDecision::Allow => None,
        PermissionDecision::DenyPolicy { reason }
        | PermissionDecision::DenyPendingApproval { reason }
        | PermissionDecision::DenyFailClosed { reason } => Some(reason.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ActionSource;
    use std::collections::HashMap;

    fn action(permission_level: ActionPermissionLevel, risk_tags: Vec<ActionRiskTag>) -> ActionInfo {
        ActionInfo {
            id: "builtin__test_tool".to_string(),
            name: "test_tool".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "test".to_string(),
            parameters: vec![],
            needs_feedback: false,
            risk_tags,
            permission_level,
        }
    }

    fn settings(max_permission_level: ActionPermissionLevel, blocked_risk_tags: Vec<ActionRiskTag>) -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level,
            blocked_risk_tags,
        }
    }

    #[test]
    fn allows_safe_read_when_not_blocked() {
        let decision = evaluate_permission_decision(
            &action(ActionPermissionLevel::Safe, vec![ActionRiskTag::Read]),
            &settings(ActionPermissionLevel::Elevated, vec![]),
        );

        assert_eq!(decision, PermissionDecision::Allow);
    }

    #[test]
    fn denies_policy_for_blocked_read_or_external() {
        let read_decision = evaluate_permission_decision(
            &action(ActionPermissionLevel::Safe, vec![ActionRiskTag::Read]),
            &settings(ActionPermissionLevel::Elevated, vec![ActionRiskTag::Read]),
        );
        let external_decision = evaluate_permission_decision(
            &action(ActionPermissionLevel::Safe, vec![ActionRiskTag::External]),
            &settings(ActionPermissionLevel::Elevated, vec![ActionRiskTag::External]),
        );

        assert_eq!(
            read_decision,
            PermissionDecision::DenyPolicy {
                reason: "Denied by policy: blocked risk tag 'read'".to_string(),
            }
        );
        assert_eq!(
            external_decision,
            PermissionDecision::DenyPolicy {
                reason: "Denied by policy: blocked risk tag 'external'".to_string(),
            }
        );
    }

    #[test]
    fn denies_pending_approval_for_elevated_non_sensitive_write() {
        let elevated_decision = evaluate_permission_decision(
            &action(ActionPermissionLevel::Elevated, vec![ActionRiskTag::Write]),
            &settings(ActionPermissionLevel::Safe, vec![ActionRiskTag::Write]),
        );
        let write_decision = evaluate_permission_decision(
            &action(ActionPermissionLevel::Safe, vec![ActionRiskTag::Write]),
            &settings(ActionPermissionLevel::Elevated, vec![ActionRiskTag::Write]),
        );

        assert_eq!(
            elevated_decision,
            PermissionDecision::DenyPendingApproval {
                reason: "Denied pending approval: permission level 'elevated' requires approval".to_string(),
            }
        );
        assert_eq!(
            write_decision,
            PermissionDecision::DenyPendingApproval {
                reason: "Denied pending approval: risk tag 'write' requires approval".to_string(),
            }
        );
    }

    #[test]
    fn denies_fail_closed_for_elevated_sensitive_and_blocked_sensitive() {
        let elevated_sensitive = evaluate_permission_decision(
            &action(ActionPermissionLevel::Elevated, vec![ActionRiskTag::Sensitive]),
            &settings(ActionPermissionLevel::Safe, vec![]),
        );
        let blocked_sensitive = evaluate_permission_decision(
            &action(ActionPermissionLevel::Safe, vec![ActionRiskTag::Sensitive]),
            &settings(ActionPermissionLevel::Elevated, vec![ActionRiskTag::Sensitive]),
        );

        assert_eq!(
            elevated_sensitive,
            PermissionDecision::DenyFailClosed {
                reason: "Denied by fail-closed policy: permission level 'elevated' exceeds max allowed 'safe'".to_string(),
            }
        );
        assert_eq!(
            blocked_sensitive,
            PermissionDecision::DenyFailClosed {
                reason: "Denied by fail-closed policy: blocked risk tag 'sensitive'".to_string(),
            }
        );
    }

    #[test]
    fn prioritizes_fail_closed_over_pending_and_policy_when_multiple_conditions_match() {
        let decision = evaluate_permission_decision(
            &action(ActionPermissionLevel::Elevated, vec![ActionRiskTag::Sensitive, ActionRiskTag::Write]),
            &settings(ActionPermissionLevel::Safe, vec![ActionRiskTag::Sensitive, ActionRiskTag::Write]),
        );

        assert_eq!(
            decision,
            PermissionDecision::DenyFailClosed {
                reason: "Denied by fail-closed policy: permission level 'elevated' exceeds max allowed 'safe'".to_string(),
            }
        );
    }
}
