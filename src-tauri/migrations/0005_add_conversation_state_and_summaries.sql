-- Conversation-level state and summaries for layered context compression

ALTER TABLE conversations ADD COLUMN topic TEXT NOT NULL DEFAULT '';
ALTER TABLE conversations ADD COLUMN pinned_state TEXT NOT NULL DEFAULT '{}';

CREATE TABLE IF NOT EXISTS conversation_summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    character_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    start_message_id INTEGER NOT NULL,
    end_message_id INTEGER NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending',
    failure_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_conversation_summaries_conversation_version
    ON conversation_summaries(conversation_id, version);

CREATE INDEX IF NOT EXISTS idx_conversation_summaries_ready_lookup
    ON conversation_summaries(conversation_id, status, end_message_id DESC);

CREATE INDEX IF NOT EXISTS idx_conversation_summaries_updated_at
    ON conversation_summaries(conversation_id, updated_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_conversation_summaries_active
    ON conversation_summaries(conversation_id, status)
    WHERE status IN ('pending', 'running');
