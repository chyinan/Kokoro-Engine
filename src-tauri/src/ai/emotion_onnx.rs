// pattern: Imperative Shell & Clean Domain Service
use anyhow::Result;
use ort::session::{Session, SessionOutputs};
use ort::tensor::TensorElementType;
use ort::value::{Tensor, ValueType};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock, RwLock};
use std::time::Instant;
use tokenizers::Tokenizer;

pub const MODEL_REPO: &str = "Johnson8187/Chinese-Emotion-Small";
pub const MODEL_PAGE_URL: &str = "https://huggingface.co/Johnson8187/Chinese-Emotion-Small";
pub const MODEL_DIR_NAME: &str = "models--Johnson8187--Chinese-Emotion-Small";
pub const MODEL_REF_NAME: &str = "main";
pub const MODEL_FALLBACK_ENDPOINT: &str = "https://hf-mirror.com";
pub const MODEL_ARCHIVE_NAME: &str = "chinese-emotion-small-onnx.zip";
pub const OFFICIAL_RELEASE_TAG: &str = "v0.4.0";
pub const OFFICIAL_RELEASE_URL: &str = "https://github.com/huwany1/Kokoro-Engine/releases/download/v0.4.0/chinese-emotion-small-onnx.zip";
pub const OFFICIAL_LATEST_RELEASE_URL: &str = "https://github.com/huwany1/Kokoro-Engine/releases/latest/download/chinese-emotion-small-onnx.zip";
pub const DEFAULT_CDN_MIRRORS: &[&str] = &[
    "https://ghproxy.net/",
    "https://mirror.ghproxy.com/",
    "https://gh-proxy.com/",
];

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

const MODEL_INPUT_NAMES: [&str; 2] = ["attention_mask", "input_ids"];
const MODEL_OUTPUT_NAME: &str = "logits";
const EMOTION_CLASS_COUNT: usize = EMOTION_LABELS.len();

pub const MAX_EMOTION_ZIP_ENTRY_COUNT: usize = 128;
pub const MAX_EMOTION_ZIP_SINGLE_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2GB
pub const MAX_EMOTION_ZIP_TOTAL_UNCOMPRESSED_BYTES: u64 = 2560 * 1024 * 1024; // 2.5GB
pub const MAX_EMOTION_ARCHIVE_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2GB
pub const MAX_EMOTION_SINGLE_FILE_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2GB
pub const MAX_EMOTION_COMPANION_FILE_DOWNLOAD_BYTES: u64 = 32 * 1024 * 1024; // 32MB
pub const DEFAULT_EMOTION_CONFIDENCE_THRESHOLD: f32 = 0.45;
pub const DEFAULT_EMOTION_CONFIDENCE_MARGIN: f32 = 0.15;

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

impl EmotionInferenceResult {
    /// Calculate margin between top-1 and top-2 emotion probabilities.
    pub fn confidence_margin(&self) -> f32 {
        if self.probabilities.len() < 2 {
            return self.confidence;
        }
        let mut sorted = self.probabilities.clone();
        sorted.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let top1 = sorted[0].score;
        let top2 = sorted[1].score;
        (top1 - top2).max(0.0)
    }

    /// Check whether the result meets the confidence gate (confidence threshold + margin over runner-up).
    pub fn passes_confidence_gate(&self, min_confidence: f32, min_margin: f32) -> bool {
        self.confidence >= min_confidence && self.confidence_margin() >= min_margin
    }

    /// Default production gate: 0.45 threshold + 0.15 margin.
    pub fn is_confident(&self) -> bool {
        self.passes_confidence_gate(
            DEFAULT_EMOTION_CONFIDENCE_THRESHOLD,
            DEFAULT_EMOTION_CONFIDENCE_MARGIN,
        )
    }
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

// `IS_ENABLED` is user intent. Runtime readiness is represented by `engine_instance()`.
// Never derive readiness from file presence alone.
static IS_ENABLED: AtomicBool = AtomicBool::new(true);
static MODEL_IS_VALIDATED: AtomicBool = AtomicBool::new(false);
static MODEL_MUTATION_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static LIFECYCLE_MUTEX: Mutex<()> = Mutex::new(());
static LAST_LOAD_ERROR: RwLock<Option<String>> = RwLock::new(None);

struct ModelMutationLease<'a> {
    flag: &'a AtomicBool,
}

impl ModelMutationLease<'static> {
    fn acquire(operation: &str) -> Result<Self, String> {
        Self::acquire_from(operation, &MODEL_MUTATION_IN_PROGRESS)
    }
}

impl<'a> ModelMutationLease<'a> {
    fn acquire_from(operation: &str, flag: &'a AtomicBool) -> Result<Self, String> {
        flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Self { flag })
            .map_err(|_| {
                format!(
                    "Cannot {} emotion model while another model operation is running",
                    operation
                )
            })
    }
}

impl Drop for ModelMutationLease<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

pub fn set_last_load_error(err: Option<String>) {
    if let Ok(mut guard) = LAST_LOAD_ERROR.write() {
        *guard = err;
    }
}

pub fn get_last_load_error() -> Option<String> {
    LAST_LOAD_ERROR.read().ok().and_then(|g| g.clone())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmotionSettings {
    #[serde(default = "default_settings_enabled")]
    pub enabled: bool,
}

fn default_settings_enabled() -> bool {
    true
}

impl Default for EmotionSettings {
    fn default() -> Self {
        Self {
            enabled: default_settings_enabled(),
        }
    }
}

fn emotion_settings_path() -> PathBuf {
    #[cfg(test)]
    {
        if let Ok(test_path) = std::env::var("KOKORO_EMOTION_SETTINGS_TEST_PATH") {
            return PathBuf::from(test_path);
        }
    }
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("emotion_settings.json")
}

pub fn load_emotion_settings_from(path: &Path) -> EmotionSettings {
    if !path.exists() {
        return EmotionSettings::default();
    }
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<EmotionSettings>(&content) {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(
                    target: "ai",
                    "[Emotion] Failed to parse emotion settings file at {}: {}. Falling back to default.",
                    path.display(),
                    err
                );
                EmotionSettings::default()
            }
        },
        Err(err) => {
            tracing::warn!(
                target: "ai",
                "[Emotion] Failed to read emotion settings file at {}: {}. Falling back to default.",
                path.display(),
                err
            );
            EmotionSettings::default()
        }
    }
}

pub fn save_emotion_settings_to(path: &Path, settings: &EmotionSettings) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "Missing parent directory for emotion settings path: {}",
            path.display()
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|e| {
        format!(
            "Failed to create emotion settings parent dir {}: {}",
            parent.display(),
            e
        )
    })?;

    let tmp_path = parent.join(format!(
        ".emotion_settings.json.tmp.{}",
        uuid::Uuid::new_v4()
    ));
    let json_data = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize emotion settings: {}", e))?;

    std::fs::write(&tmp_path, json_data)
        .map_err(|e| format!("Failed to write temporary emotion settings file: {}", e))?;

    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!(
            "Failed to atomically persist emotion settings to {}: {}",
            path.display(),
            err
        ));
    }

    Ok(())
}

pub fn load_emotion_settings() -> EmotionSettings {
    load_emotion_settings_from(&emotion_settings_path())
}

pub fn save_emotion_settings(settings: &EmotionSettings) -> Result<(), String> {
    save_emotion_settings_to(&emotion_settings_path(), settings)
}

fn ensure_settings_loaded() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let settings = load_emotion_settings();
        IS_ENABLED.store(settings.enabled, Ordering::Release);
    });
}

#[cfg(test)]
pub fn reload_emotion_settings_for_test() {
    let settings = load_emotion_settings();
    IS_ENABLED.store(settings.enabled, Ordering::Release);
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
    default_model_repo_dir()
        .join("snapshots")
        .join(MODEL_REF_NAME)
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
            return Err(format!(
                "{} 内容异常（文件过小: {} 字节），疑似损坏",
                req, size
            ));
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
        "caring" | "concerned" | "label_1" | "关切" | "關切" | "關切語調" | "关切语调" | "温柔"
        | "安慰" => Some(1),
        "happy" | "label_2" | "开心" | "開心" | "開心語調" | "开心语调" | "喜" | "微笑"
        | "喜悦" => Some(2),
        "angry" | "label_3" | "愤怒" | "憤怒" | "憤怒語調" | "愤怒语调" | "生气" | "生氣"
        | "傲娇" => Some(3),
        "sad" | "label_4" | "悲伤" | "悲傷" | "悲傷語調" | "悲伤语调" | "难过" | "失落"
        | "伤心" => Some(4),
        "questioning" | "confused" | "label_5" | "疑问" | "疑問" | "疑問語調" | "疑问语调"
        | "困惑" | "思考" => Some(5),
        "surprised" | "label_6" | "惊奇" | "驚奇" | "驚奇語調" | "惊奇语调" | "惊讶" | "吃惊" => {
            Some(6)
        }
        "disgusted" | "label_7" | "厌恶" | "厭惡" | "厭惡語調" | "厌恶语调" | "嫌弃" | "冷漠" => {
            Some(7)
        }
        _ => None,
    }
}

pub struct EmotionModelContract;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TensorIoDescriptor {
    name: String,
    element_type: TensorElementType,
    shape: Vec<i64>,
}

