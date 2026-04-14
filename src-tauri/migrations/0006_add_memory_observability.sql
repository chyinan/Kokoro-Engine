CREATE TABLE IF NOT EXISTS memory_write_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    character_id TEXT NOT NULL,
    source TEXT NOT NULL,
    trigger TEXT NOT NULL,
    extracted_count INTEGER NOT NULL DEFAULT 0,
    stored_count INTEGER NOT NULL DEFAULT 0,
    deduplicated_count INTEGER NOT NULL DEFAULT 0,
    invalidated_count INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_write_events_character_created_at
    ON memory_write_events(character_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_memory_write_events_source_trigger
    ON memory_write_events(source, trigger, created_at DESC);

CREATE TABLE IF NOT EXISTS memory_retrieval_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    character_id TEXT NOT NULL,
    query TEXT NOT NULL,
    semantic_candidates INTEGER NOT NULL DEFAULT 0,
    bm25_candidates INTEGER NOT NULL DEFAULT 0,
    fused_candidates INTEGER NOT NULL DEFAULT 0,
    injected_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_retrieval_logs_character_created_at
    ON memory_retrieval_logs(character_id, created_at DESC);
