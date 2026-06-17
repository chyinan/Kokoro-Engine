#[cfg(not(test))]
use anyhow::Result;
#[cfg(not(test))]
use fastembed::{InitOptionsUserDefined, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::path::{Component, Path};
use std::sync::Arc;

#[cfg(not(test))]
const LOCAL_MODEL_DIR: &str = "models/models--Qdrant--all-MiniLM-L6-v2-onnx";
const MODEL_REPO: &str = "Qdrant/all-MiniLM-L6-v2-onnx";
const MODEL_PAGE_URL: &str = "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx";
const MODEL_FALLBACK_ENDPOINT: &str = "https://hf-mirror.com";
#[cfg(not(test))]
const MODEL_AUX_FILES: &[&str] = &[
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
    "vocab.txt",
];
const MODEL_REF_NAME: &str = "main";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryEmbeddingModelStatus {
    pub installed: bool,
    pub repo_id: String,
    pub download_url: String,
    pub install_dir: String,
    pub model_path: String,
    pub required_files: Vec<String>,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryEmbeddingModelDownloadProgress {
    pub stage: String,
    pub message: String,
    pub current_file: String,
    pub file_index: usize,
    pub file_count: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[cfg(not(test))]
pub(crate) fn local_model_dir() -> &'static str {
    LOCAL_MODEL_DIR
}

#[cfg(not(test))]
fn required_model_files() -> Vec<&'static str> {
    let mut files = vec!["model.onnx"];
    files.extend(MODEL_AUX_FILES.iter().copied());
    files
}

#[cfg(not(test))]
fn default_model_cache_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("models")
}

#[cfg(not(test))]
fn default_model_repo_dir() -> PathBuf {
    default_model_cache_dir().join("models--Qdrant--all-MiniLM-L6-v2-onnx")
}

#[cfg(not(test))]
fn default_model_snapshot_dir() -> PathBuf {
    default_model_repo_dir()
        .join("snapshots")
        .join(MODEL_REF_NAME)
}

#[cfg(not(test))]
fn missing_required_model_files(snapshot_dir: &Path) -> Vec<String> {
    required_model_files()
        .into_iter()
        .filter(|file| !snapshot_dir.join(file).is_file())
        .map(str::to_string)
        .collect()
}

#[cfg(not(test))]
pub(crate) fn resolve_snapshot_dir(repo_dir: &Path) -> Option<PathBuf> {
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

#[cfg(not(test))]
pub(crate) fn model_search_roots() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.clone());
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.to_path_buf());
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.to_path_buf());
            candidates.push(exe_dir.join("_up_").join(".."));
        }
    }

    if let Some(app_data) = dirs_next::data_dir() {
        candidates.push(app_data.join("com.chyin.kokoro"));
    }

    candidates
}

#[cfg(not(test))]
fn find_existing_model_snapshot_dir(require_complete: bool) -> Option<PathBuf> {
    for base in model_search_roots() {
        let repo_dir = base.join(LOCAL_MODEL_DIR);
        let Some(snapshot_dir) = resolve_snapshot_dir(&repo_dir) else {
            continue;
        };
        if !require_complete || missing_required_model_files(&snapshot_dir).is_empty() {
            return Some(snapshot_dir);
        }
    }

    None
}

#[cfg(not(test))]
fn ensure_default_model_repo_layout() -> Result<()> {
    let repo_dir = default_model_repo_dir();
    let snapshot_dir = default_model_snapshot_dir();
    std::fs::create_dir_all(&snapshot_dir)?;
    std::fs::create_dir_all(repo_dir.join("refs"))?;
    std::fs::write(repo_dir.join("refs").join(MODEL_REF_NAME), MODEL_REF_NAME)?;
    Ok(())
}

