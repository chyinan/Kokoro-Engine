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
/// 0.95+ = near-identical wording; 0.85 was too aggressive and collapsed distinct memories
/// about the same topic/person into false duplicates.
const DEDUP_THRESHOLD: f32 = 0.95;

/// Cosine similarity threshold for memory consolidation clustering.
/// 0.85 requires strong topical overlap; 0.75 was too loose and merged unrelated topics.
const CONSOLIDATION_THRESHOLD: f32 = 0.85;

/// Maximum time gap (in seconds) between memories that can be consolidated together.
/// Memories created more than N days apart are unlikely to be about the same event.
const CONSOLIDATION_TIME_WINDOW_SECS: i64 = 7 * 24 * 3600; // 7 days

/// Maximum number of memories in a single consolidation cluster.
const MAX_CLUSTER_SIZE: usize = 5;

/// Local model directory path (relative to working dir).
#[allow(dead_code)]
const LOCAL_MODEL_DIR: &str = "models/models--Qdrant--all-MiniLM-L6-v2-onnx";
const MODEL_REPO: &str = "Qdrant/all-MiniLM-L6-v2-onnx";
const MODEL_AUX_FILES: &[&str] = &[
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
    "vocab.txt",
];

impl MemoryManager {
    /// Creates a new MemoryManager without downloading any models.
    /// The embedding model is lazy-loaded on first use.
    pub fn new(db: SqlitePool) -> Self {
        Self {
            embedder: tokio::sync::OnceCell::new(),
            db,
        }
    }

