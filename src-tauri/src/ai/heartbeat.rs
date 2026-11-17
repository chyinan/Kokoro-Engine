// pattern: Imperative Shell

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
    pub character_id: String,
    pub conversation_id: Option<String>,
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
    let _last_time_period = current_time_period();

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        // Get orchestrator state
        let orchestrator = match app_handle.try_state::<AIOrchestrator>() {
            Some(state) => state,
            None => continue,
        };

        let Ok(scope) = orchestrator.desktop_admission_scope().await else {
            continue;
        };
        let character = scope.0.clone();
        let behavior = orchestrator.owners.character(&character);

        // Gather metrics
        let (idle_secs, conversation_count) = {
            let state = behavior.lock().await;
            (
                state.last_activity.elapsed().as_secs(),
                state.conversation_count,
            )
        };

        // ── Autonomous Systems Updates ──

        // 1. Curiosity Decay
        {
            behavior.lock().await.curiosity.decay();
        }

        // 2. Idle Behaviors (Animations)
        {
            let mut state = behavior.lock().await;
            let idle_sys = &mut state.idle_behaviors;
            if let Some(behavior) = idle_sys.decide(idle_secs) {
                let _ = app_handle.emit(
                    "idle-behavior",
                    IdleBehaviorEvent {
                        behavior,
                        character_id: character.clone(),
                        conversation_id: scope.1.clone(),
                    },
                );
            }
        }

        // 3. Auto Backup Check (interval configured by user)
        crate::commands::auto_backup::check_and_run(&app_handle).await;

        // 4. Memory Decay Pruning (once per hour)
        if orchestrator.is_memory_enabled()
            && behavior.lock().await.last_prune.elapsed().as_secs() >= 3600
        {
            behavior.lock().await.last_prune = std::time::Instant::now();
            let memory_mgr = orchestrator.memory_manager.clone();
            let char_id = character.clone();
            tauri::async_runtime::spawn(async move {
                let _ = memory_mgr.prune_decayed_memories(&char_id, 0.05).await;
            });
        }

        // 5. Dream Memory v2 daily consolidation (once per local day after configured hour)
        if orchestrator.is_memory_enabled() {
            let memory_config = crate::config::load_memory_upgrade_config(
                &crate::ai::memory::memory_upgrade_config_path(),
            );
            let now = chrono::Local::now();
            let today = now.date_naive();
            let char_id = character.clone();
            if memory_config.dreaming_enabled
                && now.hour() >= u32::from(memory_config.dream_daily_hour)
                && behavior.lock().await.last_dream_day != Some(today)
                && idle_secs >= 60
            {
                let memory_mgr = orchestrator.memory_manager.clone();
                let day_start_ts = now.timestamp() - i64::from(now.num_seconds_from_midnight());
                match memory_mgr
                    .has_dream_job_since(&char_id, "daily_idle", day_start_ts)
                    .await
                {
                    Ok(true) => {
                        behavior.lock().await.last_dream_day = Some(today);
                        continue;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(
                            target: "memory",
                            "[Memory] Failed to check daily dream job guard: {}",
                            error
                        );
                    }
                }
                behavior.lock().await.last_dream_day = Some(today);
                let target_language = {
                    let Ok(_activation) = orchestrator.enter_chat_turn() else {
                        continue;
                    };
                    let Ok(prompt) = orchestrator.prompt_snapshot(&char_id).await else {
                        continue;
                    };
                    prompt.response_language
                };
                let memory_scope =
                    orchestrator.role_memory_write_scope(&char_id, &format!("daily-dream:{today}"));
                let provider = app_handle
                    .try_state::<crate::llm::service::LlmService>()
                    .map(|state| state.inner().clone());
                tauri::async_runtime::spawn(async move {
                    let database_guard = crate::ai::context::acquire_database_operation_read_guard().await;
                    if !database_guard.is_current() { return; }
                    let provider = if let Some(llm_state) = provider {
                        Some(llm_state.system_provider().await)
                    } else {
                        None
                    };
                    if let Err(error) = memory_mgr
                        .run_dream_scoped(
                            &char_id,
                            "daily_idle",
                            provider,
                            Some(target_language),
                            Some(&memory_scope),
                        )
                        .await
                    {
                        tracing::warn!(target: "memory", "[Memory] Daily dream job failed: {}", error);
                    }
                });
            }
        }

        // 6. Initiative System
        if idle_secs < config.idle_threshold_secs {
            continue;
        }
        if !orchestrator.is_proactive_enabled() {
            continue;
        }
        if behavior.lock().await.last_proactive.elapsed().as_secs() >= config.cooldown_secs {
            let decision = {
                let mut state = behavior.lock().await;
                let crate::chat::ownership::CharacterBehavior {
                    initiative,
                    curiosity,
                    ..
                } = &mut *state;
                initiative.decide(curiosity, conversation_count, idle_secs)
            };

            match decision {
                InitiativeDecision::StayQuiet => {
                    // Do nothing
                }
                InitiativeDecision::AskQuestion { topic } => {
                    trigger_proactive_message(
                        &app_handle,
                        &orchestrator,
                        &scope,
                        idle_secs,
                        "curiosity",
                        &format!("Ask the user about: {}", topic),
                    )
                    .await;
                    behavior.lock().await.last_proactive = std::time::Instant::now();
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
                        &scope,
                        idle_secs,
                        "initiative",
                        instruction,
                    )
                    .await;
                    behavior.lock().await.last_proactive = std::time::Instant::now();
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
    scope: &(String, Option<String>, u64),
    idle_secs: u64,
    trigger_type: &str,
    instruction: &str,
) {
    let time_ctx = time_of_day_context();

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

    let Some(request_id) = crate::chat::application::submit_proactive_bound(
        app_handle.clone(),
        full_instruction.clone(),
        scope.clone(),
    ) else {
        return;
    };

    let _ = app_handle.emit(
        "proactive-trigger",
        serde_json::json!({
            "trigger": trigger_type,
            "character_id":scope.0,"conversation_id":scope.1,"client_request_id":request_id,
            "idle_seconds": idle_secs,
            "instruction": full_instruction,
        }),
    );

    // Reset idle timer so we don't re-trigger immediately
    orchestrator.touch_character_activity(&scope.0).await;
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
