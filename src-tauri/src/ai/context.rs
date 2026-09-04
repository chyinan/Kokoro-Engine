// pattern: Mixed (needs refactoring)

use crate::ai::curiosity::CuriosityModule;
use crate::ai::database_migrations;
use crate::ai::idle_behaviors::IdleBehaviorSystem;
use crate::ai::initiative::InitiativeSystem;
use crate::ai::memory::{MemoryManager, MemoryRetrievalMode, MemorySearchResult};
use crate::ai::router::{ModelRouter, ModelType};
use crate::llm::messages::user_text_message;
use crate::llm::provider::LlmProvider;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet, VecDeque};
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

pub fn is_vision_context_message(message: &Message) -> bool {
    message.role == "context"
        || message
            .metadata
            .as_ref()
            .and_then(|meta| meta.get("type"))
            .and_then(|value| value.as_str())
            == Some("vision_observation")
}

pub fn is_memory_candidate_message(message: &Message) -> bool {
    !is_vision_context_message(message)
}

pub fn is_summary_candidate_message(message: &Message) -> bool {
    !is_vision_context_message(message)
}

fn latest_vision_context_index(messages: &[Message]) -> Option<usize> {
    messages.iter().rposition(is_vision_context_message)
}

fn should_include_message_for_llm_history(
    message: &Message,
    index: usize,
    latest_vision_index: Option<usize>,
    vision_context_history_mode: &str,
) -> bool {
    let technical_type = message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("type"))
        .and_then(|value| value.as_str());

    if matches!(technical_type, Some("translation_instruction")) {
        return false;
    }

    if is_vision_context_message(message) && vision_context_history_mode != "full" {
        return latest_vision_index == Some(index);
    }

    true
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

pub const MAX_IN_MEMORY_HISTORY_MESSAGES: usize = 20;
pub const DEFAULT_MAX_MESSAGE_CHARS: usize = 2000;
const TRUNCATION_MARKER: &str = "…[truncated]";

pub fn truncate_message_content(content: String, max_chars: usize) -> String {
    if content.chars().count() > max_chars {
        let truncated: String = content.chars().take(max_chars).collect();
        format!("{truncated}{TRUNCATION_MARKER}")
    } else {
        content
    }
}

pub trait IntoHistoryMessage {
    fn into_history_message(self, max_chars: usize) -> Message;
}

impl IntoHistoryMessage for (String, String, Option<String>) {
    fn into_history_message(self, max_chars: usize) -> Message {
        Message {
            role: self.0,
            content: truncate_message_content(self.1, max_chars),
            metadata: self
                .2
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
        }
    }
}

impl IntoHistoryMessage for Message {
    fn into_history_message(mut self, max_chars: usize) -> Message {
        self.content = truncate_message_content(self.content, max_chars);
        self
    }
}

/// 统一历史窗口同步函数：
/// 1. 严格保留最近的至多 MAX_IN_MEMORY_HISTORY_MESSAGES (20) 条消息；
/// 2. 对每条消息应用 max_chars 截断；
/// 3. 清空并重构 history；
/// 4. 返回新历史长度，用于对齐 memory_history_boundary。
pub fn sync_history_window<I, T>(
    history: &mut VecDeque<Message>,
    items: I,
    max_chars: usize,
) -> usize
where
    I: IntoIterator<Item = T>,
    T: IntoHistoryMessage,
{
    let items: Vec<_> = items.into_iter().collect();
    let start = items.len().saturating_sub(MAX_IN_MEMORY_HISTORY_MESSAGES);
    history.clear();
    for item in items.into_iter().skip(start) {
        history.push_back(item.into_history_message(max_chars));
    }
    history.len()
}

fn normalized_language_name(language: &str) -> Option<&str> {
    let trimmed = language.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn memory_write_language_instruction(response_language: &str) -> Option<String> {
    let language = normalized_language_name(response_language)?;
    Some(format!(
        "When writing or updating memory entries, write the stored memory text in {language}. \
         This includes the fact argument for store_memory. If the source text uses another language, \
         translate or summarize it into {language}; preserve proper nouns, code identifiers, \
         product names, and exact quoted phrases only when necessary."
    ))
}

/// Preserves keyword/BM25 snippets when semantic retrieval is unavailable.
fn memory_context_for_prompt(result: MemorySearchResult) -> (Vec<MemorySnippet>, Option<String>) {
    let warning = (result.mode == MemoryRetrievalMode::Unavailable).then(|| {
        "memory semantic retrieval unavailable; keyword results still included".to_string()
    });
    (result.snippets, warning)
}

fn build_conversation_summary_prompt(transcript: &str, target_language: &str) -> String {
    let language_requirement = normalized_language_name(target_language)
        .map(|language| {
            format!(
                " Write the summary in {language}. If the conversation uses another language, translate or summarize it into {language}."
            )
        })
        .unwrap_or_default();

    format!(
        "Summarize the following conversation in 2-3 sentences, focusing on key facts, \
         decisions, emotional shifts, and unresolved threads.{language_requirement} \
         Output only the summary, no preamble.\n\n{}",
        transcript
    )
}

#[derive(Debug)]
pub struct ActivationGate {
    lock: Arc<tokio::sync::RwLock<()>>,
    activating: Arc<AtomicBool>,
}

impl Default for ActivationGate {
    fn default() -> Self {
        Self {
            lock: Arc::new(tokio::sync::RwLock::new(())),
            activating: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct ActivationLockGuard {
    _write_guard: tokio::sync::OwnedRwLockWriteGuard<()>,
    activating: Arc<AtomicBool>,
    degraded: Option<Arc<Mutex<Option<String>>>>,
    fail_closed: bool,
    completed: bool,
}

impl ActivationLockGuard {
    pub fn arm_mutation(&mut self) {
        self.fail_closed = true;
    }

    pub fn mark_completed(&mut self) {
        self.completed = true;
    }

    pub fn set_degraded_target(&mut self, degraded: Arc<Mutex<Option<String>>>) {
        self.degraded = Some(degraded);
    }
}

impl std::fmt::Debug for ActivationLockGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivationLockGuard")
            .field("activating", &self.activating.load(Ordering::SeqCst))
            .field("fail_closed", &self.fail_closed)
            .field("completed", &self.completed)
            .finish()
    }
}

impl Drop for ActivationLockGuard {
    fn drop(&mut self) {
        if self.completed || !self.fail_closed {
            // Clean exit: either activation completed cleanly, or it exited early (e.g. character not found,
            // stale token, db begin failure) before entering irreversible runtime mutation.
            self.activating.store(false, Ordering::SeqCst);
        } else {
            // Guard dropped after entering irreversible mutation without being marked completed (e.g. cancelled in-flight / aborted / panic).
            // 1. Keep activating = true to prevent new chat turns from entering an unverified/torn state.
            // 2. Mark runtime degraded if degraded hook is available so prompt composition is also blocked.
            if let Some(degraded) = &self.degraded {
                if let Ok(mut lock) = degraded.try_lock() {
                    if lock.is_none() {
                        *lock = Some(
                            "Character activation was interrupted before completion. Please re-activate or select a character."
                                .to_string(),
                        );
                    }
                }
            }
            tracing::warn!(
                "ActivationLockGuard dropped without completion after mutation armed (in-flight cancellation); keeping activation gate closed and runtime degraded."
            );
        }
    }
}

pub struct ChatTurnGuard {
    _read_guard: tokio::sync::OwnedRwLockReadGuard<()>,
}

impl std::fmt::Debug for ChatTurnGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatTurnGuard").finish()
    }
}

struct ActivationReservation {
    activating: Arc<AtomicBool>,
    was_already_active: bool,
    disarmed: bool,
}

impl ActivationReservation {
    fn new(activating: Arc<AtomicBool>) -> Self {
        let was_already_active = activating.swap(true, Ordering::SeqCst);
        Self {
            activating,
            was_already_active,
            disarmed: false,
        }
    }

    fn disarm(mut self) {
        self.disarmed = true;
    }
}

impl Drop for ActivationReservation {
    fn drop(&mut self) {
        if !self.disarmed && !self.was_already_active {
            self.activating.store(false, Ordering::SeqCst);
        }
    }
}

impl ActivationGate {
    pub async fn acquire_activation_lock(&self) -> ActivationLockGuard {
        let reservation = ActivationReservation::new(self.activating.clone());
        let write_guard = self.lock.clone().write_owned().await;
        reservation.disarm();
        ActivationLockGuard {
            _write_guard: write_guard,
            activating: self.activating.clone(),
            degraded: None,
            fail_closed: false,
            completed: false,
        }
    }

