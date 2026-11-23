use super::{
    load_bot_config_from_paths, save_bot_config_file_to_path, BotConfig, BotRuntimeService,
};
use crate::error::KokoroError;
use crate::lifecycle::{AppLifecycle, TaskStage};
use crate::qqbot::protocol::{QQConversationKind, QQInboundMessage};
use crate::telegram::TelegramService;
use std::{fs, path::Path, path::PathBuf, pin::pin};

const REVOKED_CHAT: i64 = 123;
const RETAINED_CHAT: i64 = -1001234567890;
const OTHER_CHAT: i64 = 456;
const QQ_REQUEST: &str = "new-qq-request";
const QQ_USER: &str = "new-qq-user";
const QQ_GROUP: &str = "new-qq-group";

struct Fixture {
    runtime: BotRuntimeService,
    telegram: TelegramService,
    path: PathBuf,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bot_config.json");
        let mut config = BotConfig {
            revision: 40,
            selected_platform: "line".into(),
            ..Default::default()
        };
        config.telegram.enabled = true;
        config.telegram.bot_token = Some("telegram-fixture-token".into());
        config.telegram.allowed_chat_ids = vec![REVOKED_CHAT, RETAINED_CHAT];
        config.telegram.send_voice_reply = true;
        config.telegram.character_id = Some("telegram-character".into());
        config.qq.enabled = true;
        config.qq.app_id = Some("qq-fixture-id".into());
        config.qq.allowed_user_openids = vec!["existing-qq-user".into()];
        config.qq.allowed_group_openids = vec!["existing-qq-group".into()];
        config.discord.allowed_channel_ids = vec!["existing-discord-channel".into()];
        config.line.webhook_path = "/custom/line".into();
        config.line.allowed_user_ids = vec!["existing-line-user".into()];
        config.webhook.port = 4040;
        config.webhook.endpoint_path = "/custom/bot".into();
        save_bot_config_file_to_path(&path, &config).unwrap();

        let (runtime, telegram) = initialize_from_disk(&path);
        Self {
            runtime,
            telegram,
            path,
            _directory: directory,
        }
    }

    fn persisted(&self) -> BotConfig {
        serde_json::from_slice(&fs::read(&self.path).unwrap()).unwrap()
    }

    async fn assert_current(&self, expected: &BotConfig) {
        assert_eq!(self.persisted(), *expected);
        assert_eq!(self.runtime.current_config().await, *expected);
        assert_eq!(self.telegram.get_config().await, expected.telegram);
    }
}

fn initialize_from_disk(path: &Path) -> (BotRuntimeService, TelegramService) {
    let legacy_path = path.with_file_name("telegram_config.json");
    let config = load_bot_config_from_paths(path, &legacy_path);
    let lifecycle = AppLifecycle::new();
    let work = lifecycle.scope(TaskStage::Work, "bot config regression");
    let service = lifecycle.scope(TaskStage::Service, "bot config regression");
    let runtime =
        BotRuntimeService::with_config_path(config, work.clone(), service.clone(), path.into());
    let telegram = TelegramService::new(&runtime, work, service);
    (runtime, telegram)
}

async fn pending_group_request(runtime: &BotRuntimeService) -> bool {
    let message = QQInboundMessage {
        kind: QQConversationKind::Group,
        message_id: QQ_REQUEST.into(),
        content: "local test prompt".into(),
        user_openid: QQ_USER.into(),
        group_openid: Some(QQ_GROUP.into()),
    };
    runtime
        .qq_authorization_broker
        .request_for_test(&message)
        .await
}

#[tokio::test]
async fn telegram_revocation_and_qq_approval_survive_reload() {
    let fixture = Fixture::new();
    let initial = fixture.runtime.current_config().await;
    let (telegram_revision, mut telegram) = fixture.runtime.current_telegram_config().await;
    let candidates = [REVOKED_CHAT, RETAINED_CHAT, OTHER_CHAT];
    fixture
        .telegram
        .assert_dispatched_text_chats(&candidates, &[REVOKED_CHAT, RETAINED_CHAT])
        .await;

    let mut expected = initial.clone();
    telegram.allowed_chat_ids = vec![RETAINED_CHAT];
    expected.telegram = telegram.clone();
    expected.revision += 1;
    let saved = fixture
        .runtime
        .save_telegram_config(telegram_revision, telegram)
        .await
        .unwrap();
    assert_eq!(saved, expected);
    fixture.assert_current(&expected).await;

    let mut pending = pin!(pending_group_request(&fixture.runtime));
    assert!(futures::poll!(pending.as_mut()).is_pending());
    let approved = fixture
        .runtime
        .respond_to_qq_authorization(QQ_REQUEST, QQ_USER, Some(QQ_GROUP), true)
        .await
        .unwrap();
    assert!(pending.await);
    expected.revision += 1;
    expected.qq.allowed_user_openids.push(QQ_USER.into());
    expected.qq.allowed_group_openids.push(QQ_GROUP.into());
    assert_eq!(
        approved,
        (
            expected.revision,
            Some(QQ_USER.into()),
            Some(QQ_GROUP.into())
        )
    );
    fixture.assert_current(&expected).await;

    let mut updated = expected.clone();
    updated.webhook.port = 4041;
    let saved = fixture.runtime.save_config(updated.clone()).await.unwrap();
    updated.revision += 1;
    assert_eq!(saved, updated);
    fixture.assert_current(&updated).await;

    let (reloaded, reloaded_telegram) = initialize_from_disk(&fixture.path);
    assert_eq!(reloaded.current_config().await, updated);
    assert_eq!(reloaded_telegram.get_config().await, updated.telegram);
    reloaded_telegram
        .assert_dispatched_text_chats(&candidates, &[RETAINED_CHAT])
        .await;
}

