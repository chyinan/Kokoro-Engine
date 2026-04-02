use crate::actions::registry::{ActionContext, ActionInfo, ActionRegistry, ActionResult};
use crate::actions::tool_settings::ToolSettings;
use std::collections::HashMap;
use std::sync::Arc;
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

    for tool_call in tool_calls {
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
                    };
                    handler.execute(tool_call.args.clone(), ctx).await.map_err(|e| e.0)
                }
            }
            Err(error) => Err(error.0.clone()),
        };

        outcomes.push(ToolExecutionOutcome {
            invocation: tool_call.clone(),
            action: resolved.ok().map(|(action, _)| action),
            result,
            needs_feedback,
        });
    }

    outcomes
}