    fn resolve_snapshot_dir(repo_dir: &std::path::Path) -> Option<std::path::PathBuf> {
        use std::fs;
        let refs_main = repo_dir.join("refs").join("main");
        if let Ok(rev) = fs::read_to_string(&refs_main) {
            let rev = rev.trim();
            if !rev.is_empty() {
                let snapshot = repo_dir.join("snapshots").join(rev);
                if snapshot.exists() {
                    return Some(snapshot);
                }
            }
        }

        let snapshots = repo_dir.join("snapshots");
        fs::read_dir(&snapshots)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.is_dir())
    }

    fn model_search_roots() -> Vec<std::path::PathBuf> {
        use std::path::PathBuf;

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
                // 4. Resources dir bundled alongside the exe (Tauri build layout)
                candidates.push(exe_dir.join("_up_").join(".."));
            }
        }

        // 5. App data dir (com.chyin.kokoro) — models copied here on first run
        if let Some(app_data) = dirs_next::data_dir() {
            candidates.push(app_data.join("com.chyin.kokoro"));
        }

        candidates
    }

    async fn hydrate_missing_local_files(snapshot_dir: &std::path::Path) -> Result<bool> {
        let missing: Vec<&str> = MODEL_AUX_FILES
            .iter()
            .copied()
            .filter(|name| !snapshot_dir.join(name).exists())
            .collect();

        if missing.is_empty() {
            return Ok(false);
        }

        println!(
            "[Memory] Hydrating missing tokenizer/config files in {}: {:?}",
            snapshot_dir.display(),
            missing
        );

        let client = reqwest::Client::builder()
            .user_agent("kokoro-engine/0.1.4")
            .build()?;

        tokio::fs::create_dir_all(snapshot_dir).await?;

        for file in &missing {
            let url = format!(
                "https://huggingface.co/{}/resolve/main/{}",
                MODEL_REPO, file
            );
            let bytes = client
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            tokio::fs::write(snapshot_dir.join(file), &bytes).await?;
        }

        Ok(true)
    }

    /// Try to load the embedding model from local files (no network required).
    fn try_load_local() -> Option<TextEmbedding> {
        use fastembed::{InitOptionsUserDefined, TokenizerFiles, UserDefinedEmbeddingModel};
        use std::fs;

        let candidates = Self::model_search_roots();

        for base in &candidates {
            let repo_dir = base.join(LOCAL_MODEL_DIR);
            let Some(dir) = Self::resolve_snapshot_dir(&repo_dir) else {
                continue;
            };
            let onnx = dir.join("model.onnx");

            if onnx.exists() {
                println!("[Memory] Found local model at: {}", dir.display());

                let tokenizer = dir.join("tokenizer.json");
                let config = dir.join("config.json");
                let special = dir.join("special_tokens_map.json");
                let tok_config = dir.join("tokenizer_config.json");

                if !tokenizer.exists() || !config.exists() {
                    eprintln!(
                        "[Memory] model.onnx found but tokenizer/config missing in {}, skipping.",
                        dir.display()
                    );
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

                // 1.5 If a partial local cache exists, fill in the small tokenizer/config files
                // and retry local loading before falling back to fastembed's downloader again.
                for base in Self::model_search_roots() {
                    let repo_dir = base.join(LOCAL_MODEL_DIR);
                    let Some(snapshot_dir) = Self::resolve_snapshot_dir(&repo_dir) else {
                        continue;
                    };
                    if !snapshot_dir.join("model.onnx").exists() {
                        continue;
                    }
                    if Self::hydrate_missing_local_files(&snapshot_dir).await? {
                        if let Some(model) = Self::try_load_local() {
                            return Ok(Mutex::new(model));
                        }
                    }
                }

                // 2. Fall back to HF download — cache into app data so build can find it next time
                println!("[Memory] Local model not found, downloading from HuggingFace...");
                let cache_dir = dirs_next::data_dir()
                    .map(|d| d.join("com.chyin.kokoro").join("models"))
                    .unwrap_or_else(|| std::path::PathBuf::from("models"));
                let _ = std::fs::create_dir_all(&cache_dir);
                let model = TextEmbedding::try_new(
                    InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                        .with_cache_dir(cache_dir),
                )?;

                // Persist any missing small files so the next startup can load locally
                // instead of repeatedly treating the cache as incomplete.
                let app_repo_dir = dirs_next::data_dir()
                    .map(|d| d.join("com.chyin.kokoro").join(LOCAL_MODEL_DIR))
                    .unwrap_or_else(|| std::path::PathBuf::from(LOCAL_MODEL_DIR));
                if let Some(snapshot_dir) = Self::resolve_snapshot_dir(&app_repo_dir) {
                    let _ = Self::hydrate_missing_local_files(&snapshot_dir).await;
                }

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
        let embedding = self.embed(content).await?;
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
            "INSERT INTO memories (content, embedding, created_at, updated_at, importance, character_id, tier) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(now)
        .bind(now)
        .bind(0.5) // Default importance
        .bind(character_id)
        .bind("ephemeral")
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Return all memory content strings for a given character (used for dedup in extraction).
    pub async fn get_all_memory_contents(&self, character_id: &str) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT content FROM memories WHERE character_id = ? ORDER BY importance DESC LIMIT 50")
            .bind(character_id)
            .fetch_all(&self.db)
            .await?;
        Ok(rows.iter().map(|r| r.get::<String, _>("content")).collect())
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
                println!("[Memory] Dedup: similarity={:.3} > {:.3}, refreshing id={}", sim, DEDUP_THRESHOLD, id);
                // Refresh updated_at only — created_at must remain immutable
                sqlx::query("UPDATE memories SET updated_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(id)
                    .execute(&self.db)
                    .await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Like `deduplicate_or_refresh`, but also upgrades importance and tier if the
    /// new extraction has higher importance than the existing duplicate.
    async fn deduplicate_or_upgrade(
        &self,
        new_embedding: &[f32],
        character_id: &str,
        now: i64,
        new_importance: f64,
    ) -> Result<bool> {
        let rows =
            sqlx::query("SELECT id, embedding, importance FROM memories WHERE character_id = ?")
                .bind(character_id)
                .fetch_all(&self.db)
                .await?;

        for row in rows {
            let existing_bytes: Vec<u8> = row.get("embedding");
            let existing: Vec<f32> = bincode::deserialize(&existing_bytes)?;
            let sim = cosine_similarity(new_embedding, &existing);
            if sim > DEDUP_THRESHOLD {
                let id: i64 = row.get("id");
                println!("[Memory] Dedup-upgrade: similarity={:.3} > {:.3}, upgrading id={}", sim, DEDUP_THRESHOLD, id);
                let existing_importance: f64 = row.get("importance");
                let best_importance = existing_importance.max(new_importance);
                let tier = if best_importance >= 0.8 {
                    "core"
                } else {
                    "ephemeral"
                };
                sqlx::query(
                    "UPDATE memories SET updated_at = ?, importance = ?, tier = ? WHERE id = ?",
                )
                .bind(now)
                .bind(best_importance)
                .bind(tier)
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
        // Run semantic search and BM25 search in parallel
        let semantic_results = self.semantic_search(query, limit * 2, character_id).await?;
        let bm25_results = self.bm25_search(query, character_id, limit * 2).await.unwrap_or_default();

        // RRF (Reciprocal Rank Fusion) with k=60
        let k = 60.0_f32;
        let mut rrf_scores: std::collections::HashMap<i64, (f32, MemorySnippet)> = std::collections::HashMap::new();

        for (rank, mem) in semantic_results.iter().enumerate() {
            let score = 1.0 / (k + rank as f32 + 1.0);
            rrf_scores.entry(mem.id).or_insert((0.0, mem.clone())).0 += score;
        }

        for (rank, (id, _bm25_score)) in bm25_results.iter().enumerate() {
            let score = 1.0 / (k + rank as f32 + 1.0);
            if let Some(entry) = rrf_scores.get_mut(id) {
                entry.0 += score;
            } else {
                // BM25 found a memory not in semantic results — fetch it
                if let Ok(Some(snippet)) = self.fetch_memory_snippet(*id).await {
                    rrf_scores.insert(*id, (score, snippet));
                }
            }
        }

        let mut fused: Vec<(f32, MemorySnippet)> = rrf_scores.into_values().collect();
        fused.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        Ok(fused.into_iter().take(limit).map(|(_, m)| m).collect())
    }

    /// Pure semantic (embedding) search with time decay, respecting tier.
    async fn semantic_search(
        &self,
        query: &str,
        limit: usize,
        character_id: &str,
    ) -> Result<Vec<MemorySnippet>> {
        let query_embedding = self.embed(query).await?;

        let rows =
            sqlx::query("SELECT id, content, embedding, created_at, importance, tier FROM memories WHERE character_id = ?")
                .bind(character_id)
                .fetch_all(&self.db)
                .await?;

        let mut scored_memories: Vec<(MemorySnippet, f32)> = Vec::new();
        let now = chrono::Utc::now().timestamp();

        for row in rows {
            let embedding_bytes: Vec<u8> = row.get("embedding");
            let embedding: Vec<f32> = bincode::deserialize(&embedding_bytes)?;

            let similarity = cosine_similarity(&query_embedding, &embedding);

            let created_at: i64 = row.get("created_at");
            let tier: String = row.get("tier");

            // Core memories never decay; ephemeral memories use time decay
            let decay = if tier == "core" {
                1.0_f32
            } else {
                let age_days = (now - created_at) as f64 / 86400.0;
                (0.5_f64).powf(age_days / MEMORY_HALF_LIFE_DAYS) as f32
            };
            let final_score = similarity * decay;

            let memory = MemorySnippet {
                id: row.get("id"),
                content: row.get("content"),
                embedding: embedding_bytes,
                created_at,
                importance: row.get("importance"),
                tier,
            };

            scored_memories.push((memory, final_score));
        }

        scored_memories.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored_memories
            .into_iter()
            .take(limit)
            .map(|(m, _)| m)
            .collect())
    }

    /// BM25 keyword search via FTS5. Returns (memory_id, bm25_score) pairs.
    async fn bm25_search(
        &self,
        query: &str,
        character_id: &str,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>> {
        let fts_query = escape_fts5_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query(
            "SELECT m.id, bm25(memories_fts) AS score \
             FROM memories_fts f \
             JOIN memories m ON m.id = f.rowid \
             WHERE memories_fts MATCH ? AND m.character_id = ? \
             ORDER BY score \
             LIMIT ?",
        )
        .bind(&fts_query)
        .bind(character_id)
        .bind(limit as i64)
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let id: i64 = r.get("id");
                let score: f64 = r.get("score");
                (id, score)
            })
            .collect())
    }

    /// Fetch a single memory snippet by ID.
    async fn fetch_memory_snippet(&self, id: i64) -> Result<Option<MemorySnippet>> {
        let row = sqlx::query(
            "SELECT id, content, embedding, created_at, importance, tier FROM memories WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;

        Ok(row.map(|r| MemorySnippet {
            id: r.get("id"),
            content: r.get("content"),
            embedding: r.get("embedding"),
            created_at: r.get("created_at"),
            importance: r.get("importance"),
            tier: r.get("tier"),
        }))
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

        // Deduplication check — also upgrades importance/tier if duplicate found
        if let Ok(true) = self
            .deduplicate_or_upgrade(&embedding, character_id, now, importance)
            .await
        {
            return Ok(());
        }

        let tier = if importance >= 0.8 { "core" } else { "ephemeral" };

        sqlx::query(
            "INSERT INTO memories (content, embedding, created_at, updated_at, importance, character_id, tier) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(now)
        .bind(now)
        .bind(importance.clamp(0.0, 1.0))
        .bind(character_id)
        .bind(tier)
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
            "SELECT rowid AS rowid, content, created_at, importance, tier FROM memories WHERE character_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(character_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.db)
        .await?;

        let now = chrono::Utc::now().timestamp();
        Ok(rows
            .into_iter()
            .map(|r| {
                let effective_importance = if r.tier == "core" {
                    r.importance
                } else {
                    let age_days = (now - r.created_at) as f64 / 86400.0;
                    let decay = (0.5_f64).powf(age_days / MEMORY_HALF_LIFE_DAYS);
                    r.importance * decay
                };
                MemoryRecord {
                    id: r.rowid,
                    content: r.content,
                    created_at: r.created_at,
                    importance: effective_importance,
                    tier: r.tier,
                }
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
    /// Automatically syncs tier based on new importance.
    pub async fn update_memory(&self, id: i64, content: &str, importance: f64) -> Result<()> {
        let embedding = self.embed(content).await?;
        let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
        let clamped = importance.clamp(0.0, 1.0);
        let tier = if clamped >= 0.8 { "core" } else { "ephemeral" };

        sqlx::query(
            "UPDATE memories SET content = ?, embedding = ?, importance = ?, tier = ? WHERE rowid = ?",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(clamped)
        .bind(tier)
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

    /// Update a memory's tier (e.g. "core" or "ephemeral").
    pub async fn update_memory_tier(&self, id: i64, tier: &str) -> Result<()> {
        sqlx::query("UPDATE memories SET tier = ? WHERE rowid = ?")
            .bind(tier)
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Delete ephemeral memories whose effective importance (original × decay) has fallen
    /// below `threshold`. Core memories are never pruned.
    /// Returns the number of deleted rows.
    pub async fn prune_decayed_memories(
        &self,
        character_id: &str,
        threshold: f64,
    ) -> Result<u64> {
        let now = chrono::Utc::now().timestamp();
        // Compute the minimum age (in seconds) at which a memory with max importance (1.0)
        // would decay below the threshold:
        //   threshold = importance * 0.5^(age_days / half_life)
        // Worst case importance = 1.0, so:
        //   age_days > half_life * log2(1 / threshold)
        let min_age_days = MEMORY_HALF_LIFE_DAYS * (1.0_f64 / threshold).log2();
        let cutoff_ts = now - (min_age_days * 86400.0) as i64;

        // Among records old enough to potentially be below threshold, check each one.
        // We do the exact per-row check in Rust to handle varying importance values.
        let rows = sqlx::query(
            "SELECT rowid, created_at, importance FROM memories \
             WHERE character_id = ? AND tier = 'ephemeral' AND created_at < ?",
        )
        .bind(character_id)
        .bind(cutoff_ts)
        .fetch_all(&self.db)
        .await?;

        let mut deleted = 0u64;
        for row in rows {
            let id: i64 = row.get("rowid");
            let created_at: i64 = row.get("created_at");
            let importance: f64 = row.get("importance");
            let age_days = (now - created_at) as f64 / 86400.0;
            let decay = (0.5_f64).powf(age_days / MEMORY_HALF_LIFE_DAYS);
            if importance * decay < threshold {
                sqlx::query("DELETE FROM memories WHERE rowid = ?")
                    .bind(id)
                    .execute(&self.db)
                    .await?;
                deleted += 1;
            }
        }

        if deleted > 0 {
            println!(
                "[Memory] Pruned {} decayed ephemeral memories for '{}'",
                deleted, character_id
            );
        }
        Ok(deleted)
    }
}

// ── Memory Consolidation ──────────────────────────────────────

impl MemoryManager {
    /// Find clusters of similar memories and merge them via LLM.
    /// Inserts consolidated memories and deletes the source fragments.
    pub async fn consolidate_memories(
        &self,
        character_id: &str,
        provider: std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
    ) -> Result<usize> {
        // 1. Load all memories with embeddings for this character
        let rows = sqlx::query(
            "SELECT id, content, embedding, created_at, importance, tier FROM memories WHERE character_id = ?",
        )
        .bind(character_id)
        .fetch_all(&self.db)
        .await?;

        if rows.len() < 2 {
            return Ok(0);
        }

        // Parse into (id, content, embedding, importance, tier, created_at)
        let mut entries: Vec<(i64, String, Vec<f32>, f64, String, i64)> = Vec::new();
        for row in &rows {
            let embedding_bytes: Vec<u8> = row.get("embedding");
            let embedding: Vec<f32> = bincode::deserialize(&embedding_bytes)?;
            entries.push((
                row.get("id"),
                row.get("content"),
                embedding,
                row.get("importance"),
                row.get("tier"),
                row.get("created_at"),
            ));
        }

        // 2. Greedy clustering: group similar memories
        let mut used = vec![false; entries.len()];
        let mut clusters: Vec<Vec<usize>> = Vec::new();

        for i in 0..entries.len() {
            if used[i] {
                continue;
            }
            let mut cluster = vec![i];
            used[i] = true;

            for j in (i + 1)..entries.len() {
                if used[j] || cluster.len() >= MAX_CLUSTER_SIZE {
                    break;
                }
                let sim = cosine_similarity(&entries[i].2, &entries[j].2);
                let time_diff = (entries[i].5 - entries[j].5).abs();
                if sim > CONSOLIDATION_THRESHOLD && time_diff <= CONSOLIDATION_TIME_WINDOW_SECS {
                    cluster.push(j);
                    used[j] = true;
                }
            }

            // Only consolidate clusters with 2+ memories
            if cluster.len() >= 2 {
                clusters.push(cluster);
            }
        }

        if clusters.is_empty() {
            return Ok(0);
        }

        let mut consolidated_count = 0;

        // 3. For each cluster, merge via LLM
        for cluster in &clusters {
            let facts: Vec<&str> = cluster.iter().map(|&idx| entries[idx].1.as_str()).collect();
            let source_ids: Vec<i64> = cluster.iter().map(|&idx| entries[idx].0).collect();

            // Inherit max importance; if any is core, result is core
            let max_importance = cluster
                .iter()
                .map(|&idx| entries[idx].3)
                .fold(0.0_f64, f64::max);
            let tier = if cluster.iter().any(|&idx| entries[idx].4 == "core") {
                "core"
            } else {
                "ephemeral"
            };
            // 保留最早的 created_at，避免整合后记忆时间被重置
            let earliest_created_at = cluster
                .iter()
                .map(|&idx| entries[idx].5)
                .min()
                .unwrap_or_else(|| chrono::Utc::now().timestamp());

            // Call LLM to merge facts
            let merged = match merge_facts_via_llm(&facts, &provider).await {
                Ok(text) => text,
                Err(e) => {
                    eprintln!("[Memory] Consolidation LLM call failed: {}", e);
                    continue;
                }
            };

            if merged.trim().is_empty() {
                continue;
            }

            // 4. Insert consolidated memory
            let embedding = match self.embed(&merged).await {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[Memory] Failed to embed consolidated memory: {}", e);
                    continue;
                }
            };
            let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
            let now = chrono::Utc::now().timestamp();
            let consolidated_from_json = serde_json::to_string(&source_ids)?;

            sqlx::query(
                "INSERT INTO memories (content, embedding, created_at, updated_at, importance, character_id, tier, consolidated_from) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&merged)
            .bind(&embedding_bytes)
            .bind(earliest_created_at)
            .bind(now)
            .bind(max_importance)
            .bind(character_id)
            .bind(tier)
            .bind(&consolidated_from_json)
            .execute(&self.db)
            .await?;

            // 5. Delete source memories
            for id in &source_ids {
                sqlx::query("DELETE FROM memories WHERE id = ?")
                    .bind(id)
                    .execute(&self.db)
                    .await?;
            }

            consolidated_count += 1;
            println!(
                "[Memory] Consolidated {} memories into: {}",
                source_ids.len(),
                &merged[..merged.len().min(80)]
            );
        }

        Ok(consolidated_count)
    }
}

/// Row type for paginated memory listing.
#[derive(sqlx::FromRow)]
struct MemoryRow {
    rowid: i64,
    content: String,
    created_at: i64,
    importance: f64,
    tier: String,
}

/// Public record type returned to frontend via Tauri commands.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryRecord {
    pub id: i64,
    pub content: String,
    pub created_at: i64,
    pub importance: f64,
    pub tier: String,
}

/// Escape user input for FTS5 MATCH syntax.
/// Wraps each word in double quotes and joins with OR.
pub(crate) fn escape_fts5_query(query: &str) -> String {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            // Remove any double quotes from the word to prevent injection
            w.replace('"', "")
        })
        .filter(|w| !w.is_empty())
        .map(|clean| format!("\"{}\"", clean))
        .collect();
    words.join(" OR ")
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

/// Use LLM to merge multiple related facts into a single consolidated memory.
async fn merge_facts_via_llm(
    facts: &[&str],
    provider: &std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
) -> Result<String> {
    use crate::llm::openai::{Message, MessageContent};

    let facts_list = facts
        .iter()
        .enumerate()
        .map(|(i, f)| format!("{}. {}", i + 1, f))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "You are a memory consolidation assistant. Merge the following related facts into a single, \
         concise, and complete memory entry. Preserve all important details. Do not add information \
         that is not present in the original facts. Output only the merged memory text, nothing else.\n\n\
         Facts:\n{}",
        facts_list
    );

    let messages = vec![Message {
        role: "user".to_string(),
        content: MessageContent::Text(prompt),
    }];

    let result = provider
        .chat(messages, None)
        .await
        .map_err(|e| anyhow::anyhow!("LLM merge failed: {}", e))?;

    Ok(result.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 0.0001,
            "Identical vectors should have similarity 1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 0.0001,
            "Orthogonal vectors should have similarity ~0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - (-1.0)).abs() < 0.0001,
            "Opposite vectors should have similarity -1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_zero_vector_a() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(
            sim, 0.0,
            "Zero vector should produce similarity 0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_zero_vector_b() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(
            sim, 0.0,
            "Zero vector should produce similarity 0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_both_zero_vectors() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(
            sim, 0.0,
            "Both zero vectors should produce similarity 0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_scaled_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![2.0, 4.0, 6.0]; // b = 2 * a
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 0.0001,
            "Scaled vectors should have similarity 1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_escape_fts5_query_empty_string() {
        let result = escape_fts5_query("");
        assert_eq!(result, "", "Empty string should produce empty result");
    }

    #[test]
    fn test_escape_fts5_query_single_word() {
        let result = escape_fts5_query("hello");
        assert_eq!(
            result, "\"hello\"",
            "Single word should be wrapped in quotes"
        );
    }

    #[test]
    fn test_escape_fts5_query_multiple_words() {
        let result = escape_fts5_query("hello world test");
        assert_eq!(
            result, "\"hello\" OR \"world\" OR \"test\"",
            "Multiple words should be joined with OR"
        );
    }

    #[test]
    fn test_escape_fts5_query_with_embedded_quotes() {
        let result = escape_fts5_query("hello \"world\" test");
        assert_eq!(
            result, "\"hello\" OR \"world\" OR \"test\"",
            "Embedded quotes should be removed for injection prevention"
        );
    }

    #[test]
    fn test_escape_fts5_query_whitespace_only() {
        let result = escape_fts5_query("   \t  \n  ");
        assert_eq!(
            result, "",
            "Whitespace-only input should produce empty result"
        );
    }

    #[test]
    fn test_escape_fts5_query_mixed_whitespace() {
        let result = escape_fts5_query("  hello   world  ");
        assert_eq!(
            result, "\"hello\" OR \"world\"",
            "Extra whitespace should be normalized"
        );
    }

    #[test]
    fn test_escape_fts5_query_only_quotes() {
        let result = escape_fts5_query("\"\"\"");
        assert_eq!(
            result, "",
            "String with only quotes should produce empty result after filtering"
        );
    }

    #[test]
    fn test_escape_fts5_query_word_with_quotes() {
        let result = escape_fts5_query("hel\"lo wor\"ld");
        assert_eq!(
            result, "\"hello\" OR \"world\"",
            "Quotes within words should be removed"
        );
    }

    #[tokio::test]
    async fn test_memory_manager_add_and_retrieve() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");

        // Create the memories table
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                embedding BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                importance REAL DEFAULT 0.5,
                character_id TEXT NOT NULL DEFAULT 'default',
                tier TEXT NOT NULL DEFAULT 'ephemeral',
                consolidated_from TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("Failed to create memories table");

        let manager = MemoryManager::new(pool);

        // Add a memory
        let content = "Test memory content";
        let char_id = "test_char";
        manager
            .add_memory(content, char_id)
            .await
            .expect("Failed to add memory");

        // Retrieve all memories for this character
        let memories = manager
            .get_all_memory_contents(char_id)
            .await
            .expect("Failed to retrieve memories");

        assert_eq!(
            memories.len(),
            1,
            "Should have exactly one memory after adding one"
        );
        assert_eq!(
            memories[0], content,
            "Retrieved memory content should match what was added"
        );
    }

    #[tokio::test]
    async fn test_memory_manager_character_isolation() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");

        // Create the memories table
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                embedding BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                importance REAL DEFAULT 0.5,
                character_id TEXT NOT NULL DEFAULT 'default',
                tier TEXT NOT NULL DEFAULT 'ephemeral',
                consolidated_from TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("Failed to create memories table");

        let manager = MemoryManager::new(pool);

        // Add memories for different characters
        manager
            .add_memory("Alice's memory", "alice")
            .await
            .expect("Failed to add Alice's memory");
        manager
            .add_memory("Bob's memory", "bob")
            .await
            .expect("Failed to add Bob's memory");

        // Retrieve memories for Alice
        let alice_memories = manager
            .get_all_memory_contents("alice")
            .await
            .expect("Failed to retrieve Alice's memories");

        // Retrieve memories for Bob
        let bob_memories = manager
            .get_all_memory_contents("bob")
            .await
            .expect("Failed to retrieve Bob's memories");

        assert_eq!(
            alice_memories.len(),
            1,
            "Alice should have exactly one memory"
        );
        assert_eq!(
            bob_memories.len(),
            1,
            "Bob should have exactly one memory"
        );
        assert_eq!(alice_memories[0], "Alice's memory");
        assert_eq!(bob_memories[0], "Bob's memory");
    }

    #[tokio::test]
    async fn test_memory_manager_empty_character() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");

        // Create the memories table
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                embedding BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                importance REAL DEFAULT 0.5,
                character_id TEXT NOT NULL DEFAULT 'default',
                tier TEXT NOT NULL DEFAULT 'ephemeral',
                consolidated_from TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("Failed to create memories table");

        let manager = MemoryManager::new(pool);

        // Retrieve memories for a character with no memories
        let memories = manager
            .get_all_memory_contents("nonexistent_char")
            .await
            .expect("Failed to retrieve memories");

        assert_eq!(
            memories.len(),
            0,
            "Should return empty list for character with no memories"
        );
    }

    #[tokio::test]
    async fn test_memory_manager_multiple_memories() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");

        // Create the memories table
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                embedding BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                importance REAL DEFAULT 0.5,
                character_id TEXT NOT NULL DEFAULT 'default',
                tier TEXT NOT NULL DEFAULT 'ephemeral',
                consolidated_from TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("Failed to create memories table");

        let manager = MemoryManager::new(pool);

        let char_id = "test_char";

        // Add multiple distinct memories (with unique content to avoid deduplication)
        let memories_to_add = vec![
            "The user likes to play chess on weekends",
            "The user works as a software engineer in San Francisco",
            "The user has a cat named Whiskers",
            "The user enjoys reading science fiction novels",
            "The user prefers coffee over tea in the morning",
        ];

        for content in &memories_to_add {
            manager
                .add_memory(content, char_id)
                .await
                .expect("Failed to add memory");
        }

        // Retrieve all memories
        let memories = manager
            .get_all_memory_contents(char_id)
            .await
            .expect("Failed to retrieve memories");

        assert_eq!(
            memories.len(),
            5,
            "Should have exactly 5 distinct memories after adding 5"
        );
    }
}