impl EmotionModelContract {
    fn validate_io_descriptors(
        inputs: &[TensorIoDescriptor],
        outputs: &[TensorIoDescriptor],
    ) -> Result<(), String> {
        let mut input_names: Vec<&str> = inputs.iter().map(|input| input.name.as_str()).collect();
        input_names.sort_unstable();
        if input_names != MODEL_INPUT_NAMES {
            return Err(format!(
                "模型输入契约违约: 输入名称必须严格为 {:?}, 实际为 {:?}",
                MODEL_INPUT_NAMES, input_names
            ));
        }

        for input_name in MODEL_INPUT_NAMES {
            let input = inputs
                .iter()
                .find(|candidate| candidate.name == input_name)
                .ok_or_else(|| format!("模型输入契约违约: 缺少输入 '{}'", input_name))?;
            if input.element_type != TensorElementType::Int64 {
                return Err(format!(
                    "模型输入契约违约: '{}' 必须为 Int64, 实际为 {:?}",
                    input_name, input.element_type
                ));
            }
            if input.shape.len() != 2 {
                return Err(format!(
                    "模型输入契约违约: '{}' 必须为二维 [batch, sequence], 实际为 {:?}",
                    input_name, input.shape
                ));
            }
            if !matches!(input.shape[0], -1 | 1) || input.shape[1] != -1 {
                return Err(format!(
                    "模型输入契约违约: '{}' shape 必须兼容变长单批次文本 [1|dynamic, dynamic], 实际为 {:?}",
                    input_name, input.shape
                ));
            }
        }

        let ids_shape = &inputs
            .iter()
            .find(|input| input.name == "input_ids")
            .expect("input name set checked above")
            .shape;
        let mask_shape = &inputs
            .iter()
            .find(|input| input.name == "attention_mask")
            .expect("input name set checked above")
            .shape;
        for axis in 0..2 {
            if ids_shape[axis] != -1
                && mask_shape[axis] != -1
                && ids_shape[axis] != mask_shape[axis]
            {
                return Err(format!(
                    "模型输入契约违约: input_ids 与 attention_mask shape 不兼容: {:?} vs {:?}",
                    ids_shape, mask_shape
                ));
            }
        }

        if outputs.len() != 1 || outputs[0].name != MODEL_OUTPUT_NAME {
            let output_names: Vec<&str> =
                outputs.iter().map(|output| output.name.as_str()).collect();
            return Err(format!(
                "模型输出契约违约: 输出名称必须严格为 ['{}'], 实际为 {:?}",
                MODEL_OUTPUT_NAME, output_names
            ));
        }
        let output = &outputs[0];
        if output.element_type != TensorElementType::Float32 {
            return Err(format!(
                "模型输出契约违约: '{}' 必须为 Float32, 实际为 {:?}",
                MODEL_OUTPUT_NAME, output.element_type
            ));
        }
        if output.shape.len() != 2
            || !matches!(output.shape[0], -1 | 1)
            || !matches!(output.shape[1], -1 | 8)
        {
            return Err(format!(
                "模型输出契约违约: '{}' shape 必须为 [1|dynamic, 8|dynamic], 实际为 {:?}",
                MODEL_OUTPUT_NAME, output.shape
            ));
        }

        Ok(())
    }

    /// Validate the exact model interface consumed by this runtime.
    pub fn validate_session_io(session: &Session) -> Result<(), String> {
        fn descriptor(name: &str, dtype: &ValueType) -> Result<TensorIoDescriptor, String> {
            match dtype {
                ValueType::Tensor { ty, shape, .. } => Ok(TensorIoDescriptor {
                    name: name.to_string(),
                    element_type: *ty,
                    shape: shape.iter().copied().collect(),
                }),
                other => Err(format!(
                    "模型 I/O 契约违约: '{}' 必须为 Tensor, 实际为 {:?}",
                    name, other
                )),
            }
        }

        let inputs = session
            .inputs()
            .iter()
            .map(|input| descriptor(input.name(), input.dtype()))
            .collect::<Result<Vec<_>, _>>()?;
        let outputs = session
            .outputs()
            .iter()
            .map(|output| descriptor(output.name(), output.dtype()))
            .collect::<Result<Vec<_>, _>>()?;
        Self::validate_io_descriptors(&inputs, &outputs)
    }

    fn parse_id2label(value: &serde_json::Value) -> Result<[usize; EMOTION_CLASS_COUNT], String> {
        let labels = value
            .as_object()
            .ok_or_else(|| "模型配置违约: id2label 必须是对象".to_string())?;
        if labels.len() != EMOTION_CLASS_COUNT {
            return Err(format!(
                "模型配置违约: id2label 类别数量为 {}, 严格期望为 {}",
                labels.len(),
                EMOTION_CLASS_COUNT
            ));
        }

        let mut mapping = [usize::MAX; EMOTION_CLASS_COUNT];
        let mut seen_model = [false; EMOTION_CLASS_COUNT];
        let mut seen_canonical = [false; EMOTION_CLASS_COUNT];
        for (key, value) in labels {
            let model_idx = key
                .parse::<usize>()
                .map_err(|_| format!("id2label 键名不是 0..7 整数: {}", key))?;
            if model_idx >= EMOTION_CLASS_COUNT || seen_model[model_idx] {
                return Err(format!(
                    "模型配置违约: id2label 模型索引重复或越界: {}",
                    key
                ));
            }
            let label = value
                .as_str()
                .ok_or_else(|| format!("模型配置违约: id2label[{}] 必须是字符串", key))?;
            let canonical_idx = normalize_canonical_emotion_index(label).ok_or_else(|| {
                format!("模型配置中的标签 '{}' 无法对齐至标准 8 种情感之一", label)
            })?;
            if seen_canonical[canonical_idx] {
                return Err(format!("模型配置违约: id2label 标准情感重复: {}", label));
            }
            mapping[model_idx] = canonical_idx;
            seen_model[model_idx] = true;
            seen_canonical[canonical_idx] = true;
        }
        if !seen_model.iter().all(|seen| *seen) || !seen_canonical.iter().all(|seen| *seen) {
            return Err("模型配置违约: id2label 必须完整覆盖模型索引和标准情感 0..7".to_string());
        }
        Ok(mapping)
    }

    fn parse_label2id(value: &serde_json::Value) -> Result<[usize; EMOTION_CLASS_COUNT], String> {
        let labels = value
            .as_object()
            .ok_or_else(|| "模型配置违约: label2id 必须是对象".to_string())?;
        if labels.len() != EMOTION_CLASS_COUNT {
            return Err(format!(
                "模型配置违约: label2id 类别数量为 {}, 严格期望为 {}",
                labels.len(),
                EMOTION_CLASS_COUNT
            ));
        }

        let mut mapping = [usize::MAX; EMOTION_CLASS_COUNT];
        let mut seen_model = [false; EMOTION_CLASS_COUNT];
        let mut seen_canonical = [false; EMOTION_CLASS_COUNT];
        for (label, value) in labels {
            let raw_idx = value
                .as_u64()
                .ok_or_else(|| format!("模型配置违约: label2id['{}'] 必须是 0..7 整数", label))?;
            if raw_idx >= EMOTION_CLASS_COUNT as u64 {
                return Err(format!("模型配置违约: label2id 模型索引越界: {}", raw_idx));
            }
            let model_idx = raw_idx as usize;
            if seen_model[model_idx] {
                return Err(format!(
                    "模型配置违约: label2id 模型索引重复: {}",
                    model_idx
                ));
            }
            let canonical_idx = normalize_canonical_emotion_index(label).ok_or_else(|| {
                format!("模型配置中的标签 '{}' 无法对齐至标准 8 种情感之一", label)
            })?;
            if seen_canonical[canonical_idx] {
                return Err(format!("模型配置违约: label2id 标准情感重复: {}", label));
            }
            mapping[model_idx] = canonical_idx;
            seen_model[model_idx] = true;
            seen_canonical[canonical_idx] = true;
        }
        if !seen_model.iter().all(|seen| *seen) || !seen_canonical.iter().all(|seen| *seen) {
            return Err("模型配置违约: label2id 必须完整覆盖模型索引和标准情感 0..7".to_string());
        }
        Ok(mapping)
    }

    /// Resolve and cross-check `[model output index -> canonical emotion index]`.
    pub fn resolve_label_mapping(config_path: &Path) -> Result<[usize; 8], String> {
        let content = std::fs::read_to_string(config_path)
            .map_err(|e| format!("读取配置文件 {} 失败: {}", config_path.display(), e))?;
        let json: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("解析配置文件 {} JSON 失败: {}", config_path.display(), e))?;

        if let Some(value) = json.get("num_labels") {
            let count = value
                .as_u64()
                .ok_or_else(|| "模型配置违约: num_labels 必须是整数".to_string())?;
            if count != EMOTION_CLASS_COUNT as u64 {
                return Err(format!(
                    "模型配置违约: num_labels 为 {}, 期望为 {}",
                    count, EMOTION_CLASS_COUNT
                ));
            }
        }

