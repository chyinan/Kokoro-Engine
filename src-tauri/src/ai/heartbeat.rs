use crate::ai::context::AIOrchestrator;
use crate::ai::initiative::InitiativeDecision;
use chrono::Timelike;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Configuration for the heartbeat system.
pub struct HeartbeatConfig {
    /// Seconds of idle before triggering a proactive message.
    pub idle_threshold_secs: u64,
    /// Minimum seconds between proactive messages (cooldown).
    pub cooldown_secs: u64,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            idle_threshold_secs: 300, // 5 minutes
            cooldown_secs: 600,       // 10 minutes between proactive messages
        }
    }
}

/// Event emitted when the character performs an idle animation.
#[derive(Debug, Clone, Serialize)]
struct IdleBehaviorEvent {
    pub behavior: crate::ai::idle_behaviors::IdleBehavior,
}

/// Get a time-of-day greeting context string.
fn time_of_day_context() -> &'static str {
    let hour = chrono::Local::now().hour();
    match hour {
        5..=8 => "It is early morning. The user may have just woken up.",
        9..=11 => "It is mid-morning.",
        12..=13 => "It is noon / lunchtime.",
        14..=17 => "It is afternoon.",
        18..=20 => "It is evening.",
        21..=23 => "It is night.",
        _ => "It is late night / early hours. The user should probably rest.",
    }
}

/// Main heartbeat loop. Spawned once at app startup.
pub async fn heartbeat_loop(app_handle: AppHandle) {
    let config = HeartbeatConfig::default();
    let mut last_proactive_ts = std::time::Instant::now();
    let _last_time_period = current_time_period();
    let mut last_prune_ts = std::time::Instant::now();

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        // Get orchestrator state
        let orchestrator = match app_handle.try_state::<AIOrchestrator>() {
            Some(state) => state,
            None => continue,
        };

        // Gather metrics
        let idle_secs = orchestrator.idle_seconds().await;
        let conversation_count = orchestrator.get_conversation_count().await;

        // ── Autonomous Systems Updates ──

        // 1. Curiosity Decay
        {
            let mut curiosity = orchestrator.curiosity.lock().await;
            curiosity.decay();
        }

        // 2. Idle Behaviors (Animations)
        {
            let mut idle_sys = orchestrator.idle_behaviors.lock().await;
            if let Some(behavior) = idle_sys.decide(idle_secs) {
                let _ = app_handle.emit("idle-behavior", IdleBehaviorEvent { behavior });
            }
        }

        // 3. Auto Backup Check (interval configured by user)
        crate::commands::auto_backup::check_and_run(&app_handle).await;

        // 4. Memory Decay Pruning (once per hour)
        if orchestrator.is_memory_enabled() && last_prune_ts.elapsed().as_secs() >= 3600 {
            last_prune_ts = std::time::Instant::now();
            let memory_mgr = orchestrator.memory_manager.clone();
            let char_id = orchestrator.get_character_id().await;
            tauri::async_runtime::spawn(async move {
                let _ = memory_mgr.prune_decayed_memories(&char_id, 0.05).await;
            });
        }

        // 5. Initiative System
        if idle_secs < config.idle_threshold_secs {
            continue;
        }
        if !orchestrator.is_proactive_enabled() {
            continue;
        }
        if last_proactive_ts.elapsed().as_secs() >= config.cooldown_secs {
            let decision = {
                let mut initiative = orchestrator.initiative.lock().await;
                let mut curiosity = orchestrator.curiosity.lock().await;
                initiative.decide(&mut curiosity, conversation_count, idle_secs)
            };

            match decision {
                InitiativeDecision::StayQuiet => {
                    // Do nothing
                }
                InitiativeDecision::AskQuestion { topic } => {
                    trigger_proactive_message(
                        &app_handle,
                        &orchestrator,
                        "curiosity",
                        &format!("Ask the user about: {}", topic),
                    )
                    .await;
                    last_proactive_ts = std::time::Instant::now();
                }
                InitiativeDecision::ShareThought { topic } => {
                    let instruction = if topic == "random" {
                        "Share a random thought or observation relevant to the current context/time."
                    } else {
                        &format!("Share a thought about: {}", topic)
                    };
                    trigger_proactive_message(
                        &app_handle,
                        &orchestrator,
                        "initiative",
                        instruction,
                    )
                    .await;
                    last_proactive_ts = std::time::Instant::now();
                }
                InitiativeDecision::VideoShare { .. } => {
                    // Not implemented
                }
            }
        }
    }
}

async fn trigger_proactive_message(
    app_handle: &AppHandle,
    orchestrator: &AIOrchestrator,
    trigger_type: &str,
    instruction: &str,
) {
    let time_ctx = time_of_day_context();
    let idle_secs = orchestrator.idle_seconds().await;

    let full_instruction = format!(
        "User has been idle for {:.0} minutes. {} {}",
        idle_secs as f64 / 60.0,
        time_ctx,
        instruction
    );

    tracing::info!(
        target: "chat",
        "[Heartbeat] Trigger '{}' fired: {}",
        trigger_type, instruction
    );

    let _ = app_handle.emit(
        "proactive-trigger",
        serde_json::json!({
            "trigger": trigger_type,
            "idle_seconds": idle_secs,
            "instruction": full_instruction,
        }),
    );

    // Reset idle timer so we don't re-trigger immediately
    orchestrator.touch_activity().await;
}

/// Time period enum for detecting transitions.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TimePeriod {
    EarlyMorning,
    Morning,
    Noon,
    Afternoon,
    Evening,
    Night,
    LateNight,
}

fn current_time_period() -> TimePeriod {
    let hour = chrono::Local::now().hour();
    match hour {
        5..=8 => TimePeriod::EarlyMorning,
        9..=11 => TimePeriod::Morning,
        12..=13 => TimePeriod::Noon,
        14..=17 => TimePeriod::Afternoon,
        18..=20 => TimePeriod::Evening,
        21..=23 => TimePeriod::Night,
        _ => TimePeriod::LateNight,
    }
}
