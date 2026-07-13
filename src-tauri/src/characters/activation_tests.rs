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
async fn empty_and_consumed_or_deleted_greetings_are_not_staged() {
    let pool = pool().await;
    insert_character(&pool, "empty", "   ", json!({})).await;
    insert_character(&pool, "consumed", "Once", json!({})).await;
    sqlx::query("UPDATE characters SET greeting_consumed_at = 10, greeting_message_id = 999 WHERE id = 'consumed'").execute(&pool).await.unwrap();
    let coordinator = ActivationCoordinator::default();

    let empty = coordinator
        .prepare(
            &pool,
            "empty",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();
    let consumed = coordinator
        .prepare(
            &pool,
            "consumed",
            &config(vec![], None),
            &[],
            &TestBackend::default(),
        )
        .await
        .unwrap();

    assert_eq!(empty.greeting_action, GreetingAction::None);
    assert_eq!(consumed.greeting_action, GreetingAction::None);
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
