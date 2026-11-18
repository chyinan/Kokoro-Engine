// pattern: Mixed (unavoidable)
// Reason: 应用入口文件需要同时声明模块、注册 Tauri 命令、初始化服务与恢复磁盘状态，天然属于编排层。
pub mod actions;
pub mod ai;
mod app_shutdown;
pub(crate) mod bot;
pub mod characters;
pub mod chat;
pub mod commands;
pub mod config;
mod desktop_chat;
mod desktop_heartbeat;
pub mod error;
pub mod hooks;
pub mod imagegen;
pub mod ipc;
mod lifecycle;
pub mod live2d;
pub mod llm;
pub mod mcp;
pub mod mods;
pub mod qqbot;
pub mod registry;
pub mod stt;
pub mod telegram;
pub mod tts;
pub mod utils;
pub mod vision;
use crate::hooks::{AuditLogHookHandler, HookRuntime};
use crate::lifecycle::{AppLifecycle, TaskScope, TaskStage};
use crate::mods::ModManager;
use crate::utils::logging::init_logging;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;

fn spawn_startup<F>(
    scope: &TaskScope,
    name: &str,
    delay: Duration,
    start: F,
) where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let cancel = scope.token();
    if let Err(error) = scope.spawn_durable(name, async move {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
        start.await;
    }) {
        tracing::debug!(target: "startup", task = name, %error, "skipped startup job");
    }
}

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

    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }

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
            if is_usable_dylib(&candidate) {
                tracing::info!(target: "tools", "Using bundled ONNX Runtime: {}", candidate.display());
                std::env::set_var("ORT_DYLIB_PATH", &candidate);
                break;
            }
        }
    }

    let ipc = ipc::builder();
    let desktop_owner = Arc::new(std::sync::Mutex::new(None));
    let setup_desktop_owner = desktop_owner.clone();
    let lifecycle = AppLifecycle::with_runtime(tauri::async_runtime::handle().inner().clone());
    let exit_completion = app_shutdown::ExitCompletion::default();
    let application = tauri::Builder::default()
        .manage(lifecycle)
        .manage(exit_completion.clone())
        .manage(crate::characters::activation::ActivationCoordinator::default())
        .manage(Arc::new(crate::chat::runtime::ChatRuntimeState::default()))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .register_uri_scheme_protocol("mod", crate::mods::protocol::handle_mod_request)
        .register_uri_scheme_protocol("live2d", commands::live2d_protocol::handle_live2d_request())
        .register_uri_scheme_protocol(
            "character-instance-resource",
            commands::live2d_protocol::handle_character_instance_resource_request(),
        )
        .invoke_handler(ipc::invoke_handler(&ipc))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
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
        .setup(move |app| {
            ipc.mount_events(app);
            let host = Arc::new(app.handle().clone());
            let desktop = desktop_chat::TauriChatDesktop::new(&host);
            *setup_desktop_owner.lock().unwrap() = Some(host);
            let startup_begin = std::time::Instant::now();
            tracing::info!(target: "startup", "setup begin");
            app.manage(crate::commands::pet::PetShortcutState::default());
            let app_lifecycle = app.state::<AppLifecycle>().inner().clone();
            let startup_scope = app_lifecycle.scope(TaskStage::Work, "startup");

            let app_handle = app.handle();
            let lifecycle_for_ai = app_lifecycle.clone();
            tauri::async_runtime::block_on(async move {
                tracing::info!(
                    target: "startup",
                    "stage=ai.init.start elapsed_ms={}",
                    startup_begin.elapsed().as_millis()
                );
                let app_data_dir = crate::utils::paths::data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("com.chyin.kokoro");
                let _ = std::fs::create_dir_all(&app_data_dir);
                let db_path = app_data_dir.join("kokoro.db");
                let db_url = format!("sqlite:///{}", db_path.to_string_lossy().replace('\\', "/"));
                match crate::ai::context::AIOrchestrator::new_with_lifecycle(
                    &db_url,
                    lifecycle_for_ai,
                ).await {
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

                        // Restore jailbreak_prompt from disk (with backup recovery)
                        let jailbreak_path = app_data_dir.join("jailbreak_prompt.json");
                        if let Some(prompt) = crate::config::load_jailbreak_prompt(&jailbreak_path) {
                            if !prompt.is_empty() {
                                orchestrator.set_jailbreak_prompt(prompt.clone()).await;
                                tracing::info!(target: "ai", "Restored jailbreak_prompt ({} chars)", prompt.len());
                            }
                        }

                        crate::chat::settings::ContextSettings::restore(
                            &orchestrator,
                            &app_data_dir.join("context_settings.json"),
                        ).await;

                        let vision_config_path = app_data_dir.join("vision_config.json");
                        let vision_config = crate::vision::config::load_config(&vision_config_path);
                        orchestrator
                            .set_vision_context_history_mode(
                                vision_config.vision_context_history_mode.clone(),
                            )
                            .await;
                        tracing::info!(
                            target: "ai",
                            "Restored vision_context_history_mode={}",
                            vision_config.vision_context_history_mode
                        );

                        // Restore current_conversation_id from disk and reload history
                        let conv_id_path = app_data_dir.join("current_conversation_id.json");
                        if let Ok(content) = std::fs::read_to_string(&conv_id_path) {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(id) = val.get("conversation_id").and_then(|v| v.as_str()) {
                                    {
                                        let mut conv_id = orchestrator.selection.current_conversation_id.lock().await;
                                        *conv_id = Some(id.to_string());
                                    }
                                    // Reload messages into in-memory history so LLM has conversation context
                                    if let Ok(rows) = sqlx::query_as::<_, (String, String, Option<String>)>(
                                        "SELECT role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC"
                                    )
                                    .bind(id)
                                    .fetch_all(&orchestrator.db)
                                    .await {
                                        let total = rows.len();
                                        orchestrator.sync_history_from_rows(rows).await;
                                        tracing::info!(target: "ai", "Restored current_conversation_id: {} ({} messages, windowed to {})", id, total, orchestrator.selection.history.lock().await.len());
                                    }
                                }
                            }
                        }

                        if let Some(profile) =
                            crate::commands::context::load_user_profile_settings_from_app_data(
                                &app_data_dir,
                            )
                        {
                            orchestrator.set_user_name(profile.user_name.clone()).await;
                            tracing::info!(
                                target: "ai",
                                "Restored user_profile: user_name={}, persona_chars={}",
                                profile.user_name,
                                profile.user_persona.len()
                            );
                        }

                        app_handle.manage(orchestrator);

                        // The committed activation snapshot is authoritative for crash/window
                        // recovery. Legacy ID-only persistence is used only before the first
                        // coordinated activation exists.
                        let coordinator = app_handle
                            .state::<crate::characters::activation::ActivationCoordinator>();
                        let orch = app_handle.state::<crate::ai::context::AIOrchestrator>();
                        match crate::commands::characters::recover_committed_character_runtime_for_startup(
                            &coordinator,
                            &orch,
                            &app_data_dir,
                        )
                        .await
                        {
                            Ok(Some(committed)) => {
                                orch.clear_runtime_degraded().await;
                                tracing::info!(
                                    target: "ai",
                                    "Restored committed character runtime: character_id={} revision={}",
                                    committed.runtime.character_id,
                                    committed.revision
                                );
                            }
                            Ok(None) => {
                                orch.clear_runtime_degraded().await;
                                if let Some(char_id) = crate::ai::context::AIOrchestrator::load_active_character_id() {
                                    orch.set_character_id(char_id.clone()).await;
                                    tracing::info!(target: "ai", "Restored legacy active_character_id: {}", char_id);
                                }
                            }
                            Err(error) => {
                                let error_message = format!("Failed to restore committed character runtime: {error}");
                                tracing::error!(
                                    target: "ai",
                                    "{}",
                                    error_message
                                );
                                orch.set_runtime_degraded(Some(error_message)).await;
                                orch.reset_history_and_boundary().await;
                            }
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
            let app_data = crate::utils::paths::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("com.chyin.kokoro");

            // Initialize the versioned character catalog before any IPC command can use it.
            let character_catalog = crate::characters::catalog::CharacterCatalog::new(
                app_data.join("characters"),
                semver::Version::parse(env!("CARGO_PKG_VERSION"))
                    .expect("Cargo package version must be semantic"),
            );
            let bundled_characters = app
                .path()
                .resource_dir()
                .ok()
                .map(|root| root.join("characters"))
                .filter(|path| path.is_dir())
                .or_else(|| {
                    [
                        std::path::PathBuf::from("characters"),
                        std::path::PathBuf::from("../characters"),
                    ]
                    .into_iter()
                    .find(|path| path.is_dir())
                });
            if let Some(bundled_characters) = bundled_characters {
                match character_catalog.install_bundled(&bundled_characters) {
                    Ok(installed) if !installed.is_empty() => tracing::info!(
                        target: "characters",
                        "installed or refreshed {} bundled character package(s)",
                        installed.len()
                    ),
                    Ok(_) => {}
                    Err(error) => tracing::error!(
                        target: "characters",
                        "failed to initialize bundled character catalog: {}",
                        error
                    ),
                }
            }
            app.manage(character_catalog);

            // TTS
            let tts_config_path = app_data.join("tts_config.json");
            let tts_config = crate::tts::load_config(&tts_config_path);

            tracing::info!(
                target: "startup",
                "stage=tts.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let tts_service = tauri::async_runtime::block_on(async {
                crate::tts::TtsService::init_from_config_with_scope(
                    &tts_config,
                    app_lifecycle.scope(TaskStage::Service, "tts-process"),
                ).await
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
            app.manage(crate::imagegen::WindowSizeState::new());

            // LLM
            let llm_config_path = app_data.join("llm_config.json");
            let llm_config = crate::llm::llm_config::load_config(&llm_config_path);
            tracing::info!(
                target: "startup",
                "stage=llm.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let llm_service = crate::llm::service::LlmService::from_config_with_lifecycle(
                llm_config,
                llm_config_path,
                app_lifecycle.clone(),
            );
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
                crate::stt::SttService::init_from_config_with_scope(
                    &stt_config,
                    app_lifecycle.scope(TaskStage::Work, "stt"),
                ).await
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
            app.manage(Arc::new(crate::chat::application::PendingToolApprovalState::new()));
            app.manage(Arc::new(crate::chat::application::TurnCancellationState::new()));

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
            let mut mcp_manager = crate::mcp::McpManager::new_with_scope(
                mcp_config_path.to_str().unwrap_or("mcp_servers.json"),
                app_lifecycle.scope(TaskStage::Service, "mcp"),
            );
            mcp_manager.load_configs();
            let mcp_manager = Arc::new(tokio::sync::Mutex::new(mcp_manager));
            app.manage(mcp_manager.clone());
            tracing::info!(
                target: "startup",
                "stage=mcp.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // Each supported server connects independently outside application state locks.
            let mcp_app = app.handle().clone();
            spawn_startup(&startup_scope, "mcp-connect", Duration::from_secs(3), async move {
                if let Some(registry) = mcp_app.try_state::<Arc<tokio::sync::RwLock<crate::actions::ActionRegistry>>>() {
                    crate::mcp::bridge::start_enabled(&mcp_manager, registry.inner()).await;
                }
            });

            // Vision Server
            tracing::info!(
                target: "startup",
                "stage=vision.server.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let mut vision_server = crate::vision::server::VisionServer::new(&app_data);
            let vision_http = app_lifecycle.scope(TaskStage::Service, "vision-http");
            tauri::async_runtime::block_on(async {
                if let Err(error) = vision_server.start(&vision_http) {
                    tracing::error!(target: "vision", %error, "vision file server did not start");
                }
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
            app.manage(mod_manager.init(app.handle().clone(), &app_lifecycle));
            app.manage(tokio::sync::Mutex::new(mod_manager));
            tracing::info!(
                target: "startup",
                "stage=mods.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // Vision Watcher
            let vision_config_path = app_data.join("vision_config.json");
            let vision_config = crate::vision::config::load_config(&vision_config_path);
            let llm_svc_for_vision = app.state::<crate::llm::service::LlmService>().inner().clone();
            tracing::info!(
                target: "startup",
                "stage=vision.watcher.init.start elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );
            let vision_watcher = crate::vision::watcher::VisionWatcher::new_with_scope(
                vision_config.clone(),
                app_lifecycle.scope(TaskStage::Work, "vision"),
            ).with_llm_service(llm_svc_for_vision);
            app.manage(vision_watcher.clone());
            tracing::info!(
                target: "startup",
                "stage=vision.watcher.init.done elapsed_ms={}",
                startup_begin.elapsed().as_millis()
            );

            // Auto-start vision watcher if previously enabled
            if vision_config.vlm_enabled && vision_config.auto_vision_enabled {
                let watcher_handle = app.handle().clone();
                spawn_startup(&startup_scope, "vision-watcher", Duration::from_secs(2), async move {
                    if let Err(error) = vision_watcher.start(watcher_handle).await {
                        tracing::warn!(target: "vision", %error, "vision watcher did not start");
                    }
                });
            }

            // Audio Buffer for Streaming STT
            app.manage(crate::stt::stream::AudioBuffer::new());
            app.manage(crate::stt::NativeMicState::new(
                app_lifecycle.scope(TaskStage::Work, "native-mic-work"),
                app_lifecycle.scope(TaskStage::Service, "native-mic"),
            ));
            app.manage(crate::stt::NativeWakeWordState::new(
                app_lifecycle.scope(TaskStage::Work, "wake-word-work"),
                app_lifecycle.scope(TaskStage::Service, "wake-word"),
            ));

            let bot_services = desktop_chat::install_services(app.handle(), &app_data, desktop);

            if let Some(chat) = app.try_state::<crate::chat::application::ChatService>() {
                let heartbeat_handle = app.handle().clone();
                let chat = chat.inner().clone();
                let heartbeat = app_lifecycle.scope(TaskStage::Work, "heartbeat");
                if let Err(error) = heartbeat.spawn_durable(
                    "loop",
                    desktop_heartbeat::heartbeat_loop(heartbeat_handle, chat, heartbeat.clone()),
                ) {
                    tracing::warn!(target: "startup", %error, "heartbeat did not start");
                }
            }

            let bot_config = crate::bot::load_bot_config();
            let telegram_config = bot_config.telegram.clone();
            let telegram_enabled = telegram_config.enabled;
            let bot_runtime_service = crate::bot::BotRuntimeService::new(
                bot_config,
                app_lifecycle.scope(TaskStage::Work, "bot-requests"),
                app_lifecycle.scope(TaskStage::Service, "bot"),
            );
            app.manage(bot_runtime_service.clone());
            let telegram_service = crate::telegram::TelegramService::new(
                telegram_config,
                app_lifecycle.scope(TaskStage::Work, "telegram-updates"),
                app_lifecycle.scope(TaskStage::Service, "telegram"),
            );
            app.manage(telegram_service.clone());

            if telegram_enabled {
                let telegram_services = bot_services.clone();
                spawn_startup(&startup_scope, "telegram", Duration::from_secs(3), async move {
                    if let Err(error) = telegram_service.start(telegram_services).await {
                        tracing::error!(target: "telegram", "[Telegram] Auto-start failed: {error}");
                    }
                });
            }
            spawn_startup(&startup_scope, "bot", Duration::from_secs(3), async move {
                bot_runtime_service.start_enabled(bot_services).await;
            });

            // Global shortcut + Pet window auto-start
            {
                let pet_cfg = crate::commands::pet::load_pet_config();
                let pet_enabled = pet_cfg.enabled;

                if let Err(error) =
                    crate::commands::pet::register_pet_shortcut(app.handle(), &pet_cfg.shortcut)
                {
                    tracing::error!(target: "pet", "failed to register pet shortcut: {}", error);
                }

                if pet_enabled {
                    let pet_app = app.handle().clone();
                    spawn_startup(&startup_scope, "pet", Duration::from_millis(500), async move {
                        auto_start_pet_on_launch(
                            true,
                            Duration::ZERO,
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
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    let on_event = move |app: &tauri::AppHandle, event| match event {
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            app_shutdown::on_exit_requested(app, code, api);
        }
        tauri::RunEvent::Exit => {
            desktop_owner.lock().unwrap().take();
        }
        _ => {}
    };
    #[cfg(mobile)]
    application.run(on_event);
    #[cfg(not(mobile))]
    {
        // Wry can finish its event loop with code zero even when a
        // RequestExit carried a nonzero code. Retain the lifecycle's code
        // independently of Tauri state until native cleanup returns.
        let runtime_code = application.run_return(on_event);
        std::process::exit(exit_completion.code().unwrap_or(runtime_code));
    }
}

fn is_usable_dylib(path: &Path) -> bool {
    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
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
}
