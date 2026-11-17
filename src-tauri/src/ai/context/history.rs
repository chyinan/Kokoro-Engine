use super::{sync_history_window, AIOrchestrator, Message};
use crate::chat::tags::{is_visible_message_meta, is_visible_message_raw};
use crate::error::KokoroError;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

impl AIOrchestrator {
    pub(crate) async fn delete_last_messages(
        &self,
        count: usize,
        expected_conversation_id: Option<&str>,
        expected_tail_message_id: Option<i64>,
        expected_history: Option<HistoryDeletionTarget>,
    ) -> Result<(), KokoroError> {
        let _activation_guard = self.enter_chat_turn().map_err(KokoroError::Chat)?;
        let _history_guard = self
            .try_acquire_chat_turn("history_mutation")
            .await
            .map_err(KokoroError::Chat)?;

        let max_chars = *self.max_message_chars.lock().await;
        delete_history_tail(
            count,
            &self.db,
            &self.selection.history,
            &self.selection.current_conversation_id,
            max_chars,
            Some(&self.selection.memory_history_boundary),
            expected_conversation_id,
            &self.selection.conversation_switch_lock,
            expected_tail_message_id,
            expected_history
                .map(|expected| (self.selection.conversation_generation.as_ref(), expected)),
        )
        .await
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDeletionTarget {
    generation: u64,
    history_revision: i64,
    keep_through_id: i64,
}

fn changed_history_before_deletion() -> KokoroError {
    KokoroError::Validation("Conversation history changed before deletion".into())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn delete_history_tail(
    count: usize,
    db: &SqlitePool,
    history: &Arc<Mutex<VecDeque<Message>>>,
    current_conversation_id: &Arc<Mutex<Option<String>>>,
    max_chars: usize,
    memory_history_boundary: Option<&Arc<Mutex<usize>>>,
    expected_conversation_id: Option<&str>,
    conversation_switch_lock: &Arc<Mutex<()>>,
    expected_tail_message_id: Option<i64>,
    expected_history: Option<(&AtomicU64, HistoryDeletionTarget)>,
) -> Result<(), KokoroError> {
    if count == 0 && expected_history.is_none() {
        return Ok(());
    }

    // 会话切换锁：与 load_conversation / clear_history / edit 等会话历史重写路径互斥。
    // 持有到函数结束，确保删除期间发生的会话切换要么等待、要么在删除前完成，
    // 旧删除操作永远不会把旧会话的历史覆盖到新会话的内存上下文上。
    let _switch_guard = conversation_switch_lock.lock().await;

    let conv_id = current_conversation_id.lock().await.clone();
    if expected_history
        .as_ref()
        .is_some_and(|(generation, expected)| {
            expected_conversation_id.is_none()
                || conv_id.as_deref() != expected_conversation_id
                || generation.load(Ordering::SeqCst) != expected.generation
        })
    {
        return Err(changed_history_before_deletion());
    }
    // 竞态防护：调用方声明了发起删除时所属的会话。若后端当前会话已在删除 IPC 在途期间
    // 被切换（如用户点选了另一会话，load_conversation 会更新 current_conversation_id），
    // 则放弃删除，防止误删新会话的消息。
    if let Some(expected) = expected_conversation_id {
        if conv_id.as_deref() != Some(expected) {
            tracing::warn!(
                target: "ai",
                "Skipping delete_last_messages: expected conversation {:?} but current is {:?}",
                expected,
                conv_id
            );
            return Ok(());
        }
    }
    if let Some(ref conversation_id) = conv_id {
        let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
        if let Some((_, expected)) = &expected_history {
            let revision =
                crate::chat::retry::conversation_message_revision(&mut tx, conversation_id).await?;
            if revision != Some(expected.history_revision) {
                return Err(changed_history_before_deletion());
            }
        }
        // 读取该会话的所有行（id, role, metadata），按 id 升序排列
        let rows: Vec<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT id, role, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conversation_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;

        // 收集所有可见消息在 rows 中的索引
        let visible_indices: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, (_, role, metadata))| is_visible_message_raw(role, metadata.as_deref()))
            .map(|(idx, _)| idx)
            .collect();

        if let Some(expected_tail) = expected_tail_message_id {
            let actual_tail = visible_indices.last().map(|index| rows[*index].0);
            if actual_tail != Some(expected_tail) {
                tracing::warn!(
                    target: "ai",
                    "Skipping delete_last_messages: expected tail {:?} but current tail is {:?}",
                    expected_tail,
                    actual_tail
                );
                return Err(changed_history_before_deletion());
            }
        }

        let total_visible = visible_indices.len();
        let ids_to_delete: Vec<i64> = if let Some((_, expected)) = &expected_history {
            let keep_through = rows
                .iter()
                .position(|row| row.0 == expected.keep_through_id)
                .ok_or_else(changed_history_before_deletion)?;
            rows[keep_through + 1..].iter().map(|row| row.0).collect()
        } else if count >= total_visible {
            // 如果要删除的可见条数 >= 当前总可见条数，则删除该会话的所有消息（包括不可见技术行）
            rows.iter().map(|(id, _, _)| *id).collect()
        } else {
            // 保留前 total_visible - count 条可见消息
            let keep_visible_count = total_visible - count;
            let last_keep_idx = visible_indices[keep_visible_count - 1];
            // 从最后一条保留的可见消息之后的所有行（包括伴生的 assistant_tool_calls / tool_result 等不可见技术行与待删可见消息）全部删除
            rows[last_keep_idx + 1..]
                .iter()
                .map(|(id, _, _)| *id)
                .collect()
        };

        if !ids_to_delete.is_empty() {
            for id in &ids_to_delete {
                sqlx::query("DELETE FROM conversation_messages WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| KokoroError::Database(e.to_string()))?;
            }
            tracing::info!(
                target: "ai",
                "Deleted {} DB row(s) for {} visible message(s)",
                ids_to_delete.len(),
                count.min(total_visible)
            );
        }

        // 用权威数据库的剩余行重新同步 history，彻底杜绝孤儿技术行（assistant_tool_calls/tool_result）残留，并应用统一的 20 条滑动窗口与字符截断
        let remaining_rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conversation_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| KokoroError::Database(e.to_string()))?;
        tx.commit().await?;

        let mut hist = history.lock().await;
        let count = sync_history_window(&mut hist, remaining_rows, max_chars);
        if let Some(boundary_lock) = memory_history_boundary {
            *boundary_lock.lock().await = count;
        }
    } else {
        // 纯内存模式同步：同样按可见消息边界截断
        let mut hist = history.lock().await;
        let visible_indices: Vec<usize> = hist
            .iter()
            .enumerate()
            .filter(|(_, msg)| is_visible_message_meta(&msg.role, msg.metadata.as_ref()))
            .map(|(idx, _)| idx)
            .collect();
        let total_visible = visible_indices.len();
        if count >= total_visible {
            hist.clear();
        } else {
            let keep_visible_count = total_visible - count;
            let last_keep_idx = visible_indices[keep_visible_count - 1];
            hist.truncate(last_keep_idx + 1);
        }
        let len = hist.len();
        if let Some(boundary_lock) = memory_history_boundary {
            let mut boundary = boundary_lock.lock().await;
            if *boundary > len {
                *boundary = len;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switch_lock() -> Arc<Mutex<()>> {
        Arc::new(Mutex::new(()))
    }

    async fn setup_test_context_db() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePool::connect("sqlite::memory:")
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                character_id TEXT NOT NULL,
                title TEXT NOT NULL,
                topic TEXT NOT NULL DEFAULT '',
                pinned_state TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE conversation_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        pool
    }

    #[test]
    fn test_is_visible_message_classification() {
        // Invisible cases
        assert!(!is_visible_message_raw("tool", None));
        assert!(!is_visible_message_raw(
            "tool",
            Some(r#"{"type":"tool_result"}"#)
        ));
        assert!(!is_visible_message_raw(
            "assistant",
            Some(r#"{"type":"assistant_tool_calls"}"#)
        ));
        assert!(!is_visible_message_raw(
            "assistant",
            Some(r#"{"type":"translation_instruction"}"#)
        ));

        // Visible cases
        assert!(is_visible_message_raw("user", None));
        assert!(is_visible_message_raw("user", Some(r#"{"images":[]}"#)));
        assert!(is_visible_message_raw("assistant", None));
        assert!(is_visible_message_raw(
            "assistant",
            Some(r#"{"turn_id":"t-1","translation":"hello"}"#)
        ));
        assert!(is_visible_message_raw(
            "context",
            Some(r#"{"type":"vision_context"}"#)
        ));
    }

    #[tokio::test]
    async fn test_delete_last_messages_cleans_assistant_tool_calls_and_results() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-1', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 1. User message (id 1)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'user', 'What is the weather?', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 2. Technical tool call message (id 2)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'assistant', 'call weather', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 3. Technical tool result message (id 3)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'tool', '{\"temp\": 25}', '{\"type\":\"tool_result\"}', ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // 4. Final assistant answer (id 4)
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-1', 'assistant', 'It is 25C.', NULL, ?)")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-1".to_string())));

        // Call delete_history_tail(1, ...) to delete the final assistant response
        delete_history_tail(
            1,
            &pool,
            &history,
            &current_conv,
            2000,
            None,
            None,
            &switch_lock(),
            None,
            None,
        )
        .await
        .unwrap();

        // Ensure rows 2, 3, 4 are completely deleted, and only id 1 remains
        let remaining_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = 'conv-1' ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining_ids, vec![1], "Orphan assistant_tool_calls and tool_result rows must be deleted along with the assistant message");

        // Ensure history synchronization contains only the user message
        let hist = history.lock().await;
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].role, "user");
        assert_eq!(hist[0].content, "What is the weather?");
    }

    #[tokio::test]
    async fn test_delete_last_messages_preserves_earlier_turns_tool_calls() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-2', 'char-1', 'Test', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // Turn 1
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'user', 'Turn 1 User', NULL, ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'Turn 1 ToolCall', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'tool', 'Turn 1 Result', '{\"type\":\"tool_result\"}', ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'Turn 1 Answer', NULL, ?)")
            .bind(&now).execute(&pool).await.unwrap();

        // Turn 2
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'user', 'Turn 2 User', NULL, ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'Turn 2 ToolCall', '{\"type\":\"assistant_tool_calls\"}', ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'tool', 'Turn 2 Result', '{\"type\":\"tool_result\"}', ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-2', 'assistant', 'Turn 2 Answer', NULL, ?)")
            .bind(&now).execute(&pool).await.unwrap();

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-2".to_string())));

        // Delete 1 visible message (Turn 2 Answer)
        delete_history_tail(
            1,
            &pool,
            &history,
            &current_conv,
            2000,
            None,
            None,
            &switch_lock(),
            None,
            None,
        )
        .await
        .unwrap();

        // Should keep ids 1..=5 (Turn 1 completely intact + Turn 2 User), delete ids 6, 7, 8
        let remaining_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = 'conv-2' ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining_ids, vec![1, 2, 3, 4, 5]);

        // Delete another 2 visible messages (Turn 2 User [5] and Turn 1 Answer [4])
        delete_history_tail(
            2,
            &pool,
            &history,
            &current_conv,
            2000,
            None,
            None,
            &switch_lock(),
            None,
            None,
        )
        .await
        .unwrap();

        // Only Turn 1 User (id 1) should remain, all subsequent tool rows and answers removed
        let remaining_ids2: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = 'conv-2' ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining_ids2, vec![1]);
    }

    #[tokio::test]
    async fn test_delete_last_messages_in_memory_mode() {
        let pool = setup_test_context_db().await;
        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(None)); // In-memory mode

        {
            let mut hist = history.lock().await;
            hist.push_back(crate::ai::context::Message {
                role: "user".to_string(),
                content: "User prompt".to_string(),
                metadata: None,
            });
            hist.push_back(crate::ai::context::Message {
                role: "assistant".to_string(),
                content: "calling".to_string(),
                metadata: Some(serde_json::json!({"type": "assistant_tool_calls"})),
            });
            hist.push_back(crate::ai::context::Message {
                role: "tool".to_string(),
                content: "result".to_string(),
                metadata: Some(serde_json::json!({"type": "tool_result"})),
            });
            hist.push_back(crate::ai::context::Message {
                role: "assistant".to_string(),
                content: "Final answer".to_string(),
                metadata: None,
            });
        }

        // Delete last 1 visible message
        delete_history_tail(
            1,
            &pool,
            &history,
            &current_conv,
            2000,
            None,
            None,
            &switch_lock(),
            None,
            None,
        )
        .await
        .unwrap();

        let hist = history.lock().await;
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].role, "user");
        assert_eq!(hist[0].content, "User prompt");
    }

    #[tokio::test]
    async fn test_delete_last_messages_enforces_20_limit_and_truncation() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-long', 'char-1', 'Long Conv', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        // Insert 35 messages (17 user-assistant pairs + 1 extra user)
        for i in 0..35 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            let content = if i == 30 {
                // An extra long message to verify truncation
                "L".repeat(100)
            } else {
                format!("Message {i}")
            };
            sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-long', ?, ?, NULL, ?)")
                .bind(role)
                .bind(&content)
                .bind(&now)
                .execute(&pool)
                .await
                .unwrap();
        }

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-long".to_string())));
        let memory_boundary = Arc::new(Mutex::new(0));