    pub fn enter_chat_turn(&self) -> Result<ChatTurnGuard, String> {
        if self.activating.load(Ordering::SeqCst) {
            return Err(
                "Character activation is in progress. Please retry after activation completes."
                    .to_string(),
            );
        }
        match self.lock.clone().try_read_owned() {
            Ok(read_guard) => {
                if self.activating.load(Ordering::SeqCst) {
                    return Err(
                        "Character activation is in progress. Please retry after activation completes."
                            .to_string(),
                    );
                }
                Ok(ChatTurnGuard {
                    _read_guard: read_guard,
                })
            }
            Err(_) => Err(
                "Character activation is in progress. Please retry after activation completes."
                    .to_string(),
            ),
        }
    }

    pub fn is_activating(&self) -> bool {
        self.activating.load(Ordering::SeqCst)
    }
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
    pub(crate) memory_history_boundary: Arc<Mutex<usize>>,
    /// Current character ID for memory isolation.
    pub(crate) character_id: Arc<Mutex<String>>,
    /// In-memory cooldown map for memory event trigger throttling.
    memory_event_cooldowns: Arc<Mutex<HashMap<String, Instant>>>,
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
    /// Jailbreak prompt prefix (prepended to all system prompts). None = not loaded from disk yet, Some("") = explicitly empty.
    pub jailbreak_prompt: Arc<Mutex<Option<String>>>,
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
    /// Serializes conversation history/pointer rewrites (delete/edit/clear/switch/activation)
    /// so a stale operation can never clobber the active conversation's in-memory context.
    /// Must be the OUTERMOST lock wherever it is taken.
    pub conversation_switch_lock: Arc<Mutex<()>>,
    /// Whether this orchestrator should update the desktop hot-reload conversation pointer.
    persist_conversation_selection: bool,
    /// Context management strategy: "window" | "summary"
    pub context_strategy: Arc<Mutex<String>>,
    /// Max characters per message before truncation
    pub max_message_chars: Arc<Mutex<usize>>,
    /// Screen context history injected into the LLM prompt: "latest" | "full".
    pub vision_context_history_mode: Arc<Mutex<String>>,
    /// If startup or activation recovery failed, stores the degradation reason.
    /// While set, LLM chat requests are blocked to prevent cross-character context leakage.
    pub runtime_degraded: Arc<Mutex<Option<String>>>,
    /// Concurrency gate to block chat turns during character activation and recovery.
    pub activation_gate: Arc<ActivationGate>,
}

impl AIOrchestrator {
    pub async fn new(db_url: &str) -> Result<Self> {
        // Create database if it doesn't exist
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(db_url)?.create_if_missing(true);
        Self::with_connect_options(options).await
    }

    /// Test-only constructor that can disable SQLite foreign key enforcement
    /// so failure paths (orphan inserts, missing parent tables) can be
    /// exercised deterministically. Production always runs with FK ON.
    #[cfg(test)]
    async fn new_for_tests_with_foreign_keys(db_url: &str, fk_on: bool) -> Result<Self> {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(db_url)?
            .create_if_missing(true)
            .foreign_keys(fk_on);
        Self::with_connect_options(options).await
    }

