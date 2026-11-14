use std::sync::Arc;

use crate::ai::context::{AIOrchestrator, Message};
use crate::ai::memory_event_ingress::{
    build_cooldown_key, select_memory_ingress_decision, should_use_structured_extraction,
    MemoryEventIngressOptions,
};
use crate::ai::memory_extractor;
use crate::llm::provider::LlmProvider;

/// Schedule memory work only after the final chat persistence boundary.
pub(super) async fn schedule_memory_tasks(state: &AIOrchestrator, completed: CompletedTurn<'_>) {
    let CompletedTurn {
        char_id,
        conversation_id,
        source,
        user_text,
        memory_history,
        memory_msg_count,
        memory_target_language,
        system_provider,
    } = completed;
    if !state.is_memory_enabled() {
        return;
    }
    let msg_count = state.get_message_count().await;
    let upgrade_config =
        crate::config::load_memory_upgrade_config(&crate::ai::memory::memory_upgrade_config_path());
    let ingress_options = MemoryEventIngressOptions {
        enabled: upgrade_config.event_trigger_enabled,
        event_cooldown_secs: upgrade_config.event_cooldown_secs,
        intent_routing_enabled: upgrade_config.intent_routing_enabled,
    };
    tracing::info!(
        target: "memory",
        "[Memory] User message count: {}, memory trigger count: {}",
        msg_count, memory_msg_count
    );

    if let Some((cid, decision)) =
        conversation_id.zip(select_memory_ingress_decision(user_text, &ingress_options))
    {
        let cooldown_key = build_cooldown_key(char_id, cid, decision.event.event_type);
        if state
            .should_trigger_memory_event(&cooldown_key, decision.event.cooldown_secs)
            .await
        {
            tracing::info!(
                target: "memory",
                "[Memory] Triggering event-driven extraction (trigger={}, count={})",
                decision.trigger_label,
                msg_count
            );

            let history = memory_history.clone();
            let memory_mgr = state.memory_manager.clone();
            let char_id_for_mem = char_id.to_owned();
            let provider_for_mem = system_provider.clone();
            let memory_enabled = state.memory_enabled_flag();
            let observation_started_at = std::time::Instant::now();
            let trigger_for_observation = decision.trigger_label.to_string();
            let extraction_options = memory_extractor::MemoryExtractionOptions {
                structured_memory_enabled: should_use_structured_extraction(
                    upgrade_config.structured_memory_enabled,
                    &ingress_options,
                ),
                target_language: Some(memory_target_language.to_owned()),
            };
            tauri::async_runtime::spawn(async move {
                if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let _ = memory_mgr
                    .record_periodic_write_if_enabled(
                        &char_id_for_mem,
                        source,
                        &trigger_for_observation,
                        observation_started_at,
                    )
                    .await;
                memory_extractor::extract_and_store_memories_with_options(
                    &history,
                    &memory_mgr,
                    provider_for_mem,
                    char_id_for_mem,
                    extraction_options,
                )
                .await;
            });
        }
    }

    if memory_msg_count > 0 && memory_msg_count.is_multiple_of(5) {
        tracing::info!(
            target: "memory",
            "[Memory] Triggering memory extraction (count={})",
            msg_count
        );
        let history = memory_history.clone();
        let memory_mgr = state.memory_manager.clone();
        let char_id_for_mem = char_id.to_owned();
        let provider_for_mem = system_provider.clone();
        let memory_enabled = state.memory_enabled_flag();
        let observation_started_at = std::time::Instant::now();
        let extraction_options = memory_extractor::MemoryExtractionOptions {
            structured_memory_enabled: false,
            target_language: Some(memory_target_language.to_owned()),
        };
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let _ = memory_mgr
                .record_periodic_write_if_enabled(
                    &char_id_for_mem,
                    source,
                    "periodic_extraction",
                    observation_started_at,
                )
                .await;
            memory_extractor::extract_and_store_memories_with_options(
                &history,
                &memory_mgr,
                provider_for_mem,
                char_id_for_mem,
                extraction_options,
            )
            .await;
        });
    }

    // Periodic memory consolidation (every 20 user messages)
    if memory_msg_count > 0 && memory_msg_count.is_multiple_of(20) {
        let memory_mgr = state.memory_manager.clone();
        let char_id_for_consolidation = char_id.to_owned();
        let provider_for_consolidation = system_provider.clone();
        let memory_enabled = state.memory_enabled_flag();
        let observation_started_at = std::time::Instant::now();
        let target_language_for_consolidation = memory_target_language.to_owned();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let _ = memory_mgr
                .periodic_consolidation_observation(
                    &char_id_for_consolidation,
                    source,
                    observation_started_at,
                )
                .await;
            match memory_mgr
                .consolidate_memories_with_language(
                    &char_id_for_consolidation,
                    provider_for_consolidation,
                    Some(target_language_for_consolidation),
                )
                .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!(target: "memory", "[Memory] Consolidated {} memory clusters", count);
                }
                Err(e) => {
                    tracing::error!(target: "memory", "[Memory] Consolidation failed: {}", e);
                }
                _ => {}
            }
        });
    }
}

pub(super) struct CompletedTurn<'a> {
    pub char_id: &'a str,
    pub conversation_id: Option<&'a str>,
    pub source: &'static str,
    pub user_text: &'a str,
    pub memory_history: Vec<Message>,
    pub memory_msg_count: u64,
    pub memory_target_language: &'a str,
    pub system_provider: Arc<dyn LlmProvider>,
}
