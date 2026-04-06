//! Built-in tool handlers for the Tool Registry.

use super::registry::{
    ActionContext, ActionError, ActionHandler, ActionParam, ActionPermissionLevel, ActionResult,
    ActionRiskTag,
};
use async_trait::async_trait;
use std::collections::HashMap;
use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

// ── get_time ───────────────────────────────────────────

pub struct GetTimeAction;

#[async_trait]
impl ActionHandler for GetTimeAction {
    fn name(&self) -> &str {
        "get_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![]
    }

    fn needs_feedback(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        _args: HashMap<String, String>,
        _ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let now = chrono::Local::now();
        let formatted = now.format("%Y-%m-%d %H:%M:%S (%A)").to_string();
        Ok(ActionResult::ok_with_data(
            format!("Current time: {}", formatted),
            serde_json::json!({ "time": formatted }),
        ))
    }
}

// ── play_cue ──────────────────────────────────

pub struct PlayCueAction;

#[async_trait]
impl ActionHandler for PlayCueAction {
    fn name(&self) -> &str {
        "play_cue"
    }

    fn description(&self) -> &str {
        "Trigger a configured Live2D cue"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![ActionParam {
            name: "cue".to_string(),
            description: "Configured Live2D cue name for the active model".to_string(),
            required: true,
        }]
    }

    fn risk_tags(&self) -> Vec<ActionRiskTag> {
        vec![ActionRiskTag::Write]
    }

    fn permission_level(&self) -> ActionPermissionLevel {
        ActionPermissionLevel::Elevated
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let cue = args
            .get("cue")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ActionError("Missing 'cue' parameter".into()))?;

        let profile = crate::commands::live2d::load_active_live2d_profile()
            .ok_or_else(|| ActionError("No active Live2D model profile loaded".into()))?;
        if !profile.cue_map.contains_key(cue) {
            let available_cues = profile
                .cue_map
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ActionError(format!(
                "Unknown cue '{}'. Available configured cues: {}",
                cue,
                if available_cues.is_empty() {
                    "(none)"
                } else {
                    &available_cues
                }
            )));
        }

        // Emit cue event to frontend
        let _ = ctx.app.emit(
            "chat-cue",
            serde_json::json!({ "cue": cue, "source": "builtin-play-cue" }),
        );

        Ok(ActionResult::ok(format!("Cue triggered: {}", cue)))
    }
}

// ── set_background ─────────────────────────────────────

pub struct SetBackgroundAction;

#[async_trait]
impl ActionHandler for SetBackgroundAction {
    fn name(&self) -> &str {
        "set_background"
    }

    fn description(&self) -> &str {
        "Generate and set a new background image based on a description"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![ActionParam {
            name: "prompt".to_string(),
            description: "English description of the desired background scene".to_string(),
            required: true,
        }]
    }

    fn risk_tags(&self) -> Vec<ActionRiskTag> {
        vec![ActionRiskTag::Write]
    }

    fn permission_level(&self) -> ActionPermissionLevel {
        ActionPermissionLevel::Elevated
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let prompt = args
            .get("prompt")
            .ok_or_else(|| ActionError("Missing 'prompt' parameter".into()))?;

        // Emit image gen event (reuses existing infrastructure)
        let _ = ctx
            .app
            .emit("chat-imagegen", serde_json::json!({ "prompt": prompt }));

        Ok(ActionResult::ok(format!(
            "Background generation triggered: {}",
            prompt
        )))
    }
}

// ── search_memory ──────────────────────────────────────

pub struct SearchMemoryAction;

fn ensure_memory_enabled(ctx: &ActionContext) -> Result<(), ActionError> {
    let orchestrator = ctx.app.state::<crate::ai::context::AIOrchestrator>();
    if orchestrator.is_memory_enabled() {
        Ok(())
    } else {
        Err(ActionError("Memory system is currently disabled.".into()))
    }
}

#[async_trait]
impl ActionHandler for SearchMemoryAction {
    fn name(&self) -> &str {
        "search_memory"
    }

    fn description(&self) -> &str {
        "Search through your memories about the user"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![ActionParam {
            name: "query".to_string(),
            description: "What to search for in memories".to_string(),
            required: true,
        }]
    }

    fn needs_feedback(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        ensure_memory_enabled(&ctx)?;
        let query = args
            .get("query")
            .ok_or_else(|| ActionError("Missing 'query' parameter".into()))?;

        // Get MemoryManager from app state
        let orchestrator = ctx.app.state::<crate::ai::context::AIOrchestrator>();
        let char_id = ctx.character_id.clone();
        let memories = orchestrator
            .memory_manager
            .search_memories(query, 5, &char_id)
            .await
            .map_err(|e| ActionError(format!("Memory search failed: {}", e)))?;

        if memories.is_empty() {
            Ok(ActionResult::ok("No relevant memories found."))
        } else {
            let results: Vec<String> = memories.iter().map(|m| m.content.clone()).collect();
            Ok(ActionResult::ok_with_data(
                format!("Found {} memories.", results.len()),
                serde_json::json!({ "memories": results }),
            ))
        }
    }
}

