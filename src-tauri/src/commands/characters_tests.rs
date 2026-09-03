// pattern: Imperative Shell

use super::*;
use crate::characters::catalog::CharacterCatalog;
use serde_json::json;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::fs;
use std::sync::Arc;
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

#[test]
fn template_catalog_omits_the_documentation_scaffold() {
    let (temp, catalog) = catalog_with_template("1.1.0", "Hello from catalog");
    let package_dir = temp.path().join("my-character").join("0.1.0");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("character.json"),
        json!({
            "schema_version": 1,
            "engine_version": ">=0.3.0, <0.4.0",
            "id": "my-character",
            "version": "0.1.0",
            "name": "My Character",
            "description": "Documentation scaffold",
            "author": "Kokoro Project",
            "license": "CC-BY-4.0",
            "persona": "Scaffold persona",
            "greeting": "Hello",
        })
        .to_string(),
    )
    .unwrap();
    fs::write(package_dir.join("LICENSE.md"), "test license").unwrap();

    let templates = list_character_templates_from_catalog(&catalog).unwrap();

    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].id, "kokoro");
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
async fn updating_png_avatar_replaces_the_managed_resource_and_reference() {
    let pool = migrated_pool().await;
    let temp = TempDir::new().unwrap();
    let old_png = [137, 80, 78, 71, 13, 10, 26, 10, 1];
    let new_png = [137, 80, 78, 71, 13, 10, 26, 10, 2];
    create_character_with_avatar_in_pool(
        &pool,
        temp.path(),
        complete_create_request("avatar-update"),
        &old_png,
    )
    .await
    .unwrap();

    update_character_with_avatar_in_pool(
        &pool,
        temp.path(),
        UpdateCharacterRequest {
            id: "avatar-update".to_string(),
            name: "Kokoro".to_string(),
            persona: "Warm and attentive".to_string(),
            user_nickname: "Friend".to_string(),
            source_format: "template".to_string(),
            updated_at: 200,
            description: None,
            avatar_path: None,
            greeting: None,
            example_dialogue: None,
            runtime_profile_json: None,
            user_modified_at: None,
        },
        &new_png,
    )
    .await
    .unwrap();

    let updated = list_characters_from_pool(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|character| character.id == "avatar-update")
        .unwrap();
    assert_eq!(
        updated.avatar_path.as_deref(),
        Some("character-instance-resource://avatar-update/avatar.png")
    );
    assert_eq!(
        fs::read(
            temp.path()
                .join("character-instance-resources/avatar-update/avatar.png")
        )
        .unwrap(),
        new_png
    );
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

#[tokio::test]
async fn orchestrator_activation_backend_restore_restores_history_and_memory_boundary() {
    let orchestrator = AIOrchestrator::new("sqlite::memory:").await.unwrap();
    let temp = TempDir::new().unwrap();

    sqlx::query(
        "INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-1', 'restored-char', 'Test', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&orchestrator.db)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO conversation_messages (conversation_id, role, content, created_at) VALUES ('conv-1', 'user', 'Hi there', '2026-01-01T00:00:01Z'), ('conv-1', 'assistant', 'Hello!', '2026-01-01T00:00:02Z')",
    )
    .execute(&orchestrator.db)
    .await
    .unwrap();

    let backend = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data: temp.path().to_path_buf(),
    };

    let snapshot = BackendRuntimeSnapshot {
        character_id: "restored-char".into(),
        character_name: "Restored Name".into(),
        current_conversation_id: Some("conv-1".into()),
        ..Default::default()
    };

    backend.restore(&snapshot).await.unwrap();

    assert_eq!(orchestrator.get_character_id().await, "restored-char");
    let history = orchestrator.history.lock().await;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "Hi there");
    assert_eq!(history[1].content, "Hello!");
    assert_eq!(orchestrator.memory_history_boundary().await, 2);
}