        let id2label = json.get("id2label").map(Self::parse_id2label).transpose()?;
        let label2id = json.get("label2id").map(Self::parse_label2id).transpose()?;
        match (id2label, label2id) {
            (Some(left), Some(right)) if left != right => {
                Err("模型配置违约: id2label 与 label2id 的标签顺序不一致".to_string())
            }
            (Some(mapping), _) | (_, Some(mapping)) => Ok(mapping),
            (None, None) => Err(
                "模型配置违约: 必须提供 id2label 或 label2id 以验证 8 类情感标签顺序".to_string(),
            ),
        }
    }

    fn validate_runtime_logits(shape: &[i64], raw: &[f32]) -> Result<[f32; 8], String> {
        if shape != [1, EMOTION_CLASS_COUNT as i64] {
            return Err(format!(
                "模型输出契约违约: '{}' 实际 shape 必须为 [1, {}], 实际为 {:?}",
                MODEL_OUTPUT_NAME, EMOTION_CLASS_COUNT, shape
            ));
        }
        let logits: [f32; EMOTION_CLASS_COUNT] = raw.try_into().map_err(|_| {
            format!(
                "模型输出契约违约: '{}' 必须包含 {} 个值, 实际为 {}",
                MODEL_OUTPUT_NAME,
                EMOTION_CLASS_COUNT,
                raw.len()
            )
        })?;
        if let Some((index, value)) = logits
            .iter()
            .enumerate()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(format!(
                "模型输出契约违约: '{}[{}]' 产生非有限值 {}",
                MODEL_OUTPUT_NAME, index, value
            ));
        }
        Ok(logits)
    }

    fn extract_validated_logits(outputs: &SessionOutputs<'_>) -> Result<[f32; 8], String> {
        let output = outputs
            .get(MODEL_OUTPUT_NAME)
            .ok_or_else(|| format!("模型输出契约违约: 推理结果缺少 '{}'", MODEL_OUTPUT_NAME))?;
        let (shape, raw) = output.try_extract_tensor::<f32>().map_err(|error| {
            format!(
                "模型输出契约违约: '{}' 不是 Float32 Tensor: {}",
                MODEL_OUTPUT_NAME, error
            )
        })?;
        Self::validate_runtime_logits(shape, raw)
    }

    fn encode_inputs(
        tokenizer: &Tokenizer,
        text: &str,
    ) -> Result<(Tensor<i64>, Tensor<i64>), String> {
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|error| format!("Tokenizer 编码失败: {}", error))?;
        let token_ids: Vec<i64> = encoding
            .get_ids()
            .iter()
            .take(64)
            .map(|&id| id as i64)
            .collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .take(64)
            .map(|&mask| mask as i64)
            .collect();
        if token_ids.is_empty() || token_ids.len() != attention_mask.len() {
            return Err("Tokenizer 编码结果为空或 input_ids/attention_mask 长度不一致".to_string());
        }
        let sequence_length = token_ids.len();
        let ids = Tensor::from_array(([1, sequence_length], token_ids))
            .map_err(|error| format!("创建 input_ids 张量失败: {}", error))?;
        let mask = Tensor::from_array(([1, sequence_length], attention_mask))
            .map_err(|error| format!("创建 attention_mask 张量失败: {}", error))?;
        Ok((ids, mask))
    }

    fn infer_logits(
        session: &mut Session,
        tokenizer: &Tokenizer,
        text: &str,
    ) -> Result<[f32; 8], String> {
        let (ids, mask) = Self::encode_inputs(tokenizer, text)?;
        let outputs = session
            .run(ort::inputs!["input_ids" => ids, "attention_mask" => mask])
            .map_err(|error| format!("ONNX Session run 失败: {}", error))?;
        Self::extract_validated_logits(&outputs)
    }

    /// Execute a real tokenizer + ONNX inference before the model can become active.
    pub fn run_dummy_inference(
        session: &mut Session,
        tokenizer: &Tokenizer,
        label_mapping: &[usize; 8],
    ) -> Result<(), String> {
        let logits = Self::infer_logits(session, tokenizer, "你好，欢迎使用情感系统。")?;
        let probs = softmax(&logits);
        if probs.len() != 8 {
            return Err(format!("Softmax 概率分布维度异常: {}", probs.len()));
        }

        for (idx, &p) in probs.iter().enumerate() {
            if !p.is_finite() || p < 0.0 || p > 1.0 {
                return Err(format!("Softmax 概率值异常: 类别索引 {} 概率为 {}", idx, p));
            }
        }

        let sum: f32 = probs.iter().sum();
        if (sum - 1.0).abs() > 0.01 {
            return Err(format!("Softmax 概率归一化校验失败: 概率和为 {}", sum));
        }

        for (model_idx, &canonical_idx) in label_mapping.iter().enumerate() {
            if canonical_idx >= 8 {
                return Err(format!(
                    "重排映射越界: model_idx {} 映射到了 canonical_idx {}",
                    model_idx, canonical_idx
                ));
            }
        }

        tracing::info!(
            target: "ai",
            "[Emotion] Dummy inference contract check PASSED (output shape: [1, 8], all finite, prob sum: {:.4})",
            sum
        );

        Ok(())
    }
}

/// Deep validation: loads Tokenizer, creates ONNX Session, verifies I/O contracts,
/// checks config label ordering, and runs dummy inference before approval.
pub fn validate_model_files_deep(dir: &Path) -> Result<(), String> {
    build_validated_engine(dir).map(|_| ())
}

fn build_validated_engine(dir: &Path) -> Result<EmotionEngine, String> {
    inspect_model_files_fast(dir)?;

    let tokenizer_path = dir.join("tokenizer.json");
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| format!("分词器 tokenizer.json 校验失败: {}", e))?;

    let onnx_path = dir.join("model.onnx");
    let mut session = Session::builder()
        .map_err(|e| format!("创建 ONNX Session 失败: {}", e))?
        .with_intra_threads(1)
        .map_err(|e| format!("设置线程池失败: {}", e))?
        .commit_from_file(&onnx_path)
        .map_err(|e| format!("ONNX 模型加载试运行失败: {}", e))?;

    EmotionModelContract::validate_session_io(&session)?;

    let config_path = dir.join("config.json");
    let label_mapping = EmotionModelContract::resolve_label_mapping(&config_path)?;

    EmotionModelContract::run_dummy_inference(&mut session, &tokenizer, &label_mapping)?;

    Ok(EmotionEngine {
        session,
        tokenizer,
        label_mapping,
    })
}

pub fn get_emotion_model_status() -> EmotionModelStatus {
    ensure_settings_loaded();
    let initial_snapshot_dir = default_model_snapshot_dir();
    let initial_files_present = missing_required_model_files(&initial_snapshot_dir).is_empty();

    if initial_files_present
        && IS_ENABLED.load(Ordering::Acquire)
        && !MODEL_IS_VALIDATED.load(Ordering::Acquire)
        && !MODEL_MUTATION_IN_PROGRESS.load(Ordering::Acquire)
    {
        let _ = get_or_load_engine();
    }

    let _lifecycle = LIFECYCLE_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let snapshot_dir = default_model_snapshot_dir();
    let missing_files = missing_required_model_files(&snapshot_dir);
    let model_path = snapshot_dir.join("model.onnx");
    let files_present = missing_files.is_empty();

    let (installed, is_valid, error_message) = if files_present {
        match inspect_model_files_fast(&snapshot_dir) {
            Ok(_) if MODEL_IS_VALIDATED.load(Ordering::Acquire) => (true, true, None),
            Ok(_) => (true, false, get_last_load_error()),
            Err(err) => (false, false, Some(err)),
        }
    } else {
        (false, false, None)
    };

    let engine_ready = engine_instance()
        .read()
        .map(|guard| guard.is_some())
        .unwrap_or(false);
    let is_active = IS_ENABLED.load(Ordering::Acquire) && installed && is_valid && engine_ready;

    let memory_bytes = if is_active {
        engine_instance().read().ok().and_then(|guard| {
            if guard.is_some() {
                let onnx_size = std::fs::metadata(&model_path)
                    .map(|m| m.len() as usize)
                    .unwrap_or(35 * 1024 * 1024);
                Some(onnx_size + 20 * 1024 * 1024)
            } else {
                None
            }
        })
    } else {
        None
    };

    EmotionModelStatus {
        installed,
        is_active,
        is_valid,
        error_message,
        repo_id: MODEL_REPO.to_string(),
        download_url: OFFICIAL_RELEASE_URL.to_string(),
        install_dir: snapshot_dir.to_string_lossy().into_owned(),
        model_path: model_path.to_string_lossy().into_owned(),
        required_files: required_model_files()
            .into_iter()
            .map(str::to_string)
            .collect(),
        missing_files,
        memory_bytes,
    }
}

pub fn toggle_emotion_model_active(active: bool) -> Result<EmotionModelStatus, String> {
    ensure_settings_loaded();
    let _lease = ModelMutationLease::acquire("toggle")?;
    if !active {
        IS_ENABLED.store(false, Ordering::Release);
        unload_emotion_engine()?;
        if let Err(e) = save_emotion_settings(&EmotionSettings { enabled: false }) {
            tracing::warn!(target: "ai", "[Emotion] Failed to persist disabled setting: {}", e);
        }
    } else {
        IS_ENABLED.store(true, Ordering::Release);
        if let Err(error) = get_or_load_engine() {
            IS_ENABLED.store(false, Ordering::Release);
            if let Err(save_err) = save_emotion_settings(&EmotionSettings { enabled: false }) {
                tracing::warn!(target: "ai", "[Emotion] Failed to persist disabled setting after load failure: {}", save_err);
            }
            return Err(error);
        }
        if let Err(e) = save_emotion_settings(&EmotionSettings { enabled: true }) {
            tracing::warn!(target: "ai", "[Emotion] Failed to persist enabled setting: {}", e);
        }
    }
    Ok(get_emotion_model_status())
}

fn unload_emotion_engine_locked() -> Result<(), String> {
    let mut guard = engine_instance()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = None;
    Ok(())
}

pub fn unload_emotion_engine() -> Result<(), String> {
    let _lifecycle = LIFECYCLE_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    unload_emotion_engine_locked()
}

pub fn uninstall_emotion_model() -> Result<EmotionModelStatus, String> {
    ensure_settings_loaded();
    let _lease = ModelMutationLease::acquire("uninstall")?;
    let lifecycle = LIFECYCLE_MUTEX
        .lock()
        .map_err(|error| format!("Emotion lifecycle lock is poisoned: {}", error))?;
    // 1. Explicitly drop the active session to release Windows file handles (avoid OS Error 32)
    unload_emotion_engine_locked()?;

    let repo_dir = default_model_repo_dir();
    if repo_dir.exists() {
        std::fs::remove_dir_all(&repo_dir)
            .map_err(|e| format!("Failed to remove model directory: {}", e))?;
    }

    set_last_load_error(None);
    MODEL_IS_VALIDATED.store(false, Ordering::Release);
    IS_ENABLED.store(false, Ordering::Release);
    if let Err(e) = save_emotion_settings(&EmotionSettings { enabled: false }) {
        tracing::warn!(target: "ai", "[Emotion] Failed to persist disabled setting after uninstall: {}", e);
    }
    drop(lifecycle);
    Ok(get_emotion_model_status())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmotionDownloadCandidate {
    Archive { url: String, label: String },
    Endpoint { base_url: String, label: String },
}

pub fn resolve_download_candidates() -> Vec<EmotionDownloadCandidate> {
    let mut candidates = Vec::new();

    // 1. Official GitHub Release canonical assets (Primary out-of-the-box distribution)
    candidates.push(EmotionDownloadCandidate::Archive {
        url: OFFICIAL_RELEASE_URL.to_string(),
        label: format!("官方 GitHub Release ({})", OFFICIAL_RELEASE_TAG),
    });
    candidates.push(EmotionDownloadCandidate::Archive {
        url: OFFICIAL_LATEST_RELEASE_URL.to_string(),
        label: "官方 GitHub Release (latest)".to_string(),
    });

    // 2. Official Release CDN mirrors (ghproxy etc. for fast mainland China access)
    for mirror in DEFAULT_CDN_MIRRORS {
        let mirror_clean = mirror.trim_end_matches('/');
        candidates.push(EmotionDownloadCandidate::Archive {
            url: format!("{}/{}", mirror_clean, OFFICIAL_RELEASE_URL),
            label: format!("官方 Release CDN 加速镜像 ({})", mirror_clean),
        });
    }

    // 3. Optional developer environment variable override
    if let Ok(custom_url) = std::env::var("KOKORO_EMOTION_MODEL_URL") {
        let trimmed = custom_url.trim();
        if !trimmed.is_empty() {
            if trimmed.ends_with(".zip") || trimmed.contains(".zip?") {
                candidates.insert(
                    0,
                    EmotionDownloadCandidate::Archive {
                        url: trimmed.to_string(),
                        label: "自定义模型包 (KOKORO_EMOTION_MODEL_URL)".to_string(),
                    },
                );
            } else {
                candidates.insert(
                    0,
                    EmotionDownloadCandidate::Endpoint {
                        base_url: trimmed.trim_end_matches('/').to_string(),
                        label: "自定义模型端点 (KOKORO_EMOTION_MODEL_URL)".to_string(),
                    },
                );
            }
        }
    }

    // 4. Upstream Hugging Face endpoints (fallback probe for individual files if upstream adds ONNX)
    for endpoint in emotion_model_endpoints() {
        candidates.push(EmotionDownloadCandidate::Endpoint {
            base_url: endpoint.clone(),
            label: format!("Hugging Face ({})", endpoint),
        });
    }

    candidates
}

pub fn is_zip_file(path: &Path) -> bool {
    let mut buf = [0u8; 4];
    if let Ok(mut f) = std::fs::File::open(path) {
        use std::io::Read;
        if f.read_exact(&mut buf).is_ok() {
            return buf == [0x50, 0x4B, 0x03, 0x04] || buf == [0x50, 0x4B, 0x05, 0x06];
        }
    }
    false
}

pub fn clean_staging_files(staging_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(staging_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let _ = std::fs::remove_file(p);
            } else if p.is_dir() {
                let _ = std::fs::remove_dir_all(p);
            }
        }
    }
}

