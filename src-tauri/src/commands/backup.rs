use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Row, SqlitePool};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use crate::ai::context::AIOrchestrator;
use tauri::AppHandle;
use tauri::Manager;
use tauri::State;
use zip::write::SimpleFileOptions;

/// All JSON config filenames we back up.
const CONFIG_FILES: &[&str] = &[
    "llm_config.json",
    "tts_config.json",
    "stt_config.json",
    "vision_config.json",
    "imagegen_config.json",
    "mcp_servers.json",
    "telegram_config.json",
    "jailbreak_prompt.json",
    "proactive_enabled.json",
    "emotion_state.json",
    "context_settings.json",
    "current_conversation_id.json",
];

// ── Types ────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: String,
    pub created_at: String,
    pub app_version: String,
}

#[derive(Debug, Serialize)]
pub struct BackupStats {
    pub memories: i64,
    pub conversations: i64,
    pub messages: i64,
    pub configs: usize,
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub path: String,
    pub size_bytes: u64,
    pub stats: BackupStats,
}

#[derive(Debug, Serialize)]
pub struct ImportPreview {
    pub manifest: BackupManifest,
    pub has_database: bool,
    pub has_configs: bool,
    pub config_files: Vec<String>,
    pub stats: BackupStats,
}

#[derive(Debug, Deserialize)]
pub struct ImportOptions {
    pub import_database: bool,
    pub import_configs: bool,
    pub conflict_strategy: String, // "skip" | "overwrite"
    pub target_character_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub imported_memories: i64,
    pub imported_conversations: i64,
    pub imported_configs: usize,
    pub characters_json: Option<String>,
    pub debug_log: Vec<String>,
}

// ── Helpers ──────────────────────────────────────────

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {}", e))
}

fn db_path(app_data: &Path) -> PathBuf {
    app_data.join("kokoro.db")
}

pub fn db_path_pub(app_data: &Path) -> PathBuf {
    db_path(app_data)
}

/// Validate that a filename from a ZIP entry is safe (no path traversal).
/// RAII 临时目录守卫：离开作用域时自动删除目录，确保错误路径也能清理
struct TempDirGuard(std::path::PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn is_safe_filename(name: &str) -> bool {
    !name.contains("..") && !name.starts_with('/') && !name.starts_with('\\') && !name.contains(':')
}

/// Open a read-only sqlx pool to a given DB file.
async fn open_readonly_pool(path: &Path) -> Result<SqlitePool, String> {
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let options = SqliteConnectOptions::from_str(&url)
        .map_err(|e| format!("Invalid DB path: {}", e))?
        .read_only(true);
    SqlitePool::connect_with(options)
        .await
        .map_err(|e| format!("Failed to open DB: {}", e))
}

/// 受限的表名枚举，防止 count_rows 被传入任意字符串
enum CountTable {
    Memories,
    Conversations,
    ConversationMessages,
}

impl CountTable {
    fn as_sql(&self) -> &'static str {
        match self {
            CountTable::Memories => "SELECT COUNT(*) as cnt FROM memories",
            CountTable::Conversations => "SELECT COUNT(*) as cnt FROM conversations",
            CountTable::ConversationMessages => "SELECT COUNT(*) as cnt FROM conversation_messages",
        }
    }
}

/// Count rows in a table via sqlx. Returns 0 on any error.
async fn count_rows(pool: &SqlitePool, table: CountTable) -> i64 {
    sqlx::query(table.as_sql())
        .fetch_one(pool)
        .await
        .and_then(|row| row.try_get::<i64, _>("cnt"))
        .unwrap_or(0)
}

