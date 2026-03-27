-- FTS5 virtual table for hybrid memory retrieval (BM25 keyword search)

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    content,
    content='memories',
    content_rowid='id'
);

-- Sync triggers: keep FTS index in sync with memories table
CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content);
END;

CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content);
    INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
END;

-- Rebuild FTS index to backfill any existing data
INSERT INTO memories_fts(memories_fts) VALUES('rebuild');
