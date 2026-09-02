// pattern: Imperative Shell

use crate::ai::context::AIOrchestrator;
use crate::characters::activation::{
    ActivationCoordinator, ActivationRuntimeBackend, BackendRuntimeSnapshot,
    CharacterActivationToken, CommittedCharacterRuntime, LocalTtsPreset,
};
use crate::characters::catalog::{CatalogEntry, CharacterCatalog, DOCUMENTATION_TEMPLATE_ID};
use crate::characters::instance_resource::{
    instance_avatar_reference, parse_instance_avatar_reference, validate_avatar_bytes,
    validate_instance_id,
};
use crate::characters::manifest::CharacterTemplateManifest;
use crate::error::KokoroError;
use async_trait::async_trait;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

struct OrchestratorActivationBackend<'a> {
    orchestrator: &'a AIOrchestrator,
    app_data: PathBuf,
}

#[async_trait]
impl ActivationRuntimeBackend for OrchestratorActivationBackend<'_> {
    async fn snapshot(&self) -> Result<BackendRuntimeSnapshot, KokoroError> {
        let character_id = self.orchestrator.get_character_id().await;
        let identity = sqlx::query_as::<_, (String, String)>(
            "SELECT name, user_nickname FROM characters WHERE id = ?",
        )
        .bind(&character_id)
        .fetch_optional(&self.orchestrator.db)
        .await?;
        let (character_name, user_name) = identity
            .map(|(name, user)| {
                let user = if user.trim().is_empty() {
                    "User".to_string()
                } else {
                    user
                };
                (name, user)
            })
            .unwrap_or_else(|| ("Kokoro".to_string(), "User".to_string()));
        Ok(BackendRuntimeSnapshot {
            character_id,
            character_name,
            user_name,
            system_prompt: self.orchestrator.system_prompt.lock().await.clone(),
            response_language: self.orchestrator.response_language.lock().await.clone(),
            proactive_enabled: self.orchestrator.is_proactive_enabled(),
            current_conversation_id: self
                .orchestrator
                .current_conversation_id
                .lock()
                .await
                .clone(),
            ..Default::default()
        })
    }

    async fn apply(&self, snapshot: &BackendRuntimeSnapshot) -> Result<(), KokoroError> {
        apply_orchestrator_runtime(self.orchestrator, snapshot, &self.app_data).await
    }

    async fn restore(&self, snapshot: &BackendRuntimeSnapshot) -> Result<(), KokoroError> {
        apply_orchestrator_runtime(self.orchestrator, snapshot, &self.app_data).await?;
        sync_orchestrator_history(
            self.orchestrator,
            snapshot.current_conversation_id.as_deref(),
        )
        .await?;
        if !snapshot.character_id.is_empty() {
            self.orchestrator.clear_runtime_degraded().await;
        }
        Ok(())
    }

    async fn sync_history(&self, conversation_id: Option<&str>) -> Result<(), KokoroError> {
        sync_orchestrator_history(self.orchestrator, conversation_id).await
    }

    async fn set_degraded(&self, reason: Option<String>) {
        self.orchestrator.set_runtime_degraded(reason).await;
    }

    async fn clear_degraded(&self) {
        self.orchestrator.clear_runtime_degraded().await;
    }

    async fn lock_activation(&self) -> Result<Box<dyn std::any::Any + Send>, KokoroError> {
        let guard = self.orchestrator.acquire_activation_lock().await;
        Ok(Box::new(guard))
    }
}

async fn apply_orchestrator_runtime(
    orchestrator: &AIOrchestrator,
    snapshot: &BackendRuntimeSnapshot,
    app_data: &Path,
) -> Result<(), KokoroError> {
    persist_runtime_backend_snapshot(app_data, snapshot)?;
    orchestrator
        .set_system_prompt(snapshot.system_prompt.clone())
        .await;
    orchestrator
        .set_character_name(snapshot.character_name.clone())
        .await;
    orchestrator.set_user_name(snapshot.user_name.clone()).await;
    orchestrator
        .set_character_id(snapshot.character_id.clone())
        .await;
    orchestrator
        .set_response_language(snapshot.response_language.clone())
        .await;
    orchestrator.set_proactive_enabled(snapshot.proactive_enabled);
    *orchestrator.current_conversation_id.lock().await = snapshot.current_conversation_id.clone();
    Ok(())
}

async fn sync_orchestrator_history(
    orchestrator: &AIOrchestrator,
    conversation_id: Option<&str>,
) -> Result<(), KokoroError> {
    let new_messages = if let Some(conv_id) = conversation_id {
        let rows = match sqlx::query_as::<_, (String, String, Option<String>, String)>(
            "SELECT role, content, metadata, created_at FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conv_id)
        .fetch_all(&orchestrator.db)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                orchestrator.reset_history_and_boundary().await;
                return Err(KokoroError::Database(format!(
                    "failed to load conversation messages: {e}"
                )));
            }
        };

        let mut msgs = Vec::with_capacity(rows.len());
        for (role, content, metadata, _) in rows {
            msgs.push(crate::ai::context::Message {
                role,
                content,
                metadata: metadata
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
            });
        }
        msgs
    } else {
        Vec::new()
    };

    orchestrator.reset_history_and_boundary().await;
    let count = new_messages.len();
    {
        let mut history = orchestrator.history.lock().await;
        for msg in new_messages {
            history.push_back(msg);
        }
    }
    orchestrator.set_memory_history_boundary(count).await;
    Ok(())
}