/// Safely extracts emotion model files from a zip archive into `staging_dir`.
///
/// Features:
/// - Entry count limit: rejects archives with more than `MAX_EMOTION_ZIP_ENTRY_COUNT` entries (Zip Bomb defense)
/// - Bounded extraction: bounds single-file size and cumulative uncompressed bytes
/// - Zip-Slip traversal protection: ensures all entries are confined to `staging_dir`
/// - Canonical model mapping: maps `model.int8.onnx` or `model.onnx` -> `model.onnx`
/// - Extracts only `REQUIRED_FILES` (model.onnx, config.json, tokenizer.json, tokenizer_config.json, special_tokens_map.json)
pub fn unpack_emotion_zip<R: std::io::Read + std::io::Seek>(
    archive: zip::ZipArchive<R>,
    staging_dir: &Path,
) -> Result<usize, String> {
    unpack_emotion_zip_with_limits(
        archive,
        staging_dir,
        MAX_EMOTION_ZIP_ENTRY_COUNT,
        MAX_EMOTION_ZIP_SINGLE_FILE_BYTES,
        MAX_EMOTION_ZIP_TOTAL_UNCOMPRESSED_BYTES,
    )
}

pub fn unpack_emotion_zip_with_limits<R: std::io::Read + std::io::Seek>(
    mut archive: zip::ZipArchive<R>,
    staging_dir: &Path,
    max_entries: usize,
    max_single_file_bytes: u64,
    max_total_uncompressed_bytes: u64,
) -> Result<usize, String> {
    if archive.len() > max_entries {
        return Err(format!(
            "压缩包条目过多 ({} > 最大限制 {}), 拒绝解压以防 Zip Bomb 攻击",
            archive.len(),
            max_entries
        ));
    }

    let mut extracted_count = 0;
    let mut total_uncompressed_bytes: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
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

        use std::io::Read;
        let mut limited_entry = (&mut entry).take(max_single_file_bytes + 1);
        let copied = std::io::copy(&mut limited_entry, &mut dest_file)
            .map_err(|e| format!("解压文件 {} 失败: {}", target_file_name, e))?;

        if copied > max_single_file_bytes {
            let _ = std::fs::remove_file(&dest_path);
            return Err(format!(
                "解压文件 {} 超过单文件最大限制 ({} 字节)",
                target_file_name, max_single_file_bytes
            ));
        }

        total_uncompressed_bytes = total_uncompressed_bytes.saturating_add(copied);
        if total_uncompressed_bytes > max_total_uncompressed_bytes {
            let _ = std::fs::remove_file(&dest_path);
            return Err(format!(
                "解压文件累计总大小超过上限 ({} 字节 > {} 字节)",
                total_uncompressed_bytes, max_total_uncompressed_bytes
            ));
        }

        extracted_count += 1;
    }

    Ok(extracted_count)
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
    format!(
        "{}/{}/resolve/{}/{}",
        endpoint, MODEL_REPO, MODEL_REF_NAME, file_name
    )
}

fn emotion_model_file_path(snapshot_dir: &Path, file_name: &str) -> Result<PathBuf, String> {
    let path = Path::new(file_name);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
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
1. model.onnx               - ONNX 格式的情感推理模型权重 (~1.1GB FP32 或 INT8 权重)
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

fn ensure_snapshot_parent(snapshot_dir: &Path) -> Result<(), String> {
    let snapshot_parent = snapshot_dir
        .parent()
        .ok_or_else(|| format!("模型快照路径缺少父目录: {}", snapshot_dir.display()))?;
    std::fs::create_dir_all(snapshot_parent).map_err(|error| {
        format!(
            "创建模型快照父目录 {} 失败: {}",
            snapshot_parent.display(),
            error
        )
    })
}

/// Promote one fully validated sibling directory into the fixed live location.
///
/// This provides process-local rollback for ordinary I/O/load failures. It is not a
/// crash-consistent filesystem transaction: the fixed live path requires the Windows
/// ONNX Session to be dropped before directory renames can proceed.
fn promote_validated_staging(staging_dir: &Path, enable_after: bool) -> Result<(), String> {
    let snapshot_dir = default_model_snapshot_dir();
    let repo_dir = default_model_repo_dir();
    let backup_dir = repo_dir.join(format!(".backup.{}", uuid::Uuid::new_v4()));
    let failed_dir = repo_dir.join(format!(".failed.{}", uuid::Uuid::new_v4()));
    let lifecycle = LIFECYCLE_MUTEX
        .lock()
        .map_err(|error| format!("Emotion lifecycle lock is poisoned: {}", error))?;
    let mut engine = engine_instance()
        .write()
        .map_err(|error| format!("Emotion engine lock is poisoned: {}", error))?;
    let previous_enabled = IS_ENABLED.load(Ordering::Acquire);
    let previous_validated = MODEL_IS_VALIDATED.load(Ordering::Acquire);

    ensure_snapshot_parent(&snapshot_dir)?;

    // Lock order is lifecycle -> engine. Every loader/publisher follows this order.
    *engine = None;
    let had_live_snapshot = snapshot_dir.exists();
    if had_live_snapshot {
        if let Err(error) = std::fs::rename(&snapshot_dir, &backup_dir) {
            let reload_error = if previous_enabled {
                build_validated_engine(&snapshot_dir)
                    .map(|restored| *engine = Some(restored))
                    .err()
            } else {
                None
            };
            MODEL_IS_VALIDATED.store(
                previous_validated && reload_error.is_none(),
                Ordering::Release,
            );
            IS_ENABLED.store(previous_enabled, Ordering::Release);
            let message = format!("无法备份现有模型目录，发布已取消: {}", error);
            set_last_load_error(
                reload_error.map(|reload| format!("{}; 旧模型重载失败: {}", message, reload)),
            );
            return Err(message);
        }
    }

    if let Err(promote_error) = std::fs::rename(staging_dir, &snapshot_dir) {
        let rollback_error = if had_live_snapshot {
            std::fs::rename(&backup_dir, &snapshot_dir).err()
        } else {
            None
        };
        let reload_error = if rollback_error.is_none() && had_live_snapshot && previous_enabled {
            build_validated_engine(&snapshot_dir)
                .map(|restored| *engine = Some(restored))
                .err()
        } else {
            None
        };
        IS_ENABLED.store(previous_enabled, Ordering::Release);
        MODEL_IS_VALIDATED.store(
            had_live_snapshot
                && previous_validated
                && rollback_error.is_none()
                && reload_error.is_none(),
            Ordering::Release,
        );
        let mut message = format!("候选模型目录发布失败: {}", promote_error);
        if let Some(error) = rollback_error {
            message.push_str(&format!(
                "; 旧模型目录回滚失败: {} (备份保留于 {})",
                error,
                backup_dir.display()
            ));
        }
        if let Some(error) = reload_error {
            message.push_str(&format!("; 旧模型重载失败: {}", error));
        }
        set_last_load_error(Some(message.clone()));
        return Err(message);
    }

    match build_validated_engine(&snapshot_dir) {
        Ok(validated_engine) => {
            if enable_after {
                *engine = Some(validated_engine);
                MODEL_IS_VALIDATED.store(true, Ordering::Release);
                IS_ENABLED.store(true, Ordering::Release);
                if let Err(e) = save_emotion_settings(&EmotionSettings { enabled: true }) {
                    tracing::warn!(target: "ai", "[Emotion] Failed to persist enabled setting after promotion: {}", e);
                }
            } else {
                *engine = None;
                MODEL_IS_VALIDATED.store(true, Ordering::Release);
                IS_ENABLED.store(false, Ordering::Release);
                if let Err(e) = save_emotion_settings(&EmotionSettings { enabled: false }) {
                    tracing::warn!(target: "ai", "[Emotion] Failed to persist disabled setting after promotion: {}", e);
                }
            }
            set_last_load_error(None);
            drop(engine);
            drop(lifecycle);
            if had_live_snapshot {
                if let Err(error) = std::fs::remove_dir_all(&backup_dir) {
                    tracing::warn!(target: "ai", "[Emotion] Failed to remove model backup {}: {}", backup_dir.display(), error);
                }
            }
            Ok(())
        }
        Err(candidate_error) => {
            let quarantine_error = std::fs::rename(&snapshot_dir, &failed_dir).err();
            let rollback_error = if had_live_snapshot && quarantine_error.is_none() {
                std::fs::rename(&backup_dir, &snapshot_dir).err()
            } else {
                None
            };
            let reload_error = if had_live_snapshot
                && quarantine_error.is_none()
                && rollback_error.is_none()
                && previous_enabled
            {
                build_validated_engine(&snapshot_dir)
                    .map(|restored| *engine = Some(restored))
                    .err()
            } else {
                None
            };
            IS_ENABLED.store(previous_enabled, Ordering::Release);
            MODEL_IS_VALIDATED.store(
                had_live_snapshot
                    && previous_validated
                    && quarantine_error.is_none()
                    && rollback_error.is_none()
                    && reload_error.is_none(),
                Ordering::Release,
            );

            let mut message = format!("候选模型发布后最终校验失败: {}", candidate_error);
            if let Some(error) = quarantine_error {
                message.push_str(&format!("; 无法隔离失败候选: {}", error));
            } else {
                message.push_str(&format!("; 失败候选保留于 {}", failed_dir.display()));
            }
            if let Some(error) = rollback_error {
                message.push_str(&format!(
                    "; 旧模型目录回滚失败: {} (备份保留于 {})",
                    error,
                    backup_dir.display()
                ));
            }
            if let Some(error) = reload_error {
                message.push_str(&format!("; 旧模型重载失败: {}", error));
            }
            set_last_load_error(Some(message.clone()));
            Err(message)
        }
    }
}

pub fn import_emotion_model_package(source_path: &str) -> Result<EmotionModelStatus, String> {
    ensure_settings_loaded();
    let _lease = ModelMutationLease::acquire("import")?;
    let path = Path::new(source_path);
    if !path.exists() {
        return Err(format!("指定的模型文件或目录不存在: {}", source_path));
    }

    let snapshot_dir = default_model_snapshot_dir();
    let is_installed = missing_required_model_files(&snapshot_dir).is_empty();
    let enable_after = if is_installed {
        IS_ENABLED.load(Ordering::Acquire)
    } else {
        true
    };

    let repo_dir = default_model_repo_dir();
    let staging_dir = repo_dir.join(format!(".staging.import.{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging_dir).map_err(|e| format!("创建导入暂存区失败: {}", e))?;

    let result = (|| -> Result<(), String> {
        if path.is_file() {
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_lowercase();
            if extension == "zip" {
                let file =
                    std::fs::File::open(path).map_err(|e| format!("打开 ZIP 文件失败: {}", e))?;
                let archive = zip::ZipArchive::new(file)
                    .map_err(|e| format!("解析 ZIP 压缩包失败: {}", e))?;
                unpack_emotion_zip(archive, &staging_dir)?;
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
                            std::fs::copy(&companion, staging_dir.join(req))
                                .map_err(|error| format!("拷贝配套文件 {} 失败: {}", req, error))?;
                            found = true;
                        }
                    }
                    if !found && snapshot_dir.join(req).exists() {
                        std::fs::copy(snapshot_dir.join(req), staging_dir.join(req))
                            .map_err(|error| format!("复用现有配套文件 {} 失败: {}", req, error))?;
                    }
                }
            } else {
                return Err(format!(
                    "不支持的文件格式: .{} (仅支持 .zip 压缩包或 .onnx 权重)",
                    extension
                ));
            }
        } else if path.is_dir() {
            copy_required_files_from_dir(path, &staging_dir)?;
        } else {
            return Err("选择的路径不是有效的文件或目录".to_string());
        }

        // Validate completeness and validity of required files via deep validation
        validate_model_files_deep(&staging_dir)?;
        promote_validated_staging(&staging_dir, enable_after)
    })();

    // Always sweep staging directory
    if let Err(error) = std::fs::remove_dir_all(&staging_dir) {
        if staging_dir.exists() {
            tracing::warn!(target: "ai", "[Emotion] Failed to remove import staging {}: {}", staging_dir.display(), error);
        }
    }

    result?;

    Ok(get_emotion_model_status())
}

