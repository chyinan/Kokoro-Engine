// pattern: Mixed (needs refactoring)
// Reason: 该命令文件同时承担 Tauri 命令编排与最小 hook 接线；本次只做直调 action deny 对齐，不额外拆分命令层。
use crate::actions::executor::{
    build_action_hook_payload, continue_unless_denied, denied_by_hook_message, ToolInvocation,
};
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
}

fn deny_hook_validation_error(reason: &str) -> KokoroError {
    KokoroError::Validation(denied_by_hook_message(reason))
}

fn continue_direct_action<T>(
    gate: HookOutcome,
    on_continue: impl FnOnce() -> T,
) -> Result<T, KokoroError> {
    continue_unless_denied(gate, on_continue)
        .map_err(|message| deny_hook_validation_error(message.strip_prefix("Denied by hook: ").unwrap_or(&message)))
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
    let invocation = build_direct_invocation(&name, &args);

    if let Err(error) = gate_direct_action(&app, &character_id, &invocation).await {
        emit_after_action_hook(&app, &character_id, &invocation, None, &Err(error.clone())).await;
        return Err(error);
    }

    let action = {
        let registry = registry_state.read().await;
        registry
            .resolve_action(&name)
            .map_err(|e| KokoroError::Validation(e.to_string()))
    };
    let action = match action {
        Ok(action) => action,
        Err(error) => {
            emit_after_action_hook(&app, &character_id, &invocation, None, &Err(error.clone())).await;
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
            &invocation,
            Some(&action),
            &Err(error.clone()),
        )
        .await;
        return Err(error);
    }

    let ctx = ActionContext {
        app: app.clone(),
        character_id: character_id.clone(),
        conversation_id: None,
        source: Some("direct_execute".to_string()),
    };
    let result = {
        let registry = registry_state.read().await;
        registry
            .execute(&action.id, args, ctx)
            .await
            .map_err(|e| KokoroError::Internal(e.to_string()))
    };

    emit_after_action_hook(&app, &character_id, &invocation, Some(&action), &result).await;
    result
}
