use crate::ai::context::history::HistoryDeletionTarget;
use crate::ai::context::AIOrchestrator;
use crate::error::KokoroError;
use crate::llm::messages::{system_message, user_text_message};
use crate::llm::provider::{build_openai_client, create_chat};
use tauri::{AppHandle, Manager, State};

pub use crate::config::MemoryUpgradeConfig;

fn memory_upgrade_config_path() -> std::path::PathBuf {
    crate::ai::memory::memory_upgrade_config_path()
}

const USER_PROFILE_SETTINGS_FILE: &str = "user_profile.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserProfileSettings {
    pub user_name: String,
    pub user_persona: String,
}

impl Default for UserProfileSettings {
    fn default() -> Self {
        Self {
            user_name: "User".to_string(),
            user_persona: String::new(),
        }
    }
}

fn app_data_dir(app: &AppHandle) -> Result<std::path::PathBuf, KokoroError> {
    app.path()
        .app_data_dir()
        .map_err(|e| KokoroError::Internal(format!("Failed to resolve app data dir: {}", e)))
}

fn user_profile_settings_path(app_data: &std::path::Path) -> std::path::PathBuf {
    app_data.join(USER_PROFILE_SETTINGS_FILE)
}

pub fn load_user_profile_settings_from_app_data(
    app_data: &std::path::Path,
) -> Option<UserProfileSettings> {
    let path = user_profile_settings_path(app_data);
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_user_profile_settings_to_app_data(
    app_data: &std::path::Path,
    settings: &UserProfileSettings,
) -> Result<(), KokoroError> {
    std::fs::create_dir_all(app_data).map_err(KokoroError::from)?;
    let path = user_profile_settings_path(app_data);
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| KokoroError::Config(format!("Serialize error: {}", e)))?;
    std::fs::write(path, json).map_err(KokoroError::from)
}

fn update_user_profile_settings<F>(app: &AppHandle, update: F) -> Result<(), KokoroError>
where
    F: FnOnce(&mut UserProfileSettings),
{
    let app_data = app_data_dir(app)?;
    let mut settings = load_user_profile_settings_from_app_data(&app_data).unwrap_or_default();
    update(&mut settings);
    save_user_profile_settings_to_app_data(&app_data, &settings)
}

#[tauri::command]
pub async fn set_memory_upgrade_config(config: MemoryUpgradeConfig) -> Result<(), KokoroError> {
    crate::config::save_memory_upgrade_config(&memory_upgrade_config_path(), &config)
}

#[tauri::command]
pub async fn get_memory_upgrade_config() -> Result<MemoryUpgradeConfig, KokoroError> {
    Ok(crate::config::load_memory_upgrade_config(
        &memory_upgrade_config_path(),
    ))
}

