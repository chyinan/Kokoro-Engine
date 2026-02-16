use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use sqlx::{Row, SqlitePool};
use tokio::sync::Mutex;

use crate::ai::context::MemorySnippet;

pub struct MemoryManager {
    embedder: tokio::sync::OnceCell<Mutex<TextEmbedding>>,
    db: SqlitePool,
}

/// Half-life in days for memory decay (memories lose 50% relevance every N days).
const MEMORY_HALF_LIFE_DAYS: f64 = 30.0;

/// Cosine similarity threshold above which a new memory is considered a duplicate.
const DEDUP_THRESHOLD: f32 = 0.9;

/// Local model directory path (relative to working dir).
#[allow(dead_code)]
const LOCAL_MODEL_DIR: &str =
    "models/models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/5f1b8cd78bc4fb444dd171e59b18f3a3af89a079";

impl MemoryManager {
    /// Creates a new MemoryManager without downloading any models.
    /// The embedding model is lazy-loaded on first use.
    pub fn new(db: SqlitePool) -> Self {
        Self {
            embedder: tokio::sync::OnceCell::new(),
            db,
        }
    }

    /// Try to load the embedding model from local files (no network required).
    fn try_load_local() -> Option<TextEmbedding> {
        use fastembed::{InitOptionsUserDefined, TokenizerFiles, UserDefinedEmbeddingModel};
        use std::fs;
        use std::path::PathBuf;

        const SNAPSHOT: &str =
            "models/models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/5f1b8cd78bc4fb444dd171e59b18f3a3af89a079";

        // Search multiple candidate base directories
        let mut candidates: Vec<PathBuf> = Vec::new();

        // 1. Current working directory
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.clone());
            // 2. Parent of CWD (handles `src-tauri/` → project root)
            if let Some(parent) = cwd.parent() {
                candidates.push(parent.to_path_buf());
            }
        }

        // 3. Directory of the executable itself
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.to_path_buf());
            }
        }

        for base in &candidates {
            let dir = base.join(SNAPSHOT);
            let onnx = dir.join("model.onnx");

            if onnx.exists() {
                println!("[Memory] Found local model at: {}", dir.display());

                let tokenizer = dir.join("tokenizer.json");
                let config = dir.join("config.json");
                let special = dir.join("special_tokens_map.json");
                let tok_config = dir.join("tokenizer_config.json");

                if !tokenizer.exists() || !config.exists() {
                    eprintln!("[Memory] model.onnx found but tokenizer/config missing, skipping.");
                    continue;
                }

                let model_def = UserDefinedEmbeddingModel::new(
                    fs::read(&onnx).ok()?,
                    TokenizerFiles {
                        tokenizer_file: fs::read(&tokenizer).ok()?,
                        config_file: fs::read(&config).ok()?,
                        special_tokens_map_file: fs::read(&special).unwrap_or_default(),
                        tokenizer_config_file: fs::read(&tok_config).unwrap_or_default(),
                    },
                );

                match TextEmbedding::try_new_from_user_defined(
                    model_def,
                    InitOptionsUserDefined::default(),
                ) {
                    Ok(model) => {
                        println!("[Memory] Embedding model loaded successfully from local files.");
                        return Some(model);
                    }
                    Err(e) => {
                        eprintln!("[Memory] Failed to load local model: {}", e);
                    }
                }
            }
        }

        println!(
            "[Memory] No local model found. Searched: {:?}",
            candidates
                .iter()
                .map(|c| c.display().to_string())
                .collect::<Vec<_>>()
        );
        None
    }

    /// Lazily initializes the embedding model on first call.
    /// Tries local files first, then falls back to HuggingFace download.
    async fn get_embedder(&self) -> Result<&Mutex<TextEmbedding>> {
        self.embedder
            .get_or_try_init(|| async {
                // 1. Try local files (no network)
                if let Some(model) = Self::try_load_local() {
                    return Ok(Mutex::new(model));
                }

                // 2. Fall back to HF download
                println!("[Memory] Local model not found, downloading from HuggingFace...");
                let model = TextEmbedding::try_new(
                    InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                        .with_cache_dir(std::path::PathBuf::from("models")),
                )?;
                println!("[Memory] Embedding model downloaded and loaded successfully.");
                Ok(Mutex::new(model))
            })
            .await
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let embedder = self.get_embedder().await?;
        let mut guard = embedder.lock().await;
        let embeddings = guard.embed(vec![text], None)?;
        Ok(embeddings[0].clone())
    }

    pub async fn add_memory(&self, content: &str, character_id: &str) -> Result<()> {
        let embedding = self.embed(&content).await?;
        let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
        let now = chrono::Utc::now().timestamp();

        // Deduplication: check if a very similar memory already exists
        if let Ok(true) = self
            .deduplicate_or_refresh(&embedding, character_id, now)
            .await
        {
            println!(
                "[Memory] Deduplicated: refreshed existing memory for '{}'",
                &content[..content.len().min(50)]
            );
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO memories (content, embedding, created_at, importance, character_id) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(now)
        .bind(0.5) // Default importance
        .bind(character_id)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Check for duplicate memories. If a near-duplicate exists (similarity > threshold),
    /// refresh its timestamp instead of inserting a new row. Returns true if deduplicated.
    async fn deduplicate_or_refresh(
        &self,
        new_embedding: &[f32],
        character_id: &str,
        now: i64,
    ) -> Result<bool> {
        let rows = sqlx::query("SELECT id, embedding FROM memories WHERE character_id = ?")
            .bind(character_id)
            .fetch_all(&self.db)
            .await?;

        for row in rows {
            let existing_bytes: Vec<u8> = row.get("embedding");
            let existing: Vec<f32> = bincode::deserialize(&existing_bytes)?;
            let sim = cosine_similarity(new_embedding, &existing);
            if sim > DEDUP_THRESHOLD {
                let id: i64 = row.get("id");
                // Refresh the timestamp ("re-remember" this fact)
                sqlx::query("UPDATE memories SET created_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(id)
                    .execute(&self.db)
                    .await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn search_memories(
        &self,
        query: &str,
        limit: usize,
        character_id: &str,
    ) -> Result<Vec<MemorySnippet>> {
        let query_embedding = self.embed(query).await?;

        // Fetch memories for the given character (assuming < 10k items per character)
        // For larger datasets, we'd use a real vector index or sqlite-vss
        let rows =
            sqlx::query("SELECT id, content, embedding, created_at, importance FROM memories WHERE character_id = ?")
                .bind(character_id)
                .fetch_all(&self.db)
                .await?;

        let mut scored_memories: Vec<(MemorySnippet, f32)> = Vec::new();
        let now = chrono::Utc::now().timestamp();

        for row in rows {
            let embedding_bytes: Vec<u8> = row.get("embedding");
            let embedding: Vec<f32> = bincode::deserialize(&embedding_bytes)?;

            let similarity = cosine_similarity(&query_embedding, &embedding);

            // Apply time decay: score = similarity * 0.5^(age_days / half_life)
            let created_at: i64 = row.get("created_at");
            let age_days = (now - created_at) as f64 / 86400.0;
            let decay = (0.5_f64).powf(age_days / MEMORY_HALF_LIFE_DAYS) as f32;
            let final_score = similarity * decay;

            let memory = MemorySnippet {
                id: row.get("id"),
                content: row.get("content"),
                embedding: embedding_bytes,
                created_at,
                importance: row.get("importance"),
            };

            scored_memories.push((memory, final_score));
        }

        // Sort by similarity descending
        scored_memories.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Take top K
        Ok(scored_memories
            .into_iter()
            .take(limit)
            .map(|(m, _)| m)
            .collect())
    }
}

// ── Session Summaries ──────────────────────────────────────

impl MemoryManager {
    /// Ensure the session_summaries table exists.
    pub async fn ensure_session_summaries_table(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS session_summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                character_id TEXT NOT NULL,
                summary TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );",
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Save a session summary for a character.
    pub async fn save_session_summary(&self, character_id: &str, summary: &str) -> Result<()> {
        self.ensure_session_summaries_table().await?;
        sqlx::query(
            "INSERT INTO session_summaries (character_id, summary, created_at) VALUES (?, ?, ?)",
        )
        .bind(character_id)
        .bind(summary)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Get the most recent N session summaries for a character.
    pub async fn get_recent_summaries(
        &self,
        character_id: &str,
        limit: usize,
    ) -> Result<Vec<String>> {
        self.ensure_session_summaries_table().await?;
        let rows = sqlx::query(
            "SELECT summary FROM session_summaries WHERE character_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(character_id)
        .bind(limit as i64)
        .fetch_all(&self.db)
        .await?;

        Ok(rows.iter().map(|r| r.get("summary")).collect())
    }

    // ── Emotion Persistence ────────────────────────────────

    async fn ensure_emotion_snapshots_table(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS emotion_snapshots (
                character_id TEXT PRIMARY KEY,
                emotion TEXT NOT NULL,
                mood REAL NOT NULL,
                accumulated_inertia REAL NOT NULL,
                updated_at INTEGER NOT NULL
            );",
        )
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Save an emotion snapshot for a character (upsert).
    pub async fn save_emotion_snapshot(
        &self,
        character_id: &str,
        snap: &crate::ai::emotion::EmotionSnapshot,
    ) -> Result<()> {
        self.ensure_emotion_snapshots_table().await?;
        sqlx::query(
            "INSERT OR REPLACE INTO emotion_snapshots \
             (character_id, emotion, mood, accumulated_inertia, updated_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(character_id)
        .bind(&snap.emotion)
        .bind(snap.mood)
        .bind(snap.accumulated_inertia)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Load the most recent emotion snapshot for a character.
    pub async fn load_emotion_snapshot(
        &self,
        character_id: &str,
    ) -> Result<Option<crate::ai::emotion::EmotionSnapshot>> {
        self.ensure_emotion_snapshots_table().await?;
        let row = sqlx::query(
            "SELECT emotion, mood, accumulated_inertia FROM emotion_snapshots WHERE character_id = ?",
        )
        .bind(character_id)
        .fetch_optional(&self.db)
        .await?;

        Ok(row.map(|r| crate::ai::emotion::EmotionSnapshot {
            emotion: r.get("emotion"),
            mood: r.get("mood"),
            accumulated_inertia: r.get("accumulated_inertia"),
        }))
    }

    // ── Smart Memory Importance ────────────────────────────

    /// Add a memory with an explicit importance score (0.0-1.0).
    /// Higher importance memories decay slower during search.
    pub async fn add_memory_with_importance(
        &self,
        content: &str,
        character_id: &str,
        importance: f64,
    ) -> Result<()> {
        let embedding = self.embed(content).await?;
        let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
        let now = chrono::Utc::now().timestamp();

        // Deduplication check
        if let Ok(true) = self
            .deduplicate_or_refresh(&embedding, character_id, now)
            .await
        {
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO memories (content, embedding, created_at, importance, character_id) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(now)
        .bind(importance.clamp(0.0, 1.0))
        .bind(character_id)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    // ── Memory CRUD (for viewer/editor UI) ────────────────

    /// List all memories for a character, paginated, ordered by creation time desc.
    pub async fn list_memories(
        &self,
        character_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<MemoryRecord>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            "SELECT rowid AS rowid, content, created_at, importance FROM memories WHERE character_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(character_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| MemoryRecord {
                id: r.rowid,
                content: r.content,
                created_at: r.created_at,
                importance: r.importance,
            })
            .collect())
    }

    /// Count total memories for a character.
    pub async fn count_memories(&self, character_id: &str) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM memories WHERE character_id = ?")
            .bind(character_id)
            .fetch_one(&self.db)
            .await?;
        Ok(row.0)
    }

    /// Update a memory's content and importance. Re-embeds the content.
    pub async fn update_memory(&self, id: i64, content: &str, importance: f64) -> Result<()> {
        let embedding = self.embed(content).await?;
        let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;

        sqlx::query(
            "UPDATE memories SET content = ?, embedding = ?, importance = ? WHERE rowid = ?",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(importance.clamp(0.0, 1.0))
        .bind(id)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Delete a memory by ID.
    pub async fn delete_memory(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM memories WHERE rowid = ?")
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

/// Row type for paginated memory listing.
#[derive(sqlx::FromRow)]
struct MemoryRow {
    rowid: i64,
    content: String,
    created_at: i64,
    importance: f64,
}

/// Public record type returned to frontend via Tauri commands.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryRecord {
    pub id: i64,
    pub content: String,
    pub created_at: i64,
    pub importance: f64,
}

pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot_product: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a * norm_b)
    }
}