#[tokio::test]
async fn queued_telegram_and_qq_edits_reject_a_stale_full_config_save() {
    let fixture = Fixture::new();
    let mut stale = fixture.runtime.current_config().await;
    let (telegram_revision, mut telegram) = fixture.runtime.current_telegram_config().await;
    let mut expected = stale.clone();
    telegram.allowed_chat_ids = vec![RETAINED_CHAT];
    expected.telegram = telegram.clone();
    stale.selected_platform = "discord".into();

    let mut pending = pin!(pending_group_request(&fixture.runtime));
    assert!(futures::poll!(pending.as_mut()).is_pending());

    // Queue the real entry points on the owner's lock in a known order.
    // Each future remains alive and keeps its place until the lock is released.
    let hold_writes = fixture.runtime.config.write().await;
    let mut revoke = pin!(fixture
        .runtime
        .save_telegram_config(telegram_revision, telegram));
    let mut approve = pin!(fixture.runtime.respond_to_qq_authorization(
        QQ_REQUEST,
        QQ_USER,
        Some(QQ_GROUP),
        true,
    ));
    let mut save_stale = pin!(fixture.runtime.save_config(stale));
    assert!(futures::poll!(revoke.as_mut()).is_pending());
    assert!(futures::poll!(approve.as_mut()).is_pending());
    assert!(futures::poll!(save_stale.as_mut()).is_pending());
    assert_eq!(fixture.persisted(), *hold_writes);
    drop(hold_writes);

    expected.revision += 1;
    let after_revocation = expected.clone();
    expected.revision += 1;
    expected.qq.allowed_user_openids.push(QQ_USER.into());
    expected.qq.allowed_group_openids.push(QQ_GROUP.into());

    let (_, _, stale_result) = tokio::join!(
        async {
            assert_eq!(revoke.await.unwrap(), after_revocation);
            assert_eq!(fixture.persisted(), after_revocation);
        },
        async {
            assert_eq!(approve.await.unwrap().0, expected.revision);
            assert_eq!(fixture.persisted(), expected);
        },
        save_stale,
    );
    assert!(pending.await);

    let error = stale_result.expect_err("stale save must conflict");
    assert!(matches!(error, KokoroError::BotConfigConflict(_)));
    assert_eq!(
        serde_json::to_value(error).unwrap()["code"],
        "BotConfigConflict"
    );
    fixture.assert_current(&expected).await;
    fixture
        .telegram
        .assert_dispatched_text_chats(&[REVOKED_CHAT, RETAINED_CHAT, OTHER_CHAT], &[RETAINED_CHAT])
        .await;
}

#[tokio::test]
async fn telegram_and_full_config_saves_preserve_both_edits_after_retry_in_either_order() {
    for telegram_first in [true, false] {
        let fixture = Fixture::new();
        let mut character_edit = fixture.runtime.current_config().await;
        let (voice_revision, mut voice_edit) = fixture.runtime.current_telegram_config().await;
        assert_eq!(character_edit.revision, voice_revision);
        character_edit.telegram.character_id = Some("another-character".into());
        voice_edit.send_voice_reply = false;

        let saved = if telegram_first {
            let first = fixture
                .runtime
                .save_telegram_config(voice_revision, voice_edit)
                .await
                .unwrap();
            assert!(matches!(
                fixture.runtime.save_config(character_edit).await,
                Err(KokoroError::BotConfigConflict(_))
            ));
            fixture.assert_current(&first).await;

            let mut refreshed = fixture.runtime.current_config().await;
            refreshed.telegram.character_id = Some("another-character".into());
            fixture.runtime.save_config(refreshed).await.unwrap()
        } else {
            let first = fixture.runtime.save_config(character_edit).await.unwrap();
            assert!(matches!(
                fixture
                    .runtime
                    .save_telegram_config(voice_revision, voice_edit)
                    .await,
                Err(KokoroError::BotConfigConflict(_))
            ));
            fixture.assert_current(&first).await;

            let (revision, mut refreshed) = fixture.runtime.current_telegram_config().await;
            refreshed.send_voice_reply = false;
            fixture
                .runtime
                .save_telegram_config(revision, refreshed)
                .await
                .unwrap()
        };

        assert_eq!(saved.revision, voice_revision + 2);
        assert!(!saved.telegram.send_voice_reply);
        assert_eq!(
            saved.telegram.character_id.as_deref(),
            Some("another-character")
        );
        fixture.assert_current(&saved).await;
    }
}

