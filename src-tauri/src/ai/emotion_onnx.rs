// pattern: Imperative Shell & Clean Domain Service
use anyhow::Result;
use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Instant;
use tokenizers::Tokenizer;

pub const MODEL_REPO: &str = "Johnson8187/Chinese-Emotion-Small";
pub const MODEL_PAGE_URL: &str = "https://huggingface.co/Johnson8187/Chinese-Emotion-Small";
pub const MODEL_DIR_NAME: &str = "models--Johnson8187--Chinese-Emotion-Small";
pub const MODEL_REF_NAME: &str = "main";
pub const MODEL_FALLBACK_ENDPOINT: &str = "https://hf-mirror.com";

pub const EMOTION_LABELS: [(&str, &str); 8] = [
    ("neutral", "平淡語氣"),
    ("caring", "關切語調"),
    ("happy", "開心語調"),
    ("angry", "憤怒語調"),
    ("sad", "悲傷語調"),
    ("questioning", "疑問語調"),
    ("surprised", "驚奇語調"),
    ("disgusted", "厭惡語調"),
];

const REQUIRED_FILES: &[&str] = &[
    "model.onnx",
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmotionModelStatus {
    pub installed: bool,
    pub is_active: bool,
    pub repo_id: String,
    pub download_url: String,
    pub install_dir: String,
    pub model_path: String,
    pub required_files: Vec<String>,
    pub missing_files: Vec<String>,
    pub memory_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmotionInferenceProbability {
    pub label: String,
    pub label_zh: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmotionInferenceResult {
    pub dominant_emotion: String,
    pub label_zh: String,
    pub confidence: f32,
    pub probabilities: Vec<EmotionInferenceProbability>,
    pub mapped_cue: Option<String>,
    pub latency_ms: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmotionModelDownloadProgress {
    pub stage: String,
    pub message: String,
    pub current_file: String,
    pub file_index: usize,
    pub file_count: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

pub struct EmotionEngine {
    session: Session,
    tokenizer: Tokenizer,
}

fn engine_instance() -> &'static Arc<RwLock<Option<EmotionEngine>>> {
    static INSTANCE: OnceLock<Arc<RwLock<Option<EmotionEngine>>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(RwLock::new(None)))
}

static IS_ACTIVE: AtomicBool = AtomicBool::new(true);

fn default_model_cache_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("models")
}

fn default_model_repo_dir() -> PathBuf {
    default_model_cache_dir().join(MODEL_DIR_NAME)
}

fn default_model_snapshot_dir() -> PathBuf {
    default_model_repo_dir().join("snapshots").join(MODEL_REF_NAME)
}

fn required_model_files() -> Vec<&'static str> {
    REQUIRED_FILES.to_vec()
}

fn missing_required_model_files(snapshot_dir: &Path) -> Vec<String> {
    required_model_files()
        .into_iter()
        .filter(|file| !snapshot_dir.join(file).is_file())
        .map(str::to_string)
        .collect()
}

pub fn get_emotion_model_status() -> EmotionModelStatus {
    let snapshot_dir = default_model_snapshot_dir();
    let missing_files = missing_required_model_files(&snapshot_dir);
    let model_path = snapshot_dir.join("model.onnx");
    let installed = missing_files.is_empty();
    let is_active = IS_ACTIVE.load(Ordering::Relaxed) && installed;

    let memory_bytes = if installed {
        engine_instance()
            .read()
            .ok()
            .and_then(|guard| if guard.is_some() { Some(35 * 1024 * 1024) } else { None })
    } else {
        None
    };

    EmotionModelStatus {
        installed,
        is_active,
        repo_id: MODEL_REPO.to_string(),
        download_url: MODEL_PAGE_URL.to_string(),
        install_dir: snapshot_dir.to_string_lossy().into_owned(),
        model_path: model_path.to_string_lossy().into_owned(),
        required_files: required_model_files().into_iter().map(str::to_string).collect(),
        missing_files,
        memory_bytes,
    }
}

pub fn toggle_emotion_model_active(active: bool) -> Result<EmotionModelStatus, String> {
    let current_status = get_emotion_model_status();
    if active && !current_status.installed {
        return Err("Cannot activate emotion model: files not installed".to_string());
    }

    IS_ACTIVE.store(active, Ordering::Relaxed);
    if !active {
        // Drop session memory if turned off
        if let Ok(mut guard) = engine_instance().write() {
            *guard = None;
        }
    }
    Ok(get_emotion_model_status())
}

pub fn unload_emotion_engine() {
    if let Ok(mut guard) = engine_instance().write() {
        *guard = None;
    }
}

pub fn uninstall_emotion_model() -> Result<EmotionModelStatus, String> {
    // 1. Explicitly drop the active session to release Windows file handles (avoid OS Error 32)
    unload_emotion_engine();
    std::thread::yield_now();

    let repo_dir = default_model_repo_dir();
    if repo_dir.exists() {
        std::fs::remove_dir_all(&repo_dir).map_err(|e| format!("Failed to remove model directory: {}", e))?;
    }

    IS_ACTIVE.store(false, Ordering::Relaxed);
    Ok(get_emotion_model_status())
}

fn emotion_model_endpoint() -> String {
    std::env::var("HF_ENDPOINT")
        .unwrap_or_else(|_| "https://huggingface.co".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn emotion_model_endpoints() -> Vec<String> {
    let primary = emotion_model_endpoint();
    let fallback = MODEL_FALLBACK_ENDPOINT.trim_end_matches('/').to_string();
    if primary.eq_ignore_ascii_case(&fallback) {
        vec![primary]
    } else {
        vec![primary, fallback]
    }
}

fn emotion_model_file_url(endpoint: &str, file_name: &str) -> String {
    format!("{}/{}/resolve/{}/{}", endpoint, MODEL_REPO, MODEL_REF_NAME, file_name)
}

fn emotion_model_file_path(snapshot_dir: &Path, file_name: &str) -> Result<PathBuf, String> {
    let path = Path::new(file_name);
    if path.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("Invalid file path: {}", file_name));
    }
    Ok(snapshot_dir.join(path))
}

fn build_download_progress(
    stage: &str,
    message: String,
    current_file: String,
    file_index: usize,
    file_count: usize,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> EmotionModelDownloadProgress {
    EmotionModelDownloadProgress {
        stage: stage.to_string(),
        message,
        current_file,
        file_index,
        file_count,
        downloaded_bytes,
        total_bytes,
    }
}

pub fn open_emotion_model_dir() -> Result<String, String> {
    let snapshot_dir = default_model_snapshot_dir();
    std::fs::create_dir_all(&snapshot_dir)
        .map_err(|e| format!("Failed to create model directory: {}", e))?;

    let readme_path = snapshot_dir.join("README.txt");
    if !readme_path.exists() {
        let readme_content = r#"======================================================================
Kokoro-Engine 本地中文轻量级情感模型 (Chinese-Emotion-Small) 离线部署说明
======================================================================

【必需文件清单 / Required Files】
本目录下必须包含以下 5 个模型组件文件：
1. model.onnx               - ONNX 格式的情感推理模型权重 (~25MB INT8 或原生权重)
2. config.json              - 模型架构与 8 种中文情感分类标签配置
3. tokenizer.json           - Hugging Face Fast Tokenizer 分词词表
4. tokenizer_config.json    - 分词器超参配置
5. special_tokens_map.json  - 特殊 Token 映射

【离线获取方式 / Offline Download Channels】
若因网络环境受限（如防火墙、无外网、Hugging Face 访问困难），您可以：
1. 从官方 GitHub Releases 下载预打包好的压缩包 (chinese-emotion-small-onnx.zip):
   https://github.com/huwany1/Kokoro-Engine/releases
2. 从国内网盘或镜像备份下载；
3. 将下载的文件直接解压并拖入本文件夹中。

【生效方法】
文件放入本目录后，切回 Kokoro-Engine 界面即可自动识别并启动；
亦可在软件界面的“增值体验包”设置面板中点击【手动导入】选取 .zip 文件。
"#;
        let _ = std::fs::write(&readme_path, readme_content);
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(&snapshot_dir)
            .spawn()
            .map_err(|e| format!("Failed to open explorer: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(&snapshot_dir)
            .spawn()
            .map_err(|e| format!("Failed to open directory: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(&snapshot_dir)
            .spawn()
            .map_err(|e| format!("Failed to open directory: {}", e))?;
    }

    Ok(snapshot_dir.to_string_lossy().into_owned())
}

fn copy_required_files_from_dir(src_dir: &Path, staging_dir: &Path) -> Result<(), String> {
    for req in REQUIRED_FILES {
        let mut found = None;
        let direct = src_dir.join(req);
        if direct.is_file() {
            found = Some(direct);
        } else if *req == "model.onnx" && src_dir.join("model.int8.onnx").is_file() {
            found = Some(src_dir.join("model.int8.onnx"));
        } else if let Ok(entries) = std::fs::read_dir(src_dir) {
            for entry in entries.flatten() {
                let sub = entry.path();
                if sub.is_dir() {
                    let sub_direct = sub.join(req);
                    if sub_direct.is_file() {
                        found = Some(sub_direct);
                        break;
                    } else if *req == "model.onnx" && sub.join("model.int8.onnx").is_file() {
                        found = Some(sub.join("model.int8.onnx"));
                        break;
                    }
                }
            }
        }

        if let Some(src_file) = found {
            std::fs::copy(&src_file, staging_dir.join(req))
                .map_err(|e| format!("Failed to copy file {}: {}", req, e))?;
        }
    }
    Ok(())
}

pub fn import_emotion_model_package(source_path: &str) -> Result<EmotionModelStatus, String> {
    let path = Path::new(source_path);
    if !path.exists() {
        return Err(format!("指定的模型文件或目录不存在: {}", source_path));
    }

    let repo_dir = default_model_repo_dir();
    let staging_dir = repo_dir.join(format!(".staging.import.{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging_dir)
        .map_err(|e| format!("创建导入暂存区失败: {}", e))?;

    let result = (|| -> Result<(), String> {
        if path.is_file() {
            let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_lowercase();
            if extension == "zip" {
                let file = std::fs::File::open(path)
                    .map_err(|e| format!("打开 ZIP 文件失败: {}", e))?;
                let mut archive = zip::ZipArchive::new(file)
                    .map_err(|e| format!("解析 ZIP 压缩包失败: {}", e))?;

                for i in 0..archive.len() {
                    let mut entry = archive.by_index(i)
                        .map_err(|e| format!("读取压缩包条目失败: {}", e))?;

                    if entry.is_dir() {
                        continue;
                    }

                    // Security: Guard against Zip Slip directory traversal
                    let enclosed_name = match entry.enclosed_name() {
                        Some(p) => p.to_path_buf(),
                        None => continue,
                    };

                    let file_name = match enclosed_name.file_name().and_then(|n| n.to_str()) {
                        Some(name) => name.to_string(),
                        None => continue,
                    };

                    let target_file_name = if file_name == "model.int8.onnx" || file_name == "model.onnx" {
                        "model.onnx"
                    } else if REQUIRED_FILES.contains(&file_name.as_str()) {
                        file_name.as_str()
                    } else {
                        continue;
                    };

                    let dest_path = staging_dir.join(target_file_name);
                    let mut dest_file = std::fs::File::create(&dest_path)
                        .map_err(|e| format!("创建暂存文件 {} 失败: {}", target_file_name, e))?;
                    std::io::copy(&mut entry, &mut dest_file)
                        .map_err(|e| format!("解压文件 {} 失败: {}", target_file_name, e))?;
                }
            } else if extension == "onnx" {
                let dest_path = staging_dir.join("model.onnx");
                std::fs::copy(path, &dest_path)
                    .map_err(|e| format!("拷贝 ONNX 文件失败: {}", e))?;

                let parent_dir = path.parent();
                let snapshot_dir = default_model_snapshot_dir();
                for &req in REQUIRED_FILES.iter().filter(|&&f| f != "model.onnx") {
                    let mut found = false;
                    if let Some(p) = parent_dir {
                        let companion = p.join(req);
                        if companion.exists() {
                            let _ = std::fs::copy(&companion, staging_dir.join(req));
                            found = true;
                        }
                    }
                    if !found && snapshot_dir.join(req).exists() {
                        let _ = std::fs::copy(snapshot_dir.join(req), staging_dir.join(req));
                    }
                }
            } else {
                return Err(format!("不支持的文件格式: .{} (仅支持 .zip 压缩包或 .onnx 权重)", extension));
            }
        } else if path.is_dir() {
            copy_required_files_from_dir(path, &staging_dir)?;
        } else {
            return Err("选择的路径不是有效的文件或目录".to_string());
        }

        // Validate completeness of required files
        let missing = missing_required_model_files(&staging_dir);
        if !missing.is_empty() {
            return Err(format!(
                "模型包缺失必需组件: {}。请确保包含完整的 5 个组件 (model.onnx, config.json, tokenizer.json, tokenizer_config.json, special_tokens_map.json)",
                missing.join(", ")
            ));
        }

        let onnx_path = staging_dir.join("model.onnx");
        let onnx_size = std::fs::metadata(&onnx_path).map(|m| m.len()).unwrap_or(0);
        if onnx_size < 10 * 1024 * 1024 {
            return Err(format!(
                "model.onnx 尺寸过小 ({:.1} MB)，疑似非完整权重文件",
                onnx_size as f64 / (1024.0 * 1024.0)
            ));
        }

        // Dry-run tokenizer test
        let tokenizer_path = staging_dir.join("tokenizer.json");
        Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("分词器词表 tokenizer.json 校验失败: {}", e))?;

        // Dry-run Session test
        let _test_session = Session::builder()
            .map_err(|e| format!("创建 ONNX Session 失败: {}", e))?
            .with_intra_threads(1)
            .map_err(|e| format!("设置线程池失败: {}", e))?
            .commit_from_file(&onnx_path)
            .map_err(|e| format!("ONNX 模型加载试运行失败: {}", e))?;

        // Safe session drop to release Windows handle before overwriting
        unload_emotion_engine();
        std::thread::yield_now();

        let snapshot_dir = default_model_snapshot_dir();
        std::fs::create_dir_all(&snapshot_dir)
            .map_err(|e| format!("创建目标目录失败: {}", e))?;

        for req in REQUIRED_FILES {
            let src = staging_dir.join(req);
            let dst = snapshot_dir.join(req);
            if src.exists() {
                let _ = std::fs::copy(&src, &dst);
            }
        }

        Ok(())
    })();

    // Always sweep staging directory
    let _ = std::fs::remove_dir_all(&staging_dir);

    result?;

    IS_ACTIVE.store(true, Ordering::Relaxed);
    get_or_load_engine()?;

    Ok(get_emotion_model_status())
}

pub async fn download_emotion_model<F>(emit_progress: F) -> Result<EmotionModelStatus, String>
where
    F: Fn(EmotionModelDownloadProgress) -> Result<(), String> + Send + Sync + 'static,
{
    let snapshot_dir = default_model_snapshot_dir();
    let repo_dir = default_model_repo_dir();
    std::fs::create_dir_all(&snapshot_dir)
        .map_err(|e| format!("Failed to create snapshot dir: {}", e))?;
    std::fs::create_dir_all(repo_dir.join("refs"))
        .map_err(|e| format!("Failed to create refs dir: {}", e))?;
    let _ = std::fs::write(repo_dir.join("refs").join(MODEL_REF_NAME), MODEL_REF_NAME);

    let missing = missing_required_model_files(&snapshot_dir);
    let file_count = missing.len();
    if file_count == 0 {
        IS_ACTIVE.store(true, Ordering::Relaxed);
        let _ = get_or_load_engine();
        return Ok(get_emotion_model_status());
    }

    let emit_progress = Arc::new(emit_progress);

    // Staging isolation directory: never write partial or corrupt files directly to snapshot_dir
    let staging_dir = repo_dir.join(format!(".staging.download.{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging_dir)
        .map_err(|e| format!("创建下载暂存区失败: {}", e))?;

    // Copy any already existing valid files into staging_dir
    for req in REQUIRED_FILES {
        let existing = snapshot_dir.join(req);
        if existing.is_file() {
            let _ = std::fs::copy(&existing, staging_dir.join(req));
        }
    }

    let endpoints = emotion_model_endpoints();
    let client = reqwest::Client::builder()
        .user_agent("kokoro-engine/0.4.0")
        .build()
        .map_err(|e| format!("Failed to build reqwest client: {}", e))?;

    let download_result = async {
        for (index, file_name) in missing.iter().enumerate() {
            let target_path = emotion_model_file_path(&staging_dir, file_name)?;
            let mut download_ok = false;
            let mut last_err = String::new();

            for endpoint in &endpoints {
                let url = emotion_model_file_url(endpoint, file_name);
                let progress_sender = emit_progress.clone();
                let fname = file_name.clone();

                emit_progress(build_download_progress(
                    "downloading",
                    format!("正在下载 {} ({}/{})", file_name, index + 1, file_count),
                    file_name.clone(),
                    index + 1,
                    file_count,
                    0,
                    None,
                ))?;

                let dl_res = crate::utils::download::download_file_with_progress(
                    &client,
                    &url,
                    &target_path,
                    crate::utils::download::DownloadOptions::default(),
                    Arc::new(move |p| {
                        progress_sender(build_download_progress(
                            "downloading",
                            format!("正在下载 {} ({}/{})", fname, index + 1, file_count),
                            fname.clone(),
                            index + 1,
                            file_count,
                            p.downloaded_bytes,
                            p.total_bytes,
                        ))
                    }),
                )
                .await;

                match dl_res {
                    Ok(_) => {
                        download_ok = true;
                        break;
                    }
                    Err(err) => {
                        last_err = err;
                    }
                }
            }

            if !download_ok {
                // Check local fallback directory (e.g. scratch/emotion_onnx_export)
                let local_fallbacks = [
                    PathBuf::from("scratch/emotion_onnx_export").join(file_name),
                    PathBuf::from("../scratch/emotion_onnx_export").join(file_name),
                ];
                let mut recovered = false;
                for fb in &local_fallbacks {
                    if fb.exists() && std::fs::copy(fb, &target_path).is_ok() {
                        recovered = true;
                        break;
                    }
                }
                if !recovered {
                    let err_msg = if last_err.contains("404") {
                        format!("上游源未提供原生 {} (HTTP 404)。请使用【手动导入】选取离线模型包，或点击【打开存储目录】查阅部署指引。", file_name)
                    } else {
                        format!("下载 {} 失败: {}。国内网络如受限，建议使用【手动导入】或【打开存储目录】放置模型包。", file_name, last_err)
                    };
                    return Err(err_msg);
                }
            }
        }

        emit_progress(build_download_progress(
            "verifying",
            "正在校验模型权重完整性...".to_string(),
            "model.onnx".to_string(),
            file_count,
            file_count,
            0,
            None,
        ))?;

        // Triple-gate verification in staging
        let onnx_path = staging_dir.join("model.onnx");
        if !onnx_path.exists() {
            return Err("Staging 校验失败: model.onnx 丢失".to_string());
        }
        let onnx_size = std::fs::metadata(&onnx_path).map(|m| m.len()).unwrap_or(0);
        if onnx_size < 10 * 1024 * 1024 {
            return Err(format!(
                "model.onnx 尺寸过小 ({:.1} MB)，疑似下载损坏",
                onnx_size as f64 / (1024.0 * 1024.0)
            ));
        }

        Tokenizer::from_file(staging_dir.join("tokenizer.json"))
            .map_err(|e| format!("分词器校验失败: {}", e))?;

        let _test_session = Session::builder()
            .map_err(|e| format!("创建测试 Session 失败: {}", e))?
            .with_intra_threads(1)
            .map_err(|e| format!("配置线程失败: {}", e))?
            .commit_from_file(&onnx_path)
            .map_err(|e| format!("加载 ONNX 模型校验失败: {}", e))?;

        // Atomic promote: drop existing session, copy staging to snapshot_dir
        unload_emotion_engine();
        std::thread::yield_now();

        for req in REQUIRED_FILES {
            let src = staging_dir.join(req);
            let dst = snapshot_dir.join(req);
            if src.exists() {
                let _ = std::fs::copy(&src, &dst);
            }
        }

        Ok(())
    }.await;

    // Clean up staging directory unconditionally
    let _ = std::fs::remove_dir_all(&staging_dir);

    download_result?;

    IS_ACTIVE.store(true, Ordering::Relaxed);
    get_or_load_engine()?;

    emit_progress(build_download_progress(
        "ready",
        "情感模型已就绪并启动".to_string(),
        "model.onnx".to_string(),
        file_count,
        file_count,
        0,
        None,
    ))?;

    Ok(get_emotion_model_status())
}

fn get_or_load_engine() -> Result<(), String> {
    if !IS_ACTIVE.load(Ordering::Relaxed) {
        return Err("Emotion engine is disabled".to_string());
    }

    if let Ok(guard) = engine_instance().read() {
        if guard.is_some() {
            return Ok(());
        }
    }

    let mut guard = engine_instance().write().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(());
    }

    let snapshot_dir = default_model_snapshot_dir();
    let model_path = snapshot_dir.join("model.onnx");
    let tokenizer_path = snapshot_dir.join("tokenizer.json");

    if !model_path.exists() || !tokenizer_path.exists() {
        return Err("Emotion model or tokenizer files not found on disk".to_string());
    }

    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| format!("Failed to load tokenizer from {}: {}", tokenizer_path.display(), e))?;

    let session = Session::builder()
        .map_err(|e| format!("Failed to create Ort session builder: {}", e))?
        .with_intra_threads(1)
        .map_err(|e| format!("Failed to set intra threads: {}", e))?
        .commit_from_file(&model_path)
        .map_err(|e| format!("Failed to load ONNX session from {}: {}", model_path.display(), e))?;

    *guard = Some(EmotionEngine { session, tokenizer });
    tracing::info!(target: "ai", "[Emotion] Loaded Chinese-Emotion-Small ONNX engine successfully.");
    Ok(())
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum == 0.0 {
        vec![1.0 / logits.len() as f32; logits.len()]
    } else {
        exps.iter().map(|&x| x / sum).collect()
    }
}

pub fn map_emotion_to_live2d_cue(dominant: &str, available_cues: &[String]) -> Option<String> {
    if available_cues.is_empty() {
        return None;
    }

    let search_candidates: &[&str] = match dominant {
        "happy" => &["笑", "微笑", "开心", "高兴", "喜", "happy", "smile"],
        "sad" => &["悲", "难过", "失落", "伤心", "哭", "sad"],
        "angry" => &["生氣", "生气", "傲娇", "愤怒", "恼", "angry"],
        "surprised" => &["惊讶", "惊奇", "吃惊", "睁眼", "surprised"],
        "questioning" => &["疑惑", "疑问", "思考", "困惑", "歪头", "question", "confused"],
        "caring" => &["关切", "温柔", "微笑", "安慰", "care", "gentle"],
        "disgusted" => &["嫌弃", "厌恶", "冷漠", "白眼", "disgust"],
        _ => &["平静", "默认", "普通", "neutral", "default"],
    };

    for &cand in search_candidates {
        for cue in available_cues {
            if cue.eq_ignore_ascii_case(cand) || cue.contains(cand) {
                return Some(cue.clone());
            }
        }
    }

    None
}

pub fn infer_emotion(text: &str) -> Result<EmotionInferenceResult, String> {
    let start_time = Instant::now();

    // 1. Text truncation: tail 128 characters (emotional cues in Chinese dialogue are strongest at the sentence end)
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(EmotionInferenceResult {
            dominant_emotion: "neutral".to_string(),
            label_zh: "平淡語氣".to_string(),
            confidence: 1.0,
            probabilities: EMOTION_LABELS
                .iter()
                .enumerate()
                .map(|(i, &(eng, zh))| EmotionInferenceProbability {
                    label: eng.to_string(),
                    label_zh: zh.to_string(),
                    score: if i == 0 { 1.0 } else { 0.0 },
                })
                .collect(),
            mapped_cue: None,
            latency_ms: 0.1,
        });
    }

    let truncated_text: String = trimmed.chars().rev().take(128).collect::<Vec<_>>().into_iter().rev().collect();

    // 2. Ensure engine is loaded
    get_or_load_engine()?;

    let mut guard = engine_instance().write().map_err(|e| e.to_string())?;
    let engine = guard.as_mut().ok_or_else(|| "Emotion engine is not loaded".to_string())?;

    // 3. Tokenize with max_length 64 to avoid quadratic attention explosion
    let encoding = engine
        .tokenizer
        .encode(truncated_text.as_str(), true)
        .map_err(|e| format!("Tokenizer encode failed: {}", e))?;

    let token_ids: Vec<i64> = encoding.get_ids().iter().take(64).map(|&id| id as i64).collect();
    let attention_mask: Vec<i64> = encoding.get_attention_mask().iter().take(64).map(|&m| m as i64).collect();
    let seq_len = token_ids.len();

    let ids_tensor = Tensor::from_array(([1, seq_len], token_ids))
        .map_err(|e| format!("Failed to create input_ids tensor: {}", e))?;
    let mask_tensor = Tensor::from_array(([1, seq_len], attention_mask))
        .map_err(|e| format!("Failed to create attention_mask tensor: {}", e))?;

    // 4. Run ONNX Session
    let inputs = ort::inputs![
        "input_ids" => ids_tensor,
        "attention_mask" => mask_tensor,
    ];

    let outputs = engine
        .session
        .run(inputs)
        .map_err(|e| format!("ONNX Session run failed: {}", e))?;

    // 5. Extract logits
    let first_output = outputs
        .into_iter()
        .next()
        .ok_or_else(|| "No output tensor returned by model".to_string())?;

    let (_, tensor_ref) = first_output;
    let tensor = tensor_ref
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("Failed to extract tensor as f32: {}", e))?;

    let raw_slice = tensor.1;
    let slice_len = raw_slice.len().min(8);
    let logits = &raw_slice[..slice_len];

    let probs = softmax(logits);

    let mut best_idx = 0;
    let mut best_score = 0.0f32;
    let mut prob_list = Vec::with_capacity(8);

    for (idx, &(eng, zh)) in EMOTION_LABELS.iter().enumerate() {
        let score = probs.get(idx).copied().unwrap_or(0.0);
        if score > best_score {
            best_score = score;
            best_idx = idx;
        }
        prob_list.push(EmotionInferenceProbability {
            label: eng.to_string(),
            label_zh: zh.to_string(),
            score,
        });
    }

    let (dominant_eng, dominant_zh) = EMOTION_LABELS[best_idx];

    // Check Live2D profile for cue mapping
    let mapped_cue = crate::commands::live2d::load_active_live2d_profile().and_then(|prof| {
        let keys: Vec<String> = prof.cue_map.keys().cloned().collect();
        map_emotion_to_live2d_cue(dominant_eng, &keys)
    });

    let latency_ms = start_time.elapsed().as_secs_f32() * 1000.0;

    Ok(EmotionInferenceResult {
        dominant_emotion: dominant_eng.to_string(),
        label_zh: dominant_zh.to_string(),
        confidence: best_score,
        probabilities: prob_list,
        mapped_cue,
        latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_softmax_sum_to_one() {
        let logits = vec![2.0, 1.0, 0.1, -1.0, 0.5, 3.0, -0.5, 0.0];
        let probs = softmax(&logits);
        assert_eq!(probs.len(), 8);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_map_emotion_to_live2d_cue() {
        let cues = vec!["微笑".to_string(), "悲".to_string(), "平静".to_string()];
        assert_eq!(map_emotion_to_live2d_cue("happy", &cues), Some("微笑".to_string()));
        assert_eq!(map_emotion_to_live2d_cue("sad", &cues), Some("悲".to_string()));
        assert_eq!(map_emotion_to_live2d_cue("neutral", &cues), Some("平静".to_string()));
    }

    #[test]
    fn test_local_model_installed_and_inference() {
        let status = get_emotion_model_status();
        if status.installed {
            let res = infer_emotion("今天真是太开心了，所有任务都顺利完成了！").expect("inference should succeed");
            assert!(!res.dominant_emotion.is_empty());
            assert_eq!(res.dominant_emotion, "happy");
            assert!(res.confidence > 0.5);
        }
    }

    #[test]
    fn test_import_nonexistent_path_fails() {
        let res = import_emotion_model_package("non_existent_folder_xyz_123_kokoro_test");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("不存在"));
    }
}