fn persist_runtime_backend_snapshot(
    app_data: &Path,
    snapshot: &BackendRuntimeSnapshot,
) -> Result<(), KokoroError> {
    fs::create_dir_all(app_data).map_err(|error| {
        KokoroError::Internal(format!("failed to create app data directory: {error}"))
    })?;
    replace_json_file(
        app_data,
        "active_character_id.json",
        &serde_json::json!({ "character_id": snapshot.character_id }),
    )?;
    replace_json_file(
        app_data,
        "current_conversation_id.json",
        &serde_json::json!({ "conversation_id": snapshot.current_conversation_id }),
    )?;
    let complete = serde_json::to_value(snapshot).map_err(|error| {
        KokoroError::Internal(format!(
            "failed to serialize backend runtime snapshot: {error}"
        ))
    })?;
    replace_json_file(app_data, "character_runtime_backend.json", &complete)
}

fn replace_json_file(
    directory: &Path,
    file_name: &str,
    value: &serde_json::Value,
) -> Result<(), KokoroError> {
    let target = directory.join(file_name);
    let temporary = directory.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let backup = directory.join(format!(".{file_name}.{}.backup", Uuid::new_v4()));
    let bytes = serde_json::to_vec(value).map_err(|error| {
        KokoroError::Internal(format!("failed to serialize {file_name}: {error}"))
    })?;
    fs::write(&temporary, bytes)
        .map_err(|error| KokoroError::Internal(format!("failed to write {file_name}: {error}")))?;
    if !target.exists() {
        return fs::rename(&temporary, &target).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            KokoroError::Internal(format!("failed to persist {file_name}: {error}"))
        });
    }

    fs::rename(&target, &backup).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        KokoroError::Internal(format!("failed to stage existing {file_name}: {error}"))
    })?;
    if let Err(error) = fs::rename(&temporary, &target) {
        let _ = fs::rename(&backup, &target);
        let _ = fs::remove_file(&temporary);
        return Err(KokoroError::Internal(format!(
            "failed to persist {file_name}: {error}"
        )));
    }
    fs::remove_file(&backup)
        .map_err(|error| KokoroError::Internal(format!("failed to finalize {file_name}: {error}")))
}

fn allowlisted_local_tts_presets() -> Vec<LocalTtsPreset> {
    vec![LocalTtsPreset {
        id: "gpt-sovits-loopback".to_string(),
        provider_type: "gpt_sovits".to_string(),
        endpoint: "http://127.0.0.1:9880".to_string(),
    }]
}

use super::character_instance_core::{
    build_reconcile_preview, create_request_from_manifest, parse_template_defaults,
    validate_create_request, validate_duplicate_request, validate_update_request,
};
pub use super::character_instance_core::{
    ApplyCharacterTemplateReconciliationRequest, CharacterRecord, CreateCharacterRequest,
    DuplicateCharacterRequest, InstantiateCharacterTemplateRequest,
    ReconcileCharacterTemplatePreview, ReconcileCharacterTemplateRequest,
    RestoreCharacterDefaultsRequest, UpdateCharacterRequest,
};

const CHARACTER_COLUMNS: &str =
    "id, name, persona, user_nickname, source_format, created_at, updated_at, \
     template_id, template_version, template_snapshot_json, description, avatar_path, \
     greeting, greeting_consumed_at, greeting_message_id, example_dialogue, \
     runtime_profile_json, user_modified_at";

#[tauri::command]
pub async fn list_characters(
    orchestrator: State<'_, AIOrchestrator>,
) -> Result<Vec<CharacterRecord>, KokoroError> {
    list_characters_from_pool(&orchestrator.db).await
}

#[tauri::command]
pub async fn prepare_character_activation(
    character_id: String,
    allow_local_preset: Option<bool>,
    coordinator: State<'_, ActivationCoordinator>,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<CharacterActivationToken, KokoroError> {
    let app_data = resolve_app_data(&app)?;
    let tts_config = crate::tts::config::load_config(&app_data.join("tts_config.json"));
    let backend = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data: app_data.clone(),
    };
    let local_presets = allowlisted_local_tts_presets();
    let allowed_presets = if allow_local_preset.unwrap_or(true) {
        local_presets.as_slice()
    } else {
        &[]
    };
    coordinator
        .prepare_with_package_root(
            &orchestrator.db,
            &app_data.join("characters"),
            &character_id,
            &tts_config,
            allowed_presets,
            &backend,
        )
        .await
}

#[tauri::command]
pub async fn commit_character_activation(
    token: CharacterActivationToken,
    coordinator: State<'_, ActivationCoordinator>,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<CommittedCharacterRuntime, KokoroError> {
    let app_data = resolve_app_data(&app)?;
    let backend = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data,
    };
    coordinator.commit(&orchestrator.db, token, &backend).await
}

#[tauri::command]
pub async fn get_committed_character_runtime(
    coordinator: State<'_, ActivationCoordinator>,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<Option<CommittedCharacterRuntime>, KokoroError> {
    let backend = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data: resolve_app_data(&app)?,
    };
    coordinator
        .recover_committed(&orchestrator.db, &backend)
        .await
}

pub(crate) async fn recover_committed_character_runtime_for_startup(
    coordinator: &ActivationCoordinator,
    orchestrator: &AIOrchestrator,
    app_data: &Path,
) -> Result<Option<CommittedCharacterRuntime>, KokoroError> {
    let backend = OrchestratorActivationBackend {
        orchestrator,
        app_data: app_data.to_path_buf(),
    };
    coordinator
        .recover_committed(&orchestrator.db, &backend)
        .await
}