// ── store_memory ───────────────────────────────────────

pub struct StoreMemoryAction;

#[async_trait]
impl ActionHandler for StoreMemoryAction {
    fn name(&self) -> &str {
        "store_memory"
    }

    fn description(&self) -> &str {
        "Store an important fact or detail about the user to remember for future conversations"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![
            ActionParam {
                name: "fact".to_string(),
                description: "The fact or detail to remember (concise, factual statement)".to_string(),
                required: true,
            },
            ActionParam {
                name: "importance".to_string(),
                description: "Importance from 0.0 to 1.0 (0.9=critical like name/birthday, 0.7=preferences, 0.5=interesting details, 0.3=minor)".to_string(),
                required: false,
            },
        ]
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        ensure_memory_enabled(&ctx)?;
        let fact = args
            .get("fact")
            .ok_or_else(|| ActionError("Missing 'fact' parameter".into()))?;

        let importance: f64 = args
            .get("importance")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.6);

        let orchestrator = ctx.app.state::<crate::ai::context::AIOrchestrator>();
        let char_id = ctx.character_id.clone();

        orchestrator
            .memory_manager
            .add_memory_with_importance(fact, &char_id, importance)
            .await
            .map_err(|e| ActionError(format!("Failed to store memory: {}", e)))?;

        tracing::info!(
            target: "tools",
            "[Memory] Tool stored: '{}' (importance={:.1}) for '{}'",
            fact.chars().take(60).collect::<String>(),
            importance,
            char_id
        );

        // Notify frontend to refresh memory panel
        let _ = ctx.app.emit("memory:updated", &char_id);

        Ok(ActionResult::ok(format!(
            "Remembered: \"{}\" (importance: {:.1})",
            fact, importance
        )))
    }
}

// ── forget_memory ──────────────────────────────────────

pub struct ForgetMemoryAction;

#[async_trait]
impl ActionHandler for ForgetMemoryAction {
    fn name(&self) -> &str {
        "forget_memory"
    }

    fn description(&self) -> &str {
        "Search and remove a specific memory when the user asks you to forget something"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![ActionParam {
            name: "query".to_string(),
            description: "Description of the memory to forget".to_string(),
            required: true,
        }]
    }

    fn needs_feedback(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        ensure_memory_enabled(&ctx)?;
        let query = args
            .get("query")
            .ok_or_else(|| ActionError("Missing 'query' parameter".into()))?;

        let orchestrator = ctx.app.state::<crate::ai::context::AIOrchestrator>();
        let char_id = ctx.character_id.clone();

        // Find the most relevant memory matching the query
        let memories = orchestrator
            .memory_manager
            .search_memories(query, 1, &char_id)
            .await
            .map_err(|e| ActionError(format!("Memory search failed: {}", e)))?;

        if let Some(mem) = memories.first() {
            let content = mem.content.clone();
            orchestrator
                .memory_manager
                .delete_memory(mem.id)
                .await
                .map_err(|e| ActionError(format!("Failed to delete memory: {}", e)))?;

            tracing::info!(
                target: "tools",
                "[Memory] Tool forgot: '{}' for '{}'",
                &content[..content.len().min(60)],
                char_id
            );

            Ok(ActionResult::ok(format!("Forgot: \"{}\"", content)))
        } else {
            Ok(ActionResult::ok("No matching memory found to forget."))
        }
    }
}

// ── send_notification ──────────────────────────────────

pub struct SendNotificationAction;

#[async_trait]
impl ActionHandler for SendNotificationAction {
    fn name(&self) -> &str {
        "send_notification"
    }

    fn description(&self) -> &str {
        "Send a notification popup to the user"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![
            ActionParam {
                name: "title".to_string(),
                description: "Notification title".to_string(),
                required: true,
            },
            ActionParam {
                name: "message".to_string(),
                description: "Notification body text".to_string(),
                required: true,
            },
        ]
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let title = args
            .get("title")
            .ok_or_else(|| ActionError("Missing 'title' parameter".into()))?;
        let message = args
            .get("message")
            .ok_or_else(|| ActionError("Missing 'message' parameter".into()))?;

        ctx.app
            .notification()
            .builder()
            .title(title)
            .body(message)
            .show()
            .map_err(|e| ActionError(format!("Failed to show native notification: {}", e)))?;

        Ok(ActionResult::ok(format!("Notification shown: {}", title)))
    }
}

// ── Factory ────────────────────────────────────────────

/// Register all built-in action handlers into the given registry.
pub fn register_builtins(registry: &mut super::registry::ActionRegistry) {
    registry.register(GetTimeAction);
    registry.register(PlayCueAction);
    registry.register(SetBackgroundAction);
    registry.register(SearchMemoryAction);
    registry.register(StoreMemoryAction);
    registry.register(ForgetMemoryAction);
    registry.register(SendNotificationAction);
}
