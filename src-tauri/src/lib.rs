pub mod actions;
pub mod ai;
pub mod commands;
pub mod config;
pub mod imagegen;
pub mod llm;
pub mod mcp;
pub mod mods;
pub mod stt;
pub mod telegram;
pub mod tts;
pub mod utils;
pub mod vision;
use crate::mods::ModManager;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_uri_scheme_protocol("mod", crate::mods::protocol::handle_mod_request)
        .register_uri_scheme_protocol("live2d", {
            // Compute models dir eagerly — protocol handler runs before .setup()
            let app_data = dirs_next::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("com.chyin.kokoro");
            let models_dir = app_data.join("live2d_models");
            commands::live2d_protocol::handle_live2d_request(models_dir)
        })
        .invoke_handler(tauri::generate_handler![
            commands::system::get_engine_info,
            commands::system::get_system_status,
            commands::character::get_character_state,
            commands::character::set_expression,
            commands::character::send_message,
            commands::database::init_db,
            commands::database::test_vector_store,
            commands::chat::stream_chat,
            commands::context::set_persona,
            commands::context::set_character_name,
            commands::context::set_user_name,
            commands::context::set_response_language,
            commands::context::set_user_language,
            commands::context::set_jailbreak_prompt,
            commands::context::get_jailbreak_prompt,
            commands::context::set_proactive_enabled,
            commands::context::get_proactive_enabled,
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
            commands::live2d::list_live2d_models,
            commands::live2d::delete_live2d_model,
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
            commands::conversation::list_conversations,
            commands::conversation::load_conversation,
            commands::conversation::delete_conversation,
            commands::conversation::create_conversation,
            commands::conversation::rename_conversation,
            commands::llm::get_llm_config,
            commands::llm::save_llm_config,
            commands::llm::list_ollama_models,
            commands::llm::pull_ollama_model,
            commands::stt::transcribe_audio,
            commands::stt::get_stt_config,
            commands::stt::save_stt_config,
            commands::actions::list_actions,
            commands::actions::execute_action,
            commands::mcp::list_mcp_servers,
            commands::mcp::add_mcp_server,
            commands::mcp::remove_mcp_server,
            commands::mcp::refresh_mcp_tools,
            commands::mcp::reconnect_mcp_server,
            commands::singing::check_rvc_status,
            commands::singing::list_rvc_models,
            commands::singing::list_rvc_models,
            commands::singing::convert_singing,
            commands::telegram::get_telegram_config,
            commands::telegram::save_telegram_config,
            commands::telegram::start_telegram_bot,
            commands::telegram::stop_telegram_bot,
            commands::telegram::get_telegram_status,
            stt::stream::process_audio_chunk,
            stt::stream::complete_audio_stream,
            stt::stream::discard_audio_stream,
            stt::stream::snapshot_audio_stream,
            stt::stream::prune_audio_buffer,
        ])
        .setup(|app| {
            let app_handle = app.handle();
            tauri::async_runtime::block_on(async move {
                let db_url = "sqlite://kokoro.db";
                match crate::ai::context::AIOrchestrator::new(db_url).await {
                    Ok(orchestrator) => {
                        let app_data_dir = dirs_next::data_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("com.chyin.kokoro");

                        // Restore proactive_enabled from disk
                        let proactive_path = app_data_dir.join("proactive_enabled.json");
                        if let Ok(content) = std::fs::read_to_string(&proactive_path) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(enabled) = val.get("enabled").and_then(|v| v.as_bool()) {
                                    orchestrator.set_proactive_enabled(enabled);
                                    println!("[AI] Restored proactive_enabled={}", enabled);
                                }
                            }
                        }

                        // Restore jailbreak_prompt from disk
                        let jailbreak_path = app_data_dir.join("jailbreak_prompt.json");
                        if let Ok(content) = std::fs::read_to_string(&jailbreak_path) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(prompt) = val.get("prompt").and_then(|v| v.as_str()) {
                                    orchestrator.set_jailbreak_prompt(prompt.to_string()).await;
                                    println!("[AI] Restored jailbreak_prompt ({} chars)", prompt.len());
                                }
                            }
                        }

                        // Restore emotion state from disk
                        if let Err(e) = orchestrator.load_emotion_state().await {
                            println!("[AI] Failed to restore emotion state: {}", e);
                        }

                        app_handle.manage(orchestrator);
                    }
                    Err(e) => {
                        eprintln!("AI Orchestrator init failed (will run without AI): {}", e);
                        // Do NOT panic — allow app to continue running
                    }
                }
            });

            // Initialize TTS Service from config
            let app_data = dirs_next::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("com.chyin.kokoro");

            // TTS
            let tts_config_path = app_data.join("tts_config.json");
            let tts_config = crate::tts::load_config(&tts_config_path);

            let tts_service = tauri::async_runtime::block_on(async {
                crate::tts::TtsService::init_from_config(&tts_config).await
            });
            app.manage(tts_service);

            // ImageGen
            let imagegen_config_path = app_data.join("imagegen_config.json");
            let imagegen_config = crate::imagegen::config::load_config(&imagegen_config_path);

            let imagegen_service = tauri::async_runtime::block_on(async {
                crate::imagegen::ImageGenService::init_from_config(&imagegen_config).await
            });
            app.manage(imagegen_service);

            // LLM
            let llm_config_path = app_data.join("llm_config.json");
            let llm_config = crate::llm::llm_config::load_config(&llm_config_path);
            let llm_service =
                crate::llm::service::LlmService::from_config(llm_config, llm_config_path);
            app.manage(llm_service);

            // STT
            let stt_config_path = app_data.join("stt_config.json");
            let stt_config = crate::stt::load_config(&stt_config_path);
            let stt_service = tauri::async_runtime::block_on(async {
                crate::stt::SttService::init_from_config(&stt_config).await
            });
            app.manage(stt_service);

            // Action Registry
            let mut action_registry = crate::actions::ActionRegistry::new();
            crate::actions::builtin::register_builtins(&mut action_registry);
            app.manage(std::sync::Arc::new(tokio::sync::RwLock::new(
                action_registry,
            )));

            // MCP Manager
            let mcp_config_path = app_data.join("mcp_servers.json");
            let mut mcp_manager =
                crate::mcp::McpManager::new(mcp_config_path.to_str().unwrap_or("mcp_servers.json"));
            mcp_manager.load_configs();
            let mcp_manager = Arc::new(tokio::sync::Mutex::new(mcp_manager));
            app.manage(mcp_manager.clone());

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
                        let connect_result = {
                            let mut mgr = mgr_arc.lock().await;
                            let result = mgr.connect_server(&cfg).await;
                            mgr.clear_connecting(&cfg.name);
                            if let Err(ref e) = result {
                                mgr.set_connection_error(&cfg.name, e.to_string());
                            }
                            result
                        };
                        if let Ok(()) = connect_result {
                            println!("[MCP] Connected '{}', registering tools...", cfg.name);
                            if let Some(registry) =
                                app_handle.try_state::<std::sync::Arc<tokio::sync::RwLock<crate::actions::ActionRegistry>>>()
                            {
                                crate::mcp::bridge::register_mcp_tools(&mgr_arc, registry.inner()).await;
                            }
                        } else if let Err(e) = connect_result {
                            eprintln!("[MCP] Failed to connect '{}': {}", cfg.name, e);
                        }
                    }));
                }

                // Wait for all to finish (fire-and-forget is also fine)
                for h in handles {
                    let _ = h.await;
                }
            });

            // Vision Server
            let mut vision_server = crate::vision::server::VisionServer::new(&app_data);
            tauri::async_runtime::block_on(async {
                vision_server.start().await;
            });
            app.manage(Arc::new(tokio::sync::Mutex::new(vision_server)));

            // ModManager init: spawns QuickJS thread + event relay
            let mut mods_path = std::path::PathBuf::from("mods");
            if !mods_path.exists() {
                 let parent_mods = std::path::Path::new("../mods");
                 if parent_mods.exists() {
                     mods_path = parent_mods.to_path_buf();
                 }
            }
            let mut mod_manager = ModManager::new(mods_path);
            mod_manager.init(app.handle().clone());
            app.manage(tokio::sync::Mutex::new(mod_manager));

            // Heartbeat — proactive behavior background loop
            let heartbeat_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                crate::ai::heartbeat::heartbeat_loop(heartbeat_handle).await;
            });

            // Vision Watcher
            let vision_config_path = app_data.join("vision_config.json");
            let vision_config = crate::vision::config::load_config(&vision_config_path);
            let vision_watcher = crate::vision::watcher::VisionWatcher::new(vision_config.clone());
            app.manage(vision_watcher.clone());

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

            // Telegram Bot
            let telegram_config_path = app_data.join("telegram_config.json");
            let telegram_config = crate::telegram::load_config(&telegram_config_path);
            let telegram_enabled = telegram_config.enabled;
            let telegram_service = crate::telegram::TelegramService::new(telegram_config);
            app.manage(telegram_service.clone());

            // Auto-start Telegram bot if enabled
            if telegram_enabled {
                let tg_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Delay to let all services initialize
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    if let Err(e) = telegram_service.start(tg_app).await {
                        eprintln!("[Telegram] Auto-start failed: {}", e);
                    }
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
