-- Initial schema: memories, characters, conversations, conversation_messages

CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL,
    embedding BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT 0,
    importance REAL DEFAULT 0.5,
    character_id TEXT NOT NULL DEFAULT 'default',
    tier TEXT NOT NULL DEFAULT 'ephemeral',
    consolidated_from TEXT
);

CREATE TABLE IF NOT EXISTS characters (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    persona TEXT NOT NULL DEFAULT '',
    user_nickname TEXT NOT NULL DEFAULT 'User',
    source_format TEXT NOT NULL DEFAULT 'manual',
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '新对话',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS conversation_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    metadata TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);
