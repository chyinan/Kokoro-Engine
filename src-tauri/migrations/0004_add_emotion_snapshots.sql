-- Per-character emotion state persistence

CREATE TABLE IF NOT EXISTS emotion_snapshots (
    character_id TEXT PRIMARY KEY,
    emotion TEXT NOT NULL,
    mood REAL NOT NULL,
    accumulated_inertia REAL NOT NULL,
    updated_at INTEGER NOT NULL
);
