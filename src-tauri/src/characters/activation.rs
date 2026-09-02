// pattern: Imperative Shell

use crate::characters::manifest::{
    CharacterRecommendations, CharacterRuntimeProfile, CharacterTemplateManifest,
    CharacterTtsProfile,
};
use crate::error::KokoroError;
use crate::tts::config::TtsSystemConfig;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::fs;
use std::path::Path;
use tokio::sync::Mutex;
use uuid::Uuid;

const COMMITTED_RUNTIME_TABLE: &str = "character_activation_runtime";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BackendRuntimeSnapshot {
    pub character_id: String,
    pub character_name: String,
    pub user_name: String,
    pub system_prompt: String,
    pub response_language: String,
    pub proactive_enabled: bool,
    pub current_conversation_id: Option<String>,
    pub live2d_model: Option<PackageAssetReference>,
    pub background: Option<PackageAssetReference>,
    pub cue_profile: Option<PackageAssetReference>,
    pub tts: ResolvedTts,
}

pub type ResolvedCharacterRuntime = BackendRuntimeSnapshot;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum PackageAssetReference {
    Package {
        template_id: String,
        template_version: String,
        path: String,
    },
    Library {
        model_id: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedTtsMode {
    ConfiguredProvider,
    LocalPresetConfirmation,
    Browser,
    #[default]
    TextOnly,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTts {
    pub mode: ResolvedTtsMode,
    pub provider_id: Option<String>,
    pub provider_type: Option<String>,
    pub local_preset: Option<String>,
    pub endpoint: Option<String>,
    pub voice: Option<String>,
    pub speed: Option<f64>,
    pub pitch: Option<f64>,
    pub requires_save_confirmation: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptPayload {
    pub character_name: String,
    pub user_name: String,
    pub persona: String,
    pub example_dialogue: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRecommendations {
    pub vision: Option<bool>,
    pub memory: Option<bool>,
    pub mcp_servers: Vec<String>,
    pub bot_platforms: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GreetingAction {
    None,
    ConsumeWithoutEmit,
    Emit { content: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharacterActivationToken {
    pub revision: u64,
    pub nonce: String,
    pub character_updated_at: i64,
    pub previous_committed: BackendRuntimeSnapshot,
    pub resolved_runtime: ResolvedCharacterRuntime,
    pub prompt: PromptPayload,
    pub target_conversation_id: Option<String>,
    pub greeting_action: GreetingAction,
    pub recommendations: CapabilityRecommendations,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommittedCharacterRuntime {
    pub revision: u64,
    pub runtime: ResolvedCharacterRuntime,
    pub target_conversation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalTtsPreset {
    pub id: String,
    pub provider_type: String,
    pub endpoint: String,
}

#[async_trait]
pub trait ActivationRuntimeBackend: Send + Sync {
    async fn snapshot(&self) -> Result<BackendRuntimeSnapshot, KokoroError>;
    async fn apply(&self, snapshot: &BackendRuntimeSnapshot) -> Result<(), KokoroError>;
    async fn restore(&self, snapshot: &BackendRuntimeSnapshot) -> Result<(), KokoroError>;
    async fn sync_history(&self, conversation_id: Option<&str>) -> Result<(), KokoroError> {
        let _ = conversation_id;
        Ok(())
    }
    async fn set_degraded(&self, reason: Option<String>) {
        let _ = reason;
    }
    async fn clear_degraded(&self) {}
    async fn lock_activation(&self) -> Result<Box<dyn std::any::Any + Send>, KokoroError> {
        Ok(Box::new(()))
    }
}

#[derive(Clone)]
struct PreparedActivation {
    token: CharacterActivationToken,
}

#[derive(Default)]
struct ActivationState {
    next_revision: u64,
    latest_prepared_revision: u64,
    committed_revision: u64,
    prepared: Option<PreparedActivation>,
}

#[derive(Default)]
pub struct ActivationCoordinator {
    state: Mutex<ActivationState>,
}

impl ActivationCoordinator {
    #[cfg(test)]
    pub async fn prepare<B: ActivationRuntimeBackend>(
        &self,
        pool: &SqlitePool,
        character_id: &str,
        tts_config: &TtsSystemConfig,
        local_presets: &[LocalTtsPreset],
        backend: &B,
    ) -> Result<CharacterActivationToken, KokoroError> {
        self.prepare_internal(pool, None, character_id, tts_config, local_presets, backend)
            .await
    }

    pub async fn prepare_with_package_root<B: ActivationRuntimeBackend>(
        &self,
        pool: &SqlitePool,
        package_root: &Path,
        character_id: &str,
        tts_config: &TtsSystemConfig,
        local_presets: &[LocalTtsPreset],
        backend: &B,
    ) -> Result<CharacterActivationToken, KokoroError> {
        self.prepare_internal(
            pool,
            Some(package_root),
            character_id,
            tts_config,
            local_presets,
            backend,
        )
        .await
    }

    async fn prepare_internal<B: ActivationRuntimeBackend>(
        &self,
        pool: &SqlitePool,
        package_root: Option<&Path>,
        character_id: &str,
        tts_config: &TtsSystemConfig,
        local_presets: &[LocalTtsPreset],
        backend: &B,
    ) -> Result<CharacterActivationToken, KokoroError> {
        let character_id = character_id.trim();
        if character_id.is_empty() {
            return Err(KokoroError::Validation(
                "character id cannot be empty".to_string(),
            ));
        }

        // The single state mutex serializes prepare and commit revisions. No database write occurs
        // in prepare, so callers can safely abandon a token before frontend application.
        let mut state = self.state.lock().await;
        let persisted_committed = self.get_committed(pool).await?;
        if let Some(committed) = &persisted_committed {
            state.next_revision = state.next_revision.max(committed.revision);
            state.committed_revision = state.committed_revision.max(committed.revision);
        }
        state.next_revision = state.next_revision.checked_add(1).ok_or_else(|| {
            KokoroError::Internal("character activation revision exhausted".to_string())
        })?;
        let revision = state.next_revision;
        state.latest_prepared_revision = revision;

        let row = sqlx::query(
            "SELECT name, persona, user_nickname, source_format, updated_at, greeting, \
                    greeting_consumed_at, example_dialogue, runtime_profile_json, template_id, \
                    template_version, template_snapshot_json \
             FROM characters WHERE id = ?",
        )
        .bind(character_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| KokoroError::NotFound(format!("character '{character_id}' not found")))?;

        let requested_runtime: CharacterRuntimeProfile = serde_json::from_str(
            &row.try_get::<String, _>("runtime_profile_json")?,
        )
        .map_err(|error| {
            KokoroError::Validation(format!("invalid character runtime profile: {error}"))
        })?;
        let previous_committed = match persisted_committed {
            Some(committed) => committed.runtime,
            None => backend.snapshot().await?,
        };
        let character_name = row.try_get::<String, _>("name")?;
        let user_name = normalized_user_name(row.try_get::<String, _>("user_nickname")?);
        let persona = row.try_get::<String, _>("persona")?;
        let template_content = resolve_template_content(
            &row.try_get::<String, _>("source_format")?,
            row.try_get::<Option<String>, _>("template_id")?.as_deref(),
            row.try_get::<Option<String>, _>("template_version")?
                .as_deref(),
            row.try_get::<Option<String>, _>("template_snapshot_json")?
                .as_deref(),
            package_root,
        )?;
        let target_conversation_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM conversations WHERE character_id = ? ORDER BY updated_at DESC, id ASC LIMIT 1",
        )
        .bind(character_id)
        .fetch_optional(pool)
        .await?;
        let resolved_runtime = BackendRuntimeSnapshot {
            character_id: character_id.to_string(),
            character_name: character_name.clone(),
            user_name: user_name.clone(),
            system_prompt: compose_system_prompt(
                &persona,
                &row.try_get::<String, _>("example_dialogue")?,
            ),
            response_language: requested_runtime
                .response_language
                .unwrap_or_else(|| previous_committed.response_language.clone()),
            proactive_enabled: requested_runtime
                .proactive_enabled
                .unwrap_or(previous_committed.proactive_enabled),
            current_conversation_id: target_conversation_id.clone(),
            live2d_model: resolve_library_live2d_reference(
                package_root,
                requested_runtime.live2d_model.as_deref(),
            )
            .or(template_content.live2d_model),
            background: template_content.background,
            cue_profile: template_content.cue_profile,
            tts: resolve_tts(requested_runtime.tts.as_ref(), tts_config, local_presets),
        };
        let greeting = row.try_get::<String, _>("greeting")?;
        let greeting_action = match (
            row.try_get::<Option<i64>, _>("greeting_consumed_at")?,
            greeting.trim().is_empty(),
        ) {
            (None, true) => GreetingAction::ConsumeWithoutEmit,
            (None, false) => GreetingAction::Emit { content: greeting },
            (Some(_), _) => GreetingAction::None,
        };
        let token = CharacterActivationToken {
            revision,
            nonce: Uuid::new_v4().to_string(),
            character_updated_at: row.try_get("updated_at")?,
            previous_committed,
            resolved_runtime,
            prompt: PromptPayload {
                character_name,
                user_name,
                persona,
                example_dialogue: row.try_get("example_dialogue")?,
            },
            target_conversation_id,
            greeting_action,
            recommendations: template_content.recommendations,
        };
        state.prepared = Some(PreparedActivation {
            token: token.clone(),
        });
        Ok(token)
    }

    pub async fn commit<B: ActivationRuntimeBackend>(
        &self,
        pool: &SqlitePool,
        submitted_token: CharacterActivationToken,
        backend: &B,
    ) -> Result<CommittedCharacterRuntime, KokoroError> {
        let mut state = self.state.lock().await;
        if submitted_token.revision != state.latest_prepared_revision
            || submitted_token.revision <= state.committed_revision
        {
            return Err(stale_token_error());
        }
        let prepared = state.prepared.as_ref().ok_or_else(stale_token_error)?;
        if submitted_token.revision != prepared.token.revision
            || submitted_token.nonce != prepared.token.nonce
        {
            return Err(stale_token_error());
        }
        let prepared = state
            .prepared
            .take()
            .expect("validated prepared activation remains present");
        // Only the opaque nonce and revision cross back into the trust boundary. All runtime,
        // prompt, greeting, conversation, and rollback fields come from this server-owned copy.
        let token = prepared.token;

        let mut transaction = pool.begin().await?;
        let live_updated_at =
            sqlx::query_scalar::<_, i64>("SELECT updated_at FROM characters WHERE id = ?")
                .bind(&token.resolved_runtime.character_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| {
                    KokoroError::NotFound(format!(
                        "character '{}' not found",
                        token.resolved_runtime.character_id
                    ))
                })?;
        if live_updated_at != token.character_updated_at {
            return Err(stale_token_error());
        }

        let conversation_id = ensure_owned_conversation(&mut transaction, &token).await?;
        stage_greeting(&mut transaction, &token, &conversation_id).await?;
        ensure_committed_runtime_table(&mut transaction).await?;
        let mut applied_runtime = token.resolved_runtime.clone();
        applied_runtime.current_conversation_id = Some(conversation_id.clone());
        let committed = CommittedCharacterRuntime {
            revision: token.revision,
            runtime: applied_runtime.clone(),
            target_conversation_id: conversation_id,
        };
        let committed_json = serde_json::to_string(&committed).map_err(|error| {
            KokoroError::Internal(format!(
                "failed to serialize committed character runtime: {error}"
            ))
        })?;
        sqlx::query(
            "INSERT INTO character_activation_runtime (singleton, revision, runtime_json) \
             VALUES (1, ?, ?) ON CONFLICT(singleton) DO UPDATE SET revision = excluded.revision, runtime_json = excluded.runtime_json",
        )
        .bind(i64::try_from(token.revision).map_err(|_| {
            KokoroError::Internal("character activation revision exceeds SQLite range".into())
        })?)
        .bind(committed_json)
        .execute(&mut *transaction)
        .await?;

        let _activation_lock = backend.lock_activation().await?;

        if let Err(error) = backend.apply(&applied_runtime).await {
            if let Err(restore_error) = backend.restore(&token.previous_committed).await {
                let recover_res = self.recover_committed_backend(pool, backend).await;
                if let Err(recover_error) = recover_res {
                    let reason = format!(
                        "failed to apply activation: {error}; failed to restore backend: {restore_error}; failed to recover backend from DB: {recover_error}"
                    );
                    backend.set_degraded(Some(reason.clone())).await;
                    return Err(KokoroError::Internal(reason));
                }
                return Err(KokoroError::Internal(format!(
                    "failed to apply activation: {error}; failed to restore backend: {restore_error} (recovered from committed state)"
                )));
            }
            return Err(error);
        }
        if let Err(error) = transaction.commit().await {
            let restore_result = backend.restore(&token.previous_committed).await;
            if let Err(restore_error) = restore_result {
                let recover_res = self.recover_committed_backend(pool, backend).await;
                if let Err(recover_error) = recover_res {
                    let reason = format!(
                        "failed to commit activation: {error}; failed to restore backend: {restore_error}; failed to recover backend from DB: {recover_error}"
                    );
                    backend.set_degraded(Some(reason.clone())).await;
                    return Err(KokoroError::Internal(reason));
                }
                return Err(KokoroError::Internal(format!(
                    "failed to commit activation: {error}; failed to restore backend: {restore_error} (recovered from committed state)"
                )));
            }
            return Err(error.into());
        }

        if let Err(history_err) = backend
            .sync_history(applied_runtime.current_conversation_id.as_deref())
            .await
        {
            tracing::error!(
                "Failed to sync conversation history after character activation: {history_err}"
            );
            let rollback_db_result = self
                .rollback_committed_state(pool, &token, state.committed_revision)
                .await;

            if let Err(rollback_error) = rollback_db_result {
                tracing::error!(
                    "Failed to rollback committed state in SQLite after history sync failure: {rollback_error}. Re-aligning backend to committed SQLite state."
                );
                state.committed_revision = token.revision;
                let recover_result = self.recover_committed_backend(pool, backend).await;
                if let Err(recover_error) = recover_result {
                    let reason = format!(
                        "failed to sync history: {history_err}; failed to rollback committed state: {rollback_error}; failed to re-align backend: {recover_error}"
                    );
                    backend.set_degraded(Some(reason.clone())).await;
                    return Err(KokoroError::Internal(reason));
                }
                return Err(KokoroError::Internal(format!(
                    "failed to sync history: {history_err}; failed to rollback committed state: {rollback_error} (backend retained committed runtime)"
                )));
            }

            let restore_result = backend.restore(&token.previous_committed).await;
            if let Err(restore_error) = restore_result {
                tracing::error!(
                    "Failed to restore backend after DB rollback: {restore_error}. Re-aligning backend from committed SQLite state."
                );
                let recover_result = self.recover_committed_backend(pool, backend).await;
                if let Err(recover_error) = recover_result {
                    let reason = format!(
                        "failed to sync history: {history_err}; failed to restore backend: {restore_error}; failed to recover backend from DB: {recover_error}"
                    );
                    backend.set_degraded(Some(reason.clone())).await;
                    return Err(KokoroError::Internal(reason));
                }
                return Err(KokoroError::Internal(format!(
                    "failed to sync history: {history_err}; failed to restore backend: {restore_error}"
                )));
            }

            return Err(KokoroError::Internal(format!(
                "failed to sync conversation history: {history_err}"
            )));
        }

        state.committed_revision = token.revision;
        backend.clear_degraded().await;
        Ok(committed)
    }

    async fn rollback_committed_state(
        &self,
        pool: &SqlitePool,
        token: &CharacterActivationToken,
        previous_revision: u64,
    ) -> Result<(), KokoroError> {
        let mut transaction = pool.begin().await?;
        rollback_committed_runtime_table_in_tx(
            &mut transaction,
            &token.previous_committed,
            previous_revision,
        )
        .await?;
        revert_staged_greeting_in_tx(&mut transaction, token).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn recover_committed_backend<B: ActivationRuntimeBackend>(
        &self,
        pool: &SqlitePool,
        backend: &B,
    ) -> Result<(), KokoroError> {
        let Some(committed) = self.get_committed(pool).await? else {
            let clear_res = backend.sync_history(None).await;
            let reason =
                "no committed character runtime found in database during recovery".to_string();
            backend.set_degraded(Some(reason.clone())).await;
            if let Err(clear_err) = clear_res {
                return Err(KokoroError::Internal(format!(
                    "{reason}; failed to clear in-memory history: {clear_err}"
                )));
            }
            return Err(KokoroError::Internal(reason));
        };
        if let Err(apply_err) = backend.apply(&committed.runtime).await {
            let clear_res = backend.sync_history(None).await;
            let reason = format!("failed to apply committed runtime during recovery: {apply_err}");
            backend.set_degraded(Some(reason.clone())).await;
            if let Err(clear_err) = clear_res {
                return Err(KokoroError::Internal(format!(
                    "{reason}; failed to clear in-memory history: {clear_err}"
                )));
            }
            return Err(KokoroError::Internal(reason));
        }
        if let Err(sync_err) = backend
            .sync_history(committed.runtime.current_conversation_id.as_deref())
            .await
        {
            tracing::warn!(
                target: "activation",
                "Failed to sync conversation history during backend recovery: {sync_err}. Clearing in-memory history."
            );
            let clear_res = backend.sync_history(None).await;
            let reason =
                format!("failed to sync conversation history during backend recovery: {sync_err}");
            backend.set_degraded(Some(reason.clone())).await;
            if let Err(clear_err) = clear_res {
                return Err(KokoroError::Internal(format!(
                    "{reason}; also failed to clear in-memory history: {clear_err}"
                )));
            }
            return Err(KokoroError::Internal(reason));
        }
        backend.clear_degraded().await;
        Ok(())
    }

    pub async fn recover_committed<B: ActivationRuntimeBackend>(
        &self,
        pool: &SqlitePool,
        backend: &B,
    ) -> Result<Option<CommittedCharacterRuntime>, KokoroError> {
        let mut state = self.state.lock().await;
        let Some(committed) = self.get_committed(pool).await? else {
            return Ok(None);
        };
        let initial_snapshot = backend.snapshot().await?;
        let _activation_lock = backend.lock_activation().await?;
        if let Err(apply_err) = backend.apply(&committed.runtime).await {
            return Err(compensate_recovery_failure(
                backend,
                &initial_snapshot,
                "failed to apply committed runtime",
                apply_err,
            )
            .await);
        }
        if let Err(sync_err) = backend
            .sync_history(committed.runtime.current_conversation_id.as_deref())
            .await
        {
            return Err(compensate_recovery_failure(
                backend,
                &initial_snapshot,
                "failed to recover conversation history",
                sync_err,
            )
            .await);
        }
        backend.clear_degraded().await;
        state.next_revision = state.next_revision.max(committed.revision);
        state.committed_revision = state.committed_revision.max(committed.revision);
        Ok(Some(committed))
    }

    pub async fn get_committed(
        &self,
        pool: &SqlitePool,
    ) -> Result<Option<CommittedCharacterRuntime>, KokoroError> {
        let table_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind(COMMITTED_RUNTIME_TABLE)
        .fetch_one(pool)
        .await?
            > 0;
        if !table_exists {
            return Ok(None);
        }
        let json = sqlx::query_scalar::<_, String>(
            "SELECT runtime_json FROM character_activation_runtime WHERE singleton = 1",
        )
        .fetch_optional(pool)
        .await?;
        json.map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                KokoroError::Internal(format!("invalid committed character runtime: {error}"))
            })
        })
        .transpose()
    }
}

/// Encapsulates compensation during recovery failure to guarantee consistent state:
/// Restores initial snapshot, clears dirty in-memory history, marks degraded, and returns accurate diagnostics.
async fn compensate_recovery_failure<B: ActivationRuntimeBackend>(
    backend: &B,
    initial_snapshot: &BackendRuntimeSnapshot,
    context_prefix: &str,
    primary_error: KokoroError,
) -> KokoroError {
    let restore_res = backend.restore(initial_snapshot).await;
    let clear_res = backend.sync_history(None).await;
    let reason = format!("{context_prefix}: {primary_error}");

    backend.set_degraded(Some(reason.clone())).await;

    let mut compensation_details = Vec::new();
    match (restore_res, clear_res) {
        (Ok(()), Ok(())) => {
            compensation_details.push("cleared dirty history and rolled back runtime".to_string());
        }
        (Ok(()), Err(clear_err)) => {
            compensation_details.push(format!(
                "rolled back runtime; failed to clear dirty history: {clear_err}"
            ));
        }
        (Err(restore_err), Ok(())) => {
            compensation_details.push(format!(
                "failed to restore backend: {restore_err}; cleared dirty history"
            ));
        }
        (Err(restore_err), Err(clear_err)) => {
            compensation_details.push(format!(
                "failed to restore backend: {restore_err}; failed to clear dirty history: {clear_err}"
            ));
        }
    }

    KokoroError::Internal(format!("{reason} ({})", compensation_details.join("; ")))
}

fn normalized_user_name(name: String) -> String {
    if name.trim().is_empty() {
        "User".to_string()
    } else {
        name
    }
}

fn resolve_tts(
    requested: Option<&CharacterTtsProfile>,
    config: &TtsSystemConfig,
    local_presets: &[LocalTtsPreset],
) -> ResolvedTts {
    let requested = requested.cloned().unwrap_or_default();
    if requested.enabled == Some(false) {
        return ResolvedTts {
            voice: requested.voice,
            speed: requested.speed,
            pitch: requested.pitch,
            ..Default::default()
        };
    }
    let enabled = |provider: &&crate::tts::config::ProviderConfig| provider.enabled;
    let matching_id = requested.provider_id.as_deref().and_then(|id| {
        config.providers.iter().filter(enabled).find(|provider| {
            provider.id == id
                && requested
                    .provider_type
                    .as_deref()
                    .is_none_or(|kind| kind == provider.provider_type)
        })
    });
    let matching_type_default = requested.provider_type.as_deref().and_then(|kind| {
        let default_id = config.default_provider.as_deref()?;
        config
            .providers
            .iter()
            .filter(enabled)
            .find(|provider| provider.id == default_id && provider.provider_type == kind)
    });
    if let Some(provider) = matching_id.or(matching_type_default) {
        return ResolvedTts {
            mode: if provider.provider_type == "browser" {
                ResolvedTtsMode::Browser
            } else {
                ResolvedTtsMode::ConfiguredProvider
            },
            provider_id: Some(provider.id.clone()),
            provider_type: Some(provider.provider_type.clone()),
            voice: requested.voice.or_else(|| provider.default_voice.clone()),
            speed: requested.speed,
            pitch: requested.pitch,
            ..Default::default()
        };
    }

    if let Some(preset) = requested.local_preset.as_deref().and_then(|id| {
        local_presets.iter().find(|preset| {
            preset.id == id
                && requested
                    .provider_type
                    .as_deref()
                    .is_none_or(|kind| kind == preset.provider_type)
        })
    }) {
        return ResolvedTts {
            mode: ResolvedTtsMode::LocalPresetConfirmation,
            provider_type: Some(preset.provider_type.clone()),
            local_preset: Some(preset.id.clone()),
            endpoint: Some(preset.endpoint.clone()),
            voice: requested.voice,
            speed: requested.speed,
            pitch: requested.pitch,
            requires_save_confirmation: true,
            provider_id: None,
        };
    }

    if let Some(browser) = config
        .providers
        .iter()
        .find(|provider| provider.enabled && provider.provider_type == "browser")
    {
        return ResolvedTts {
            mode: ResolvedTtsMode::Browser,
            provider_id: Some(browser.id.clone()),
            provider_type: Some("browser".to_string()),
            voice: requested.voice,
            speed: requested.speed,
            pitch: requested.pitch,
            ..Default::default()
        };
    }
    ResolvedTts {
        voice: requested.voice,
        speed: requested.speed,
        pitch: requested.pitch,
        ..Default::default()
    }
}

fn resolve_library_live2d_reference(
    package_root: Option<&Path>,
    model_id: Option<&str>,
) -> Option<PackageAssetReference> {
    let (Some(package_root), Some(model_id)) = (package_root, model_id) else {
        return None;
    };
    let relative = Path::new(model_id);
    if model_id.trim().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            !matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return None;
    }
    let models_root = package_root.parent()?.join("live2d_models");
    let metadata = fs::symlink_metadata(&models_root).ok()?;
    if asset_metadata_is_redirected(&metadata) || !metadata.is_dir() {
        return None;
    }
    let canonical_root = models_root.canonicalize().ok()?;
    if canonical_root != models_root {
        return None;
    }
    let expected = models_root.join(relative);
    let metadata = fs::symlink_metadata(&expected).ok()?;
    if asset_metadata_is_redirected(&metadata) || !metadata.is_file() {
        return None;
    }
    let canonical = expected.canonicalize().ok()?;
    if !canonical.starts_with(&canonical_root) {
        return None;
    }
    Some(PackageAssetReference::Library {
        model_id: model_id.to_string(),
    })
}

#[derive(Default)]
struct ResolvedTemplateContent {
    live2d_model: Option<PackageAssetReference>,
    background: Option<PackageAssetReference>,
    cue_profile: Option<PackageAssetReference>,
    recommendations: CapabilityRecommendations,
}

fn resolve_template_content(
    source_format: &str,
    template_id: Option<&str>,
    template_version: Option<&str>,
    snapshot: Option<&str>,
    package_root: Option<&Path>,
) -> Result<ResolvedTemplateContent, KokoroError> {
    if source_format != "template" {
        return Ok(ResolvedTemplateContent::default());
    }
    let (Some(template_id), Some(template_version), Some(snapshot)) =
        (template_id, template_version, snapshot)
    else {
        return Ok(ResolvedTemplateContent::default());
    };
    let manifest = CharacterTemplateManifest::from_json(snapshot).map_err(|error| {
        KokoroError::Validation(format!("invalid character template snapshot: {error}"))
    })?;
    if manifest.id != template_id || manifest.version != template_version {
        return Err(KokoroError::Validation(
            "character template snapshot does not match instance origin".to_string(),
        ));
    }
    let recommendations: CharacterRecommendations =
        manifest.recommendations.clone().unwrap_or_default();
    let assets = manifest.assets.clone().unwrap_or_default();
    let package_dir = package_root.and_then(|root| {
        resolve_owned_package_directory(root, &manifest)
            .map_err(|error| {
                tracing::warn!(
                    target: "characters",
                    template_id,
                    template_version,
                    "character activation asset fallback: {}",
                    error
                );
                error
            })
            .ok()
    });
    Ok(ResolvedTemplateContent {
        live2d_model: resolve_package_asset(
            package_dir.as_deref(),
            template_id,
            template_version,
            assets.live2d_model.as_deref(),
            PackageAssetRole::Live2dModel,
        ),
        background: resolve_package_asset(
            package_dir.as_deref(),
            template_id,
            template_version,
            assets.background.as_deref(),
            PackageAssetRole::Background,
        ),
        cue_profile: resolve_package_asset(
            package_dir.as_deref(),
            template_id,
            template_version,
            assets.cue_profile.as_deref(),
            PackageAssetRole::CueProfile,
        ),
        recommendations: sanitize_recommendations(&recommendations),
    })
}

fn sanitize_recommendations(
    recommendations: &CharacterRecommendations,
) -> CapabilityRecommendations {
    const BOT_PLATFORM_ALLOWLIST: &[&str] = &["telegram", "qq", "discord", "line", "webhook"];

    fn normalized_unique(values: Option<&Vec<String>>, allowlist: Option<&[&str]>) -> Vec<String> {
        let mut normalized = Vec::new();
        for value in values.into_iter().flatten() {
            let value = value.trim();
            if value.is_empty()
                || allowlist.is_some_and(|allowed| !allowed.contains(&value))
                || normalized.iter().any(|current| current == value)
            {
                continue;
            }
            normalized.push(value.to_string());
        }
        normalized
    }

    CapabilityRecommendations {
        vision: recommendations.vision,
        memory: recommendations.memory,
        mcp_servers: normalized_unique(recommendations.mcp_servers.as_ref(), None),
        bot_platforms: normalized_unique(
            recommendations.bot_platforms.as_ref(),
            Some(BOT_PLATFORM_ALLOWLIST),
        ),
    }
}

#[derive(Clone, Copy)]
enum PackageAssetRole {
    Live2dModel,
    Background,
    CueProfile,
}

fn resolve_owned_package_directory(
    package_root: &Path,
    expected_manifest: &CharacterTemplateManifest,
) -> Result<std::path::PathBuf, KokoroError> {
    let canonical_root = package_root.canonicalize().map_err(|error| {
        KokoroError::Validation(format!("character package root is unavailable: {error}"))
    })?;
    let expected_id_dir = canonical_root.join(&expected_manifest.id);
    reject_redirected_directory(&expected_id_dir, "character package id directory")?;
    let canonical_id_dir = expected_id_dir.canonicalize().map_err(|error| {
        KokoroError::Validation(format!(
            "character package id directory is unavailable: {error}"
        ))
    })?;
    if canonical_id_dir != expected_id_dir || canonical_id_dir.parent() != Some(&canonical_root) {
        return Err(KokoroError::Validation(
            "character package id directory is not directly owned by the catalog".to_string(),
        ));
    }

    let expected_package_dir = canonical_id_dir.join(&expected_manifest.version);
    reject_redirected_directory(&expected_package_dir, "character package version directory")?;
    let canonical_package_dir = expected_package_dir.canonicalize().map_err(|error| {
        KokoroError::Validation(format!("character package version is unavailable: {error}"))
    })?;
    if canonical_package_dir != expected_package_dir
        || canonical_package_dir.parent() != Some(&canonical_id_dir)
    {
        return Err(KokoroError::Validation(
            "character package version is not directly owned by its template".to_string(),
        ));
    }

    let installed_raw =
        fs::read_to_string(canonical_package_dir.join("character.json")).map_err(|error| {
            KokoroError::Validation(format!(
                "installed character manifest is unavailable: {error}"
            ))
        })?;
    let installed = CharacterTemplateManifest::from_json(&installed_raw).map_err(|error| {
        KokoroError::Validation(format!("installed character manifest is invalid: {error}"))
    })?;
    if &installed != expected_manifest {
        return Err(KokoroError::Validation(
            "installed character package does not match the instance template snapshot".to_string(),
        ));
    }
    Ok(canonical_package_dir)
}

fn reject_redirected_directory(path: &Path, label: &str) -> Result<(), KokoroError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| KokoroError::Validation(format!("{label} is unavailable: {error}")))?;
    if asset_metadata_is_redirected(&metadata) || !metadata.is_dir() {
        return Err(KokoroError::Validation(format!(
            "{label} must be a non-redirected directory"
        )));
    }
    Ok(())
}

fn resolve_package_asset(
    package_dir: Option<&Path>,
    template_id: &str,
    template_version: &str,
    relative: Option<&str>,
    role: PackageAssetRole,
) -> Option<PackageAssetReference> {
    let (Some(package_dir), Some(relative)) = (package_dir, relative) else {
        return None;
    };
    let expected = package_dir.join(relative);
    let metadata = fs::symlink_metadata(&expected).ok()?;
    if asset_metadata_is_redirected(&metadata) || !metadata.is_file() {
        return None;
    }
    let canonical = expected.canonicalize().ok()?;
    if !canonical.starts_with(package_dir) {
        return None;
    }
    if matches!(role, PackageAssetRole::CueProfile) && !is_valid_cue_profile(&canonical) {
        return None;
    }
    Some(PackageAssetReference::Package {
        template_id: template_id.to_string(),
        template_version: template_version.to_string(),
        path: canonical.to_string_lossy().into_owned(),
    })
}

fn is_valid_cue_profile(path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
        && value.get("cues").is_some_and(serde_json::Value::is_object)
}

#[cfg(not(windows))]
fn asset_metadata_is_redirected(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn asset_metadata_is_redirected(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn compose_system_prompt(persona: &str, example_dialogue: &str) -> String {
    let persona = persona.trim();
    let example_dialogue = example_dialogue.trim();
    if example_dialogue.is_empty() {
        return format!("<character_persona>\n{persona}\n</character_persona>");
    }
    format!(
        "<character_persona>\n{persona}\n</character_persona>\n\n\
         <example_dialogue purpose=\"style_reference_only\">\n{example_dialogue}\n</example_dialogue>"
    )
}

async fn ensure_owned_conversation(
    transaction: &mut Transaction<'_, Sqlite>,
    token: &CharacterActivationToken,
) -> Result<String, KokoroError> {
    if let Some(id) = token.target_conversation_id.as_deref() {
        let owner =
            sqlx::query_scalar::<_, String>("SELECT character_id FROM conversations WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut **transaction)
                .await?;
        if owner.as_deref() != Some(token.resolved_runtime.character_id.as_str()) {
            return Err(stale_token_error());
        }
        return Ok(id.to_string());
    }

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at) \
         VALUES (?, ?, '新对话', '', '{}', ?, ?)",
    )
    .bind(&id)
    .bind(&token.resolved_runtime.character_id)
    .bind(&now)
    .bind(&now)
    .execute(&mut **transaction)
    .await?;
    Ok(id)
}

async fn stage_greeting(
    transaction: &mut Transaction<'_, Sqlite>,
    token: &CharacterActivationToken,
    conversation_id: &str,
) -> Result<(), KokoroError> {
    let now = chrono::Utc::now();
    if token.greeting_action == GreetingAction::ConsumeWithoutEmit {
        let updated = sqlx::query(
            "UPDATE characters SET greeting_consumed_at = ?, greeting_message_id = NULL \
             WHERE id = ? AND greeting_consumed_at IS NULL",
        )
        .bind(now.timestamp())
        .bind(&token.resolved_runtime.character_id)
        .execute(&mut **transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(stale_token_error());
        }
        return Ok(());
    }
    let GreetingAction::Emit { content } = &token.greeting_action else {
        return Ok(());
    };
    let inserted = sqlx::query(
        "INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) \
         SELECT ?, 'assistant', ?, NULL, ? WHERE EXISTS (\
             SELECT 1 FROM characters WHERE id = ? AND greeting_consumed_at IS NULL\
         )",
    )
    .bind(conversation_id)
    .bind(content)
    .bind(now.to_rfc3339())
    .bind(&token.resolved_runtime.character_id)
    .execute(&mut **transaction)
    .await?;
    if inserted.rows_affected() != 1 {
        return Err(stale_token_error());
    }
    let message_id = inserted.last_insert_rowid();
    let updated = sqlx::query(
        "UPDATE characters SET greeting_consumed_at = ?, greeting_message_id = ? \
         WHERE id = ? AND greeting_consumed_at IS NULL",
    )
    .bind(now.timestamp())
    .bind(message_id)
    .bind(&token.resolved_runtime.character_id)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(stale_token_error());
    }
    Ok(())
}

async fn rollback_committed_runtime_table_in_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    previous_committed: &BackendRuntimeSnapshot,
    previous_revision: u64,
) -> Result<(), KokoroError> {
    if previous_committed.character_id.is_empty() {
        sqlx::query("DELETE FROM character_activation_runtime WHERE singleton = 1")
            .execute(&mut **transaction)
            .await?;
    } else {
        let previous_record = CommittedCharacterRuntime {
            revision: previous_revision,
            runtime: previous_committed.clone(),
            target_conversation_id: previous_committed
                .current_conversation_id
                .clone()
                .unwrap_or_default(),
        };
        let previous_json = serde_json::to_string(&previous_record).map_err(|error| {
            KokoroError::Internal(format!(
                "failed to serialize previous committed character runtime: {error}"
            ))
        })?;
        sqlx::query(
            "INSERT INTO character_activation_runtime (singleton, revision, runtime_json) \
             VALUES (1, ?, ?) ON CONFLICT(singleton) DO UPDATE SET revision = excluded.revision, runtime_json = excluded.runtime_json",
        )
        .bind(i64::try_from(previous_revision).map_err(|_| {
            KokoroError::Internal("character activation revision exceeds SQLite range".into())
        })?)
        .bind(previous_json)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn revert_staged_greeting_in_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    token: &CharacterActivationToken,
) -> Result<(), KokoroError> {
    match &token.greeting_action {
        GreetingAction::None => Ok(()),
        GreetingAction::ConsumeWithoutEmit => {
            sqlx::query(
                "UPDATE characters SET greeting_consumed_at = NULL, greeting_message_id = NULL \
                 WHERE id = ?",
            )
            .bind(&token.resolved_runtime.character_id)
            .execute(&mut **transaction)
            .await?;
            Ok(())
        }
        GreetingAction::Emit { .. } => {
            let message_id = sqlx::query_scalar::<_, Option<i64>>(
                "SELECT greeting_message_id FROM characters WHERE id = ?",
            )
            .bind(&token.resolved_runtime.character_id)
            .fetch_optional(&mut **transaction)
            .await?
            .flatten();

            if let Some(msg_id) = message_id {
                sqlx::query("DELETE FROM conversation_messages WHERE id = ?")
                    .bind(msg_id)
                    .execute(&mut **transaction)
                    .await?;
            }
            sqlx::query(
                "UPDATE characters SET greeting_consumed_at = NULL, greeting_message_id = NULL \
                 WHERE id = ?",
            )
            .bind(&token.resolved_runtime.character_id)
            .execute(&mut **transaction)
            .await?;
            Ok(())
        }
    }
}

async fn ensure_committed_runtime_table(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<(), KokoroError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS character_activation_runtime (\
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),\
            revision INTEGER NOT NULL,\
            runtime_json TEXT NOT NULL\
         )",
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn stale_token_error() -> KokoroError {
    KokoroError::Validation("stale activation token; prepare activation again".to_string())
}