        // Delete the last visible message (message 34, leaving 34 messages: 0..34)
        delete_history_tail(
            1,
            &pool,
            &history,
            &current_conv,
            40,
            Some(&memory_boundary),
            None,
            &switch_lock(),
            None,
            None,
        )
        .await
        .unwrap();

        // Check DB row count: 34 rows remain
        let total_db_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id = 'conv-long'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(total_db_rows, 34);

        // Check in-memory history: must NOT have 34 messages! Must be strictly capped at 20!
        let hist = history.lock().await;
        assert_eq!(
            hist.len(),
            20,
            "Memory history must enforce 20 message window limit"
        );
        // Messages remaining in DB are 0..34. The last 20 are indices 14..34.
        assert_eq!(hist[0].content, "Message 14");
        // Message 30 should be present in history and truncated to 40 chars
        let msg_30 = hist
            .iter()
            .find(|m| m.content.starts_with("LLLL"))
            .expect("Message 30 should be in history");
        assert!(msg_30.content.ends_with("…[truncated]"));
        assert_eq!(msg_30.content, format!("{}…[truncated]", "L".repeat(40)));

        // Memory boundary should be synchronized to 20
        assert_eq!(*memory_boundary.lock().await, 20);
    }

    #[tokio::test]
    async fn test_delete_last_messages_skips_when_conversation_changed() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-current', 'char-1', 'Current', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        for i in 0..2 {
            let role = if i == 0 { "user" } else { "assistant" };
            sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-current', ?, ?, NULL, ?)")
                .bind(role)
                .bind(format!("Message {i}"))
                .bind(&now)
                .execute(&pool)
                .await
                .unwrap();
        }

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-current".to_string())));

        // 期望会话与后端当前会话不一致（用户已在删除 IPC 在途期间切换会话）→ 应整体放弃删除
        delete_history_tail(
            1,
            &pool,
            &history,
            &current_conv,
            2000,
            None,
            Some("conv-other"),
            &switch_lock(),
            None,
            None,
        )
        .await
        .unwrap();

        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id = 'conv-current'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            remaining, 2,
            "Stale delete must not touch the current conversation"
        );
    }

    #[tokio::test]
    async fn test_delete_last_messages_rejects_stale_expected_tail() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES ('conv-tail', 'char-1', 'Tail guard', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();

        for (role, content) in [("user", "Question"), ("assistant", "Answer")] {
            sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES ('conv-tail', ?, ?, NULL, ?)")
                .bind(role)
                .bind(content)
                .bind(&now)
                .execute(&pool)
                .await
                .unwrap();
        }

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-tail".to_string())));
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM conversation_messages WHERE conversation_id = 'conv-tail' ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let expected_tail = *ids.last().unwrap();

        delete_history_tail(
            1,
            &pool,
            &history,
            &current_conv,
            2000,
            None,
            Some("conv-tail"),
            &switch_lock(),
            Some(expected_tail),
            None,
        )
        .await
        .unwrap();

        let stale_result = delete_history_tail(
            1,
            &pool,
            &history,
            &current_conv,
            2000,
            None,
            Some("conv-tail"),
            &switch_lock(),
            Some(expected_tail),
            None,
        )
        .await;
        assert!(matches!(stale_result, Err(KokoroError::Validation(_))));

        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM conversation_messages WHERE conversation_id = 'conv-tail'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            remaining, 1,
            "A stale retry must not delete the remaining history"
        );
    }

    #[tokio::test]
    async fn test_versioned_history_deletion_checks_generation_and_same_tail_writes() {
        let pool = setup_test_context_db().await;
        sqlx::raw_sql(include_str!(
            "../../../migrations/0014_conversation_message_revision.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(
            "INSERT INTO conversations (id, character_id, title, created_at, updated_at)
                VALUES ('conv', 'char-1', 'Test', 'now', 'now');
             INSERT INTO conversation_messages (id, conversation_id, role, content, metadata, created_at)
                VALUES (1, 'conv', 'assistant', 'Keep', NULL, 'now'),
                       (2, 'conv', 'tool', 'Result for kept reply', '{\"turn_id\":\"kept\"}', 'now'),
                       (3, 'conv', 'tool', 'Pet tool result', '{\"turn_id\":\"pet\"}', 'now');",
        )
        .execute(&pool)
        .await
        .unwrap();
        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv".to_string())));
        let generation = AtomicU64::new(3);
        let lock = switch_lock();
        for (expected_generation, revision) in [(2, 3), (3, 2), (3, 3)] {
            let result = delete_history_tail(
                0,
                &pool,
                &history,
                &current_conv,
                2000,
                None,
                Some("conv"),
                &lock,
                Some(1),
                Some((
                    &generation,
                    HistoryDeletionTarget {
                        generation: expected_generation,
                        history_revision: revision,
                        keep_through_id: 2,
                    },
                )),
            )
            .await;
            let remaining: Vec<i64> =
                sqlx::query_scalar("SELECT id FROM conversation_messages ORDER BY id")
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            if (expected_generation, revision) == (3, 3) {
                result.unwrap();
                assert_eq!(remaining, [1, 2]);
            } else {
                assert!(matches!(result, Err(KokoroError::Validation(_))));
                assert_eq!(remaining, [1, 2, 3]);
            }
        }
    }

    #[tokio::test]
    async fn test_delete_serializes_with_conversation_switch() {
        let pool = setup_test_context_db().await;
        let now = chrono::Utc::now().to_rfc3339();

        // 会话 A 与 B 各一条可见消息
        for (conv, msg) in [("conv-a", "A message"), ("conv-b", "B message")] {
            sqlx::query("INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES (?, 'char-1', 'Test', ?, ?)")
                .bind(conv)
                .bind(&now)
                .bind(&now)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES (?, 'assistant', ?, NULL, ?)")
                .bind(conv)
                .bind(msg)
                .bind(&now)
                .execute(&pool)
                .await
                .unwrap();
        }

        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-a".to_string())));
        let switch_lock = Arc::new(Mutex::new(()));

        // 模拟删除在途：先持有会话切换锁，删除与切换任务在锁上排队
        let guard = switch_lock.lock().await;

        let delete_task = {
            let history = history.clone();
            let current_conv = current_conv.clone();
            let switch_lock = switch_lock.clone();
            tokio::spawn(async move {
                delete_history_tail(
                    1,
                    &pool,
                    &history,
                    &current_conv,
                    2000,
                    None,
                    None,
                    &switch_lock,
                    None,
                    None,
                )
                .await
            })
        };

        // 与 load_conversation 相同的持锁语义：先写 history，后置 conv
        let switch_task = {
            let history = history.clone();
            let current_conv = current_conv.clone();
            let switch_lock = switch_lock.clone();
            tokio::spawn(async move {
                let _guard = switch_lock.lock().await;
                *current_conv.lock().await = Some("conv-b".to_string());
                let mut hist = history.lock().await;
                crate::ai::context::sync_history_window(
                    &mut hist,
                    vec![("assistant".to_string(), "B message".to_string(), None)],
                    2000,
                );
            })
        };

        // 锁被外部持有期间，删除与切换都必须保持挂起（串行化生效）
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !delete_task.is_finished(),
            "delete must wait for the switch lock"
        );
        assert!(
            !switch_task.is_finished(),
            "switch must wait for the switch lock"
        );

        drop(guard);
        delete_task.await.unwrap().unwrap();
        switch_task.await.unwrap();

        // tokio Mutex 为 FIFO：删除先执行、切换后执行。最终历史必须属于会话 B，
        // 旧删除操作绝不覆盖新会话的内存上下文
        assert_eq!(*current_conv.lock().await, Some("conv-b".to_string()));
        let hist = history.lock().await;
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].content, "B message");
    }

    #[tokio::test]
    async fn test_delete_waits_for_conversation_switch_lock() {
        let pool = setup_test_context_db().await;
        let history = Arc::new(Mutex::new(VecDeque::new()));
        let current_conv = Arc::new(Mutex::new(Some("conv-a".to_string())));
        let switch_lock = Arc::new(Mutex::new(()));

        let guard = switch_lock.lock().await;
        let task = {
            let history = history.clone();
            let current_conv = current_conv.clone();
            let switch_lock = switch_lock.clone();
            tokio::spawn(async move {
                delete_history_tail(
                    1,
                    &pool,
                    &history,
                    &current_conv,
                    2000,
                    None,
                    None,
                    &switch_lock,
                    None,
                    None,
                )
                .await
            })
        };

        // 锁覆盖整个函数体：外部持锁期间删除不得推进
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !task.is_finished(),
            "delete must not proceed while the switch lock is held"
        );

        drop(guard);
        task.await.unwrap().unwrap();
    }
}