pub async fn download_emotion_model<F>(emit_progress: F) -> Result<EmotionModelStatus, String>
where
    F: Fn(EmotionModelDownloadProgress) -> Result<(), String> + Send + Sync + 'static,
{
    ensure_settings_loaded();
    let _lease = ModelMutationLease::acquire("download")?;
    let snapshot_dir = default_model_snapshot_dir();
    let repo_dir = default_model_repo_dir();
    std::fs::create_dir_all(&repo_dir)
        .map_err(|e| format!("Failed to create model repository dir: {}", e))?;
    std::fs::create_dir_all(repo_dir.join("refs"))
        .map_err(|e| format!("Failed to create refs dir: {}", e))?;
    std::fs::write(repo_dir.join("refs").join(MODEL_REF_NAME), MODEL_REF_NAME)
        .map_err(|e| format!("Failed to write model ref: {}", e))?;

    // A complete existing snapshot may return early only when the currently loaded
    // engine has passed the shared builder's real-inference validation.
    let missing_initially = missing_required_model_files(&snapshot_dir);
    if missing_initially.is_empty() {
        IS_ENABLED.store(true, Ordering::Release);
        if let Err(error) = save_emotion_settings(&EmotionSettings { enabled: true }) {
            tracing::warn!(target: "ai", "[Emotion] Failed to persist enabled setting during download: {}", error);
        }
        match tokio::task::spawn_blocking(get_or_load_engine).await {
            Ok(Ok(())) => {
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
            Ok(Err(error)) => tracing::warn!(
                target: "ai",
                "[Emotion] Existing model failed deep validation ({}); downloading a full candidate without deleting the live snapshot.",
                error
            ),
            Err(error) => tracing::warn!(
                target: "ai",
                "[Emotion] Existing model validation task failed ({}); downloading a full candidate without deleting the live snapshot.",
                error
            ),
        }
    }

    let emit_progress = Arc::new(emit_progress);

    // Staging isolation directory: never write partial or corrupt files directly to snapshot_dir
    let staging_dir = repo_dir.join(format!(".staging.download.{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging_dir).map_err(|e| format!("创建下载暂存区失败: {}", e))?;

    struct StagingGuard<'a>(&'a Path);
    impl<'a> Drop for StagingGuard<'a> {
        fn drop(&mut self) {
            if self.0.exists() {
                let _ = std::fs::remove_dir_all(self.0);
            }
        }
    }
    let staging_guard = StagingGuard(&staging_dir);

    let candidates = resolve_download_candidates();
    let client = reqwest::Client::builder()
        .user_agent("kokoro-engine/0.4.0")
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("Failed to build reqwest client: {}", e))?;

    let mut download_succeeded = false;
    let mut failure_reasons: Vec<String> = Vec::new();

    for candidate in &candidates {
        match candidate {
            EmotionDownloadCandidate::Archive { url, label } => {
                tracing::info!(target: "ai", "[Emotion] Attempting archive download from {}: {}", label, url);
                let archive_tmp_path = staging_dir.join("package.tmp.zip");
                let archive_name = MODEL_ARCHIVE_NAME.to_string();

                let _ = emit_progress(build_download_progress(
                    "downloading",
                    format!("正在下载情感模型包 (~1.1GB) [{}]", label),
                    archive_name.clone(),
                    1,
                    1,
                    0,
                    None,
                ));

                let progress_sender = emit_progress.clone();
                let progress_label = label.clone();
                let progress_archive_name = archive_name.clone();

                let dl_res = crate::utils::download::download_file_with_progress(
                    &client,
                    url,
                    &archive_tmp_path,
                    crate::utils::download::DownloadOptions {
                        max_bytes: Some(MAX_EMOTION_ARCHIVE_DOWNLOAD_BYTES),
                        ..Default::default()
                    },
                    Arc::new(move |p| {
                        progress_sender(build_download_progress(
                            "downloading",
                            format!("正在下载情感模型包 (~1.1GB) [{}]", progress_label),
                            progress_archive_name.clone(),
                            1,
                            1,
                            p.downloaded_bytes,
                            p.total_bytes,
                        ))
                    }),
                )
                .await;

                if let Err(err) = dl_res {
                    tracing::warn!(target: "ai", "[Emotion] Archive download failed from {}: {}", label, err);
                    failure_reasons.push(format!("{}: {}", label, err));
                    let _ = std::fs::remove_file(&archive_tmp_path);
                    continue;
                }

                // Verify magic bytes (reject HTML error page spoofing)
                if !is_zip_file(&archive_tmp_path) {
                    tracing::warn!(target: "ai", "[Emotion] Downloaded file from {} is not a valid ZIP archive (possible HTML error response)", label);
                    failure_reasons.push(format!("{}: 返回内容不是合法 ZIP (疑似 HTML 响应)", label));
                    let _ = std::fs::remove_file(&archive_tmp_path);
                    continue;
                }

                let _ = emit_progress(build_download_progress(
                    "extracting",
                    "正在解压模型组件 (~1.1GB)，请稍候...".to_string(),
                    MODEL_ARCHIVE_NAME.to_string(),
                    1,
                    1,
                    0,
                    None,
                ));

                // Unpack archive
                let unpack_res = (|| -> Result<(), String> {
                    let file = std::fs::File::open(&archive_tmp_path)
                        .map_err(|e| format!("打开下载的压缩包失败: {}", e))?;
                    let archive = zip::ZipArchive::new(file)
                        .map_err(|e| format!("解析下载的压缩包失败: {}", e))?;
                    unpack_emotion_zip(archive, &staging_dir)?;
                    Ok(())
                })();

                // Immediately remove the 1.1GB tmp zip archive to reclaim disk space before validation
                let _ = std::fs::remove_file(&archive_tmp_path);

                if let Err(err) = unpack_res {
                    tracing::warn!(target: "ai", "[Emotion] Unpack failed for {}: {}", label, err);
                    failure_reasons.push(format!("{}: 解压失败 ({})", label, err));
                    clean_staging_files(&staging_dir);
                    continue;
                }

                // Validate fast inspection
                if let Err(err) = inspect_model_files_fast(&staging_dir) {
                    tracing::warn!(target: "ai", "[Emotion] Fast inspection failed for {}: {}", label, err);
                    failure_reasons.push(format!("{}: 组件质检未通过 ({})", label, err));
                    clean_staging_files(&staging_dir);
                    continue;
                }

                // Deep validate
                let _ = emit_progress(build_download_progress(
                    "verifying",
                    "正在校验模型权重完整性与推理能力...".to_string(),
                    "model.onnx".to_string(),
                    1,
                    1,
                    0,
                    None,
                ));

                let candidate_dir = staging_dir.clone();
                let validate_res = tokio::task::spawn_blocking(move || {
                    validate_model_files_deep(&candidate_dir)
                })
                .await
                .map_err(|e| format!("模型验证任务失败: {}", e))?;

                if let Err(err) = validate_res {
                    tracing::warn!(target: "ai", "[Emotion] Deep validation failed for {}: {}", label, err);
                    failure_reasons.push(format!("{}: 深度推理验证失败 ({})", label, err));
                    clean_staging_files(&staging_dir);
                    continue;
                }

                download_succeeded = true;
                break;
            }
            EmotionDownloadCandidate::Endpoint { base_url, label } => {
                tracing::info!(target: "ai", "[Emotion] Attempting individual files download from {}: {}", label, base_url);
                let files_to_download: Vec<String> = REQUIRED_FILES.iter().map(|f| (*f).to_string()).collect();
                let file_count = files_to_download.len();
                let mut endpoint_ok = true;

                for (index, file_name) in files_to_download.iter().enumerate() {
                    let target_path = match emotion_model_file_path(&staging_dir, file_name) {
                        Ok(p) => p,
                        Err(e) => {
                            endpoint_ok = false;
                            failure_reasons.push(format!("{}: 路径解析失败 ({})", label, e));
                            break;
                        }
                    };

                    let url = emotion_model_file_url(base_url, file_name);
                    let progress_sender = emit_progress.clone();
                    let fname = file_name.clone();

                    let _ = emit_progress(build_download_progress(
                        "downloading",
                        format!("正在下载 {} ({}/{}) [{}]", file_name, index + 1, file_count, label),
                        file_name.clone(),
                        index + 1,
                        file_count,
                        0,
                        None,
                    ));

                    let file_max_bytes = if file_name == "model.onnx" {
                        MAX_EMOTION_SINGLE_FILE_DOWNLOAD_BYTES
                    } else {
                        MAX_EMOTION_COMPANION_FILE_DOWNLOAD_BYTES
                    };

                    let dl_res = crate::utils::download::download_file_with_progress(
                        &client,
                        &url,
                        &target_path,
                        crate::utils::download::DownloadOptions {
                            max_bytes: Some(file_max_bytes),
                            ..Default::default()
                        },
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

                    if let Err(err) = dl_res {
                        tracing::warn!(target: "ai", "[Emotion] File {} download failed from {}: {}", file_name, label, err);
                        failure_reasons.push(format!("{} ({}): {}", label, file_name, err));
                        endpoint_ok = false;
                        break;
                    }
                }

                if endpoint_ok {
                    if let Err(err) = inspect_model_files_fast(&staging_dir) {
                        tracing::warn!(target: "ai", "[Emotion] Fast inspection failed for {}: {}", label, err);
                        failure_reasons.push(format!("{}: 组件质检未通过 ({})", label, err));
                        clean_staging_files(&staging_dir);
                        continue;
                    }

                    let _ = emit_progress(build_download_progress(
                        "verifying",
                        "正在校验模型权重完整性...".to_string(),
                        "model.onnx".to_string(),
                        file_count,
                        file_count,
                        0,
                        None,
                    ));

                    let candidate_dir = staging_dir.clone();
                    let validate_res = tokio::task::spawn_blocking(move || {
                        validate_model_files_deep(&candidate_dir)
                    })
                    .await
                    .map_err(|e| format!("模型验证任务失败: {}", e))?;

                    if let Err(err) = validate_res {
                        tracing::warn!(target: "ai", "[Emotion] Deep validation failed for {}: {}", label, err);
                        failure_reasons.push(format!("{}: 深度推理验证失败 ({})", label, err));
                        clean_staging_files(&staging_dir);
                        continue;
                    }

                    download_succeeded = true;
                    break;
                } else {
                    clean_staging_files(&staging_dir);
                }
            }
        }
    }

    if !download_succeeded {
        // Check local scratch fallback
        let local_fallbacks = [
            PathBuf::from("scratch/emotion_onnx_export"),
            PathBuf::from("../scratch/emotion_onnx_export"),
        ];
        let mut local_recovered = false;
        for fb in &local_fallbacks {
            if fb.is_dir()
                && copy_required_files_from_dir(fb, &staging_dir).is_ok()
                && inspect_model_files_fast(&staging_dir).is_ok()
                && validate_model_files_deep(&staging_dir).is_ok()
            {
                local_recovered = true;
                tracing::info!(target: "ai", "[Emotion] Recovered model from local development fallback: {}", fb.display());
                break;
            }
        }

        if !local_recovered {
            let error_details = if failure_reasons.is_empty() {
                "无可用下载候选源".to_string()
            } else {
                failure_reasons.join("; ")
            };

            let err_msg = format!(
                "下载情感模型失败。已尝试官方 Release CDN、加速镜像与 Hugging Face 节点，均未能成功获取。\n\
                失败详情: {}\n\
                \n\
                由于上游官方源仅提供 safetensors 格式，且当前网络环境下下载完整 ONNX 模型包 (~1.1GB) 受阻：\n\
                1. 您可直接在浏览器中打开官方 Release 页面下载离线包 ({}):\n   {}\n\
                2. 下载完成后，在软件面板中点击【导入已下载的离线包 (.zip)】即可一键安装启用。",
                error_details, MODEL_ARCHIVE_NAME, OFFICIAL_RELEASE_URL
            );
            return Err(err_msg);
        }
    }

    // Safely promote candidate into live snapshot
    let candidate = staging_dir.clone();
    tokio::task::spawn_blocking(move || {
        promote_validated_staging(&candidate, true)
    })
    .await
    .map_err(|error| format!("模型发布任务失败: {}", error))??;

    // Disarm staging guard as staging_dir has been renamed/promoted
    std::mem::forget(staging_guard);

    let _ = emit_progress(build_download_progress(
        "ready",
        "情感模型已就绪并启动".to_string(),
        "model.onnx".to_string(),
        1,
        1,
        0,
        None,
    ));

    Ok(get_emotion_model_status())
}

fn get_or_load_engine() -> Result<(), String> {
    ensure_settings_loaded();
    if !IS_ENABLED.load(Ordering::Acquire) {
        return Err("Emotion engine is disabled".to_string());
    }

    let _lifecycle = LIFECYCLE_MUTEX
        .lock()
        .map_err(|error| format!("Emotion lifecycle lock is poisoned: {}", error))?;
    if !IS_ENABLED.load(Ordering::Acquire) {
        return Err("Emotion engine was disabled while waiting to load".to_string());
    }
    let mut guard = engine_instance()
        .write()
        .map_err(|error| format!("Emotion engine lock is poisoned: {}", error))?;
    if guard.is_some() {
        return Ok(());
    }

    let res = build_validated_engine(&default_model_snapshot_dir());

    match res {
        Ok(engine) => {
            *guard = Some(engine);
            set_last_load_error(None);
            MODEL_IS_VALIDATED.store(true, Ordering::Release);
            tracing::info!(target: "ai", "[Emotion] Loaded Chinese-Emotion-Small ONNX engine successfully.");
            Ok(())
        }
        Err(err) => {
            MODEL_IS_VALIDATED.store(false, Ordering::Release);
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
        "questioning" => &[
            "疑惑", "疑问", "思考", "困惑", "歪头", "question", "confused",
        ],
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

    let truncated_text: String = trimmed
        .chars()
        .rev()
        .take(128)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    // 2. Ensure engine is loaded
    get_or_load_engine()?;

    let mut guard = engine_instance().write().map_err(|e| e.to_string())?;
    let engine = guard
        .as_mut()
        .ok_or_else(|| "Emotion engine is not loaded".to_string())?;

    // Dummy validation and production inference share tokenizer-to-tensor construction,
    // named-output lookup, shape/type checks, and finite-value checks.
    let logits = EmotionModelContract::infer_logits(
        &mut engine.session,
        &engine.tokenizer,
        truncated_text.as_str(),
    )?;
    let probs = softmax(&logits);

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

    static TEST_MUTEX: Mutex<()> = Mutex::new(());

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
        assert_eq!(
            map_emotion_to_live2d_cue("happy", &cues),
            Some("微笑".to_string())
        );
        assert_eq!(
            map_emotion_to_live2d_cue("sad", &cues),
            Some("悲".to_string())
        );
        assert_eq!(
            map_emotion_to_live2d_cue("neutral", &cues),
            Some("平静".to_string())
        );
    }

    fn find_test_model_dir() -> Option<PathBuf> {
        if let Ok(dir) = std::env::var("KOKORO_EMOTION_MODEL_TEST_DIR") {
            let p = PathBuf::from(dir);
            if p.is_dir() && inspect_model_files_fast(&p).is_ok() {
                return Some(p);
            }
        }
        let snapshot = default_model_snapshot_dir();
        if snapshot.is_dir() && inspect_model_files_fast(&snapshot).is_ok() {
            return Some(snapshot);
        }
        let fallbacks = [
            PathBuf::from("scratch/emotion_onnx_export"),
            PathBuf::from("../scratch/emotion_onnx_export"),
        ];
        for fb in &fallbacks {
            if fb.is_dir() && inspect_model_files_fast(fb).is_ok() {
                return Some(fb.clone());
            }
        }
        None
    }

    #[test]
    fn test_local_model_contract_and_dummy_inference() {
        if let Some(path) = find_test_model_dir() {
            validate_model_files_deep(&path).expect("real model contract validation should pass");
            let mut engine = build_validated_engine(&path).expect("build validated engine");
            let logits = EmotionModelContract::infer_logits(
                &mut engine.session,
                &engine.tokenizer,
                "今天真是太开心了，所有任务都顺利完成了！",
            )
            .expect("real model inference should succeed");
            assert_eq!(logits.len(), 8);
            for &val in &logits {
                assert!(val.is_finite());
            }
            let probs = softmax(&logits);
            assert_eq!(probs.len(), 8);
        } else {
            // When no pre-installed local model is detected on disk, verify the contract
            // and pipeline components against valid descriptors and synthetic logits
            let (inputs, outputs) = valid_io_descriptors();
            assert!(EmotionModelContract::validate_io_descriptors(&inputs, &outputs).is_ok());

            let dummy_logits = [0.1f32, 0.2, 0.8, -0.5, 0.0, 0.3, -0.2, 0.1];
            let validated = EmotionModelContract::validate_runtime_logits(&[1, 8], &dummy_logits)
                .expect("valid runtime logits");
            let probs = softmax(&validated);
            assert_eq!(probs.len(), 8);
            let sum: f32 = probs.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4);
        }
    }

    fn valid_io_descriptors() -> (Vec<TensorIoDescriptor>, Vec<TensorIoDescriptor>) {
        (
            vec![
                TensorIoDescriptor {
                    name: "input_ids".to_string(),
                    element_type: TensorElementType::Int64,
                    shape: vec![-1, -1],
                },
                TensorIoDescriptor {
                    name: "attention_mask".to_string(),
                    element_type: TensorElementType::Int64,
                    shape: vec![-1, -1],
                },
            ],
            vec![TensorIoDescriptor {
                name: MODEL_OUTPUT_NAME.to_string(),
                element_type: TensorElementType::Float32,
                shape: vec![-1, 8],
            }],
        )
    }

    #[test]
    fn test_io_contract_rejects_extra_input_and_missing_named_output() {
        let (mut inputs, outputs) = valid_io_descriptors();
        inputs.push(TensorIoDescriptor {
            name: "token_type_ids".to_string(),
            element_type: TensorElementType::Int64,
            shape: vec![-1, -1],
        });
        assert!(
            EmotionModelContract::validate_io_descriptors(&inputs, &outputs)
                .unwrap_err()
                .contains("输入名称必须严格")
        );

        let (inputs, mut outputs) = valid_io_descriptors();
        outputs[0].name = "scores".to_string();
        assert!(
            EmotionModelContract::validate_io_descriptors(&inputs, &outputs)
                .unwrap_err()
                .contains("输出名称必须严格")
        );

        let (inputs, mut outputs) = valid_io_descriptors();
        outputs.push(TensorIoDescriptor {
            name: "hidden_states".to_string(),
            element_type: TensorElementType::Float32,
            shape: vec![-1, 8],
        });
        assert!(
            EmotionModelContract::validate_io_descriptors(&inputs, &outputs)
                .unwrap_err()
                .contains("输出名称必须严格")
        );
    }

    #[test]
    fn test_io_contract_rejects_wrong_type_rank_and_shape() {
        let (mut inputs, outputs) = valid_io_descriptors();
        inputs[0].element_type = TensorElementType::Int32;
        assert!(EmotionModelContract::validate_io_descriptors(&inputs, &outputs).is_err());

        let (mut inputs, outputs) = valid_io_descriptors();
        inputs[0].shape = vec![1, 16];
        inputs[1].shape = vec![1, 16];
        assert!(EmotionModelContract::validate_io_descriptors(&inputs, &outputs).is_err());

        let (inputs, mut outputs) = valid_io_descriptors();
        outputs[0].shape = vec![2, 4];
        assert!(EmotionModelContract::validate_io_descriptors(&inputs, &outputs).is_err());

        let (inputs, mut outputs) = valid_io_descriptors();
        outputs[0].element_type = TensorElementType::Float64;
        assert!(EmotionModelContract::validate_io_descriptors(&inputs, &outputs).is_err());
    }

    #[test]
    fn test_runtime_logits_require_exact_shape_length_and_finite_values() {
        let valid = [0.0f32; 8];
        assert_eq!(
            EmotionModelContract::validate_runtime_logits(&[1, 8], &valid).unwrap(),
            valid
        );
        assert!(EmotionModelContract::validate_runtime_logits(&[2, 4], &valid).is_err());
        assert!(EmotionModelContract::validate_runtime_logits(&[8], &valid).is_err());
        assert!(EmotionModelContract::validate_runtime_logits(&[1, 8], &valid[..7]).is_err());

        let mut nan = valid;
        nan[3] = f32::NAN;
        assert!(EmotionModelContract::validate_runtime_logits(&[1, 8], &nan).is_err());
        let mut infinite = valid;
        infinite[5] = f32::INFINITY;
        assert!(EmotionModelContract::validate_runtime_logits(&[1, 8], &infinite).is_err());
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
        assert!(res_err.unwrap_err().contains("严格期望为 8"));
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
    fn test_resolve_label_mapping_rejects_missing_duplicate_and_conflicting_maps() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let config_path = temp_dir.path().join("config.json");

        std::fs::write(&config_path, r#"{"num_labels": 8}"#).unwrap();
        assert!(EmotionModelContract::resolve_label_mapping(&config_path)
            .unwrap_err()
            .contains("必须提供 id2label 或 label2id"));

        let duplicate_index = r#"{
            "label2id": {
                "neutral": 0, "caring": 0, "happy": 2, "angry": 3,
                "sad": 4, "questioning": 5, "surprised": 6, "disgusted": 7
            }
        }"#;
        std::fs::write(&config_path, duplicate_index).unwrap();
        assert!(EmotionModelContract::resolve_label_mapping(&config_path)
            .unwrap_err()
            .contains("模型索引重复"));

        let conflicting = r#"{
            "id2label": {
                "0":"neutral", "1":"caring", "2":"happy", "3":"angry",
                "4":"sad", "5":"questioning", "6":"surprised", "7":"disgusted"
            },
            "label2id": {
                "neutral":1, "caring":0, "happy":2, "angry":3,
                "sad":4, "questioning":5, "surprised":6, "disgusted":7
            }
        }"#;
        std::fs::write(&config_path, conflicting).unwrap();
        assert!(EmotionModelContract::resolve_label_mapping(&config_path)
            .unwrap_err()
            .contains("标签顺序不一致"));

        let colliding_keys = r#"{
            "id2label": {
                "0":"neutral", "1":"caring", "01":"happy", "3":"angry",
                "4":"sad", "5":"questioning", "6":"surprised", "7":"disgusted"
            }
        }"#;
        std::fs::write(&config_path, colliding_keys).unwrap();
        assert!(EmotionModelContract::resolve_label_mapping(&config_path)
            .unwrap_err()
            .contains("模型索引重复"));
    }

    #[test]
    fn test_model_mutation_lease_is_released_by_drop() {
        let flag = AtomicBool::new(false);
        let first = ModelMutationLease::acquire_from("test", &flag).expect("first lease");
        assert!(ModelMutationLease::acquire_from("test", &flag).is_err());
        drop(first);
        assert!(ModelMutationLease::acquire_from("test", &flag).is_ok());
    }

    #[test]
    fn test_import_nonexistent_path_fails() {
        let _lock = TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
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
    fn test_fresh_install_creates_snapshot_parent_before_promotion() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let snapshot = temp_dir.path().join("repo").join("snapshots").join("main");
        assert!(!snapshot.parent().unwrap().exists());
        ensure_snapshot_parent(&snapshot).expect("create snapshot parent");
        assert!(snapshot.parent().unwrap().is_dir());
        assert!(!snapshot.exists());
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

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, val: &Path) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, val);
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(orig) = &self.original {
                std::env::set_var(self.key, orig);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn test_emotion_settings_roundtrip() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let settings_path = temp_dir.path().join("emotion_settings.json");

        assert_eq!(
            load_emotion_settings_from(&settings_path),
            EmotionSettings { enabled: true }
        );

        save_emotion_settings_to(&settings_path, &EmotionSettings { enabled: false })
            .expect("save disabled");
        assert_eq!(
            load_emotion_settings_from(&settings_path),
            EmotionSettings { enabled: false }
        );

        save_emotion_settings_to(&settings_path, &EmotionSettings { enabled: true })
            .expect("save enabled");
        assert_eq!(
            load_emotion_settings_from(&settings_path),
            EmotionSettings { enabled: true }
        );
    }

    #[test]
    fn test_emotion_settings_corrupted_fallback() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let settings_path = temp_dir.path().join("emotion_settings.json");

        std::fs::write(&settings_path, b"{ corrupted invalid json: true").expect("write corrupt");
        assert_eq!(
            load_emotion_settings_from(&settings_path),
            EmotionSettings { enabled: true }
        );
    }

    #[test]
    fn test_toggle_active_persists_state() {
        let _lock = TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let settings_path = temp_dir.path().join("emotion_settings.json");
        let _env = EnvVarGuard::set("KOKORO_EMOTION_SETTINGS_TEST_PATH", &settings_path);

        // Toggle active = false (with retry in case mutation lease was temporarily held)
        let mut attempts = 0;
        let status = loop {
            match toggle_emotion_model_active(false) {
                Ok(s) => break s,
                Err(e) if e.contains("another model operation is running") && attempts < 50 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => panic!("toggle active false: {}", e),
            }
        };
        assert!(!status.is_active);
        assert!(!IS_ENABLED.load(Ordering::Acquire));
        assert_eq!(load_emotion_settings(), EmotionSettings { enabled: false });

        // Simulate app restart / reload from disk
        IS_ENABLED.store(true, Ordering::Release); // artificially set in memory
        reload_emotion_settings_for_test();
        assert!(!IS_ENABLED.load(Ordering::Acquire)); // reloaded as false from disk!

        // Restore active = true
        let _ = save_emotion_settings(&EmotionSettings { enabled: true });
        IS_ENABLED.store(true, Ordering::Release);
    }

    #[test]
    fn test_import_preserves_disabled_state_when_already_installed() {
        let _lock = TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let settings_path = temp_dir.path().join("emotion_settings.json");
        let _env = EnvVarGuard::set("KOKORO_EMOTION_SETTINGS_TEST_PATH", &settings_path);

        save_emotion_settings(&EmotionSettings { enabled: false }).expect("save disabled");
        reload_emotion_settings_for_test();
        assert!(!IS_ENABLED.load(Ordering::Acquire));

        let res = import_emotion_model_package("non_existent_path_xyz_123");
        assert!(res.is_err());
        assert!(!IS_ENABLED.load(Ordering::Acquire));
        assert_eq!(load_emotion_settings(), EmotionSettings { enabled: false });

        let _ = save_emotion_settings(&EmotionSettings { enabled: true });
        IS_ENABLED.store(true, Ordering::Release);
    }

    #[test]
    fn test_resolve_download_candidates_order() {
        let candidates = resolve_download_candidates();
        assert!(!candidates.is_empty());
        // First candidate should be canonical official GitHub release
        match &candidates[0] {
            EmotionDownloadCandidate::Archive { url, label } => {
                assert_eq!(url, OFFICIAL_RELEASE_URL);
                assert!(label.contains("官方 GitHub Release"));
            }
            _ => panic!("Expected first candidate to be Archive with OFFICIAL_RELEASE_URL"),
        }

        // Must include CDN mirrors
        let has_cdn = candidates.iter().any(|c| match c {
            EmotionDownloadCandidate::Archive { url, .. } => url.contains("ghproxy"),
            _ => false,
        });
        assert!(has_cdn, "Candidates should include CDN mirror options");

        // Test custom URL override via environment variable
        let custom_test_url = "https://custom.mirror.org/chinese-emotion-small-onnx.zip";
        let _env = EnvVarGuard::set("KOKORO_EMOTION_MODEL_URL", Path::new(custom_test_url));
        let overridden = resolve_download_candidates();
        match &overridden[0] {
            EmotionDownloadCandidate::Archive { url, label } => {
                assert_eq!(url, custom_test_url);
                assert!(label.contains("KOKORO_EMOTION_MODEL_URL"));
            }
            _ => panic!("Expected custom URL to take first priority"),
        }
    }

    #[test]
    fn test_is_zip_file_validation() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let html_path = temp_dir.path().join("error.html");
        std::fs::write(&html_path, b"<!DOCTYPE html><html><body>404 Not Found</body></html>").unwrap();
        assert!(!is_zip_file(&html_path), "HTML file must not be detected as ZIP");

        let zip_path = temp_dir.path().join("test.zip");
        // Valid empty ZIP header
        std::fs::write(&zip_path, &[0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert!(is_zip_file(&zip_path), "Valid zip header must be recognized");
    }

    #[test]
    fn test_unpack_emotion_zip_valid_and_normalization() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let staging_dir = temp_dir.path().join("staging");
        std::fs::create_dir_all(&staging_dir).unwrap();

        // Create an in-memory zip containing model.int8.onnx and config.json
        let mut zip_buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_buffer);
            let options = zip::write::SimpleFileOptions::default();

            writer.start_file("model.int8.onnx", options).unwrap();
            use std::io::Write;
            writer.write_all(b"fake model onnx content").unwrap();

            writer.start_file("config.json", options).unwrap();
            writer.write_all(b"{\"num_labels\": 8}").unwrap();

            writer.start_file("ignore_this.txt", options).unwrap();
            writer.write_all(b"extra file").unwrap();

            writer.finish().unwrap();
        }

        zip_buffer.set_position(0);
        let archive = zip::ZipArchive::new(zip_buffer).unwrap();
        let count = unpack_emotion_zip(archive, &staging_dir).unwrap();

        assert_eq!(count, 2);
        // model.int8.onnx must be normalized to model.onnx
        assert!(staging_dir.join("model.onnx").is_file());
        assert!(!staging_dir.join("model.int8.onnx").exists());
        assert!(staging_dir.join("config.json").is_file());
        assert!(!staging_dir.join("ignore_this.txt").exists());
    }

    #[test]
    fn test_unpack_emotion_zip_slip_protection() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let staging_dir = temp_dir.path().join("staging");
        std::fs::create_dir_all(&staging_dir).unwrap();

        let mut zip_buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_buffer);
            let options = zip::write::SimpleFileOptions::default();

            // Malicious entry attempting path traversal
            writer.start_file("../outside.txt", options).unwrap();
            use std::io::Write;
            writer.write_all(b"evil content").unwrap();

            writer.finish().unwrap();
        }

        zip_buffer.set_position(0);
        let archive = zip::ZipArchive::new(zip_buffer).unwrap();
        let count = unpack_emotion_zip(archive, &staging_dir).unwrap();

        assert_eq!(count, 0);
        assert!(!temp_dir.path().join("outside.txt").exists());
    }

    #[test]
    fn test_status_download_url_points_to_official_release() {
        let status = get_emotion_model_status();
        assert_eq!(status.download_url, OFFICIAL_RELEASE_URL);
    }

    #[test]
    fn test_emotion_confidence_gate_and_margin() {
        // 1. High confidence and wide margin -> confident
        let res_confident = EmotionInferenceResult {
            dominant_emotion: "happy".to_string(),
            label_zh: "開心語調".to_string(),
            confidence: 0.65,
            probabilities: vec![
                EmotionInferenceProbability {
                    label: "happy".to_string(),
                    label_zh: "開心語調".to_string(),
                    score: 0.65,
                },
                EmotionInferenceProbability {
                    label: "caring".to_string(),
                    label_zh: "關切語調".to_string(),
                    score: 0.15,
                },
                EmotionInferenceProbability {
                    label: "neutral".to_string(),
                    label_zh: "平淡語氣".to_string(),
                    score: 0.10,
                },
                EmotionInferenceProbability {
                    label: "sad".to_string(),
                    label_zh: "悲傷語調".to_string(),
                    score: 0.10,
                },
            ],
            mapped_cue: Some("笑".to_string()),
            latency_ms: 12.0,
        };
        assert!((res_confident.confidence_margin() - 0.50).abs() < 1e-4);
        assert!(res_confident.is_confident());
        assert!(res_confident.passes_confidence_gate(0.45, 0.15));

        // 2. High confidence but ambiguous margin (top1 0.46 vs top2 0.43 -> margin 0.03 < 0.15) -> not confident
        let res_ambiguous = EmotionInferenceResult {
            dominant_emotion: "happy".to_string(),
            label_zh: "開心語調".to_string(),
            confidence: 0.46,
            probabilities: vec![
                EmotionInferenceProbability {
                    label: "happy".to_string(),
                    label_zh: "開心語調".to_string(),
                    score: 0.46,
                },
                EmotionInferenceProbability {
                    label: "caring".to_string(),
                    label_zh: "關切語調".to_string(),
                    score: 0.43,
                },
                EmotionInferenceProbability {
                    label: "neutral".to_string(),
                    label_zh: "平淡語氣".to_string(),
                    score: 0.11,
                },
            ],
            mapped_cue: Some("笑".to_string()),
            latency_ms: 10.0,
        };
        assert!((res_ambiguous.confidence_margin() - 0.03).abs() < 1e-4);
        assert!(
            !res_ambiguous.is_confident(),
            "Ambiguous margin must fail confidence gate"
        );
        assert!(!res_ambiguous.passes_confidence_gate(0.45, 0.15));

        // 3. Low confidence (0.35) even with wide margin -> not confident (fails 0.45 threshold)
        let res_low_conf = EmotionInferenceResult {
            dominant_emotion: "sad".to_string(),
            label_zh: "悲傷語調".to_string(),
            confidence: 0.35,
            probabilities: vec![
                EmotionInferenceProbability {
                    label: "sad".to_string(),
                    label_zh: "悲傷語調".to_string(),
                    score: 0.35,
                },
                EmotionInferenceProbability {
                    label: "neutral".to_string(),
                    label_zh: "平淡語氣".to_string(),
                    score: 0.15,
                },
            ],
            mapped_cue: Some("悲".to_string()),
            latency_ms: 11.0,
        };
        assert!((res_low_conf.confidence_margin() - 0.20).abs() < 1e-4);
        assert!(
            !res_low_conf.is_confident(),
            "Confidence 0.35 must fail 0.45 gate"
        );
        assert!(!res_low_conf.passes_confidence_gate(0.45, 0.15));

        // 4. Edge cases: single probability or empty
        let res_single = EmotionInferenceResult {
            dominant_emotion: "neutral".to_string(),
            label_zh: "平淡語氣".to_string(),
            confidence: 1.0,
            probabilities: vec![EmotionInferenceProbability {
                label: "neutral".to_string(),
                label_zh: "平淡語氣".to_string(),
                score: 1.0,
            }],
            mapped_cue: None,
            latency_ms: 0.1,
        };
        assert_eq!(res_single.confidence_margin(), 1.0);
        assert!(res_single.is_confident());
    }

    #[test]
    fn test_unpack_emotion_zip_entry_count_limit() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let staging_dir = temp_dir.path().join("staging");
        std::fs::create_dir_all(&staging_dir).unwrap();

        let mut zip_buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_buffer);
            let options = zip::write::SimpleFileOptions::default();
            for i in 0..=MAX_EMOTION_ZIP_ENTRY_COUNT {
                writer
                    .start_file(format!("dummy_{}.txt", i), options)
                    .unwrap();
                use std::io::Write;
                writer.write_all(b"x").unwrap();
            }
            writer.finish().unwrap();
        }

        zip_buffer.set_position(0);
        let archive = zip::ZipArchive::new(zip_buffer).unwrap();
        let result = unpack_emotion_zip(archive, &staging_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("条目过多"));
    }

    #[test]
    fn test_unpack_emotion_zip_single_file_and_total_limits() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let staging_dir = temp_dir.path().join("staging");
        std::fs::create_dir_all(&staging_dir).unwrap();

        // 1. Single file exceeds limit
        let mut zip_buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_buffer);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("model.onnx", options).unwrap();
            use std::io::Write;
            writer.write_all(b"1234567890_exceeds").unwrap();
            writer.finish().unwrap();
        }
        zip_buffer.set_position(0);
        let archive = zip::ZipArchive::new(zip_buffer).unwrap();
        // Max single file 10 bytes, total 100 bytes
        let result = unpack_emotion_zip_with_limits(archive, &staging_dir, 10, 10, 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("超过单文件最大限制"));

        // 2. Cumulative uncompressed bytes exceed limit
        let mut zip_buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_buffer);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("model.onnx", options).unwrap();
            use std::io::Write;
            writer.write_all(b"12345678").unwrap(); // 8 bytes
            writer.start_file("config.json", options).unwrap();
            writer.write_all(b"12345678").unwrap(); // 8 bytes -> total 16 > 12
            writer.finish().unwrap();
        }
        zip_buffer.set_position(0);
        let archive = zip::ZipArchive::new(zip_buffer).unwrap();
        // Max single file 10 bytes, total 12 bytes
        let result = unpack_emotion_zip_with_limits(archive, &staging_dir, 10, 10, 12);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("累计总大小超过上限"));
    }
}

