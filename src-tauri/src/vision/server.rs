use crate::lifecycle::TaskScope;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use warp::Filter;

/// Maximum upload size: 5 MB
pub(crate) const MAX_UPLOAD_SIZE: usize = 5 * 1024 * 1024;

/// Manages the local vision image file server.
/// Serves uploaded images at `http://127.0.0.1:{port}/vision/{filename}`.
pub struct VisionServer {
    pub port: u16,
    pub upload_dir: PathBuf,
    image_counter: AtomicU32,
    stop: Option<CancellationToken>,
}

impl VisionServer {
    /// Create a new VisionServer, creating the upload directory and cleaning old files.
    pub fn new(app_data_dir: &std::path::Path) -> Self {
        let upload_dir = app_data_dir.join("vision_uploads");
        let _ = std::fs::create_dir_all(&upload_dir);

        // Cleanup old files from previous sessions
        if let Ok(entries) = std::fs::read_dir(&upload_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }

        Self {
            port: 0, // assigned after bind
            upload_dir,
            image_counter: AtomicU32::new(1),
            stop: None,
        }
    }

    /// Start the HTTP file server in background. Stores the actual port.
    pub(crate) fn start(&mut self, scope: &TaskScope) -> Result<(), String> {
        if self.stop.is_some() {
            return Ok(());
        }
        let _start = scope.lease("bind").map_err(|error| error.to_string())?;
        let stop = scope.token().child_token();
        let dir = self.upload_dir.clone();

        // Bind to port 0 to get a random free port
        let vision_route = warp::path("vision")
            .and(warp::path::param::<String>())
            .and(warp::get())
            .and({
                let dir = dir.clone();
                warp::any().map(move || dir.clone())
            })
            .and_then(serve_file);

        let stop_listener = stop.clone();
        let (addr, listener) = warp::serve(vision_route)
            .try_bind_with_graceful_shutdown(([127, 0, 0, 1], 0), async move {
                stop_listener.cancelled().await;
            })
            .map_err(|error| error.to_string())?;
        scope
            .spawn_durable("http-listener", listener)
            .map_err(|error| error.to_string())?;

        // Delete files older than 30 minutes, every 10 minutes.
        let cleanup_dir = self.upload_dir.clone();
        let stop_cleanup = stop.clone();
        if let Err(error) = scope.spawn_durable("upload-ttl-cleanup", async move {
            loop {
                tokio::select! {
                    biased;
                    _ = stop_cleanup.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(600)) => {}
                }
                prune_old_uploads(&cleanup_dir, &stop_cleanup).await;
            }
        }) {
            stop.cancel();
            return Err(error.to_string());
        }
        self.stop = Some(stop);
        self.port = addr.port();
        tracing::info!(
            target: "vision",
            port = self.port,
            "static file server listening on localhost",
        );
        Ok(())
    }

    /// Save uploaded bytes to a random filename and return the served URL.
    pub fn upload(&self, file_bytes: &[u8], original_filename: &str) -> Result<String, String> {
        // Validate file size
        if file_bytes.len() > MAX_UPLOAD_SIZE {
            return Err(format!(
                "File too large: {} bytes (max {})",
                file_bytes.len(),
                MAX_UPLOAD_SIZE
            ));
        }

        // Validate MIME type by checking magic bytes
        let mime = detect_image_mime(file_bytes)
            .ok_or_else(|| "Invalid image file: unrecognized format".to_string())?;

        // Generate sequential filename with UUID for uniqueness: image_1_abcd1234.png
        let ext = mime_to_extension(&mime);
        let seq = self.image_counter.fetch_add(1, Ordering::Relaxed);
        let short_id = &Uuid::new_v4().to_string()[..8];
        let filename = format!("image_{}_{}.{}", seq, short_id, ext);

        // Prevent path traversal
        let safe_path = self.upload_dir.join(&filename);
        if !safe_path.starts_with(&self.upload_dir) {
            return Err("Invalid filename".to_string());
        }

        std::fs::write(&safe_path, file_bytes)
            .map_err(|e| format!("Failed to write file: {}", e))?;

        let url = format!("http://127.0.0.1:{}/vision/{}", self.port, filename);
        tracing::info!(target: "vision", "Uploaded {} -> {}", original_filename, url);

        Ok(url)
    }
}

impl Drop for VisionServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.cancel();
        }
    }
}

async fn prune_old_uploads(dir: &std::path::Path, stop: &CancellationToken) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    let cutoff = SystemTime::now() - Duration::from_secs(1800);
    while !stop.is_cancelled() {
        let Ok(Some(entry)) = entries.next_entry().await else {
            break;
        };
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if metadata.modified().is_ok_and(|modified| modified < cutoff) {
            match tokio::fs::remove_file(entry.path()).await {
                Ok(()) => {
                    tracing::debug!(target: "vision", "TTL cleanup removed {:?}", entry.path())
                }
                Err(error) => {
                    tracing::warn!(target: "vision", %error, "could not remove expired vision upload")
                }
            }
        }
    }
}

/// Serve a file from the upload directory with path traversal protection.
async fn serve_file(filename: String, dir: PathBuf) -> Result<impl warp::Reply, warp::Rejection> {
    // Sanitize: only allow simple filenames (uuid.ext)
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err(warp::reject::not_found());
    }

    let path = dir.join(&filename);
    if !path.starts_with(&dir) || !path.exists() {
        return Err(warp::reject::not_found());
    }

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| warp::reject::not_found())?;

    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    };

    Ok(warp::http::Response::builder()
        .header("Content-Type", mime)
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*")
        .body(bytes)
        .unwrap())
}

/// Detect image MIME type from magic bytes.
pub(crate) fn detect_image_mime(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 4 {
        return None;
    }
    if bytes.starts_with(b"\x89PNG") {
        Some("image/png".into())
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg".into())
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif".into())
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 11 && &bytes[8..12] == b"WEBP" {
        Some("image/webp".into())
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp".into())
    } else {
        None
    }
}

pub(crate) fn mime_to_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "bin",
    }
}
