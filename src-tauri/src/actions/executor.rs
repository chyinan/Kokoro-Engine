// pattern: Mixed (needs refactoring)
// Reason: P2.1 需要把 action deny 的纯结果整形与 executor 编排保持最小范围共置，避免额外扩散模块边界。
use crate::actions::permission::{
    evaluate_permission_decision, risk_tag_label, PermissionDecision,
};
use crate::actions::registry::{ActionContext, ActionInfo, ActionRegistry, ActionResult};
use crate::actions::tool_settings::ToolSettings;
use crate::hooks::types::HookModifyPolicy;
use crate::hooks::{
    ActionHookPayload, BeforeActionArgsPayload, HookEvent, HookOutcome, HookPayload, HookRuntime,
};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ToolInvocation {
    pub tool_call_id: Option<String>,
    pub name: String,
    pub args: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ToolExecutionOutcome {
    pub invocation: ToolInvocation,
    pub action: Option<ActionInfo>,
    pub result: Result<ActionResult, String>,
    pub needs_feedback: bool,
    pub permission_decision: Option<PermissionDecision>,
}

pub(crate) fn denied_by_hook_message(reason: &str) -> String {
    format!("Denied by hook: {}", reason)
}

pub(crate) fn continue_unless_denied<T>(
    gate: HookOutcome,
    on_continue: impl FnOnce() -> T,
) -> Result<T, String> {
    match gate {
        HookOutcome::Continue => Ok(on_continue()),
        HookOutcome::Deny { reason } => Err(denied_by_hook_message(&reason)),
    }
}

pub(crate) fn build_action_hook_payload(
    conversation_id: Option<String>,
    character_id: &str,
    source: Option<String>,
    invocation: &ToolInvocation,
    action: Option<&ActionInfo>,
    success: Option<bool>,
    result_message: Option<String>,
) -> HookPayload {
    HookPayload::Action(ActionHookPayload {
        conversation_id,
        character_id: character_id.to_string(),
        tool_call_id: invocation.tool_call_id.clone(),
        action_id: action.map(|value| value.id.clone()),
        action_name: action
            .map(|value| value.name.clone())
            .unwrap_or_else(|| invocation.name.clone()),
        args: invocation.args.clone(),
        success,
        result_message,
        source,
    })
}

pub(crate) fn build_before_action_args_payload(
    conversation_id: Option<String>,
    character_id: &str,
    source: Option<String>,
    invocation: &ToolInvocation,
    action: &ActionInfo,
) -> BeforeActionArgsPayload {
    BeforeActionArgsPayload {
        conversation_id,
        character_id: character_id.to_string(),
        tool_call_id: invocation.tool_call_id.clone(),
        action_id: action.id.clone(),
        action_name: action.name.clone(),
        args: invocation.args.clone(),
        source,
    }
}

pub(crate) fn apply_before_action_args_payload(
    payload: BeforeActionArgsPayload,
) -> HashMap<String, String> {
    payload.args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::registry::{ActionPermissionLevel, ActionRiskTag, ActionSource};

    fn sample_invocation() -> ToolInvocation {
        ToolInvocation {
            tool_call_id: Some("tool-call-1".to_string()),
            name: "search_memory".to_string(),
            args: HashMap::from([("query".to_string(), "kokoro".to_string())]),
        }
    }

    fn sample_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__search_memory".to_string(),
            name: "search_memory".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "Search memory".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![ActionRiskTag::Read],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_elevated_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__set_background".to_string(),
            name: "set_background".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "Set background".to_string(),
            parameters: vec![],
            needs_feedback: false,
            risk_tags: vec![ActionRiskTag::Write],
            permission_level: ActionPermissionLevel::Elevated,
        }
    }

    fn sample_sensitive_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__store_memory".to_string(),
            name: "store_memory".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "Store memory".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![ActionRiskTag::Sensitive],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_safe_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Safe,
            blocked_risk_tags: Vec::new(),
        }
    }

    fn sample_default_policy_settings() -> ToolSettings {
        ToolSettings::default()
    }

    fn sample_read_blocking_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Elevated,
            blocked_risk_tags: vec![ActionRiskTag::Read],
        }
    }

    fn sample_write_blocking_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Elevated,
            blocked_risk_tags: vec![ActionRiskTag::Write],
        }
    }

    fn sample_sensitive_blocking_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Elevated,
            blocked_risk_tags: vec![ActionRiskTag::Sensitive],
        }
    }

    fn sample_external_blocking_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Elevated,
            blocked_risk_tags: vec![ActionRiskTag::External],
        }
    }

    fn sample_safe_ceiling_with_write_blocked_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Safe,
            blocked_risk_tags: vec![ActionRiskTag::Write],
        }
    }

    fn sample_safe_ceiling_with_sensitive_blocked_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Safe,
            blocked_risk_tags: vec![ActionRiskTag::Sensitive],
        }
    }

    fn sample_write_safe_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__write_note".to_string(),
            name: "write_note".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "Write note".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![ActionRiskTag::Write],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_sensitive_safe_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__read_secret".to_string(),
            name: "read_secret".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "Read secret".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![ActionRiskTag::Sensitive],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_sensitive_elevated_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__store_secret".to_string(),
            name: "store_secret".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "Store secret".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![ActionRiskTag::Sensitive],
            permission_level: ActionPermissionLevel::Elevated,
        }
    }

    fn sample_read_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__read_memory".to_string(),
            name: "read_memory".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "Read memory".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![ActionRiskTag::Read],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_external_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__call_api".to_string(),
            name: "call_api".to_string(),
            source: ActionSource::Builtin,
            server_name: None,
            description: "Call api".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![ActionRiskTag::External],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_default_allow_action() -> ActionInfo {
        sample_action()
    }

    fn sample_default_allow_settings() -> ToolSettings {
        sample_default_policy_settings()
    }

    fn sample_policy_permission_action() -> ActionInfo {
        sample_elevated_action()
    }

    fn sample_policy_permission_settings() -> ToolSettings {
        sample_safe_settings()
    }

    fn sample_policy_only_action() -> ActionInfo {
        sample_read_action()
    }

    fn sample_policy_only_settings() -> ToolSettings {
        sample_read_blocking_settings()
    }

    fn sample_policy_external_action() -> ActionInfo {
        sample_external_action()
    }

    fn sample_policy_external_settings() -> ToolSettings {
        sample_external_blocking_settings()
    }

    fn sample_pending_elevated_action() -> ActionInfo {
        sample_elevated_action()
    }

    fn sample_pending_elevated_settings() -> ToolSettings {
        sample_safe_settings()
    }

    fn sample_pending_write_action() -> ActionInfo {
        sample_write_safe_action()
    }

    fn sample_pending_write_settings() -> ToolSettings {
        sample_write_blocking_settings()
    }

    fn sample_pending_prefers_permission_action() -> ActionInfo {
        sample_elevated_action()
    }

    fn sample_pending_prefers_permission_settings() -> ToolSettings {
        sample_safe_ceiling_with_write_blocked_settings()
    }

    fn sample_pending_non_high_risk_action() -> ActionInfo {
        sample_read_action()
    }

    fn sample_pending_non_high_risk_settings() -> ToolSettings {
        sample_read_blocking_settings()
    }

    fn sample_pending_read_only_action() -> ActionInfo {
        sample_read_action()
    }

    fn sample_pending_read_only_settings() -> ToolSettings {
        sample_read_blocking_settings()
    }

    fn sample_fail_closed_sensitive_action() -> ActionInfo {
        sample_sensitive_safe_action()
    }

    fn sample_fail_closed_sensitive_settings() -> ToolSettings {
        sample_sensitive_blocking_settings()
    }

    fn sample_fail_closed_prefers_permission_action() -> ActionInfo {
        sample_sensitive_elevated_action()
    }

    fn sample_fail_closed_prefers_permission_settings() -> ToolSettings {
        sample_safe_ceiling_with_sensitive_blocked_settings()
    }

    fn sample_fail_closed_non_high_risk_action() -> ActionInfo {
        sample_external_action()
    }

    fn sample_fail_closed_non_high_risk_settings() -> ToolSettings {
        sample_external_blocking_settings()
    }

    fn sample_fail_closed_external_only_action() -> ActionInfo {
        sample_external_action()
    }

    fn sample_fail_closed_external_only_settings() -> ToolSettings {
        sample_external_blocking_settings()
    }

    fn sample_no_denial_action() -> ActionInfo {
        sample_default_allow_action()
    }

    fn sample_no_denial_settings() -> ToolSettings {
        sample_default_allow_settings()
    }

    fn sample_policy_tag_action() -> ActionInfo {
        sample_read_action()
    }

    fn sample_policy_tag_settings() -> ToolSettings {
        sample_read_blocking_settings()
    }

    fn sample_pending_write_only_action() -> ActionInfo {
        sample_write_safe_action()
    }

    fn sample_pending_write_only_settings() -> ToolSettings {
        sample_write_blocking_settings()
    }

    fn sample_fail_closed_sensitive_only_action() -> ActionInfo {
        sample_sensitive_safe_action()
    }

    fn sample_fail_closed_sensitive_only_settings() -> ToolSettings {
        sample_sensitive_blocking_settings()
    }

    fn policy_denial_reason(action: &ActionInfo, settings: &ToolSettings) -> Option<String> {
        match evaluate_permission_decision(action, settings) {
            PermissionDecision::DenyPolicy { reason } => Some(reason),
            _ => None,
        }
    }

    fn approval_pending_reason(action: &ActionInfo, settings: &ToolSettings) -> Option<String> {
        match evaluate_permission_decision(action, settings) {
            PermissionDecision::DenyPendingApproval { reason } => Some(reason),
            _ => None,
        }
    }

    fn high_risk_fail_closed_reason(
        action: &ActionInfo,
        settings: &ToolSettings,
    ) -> Option<String> {
        match evaluate_permission_decision(action, settings) {
            PermissionDecision::DenyFailClosed { reason } => Some(reason),
            _ => None,
        }
    }

    #[test]
    fn build_action_hook_payload_marks_failures() {
        let payload = build_action_hook_payload(
            None,
            "char-1",
            Some("chat".to_string()),
            &sample_invocation(),
            Some(&sample_action()),
            Some(false),
            Some("Tool disabled".to_string()),
        );

        let HookPayload::Action(action) = payload else {
            panic!("expected action payload");
        };

        assert_eq!(action.action_id.as_deref(), Some("builtin__search_memory"));
        assert_eq!(action.success, Some(false));
        assert_eq!(action.result_message.as_deref(), Some("Tool disabled"));
        assert_eq!(action.source.as_deref(), Some("chat"));
    }

    #[test]
    fn build_action_hook_payload_preserves_success_result() {
        let payload = build_action_hook_payload(
            Some("conv-1".to_string()),
            "char-1",
            Some("direct_execute".to_string()),
            &sample_invocation(),
            Some(&sample_action()),
            Some(true),
            Some("ok".to_string()),
        );

        let HookPayload::Action(action) = payload else {
            panic!("expected action payload");
        };

        assert_eq!(action.conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(action.action_name, "search_memory");
        assert_eq!(action.success, Some(true));
        assert_eq!(action.result_message.as_deref(), Some("ok"));
    }

    #[test]
    fn denied_by_hook_message_uses_stable_prefix() {
        assert_eq!(denied_by_hook_message("blocked"), "Denied by hook: blocked");
    }

    #[test]
    fn continue_unless_denied_short_circuits_on_deny() {
        let mut called = false;
        let result = continue_unless_denied(
            HookOutcome::Deny {
                reason: "blocked".to_string(),
            },
            || {
                called = true;
                "executed"
            },
        );

        assert_eq!(result, Err("Denied by hook: blocked".to_string()));
        assert!(!called);
    }

    #[test]
    fn continue_unless_denied_runs_continuation_on_continue() {
        let mut called = false;
        let result = continue_unless_denied(HookOutcome::Continue, || {
            called = true;
            "executed"
        });

        assert_eq!(result, Ok("executed"));
        assert!(called);
    }

    #[test]
    fn build_action_hook_payload_carries_denied_result() {
        let payload = build_action_hook_payload(
            None,
            "char-1",
            Some("chat".to_string()),
            &sample_invocation(),
            Some(&sample_action()),
            Some(false),
            Some(denied_by_hook_message("blocked")),
        );

        let HookPayload::Action(action) = payload else {
            panic!("expected action payload");
        };

        assert_eq!(action.success, Some(false));
        assert_eq!(
            action.result_message.as_deref(),
            Some("Denied by hook: blocked")
        );
    }

    #[test]
    fn build_before_action_args_payload_uses_resolved_action_identity() {
        let payload = build_before_action_args_payload(
            None,
            "char-1",
            Some("chat".to_string()),
            &sample_invocation(),
            &sample_action(),
        );

        assert_eq!(payload.character_id, "char-1");
        assert_eq!(payload.tool_call_id.as_deref(), Some("tool-call-1"));
        assert_eq!(payload.action_id, "builtin__search_memory");
        assert_eq!(payload.action_name, "search_memory");
        assert_eq!(payload.args.get("query"), Some(&"kokoro".to_string()));
        assert_eq!(payload.source.as_deref(), Some("chat"));
    }

    #[test]
    fn apply_before_action_args_payload_returns_modified_args() {
        let mut payload = build_before_action_args_payload(
            None,
            "char-1",
            Some("chat".to_string()),
            &sample_invocation(),
            &sample_action(),
        );
        payload
            .args
            .insert("query".to_string(), "kokoro refined".to_string());
        payload.args.insert("limit".to_string(), "3".to_string());

        let args = apply_before_action_args_payload(payload);

        assert_eq!(args.get("query"), Some(&"kokoro refined".to_string()));
        assert_eq!(args.get("limit"), Some(&"3".to_string()));
    }

    #[test]
    fn policy_denial_reason_allows_safe_action_under_default_policy() {
        assert_eq!(
            policy_denial_reason(&sample_action(), &sample_default_policy_settings()),
            None
        );
    }

    #[test]
    fn policy_denial_reason_blocks_elevated_action_when_max_is_safe() {
        assert_eq!(
            policy_denial_reason(
                &sample_policy_permission_action(),
                &sample_policy_permission_settings()
            ),
            None
        );
    }

    #[test]
    fn policy_denial_reason_blocks_low_risk_tag_without_approval_semantics() {
        assert_eq!(
            policy_denial_reason(&sample_policy_only_action(), &sample_policy_only_settings()),
            Some("Denied by policy: blocked risk tag 'read'".to_string())
        );
        assert_eq!(
            policy_denial_reason(
                &sample_policy_external_action(),
                &sample_policy_external_settings()
            ),
            Some("Denied by policy: blocked risk tag 'external'".to_string())
        );
    }

    #[test]
    fn approval_pending_reason_requires_approval_for_elevated_action() {
        assert_eq!(
            approval_pending_reason(
                &sample_pending_elevated_action(),
                &sample_pending_elevated_settings()
            ),
            Some(
                "Denied pending approval: permission level 'elevated' requires approval"
                    .to_string()
            )
        );
    }

    #[test]
    fn approval_pending_reason_requires_approval_for_write_and_sensitive_tags() {
        assert_eq!(
            approval_pending_reason(
                &sample_pending_write_action(),
                &sample_pending_write_settings()
            ),
            Some("Denied pending approval: risk tag 'write' requires approval".to_string())
        );
        assert_eq!(
            approval_pending_reason(
                &sample_sensitive_action(),
                &sample_sensitive_blocking_settings()
            ),
            None
        );
    }

    #[test]
    fn approval_pending_reason_prefers_permission_message_when_permission_and_tag_both_require_approval(
    ) {
        assert_eq!(
            approval_pending_reason(
                &sample_pending_prefers_permission_action(),
                &sample_pending_prefers_permission_settings(),
            ),
            Some(
                "Denied pending approval: permission level 'elevated' requires approval"
                    .to_string()
            )
        );
    }

    #[test]
    fn approval_pending_reason_allows_default_safe_action() {
        assert_eq!(
            approval_pending_reason(
                &sample_default_allow_action(),
                &sample_default_allow_settings()
            ),
            None
        );
    }

    #[test]
    fn high_risk_fail_closed_reason_blocks_elevated_sensitive_action_when_max_is_safe() {
        assert_eq!(
            high_risk_fail_closed_reason(
                &sample_fail_closed_prefers_permission_action(),
                &sample_fail_closed_prefers_permission_settings(),
            ),
            Some(
                "Denied by fail-closed policy: permission level 'elevated' exceeds max allowed 'safe'"
                    .to_string()
            )
        );
    }

    #[test]
    fn high_risk_fail_closed_reason_blocks_sensitive_tag() {
        assert_eq!(
            high_risk_fail_closed_reason(
                &sample_fail_closed_sensitive_action(),
                &sample_fail_closed_sensitive_settings(),
            ),
            Some("Denied by fail-closed policy: blocked risk tag 'sensitive'".to_string())
        );
    }

    #[test]
    fn approval_pending_and_fail_closed_keep_distinct_non_matching_tags_open() {
        assert_eq!(
            approval_pending_reason(
                &sample_pending_non_high_risk_action(),
                &sample_pending_non_high_risk_settings(),
            ),
            None
        );
        assert_eq!(
            high_risk_fail_closed_reason(
                &sample_fail_closed_non_high_risk_action(),
                &sample_fail_closed_non_high_risk_settings(),
            ),
            None
        );
    }

    #[test]
    fn deny_helpers_keep_stable_prefixes() {
        let policy =
            policy_denial_reason(&sample_policy_only_action(), &sample_policy_only_settings())
                .expect("policy should deny blocked read tag");
        let pending = approval_pending_reason(
            &sample_pending_elevated_action(),
            &sample_pending_elevated_settings(),
        )
        .expect("approval should deny elevated action under safe ceiling");
        let fail_closed = high_risk_fail_closed_reason(
            &sample_fail_closed_sensitive_action(),
            &sample_fail_closed_sensitive_settings(),
        )
        .expect("fail-closed should deny sensitive action");

        assert!(policy.starts_with("Denied by policy:"));
        assert!(pending.starts_with("Denied pending approval:"));
        assert!(fail_closed.starts_with("Denied by fail-closed policy:"));
    }

    #[test]
    fn deny_helpers_allow_default_policy_when_no_rule_matches() {
        assert_eq!(
            policy_denial_reason(&sample_no_denial_action(), &sample_no_denial_settings()),
            None
        );
        assert_eq!(
            approval_pending_reason(&sample_no_denial_action(), &sample_no_denial_settings()),
            None
        );
        assert_eq!(
            high_risk_fail_closed_reason(&sample_no_denial_action(), &sample_no_denial_settings()),
            None
        );
    }

    #[test]
    fn deny_helpers_keep_policy_pending_and_fail_closed_semantics_distinct() {
        assert_eq!(
            policy_denial_reason(&sample_policy_tag_action(), &sample_policy_tag_settings()),
            Some("Denied by policy: blocked risk tag 'read'".to_string())
        );
        assert_eq!(
            approval_pending_reason(
                &sample_pending_write_only_action(),
                &sample_pending_write_only_settings()
            ),
            Some("Denied pending approval: risk tag 'write' requires approval".to_string())
        );
        assert_eq!(
            high_risk_fail_closed_reason(
                &sample_fail_closed_sensitive_only_action(),
                &sample_fail_closed_sensitive_only_settings(),
            ),
            Some("Denied by fail-closed policy: blocked risk tag 'sensitive'".to_string())
        );
    }

    #[test]
    fn approval_pending_reason_does_not_handle_read_only_blocks() {
        assert_eq!(
            approval_pending_reason(
                &sample_pending_read_only_action(),
                &sample_pending_read_only_settings()
            ),
            None
        );
    }

    #[test]
    fn high_risk_fail_closed_reason_does_not_handle_external_only_blocks() {
        assert_eq!(
            high_risk_fail_closed_reason(
                &sample_fail_closed_external_only_action(),
                &sample_fail_closed_external_only_settings(),
            ),
            None
        );
    }

    #[test]
    fn approval_pending_reason_uses_stable_prefix() {
        let reason = approval_pending_reason(
            &sample_pending_elevated_action(),
            &sample_pending_elevated_settings(),
        )
        .expect("approval-pending should deny elevated action under safe ceiling");

        assert!(reason.starts_with("Denied pending approval:"));
    }

    #[test]
    fn high_risk_fail_closed_reason_uses_stable_prefix() {
        let reason = high_risk_fail_closed_reason(
            &sample_fail_closed_sensitive_action(),
            &sample_fail_closed_sensitive_settings(),
        )
        .expect("fail-closed should deny sensitive action");

        assert!(reason.starts_with("Denied by fail-closed policy:"));
    }

    #[test]
    fn policy_denial_reason_uses_stable_prefix() {
        let reason =
            policy_denial_reason(&sample_policy_only_action(), &sample_policy_only_settings())
                .expect("policy should deny blocked read tag");

        assert!(reason.starts_with("Denied by policy:"));
    }
}

