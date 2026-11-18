use crate::{commands, stt};
use specta_typescript::Typescript;
use std::path::Path;
use tauri::ipc::{Invoke, InvokeBody};
use tauri::Wry;
use tauri_specta::{collect_commands, Builder, ErrorHandlingMode};

pub(crate) mod events;
#[cfg(test)]
mod tests;
pub(crate) mod wire;

pub(crate) fn builder() -> Builder<Wry> {
    Builder::new()
        .commands(collect_commands![
            commands::system::get_engine_info,
            commands::system::check_latest_release,
            commands::system::get_system_status,
            commands::system::set_window_size,
            commands::character::get_character_state,
            commands::character::play_cue,
            commands::character::send_message,
            commands::database::init_db,
            commands::database::test_vector_store,
            commands::chat::stream_chat,
            commands::chat::retry_chat_persistence,
            commands::chat::get_chat_runtime_snapshot,
            commands::chat::get_chat_view,
            commands::chat::recover_chat_draft_submission,
            commands::chat::finish_chat_playback,
            commands::chat::get_context_settings,
            commands::chat::set_context_settings,
            commands::chat::approve_tool_approval,
            commands::chat::reject_tool_approval,
            commands::chat::cancel_chat_turn,
            commands::chat::is_chat_busy,
            commands::context::set_persona,
            commands::context::set_character_name,
            commands::context::set_active_character_id,
            commands::context::get_user_profile_settings,
            commands::context::set_user_name,
            commands::context::set_user_persona,
            commands::context::set_response_language,
            commands::context::set_user_language,
            commands::context::set_jailbreak_prompt,
            commands::context::get_jailbreak_prompt,
            commands::context::set_proactive_enabled,
            commands::context::get_proactive_enabled,
            commands::context::set_memory_enabled,
            commands::context::get_memory_enabled,
            commands::context::set_memory_upgrade_config,
            commands::context::get_memory_upgrade_config,
            commands::context::get_memory_observability_summary,
            commands::context::get_latest_memory_write_event,
            commands::context::get_latest_memory_retrieval_log,
            commands::context::get_latest_memory_retrieval_eval_summary,
            commands::context::clear_history,
            commands::context::delete_last_messages,
            commands::context::end_session,
            commands::tts::synthesize,
            commands::tts::cancel_tts,
            commands::tts::list_tts_providers,
            commands::tts::list_tts_voices,
            commands::tts::list_openai_tts_models,
            commands::tts::get_tts_provider_status,
            commands::tts::clear_tts_cache,
            commands::tts::get_tts_config,
            commands::tts::save_tts_config,
            commands::tts::list_gpt_sovits_models,
            commands::mods::list_mods,
            commands::mods::load_mod,
            commands::mods::install_mod,
            commands::mods::update_mod,
            commands::mods::install_mod_from_registry,
            commands::mods::remove_mod,
            commands::mods::install_mod_from_url,
            commands::mods::get_mod_theme,
            commands::mods::get_mod_layout,
            commands::mods::dispatch_mod_event,
            commands::mods::unload_mod,
            commands::live2d::import_live2d_zip,
            commands::live2d::import_live2d_folder,
            commands::live2d::export_live2d_model,
            commands::live2d::list_live2d_models,
            commands::live2d::delete_live2d_model,
            commands::live2d::rename_live2d_model,
            commands::live2d::get_live2d_model_profile,
            commands::live2d::save_live2d_model_profile,
            commands::live2d::set_active_live2d_model,
            commands::imagegen::generate_image,
            commands::imagegen::get_imagegen_config,
            commands::imagegen::save_imagegen_config,
            commands::imagegen::test_sd_connection,
            commands::vision::upload_vision_image,
            commands::vision::get_vision_config,
            commands::vision::list_vision_screens,
            commands::vision::save_vision_config,
            commands::vision::start_vision_watcher,
            commands::vision::stop_vision_watcher,
            commands::vision::set_vision_text_input_focused,
            commands::vision::capture_screen_now,
            commands::memory::list_memories,
            commands::memory::update_memory,
            commands::memory::delete_memory,
            commands::memory::update_memory_tier,
            commands::memory::run_dream_now,
            commands::memory::get_dreaming_summary,
            commands::memory::list_dream_jobs,
            commands::memory::list_dream_proposals,
            commands::memory::approve_dream_proposal,
            commands::memory::reject_dream_proposal,
            commands::memory::get_memory_embedding_model_status,
            commands::memory::download_memory_embedding_model,
            commands::characters::list_characters,
            commands::characters::create_character,
            commands::characters::create_character_with_avatar,
            commands::characters::update_character_with_avatar,
            commands::characters::update_character,
            commands::characters::delete_character,
            commands::characters::duplicate_character,
            commands::characters::restore_character_defaults,
            commands::characters::list_character_templates,
            commands::characters::instantiate_character_template,
            commands::characters::reconcile_character_template,
            commands::characters::apply_character_template_reconciliation,
            commands::characters::prepare_character_activation,
            commands::characters::commit_character_activation,
            commands::characters::get_committed_character_runtime,
            commands::registry::list_registry_entries,
            commands::registry::install_character_from_registry,
            commands::registry::install_character_from_url,
            commands::registry::remove_character_package,
            commands::conversation::list_conversations,
            commands::conversation::load_conversation,
            commands::conversation::delete_conversation,
            commands::conversation::create_conversation,
            commands::conversation::rename_conversation,
            commands::conversation::update_conversation_state,
            commands::conversation::edit_conversation_message,
            commands::conversation::list_character_ids,
            commands::llm::get_llm_config,
            commands::llm::save_llm_config,
            commands::llm::test_llm_connection,
            commands::llm::test_mod_llm_connection,
            commands::llm::list_ollama_models,
            commands::llm::list_anthropic_models,
            commands::llm::get_llama_cpp_status,
            commands::llm::get_codex_runtime_status,
            commands::llm::list_codex_runtime_models,
            commands::stt::transcribe_audio,
            commands::stt::get_stt_config,
            commands::stt::save_stt_config,
            commands::stt::transcribe_wake_word_audio,
            commands::stt::start_native_mic,
            commands::stt::stop_native_mic,
            commands::stt::start_native_wake_word,
            commands::stt::stop_native_wake_word,
            commands::stt::get_sensevoice_local_status,
            commands::stt::download_sensevoice_local_model,
            commands::actions::list_actions,
            commands::actions::list_builtin_tools,
            commands::actions::execute_action,
            commands::tool_settings::get_tool_settings,
            commands::tool_settings::save_tool_settings,
            commands::mcp::list_mcp_servers,
            commands::mcp::add_mcp_server,
            commands::mcp::remove_mcp_server,
            commands::mcp::refresh_mcp_tools,
            commands::mcp::reconnect_mcp_server,
            commands::mcp::toggle_mcp_server,
            commands::bot::get_bot_config,
            commands::bot::save_bot_config,
            commands::bot::respond_qq_authorization,
            commands::bot::start_bot_platform,
            commands::bot::stop_bot_platform,
            commands::bot::get_bot_status,
            commands::telegram::get_telegram_config,
            commands::telegram::save_telegram_config,
            commands::telegram::start_telegram_bot,
            commands::telegram::stop_telegram_bot,
            commands::telegram::get_telegram_status,
            commands::backup::export_data,
            commands::backup::preview_import,
            commands::backup::import_data,
            commands::auto_backup::get_auto_backup_config,
            commands::auto_backup::save_auto_backup_config,
            commands::auto_backup::run_auto_backup_now,
            commands::pet::show_pet_window,
            commands::pet::hide_pet_window,
            commands::pet::toggle_pet_window,
            commands::pet::set_pet_drag_mode,
            commands::pet::get_pet_config,
            commands::pet::save_pet_config,
            commands::pet::move_pet_window,
            commands::pet::resize_pet_window,
            commands::pet::show_bubble_window,
            commands::pet::update_bubble_text,
            commands::pet::hide_bubble_window,
            stt::stream::process_audio_chunk,
            stt::stream::complete_audio_stream,
            stt::stream::discard_audio_stream,
            stt::stream::snapshot_audio_stream,
            stt::stream::prune_audio_buffer,
        ])
        // Every application invoke and checked response rejects integer tokens
        // outside the JavaScript safe range before crossing into its consumer.
        .dangerously_cast_bigints_to_number()
        .error_handling(ErrorHandlingMode::Throw)
        .events(registered_events())
        .typ::<crate::error::KokoroError>()
}

