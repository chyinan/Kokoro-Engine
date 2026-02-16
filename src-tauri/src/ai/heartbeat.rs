//! Heartbeat System — Background timer for proactive character behavior.
//!
//! Runs a loop every 30 seconds, checks autonomous systems (Curiosity, Initiative, Idle),
//! and triggers proactive messages or idle animations.

use crate::ai::context::AIOrchestrator;
use crate::ai::initiative::InitiativeDecision;
use crate::llm::openai::{Message as LLMMessage, MessageContent};
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

/// Get relationship depth description based on conversation count.
fn relationship_context(count: u64) -> &'static str {
    match count {
        0..=10 => "You have just met the user. Stay polite and keep an appropriate distance.",
        11..=50 => "You are somewhat familiar with the user. You can be a bit more casual.",
        51..=200 => "You are good friends with the user. Speak in a close, natural way.",
        _ => "You are very close with the user. Speak in an intimate, natural manner.",
    }
}

/// Main heartbeat loop. Spawned once at app startup.
pub async fn heartbeat_loop(app_handle: AppHandle) {
    let config = HeartbeatConfig::default();
    let mut last_proactive_ts = std::time::Instant::now();
    let _last_time_period = current_time_period(); // Tracked for time-change triggers (future)

    loop {
        // Heartbeat tick rate: 5s when active, 30s when idle?
        // For now, stick to 10s to make idle animations feel responsive enough
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        // Get orchestrator state
        let orchestrator = match app_handle.try_state::<AIOrchestrator>() {
            Some(state) => state,
            None => continue,
        };

        // Gather metrics
        let idle_secs = orchestrator.idle_seconds().await;
        let conversation_count = orchestrator.get_conversation_count().await;
        // let now_period = current_time_period();

        // ── Autonomous Systems Updates ──

        // 1. Curiosity Decay
        {
            let mut curiosity = orchestrator.curiosity.lock().await;
            curiosity.decay();
        }

        // 2. Idle Behaviors (Animations)
        {
            let emotion = orchestrator.emotion_state.lock().await;
            let mut idle_sys = orchestrator.idle_behaviors.lock().await;
            if let Some(behavior) = idle_sys.decide(&emotion, idle_secs) {
                let _ = app_handle.emit("idle-behavior", IdleBehaviorEvent { behavior });
            }
        }

        // 3. Emotion System (Decay, Snapshot, Expression Frame)
        {
            let mut emotion = orchestrator.emotion_state.lock().await;

            // Decay
            emotion.decay_toward_default();

            // Snapshot
            if idle_secs % 60 < 10 {
                // Save roughly every minute
                let snap = emotion.snapshot();
                let char_id = orchestrator.get_character_id().await;
                let _ = orchestrator
                    .memory_manager
                    .save_emotion_snapshot(&char_id, &snap)
                    .await;
            }

            // Expression Frame
            let trend = emotion.detect_trend();
            let trend_str = match trend {
                crate::ai::emotion::EmotionTrend::Rising => "rising",
                crate::ai::emotion::EmotionTrend::Falling => "falling",
                crate::ai::emotion::EmotionTrend::Stable => "stable",
            };
            let frame = crate::ai::expression_driver::compute_expression_frame(
                emotion.current_emotion(),
                emotion.mood(),
                trend_str,
                emotion.personality().expressiveness,
            );
            let _ = app_handle.emit("expression-frame", &frame);

            // Emotion Events
            let mood_hist = emotion.mood_history();
            let events =
                crate::ai::emotion_events::check_emotion_triggers(emotion.mood(), &mood_hist);
            for event in &events {
                let _ = app_handle.emit("emotion-event", event);
            }
        }

        // 4. Initiative System (Proactive Messaging)
        // Only run initiative check if cooldown has passed
        if last_proactive_ts.elapsed().as_secs() >= config.cooldown_secs {
            let decision = {
                let mut initiative = orchestrator.initiative.lock().await;
                let mut curiosity = orchestrator.curiosity.lock().await;
                let emotion = orchestrator.emotion_state.lock().await;

                initiative.decide(&mut curiosity, &emotion, conversation_count, idle_secs)
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
    let conversation_count = orchestrator.get_conversation_count().await;
    let rel_ctx = relationship_context(conversation_count);
    let emotion_desc = orchestrator.get_emotion_description().await;
    let system_prompt = orchestrator.system_prompt.lock().await.clone();
    let idle_secs = orchestrator.idle_seconds().await;

    let full_instruction = format!(
        "User has been idle for {:.0} minutes.\n{}\nInstruction: {}\n",
        idle_secs as f64 / 60.0,
        time_ctx,
        instruction
    );

    // Read language settings so proactive messages respect them
    let resp_lang = {
        let lang = orchestrator.response_language.lock().await;
        lang.clone()
    };
    let user_lang = {
        let lang = orchestrator.user_language.lock().await;
        lang.clone()
    };

    let mut lang_instruction = String::new();
    if !resp_lang.is_empty() {
        lang_instruction.push_str(&format!(
            "\n\nCRITICAL INSTRUCTION — LANGUAGE REQUIREMENT:\n\
             You MUST respond ENTIRELY in {}. \
             Regardless of what language the user writes in, \
             your reply MUST be written in {} only. \
             Do NOT switch to any other language. This is non-negotiable.",
            resp_lang, resp_lang
        ));
    }
    if !user_lang.is_empty() && !resp_lang.is_empty() && user_lang != resp_lang {
        lang_instruction.push_str(&format!(
            "\n\nIMPORTANT: After your dialogue response (but BEFORE the [EMOTION:...] tag), \
             append a translation of your ENTIRE dialogue response into {} using this EXACT format:\n\
             [TRANSLATE: <your entire response translated into {}>]\n\
             Only translate the dialogue text. Do NOT include any control tags inside the translation.\n\
             This translation tag is mandatory for every response.",
            user_lang, user_lang
        ));
    }

    // Build recent conversation history so the proactive message is contextually relevant
    let recent_history = orchestrator.get_recent_history(6).await;

    let mut messages = vec![
        LLMMessage {
            role: "system".to_string(),
            content: MessageContent::Text(format!(
                "{}\n\n{}\n{}\n{}\n\n{}\n\n{}{}",
                system_prompt,
                emotion_desc,
                rel_ctx,
                time_ctx,
                full_instruction,
                concat!(
                    "IMPORTANT: Keep your message short (1-2 sentences). ",
                    "Be natural and in character. ",
                    "Your message should feel like a natural continuation of the conversation, not a random topic change. ",
                    "At the end, append an emotion tag: [EMOTION:<emotion>|MOOD:<value>]\n",
                    "Do NOT use any other tags."
                ),
                lang_instruction
            )),
        },
    ];

    // Inject recent conversation history so the AI knows what was discussed
    for msg in &recent_history {
        messages.push(LLMMessage {
            role: msg.role.clone(),
            content: MessageContent::Text(msg.content.clone()),
        });
    }

    // Final user instruction for generation
    messages.push(LLMMessage {
        role: "user".to_string(),
        content: MessageContent::Text("(System: generate a proactive message)".to_string()),
    });

    println!(
        "[Heartbeat] Trigger '{}' fired: {}",
        trigger_type, instruction
    );

    let _ = app_handle.emit(
        "proactive-trigger",
        serde_json::json!({
            "trigger": trigger_type,
            "idle_seconds": idle_secs,
            "instruction": instruction,
            "prompt_messages": messages,
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
