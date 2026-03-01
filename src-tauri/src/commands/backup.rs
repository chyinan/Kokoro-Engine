use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Row, SqlitePool};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tauri::AppHandle;
use tauri::Manager;
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
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub imported_memories: i64,
    pub imported_conversations: i64,
    pub imported_configs: usize,
}

// ── Helpers ──────────────────────────────────────────

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {}", e))
}

fn db_path(_app_data: &Path) -> PathBuf {
    // The DB uses a relative path "sqlite://kokoro.db" (see lib.rs),
    // so it lives in the current working directory, not app_data_dir.
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("kokoro.db")
}

/// Validate that a filename from a ZIP entry is safe (no path traversal).
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

/// Count rows in a table via sqlx. Returns 0 on any error.
async fn count_rows(pool: &SqlitePool, table: &str) -> i64 {
    // table names are hardcoded constants, safe to interpolate
    let query = format!("SELECT COUNT(*) as cnt FROM {}", table);
    sqlx::query(&query)
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
    let memories = count_rows(&pool, "memories").await;
    let conversations = count_rows(&pool, "conversations").await;
    let messages = count_rows(&pool, "conversation_messages").await;
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
pub async fn export_data(app: AppHandle, export_path: String) -> Result<ExportResult, String> {
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

    // 4. configs/
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
        let tmp_dir = std::env::temp_dir().join("kokoro_import_preview");
        let _ = fs::create_dir_all(&tmp_dir);
        let tmp_db = tmp_dir.join("preview.db");

        {
            let mut entry = archive
                .by_name("kokoro.db")
                .map_err(|e| format!("Failed to read DB from ZIP: {}", e))?;
            let mut out = fs::File::create(&tmp_db)
                .map_err(|e| format!("Failed to create temp DB: {}", e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed to extract DB: {}", e))?;
        }

        let s = gather_stats(&tmp_db).await;
        let _ = fs::remove_file(&tmp_db);
        let _ = fs::remove_dir(&tmp_dir);
        s
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
    file_path: String,
    options: ImportOptions,
) -> Result<ImportResult, String> {
    let app_data = app_data_dir(&app)?;
    let target_db = db_path(&app_data);

    // Phase 1: Extract everything from ZIP synchronously (ZipFile is !Send)
    let tmp_dir = std::env::temp_dir().join("kokoro_import");
    let _ = fs::create_dir_all(&tmp_dir);

    let mut has_db = false;
    let mut extracted_configs: Vec<(String, String)> = Vec::new();

    {
        let path = PathBuf::from(&file_path);
        let file = fs::File::open(&path).map_err(|e| format!("Failed to open file: {}", e))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("Invalid ZIP archive: {}", e))?;

        // Extract DB if requested
        if options.import_database && archive.by_name("kokoro.db").is_ok() {
            has_db = true;
            let extract_target = if options.conflict_strategy == "overwrite" {
                target_db.clone()
            } else {
                tmp_dir.join("import.db")
            };
            let mut entry = archive
                .by_name("kokoro.db")
                .map_err(|e| format!("Failed to read DB: {}", e))?;
            let mut out = fs::File::create(&extract_target)
                .map_err(|e| format!("Failed to write DB: {}", e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("Failed to extract DB: {}", e))?;
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
    };

    if has_db {
        if options.conflict_strategy == "overwrite" {
            let stats = gather_stats(&target_db).await;
            result.imported_memories = stats.memories;
            result.imported_conversations = stats.conversations;
        } else {
            let tmp_db = tmp_dir.join("import.db");

            let url = format!(
                "sqlite://{}",
                target_db.to_string_lossy().replace('\\', "/")
            );
            let pool_opts = SqliteConnectOptions::from_str(&url)
                .map_err(|e| format!("Invalid DB path: {}", e))?;
            let pool = SqlitePool::connect_with(pool_opts)
                .await
                .map_err(|e| format!("Failed to open target DB: {}", e))?;

            let attach_path = tmp_db.to_string_lossy().replace('\\', "/").replace('\'', "''");
            sqlx::query(&format!(
                "ATTACH DATABASE '{}' AS import_db",
                attach_path
            ))
            .execute(&pool)
            .await
            .map_err(|e| format!("ATTACH failed: {}", e))?;

            if let Ok(r) = sqlx::query(
                "INSERT OR IGNORE INTO memories SELECT * FROM import_db.memories",
            )
            .execute(&pool)
            .await
            {
                result.imported_memories = r.rows_affected() as i64;
            }

            if let Ok(r) = sqlx::query(
                "INSERT OR IGNORE INTO conversations SELECT * FROM import_db.conversations",
            )
            .execute(&pool)
            .await
            {
                result.imported_conversations = r.rows_affected() as i64;
            }

            let _ = sqlx::query(
                "INSERT OR IGNORE INTO conversation_messages SELECT * FROM import_db.conversation_messages",
            )
            .execute(&pool)
            .await;

            let _ = sqlx::query(
                "INSERT OR IGNORE INTO characters SELECT * FROM import_db.characters",
            )
            .execute(&pool)
            .await;

            let _ = sqlx::query("DETACH DATABASE import_db")
                .execute(&pool)
                .await;

            pool.close().await;

            let _ = fs::remove_file(&tmp_db);
        }
    }

    // Phase 3: Write config files
    for (filename, content) in &extracted_configs {
        let target = app_data.join(filename);
        fs::write(&target, content)
            .map_err(|e| format!("Failed to write config {}: {}", filename, e))?;
        result.imported_configs += 1;
    }

    let _ = fs::remove_dir(&tmp_dir);

    println!(
        "[Backup] Imported: {} memories, {} conversations, {} configs",
        result.imported_memories, result.imported_conversations, result.imported_configs
    );

    Ok(result)
}
