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
            character_id TEXT NOT NULL DEFAULT 'default'
        );",
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
        "INSERT INTO memories (content, embedding, created_at, importance, character_id) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(content)
    .bind(embedding_bytes)
    .bind(chrono::Utc::now().timestamp())
    .bind(0.5)
    .bind(character_id)
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
