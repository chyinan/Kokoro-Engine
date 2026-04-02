use crate::ai::curiosity::CuriosityModule;
use crate::ai::idle_behaviors::IdleBehaviorSystem;
use crate::ai::initiative::InitiativeSystem;
use crate::ai::memory::MemoryManager;
use crate::ai::router::{ModelRouter, ModelType};
use crate::llm::messages::user_text_message;
use crate::llm::provider::LlmProvider;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    // Optional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnippet {
    pub id: i64,
    pub content: String,
    pub embedding: Vec<u8>,
    pub created_at: i64,
    pub importance: f64,
    pub tier: String,
}

pub struct AIOrchestrator {
    pub db: SqlitePool,
    pub system_prompt: Arc<Mutex<String>>,
    pub history: Arc<Mutex<VecDeque<Message>>>,
    pub max_history_tokens: usize, // Soft limit for history
    pub memory_manager: Arc<MemoryManager>,
    pub router: Arc<ModelRouter>,
    /// Counts user messages for periodic memory extraction triggers.
    message_count: Arc<Mutex<u64>>,
    /// Counts user messages that occurred while the memory system was enabled.
    memory_trigger_count: Arc<Mutex<u64>>,
    /// History index boundary used to prevent extracting conversations from disabled periods.
    memory_history_boundary: Arc<Mutex<usize>>,
    /// Current character ID for memory isolation.
    character_id: Arc<Mutex<String>>,
    /// Global toggle for all automatic memory reads/writes/injection.
    memory_enabled: Arc<AtomicBool>,
    /// Timestamp of last user activity (for idle detection).
    pub last_activity: Arc<Mutex<Instant>>,
    /// Total message count across sessions (for relationship depth).
    pub conversation_count: Arc<Mutex<u64>>,
    /// Preferred response language (e.g. "日本語", "English"). Empty = auto.
    pub response_language: Arc<Mutex<String>>,
    /// User's display language for inline translation (e.g. "中文"). Empty = disabled.
    pub user_language: Arc<Mutex<String>>,
    /// Jailbreak prompt prefix (prepended to all system prompts). Empty = disabled.
    pub jailbreak_prompt: Arc<Mutex<String>>,
    /// Character name for {{char}} placeholder replacement.
    character_name: Arc<Mutex<String>>,
    /// User name for {{user}} placeholder replacement.
    user_name: Arc<Mutex<String>>,

    // Autonomous Behavior Modules
    pub curiosity: Arc<Mutex<CuriosityModule>>,
    pub initiative: Arc<Mutex<InitiativeSystem>>,
    pub idle_behaviors: Arc<Mutex<IdleBehaviorSystem>>,
    /// Whether proactive (idle auto-talk) messages are enabled.
    pub proactive_enabled: Arc<std::sync::atomic::AtomicBool>,
    /// 当前活跃对话 ID
    pub current_conversation_id: Arc<Mutex<Option<String>>>,
    /// Context management strategy: "window" | "summary"
    pub context_strategy: Arc<Mutex<String>>,
    /// Max characters per message before truncation
    pub max_message_chars: Arc<Mutex<usize>>,
}

impl AIOrchestrator {
    pub async fn new(db_url: &str) -> Result<Self> {
        // Create database if it doesn't exist
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(db_url)?.create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await?;

        // Run all database migrations
        sqlx::migrate!("./migrations").run(&pool).await?;

        let memory_manager = Arc::new(MemoryManager::new(pool.clone()));

        Ok(Self {
            db: pool,
            system_prompt: Arc::new(Mutex::new("You are a helpful assistant.".to_string())),
            history: Arc::new(Mutex::new(VecDeque::new())),
            max_history_tokens: 4000,
            memory_manager,
            router: Arc::new(ModelRouter::new()),
            message_count: Arc::new(Mutex::new(0)),
            memory_trigger_count: Arc::new(Mutex::new(0)),
            memory_history_boundary: Arc::new(Mutex::new(0)),
            character_id: Arc::new(Mutex::new("default".to_string())),
            memory_enabled: Arc::new(AtomicBool::new(true)),
            last_activity: Arc::new(Mutex::new(Instant::now())),
            conversation_count: Arc::new(Mutex::new(0)),
            response_language: Arc::new(Mutex::new(String::new())),
            user_language: Arc::new(Mutex::new(String::new())),
            jailbreak_prompt: Arc::new(Mutex::new(String::new())),
            character_name: Arc::new(Mutex::new("Kokoro".to_string())),
            user_name: Arc::new(Mutex::new("User".to_string())),
            curiosity: Arc::new(Mutex::new(CuriosityModule::new())),
            initiative: Arc::new(Mutex::new(InitiativeSystem::new())),
            idle_behaviors: Arc::new(Mutex::new(IdleBehaviorSystem::new())),
            proactive_enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            current_conversation_id: Arc::new(Mutex::new(None)),
            context_strategy: Arc::new(Mutex::new("window".to_string())),
            max_message_chars: Arc::new(Mutex::new(2000)),
        })
    }

