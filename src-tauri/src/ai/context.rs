use crate::ai::curiosity::CuriosityModule;
use crate::ai::emotion::{EmotionPersonality, EmotionState};
use crate::ai::idle_behaviors::IdleBehaviorSystem;
use crate::ai::initiative::InitiativeSystem;
use crate::ai::memory::MemoryManager;
use crate::ai::router::{ModelRouter, ModelType};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::str::FromStr;
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
    /// Current character ID for memory isolation.
    character_id: Arc<Mutex<String>>,
    /// Emotion state with per-character personality.
    pub emotion_state: Arc<Mutex<EmotionState>>,
    /// Timestamp of last user activity (for idle detection).
    pub last_activity: Arc<Mutex<Instant>>,
    /// Total message count across sessions (for relationship depth).
    pub conversation_count: Arc<Mutex<u64>>,
    /// Preferred response language (e.g. "日本語", "English"). Empty = auto.
    pub response_language: Arc<Mutex<String>>,
    /// User's display language for inline translation (e.g. "中文"). Empty = disabled.
    pub user_language: Arc<Mutex<String>>,

    // Autonomous Behavior Modules
    pub curiosity: Arc<Mutex<CuriosityModule>>,
    pub initiative: Arc<Mutex<InitiativeSystem>>,
    pub idle_behaviors: Arc<Mutex<IdleBehaviorSystem>>,
    /// Whether proactive (idle auto-talk) messages are enabled.
    pub proactive_enabled: Arc<std::sync::atomic::AtomicBool>,
    /// 当前活跃对话 ID
    pub current_conversation_id: Arc<Mutex<Option<String>>>,
}

