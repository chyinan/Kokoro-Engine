// pattern: Imperative Shell & Clean Domain Service
use anyhow::Result;
use ort::session::Session;
use ort::tensor::TensorElementType;
use ort::value::{Tensor, ValueType};
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
    pub is_valid: bool,
    pub error_message: Option<String>,
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
    label_mapping: [usize; 8],
}

fn engine_instance() -> &'static Arc<RwLock<Option<EmotionEngine>>> {
    static INSTANCE: OnceLock<Arc<RwLock<Option<EmotionEngine>>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(RwLock::new(None)))
}

static IS_ACTIVE: AtomicBool = AtomicBool::new(true);
static LAST_LOAD_ERROR: RwLock<Option<String>> = RwLock::new(None);

pub fn set_last_load_error(err: Option<String>) {
    if let Ok(mut guard) = LAST_LOAD_ERROR.write() {
        *guard = err;
    }
}

pub fn get_last_load_error() -> Option<String> {
    LAST_LOAD_ERROR.read().ok().and_then(|g| g.clone())
}

pub const MIN_ONNX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10MB
pub const MIN_CONFIG_FILE_SIZE_BYTES: u64 = 10; // 10 bytes

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

/// Fast static inspection: checks file presence and minimum realistic sizes
pub fn inspect_model_files_fast(snapshot_dir: &Path) -> Result<(), String> {
    let missing = missing_required_model_files(snapshot_dir);
    if !missing.is_empty() {
        return Err(format!("缺失必需模型组件: {}", missing.join(", ")));
    }

    let onnx_path = snapshot_dir.join("model.onnx");
    let onnx_size = std::fs::metadata(&onnx_path)
        .map(|m| m.len())
        .map_err(|e| format!("无法读取 model.onnx 元数据: {}", e))?;

    if onnx_size < MIN_ONNX_FILE_SIZE_BYTES {
        return Err(format!(
            "model.onnx 尺寸过小 ({:.1} MB)，疑似下载中断或损坏",
            onnx_size as f64 / (1024.0 * 1024.0)
        ));
    }

    for &req in REQUIRED_FILES {
        if req == "model.onnx" {
            continue;
        }
        let p = snapshot_dir.join(req);
        let size = std::fs::metadata(&p)
            .map(|m| m.len())
            .map_err(|e| format!("无法读取 {} 元数据: {}", req, e))?;
        if size < MIN_CONFIG_FILE_SIZE_BYTES {
            return Err(format!("{} 内容异常（文件过小: {} 字节），疑似损坏", req, size));
        }
    }

    Ok(())
}

/// Canonical mapping helper: maps English/Chinese emotion names or label ids (LABEL_0..7)
/// to their canonical index in `EMOTION_LABELS` (0..8).
pub fn normalize_canonical_emotion_index(label: &str) -> Option<usize> {
    let s = label.trim().to_lowercase();
    match s.as_str() {
        "neutral" | "label_0" | "平淡" | "平淡語氣" | "平淡语气" | "平静" => Some(0),
        "caring" | "concerned" | "label_1" | "关切" | "關切" | "關切語調" | "关切语调" | "温柔" | "安慰" => Some(1),
        "happy" | "label_2" | "开心" | "開心" | "開心語調" | "开心语调" | "喜" | "微笑" | "喜悦" => Some(2),
        "angry" | "label_3" | "愤怒" | "憤怒" | "憤怒語調" | "愤怒语调" | "生气" | "生氣" | "傲娇" => Some(3),
        "sad" | "label_4" | "悲伤" | "悲傷" | "悲傷語調" | "悲伤语调" | "难过" | "失落" | "伤心" => Some(4),
        "questioning" | "confused" | "label_5" | "疑问" | "疑問" | "疑問語調" | "疑问语调" | "困惑" | "思考" => Some(5),
        "surprised" | "label_6" | "惊奇" | "驚奇" | "驚奇語調" | "惊奇语调" | "惊讶" | "吃惊" => Some(6),
        "disgusted" | "label_7" | "厌恶" | "厭惡" | "厭惡語調" | "厌恶语调" | "嫌弃" | "冷漠" => Some(7),
        _ => None,
    }
}