    pub async fn set_system_prompt(&self, prompt: String) {
        self.set_system_prompt_with_reset(prompt, true).await;
    }

    pub async fn set_system_prompt_with_reset(&self, prompt: String, _reset_emotion: bool) {
        let mut sp = self.system_prompt.lock().await;
        *sp = prompt;
    }

    pub async fn set_jailbreak_prompt(&self, prompt: String) {
        let mut jp = self.jailbreak_prompt.lock().await;
        *jp = prompt;
    }

    pub async fn get_jailbreak_prompt(&self) -> String {
        let jp = self.jailbreak_prompt.lock().await;
        jp.clone()
    }

    pub async fn set_response_language(&self, language: String) {
        let mut lang = self.response_language.lock().await;
        *lang = language;
    }

    pub async fn set_user_language(&self, language: String) {
        let mut lang = self.user_language.lock().await;
        *lang = language;
    }

    pub async fn set_character_name(&self, name: String) {
        let mut cn = self.character_name.lock().await;
        *cn = name;
    }

    pub async fn set_user_name(&self, name: String) {
        let mut un = self.user_name.lock().await;
        *un = name;
    }

    /// Enable or disable proactive (idle auto-talk) messages.
    pub fn set_proactive_enabled(&self, enabled: bool) {
        self.proactive_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    /// Check if proactive messages are enabled.
    pub fn is_proactive_enabled(&self) -> bool {
        self.proactive_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Record user activity (resets idle timer).
    pub async fn touch_activity(&self) {
        let mut ts = self.last_activity.lock().await;
        *ts = Instant::now();
        let mut count = self.conversation_count.lock().await;
        *count += 1;
    }

    /// Get seconds since last user activity.
    pub async fn idle_seconds(&self) -> u64 {
        let ts = self.last_activity.lock().await;
        ts.elapsed().as_secs()
    }

    /// Get total conversation message count (approximate relationship depth).
    pub async fn get_conversation_count(&self) -> u64 {
        *self.conversation_count.lock().await
    }

    pub async fn set_character_id(&self, id: String) {
        let mut cid = self.character_id.lock().await;
        *cid = id;
    }

    pub async fn get_character_id(&self) -> String {
        self.character_id.lock().await.clone()
    }

    pub async fn add_message(&self, role: String, content: String, character_id: &str) {
        self.add_message_with_metadata(role, content, None, character_id, None)
            .await;
    }

    pub async fn add_message_with_metadata(
        &self,
        role: String,
        content: String,
        metadata: Option<String>,
        character_id: &str,
        summary_provider: Option<Arc<dyn LlmProvider>>,
    ) {
        let summary_provider = summary_provider.clone();
        // Track user message count for memory extraction triggers
        if role == "user" {
            let mut count = self.message_count.lock().await;
            *count += 1;
            if self.is_memory_enabled() {
                let mut memory_count = self.memory_trigger_count.lock().await;
                *memory_count += 1;
            }
        }

        // Truncate single message if too long
        let max_chars = *self.max_message_chars.lock().await;
        let content = if content.chars().count() > max_chars {
            let truncated: String = content.chars().take(max_chars).collect();
            format!("{}…[truncated]", truncated)
        } else {
            content
        };

        // Persist to database FIRST so no code path can skip it
        let _ = self
            .persist_message(&role, &content, metadata.as_deref(), character_id)
            .await;
        let current_conversation_id = self.current_conversation_id.lock().await.clone();

        let mut history = self.history.lock().await;
        let parsed_metadata = metadata
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
        history.push_back(Message {
            role: role.clone(),
            content: content.clone(),
            metadata: parsed_metadata,
        });

        // Rolling window: keep at most 20 messages in memory. Summary generation is now
        // non-destructive and derives from persisted conversation_messages instead of popped history.
        let strategy = self.context_strategy.lock().await.clone();
        let evicted = if history.len() > 20 {
            history.pop_front();
            true
        } else {
            false
        };
        drop(history);

        if evicted {
            let mut boundary = self.memory_history_boundary.lock().await;
            *boundary = boundary.saturating_sub(1);
        }

        if strategy == "summary" && self.is_memory_enabled() {
            if let (Some(conversation_id), Some(provider)) =
                (current_conversation_id.clone(), summary_provider)
            {
                let memory_manager = self.memory_manager.clone();
                let cid = character_id.to_string();
                tauri::async_runtime::spawn(async move {
                    let task = match memory_manager
                        .get_conversation_summary_task(&conversation_id, &cid)
                        .await
                    {
                        Ok(Some(task)) => task,
                        Ok(None) => return,
                        Err(e) => {
                            eprintln!(
                                "[Context] Failed to prepare conversation summary task for '{}': {}",
                                conversation_id, e
                            );
                            return;
                        }
                    };

                    if let Err(e) = memory_manager
                        .mark_conversation_summary_running(task.record_id)
                        .await
                    {
                        eprintln!(
                            "[Context] Failed to mark summary task running for '{}': {}",
                            conversation_id, e
                        );
                        return;
                    }

                    let prompt = format!(
                        "Summarize the following conversation in 2-3 sentences, focusing on key facts, decisions, emotional shifts, and unresolved threads. Output only the summary, no preamble.\n\n{}",
                        task.transcript
                    );

                    match provider.chat(vec![user_text_message(prompt)], None).await {
                        Ok(text) if !text.trim().is_empty() => {
                            let summary = text.trim().to_string();
                            if let Err(e) = memory_manager
                                .complete_conversation_summary(task.record_id, &summary)
                                .await
                            {
                                eprintln!(
                                    "[Context] Failed to persist conversation summary for '{}': {}",
                                    conversation_id, e
                                );
                            }
                        }
                        Ok(_) => {
                            let _ = memory_manager
                                .fail_conversation_summary(
                                    task.record_id,
                                    "summary provider returned empty output",
                                )
                                .await;
                        }
                        Err(e) => {
                            let _ = memory_manager
                                .fail_conversation_summary(task.record_id, &e.to_string())
                                .await;
                        }
                    }
                });
            }
        }
    }

    /// 将消息持久化到 SQLite，如果没有活跃对话则自动创建
    async fn persist_message(
        &self,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        character_id: &str,
    ) -> Result<()> {
        let cid = character_id;
        let mut conv_id_lock = self.current_conversation_id.lock().await;

        let conv_id = if let Some(ref id) = *conv_id_lock {
            id.clone()
        } else {
            // 自动创建新对话
            let new_id = uuid::Uuid::new_v4().to_string();
            let title = if role == "user" {
                let chars: Vec<char> = content.chars().collect();
                if chars.len() > 20 {
                    format!("{}...", chars[..20].iter().collect::<String>())
                } else {
                    content.to_string()
                }
            } else {
                "新对话".to_string()
            };
            let now = chrono::Utc::now().to_rfc3339();

            sqlx::query(
                "INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at) VALUES (?, ?, ?, '', '{}', ?, ?)"
            )
            .bind(&new_id)
            .bind(cid)
            .bind(&title)
            .bind(&now)
            .bind(&now)
            .execute(&self.db)
            .await?;

            *conv_id_lock = Some(new_id.clone());
            // Persist conversation_id to disk for hot-reload recovery
            Self::persist_conversation_id(Some(&new_id));
            new_id
        };
        drop(conv_id_lock);

        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&conv_id)
        .bind(role)
        .bind(content)
        .bind(metadata)
        .bind(&now)
        .execute(&self.db)
        .await?;

        // 更新对话的 updated_at
        sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&conv_id)
            .execute(&self.db)
            .await?;

        Ok(())
    }