impl ToolExecutionOutcome {
    pub fn tool_id(&self) -> &str {
        self.action
            .as_ref()
            .map(|action| action.id.as_str())
            .unwrap_or(self.invocation.name.as_str())
    }

    pub fn tool_name(&self) -> &str {
        self.action
            .as_ref()
            .map(|action| action.name.as_str())
            .unwrap_or(self.invocation.name.as_str())
    }

    pub fn tool_source(&self) -> Option<&str> {
        self.action.as_ref().map(|action| match action.source {
            crate::actions::registry::ActionSource::Builtin => "builtin",
            crate::actions::registry::ActionSource::Mcp => "mcp",
        })
    }

    pub fn tool_server_name(&self) -> Option<&str> {
        self.action
            .as_ref()
            .and_then(|action| action.server_name.as_deref())
    }

    pub fn tool_needs_feedback(&self) -> bool {
        self.needs_feedback
    }

    pub fn tool_permission_level(&self) -> Option<&str> {
        self.action
            .as_ref()
            .map(|action| match action.permission_level {
                crate::actions::registry::ActionPermissionLevel::Safe => "safe",
                crate::actions::registry::ActionPermissionLevel::Elevated => "elevated",
            })
    }

    pub fn tool_risk_tags(&self) -> Vec<&'static str> {
        self.action
            .as_ref()
            .map(|action| {
                action
                    .risk_tags
                    .iter()
                    .map(risk_tag_label)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub fn result_line(&self) -> String {
        match &self.result {
            Ok(result) => format!("- {}: {}", self.tool_id(), result.message),
            Err(error) => format!("- {}: Error: {}", self.tool_id(), error),
        }
    }
}

pub(crate) fn tool_metadata_value(
    outcome: &ToolExecutionOutcome,
    tool_call_id: &str,
    turn_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_result",
        "turn_id": turn_id,
        "tool_call_id": tool_call_id,
        "tool_id": outcome.tool_id(),
        "tool_name": outcome.tool_name(),
        "source": outcome.tool_source(),
        "server_name": outcome.tool_server_name(),
        "needs_feedback": outcome.needs_feedback,
        "permission_level": outcome.tool_permission_level(),
        "risk_tags": outcome.tool_risk_tags(),
    })
}