async fn gather_stats(path: &Path) -> BackupStats {
    let pool = match open_readonly_pool(path).await {
        Ok(p) => p,
        Err(_) => {
            return BackupStats {
                memories: 0,
                conversations: 0,
                messages: 0,
                configs: 0,
            }
        }
    };
    let memories = count_rows(&pool, CountTable::Memories).await;
    let conversations = count_rows(&pool, CountTable::Conversations).await;
    let messages = count_rows(&pool, CountTable::ConversationMessages).await;
    pool.close().await;
    BackupStats {
        memories,
        conversations,
        messages,
        configs: 0,
    }
}

// ── Commands ─────────────────────────────────────────

#[tauri::command]
pub async fn export_data(app: AppHandle, export_path: String, characters_json: Option<String>) -> Result<ExportResult, String> {
    let app_data = app_data_dir(&app)?;
    let db = db_path(&app_data);

    let out_path = PathBuf::from(&export_path);
    let file =
        fs::File::create(&out_path).map_err(|e| format!("Failed to create export file: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // 1. Gather stats before copying
    let mut stats = gather_stats(&db).await;
    let mut config_count: usize = 0;

    // 2. manifest.json
    let manifest = BackupManifest {
        version: "1".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("Serialize error: {}", e))?;
    zip.start_file("manifest.json", options)
        .map_err(|e| format!("ZIP error: {}", e))?;
    zip.write_all(manifest_json.as_bytes())
        .map_err(|e| format!("ZIP write error: {}", e))?;

    // 3. kokoro.db — fs::copy to temp to avoid WAL lock issues
    if db.exists() {
        let tmp_db = app_data.join("kokoro_backup_tmp.db");
        fs::copy(&db, &tmp_db).map_err(|e| format!("Failed to copy DB: {}", e))?;
        // Also copy WAL/SHM if present so the copy is consistent
        let wal = db.with_extension("db-wal");
        let shm = db.with_extension("db-shm");
        if wal.exists() {
            let _ = fs::copy(&wal, tmp_db.with_extension("db-wal"));
        }
        if shm.exists() {
            let _ = fs::copy(&shm, tmp_db.with_extension("db-shm"));
        }

        // Checkpoint the temp copy to merge WAL into main DB file
        {
            let url = format!(
                "sqlite://{}",
                tmp_db.to_string_lossy().replace('\\', "/")
            );
            if let Ok(opts) = SqliteConnectOptions::from_str(&url) {
                if let Ok(pool) = SqlitePool::connect_with(opts).await {
                    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                        .execute(&pool)
                        .await;
                    pool.close().await;
                }
            }
        }

        let mut db_bytes = Vec::new();
        fs::File::open(&tmp_db)
            .map_err(|e| format!("Failed to open tmp DB: {}", e))?
            .read_to_end(&mut db_bytes)
            .map_err(|e| format!("Failed to read tmp DB: {}", e))?;

        // Clean up temp files
        let _ = fs::remove_file(&tmp_db);
        let _ = fs::remove_file(tmp_db.with_extension("db-wal"));
        let _ = fs::remove_file(tmp_db.with_extension("db-shm"));

        zip.start_file("kokoro.db", options)
            .map_err(|e| format!("ZIP error: {}", e))?;
        zip.write_all(&db_bytes)
            .map_err(|e| format!("ZIP write error: {}", e))?;
    }

    // 4. characters.json (from IndexedDB, serialized by frontend)
    if let Some(ref chars) = characters_json {
        zip.start_file("characters.json", options)
            .map_err(|e| format!("ZIP error: {}", e))?;
        zip.write_all(chars.as_bytes())
            .map_err(|e| format!("ZIP write error: {}", e))?;
    }

    // 5. configs/
    for name in CONFIG_FILES {
        let cfg_path = app_data.join(name);
        if cfg_path.exists() {
            if let Ok(content) = fs::read_to_string(&cfg_path) {
                let entry = format!("configs/{}", name);
                zip.start_file(&entry, options)
                    .map_err(|e| format!("ZIP error: {}", e))?;
                zip.write_all(content.as_bytes())
                    .map_err(|e| format!("ZIP write error: {}", e))?;
                config_count += 1;
            }
        }
    }

    zip.finish().map_err(|e| format!("ZIP finish error: {}", e))?;

    let size_bytes = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    stats.configs = config_count;

    println!(
        "[Backup] Exported to {} ({} bytes, {} memories, {} conversations, {} configs)",
        export_path, size_bytes, stats.memories, stats.conversations, stats.configs
    );

    Ok(ExportResult {
        path: export_path,
        size_bytes,
        stats,
    })
}

/// 核心导出逻辑，供自动备份模块复用（不需要 AppHandle）
pub async fn export_data_to_path(
    app_data: &Path,
    out_path: &Path,
    characters_json: Option<String>,
) -> Result<ExportResult, String> {
    let db = db_path(app_data);

    let file = fs::File::create(out_path)
        .map_err(|e| format!("Failed to create export file: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut stats = gather_stats(&db).await;
    let mut config_count: usize = 0;

    let manifest = BackupManifest {
        version: "1".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("Serialize error: {}", e))?;
    zip.start_file("manifest.json", options)
        .map_err(|e| format!("ZIP error: {}", e))?;
    zip.write_all(manifest_json.as_bytes())
        .map_err(|e| format!("ZIP write error: {}", e))?;

    if db.exists() {
        let tmp_db = app_data.join("kokoro_autobackup_tmp.db");
        fs::copy(&db, &tmp_db).map_err(|e| format!("Failed to copy DB: {}", e))?;
        let wal = db.with_extension("db-wal");
        let shm = db.with_extension("db-shm");
        if wal.exists() { let _ = fs::copy(&wal, tmp_db.with_extension("db-wal")); }
        if shm.exists() { let _ = fs::copy(&shm, tmp_db.with_extension("db-shm")); }
        {
            let url = format!("sqlite://{}", tmp_db.to_string_lossy().replace('\\', "/"));
            if let Ok(opts) = SqliteConnectOptions::from_str(&url) {
                if let Ok(pool) = SqlitePool::connect_with(opts).await {
                    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await;
                    pool.close().await;
                }
            }
        }
        let mut db_bytes = Vec::new();
        fs::File::open(&tmp_db)
            .map_err(|e| format!("Failed to open tmp DB: {}", e))?
            .read_to_end(&mut db_bytes)
            .map_err(|e| format!("Failed to read tmp DB: {}", e))?;
        let _ = fs::remove_file(&tmp_db);
        let _ = fs::remove_file(tmp_db.with_extension("db-wal"));
        let _ = fs::remove_file(tmp_db.with_extension("db-shm"));
        zip.start_file("kokoro.db", options)
            .map_err(|e| format!("ZIP error: {}", e))?;
        zip.write_all(&db_bytes)
            .map_err(|e| format!("ZIP write error: {}", e))?;
    }

    if let Some(ref chars) = characters_json {
        zip.start_file("characters.json", options)
            .map_err(|e| format!("ZIP error: {}", e))?;
        zip.write_all(chars.as_bytes())
            .map_err(|e| format!("ZIP write error: {}", e))?;
    }

    for name in CONFIG_FILES {
        let cfg_path = app_data.join(name);
        if cfg_path.exists() {
            if let Ok(content) = fs::read_to_string(&cfg_path) {
                let entry = format!("configs/{}", name);
                zip.start_file(&entry, options)
                    .map_err(|e| format!("ZIP error: {}", e))?;
                zip.write_all(content.as_bytes())
                    .map_err(|e| format!("ZIP write error: {}", e))?;
                config_count += 1;
            }
        }
    }

    zip.finish().map_err(|e| format!("ZIP finish error: {}", e))?;

    let size_bytes = fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
    stats.configs = config_count;

    Ok(ExportResult {
        path: out_path.to_string_lossy().to_string(),
        size_bytes,
        stats,
    })
}

#[tauri::command]
pub async fn preview_import(file_path: String) -> Result<ImportPreview, String> {
    let path = PathBuf::from(&file_path);
    let file = fs::File::open(&path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Invalid ZIP archive: {}", e))?;

    // Read manifest
    let manifest: BackupManifest = {
        let mut entry = archive
            .by_name("manifest.json")
            .map_err(|_| "Missing manifest.json in backup file".to_string())?;
        let mut buf = String::new();
        entry
            .read_to_string(&mut buf)
            .map_err(|e| format!("Failed to read manifest: {}", e))?;
        serde_json::from_str(&buf).map_err(|e| format!("Invalid manifest: {}", e))?
    };

    let has_database = archive.by_name("kokoro.db").is_ok();

    // Collect config file names
    let mut config_files: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            let name = entry.name().to_string();
            if name.starts_with("configs/") && name.len() > 8 {
                config_files.push(name.trim_start_matches("configs/").to_string());
            }
        }
    }
    let has_configs = !config_files.is_empty();

    // If DB present, extract to temp and count rows
    let stats = if has_database {
        let tmp_dir_path = std::env::temp_dir().join("kokoro_import_preview");
        fs::create_dir_all(&tmp_dir_path)
            .map_err(|e| format!("Failed to create temp dir: {}", e))?;
        // RAII 守卫：无论成功还是失败都会自动清理临时目录
        let _tmp_guard = TempDirGuard(tmp_dir_path.clone());
        let tmp_db = tmp_dir_path.join("preview.db");

        {
            let mut entry = archive
                .by_name("kokoro.db")
                .map_err(|e| format!("Failed to read DB from ZIP: {}", e))?;
            let mut out = fs::File::create(&tmp_db)
                .map_err(|e| format!("Failed to create temp DB: {}", e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed to extract DB: {}", e))?;
        }

        gather_stats(&tmp_db).await
    } else {
        BackupStats {
            memories: 0,
            conversations: 0,
            messages: 0,
            configs: 0,
        }
    };

    Ok(ImportPreview {
        manifest,
        has_database,
        has_configs,
        config_files,
        stats,
    })
}

#[tauri::command]
pub async fn import_data(
    app: AppHandle,
    orchestrator: State<'_, AIOrchestrator>,
    file_path: String,
    options: ImportOptions,
) -> Result<ImportResult, String> {
    let app_data = app_data_dir(&app)?;

    // Phase 1: Extract everything from ZIP synchronously (ZipFile is !Send)
    let tmp_dir = std::env::temp_dir().join("kokoro_import");
    fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    // RAII 守卫：无论成功还是失败都会自动清理临时目录
    let _tmp_guard = TempDirGuard(tmp_dir.clone());

    let mut has_db = false;
    let mut extracted_configs: Vec<(String, String)> = Vec::new();
    let mut characters_json: Option<String> = None;

    {
        let path = PathBuf::from(&file_path);
        let file = fs::File::open(&path).map_err(|e| format!("Failed to open file: {}", e))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Invalid ZIP archive: {}", e))?;

        // Extract DB if requested — always to a temp file to avoid clobbering the live DB
        if options.import_database && archive.by_name("kokoro.db").is_ok() {
            has_db = true;
            let mut entry = archive
                .by_name("kokoro.db")
                .map_err(|e| format!("Failed to read DB: {}", e))?;
            let mut out = fs::File::create(tmp_dir.join("import.db"))
                .map_err(|e| format!("Failed to write DB: {}", e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed to extract DB: {}", e))?;
        }

        // Extract characters.json if present
        if let Ok(mut entry) = archive.by_name("characters.json") {
            let mut content = String::new();
            if entry.read_to_string(&mut content).is_ok() {
                characters_json = Some(content);
            }
        }

        // Extract configs into memory
        if options.import_configs {
            for i in 0..archive.len() {
                let mut entry = archive
                    .by_index(i)
                    .map_err(|e| format!("ZIP entry error: {}", e))?;
                let name = entry.name().to_string();
                if !name.starts_with("configs/") || name.len() <= 8 {
                    continue;
                }
                let filename = name.trim_start_matches("configs/").to_string();
                if !is_safe_filename(&filename) {
                    continue;
                }
                let target = app_data.join(&filename);

                if options.conflict_strategy == "skip" && target.exists() {
                    continue;
                }

                let mut content = String::new();
                entry
                    .read_to_string(&mut content)
                    .map_err(|e| format!("Failed to read config {}: {}", filename, e))?;
                extracted_configs.push((filename, content));
            }
        }
    }
    // archive is dropped here — safe to .await below

    // Phase 2: Async DB operations
    let mut result = ImportResult {
        imported_memories: 0,
        imported_conversations: 0,
        imported_configs: 0,
        characters_json,
        debug_log: Vec::new(),
    };

    if has_db {
        let tmp_db = tmp_dir.join("import.db");
        // 必须用同一个连接：ATTACH DATABASE 是连接级别的操作
        let mut conn = orchestrator.db.acquire().await
            .map_err(|e| format!("Failed to acquire DB connection: {}", e))?;

        let attach_path = tmp_db.to_string_lossy().replace('\\', "/");
        println!("[Backup] Attaching import DB from: {}", attach_path);
        // 使用参数绑定防止 SQL 注入
        sqlx::query("ATTACH DATABASE ? AS import_db")
            .bind(&attach_path)
            .execute(&mut *conn)
            .await
            .map_err(|e| format!("ATTACH failed: {}", e))?;

        // 验证 ATTACH 成功，能读到数据
        let import_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_db.memories")
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| format!("Failed to count import_db.memories: {}", e))?;
        println!("[Backup] import_db.memories count: {}", import_count);
        result.debug_log.push(format!("import_db.memories count: {}", import_count));

        // 打印备份里实际的 character_id 分布
        let char_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT character_id FROM import_db.memories"
        )
        .fetch_all(&mut *conn)
        .await
        .unwrap_or_default();
        println!("[Backup] import_db.memories character_ids: {:?}", char_ids);
        result.debug_log.push(format!("import_db character_ids: {:?}", char_ids));
        result.debug_log.push(format!("target_character_id: {:?}", options.target_character_id));

        if options.conflict_strategy == "overwrite" {
            // 先删除 FTS 触发器，避免批量操作时触发器访问损坏的 FTS 索引
            sqlx::query("DROP TRIGGER IF EXISTS memories_ai").execute(&mut *conn).await.ok();
            sqlx::query("DROP TRIGGER IF EXISTS memories_ad").execute(&mut *conn).await.ok();
            sqlx::query("DROP TRIGGER IF EXISTS memories_au").execute(&mut *conn).await.ok();

            sqlx::query("DELETE FROM conversation_messages").execute(&mut *conn).await
                .map_err(|e| format!("DELETE conversation_messages failed: {}", e))?;
            sqlx::query("DELETE FROM conversations").execute(&mut *conn).await
                .map_err(|e| format!("DELETE conversations failed: {}", e))?;
            sqlx::query("DELETE FROM memories").execute(&mut *conn).await
                .map_err(|e| format!("DELETE memories failed: {}", e))?;

            let r = sqlx::query("INSERT INTO memories SELECT * FROM import_db.memories")
                .execute(&mut *conn).await
                .map_err(|e| format!("INSERT memories failed: {}", e))?;
            result.imported_memories = r.rows_affected() as i64;
            println!("[Backup] Inserted {} memories", result.imported_memories);
            result.debug_log.push(format!("inserted memories: {}", result.imported_memories));

            let r = sqlx::query("INSERT INTO conversations SELECT * FROM import_db.conversations")
                .execute(&mut *conn).await
                .map_err(|e| format!("INSERT conversations failed: {}", e))?;
            result.imported_conversations = r.rows_affected() as i64;
            result.debug_log.push(format!("inserted conversations: {}", result.imported_conversations));

            sqlx::query("INSERT INTO conversation_messages SELECT * FROM import_db.conversation_messages")
                .execute(&mut *conn).await
                .map_err(|e| format!("INSERT conversation_messages failed: {}", e))?;

            // 重建 FTS 索引并恢复触发器
            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')").execute(&mut *conn).await.ok();
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content); END").execute(&mut *conn).await.ok();
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content); END").execute(&mut *conn).await.ok();
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content); INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content); END").execute(&mut *conn).await.ok();
        } else {
            // skip 模式：先重建 FTS 以防损坏
            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')").execute(&mut *conn).await.ok();

            let r = sqlx::query("INSERT OR IGNORE INTO memories SELECT * FROM import_db.memories")
                .execute(&mut *conn).await
                .map_err(|e| format!("INSERT OR IGNORE memories failed: {}", e))?;
            result.imported_memories = r.rows_affected() as i64;
            println!("[Backup] Inserted {} memories (skip mode)", result.imported_memories);
            result.debug_log.push(format!("inserted memories (skip): {}", result.imported_memories));

            let r = sqlx::query("INSERT OR IGNORE INTO conversations SELECT * FROM import_db.conversations")
                .execute(&mut *conn).await
                .map_err(|e| format!("INSERT OR IGNORE conversations failed: {}", e))?;
            result.imported_conversations = r.rows_affected() as i64;
            result.debug_log.push(format!("inserted conversations (skip): {}", result.imported_conversations));

            sqlx::query("INSERT OR IGNORE INTO conversation_messages SELECT * FROM import_db.conversation_messages")
                .execute(&mut *conn).await
                .map_err(|e| format!("INSERT OR IGNORE conversation_messages failed: {}", e))?;

            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')").execute(&mut *conn).await.ok();
        }

        sqlx::query("DETACH DATABASE import_db").execute(&mut *conn).await
            .map_err(|e| format!("DETACH failed: {}", e))?;

        // 如果指定了目标 character_id，把所有导入的记忆和对话重映射过去
        if let Some(ref target_id) = options.target_character_id {
            println!("[Backup] Remapping character_id to '{}'", target_id);
            result.debug_log.push(format!("remapping all character_ids to: {}", target_id));
            let r = sqlx::query("UPDATE memories SET character_id = ? WHERE character_id != ?")
                .bind(target_id)
                .bind(target_id)
                .execute(&mut *conn).await.ok();
            result.debug_log.push(format!("memories remapped: {}", r.map(|r| r.rows_affected()).unwrap_or(0)));
            sqlx::query("UPDATE conversations SET character_id = ? WHERE character_id != ?")
                .bind(target_id)
                .bind(target_id)
                .execute(&mut *conn).await.ok();
        } else {
            result.debug_log.push("no target_character_id — remap skipped".to_string());
        }

        // 持久化 target_character_id，确保重启后后端能正确恢复
        if let Some(ref target_id) = options.target_character_id {
            crate::ai::context::AIOrchestrator::persist_active_character_id(target_id);
            result.debug_log.push(format!("persisted active_character_id: {}", target_id));
        }

        drop(conn);
        // tmp_db 由 _tmp_guard 在函数结束时自动清理，无需手动删除
    }

    // Phase 3: Write config files
    for (filename, content) in &extracted_configs {
        let target = app_data.join(filename);
        fs::write(&target, content)
            .map_err(|e| format!("Failed to write config {}: {}", filename, e))?;
        result.imported_configs += 1;
    }

    // tmp_dir 由 _tmp_guard 自动清理

    println!(
        "[Backup] Imported: {} memories, {} conversations, {} configs",
        result.imported_memories, result.imported_conversations, result.imported_configs
    );

    Ok(result)
}
