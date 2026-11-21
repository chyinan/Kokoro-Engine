use super::{
    saved_external_conversation_id, saved_external_input, EXTERNAL_REQUEST_OWNER_SQL,
    SAVED_EXTERNAL_CONVERSATION_SQL, SAVED_EXTERNAL_INPUT_SQL, SAVED_EXTERNAL_MESSAGE_SQL,
    SAVED_EXTERNAL_METADATA_SQL, SAVED_NON_USER_MESSAGE_IDENTITY_SQL,
    SAVED_USER_MESSAGE_IDENTITY_SQL,
};
use crate::ai::database_migrations;
use sqlx::{migrate::Migrate, sqlite::SqlitePoolOptions, SqlitePool};

const HIT_METADATA: &str = r#"{"chat_save_id":"plan-hit:user","client_request_id":"plan-hit","source":"webhook","external_input_text":"caption"}"#;

async fn seed_history(pool: &SqlitePool) {
    sqlx::raw_sql(
        "WITH RECURSIVE ids(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM ids WHERE i<7)
         INSERT INTO characters(id,name) SELECT 'character-'||i, 'Character '||i FROM ids;
         WITH RECURSIVE ids(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM ids WHERE i<255)
         INSERT INTO conversations(id,character_id,created_at,updated_at)
         SELECT 'conversation-'||i, 'character-'||(i%8), 'now', 'now' FROM ids;
         WITH RECURSIVE ids(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM ids WHERE i<32767)
         INSERT INTO conversation_messages(conversation_id,role,content,metadata,created_at)
         SELECT 'conversation-'||(i%256),
                CASE (i/256)%5 WHEN 0 THEN 'user' WHEN 1 THEN 'assistant'
                    WHEN 2 THEN 'user' WHEN 3 THEN 'tool' ELSE 'system' END,
                'history message '||i,
                CASE i%13 WHEN 0 THEN NULL WHEN 1 THEN 'malformed' WHEN 2 THEN ''
                    WHEN 3 THEN '{}' WHEN 4 THEN 'null' WHEN 5 THEN json_object('chat_save_id',NULL)
                    WHEN 6 THEN json_object('client_request_id','legacy-'||i,'source','webhook')
                    ELSE json_object('chat_save_id','history-'||i||':user',
                                     'client_request_id','history-'||i,'source','webhook') END,
                'now' FROM ids;",
    )
    .execute(pool)
    .await
    .unwrap();

    for (conversation, role, content, metadata) in [
        (
            "conversation-8",
            "user",
            "prepared transcription",
            HIT_METADATA,
        ),
        (
            "conversation-9",
            "user",
            "foreign input",
            r#"{"chat_save_id":"foreign-only:user","source":"webhook"}"#,
        ),
        (
            "conversation-0",
            "assistant",
            "non-user input",
            r#"{"chat_save_id":"wrong-role:user","source":"webhook"}"#,
        ),
    ] {
        sqlx::query(
            "INSERT INTO conversation_messages(conversation_id,role,content,metadata,created_at)
             VALUES (?,?,?,?,'now')",
        )
        .bind(conversation)
        .bind(role)
        .bind(content)
        .bind(metadata)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn assert_lookup_rows(pool: &SqlitePool) {
    for (character, request, hit) in [
        ("character-0", "plan-hit", true),
        ("character-0", "plan-miss", false),
        ("character-1", "plan-hit", false),
        ("character-0", "foreign-only", false),
        ("character-0", "wrong-role", false),
        ("character-0", "legacy-32", false),
    ] {
        assert_eq!(
            saved_external_conversation_id(pool, character, request)
                .await
                .unwrap(),
            hit.then(|| "conversation-8".to_owned()),
            "intake: {character}/{request}"
        );
        assert_eq!(
            saved_external_input(pool, character, request)
                .await
                .unwrap(),
            hit.then(|| (
                "prepared transcription".to_owned(),
                Some(HIT_METADATA.to_owned())
            )),
            "core and generic Bot: {character}/{request}"
        );
        let saved_message: Option<(String, String)> = sqlx::query_as(SAVED_EXTERNAL_MESSAGE_SQL)
            .bind(character)
            .bind(format!("{request}:user"))
            .fetch_optional(pool)
            .await
            .unwrap();
        assert_eq!(
            saved_message,
            hit.then(|| (
                "conversation-8".to_owned(),
                "prepared transcription".to_owned()
            )),
            "Telegram: {character}/{request}"
        );
    }
}

async fn assert_keyed_plan(
    pool: &SqlitePool,
    scenario: &str,
    sql: &str,
    bindings: &[&str],
    indexes: &[&str],
) {
    let explain = format!("EXPLAIN QUERY PLAN {sql}");
    let mut query = sqlx::query_as::<_, (i64, i64, i64, String)>(&explain).persistent(false);
    for binding in bindings {
        query = query.bind(*binding);
    }
    let steps = query.fetch_all(pool).await.unwrap();
    let details = steps.iter().map(|step| step.3.as_str()).collect::<Vec<_>>();
    println!("{scenario} {bindings:?}\n{}", details.join("\n"));

    for index in indexes {
        assert!(
            details.iter().any(|detail| {
                detail.starts_with("SEARCH ")
                    && detail.split_ascii_whitespace().any(|word| word == *index)
                    && detail.contains("<expr>=?")
                    && (*index != "idx_chat_non_user_save_id"
                        || detail.contains("(conversation_id=? AND role=? AND <expr>=?)"))
            }),
            "{scenario} {bindings:?} expected equality on {index}: {details:?}"
        );
    }
    assert!(
        !details.iter().any(|detail| {
            detail.starts_with("SCAN ")
                && detail.split_ascii_whitespace().any(|word| {
                    matches!(word, "m" | "conversation_messages") || word.starts_with("idx_chat_")
                })
        }),
        "{scenario} {bindings:?} scanned messages or a chat index: {details:?}"
    );
}

#[tokio::test]
async fn downstream_save_id_lookups_stay_keyed_after_upgrade_and_analyze() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let mut connection = pool.acquire().await.unwrap();
    connection.ensure_migrations_table().await.unwrap();
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .take_while(|migration| migration.version <= 15)
    {
        connection.apply(migration).await.unwrap();
    }
    drop(connection);

    seed_history(&pool).await;
    sqlx::raw_sql(
        "INSERT INTO conversation_messages(conversation_id,role,content,metadata,created_at) VALUES
           ('conversation-8','tool','result','{\"chat_save_id\":\"plan-hit:tool\"}','now'),
           ('conversation-8','assistant','reply','{\"chat_save_id\":\"plan-hit:assistant\"}','now'),
           ('conversation-8','assistant','malformed','{','now'),
           ('conversation-8','assistant','empty','','now');",
    )
    .execute(&pool)
    .await
    .unwrap();
    database_migrations::run(&pool).await.unwrap();

    for phase in ["before ANALYZE", "after ANALYZE"] {
        if phase == "after ANALYZE" {
            sqlx::raw_sql("PRAGMA analysis_limit=0; ANALYZE;")
                .execute(&pool)
                .await
                .unwrap();
        }
        for (request, hit) in [("plan-hit", true), ("plan-miss", false)] {
            let save_id = format!("{request}:user");
            let keys = ["conversation-8", &save_id];
            for (path, sql) in [
                ("original input metadata", SAVED_EXTERNAL_METADATA_SQL),
                ("user write", SAVED_USER_MESSAGE_IDENTITY_SQL),
            ] {
                assert_keyed_plan(
                    &pool,
                    &format!("{phase} {path}"),
                    sql,
                    &keys,
                    &["idx_chat_user_save_id"],
                )
                .await;
            }
            let metadata: Option<String> = sqlx::query_scalar(SAVED_EXTERNAL_METADATA_SQL)
                .bind(keys[0])
                .bind(keys[1])
                .fetch_optional(&pool)
                .await
                .unwrap();
            assert_eq!(metadata.as_deref(), hit.then_some(HIT_METADATA));

            for (role, content) in [("tool", "result"), ("assistant", "reply")] {
                let save_id = format!("{request}:{role}");
                assert_keyed_plan(
                    &pool,
                    phase,
                    SAVED_NON_USER_MESSAGE_IDENTITY_SQL,
                    &["conversation-8", role, &save_id],
                    &["idx_chat_non_user_save_id"],
                )
                .await;
                let row: Option<(i64, String, Option<String>)> =
                    sqlx::query_as(SAVED_NON_USER_MESSAGE_IDENTITY_SQL)
                        .bind("conversation-8")
                        .bind(role)
                        .bind(save_id)
                        .fetch_optional(&pool)
                        .await
                        .unwrap();
                assert_eq!(
                    row.as_ref().map(|(_, text, _)| text.as_str()),
                    hit.then_some(content)
                );
            }
        }
    }
}

#[tokio::test]
async fn external_request_lookups_use_save_id_equality_before_and_after_analyze() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    database_migrations::run(&pool).await.unwrap();
    seed_history(&pool).await;

    let (messages, roles, conversations): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(DISTINCT role), COUNT(DISTINCT conversation_id)
         FROM conversation_messages",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((messages, roles, conversations), (32_771, 4, 256));

    for phase in ["before ANALYZE", "after ANALYZE"] {
        if phase == "after ANALYZE" {
            // Stay on one connection and reprepare EXPLAIN using complete statistics.
            sqlx::raw_sql("PRAGMA analysis_limit=0; ANALYZE;")
                .execute(&pool)
                .await
                .unwrap();
        }
        assert_lookup_rows(&pool).await;
        for request in ["plan-hit", "plan-miss"] {
            let save_id = format!("{request}:user");
            for (path, sql) in [
                ("intake", SAVED_EXTERNAL_CONVERSATION_SQL),
                ("chat core / generic Bot", SAVED_EXTERNAL_INPUT_SQL),
                ("Telegram", SAVED_EXTERNAL_MESSAGE_SQL),
            ] {
                assert_keyed_plan(
                    &pool,
                    &format!("{phase} {path}"),
                    sql,
                    &["character-0", &save_id],
                    &["idx_chat_user_save_id"],
                )
                .await;
            }
            assert_keyed_plan(
                &pool,
                &format!("{phase} owner control"),
                EXTERNAL_REQUEST_OWNER_SQL,
                &[&save_id, "webhook", request],
                &["idx_chat_user_save_id", "idx_chat_user_request_id"],
            )
            .await;
        }
    }
}