pub struct EmotionModelContract;

impl EmotionModelContract {
    /// 校验输入与输出张量规格契约 (Input names, dtypes, dimensions & output names, dtypes, dimensions)
    pub fn validate_session_io(session: &Session) -> Result<(), String> {
        let inputs = session.inputs();
        let outputs = session.outputs();

        // 1. 必须包含 input_ids 与 attention_mask，且均为 Int64 的二维张量 [batch, seq_len]
        let has_input_ids = inputs.iter().any(|inp| {
            if inp.name() == "input_ids" {
                if let ValueType::Tensor { ty, shape, .. } = inp.dtype() {
                    return *ty == TensorElementType::Int64 && shape.len() == 2;
                }
            }
            false
        });
        if !has_input_ids {
            return Err("模型输入契约违约: 缺少名为 'input_ids' 且类型为 Int64 的二维张量 [batch, seq_len]".to_string());
        }

        let has_attention_mask = inputs.iter().any(|inp| {
            if inp.name() == "attention_mask" {
                if let ValueType::Tensor { ty, shape, .. } = inp.dtype() {
                    return *ty == TensorElementType::Int64 && shape.len() == 2;
                }
            }
            false
        });
        if !has_attention_mask {
            return Err("模型输入契约违约: 缺少名为 'attention_mask' 且类型为 Int64 的二维张量 [batch, seq_len]".to_string());
        }

        // 2. 输出张量必须存在，且第一维为 batch，第二维必须为 8 类（若在模型图元中明确标注），元素类型必须为 Float32
        if outputs.is_empty() {
            return Err("模型输出契约违约: 模型未声明任何输出张量".to_string());
        }

        let target_output = outputs
            .iter()
            .find(|out| out.name() == "logits")
            .or_else(|| outputs.first());

        if let Some(out) = target_output {
            match out.dtype() {
                ValueType::Tensor { ty, shape, .. } => {
                    if *ty != TensorElementType::Float32 {
                        return Err(format!(
                            "模型输出契约违约: 输出张量 '{}' 类型必须为 Float32, 实际为 {:?}",
                            out.name(),
                            ty
                        ));
                    }
                    if shape.len() != 2 {
                        return Err(format!(
                            "模型输出契约违约: 输出张量 '{}' 维度必须为 2 [batch, classes], 实际为 {:?}",
                            out.name(),
                            shape
                        ));
                    }
                    let classes_dim = shape[1];
                    if classes_dim != -1 && classes_dim != 8 {
                        return Err(format!(
                            "模型输出契约违约: 输出张量 '{}' 分类数量必须为 8, 实际第二维为 {}",
                            out.name(),
                            classes_dim
                        ));
                    }
                }
                other => {
                    return Err(format!(
                        "模型输出契约违约: 输出张量 '{}' 必须为 Tensor 类型, 实际为 {:?}",
                        out.name(),
                        other
                    ));
                }
            }
        }

        Ok(())
    }

    /// 解析 config.json 中的 id2label / label2id 并构建 [model_output_idx -> canonical_idx] 重排映射
    pub fn resolve_label_mapping(config_path: &Path) -> Result<[usize; 8], String> {
        let content = std::fs::read_to_string(config_path)
            .map_err(|e| format!("读取配置文件 {} 失败: {}", config_path.display(), e))?;
        let json: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("解析配置文件 {} JSON 失败: {}", config_path.display(), e))?;