pub(crate) async fn activate_character_for_package_removal(
    coordinator: &ActivationCoordinator,
    orchestrator: &AIOrchestrator,
    app_data: &Path,
    character_id: &str,
) -> Result<CommittedCharacterRuntime, KokoroError> {
    let backend = OrchestratorActivationBackend {
        orchestrator,
        app_data: app_data.to_path_buf(),
    };
    let tts_config = crate::tts::config::load_config(&app_data.join("tts_config.json"));
    let token = coordinator
        .prepare_with_package_root(
            &orchestrator.db,
            &app_data.join("characters"),
            character_id,
            &tts_config,
            &allowlisted_local_tts_presets(),
            &backend,
        )
        .await?;
    coordinator.commit(&orchestrator.db, token, &backend).await
}

#[tauri::command]
pub async fn create_character(
    request: CreateCharacterRequest,
    orchestrator: State<'_, AIOrchestrator>,
) -> Result<(), KokoroError> {
    create_character_in_pool(&orchestrator.db, request).await
}

#[tauri::command]
pub async fn create_character_with_avatar(
    request: CreateCharacterRequest,
    avatar_bytes: Vec<u8>,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<(), KokoroError> {
    let app_data = resolve_app_data(&app)?;
    create_character_with_avatar_in_pool(&orchestrator.db, &app_data, request, &avatar_bytes).await
}

#[tauri::command]
pub async fn update_character_with_avatar(
    request: UpdateCharacterRequest,
    avatar_bytes: Vec<u8>,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<(), KokoroError> {
    let app_data = resolve_app_data(&app)?;
    update_character_with_avatar_in_pool(&orchestrator.db, &app_data, request, &avatar_bytes).await
}

#[tauri::command]
pub async fn update_character(
    request: UpdateCharacterRequest,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<(), KokoroError> {
    let app_data = resolve_app_data(&app)?;
    update_character_with_resources_in_pool(&orchestrator.db, &app_data, request).await
}

#[tauri::command]
pub async fn delete_character(
    id: String,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<(), KokoroError> {
    let app_data = resolve_app_data(&app)?;
    delete_character_with_resources_in_pool(&orchestrator.db, &app_data, &id).await
}

#[tauri::command]
pub async fn duplicate_character(
    request: DuplicateCharacterRequest,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<(), KokoroError> {
    let app_data = resolve_app_data(&app)?;
    duplicate_character_with_resources_in_pool(&orchestrator.db, &app_data, request).await
}

#[tauri::command]
pub async fn restore_character_defaults(
    request: RestoreCharacterDefaultsRequest,
    orchestrator: State<'_, AIOrchestrator>,
    app: AppHandle,
) -> Result<(), KokoroError> {
    let app_data = resolve_app_data(&app)?;
    restore_character_defaults_with_resources_in_pool(&orchestrator.db, &app_data, request).await
}

#[tauri::command]
pub fn list_character_templates(
    catalog: State<'_, CharacterCatalog>,
) -> Result<Vec<CharacterTemplateManifest>, KokoroError> {
    list_character_templates_from_catalog(&catalog)
}

#[tauri::command]
pub async fn instantiate_character_template(
    request: InstantiateCharacterTemplateRequest,
    orchestrator: State<'_, AIOrchestrator>,
    catalog: State<'_, CharacterCatalog>,
) -> Result<(), KokoroError> {
    instantiate_character_template_in_pool(&orchestrator.db, &catalog, request).await
}

#[tauri::command]
pub async fn reconcile_character_template(
    request: ReconcileCharacterTemplateRequest,
    orchestrator: State<'_, AIOrchestrator>,
    catalog: State<'_, CharacterCatalog>,
) -> Result<ReconcileCharacterTemplatePreview, KokoroError> {
    reconcile_character_template_from_pool(&orchestrator.db, &catalog, request).await
}

#[tauri::command]
pub async fn apply_character_template_reconciliation(
    request: ApplyCharacterTemplateReconciliationRequest,
    orchestrator: State<'_, AIOrchestrator>,
    catalog: State<'_, CharacterCatalog>,
    app: AppHandle,
) -> Result<(), KokoroError> {
    let app_data = resolve_app_data(&app)?;
    apply_character_template_reconciliation_with_resources_in_pool(
        &orchestrator.db,
        &catalog,
        &app_data,
        request,
    )
    .await
}

pub(crate) fn list_character_templates_from_catalog(
    catalog: &CharacterCatalog,
) -> Result<Vec<CharacterTemplateManifest>, KokoroError> {
    Ok(discover_catalog(catalog)?
        .into_iter()
        .filter(|entry| entry.manifest.id != DOCUMENTATION_TEMPLATE_ID)
        .map(|entry| entry.manifest)
        .collect())
}

pub(crate) async fn instantiate_character_template_in_pool(
    pool: &SqlitePool,
    catalog: &CharacterCatalog,
    request: InstantiateCharacterTemplateRequest,
) -> Result<(), KokoroError> {
    let entry = find_catalog_entry(catalog, &request.template_id, &request.template_version)?;
    let create_request =
        create_request_from_manifest(&entry.manifest, &request).map_err(KokoroError::Validation)?;
    create_character_in_pool(pool, create_request).await
}

pub(crate) async fn reconcile_character_template_from_pool(
    pool: &SqlitePool,
    catalog: &CharacterCatalog,
    request: ReconcileCharacterTemplateRequest,
) -> Result<ReconcileCharacterTemplatePreview, KokoroError> {
    if request.instance_id.trim().is_empty() || request.template_version.trim().is_empty() {
        return Err(KokoroError::Validation(
            "instance id and template version cannot be empty".to_string(),
        ));
    }
    let character = find_character_from_pool(pool, &request.instance_id)
        .await?
        .ok_or_else(|| {
            KokoroError::NotFound(format!("character '{}' not found", request.instance_id))
        })?;
    let template_id = character.template_id.as_deref().ok_or_else(|| {
        KokoroError::Validation("character is not linked to a template".to_string())
    })?;
    let entry = find_catalog_entry(catalog, template_id, &request.template_version)?;
    build_reconcile_preview(&character, &entry.manifest).map_err(KokoroError::Validation)
}

pub(crate) async fn apply_character_template_reconciliation_in_pool(
    pool: &SqlitePool,
    catalog: &CharacterCatalog,
    request: ApplyCharacterTemplateReconciliationRequest,
) -> Result<(), KokoroError> {
    for (label, value) in [
        ("instance id", request.instance_id.as_str()),
        (
            "expected current template version",
            request.expected_current_template_version.as_str(),
        ),
        (
            "expected new template version",
            request.expected_new_template_version.as_str(),
        ),
        ("selected character name", request.selected.name.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(KokoroError::Validation(format!("{label} cannot be empty")));
        }
    }

    let current = find_character_from_pool(pool, &request.instance_id)
        .await?
        .ok_or_else(|| {
            KokoroError::NotFound(format!("character '{}' not found", request.instance_id))
        })?;
    let template_id = current.template_id.as_deref().ok_or_else(|| {
        KokoroError::Validation("character is not linked to a template".to_string())
    })?;
    let entry = find_catalog_entry(catalog, template_id, &request.expected_new_template_version)?;
    let snapshot = serde_json::to_string(&entry.manifest).map_err(|error| {
        KokoroError::Internal(format!("failed to serialize template snapshot: {error}"))
    })?;
    let runtime = serde_json::to_string(&request.selected.runtime.clone().unwrap_or_default())
        .map_err(|error| {
            KokoroError::Validation(format!("failed to serialize selected runtime: {error}"))
        })?;

    let mut transaction = pool.begin().await?;
    let live_version: Option<String> = sqlx::query_scalar(
        "SELECT template_version FROM characters WHERE id = ? AND template_id = ?",
    )
    .bind(&request.instance_id)
    .bind(template_id)
    .fetch_optional(&mut *transaction)
    .await?
    .flatten();
    if live_version.as_deref() != Some(request.expected_current_template_version.as_str()) {
        return Err(KokoroError::Validation(
            "character template version changed; preview reconciliation again".to_string(),
        ));
    }

    let result = sqlx::query(
        "UPDATE characters SET name = ?, description = ?, avatar_path = ?, persona = ?, \
         greeting = ?, example_dialogue = ?, runtime_profile_json = ?, template_version = ?, \
         template_snapshot_json = ?, updated_at = ? WHERE id = ? AND template_version = ?",
    )
    .bind(request.selected.name)
    .bind(request.selected.description)
    .bind(request.selected.avatar)
    .bind(request.selected.persona)
    .bind(request.selected.greeting)
    .bind(request.selected.example_dialogue.unwrap_or_default())
    .bind(runtime)
    .bind(&request.expected_new_template_version)
    .bind(snapshot)
    .bind(request.updated_at)
    .bind(&request.instance_id)
    .bind(&request.expected_current_template_version)
    .execute(&mut *transaction)
    .await?;
    require_affected_character(result.rows_affected(), &request.instance_id)?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) async fn apply_character_template_reconciliation_with_resources_in_pool(
    pool: &SqlitePool,
    catalog: &CharacterCatalog,
    app_data: &Path,
    request: ApplyCharacterTemplateReconciliationRequest,
) -> Result<(), KokoroError> {
    let current = find_character_from_pool(pool, &request.instance_id)
        .await?
        .ok_or_else(|| {
            KokoroError::NotFound(format!("character '{}' not found", request.instance_id))
        })?;
    let mut removal =
        stage_owned_avatar_if_replaced(app_data, &current, request.selected.avatar.as_deref())?;
    if let Err(error) =
        apply_character_template_reconciliation_in_pool(pool, catalog, request).await
    {
        return Err(recover_managed_avatar_failure(&mut removal, error, Ok(())));
    }
    removal.finalize()?;
    Ok(())
}

pub(crate) async fn list_characters_from_pool(
    pool: &SqlitePool,
) -> Result<Vec<CharacterRecord>, KokoroError> {
    let query = format!("SELECT {CHARACTER_COLUMNS} FROM characters ORDER BY created_at ASC");
    let rows = sqlx::query(&query).fetch_all(pool).await?;
    rows.into_iter()
        .map(character_record_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(crate) async fn create_character_in_pool(
    pool: &SqlitePool,
    request: CreateCharacterRequest,
) -> Result<(), KokoroError> {
    validate_create_request(&request).map_err(KokoroError::Validation)?;
    sqlx::query(
        "INSERT INTO characters (\
            id, name, persona, user_nickname, source_format, created_at, updated_at, \
            template_id, template_version, template_snapshot_json, description, avatar_path, \
            greeting, example_dialogue, runtime_profile_json, user_modified_at\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&request.id)
    .bind(&request.name)
    .bind(&request.persona)
    .bind(&request.user_nickname)
    .bind(&request.source_format)
    .bind(request.created_at)
    .bind(request.updated_at)
    .bind(&request.template_id)
    .bind(&request.template_version)
    .bind(&request.template_snapshot_json)
    .bind(&request.description)
    .bind(&request.avatar_path)
    .bind(&request.greeting)
    .bind(&request.example_dialogue)
    .bind(&request.runtime_profile_json)
    .bind(request.user_modified_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn create_character_with_avatar_in_pool(
    pool: &SqlitePool,
    app_data: &Path,
    mut request: CreateCharacterRequest,
    avatar_bytes: &[u8],
) -> Result<(), KokoroError> {
    validate_create_request(&request).map_err(KokoroError::Validation)?;
    validate_instance_id(&request.id)
        .map_err(|error| KokoroError::Validation(error.to_string()))?;
    validate_avatar_bytes(avatar_bytes)
        .map_err(|error| KokoroError::Validation(error.to_string()))?;

    let character_id = request.id.clone();
    let resources_root = ensure_managed_avatar_root(app_data)?;
    let resource_directory = resources_root.join(&character_id);
    match fs::symlink_metadata(&resource_directory) {
        Ok(metadata) => {
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                return Err(KokoroError::Validation(
                    "character avatar resource directory is unsafe".to_string(),
                ));
            }
            return Err(KokoroError::Validation(format!(
                "managed avatar resource already exists for character '{}'",
                character_id
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    fs::create_dir(&resource_directory)?;

    let persisted = persist_character_avatar(app_data, &request.id, avatar_bytes);
    let avatar_reference = match persisted {
        Ok(reference) => reference,
        Err(error) => {
            let cleanup = remove_managed_avatar_directory(app_data, &character_id);
            return Err(recover_cleanup_failure(
                KokoroError::Validation(error),
                cleanup,
            ));
        }
    };
    request.avatar_path = Some(avatar_reference);
    if let Err(error) = create_character_in_pool(pool, request).await {
        let cleanup = remove_managed_avatar_directory(app_data, &character_id);
        return Err(recover_cleanup_failure(error, cleanup));
    }
    Ok(())
}

pub(crate) async fn update_character_in_pool(
    pool: &SqlitePool,
    request: UpdateCharacterRequest,
) -> Result<(), KokoroError> {
    validate_update_request(&request).map_err(KokoroError::Validation)?;
    let has_avatar_patch = request.avatar_path.is_some();
    let avatar_path = request.avatar_path.clone().flatten();
    let user_modified_at = request.user_modified_at.unwrap_or(request.updated_at);
    let result = sqlx::query(
        "UPDATE characters SET \
            name = ?, persona = ?, user_nickname = ?, source_format = ?, updated_at = ?, \
            description = COALESCE(?, description), \
            avatar_path = CASE WHEN ? THEN ? ELSE avatar_path END, \
            greeting = COALESCE(?, greeting), \
            example_dialogue = COALESCE(?, example_dialogue), \
            runtime_profile_json = COALESCE(?, runtime_profile_json), \
            user_modified_at = ? \
         WHERE id = ?",
    )
    .bind(&request.name)
    .bind(&request.persona)
    .bind(&request.user_nickname)
    .bind(&request.source_format)
    .bind(request.updated_at)
    .bind(&request.description)
    .bind(has_avatar_patch)
    .bind(avatar_path)
    .bind(&request.greeting)
    .bind(&request.example_dialogue)
    .bind(&request.runtime_profile_json)
    .bind(user_modified_at)
    .bind(&request.id)
    .execute(pool)
    .await?;
    require_affected_character(result.rows_affected(), &request.id)
}

pub(crate) async fn update_character_with_avatar_in_pool(
    pool: &SqlitePool,
    app_data: &Path,
    mut request: UpdateCharacterRequest,
    avatar_bytes: &[u8],
) -> Result<(), KokoroError> {
    validate_update_request(&request).map_err(KokoroError::Validation)?;
    validate_instance_id(&request.id)
        .map_err(|error| KokoroError::Validation(error.to_string()))?;
    validate_avatar_bytes(avatar_bytes)
        .map_err(|error| KokoroError::Validation(error.to_string()))?;

    let current = find_character_from_pool(pool, &request.id)
        .await?
        .ok_or_else(|| KokoroError::NotFound(format!("character '{}' not found", request.id)))?;
    let owns_current_avatar = current
        .avatar_path
        .as_deref()
        .and_then(parse_instance_avatar_reference)
        == Some(current.id.as_str());
    let mut removal = if owns_current_avatar {
        ManagedAvatarRemoval::stage(app_data, &current.id)?
    } else {
        ManagedAvatarRemoval::empty()
    };

    let avatar_reference = match persist_character_avatar(app_data, &request.id, avatar_bytes) {
        Ok(reference) => reference,
        Err(error) => {
            let cleanup = remove_managed_avatar_directory(app_data, &request.id);
            return Err(recover_managed_avatar_failure(
                &mut removal,
                KokoroError::Validation(error),
                cleanup,
            ));
        }
    };
    request.avatar_path = Some(Some(avatar_reference));
    if let Err(error) = update_character_in_pool(pool, request).await {
        let cleanup = remove_managed_avatar_directory(app_data, &current.id);
        return Err(recover_managed_avatar_failure(&mut removal, error, cleanup));
    }
    removal.finalize()?;
    Ok(())
}

pub(crate) async fn update_character_with_resources_in_pool(
    pool: &SqlitePool,
    app_data: &Path,
    request: UpdateCharacterRequest,
) -> Result<(), KokoroError> {
    let current = find_character_from_pool(pool, &request.id)
        .await?
        .ok_or_else(|| KokoroError::NotFound(format!("character '{}' not found", request.id)))?;
    let mut removal = match request.avatar_path.as_ref() {
        Some(next_avatar) => {
            stage_owned_avatar_if_replaced(app_data, &current, next_avatar.as_deref())?
        }
        None => ManagedAvatarRemoval::empty(),
    };
    if let Err(error) = update_character_in_pool(pool, request).await {
        return Err(recover_managed_avatar_failure(&mut removal, error, Ok(())));
    }
    removal.finalize()?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn delete_character_in_pool(
    pool: &SqlitePool,
    id: &str,
) -> Result<(), KokoroError> {
    if id.trim().is_empty() {
        return Err(KokoroError::Validation(
            "character id cannot be empty".to_string(),
        ));
    }
    sqlx::query("DELETE FROM characters WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn delete_character_with_resources_in_pool(
    pool: &SqlitePool,
    app_data: &Path,
    id: &str,
) -> Result<(), KokoroError> {
    if id.trim().is_empty() {
        return Err(KokoroError::Validation(
            "character id cannot be empty".to_string(),
        ));
    }
    let character = find_character_from_pool(pool, id).await?;
    let owned_avatar = character
        .as_ref()
        .and_then(|record| record.avatar_path.as_deref())
        .and_then(parse_instance_avatar_reference)
        .filter(|resource_id| *resource_id == id);
    let mut removal = if owned_avatar.is_some() {
        ManagedAvatarRemoval::stage(app_data, id)?
    } else {
        ManagedAvatarRemoval::empty()
    };

    let mut transaction = pool.begin().await?;
    let delete_result = sqlx::query("DELETE FROM characters WHERE id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await;
    if let Err(error) = delete_result {
        drop(transaction);
        return Err(recover_managed_avatar_failure(
            &mut removal,
            error.into(),
            Ok(()),
        ));
    }
    if let Err(error) = transaction.commit().await {
        return Err(recover_managed_avatar_failure(
            &mut removal,
            error.into(),
            Ok(()),
        ));
    }
    removal.finalize()?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn duplicate_character_in_pool(
    pool: &SqlitePool,
    request: DuplicateCharacterRequest,
) -> Result<(), KokoroError> {
    validate_duplicate_request(&request).map_err(KokoroError::Validation)?;
    let source = find_character_from_pool(pool, &request.source_id)
        .await?
        .ok_or_else(|| {
            KokoroError::NotFound(format!("character '{}' not found", request.source_id))
        })?;
    create_character_in_pool(pool, duplicate_create_request(source, request)).await
}

pub(crate) async fn duplicate_character_with_resources_in_pool(
    pool: &SqlitePool,
    app_data: &Path,
    request: DuplicateCharacterRequest,
) -> Result<(), KokoroError> {
    validate_duplicate_request(&request).map_err(KokoroError::Validation)?;
    let source = find_character_from_pool(pool, &request.source_id)
        .await?
        .ok_or_else(|| {
            KokoroError::NotFound(format!("character '{}' not found", request.source_id))
        })?;
    let owns_avatar = source
        .avatar_path
        .as_deref()
        .and_then(parse_instance_avatar_reference)
        == Some(source.id.as_str());
    if owns_avatar {
        let avatar_bytes = read_managed_avatar(app_data, &source.id)?;
        let create_request = duplicate_create_request(source, request);
        create_character_with_avatar_in_pool(pool, app_data, create_request, &avatar_bytes).await
    } else {
        create_character_in_pool(pool, duplicate_create_request(source, request)).await
    }
}

pub(crate) async fn restore_character_defaults_in_pool(
    pool: &SqlitePool,
    request: RestoreCharacterDefaultsRequest,
) -> Result<(), KokoroError> {
    if request.id.trim().is_empty() {
        return Err(KokoroError::Validation(
            "character id cannot be empty".to_string(),
        ));
    }
    let character = find_character_from_pool(pool, &request.id)
        .await?
        .ok_or_else(|| KokoroError::NotFound(format!("character '{}' not found", request.id)))?;
    let defaults = parse_template_defaults(character.template_snapshot_json.as_deref())
        .map_err(KokoroError::Validation)?;
    let result = sqlx::query(
        "UPDATE characters SET \
            name = ?, description = ?, avatar_path = ?, persona = ?, greeting = ?, \
            example_dialogue = ?, runtime_profile_json = ?, user_modified_at = NULL, updated_at = ? \
         WHERE id = ?",
    )
    .bind(defaults.name)
    .bind(defaults.description)
    .bind(defaults.avatar_path)
    .bind(defaults.persona)
    .bind(defaults.greeting)
    .bind(defaults.example_dialogue)
    .bind(defaults.runtime_profile_json)
    .bind(request.updated_at)
    .bind(&request.id)
    .execute(pool)
    .await?;
    require_affected_character(result.rows_affected(), &request.id)
}

pub(crate) async fn restore_character_defaults_with_resources_in_pool(
    pool: &SqlitePool,
    app_data: &Path,
    request: RestoreCharacterDefaultsRequest,
) -> Result<(), KokoroError> {
    let current = find_character_from_pool(pool, &request.id)
        .await?
        .ok_or_else(|| KokoroError::NotFound(format!("character '{}' not found", request.id)))?;
    let defaults = parse_template_defaults(current.template_snapshot_json.as_deref())
        .map_err(KokoroError::Validation)?;
    let mut removal =
        stage_owned_avatar_if_replaced(app_data, &current, defaults.avatar_path.as_deref())?;
    if let Err(error) = restore_character_defaults_in_pool(pool, request).await {
        return Err(recover_managed_avatar_failure(&mut removal, error, Ok(())));
    }
    removal.finalize()?;
    Ok(())
}

fn duplicate_create_request(
    source: CharacterRecord,
    request: DuplicateCharacterRequest,
) -> CreateCharacterRequest {
    let name = request
        .new_name
        .unwrap_or_else(|| format!("{} Copy", source.name));
    CreateCharacterRequest {
        id: request.new_id,
        name,
        persona: source.persona,
        user_nickname: source.user_nickname,
        source_format: source.source_format,
        created_at: request.created_at,
        updated_at: request.updated_at,
        template_id: source.template_id,
        template_version: source.template_version,
        template_snapshot_json: source.template_snapshot_json,
        description: source.description,
        avatar_path: source.avatar_path,
        greeting: source.greeting,
        example_dialogue: source.example_dialogue,
        runtime_profile_json: source.runtime_profile_json,
        user_modified_at: source.user_modified_at,
    }
}

#[cfg(windows)]
fn is_filesystem_redirect(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_filesystem_redirect(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn inspect_managed_avatar_root(app_data: &Path) -> Result<Option<PathBuf>, KokoroError> {
    let root = app_data.join("character-instance-resources");
    match fs::symlink_metadata(&root) {
        Ok(metadata) => {
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                return Err(KokoroError::Validation(
                    "managed character avatar resource root is unsafe".to_string(),
                ));
            }
            Ok(Some(root))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn ensure_managed_avatar_root(app_data: &Path) -> Result<PathBuf, KokoroError> {
    let root = app_data.join("character-instance-resources");
    if inspect_managed_avatar_root(app_data)?.is_none() {
        fs::create_dir_all(&root)?;
    }
    inspect_managed_avatar_root(app_data)?.ok_or_else(|| {
        KokoroError::Internal("managed character avatar resource root disappeared".to_string())
    })
}

fn recover_cleanup_failure(
    operation_error: KokoroError,
    cleanup_result: Result<(), KokoroError>,
) -> KokoroError {
    match cleanup_result {
        Ok(()) => operation_error,
        Err(cleanup_error) => KokoroError::Internal(format!(
            "avatar operation failed: {operation_error}; cleanup also failed: {cleanup_error}"
        )),
    }
}

fn recover_managed_avatar_failure(
    removal: &mut ManagedAvatarRemoval,
    operation_error: KokoroError,
    cleanup_result: Result<(), KokoroError>,
) -> KokoroError {
    let cleanup_error = cleanup_result.err();
    let restore_error = removal.rollback().err();
    if cleanup_error.is_none() && restore_error.is_none() {
        return operation_error;
    }

    KokoroError::Internal(format!(
        "avatar operation failed: {operation_error}; cleanup: {}; restoration: {}",
        cleanup_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "ok".to_string()),
        restore_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "ok".to_string()),
    ))
}

fn remove_managed_avatar_directory(app_data: &Path, instance_id: &str) -> Result<(), KokoroError> {
    validate_instance_id(instance_id)
        .map_err(|error| KokoroError::Validation(error.to_string()))?;
    let Some(resources_root) = inspect_managed_avatar_root(app_data)? else {
        return Ok(());
    };
    let directory = resources_root.join(instance_id);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) => {
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                return Err(KokoroError::Validation(
                    "character avatar resource directory is unsafe".to_string(),
                ));
            }
            fs::remove_dir_all(&directory)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn stage_owned_avatar_if_replaced(
    app_data: &Path,
    current: &CharacterRecord,
    next_avatar: Option<&str>,
) -> Result<ManagedAvatarRemoval, KokoroError> {
    let owns_current_avatar = current
        .avatar_path
        .as_deref()
        .and_then(parse_instance_avatar_reference)
        == Some(current.id.as_str());
    if owns_current_avatar && current.avatar_path.as_deref() != next_avatar {
        ManagedAvatarRemoval::stage(app_data, &current.id)
    } else {
        Ok(ManagedAvatarRemoval::empty())
    }
}

fn read_managed_avatar(app_data: &Path, instance_id: &str) -> Result<Vec<u8>, KokoroError> {
    validate_instance_id(instance_id)
        .map_err(|error| KokoroError::Validation(error.to_string()))?;
    let resources_root = inspect_managed_avatar_root(app_data)?.ok_or_else(|| {
        KokoroError::NotFound("managed avatar resource directory not found".to_string())
    })?;
    let directory = resources_root.join(instance_id);
    let directory_metadata = fs::symlink_metadata(&directory)?;
    if is_filesystem_redirect(&directory_metadata) || !directory_metadata.is_dir() {
        return Err(KokoroError::Validation(
            "character avatar resource directory is unsafe".to_string(),
        ));
    }
    let avatar = directory.join("avatar.png");
    let avatar_metadata = fs::symlink_metadata(&avatar)?;
    if is_filesystem_redirect(&avatar_metadata) || !avatar_metadata.is_file() {
        return Err(KokoroError::Validation(
            "character avatar resource file is unsafe".to_string(),
        ));
    }
    let bytes = fs::read(avatar)?;
    validate_avatar_bytes(&bytes).map_err(|error| KokoroError::Validation(error.to_string()))?;
    Ok(bytes)
}

struct ManagedAvatarRemoval {
    original: Option<std::path::PathBuf>,
    backup: Option<std::path::PathBuf>,
    is_armed: bool,
}

impl ManagedAvatarRemoval {
    fn empty() -> Self {
        Self {
            original: None,
            backup: None,
            is_armed: false,
        }
    }

    fn stage(app_data: &Path, instance_id: &str) -> Result<Self, KokoroError> {
        validate_instance_id(instance_id)
            .map_err(|error| KokoroError::Validation(error.to_string()))?;
        let Some(resources_root) = inspect_managed_avatar_root(app_data)? else {
            return Ok(Self::empty());
        };
        let original = resources_root.join(instance_id);
        match fs::symlink_metadata(&original) {
            Ok(metadata) => {
                if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                    return Err(KokoroError::Validation(
                        "character avatar resource directory is unsafe".to_string(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::empty());
            }
            Err(error) => return Err(error.into()),
        }
        let backup =
            original.with_file_name(format!(".{instance_id}.delete-backup-{}", Uuid::new_v4()));
        fs::rename(&original, &backup)?;
        Ok(Self {
            original: Some(original),
            backup: Some(backup),
            is_armed: true,
        })
    }

    fn finalize(&mut self) -> Result<(), KokoroError> {
        self.is_armed = false;
        if let Some(backup) = self.backup.take() {
            fs::remove_dir_all(&backup).map_err(|error| {
                KokoroError::Io(format!(
                    "failed to remove deleted character resource backup '{}': {error}",
                    backup.display()
                ))
            })?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), KokoroError> {
        if !self.is_armed {
            return Ok(());
        }
        let original = self.original.as_ref().ok_or_else(|| {
            KokoroError::Internal("managed avatar rollback has no original path".to_string())
        })?;
        let backup = self.backup.as_ref().ok_or_else(|| {
            KokoroError::Internal("managed avatar rollback has no backup path".to_string())
        })?;
        fs::rename(backup, original).map_err(|error| {
            KokoroError::Io(format!(
                "failed to restore managed avatar resource '{}': {error}",
                original.display()
            ))
        })?;
        self.is_armed = false;
        self.backup = None;
        Ok(())
    }
}

impl Drop for ManagedAvatarRemoval {
    fn drop(&mut self) {
        if !self.is_armed {
            return;
        }
        if let Err(error) = self.rollback() {
            tracing::error!(
                target: "characters",
                "failed to rollback managed avatar resource removal: {error}"
            );
        }
    }
}

async fn find_character_from_pool(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<CharacterRecord>, KokoroError> {
    let query = format!("SELECT {CHARACTER_COLUMNS} FROM characters WHERE id = ?");
    let row = sqlx::query(&query).bind(id).fetch_optional(pool).await?;
    row.map(character_record_from_row)
        .transpose()
        .map_err(Into::into)
}

fn character_record_from_row(row: SqliteRow) -> Result<CharacterRecord, sqlx::Error> {
    Ok(CharacterRecord {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        persona: row.try_get("persona")?,
        user_nickname: row.try_get("user_nickname")?,
        source_format: row.try_get("source_format")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        template_id: row.try_get("template_id")?,
        template_version: row.try_get("template_version")?,
        template_snapshot_json: row.try_get("template_snapshot_json")?,
        description: row.try_get("description")?,
        avatar_path: row.try_get("avatar_path")?,
        greeting: row.try_get("greeting")?,
        greeting_consumed_at: row.try_get("greeting_consumed_at")?,
        greeting_message_id: row.try_get("greeting_message_id")?,
        example_dialogue: row.try_get("example_dialogue")?,
        runtime_profile_json: row.try_get("runtime_profile_json")?,
        user_modified_at: row.try_get("user_modified_at")?,
    })
}

fn require_affected_character(rows_affected: u64, id: &str) -> Result<(), KokoroError> {
    if rows_affected == 0 {
        Err(KokoroError::NotFound(format!("character '{id}' not found")))
    } else {
        Ok(())
    }
}

fn discover_catalog(catalog: &CharacterCatalog) -> Result<Vec<CatalogEntry>, KokoroError> {
    catalog.discover().map_err(|error| {
        KokoroError::Validation(format!("failed to read character catalog: {error}"))
    })
}

fn find_catalog_entry(
    catalog: &CharacterCatalog,
    template_id: &str,
    template_version: &str,
) -> Result<CatalogEntry, KokoroError> {
    discover_catalog(catalog)?
        .into_iter()
        .find(|entry| {
            entry.manifest.id == template_id && entry.manifest.version == template_version
        })
        .ok_or_else(|| {
            KokoroError::NotFound(format!(
                "character template '{template_id}' version '{template_version}' not found"
            ))
        })
}

pub(crate) fn persist_character_avatar(
    app_data: &Path,
    instance_id: &str,
    bytes: &[u8],
) -> Result<String, String> {
    validate_instance_id(instance_id).map_err(|error| error.to_string())?;
    validate_avatar_bytes(bytes).map_err(|error| error.to_string())?;
    let root = ensure_managed_avatar_root(app_data).map_err(|error| error.to_string())?;
    let directory = root.join(instance_id);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) => {
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                return Err("character avatar resource directory is unsafe".to_string());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let metadata = fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                return Err("character avatar resource directory is unsafe".to_string());
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    let target = directory.join("avatar.png");
    let temporary = directory.join(format!(".avatar-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    let target_exists = match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if is_filesystem_redirect(&metadata) || !metadata.is_file() {
                return Err("character avatar resource file is unsafe".to_string());
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.to_string()),
    };
    if target_exists {
        let backup = directory.join(format!(".avatar-{}.backup", Uuid::new_v4()));
        fs::rename(&target, &backup).map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&temporary, &target) {
            let restore_error = fs::rename(&backup, &target).err();
            let cleanup_error = fs::remove_file(&temporary).err();
            return Err(format!(
                "failed to replace character avatar: {error}; restoration: {}; cleanup: {}",
                restore_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "ok".to_string()),
                cleanup_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "ok".to_string()),
            ));
        }
        fs::remove_file(backup).map_err(|error| error.to_string())?;
    } else {
        fs::rename(&temporary, &target).map_err(|error| error.to_string())?;
    }
    instance_avatar_reference(instance_id).map_err(|error| error.to_string())
}

fn resolve_app_data(app: &AppHandle) -> Result<std::path::PathBuf, KokoroError> {
    app.path()
        .app_data_dir()
        .map_err(|error| KokoroError::Internal(format!("failed to resolve app data: {error}")))
}

#[cfg(test)]
#[path = "characters_tests.rs"]
mod tests;