#[tokio::test]
async fn orchestrator_runtime_degraded_blocks_prompt_and_clears_on_successful_apply() {
    let orchestrator = AIOrchestrator::new("sqlite::memory:").await.unwrap();
    let temp = TempDir::new().unwrap();

    orchestrator
        .set_runtime_degraded(Some("Startup recovery failed".to_string()))
        .await;
    assert!(orchestrator.get_runtime_degraded().await.is_some());

    let err = orchestrator
        .compose_prompt("Hello", false, None, false, "char-1")
        .await
        .expect_err("compose_prompt must fail when degraded");
    assert!(err.to_string().contains("Character runtime is degraded"));

    let backend = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data: temp.path().to_path_buf(),
    };
    let snapshot = BackendRuntimeSnapshot {
        character_id: "char-1".into(),
        character_name: "Char One".into(),
        ..Default::default()
    };
    backend.apply(&snapshot).await.unwrap();

    // Degradation is NOT cleared by apply alone (history sync has not completed yet)
    assert!(orchestrator.get_runtime_degraded().await.is_some());

    // Successful restore (or completed coordinator activation with history sync) clears degradation
    backend.restore(&snapshot).await.unwrap();
    assert!(orchestrator.get_runtime_degraded().await.is_none());
    let (messages, _) = orchestrator
        .compose_prompt("Hello", false, None, false, "char-1")
        .await
        .expect("compose_prompt should succeed once runtime is applied");
    assert!(!messages.is_empty());
}

