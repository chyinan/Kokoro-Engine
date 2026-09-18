// pattern: Imperative Shell
use futures::{stream, StreamExt, TryStreamExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;

const DEFAULT_PARALLEL_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_PARALLEL_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_PARALLEL_DOWNLOADS: usize = 4;

pub type DownloadProgressCallback =
    Arc<dyn Fn(DownloadProgress) -> Result<(), String> + Send + Sync>;

#[derive(Clone, Debug)]
pub struct DownloadCancelHandle {
    sender: Arc<tokio::sync::watch::Sender<bool>>,
}

#[derive(Clone, Debug)]
pub struct DownloadCancellationToken {
    receiver: tokio::sync::watch::Receiver<bool>,
}

impl DownloadCancelHandle {
    pub fn new() -> (Self, DownloadCancellationToken) {
        let (sender, receiver) = tokio::sync::watch::channel(false);
        (
            Self {
                sender: Arc::new(sender),
            },
            DownloadCancellationToken { receiver },
        )
    }

    pub fn cancel(&self) {
        let _ = self.sender.send(true);
    }
}

impl DownloadCancellationToken {
    pub fn is_cancelled(&self) -> bool {
        *self.receiver.borrow()
    }

    pub async fn wait_for_cancellation(&mut self) {
        if *self.receiver.borrow() {
            return;
        }
        while self.receiver.changed().await.is_ok() {
            if *self.receiver.borrow() {
                break;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Clone)]
pub struct DownloadOptions {
    pub parallel_threshold_bytes: u64,
    pub parallel_chunk_bytes: u64,
    pub parallel_downloads: usize,
    pub max_bytes: Option<u64>,
    pub overall_timeout: Option<std::time::Duration>,
    pub cancel_token: Option<DownloadCancellationToken>,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            parallel_threshold_bytes: DEFAULT_PARALLEL_THRESHOLD_BYTES,
            parallel_chunk_bytes: DEFAULT_PARALLEL_CHUNK_BYTES,
            parallel_downloads: DEFAULT_PARALLEL_DOWNLOADS,
            max_bytes: None,
            overall_timeout: None,
            cancel_token: None,
        }
    }
}

struct TempDownloadGuard {
    path: PathBuf,
    active: bool,
}

impl TempDownloadGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, active: true }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for TempDownloadGuard {
    fn drop(&mut self) {
        if self.active && self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub async fn download_file_with_progress(
    client: &reqwest::Client,
    url: &str,
    target_path: &Path,
    options: DownloadOptions,
    progress: DownloadProgressCallback,
) -> Result<DownloadProgress, String> {
    if let Some(timeout) = options.overall_timeout {
        match tokio::time::timeout(
            timeout,
            download_file_with_progress_inner(client, url, target_path, options, progress),
        )
        .await
        {
            Ok(res) => res,
            Err(_) => Err(format!(
                "下载超时 (超过全局操作时限 {:?})",
                timeout
            )),
        }
    } else {
        download_file_with_progress_inner(client, url, target_path, options, progress).await
    }
}

async fn download_file_with_progress_inner(
    client: &reqwest::Client,
    url: &str,
    target_path: &Path,
    options: DownloadOptions,
    progress: DownloadProgressCallback,
) -> Result<DownloadProgress, String> {
    if let Some(ref token) = options.cancel_token {
        if token.is_cancelled() {
            return Err("下载已被取消".to_string());
        }
    }

    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("Failed to create download directory: {}", error))?;
    }

    let tmp_path = temporary_download_path(target_path);
    let mut guard = TempDownloadGuard::new(tmp_path.clone());
    let (total_bytes, range_supported) = probe_download(client, url, options.cancel_token.as_ref()).await?;

    if let (Some(max), Some(total)) = (options.max_bytes, total_bytes) {
        if total > max {
            return Err(format!(
                "下载文件大小超过允许的最大限制 ({} 字节 > {} 字节)",
                total, max
            ));
        }
    }

    let download_result = if range_supported
        && total_bytes
            .map(|bytes| bytes >= options.parallel_threshold_bytes)
            .unwrap_or(false)
    {
        if let Err(error) = download_parallel(
            client,
            url,
            &tmp_path,
            total_bytes.unwrap(),
            &options,
            &progress,
        )
        .await
        {
            if let Some(ref token) = options.cancel_token {
                if token.is_cancelled() {
                    return Err(error);
                }
            }
            tracing::warn!(
                target: "tools",
                "[Download] Parallel download failed for {}, falling back to single stream: {}",
                url,
                error
            );
            download_single(
                client,
                url,
                &tmp_path,
                total_bytes,
                options.max_bytes,
                options.cancel_token.as_ref(),
                &progress,
            )
            .await
        } else {
            Ok(())
        }
    } else {
        download_single(
            client,
            url,
            &tmp_path,
            total_bytes,
            options.max_bytes,
            options.cancel_token.as_ref(),
            &progress,
        )
        .await
    };

