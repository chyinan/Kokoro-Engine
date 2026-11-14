//! Memory cadence belongs to a committed external conversation, not a short-lived fork.
use super::persistence::ConversationTarget;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Clone)]
pub(crate) struct MemoryTicket {
    epoch: u64,
    database_generation: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MemoryWork {
    pub count: u64,
    pub first_message_id: i64,
    pub through_message_id: i64,
    epoch: u64,
    database_generation: u64,
}

impl MemoryWork {
    pub fn extract(self) -> bool {
        self.count.is_multiple_of(5)
    }
    pub fn consolidate(self) -> bool {
        self.count.is_multiple_of(20)
    }
}

#[derive(Default)]
struct ConversationMemory {
    first_message_id: i64,
    through_message_id: i64,
    completed: u64,
}

#[derive(Default)]
struct MemoryState {
    epoch: u64,
    database_generation: u64,
    conversations: HashMap<(String, String), ConversationMemory>,
}

impl MemoryState {
    fn sync_database(&mut self) {
        let generation = crate::ai::context::database_operation_generation();
        if self.database_generation != generation {
            self.database_generation = generation;
            self.epoch += 1;
            self.conversations.clear();
        }
    }
}

#[derive(Default)]
pub(crate) struct ExternalMemory(Mutex<MemoryState>);

impl ExternalMemory {
    pub fn set_enabled(&self, flag: &AtomicBool, enabled: bool) {
        let mut state = self.0.lock().unwrap();
        state.epoch += 1;
        state.conversations.clear();
        flag.store(enabled, Ordering::SeqCst);
    }

    pub fn admit(
        &self,
        target: &ConversationTarget,
        user_id: i64,
        enabled: &AtomicBool,
    ) -> Option<MemoryTicket> {
        let mut state = self.0.lock().unwrap();
        state.sync_database();
        if !enabled.load(Ordering::SeqCst) {
            return None;
        }
        state
            .conversations
            .entry((target.character_id.clone(), target.conversation_id.clone()))
            .and_modify(|conversation| {
                conversation.first_message_id = conversation.first_message_id.min(user_id)
            })
            .or_insert(ConversationMemory {
                first_message_id: user_id,
                through_message_id: user_id,
                completed: 0,
            });
        Some(MemoryTicket {
            epoch: state.epoch,
            database_generation: state.database_generation,
        })
    }

    /// Called only for a newly committed assistant row. SQLite's save identity
    /// makes duplicate delivery a no-op before it can increment this counter.
    pub fn complete(
        &self,
        target: &ConversationTarget,
        ticket: Option<&MemoryTicket>,
        assistant_id: i64,
    ) -> Option<MemoryWork> {
        let ticket = ticket?;
        let mut state = self.0.lock().unwrap();
        state.sync_database();
        if ticket.epoch != state.epoch || ticket.database_generation != state.database_generation {
            return None;
        }
        let conversation = state
            .conversations
            .get_mut(&(target.character_id.clone(), target.conversation_id.clone()))?;
        conversation.completed += 1;
        conversation.through_message_id = conversation.through_message_id.max(assistant_id);
        Some(MemoryWork {
            count: conversation.completed,
            first_message_id: conversation.first_message_id,
            through_message_id: conversation.through_message_id,
            epoch: ticket.epoch,
            database_generation: ticket.database_generation,
        })
    }

    pub fn is_current(&self, work: MemoryWork) -> bool {
        let mut state = self.0.lock().unwrap();
        state.sync_database();
        state.epoch == work.epoch && state.database_generation == work.database_generation
    }
}
