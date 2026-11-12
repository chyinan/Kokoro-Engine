//! Application-owned chat journal. A WebView is a subscriber, never the owner.
use crate::commands::chat::{ChatRequest, StreamChatResponse};
use crate::mods::manager::ModChatEvents;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tokio::sync::watch;

#[derive(Clone, Debug, Serialize)]
pub struct ChatTurnSnapshot {
    pub revision: u64,
    pub client_request_id: String,
    pub turn_id: Option<String>,
    pub character_id: Option<String>,
    pub conversation_id: Option<String>,
    pub user_message_id: Option<i64>,
    pub assistant_message_id: Option<i64>,
    pub message: String,
    pub images: Vec<String>,
    pub hidden: bool,
    pub status: String,
    pub text: String,
    pub translation: Option<String>,
    pub tools: Vec<Value>,
    pub error: Option<String>,
}

type Outcome = Result<StreamChatResponse, String>;
struct Entry {
    fingerprint: String,
    snapshot: ChatTurnSnapshot,
    outcome: watch::Sender<Option<Outcome>>,
}
#[derive(Default)]
struct Journal {
    revision: u64,
    entries: HashMap<String, Entry>,
    retired: HashMap<String, (String, Outcome)>,
    abandoned_drafts: HashMap<String, HashSet<String>>,
}
#[derive(Default)]
pub struct ChatRuntimeState(
    Mutex<Journal>,
    Mutex<Option<(String, String, std::time::Instant)>>,
);