#[tauri::command]
pub async fn get_memory_observability_summary(
    state: State<'_, AIOrchestrator>,
) -> Result<crate::ai::memory::MemoryObservabilitySummary, KokoroError> {
    state
        .memory_manager
        .memory_observability_summary()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[tauri::command]
pub async fn get_latest_memory_write_event(
    state: State<'_, AIOrchestrator>,
) -> Result<Option<crate::ai::memory::MemoryWriteEventRecord>, KokoroError> {
    state
        .memory_manager
        .latest_memory_write_event()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[tauri::command]
pub async fn get_latest_memory_retrieval_log(
    state: State<'_, AIOrchestrator>,
) -> Result<Option<crate::ai::memory::MemoryRetrievalLogRecord>, KokoroError> {
    state
        .memory_manager
        .latest_memory_retrieval_log()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[tauri::command]
pub async fn get_latest_memory_retrieval_eval_summary(
    state: State<'_, AIOrchestrator>,
) -> Result<Option<crate::ai::memory::MemoryRetrievalEvalSummary>, KokoroError> {
    state
        .memory_manager
        .latest_memory_retrieval_eval_summary()
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_upgrade_config_roundtrip_uses_shared_path_rules() {
        let path = memory_upgrade_config_path();
        assert!(
            path.ends_with("com.chyin.kokoro/memory_upgrade_config.json")
                || path.ends_with("com.chyin.kokoro\\memory_upgrade_config.json")
        );
    }

    #[tokio::test]
    async fn jailbreak_prompt_recovers_from_backup_when_target_corrupted() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let jailbreak_path = temp_dir.path().join("jailbreak_prompt.json");
        let backup_path = temp_dir.path().join(".jailbreak_prompt.json.backup");

        // 1. Write corrupted content to target
        std::fs::write(&jailbreak_path, "INVALID_CORRUPTED_JSON{{{{").expect("write corrupted");

        // 2. Write valid prompt to backup
        let valid_json = serde_json::to_string(&crate::config::JailbreakConfig {
            prompt: "system prompt to be recovered".to_string(),
        })
        .unwrap();
        std::fs::write(&backup_path, valid_json).expect("write backup");

        // 3. Create orchestrator
        let orchestrator = AIOrchestrator::new("sqlite::memory:").await.unwrap();

        // 4. Perform load / startup recovery
        let prompt_opt = crate::config::load_jailbreak_prompt(&jailbreak_path);
        assert_eq!(
            prompt_opt,
            Some("system prompt to be recovered".to_string())
        );

        if let Some(ref prompt) = prompt_opt {
            orchestrator.set_jailbreak_prompt(prompt.clone()).await;
        }

        // Verify MEMORY is recovered
        assert_eq!(
            orchestrator.get_jailbreak_prompt().await,
            Some("system prompt to be recovered".to_string())
        );

        // Verify DISK target is recovered and parseable
        let disk_content = std::fs::read_to_string(&jailbreak_path).expect("read restored target");
        let disk_val: crate::config::JailbreakConfig =
            serde_json::from_str(&disk_content).expect("parse restored target");
        assert_eq!(disk_val.prompt, "system prompt to be recovered");

        // Verify NEXT STARTUP recovers cleanly from the now-restored target (even if backup is removed)
        std::fs::remove_file(&backup_path).expect("remove backup");
        let next_startup_prompt = crate::config::load_jailbreak_prompt(&jailbreak_path);
        assert_eq!(
            next_startup_prompt,
            Some("system prompt to be recovered".to_string())
        );
    }

    #[test]
    fn jailbreak_prompt_recovers_from_backup_when_target_missing() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let jailbreak_path = temp_dir.path().join("jailbreak_prompt.json");
        let backup_path = temp_dir.path().join(".jailbreak_prompt.json.backup");

        let valid_json = serde_json::to_string(&crate::config::JailbreakConfig {
            prompt: "backup only prompt".to_string(),
        })
        .unwrap();
        std::fs::write(&backup_path, valid_json).expect("write backup");

        assert!(!jailbreak_path.exists());
        let prompt_opt = crate::config::load_jailbreak_prompt(&jailbreak_path);
        assert_eq!(prompt_opt, Some("backup only prompt".to_string()));
        assert!(jailbreak_path.exists());
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct MemorySystemConfig {
    enabled: bool,
}

fn memory_config_path() -> std::path::PathBuf {
    crate::utils::paths::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("memory_system_config.json")
}

#[tauri::command]
pub async fn set_persona(
    prompt: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let _ = (prompt, state);
    Err(KokoroError::Validation("Deprecated partial character update; use prepare_character_activation/apply/commit or character runtime overrides".into()))
}

#[tauri::command]
pub async fn set_character_name(
    name: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let _ = (name, state);
    Err(KokoroError::Validation("Deprecated partial character update; use prepare_character_activation/apply/commit or character runtime overrides".into()))
}

#[tauri::command]
pub async fn set_active_character_id(
    id: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let _ = (id, state);
    Err(KokoroError::Validation("Deprecated partial character update; use prepare_character_activation/apply/commit or character runtime overrides".into()))
}

#[tauri::command]
pub async fn get_user_profile_settings(
    app: AppHandle,
) -> Result<Option<UserProfileSettings>, KokoroError> {
    let app_data = app_data_dir(&app)?;
    Ok(load_user_profile_settings_from_app_data(&app_data))
}

#[tauri::command]
pub async fn set_user_name(
    name: String,
    app: AppHandle,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let _activation = state.acquire_activation_lock().await;
    update_user_profile_settings(&app, |settings| {
        settings.user_name = name.clone();
    })?;
    state.set_user_name(name).await;
    Ok(())
}

#[tauri::command]
pub async fn set_user_persona(persona: String, app: AppHandle) -> Result<(), KokoroError> {
    update_user_profile_settings(&app, |settings| {
        settings.user_persona = persona;
    })?;
    Ok(())
}

#[tauri::command]
pub async fn set_response_language(
    language: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let _ = (language, state);
    Err(KokoroError::Validation("Deprecated partial character update; use prepare_character_activation/apply/commit or character runtime overrides".into()))
}

#[tauri::command]
pub async fn set_user_language(
    language: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let _activation = state.acquire_activation_lock().await;
    state.set_user_language(language).await;
    Ok(())
}

pub(crate) fn jailbreak_prompt_path() -> std::path::PathBuf {
    crate::utils::paths::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("jailbreak_prompt.json")
}

#[tauri::command]
pub async fn set_jailbreak_prompt(
    prompt: String,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let _activation = state.acquire_activation_lock().await;
    // Persist to disk first to avoid split-brain if disk write fails
    let path = jailbreak_prompt_path();
    crate::config::save_json_config(
        &path,
        &crate::config::JailbreakConfig {
            prompt: prompt.clone(),
        },
        "JAILBREAK",
    )?;

    state.set_jailbreak_prompt(prompt).await;
    Ok(())
}

#[tauri::command]
pub async fn get_jailbreak_prompt(state: State<'_, AIOrchestrator>) -> Result<String, KokoroError> {
    let _activation = state.acquire_activation_lock().await;
    if let Some(memory_prompt) = state.get_jailbreak_prompt().await {
        return Ok(memory_prompt);
    }

    // Fallback read from disk with backup recovery
    let path = jailbreak_prompt_path();
    let prompt = crate::config::load_jailbreak_prompt(&path).unwrap_or_default();
    if !prompt.is_empty() {
        state.set_jailbreak_prompt(prompt.clone()).await;
    }
    Ok(prompt)
}

#[tauri::command]
pub async fn set_proactive_enabled(
    enabled: bool,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_proactive_enabled(enabled);
    tracing::info!(
        target: "ai",
        "Proactive messages {}",
        if enabled { "enabled" } else { "disabled" }
    );

    // Persist to disk
    let app_data = crate::utils::paths::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro");
    let path = app_data.join("proactive_enabled.json");
    let _ = std::fs::write(&path, serde_json::json!({ "enabled": enabled }).to_string());
    Ok(())
}

#[tauri::command]
pub async fn get_proactive_enabled(state: State<'_, AIOrchestrator>) -> Result<bool, KokoroError> {
    Ok(state.is_proactive_enabled())
}

#[tauri::command]
pub async fn set_memory_enabled(
    enabled: bool,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state.set_memory_enabled(enabled).await;
    crate::config::save_json_config(
        &memory_config_path(),
        &MemorySystemConfig { enabled },
        "MEMORY",
    )
}

#[tauri::command]
pub async fn get_memory_enabled(state: State<'_, AIOrchestrator>) -> Result<bool, KokoroError> {
    Ok(state.is_memory_enabled())
}

#[tauri::command]
pub async fn clear_history(state: State<'_, AIOrchestrator>) -> Result<(), KokoroError> {
    let _activation_guard = state.enter_chat_turn().map_err(KokoroError::Chat)?;
    state.clear_history().await;
    Ok(())
}

pub use crate::chat::tags::{is_visible_message_meta, is_visible_message_raw};

#[tauri::command]
pub async fn delete_last_messages(
    count: usize,
    expected_conversation_id: Option<String>,
    expected_tail_message_id: Option<i64>,
    expected_history: Option<HistoryDeletionTarget>,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    state
        .delete_last_messages(
            count,
            expected_conversation_id.as_deref(),
            expected_tail_message_id,
            expected_history,
        )
        .await
}

/// End the current session: generate a summary from recent history, save it,
/// then clear conversation history. The summary generation runs in background.
#[derive(serde::Deserialize)]
pub struct EndSessionRequest {
    pub api_key: String,
    pub endpoint: Option<String>,
    pub model: Option<String>,
}

#[tauri::command]
pub async fn end_session(
    request: EndSessionRequest,
    state: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    let _activation_guard = state.enter_chat_turn().map_err(KokoroError::Chat)?;
    let _history_guard = state
        .try_acquire_chat_turn("history_mutation")
        .await
        .map_err(KokoroError::Chat)?;
    if !state.is_memory_enabled() {
        state.clear_history().await;
        return Ok(());
    }

    let history = state.get_recent_history(20).await;
    let char_id = state.get_character_id().await;
    let memory_mgr = state.memory_manager.clone();
    let memory_enabled = state.memory_enabled_flag();
    let summary_language = state.response_language.lock().await.clone();

    // Clear history immediately so the user can start fresh
    state.clear_history().await;

    // Generate session summary in the background
    if history.len() >= 2 {
        tauri::async_runtime::spawn(async move {
            let database_operation_guard =
                crate::ai::context::acquire_database_operation_read_guard().await;
            if !database_operation_guard.is_current() {
                tracing::info!(
                    target: "context",
                    "discarding session summary started before a database restore"
                );
                return;
            }
            let transcript = history
                .iter()
                .filter(|m| crate::ai::context::is_summary_candidate_message(m))
                .map(|m| format!("{}: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n");
            let language_rule = if summary_language.trim().is_empty() {
                "Write the summary in the language the users were speaking.".to_string()
            } else {
                let language = summary_language.trim();
                format!(
                    "Write the summary in {language}. If the conversation uses another language, translate or summarize it into {language}."
                )
            };

            let messages = vec![
                system_message(format!(
                    "You are a conversation summarizer. Write a brief 2-3 sentence summary of this conversation. \
                     {language_rule} Focus on key topics discussed, any emotional moments, and important \
                     information shared. Write from a third-person perspective.\n\
                     Output ONLY the summary, no labels or formatting."
                )),
                user_text_message(format!("Summarize this conversation:\n\n{}", transcript)),
            ];

            let client = build_openai_client(request.api_key, request.endpoint);
            let model = request.model.unwrap_or_else(|| "gpt-4".to_string());

            match create_chat(&client, &model, messages, None).await {
                Ok(summary) => {
                    let summary = summary.trim().to_string();
                    if !summary.is_empty() {
                        if !memory_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                            tracing::info!(target: "ai", "Skip saving summary because memory is disabled");
                            return;
                        }
                        if let Err(e) = memory_mgr
                            .save_session_summary_unlocked(&char_id, &summary)
                            .await
                        {
                            tracing::error!(target: "ai", "Failed to save summary: {}", e);
                        } else {
                            tracing::info!(
                                target: "ai",
                                "Saved summary for '{}': {}",
                                char_id,
                                &summary[..summary.len().min(80)]
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(target: "ai", "Summary generation failed: {}", e);
                }
            }
        });
    }

    Ok(())
}