    /// Persist current_conversation_id to disk for hot-reload recovery.
    pub fn persist_conversation_id(id: Option<&str>) {
        let app_data = dirs_next::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("com.chyin.kokoro");
        let _ = std::fs::create_dir_all(&app_data);
        let path = app_data.join("current_conversation_id.json");
        let json = serde_json::json!({ "conversation_id": id });
        if let Err(e) = std::fs::write(&path, json.to_string()) {
            eprintln!("[Context] Failed to persist conversation_id: {}", e);
        }
    }

    /// Persist the active character ID to disk so Telegram can read it.
    pub fn persist_active_character_id(id: &str) {
        let app_data = dirs_next::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("com.chyin.kokoro");
        let _ = std::fs::create_dir_all(&app_data);
        let path = app_data.join("active_character_id.json");
        let json = serde_json::json!({ "character_id": id });
        if let Err(e) = std::fs::write(&path, json.to_string()) {
            eprintln!("[Context] Failed to persist active_character_id: {}", e);
        }
    }

    /// Load the persisted active character ID from disk.
    pub fn load_active_character_id() -> Option<String> {
        let path = dirs_next::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("com.chyin.kokoro")
            .join("active_character_id.json");
        let content = std::fs::read_to_string(&path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        v["character_id"].as_str().map(|s| s.to_string())
    }

    /// Insert a streaming assistant draft into the DB. Returns the row id for later update.
    pub async fn persist_streaming_draft(&self, content: &str, character_id: &str) -> Result<i64> {
        let cid = character_id;
        let mut conv_id_lock = self.current_conversation_id.lock().await;

        // Ensure conversation exists
        let conv_id = if let Some(ref id) = *conv_id_lock {
            id.clone()
        } else {
            let new_id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            sqlx::query(
                "INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at) VALUES (?, ?, ?, '', '{}', ?, ?)"
            )
            .bind(&new_id)
            .bind(cid)
            .bind("新对话")
            .bind(&now)
            .bind(&now)
            .execute(&self.db)
            .await?;
            *conv_id_lock = Some(new_id.clone());
            Self::persist_conversation_id(Some(&new_id));
            new_id
        };
        drop(conv_id_lock);

        let now = chrono::Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES (?, 'assistant', ?, NULL, ?)"
        )
        .bind(&conv_id)
        .bind(content)
        .bind(&now)
        .execute(&self.db)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Update a streaming draft row with final content and metadata.
    pub async fn update_streaming_draft(
        &self,
        row_id: i64,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE conversation_messages SET content = ?, metadata = ? WHERE id = ?")
            .bind(content)
            .bind(metadata)
            .bind(row_id)
            .execute(&self.db)
            .await?;

        // Update conversation updated_at
        let conv_id = self.current_conversation_id.lock().await.clone();
        if let Some(ref id) = conv_id {
            let now = chrono::Utc::now().to_rfc3339();
            sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
                .bind(&now)
                .bind(id)
                .execute(&self.db)
                .await?;
        }
        Ok(())
    }

    /// Returns the total count of user messages in this session.
    pub async fn get_message_count(&self) -> u64 {
        *self.message_count.lock().await
    }

    pub async fn get_memory_trigger_count(&self) -> u64 {
        *self.memory_trigger_count.lock().await
    }

    /// Returns the last `n` messages from history for memory extraction.
    pub async fn get_recent_history(&self, n: usize) -> Vec<Message> {
        let history = self.history.lock().await;
        let start = if history.len() > n {
            history.len() - n
        } else {
            0
        };
        history.iter().skip(start).cloned().collect()
    }

    /// Returns the last `n` messages after the current memory boundary.
    pub async fn get_recent_memory_history(&self, n: usize) -> Vec<Message> {
        let history = self.history.lock().await;
        let boundary = (*self.memory_history_boundary.lock().await).min(history.len());
        let visible_len = history.len().saturating_sub(boundary);
        let start = boundary + visible_len.saturating_sub(n);
        history.iter().skip(start).cloned().collect()
    }

    pub fn is_memory_enabled(&self) -> bool {
        self.memory_enabled.load(Ordering::SeqCst)
    }

    pub fn memory_enabled_flag(&self) -> Arc<AtomicBool> {
        self.memory_enabled.clone()
    }

    pub async fn set_memory_enabled(&self, enabled: bool) {
        self.memory_enabled.store(enabled, Ordering::SeqCst);
        {
            let mut trigger_count = self.memory_trigger_count.lock().await;
            *trigger_count = 0;
        }
        {
            let history_len = self.history.lock().await.len();
            let mut boundary = self.memory_history_boundary.lock().await;
            *boundary = history_len;
        }
    }

    /// Append a message to in-memory history only and keep the memory boundary aligned
    /// with the rolling window behavior used by assistant streaming responses.
    pub async fn push_history_message(&self, message: Message) {
        let mut history = self.history.lock().await;
        history.push_back(message);
        let evicted = if history.len() > 20 {
            history.pop_front();
            true
        } else {
            false
        };
        drop(history);

        if evicted {
            let mut boundary = self.memory_history_boundary.lock().await;
            *boundary = boundary.saturating_sub(1);
        }
    }

    /// Composes a prompt based on the user query, budgeting tokens for context
    pub async fn compose_prompt(
        &self,
        query: &str,
        _allow_image_gen: bool,
        tool_prompt: Option<String>,
        native_tools_enabled: bool,
        character_id: &str,
    ) -> Result<Vec<Message>> {
        // 1. Determine Model logic
        let model_type = self.router.route(query);
        let _max_context = match model_type {
            ModelType::Fast => 8000,
            ModelType::Smart => 32000,
            ModelType::Cheap => 4096,
        };

        // 2. Retrieval (RAG)
        // Only if query looks like it needs context or every N turns
        // For now, always try to fetch relevant memories (scoped to current character)
        let cid = character_id;
        let current_conversation_id = self.current_conversation_id.lock().await.clone();
        let memories = if self.is_memory_enabled() {
            self.memory_manager
                .search_memories(query, 5, cid)
                .await
                .ok()
        } else {
            None
        };
        let conversation_summary = if self.is_memory_enabled() {
            if let Some(ref conversation_id) = current_conversation_id {
                self.memory_manager
                    .get_latest_conversation_summary(conversation_id)
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            }
        } else {
            None
        };
        let conversation_state = if let Some(ref conversation_id) = current_conversation_id {
            sqlx::query("SELECT topic, pinned_state FROM conversations WHERE id = ?")
                .bind(conversation_id)
                .fetch_optional(&self.db)
                .await
                .ok()
                .flatten()
                .map(|row| {
                    (
                        row.get::<String, _>("topic"),
                        row.get::<String, _>("pinned_state"),
                    )
                })
        } else {
            None
        };

        // Read all lock-guarded values upfront and drop locks immediately.
        // This prevents holding multiple mutexes across .await points.
        let sp = self.system_prompt.lock().await.clone();
        let history_snapshot: Vec<Message> = self.history.lock().await.iter().cloned().collect();
        let recent_history_snapshot: Vec<Message> = history_snapshot
            .iter()
            .filter(|msg| {
                let technical_type = msg
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.get("type"))
                    .and_then(|value| value.as_str());
                !matches!(technical_type, Some("translation_instruction"))
            })
            .cloned()
            .collect();

        // -- Read response language early so all sections can reference it --
        let resp_lang = self.response_language.lock().await.clone();

        let mut final_messages = Vec::new();

        // ── Main System Message ──────────────────────────────────────────────
        // Structure: <rules> (highest attention) → <character> → <memory> → <tools> → <live2d> → <language>
        // Placing core_persona rules at the TOP exploits primacy attention effect.
        let mut system_parts: Vec<String> = Vec::new();

        // Section 1: Core persona rules (MUST be first for primacy effect)
        system_parts.push(format!(
            "<rules>\n{}\n</rules>",
            crate::ai::prompts::core_persona_prompt(native_tools_enabled)
        ));

        // Section 2: Character persona (jailbreak + system prompt)
        let jailbreak = self.jailbreak_prompt.lock().await.clone();
        let character_block = if !jailbreak.is_empty() {
            let char_name = self.character_name.lock().await.clone();
            let user_name = self.user_name.lock().await.clone();
            // Preserve base system prompt alongside jailbreak
            let processed_jailbreak = jailbreak
                .replace("{{char}}", &char_name)
                .replace("{{user}}", &user_name);
            if sp.is_empty() {
                processed_jailbreak
            } else {
                format!("{processed_jailbreak}\n\n{sp}")
            }
        } else {
            sp.clone()
        };

        // Emotion state hint — subtly colors tone without overriding character persona
        system_parts.push(format!(
            "<character>\n{}\n</character>",
            character_block
        ));

        // Section 3: Long-term memory (higher priority than summaries)
        if let Some(ref mems) = memories {
            if !mems.is_empty() {
                let memory_block = mems
                    .iter()
                    .map(|m| format!("- {}", m.content))
                    .collect::<Vec<_>>()
                    .join("\n");
                system_parts.push(format!(
                    concat!(
                        "<long_term_memory>\n",
                        "You remember these important facts and events about the user and your shared history:\n{}\n\n",
                        "These long-term memories have higher priority than any conversation summary. ",
                        "Naturally reference them when relevant. Do not list them mechanically, and do not force them into unrelated topics.\n",
                        "</long_term_memory>"
                    ),
                    memory_block
                ));
            }
        }

        // Section 4: Conversation state (stable session facts)
        if let Some((topic, pinned_state)) = conversation_state {
            let normalized_topic = topic.trim();
            let normalized_pinned = pinned_state.trim();
            if !normalized_topic.is_empty() || normalized_pinned != "{}" {
                let mut state_lines = Vec::new();
                if !normalized_topic.is_empty() {
                    state_lines.push(format!("Current conversation topic: {}", normalized_topic));
                }
                if normalized_pinned != "{}" {
                    state_lines.push(format!("Pinned conversation state: {}", normalized_pinned));
                }
                system_parts.push(format!(
                    "<conversation_state>\n{}\n</conversation_state>",
                    state_lines.join("\n")
                ));
            }
        }

        // Section 5: Conversation summary (lower priority than long-term memory and recent raw messages)
        if let Some(summary_record) = conversation_summary {
            if !summary_record.summary.trim().is_empty() {
                system_parts.push(format!(
                    concat!(
                        "<conversation_summary>\n",
                        "This is a compressed summary of earlier messages in the current conversation:\n{}\n\n",
                        "Use it as background only. If it conflicts with long-term memory or recent raw messages, trust long-term memory and recent raw messages.\n",
                        "</conversation_summary>"
                    ),
                    summary_record.summary.trim()
                ));
            }
        } else if self.is_memory_enabled() {
            if let Ok(summaries) = self.memory_manager.get_recent_summaries(cid, 2).await {
                if !summaries.is_empty() {
                    let summary_block = summaries
                        .iter()
                        .enumerate()
                        .map(|(i, s)| format!("{}. {}", i + 1, s))
                        .collect::<Vec<_>>()
                        .join("\n");
                    system_parts.push(format!(
                        "<conversation_summary>\nFallback summaries from recent sessions (most recent first):\n{}\n</conversation_summary>",
                        summary_block
                    ));
                }
            }
        }

        // Section 6: Tool prompt
        if let Some(ref tp) = tool_prompt {
            if !tp.is_empty() {
                system_parts.push(format!("<tools>\n{}\n</tools>", tp));
            }
        }

        // Section 5: Live2D cues
        if let Some(profile) = crate::commands::live2d::load_active_live2d_profile() {
            if !profile.cue_map.is_empty() {
                let cue_lines = profile
                    .cue_map
                    .iter()
                    .filter_map(|(cue, binding)| {
                        (!binding.exclude_from_prompt).then_some(cue.clone())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                if !cue_lines.is_empty() {
                    system_parts.push(format!(
                        "<live2d>\nAvailable cues for the active model: {}.\n\
                         If the current reply clearly fits one of these existing cues, call the play_cue tool at an appropriate moment.\n\
                         When calling play_cue, the cue argument must be exactly one item from this list.\n\
                         Never invent a new cue name from an emotion word or description.\n\
                         Do not rely only on text to describe expressions or actions when a matching cue should be triggered — always call the tool instead.\n\
                         </live2d>",
                        cue_lines
                    ));
                }
            }
        }

        // Section 6: Language requirement
        if !resp_lang.is_empty() {
            system_parts.push(format!(
                "<language>\nYou speak {}. All your replies must be in {}.\n</language>",
                resp_lang, resp_lang
            ));
        }

        // Push the single consolidated system message
        final_messages.push(Message {
            role: "system".to_string(),
            content: system_parts.join("\n\n"),
            metadata: None,
        });

        // -- Translation Instruction (kept separate at end for instruction clarity) --
        {
            let user_lang = self.user_language.lock().await;
            if !user_lang.is_empty() && !resp_lang.is_empty() && *user_lang != resp_lang {
                final_messages.push(Message {
                    role: "system".to_string(),
                    content: format!(
                        "IMPORTANT: After your dialogue response, \
                         append a translation of your ENTIRE dialogue response into {} using this EXACT format:\n\
                         [TRANSLATE: <your entire response translated into {}>]\n\
                         The content inside [TRANSLATE:...] MUST be written in {}, NOT in {}. \
                         This is an explicit exception to the language rule above. \
                         Only translate the dialogue text. Do NOT include any control tags inside the translation.\n\
                         This translation tag is mandatory for every response.",
                        user_lang, user_lang, user_lang, resp_lang
                    ),
                    metadata: Some(serde_json::json!({"type": "translation_instruction"})),
                });
            }
        }
        // -- Recent History (P2) --
        // Token-budget-aware trimming: walk backwards from newest, stop when budget exhausted.
        const CHARS_PER_TOKEN: usize = 2; // conservative for mixed CJK/Latin
        const HISTORY_TOKEN_BUDGET: usize = 6000;
        let budget_chars = HISTORY_TOKEN_BUDGET * CHARS_PER_TOKEN;
        let mut used_chars = 0usize;
        let mut selected: Vec<&Message> = Vec::new();

        for msg in recent_history_snapshot.iter().rev() {
            let msg_chars = msg.content.chars().count();
            if used_chars + msg_chars > budget_chars && !selected.is_empty() {
                break;
            }
            used_chars += msg_chars;
            selected.push(msg);
            if selected.len() >= 20 {
                break;
            }
        }
        selected.reverse();
        for msg in selected {
            final_messages.push(msg.clone());
        }

        // -- Final Language Reminder (recency effect) --
        // Placed after history so it's the last system instruction the LLM sees.
        // LLMs pay strongest attention to the beginning and end of context.
        if !resp_lang.is_empty() {
            final_messages.push(Message {
                role: "system".to_string(),
                content: format!(
                    "[Reminder] Respond in {} only. Do not follow the user's input language.",
                    resp_lang
                ),
                metadata: Some(serde_json::json!({"type": "language_reminder"})),
            });
        }

        // -- Current User Query --
        // (Caller usually adds this, but if we are composing the full context for the LLM API, we need it in history or appended)
        // Assuming caller will append the *current* user message to this list or has already added it to history?
        // Standard pattern: Add generic history, then caller adds current prompt.
        // BUT current prompt is needed for RAG.
        // We will assume the caller handles the *current* message appending to this returned context,
        // OR we can make `compose_prompt` take the current message and add it.
        // Let's stick to returning context *state*.

        Ok(final_messages)
    }

    pub async fn get_context_settings(&self) -> (String, usize) {
        let strategy = self.context_strategy.lock().await.clone();
        let max_chars = *self.max_message_chars.lock().await;
        (strategy, max_chars)
    }

    pub async fn set_context_settings(&self, strategy: String, max_chars: usize) {
        *self.context_strategy.lock().await = strategy;
        *self.max_message_chars.lock().await = max_chars;
    }

    pub async fn clear_history(&self) {
        let mut history = self.history.lock().await;
        history.clear();
        drop(history);
        *self.memory_history_boundary.lock().await = 0;
        *self.memory_trigger_count.lock().await = 0;
        // 清空当前对话 ID，下次发消息时会创建新对话
        let mut conv_id = self.current_conversation_id.lock().await;
        *conv_id = None;
        Self::persist_conversation_id(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_test_orchestrator() -> AIOrchestrator {
        AIOrchestrator::new("sqlite::memory:")
            .await
            .expect("Failed to create test orchestrator")
    }

    #[tokio::test]
    async fn test_add_message_truncation() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator
            .set_character_name("TestChar".to_string())
            .await;

        // Set max_message_chars to 50
        *orchestrator.max_message_chars.lock().await = 50;

        // Add a message longer than 50 chars
        let long_message =
            "This is a very long message that exceeds the maximum character limit".to_string();
        orchestrator
            .add_message("user".to_string(), long_message, "test_char")
            .await;

        let history = orchestrator.history.lock().await;
        assert_eq!(history.len(), 1, "History should contain one message");

        let msg = &history[0];
        assert!(
            msg.content.ends_with("…[truncated]"),
            "Message should end with truncation marker"
        );
        // Check character count, not byte length (ellipsis is multi-byte)
        let char_count = msg.content.chars().count();
        assert!(
            char_count <= 63, // 50 chars + "…[truncated]" (13 chars)
            "Truncated message should not exceed max + marker length, got {} chars",
            char_count
        );
    }

    #[tokio::test]
    async fn test_add_message_rolling_window() {
        let orchestrator = setup_test_orchestrator().await;

        // Add 35 messages (exceeds 20 limit)
        for i in 0..35 {
            orchestrator
                .add_message("user".to_string(), format!("Message {}", i), "test_char")
                .await;
        }

        let history = orchestrator.history.lock().await;
        assert!(
            history.len() <= 20,
            "History should not exceed 20 messages, got {}",
            history.len()
        );
    }

    #[tokio::test]
    async fn test_get_recent_history_fewer_than_n() {
        let orchestrator = setup_test_orchestrator().await;

        // Add 5 messages
        for i in 0..5 {
            orchestrator
                .add_message("user".to_string(), format!("Message {}", i), "test_char")
                .await;
        }

        // Request 10 messages (more than available)
        let recent = orchestrator.get_recent_history(10).await;
        assert_eq!(
            recent.len(),
            5,
            "Should return all 5 messages when requesting more than available"
        );
    }

    #[tokio::test]
    async fn test_get_recent_history_exact_n() {
        let orchestrator = setup_test_orchestrator().await;

        // Add 10 messages
        for i in 0..10 {
            orchestrator
                .add_message("user".to_string(), format!("Message {}", i), "test_char")
                .await;
        }

        // Request exactly 5 messages
        let recent = orchestrator.get_recent_history(5).await;
        assert_eq!(recent.len(), 5, "Should return exactly 5 messages");
        assert_eq!(
            recent[0].content, "Message 5",
            "Should return the last 5 messages"
        );
        assert_eq!(
            recent[4].content, "Message 9",
            "Last message should be Message 9"
        );
    }

    #[tokio::test]
    async fn test_clear_history_resets_state() {
        let orchestrator = setup_test_orchestrator().await;

        // Add some messages
        for i in 0..5 {
            orchestrator
                .add_message("user".to_string(), format!("Message {}", i), "test_char")
                .await;
        }

        // Verify messages were added
        {
            let history = orchestrator.history.lock().await;
            assert_eq!(history.len(), 5, "Should have 5 messages before clear");
        }

        // Clear history
        orchestrator.clear_history().await;

        // Verify all state is reset
        {
            let history = orchestrator.history.lock().await;
            assert_eq!(history.len(), 0, "History should be empty after clear");
        }

        {
            let boundary = *orchestrator.memory_history_boundary.lock().await;
            assert_eq!(boundary, 0, "Memory boundary should be 0 after clear");
        }

        {
            let trigger_count = *orchestrator.memory_trigger_count.lock().await;
            assert_eq!(
                trigger_count, 0,
                "Memory trigger count should be 0 after clear"
            );
        }

        {
            let conv_id = orchestrator.current_conversation_id.lock().await;
            assert_eq!(
                *conv_id, None,
                "Current conversation ID should be None after clear"
            );
        }
    }

    #[tokio::test]
    async fn test_set_memory_enabled_false_resets_trigger_count() {
        let orchestrator = setup_test_orchestrator().await;

        // Add some user messages to increment trigger count
        for i in 0..3 {
            orchestrator
                .add_message("user".to_string(), format!("Message {}", i), "test_char")
                .await;
        }

        // Verify trigger count was incremented
        {
            let trigger_count = *orchestrator.memory_trigger_count.lock().await;
            assert_eq!(
                trigger_count, 3,
                "Trigger count should be 3 after 3 user messages"
            );
        }

        // Disable memory
        orchestrator.set_memory_enabled(false).await;

        // Verify trigger count was reset
        {
            let trigger_count = *orchestrator.memory_trigger_count.lock().await;
            assert_eq!(
                trigger_count, 0,
                "Trigger count should be 0 after disabling memory"
            );
        }

        // Verify memory is disabled
        assert!(
            !orchestrator.is_memory_enabled(),
            "Memory should be disabled"
        );
    }

    #[tokio::test]
    async fn test_set_memory_enabled_sets_boundary() {
        let orchestrator = setup_test_orchestrator().await;

        // Add some messages
        for i in 0..5 {
            orchestrator
                .add_message("user".to_string(), format!("Message {}", i), "test_char")
                .await;
        }

        // Disable memory (should set boundary to current history length)
        orchestrator.set_memory_enabled(false).await;

        let boundary = *orchestrator.memory_history_boundary.lock().await;
        assert_eq!(
            boundary, 5,
            "Boundary should be set to history length (5) when disabling memory"
        );
    }

    #[tokio::test]
    async fn test_push_history_message_respects_rolling_window() {
        let orchestrator = setup_test_orchestrator().await;

        // Manually push 35 messages to exceed the 20 limit
        for i in 0..35 {
            orchestrator
                .push_history_message(Message {
                    role: "user".to_string(),
                    content: format!("Message {}", i),
                    metadata: None,
                })
                .await;
        }

        let history = orchestrator.history.lock().await;
        assert!(
            history.len() <= 20,
            "History should not exceed 20 messages after push_history_message"
        );
    }

    #[tokio::test]
    async fn test_message_count_increments_on_user_message() {
        let orchestrator = setup_test_orchestrator().await;

        // Add user messages
        for i in 0..3 {
            orchestrator
                .add_message("user".to_string(), format!("Message {}", i), "test_char")
                .await;
        }

        let count = *orchestrator.message_count.lock().await;
        assert_eq!(count, 3, "Message count should be 3 after 3 user messages");
    }

    #[tokio::test]
    async fn test_message_count_not_incremented_on_assistant_message() {
        let orchestrator = setup_test_orchestrator().await;

        // Add assistant message
        orchestrator
            .add_message("assistant".to_string(), "Response".to_string(), "test_char")
            .await;

        let count = *orchestrator.message_count.lock().await;
        assert_eq!(
            count, 0,
            "Message count should remain 0 for non-user messages"
        );
    }
}
