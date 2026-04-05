// pattern: Mixed (needs refactoring)
// Reason: 该命令文件同时承担 Tauri 命令编排与最小 hook 接线；本次只做直调 action deny 对齐，不额外拆分命令层。
use crate::actions::executor::{
    apply_before_action_args_payload, build_action_hook_payload,
    build_before_action_args_payload, continue_unless_denied, denied_by_hook_message,
    ToolInvocation,
};
use crate::actions::permission::{evaluate_permission_decision, PermissionDecision};
use crate::actions::tool_settings::ToolSettings;
use crate::actions::{ActionContext, ActionInfo, ActionRegistry, ActionResult};
use crate::error::KokoroError;
use crate::hooks::{HookEvent, HookOutcome, HookRuntime};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{command, AppHandle, Manager, State};
use tokio::sync::RwLock;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::registry::{ActionPermissionLevel, ActionRiskTag};

    fn sample_elevated_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__set_background".to_string(),
            name: "set_background".to_string(),
            source: crate::actions::registry::ActionSource::Builtin,
            server_name: None,
            description: "Set background".to_string(),
            parameters: vec![],
            needs_feedback: false,
            risk_tags: vec![ActionRiskTag::Write],
            permission_level: ActionPermissionLevel::Elevated,
        }
    }

    fn sample_read_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__search_memory".to_string(),
            name: "search_memory".to_string(),
            source: crate::actions::registry::ActionSource::Builtin,
            server_name: None,
            description: "Search memory".to_string(),
            parameters: vec![],
            needs_feedback: true,
            risk_tags: vec![ActionRiskTag::Read],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_write_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__write_note".to_string(),
            name: "write_note".to_string(),
            source: crate::actions::registry::ActionSource::Builtin,
            server_name: None,
            description: "Write note".to_string(),
            parameters: vec![],
            needs_feedback: false,
            risk_tags: vec![ActionRiskTag::Write],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_sensitive_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__store_memory".to_string(),
            name: "store_memory".to_string(),
            source: crate::actions::registry::ActionSource::Builtin,
            server_name: None,
            description: "Store memory".to_string(),
            parameters: vec![],
            needs_feedback: false,
            risk_tags: vec![ActionRiskTag::Sensitive],
            permission_level: ActionPermissionLevel::Safe,
        }
    }

    fn sample_sensitive_elevated_action() -> ActionInfo {
        ActionInfo {
            id: "builtin__store_secret".to_string(),
            name: "store_secret".to_string(),
            source: crate::actions::registry::ActionSource::Builtin,
            server_name: None,
            description: "Store secret".to_string(),
            parameters: vec![],
            needs_feedback: false,
            risk_tags: vec![ActionRiskTag::Sensitive],
            permission_level: ActionPermissionLevel::Elevated,
        }
    }

    fn sample_safe_policy_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Safe,
            blocked_risk_tags: Vec::new(),
        }
    }

    fn sample_read_blocking_policy_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Elevated,
            blocked_risk_tags: vec![ActionRiskTag::Read],
        }
    }

    fn sample_write_blocking_policy_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Elevated,
            blocked_risk_tags: vec![ActionRiskTag::Write],
        }
    }

    fn sample_sensitive_blocking_policy_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Elevated,
            blocked_risk_tags: vec![ActionRiskTag::Sensitive],
        }
    }

    fn sample_safe_blocked_sensitive_settings() -> ToolSettings {
        ToolSettings {
            max_tool_rounds: 10,
            enabled_tools: HashMap::new(),
            max_permission_level: ActionPermissionLevel::Safe,
            blocked_risk_tags: vec![ActionRiskTag::Sensitive],
        }
    }

    #[test]
    fn continue_direct_action_short_circuits_on_deny() {
        let mut called = false;
        let result = continue_direct_action(
            HookOutcome::Deny {
                reason: "blocked".to_string(),
            },
            || {
                called = true;
                "executed"
            },
        );

        match result {
            Err(KokoroError::Validation(message)) => {
                assert_eq!(message, "Denied by hook: blocked");
            }
            other => panic!("expected validation error, got {other:?}"),
        }
        assert!(!called);
    }

    #[test]
    fn continue_direct_action_keeps_stable_message_format() {
        let result = continue_direct_action(
            HookOutcome::Deny {
                reason: "blocked".to_string(),
            },
            || "executed",
        );

        match result {
            Err(KokoroError::Validation(message)) => {
                assert_eq!(message, "Denied by hook: blocked");
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn direct_action_args_modify_helper_returns_modified_args() {
        let invocation = build_direct_invocation(
            "search_memory",
            &HashMap::from([("query".to_string(), "kokoro".to_string())]),
        );
        let action = sample_read_action();

        let mut payload = build_before_action_args_payload(
            None,
            "char-1",
            Some("direct_execute".to_string()),
            &invocation,
            &action,
        );
        payload.args.insert("query".to_string(), "refined".to_string());
        payload.args.insert("limit".to_string(), "2".to_string());

        let args = apply_before_action_args_payload(payload);

        assert_eq!(args.get("query"), Some(&"refined".to_string()));
        assert_eq!(args.get("limit"), Some(&"2".to_string()));
    }

    #[test]
    fn apply_direct_action_args_modification_returns_modified_args() {
        let invocation = build_direct_invocation(
            "search_memory",
            &HashMap::from([("query".to_string(), "kokoro".to_string())]),
        );
        let action = sample_read_action();

        let args = apply_direct_action_args_modification(
            "char-1",
            &invocation,
            &action,
            HashMap::from([
                ("query".to_string(), "refined".to_string()),
                ("limit".to_string(), "2".to_string()),
            ]),
        );

        assert_eq!(args.get("query"), Some(&"refined".to_string()));
        assert_eq!(args.get("limit"), Some(&"2".to_string()));
    }

    #[test]
    fn direct_execute_policy_denial_uses_shared_helper_message() {
        let decision = evaluate_permission_decision(&sample_read_action(), &sample_read_blocking_policy_settings());

        assert_eq!(
            decision,
            PermissionDecision::DenyPolicy {
                reason: "Denied by policy: blocked risk tag 'read'".to_string(),
            }
        );
    }

    #[test]
    fn direct_execute_pending_approval_uses_shared_helper_message() {
        let decision = evaluate_permission_decision(&sample_elevated_action(), &sample_safe_policy_settings());

        assert_eq!(
            decision,
            PermissionDecision::DenyPendingApproval {
                reason: "Denied pending approval: permission level 'elevated' requires approval".to_string(),
            }
        );
    }

    #[test]
    fn direct_execute_pending_approval_can_block_write_tag() {
        let decision = evaluate_permission_decision(&sample_write_action(), &sample_write_blocking_policy_settings());

        assert_eq!(
            decision,
            PermissionDecision::DenyPendingApproval {
                reason: "Denied pending approval: risk tag 'write' requires approval".to_string(),
            }
        );
    }

    #[test]
    fn direct_execute_fail_closed_uses_shared_helper_message() {
        let decision = evaluate_permission_decision(
            &sample_sensitive_elevated_action(),
            &sample_safe_blocked_sensitive_settings(),
        );

        assert_eq!(
            decision,
            PermissionDecision::DenyFailClosed {
                reason: "Denied by fail-closed policy: permission level 'elevated' exceeds max allowed 'safe'".to_string(),
            }
        );
    }

    #[test]
    fn direct_execute_fail_closed_can_block_sensitive_tag() {
        let decision = evaluate_permission_decision(&sample_sensitive_action(), &sample_sensitive_blocking_policy_settings());

        assert_eq!(
            decision,
            PermissionDecision::DenyFailClosed {
                reason: "Denied by fail-closed policy: blocked risk tag 'sensitive'".to_string(),
            }
        );
    }

    #[test]
    fn build_tool_invocation_from_input_accepts_unique_alias() {
        let mut registry = ActionRegistry::new();
        registry.register(crate::actions::builtin::GetTimeAction);

        let invocation = build_tool_invocation_from_input(
            &registry,
            "get_time",
            HashMap::from([("tz".to_string(), "UTC".to_string())]),
            None,
        )
        .unwrap();

        assert_eq!(invocation.name, "builtin__get_time");
        assert_eq!(invocation.args.get("tz"), Some(&"UTC".to_string()));
    }

    #[test]
    fn build_tool_invocation_from_input_accepts_canonical_id() {
        let mut registry = ActionRegistry::new();
        registry.register(crate::actions::builtin::GetTimeAction);

        let invocation = build_tool_invocation_from_input(
            &registry,
            "builtin__get_time",
            HashMap::new(),
            None,
        )
        .unwrap();

        assert_eq!(invocation.name, "builtin__get_time");
    }

    #[test]
    fn build_tool_invocation_from_input_rejects_ambiguous_alias() {
        struct SearchAction;

        #[async_trait::async_trait]
        impl crate::actions::registry::ActionHandler for SearchAction {
            fn name(&self) -> &str { "search" }
            fn description(&self) -> &str { "search" }
            fn parameters(&self) -> Vec<crate::actions::registry::ActionParam> { vec![] }
            async fn execute(
                &self,
                _args: HashMap<String, String>,
                _ctx: ActionContext,
            ) -> Result<ActionResult, crate::actions::registry::ActionError> {
                Ok(ActionResult::ok("ok"))
            }
        }

        let mut registry = ActionRegistry::new();
        registry.register(SearchAction);
        registry.register_mcp("server_a", SearchAction);

        let err = build_tool_invocation_from_input(&registry, "search", HashMap::new(), None).unwrap_err();
        assert!(err.0.contains("Ambiguous tool 'search'"));
    }

    #[test]
    fn build_direct_invocation_keeps_raw_name_until_boundary_resolves() {
        let invocation = build_direct_invocation(
            "get_time",
            &HashMap::from([("tz".to_string(), "UTC".to_string())]),
        );

        assert_eq!(invocation.name, "get_time");
        assert_eq!(invocation.args.get("tz"), Some(&"UTC".to_string()));
    }
}

fn deny_hook_validation_error(reason: &str) -> KokoroError {
    KokoroError::Validation(denied_by_hook_message(reason))
}

fn continue_direct_action<T>(
    gate: HookOutcome,
    on_continue: impl FnOnce() -> T,
) -> Result<T, KokoroError> {
    continue_unless_denied(gate, on_continue).map_err(|message| {
        deny_hook_validation_error(message.strip_prefix("Denied by hook: ").unwrap_or(&message))
    })
}

fn result_message_for_hook(result: &Result<ActionResult, KokoroError>) -> String {
    match result {
        Ok(value) => value.message.clone(),
        Err(error) => hook_error_message(error),
    }
}

fn hook_error_message(error: &KokoroError) -> String {
    match error {
        KokoroError::Config(message)
        | KokoroError::Database(message)
        | KokoroError::Llm(message)
        | KokoroError::Tts(message)
        | KokoroError::Stt(message)
        | KokoroError::Io(message)
        | KokoroError::ExternalService(message)
        | KokoroError::Mod(message)
        | KokoroError::NotFound(message)
        | KokoroError::Unauthorized(message)
        | KokoroError::Internal(message)
        | KokoroError::Chat(message)
        | KokoroError::Validation(message) => message.clone(),
    }
}

async fn emit_after_action_hook(
    app: &AppHandle,
    character_id: &str,
    invocation: &ToolInvocation,
    action: Option<&ActionInfo>,
    result: &Result<ActionResult, KokoroError>,
) {
    if let Some(hooks) = app.try_state::<HookRuntime>() {
        hooks
            .emit_best_effort(
                &HookEvent::AfterActionInvoke,
                &build_action_hook_payload(
                    None,
                    character_id,
                    Some("direct_execute".to_string()),
                    invocation,
                    action,
                    Some(result.is_ok()),
                    Some(result_message_for_hook(result)),
                ),
            )
            .await;
    }
}

async fn gate_direct_action(
    app: &AppHandle,
    character_id: &str,
    invocation: &ToolInvocation,
) -> Result<(), KokoroError> {
    let Some(hooks) = app.try_state::<HookRuntime>() else {
        return Ok(());
    };

    continue_direct_action(
        hooks
            .emit_action_gate(
                &HookEvent::BeforeActionInvoke,
                &build_action_hook_payload(
                    None,
                    character_id,
                    Some("direct_execute".to_string()),
                    invocation,
                    None,
                    None,
                    None,
                ),
            )
            .await,
        || (),
    )?;

    Ok(())
}

fn build_direct_invocation(name: &str, args: &HashMap<String, String>) -> ToolInvocation {
    ToolInvocation {
        tool_call_id: None,
        name: name.to_string(),
        args: args.clone(),
    }
}

fn build_resolved_direct_invocation(action_id: &str, args: &HashMap<String, String>) -> ToolInvocation {
    ToolInvocation {
        tool_call_id: None,
        name: action_id.to_string(),
        args: args.clone(),
    }
}

pub(crate) fn build_tool_invocation_from_input(
    registry: &ActionRegistry,
    input: &str,
    args: HashMap<String, String>,
    tool_call_id: Option<String>,
) -> Result<ToolInvocation, crate::actions::registry::ActionError> {
    let action_id = registry.resolve_action_id_for_input(input)?;
    Ok(ToolInvocation {
        tool_call_id,
        name: action_id,
        args,
    })
}

async fn resolve_action_id_at_boundary(
    registry_state: &State<'_, Arc<RwLock<ActionRegistry>>>,
    input: &str,
) -> Result<String, KokoroError> {
    let registry = registry_state.read().await;
    registry
        .resolve_action_id_for_input(input)
        .map_err(|error| KokoroError::Validation(error.0))
}

#[cfg(test)]
fn apply_direct_action_args_modification(
    character_id: &str,
    invocation: &ToolInvocation,
    action: &ActionInfo,
    args: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut payload = build_before_action_args_payload(
        None,
        character_id,
        Some("direct_execute".to_string()),
        invocation,
        action,
    );
    payload.args = args;
    apply_before_action_args_payload(payload)
}

async fn modify_direct_action_args(
    app: &AppHandle,
    character_id: &str,
    invocation: &ToolInvocation,
    action: &ActionInfo,
    args: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut payload = build_before_action_args_payload(
        None,
        character_id,
        Some("direct_execute".to_string()),
        invocation,
        action,
    );
    payload.args = args;

    if let Some(hooks) = app.try_state::<HookRuntime>() {
        hooks.emit_before_action_args_modify(&mut payload).await;
    }

    apply_before_action_args_payload(payload)
}

#[command]
pub async fn list_actions(
    state: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<Vec<crate::actions::ActionInfo>, KokoroError> {
    let registry = state.read().await;
    Ok(registry.list_actions())
}

#[command]
pub async fn list_builtin_tools(
    registry_state: State<'_, Arc<RwLock<ActionRegistry>>>,
) -> Result<Vec<ActionInfo>, KokoroError> {
    let registry = registry_state.read().await;
    Ok(registry.list_builtin_actions())
}

#[command]
pub async fn execute_action(
    app: AppHandle,
    registry_state: State<'_, Arc<RwLock<ActionRegistry>>>,
    tool_settings_state: State<'_, Arc<RwLock<ToolSettings>>>,
    name: String,
    args: HashMap<String, String>,
    character_id: Option<String>,
) -> Result<ActionResult, KokoroError> {
    let character_id = character_id.unwrap_or_else(|| "default".to_string());
    let raw_invocation = build_direct_invocation(&name, &args);

    if let Err(error) = gate_direct_action(&app, &character_id, &raw_invocation).await {
        emit_after_action_hook(&app, &character_id, &raw_invocation, None, &Err(error.clone())).await;
        return Err(error);
    }

    let action_id = match resolve_action_id_at_boundary(&registry_state, &name).await {
        Ok(action_id) => action_id,
        Err(error) => {
            emit_after_action_hook(&app, &character_id, &raw_invocation, None, &Err(error.clone())).await;
            return Err(error);
        }
    };
    let invocation = build_resolved_direct_invocation(&action_id, &args);

    let action = {
        let registry = registry_state.read().await;
        registry
            .resolve_action(&action_id)
            .map_err(|e| KokoroError::Validation(e.to_string()))
    };
    let action = match action {
        Ok(action) => action,
        Err(error) => {
            emit_after_action_hook(&app, &character_id, &raw_invocation, None, &Err(error.clone())).await;
            return Err(error);
        }
    };

    let enabled = {
        let tool_settings = tool_settings_state.read().await;
        tool_settings.is_enabled(&action.id)
    };
    if !enabled {
        let error = KokoroError::Validation(format!("Tool '{}' is disabled", action.id));
        emit_after_action_hook(
            &app,
            &character_id,
            &raw_invocation,
            Some(&action),
            &Err(error.clone()),
        )
        .await;
        return Err(error);
    }

    let permission_decision = {
        let tool_settings = tool_settings_state.read().await;
        evaluate_permission_decision(&action, &tool_settings)
    };
    if let PermissionDecision::DenyPolicy { reason }
        | PermissionDecision::DenyPendingApproval { reason }
        | PermissionDecision::DenyFailClosed { reason } = permission_decision
    {
        let error = KokoroError::Validation(reason);
        emit_after_action_hook(
            &app,
            &character_id,
            &raw_invocation,
            Some(&action),
            &Err(error.clone()),
        )
        .await;
        return Err(error);
    }

    let effective_args = modify_direct_action_args(
        &app,
        &character_id,
        &invocation,
        &action,
        args,
    )
    .await;

    let ctx = ActionContext {
        app: app.clone(),
        character_id: character_id.clone(),
        conversation_id: None,
        source: Some("direct_execute".to_string()),
    };
    let result = {
        let registry = registry_state.read().await;
        registry
            .execute(&action.id, effective_args, ctx)
            .await
            .map_err(|e| KokoroError::Internal(e.to_string()))
    };

    emit_after_action_hook(&app, &character_id, &raw_invocation, Some(&action), &result).await;
    result
}