#[tokio::test]
async fn stale_telegram_voice_edit_conflicts_after_full_config_revocation() {
    let fixture = Fixture::new();
    let (stale_revision, mut telegram) = fixture.runtime.current_telegram_config().await;
    let mut revocation = fixture.runtime.current_config().await;
    assert_eq!(revocation.revision, stale_revision);
    revocation.telegram.allowed_chat_ids = vec![RETAINED_CHAT];
    let committed = fixture.runtime.save_config(revocation).await.unwrap();
    let committed_bytes = fs::read(&fixture.path).unwrap();

    telegram.send_voice_reply = false;
    assert!(matches!(
        fixture
            .runtime
            .save_telegram_config(stale_revision, telegram)
            .await,
        Err(KokoroError::BotConfigConflict(_))
    ));
    fixture.assert_current(&committed).await;
    assert_eq!(fs::read(&fixture.path).unwrap(), committed_bytes);
    let candidates = [REVOKED_CHAT, RETAINED_CHAT, OTHER_CHAT];
    fixture
        .telegram
        .assert_dispatched_text_chats(&candidates, &[RETAINED_CHAT])
        .await;
    let (reloaded, reloaded_telegram) = initialize_from_disk(&fixture.path);
    assert_eq!(reloaded.current_config().await, committed);
    reloaded_telegram
        .assert_dispatched_text_chats(&candidates, &[RETAINED_CHAT])
        .await;

    let (revision, mut refreshed) = fixture.runtime.current_telegram_config().await;
    refreshed.send_voice_reply = false;
    let retried = fixture
        .runtime
        .save_telegram_config(revision, refreshed)
        .await
        .unwrap();
    assert_eq!(retried.telegram.allowed_chat_ids, vec![RETAINED_CHAT]);
    assert!(!retried.telegram.send_voice_reply);
    fixture.assert_current(&retried).await;
}

#[tokio::test]
async fn failed_bot_config_writes_keep_the_last_persisted_revision_and_authorization() {
    let fixture = Fixture::new();
    let (revision, mut replacement) = fixture.runtime.current_telegram_config().await;
    replacement.allowed_chat_ids = vec![RETAINED_CHAT];
    let committed = fixture
        .runtime
        .save_telegram_config(revision, replacement)
        .await
        .unwrap();
    let committed_bytes = fs::read(&fixture.path).unwrap();

    // A directory at the file path blocks the real atomic replacement on Windows
    // and Unix. Keep the last committed file in the same temporary directory.
    let original_path = fixture.path.with_file_name("last-committed.json");
    fs::rename(&fixture.path, &original_path).unwrap();
    fs::create_dir(&fixture.path).unwrap();

    let mut rejected = committed.clone();
    rejected.telegram.allowed_chat_ids.push(REVOKED_CHAT);
    assert!(matches!(
        fixture
            .runtime
            .save_telegram_config(rejected.revision, rejected.telegram.clone())
            .await,
        Err(KokoroError::Config(_))
    ));
    rejected.selected_platform = "discord".into();
    assert!(matches!(
        fixture.runtime.save_config(rejected).await,
        Err(KokoroError::Config(_))
    ));

    let mut pending = pin!(pending_group_request(&fixture.runtime));
    assert!(futures::poll!(pending.as_mut()).is_pending());
    assert!(matches!(
        fixture
            .runtime
            .respond_to_qq_authorization(QQ_REQUEST, QQ_USER, Some(QQ_GROUP), true)
            .await,
        Err(KokoroError::Config(_))
    ));
    assert!(!pending.await);
    assert_eq!(fixture.runtime.current_config().await, committed);
    assert_eq!(fixture.telegram.get_config().await, committed.telegram);
    fixture
        .telegram
        .assert_dispatched_text_chats(&[REVOKED_CHAT, RETAINED_CHAT, OTHER_CHAT], &[RETAINED_CHAT])
        .await;

    assert_eq!(fs::read(&original_path).unwrap(), committed_bytes);
    fs::remove_dir(&fixture.path).unwrap();
    fs::rename(&original_path, &fixture.path).unwrap();
    fixture.assert_current(&committed).await;
    let (reloaded, _) = initialize_from_disk(&fixture.path);
    assert_eq!(reloaded.current_config().await, committed);
}
