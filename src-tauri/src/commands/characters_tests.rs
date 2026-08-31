// pattern: Imperative Shell

use super::*;
use crate::characters::catalog::CharacterCatalog;
use serde_json::json;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::fs;
use tempfile::TempDir;

async fn migrated_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

async fn install_abort_update_trigger(pool: &SqlitePool, id: &str, name: &str) {
    sqlx::query(&format!(
        "CREATE TRIGGER {name} BEFORE UPDATE ON characters \
         WHEN OLD.id = '{id}' BEGIN SELECT RAISE(ABORT, 'forced update failure'); END"
    ))
    .execute(pool)
    .await
    .unwrap();
}

fn complete_create_request(id: &str) -> CreateCharacterRequest {
    CreateCharacterRequest {
        id: id.to_string(),
        name: "Kokoro".to_string(),
        persona: "Warm and attentive".to_string(),
        user_nickname: "Friend".to_string(),
        source_format: "template".to_string(),
        created_at: 100,
        updated_at: 100,
        template_id: Some("kokoro".to_string()),
        template_version: Some("1.0.0".to_string()),
        template_snapshot_json: Some(template_snapshot_json()),
        description: "A daily companion".to_string(),
        avatar_path: Some("avatar.webp".to_string()),
        greeting: "Hello".to_string(),
        example_dialogue: "User: Hi\nKokoro: Hello".to_string(),
        runtime_profile_json: json!({"response_language": "en"}).to_string(),
        user_modified_at: None,
    }
}

fn template_snapshot_json() -> String {
    json!({
        "schema_version": 1,
        "engine_version": ">=0.3.0, <0.4.0",
        "id": "kokoro",
        "version": "1.0.0",
        "name": "Kokoro Default",
        "description": "Template description",
        "author": "Kokoro Project",
        "license": "CC-BY-4.0",
        "avatar": "avatar.webp",
        "persona": "Template persona",
        "greeting": "Template greeting",
        "example_dialogue": "User: Hello\nKokoro: Hi",
        "runtime": {
            "tts": {
                "provider_type": "edge",
                "provider_id": "edge-default",
                "local_preset": "edge-default",
                "voice": "en-US-AriaNeural",
                "speed": 1.0,
                "pitch": 0.0
            },
            "response_language": "en",
            "proactive_enabled": true
        }
    })
    .to_string()
}

fn catalog_with_template(version: &str, greeting: &str) -> (TempDir, CharacterCatalog) {
    let temp = TempDir::new().unwrap();
    let package_dir = temp.path().join("kokoro").join(version);
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("character.json"),
        json!({
            "schema_version": 1,
            "engine_version": ">=0.3.0, <0.4.0",
            "id": "kokoro",
            "version": version,
            "name": "Kokoro Template",
            "description": "Catalog description",
            "author": "Kokoro Project",
            "license": "CC-BY-4.0",
            "persona": "Catalog persona",
            "greeting": greeting,
            "example_dialogue": "User: Hello\nKokoro: Hi",
            "runtime": {"response_language": "en"}
        })
        .to_string(),
    )
    .unwrap();
    fs::write(package_dir.join("LICENSE.md"), "test license").unwrap();
    let catalog = CharacterCatalog::new(
        temp.path().to_path_buf(),
        semver::Version::parse("0.3.1").unwrap(),
    );
    (temp, catalog)
}

#[test]
fn template_catalog_returns_validated_manifests() {
    let (_temp, catalog) = catalog_with_template("1.1.0", "Hello from catalog");

    let templates = list_character_templates_from_catalog(&catalog).unwrap();

    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].id, "kokoro");
    assert_eq!(templates[0].version, "1.1.0");
}

#[tokio::test]
async fn instantiate_creates_user_owned_instance_with_unconsumed_greeting() {
    let pool = migrated_pool().await;
    let (_temp, catalog) = catalog_with_template("1.1.0", "Hello from catalog");

    instantiate_character_template_in_pool(
        &pool,
        &catalog,
        InstantiateCharacterTemplateRequest {
            template_id: "kokoro".to_string(),
            template_version: "1.1.0".to_string(),
            instance_id: "catalog-instance".to_string(),
            user_nickname: "Friend".to_string(),
            created_at: 600,
            updated_at: 600,
        },
    )
    .await
    .unwrap();

    let instance = list_characters_from_pool(&pool).await.unwrap().remove(0);
    assert_eq!(instance.template_id.as_deref(), Some("kokoro"));
    assert_eq!(instance.template_version.as_deref(), Some("1.1.0"));
    assert_eq!(instance.greeting, "Hello from catalog");
    assert_eq!(instance.greeting_consumed_at, None);
    assert_eq!(instance.greeting_message_id, None);
}

