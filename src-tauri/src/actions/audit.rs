use crate::actions::permission::PermissionDecision;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAuditDecision {
    Allow,
    PolicyDeny,
    PendingApproval,
    FailClosed,
    ApprovedAfterPending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAuditEvent {
    pub tool_id: String,
    pub tool_name: String,
    pub source: String,
    pub server_name: Option<String>,
    pub invocation_source: String,
    pub risk_tags: Vec<String>,
    pub permission_level: String,
    pub decision: ToolAuditDecision,
    pub reason: Option<String>,
    pub approved_by_user: Option<bool>,
    pub conversation_id: Option<String>,
    pub character_id: Option<String>,
}

fn tool_audit_decision_from_permission(
    decision: &PermissionDecision,
    approved_by_user: Option<bool>,
) -> ToolAuditDecision {
    match (decision, approved_by_user) {
        (PermissionDecision::Allow, _) => ToolAuditDecision::Allow,
        (PermissionDecision::DenyPolicy { .. }, _) => ToolAuditDecision::PolicyDeny,
        (PermissionDecision::DenyPendingApproval { .. }, Some(true)) => ToolAuditDecision::ApprovedAfterPending,
        (PermissionDecision::DenyPendingApproval { .. }, _) => ToolAuditDecision::PendingApproval,
        (PermissionDecision::DenyFailClosed { .. }, _) => ToolAuditDecision::FailClosed,
    }
}

pub fn build_tool_audit_event(
    tool_id: &str,
    tool_name: &str,
    source: &str,
    server_name: Option<&str>,
    invocation_source: &str,
    risk_tags: &[&str],
    permission_level: &str,
    decision: &PermissionDecision,
    approved_by_user: Option<bool>,
    conversation_id: Option<&str>,
    character_id: Option<&str>,
) -> ToolAuditEvent {
    let audit_decision = tool_audit_decision_from_permission(decision, approved_by_user);

    let reason = match decision {
        PermissionDecision::Allow => None,
        PermissionDecision::DenyPolicy { reason }
        | PermissionDecision::DenyPendingApproval { reason }
        | PermissionDecision::DenyFailClosed { reason } => Some(reason.clone()),
    };

    ToolAuditEvent {
        tool_id: tool_id.to_string(),
        tool_name: tool_name.to_string(),
        source: source.to_string(),
        server_name: server_name.map(ToString::to_string),
        invocation_source: invocation_source.to_string(),
        risk_tags: risk_tags.iter().map(|tag| (*tag).to_string()).collect(),
        permission_level: permission_level.to_string(),
        decision: audit_decision,
        reason,
        approved_by_user,
        conversation_id: conversation_id.map(ToString::to_string),
        character_id: character_id.map(ToString::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(decision: PermissionDecision, approved_by_user: Option<bool>) -> ToolAuditEvent {
        build_tool_audit_event(
            "builtin__write_note",
            "write_note",
            "builtin",
            None,
            "chat",
            &["write"],
            "safe",
            &decision,
            approved_by_user,
            Some("conv-1"),
            Some("char-1"),
        )
    }

    #[test]
    fn builds_allow_audit_event() {
        let audit = event(PermissionDecision::Allow, None);
        assert_eq!(audit.decision, ToolAuditDecision::Allow);
        assert_eq!(audit.tool_id, "builtin__write_note");
        assert_eq!(audit.invocation_source, "chat");
    }

    #[test]
    fn builds_policy_deny_audit_event() {
        let audit = event(
            PermissionDecision::DenyPolicy {
                reason: "Denied by policy: blocked risk tag 'read'".to_string(),
            },
            None,
        );
        assert_eq!(audit.decision, ToolAuditDecision::PolicyDeny);
        assert_eq!(audit.reason.as_deref(), Some("Denied by policy: blocked risk tag 'read'"));
    }

    #[test]
    fn builds_pending_approval_audit_event() {
        let audit = event(
            PermissionDecision::DenyPendingApproval {
                reason: "Denied pending approval: risk tag 'write' requires approval".to_string(),
            },
            None,
        );
        assert_eq!(audit.decision, ToolAuditDecision::PendingApproval);
    }

    #[test]
    fn builds_fail_closed_audit_event() {
        let audit = event(
            PermissionDecision::DenyFailClosed {
                reason: "Denied by fail-closed policy: blocked risk tag 'sensitive'".to_string(),
            },
            None,
        );
        assert_eq!(audit.decision, ToolAuditDecision::FailClosed);
    }

    #[test]
    fn builds_approved_after_pending_audit_event() {
        let audit = event(
            PermissionDecision::DenyPendingApproval {
                reason: "Denied pending approval: permission level 'elevated' requires approval".to_string(),
            },
            Some(true),
        );
        assert_eq!(audit.decision, ToolAuditDecision::ApprovedAfterPending);
        assert_eq!(audit.approved_by_user, Some(true));
    }
}
