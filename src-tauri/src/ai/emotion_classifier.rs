use anyhow::{anyhow, Context, Result};
use ndarray::Array2;
use ort::{session::Session, value::TensorRef};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokenizers::Tokenizer;
use tokio::sync::OnceCell;


const LOCAL_MODEL_DIR: &str = "models/models--AdamCodd--tinybert-emotion-balanced";
const MODEL_REPO: &str = "AdamCodd/tinybert-emotion-balanced";
const DEFAULT_MODEL_NAME: &str = "model_int8.onnx";
const REMOTE_MODEL_PATH: &str = "onnx/model_int8.onnx";
const REQUIRED_AUX_FILES: &[&str] = &["tokenizer.json", "config.json"];
const FALLBACK_MODEL_NAMES: &[&str] = &[
    "model_quantized.onnx",
    "model_uint8.onnx",
    "model.onnx",
    "model_fp16.onnx",
];
const MAX_SEQUENCE_LEN: usize = 128;
const DEFAULT_LABELS: [&str; 6] = ["sadness", "joy", "love", "anger", "fear", "surprise"];

static CLASSIFIER: OnceCell<Option<Arc<EmotionClassifier>>> = OnceCell::const_new();

#[derive(Debug, Clone)]
pub struct EmotionClassification {
    pub label: String,
    pub score: f32,
    pub raw_mood: f32,
}

struct EmotionClassifier {
    tokenizer: Tokenizer,
    session: std::sync::Mutex<Session>,
    input_names: ModelInputs,
    labels: Vec<String>,
}

#[derive(Default)]
struct ModelInputs {
    input_ids: Option<String>,
    attention_mask: Option<String>,
    token_type_ids: Option<String>,
}

#[derive(Deserialize)]
struct ModelConfig {
    #[serde(default)]
    id2label: std::collections::HashMap<String, String>,
    #[serde(default)]
    label2id: std::collections::HashMap<String, usize>,
}

pub async fn classify_text(text: &str) -> Option<EmotionClassification> {
    if let Some(classifier) = get_classifier().await {
        match classifier.classify(text) {
            Ok(result) => return Some(result),
            Err(error) => {
                eprintln!("[EmotionClassifier] Inference failed: {error}");
            }
        }
    }

    None
}

async fn get_classifier() -> Option<&'static Arc<EmotionClassifier>> {
    CLASSIFIER
        .get_or_init(|| async {
            if let Err(error) = ensure_local_model_available().await {
                eprintln!("[EmotionClassifier] Failed to hydrate local model cache: {error}");
            }
            match EmotionClassifier::load() {
                Ok(classifier) => {
                    println!("[EmotionClassifier] Loaded local ONNX emotion model.");
                    Some(Arc::new(classifier))
                }
                Err(error) => {
                    eprintln!("[EmotionClassifier] Local model unavailable: {error}");
                    None
                }
            }
        })
        .await
        .as_ref()
}

async fn ensure_local_model_available() -> Result<()> {
    if let Some(model_dir) = locate_model_dir() {
        if resolve_model_path(&model_dir).is_some() {
            return Ok(());
        }
    }

    let target_dir = default_model_cache_dir();
    let target_model = target_dir.join(DEFAULT_MODEL_NAME);
    if target_model.exists() {
        return Ok(());
    }
    tokio::fs::create_dir_all(&target_dir).await?;

    let client = reqwest::Client::builder()
        .user_agent("kokoro-engine/0.1.8")
        .build()?;

    println!(
        "[EmotionClassifier] Downloading local ONNX emotion model to {}",
        target_model.display()
    );
    download_file(&client, REMOTE_MODEL_PATH, target_model).await?;

    for file_name in REQUIRED_AUX_FILES {
        download_file(&client, file_name, target_dir.join(file_name)).await?;
    }

    Ok(())
}

pub fn compute_raw_mood(label: &str, score: f32) -> f32 {
    let score = score.clamp(0.0, 1.0);
    match label {
        "joy" | "love" | "surprise" => 0.5 + score * 0.45,
        "sadness" | "anger" | "fear" => 0.5 - score * 0.45,
        _ => 0.5,
    }
}

