use super::{validate_external_request_owner, EXTERNAL_REQUEST_OWNER_SQL};
use crate::{ai::database_migrations, error::KokoroError};
use sqlx::{
    migrate::Migrate,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};

const HISTORICAL_VERSION: i64 = 14;

async fn seed_history(pool: &SqlitePool) {
    // Apply the released SQL and checksums up to the last version before the new indexes.
    let migrator = sqlx::migrate!("./migrations");
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in migrator
        .iter()
        .filter(|migration| migration.version <= HISTORICAL_VERSION)
    {
        connection.apply(migration).await.unwrap();
    }

    sqlx::query(
        "INSERT INTO characters(id, name) VALUES ('local-owner', 'Local'), ('other-owner', 'Other')",
    )
    .execute(&mut *connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO conversations(id, character_id, title, created_at, updated_at) VALUES
         ('local-history', 'local-owner', 'Local history', 'now', 'now'),
         ('other-history', 'other-owner', 'Other history', 'now', 'now')",
    )
    .execute(&mut *connection)
    .await
    .unwrap();

    for (index, (conversation, role, metadata)) in [
        ("local-history", "user", Some(r#"{"chat_save_id":"safe-current:user"}"#)),
        ("other-history", "user", Some(r#"{"chat_save_id":"foreign-current:user","source":"telegram"}"#)),
        ("local-history", "user", Some(r#"{"source":"webhook","client_request_id":"unsafe-current"}"#)),
        ("local-history", "user", Some(r#"{"source":"webhook","client_request_id":"old-safe","chat_save_id":"new-safe:user"}"#)),
        ("local-history", "user", Some(r#"{"source":"webhook","client_request_id":"old-unsafe","chat_save_id":null}"#)),
        ("local-history", "user", Some(r#"{"chat_save_id":"old-save-only:user"}"#)),
        ("other-history", "user", Some(r#"{"source":"webhook","client_request_id":"old-foreign"}"#)),
        ("local-history", "user", Some(r#"{"chat_save_id":"multi-current:user"}"#)),
        ("other-history", "user", Some(r#"{"chat_save_id":"multi-current:user"}"#)),
        ("local-history", "user", Some(r#"{"source":"webhook","client_request_id":"multi-old","chat_save_id":"multi-new:user"}"#)),
        ("other-history", "user", Some(r#"{"source":"webhook","client_request_id":"multi-old","chat_save_id":"different-save:user"}"#)),
        ("local-history", "user", Some(r#"{"chat_save_id":"mixed-candidates:user"}"#)),
        ("other-history", "user", Some(r#"{"source":"webhook","client_request_id":"mixed-old"}"#)),
        ("local-history", "user", Some(r#"{"chat_save_id":"mixed-unsafe:user"}"#)),
        ("local-history", "user", Some(r#"{"source":"webhook","client_request_id":"mixed-unsafe"}"#)),
        ("other-history", "assistant", Some(r#"{"chat_save_id":"hidden-role:user"}"#)),
        ("other-history", "tool", Some(r#"{"source":"webhook","client_request_id":"hidden-role-alias"}"#)),
        ("other-history", "user", Some(r#"{"source":"telegram","client_request_id":"different-source"}"#)),
        ("other-history", "user", Some(r#"{"client_request_id":"bad-metadata"}"#)),
        ("other-history", "user", Some(r#"{"source":"webhook","client_request_id":null}"#)),
        ("other-history", "user", Some(r#"{"source":"webhook"}"#)),
        ("other-history", "user", Some(r#"{"source":"webhook","client_request_id":"bad-metadata","chat_save_id":"bad-metadata:user""#)),
        ("other-history", "user", Some("null")),
        ("other-history", "user", None),
    ]
    .into_iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO conversation_messages(conversation_id, role, content, metadata, created_at)
             VALUES (?, ?, ?, ?, 'historical timestamp')",
        )
        .bind(conversation)
        .bind(role)
        .bind(format!("saved message {index}"))
        .bind(metadata)
        .execute(&mut *connection)
        .await
        .unwrap();
    }
}

type HistoryRow = (i64, String, String, String, Option<String>, String);

async fn history_rows(pool: &SqlitePool) -> Vec<HistoryRow> {
    sqlx::query_as(
        "SELECT id, conversation_id, role, content, metadata, created_at
         FROM conversation_messages ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn assert_owner_decisions(pool: &SqlitePool) {
    const FOREIGN: &str = "another character";
    const UNSAFE: &str = "cannot be safely replayed";
    for (source, request, previous, conflict) in [
        ("webhook", "no-hit", None, None),
        ("webhook", "safe-current", None, None),
        ("webhook", "foreign-current", None, Some(FOREIGN)),
        ("webhook", "unsafe-current", None, Some(UNSAFE)),
        ("webhook", "new-safe", Some("old-safe"), None),
        ("webhook", "new-unsafe", Some("old-unsafe"), Some(UNSAFE)),
        (
            "webhook",
            "new-save-only",
            Some("old-save-only"),
            Some(UNSAFE),
        ),
        ("webhook", "new-foreign", Some("old-foreign"), Some(FOREIGN)),
        ("webhook", "multi-current", None, Some(FOREIGN)),
        ("webhook", "multi-new", Some("multi-old"), Some(FOREIGN)),
        (
            "webhook",
            "mixed-candidates",
            Some("mixed-old"),
            Some(FOREIGN),
        ),
        ("webhook", "mixed-unsafe", None, Some(UNSAFE)),
        ("webhook", "hidden-role", Some("hidden-role-alias"), None),
        ("webhook", "different-source", None, None),
        ("telegram", "different-source", None, Some(FOREIGN)),
        ("webhook", "bad-metadata", None, None),
    ] {
        let previous: Vec<String> = previous.into_iter().map(str::to_owned).collect();
        let result =
            validate_external_request_owner(pool, "local-owner", source, request, &previous).await;
        match conflict {
            None => assert_eq!(result, Ok(()), "{source}/{request}"),
            Some(reason) => assert!(
                matches!(result, Err(KokoroError::Chat(ref error)) if error.starts_with("chat_request_conflict:") && error.contains(reason)),
                "{source}/{request}: expected ownership conflict {reason}, got {result:?}"
            ),
        }
    }
}

async fn assert_owner_uses_candidate_indexes(pool: &SqlitePool, request: &str) {
    let sql = format!("EXPLAIN QUERY PLAN {EXTERNAL_REQUEST_OWNER_SQL}");
    let steps: Vec<(i64, i64, i64, String)> = sqlx::query_as(&sql)
        .persistent(false)
        .bind(format!("{request}:user"))
        .bind("webhook")
        .bind(request)
        .fetch_all(pool)
        .await
        .unwrap();
    for index in ["idx_chat_user_save_id", "idx_chat_user_request_id"] {
        assert!(
            steps.iter().any(|(_, _, _, detail)| {
                detail.starts_with("SEARCH ")
                    && detail.split_ascii_whitespace().any(|word| word == index)
                    && detail.contains("=?")
            }),
            "expected keyed access through {index}: {steps:?}"
        );
    }
    assert!(
        steps.iter().all(|(_, _, _, detail)| {
            !detail.starts_with("SCAN ")
                || !detail.split_ascii_whitespace().any(|word| {
                    matches!(word, "m" | "conversation_messages")
                        || word.starts_with("idx_chat_user_")
                })
        }),
        "owner validation scanned the messages table or a user index: {steps:?}"
    );
}

#[tokio::test]
async fn external_owner_checks_preserve_historical_messages_across_migration_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("external-owners.sqlite"))
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options.clone()).await.unwrap();
    seed_history(&pool).await;
    let historical_rows = history_rows(&pool).await;
    let historic_version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(historic_version, HISTORICAL_VERSION);

    database_migrations::run(&pool).await.unwrap();
    let upgraded_version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(upgraded_version > historic_version);
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema WHERE type='index' AND tbl_name='conversation_messages'
         AND name IN ('idx_chat_user_request_id', 'idx_chat_user_save_id') ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        indexes,
        ["idx_chat_user_request_id", "idx_chat_user_save_id"]
    );
    assert_owner_decisions(&pool).await;
    assert_eq!(history_rows(&pool).await, historical_rows);

    pool.close().await;
    let reopened = SqlitePool::connect_with(options.create_if_missing(false))
        .await
        .unwrap();
    database_migrations::run(&reopened).await.unwrap();
    // A connection that prepared statements before the DDL can report its old
    // schema on EXPLAIN until SQLite next executes a statement against it.
    assert_owner_uses_candidate_indexes(&reopened, "mixed-candidates").await;
    assert_owner_decisions(&reopened).await;
    assert_eq!(history_rows(&reopened).await, historical_rows);
    reopened.close().await;
}

#[tokio::test]
async fn external_owner_checks_stay_keyed_after_analyze_with_sparse_legacy_metadata() {
    // Keep EXPLAIN and ANALYZE on the same connection so the planner sees its statistics.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    database_migrations::run(&pool).await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO characters(id,name) VALUES ('fixture','Fixture');
         INSERT INTO conversations(id,character_id,title,created_at,updated_at)
             VALUES ('fixture','fixture','Fixture','now','now');
         WITH RECURSIVE ids(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM ids WHERE i<256)
         INSERT INTO conversation_messages(conversation_id,role,content,metadata,created_at)
             SELECT 'fixture','user','message',json_object('chat_save_id','request-'||i||':user'),'now' FROM ids;",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert_owner_uses_candidate_indexes(&pool, "request-50").await;
    sqlx::query("ANALYZE").execute(&pool).await.unwrap();
    // A database with no legacy messages still needs a bounded miss on that branch.
    assert_owner_uses_candidate_indexes(&pool, "request-50").await;

    sqlx::query(
        "INSERT INTO conversation_messages(conversation_id,role,content,metadata,created_at)
         VALUES ('fixture','user','legacy',?,'now')",
    )
    .bind(r#"{"source":"webhook","client_request_id":"legacy"}"#)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("ANALYZE").execute(&pool).await.unwrap();
    assert_owner_uses_candidate_indexes(&pool, "request-50").await;
}
