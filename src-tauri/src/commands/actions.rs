use crate::actions::{ActionContext, ActionRegistry, ActionResult};
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
pub async fn execute_action(
    app: AppHandle,
    state: State<'_, Arc<RwLock<ActionRegistry>>>,
    name: String,
    args: HashMap<String, String>,
    character_id: Option<String>,
) -> Result<ActionResult, KokoroError> {
    let registry = state.read().await;
    let ctx = ActionContext {
        app: app.clone(),
        character_id: character_id.unwrap_or_else(|| "default".to_string()),
    };
    registry
        .execute(&name, args, ctx)
        .await
        .map_err(|e| KokoroError::Internal(e.to_string()))
}