impl EmotionClassifier {
    fn load() -> Result<Self> {
        let model_dir = locate_model_dir().context("emotion model directory not found")?;
        let tokenizer_path = model_dir.join("tokenizer.json");
        let config_path = model_dir.join("config.json");
        let model_path = resolve_model_path(&model_dir)
            .with_context(|| format!("no ONNX model file found in {}", model_dir.display()))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|error| anyhow!(error.to_string()))
            .with_context(|| format!("failed to load tokenizer from {}", tokenizer_path.display()))?;

        let session = Session::builder()?
            .commit_from_file(&model_path)
            .with_context(|| format!("failed to load ONNX session from {}", model_path.display()))?;

        let input_names = ModelInputs::from_session(&session)?;
        let labels = load_labels(&config_path)?;

        Ok(Self {
            tokenizer,
            session: std::sync::Mutex::new(session),
            input_names,
            labels,
        })
    }

    fn classify(&self, text: &str) -> Result<EmotionClassification> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| anyhow!(error.to_string()))?;
        let mut input_ids: Vec<i64> = encoding.get_ids().iter().map(|value| i64::from(*value)).collect();
        let mut attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|value| i64::from(*value))
            .collect();
        let mut token_type_ids: Vec<i64> = encoding
            .get_type_ids()
            .iter()
            .map(|value| i64::from(*value))
            .collect();

        if input_ids.len() > MAX_SEQUENCE_LEN {
            input_ids.truncate(MAX_SEQUENCE_LEN);
            attention_mask.truncate(MAX_SEQUENCE_LEN);
            token_type_ids.truncate(MAX_SEQUENCE_LEN);
        }

        let seq_len = input_ids.len().max(1);
        if input_ids.is_empty() {
            input_ids.push(0);
            attention_mask.push(1);
            token_type_ids.push(0);
        }

        let input_ids = Array2::from_shape_vec((1, seq_len), input_ids)?;
        let attention_mask = Array2::from_shape_vec((1, seq_len), attention_mask)?;
        let token_type_ids = Array2::from_shape_vec((1, seq_len), token_type_ids)?;

        let mut session_inputs = ort::inputs![
            self.input_names
                .input_ids
                .as_deref()
                .ok_or_else(|| anyhow!("missing input_ids name"))?
                => TensorRef::from_array_view(input_ids.view())?,
            self.input_names
                .attention_mask
                .as_deref()
                .ok_or_else(|| anyhow!("missing attention_mask name"))?
                => TensorRef::from_array_view(attention_mask.view())?,
        ];

        if let Some(token_type_name) = self.input_names.token_type_ids.as_deref() {
            session_inputs.push((
                token_type_name.to_string().into(),
                TensorRef::from_array_view(token_type_ids.view())?.into(),
            ));
        }

        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("emotion classifier session mutex poisoned"))?;
        let outputs = session.run(session_inputs)?;
        let logits = outputs[0].try_extract_array::<f32>()?;
        let row = logits
            .view()
            .into_dimensionality::<ndarray::Ix2>()?
            .row(0)
            .to_vec();
        let probabilities = softmax(&row);
        let (best_index, best_score) = probabilities
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .ok_or_else(|| anyhow!("emotion classifier produced empty logits"))?;
        let label = self
            .labels
            .get(best_index)
            .cloned()
            .unwrap_or_else(|| DEFAULT_LABELS[best_index.min(DEFAULT_LABELS.len() - 1)].to_string());

        Ok(EmotionClassification {
            raw_mood: compute_raw_mood(&label, best_score),
            label,
            score: best_score,
        })
    }
}

impl ModelInputs {
    fn from_session(session: &Session) -> Result<Self> {
        let mut inputs = Self::default();
        for input in session.inputs() {
            let name = input.name().to_string();
            match name.as_str() {
                "input_ids" => inputs.input_ids = Some(name),
                "attention_mask" => inputs.attention_mask = Some(name),
                "token_type_ids" => inputs.token_type_ids = Some(name),
                _ => {}
            }
        }

        if inputs.input_ids.is_none() || inputs.attention_mask.is_none() {
            return Err(anyhow!(
                "emotion model missing expected inputs: input_ids/attention_mask"
            ));
        }

        Ok(inputs)
    }
}

