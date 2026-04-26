// pattern: Mixed (unavoidable)
// Reason: 应用入口文件需要同时声明模块、注册 Tauri 命令、初始化服务与恢复磁盘状态，天然属于编排层。
pub mod actions;
pub mod ai;
pub mod commands;
pub mod config;
pub mod error;
pub mod hooks;
pub mod imagegen;
pub mod llm;
pub mod mcp;
pub mod mods;
pub mod stt;
pub mod telegram;
pub mod tts;
pub mod utils;
pub mod vision;
use crate::hooks::{AuditLogHookHandler, HookRuntime};
use crate::mods::ModManager;
use crate::utils::logging::init_logging;
use std::sync::Arc;
use tauri::Manager;

async fn auto_start_pet_on_launch<F, Fut>(
    pet_enabled: bool,
    delay: std::time::Duration,
    show_pet: F,
) where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), crate::error::KokoroError>>,
{
    if !pet_enabled {
        return;
    }

    tokio::time::sleep(delay).await;

    if let Err(e) = show_pet().await {
        tracing::error!(target: "pet", "auto-start failed: {}", e);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();

    // Pin the ONNX Runtime dylib to the copy we ship, so the ort crate
    // never accidentally loads an incompatible system-wide library
    // (e.g. an older onnxruntime.dll in C:\Windows\System32 on Windows).
    if std::env::var("ORT_DYLIB_PATH").is_err() {
        #[cfg(target_os = "windows")]
        const ORT_LIB_NAME: &str = "onnxruntime.dll";
        #[cfg(target_os = "macos")]
        const ORT_LIB_NAME: &str = "libonnxruntime.dylib";
        #[cfg(target_os = "linux")]
        const ORT_LIB_NAME: &str = "libonnxruntime.so";

        let search_roots = [
            std::env::current_dir().ok(),
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf())),
        ];
        for root in search_roots.into_iter().flatten() {
            let candidate = root.join(ORT_LIB_NAME);
            if candidate.exists() {
                tracing::info!(target: "tools", "Using bundled ONNX Runtime: {}", candidate.display());
                std::env::set_var("ORT_DYLIB_PATH", &candidate);
                break;
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .register_uri_scheme_protocol("mod", crate::mods::protocol::handle_mod_request)
        .register_uri_scheme_protocol("live2d", commands::live2d_protocol::handle_live2d_request())
        .invoke_handler(tauri::generate_handler![
            commands::system::get_engine_info,
            commands::system::get_system_status,
            commands::system::set_window_size,
            commands::character::get_character_state,
            commands::character::play_cue,
            commands::character::send_message,
            commands::database::init_db,
            commands::database::test_vector_store,
            commands::chat::stream_chat,
            commands::chat::get_context_settings,
            commands::chat::set_context_settings,
            commands::chat::approve_tool_approval,
            commands::chat::reject_tool_approval,
            commands::chat::cancel_chat_turn,
            commands::context::set_persona,
            commands::context::set_character_name,
            commands::context::set_active_character_id,
            commands::context::set_user_name,
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
            commands::tts::list_tts_providers,
            commands::tts::list_tts_voices,
            commands::tts::get_tts_provider_status,
            commands::tts::clear_tts_cache,
            commands::tts::get_tts_config,
            commands::tts::save_tts_config,
            commands::tts::list_gpt_sovits_models,
            commands::mods::list_mods,
            commands::mods::load_mod,
            commands::mods::install_mod,
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
            commands::vision::save_vision_config,
            commands::vision::start_vision_watcher,
            commands::vision::stop_vision_watcher,
            commands::vision::capture_screen_now,
            commands::memory::list_memories,
            commands::memory::update_memory,
            commands::memory::delete_memory,
            commands::memory::update_memory_tier,
            commands::characters::list_characters,
            commands::characters::create_character,
            commands::characters::update_character,
            commands::characters::delete_character,
            commands::conversation::list_conversations,
            commands::conversation::load_conversation,
            commands::conversation::delete_conversation,
            commands::conversation::create_conversation,
            commands::conversation::rename_conversation,
            commands::conversation::update_conversation_state,
            commands::conversation::list_character_ids,
            commands::llm::get_llm_config,
            commands::llm::save_llm_config,
            commands::llm::list_ollama_models,
            commands::llm::list_anthropic_models,
            commands::llm::get_llama_cpp_status,
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
            commands::singing::check_rvc_status,
            commands::singing::list_rvc_models,
            commands::singing::convert_singing,
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
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if window.label() == "main" {
                    window.app_handle().exit(0);
                    return;
                }
            }

            if window.label() != "pet" {
                return;
            }

            let app = window.app_handle();
            match event {
                tauri::WindowEvent::Moved(_)
                | tauri::WindowEvent::Resized(_)
                | tauri::WindowEvent::ScaleFactorChanged { .. } => {
                    if let Err(error) = crate::commands::pet::sync_bubble_window_to_pet(app) {
                        tracing::warn!(target: "pet", "failed to sync bubble after pet window event: {}", error);
                    }
                }
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
                    if let Err(error) = crate::commands::pet::hide_bubble_window_if_open(app) {
                        tracing::warn!(target: "pet", "failed to hide bubble after pet window closed: {}", error);
                    }
                }
                _ => {}
            }
        })
        .setup(|app| {
            let startup_begin = std::time::Instant::now();
            tracing::info!(target: "startup", "setup begin");

            let app_handle = app.handle();
            tauri::async_runtime::block_on(async move {
                tracing::info!(
                    target: "startup",
                    "stage=ai.init.start elapsed_ms={}",
                    startup_begin.elapsed().as_millis()
                );
                let app_data_dir = dirs_next::data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("com.chyin.kokoro");
                let _ = std::fs::create_dir_all(&app_data_dir);
                let db_path = app_data_dir.join("kokoro.db");
                let db_url = format!("sqlite:///{}", db_path.to_string_lossy().replace('\\', "/"));
                match crate::ai::context::AIOrchestrator::new(&db_url).await {
                    Ok(orchestrator) => {
                        tracing::info!(
                            target: "startup",
                            "stage=ai.init.ok elapsed_ms={}",
                            startup_begin.elapsed().as_millis()
                        );

                        // Restore proactive_enabled from disk
                        let proactive_path = app_data_dir.join("proactive_enabled.json");
                        if let Ok(content) = std::fs::read_to_string(&proactive_path) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(enabled) = val.get("enabled").and_then(|v| v.as_bool()) {
                                    orchestrator.set_proactive_enabled(enabled);
                                    tracing::info!(target: "ai", "Restored proactive_enabled={}", enabled);
                                }
                            }
                        }

                        // Restore memory_enabled from disk
                        let memory_cfg_path = app_data_dir.join("memory_system_config.json");
                        let memory_cfg: serde_json::Value = crate::config::load_json_config(
                            &memory_cfg_path,
                            "MEMORY",
                        );
                        let memory_enabled = memory_cfg
                            .get("enabled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        orchestrator.set_memory_enabled(memory_enabled).await;
                        tracing::info!(target: "ai", "Restored memory_enabled={}", memory_enabled);

                        // Restore jailbreak_prompt from disk
                        let jailbreak_path = app_data_dir.join("jailbreak_prompt.json");
                        if let Ok(content) = std::fs::read_to_string(&jailbreak_path) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(prompt) = val.get("prompt").and_then(|v| v.as_str()) {
                                    orchestrator.set_jailbreak_prompt(prompt.to_string()).await;
                                    tracing::info!(target: "ai", "Restored jailbreak_prompt ({} chars)", prompt.len());
                                }
                            }
                        }

                        // Restore context_settings from disk
                        let ctx_settings_path = app_data_dir.join("context_settings.json");
                        if let Ok(content) = std::fs::read_to_string(&ctx_settings_path) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                                let strategy = val.get("strategy")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("window")
                                    .to_string();
                                let max_chars = val.get("max_message_chars")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(2000) as usize;
                                orchestrator.set_context_settings(strategy.clone(), max_chars).await;
                                tracing::info!(target: "ai", "Restored context_settings: strategy={}, max_chars={}", strategy, max_chars);
                            }
                        }

                        // Restore current_conversation_id from disk and reload history
                        let conv_id_path = app_data_dir.join("current_conversation_id.json");
                        if let Ok(content) = std::fs::read_to_string(&conv_id_path) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(id) = val.get("conversation_id").and_then(|v| v.as_str()) {
                                    {
                                        let mut conv_id = orchestrator.current_conversation_id.lock().await;
                                        *conv_id = Some(id.to_string());
                                    }
                                    // Reload messages into in-memory history so LLM has conversation context
                                    if let Ok(rows) = sqlx::query_as::<_, (String, String, Option<String>)>(
                                        "SELECT role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC"
                                    )
                                    .bind(id)
                                    .fetch_all(&orchestrator.db)
                                    .await {
                                        let mut history = orchestrator.history.lock().await;
                                        history.clear();
                                        for (role, content, metadata) in &rows {
                                            history.push_back(crate::ai::context::Message {
                                                role: role.clone(),
                                                content: content.clone(),
                                                metadata: metadata
                                                    .as_deref()
                                                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
                                            });
                                        }
                                        tracing::info!(target: "ai", "Restored current_conversation_id: {} ({} messages)", id, rows.len());
                                    }
                                }
                            }
                        }

                        app_handle.manage(orchestrator);

                        // Restore active_character_id from disk
                        if let Some(char_id) = crate::ai::context::AIOrchestrator::load_active_character_id() {
                            let orch = app_handle.state::<crate::ai::context::AIOrchestrator>();
                            orch.set_character_id(char_id.clone()).await;
                            tracing::info!(target: "ai", "Restored active_character_id: {}", char_id);
                        }

                    }
                    Err(e) => {
                        tracing::error!(target: "ai", "AI Orchestrator init failed (will run without AI): {}", e);
                        tracing::error!(
                            target: "startup",
                            "stage=ai.init.err elapsed_ms={} error={}",
                            startup_begin.elapsed().as_millis(),
                            e
                        );
                        // Do NOT panic — allow app to continue running
                    }
                }
            });

            tracing::info!(
                target: "startup",
                "stage=ai.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // Initialize TTS Service from config
            let app_data = dirs_next::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("com.chyin.kokoro");

            // TTS
            let tts_config_path = app_data.join("tts_config.json");
            let tts_config = crate::tts::load_config(&tts_config_path);

            tracing::info!(
                target: "startup",
                "stage=tts.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let tts_service = tauri::async_runtime::block_on(async {
                crate::tts::TtsService::init_from_config(&tts_config).await
            });
            app.manage(tts_service);
            tracing::info!(
                target: "startup",
                "stage=tts.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // ImageGen
            let imagegen_config_path = app_data.join("imagegen_config.json");
            let imagegen_config = crate::imagegen::config::load_config(&imagegen_config_path);

            tracing::info!(
                target: "startup",
                "stage=imagegen.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let imagegen_service = tauri::async_runtime::block_on(async {
                crate::imagegen::ImageGenService::init_from_config(&imagegen_config).await
            });
            app.manage(imagegen_service);
            tracing::info!(
                target: "startup",
                "stage=imagegen.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // WindowSizeState
            app.manage(crate::commands::system::WindowSizeState::new());

            // LLM
            let llm_config_path = app_data.join("llm_config.json");
            let llm_config = crate::llm::llm_config::load_config(&llm_config_path);
            tracing::info!(
                target: "startup",
                "stage=llm.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let llm_service =
                crate::llm::service::LlmService::from_config(llm_config, llm_config_path);
            app.manage(llm_service);
            tracing::info!(
                target: "startup",
                "stage=llm.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // STT
            let stt_config_path = app_data.join("stt_config.json");
            let stt_config = crate::stt::load_config(&stt_config_path);
            tracing::info!(
                target: "startup",
                "stage=stt.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let stt_service = tauri::async_runtime::block_on(async {
                crate::stt::SttService::init_from_config(&stt_config).await
            });
            app.manage(stt_service);
            tracing::info!(
                target: "startup",
                "stage=stt.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            let hook_runtime = HookRuntime::new();
            hook_runtime.register(Arc::new(AuditLogHookHandler));
            app.manage(hook_runtime);
            app.manage(Arc::new(crate::commands::chat::PendingToolApprovalState::new()));
            app.manage(Arc::new(crate::commands::chat::TurnCancellationState::new()));

            // Action Registry
            let mut action_registry = crate::actions::ActionRegistry::new();
            crate::actions::builtin::register_builtins(&mut action_registry);

            let tool_settings_path = app_data.join("tool_settings.json");
            let mut tool_settings = crate::actions::tool_settings::load_config(&tool_settings_path);
            if action_registry.migrate_tool_settings(&mut tool_settings) {
                if let Err(e) = crate::actions::tool_settings::save_config(&tool_settings_path, &tool_settings) {
                    tracing::error!(target: "tools", "Failed to persist migrated tool settings: {}", e);
                }
            }

            app.manage(std::sync::Arc::new(tokio::sync::RwLock::new(
                action_registry,
            )));
            app.manage(Arc::new(tokio::sync::RwLock::new(tool_settings)));

            // MCP Manager
            let mcp_config_path = app_data.join("mcp_servers.json");
            tracing::info!(
                target: "startup",
                "stage=mcp.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let mut mcp_manager =
                crate::mcp::McpManager::new(mcp_config_path.to_str().unwrap_or("mcp_servers.json"));
            mcp_manager.load_configs();
            let mcp_manager = Arc::new(tokio::sync::Mutex::new(mcp_manager));
            app.manage(mcp_manager.clone());
            tracing::info!(
                target: "startup",
                "stage=mcp.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // Connect MCP servers in background — per-server tasks so the
            // manager lock is only held briefly and list_mcp_servers stays responsive.
            let mcp_mgr_clone = mcp_manager.clone();
            let mcp_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Delay to let app fully init
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

                // Grab configs & mark "connecting", then release lock immediately
                let configs = {
                    let mut mgr = mcp_mgr_clone.lock().await;
                    mgr.prepare_connect_all()
                };

                // Spawn a task per server so they connect in parallel
                let mut handles = Vec::new();
                for cfg in configs {
                    let mgr_arc = mcp_mgr_clone.clone();
                    let app_handle = mcp_app.clone();
                    handles.push(tauri::async_runtime::spawn(async move {
                        // Slow I/O (process spawn / TCP / MCP handshake) happens
                        // outside the lock so list_mcp_servers stays responsive.
                        let build_result =
                            crate::mcp::manager::build_connected_client(&cfg).await;

                        // Brief lock only to insert the result.
                        let connect_result = {
                            let mut mgr = mgr_arc.lock().await;
                            mgr.clear_connecting(&cfg.name);
                            match build_result {
                                Ok(client) => {
                                    mgr.insert_client(cfg.name.clone(), client);
                                    Ok(())
                                }
                                Err(e) => {
                                    let display_error = match &e {
                                        crate::error::KokoroError::Config(message)
                                        | crate::error::KokoroError::Database(message)
                                        | crate::error::KokoroError::Llm(message)
                                        | crate::error::KokoroError::Tts(message)
                                        | crate::error::KokoroError::Stt(message)
                                        | crate::error::KokoroError::Io(message)
                                        | crate::error::KokoroError::ExternalService(message)
                                        | crate::error::KokoroError::Mod(message)
                                        | crate::error::KokoroError::NotFound(message)
                                        | crate::error::KokoroError::Unauthorized(message)
                                        | crate::error::KokoroError::Internal(message)
                                        | crate::error::KokoroError::Chat(message)
                                        | crate::error::KokoroError::Validation(message) => message.clone(),
                                    };
                                    mgr.set_connection_error(&cfg.name, display_error);
                                    Err(e)
                                }
                            }
                        };

                        if let Ok(()) = connect_result {
                            tracing::info!(target: "mcp", "Connected '{}', registering tools...", cfg.name);
                            if let Some(registry) =
                                app_handle.try_state::<std::sync::Arc<tokio::sync::RwLock<crate::actions::ActionRegistry>>>()
                            {
                                crate::mcp::bridge::register_mcp_tools(&mgr_arc, registry.inner()).await;
                            }
                        } else if let Err(e) = connect_result {
                            tracing::error!(target: "mcp", "Failed to connect '{}': {}", cfg.name, e);
                        }
                    }));
                }

                // Wait for all to finish (fire-and-forget is also fine)
                for h in handles {
                    let _ = h.await;
                }
            });

            // Vision Server
            tracing::info!(
                target: "startup",
                "stage=vision.server.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let mut vision_server = crate::vision::server::VisionServer::new(&app_data);
            tauri::async_runtime::block_on(async {
                vision_server.start().await;
            });
            app.manage(Arc::new(tokio::sync::Mutex::new(vision_server)));
            tracing::info!(
                target: "startup",
                "stage=vision.server.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // ModManager init: spawns QuickJS thread + event relay
            // In debug (dev) mode, fall back to the project-relative `mods/` directory
            // so developers can iterate without copying mods to the app data dir.
            // In release builds, always use the absolute app data path so macOS/Linux
            // bundled apps find mods regardless of the process working directory.
            #[cfg(debug_assertions)]
            let mods_path = {
                let direct = std::path::PathBuf::from("mods");
                if direct.exists() {
                    direct
                } else {
                    let parent = std::path::PathBuf::from("../mods");
                    if parent.exists() {
                        parent
                    } else {
                        app_data.join("mods")
                    }
                }
            };
            #[cfg(not(debug_assertions))]
            let mods_path = app_data.join("mods");
            let _ = std::fs::create_dir_all(&mods_path);

            // On first run, copy bundled default mods from the app resource dir
            // to the user's app data mods dir. Only copies mods that don't exist yet,
            // so user deletions are respected and not overwritten on next launch.
            #[cfg(not(debug_assertions))]
            if let Ok(resource_dir) = app.path().resource_dir() {
                let bundled_mods = resource_dir.join("mods");
                if bundled_mods.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&bundled_mods) {
                        for entry in entries.flatten() {
                            let target = mods_path.join(entry.file_name());
                            if !target.exists() {
                                if let Err(e) = copy_dir_all(entry.path(), &target) {
                                    tracing::error!(target: "mods", "[Mods] Failed to copy bundled mod {:?}: {}", entry.file_name(), e);
                                } else {
                                    tracing::info!(target: "mods", "[Mods] Installed bundled mod {:?}", entry.file_name());
                                }
                            }
                        }
                    }
                }
            }

            tracing::info!(
                target: "startup",
                "stage=mods.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let mut mod_manager = ModManager::new(mods_path);
            mod_manager.init(app.handle().clone());
            app.manage(tokio::sync::Mutex::new(mod_manager));
            tracing::info!(
                target: "startup",
                "stage=mods.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // Heartbeat — proactive behavior background loop
            let heartbeat_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                crate::ai::heartbeat::heartbeat_loop(heartbeat_handle).await;
            });

            // Vision Watcher
            let vision_config_path = app_data.join("vision_config.json");
            let vision_config = crate::vision::config::load_config(&vision_config_path);
            let llm_svc_for_vision = app.state::<crate::llm::service::LlmService>().inner().clone();
            tracing::info!(
                target: "startup",
                "stage=vision.watcher.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let vision_watcher = crate::vision::watcher::VisionWatcher::new(vision_config.clone())
                .with_llm_service(llm_svc_for_vision);
            app.manage(vision_watcher.clone());
            tracing::info!(
                target: "startup",
                "stage=vision.watcher.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // Auto-start vision watcher if previously enabled
            if vision_config.enabled {
                let watcher_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Small delay to let the app fully initialize
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    vision_watcher.start(watcher_handle);
                });
            }

            // Audio Buffer for Streaming STT
            app.manage(crate::stt::stream::AudioBuffer::new());
            app.manage(crate::stt::NativeMicState::new());
            app.manage(crate::stt::NativeWakeWordState::new());

            // Telegram Bot
            let telegram_config_path = app_data.join("telegram_config.json");
            let telegram_config = crate::telegram::load_config(&telegram_config_path);
            let telegram_enabled = telegram_config.enabled;
            tracing::info!(
                target: "startup",
                "stage=telegram.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let telegram_service = crate::telegram::TelegramService::new(telegram_config);
            app.manage(telegram_service.clone());
            tracing::info!(
                target: "startup",
                "stage=telegram.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // Auto-start Telegram bot if enabled
            if telegram_enabled {
                let tg_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Delay to let all services initialize
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    if let Err(e) = telegram_service.start(tg_app).await {
                        tracing::error!(target: "telegram", "[Telegram] Auto-start failed: {}", e);
                    }
                });
            }

            // Global shortcut + Pet window auto-start
            {
                use tauri::Emitter;
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
                let pet_cfg = crate::commands::pet::load_pet_config();
                let shortcut_str = pet_cfg.shortcut.clone();
                let pet_enabled = pet_cfg.enabled;

                if let Ok(shortcut) = shortcut_str.parse::<tauri_plugin_global_shortcut::Shortcut>() {
                    let shortcut_app = app.handle().clone();
                    let _ = app.handle().global_shortcut().on_shortcut(shortcut, move |_app, _sc, event| {
                        if event.state() == ShortcutState::Pressed {
                            if let Some(pet_win) = shortcut_app.get_webview_window("pet") {
                                let _ = pet_win.emit("toggle-chat-input", ());
                            }
                        }
                    });
                }

                if pet_enabled {
                    let pet_app = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        auto_start_pet_on_launch(
                            true,
                            std::time::Duration::from_millis(500),
                            move || async move { crate::commands::pet::show_pet_window(pet_app).await },
                        )
                        .await;
                    });
                }
            }

            tracing::info!(
                target: "startup",
                "setup done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Recursively copy a directory tree from `src` to `dst`.
#[cfg(not(debug_assertions))]
fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::auto_start_pet_on_launch;
    use crate::error::KokoroError;
    use crate::utils::logging::format_log_line;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn auto_start_pet_on_launch_calls_show_when_enabled() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_closure = calls.clone();

        auto_start_pet_on_launch(true, Duration::from_millis(0), move || {
            let calls_for_closure = calls_for_closure.clone();
            async move {
                calls_for_closure.fetch_add(1, Ordering::SeqCst);
                Ok::<(), KokoroError>(())
            }
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn auto_start_pet_on_launch_skips_show_when_disabled() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_closure = calls.clone();

        auto_start_pet_on_launch(false, Duration::from_millis(0), move || {
            let calls_for_closure = calls_for_closure.clone();
            async move {
                calls_for_closure.fetch_add(1, Ordering::SeqCst);
                Ok::<(), KokoroError>(())
            }
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn startup_log_line_uses_level_target_structure() {
        let line = format_log_line("INFO", "ai", "startup", false);
        assert!(line.starts_with("[INFO][ai] "));
    }

    #[test]
    fn invoke_handler_does_not_register_duplicate_rvc_list_command() {
        let source = include_str!("lib.rs");
        let handler_start = source
            .find(".invoke_handler(tauri::generate_handler![")
            .expect("invoke_handler block should exist");
        let block_start = handler_start
            + source[handler_start..]
                .find('[')
                .expect("generate_handler should include `[`")
            + 1;
        let block_end = source[block_start..]
            .find("])")
            .map(|idx| block_start + idx)
            .expect("generate_handler should include closing `])`");

        let block = &source[block_start..block_end];
        let needle = "commands::singing::list_rvc_models";
        let occurrences = block.matches(needle).count();

        assert_eq!(
            occurrences, 1,
            "expected exactly one registration for `{needle}`, found {occurrences}"
        );
    }

    #[test]
    fn invoke_handler_has_no_duplicate_command_registrations() {
        let source = include_str!("lib.rs");
        let handler_start = source
            .find(".invoke_handler(tauri::generate_handler![")
            .expect("invoke_handler block should exist");
        let block_start = handler_start
            + source[handler_start..]
                .find('[')
                .expect("generate_handler should include `[`")
            + 1;
        let block_end = source[block_start..]
            .find("])")
            .map(|idx| block_start + idx)
            .expect("generate_handler should include closing `])`");

        let block = &source[block_start..block_end];
        let mut seen = HashSet::new();
        let mut duplicates = Vec::new();

        for line in block.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }
            let command = trimmed.trim_end_matches(',');
            if command.is_empty() {
                continue;
            }

            if !seen.insert(command.to_string()) {
                duplicates.push(command.to_string());
            }
        }

        assert!(
            duplicates.is_empty(),
            "found duplicate Tauri command registrations: {:?}",
            duplicates
        );
    }
}
