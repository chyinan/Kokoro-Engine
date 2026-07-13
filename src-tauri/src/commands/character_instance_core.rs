// pattern: Functional Core

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::characters::manifest::{CharacterRuntimeProfile, CharacterTemplateManifest};
use crate::characters::merge::{merge_character_template, CharacterTemplateFields};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterRecord {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub user_nickname: String,
    pub source_format: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub template_id: Option<String>,
    pub template_version: Option<String>,
    pub template_snapshot_json: Option<String>,
    pub description: String,
    pub avatar_path: Option<String>,
    pub greeting: String,
    pub greeting_consumed_at: Option<i64>,
    pub greeting_message_id: Option<i64>,
    pub example_dialogue: String,
    pub runtime_profile_json: String,
    pub user_modified_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateCharacterRequest {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub user_nickname: String,
    pub source_format: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub template_id: Option<String>,
    pub template_version: Option<String>,
    pub template_snapshot_json: Option<String>,
    #[serde(default)]
    pub description: String,
    pub avatar_path: Option<String>,
    #[serde(default)]
    pub greeting: String,
    #[serde(default)]
    pub example_dialogue: String,
    #[serde(default = "default_runtime_profile_json")]
    pub runtime_profile_json: String,
    pub user_modified_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCharacterRequest {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub user_nickname: String,
    pub source_format: String,
    pub updated_at: i64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub avatar_path: Option<Option<String>>,
    #[serde(default)]
    pub greeting: Option<String>,
    #[serde(default)]
    pub example_dialogue: Option<String>,
    #[serde(default)]
    pub runtime_profile_json: Option<String>,
    #[serde(default)]
    pub user_modified_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DuplicateCharacterRequest {
    pub source_id: String,
    pub new_id: String,
    pub new_name: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RestoreCharacterDefaultsRequest {
    pub id: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstantiateCharacterTemplateRequest {
    pub template_id: String,
    pub template_version: String,
    pub instance_id: String,
    pub user_nickname: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReconcileCharacterTemplateRequest {
    pub instance_id: String,
    pub template_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApplyCharacterTemplateReconciliationRequest {
    pub instance_id: String,
    pub expected_current_template_version: String,
    pub expected_new_template_version: String,
    pub selected: CharacterTemplateFields,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconcileConflict {
    pub field: String,
    pub old_value: Value,
    pub user_value: Value,
    pub new_value: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconcileCharacterTemplatePreview {
    pub instance_id: String,
    pub template_id: String,
    pub current_template_version: String,
    pub available_template_version: String,
    pub merged: CharacterTemplateFields,
    pub conflicts: Vec<ReconcileConflict>,
}

#[derive(Debug, Clone)]
pub struct TemplateDefaults {
    pub name: String,
    pub description: String,
    pub avatar_path: Option<String>,
    pub persona: String,
    pub greeting: String,
    pub example_dialogue: String,
    pub runtime_profile_json: String,
}

#[derive(Debug, Deserialize)]
struct TemplateSnapshot {
    name: String,
    description: String,
    #[serde(default)]
    avatar: Option<String>,
    persona: String,
    greeting: String,
    #[serde(default)]
    example_dialogue: Option<String>,
    #[serde(default)]
    runtime: Option<Value>,
}

pub fn validate_create_request(request: &CreateCharacterRequest) -> Result<(), String> {
    validate_required_text("character id", &request.id)?;
    validate_required_text("character name", &request.name)?;
    validate_template_origin(
        request.template_id.as_deref(),
        request.template_version.as_deref(),
        request.template_snapshot_json.as_deref(),
    )?;
    validate_json_object("runtime profile", &request.runtime_profile_json)?;
    Ok(())
}

pub fn validate_update_request(request: &UpdateCharacterRequest) -> Result<(), String> {
    validate_required_text("character id", &request.id)?;
    validate_required_text("character name", &request.name)?;
    if let Some(runtime_profile_json) = request.runtime_profile_json.as_deref() {
        validate_json_object("runtime profile", runtime_profile_json)?;
    }
    Ok(())
}

pub fn validate_duplicate_request(request: &DuplicateCharacterRequest) -> Result<(), String> {
    validate_required_text("source character id", &request.source_id)?;
    validate_required_text("new character id", &request.new_id)?;
    if let Some(name) = request.new_name.as_deref() {
        validate_required_text("new character name", name)?;
    }
    Ok(())
}

pub fn create_request_from_manifest(
    manifest: &CharacterTemplateManifest,
    request: &InstantiateCharacterTemplateRequest,
) -> Result<CreateCharacterRequest, String> {
    validate_required_text("template id", &request.template_id)?;
    validate_required_text("template version", &request.template_version)?;
    validate_required_text("instance id", &request.instance_id)?;
    if manifest.id != request.template_id || manifest.version != request.template_version {
        return Err("selected template does not match the requested id and version".to_string());
    }
    let template_snapshot_json = serde_json::to_string(manifest)
        .map_err(|error| format!("failed to serialize template snapshot: {error}"))?;
    let runtime_profile_json = serde_json::to_string(&manifest.runtime.clone().unwrap_or_default())
        .map_err(|error| format!("failed to serialize template runtime: {error}"))?;

    Ok(CreateCharacterRequest {
        id: request.instance_id.clone(),
        name: manifest.name.clone(),
        persona: manifest.persona.clone(),
        user_nickname: request.user_nickname.clone(),
        source_format: "template".to_string(),
        created_at: request.created_at,
        updated_at: request.updated_at,
        template_id: Some(manifest.id.clone()),
        template_version: Some(manifest.version.clone()),
        template_snapshot_json: Some(template_snapshot_json),
        description: manifest.description.clone(),
        avatar_path: manifest.avatar.clone(),
        greeting: manifest.greeting.clone(),
        example_dialogue: manifest.example_dialogue.clone().unwrap_or_default(),
        runtime_profile_json,
        user_modified_at: None,
    })
}

pub fn build_reconcile_preview(
    character: &CharacterRecord,
    new_manifest: &CharacterTemplateManifest,
) -> Result<ReconcileCharacterTemplatePreview, String> {
    let template_id = character
        .template_id
        .as_deref()
        .ok_or_else(|| "character is not linked to a template".to_string())?;
    let current_template_version = character
        .template_version
        .as_deref()
        .ok_or_else(|| "character has no template version".to_string())?;
    if template_id != new_manifest.id {
        return Err("new template id does not match the character origin".to_string());
    }
    let old_snapshot = character
        .template_snapshot_json
        .as_deref()
        .ok_or_else(|| "character has no template snapshot".to_string())?;
    let old_manifest = CharacterTemplateManifest::from_json(old_snapshot)
        .map_err(|error| format!("invalid stored template snapshot: {error}"))?;
    let old_fields = CharacterTemplateFields::from(&old_manifest);
    let runtime: CharacterRuntimeProfile = serde_json::from_str(&character.runtime_profile_json)
        .map_err(|error| format!("invalid character runtime profile: {error}"))?;
    let user_example_dialogue =
        if old_fields.example_dialogue.is_none() && character.example_dialogue.is_empty() {
            None
        } else {
            Some(character.example_dialogue.clone())
        };
    let user_fields = CharacterTemplateFields {
        name: character.name.clone(),
        description: character.description.clone(),
        avatar: character.avatar_path.clone(),
        persona: character.persona.clone(),
        greeting: character.greeting.clone(),
        example_dialogue: user_example_dialogue,
        assets: old_fields.assets.clone(),
        runtime: Some(runtime),
    };
    let new_fields = CharacterTemplateFields::from(new_manifest);
    let merge = merge_character_template(&old_fields, &user_fields, &new_fields);

    Ok(ReconcileCharacterTemplatePreview {
        instance_id: character.id.clone(),
        template_id: template_id.to_string(),
        current_template_version: current_template_version.to_string(),
        available_template_version: new_manifest.version.clone(),
        merged: merge.merged,
        conflicts: merge
            .conflicts
            .into_iter()
            .map(|conflict| ReconcileConflict {
                field: conflict.field,
                old_value: conflict.old,
                user_value: conflict.user,
                new_value: conflict.new,
            })
            .collect(),
    })
}

pub fn parse_template_defaults(snapshot_json: Option<&str>) -> Result<TemplateDefaults, String> {
    let snapshot_json =
        snapshot_json.ok_or_else(|| "character has no template snapshot to restore".to_string())?;
    let snapshot: TemplateSnapshot = serde_json::from_str(snapshot_json)
        .map_err(|error| format!("invalid template snapshot: {error}"))?;
    validate_required_text("template name", &snapshot.name)?;
    let runtime = snapshot.runtime.unwrap_or_else(|| serde_json::json!({}));
    validate_json_value_object("template runtime", &runtime)?;
    let runtime_profile_json = serde_json::to_string(&runtime)
        .map_err(|error| format!("failed to serialize template runtime: {error}"))?;

    Ok(TemplateDefaults {
        name: snapshot.name,
        description: snapshot.description,
        avatar_path: snapshot.avatar,
        persona: snapshot.persona,
        greeting: snapshot.greeting,
        example_dialogue: snapshot.example_dialogue.unwrap_or_default(),
        runtime_profile_json,
    })
}

fn validate_template_origin(
    template_id: Option<&str>,
    template_version: Option<&str>,
    template_snapshot_json: Option<&str>,
) -> Result<(), String> {
    match (template_id, template_version, template_snapshot_json) {
        (None, None, None) => Ok(()),
        (Some(id), Some(version), Some(snapshot)) => {
            validate_required_text("template id", id)?;
            validate_required_text("template version", version)?;
            serde_json::from_str::<Value>(snapshot)
                .map_err(|error| format!("invalid template snapshot: {error}"))?;
            Ok(())
        }
        _ => Err(
            "template id, version, and snapshot must either all be present or all be absent"
                .to_string(),
        ),
    }
}

fn validate_json_object(label: &str, json: &str) -> Result<(), String> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid {label}: {error}"))?;
    validate_json_value_object(label, &value)
}

fn validate_json_value_object(label: &str, value: &Value) -> Result<(), String> {
    if value.is_object() {
        Ok(())
    } else {
        Err(format!("invalid {label}: expected a JSON object"))
    }
}

fn validate_required_text(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} cannot be empty"))
    } else {
        Ok(())
    }
}

fn default_runtime_profile_json() -> String {
    "{}".to_string()
}