fn build_download_progress(
    stage: &str,
    message: String,
    current_file: String,
    file_index: usize,
    file_count: usize,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> MemoryEmbeddingModelDownloadProgress {
    MemoryEmbeddingModelDownloadProgress {
        stage: stage.to_string(),
        message,
        current_file,
        file_index,
        file_count,
        downloaded_bytes,
        total_bytes,
    }
}

#[cfg(not(test))]
pub fn memory_embedding_model_status() -> MemoryEmbeddingModelStatus {
    let snapshot_dir = find_existing_model_snapshot_dir(true)
        .or_else(|| find_existing_model_snapshot_dir(false))
        .unwrap_or_else(default_model_snapshot_dir);
    let missing_files = missing_required_model_files(&snapshot_dir);
    let model_path = snapshot_dir.join("model.onnx");

    MemoryEmbeddingModelStatus {
        installed: missing_files.is_empty(),
        repo_id: MODEL_REPO.to_string(),
        download_url: MODEL_PAGE_URL.to_string(),
        install_dir: snapshot_dir.to_string_lossy().into_owned(),
        model_path: model_path.to_string_lossy().into_owned(),
        required_files: required_model_files()
            .into_iter()
            .map(str::to_string)
            .collect(),
        missing_files,
    }
}

#[cfg(test)]
pub fn memory_embedding_model_status() -> MemoryEmbeddingModelStatus {
    MemoryEmbeddingModelStatus {
        installed: true,
        repo_id: MODEL_REPO.to_string(),
        download_url: MODEL_PAGE_URL.to_string(),
        install_dir: String::new(),
        model_path: String::new(),
        required_files: vec!["model.onnx".to_string()],
        missing_files: Vec::new(),
    }
}

#[cfg(not(test))]
fn memory_model_endpoint() -> String {
    std::env::var("HF_ENDPOINT")
        .unwrap_or_else(|_| "https://huggingface.co".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn memory_model_endpoint_candidates(primary_endpoint: &str) -> Vec<String> {
    let primary_endpoint = primary_endpoint.trim_end_matches('/').to_string();
    let fallback_endpoint = MODEL_FALLBACK_ENDPOINT.to_string();
    let mut endpoints = vec![primary_endpoint.clone()];

    if !primary_endpoint.eq_ignore_ascii_case(MODEL_FALLBACK_ENDPOINT) {
        endpoints.push(fallback_endpoint);
    }

    endpoints
}

#[cfg(not(test))]
fn memory_model_endpoints() -> Vec<String> {
    memory_model_endpoint_candidates(&memory_model_endpoint())
}

fn memory_model_file_url(endpoint: &str, file_name: &str) -> String {
    format!(
        "{}/{}/resolve/{}/{}",
        endpoint, MODEL_REPO, MODEL_REF_NAME, file_name
    )
}

fn memory_model_file_path(
    snapshot_dir: &Path,
    file_name: &str,
) -> std::result::Result<PathBuf, String> {
    let path = Path::new(file_name);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!("Invalid model file path: {}", file_name));
    }
    Ok(snapshot_dir.join(path))
}

fn emit_memory_model_progress(
    emit_progress: &Arc<
        dyn Fn(MemoryEmbeddingModelDownloadProgress) -> std::result::Result<(), String>
            + Send
            + Sync,
    >,
    file_name: &str,
    file_index: usize,
    file_count: usize,
    progress: crate::utils::download::DownloadProgress,
) -> std::result::Result<(), String> {
    emit_progress(build_download_progress(
        "downloading",
        format!("Downloading {} ({}/{})", file_name, file_index, file_count),
        file_name.to_string(),
        file_index,
        file_count,
        progress.downloaded_bytes,
        progress.total_bytes,
    ))
}

async fn download_memory_model_file(
    client: &reqwest::Client,
    endpoints: &[String],
    snapshot_dir: &Path,
    file_name: &str,
    file_index: usize,
    file_count: usize,
    emit_progress: &Arc<
        dyn Fn(MemoryEmbeddingModelDownloadProgress) -> std::result::Result<(), String>
            + Send
            + Sync,
    >,
) -> std::result::Result<(), String> {
    let target_path = memory_model_file_path(snapshot_dir, file_name)?;
    let mut errors = Vec::new();

    for (attempt_index, endpoint) in endpoints.iter().enumerate() {
        let url = memory_model_file_url(endpoint, file_name);
        let download_progress = emit_progress.clone();
        let file_name_for_progress = file_name.to_string();

        let result = crate::utils::download::download_file_with_progress(
            client,
            &url,
            &target_path,
            crate::utils::download::DownloadOptions::default(),
            Arc::new(move |progress| {
                emit_memory_model_progress(
                    &download_progress,
                    &file_name_for_progress,
                    file_index,
                    file_count,
                    progress,
                )
            }),
        )
        .await;

        match result {
            Ok(final_progress) => {
                emit_progress(build_download_progress(
                    "complete",
                    format!("Finished {} ({}/{})", file_name, file_index, file_count),
                    file_name.to_string(),
                    file_index,
                    file_count,
                    final_progress.downloaded_bytes,
                    final_progress.total_bytes,
                ))?;

                return Ok(());
            }
            Err(error) => {
                errors.push(format!("{}: {}", url, error));
                if let Some(next_endpoint) = endpoints.get(attempt_index + 1) {
                    tracing::warn!(
                        target: "memory",
                        "[Memory] Download failed for {} from {}, retrying via {}: {}",
                        file_name,
                        endpoint,
                        next_endpoint,
                        error
                    );
                    emit_progress(build_download_progress(
                        "downloading",
                        format!(
                            "Download failed from {}; retrying via {}",
                            endpoint, next_endpoint
                        ),
                        file_name.to_string(),
                        file_index,
                        file_count,
                        0,
                        None,
                    ))?;
                }
            }
        }
    }

    Err(format!(
        "Failed to download {} from all endpoints: {}",
        file_name,
        errors.join("; ")
    ))
}

#[cfg(not(test))]
pub(crate) async fn hydrate_missing_local_files(snapshot_dir: &Path) -> Result<bool> {
    let missing: Vec<&str> = MODEL_AUX_FILES
        .iter()
        .copied()
        .filter(|name| !snapshot_dir.join(name).exists())
        .collect();

    if missing.is_empty() {
        return Ok(false);
    }

    tracing::info!(
        target: "memory",
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

#[cfg(not(test))]
pub(crate) fn try_load_local_embedding_model() -> Option<TextEmbedding> {
    use std::fs;

    let candidates = model_search_roots();

    for base in &candidates {
        let repo_dir = base.join(LOCAL_MODEL_DIR);
        let Some(dir) = resolve_snapshot_dir(&repo_dir) else {
            continue;
        };
        let onnx = dir.join("model.onnx");

        if onnx.exists() {
            tracing::info!(target: "memory", "[Memory] Found local model at: {}", dir.display());

            let tokenizer = dir.join("tokenizer.json");
            let config = dir.join("config.json");
            let special = dir.join("special_tokens_map.json");
            let tok_config = dir.join("tokenizer_config.json");

            if !tokenizer.exists() || !config.exists() {
                tracing::error!(
                    target: "memory",
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
                    tracing::info!(target: "memory", "[Memory] Embedding model loaded successfully from local files.");
                    return Some(model);
                }
                Err(error) => {
                    tracing::error!(target: "memory", "[Memory] Failed to load local model: {}", error);
                }
            }
        }
    }

    tracing::info!(
        target: "memory",
        "[Memory] No local model found. Searched: {:?}",
        candidates
            .iter()
            .map(|candidate| candidate.display().to_string())
            .collect::<Vec<_>>()
    );
    None
}

#[cfg(not(test))]
pub async fn download_memory_embedding_model<F>(
    emit_progress: F,
) -> std::result::Result<MemoryEmbeddingModelStatus, String>
where
    F: Fn(MemoryEmbeddingModelDownloadProgress) -> std::result::Result<(), String>
        + Send
        + Sync
        + 'static,
{
    let status = memory_embedding_model_status();
    if status.installed {
        emit_progress(build_download_progress(
            "ready",
            "Memory embedding model is already installed".to_string(),
            "model.onnx".to_string(),
            0,
            0,
            0,
            None,
        ))?;
        return Ok(status);
    }

    ensure_default_model_repo_layout().map_err(|error| error.to_string())?;

    let snapshot_dir = default_model_snapshot_dir();
    let missing_files = missing_required_model_files(&snapshot_dir);
    let file_count = missing_files.len();
    let emit_progress: Arc<
        dyn Fn(MemoryEmbeddingModelDownloadProgress) -> std::result::Result<(), String>
            + Send
            + Sync,
    > = Arc::new(emit_progress);

    emit_progress(build_download_progress(
        "checking",
        "Checking required memory model files".to_string(),
        String::new(),
        0,
        file_count,
        0,
        None,
    ))?;

    let endpoints = memory_model_endpoints();
    let client = reqwest::Client::builder()
        .user_agent("kokoro-engine/0.2.7")
        .build()
        .map_err(|error| format!("Failed to initialize model downloader: {}", error))?;

    for (index, file_name) in missing_files.iter().enumerate() {
        download_memory_model_file(
            &client,
            &endpoints,
            &snapshot_dir,
            file_name,
            index + 1,
            file_count,
            &emit_progress,
        )
        .await?;
    }

    emit_progress(build_download_progress(
        "verifying",
        "Verifying downloaded memory model".to_string(),
        "model.onnx".to_string(),
        file_count,
        file_count,
        0,
        None,
    ))?;

    if try_load_local_embedding_model().is_none() {
        return Err(
            "Model files were downloaded, but local verification failed. Please retry.".to_string(),
        );
    }

    let final_status = memory_embedding_model_status();
    if !final_status.installed {
        return Err("Model download finished, but required files are still missing.".to_string());
    }

    emit_progress(build_download_progress(
        "ready",
        "Memory embedding model is ready".to_string(),
        "model.onnx".to_string(),
        file_count,
        file_count,
        0,
        None,
    ))?;

    Ok(final_status)
}

#[cfg(test)]
pub async fn download_memory_embedding_model<F>(
    _emit_progress: F,
) -> std::result::Result<MemoryEmbeddingModelStatus, String>
where
    F: Fn(MemoryEmbeddingModelDownloadProgress) -> std::result::Result<(), String>
        + Send
        + Sync
        + 'static,
{
    Ok(memory_embedding_model_status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn memory_model_endpoint_candidates_adds_hf_mirror_fallback() {
        assert_eq!(
            memory_model_endpoint_candidates("https://huggingface.co/"),
            vec![
                "https://huggingface.co".to_string(),
                "https://hf-mirror.com".to_string()
            ]
        );
    }

    #[test]
    fn memory_model_endpoint_candidates_deduplicates_hf_mirror() {
        assert_eq!(
            memory_model_endpoint_candidates("https://hf-mirror.com/"),
            vec!["https://hf-mirror.com".to_string()]
        );
    }

    #[test]
    fn memory_model_file_url_uses_original_repo_layout_for_mirror() {
        assert_eq!(
            memory_model_file_url("https://hf-mirror.com", "model.onnx"),
            "https://hf-mirror.com/Qdrant/all-MiniLM-L6-v2-onnx/resolve/main/model.onnx"
        );
    }

    #[tokio::test]
    async fn download_memory_model_file_retries_mirror_after_primary_failure() {
        let primary_server = MockServer::start().await;
        let mirror_server = MockServer::start().await;
        let model_path = format!("/{MODEL_REPO}/resolve/{MODEL_REF_NAME}/model.onnx");

        Mock::given(method("GET"))
            .and(path(model_path.clone()))
            .respond_with(ResponseTemplate::new(503))
            .mount(&primary_server)
            .await;
        Mock::given(method("GET"))
            .and(path(model_path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes("mirror-model"))
            .mount(&mirror_server)
            .await;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let progress_events = Arc::new(StdMutex::new(Vec::new()));
        let captured_events = Arc::clone(&progress_events);
        let emit_progress: Arc<
            dyn Fn(MemoryEmbeddingModelDownloadProgress) -> std::result::Result<(), String>
                + Send
                + Sync,
        > = Arc::new(move |event| {
            captured_events.lock().expect("progress lock").push(event);
            Ok(())
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let endpoints = vec![primary_server.uri(), mirror_server.uri()];

        download_memory_model_file(
            &client,
            &endpoints,
            temp_dir.path(),
            "model.onnx",
            1,
            1,
            &emit_progress,
        )
        .await
        .expect("mirror fallback should download the model file");

        assert_eq!(
            std::fs::read_to_string(temp_dir.path().join("model.onnx")).expect("downloaded model"),
            "mirror-model"
        );
        assert_eq!(
            primary_server
                .received_requests()
                .await
                .expect("primary requests")
                .len(),
            2
        );
        assert_eq!(
            mirror_server
                .received_requests()
                .await
                .expect("mirror requests")
                .len(),
            2
        );
        assert!(
            progress_events
                .lock()
                .expect("progress lock")
                .iter()
                .any(|event| event.message.contains("retrying via")),
            "retry progress event should be emitted"
        );
    }
}
