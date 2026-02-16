//! Built-in action handlers for the Action Registry.

use super::registry::{ActionContext, ActionError, ActionHandler, ActionParam, ActionResult};
use async_trait::async_trait;
use std::collections::HashMap;
use tauri::Emitter;
use tauri::Manager;

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

// ── change_expression ──────────────────────────────────

pub struct ChangeExpressionAction;

#[async_trait]
impl ActionHandler for ChangeExpressionAction {
    fn name(&self) -> &str {
        "change_expression"
    }

    fn description(&self) -> &str {
        "Change the character's facial expression"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![ActionParam {
            name: "expression".to_string(),
            description: "One of: neutral, happy, sad, angry, surprised, thinking, shy, smug, worried, excited".to_string(),
            required: true,
        }]
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let expression = args
            .get("expression")
            .ok_or_else(|| ActionError("Missing 'expression' parameter".into()))?;

        let valid = [
            "neutral",
            "happy",
            "sad",
            "angry",
            "surprised",
            "thinking",
            "shy",
            "smug",
            "worried",
            "excited",
        ];
        let expr = expression.to_lowercase();
        if !valid.contains(&expr.as_str()) {
            return Ok(ActionResult::err(format!(
                "Invalid expression: {}",
                expression
            )));
        }

        // Emit expression change event to frontend
        let _ = ctx.app.emit(
            "chat-expression",
            serde_json::json!({ "expression": expr, "mood": 0.5 }),
        );

        Ok(ActionResult::ok(format!("Expression changed to: {}", expr)))
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

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
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

        println!(
            "[Memory] Tool stored: '{}' (importance={:.1}) for '{}'",
            &fact[..fact.len().min(60)],
            importance,
            char_id
        );

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

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
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

            println!(
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

        let _ = ctx.app.emit(
            "notification",
            serde_json::json!({ "title": title, "message": message }),
        );

        Ok(ActionResult::ok(format!("Notification sent: {}", title)))
    }
}

// ── play_sound ─────────────────────────────────────────

pub struct PlaySoundAction;

#[async_trait]
impl ActionHandler for PlaySoundAction {
    fn name(&self) -> &str {
        "play_sound"
    }

    fn description(&self) -> &str {
        "Play a sound effect"
    }

    fn parameters(&self) -> Vec<ActionParam> {
        vec![ActionParam {
            name: "sound".to_string(),
            description: "One of: alert, chime, laugh, applause, ding".to_string(),
            required: true,
        }]
    }

    async fn execute(
        &self,
        args: HashMap<String, String>,
        ctx: ActionContext,
    ) -> Result<ActionResult, ActionError> {
        let sound = args
            .get("sound")
            .ok_or_else(|| ActionError("Missing 'sound' parameter".into()))?;

        let valid = ["alert", "chime", "laugh", "applause", "ding"];
        let snd = sound.to_lowercase();
        if !valid.contains(&snd.as_str()) {
            return Ok(ActionResult::err(format!("Unknown sound: {}", sound)));
        }

        let _ = ctx
            .app
            .emit("play-sound", serde_json::json!({ "sound": snd }));

        Ok(ActionResult::ok(format!("Playing sound: {}", snd)))
    }
}

// ── Factory ────────────────────────────────────────────

/// Register all built-in action handlers into the given registry.
pub fn register_builtins(registry: &mut super::registry::ActionRegistry) {
    registry.register(GetTimeAction);
    registry.register(ChangeExpressionAction);
    registry.register(SetBackgroundAction);
    registry.register(SearchMemoryAction);
    registry.register(StoreMemoryAction);
    registry.register(ForgetMemoryAction);
    registry.register(SendNotificationAction);
    registry.register(PlaySoundAction);
}
