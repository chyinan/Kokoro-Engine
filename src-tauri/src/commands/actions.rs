use crate::actions::tool_settings::ToolSettings;
use crate::actions::{ActionContext, ActionInfo, ActionRegistry, ActionResult};
use crate::error::KokoroError;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{command, AppHandle, State};
use tokio::sync::RwLock;

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
    let tool_settings = tool_settings_state.read().await;
    if !tool_settings.is_enabled(&name) {
        return Err(KokoroError::Validation(format!(
            "Tool '{}' is disabled",
            name
        )));
    }
    drop(tool_settings);
    let registry = registry_state.read().await;
    let ctx = ActionContext {
        app: app.clone(),
        character_id: character_id.unwrap_or_else(|| "default".to_string()),
    };
    registry
        .execute(&name, args, ctx)
        .await
        .map_err(|e| KokoroError::Internal(e.to_string()))
}