fn registered_events() -> tauri_specta::Events {
    use self::events::*;
    use crate::ai::memory_embedding_model::MemoryEmbeddingModelDownloadProgress;
    use crate::chat::event_payloads::*;
    use crate::chat::runtime::ChatTurnSnapshot;
    use crate::commands::bot::QqAuthorizationApproved;
    use crate::commands::pet::PetConfigUpdated;
    use crate::desktop_heartbeat::IdleBehaviorEvent;
    use crate::mods::manager::{ScriptEventPayload, UiMessagePayload};
    use crate::stt::{mic::MicVolumeEvent, sensevoice_local::SenseVoiceLocalDownloadProgress};
    use crate::tts::manager::{
        TtsAudioEvent, TtsBrowserDelegateEvent, TtsEndEvent, TtsErrorEvent, TtsStartEvent,
    };

    tauri_specta::collect_events![
        BubbleTextUpdate,
        CameraObservation,
        CharacterRuntimeCommitted,
        ChatError,
        ChatFailure,
        ChatImageGenError,
        ChatImageGenEvent,
        ChatTurnAcknowledgedEvent,
        ChatTurnBoundEvent,
        ChatTurnDeltaEvent,
        ChatTurnFinishEvent,
        ChatTurnSnapshot,
        ChatTurnStartEvent,
        ChatTurnTextCompleteEvent,
        ChatTurnToolEvent,
        ChatTurnTranslationEvent,
        ChatTypingEvent,
        ChatWarningEvent,
        CueEvent,
        IdleBehaviorEvent,
        ImagegenError,
        ImageGenDone,
        Live2dModelSelectionUpdated,
        Live2dProfileUpdated,
        MemoryEmbeddingModelDownloadProgress,
        MemoryUpdated,
        MicVolumeEvent,
        ModComponentsRegister,
        ModLayoutOverride,
        ModThemeOverride,
        ModUnload,
        PetConfigUpdated,
        PetWindowClosed,
        ProactiveTrigger,
        QqAuthorizationApproved,
        QqAuthorizationExpired,
        QqAuthorizationRequest,
        ScriptEventPayload,
        ShowMainWindow,
        SenseVoiceLocalDownloadProgress,
        SttMicAutoStop,
        SttWakeWordDetected,
        SttWakeWordError,
        TelegramChatSync,
        ToggleChatInput,
        TtsAudioEvent,
        TtsBrowserDelegateEvent,
        TtsEndEvent,
        TtsErrorEvent,
        TtsPlaybackErrorEvent,
        TtsStartEvent,
        UiMessagePayload,
        VisionObservationEvent,
        VisionStatusChanged,
        VoiceInterruptStt,
    ]
}

pub(crate) fn invoke_handler(
    builder: &Builder<Wry>,
) -> impl Fn(Invoke<Wry>) -> bool + Send + Sync + 'static + use<'_> {
    let dispatch = builder.invoke_handler();
    move |invoke| {
        if let InvokeBody::Json(value) = invoke.message.payload() {
            if let Err(error) = wire::validate_json(value) {
                invoke
                    .resolver
                    .reject(crate::error::KokoroError::Validation(error.into()));
                return true;
            }
        }
        dispatch(invoke)
    }
}

/// Export the same application-owned IPC interface installed at startup.
pub fn export(path: &Path) -> Result<(), impl std::error::Error> {
    builder().export(Typescript::default(), path)
}
