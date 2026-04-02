use crate::actions::registry::{ActionContext, ActionInfo, ActionRegistry, ActionResult};
use crate::actions::tool_settings::ToolSettings;
use crate::hooks::{ActionHookPayload, HookEvent, HookPayload, HookRuntime};
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
}

fn build_action_hook_payload(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::registry::ActionSource;

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

    pub fn result_line(&self) -> String {
        match &self.result {
            Ok(result) => format!("- {}: {}", self.tool_id(), result.message),
            Err(error) => format!("- {}: Error: {}", self.tool_id(), error),
        }
    }
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
        if let Some(hooks) = hook_runtime.as_ref() {
            hooks
                .emit_best_effort(
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
                .await;
        }

        let resolved = {
            let registry = registry_state.read().await;
            registry.resolve_action_for_execution(&tool_call.name)
        };
        let needs_feedback = resolved
            .as_ref()
            .map(|(action, _)| action.needs_feedback)
            .unwrap_or(true);

        let result = match &resolved {
            Ok((action, handler)) => {
                let enabled = {
                    let tool_settings = tool_settings_state.read().await;
                    tool_settings.is_enabled(&action.id)
                };

                if !enabled {
                    Err(format!("Tool '{}' is disabled", action.id))
                } else {
                    let ctx = ActionContext {
                        app: app.clone(),
                        character_id: character_id.to_string(),
                        conversation_id: None,
                        source: Some("chat".to_string()),
                    };
                    handler.execute(tool_call.args.clone(), ctx).await.map_err(|e| e.0)
                }
            }
            Err(error) => Err(error.0.clone()),
        };

        let action = resolved.ok().map(|(action, _)| action);

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
        });
    }

    outcomes
}
