//! Auto Backup — 定时自动备份记忆数据到指定目录

use crate::commands::backup::export_data_to_path;
use crate::error::KokoroError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const CONFIG_FILE: &str = "auto_backup_config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoBackupConfig {
    /// 是否启用自动备份
    pub enabled: bool,
    /// 备份文件保存目录
    pub backup_dir: String,
    /// 备份间隔（天），1-7
    pub interval_days: u32,
    /// 是否自动清理旧备份
    pub auto_cleanup: bool,
    /// 保留最近 N 天内的备份，超出则删除
    pub keep_days: u32,
}

impl Default for AutoBackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backup_dir: String::new(),
            interval_days: 1,
            auto_cleanup: false,
            keep_days: 30,
        }
    }
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, KokoroError> {
    app.path()
        .app_data_dir()
        .map_err(|e| KokoroError::Internal(format!("Failed to resolve app data dir: {}", e)))
}

fn config_path(app_data: &Path) -> PathBuf {
    app_data.join(CONFIG_FILE)
}

fn load_config(app_data: &Path) -> AutoBackupConfig {
    let path = config_path(app_data);
    if let Ok(content) = fs::read_to_string(&path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        AutoBackupConfig::default()
    }
}

fn save_config_to_disk(app_data: &Path, config: &AutoBackupConfig) -> Result<(), KokoroError> {
    let path = config_path(app_data);
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| KokoroError::Config(format!("Serialize error: {}", e)))?;
    fs::write(&path, json).map_err(KokoroError::from)
}

// ── IPC Commands ──────────────────────────────────

#[tauri::command]
pub async fn get_auto_backup_config(app: AppHandle) -> Result<AutoBackupConfig, KokoroError> {
    let app_data = app_data_dir(&app)?;
    Ok(load_config(&app_data))
}

#[tauri::command]
pub async fn save_auto_backup_config(
    app: AppHandle,
    config: AutoBackupConfig,
) -> Result<(), KokoroError> {
    let app_data = app_data_dir(&app)?;
    save_config_to_disk(&app_data, &config)
}

#[tauri::command]
pub async fn run_auto_backup_now(app: AppHandle) -> Result<String, KokoroError> {
    let app_data = app_data_dir(&app)?;
    let config = load_config(&app_data);
    if config.backup_dir.is_empty() {
        return Err(KokoroError::Validation("Backup directory not set".to_string()));
    }
    do_backup(&app_data, &config).await
}

pub async fn do_backup(app_data: &Path, config: &AutoBackupConfig) -> Result<String, KokoroError> {
    let dir = PathBuf::from(&config.backup_dir);
    fs::create_dir_all(&dir).map_err(KokoroError::from)?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("kokoro-auto-{}.kokoro", timestamp);
    let out_path = dir.join(&filename);
    export_data_to_path(app_data, &out_path, None).await?;
    println!("[AutoBackup] Backup saved to {}", out_path.display());
    if config.auto_cleanup && config.keep_days > 0 {
        cleanup_old_backups(&dir, config.keep_days);
    }
    Ok(out_path.to_string_lossy().to_string())
}

/// 删除目录中超过 keep_days 天的 .kokoro 备份文件
fn cleanup_old_backups(dir: &PathBuf, keep_days: u32) {
    let cutoff = chrono::Local::now()
        - chrono::Duration::days(keep_days as i64);
    let cutoff_ts = cutoff.timestamp();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("kokoro") {
            continue;
        }
        // 只清理自动备份文件（文件名以 kokoro-auto- 开头）
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !fname.starts_with("kokoro-auto-") {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                let secs = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(i64::MAX);
                if secs < cutoff_ts {
                    let _ = fs::remove_file(&path);
                    println!("[AutoBackup] Removed old backup: {}", path.display());
                }
            }
        }
    }
}

/// 由 heartbeat 调用：检查是否需要执行自动备份
pub async fn check_and_run(app_handle: &AppHandle) {
    let app_data = match app_handle.path().app_data_dir() {
        Ok(p) => p,
        Err(_) => return,
    };
    let config = load_config(&app_data);
    if !config.enabled || config.backup_dir.is_empty() {
        return;
    }

    // 读取上次备份时间戳
    let last_ts_path = app_data.join("auto_backup_last.txt");
    let last_ts: i64 = fs::read_to_string(&last_ts_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let now_ts = chrono::Utc::now().timestamp();
    let interval_secs = config.interval_days as i64 * 86400;

    if now_ts - last_ts < interval_secs {
        return;
    }

    match do_backup(&app_data, &config).await {
        Ok(path) => {
            println!("[AutoBackup] Auto backup completed: {}", path);
            let _ = fs::write(&last_ts_path, now_ts.to_string());
        }
        Err(e) => {
            eprintln!("[AutoBackup] Auto backup failed: {}", e);
        }
    }
}