impl AIOrchestrator {
    pub async fn new(db_url: &str) -> Result<Self> {
        // Create database if it doesn't exist
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(db_url)?.create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await?;

        // Ensure tables exist
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                embedding BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                importance REAL DEFAULT 0.5,
                character_id TEXT NOT NULL DEFAULT 'default',
                tier TEXT NOT NULL DEFAULT 'ephemeral',
                consolidated_from TEXT
            );",
        )
        .execute(&pool)
        .await?;

        // Migration: add character_id column to existing databases that lack it
        let _ = sqlx::query(
            "ALTER TABLE memories ADD COLUMN character_id TEXT NOT NULL DEFAULT 'default'",
        )
        .execute(&pool)
        .await;

        // Migration: add tier column for tiered memory (Phase 1)
        let _ = sqlx::query(
            "ALTER TABLE memories ADD COLUMN tier TEXT NOT NULL DEFAULT 'ephemeral'",
        )
        .execute(&pool)
        .await;

        // Migration: add consolidated_from column for memory consolidation (Phase 3)
        let _ = sqlx::query(
            "ALTER TABLE memories ADD COLUMN consolidated_from TEXT",
        )
        .execute(&pool)
        .await;

        // FTS5 virtual table for hybrid retrieval (Phase 2)
        sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(content, content='memories', content_rowid='id');",
        )
        .execute(&pool)
        .await?;

        // Sync triggers: keep FTS index in sync with memories table
        let _ = sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
            END;",
        )
        .execute(&pool)
        .await;

        let _ = sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content);
            END;",
        )
        .execute(&pool)
        .await;

        let _ = sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content);
                INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
            END;",
        )
        .execute(&pool)
        .await;

        // Rebuild FTS index to backfill any existing data
        let _ = sqlx::query(
            "INSERT INTO memories_fts(memories_fts) VALUES('rebuild');",
        )
        .execute(&pool)
        .await;

        // 对话记录持久化表
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                character_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '新对话',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS conversation_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
            );",
        )
        .execute(&pool)
        .await?;

        let memory_manager = Arc::new(MemoryManager::new(pool.clone()));

        Ok(Self {
            db: pool,
            system_prompt: Arc::new(Mutex::new("You are a helpful assistant.".to_string())),
            history: Arc::new(Mutex::new(VecDeque::new())),
            max_history_tokens: 4000,
            memory_manager,
            router: Arc::new(ModelRouter::new()),
            message_count: Arc::new(Mutex::new(0)),
            character_id: Arc::new(Mutex::new("default".to_string())),
            emotion_state: Arc::new(Mutex::new(EmotionState::new(EmotionPersonality::default()))),
            last_activity: Arc::new(Mutex::new(Instant::now())),
            conversation_count: Arc::new(Mutex::new(0)),
            response_language: Arc::new(Mutex::new(String::new())),
            user_language: Arc::new(Mutex::new(String::new())),
            curiosity: Arc::new(Mutex::new(CuriosityModule::new())),
            initiative: Arc::new(Mutex::new(InitiativeSystem::new())),
            idle_behaviors: Arc::new(Mutex::new(IdleBehaviorSystem::new())),
            proactive_enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            current_conversation_id: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn set_system_prompt(&self, prompt: String) {
        // Parse emotion personality from the new persona text
        let personality = EmotionPersonality::parse_from_persona(&prompt);
        {
            let mut emotion = self.emotion_state.lock().await;
            emotion.set_personality(personality);
        }
        let mut sp = self.system_prompt.lock().await;
        *sp = prompt;
    }

    pub async fn set_response_language(&self, language: String) {
        let mut lang = self.response_language.lock().await;
        *lang = language;
    }

    pub async fn set_user_language(&self, language: String) {
        let mut lang = self.user_language.lock().await;
        *lang = language;
    }

    /// Enable or disable proactive (idle auto-talk) messages.
    pub fn set_proactive_enabled(&self, enabled: bool) {
        self.proactive_enabled.store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    /// Check if proactive messages are enabled.
    pub fn is_proactive_enabled(&self) -> bool {
        self.proactive_enabled.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Update emotion state with smoothing and return the smoothed values.
    pub async fn update_emotion(&self, raw_emotion: &str, raw_mood: f32) -> (String, f32) {
        let mut emotion = self.emotion_state.lock().await;
        emotion.update(raw_emotion, raw_mood)
    }

    /// Get a natural-language description of current emotion for prompt injection.
    pub async fn get_emotion_description(&self) -> String {
        let emotion = self.emotion_state.lock().await;
        emotion.describe()
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
        let changed = *cid != id;
        *cid = id.clone();
        drop(cid);

        // Restore emotion snapshot from disk when switching characters
        if changed {
            if let Ok(Some(snap)) = self.memory_manager.load_emotion_snapshot(&id).await {
                let mut emotion = self.emotion_state.lock().await;
                emotion.restore_from_snapshot(&snap);
                println!(
                    "[Emotion] Restored snapshot for '{}': {} (mood={:.2})",
                    id, snap.emotion, snap.mood
                );
            }
        }
    }

    pub async fn get_character_id(&self) -> String {
        self.character_id.lock().await.clone()
    }

    pub async fn add_message(&self, role: String, content: String) {
        // Track user message count for memory extraction triggers
        if role == "user" {
            let mut count = self.message_count.lock().await;
            *count += 1;
        }

        let mut history = self.history.lock().await;
        history.push_back(Message {
            role: role.clone(),
            content: content.clone(),
            metadata: None,
        });

        // Basic rolling window by count for now, implementation should be token-based primarily
        if history.len() > 30 {
            history.pop_front();
        }
        drop(history);

        // 持久化到数据库
        let _ = self.persist_message(&role, &content).await;
    }

    /// 将消息持久化到 SQLite，如果没有活跃对话则自动创建
    async fn persist_message(&self, role: &str, content: &str) -> Result<()> {
        let cid = self.character_id.lock().await.clone();
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
                "INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES (?, ?, ?, ?, ?)"
            )
            .bind(&new_id)
            .bind(&cid)
            .bind(&title)
            .bind(&now)
            .bind(&now)
            .execute(&self.db)
            .await?;

            *conv_id_lock = Some(new_id.clone());
            new_id
        };
        drop(conv_id_lock);

        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, created_at) VALUES (?, ?, ?, ?)"
        )
        .bind(&conv_id)
        .bind(role)
        .bind(content)
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

    /// Returns the total count of user messages in this session.
    pub async fn get_message_count(&self) -> u64 {
        *self.message_count.lock().await
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

    /// Composes a prompt based on the user query, budgeting tokens for context
    pub async fn compose_prompt(
        &self,
        query: &str,
        _allow_image_gen: bool,
        _tool_prompt: Option<String>,
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
        let cid = self.character_id.lock().await.clone();
        let memories = self
            .memory_manager
            .search_memories(query, 5, &cid)
            .await
            .ok();

        let sp = self.system_prompt.lock().await;
        let history = self.history.lock().await;

        // -- Read response language early so all sections can reference it --
        let resp_lang = {
            let lang = self.response_language.lock().await;
            lang.clone()
        };

        let mut final_messages = Vec::new();

        // -- System Prompt (P0) --
        // Embed language requirement early for primacy effect
        let lang_preamble = if !resp_lang.is_empty() {
            format!("\n\n[LANGUAGE: You speak {}. All your replies must be in {}.]", resp_lang, resp_lang)
        } else {
            String::new()
        };
        final_messages.push(Message {
            role: "system".to_string(),
            content: format!(
                "{}\n\n{}{}",
                sp.clone(),
                crate::ai::prompts::CORE_PERSONA_PROMPT,
                lang_preamble
            ),
            metadata: None,
        });

        // -- Emotion Context (P0.25) --
        // Inject current emotional state so the LLM responds in character
        let (emotion_desc, current_mood, current_emotion, mood_hist, _expressiveness) = {
            let emotion = self.emotion_state.lock().await;
            (
                emotion.describe(),
                emotion.mood(),
                emotion.current_emotion().to_string(),
                emotion.mood_history(),
                emotion.personality().expressiveness,
            )
        };
        final_messages.push(Message {
            role: "system".to_string(),
            content: emotion_desc,
            metadata: Some(serde_json::json!({"type": "emotion_context"})),
        });

        // -- Style Adaptation (P0.3) --
        // Adjust speaking style based on relationship depth and mood
        let conversation_count = self.get_conversation_count().await;
        let style = crate::ai::style_adapter::compute_style(
            conversation_count,
            current_mood,
            &current_emotion,
        );
        final_messages.push(Message {
            role: "system".to_string(),
            content: style.prompt_instruction,
            metadata: Some(
                serde_json::json!({"type": "style_directive", "tier": format!("{:?}", style.tier)}),
            ),
        });

        // -- Emotion Events (P0.35) --
        // Inject special behavior instructions when mood is extreme
        let emotion_events =
            crate::ai::emotion_events::check_emotion_triggers(current_mood, &mood_hist);
        for event in &emotion_events {
            final_messages.push(Message {
                role: "system".to_string(),
                content: event.system_instruction.clone(),
                metadata: Some(serde_json::json!({"type": "emotion_event", "event": format!("{:?}", event.event_type)})),
            });
        }

        // -- Relevant Memories (P1) --
        // Upgraded: instruct the LLM to naturally reference these in conversation
        if let Some(mems) = memories {
            if !mems.is_empty() {
                let memory_block = mems
                    .iter()
                    .map(|m| format!("- {}", m.content))
                    .collect::<Vec<_>>()
                    .join("\n");

                final_messages.push(Message {
                    role: "system".to_string(),
                    content: format!(
                        concat!(
                            "You remember these things about the user:\n{}\n\n",
                            "Naturally reference these memories in conversation so the user feels you truly remember what they said. ",
                            "Do not list them mechanically; weave them naturally into the dialogue. ",
                            "If a memory is not relevant to the current topic, do not force it."
                        ),
                        memory_block
                    ),
                    metadata: Some(serde_json::json!({"type": "memory_injection"})),
                });
            }
        }

        // -- Session Summaries (P1.5) --
        // Inject recent session summaries so the character remembers past conversations
        if let Ok(summaries) = self.memory_manager.get_recent_summaries(&cid, 2).await {
            if !summaries.is_empty() {
                let summary_block = summaries
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("{}. {}", i + 1, s))
                    .collect::<Vec<_>>()
                    .join("\n");
                final_messages.push(Message {
                    role: "system".to_string(),
                    content: format!(
                        "Previous conversation summaries (most recent first):\n{}",
                        summary_block
                    ),
                    metadata: Some(serde_json::json!({"type": "session_summary"})),
                });
            }
        }

        // -- Response Language Instruction (moved here for higher attention) --
        // Force the LLM to respond in the user-configured language.
        // Placed just before history so the LLM pays more attention to it.
        if !resp_lang.is_empty() {
            final_messages.push(Message {
                role: "system".to_string(),
                content: format!(
                    "CRITICAL INSTRUCTION — LANGUAGE REQUIREMENT:\n\
                     You MUST respond ENTIRELY in {}. \
                     Regardless of what language the user writes in, \
                     your reply MUST be written in {} only. \
                     Do NOT switch to the user's input language. This is non-negotiable.",
                    resp_lang, resp_lang
                ),
                metadata: Some(serde_json::json!({"type": "language_instruction"})),
            });
        }

        // -- Translation Instruction --
        // When response language and user language differ, ask LLM to append inline translation
        {
            let user_lang = self.user_language.lock().await;
            if !user_lang.is_empty() && !resp_lang.is_empty() && *user_lang != resp_lang {
                final_messages.push(Message {
                    role: "system".to_string(),
                    content: format!(
                        "IMPORTANT: After your dialogue response (but BEFORE the [ACTION:...] and [EMOTION:...] tags), \
                         append a translation of your ENTIRE dialogue response into {} using this EXACT format:\n\
                         [TRANSLATE: <your entire response translated into {}>]\n\
                         Only translate the dialogue text. Do NOT include any control tags like [ACTION:...], [EMOTION:...], or [IMAGE_PROMPT:...] inside the translation.\n\
                         This translation tag is mandatory for every response.",
                        user_lang, user_lang
                    ),
                    metadata: Some(serde_json::json!({"type": "translation_instruction"})),
                });
            }
        }

        // -- Recent History (P2) --
        // Simple strategy: take last N messages that fit
        // A real tokenizer count is needed here for precision.
        // We will approximate 1 word = 1.3 tokens or just take last 10 messages.
        let recent_count = 10;
        let start_index = if history.len() > recent_count {
            history.len() - recent_count
        } else {
            0
        };

        for msg in history.iter().skip(start_index) {
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

    pub async fn clear_history(&self) {
        let mut history = self.history.lock().await;
        history.clear();
        // 清空当前对话 ID，下次发消息时会创建新对话
        let mut conv_id = self.current_conversation_id.lock().await;
        *conv_id = None;
    }
}