fn load_labels(config_path: &Path) -> Result<Vec<String>> {
    if !config_path.exists() {
        return Ok(DEFAULT_LABELS.iter().map(|label| (*label).to_string()).collect());
    }

    let content = fs::read_to_string(config_path)?;
    let config: ModelConfig = serde_json::from_str(&content)?;

    if !config.id2label.is_empty() {
        let mut pairs = config
            .id2label
            .iter()
            .filter_map(|(key, value)| key.parse::<usize>().ok().map(|index| (index, value.clone())))
            .collect::<Vec<_>>();
        pairs.sort_by_key(|(index, _)| *index);
        return Ok(pairs.into_iter().map(|(_, label)| normalize_label(&label)).collect());
    }

    if !config.label2id.is_empty() {
        let mut pairs = config
            .label2id
            .iter()
            .map(|(label, index)| (*index, label.clone()))
            .collect::<Vec<_>>();
        pairs.sort_by_key(|(index, _)| *index);
        return Ok(pairs.into_iter().map(|(_, label)| normalize_label(&label)).collect());
    }

    Ok(DEFAULT_LABELS.iter().map(|label| (*label).to_string()).collect())
}

fn normalize_label(label: &str) -> String {
    label.trim().to_lowercase()
}

fn resolve_model_path(model_dir: &Path) -> Option<PathBuf> {
    let preferred = model_dir.join(DEFAULT_MODEL_NAME);
    if preferred.exists() {
        return Some(preferred);
    }

    for file_name in FALLBACK_MODEL_NAMES {
        let candidate = model_dir.join(file_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn locate_model_dir() -> Option<PathBuf> {
    for base in model_search_roots() {
        let repo_dir = base.join(LOCAL_MODEL_DIR);
        if let Some(snapshot) = resolve_snapshot_dir(&repo_dir) {
            return Some(snapshot);
        }
        if repo_dir.join(DEFAULT_MODEL_NAME).exists() {
            return Some(repo_dir);
        }
    }

    None
}

fn default_model_cache_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join(LOCAL_MODEL_DIR)
}

fn model_search_roots() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.clone());
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.to_path_buf());
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.to_path_buf());
        }
    }

    if let Some(app_data) = dirs_next::data_dir() {
        candidates.push(app_data.join("com.chyin.kokoro"));
    }

    candidates
}

fn resolve_snapshot_dir(repo_dir: &Path) -> Option<PathBuf> {
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

async fn download_file(client: &reqwest::Client, remote_path: &str, destination: PathBuf) -> Result<()> {
    if destination.exists() {
        return Ok(());
    }

    let url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        MODEL_REPO, remote_path
    );
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    tokio::fs::write(destination, bytes).await?;
    Ok(())
}

fn softmax(values: &[f32]) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }
    let max = values
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let exps = values.iter().map(|value| (value - max).exp()).collect::<Vec<_>>();
    let sum = exps.iter().sum::<f32>().max(f32::EPSILON);
    exps.into_iter().map(|value| value / sum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_mood_maps_above_midpoint() {
        let result = EmotionClassification {
            label: "joy".to_string(),
            score: 0.8,
            raw_mood: compute_raw_mood("joy", 0.8),
        };
        assert_eq!(result.label, "joy");
        assert!(result.raw_mood > 0.5);
    }

    #[test]
    fn negative_mood_maps_below_midpoint() {
        let result = EmotionClassification {
            label: "anger".to_string(),
            score: 0.8,
            raw_mood: compute_raw_mood("anger", 0.8),
        };
        assert_eq!(result.label, "anger");
        assert!(result.raw_mood < 0.5);
    }

    #[test]
    fn neutral_mood_stays_neutral() {
        let result = EmotionClassification {
            label: "neutral".to_string(),
            score: 0.0,
            raw_mood: compute_raw_mood("neutral", 0.0),
        };
        assert_eq!(result.label, "neutral");
        assert!((result.raw_mood - 0.5).abs() < 0.001);
    }

    #[test]
    fn softmax_normalizes_distribution() {
        let distribution = softmax(&[1.0, 2.0, 3.0]);
        let sum = distribution.iter().sum::<f32>();
        assert!((sum - 1.0).abs() < 0.0001);
    }
}