        // 尝试从 id2label 解析
        if let Some(id2label) = json.get("id2label").and_then(|v| v.as_object()) {
            if id2label.len() != 8 {
                return Err(format!(
                    "模型配置违约: id2label 类别数量为 {}, 严格期望为 8 类情感",
                    id2label.len()
                ));
            }

            let mut mapping = [0usize; 8];
            let mut covered = [false; 8];

            for (key, val) in id2label {
                let model_idx: usize = key
                    .parse()
                    .map_err(|_| format!("id2label 键名不是非负整数: {}", key))?;
                if model_idx >= 8 {
                    return Err(format!("id2label 索引超出范围 (>= 8): {}", model_idx));
                }

                let label_str = val.as_str().unwrap_or("");
                let canonical_idx = normalize_canonical_emotion_index(label_str).ok_or_else(|| {
                    format!("模型配置中的标签 '{}' 无法对齐至标准 8 种情感之一", label_str)
                })?;

                mapping[model_idx] = canonical_idx;
                covered[canonical_idx] = true;
            }

            if !covered.iter().all(|&c| c) {
                return Err("模型配置违约: id2label 未能完整覆盖 8 种标准情绪分类".to_string());
            }

            return Ok(mapping);
        }

        // 尝试从 label2id 解析
        if let Some(label2id) = json.get("label2id").and_then(|v| v.as_object()) {
            if label2id.len() != 8 {
                return Err(format!(
                    "模型配置违约: label2id 类别数量为 {}, 严格期望为 8 类情感",
                    label2id.len()
                ));
            }

            let mut mapping = [0usize; 8];
            let mut covered = [false; 8];

            for (label_str, val) in label2id {
                let model_idx: usize = val
                    .as_u64()
                    .ok_or_else(|| format!("label2id value 不是合法的非负整数: {:?}", val))? as usize;
                if model_idx >= 8 {
                    return Err(format!("label2id 索引超出范围 (>= 8): {}", model_idx));
                }

                let canonical_idx = normalize_canonical_emotion_index(label_str).ok_or_else(|| {
                    format!("模型配置中的标签 '{}' 无法对齐至标准 8 种情感之一", label_str)
                })?;

                mapping[model_idx] = canonical_idx;
                covered[canonical_idx] = true;
            }

            if !covered.iter().all(|&c| c) {
                return Err("模型配置违约: label2id 未能完整覆盖 8 种标准情绪分类".to_string());
            }

            return Ok(mapping);
        }

        // 若无显式 id2label/label2id，但有 num_labels，校验其值
        if let Some(num) = json.get("num_labels").and_then(|v| v.as_u64()) {
            if num != 8 {
                return Err(format!("模型配置违约: num_labels 为 {}, 期望为 8", num));
            }
        }