#[tokio::test]
async fn non_startup_activation_recovery_failure_sets_degraded_and_blocks_chat() {
    let orchestrator = AIOrchestrator::new("sqlite::memory:").await.unwrap();
    let temp = TempDir::new().unwrap();
    let coordinator = ActivationCoordinator::default();
    let backend = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data: temp.path().to_path_buf(),
    };

    // 1. Set up two characters in database with empty greetings so staging doesn't touch messages table
    create_character_in_pool(
        &orchestrator.db,
        CreateCharacterRequest {
            id: "char-1".into(),
            name: "Char One".into(),
            persona: "You are Char One.".into(),
            user_nickname: "User".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: String::new(),
            example_dialogue: "Example".into(),
            runtime_profile_json: "{}".into(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    create_character_in_pool(
        &orchestrator.db,
        CreateCharacterRequest {
            id: "char-2".into(),
            name: "Char Two".into(),
            persona: "You are Char Two.".into(),
            user_nickname: "User".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: String::new(),
            example_dialogue: "Example".into(),
            runtime_profile_json: "{}".into(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    // 2. Activate char-1 successfully first (non-degraded)
    let tts_config = crate::tts::config::TtsSystemConfig::default();
    let token1 = coordinator
        .prepare(&orchestrator.db, "char-1", &tts_config, &[], &backend)
        .await
        .unwrap();
    coordinator
        .commit(&orchestrator.db, token1, &backend)
        .await
        .unwrap();

    assert_eq!(orchestrator.get_character_id().await, "char-1");
    assert!(orchestrator.get_runtime_degraded().await.is_none());

    // 3. Prepare activation for char-2
    let token2 = coordinator
        .prepare(&orchestrator.db, "char-2", &tts_config, &[], &backend)
        .await
        .unwrap();

    // 4. Rename conversation_messages table so history sync fails on SELECT,
    // causing both the new activation sync and the rollback restore sync to fail!
    sqlx::query("ALTER TABLE conversation_messages RENAME TO conversation_messages_backup")
        .execute(&orchestrator.db)
        .await
        .unwrap();

    // 5. Attempt commit in non-startup context
    let err = coordinator
        .commit(&orchestrator.db, token2, &backend)
        .await
        .expect_err("activation commit must fail when history sync fails");

    let err_str = err.to_string();
    assert!(
        err_str.contains("failed to sync history")
            || err_str.contains("failed to sync conversation history"),
        "error was: {err_str}"
    );

    // 6. Verify that the runtime is now degraded and in-memory history is cleared
    let degraded_reason = orchestrator.get_runtime_degraded().await;
    assert!(
        degraded_reason.is_some(),
        "orchestrator runtime_degraded MUST be set when recovery fails"
    );
    assert!(
        orchestrator.history.lock().await.is_empty(),
        "history must be cleared when sync fails during recovery"
    );

    // 7. Verify compose_prompt is rejected with degraded error
    let prompt_err = orchestrator
        .compose_prompt("Hello", false, None, false, "char-1")
        .await
        .expect_err("compose_prompt must be blocked when runtime is degraded");
    assert!(
        prompt_err
            .to_string()
            .contains("Character runtime is degraded"),
        "prompt error was: {prompt_err}"
    );
}

struct DelayedActivationBackend {
    orchestrator: Arc<AIOrchestrator>,
    app_data: PathBuf,
    delay_ms: u64,
}

#[async_trait::async_trait]
impl ActivationRuntimeBackend for DelayedActivationBackend {
    async fn snapshot(&self) -> Result<BackendRuntimeSnapshot, KokoroError> {
        let backend = OrchestratorActivationBackend {
            orchestrator: &self.orchestrator,
            app_data: self.app_data.clone(),
        };
        backend.snapshot().await
    }
    async fn apply(&self, snapshot: &BackendRuntimeSnapshot) -> Result<(), KokoroError> {
        let backend = OrchestratorActivationBackend {
            orchestrator: &self.orchestrator,
            app_data: self.app_data.clone(),
        };
        backend.apply(snapshot).await?;
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        Ok(())
    }
    async fn restore(&self, snapshot: &BackendRuntimeSnapshot) -> Result<(), KokoroError> {
        let backend = OrchestratorActivationBackend {
            orchestrator: &self.orchestrator,
            app_data: self.app_data.clone(),
        };
        backend.restore(snapshot).await
    }
    async fn sync_history(&self, conversation_id: Option<&str>) -> Result<(), KokoroError> {
        let backend = OrchestratorActivationBackend {
            orchestrator: &self.orchestrator,
            app_data: self.app_data.clone(),
        };
        backend.sync_history(conversation_id).await
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

#[tokio::test]
async fn concurrent_chat_during_activation_is_blocked_and_prevents_context_leak_and_message_loss() {
    let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
    let temp = TempDir::new().unwrap();
    let coordinator = Arc::new(ActivationCoordinator::default());

    // 1. Create Char 1 and Char 2 in SQLite
    create_character_in_pool(
        &orchestrator.db,
        CreateCharacterRequest {
            id: "char-1".into(),
            name: "Char One".into(),
            persona: "You are Character One.".into(),
            user_nickname: "User".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: String::new(),
            example_dialogue: "Example".into(),
            runtime_profile_json: "{}".into(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    create_character_in_pool(
        &orchestrator.db,
        CreateCharacterRequest {
            id: "char-2".into(),
            name: "Char Two".into(),
            persona: "You are Character Two.".into(),
            user_nickname: "User".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: String::new(),
            example_dialogue: "Example".into(),
            runtime_profile_json: "{}".into(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    let tts_config = crate::tts::config::TtsSystemConfig::default();

    // 2. Activate char-1
    let backend1 = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data: temp.path().to_path_buf(),
    };
    let token1 = coordinator
        .prepare(&orchestrator.db, "char-1", &tts_config, &[], &backend1)
        .await
        .unwrap();
    coordinator
        .commit(&orchestrator.db, token1, &backend1)
        .await
        .unwrap();

    // Append a message to char-1's in-memory history
    orchestrator
        .push_history_message(crate::ai::context::Message {
            role: "user".into(),
            content: "Hello from char-1 conversation".into(),
            metadata: None,
        })
        .await;

    assert_eq!(orchestrator.get_character_id().await, "char-1");
    assert!(!orchestrator.history.lock().await.is_empty());
    assert!(!orchestrator.is_activating());

    // 3. Prepare activation for char-2
    let token2 = coordinator
        .prepare(&orchestrator.db, "char-2", &tts_config, &[], &backend1)
        .await
        .unwrap();

    // 4. Commit char-2 activation with an artificial delay in apply
    let delayed_backend = DelayedActivationBackend {
        orchestrator: orchestrator.clone(),
        app_data: temp.path().to_path_buf(),
        delay_ms: 100,
    };

    let coord_clone = coordinator.clone();
    let db_clone = orchestrator.db.clone();
    let orch_clone = orchestrator.clone();

    // Spawn the activation in a background task
    let activation_task = tokio::spawn(async move {
        coord_clone
            .commit(&db_clone, token2, &delayed_backend)
            .await
    });

    // Wait briefly so the background task enters the activation lock and is sleeping in apply
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    // 5. Verify that during activation, chat turns and compose_prompt are BLOCKED by the gate
    assert!(
        orch_clone.is_activating(),
        "orchestrator must report is_activating() = true during activation"
    );

    // Attempting to enter chat turn must fail
    let turn_err = orch_clone
        .enter_chat_turn()
        .expect_err("enter_chat_turn must be blocked during activation");
    assert!(
        turn_err.contains("Character activation is in progress"),
        "error was: {turn_err}"
    );

    // Attempting to compose prompt must fail
    let prompt_err = orch_clone
        .compose_prompt("Hello during activation", false, None, false, "char-1")
        .await
        .expect_err("compose_prompt must be blocked during activation");
    assert!(
        prompt_err
            .to_string()
            .contains("Character activation is in progress"),
        "error was: {prompt_err}"
    );

    // 6. Await activation completion
    let commit_res = activation_task.await.unwrap();
    assert!(commit_res.is_ok(), "activation must complete successfully");

    // 7. Verify gate is released and normal chat succeeds with new character
    assert!(
        !orchestrator.is_activating(),
        "activation gate must be released after commit"
    );
    assert_eq!(orchestrator.get_character_id().await, "char-2");

    let chat_guard = orchestrator.enter_chat_turn();
    assert!(
        chat_guard.is_ok(),
        "chat turn must be allowed after activation completes"
    );
    drop(chat_guard);

    let (messages, _) = orchestrator
        .compose_prompt("Hello after activation", false, None, false, "char-2")
        .await
        .expect("compose_prompt must succeed after activation completes");
    assert!(!messages.is_empty());
}

#[tokio::test]
async fn test_active_stream_chat_turn_completes_when_activation_starts_concurrently() {
    let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
    let temp = TempDir::new().unwrap();
    let coordinator = Arc::new(ActivationCoordinator::default());

    create_character_in_pool(
        &orchestrator.db,
        CreateCharacterRequest {
            id: "char-1".into(),
            name: "Char One".into(),
            persona: "You are Character One.".into(),
            user_nickname: "User".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: String::new(),
            example_dialogue: "Example".into(),
            runtime_profile_json: "{}".into(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    create_character_in_pool(
        &orchestrator.db,
        CreateCharacterRequest {
            id: "char-2".into(),
            name: "Char Two".into(),
            persona: "You are Character Two.".into(),
            user_nickname: "User".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: String::new(),
            example_dialogue: "Example".into(),
            runtime_profile_json: "{}".into(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    let tts_config = crate::tts::config::TtsSystemConfig::default();
    let backend1 = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data: temp.path().to_path_buf(),
    };
    let token1 = coordinator
        .prepare(&orchestrator.db, "char-1", &tts_config, &[], &backend1)
        .await
        .unwrap();
    coordinator
        .commit(&orchestrator.db, token1, &backend1)
        .await
        .unwrap();

    assert_eq!(orchestrator.get_character_id().await, "char-1");

    // 1. stream_chat begins: acquires turn guard at entry
    let chat_turn_guard = orchestrator
        .enter_chat_turn()
        .expect("must enter chat turn before activation");

    // 2. stream_chat records user message
    orchestrator
        .add_message(
            "user".into(),
            "User message during active turn".into(),
            "char-1",
        )
        .await;

    // 3. Concurrently, activation for char-2 begins in background
    let token2 = coordinator
        .prepare(&orchestrator.db, "char-2", &tts_config, &[], &backend1)
        .await
        .unwrap();

    let delayed_backend = DelayedActivationBackend {
        orchestrator: orchestrator.clone(),
        app_data: temp.path().to_path_buf(),
        delay_ms: 100,
    };

    let coord_clone = coordinator.clone();
    let db_clone = orchestrator.db.clone();
    let activation_task = tokio::spawn(async move {
        coord_clone
            .commit(&db_clone, token2, &delayed_backend)
            .await
    });

    // Wait briefly so activation sets `activating = true` and waits on write lock
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        orchestrator.is_activating(),
        "orchestrator must be in activating state"
    );

    // 4. stream_chat calls compose_prompt_with_guard:
    // With compose_prompt_with_guard, this MUST SUCCEED even though is_activating() == true!
    let (messages, _) = orchestrator
        .compose_prompt_with_guard(
            "User message during active turn",
            false,
            None,
            false,
            "char-1",
            &chat_turn_guard,
        )
        .await
        .expect("active turn holding guard must be able to compose prompt despite activating=true");
    assert!(!messages.is_empty());

    // 5. stream_chat finishes: persists assistant message and then drops guard
    orchestrator
        .add_message(
            "assistant".into(),
            "Assistant reply completing the turn".into(),
            "char-1",
        )
        .await;
    drop(chat_turn_guard);

    // 6. Activation completes
    let commit_res = activation_task.await.unwrap();
    assert!(
        commit_res.is_ok(),
        "activation must succeed after turn completes"
    );

    assert_eq!(orchestrator.get_character_id().await, "char-2");
    assert!(!orchestrator.is_activating());

    // 7. Verify char-1 history has the complete turn
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT role, content FROM conversation_messages ORDER BY id ASC",
    )
    .fetch_all(&orchestrator.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "user");
    assert_eq!(rows[1].0, "assistant");
}

#[tokio::test]
async fn test_non_webhook_bot_turn_during_activation_is_blocked_at_entry_without_message_pollution()
{
    let orchestrator = Arc::new(AIOrchestrator::new("sqlite::memory:").await.unwrap());
    let temp = TempDir::new().unwrap();
    let coordinator = Arc::new(ActivationCoordinator::default());

    create_character_in_pool(
        &orchestrator.db,
        CreateCharacterRequest {
            id: "char-1".into(),
            name: "Char One".into(),
            persona: "You are Character One.".into(),
            user_nickname: "User".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: String::new(),
            example_dialogue: "Example".into(),
            runtime_profile_json: "{}".into(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    create_character_in_pool(
        &orchestrator.db,
        CreateCharacterRequest {
            id: "char-2".into(),
            name: "Char Two".into(),
            persona: "You are Character Two.".into(),
            user_nickname: "User".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: String::new(),
            example_dialogue: "Example".into(),
            runtime_profile_json: "{}".into(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();

    let tts_config = crate::tts::config::TtsSystemConfig::default();
    let backend1 = OrchestratorActivationBackend {
        orchestrator: &orchestrator,
        app_data: temp.path().to_path_buf(),
    };
    let token1 = coordinator
        .prepare(&orchestrator.db, "char-1", &tts_config, &[], &backend1)
        .await
        .unwrap();
    coordinator
        .commit(&orchestrator.db, token1, &backend1)
        .await
        .unwrap();

    // 1. Begin activation for char-2 with a delay
    let token2 = coordinator
        .prepare(&orchestrator.db, "char-2", &tts_config, &[], &backend1)
        .await
        .unwrap();

    let delayed_backend = DelayedActivationBackend {
        orchestrator: orchestrator.clone(),
        app_data: temp.path().to_path_buf(),
        delay_ms: 100,
    };

    let coord_clone = coordinator.clone();
    let db_clone = orchestrator.db.clone();
    let activation_task = tokio::spawn(async move {
        coord_clone
            .commit(&db_clone, token2, &delayed_backend)
            .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(orchestrator.is_activating());

    // 2. Simulate Non-Webhook Bot (e.g. LINE, Discord, QQ, Telegram) receiving a message during activation:
    // With the fix, the bot entry checks enter_chat_turn() BEFORE add_message!
    let bot_turn_res = orchestrator.enter_chat_turn();
    assert!(
        bot_turn_res.is_err(),
        "bot turn entry must be rejected during activation"
    );
    let err_msg = bot_turn_res.err().unwrap();
    assert!(
        err_msg.contains("Character activation is in progress"),
        "error was: {err_msg}"
    );

    // 3. Verify NO user message was added to the database or history (no partial turn pollution)
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
        .fetch_one(&orchestrator.db)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "no messages should be written when bot turn is blocked at entry"
    );
    assert!(orchestrator.history.lock().await.is_empty());

    // 4. Await activation completion
    let commit_res = activation_task.await.unwrap();
    assert!(commit_res.is_ok());
    assert_eq!(orchestrator.get_character_id().await, "char-2");

    // 5. Bot retry after activation completes succeeds cleanly under char-2
    let bot_turn_retry = orchestrator.enter_chat_turn();
    assert!(
        bot_turn_retry.is_ok(),
        "bot turn must succeed after activation completes"
    );
    let guard = bot_turn_retry.unwrap();
    orchestrator
        .add_message("user".into(), "Hello to char 2".into(), "char-2")
        .await;
    let (messages, _) = orchestrator
        .compose_prompt_with_guard("Hello to char 2", false, None, false, "char-2", &guard)
        .await
        .expect("compose prompt must succeed for new character");
    assert!(!messages.is_empty());
    drop(guard);
}
