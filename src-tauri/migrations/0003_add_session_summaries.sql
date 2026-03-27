-- Session summaries for context compression

CREATE TABLE IF NOT EXISTS session_summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    character_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
