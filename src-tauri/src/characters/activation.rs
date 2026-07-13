// pattern: Imperative Shell

use crate::characters::manifest::{
    CharacterAssets, CharacterRecommendations, CharacterRuntimeProfile, CharacterTemplateManifest,
    CharacterTtsProfile,
};
use crate::error::KokoroError;
use crate::tts::config::TtsSystemConfig;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
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
    pub live2d_model: Option<String>,
    pub background: Option<String>,
    pub cue_profile: Option<String>,
    pub tts: ResolvedTts,
}

pub type ResolvedCharacterRuntime = BackendRuntimeSnapshot;

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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GreetingAction {
    None,
    Emit { content: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharacterActivationToken {
    pub revision: u64,
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
}

#[derive(Default)]
struct ActivationState {
    next_revision: u64,
    latest_prepared_revision: u64,
    committed_revision: u64,
}

#[derive(Default)]
pub struct ActivationCoordinator {
    state: Mutex<ActivationState>,
}

impl ActivationCoordinator {
    pub async fn prepare<B: ActivationRuntimeBackend>(
        &self,
        pool: &SqlitePool,
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
            system_prompt: persona.clone(),
            response_language: requested_runtime
                .response_language
                .unwrap_or_else(|| previous_committed.response_language.clone()),
            proactive_enabled: requested_runtime
                .proactive_enabled
                .unwrap_or(previous_committed.proactive_enabled),
            current_conversation_id: target_conversation_id.clone(),
            live2d_model: template_content.assets.live2d_model,
            background: template_content.assets.background,
            cue_profile: template_content.assets.cue_profile,
            tts: resolve_tts(requested_runtime.tts.as_ref(), tts_config, local_presets),
        };
        let greeting = row.try_get::<String, _>("greeting")?;
        let greeting_action = if row
            .try_get::<Option<i64>, _>("greeting_consumed_at")?
            .is_none()
            && !greeting.trim().is_empty()
        {
            GreetingAction::Emit { content: greeting }
        } else {
            GreetingAction::None
        };
        Ok(CharacterActivationToken {
            revision,
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
        })
    }

    pub async fn commit<B: ActivationRuntimeBackend>(
        &self,
        pool: &SqlitePool,
        token: CharacterActivationToken,
        backend: &B,
    ) -> Result<CommittedCharacterRuntime, KokoroError> {
        let mut state = self.state.lock().await;
        if token.revision != state.latest_prepared_revision
            || token.revision <= state.committed_revision
        {
            return Err(stale_token_error());
        }

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

        if let Err(error) = backend.apply(&applied_runtime).await {
            if let Err(restore_error) = backend.restore(&token.previous_committed).await {
                return Err(KokoroError::Internal(format!(
                    "failed to apply activation: {error}; failed to restore backend: {restore_error}"
                )));
            }
            return Err(error);
        }
        if let Err(error) = transaction.commit().await {
            let restore_result = backend.restore(&token.previous_committed).await;
            if let Err(restore_error) = restore_result {
                return Err(KokoroError::Internal(format!(
                    "failed to commit activation: {error}; failed to restore backend: {restore_error}"
                )));
            }
            return Err(error.into());
        }

        state.committed_revision = token.revision;
        Ok(committed)
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

#[derive(Default)]
struct ResolvedTemplateContent {
    assets: CharacterAssets,
    recommendations: CapabilityRecommendations,
}

fn resolve_template_content(
    source_format: &str,
    template_id: Option<&str>,
    template_version: Option<&str>,
    snapshot: Option<&str>,
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
    let recommendations: CharacterRecommendations = manifest.recommendations.unwrap_or_default();
    Ok(ResolvedTemplateContent {
        assets: manifest.assets.unwrap_or_default(),
        recommendations: CapabilityRecommendations {
            vision: recommendations.vision,
            memory: recommendations.memory,
            mcp_servers: recommendations.mcp_servers.unwrap_or_default(),
        },
    })
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
    let GreetingAction::Emit { content } = &token.greeting_action else {
        return Ok(());
    };
    let now = chrono::Utc::now();
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