    download_result?;

    let downloaded_bytes = tokio::fs::metadata(&tmp_path)
        .await
        .map(|metadata| metadata.len())
        .unwrap_or_else(|_| total_bytes.unwrap_or(0));
    let final_total_bytes = total_bytes.or(Some(downloaded_bytes));

    crate::config::atomic_replace_file(&tmp_path, target_path)
        .map_err(|error| format!("Failed to finalize download: {}", error))?;

    guard.disarm();

    let final_progress = DownloadProgress {
        downloaded_bytes,
        total_bytes: final_total_bytes,
    };
    progress(final_progress.clone())?;
    Ok(final_progress)
}

fn temporary_download_path(target_path: &Path) -> PathBuf {
    let file_name = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    target_path.with_file_name(format!(".{file_name}.{}.download", Uuid::new_v4()))
}

fn content_range_total(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit('/').next())
        .and_then(|total| total.parse::<u64>().ok())
}

async fn probe_download(
    client: &reqwest::Client,
    url: &str,
    cancel_token: Option<&DownloadCancellationToken>,
) -> Result<(Option<u64>, bool), String> {
    if let Some(token) = cancel_token {
        if token.is_cancelled() {
            return Err("下载已被取消".to_string());
        }
    }

    let req = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send();

    let response = if let Some(mut token) = cancel_token.cloned() {
        tokio::select! {
            biased;
            _ = token.wait_for_cancellation() => {
                return Err("下载已被取消".to_string());
            }
            res = req => match res {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(
                        target: "tools",
                        "[Download] Range probe failed for {}, falling back to single stream: {}",
                        url,
                        error
                    );
                    return Ok((None, false));
                }
            }
        }
    } else {
        match req.await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(
                    target: "tools",
                    "[Download] Range probe failed for {}, falling back to single stream: {}",
                    url,
                    error
                );
                return Ok((None, false));
            }
        }
    };
    let status = response.status();
    if !status.is_success() {
        tracing::warn!(
            target: "tools",
            "[Download] Range probe returned {} for {}, falling back to single stream",
            status,
            url
        );
        return Ok((None, false));
    }

    let content_range_total = content_range_total(&response);
    let total_bytes = content_range_total.or_else(|| response.content_length());
    let range_supported =
        status == reqwest::StatusCode::PARTIAL_CONTENT && content_range_total.is_some();
    Ok((total_bytes, range_supported))
}

async fn download_single(
    client: &reqwest::Client,
    url: &str,
    tmp_path: &Path,
    probed_total_bytes: Option<u64>,
    max_bytes: Option<u64>,
    cancel_token: Option<&DownloadCancellationToken>,
    progress: &DownloadProgressCallback,
) -> Result<(), String> {
    if let Some(token) = cancel_token {
        if token.is_cancelled() {
            return Err("下载已被取消".to_string());
        }
    }

    let req = client.get(url).send();
    let response = if let Some(mut token) = cancel_token.cloned() {
        tokio::select! {
            biased;
            _ = token.wait_for_cancellation() => {
                return Err("下载已被取消".to_string());
            }
            res = req => res.map_err(|error| format!("Failed to start download: {}", error))?,
        }
    } else {
        req.await
            .map_err(|error| format!("Failed to start download: {}", error))?
    };
    let response = response
        .error_for_status()
        .map_err(|error| format!("Download failed: {}", error))?;

    let total_bytes = response.content_length().or(probed_total_bytes);
    if let (Some(max), Some(total)) = (max_bytes, total_bytes) {
        if total > max {
            return Err(format!(
                "下载文件大小超过允许的最大限制 ({} 字节 > {} 字节)",
                total, max
            ));
        }
    }
    let mut downloaded_bytes = 0u64;
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(tmp_path)
        .await
        .map_err(|error| format!("Failed to create download file: {}", error))?;

    progress(DownloadProgress {
        downloaded_bytes,
        total_bytes,
    })?;

    let mut cancel_token_stream = cancel_token.cloned();
    loop {
        let chunk_opt = if let Some(ref mut token) = cancel_token_stream {
            tokio::select! {
                biased;
                _ = token.wait_for_cancellation() => {
                    return Err("下载已被取消".to_string());
                }
                item = stream.next() => item,
            }
        } else {
            stream.next().await
        };

        match chunk_opt {
            Some(chunk) => {
                let chunk = chunk.map_err(|error| format!("Download stream error: {}", error))?;
                file.write_all(&chunk)
                    .await
                    .map_err(|error| format!("Failed to write download: {}", error))?;
                downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);
                if let Some(max) = max_bytes {
                    if downloaded_bytes > max {
                        return Err(format!(
                            "下载数据量超过允许的最大限制 ({} 字节 > {} 字节)",
                            downloaded_bytes, max
                        ));
                    }
                }
                progress(DownloadProgress {
                    downloaded_bytes,
                    total_bytes,
                })?;
            }
            None => break,
        }
    }

    file.flush()
        .await
        .map_err(|error| format!("Failed to flush download: {}", error))?;
    Ok(())
}

