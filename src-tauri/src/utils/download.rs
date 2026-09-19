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
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            parallel_threshold_bytes: DEFAULT_PARALLEL_THRESHOLD_BYTES,
            parallel_chunk_bytes: DEFAULT_PARALLEL_CHUNK_BYTES,
            parallel_downloads: DEFAULT_PARALLEL_DOWNLOADS,
        }
    }
}

struct TemporaryDownloadGuard {
    path: PathBuf,
    committed: bool,
}

impl TemporaryDownloadGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryDownloadGuard {
    fn drop(&mut self) {
        if !self.committed {
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
    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("Failed to create download directory: {}", error))?;
    }

    let mut tmp_path = TemporaryDownloadGuard::new(temporary_download_path(target_path));
    let (total_bytes, range_supported) = probe_download(client, url).await?;

    if range_supported
        && total_bytes
            .map(|bytes| bytes >= options.parallel_threshold_bytes)
            .unwrap_or(false)
    {
        if let Err(error) = download_parallel(
            client,
            url,
            tmp_path.path(),
            total_bytes.unwrap(),
            &options,
            &progress,
        )
        .await
        {
            tracing::warn!(
                target: "tools",
                "[Download] Parallel download failed for {}, falling back to single stream: {}",
                url,
                error
            );
            download_single(client, url, tmp_path.path(), total_bytes, &progress).await?;
        }
    } else {
        download_single(client, url, tmp_path.path(), total_bytes, &progress).await?;
    }

    let downloaded_bytes = tokio::fs::metadata(tmp_path.path())
        .await
        .map(|metadata| metadata.len())
        .unwrap_or_else(|_| total_bytes.unwrap_or(0));
    let final_total_bytes = total_bytes.or(Some(downloaded_bytes));

    crate::config::atomic_replace_file(tmp_path.path(), target_path)
        .map_err(|error| format!("Failed to finalize download: {}", error))?;
    tmp_path.commit();

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
) -> Result<(Option<u64>, bool), String> {
    let response = match client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
    {
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
    progress: &DownloadProgressCallback,
) -> Result<(), String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Failed to start download: {}", error))?
        .error_for_status()
        .map_err(|error| format!("Download failed: {}", error))?;
    let total_bytes = response.content_length().or(probed_total_bytes);
    let mut downloaded_bytes = 0u64;
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(tmp_path)
        .await
        .map_err(|error| format!("Failed to create download file: {}", error))?;

    progress(DownloadProgress {
        downloaded_bytes,
        total_bytes,
    })?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("Download stream error: {}", error))?;
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("Failed to write download: {}", error))?;
        downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);
        progress(DownloadProgress {
            downloaded_bytes,
            total_bytes,
        })?;
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

            async move {
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
        assert!(
            std::fs::read_dir(temp.path())
                .unwrap()
                .all(|entry| entry.is_ok()),
            "successful downloads should not leave unreadable entries"
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn failed_download_removes_partial_temporary_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/model"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes("partial-content"))
            .mount(&server)
            .await;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("model.bin");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let result = download_file_with_progress(
            &client,
            &format!("{}/model", server.uri()),
            &target,
            DownloadOptions::default(),
            Arc::new(|_| Err("stop after first progress event".to_string())),
        )
        .await;

        assert!(result.is_err());
        assert!(!target.exists());
        let leftovers: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "failed downloads must not leave temporary files: {leftovers:?}"
        );
    }
}