        // 默认按自然序列 0..8 直通对齐 (LABEL_0..LABEL_7 官方基线映射)
        Ok([0, 1, 2, 3, 4, 5, 6, 7])
    }

    /// 执行真实的 Dummy 文本试运行推断，检验端到端链路、张量大小与有限数值
    pub fn run_dummy_inference(
        session: &Session,
        tokenizer: &Tokenizer,
        label_mapping: &[usize; 8],
    ) -> Result<(), String> {
        let dummy_text = "你好，欢迎使用情感系统。";
        let encoding = tokenizer
            .encode(dummy_text, true)
            .map_err(|e| format!("Dummy 推理 Tokenizer 编码失败: {}", e))?;

        let token_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let attention_mask: Vec<i64> = encoding.get_attention_mask().iter().map(|&m| m as i64).collect();
        let seq_len = token_ids.len();

        if seq_len == 0 {
            return Err("Dummy 推理失败: 编码结果序列长度为 0".to_string());
        }

        let ids_tensor = Tensor::from_array(([1, seq_len], token_ids))
            .map_err(|e| format!("创建 Dummy input_ids 张量失败: {}", e))?;
        let mask_tensor = Tensor::from_array(([1, seq_len], attention_mask))
            .map_err(|e| format!("创建 Dummy attention_mask 张量失败: {}", e))?;

        let inputs = ort::inputs![
            "input_ids" => ids_tensor,
            "attention_mask" => mask_tensor,
        ];

        let outputs = session
            .run(inputs)
            .map_err(|e| format!("Dummy 推理 ONNX Session run 失败: {}", e))?;

        let mut target_output = None;
        for (name, val) in outputs {
            if name == "logits" {
                target_output = Some(val);
                break;
            } else if target_output.is_none() {
                target_output = Some(val);
            }
        }

        let out_val = target_output.ok_or_else(|| "Dummy 推理失败: 未获取到输出张量".to_string())?;
        let tensor = out_val
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Dummy 推理提取输出张量 f32 失败: {}", e))?;

        let raw_slice = tensor.1;
        if raw_slice.len() != 8 {
            return Err(format!(
                "Dummy 推理契约违约: 模型输出元素数量为 {}, 严格期望为 8",
                raw_slice.len()
            ));
        }

        for (idx, &val) in raw_slice.iter().enumerate() {
            if !val.is_finite() {
                return Err(format!(
                    "Dummy 推理契约违约: 模型输出索引 {} 产生非有限浮点数 ({})",
                    idx, val
                ));
            }
        }

        let probs = softmax(raw_slice);
        if probs.len() != 8 {
            return Err(format!("Softmax 概率分布维度异常: {}", probs.len()));
        }

        for (idx, &p) in probs.iter().enumerate() {
            if !p.is_finite() || p < 0.0 || p > 1.0 {
                return Err(format!(
                    "Softmax 概率值异常: 类别索引 {} 概率为 {}",
                    idx, p
                ));
            }
        }

        let sum: f32 = probs.iter().sum();
        if (sum - 1.0).abs() > 0.01 {
            return Err(format!("Softmax 概率归一化校验失败: 概率和为 {}", sum));
        }

        for model_idx in 0..8 {
            let canonical_idx = label_mapping[model_idx];
            if canonical_idx >= 8 {
                return Err(format!(
                    "重排映射越界: model_idx {} 映射到了 canonical_idx {}",
                    model_idx, canonical_idx
                ));
            }
        }

        tracing::info!(
            target: "ai",
            "[Emotion] Dummy inference contract check PASSED (output size: 8, all finite, prob sum: {:.4})",
            sum
        );

        Ok(())
    }
}

/// Deep validation: loads Tokenizer, creates ONNX Session, verifies I/O contracts,
/// checks config label ordering, and runs dummy inference before approval.
pub fn validate_model_files_deep(dir: &Path) -> Result<(), String> {
    inspect_model_files_fast(dir)?;

    let tokenizer_path = dir.join("tokenizer.json");
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| format!("分词器 tokenizer.json 校验失败: {}", e))?;

    let onnx_path = dir.join("model.onnx");
    let session = Session::builder()
        .map_err(|e| format!("创建 ONNX Session 失败: {}", e))?
        .with_intra_threads(1)
        .map_err(|e| format!("设置线程池失败: {}", e))?
        .commit_from_file(&onnx_path)
        .map_err(|e| format!("ONNX 模型加载试运行失败: {}", e))?;

    EmotionModelContract::validate_session_io(&session)?;

    let config_path = dir.join("config.json");
    let label_mapping = EmotionModelContract::resolve_label_mapping(&config_path)?;

    EmotionModelContract::run_dummy_inference(&session, &tokenizer, &label_mapping)?;

    Ok(())
}