async fn download_parallel(
    client: &reqwest::Client,
    url: &str,
    tmp_path: &Path,
    total_bytes: u64,
    options: &DownloadOptions,
    progress: &DownloadProgressCallback,
) -> Result<(), String> {
    if let Some(ref token) = options.cancel_token {
        if token.is_cancelled() {
            return Err("下载已被取消".to_string());
        }
    }
    if let Some(max) = options.max_bytes {
        if total_bytes > max {
            return Err(format!(
                "下载文件大小超过允许的最大限制 ({} 字节 > {} 字节)",
                total_bytes, max
            ));
        }
    }
    let file = tokio::fs::File::create(tmp_path)
        .await
        .map_err(|error| format!("Failed to create download file: {}", error))?;
    file.set_len(total_bytes)
        .await
        .map_err(|error| format!("Failed to allocate download file: {}", error))?;
    drop(file);

    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    progress(DownloadProgress {
        downloaded_bytes: 0,
        total_bytes: Some(total_bytes),
    })?;

    let chunk_size = options.parallel_chunk_bytes.max(1);
    let chunks: Vec<(u64, u64)> = (0..total_bytes)
        .step_by(chunk_size as usize)
        .map(|start| {
            let end = (start + chunk_size - 1).min(total_bytes - 1);
            (start, end)
        })
        .collect();

    stream::iter(chunks)
        .map(|(start, end)| {
            let client = client.clone();
            let url = url.to_string();
            let tmp_path = tmp_path.to_path_buf();
            let downloaded_bytes = downloaded_bytes.clone();
            let progress = progress.clone();
            let cancel_token = options.cancel_token.clone();

            async move {
                if let Some(ref token) = cancel_token {
                    if token.is_cancelled() {
                        return Err("下载已被取消".to_string());
                    }
                }

                let chunk_fut = async {
                    let response = client
                        .get(&url)
                        .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end))
                        .send()
                        .await
                        .map_err(|error| format!("Failed to request range: {}", error))?
                        .error_for_status()
                        .map_err(|error| format!("Range download failed: {}", error))?;

                    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                        return Err(format!(
                            "Range download returned {} instead of 206",
                            response.status()
                        ));
                    }

                    let content_range = response
                        .headers()
                        .get(reqwest::header::CONTENT_RANGE)
                        .and_then(|value| value.to_str().ok())
                        .and_then(parse_content_range);
                    if content_range != Some((start, end, total_bytes)) {
                        return Err(format!(
                            "Range download returned invalid Content-Range for bytes {}-{}",
                            start, end
                        ));
                    }

                    let bytes = response
                        .bytes()
                        .await
                        .map_err(|error| format!("Failed to read range: {}", error))?;
                    let expected_len = (end - start + 1) as usize;
                    if bytes.len() != expected_len {
                        return Err(format!(
                            "Range download returned {} bytes, expected {}",
                            bytes.len(),
                            expected_len
                        ));
                    }

                    let mut file = tokio::fs::OpenOptions::new()
                        .write(true)
                        .open(&tmp_path)
                        .await
                        .map_err(|error| format!("Failed to open download file: {}", error))?;
                    file.seek(std::io::SeekFrom::Start(start))
                        .await
                        .map_err(|error| format!("Failed to seek download file: {}", error))?;
                    file.write_all(&bytes)
                        .await
                        .map_err(|error| format!("Failed to write range: {}", error))?;
                    file.flush()
                        .await
                        .map_err(|error| format!("Failed to flush range: {}", error))?;

                    let total_downloaded = downloaded_bytes
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed)
                        + bytes.len() as u64;
                    progress(DownloadProgress {
                        downloaded_bytes: total_downloaded,
                        total_bytes: Some(total_bytes),
                    })?;

                    Ok::<(), String>(())
                };

                if let Some(mut token) = cancel_token {
                    tokio::select! {
                        biased;
                        _ = token.wait_for_cancellation() => Err("下载已被取消".to_string()),
                        res = chunk_fut => res,
                    }
                } else {
                    chunk_fut.await
                }
            }
        })
        .buffer_unordered(options.parallel_downloads.max(1))
        .try_collect::<Vec<_>>()
        .await?;

    Ok(())
}

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn review_r01_range_download_must_not_promote_wrong_content_range() {
        let server = MockServer::start().await;
        for (range, content_range, body) in [
            ("bytes=0-0", "bytes 0-0/8", "A"),
            ("bytes=0-3", "bytes 0-3/8", "ABCD"),
            ("bytes=4-7", "bytes 0-3/8", "ABCD"),
        ] {
            Mock::given(method("GET"))
                .and(path("/model"))
                .and(header("Range", range))
                .respond_with(
                    ResponseTemplate::new(206)
                        .insert_header("Content-Range", content_range)
                        .set_body_bytes(body),
                )
                .with_priority(1)
                .mount(&server)
                .await;
        }
        // A valid single-stream fallback is acceptable if the bad range is rejected.
        Mock::given(method("GET"))
            .and(path("/model"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes("ABCDEFGH"))
            .with_priority(10)
            .mount(&server)
            .await;
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("model.bin");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let result = download_file_with_progress(
            &client,
            &format!("{}/model", server.uri()),
            &target,
            DownloadOptions {
                parallel_threshold_bytes: 1,
                parallel_chunk_bytes: 4,
                parallel_downloads: 2,
                max_bytes: None,
                ..Default::default()
            },
            Arc::new(|_| Ok(())),
        )
        .await;

        if result.is_ok() {
            assert_eq!(
                std::fs::read(&target).unwrap(),
                b"ABCDEFGH",
                "wrong Content-Range must not be assembled and promoted as a successful download"
            );
        }
    }

    #[test]
    fn download_temp_paths_are_unique_for_concurrent_attempts() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("model.bin");
        assert_ne!(
            temporary_download_path(&target),
            temporary_download_path(&target)
        );
    }

    #[tokio::test]
    async fn download_replaces_existing_target_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/model"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes("new-content"))
            .mount(&server)
            .await;
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("model.bin");
        std::fs::write(&target, "old-content").unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        download_file_with_progress(
            &client,
            &format!("{}/model", server.uri()),
            &target,
            DownloadOptions::default(),
            Arc::new(|_| Ok(())),
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new-content");
    }

    #[tokio::test]
    async fn download_aborts_when_exceeding_max_bytes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/too-large"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes("1234567890"))
            .mount(&server)
            .await;
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("model.bin");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let result = download_file_with_progress(
            &client,
            &format!("{}/too-large", server.uri()),
            &target,
            DownloadOptions {
                max_bytes: Some(5),
                ..Default::default()
            },
            Arc::new(|_| Ok(())),
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("超过允许的最大限制"));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn download_aborts_on_cancellation_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow-model"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes("content-payload")
                    .set_delay(std::time::Duration::from_millis(500)),
            )
            .mount(&server)
            .await;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("model.bin");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let (cancel_handle, cancel_token) = DownloadCancelHandle::new();
        // Trigger cancellation in background after 50ms
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            cancel_handle.cancel();
        });

        let result = download_file_with_progress(
            &client,
            &format!("{}/slow-model", server.uri()),
            &target,
            DownloadOptions {
                cancel_token: Some(cancel_token),
                ..Default::default()
            },
            Arc::new(|_| Ok(())),
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("取消"));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn download_aborts_on_overall_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/timeout-model"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes("content-payload")
                    .set_delay(std::time::Duration::from_millis(500)),
            )
            .mount(&server)
            .await;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("model.bin");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let result = download_file_with_progress(
            &client,
            &format!("{}/timeout-model", server.uri()),
            &target,
            DownloadOptions {
                overall_timeout: Some(std::time::Duration::from_millis(50)),
                ..Default::default()
            },
            Arc::new(|_| Ok(())),
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("超时"));
        assert!(!target.exists());
    }
}