pub(crate) fn assistant_tool_call_metadata_value(
    outcome: &ToolExecutionOutcome,
    tool_call_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": tool_call_id,
        "tool_id": outcome.tool_id(),
        "tool_name": outcome.tool_name(),
        "source": outcome.tool_source(),
        "server_name": outcome.tool_server_name(),
        "needs_feedback": outcome.needs_feedback,
        "permission_level": outcome.tool_permission_level(),
        "risk_tags": outcome.tool_risk_tags(),
        "arguments": serde_json::to_string(&outcome.invocation.args)
            .unwrap_or_else(|_| "{}".to_string()),
    })
}

#[cfg(test)]
pub(crate) fn tool_metadata_value_for_test(
    outcome: &ToolExecutionOutcome,
    tool_call_id: &str,
    turn_id: &str,
) -> serde_json::Value {
    tool_metadata_value(outcome, tool_call_id, turn_id)
}

#[cfg(test)]
pub(crate) fn assistant_tool_call_metadata_value_for_test(
    outcome: &ToolExecutionOutcome,
    tool_call_id: &str,
) -> serde_json::Value {
    assistant_tool_call_metadata_value(outcome, tool_call_id)
}

pub async fn execute_tool_calls(
    app: &tauri::AppHandle,
    registry_state: &Arc<RwLock<ActionRegistry>>,
    tool_settings_state: &Arc<RwLock<ToolSettings>>,
    character_id: &str,
    tool_calls: &[ToolInvocation],
) -> Vec<ToolExecutionOutcome> {
    let mut outcomes = Vec::with_capacity(tool_calls.len());
    let hook_runtime = app.try_state::<HookRuntime>();

    for tool_call in tool_calls {
        let gate = if let Some(hooks) = hook_runtime.as_ref() {
            hooks
                .emit_action_gate(
                    &HookEvent::BeforeActionInvoke,
                    &build_action_hook_payload(
                        None,
                        character_id,
                        Some("chat".to_string()),
                        tool_call,
                        None,
                        None,
                        None,
                    ),
                )
                .await
        } else {
            HookOutcome::Continue
        };

        let gated = continue_unless_denied(gate, || ());
        let (action, needs_feedback, permission_decision, result) = match gated {
            Err(error) => (None, true, None, Err(error)),
            Ok(()) => {
                let resolved = {
                    let registry = registry_state.read().await;
                    registry.resolve_action_for_execution(&tool_call.name)
                };
                let needs_feedback = resolved
                    .as_ref()
                    .map(|(action, _)| action.needs_feedback)
                    .unwrap_or(true);

                let action = resolved.as_ref().ok().map(|(action, _)| action.clone());
                let (permission_decision, result) = match &resolved {
                    Ok((action, handler)) => {
                        let enabled = {
                            let tool_settings = tool_settings_state.read().await;
                            tool_settings.is_enabled(&action.id)
                        };

                        if !enabled {
                            (None, Err(format!("Tool '{}' is disabled", action.id)))
                        } else {
                            let permission_decision = {
                                let tool_settings = tool_settings_state.read().await;
                                evaluate_permission_decision(action, &tool_settings)
                            };
                            match permission_decision.clone() {
                                PermissionDecision::Allow => {
                                    let mut args_payload = build_before_action_args_payload(
                                        None,
                                        character_id,
                                        Some("chat".to_string()),
                                        tool_call,
                                        action,
                                    );
                                    if let Some(hooks) = hook_runtime.as_ref() {
                                        if let Err(error) = hooks
                                            .emit_before_action_args_modify(
                                                &mut args_payload,
                                                HookModifyPolicy::Strict,
                                            )
                                            .await
                                        {
                                            (Some(permission_decision), Err(error))
                                        } else {
                                            let effective_args =
                                                apply_before_action_args_payload(args_payload);
                                            let ctx = ActionContext {
                                                app: app.clone(),
                                                character_id: character_id.to_string(),
                                                conversation_id: None,
                                                source: Some("chat".to_string()),
                                            };
                                            (
                                                Some(permission_decision),
                                                handler
                                                    .execute(effective_args, ctx)
                                                    .await
                                                    .map_err(|e| e.0),
                                            )
                                        }
                                    } else {
                                        let effective_args =
                                            apply_before_action_args_payload(args_payload);
                                        let ctx = ActionContext {
                                            app: app.clone(),
                                            character_id: character_id.to_string(),
                                            conversation_id: None,
                                            source: Some("chat".to_string()),
                                        };
                                        (
                                            Some(permission_decision),
                                            handler
                                                .execute(effective_args, ctx)
                                                .await
                                                .map_err(|e| e.0),
                                        )
                                    }
                                }
                                PermissionDecision::DenyPolicy { reason }
                                | PermissionDecision::DenyPendingApproval { reason }
                                | PermissionDecision::DenyFailClosed { reason } => {
                                    (Some(permission_decision), Err(reason))
                                }
                            }
                        }
                    }
                    Err(error) => (None, Err(error.0.clone())),
                };

                (action, needs_feedback, permission_decision, result)
            }
        };

        if let Some(hooks) = hook_runtime.as_ref() {
            let result_message = match &result {
                Ok(value) => Some(value.message.clone()),
                Err(error) => Some(error.clone()),
            };
            hooks
                .emit_best_effort(
                    &HookEvent::AfterActionInvoke,
                    &build_action_hook_payload(
                        None,
                        character_id,
                        Some("chat".to_string()),
                        tool_call,
                        action.as_ref(),
                        Some(result.is_ok()),
                        result_message,
                    ),
                )
                .await;
        }

        outcomes.push(ToolExecutionOutcome {
            invocation: tool_call.clone(),
            action,
            result,
            needs_feedback,
            permission_decision,
        });
        continue;
    }

    outcomes
}