    /// Shared constructor body so tests can tweak connection options.
    async fn with_connect_options(options: sqlx::sqlite::SqliteConnectOptions) -> Result<Self> {
        let pool = SqlitePool::connect_with(options).await?;

        // Run all database migrations
        database_migrations::run(&pool).await?;

        let memory_manager = Arc::new(MemoryManager::new(pool.clone()));
        let interrupted = memory_manager.mark_interrupted_dream_jobs().await?;
        if interrupted > 0 {
            tracing::warn!(
                target: "memory",
                "[Memory] Marked {} interrupted dream job(s) from a previous process",
                interrupted
            );
        }

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
            memory_event_cooldowns: Arc::new(Mutex::new(HashMap::new())),
            memory_enabled: Arc::new(AtomicBool::new(true)),
            last_activity: Arc::new(Mutex::new(Instant::now())),
            conversation_count: Arc::new(Mutex::new(0)),
            response_language: Arc::new(Mutex::new(String::new())),
            user_language: Arc::new(Mutex::new(String::new())),
            jailbreak_prompt: Arc::new(Mutex::new(None)),
            character_name: Arc::new(Mutex::new("Kokoro".to_string())),
            user_name: Arc::new(Mutex::new("User".to_string())),
            curiosity: Arc::new(Mutex::new(CuriosityModule::new())),
            initiative: Arc::new(Mutex::new(InitiativeSystem::new())),
            idle_behaviors: Arc::new(Mutex::new(IdleBehaviorSystem::new())),
            proactive_enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            current_conversation_id: Arc::new(Mutex::new(None)),
            conversation_switch_lock: Arc::new(Mutex::new(())),
            persist_conversation_selection: true,
            context_strategy: Arc::new(Mutex::new("window".to_string())),
            max_message_chars: Arc::new(Mutex::new(2000)),
            vision_context_history_mode: Arc::new(Mutex::new("latest".to_string())),
            runtime_degraded: Arc::new(Mutex::new(None)),
            activation_gate: Arc::new(ActivationGate::default()),
        })
    }

    pub async fn fork_with_isolated_history(&self) -> Self {
        Self {
            db: self.db.clone(),
            system_prompt: Arc::new(Mutex::new(self.system_prompt.lock().await.clone())),
            history: Arc::new(Mutex::new(VecDeque::new())),
            max_history_tokens: self.max_history_tokens,
            memory_manager: self.memory_manager.clone(),
            router: self.router.clone(),
            message_count: Arc::new(Mutex::new(0)),
            memory_trigger_count: Arc::new(Mutex::new(0)),
            memory_history_boundary: Arc::new(Mutex::new(0)),
            character_id: Arc::new(Mutex::new(self.character_id.lock().await.clone())),
            memory_event_cooldowns: self.memory_event_cooldowns.clone(),
            memory_enabled: self.memory_enabled.clone(),
            last_activity: Arc::new(Mutex::new(Instant::now())),
            conversation_count: Arc::new(Mutex::new(0)),
            response_language: Arc::new(Mutex::new(self.response_language.lock().await.clone())),
            user_language: Arc::new(Mutex::new(self.user_language.lock().await.clone())),
            jailbreak_prompt: Arc::new(Mutex::new(self.jailbreak_prompt.lock().await.clone())),
            character_name: Arc::new(Mutex::new(self.character_name.lock().await.clone())),
            user_name: Arc::new(Mutex::new(self.user_name.lock().await.clone())),
            curiosity: self.curiosity.clone(),
            initiative: self.initiative.clone(),
            idle_behaviors: self.idle_behaviors.clone(),
            proactive_enabled: self.proactive_enabled.clone(),
            current_conversation_id: Arc::new(Mutex::new(None)),
            conversation_switch_lock: Arc::new(Mutex::new(())),
            persist_conversation_selection: false,
            context_strategy: self.context_strategy.clone(),
            max_message_chars: self.max_message_chars.clone(),
            vision_context_history_mode: self.vision_context_history_mode.clone(),
            runtime_degraded: self.runtime_degraded.clone(),
            activation_gate: self.activation_gate.clone(),
        }
    }

    pub async fn acquire_activation_lock(&self) -> ActivationLockGuard {
        let mut guard = self.activation_gate.acquire_activation_lock().await;
        guard.set_degraded_target(self.runtime_degraded.clone());
        guard
    }

    pub fn enter_chat_turn(&self) -> Result<ChatTurnGuard, String> {
        self.activation_gate.enter_chat_turn()
    }

    pub fn is_activating(&self) -> bool {
        self.activation_gate.is_activating()
    }

    pub async fn set_runtime_degraded(&self, reason: Option<String>) {
        *self.runtime_degraded.lock().await = reason;
    }

    pub async fn get_runtime_degraded(&self) -> Option<String> {
        self.runtime_degraded.lock().await.clone()
    }

    pub async fn clear_runtime_degraded(&self) {
        *self.runtime_degraded.lock().await = None;
    }

    pub async fn apply_character_profile(&self, character_id: &str) -> Result<bool> {
        let profile = sqlx::query_as::<_, (String, String, String)>(
            "SELECT name, persona, user_nickname FROM characters WHERE id = ?",
        )
        .bind(character_id)
        .fetch_optional(&self.db)
        .await?;
        let Some((name, persona, user_nickname)) = profile else {
            return Ok(false);
        };

        *self.system_prompt.lock().await = persona;
        *self.character_name.lock().await = name;
        *self.user_name.lock().await = if user_nickname.trim().is_empty() {
            "User".to_string()
        } else {
            user_nickname
        };
        *self.character_id.lock().await = character_id.to_string();
        Ok(true)
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
        *jp = Some(prompt);
    }

    pub async fn get_jailbreak_prompt(&self) -> Option<String> {
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
        if let Err(e) = self
            .add_message_with_metadata(role, content, None, character_id, None)
            .await
        {
            tracing::error!(
                target: "context",
                "[Context] Failed to persist message: {}",
                e
            );
        }
    }

    pub async fn add_message_with_metadata(
        &self,
        role: String,
        content: String,
        metadata: Option<String>,
        character_id: &str,
        summary_provider: Option<Arc<dyn LlmProvider>>,
    ) -> Result<(String, i64)> {
        self.add_message_with_metadata_for_conversation(
            role,
            content,
            metadata,
            character_id,
            None,
            summary_provider,
        )
        .await
    }

    pub async fn add_message_with_metadata_for_conversation(
        &self,
        role: String,
        content: String,
        metadata: Option<String>,
        character_id: &str,
        target_conversation_id: Option<&str>,
        summary_provider: Option<Arc<dyn LlmProvider>>,
    ) -> Result<(String, i64)> {
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

        // Truncate single message before it enters persisted conversation history.
        let max_chars = *self.max_message_chars.lock().await;
        let content = truncate_message_content(content, max_chars);

        // Persist to database FIRST so no code path can skip it. The message
        // row and the conversation metadata bump (title/updated_at) commit
        // atomically inside persist_message: if the metadata update fails, the
        // whole persist errors and nothing half-saved is reported as success.
        let (persisted_conv_id, persisted_msg_id) = self
            .persist_message(
                &role,
                &content,
                metadata.as_deref(),
                character_id,
                target_conversation_id,
            )
            .await?;
        // 会话切换锁：conv 校验与 history push 必须原子，防止会话切换窗口内
        // 把旧会话的消息推入新会话的内存历史
        let _switch_guard = self.conversation_switch_lock.lock().await;
        let current_conversation_id = self.current_conversation_id.lock().await.clone();

        // Only push to in-memory history if target_conversation_id is either None (implicit current)
        // or matches the active current_conversation_id. This prevents an old turn from corrupting
        // the active conversation after a conversation switch or clearHistory.
        let should_push_history = match target_conversation_id {
            Some(target_id) => current_conversation_id.as_deref() == Some(target_id),
            None => true,
        };

        if should_push_history {
            let mut history = self.history.lock().await;
            let parsed_metadata = metadata
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            history.push_back(Message {
                role: role.clone(),
                content: content.clone(),
                metadata: parsed_metadata,
            });

            // Rolling window: keep at most MAX_IN_MEMORY_HISTORY_MESSAGES messages in memory. Summary generation is now
            // non-destructive and derives from persisted conversation_messages instead of popped history.
            let evicted = if history.len() > MAX_IN_MEMORY_HISTORY_MESSAGES {
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
        drop(_switch_guard);

        let strategy = self.context_strategy.lock().await.clone();
        if strategy == "summary" && self.is_memory_enabled() {
            let conv_for_summary = target_conversation_id
                .map(|s| s.to_string())
                .or(current_conversation_id);
            if let (Some(conversation_id), Some(provider)) = (conv_for_summary, summary_provider) {
                let memory_manager = self.memory_manager.clone();
                let cid = character_id.to_string();
                let summary_language = self.response_language.lock().await.clone();
                tauri::async_runtime::spawn(async move {
                    let task = match memory_manager
                        .get_conversation_summary_task(&conversation_id, &cid)
                        .await
                    {
                        Ok(Some(task)) => task,
                        Ok(None) => return,
                        Err(e) => {
                            tracing::error!(
                                target: "context",
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
                        tracing::error!(
                            target: "context",
                            "[Context] Failed to mark summary task running for '{}': {}",
                            conversation_id, e
                        );
                        return;
                    }

                    let prompt =
                        build_conversation_summary_prompt(&task.transcript, &summary_language);

                    match provider.chat(vec![user_text_message(prompt)], None).await {
                        Ok(text) if !text.trim().is_empty() => {
                            let summary = text.trim().to_string();
                            if let Err(e) = memory_manager
                                .complete_conversation_summary(task.record_id, &summary)
                                .await
                            {
                                tracing::error!(
                                    target: "context",
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

        Ok((persisted_conv_id, persisted_msg_id))
    }

    /// 将消息持久化到 SQLite，如果没有活跃对话且未指定 target_conversation_id 则自动创建
    async fn persist_message(
        &self,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        character_id: &str,
        target_conversation_id: Option<&str>,
    ) -> Result<(String, i64)> {
        let cid = character_id;
        let conv_id = if let Some(target_id) = target_conversation_id {
            target_id.to_string()
        } else {
            let mut conv_id_lock = self.current_conversation_id.lock().await;

            let resolved_id = if let Some(ref id) = *conv_id_lock {
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
                if self.persist_conversation_selection {
                    Self::persist_conversation_id(Some(&new_id));
                }
                new_id
            };
            drop(conv_id_lock);
            resolved_id
        };

        let now = chrono::Utc::now().to_rfc3339();

        // Message row and conversation metadata (title/updated_at) are written
        // in a single transaction: a metadata UPDATE failure rolls the message
        // row back too, so callers never observe a half-saved turn reported as
        // success.
        let mut tx = self.db.begin().await?;
        let insert_res = sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&conv_id)
        .bind(role)
        .bind(content)
        .bind(metadata)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to insert message row for conversation '{}'", conv_id))?;
        let message_id = insert_res.last_insert_rowid();

        // 更新对话的 updated_at。If a hidden/context row created the
        // conversation first, let the first visible user turn restore the
        // normal user-derived title.
        let meta_update = if role == "user" {
            let chars: Vec<char> = content.chars().collect();
            let new_title = if chars.len() > 20 {
                format!("{}...", chars[..20].iter().collect::<String>())
            } else {
                content.to_string()
            };
            sqlx::query(
                "UPDATE conversations SET title = CASE WHEN title = '新对话' THEN ? ELSE title END, updated_at = ? WHERE id = ?"
            )
            .bind(&new_title)
            .bind(&now)
            .bind(&conv_id)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!(
                    "failed to update title/updated_at for conversation '{}' after persisting message {}",
                    conv_id, message_id
                )
            })?
        } else {
            sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
                .bind(&now)
                .bind(&conv_id)
                .execute(&mut *tx)
                .await
                .with_context(|| {
                    format!(
                        "failed to update updated_at for conversation '{}' after persisting message {}",
                        conv_id, message_id
                    )
                })?
        };

        // The conversation row can vanish mid-turn when the user deletes the
        // conversation concurrently (FK cascade takes care of the message row).
        // Nothing is left to update; that is a benign race, not a failure.
        if meta_update.rows_affected() == 0 {
            tracing::warn!(
                target: "context",
                "[Context] Conversation '{}' vanished while persisting message {}; metadata update skipped",
                conv_id, message_id
            );
        }

        tx.commit().await.with_context(|| {
            format!(
                "failed to commit persistence for conversation '{}', message {}",
                conv_id, message_id
            )
        })?;

        Ok((conv_id, message_id))
    }

    /// Persist conversation_id to disk for hot-reload recovery.
    pub fn persist_conversation_id(id: Option<&str>) {
        let app_data = dirs_next::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("com.chyin.kokoro");
        let _ = std::fs::create_dir_all(&app_data);
        let path = app_data.join("current_conversation_id.json");
        let json = serde_json::json!({ "conversation_id": id });
        if let Err(e) = std::fs::write(&path, json.to_string()) {
            tracing::error!(target: "context", "[Context] Failed to persist conversation_id: {}", e);
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
            tracing::error!(target: "context", "[Context] Failed to persist active_character_id: {}", e);
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

    /// Insert a streaming assistant draft into the DB for a specific conversation. Returns the row id for later update.
    pub async fn persist_streaming_draft(
        &self,
        conversation_id: &str,
        content: &str,
    ) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES (?, 'assistant', ?, NULL, ?)"
        )
        .bind(conversation_id)
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

        // Update owning conversation updated_at directly via message row's conversation_id
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE conversations SET updated_at = ? WHERE id = (SELECT conversation_id FROM conversation_messages WHERE id = ?)",
        )
        .bind(&now)
        .bind(row_id)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    pub async fn delete_message_by_id(&self, row_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM conversation_messages WHERE id = ?")
            .bind(row_id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Delete the DB rows left behind by a cancelled or failed chat turn:
    /// - rows whose metadata carries the turn's `turn_id` and a technical `type`
    ///   (`assistant_tool_calls` | `tool_result`);
    /// - `extra_row_ids` whose metadata is still NULL (unfinalized streaming drafts).
    ///
    /// A row passed via `extra_row_ids` with non-NULL metadata (e.g. a finalized
    /// assistant message) is never removed, so a fully generated answer cannot be
    /// deleted through this path. All deletes run in a single transaction. When the
    /// turn's conversation is still the active one, the in-memory history is resynced
    /// from the authoritative DB rows so the technical rows cannot leak into the next
    /// turn's prompt composition. Returns the number of deleted rows.
    pub async fn delete_turn_artifacts(
        &self,
        conversation_id: &str,
        turn_id: &str,
        extra_row_ids: &[i64],
    ) -> Result<usize> {
        // 会话切换锁：与 load_conversation / delete_last_messages / edit 等历史重写路径互斥，
        // 防止清理期间发生会话切换时把旧会话历史覆盖到新会话的内存上下文
        let _switch_guard = self.conversation_switch_lock.lock().await;
        let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
            "SELECT id, metadata FROM conversation_messages WHERE conversation_id = ? AND metadata IS NOT NULL",
        )
        .bind(conversation_id)
        .fetch_all(&self.db)
        .await?;

        let mut ids_to_delete: HashSet<i64> = rows
            .iter()
            .filter(|(_, metadata)| {
                metadata
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .is_some_and(|meta| {
                        meta.get("turn_id").and_then(|v| v.as_str()) == Some(turn_id)
                            && matches!(
                                meta.get("type").and_then(|t| t.as_str()),
                                Some("assistant_tool_calls") | Some("tool_result")
                            )
                    })
            })
            .map(|(id, _)| *id)
            .collect();

        // Only unfinalized drafts (metadata still NULL) may be removed by id.
        for row_id in extra_row_ids {
            let stored: Option<Option<String>> =
                sqlx::query_scalar("SELECT metadata FROM conversation_messages WHERE id = ?")
                    .bind(row_id)
                    .fetch_optional(&self.db)
                    .await?;
            if matches!(stored, Some(None)) {
                ids_to_delete.insert(*row_id);
            }
        }

        if ids_to_delete.is_empty() {
            return Ok(0);
        }

        let mut tx = self.db.begin().await?;
        for id in &ids_to_delete {
            sqlx::query("DELETE FROM conversation_messages WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;

        // Resync the in-memory history only when the turn's conversation is still the
        // active one (mirrors delete_last_messages_inner). The whole method runs under
        // conversation_switch_lock, so no conversation switch can interleave with this
        // check-then-resync sequence.
        if self.current_conversation_id.lock().await.as_deref() == Some(conversation_id) {
            let remaining_rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
                "SELECT role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
            )
            .bind(conversation_id)
            .fetch_all(&self.db)
            .await?;

            let max_chars = *self.max_message_chars.lock().await;
            let mut history = self.history.lock().await;
            let new_len = sync_history_window(&mut history, remaining_rows, max_chars);
            let mut boundary = self.memory_history_boundary.lock().await;
            *boundary = (*boundary).min(new_len);
        }

        tracing::info!(
            target: "ai",
            "Deleted {} turn artifact row(s) for turn {} in conversation {}",
            ids_to_delete.len(),
            turn_id,
            conversation_id
        );
        Ok(ids_to_delete.len())
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
        let filtered = history
            .iter()
            .filter(|message| is_summary_candidate_message(message))
            .cloned()
            .collect::<Vec<_>>();
        let start = filtered.len().saturating_sub(n);
        filtered.into_iter().skip(start).collect()
    }

    /// Returns the last `n` messages after the current memory boundary.
    pub async fn get_recent_memory_history(&self, n: usize) -> Vec<Message> {
        let history = self.history.lock().await;
        let boundary = (*self.memory_history_boundary.lock().await).min(history.len());
        let filtered = history
            .iter()
            .skip(boundary)
            .filter(|message| is_memory_candidate_message(message))
            .cloned()
            .collect::<Vec<_>>();
        let start = filtered.len().saturating_sub(n);
        filtered.into_iter().skip(start).collect()
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
    pub async fn push_history_message(&self, mut message: Message) {
        let max_chars = *self.max_message_chars.lock().await;
        message.content = truncate_message_content(message.content, max_chars);

        let mut history = self.history.lock().await;
        history.push_back(message);
        let evicted = if history.len() > MAX_IN_MEMORY_HISTORY_MESSAGES {
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

    /// Composes a prompt based on the user query, budgeting tokens for context.
    /// Acquires a temporary [`ChatTurnGuard`] for standalone callers.
    pub async fn compose_prompt(
        &self,
        query: &str,
        allow_image_gen: bool,
        tool_prompt: Option<String>,
        native_tools_enabled: bool,
        character_id: &str,
    ) -> Result<(Vec<Message>, Vec<String>)> {
        let _turn_guard = self
            .enter_chat_turn()
            .map_err(|reason| anyhow::anyhow!("{reason}"))?;
        self.compose_prompt_inner(
            query,
            allow_image_gen,
            tool_prompt,
            native_tools_enabled,
            character_id,
        )
        .await
    }

    /// Composes a prompt within an existing chat turn that already holds a [`ChatTurnGuard`].
    /// Does not re-enter the activation gate, ensuring an active turn can complete without re-entrancy issues.
    pub async fn compose_prompt_with_guard(
        &self,
        query: &str,
        allow_image_gen: bool,
        tool_prompt: Option<String>,
        native_tools_enabled: bool,
        character_id: &str,
        _guard: &ChatTurnGuard,
    ) -> Result<(Vec<Message>, Vec<String>)> {
        self.compose_prompt_inner(
            query,
            allow_image_gen,
            tool_prompt,
            native_tools_enabled,
            character_id,
        )
        .await
    }

    async fn compose_prompt_inner(
        &self,
        query: &str,
        _allow_image_gen: bool,
        tool_prompt: Option<String>,
        native_tools_enabled: bool,
        character_id: &str,
    ) -> Result<(Vec<Message>, Vec<String>)> {
        if let Some(reason) = self.get_runtime_degraded().await {
            anyhow::bail!(
                "Character runtime is degraded ({reason}). Please re-activate or select a character in Character Settings."
            );
        }

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
        let mut warnings: Vec<String> = Vec::new();
        let memories = if self.is_memory_enabled() {
            match self
                .memory_manager
                .search_memories_with_status(query, 5, cid)
                .await
            {
                Ok(result) => {
                    let (snippets, warning) = memory_context_for_prompt(result);
                    if let Some(warning) = warning {
                        warnings.push(warning);
                    }
                    Some(snippets)
                }
                Err(e) => {
                    warnings.push(format!("记忆检索失败（本次对话将不含记忆上下文）：{e}"));
                    None
                }
            }
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
        let vision_context_history_mode = self.vision_context_history_mode.lock().await.clone();
        let history_snapshot: Vec<Message> = self.history.lock().await.iter().cloned().collect();
        let latest_vision_index = latest_vision_context_index(&history_snapshot);
        let recent_history_snapshot: Vec<Message> = history_snapshot
            .iter()
            .enumerate()
            .filter(|(index, msg)| {
                should_include_message_for_llm_history(
                    msg,
                    *index,
                    latest_vision_index,
                    &vision_context_history_mode,
                )
            })
            .map(|(_, msg)| msg)
            .cloned()
            .collect();

        // -- Read response language early so all sections can reference it --
        let resp_lang = self.response_language.lock().await.clone();

        let mut final_messages = Vec::new();

        // ── Stable System Message ────────────────────────────────────────────
        // Keep reusable instructions before per-turn context to improve prefix cache reuse.
        let mut system_parts: Vec<String> = Vec::new();
        let mut dynamic_context_parts: Vec<String> = Vec::new();

        // Section 1: Core persona rules (MUST be first for primacy effect)
        system_parts.push(format!(
            "<rules>\n{}\n</rules>",
            crate::ai::prompts::core_persona_prompt(native_tools_enabled)
        ));

        // Section 2: Character persona (jailbreak + system prompt)
        let jailbreak = self
            .jailbreak_prompt
            .lock()
            .await
            .clone()
            .unwrap_or_default();
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
        system_parts.push(format!("<character>\n{}\n</character>", character_block));

        // Section 3: Long-term memory (higher priority than summaries)
        if let Some(ref mems) = memories {
            if !mems.is_empty() {
                let memory_block = mems
                    .iter()
                    .map(|m| format!("- {}", m.content))
                    .collect::<Vec<_>>()
                    .join("\n");
                dynamic_context_parts.push(format!(
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
                dynamic_context_parts.push(format!(
                    "<conversation_state>\n{}\n</conversation_state>",
                    state_lines.join("\n")
                ));
            }
        }

        // Section 5: Conversation summary (lower priority than long-term memory and recent raw messages)
        if let Some(summary_record) = conversation_summary {
            if !summary_record.summary.trim().is_empty() {
                dynamic_context_parts.push(format!(
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
                    dynamic_context_parts.push(format!(
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

        if let Some(memory_language_rule) = memory_write_language_instruction(&resp_lang) {
            system_parts.push(format!(
                "<memory_write_language>\n{}\n</memory_write_language>",
                memory_language_rule
            ));
        }

        final_messages.push(Message {
            role: "system".to_string(),
            content: system_parts.join("\n\n"),
            metadata: None,
        });

        if !dynamic_context_parts.is_empty() {
            final_messages.push(Message {
                role: "system".to_string(),
                content: dynamic_context_parts.join("\n\n"),
                metadata: Some(serde_json::json!({"type": "dynamic_context"})),
            });
        }

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
            if selected.len() >= MAX_IN_MEMORY_HISTORY_MESSAGES {
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

        Ok((final_messages, warnings))
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

    pub async fn set_vision_context_history_mode(&self, mode: String) {
        *self.vision_context_history_mode.lock().await =
            crate::vision::config::normalize_vision_context_history_mode(&mode);
    }

    pub async fn clear_history(&self) {
        // 会话切换锁：清空历史+重置会话指针必须与删除/加载/编辑等路径互斥，
        // 防止清空期间在途的删除操作把旧历史写回新会话
        let _switch_guard = self.conversation_switch_lock.lock().await;
        let mut history = self.history.lock().await;
        history.clear();
        drop(history);
        *self.memory_history_boundary.lock().await = 0;
        *self.memory_trigger_count.lock().await = 0;
        // 清空当前对话 ID，下次发消息时会创建新对话
        let mut conv_id = self.current_conversation_id.lock().await;
        *conv_id = None;
        if self.persist_conversation_selection {
            Self::persist_conversation_id(None);
        }
    }

    pub async fn reset_history_and_boundary(&self) {
        self.history.lock().await.clear();
        *self.memory_history_boundary.lock().await = 0;
        *self.memory_trigger_count.lock().await = 0;
    }

    /// Resets and populates in-memory history from raw database rows or messages,
    /// enforcing the unified MAX_IN_MEMORY_HISTORY_MESSAGES window and max_message_chars truncation.
    pub async fn sync_history_from_rows<I, T>(&self, items: I) -> usize
    where
        I: IntoIterator<Item = T>,
        T: IntoHistoryMessage,
    {
        let max_chars = *self.max_message_chars.lock().await;
        let mut history = self.history.lock().await;
        let count = sync_history_window(&mut history, items, max_chars);
        *self.memory_history_boundary.lock().await = count;
        count
    }

    pub async fn set_memory_history_boundary(&self, boundary: usize) {
        *self.memory_history_boundary.lock().await = boundary;
    }

    pub async fn memory_history_boundary(&self) -> usize {
        *self.memory_history_boundary.lock().await
    }

    pub async fn should_trigger_memory_event(
        &self,
        cooldown_key: &str,
        cooldown_secs: u64,
    ) -> bool {
        let now = Instant::now();
        let mut cooldowns = self.memory_event_cooldowns.lock().await;

        if let Some(last_triggered_at) = cooldowns.get(cooldown_key) {
            if now.duration_since(*last_triggered_at).as_secs() < cooldown_secs {
                return false;
            }
        }

        cooldowns.insert(cooldown_key.to_string(), now);
        true
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

    async fn insert_conversation_row(
        orchestrator: &AIOrchestrator,
        conversation_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> i64 {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, metadata, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(role)
        .bind(content)
        .bind(metadata)
        .bind(&now)
        .execute(&orchestrator.db)
        .await
        .expect("conversation row insert should succeed")
        .last_insert_rowid()
    }

    async fn insert_test_conversation(orchestrator: &AIOrchestrator, conversation_id: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO conversations (id, character_id, title, created_at, updated_at) VALUES (?, 'test', 'Test', ?, ?)",
        )
        .bind(conversation_id)
        .bind(&now)
        .bind(&now)
        .execute(&orchestrator.db)
        .await
        .expect("conversation insert should succeed");
    }

    async fn fetch_conversation_rows(
        orchestrator: &AIOrchestrator,
        conversation_id: &str,
    ) -> Vec<(i64, String, String, Option<String>)> {
        sqlx::query_as(
            "SELECT id, role, content, metadata FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conversation_id)
        .fetch_all(&orchestrator.db)
        .await
        .expect("conversation rows should load")
    }

    #[tokio::test]
    async fn delete_turn_artifacts_removes_only_turn_technical_rows_and_unfinalized_extras() {
        let orchestrator = setup_test_orchestrator().await;
        insert_test_conversation(&orchestrator, "conv-1").await;
        *orchestrator.current_conversation_id.lock().await = Some("conv-1".to_string());
        *orchestrator.memory_history_boundary.lock().await = 100;

        let user_id = insert_conversation_row(&orchestrator, "conv-1", "user", "hello", None).await;
        let draft_id =
            insert_conversation_row(&orchestrator, "conv-1", "assistant", "partial draft", None)
                .await;
        let turn_a_calls = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "assistant",
            "call weather",
            Some(r#"{"type":"assistant_tool_calls","turn_id":"turn-a"}"#),
        )
        .await;
        let turn_a_tool = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "tool",
            "sunny",
            Some(r#"{"type":"tool_result","turn_id":"turn-a"}"#),
        )
        .await;
        let turn_a_final = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "assistant",
            "final answer",
            Some(r#"{"turn_id":"turn-a"}"#),
        )
        .await;
        let turn_a_vision = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "context",
            "screen observation",
            Some(r#"{"type":"vision_observation","turn_id":"turn-a"}"#),
        )
        .await;
        let turn_b_calls = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "assistant",
            "call email",
            Some(r#"{"type":"assistant_tool_calls","turn_id":"turn-b"}"#),
        )
        .await;
        let turn_b_tool = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "tool",
            "sent",
            Some(r#"{"type":"tool_result","turn_id":"turn-b"}"#),
        )
        .await;
        let failure_id = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "system",
            "stream failed",
            Some(r#"{"type":"failure_event","event":{"turn_id":"turn-a"}}"#),
        )
        .await;

        let deleted = orchestrator
            .delete_turn_artifacts("conv-1", "turn-a", &[draft_id])
            .await
            .expect("cleanup should succeed");
        assert_eq!(deleted, 3);

        let remaining_ids: Vec<i64> = fetch_conversation_rows(&orchestrator, "conv-1")
            .await
            .iter()
            .map(|(id, _, _, _)| *id)
            .collect();
        assert!(!remaining_ids.contains(&draft_id));
        assert!(!remaining_ids.contains(&turn_a_calls));
        assert!(!remaining_ids.contains(&turn_a_tool));
        for kept in [
            user_id,
            turn_a_final,
            turn_a_vision,
            turn_b_calls,
            turn_b_tool,
            failure_id,
        ] {
            assert!(remaining_ids.contains(&kept), "row {kept} must survive");
        }

        // In-memory history is resynced from DB and the boundary clamps down to it.
        let history = orchestrator.history.lock().await;
        let roles: Vec<&str> = history.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(
            roles,
            vec![
                "user",
                "assistant",
                "context",
                "assistant",
                "tool",
                "system"
            ]
        );
        assert_eq!(
            *orchestrator.memory_history_boundary.lock().await,
            history.len()
        );
    }

    #[tokio::test]
    async fn delete_turn_artifacts_skips_finalized_extras_and_noop_cleanup() {
        let orchestrator = setup_test_orchestrator().await;
        insert_test_conversation(&orchestrator, "conv-1").await;
        *orchestrator.current_conversation_id.lock().await = Some("conv-1".to_string());
        let finalized_id = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "assistant",
            "final answer",
            Some(r#"{"turn_id":"turn-a"}"#),
        )
        .await;
        let tool_id = insert_conversation_row(
            &orchestrator,
            "conv-1",
            "tool",
            "sunny",
            Some(r#"{"type":"tool_result","turn_id":"turn-a"}"#),
        )
        .await;

        // A finalized row passed as an extra must never be deleted.
        let deleted = orchestrator
            .delete_turn_artifacts("conv-1", "turn-a", &[finalized_id])
            .await
            .expect("cleanup should succeed");
        assert_eq!(deleted, 1);
        let remaining_ids: Vec<i64> = fetch_conversation_rows(&orchestrator, "conv-1")
            .await
            .iter()
            .map(|(id, _, _, _)| *id)
            .collect();
        assert!(remaining_ids.contains(&finalized_id));
        assert!(!remaining_ids.contains(&tool_id));

        // Unknown turn id deletes nothing and leaves history untouched.
        orchestrator
            .push_history_message(Message {
                role: "user".to_string(),
                content: "in-memory only".to_string(),
                metadata: None,
            })
            .await;
        let history_len_before = orchestrator.history.lock().await.len();
        let deleted = orchestrator
            .delete_turn_artifacts("conv-1", "turn-unknown", &[])
            .await
            .expect("cleanup should succeed");
        assert_eq!(deleted, 0);
        assert_eq!(orchestrator.history.lock().await.len(), history_len_before);
    }

    #[tokio::test]
    async fn delete_turn_artifacts_resyncs_only_active_conversation() {
        let orchestrator = setup_test_orchestrator().await;
        insert_test_conversation(&orchestrator, "conv-1").await;
        // The turn's conversation is no longer the active one (user switched away).
        *orchestrator.current_conversation_id.lock().await = Some("other-conv".to_string());
        insert_conversation_row(
            &orchestrator,
            "conv-1",
            "assistant",
            "call weather",
            Some(r#"{"type":"assistant_tool_calls","turn_id":"turn-a"}"#),
        )
        .await;
        insert_conversation_row(
            &orchestrator,
            "conv-1",
            "tool",
            "sunny",
            Some(r#"{"type":"tool_result","turn_id":"turn-a"}"#),
        )
        .await;
        orchestrator
            .push_history_message(Message {
                role: "user".to_string(),
                content: "active conversation message".to_string(),
                metadata: None,
            })
            .await;

        let deleted = orchestrator
            .delete_turn_artifacts("conv-1", "turn-a", &[])
            .await
            .expect("cleanup should succeed");
        assert_eq!(deleted, 2);
        assert!(fetch_conversation_rows(&orchestrator, "conv-1")
            .await
            .is_empty());
        // History must not be resynced for a conversation that is no longer active.
        let history = orchestrator.history.lock().await;
        assert_eq!(history.len(), 1);
        assert_eq!(
            history.front().unwrap().content,
            "active conversation message"
        );
    }

    #[tokio::test]
    async fn fork_keeps_history_and_conversation_state_isolated() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator
            .push_history_message(Message {
                role: "user".to_string(),
                content: "desktop history".to_string(),
                metadata: None,
            })
            .await;
        *orchestrator.current_conversation_id.lock().await = Some("desktop".to_string());

        let fork = orchestrator.fork_with_isolated_history().await;
        fork.push_history_message(Message {
            role: "user".to_string(),
            content: "qq history".to_string(),
            metadata: None,
        })
        .await;

        assert_eq!(orchestrator.history.lock().await.len(), 1);
        assert_eq!(fork.history.lock().await.len(), 1);
        assert_eq!(
            orchestrator.history.lock().await.front().unwrap().content,
            "desktop history"
        );
        assert_eq!(
            fork.history.lock().await.front().unwrap().content,
            "qq history"
        );
        assert_eq!(
            orchestrator.current_conversation_id.lock().await.as_deref(),
            Some("desktop")
        );
        assert_eq!(*fork.current_conversation_id.lock().await, None);
        assert!(!fork.persist_conversation_selection);
    }

    #[tokio::test]
    async fn character_profile_updates_isolated_persona() {
        let orchestrator = setup_test_orchestrator().await;
        sqlx::query(
            "INSERT INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("qq-character")
        .bind("QQ Character")
        .bind("QQ Persona")
        .bind("QQ User")
        .bind("test")
        .bind(1_i64)
        .bind(1_i64)
        .execute(&orchestrator.db)
        .await
        .expect("character insert should succeed");

        let fork = orchestrator.fork_with_isolated_history().await;
        assert!(fork
            .apply_character_profile("qq-character")
            .await
            .expect("profile should load"));

        assert_eq!(*fork.system_prompt.lock().await, "QQ Persona");
        assert_eq!(*fork.character_name.lock().await, "QQ Character");
        assert_eq!(*fork.user_name.lock().await, "QQ User");
        assert_ne!(*orchestrator.system_prompt.lock().await, "QQ Persona");
    }

    #[test]
    fn memory_write_language_instruction_uses_response_language() {
        let instruction = memory_write_language_instruction("日本語").expect("instruction");

        assert!(instruction.contains("stored memory text in 日本語"));
        assert!(instruction.contains("fact argument for store_memory"));
    }

    #[test]
    fn conversation_summary_prompt_uses_response_language() {
        let prompt = build_conversation_summary_prompt("user: hello", "中文");

        assert!(prompt.contains("Write the summary in 中文"));
        assert!(prompt.contains("translate or summarize it into 中文"));
    }

    #[tokio::test]
    async fn compose_prompt_places_dynamic_context_after_stable_system() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator.set_memory_enabled(false).await;
        orchestrator
            .set_system_prompt("Character persona".to_string())
            .await;
        orchestrator.set_response_language("中文".to_string()).await;
        orchestrator
            .push_history_message(Message {
                role: "user".to_string(),
                content: "Earlier history".to_string(),
                metadata: None,
            })
            .await;

        let conversation_id = "conv-cache-order";
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO conversations \
             (id, character_id, title, topic, pinned_state, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(conversation_id)
        .bind("char-cache")
        .bind("Cache order")
        .bind("KV cache tuning")
        .bind("{}")
        .bind(&now)
        .bind(&now)
        .execute(&orchestrator.db)
        .await
        .expect("conversation insert should succeed");
        *orchestrator.current_conversation_id.lock().await = Some(conversation_id.to_string());

        let (messages, warnings) = orchestrator
            .compose_prompt(
                "hello",
                false,
                Some("Tool prompt".to_string()),
                false,
                "char-cache",
            )
            .await
            .expect("compose_prompt should succeed");

        assert!(warnings.is_empty());
        let stable = &messages[0];
        assert_eq!(stable.role, "system");
        assert!(stable.content.contains("<rules>"));
        assert!(stable.content.contains("<character>"));
        assert!(stable.content.contains("<tools>"));
        assert!(stable.content.contains("<language>"));
        assert!(!stable.content.contains("<conversation_state>"));
        assert!(!stable.content.contains("<long_term_memory>"));
        assert!(!stable.content.contains("<conversation_summary>"));

        let dynamic = &messages[1];
        assert_eq!(dynamic.role, "system");
        assert_eq!(
            dynamic
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("type"))
                .and_then(|value| value.as_str()),
            Some("dynamic_context")
        );
        assert!(dynamic.content.contains("<conversation_state>"));
        assert!(dynamic.content.contains("KV cache tuning"));

        let history_index = messages
            .iter()
            .position(|message| message.content == "Earlier history")
            .expect("history message should be included");
        assert!(
            history_index > 1,
            "dynamic context should appear before recent history"
        );
    }

    #[tokio::test]
    async fn compose_prompt_skips_dynamic_context_message_when_empty() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator.set_memory_enabled(false).await;

        let (messages, warnings) = orchestrator
            .compose_prompt("hello", false, None, true, "char-cache")
            .await
            .expect("compose_prompt should succeed");

        assert!(warnings.is_empty());
        assert!(messages.iter().all(|message| {
            message
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("type"))
                .and_then(|value| value.as_str())
                != Some("dynamic_context")
        }));
        assert!(messages
            .iter()
            .all(|message| !message.content.contains("<long_term_memory>")
                && !message.content.contains("<conversation_state>")
                && !message.content.contains("<conversation_summary>")));
    }

    #[test]
    fn unavailable_memory_search_keeps_keyword_snippets_for_chat() {
        let result = MemorySearchResult {
            snippets: vec![MemorySnippet {
                id: 7,
                content: "keyword match".to_string(),
                embedding: Vec::new(),
                created_at: 1,
                importance: 0.5,
                tier: "ephemeral".to_string(),
            }],
            mode: MemoryRetrievalMode::Unavailable,
        };

        let (snippets, warning) = memory_context_for_prompt(result);

        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].content, "keyword match");
        assert_eq!(
            warning.as_deref(),
            Some("memory semantic retrieval unavailable; keyword results still included")
        );
    }

    #[tokio::test]
    async fn recent_history_helpers_skip_vision_context_rows() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator
            .push_history_message(Message {
                role: "context".to_string(),
                content: "Raw screen summary".to_string(),
                metadata: Some(serde_json::json!({"type": "vision_observation"})),
            })
            .await;
        orchestrator
            .push_history_message(Message {
                role: "user".to_string(),
                content: "Please explain this code".to_string(),
                metadata: None,
            })
            .await;

        let summary_history = orchestrator.get_recent_history(10).await;
        let memory_history = orchestrator.get_recent_memory_history(10).await;

        assert_eq!(summary_history.len(), 1);
        assert_eq!(summary_history[0].role, "user");
        assert_eq!(memory_history.len(), 1);
        assert_eq!(memory_history[0].role, "user");
    }

    #[tokio::test]
    async fn compose_prompt_defaults_to_latest_vision_context_history() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator.set_memory_enabled(false).await;
        orchestrator
            .push_history_message(Message {
                role: "context".to_string(),
                content: "Old screen summary".to_string(),
                metadata: Some(serde_json::json!({"type": "vision_observation"})),
            })
            .await;
        orchestrator
            .push_history_message(Message {
                role: "assistant".to_string(),
                content: "Old visual reply".to_string(),
                metadata: None,
            })
            .await;
        orchestrator
            .push_history_message(Message {
                role: "context".to_string(),
                content: "Latest screen summary".to_string(),
                metadata: Some(serde_json::json!({"type": "vision_observation"})),
            })
            .await;

        let (messages, warnings) = orchestrator
            .compose_prompt("hello", false, None, true, "char-cache")
            .await
            .expect("compose_prompt should succeed");

        assert!(warnings.is_empty());
        assert!(messages
            .iter()
            .any(|message| message.content == "Latest screen summary"));
        assert!(messages
            .iter()
            .all(|message| message.content != "Old screen summary"));
        assert!(messages
            .iter()
            .any(|message| message.content == "Old visual reply"));
    }

    #[tokio::test]
    async fn compose_prompt_can_include_full_vision_context_history() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator.set_memory_enabled(false).await;
        orchestrator
            .set_vision_context_history_mode("full".to_string())
            .await;
        orchestrator
            .push_history_message(Message {
                role: "context".to_string(),
                content: "Old screen summary".to_string(),
                metadata: Some(serde_json::json!({"type": "vision_observation"})),
            })
            .await;
        orchestrator
            .push_history_message(Message {
                role: "context".to_string(),
                content: "Latest screen summary".to_string(),
                metadata: Some(serde_json::json!({"type": "vision_observation"})),
            })
            .await;

        let (messages, warnings) = orchestrator
            .compose_prompt("hello", false, None, true, "char-cache")
            .await
            .expect("compose_prompt should succeed");

        assert!(warnings.is_empty());
        assert!(messages
            .iter()
            .any(|message| message.content == "Old screen summary"));
        assert!(messages
            .iter()
            .any(|message| message.content == "Latest screen summary"));
    }

    #[tokio::test]
    async fn memory_event_cooldown_blocks_same_key_within_window() {
        let orchestrator = setup_test_orchestrator().await;

        assert!(
            orchestrator
                .should_trigger_memory_event("char-1:conv-1:preference", 60)
                .await
        );
        assert!(
            !orchestrator
                .should_trigger_memory_event("char-1:conv-1:preference", 60)
                .await
        );
    }

    #[tokio::test]
    async fn memory_event_cooldown_allows_different_keys() {
        let orchestrator = setup_test_orchestrator().await;

        assert!(
            orchestrator
                .should_trigger_memory_event("char-1:conv-1:preference", 60)
                .await
        );
        assert!(
            orchestrator
                .should_trigger_memory_event("char-1:conv-1:plan", 60)
                .await
        );
    }

    #[tokio::test]
    async fn memory_event_cooldown_allows_zero_window() {
        let orchestrator = setup_test_orchestrator().await;

        assert!(
            orchestrator
                .should_trigger_memory_event("char-1:conv-1:preference", 0)
                .await
        );
        assert!(
            orchestrator
                .should_trigger_memory_event("char-1:conv-1:preference", 0)
                .await
        );
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
    async fn test_push_history_message_truncation() {
        let orchestrator = setup_test_orchestrator().await;
        *orchestrator.max_message_chars.lock().await = 50;

        let long_message =
            "This assistant response is long enough to exceed the configured context limit"
                .to_string();
        orchestrator
            .push_history_message(Message {
                role: "assistant".to_string(),
                content: long_message,
                metadata: None,
            })
            .await;

        let history = orchestrator.history.lock().await;
        assert_eq!(history.len(), 1, "History should contain one message");

        let msg = &history[0];
        assert!(
            msg.content.ends_with(TRUNCATION_MARKER),
            "Message should end with truncation marker"
        );
        assert!(
            msg.content.chars().count() <= 50 + TRUNCATION_MARKER.chars().count(),
            "Truncated message should not exceed max + marker length"
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

    #[tokio::test]
    async fn test_degraded_runtime_blocks_compose_prompt_and_clearing_allows_it() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator
            .set_runtime_degraded(Some("Startup history sync failed".to_string()))
            .await;

        let err = orchestrator
            .compose_prompt("Hello", false, None, false, "default")
            .await
            .expect_err("compose_prompt must fail when runtime is degraded");
        assert!(err.to_string().contains("Character runtime is degraded"));
        assert!(err.to_string().contains("Startup history sync failed"));

        orchestrator.clear_runtime_degraded().await;
        let (messages, _) = orchestrator
            .compose_prompt("Hello", false, None, false, "default")
            .await
            .expect("compose_prompt must succeed when degraded state is cleared");
        assert!(!messages.is_empty());
    }

    #[tokio::test]
    async fn test_add_message_with_metadata_returns_conversation_and_message_id() {
        let orchestrator = setup_test_orchestrator().await;
        orchestrator.clear_history().await;

        let (conv_id, msg_id_1) = orchestrator
            .add_message_with_metadata(
                "user".to_string(),
                "Hello, first message!".to_string(),
                None,
                "test_char",
                None,
            )
            .await
            .expect("First message persistence should succeed");

        assert!(
            !conv_id.is_empty(),
            "Auto-created conversation ID must not be empty"
        );
        assert!(msg_id_1 > 0, "First message ID must be positive integer");

        let (conv_id_2, msg_id_2) = orchestrator
            .add_message_with_metadata(
                "assistant".to_string(),
                "Hello there!".to_string(),
                None,
                "test_char",
                None,
            )
            .await
            .expect("Second message persistence should succeed");

        assert_eq!(
            conv_id_2, conv_id,
            "Subsequent message must share the same active conversation ID"
        );
        assert!(
            msg_id_2 > msg_id_1,
            "Message ID must be auto-incremented in SQLite"
        );
    }

    #[tokio::test]
    async fn persist_message_propagates_conversation_metadata_update_failure() {
        // FK enforcement is disabled so the message INSERT succeeds even
        // though the conversations table is gone; only the metadata UPDATE
        // fails, which is exactly the error path this test targets.
        let orchestrator =
            AIOrchestrator::new_for_tests_with_foreign_keys("sqlite::memory:", false)
                .await
                .expect("FK-off test orchestrator should build");
        insert_test_conversation(&orchestrator, "conv-err").await;
        sqlx::query("DROP TABLE conversations")
            .execute(&orchestrator.db)
            .await
            .unwrap();

        let err = orchestrator
            .add_message_with_metadata_for_conversation(
                "user".to_string(),
                "hello".to_string(),
                None,
                "test_char",
                Some("conv-err"),
                None,
            )
            .await
            .expect_err("metadata update failure must surface as an error");

        assert!(
            err.to_string()
                .contains("failed to update title/updated_at"),
            "error should carry conversation metadata context, got: {}",
            err
        );

        // The failed transaction must roll the message row back so callers
        // never observe a half-saved turn.
        let orphan_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
            .fetch_one(&orchestrator.db)
            .await
            .unwrap();
        assert_eq!(
            orphan_count, 0,
            "failed transaction must roll back the message row"
        );
    }

    #[tokio::test]
    async fn persist_message_tolerates_conversation_row_vanishing_mid_persist() {
        // With FK enforcement off, a concurrently deleted conversation lets
        // the message INSERT succeed while the metadata UPDATE touches zero
        // rows. That is a benign race (in production FK cascade removes the
        // message), not a hard failure.
        let orchestrator =
            AIOrchestrator::new_for_tests_with_foreign_keys("sqlite::memory:", false)
                .await
                .expect("FK-off test orchestrator should build");
        insert_test_conversation(&orchestrator, "conv-gone").await;
        sqlx::query("DELETE FROM conversations WHERE id = 'conv-gone'")
            .execute(&orchestrator.db)
            .await
            .unwrap();

        let (conv_id, msg_id) = orchestrator
            .add_message_with_metadata_for_conversation(
                "user".to_string(),
                "hello".to_string(),
                None,
                "test_char",
                Some("conv-gone"),
                None,
            )
            .await
            .expect("vanished conversation must not fail message persistence");

        assert_eq!(conv_id, "conv-gone");
        assert!(msg_id > 0);
    }

    #[tokio::test]
    async fn persist_message_updates_conversation_title_and_updated_at() {
        let orchestrator = setup_test_orchestrator().await;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at) VALUES ('conv-title', 'test_char', '新对话', '', '{}', ?, ?)",
        )
        .bind(&now)
        .bind(&now)
        .execute(&orchestrator.db)
        .await
        .unwrap();

        orchestrator
            .add_message_with_metadata_for_conversation(
                "user".to_string(),
                "This is a long enough message to be truncated".to_string(),
                None,
                "test_char",
                Some("conv-title"),
                None,
            )
            .await
            .expect("persist should succeed");

        let (title, updated_at): (String, String) =
            sqlx::query_as("SELECT title, updated_at FROM conversations WHERE id = 'conv-title'")
                .fetch_one(&orchestrator.db)
                .await
                .unwrap();
        assert_eq!(title, "This is a long enoug...");
        assert!(
            updated_at >= now,
            "updated_at should be bumped, was {} (before {})",
            updated_at,
            now
        );
    }

    #[tokio::test]
    async fn persist_message_preserves_non_default_conversation_title() {
        let orchestrator = setup_test_orchestrator().await;
        // insert_test_conversation uses a non-default title ('Test') that the
        // user-message CASE WHEN must never overwrite.
        insert_test_conversation(&orchestrator, "conv-keep").await;

        orchestrator
            .add_message_with_metadata_for_conversation(
                "user".to_string(),
                "Hello!".to_string(),
                None,
                "test_char",
                Some("conv-keep"),
                None,
            )
            .await
            .expect("persist should succeed");

        let title: String =
            sqlx::query_scalar("SELECT title FROM conversations WHERE id = 'conv-keep'")
                .fetch_one(&orchestrator.db)
                .await
                .unwrap();
        assert_eq!(title, "Test");
    }

    #[tokio::test]
    async fn persist_message_bumps_updated_at_for_assistant_messages_without_touching_title() {
        let orchestrator = setup_test_orchestrator().await;
        insert_test_conversation(&orchestrator, "conv-bump").await;
        let before: String =
            sqlx::query_scalar("SELECT updated_at FROM conversations WHERE id = 'conv-bump'")
                .fetch_one(&orchestrator.db)
                .await
                .unwrap();

        orchestrator
            .add_message_with_metadata_for_conversation(
                "assistant".to_string(),
                "Reply".to_string(),
                None,
                "test_char",
                Some("conv-bump"),
                None,
            )
            .await
            .expect("persist should succeed");

        let (title, updated_at): (String, String) =
            sqlx::query_as("SELECT title, updated_at FROM conversations WHERE id = 'conv-bump'")
                .fetch_one(&orchestrator.db)
                .await
                .unwrap();
        assert_eq!(title, "Test");
        assert!(updated_at >= before, "updated_at should be bumped");
    }

    #[tokio::test]
    async fn persist_streaming_draft_isolates_to_given_conversation_and_does_not_alter_current_conversation_id(
    ) {
        let orchestrator = setup_test_orchestrator().await;

        // Ensure current_conversation_id is None
        *orchestrator.current_conversation_id.lock().await = None;

        // Insert conversation A
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at) VALUES ('conv-A', 'test_char', 'Title', '', '{}', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&orchestrator.db)
            .await
            .unwrap();

        let row_id = orchestrator
            .persist_streaming_draft("conv-A", "draft chunk 1")
            .await
            .expect("persist_streaming_draft should succeed");
        assert!(row_id > 0);

        // Global current_conversation_id must remain None!
        assert_eq!(*orchestrator.current_conversation_id.lock().await, None);

        // Verify message was inserted into conv-A
        let (role, content): (String, String) =
            sqlx::query_as("SELECT role, content FROM conversation_messages WHERE id = ?")
                .bind(row_id)
                .fetch_one(&orchestrator.db)
                .await
                .unwrap();
        assert_eq!(role, "assistant");
        assert_eq!(content, "draft chunk 1");
    }

    #[tokio::test]
    async fn add_message_with_metadata_for_conversation_skips_history_push_when_conversation_mismatches(
    ) {
        let orchestrator = setup_test_orchestrator().await;

        // Active conversation is conv-B
        *orchestrator.current_conversation_id.lock().await = Some("conv-B".to_string());

        // Insert conversation A
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at) VALUES ('conv-A', 'test_char', 'Title A', '', '{}', ?, ?)")
            .bind(&now)
            .bind(&now)
            .execute(&orchestrator.db)
            .await
            .unwrap();

        // Add message targeting conv-A while active is conv-B
        let (cid, mid) = orchestrator
            .add_message_with_metadata_for_conversation(
                "assistant".to_string(),
                "stale message".to_string(),
                None,
                "test_char",
                Some("conv-A"),
                None,
            )
            .await
            .unwrap();

        assert_eq!(cid, "conv-A");
        assert!(mid > 0);

        // In-memory history for active conversation (conv-B) must NOT be contaminated!
        assert_eq!(orchestrator.history.lock().await.len(), 0);
    }

    #[test]
    fn sync_history_window_enforces_20_limit_and_takes_most_recent() {
        let mut history = VecDeque::new();
        let rows: Vec<(String, String, Option<String>)> = (0..35)
            .map(|i| ("user".to_string(), format!("Message {i}"), None))
            .collect();

        let count = sync_history_window(&mut history, rows, 2000);
        assert_eq!(count, 20);
        assert_eq!(history.len(), 20);
        // Should contain the last 20 messages (indices 15..35)
        assert_eq!(history[0].content, "Message 15");
        assert_eq!(history[19].content, "Message 34");
    }

    #[test]
    fn sync_history_window_keeps_all_when_fewer_than_20() {
        let mut history = VecDeque::new();
        let rows: Vec<(String, String, Option<String>)> = (0..7)
            .map(|i| ("user".to_string(), format!("Message {i}"), None))
            .collect();

        let count = sync_history_window(&mut history, rows, 2000);
        assert_eq!(count, 7);
        assert_eq!(history.len(), 7);
        assert_eq!(history[0].content, "Message 0");
        assert_eq!(history[6].content, "Message 6");
    }

    #[test]
    fn sync_history_window_applies_max_chars_truncation() {
        let mut history = VecDeque::new();
        let long_text = "A".repeat(100);
        let rows = vec![("user".to_string(), long_text, None)];

        let count = sync_history_window(&mut history, rows, 30);
        assert_eq!(count, 1);
        assert_eq!(
            history[0].content,
            format!("{}…[truncated]", "A".repeat(30))
        );
    }
}