impl ChatRuntimeState {
    pub fn begin_playback(&self, request: &str, window: &str) {
        *self.1.lock().unwrap() = Some((request.into(), window.into(), std::time::Instant::now()));
    }
    pub fn finish_playback(&self, request: &str, window: &str) {
        let mut playback = self.1.lock().unwrap();
        if playback
            .as_ref()
            .is_some_and(|(id, owner, _)| id == request && owner == window)
        {
            *playback = None;
        }
    }
    pub fn is_playing(&self, app: &tauri::AppHandle) -> bool {
        self.1
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(_, window, started)| {
                started.elapsed() < std::time::Duration::from_secs(600)
                    && app.get_webview_window(window).is_some()
            })
    }
    /// Duplicate delivery shares the original outcome, including rejection. No queue.
    pub fn begin(
        &self,
        request: &ChatRequest,
    ) -> Result<(bool, watch::Receiver<Option<Outcome>>), String> {
        let id = request
            .client_request_id
            .as_deref()
            .ok_or("missing client_request_id")?;
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(request).map_err(|e| e.to_string())?)
        );
        let mut journal = self.0.lock().unwrap();
        if request.character_id.as_ref().is_some_and(|character| {
            journal
                .abandoned_drafts
                .get(character)
                .is_some_and(|ids| ids.contains(id))
        }) {
            return Err("chat_draft_abandoned: the draft was restored before admission".into());
        }
        if let Some((original, result)) = journal.retired.get(id) {
            if original != &fingerprint {
                return Err(
                    "chat_request_conflict: client_request_id was reused with different input"
                        .into(),
                );
            }
            let (_, receiver) = watch::channel(Some(result.clone()));
            return Ok((false, receiver));
        }
        if let Some(entry) = journal.entries.get(id) {
            if entry.fingerprint != fingerprint {
                return Err(
                    "chat_request_conflict: client_request_id was reused with different input"
                        .into(),
                );
            }
            return Ok((false, entry.outcome.subscribe()));
        }
        // Retain recent terminal requests for reconnect and idempotent retries. Active
        // entries are never evicted. Older history remains in SQLite.
        if journal.entries.len() >= 256 {
            let oldest = journal
                .entries
                .iter()
                .filter(|(_, e)| e.outcome.borrow().is_some())
                .min_by_key(|(_, e)| e.snapshot.revision)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                if let Some(entry) = journal.entries.remove(&id) {
                    let outcome = entry.outcome.borrow().clone().expect("terminal entry");
                    journal.retired.insert(id, (entry.fingerprint, outcome));
                }
            }
        }
        journal.revision += 1;
        let revision = journal.revision;
        let (outcome, rx) = watch::channel(None);
        journal.entries.insert(
            id.into(),
            Entry {
                fingerprint,
                snapshot: ChatTurnSnapshot {
                    revision,
                    client_request_id: id.into(),
                    turn_id: None,
                    character_id: request.character_id.clone(),
                    conversation_id: request.conversation_id.clone(),
                    user_message_id: None,
                    assistant_message_id: None,
                    message: request.message.clone(),
                    images: request.images.clone().unwrap_or_default(),
                    hidden: request.hidden,
                    status: "pending".into(),
                    text: String::new(),
                    translation: None,
                    tools: vec![],
                    error: None,
                },
                outcome,
            },
        );
        Ok((true, rx))
    }

    pub fn snapshots(&self) -> Vec<ChatTurnSnapshot> {
        self.journal_snapshot().1
    }

    /// Recovering a draft may race its IPC delivery. If recovery reaches the journal
    /// first, close admission for that ID before the composer can send a new request.
    pub fn recover_draft_admission(&self, character_id: &str, request_id: &str) -> bool {
        let mut journal = self.0.lock().unwrap();
        if let Some(entry) = journal.entries.get(request_id) {
            return entry.snapshot.character_id.as_deref() == Some(character_id)
                && entry.outcome.borrow().is_none();
        }
        if !journal.retired.contains_key(request_id) {
            journal
                .abandoned_drafts
                .entry(character_id.into())
                .or_default()
                .insert(request_id.into());
        }
        false
    }

    fn revision(&self) -> u64 {
        self.0.lock().unwrap().revision
    }

    pub fn turn_text(&self, id: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .entries
            .get(id)
            .map(|entry| entry.snapshot.text.clone())
    }

    pub fn journal_snapshot(&self) -> (u64, Vec<ChatTurnSnapshot>) {
        let journal = self.0.lock().unwrap();
        let mut snapshots: Vec<_> = journal
            .entries
            .values()
            .map(|e| e.snapshot.clone())
            .collect();
        snapshots.sort_by_key(|s| s.revision);
        (journal.revision, snapshots)
    }

    pub fn outcome(&self, target: &str) -> Option<watch::Receiver<Option<Outcome>>> {
        let journal = self.0.lock().unwrap();
        journal
            .entries
            .values()
            .find(|entry| {
                entry.snapshot.client_request_id == target
                    || entry.snapshot.turn_id.as_deref() == Some(target)
            })
            .map(|entry| entry.outcome.subscribe())
    }

    pub fn event(&self, name: &str, payload: &Value) -> Option<ChatTurnSnapshot> {
        let mut journal = self.0.lock().unwrap();
        let request_id = payload
            .get("client_request_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                let turn = payload.get("turn_id").and_then(Value::as_str)?;
                journal
                    .entries
                    .iter()
                    .find(|(_, e)| e.snapshot.turn_id.as_deref() == Some(turn))
                    .map(|(id, _)| id.clone())
            })?;
        if journal.entries.get(&request_id)?.outcome.borrow().is_some() {
            return None;
        }
        journal.revision += 1;
        let revision = journal.revision;
        let snapshot = &mut journal.entries.get_mut(&request_id)?.snapshot;
        snapshot.revision = revision;
        if let Some(id) = payload.get("turn_id").and_then(Value::as_str) {
            snapshot.turn_id = Some(id.into());
        }
        if let Some(id) = payload.get("conversation_id").and_then(Value::as_str) {
            snapshot.conversation_id = Some(id.into());
        }
        if let Some(id) = payload.get("user_message_id").and_then(Value::as_i64) {
            snapshot.user_message_id = Some(id);
        }
        if let Some(id) = payload.get("assistant_message_id").and_then(Value::as_i64) {
            snapshot.assistant_message_id = Some(id);
        }
        if let Some(id) = payload.get("character_id").and_then(Value::as_str) {
            snapshot.character_id = Some(id.into());
        }
        match name {
            "chat-turn-acknowledged" => snapshot.status = "accepted".into(),
            "chat-turn-start" => snapshot.status = "streaming".into(),
            "chat-turn-delta" => snapshot.text.push_str(
                payload
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            "chat-turn-text-complete" => {
                snapshot.text = payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into();
                snapshot.translation = payload
                    .get("translation")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "chat-turn-translation" => {
                snapshot.translation = payload
                    .get("translation")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            "chat-turn-tool" => snapshot.tools.push(payload.clone()),
            // Finish is only a notification: the executing task commits its terminal
            // outcome after persistence and cleanup, even if delivery fails.
            _ => {}
        }
        Some(snapshot.clone())
    }

    pub fn complete(&self, id: &str, result: Outcome) -> Option<ChatTurnSnapshot> {
        let mut journal = self.0.lock().unwrap();
        if journal.entries.get(id)?.outcome.borrow().is_some() {
            return None;
        }
        journal.revision += 1;
        let revision = journal.revision;
        let entry = journal.entries.get_mut(id)?;
        entry.snapshot.revision = revision;
        match &result {
            Ok(response) => {
                entry.snapshot.status = response
                    .status
                    .clone()
                    .unwrap_or_else(|| "completed".into());
                entry.snapshot.conversation_id = (!response.conversation_id.is_empty())
                    .then(|| response.conversation_id.clone());
                entry.snapshot.user_message_id = response.user_message_id;
                entry.snapshot.assistant_message_id = response.assistant_message_id;
            }
            Err(error) => {
                entry.snapshot.status = if error.contains("turn cancelled by user") {
                    "cancelled"
                } else if error.contains("chat_turn_busy") {
                    "rejected"
                } else {
                    "error"
                }
                .into();
                entry.snapshot.error = Some(error.clone());
            }
        }
        entry.outcome.send_replace(Some(result));
        Some(entry.snapshot.clone())
    }
}

/// Journal first, notify second. No subscriber or closed window can fail a turn.
pub async fn emit_chat_event<S: Serialize + Clone>(
    app: &tauri::AppHandle,
    name: &str,
    payload: S,
) -> Result<(), tauri::Error> {
    if let Ok(value) = serde_json::to_value(&payload) {
        if let Some(runtime) = app.try_state::<Arc<ChatRuntimeState>>() {
            if let Some(snapshot) = runtime.event(name, &value) {
                let _ = app.emit("chat-runtime-changed", snapshot);
            }
        }
        if let Some(relay) = app.try_state::<ModChatEvents>() {
            relay.enqueue_chat_event(name, value);
        }
    }
    if let Err(error) = app.emit(name, payload) {
        tracing::warn!(target: "chat", "event {name} delivery failed: {error}");
    }
    Ok(())
}

pub fn get_chat_runtime_snapshot(
    runtime: tauri::State<'_, Arc<ChatRuntimeState>>,
) -> Vec<ChatTurnSnapshot> {
    runtime.snapshots()
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ChatViewVersion {
    pub revision: u64,
    pub history_revision: i64,
    pub generation: u64,
    pub busy: bool,
}

#[derive(Serialize)]
pub struct ChatViewSnapshot {
    #[serde(flatten)]
    pub version: ChatViewVersion,
    pub character_id: String,
    pub conversation_id: Option<String>,
    pub messages: Vec<crate::commands::conversation::ConversationMessage>,
    pub turns: Vec<ChatTurnSnapshot>,
}

/// Read-only restoration. Unlike load_conversation this never switches selection,
/// increments its generation, cancels work, or writes anything.
pub async fn get_chat_view(
    state: &crate::ai::context::AIOrchestrator,
    runtime: &ChatRuntimeState,
    known_version: Option<ChatViewVersion>,
) -> Result<Option<ChatViewSnapshot>, crate::error::KokoroError> {
    const HISTORY_REVISION: &str = "SELECT revision FROM chat_history_revision WHERE id = 1";
    if let Some(known) = known_version {
        let history_revision = sqlx::query_scalar(HISTORY_REVISION).fetch_one(&state.db).await?;
        let current = ChatViewVersion {
            history_revision,
            revision: runtime.revision(),
            generation: state.current_conversation_generation(),
            busy: state.is_chat_busy(),
        };
        if known == current {
            return Ok(None);
        }
    }
    let _selection = state.conversation_switch_lock.lock().await;
    let character_id = state.character_id.lock().await.clone();
    let conversation_id = state.current_conversation_id.lock().await.clone();
    // Capture the journal before SQLite so any completed turn included in this
    // revision has already persisted its history before the read starts.
    let (revision, turns) = runtime.journal_snapshot();
    // Bind the history stamp to the same SQLite snapshot as the rows. A write
    // that races this read will still be visible to the next version check.
    let mut tx = state.db.begin().await?;
    let history_revision = sqlx::query_scalar(HISTORY_REVISION).fetch_one(&mut *tx).await?;
    let mut messages = vec![];
    if let Some(id) = &conversation_id {
        let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, String)>(
            "SELECT id, role, content, metadata, created_at FROM conversation_messages WHERE conversation_id = ? ORDER BY id ASC"
        ).bind(id).fetch_all(&mut *tx).await?;
        messages = rows
            .into_iter()
            .map(|(id, role, content, metadata, created_at)| {
                crate::commands::conversation::ConversationMessage {
                    id,
                    role,
                    content,
                    metadata,
                    created_at,
                }
            })
            .collect();
    }
    tx.commit().await?;
    Ok(Some(ChatViewSnapshot {
        version: ChatViewVersion {
            revision,
            history_revision,
            generation: state.current_conversation_generation(),
            busy: state.is_chat_busy(),
        },
        character_id,
        conversation_id,
        messages,
        turns,
    }))
}

/// One execution owner schedules speech; restoring a view never synthesizes.
pub fn speak_completed_turn(
    app: tauri::AppHandle,
    turn_id: String,
    text: String,
    speech: crate::characters::activation::ResolvedTts,
) {
    use crate::characters::activation::ResolvedTtsMode;
    let text = speech_text(&text);
    if text.trim().is_empty()
        || matches!(
            speech.mode,
            ResolvedTtsMode::TextOnly | ResolvedTtsMode::LocalPresetConfirmation
        )
    {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let config = crate::commands::tts::TtsConfig {
            provider_id: speech.provider_id,
            request_id: Some(format!("chat_{turn_id}")),
            voice: speech.voice,
            speed: speech.speed.map(|v| v as f32),
            pitch: speech.pitch.map(|v| v as f32),
            emotion: None,
        };
        if let Err(error) =
            crate::commands::tts::synthesize(app.clone(), app.state(), text, config).await
        {
            tracing::warn!(target: "tts", "chat auto speech failed: {error}");
            let _ = app.emit("chat-warning", error.to_string());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(id: &str) -> ChatRequest {
        serde_json::from_value(serde_json::json!({"message":"question", "client_request_id":id,"character_id":"character-a","conversation_id":"conversation-a"})).unwrap()
    }
    fn result(id: &str) -> Outcome {
        Ok(StreamChatResponse {
            text: "answer".into(),
            conversation_id: "conversation-a".into(),
            user_message_id: Some(1),
            assistant_message_id: Some(2),
            client_request_id: Some(id.into()),
            status: Some("completed".into()),
        })
    }
    #[tokio::test]
    async fn idle_view_check_skips_selection_lock_and_notices_history_changes() {
        let state = crate::ai::context::AIOrchestrator::new("sqlite::memory:")
            .await
            .unwrap();
        let journal = ChatRuntimeState::default();
        sqlx::query("INSERT INTO conversations (id, character_id, created_at, updated_at) VALUES ('a', 'default', 'now', 'now')")
            .execute(&state.db).await.unwrap();
        *state.current_conversation_id.lock().await = Some("a".into());
        let initial = get_chat_view(&state, &journal, None).await.unwrap().unwrap();

        // A matched version must return without contending for the selection lock.
        let guard = state.conversation_switch_lock.lock().await;
        let idle = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            get_chat_view(&state, &journal, Some(initial.version)),
        )
        .await
        .expect("idle check must not wait for selection")
        .unwrap();
        assert!(idle.is_none());
        drop(guard);

        sqlx::query("INSERT INTO conversation_messages (conversation_id, role, content, created_at) VALUES ('a', 'user', 'original', 'now')")
            .execute(&state.db).await.unwrap();
        let added = get_chat_view(&state, &journal, Some(initial.version)).await.unwrap().unwrap();
        assert_eq!(added.messages[0].content, "original");

        sqlx::query("UPDATE conversation_messages SET content = 'edited' WHERE conversation_id = 'a'")
            .execute(&state.db).await.unwrap();
        let edited = get_chat_view(&state, &journal, Some(added.version)).await.unwrap().unwrap();
        assert_eq!(edited.version.revision, added.version.revision);
        assert_eq!(edited.messages[0].content, "edited");

        sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = 'a'")
            .execute(&state.db).await.unwrap();
        let deleted = get_chat_view(&state, &journal, Some(edited.version)).await.unwrap().unwrap();
        assert!(deleted.messages.is_empty());
    }

    #[test]
    fn one_shot_camera_context_is_absent_from_turn_snapshots() {
        let journal = ChatRuntimeState::default();
        let mut with_camera = request("camera");
        let attachments = vec!["https://example.test/image.png".to_string()];
        with_camera.images = Some(attachments.clone());
        with_camera.camera_frame = Some("data:image/jpeg;base64,private-camera-frame".into());
        journal.begin(&with_camera).unwrap();
        let snapshot = journal.complete("camera", result("camera")).unwrap();
        assert_eq!(snapshot.images, attachments);
        assert!(!serde_json::to_string(&journal.snapshots()).unwrap().contains("private-camera-frame"));
    }

    #[test]
    fn duplicate_delivery_shares_outcome_without_another_execution() {
        let journal = ChatRuntimeState::default();
        let (first, a) = journal.begin(&request("a")).unwrap();
        let (second, b) = journal.begin(&request("a")).unwrap();
        assert!(first);
        assert!(!second);
        journal.complete("a", result("a")).unwrap();
        assert_eq!(*a.borrow(), *b.borrow());
        assert!(!journal.begin(&request("a")).unwrap().0);
    }
    #[test]
    fn conflict_is_rejected_without_changing_original_request() {
        let journal = ChatRuntimeState::default();
        journal.begin(&request("a")).unwrap();
        let mut other = request("a");
        other.message = "different".into();
        assert!(journal
            .begin(&other)
            .unwrap_err()
            .contains("chat_request_conflict"));
        assert_eq!(journal.snapshots()[0].message, "question");
    }
    #[test]
    fn terminal_is_single_assignment_and_ignores_late_events() {
        let journal = ChatRuntimeState::default();
        journal.begin(&request("a")).unwrap();
        journal.event(
            "chat-turn-acknowledged",
            &serde_json::json!({"client_request_id":"a","turn_id":"t"}),
        );
        journal.event(
            "chat-turn-delta",
            &serde_json::json!({"turn_id":"t","delta":"full text"}),
        );
        let before = journal.complete("a", result("a")).unwrap();
        assert!(journal.complete("a", Err("late failure".into())).is_none());
        assert!(journal
            .event(
                "chat-turn-delta",
                &serde_json::json!({"turn_id":"t","delta":"late"})
            )
            .is_none());
        let after = &journal.snapshots()[0];
        assert_eq!(before.revision, after.revision);
        assert_eq!(after.text, "full text");
        assert_eq!(after.status, "completed");
    }
    #[test]
    fn late_subscriber_gets_complete_text_and_identity_without_replaying_events() {
        let journal = ChatRuntimeState::default();
        journal.begin(&request("a")).unwrap();
        journal.event(
            "chat-turn-acknowledged",
            &serde_json::json!({"client_request_id":"a","turn_id":"t"}),
        );
        for delta in ["one ", "two"] {
            journal.event(
                "chat-turn-delta",
                &serde_json::json!({"turn_id":"t","delta":delta}),
            );
        }
        let (revision, snapshots) = journal.journal_snapshot();
        assert_eq!(snapshots[0].text, "one two");
        assert_eq!(snapshots[0].revision, revision);
        assert_eq!(snapshots[0].character_id.as_deref(), Some("character-a"));
    }
    #[test]
    fn old_request_event_cannot_modify_new_request() {
        let journal = ChatRuntimeState::default();
        journal.begin(&request("a")).unwrap();
        journal.complete("a", result("a"));
        journal.begin(&request("b")).unwrap();
        journal.event(
            "chat-turn-start",
            &serde_json::json!({"client_request_id":"a","turn_id":"old"}),
        );
        let b = journal
            .snapshots()
            .into_iter()
            .find(|s| s.client_request_id == "b")
            .unwrap();
        assert_eq!(b.status, "pending");
        assert_eq!(b.turn_id, None);
    }
    #[test]
    fn payload_retirement_never_allows_the_same_logical_request_to_execute_again() {
        let journal = ChatRuntimeState::default();
        for n in 0..300 {
            let id = n.to_string();
            journal.begin(&request(&id)).unwrap();
            journal.complete(&id, result(&id));
        }
        assert!(journal.snapshots().len() <= 256);
        let (execute_again, result) = journal.begin(&request("0")).unwrap();
        assert!(!execute_again);
        assert!(result.borrow().as_ref().unwrap().is_ok());
        let mut changed = request("0");
        changed.message = "collision".into();
        assert!(journal.begin(&changed).is_err());
    }
    #[test]
    fn cancellation_completion_race_has_one_terminal_outcome() {
        for cancellation_first in [true, false] {
            let journal = ChatRuntimeState::default();
            journal.begin(&request("a")).unwrap();
            let cancelled = Err("turn cancelled by user".into());
            let (first, second) = if cancellation_first {
                (cancelled, result("a"))
            } else {
                (result("a"), cancelled)
            };
            journal.complete("a", first).unwrap();
            assert!(journal.complete("a", second).is_none());
            assert_eq!(
                journal.snapshots()[0].status,
                if cancellation_first {
                    "cancelled"
                } else {
                    "completed"
                }
            );
        }
    }
    #[test]
    fn binding_saved_user_before_start_preserves_preparation_status() {
        let journal = ChatRuntimeState::default();
        journal.begin(&request("a")).unwrap();
        journal.event(
            "chat-turn-acknowledged",
            &serde_json::json!({"client_request_id":"a","turn_id":"t"}),
        );
        journal.event("chat-turn-bound", &serde_json::json!({"client_request_id":"a","turn_id":"t","user_message_id":41,"conversation_id":"conversation-a"}));
        let snapshot = &journal.snapshots()[0];
        assert_eq!(snapshot.status, "accepted");
        assert_eq!(snapshot.user_message_id, Some(41));
    }
    #[test]
    fn automatic_speech_does_not_read_control_markup() {
        assert_eq!(speech_text("[EMOTION:happy]Hello [TRANSLATE:你好][CUE:wave][IMAGE_PROMPT:sky][TOOL_CALL:clock]"), "Hello");
    }
}

fn speech_text(text: &str) -> String {
    let mut text =
        crate::chat::tags::strip_leaked_tags(&crate::chat::tags::strip_translate_tags(text));
    for prefix in ["[EMOTION:", "[IMAGE_PROMPT:", "[CUE:", "[TOOL_CALL:"] {
        while let Some(start) = text.find(prefix) {
            let end = text[start..]
                .find(']')
                .map(|offset| start + offset + 1)
                .unwrap_or(text.len());
            text.replace_range(start..end, "");
        }
    }
    text.trim().into()
}
