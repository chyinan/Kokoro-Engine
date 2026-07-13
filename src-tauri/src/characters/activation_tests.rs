// pattern: Imperative Shell

use super::activation::{
    ActivationCoordinator, ActivationRuntimeBackend, BackendRuntimeSnapshot, GreetingAction,
    LocalTtsPreset, ResolvedTtsMode,
};
use crate::ai::context::AIOrchestrator;
use crate::commands::characters::{create_character_in_pool, CreateCharacterRequest};
use crate::error::KokoroError;
use crate::tts::config::{ProviderConfig, TtsSystemConfig};
use async_trait::async_trait;
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
struct TestBackend {
    state: Arc<Mutex<BackendRuntimeSnapshot>>,
    fail_next_apply: Arc<Mutex<bool>>,
}

#[async_trait]
impl ActivationRuntimeBackend for TestBackend {
    async fn snapshot(&self) -> Result<BackendRuntimeSnapshot, KokoroError> {
        Ok(self.state.lock().await.clone())
    }

    async fn apply(&self, snapshot: &BackendRuntimeSnapshot) -> Result<(), KokoroError> {
        if std::mem::take(&mut *self.fail_next_apply.lock().await) {
            return Err(KokoroError::Internal(
                "injected backend apply failure".into(),
            ));
        }
        *self.state.lock().await = snapshot.clone();
        Ok(())
    }

    async fn restore(&self, snapshot: &BackendRuntimeSnapshot) -> Result<(), KokoroError> {
        *self.state.lock().await = snapshot.clone();
        Ok(())
    }
}

async fn pool() -> SqlitePool {
    let orchestrator = AIOrchestrator::new("sqlite::memory:").await.unwrap();
    orchestrator.db.clone()
}

fn provider(id: &str, provider_type: &str) -> ProviderConfig {
    ProviderConfig {
        id: id.into(),
        provider_type: provider_type.into(),
        enabled: true,
        api_key: Some("must-never-leak".into()),
        api_key_env: None,
        base_url: Some("https://private.example".into()),
        endpoint: None,
        model: None,
        default_voice: Some("configured-default".into()),
        model_path: Some("C:/secret/model".into()),
        extra: HashMap::new(),
    }
}

fn config(providers: Vec<ProviderConfig>, default_provider: Option<&str>) -> TtsSystemConfig {
    TtsSystemConfig {
        providers,
        default_provider: default_provider.map(str::to_owned),
        ..TtsSystemConfig::default()
    }
}

async fn insert_character(pool: &SqlitePool, id: &str, greeting: &str, runtime: serde_json::Value) {
    create_character_in_pool(
        pool,
        CreateCharacterRequest {
            id: id.into(),
            name: format!("Name {id}"),
            persona: format!("Persona {id}"),
            user_nickname: "Owner".into(),
            source_format: "test".into(),
            created_at: 1,
            updated_at: 1,
            template_id: None,
            template_version: None,
            template_snapshot_json: None,
            description: String::new(),
            avatar_path: None,
            greeting: greeting.into(),
            example_dialogue: "Example".into(),
            runtime_profile_json: runtime.to_string(),
            user_modified_at: None,
        },
    )
    .await
    .unwrap();
}

async fn attach_template_snapshot(
    pool: &SqlitePool,
    character_id: &str,
    assets: Option<serde_json::Value>,
) {
    let snapshot = json!({
        "schema_version": 1,
        "engine_version": ">=0.3.0, <0.4.0",
        "id": "template-origin",
        "version": "1.0.0",
        "name": "Template",
        "description": "Template description",
        "author": "Test",
        "license": "CC0-1.0",
        "persona": "Template persona",
        "greeting": "Hello",
        "assets": assets,
    });
    sqlx::query(
        "UPDATE characters SET source_format = 'template', template_id = 'template-origin', \
         template_version = '1.0.0', template_snapshot_json = ? WHERE id = ?",
    )
    .bind(snapshot.to_string())
    .bind(character_id)
    .execute(pool)
    .await
    .unwrap();
}

fn template_snapshot(assets: Option<serde_json::Value>) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "engine_version": ">=0.3.0, <0.4.0",
        "id": "template-origin",
        "version": "1.0.0",
        "name": "Template",
        "description": "Template description",
        "author": "Test",
        "license": "CC0-1.0",
        "persona": "Template persona",
        "greeting": "Hello",
        "assets": assets,
    })
}