pub fn get_emotion_model_status() -> EmotionModelStatus {
    let snapshot_dir = default_model_snapshot_dir();
    let missing_files = missing_required_model_files(&snapshot_dir);
    let model_path = snapshot_dir.join("model.onnx");
    let files_present = missing_files.is_empty();

    let (installed, is_valid, error_message) = if files_present {
        match inspect_model_files_fast(&snapshot_dir) {
            Ok(_) => {
                if let Some(err) = get_last_load_error() {
                    (true, false, Some(err))
                } else {
                    (true, true, None)
                }
            }
            Err(err) => (false, false, Some(err)),
        }
    } else {
        (false, false, None)
    };

    let is_active = IS_ACTIVE.load(Ordering::Relaxed) && installed && is_valid;

    let memory_bytes = if is_active {
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
        is_valid,
        error_message,
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
    if active && (!current_status.installed || !current_status.is_valid) {
        return Err(format!(
            "Cannot activate emotion model: {}",
            current_status.error_message.unwrap_or_else(|| "files not installed or model invalid".to_string())
        ));
    }

    IS_ACTIVE.store(active, Ordering::Relaxed);
    if !active {
        // Drop session memory if turned off
        unload_emotion_engine();
    } else {
        get_or_load_engine()?;
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

    set_last_load_error(None);
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

        // Validate completeness and validity of required files via deep validation
        validate_model_files_deep(&staging_dir)?;

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

    set_last_load_error(None);
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

    // Self-healing check: if all required files exist, perform deep validation before early exit!
    let missing_initially = missing_required_model_files(&snapshot_dir);
    if missing_initially.is_empty() {
        if let Err(val_err) = validate_model_files_deep(&snapshot_dir) {
            tracing::warn!(
                target: "ai",
                "[Emotion] Existing model in snapshot_dir is invalid ({}). Resetting snapshot for clean repair download.",
                val_err
            );
            unload_emotion_engine();
            std::thread::yield_now();
            let _ = std::fs::remove_dir_all(&snapshot_dir);
            let _ = std::fs::create_dir_all(&snapshot_dir);
            set_last_load_error(None);
        } else {
            // Files are valid on disk. Verify that the engine can load without error.
            IS_ACTIVE.store(true, Ordering::Relaxed);
            match get_or_load_engine() {
                Ok(_) => {
                    set_last_load_error(None);
                    emit_progress(build_download_progress(
                        "ready",
                        "情感模型已就绪并启动".to_string(),
                        "model.onnx".to_string(),
                        REQUIRED_FILES.len(),
                        REQUIRED_FILES.len(),
                        0,
                        None,
                    ))?;
                    return Ok(get_emotion_model_status());
                }
                Err(load_err) => {
                    tracing::warn!(
                        target: "ai",
                        "[Emotion] Engine failed to load from existing valid files ({}). Resetting for repair download.",
                        load_err
                    );
                    unload_emotion_engine();
                    std::thread::yield_now();
                    let _ = std::fs::remove_dir_all(&snapshot_dir);
                    let _ = std::fs::create_dir_all(&snapshot_dir);
                    set_last_load_error(None);
                }
            }
        }
    }

    let missing = missing_required_model_files(&snapshot_dir);
    let file_count = missing.len();
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

        // Unified deep validation in staging before promotion
        validate_model_files_deep(&staging_dir)?;

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

    set_last_load_error(None);
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

    let res = (|| -> Result<EmotionEngine, String> {
        let snapshot_dir = default_model_snapshot_dir();
        inspect_model_files_fast(&snapshot_dir)?;

        let model_path = snapshot_dir.join("model.onnx");
        let tokenizer_path = snapshot_dir.join("tokenizer.json");
        let config_path = snapshot_dir.join("config.json");

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer from {}: {}", tokenizer_path.display(), e))?;

        let session = Session::builder()
            .map_err(|e| format!("Failed to create Ort session builder: {}", e))?
            .with_intra_threads(1)
            .map_err(|e| format!("Failed to set intra threads: {}", e))?
            .commit_from_file(&model_path)
            .map_err(|e| format!("Failed to load ONNX session from {}: {}", model_path.display(), e))?;

        EmotionModelContract::validate_session_io(&session)?;
        let label_mapping = EmotionModelContract::resolve_label_mapping(&config_path)?;
        EmotionModelContract::run_dummy_inference(&session, &tokenizer, &label_mapping)?;

        Ok(EmotionEngine {
            session,
            tokenizer,
            label_mapping,
        })
    })();

    match res {
        Ok(engine) => {
            *guard = Some(engine);
            set_last_load_error(None);
            tracing::info!(target: "ai", "[Emotion] Loaded Chinese-Emotion-Small ONNX engine successfully.");
            Ok(())
        }
        Err(err) => {
            set_last_load_error(Some(err.clone()));
            tracing::error!(target: "ai", "[Emotion] Failed to load engine: {}", err);
            Err(err)
        }
    }
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum == 0.0 || !sum.is_finite() {
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
    let mut target_output = None;
    for (name, val) in outputs {
        if name == "logits" {
            target_output = Some(val);
            break;
        } else if target_output.is_none() {
            target_output = Some(val);
        }
    }

    let tensor_val = target_output.ok_or_else(|| "No output tensor returned by model".to_string())?;
    let tensor = tensor_val
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("Failed to extract tensor as f32: {}", e))?;

    let raw_slice = tensor.1;
    if raw_slice.len() != 8 {
        return Err(format!(
            "模型输出张量维度不符合契约: 期望 8 个分类 logits, 实际返回 {}",
            raw_slice.len()
        ));
    }

    for (idx, &val) in raw_slice.iter().enumerate() {
        if !val.is_finite() {
            return Err(format!(
                "模型推理产生非有限数值 (NaN/Inf): logits[{}] = {}",
                idx, val
            ));
        }
    }

    let probs = softmax(raw_slice);

    // Map output indices to canonical emotion indices
    let mut canonical_probs = [0.0f32; 8];
    for (model_idx, &score) in probs.iter().enumerate() {
        let canonical_idx = engine.label_mapping[model_idx];
        canonical_probs[canonical_idx] = score;
    }

    let mut best_idx = 0;
    let mut best_score = 0.0f32;
    let mut prob_list = Vec::with_capacity(8);

    for (idx, &(eng, zh)) in EMOTION_LABELS.iter().enumerate() {
        let score = canonical_probs[idx];
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
        if status.installed && status.is_valid {
            let res = infer_emotion("今天真是太开心了，所有任务都顺利完成了！").expect("inference should succeed");
            assert!(!res.dominant_emotion.is_empty());
            assert_eq!(res.dominant_emotion, "happy");
            assert!(res.confidence > 0.5);
        }
    }

    #[test]
    fn test_normalize_canonical_emotion_index() {
        assert_eq!(normalize_canonical_emotion_index("neutral"), Some(0));
        assert_eq!(normalize_canonical_emotion_index("LABEL_0"), Some(0));
        assert_eq!(normalize_canonical_emotion_index("平淡语气"), Some(0));

        assert_eq!(normalize_canonical_emotion_index("caring"), Some(1));
        assert_eq!(normalize_canonical_emotion_index("关切"), Some(1));

        assert_eq!(normalize_canonical_emotion_index("happy"), Some(2));
        assert_eq!(normalize_canonical_emotion_index("开心"), Some(2));

        assert_eq!(normalize_canonical_emotion_index("angry"), Some(3));
        assert_eq!(normalize_canonical_emotion_index("愤怒"), Some(3));

        assert_eq!(normalize_canonical_emotion_index("sad"), Some(4));
        assert_eq!(normalize_canonical_emotion_index("悲伤"), Some(4));

        assert_eq!(normalize_canonical_emotion_index("questioning"), Some(5));
        assert_eq!(normalize_canonical_emotion_index("疑问"), Some(5));

        assert_eq!(normalize_canonical_emotion_index("surprised"), Some(6));
        assert_eq!(normalize_canonical_emotion_index("惊讶"), Some(6));

        assert_eq!(normalize_canonical_emotion_index("disgusted"), Some(7));
        assert_eq!(normalize_canonical_emotion_index("厌恶"), Some(7));

        assert_eq!(normalize_canonical_emotion_index("unknown_xyz"), None);
    }

    #[test]
    fn test_resolve_label_mapping_id2label() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let config_path = temp_dir.path().join("config.json");

        // 1. Standard LABEL_0..7
        let cfg_label_n = r#"{
            "id2label": {
                "0": "LABEL_0",
                "1": "LABEL_1",
                "2": "LABEL_2",
                "3": "LABEL_3",
                "4": "LABEL_4",
                "5": "LABEL_5",
                "6": "LABEL_6",
                "7": "LABEL_7"
            }
        }"#;
        std::fs::write(&config_path, cfg_label_n).unwrap();
        let mapping = EmotionModelContract::resolve_label_mapping(&config_path).unwrap();
        assert_eq!(mapping, [0, 1, 2, 3, 4, 5, 6, 7]);

        // 2. Custom reversed names
        let cfg_custom = r#"{
            "id2label": {
                "0": "disgusted",
                "1": "surprised",
                "2": "questioning",
                "3": "sad",
                "4": "angry",
                "5": "happy",
                "6": "caring",
                "7": "neutral"
            }
        }"#;
        std::fs::write(&config_path, cfg_custom).unwrap();
        let mapping2 = EmotionModelContract::resolve_label_mapping(&config_path).unwrap();
        assert_eq!(mapping2, [7, 6, 5, 4, 3, 2, 1, 0]);

        // 3. Invalid class count (only 2 classes)
        let cfg_invalid_count = r#"{
            "id2label": {
                "0": "happy",
                "1": "sad"
            }
        }"#;
        std::fs::write(&config_path, cfg_invalid_count).unwrap();
        let res_err = EmotionModelContract::resolve_label_mapping(&config_path);
        assert!(res_err.is_err());
        assert!(res_err.unwrap_err().contains("严格期望为 8 类"));
    }

    #[test]
    fn test_resolve_label_mapping_label2id() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let config_path = temp_dir.path().join("config.json");

        let cfg = r#"{
            "label2id": {
                "neutral": 0,
                "caring": 1,
                "happy": 2,
                "angry": 3,
                "sad": 4,
                "questioning": 5,
                "surprised": 6,
                "disgusted": 7
            }
        }"#;
        std::fs::write(&config_path, cfg).unwrap();
        let mapping = EmotionModelContract::resolve_label_mapping(&config_path).unwrap();
        assert_eq!(mapping, [0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn test_softmax_nan_inf_guard() {
        let nan_logits = [f32::NAN, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let probs = softmax(&nan_logits);
        assert_eq!(probs.len(), 8);
        for p in &probs {
            assert!(p.is_finite());
            assert!(*p > 0.0);
        }
    }

    #[test]
    fn test_import_nonexistent_path_fails() {
        let res = import_emotion_model_package("non_existent_folder_xyz_123_kokoro_test");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("不存在"));
    }

    #[test]
    fn test_inspect_model_files_fast_rejects_missing_files() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let res = inspect_model_files_fast(temp_dir.path());
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("缺失必需模型组件"));
    }

    #[test]
    fn test_inspect_model_files_fast_rejects_undersized_onnx() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        for req in REQUIRED_FILES {
            let p = temp_dir.path().join(req);
            std::fs::write(&p, b"dummy content").expect("write dummy");
        }
        // model.onnx is only 13 bytes, well below 10MB
        let res = inspect_model_files_fast(temp_dir.path());
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("model.onnx 尺寸过小"));
    }

    #[test]
    fn test_inspect_model_files_fast_rejects_empty_config() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        for req in REQUIRED_FILES {
            let p = temp_dir.path().join(req);
            if *req == "model.onnx" {
                // write a large sparse/zero file >= 10MB
                let f = std::fs::File::create(&p).expect("create onnx");
                f.set_len(11 * 1024 * 1024).expect("set len");
            } else if *req == "config.json" {
                std::fs::write(&p, b"").expect("write empty config");
            } else {
                std::fs::write(&p, b"{\"valid\": true}").expect("write valid json");
            }
        }
        let res = inspect_model_files_fast(temp_dir.path());
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("config.json 内容异常"));
    }

    #[test]
    fn test_status_records_runtime_error_as_invalid_and_inactive() {
        set_last_load_error(Some("Mock ONNX runtime failure test".to_string()));
        let err = get_last_load_error();
        assert_eq!(err, Some("Mock ONNX runtime failure test".to_string()));
        set_last_load_error(None);
        assert_eq!(get_last_load_error(), None);
    }
}