#[tokio::test]
async fn reconcile_is_a_non_mutating_preview_with_old_user_new_conflicts() {
    let pool = migrated_pool().await;
    let mut request = complete_create_request("instance");
    request.greeting = "User greeting".to_string();
    create_character_in_pool(&pool, request).await.unwrap();
    sqlx::query(
        "UPDATE characters SET greeting_consumed_at = 101, greeting_message_id = 7 WHERE id = 'instance'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (_temp, catalog) = catalog_with_template("1.1.0", "New template greeting");

    let preview = reconcile_character_template_from_pool(
        &pool,
        &catalog,
        ReconcileCharacterTemplateRequest {
            instance_id: "instance".to_string(),
            template_version: "1.1.0".to_string(),
        },
    )
    .await
    .unwrap();

    let greeting_conflict = preview
        .conflicts
        .iter()
        .find(|conflict| conflict.field == "greeting")
        .unwrap();
    assert_eq!(greeting_conflict.old_value, json!("Template greeting"));
    assert_eq!(greeting_conflict.user_value, json!("User greeting"));
    assert_eq!(greeting_conflict.new_value, json!("New template greeting"));

    let unchanged = list_characters_from_pool(&pool).await.unwrap().remove(0);
    assert_eq!(unchanged.greeting, "User greeting");
    assert_eq!(unchanged.template_version.as_deref(), Some("1.0.0"));
    assert_eq!(unchanged.greeting_consumed_at, Some(101));
    assert_eq!(unchanged.greeting_message_id, Some(7));
}

#[tokio::test]
async fn reconcile_treats_persisted_empty_dialogue_as_old_snapshot_none() {
    let pool = migrated_pool().await;
    let mut snapshot: serde_json::Value = serde_json::from_str(&template_snapshot_json()).unwrap();
    snapshot.as_object_mut().unwrap().remove("example_dialogue");
    let mut request = complete_create_request("instance-none-dialogue");
    request.template_snapshot_json = Some(snapshot.to_string());
    request.example_dialogue = String::new();
    create_character_in_pool(&pool, request).await.unwrap();
    let (_temp, catalog) = catalog_with_template("1.1.0", "Hello");

    let preview = reconcile_character_template_from_pool(
        &pool,
        &catalog,
        ReconcileCharacterTemplateRequest {
            instance_id: "instance-none-dialogue".to_string(),
            template_version: "1.1.0".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        preview.merged.example_dialogue.as_deref(),
        Some("User: Hello\nKokoro: Hi")
    );
    assert!(!preview
        .conflicts
        .iter()
        .any(|conflict| conflict.field == "example_dialogue"));
}

#[tokio::test]
async fn apply_reconciliation_revalidates_versions_and_preserves_owned_state() {
    let pool = migrated_pool().await;
    let mut request = complete_create_request("instance");
    request.greeting = "User greeting".to_string();
    create_character_in_pool(&pool, request).await.unwrap();
    sqlx::query(
        "UPDATE characters SET greeting_consumed_at = 101, greeting_message_id = 7 WHERE id = 'instance'",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conversation', 'instance', 'Chat', 'now', 'now')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO memories (content, embedding, created_at, character_id) VALUES ('memory', X'', 1, 'instance')")
        .execute(&pool)
        .await
        .unwrap();
    let (_temp, catalog) = catalog_with_template("1.1.0", "New template greeting");
    let preview = reconcile_character_template_from_pool(
        &pool,
        &catalog,
        ReconcileCharacterTemplateRequest {
            instance_id: "instance".to_string(),
            template_version: "1.1.0".to_string(),
        },
    )
    .await
    .unwrap();
    let mut selected = preview.merged;
    selected.greeting = "New template greeting".to_string();

    apply_character_template_reconciliation_in_pool(
        &pool,
        &catalog,
        ApplyCharacterTemplateReconciliationRequest {
            instance_id: "instance".to_string(),
            expected_current_template_version: "1.0.0".to_string(),
            expected_new_template_version: "1.1.0".to_string(),
            selected,
            updated_at: 700,
        },
    )
    .await
    .unwrap();

    let restored = list_characters_from_pool(&pool).await.unwrap().remove(0);
    assert_eq!(restored.template_version.as_deref(), Some("1.1.0"));
    assert!(restored
        .template_snapshot_json
        .as_deref()
        .unwrap()
        .contains("\"version\":\"1.1.0\""));
    assert_eq!(restored.greeting, "New template greeting");
    assert_eq!(restored.greeting_consumed_at, Some(101));
    assert_eq!(restored.greeting_message_id, Some(7));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM conversations WHERE character_id = 'instance'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM memories WHERE character_id = 'instance'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );

    let stale = apply_character_template_reconciliation_in_pool(
        &pool,
        &catalog,
        ApplyCharacterTemplateReconciliationRequest {
            instance_id: "instance".to_string(),
            expected_current_template_version: "1.0.0".to_string(),
            expected_new_template_version: "1.1.0".to_string(),
            selected: crate::characters::merge::CharacterTemplateFields::from(
                &catalog_with_template("1.1.0", "ignored")
                    .1
                    .discover()
                    .unwrap()[0]
                    .manifest,
            ),
            updated_at: 701,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(stale, KokoroError::Validation(_)));
}

#[tokio::test]
async fn crud_round_trips_all_instance_fields_without_resetting_greeting_state() {
    let pool = migrated_pool().await;
    create_character_in_pool(&pool, complete_create_request("instance"))
        .await
        .unwrap();

    let created = list_characters_from_pool(&pool).await.unwrap().remove(0);
    assert_eq!(created.template_id.as_deref(), Some("kokoro"));
    assert_eq!(created.template_version.as_deref(), Some("1.0.0"));
    assert_eq!(created.description, "A daily companion");
    assert_eq!(created.avatar_path.as_deref(), Some("avatar.webp"));
    assert_eq!(created.greeting, "Hello");
    assert_eq!(created.example_dialogue, "User: Hi\nKokoro: Hello");
    assert_eq!(
        created.runtime_profile_json,
        r#"{"response_language":"en"}"#
    );
    assert_eq!(created.greeting_consumed_at, None);

    sqlx::query(
        "UPDATE characters SET greeting_consumed_at = 101, greeting_message_id = 7 WHERE id = 'instance'",
    )
    .execute(&pool)
    .await
    .unwrap();
    update_character_in_pool(
        &pool,
        UpdateCharacterRequest {
            id: "instance".to_string(),
            name: "Kokoro Edited".to_string(),
            persona: "Edited persona".to_string(),
            user_nickname: "Pal".to_string(),
            source_format: "template".to_string(),
            updated_at: 200,
            description: Some("Edited description".to_string()),
            avatar_path: Some(Some("custom-avatar.webp".to_string())),
            greeting: Some("Edited greeting".to_string()),
            example_dialogue: Some("Edited example".to_string()),
            runtime_profile_json: Some(json!({"response_language": "zh"}).to_string()),
            user_modified_at: Some(201),
        },
    )
    .await
    .unwrap();

    let updated = list_characters_from_pool(&pool).await.unwrap().remove(0);
    assert_eq!(updated.name, "Kokoro Edited");
    assert_eq!(updated.description, "Edited description");
    assert_eq!(updated.avatar_path.as_deref(), Some("custom-avatar.webp"));
    assert_eq!(updated.greeting, "Edited greeting");
    assert_eq!(updated.example_dialogue, "Edited example");
    assert_eq!(
        updated.runtime_profile_json,
        r#"{"response_language":"zh"}"#
    );
    assert_eq!(updated.user_modified_at, Some(201));
    assert_eq!(updated.greeting_consumed_at, Some(101));
    assert_eq!(updated.greeting_message_id, Some(7));
}

#[tokio::test]
async fn old_update_shape_keeps_extended_fields_through_optional_patches() {
    let pool = migrated_pool().await;
    create_character_in_pool(&pool, complete_create_request("instance"))
        .await
        .unwrap();

    update_character_in_pool(
        &pool,
        UpdateCharacterRequest {
            id: "instance".to_string(),
            name: "Renamed".to_string(),
            persona: "Changed".to_string(),
            user_nickname: "User".to_string(),
            source_format: "manual".to_string(),
            updated_at: 300,
            description: None,
            avatar_path: None,
            greeting: None,
            example_dialogue: None,
            runtime_profile_json: None,
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    let updated = list_characters_from_pool(&pool).await.unwrap().remove(0);
    assert_eq!(updated.description, "A daily companion");
    assert_eq!(updated.avatar_path.as_deref(), Some("avatar.webp"));
    assert_eq!(updated.greeting, "Hello");
    assert_eq!(updated.example_dialogue, "User: Hi\nKokoro: Hello");
    assert_eq!(updated.user_modified_at, Some(300));
}

#[tokio::test]
async fn create_rejects_invalid_ids_names_template_origins_and_runtime_json() {
    let pool = migrated_pool().await;
    for request in [
        CreateCharacterRequest {
            id: " ".to_string(),
            ..complete_create_request("valid")
        },
        CreateCharacterRequest {
            name: " ".to_string(),
            ..complete_create_request("valid")
        },
        CreateCharacterRequest {
            template_version: None,
            ..complete_create_request("valid")
        },
        CreateCharacterRequest {
            runtime_profile_json: "not-json".to_string(),
            ..complete_create_request("valid")
        },
    ] {
        let error = create_character_in_pool(&pool, request).await.unwrap_err();
        assert!(matches!(error, KokoroError::Validation(_)));
    }
}

#[tokio::test]
async fn duplicate_copies_instance_content_but_starts_with_unconsumed_greeting() {
    let pool = migrated_pool().await;
    create_character_in_pool(&pool, complete_create_request("source"))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE characters SET greeting_consumed_at = 101, greeting_message_id = 7 WHERE id = 'source'",
    )
    .execute(&pool)
    .await
    .unwrap();

    duplicate_character_in_pool(
        &pool,
        DuplicateCharacterRequest {
            source_id: "source".to_string(),
            new_id: "copy".to_string(),
            new_name: Some("Kokoro Copy".to_string()),
            created_at: 400,
            updated_at: 400,
        },
    )
    .await
    .unwrap();

    let copy = list_characters_from_pool(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|character| character.id == "copy")
        .unwrap();
    assert_eq!(copy.name, "Kokoro Copy");
    assert_eq!(copy.persona, "Warm and attentive");
    assert_eq!(copy.template_id.as_deref(), Some("kokoro"));
    assert_eq!(copy.greeting, "Hello");
    assert_eq!(copy.greeting_consumed_at, None);
    assert_eq!(copy.greeting_message_id, None);
}

#[tokio::test]
async fn restore_defaults_preserves_consumed_greeting_conversations_and_memories() {
    let pool = migrated_pool().await;
    let mut request = complete_create_request("instance");
    request.name = "User name".to_string();
    request.persona = "User persona".to_string();
    request.description = "User description".to_string();
    request.greeting = "User greeting".to_string();
    create_character_in_pool(&pool, request).await.unwrap();
    sqlx::query(
        "UPDATE characters SET greeting_consumed_at = 101, greeting_message_id = 7 WHERE id = 'instance'",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO conversations (id, character_id, title, created_at, updated_at) \
         VALUES ('conversation', 'instance', 'Chat', 'now', 'now')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO memories (content, embedding, created_at, character_id) \
         VALUES ('memory', X'', 1, 'instance')",
    )
    .execute(&pool)
    .await
    .unwrap();

    restore_character_defaults_in_pool(
        &pool,
        RestoreCharacterDefaultsRequest {
            id: "instance".to_string(),
            updated_at: 500,
        },
    )
    .await
    .unwrap();

    let restored = list_characters_from_pool(&pool).await.unwrap().remove(0);
    assert_eq!(restored.name, "Kokoro Default");
    assert_eq!(restored.description, "Template description");
    assert_eq!(restored.persona, "Template persona");
    assert_eq!(restored.greeting, "Template greeting");
    assert_eq!(restored.example_dialogue, "User: Hello\nKokoro: Hi");
    assert_eq!(restored.greeting_consumed_at, Some(101));
    assert_eq!(restored.greeting_message_id, Some(7));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM conversations WHERE character_id = 'instance'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM memories WHERE character_id = 'instance'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn delete_removes_only_the_instance_row() {
    let pool = migrated_pool().await;
    create_character_in_pool(&pool, complete_create_request("instance"))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO conversations (id, character_id, title, created_at, updated_at) \
         VALUES ('conversation', 'instance', 'Chat', 'now', 'now')",
    )
    .execute(&pool)
    .await
    .unwrap();

    delete_character_in_pool(&pool, "instance").await.unwrap();

    assert_eq!(list_characters_from_pool(&pool).await.unwrap().len(), 0);
    let conversation_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversations WHERE character_id = 'instance'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(conversation_count, 1);
}

#[test]
fn update_request_deserializes_the_legacy_ipc_shape() {
    let request: UpdateCharacterRequest = serde_json::from_value(json!({
        "id": "legacy",
        "name": "Legacy",
        "persona": "Persona",
        "user_nickname": "User",
        "source_format": "manual",
        "updated_at": 10
    }))
    .unwrap();

    assert_eq!(request.description, None);
    assert_eq!(request.avatar_path, None);
    assert_eq!(request.greeting, None);
    assert_eq!(request.runtime_profile_json, None);
}

#[test]
fn create_request_deserializes_the_legacy_ipc_shape_with_explicit_defaults() {
    let request: CreateCharacterRequest = serde_json::from_value(json!({
        "id": "legacy",
        "name": "Legacy",
        "persona": "Persona",
        "user_nickname": "User",
        "source_format": "manual",
        "created_at": 10,
        "updated_at": 10
    }))
    .unwrap();

    assert_eq!(request.template_id, None);
    assert_eq!(request.description, "");
    assert_eq!(request.avatar_path, None);
    assert_eq!(request.greeting, "");
    assert_eq!(request.example_dialogue, "");
    assert_eq!(request.runtime_profile_json, "{}");
}

#[test]
fn restore_defaults_normalizes_omitted_optional_manifest_fields() {
    let snapshot = json!({
        "name": "Legacy template",
        "description": "Description",
        "persona": "Persona",
        "greeting": "Hello"
    })
    .to_string();

    let defaults = parse_template_defaults(Some(&snapshot)).unwrap();

    assert_eq!(defaults.example_dialogue, "");
    assert_eq!(defaults.runtime_profile_json, "{}");
}

#[test]
fn restore_defaults_normalizes_null_optional_manifest_fields() {
    let snapshot = json!({
        "name": "Legacy template",
        "description": "Description",
        "avatar": null,
        "persona": "Persona",
        "greeting": "Hello",
        "example_dialogue": null,
        "runtime": null
    })
    .to_string();

    let defaults = parse_template_defaults(Some(&snapshot)).unwrap();

    assert_eq!(defaults.avatar_path, None);
    assert_eq!(defaults.example_dialogue, "");
    assert_eq!(defaults.runtime_profile_json, "{}");
}

#[test]
fn imported_png_avatar_is_persisted_as_a_typed_instance_resource() {
    let temp = TempDir::new().unwrap();
    let png = [137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0];

    let reference = persist_character_avatar(temp.path(), "instance-1", &png).unwrap();

    assert_eq!(
        reference,
        "character-instance-resource://instance-1/avatar.png"
    );
    assert_eq!(
        fs::read(
            temp.path()
                .join("character-instance-resources/instance-1/avatar.png")
        )
        .unwrap(),
        png
    );
    assert!(persist_character_avatar(temp.path(), "../escape", &png).is_err());
    assert!(persist_character_avatar(temp.path(), "instance-2", b"not-png").is_err());
}

#[tokio::test]
async fn png_backed_duplicate_owns_a_copied_resource() {
    let pool = migrated_pool().await;
    let temp = TempDir::new().unwrap();
    let png = [137, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3, 4];
    let mut source = complete_create_request("source-png");
    source.template_id = None;
    source.template_version = None;
    source.template_snapshot_json = None;
    source.avatar_path = Some(persist_character_avatar(temp.path(), "source-png", &png).unwrap());
    create_character_in_pool(&pool, source).await.unwrap();

    duplicate_character_with_resources_in_pool(
        &pool,
        temp.path(),
        DuplicateCharacterRequest {
            source_id: "source-png".to_string(),
            new_id: "copy-png".to_string(),
            new_name: None,
            created_at: 800,
            updated_at: 800,
        },
    )
    .await
    .unwrap();

    let copied = list_characters_from_pool(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|character| character.id == "copy-png")
        .unwrap();
    assert_eq!(
        copied.avatar_path.as_deref(),
        Some("character-instance-resource://copy-png/avatar.png")
    );
    assert_eq!(
        fs::read(
            temp.path()
                .join("character-instance-resources/copy-png/avatar.png")
        )
        .unwrap(),
        png
    );
}

#[tokio::test]
async fn managed_avatar_creation_and_duplicate_failures_leave_no_orphan() {
    let pool = migrated_pool().await;
    let temp = TempDir::new().unwrap();
    let png = [137, 80, 78, 71, 13, 10, 26, 10, 1];
    create_character_in_pool(&pool, complete_create_request("collision"))
        .await
        .unwrap();
    let failure = create_character_with_avatar_in_pool(
        &pool,
        temp.path(),
        complete_create_request("collision"),
        &png,
    )
    .await;
    assert!(failure.is_err());
    assert!(!temp
        .path()
        .join("character-instance-resources/collision")
        .exists());

    let mut source = complete_create_request("source-failure");
    source.template_id = None;
    source.template_version = None;
    source.template_snapshot_json = None;
    source.avatar_path =
        Some(persist_character_avatar(temp.path(), "source-failure", &png).unwrap());
    create_character_in_pool(&pool, source).await.unwrap();
    let duplicate_failure = duplicate_character_with_resources_in_pool(
        &pool,
        temp.path(),
        DuplicateCharacterRequest {
            source_id: "source-failure".to_string(),
            new_id: "collision".to_string(),
            new_name: None,
            created_at: 900,
            updated_at: 900,
        },
    )
    .await;
    assert!(duplicate_failure.is_err());
    assert!(!temp
        .path()
        .join("character-instance-resources/collision")
        .exists());
}

#[tokio::test]
async fn deleting_png_backed_character_removes_its_owned_resource() {
    let pool = migrated_pool().await;
    let temp = TempDir::new().unwrap();
    let png = [137, 80, 78, 71, 13, 10, 26, 10, 1];
    let mut request = complete_create_request("delete-png");
    request.template_id = None;
    request.template_version = None;
    request.template_snapshot_json = None;
    request.avatar_path = Some(persist_character_avatar(temp.path(), "delete-png", &png).unwrap());
    create_character_in_pool(&pool, request).await.unwrap();

    delete_character_with_resources_in_pool(&pool, temp.path(), "delete-png")
        .await
        .unwrap();

    assert!(list_characters_from_pool(&pool).await.unwrap().is_empty());
    assert!(!temp
        .path()
        .join("character-instance-resources/delete-png")
        .exists());
}

#[tokio::test]
async fn avatar_changing_update_deletes_owned_resource_and_restores_it_on_failure() {
    let pool = migrated_pool().await;
    let temp = TempDir::new().unwrap();
    let png = [137, 80, 78, 71, 13, 10, 26, 10, 1];
    for id in ["update-success", "update-failure"] {
        let mut create = complete_create_request(id);
        create.avatar_path = Some(persist_character_avatar(temp.path(), id, &png).unwrap());
        create_character_in_pool(&pool, create).await.unwrap();
    }

    update_character_with_resources_in_pool(
        &pool,
        temp.path(),
        UpdateCharacterRequest {
            id: "update-success".to_string(),
            name: "Updated".to_string(),
            persona: "Updated".to_string(),
            user_nickname: "User".to_string(),
            source_format: "manual".to_string(),
            updated_at: 1_000,
            description: None,
            avatar_path: Some(Some("replacement.webp".to_string())),
            greeting: None,
            example_dialogue: None,
            runtime_profile_json: None,
            user_modified_at: None,
        },
    )
    .await
    .unwrap();
    assert!(!temp
        .path()
        .join("character-instance-resources/update-success")
        .exists());

    install_abort_update_trigger(&pool, "update-failure", "fail_managed_update").await;
    let failure = update_character_with_resources_in_pool(
        &pool,
        temp.path(),
        UpdateCharacterRequest {
            id: "update-failure".to_string(),
            name: "Updated".to_string(),
            persona: "Updated".to_string(),
            user_nickname: "User".to_string(),
            source_format: "manual".to_string(),
            updated_at: 1_001,
            description: None,
            avatar_path: Some(None),
            greeting: None,
            example_dialogue: None,
            runtime_profile_json: None,
            user_modified_at: None,
        },
    )
    .await;
    assert!(failure.is_err());
    assert_eq!(
        fs::read(
            temp.path()
                .join("character-instance-resources/update-failure/avatar.png")
        )
        .unwrap(),
        png
    );
}

#[tokio::test]
async fn restoring_defaults_deletes_owned_resource_and_restores_it_on_failure() {
    let pool = migrated_pool().await;
    let temp = TempDir::new().unwrap();
    let png = [137, 80, 78, 71, 13, 10, 26, 10, 2];
    for id in ["restore-success", "restore-failure"] {
        let mut create = complete_create_request(id);
        create.avatar_path = Some(persist_character_avatar(temp.path(), id, &png).unwrap());
        create_character_in_pool(&pool, create).await.unwrap();
    }

    restore_character_defaults_with_resources_in_pool(
        &pool,
        temp.path(),
        RestoreCharacterDefaultsRequest {
            id: "restore-success".to_string(),
            updated_at: 1_100,
        },
    )
    .await
    .unwrap();
    assert!(!temp
        .path()
        .join("character-instance-resources/restore-success")
        .exists());

    install_abort_update_trigger(&pool, "restore-failure", "fail_managed_restore").await;
    let failure = restore_character_defaults_with_resources_in_pool(
        &pool,
        temp.path(),
        RestoreCharacterDefaultsRequest {
            id: "restore-failure".to_string(),
            updated_at: 1_101,
        },
    )
    .await;
    assert!(failure.is_err());
    assert_eq!(
        fs::read(
            temp.path()
                .join("character-instance-resources/restore-failure/avatar.png")
        )
        .unwrap(),
        png
    );
}

#[tokio::test]
async fn reconciliation_deletes_owned_resource_and_restores_it_on_failure() {
    let pool = migrated_pool().await;
    let temp = TempDir::new().unwrap();
    let png = [137, 80, 78, 71, 13, 10, 26, 10, 3];
    let (_catalog_temp, catalog) = catalog_with_template("1.1.0", "New greeting");
    for id in ["reconcile-success", "reconcile-failure"] {
        let mut create = complete_create_request(id);
        create.avatar_path = Some(persist_character_avatar(temp.path(), id, &png).unwrap());
        create_character_in_pool(&pool, create).await.unwrap();
    }

    for (id, should_fail) in [("reconcile-success", false), ("reconcile-failure", true)] {
        let preview = reconcile_character_template_from_pool(
            &pool,
            &catalog,
            ReconcileCharacterTemplateRequest {
                instance_id: id.to_string(),
                template_version: "1.1.0".to_string(),
            },
        )
        .await
        .unwrap();
        let mut selected = preview.merged;
        selected.avatar = None;
        if should_fail {
            install_abort_update_trigger(&pool, id, "fail_managed_reconciliation").await;
        }
        let result = apply_character_template_reconciliation_with_resources_in_pool(
            &pool,
            &catalog,
            temp.path(),
            ApplyCharacterTemplateReconciliationRequest {
                instance_id: id.to_string(),
                expected_current_template_version: "1.0.0".to_string(),
                expected_new_template_version: "1.1.0".to_string(),
                selected,
                updated_at: 1_200,
            },
        )
        .await;
        let avatar = temp
            .path()
            .join("character-instance-resources")
            .join(id)
            .join("avatar.png");
        if should_fail {
            assert!(result.is_err());
            assert_eq!(fs::read(avatar).unwrap(), png);
        } else {
            result.unwrap();
            assert!(!avatar.exists());
        }
    }
}

#[tokio::test]
async fn production_activation_backend_applies_and_persists_complete_selection() {
    let orchestrator = AIOrchestrator::new("sqlite::memory:").await.unwrap();
    let temp = TempDir::new().unwrap();
    let snapshot = BackendRuntimeSnapshot {
        character_id: "persistent-character".into(),
        character_name: "Persistent Character".into(),
        user_name: "Owner".into(),
        system_prompt: "<character_persona>\nPersistent\n</character_persona>".into(),
        response_language: "ja".into(),
        proactive_enabled: true,
        current_conversation_id: Some("persistent-conversation".into()),
        ..Default::default()
    };

    apply_orchestrator_runtime(&orchestrator, &snapshot, temp.path())
        .await
        .unwrap();

    assert_eq!(
        orchestrator.get_character_id().await,
        "persistent-character"
    );
    assert_eq!(
        *orchestrator.system_prompt.lock().await,
        snapshot.system_prompt
    );
    assert_eq!(
        *orchestrator.current_conversation_id.lock().await,
        Some("persistent-conversation".into())
    );
    let active: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("active_character_id.json")).unwrap())
            .unwrap();
    let conversation: serde_json::Value = serde_json::from_slice(
        &fs::read(temp.path().join("current_conversation_id.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(active["character_id"], "persistent-character");
    assert_eq!(conversation["conversation_id"], "persistent-conversation");
}
