//! Tests for per-character memory isolation.
//!
//! These tests verify:
//! 1. The `memories` table schema includes `character_id`
//! 2. Memories inserted for one character are NOT visible to another
//! 3. The `cosine_similarity` helper works correctly
//! 4. The `AIOrchestrator` migration adds the column to legacy databases
//!
//! Note: We bypass the embedding model (fastembed) by inserting fake embeddings
//! directly via SQL, then testing the search/filter layer.

use sqlx::{Row, SqlitePool};

use super::memory::cosine_similarity;

/// Helper: create an in-memory SQLite database with the memories table.
async fn setup_db() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE memories (
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
    .await
    .unwrap();

    // FTS5 virtual table
    sqlx::query(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(content, content='memories', content_rowid='id');",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Sync triggers
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
        END;",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content);
        END;",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content);
            INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
        END;",
    )
    .execute(&pool)
    .await
    .unwrap();

    pool
}

/// Helper: insert a memory row with a fake embedding.
async fn insert_memory(pool: &SqlitePool, content: &str, character_id: &str) {
    let fake_embedding: Vec<f32> = vec![0.1, 0.2, 0.3]; // deterministic fake
    let embedding_bytes = bincode::serialize(&fake_embedding).unwrap();
    sqlx::query(
        "INSERT INTO memories (content, embedding, created_at, importance, character_id, tier) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(content)
    .bind(embedding_bytes)
    .bind(chrono::Utc::now().timestamp())
    .bind(0.5)
    .bind(character_id)
    .bind("ephemeral")
    .execute(pool)
    .await
    .unwrap();
}

/// Helper: count memories for a given character_id.
async fn count_memories(pool: &SqlitePool, character_id: &str) -> i64 {
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM memories WHERE character_id = ?")
        .bind(character_id)
        .fetch_one(pool)
        .await
        .unwrap();
    row.get::<i64, _>("cnt")
}

/// Helper: count ALL memories regardless of character.
async fn count_all_memories(pool: &SqlitePool) -> i64 {
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM memories")
        .fetch_one(pool)
        .await
        .unwrap();
    row.get::<i64, _>("cnt")
}

// ── Schema Tests ───────────────────────────────────────────

#[tokio::test]
async fn schema_has_character_id_column() {
    let pool = setup_db().await;

    // Insert with character_id should succeed
    insert_memory(&pool, "test fact", "char_42").await;

    // Verify column exists and was stored
    let row = sqlx::query("SELECT character_id FROM memories WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let cid: String = row.get("character_id");
    assert_eq!(cid, "char_42");
}

#[tokio::test]
async fn schema_default_character_id_is_default() {
    let pool = setup_db().await;

    // Insert WITHOUT explicit character_id — should use 'default'
    let fake_embedding: Vec<f32> = vec![0.1, 0.2, 0.3];
    let embedding_bytes = bincode::serialize(&fake_embedding).unwrap();
    sqlx::query(
        "INSERT INTO memories (content, embedding, created_at, importance) VALUES (?, ?, ?, ?)",
    )
    .bind("legacy memory")
    .bind(embedding_bytes)
    .bind(chrono::Utc::now().timestamp())
    .bind(0.5)
    .execute(&pool)
    .await
    .unwrap();

    let row = sqlx::query("SELECT character_id FROM memories WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let cid: String = row.get("character_id");
    assert_eq!(
        cid, "default",
        "Rows without explicit character_id should default to 'default'"
    );
}

// ── Isolation Tests ────────────────────────────────────────

#[tokio::test]
async fn memories_are_isolated_per_character() {
    let pool = setup_db().await;

    // Insert memories for two different characters
    insert_memory(&pool, "Alice likes cats", "alice").await;
    insert_memory(&pool, "Alice favorite color is blue", "alice").await;
    insert_memory(&pool, "Bob is a dog person", "bob").await;

    // Total is 3
    assert_eq!(count_all_memories(&pool).await, 3);

    // But per-character counts are separate
    assert_eq!(count_memories(&pool, "alice").await, 2);
    assert_eq!(count_memories(&pool, "bob").await, 1);
    assert_eq!(
        count_memories(&pool, "charlie").await,
        0,
        "Non-existent character should have 0 memories"
    );
}

#[tokio::test]
async fn search_only_returns_matching_character() {
    let pool = setup_db().await;

    insert_memory(&pool, "User loves Rust programming", "kokoro").await;
    insert_memory(&pool, "User prefers dark themes", "kokoro").await;
    insert_memory(&pool, "User enjoys painting watercolors", "luna").await;

    // Query filtering by character_id
    let kokoro_rows = sqlx::query("SELECT content FROM memories WHERE character_id = ?")
        .bind("kokoro")
        .fetch_all(&pool)
        .await
        .unwrap();

    assert_eq!(kokoro_rows.len(), 2);
    let contents: Vec<String> = kokoro_rows.iter().map(|r| r.get("content")).collect();
    assert!(contents.contains(&"User loves Rust programming".to_string()));
    assert!(contents.contains(&"User prefers dark themes".to_string()));
    assert!(
        !contents.iter().any(|c| c.contains("watercolors")),
        "Luna's memory should NOT appear in Kokoro's results"
    );

    // Query filtering for luna
    let luna_rows = sqlx::query("SELECT content FROM memories WHERE character_id = ?")
        .bind("luna")
        .fetch_all(&pool)
        .await
        .unwrap();

    assert_eq!(luna_rows.len(), 1);
    let luna_content: String = luna_rows[0].get("content");
    assert_eq!(luna_content, "User enjoys painting watercolors");
}

#[tokio::test]
async fn same_content_different_characters_are_independent() {
    let pool = setup_db().await;

    // Same memory content stored for two different characters
    insert_memory(&pool, "User's name is Alex", "kokoro").await;
    insert_memory(&pool, "User's name is Alex", "luna").await;

    assert_eq!(count_all_memories(&pool).await, 2, "Both rows should exist");
    assert_eq!(count_memories(&pool, "kokoro").await, 1);
    assert_eq!(count_memories(&pool, "luna").await, 1);
}

// ── Migration Test ─────────────────────────────────────────

#[tokio::test]
async fn alter_table_migration_adds_column_to_legacy_db() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    // Create a LEGACY table without character_id
    sqlx::query(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            embedding BLOB NOT NULL,
            created_at INTEGER NOT NULL,
            importance REAL DEFAULT 0.5
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Insert a legacy row
    let fake_embedding: Vec<f32> = vec![0.1, 0.2, 0.3];
    let embedding_bytes = bincode::serialize(&fake_embedding).unwrap();
    sqlx::query("INSERT INTO memories (content, embedding, created_at) VALUES (?, ?, ?)")
        .bind("old memory")
        .bind(&embedding_bytes)
        .bind(0i64)
        .execute(&pool)
        .await
        .unwrap();

    // Run the ALTER TABLE migration (same as AIOrchestrator::new does)
    let _ =
        sqlx::query("ALTER TABLE memories ADD COLUMN character_id TEXT NOT NULL DEFAULT 'default'")
            .execute(&pool)
            .await;

    // Legacy row should now have character_id = 'default'
    let row = sqlx::query("SELECT character_id FROM memories WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let cid: String = row.get("character_id");
    assert_eq!(
        cid, "default",
        "Legacy rows should get 'default' after migration"
    );

    // New inserts with explicit character_id should work
    sqlx::query(
        "INSERT INTO memories (content, embedding, created_at, character_id) VALUES (?, ?, ?, ?)",
    )
    .bind("new memory")
    .bind(&embedding_bytes)
    .bind(1i64)
    .bind("kokoro_v2")
    .execute(&pool)
    .await
    .unwrap();

    let row2 = sqlx::query("SELECT character_id FROM memories WHERE id = 2")
        .fetch_one(&pool)
        .await
        .unwrap();
    let cid2: String = row2.get("character_id");
    assert_eq!(cid2, "kokoro_v2");
}

// ── Cosine Similarity Unit Tests ───────────────────────────

#[test]
fn cosine_similarity_identical_vectors() {
    let v = vec![1.0, 2.0, 3.0];
    let sim = cosine_similarity(&v, &v);
    assert!(
        (sim - 1.0).abs() < 1e-6,
        "Identical vectors should have similarity ≈ 1.0, got {}",
        sim
    );
}

#[test]
fn cosine_similarity_orthogonal_vectors() {
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![0.0, 1.0, 0.0];
    let sim = cosine_similarity(&a, &b);
    assert!(
        sim.abs() < 1e-6,
        "Orthogonal vectors should have similarity ≈ 0.0, got {}",
        sim
    );
}

#[test]
fn cosine_similarity_opposite_vectors() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![-1.0, -2.0, -3.0];
    let sim = cosine_similarity(&a, &b);
    assert!(
        (sim + 1.0).abs() < 1e-6,
        "Opposite vectors should have similarity ≈ -1.0, got {}",
        sim
    );
}

#[test]
fn cosine_similarity_zero_vector_returns_zero() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![0.0, 0.0, 0.0];
    assert_eq!(
        cosine_similarity(&a, &b),
        0.0,
        "Zero vector should yield 0.0"
    );
    assert_eq!(
        cosine_similarity(&b, &a),
        0.0,
        "Zero vector should yield 0.0 (commutative)"
    );
}

// ── Helper: insert memory with tier and importance ────────

async fn insert_memory_with_tier(
    pool: &SqlitePool,
    content: &str,
    character_id: &str,
    importance: f64,
    tier: &str,
    embedding: &[f32],
    created_at: i64,
) {
    let embedding_bytes = bincode::serialize(embedding).unwrap();
    sqlx::query(
        "INSERT INTO memories (content, embedding, created_at, importance, character_id, tier) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(content)
    .bind(embedding_bytes)
    .bind(created_at)
    .bind(importance)
    .bind(character_id)
    .bind(tier)
    .execute(pool)
    .await
    .unwrap();
}

// ── Phase 1: Tiered Memory Tests ──────────────────────────

#[tokio::test]
async fn tier_column_defaults_to_ephemeral() {
    let pool = setup_db().await;
    insert_memory(&pool, "some fact", "alice").await;

    let row = sqlx::query("SELECT tier FROM memories WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tier: String = row.get("tier");
    assert_eq!(tier, "ephemeral");
}

#[tokio::test]
async fn core_tier_can_be_set() {
    let pool = setup_db().await;
    let emb: Vec<f32> = vec![1.0, 0.0, 0.0];
    insert_memory_with_tier(&pool, "User's name is Alice", "char1", 0.9, "core", &emb, chrono::Utc::now().timestamp()).await;

    let row = sqlx::query("SELECT tier, importance FROM memories WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tier: String = row.get("tier");
    let importance: f64 = row.get("importance");
    assert_eq!(tier, "core");
    assert!((importance - 0.9).abs() < 1e-6);
}

#[tokio::test]
async fn tier_migration_defaults_existing_rows() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    // Create legacy table without tier column
    sqlx::query(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            embedding BLOB NOT NULL,
            created_at INTEGER NOT NULL,
            importance REAL DEFAULT 0.5,
            character_id TEXT NOT NULL DEFAULT 'default'
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Insert a legacy row
    let emb_bytes = bincode::serialize(&vec![0.1_f32, 0.2, 0.3]).unwrap();
    sqlx::query("INSERT INTO memories (content, embedding, created_at) VALUES (?, ?, ?)")
        .bind("old memory")
        .bind(&emb_bytes)
        .bind(0i64)
        .execute(&pool)
        .await
        .unwrap();

    // Run migration
    let _ = sqlx::query("ALTER TABLE memories ADD COLUMN tier TEXT NOT NULL DEFAULT 'ephemeral'")
        .execute(&pool)
        .await;

    let row = sqlx::query("SELECT tier FROM memories WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let tier: String = row.get("tier");
    assert_eq!(tier, "ephemeral", "Legacy rows should default to 'ephemeral'");
}

// ── Phase 2: FTS5 / Hybrid Retrieval Tests ────────────────

#[tokio::test]
async fn fts5_keyword_search_finds_exact_match() {
    let pool = setup_db().await;
    insert_memory(&pool, "User's birthday is March 15th", "alice").await;
    insert_memory(&pool, "User likes chocolate cake", "alice").await;
    insert_memory(&pool, "User works at Anthropic", "alice").await;

    // Search for "birthday" — should find exactly one
    let rows = sqlx::query(
        "SELECT m.content FROM memories_fts f JOIN memories m ON m.id = f.rowid WHERE memories_fts MATCH '\"birthday\"' AND m.character_id = 'alice'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 1);
    let content: String = rows[0].get("content");
    assert!(content.contains("birthday"));
}

#[tokio::test]
async fn fts5_syncs_on_delete() {
    let pool = setup_db().await;
    insert_memory(&pool, "User loves Rust programming", "bob").await;

    // Verify FTS has it
    let before = sqlx::query(
        "SELECT COUNT(*) as cnt FROM memories_fts WHERE memories_fts MATCH '\"Rust\"'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before.get::<i64, _>("cnt"), 1);

    // Delete the memory
    sqlx::query("DELETE FROM memories WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();

    // FTS should be empty now
    let after = sqlx::query(
        "SELECT COUNT(*) as cnt FROM memories_fts WHERE memories_fts MATCH '\"Rust\"'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(after.get::<i64, _>("cnt"), 0, "FTS should sync on delete");
}

#[tokio::test]
async fn fts5_handles_special_characters() {
    let pool = setup_db().await;
    insert_memory(&pool, "User's email is test@example.com", "alice").await;

    // The escape function wraps words in quotes
    let query = super::memory::escape_fts5_query("test@example.com");
    assert!(!query.is_empty());
    // Should not panic or error
    let result = sqlx::query(&format!(
        "SELECT COUNT(*) as cnt FROM memories_fts WHERE memories_fts MATCH '{}'",
        query.replace('\'', "''")
    ))
    .fetch_one(&pool)
    .await;
    assert!(result.is_ok());
}

// ── Phase 3: Consolidation Logic Tests ────────────────────

#[tokio::test]
async fn similar_memories_are_in_same_cluster() {
    // Test the clustering logic by checking cosine similarity threshold
    let a = vec![1.0, 0.0, 0.0];
    let b = vec![0.95, 0.31, 0.0]; // cos sim ≈ 0.95 > 0.75
    let c = vec![0.0, 0.0, 1.0]; // cos sim ≈ 0.0 < 0.75

    let sim_ab = cosine_similarity(&a, &b);
    let sim_ac = cosine_similarity(&a, &c);

    assert!(sim_ab > 0.75, "Similar vectors should exceed threshold: {}", sim_ab);
    assert!(sim_ac < 0.75, "Dissimilar vectors should be below threshold: {}", sim_ac);
}

#[tokio::test]
async fn consolidated_from_column_stores_json() {
    let pool = setup_db().await;
    let emb: Vec<f32> = vec![1.0, 0.0, 0.0];
    let emb_bytes = bincode::serialize(&emb).unwrap();
    let source_ids = vec![1i64, 2, 3];
    let json = serde_json::to_string(&source_ids).unwrap();

    sqlx::query(
        "INSERT INTO memories (content, embedding, created_at, importance, character_id, tier, consolidated_from) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("Merged memory")
    .bind(&emb_bytes)
    .bind(chrono::Utc::now().timestamp())
    .bind(0.9)
    .bind("alice")
    .bind("core")
    .bind(&json)
    .execute(&pool)
    .await
    .unwrap();

    let row = sqlx::query("SELECT consolidated_from, tier FROM memories WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let cf: String = row.get("consolidated_from");
    let tier: String = row.get("tier");
    let parsed: Vec<i64> = serde_json::from_str(&cf).unwrap();
    assert_eq!(parsed, vec![1, 2, 3]);
    assert_eq!(tier, "core");
}

#[tokio::test]
async fn escape_fts5_query_handles_empty_and_quotes() {
    use super::memory::escape_fts5_query;

    assert_eq!(escape_fts5_query(""), "");
    assert_eq!(escape_fts5_query("hello world"), "\"hello\" OR \"world\"");
    // Quotes should be stripped to prevent injection
    assert_eq!(escape_fts5_query("hello\"world"), "\"helloworld\"");
    assert_eq!(escape_fts5_query("  spaced  "), "\"spaced\"");
}
