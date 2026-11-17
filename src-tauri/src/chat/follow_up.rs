use crate::ai::context::AIOrchestrator;
use crate::ai::memory_event_ingress::{
    build_cooldown_key, select_memory_ingress_decision, should_use_structured_extraction,
    MemoryEventIngressOptions,
};
use crate::ai::memory_extractor;
use crate::chat::external_memory::MemoryWork;
use crate::chat::persistence::ExternalTurn;
use crate::llm::provider::LlmProvider;
use std::sync::Arc;
#[allow(clippy::too_many_arguments)]
pub(super) async fn schedule_external_memory_tasks(
    platform: &str,
    user_text: &str,
    char_id: &str,
    turn: &ExternalTurn,
    orchestrator: &AIOrchestrator,
    provider: Arc<dyn LlmProvider>,
    memory_target_language: &str,
    memory_work: Option<MemoryWork>,
    scope: Arc<super::ownership::MemoryWriteScope>,
) {
    let Some(work) = memory_work.filter(|work| orchestrator.external_memory.is_current(*work))
    else {
        return;
    };
    if !scope.owner_is_current() {
        return;
    }
    let msg_count = work.count;
    let memory_msg_count = work.count;
    let upgrade_config =
        crate::config::load_memory_upgrade_config(&crate::ai::memory::memory_upgrade_config_path());
    let ingress_options = MemoryEventIngressOptions {
        enabled: upgrade_config.event_trigger_enabled,
        event_cooldown_secs: upgrade_config.event_cooldown_secs,
        intent_routing_enabled: upgrade_config.intent_routing_enabled,
    };
    let memory_target_language = memory_target_language.to_owned();

    tracing::info!(
        target: "bot::memory",
        "[{}] user message count: {}, memory trigger count: {}, char_id: {}",
        platform,
        msg_count,
        memory_msg_count,
        char_id
    );

    if orchestrator.is_memory_enabled() {
        if let Some(decision) = select_memory_ingress_decision(user_text, &ingress_options) {
            let cooldown_key = build_cooldown_key(
                char_id,
                &turn.target.conversation_id,
                decision.event.event_type,
            );
            if orchestrator
                .should_trigger_memory_event(&cooldown_key, decision.event.cooldown_secs)
                .await
            {
                let history = turn.recent_committed_history(orchestrator, work, 10).await;
                let memory_mgr = orchestrator.memory_manager.clone();
                let provider_for_mem = provider.clone();
                let char_id_for_mem = char_id.to_string();
                let source = platform.to_string();
                let memory_enabled = orchestrator.memory_enabled_flag();
                let external_memory = orchestrator.external_memory.clone();
                let observation_started_at = std::time::Instant::now();
                let trigger = decision.trigger_label.to_string();
                let extraction_options = memory_extractor::MemoryExtractionOptions {
                    scope: Some(scope.clone()),
                    structured_memory_enabled: should_use_structured_extraction(
                        upgrade_config.structured_memory_enabled,
                        &ingress_options,
                    ),
                    target_language: Some(memory_target_language.clone()),
                };
                tauri::async_runtime::spawn(async move {
                    if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst)
                        || !external_memory.is_current(work)
                    {
                        return;
                    }
                    let _ = memory_mgr
                        .record_periodic_write_if_enabled(
                            &char_id_for_mem,
                            &source,
                            &trigger,
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
    }

    if orchestrator.is_memory_enabled() && work.extract() {
        let history = turn.recent_committed_history(orchestrator, work, 10).await;
        let memory_mgr = orchestrator.memory_manager.clone();
        let provider_for_mem = provider.clone();
        let char_id_for_mem = char_id.to_string();
        let source = platform.to_string();
        let memory_enabled = orchestrator.memory_enabled_flag();
        let external_memory = orchestrator.external_memory.clone();
        let observation_started_at = std::time::Instant::now();
        let extraction_options = memory_extractor::MemoryExtractionOptions {
            scope: Some(scope.clone()),
            structured_memory_enabled: false,
            target_language: Some(memory_target_language.clone()),
        };
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst)
                || !external_memory.is_current(work)
            {
                return;
            }
            let _ = memory_mgr
                .record_periodic_write_if_enabled(
                    &char_id_for_mem,
                    &source,
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

    if orchestrator.is_memory_enabled() && work.consolidate() {
        let memory_mgr = orchestrator.memory_manager.clone();
        let char_id_for_consolidation = char_id.to_string();
        let provider_for_consolidation = provider.clone();
        let memory_enabled = orchestrator.memory_enabled_flag();
        let external_memory = orchestrator.external_memory.clone();
        let observation_started_at = std::time::Instant::now();
        let scope = scope.clone();
        let source = platform.to_string();
        let target_language_for_consolidation = memory_target_language.clone();
        tauri::async_runtime::spawn(async move {
            if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst)
                || !external_memory.is_current(work)
            {
                return;
            }
            let _ = memory_mgr
                .periodic_consolidation_observation(
                    &char_id_for_consolidation,
                    &source,
                    observation_started_at,
                )
                .await;
            match memory_mgr
                .consolidate_chat_memories(
                    &char_id_for_consolidation,
                    provider_for_consolidation,
                    Some(target_language_for_consolidation),
                    &scope,
                )
                .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!(
                        target: "bot::memory",
                        "[{}] consolidated {} memory clusters",
                        source,
                        count
                    );
                }
                Err(error) => {
                    tracing::error!(
                        target: "bot::memory",
                        "[{}] memory consolidation failed: {}",
                        source,
                        error
                    );
                }
                _ => {}
            }
        });
    }
}
