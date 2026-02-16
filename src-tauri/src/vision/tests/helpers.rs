use crate::vision::server::VisionServer;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::TempDir;

// ── Image byte generators ────────────────────────────────────

/// Generate valid PNG bytes: 8-byte magic header + padding to `size`.
pub fn make_png_bytes(size: usize) -> Vec<u8> {
    let header: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut bytes = header;
    bytes.resize(size.max(8), 0xAA);
    bytes
}

/// Generate valid JPEG bytes: 3-byte magic header + padding to `size`.
pub fn make_jpeg_bytes(size: usize) -> Vec<u8> {
    let header: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0];
    let mut bytes = header;
    bytes.resize(size.max(4), 0xBB);
    bytes
}

/// Generate valid GIF bytes (GIF89a header).
pub fn make_gif_bytes() -> Vec<u8> {
    b"GIF89a\x01\x00\x01\x00\x00\x00\x00;\x00".to_vec()
}

/// Generate valid WEBP bytes (RIFF....WEBP header).
pub fn make_webp_bytes() -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&[0x00; 4]); // file size placeholder
    bytes.extend_from_slice(b"WEBP");
    bytes.resize(32, 0xCC);
    bytes
}

/// Generate valid BMP bytes (BM header).
pub fn make_bmp_bytes() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.resize(32, 0xDD);
    bytes
}

// ── Server setup helpers ────────────────────────────────────

/// Create a VisionServer with an isolated temp directory. Does NOT start HTTP.
pub fn setup_test_server() -> (VisionServer, TempDir) {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let server = VisionServer::new(tmp.path());
    (server, tmp)
}

/// Create and start a VisionServer with HTTP serving.
/// Returns (server, temp_dir) — server.port is set after this call.
pub async fn start_test_server() -> (VisionServer, TempDir) {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let mut server = VisionServer::new(tmp.path());
    server.start().await;
    (server, tmp)
}

/// Return the upload directory path for a server created from a TempDir.
pub fn upload_dir(tmp: &TempDir) -> PathBuf {
    tmp.path().join("vision_uploads")
}

// ── Metrics helper ──────────────────────────────────────────

/// Lightweight test metrics for tracking operations.
#[allow(dead_code)]
pub struct TestMetrics {
    pub uploads_total: AtomicU64,
    pub uploads_failed: AtomicU64,
    pub bytes_uploaded: AtomicU64,
}

#[allow(dead_code)]
impl TestMetrics {
    pub fn new() -> Self {
        Self {
            uploads_total: AtomicU64::new(0),
            uploads_failed: AtomicU64::new(0),
            bytes_uploaded: AtomicU64::new(0),
        }
    }

    pub fn record_upload(&self, size: u64) {
        self.uploads_total.fetch_add(1, Ordering::Relaxed);
        self.bytes_uploaded.fetch_add(size, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.uploads_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn total(&self) -> u64 {
        self.uploads_total.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> u64 {
        self.uploads_failed.load(Ordering::Relaxed)
    }
}