async fn attach_snapshot_value(
    pool: &SqlitePool,
    character_id: &str,
    snapshot: &serde_json::Value,
) {
    sqlx::query(
        "UPDATE characters SET source_format = 'template', template_id = 'template-origin', \
         template_version = '1.0.0', template_snapshot_json = ? WHERE id = ?",
    )
    .bind(snapshot.to_string())
    .bind(character_id)
    .execute(pool)
    .await
    .unwrap();
}

fn create_installed_package(
    catalog_root: &Path,
    snapshot: &serde_json::Value,
    files: &[(&str, &[u8])],
) -> PathBuf {
    let package_dir = catalog_root.join("template-origin").join("1.0.0");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(package_dir.join("character.json"), snapshot.to_string()).unwrap();
    fs::write(package_dir.join("LICENSE.md"), "test license").unwrap();
    for (relative, content) in files {
        let path = package_dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    package_dir
}

fn presets() -> Vec<LocalTtsPreset> {
    vec![LocalTtsPreset {
        id: "gpt-sovits-loopback".into(),
        provider_type: "gpt_sovits".into(),
        endpoint: "http://127.0.0.1:9880".into(),
    }]
}

#[tokio::test]
async fn instance_runtime_overrides_previous_committed_optional_values() {
    let pool = pool().await;
    insert_character(
        &pool,
        "next",
        "Hi",
        json!({
            "response_language": "ja",
            "proactive_enabled": false
        }),
    )
    .await;
    let backend = TestBackend::default();
    *backend.state.lock().await = BackendRuntimeSnapshot {
        response_language: "en".into(),
        proactive_enabled: true,
        ..Default::default()
    };

    let token = ActivationCoordinator::default()
        .prepare(&pool, "next", &config(vec![], None), &presets(), &backend)
        .await
        .unwrap();

    assert_eq!(token.resolved_runtime.response_language, "ja");
    assert!(!token.resolved_runtime.proactive_enabled);
    assert_eq!(token.previous_committed.response_language, "en");
}

#[tokio::test]
async fn tts_resolution_prefers_matching_provider_id_over_type_default_and_preset() {
    let pool = pool().await;
    insert_character(
        &pool,
        "next",
        "",
        json!({"tts": {
            "provider_id": "edge-secondary", "provider_type": "edge_tts",
            "local_preset": "gpt-sovits-loopback", "voice": "voice-a"
        }}),
    )
    .await;
    let cfg = config(
        vec![
            provider("edge-default", "edge_tts"),
            provider("edge-secondary", "edge_tts"),
        ],
        Some("edge-default"),
    );

    let token = ActivationCoordinator::default()
        .prepare(&pool, "next", &cfg, &presets(), &TestBackend::default())
        .await
        .unwrap();

    assert_eq!(
        token.resolved_runtime.tts.mode,
        ResolvedTtsMode::ConfiguredProvider
    );
    assert_eq!(
        token.resolved_runtime.tts.provider_id.as_deref(),
        Some("edge-secondary")
    );
    let serialized = serde_json::to_string(&token).unwrap();
    assert!(!serialized.contains("must-never-leak"));
    assert!(!serialized.contains("private.example"));
    assert!(!serialized.contains("C:/secret/model"));
}

#[tokio::test]
async fn tts_resolution_uses_configured_default_of_matching_type() {
    let pool = pool().await;
    insert_character(
        &pool,
        "next",
        "",
        json!({"tts": {"provider_type": "edge_tts"}}),
    )
    .await;
    let cfg = config(
        vec![
            provider("other", "edge_tts"),
            provider("default-edge", "edge_tts"),
        ],
        Some("default-edge"),
    );

    let token = ActivationCoordinator::default()
        .prepare(&pool, "next", &cfg, &[], &TestBackend::default())
        .await
        .unwrap();

    assert_eq!(
        token.resolved_runtime.tts.provider_id.as_deref(),
        Some("default-edge")
    );
}

#[tokio::test]
async fn tts_resolution_offers_allowlisted_local_preset_without_saving_it() {
    let pool = pool().await;
    insert_character(
        &pool,
        "next",
        "",
        json!({"tts": {"provider_type": "gpt_sovits", "local_preset": "gpt-sovits-loopback"}}),
    )
    .await;

    let token = ActivationCoordinator::default()
        .prepare(
            &pool,
            "next",
            &config(vec![], None),
            &presets(),
            &TestBackend::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        token.resolved_runtime.tts.mode,
        ResolvedTtsMode::LocalPresetConfirmation
    );
    assert_eq!(
        token.resolved_runtime.tts.endpoint.as_deref(),
        Some("http://127.0.0.1:9880")
    );
    assert!(token.resolved_runtime.tts.requires_save_confirmation);
}

#[tokio::test]
async fn tts_resolution_falls_back_to_browser_then_text_only() {
    let pool = pool().await;
    insert_character(&pool, "browser-char", "", json!({})).await;
    insert_character(&pool, "text-char", "", json!({})).await;
    let coordinator = ActivationCoordinator::default();
    let browser = coordinator
        .prepare(
            &pool,
            "browser-char",
            &config(vec![provider("browser", "browser")], Some("browser")),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();
    let text = coordinator
        .prepare(
            &pool,
            "text-char",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();

    assert_eq!(browser.resolved_runtime.tts.mode, ResolvedTtsMode::Browser);
    assert_eq!(text.resolved_runtime.tts.mode, ResolvedTtsMode::TextOnly);
}

#[tokio::test]
async fn prepare_includes_validated_template_asset_references() {
    let pool = pool().await;
    insert_character(&pool, "templated", "", json!({})).await;
    let snapshot = template_snapshot(Some(json!({
        "live2d_model": "models/template.model3.json",
        "background": "backgrounds/template.webp",
        "cue_profile": "cues/template.json"
    })));
    attach_snapshot_value(&pool, "templated", &snapshot).await;
    let temp = tempfile::tempdir().unwrap();
    let package_dir = create_installed_package(
        temp.path(),
        &snapshot,
        &[
            ("models/template.model3.json", b"{}"),
            ("backgrounds/template.webp", b"image"),
            ("cues/template.json", br#"{"schema_version":1,"cues":{}}"#),
        ],
    );

    let token = ActivationCoordinator::default()
        .prepare_with_package_root(
            &pool,
            temp.path(),
            "templated",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();

    let expected = |relative: &str| {
        json!({
            "source": "package",
            "template_id": "template-origin",
            "template_version": "1.0.0",
            "path": package_dir.join(relative).canonicalize().unwrap(),
        })
    };
    assert_eq!(
        json!(token.resolved_runtime.live2d_model),
        expected("models/template.model3.json")
    );
    assert_eq!(
        json!(token.resolved_runtime.background),
        expected("backgrounds/template.webp")
    );
    assert_eq!(
        json!(token.resolved_runtime.cue_profile),
        expected("cues/template.json")
    );
}

#[tokio::test]
async fn prepare_falls_back_when_optional_package_assets_are_missing_or_origin_collides() {
    let pool = pool().await;
    insert_character(&pool, "missing", "", json!({})).await;
    insert_character(&pool, "collision", "", json!({})).await;
    let snapshot = template_snapshot(Some(json!({
        "live2d_model": "models/missing.model3.json",
        "background": "backgrounds/missing.webp",
        "cue_profile": "cues/missing.json"
    })));
    attach_snapshot_value(&pool, "missing", &snapshot).await;
    attach_snapshot_value(&pool, "collision", &snapshot).await;

    let missing_root = tempfile::tempdir().unwrap();
    create_installed_package(missing_root.path(), &snapshot, &[]);
    let missing = ActivationCoordinator::default()
        .prepare_with_package_root(
            &pool,
            missing_root.path(),
            "missing",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();
    assert_eq!(missing.resolved_runtime.live2d_model, None);
    assert_eq!(missing.resolved_runtime.background, None);
    assert_eq!(missing.resolved_runtime.cue_profile, None);

    let collision_root = tempfile::tempdir().unwrap();
    let mut wrong_manifest = snapshot.clone();
    wrong_manifest["id"] = json!("different-template");
    create_installed_package(collision_root.path(), &wrong_manifest, &[]);
    let collision = ActivationCoordinator::default()
        .prepare_with_package_root(
            &pool,
            collision_root.path(),
            "collision",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();
    assert_eq!(collision.resolved_runtime.live2d_model, None);
    assert_eq!(collision.resolved_runtime.background, None);
    assert_eq!(collision.resolved_runtime.cue_profile, None);
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_falls_back_when_a_package_asset_is_a_symlink_escape() {
    use std::os::unix::fs::symlink;

    let pool = pool().await;
    insert_character(&pool, "escaped", "", json!({})).await;
    let snapshot = template_snapshot(Some(json!({ "cue_profile": "cues.json" })));
    attach_snapshot_value(&pool, "escaped", &snapshot).await;
    let temp = tempfile::tempdir().unwrap();
    let package_dir = create_installed_package(temp.path(), &snapshot, &[]);
    let outside = temp.path().join("outside.json");
    fs::write(&outside, br#"{"schema_version":1,"cues":{}}"#).unwrap();
    symlink(&outside, package_dir.join("cues.json")).unwrap();

    let token = ActivationCoordinator::default()
        .prepare_with_package_root(
            &pool,
            temp.path(),
            "escaped",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();

    assert_eq!(token.resolved_runtime.cue_profile, None);
}

#[cfg(windows)]
#[tokio::test]
async fn prepare_falls_back_when_a_package_asset_parent_is_a_junction_escape() {
    use std::process::Command;

    let pool = pool().await;
    insert_character(&pool, "escaped", "", json!({})).await;
    let snapshot = template_snapshot(Some(json!({ "cue_profile": "redirected/cues.json" })));
    attach_snapshot_value(&pool, "escaped", &snapshot).await;
    let temp = tempfile::tempdir().unwrap();
    let package_dir = create_installed_package(temp.path(), &snapshot, &[]);
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(
        outside.join("cues.json"),
        br#"{"schema_version":1,"cues":{}}"#,
    )
    .unwrap();
    let redirected = package_dir.join("redirected");
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&redirected)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(output.status.success(), "mklink /J failed");

    let token = ActivationCoordinator::default()
        .prepare_with_package_root(
            &pool,
            temp.path(),
            "escaped",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();

    assert_eq!(token.resolved_runtime.cue_profile, None);
    fs::remove_dir(&redirected).unwrap();
}

#[tokio::test]
async fn prepare_falls_back_when_the_cue_profile_is_not_valid_json() {
    let pool = pool().await;
    insert_character(&pool, "bad-cues", "", json!({})).await;
    let snapshot = template_snapshot(Some(json!({ "cue_profile": "cues.json" })));
    attach_snapshot_value(&pool, "bad-cues", &snapshot).await;
    let temp = tempfile::tempdir().unwrap();
    create_installed_package(temp.path(), &snapshot, &[("cues.json", b"not-json")]);

    let token = ActivationCoordinator::default()
        .prepare_with_package_root(
            &pool,
            temp.path(),
            "bad-cues",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();

    assert_eq!(token.resolved_runtime.cue_profile, None);
}

#[tokio::test]
async fn prepare_uses_optional_asset_fallbacks_for_manual_and_assetless_instances() {
    let pool = pool().await;
    insert_character(&pool, "manual", "", json!({})).await;
    insert_character(&pool, "assetless", "", json!({})).await;
    attach_template_snapshot(&pool, "assetless", None).await;
    let coordinator = ActivationCoordinator::default();

    for character_id in ["manual", "assetless"] {
        let token = coordinator
            .prepare(
                &pool,
                character_id,
                &config(vec![], None),
                &[],
                &TestBackend::default(),
            )
            .await
            .unwrap();
        assert_eq!(token.resolved_runtime.live2d_model, None);
        assert_eq!(token.resolved_runtime.background, None);
        assert_eq!(token.resolved_runtime.cue_profile, None);
    }
}

#[tokio::test]
async fn prepare_rejects_unsafe_template_asset_references() {
    let pool = pool().await;
    insert_character(&pool, "unsafe", "", json!({})).await;
    attach_template_snapshot(
        &pool,
        "unsafe",
        Some(json!({
            "live2d_model": "../outside.model3.json",
            "background": null,
            "cue_profile": null
        })),
    )
    .await;

    let error = ActivationCoordinator::default()
        .prepare(
            &pool,
            "unsafe",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("unsafe package path"));
}

#[tokio::test]
async fn committed_runtime_recovery_retains_template_asset_references() {
    let pool = pool().await;
    insert_character(&pool, "templated", "", json!({})).await;
    let snapshot = template_snapshot(Some(json!({
        "live2d_model": "models/recovered.model3.json",
        "background": "backgrounds/recovered.webp",
        "cue_profile": "cues/recovered.json"
    })));
    attach_snapshot_value(&pool, "templated", &snapshot).await;
    let temp = tempfile::tempdir().unwrap();
    let package_dir = create_installed_package(
        temp.path(),
        &snapshot,
        &[
            ("models/recovered.model3.json", b"{}"),
            ("backgrounds/recovered.webp", b"image"),
            ("cues/recovered.json", br#"{"schema_version":1,"cues":{}}"#),
        ],
    );
    let coordinator = ActivationCoordinator::default();
    let backend = TestBackend::default();
    let token = coordinator
        .prepare_with_package_root(
            &pool,
            temp.path(),
            "templated",
            &config(vec![], None),
            &[],
            &backend,
        )
        .await
        .unwrap();
    coordinator.commit(&pool, token, &backend).await.unwrap();

    let recovered = coordinator.get_committed(&pool).await.unwrap().unwrap();

    let expected = |relative: &str| {
        json!({
            "source": "package",
            "template_id": "template-origin",
            "template_version": "1.0.0",
            "path": package_dir.join(relative).canonicalize().unwrap(),
        })
    };
    assert_eq!(
        json!(recovered.runtime.live2d_model),
        expected("models/recovered.model3.json")
    );
    assert_eq!(
        json!(recovered.runtime.background),
        expected("backgrounds/recovered.webp")
    );
    assert_eq!(
        json!(recovered.runtime.cue_profile),
        expected("cues/recovered.json")
    );
}

#[tokio::test]
async fn prepare_selects_only_a_conversation_owned_by_the_character() {
    let pool = pool().await;
    insert_character(&pool, "first", "", json!({})).await;
    insert_character(&pool, "second", "", json!({})).await;
    for (id, owner, updated) in [
        ("foreign", "first", "2026-01-02"),
        ("owned", "second", "2026-01-01"),
    ] {
        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES (?, ?, 'Chat', ?, ?)")
            .bind(id).bind(owner).bind(updated).bind(updated).execute(&pool).await.unwrap();
    }

    let token = ActivationCoordinator::default()
        .prepare(
            &pool,
            "second",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();

    assert_eq!(token.target_conversation_id.as_deref(), Some("owned"));
}

#[tokio::test]
async fn empty_greeting_is_consumed_without_emitting_and_cannot_reactivate_after_edit() {
    let pool = pool().await;
    insert_character(&pool, "empty", "   ", json!({})).await;
    let coordinator = ActivationCoordinator::default();
    let backend = TestBackend::default();

    let token = coordinator
        .prepare(&pool, "empty", &config(vec![], None), &[], &backend)
        .await
        .unwrap();
    assert_eq!(token.greeting_action, GreetingAction::ConsumeWithoutEmit);
    coordinator.commit(&pool, token, &backend).await.unwrap();
    let consumed_at: Option<i64> =
        sqlx::query_scalar("SELECT greeting_consumed_at FROM characters WHERE id = 'empty'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let message_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(consumed_at.is_some());
    assert_eq!(message_count, 0);

    sqlx::query(
        "UPDATE characters SET greeting = 'Edited later', updated_at = 2 WHERE id = 'empty'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let after_edit = coordinator
        .prepare(&pool, "empty", &config(vec![], None), &[], &backend)
        .await
        .unwrap();

    assert_eq!(after_edit.greeting_action, GreetingAction::None);
}

#[tokio::test]
async fn consumed_or_deleted_greetings_are_not_staged() {
    let pool = pool().await;
    insert_character(&pool, "consumed", "Once", json!({})).await;
    sqlx::query("UPDATE characters SET greeting_consumed_at = 10, greeting_message_id = 999 WHERE id = 'consumed'").execute(&pool).await.unwrap();

    let token = ActivationCoordinator::default()
        .prepare(
            &pool,
            "consumed",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();

    assert_eq!(token.greeting_action, GreetingAction::None);
}

#[tokio::test]
async fn greeting_is_committed_exactly_once_to_the_owned_conversation() {
    let pool = pool().await;
    insert_character(&pool, "next", "Hello once", json!({})).await;
    let coordinator = ActivationCoordinator::default();
    let backend = TestBackend::default();
    let token = coordinator
        .prepare(&pool, "next", &config(vec![], None), &[], &backend)
        .await
        .unwrap();

    let committed = coordinator.commit(&pool, token, &backend).await.unwrap();
    let again = coordinator
        .prepare(&pool, "next", &config(vec![], None), &[], &backend)
        .await
        .unwrap();

    assert_eq!(again.greeting_action, GreetingAction::None);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages WHERE conversation_id = ? AND content = 'Hello once'")
        .bind(&committed.target_conversation_id).fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn canonical_prompt_composes_persona_and_example_dialogue_for_backend_application() {
    let pool = pool().await;
    insert_character(&pool, "prompted", "", json!({})).await;
    let coordinator = ActivationCoordinator::default();
    let backend = TestBackend::default();

    let token = coordinator
        .prepare(&pool, "prompted", &config(vec![], None), &[], &backend)
        .await
        .unwrap();
    assert!(token
        .resolved_runtime
        .system_prompt
        .contains("<character_persona>"));
    assert!(token
        .resolved_runtime
        .system_prompt
        .contains("Persona prompted"));
    assert!(token
        .resolved_runtime
        .system_prompt
        .contains("<example_dialogue>"));
    assert!(token.resolved_runtime.system_prompt.contains("Example"));

    coordinator.commit(&pool, token, &backend).await.unwrap();
    let applied = backend.snapshot().await.unwrap();
    assert!(applied.system_prompt.contains("<character_persona>"));
    assert!(applied.system_prompt.contains("<example_dialogue>"));
}

#[tokio::test]
async fn commit_uses_server_owned_runtime_when_the_returned_token_is_tampered() {
    let pool = pool().await;
    insert_character(&pool, "original", "", json!({})).await;
    insert_character(&pool, "attacker-choice", "", json!({})).await;
    let coordinator = ActivationCoordinator::default();
    let backend = TestBackend::default();
    let prepared = coordinator
        .prepare(&pool, "original", &config(vec![], None), &[], &backend)
        .await
        .unwrap();
    assert!(!prepared.nonce.trim().is_empty());

    let mut tampered_json = serde_json::to_value(&prepared).unwrap();
    tampered_json["resolved_runtime"]["character_id"] = json!("attacker-choice");
    tampered_json["resolved_runtime"]["system_prompt"] = json!("attacker prompt");
    tampered_json["resolved_runtime"]["tts"]["endpoint"] = json!("https://attacker.example/tts");
    tampered_json["resolved_runtime"]["live2d_model"] = json!({
        "source": "package",
        "template_id": "attacker",
        "template_version": "9.9.9",
        "path": "C:/outside/attacker.model3.json"
    });
    tampered_json["prompt"]["persona"] = json!("attacker persona");
    let tampered = serde_json::from_value(tampered_json).unwrap();

    let committed = coordinator.commit(&pool, tampered, &backend).await.unwrap();

    assert_eq!(committed.runtime.character_id, "original");
    assert_ne!(committed.runtime.system_prompt, "attacker prompt");
    assert_eq!(committed.runtime.tts.endpoint, None);
    assert_eq!(committed.runtime.live2d_model, None);
}

#[tokio::test]
async fn commit_uses_server_owned_greeting_and_conversation_when_token_is_tampered() {
    let pool = pool().await;
    insert_character(&pool, "original", "Original hello", json!({})).await;
    insert_character(&pool, "foreign", "", json!({})).await;
    sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('foreign-chat', 'foreign', 'Chat', '1', '1')")
        .execute(&pool)
        .await
        .unwrap();
    let coordinator = ActivationCoordinator::default();
    let backend = TestBackend::default();
    let prepared = coordinator
        .prepare(&pool, "original", &config(vec![], None), &[], &backend)
        .await
        .unwrap();
    let mut tampered = prepared.clone();
    tampered.target_conversation_id = Some("foreign-chat".into());
    tampered.greeting_action = GreetingAction::Emit {
        content: "Forged hello".into(),
    };

    let committed = coordinator.commit(&pool, tampered, &backend).await.unwrap();
    let messages: Vec<String> = sqlx::query_scalar(
        "SELECT content FROM conversation_messages WHERE conversation_id = ? ORDER BY id",
    )
    .bind(&committed.target_conversation_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_ne!(committed.target_conversation_id, "foreign-chat");
    assert_eq!(messages, vec!["Original hello"]);
}

#[tokio::test]
async fn backend_failure_restores_server_owned_previous_snapshot_not_tampered_rollback() {
    let pool = pool().await;
    insert_character(&pool, "next", "", json!({})).await;
    let coordinator = ActivationCoordinator::default();
    let backend = TestBackend::default();
    let original = BackendRuntimeSnapshot {
        character_id: "old".into(),
        system_prompt: "old prompt".into(),
        ..Default::default()
    };
    *backend.state.lock().await = original.clone();
    let mut token = coordinator
        .prepare(&pool, "next", &config(vec![], None), &[], &backend)
        .await
        .unwrap();
    token.previous_committed = BackendRuntimeSnapshot {
        character_id: "attacker".into(),
        system_prompt: "attacker rollback".into(),
        ..Default::default()
    };
    *backend.fail_next_apply.lock().await = true;

    coordinator
        .commit(&pool, token, &backend)
        .await
        .unwrap_err();

    assert_eq!(*backend.state.lock().await, original);
}

#[tokio::test]
async fn committed_runtime_recovery_reapplies_the_authoritative_snapshot_idempotently() {
    let pool = pool().await;
    insert_character(&pool, "recover", "", json!({})).await;
    let coordinator = ActivationCoordinator::default();
    let initial_backend = TestBackend::default();
    let token = coordinator
        .prepare(
            &pool,
            "recover",
            &config(vec![], None),
            &[],
            &initial_backend,
        )
        .await
        .unwrap();
    let committed = coordinator
        .commit(&pool, token, &initial_backend)
        .await
        .unwrap();
    let recovered_backend = TestBackend::default();
    let restarted = ActivationCoordinator::default();

    let first = restarted
        .recover_committed(&pool, &recovered_backend)
        .await
        .unwrap()
        .unwrap();
    let second = restarted
        .recover_committed(&pool, &recovered_backend)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first, committed);
    assert_eq!(second, committed);
    assert_eq!(
        recovered_backend.snapshot().await.unwrap(),
        committed.runtime
    );
}

#[tokio::test]
async fn concurrent_prepare_revisions_are_monotonic_and_stale_tokens_are_rejected() {
    let pool = pool().await;
    insert_character(&pool, "first", "", json!({})).await;
    insert_character(&pool, "second", "", json!({})).await;
    let coordinator = Arc::new(ActivationCoordinator::default());
    let backend = TestBackend::default();
    let cfg = config(vec![], None);
    let (first, second) = tokio::join!(
        coordinator.prepare(&pool, "first", &cfg, &[], &backend),
        coordinator.prepare(&pool, "second", &cfg, &[], &backend),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_ne!(first.revision, second.revision);
    let (stale, current) = if first.revision < second.revision {
        (first, second)
    } else {
        (second, first)
    };

    let error = coordinator
        .commit(&pool, stale, &backend)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("stale activation token"));
    coordinator.commit(&pool, current, &backend).await.unwrap();
}

#[tokio::test]
async fn activation_revision_remains_monotonic_after_coordinator_recreation() {
    let pool = pool().await;
    insert_character(&pool, "first", "", json!({})).await;
    insert_character(&pool, "second", "", json!({})).await;
    let backend = TestBackend::default();
    let first_coordinator = ActivationCoordinator::default();
    let first = first_coordinator
        .prepare(&pool, "first", &config(vec![], None), &[], &backend)
        .await
        .unwrap();
    let first_revision = first.revision;
    first_coordinator
        .commit(&pool, first, &backend)
        .await
        .unwrap();

    let after_restart = ActivationCoordinator::default()
        .prepare(&pool, "second", &config(vec![], None), &[], &backend)
        .await
        .unwrap();

    assert!(after_restart.revision > first_revision);
}

#[tokio::test]
async fn backend_failure_rolls_back_runtime_and_leaves_greeting_unconsumed() {
    let pool = pool().await;
    insert_character(
        &pool,
        "next",
        "Retry me",
        json!({"response_language": "ja"}),
    )
    .await;
    let coordinator = ActivationCoordinator::default();
    let backend = TestBackend::default();
    let original = BackendRuntimeSnapshot {
        character_id: "old".into(),
        response_language: "en".into(),
        ..Default::default()
    };
    *backend.state.lock().await = original.clone();
    let token = coordinator
        .prepare(&pool, "next", &config(vec![], None), &[], &backend)
        .await
        .unwrap();
    *backend.fail_next_apply.lock().await = true;

    coordinator
        .commit(&pool, token, &backend)
        .await
        .unwrap_err();

    assert_eq!(*backend.state.lock().await, original);
    let consumed: Option<i64> =
        sqlx::query_scalar("SELECT greeting_consumed_at FROM characters WHERE id = 'next'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(consumed, None);
    assert!(coordinator.get_committed(&pool).await.unwrap().is_none());
}
