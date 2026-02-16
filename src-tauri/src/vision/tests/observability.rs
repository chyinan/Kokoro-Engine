use super::helpers::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

// ── Vision Metrics ──────────────────────────────────────────

/// Lightweight observability counters for the vision pipeline.
/// In production, these would be integrated with a metrics system (Prometheus, etc).
pub struct VisionMetrics {
    pub uploads_total: AtomicU64,
    pub uploads_failed: AtomicU64,
    pub cleanup_deletions: AtomicU64,
    pub bytes_uploaded: AtomicU64,
    pub upload_latency_us: AtomicU64, // last upload duration in microseconds
}

impl VisionMetrics {
    pub fn new() -> Self {
        Self {
            uploads_total: AtomicU64::new(0),
            uploads_failed: AtomicU64::new(0),
            cleanup_deletions: AtomicU64::new(0),
            bytes_uploaded: AtomicU64::new(0),
            upload_latency_us: AtomicU64::new(0),
        }
    }

    pub fn record_upload(&self, size: u64, latency: std::time::Duration) {
        self.uploads_total.fetch_add(1, Ordering::Relaxed);
        self.bytes_uploaded.fetch_add(size, Ordering::Relaxed);
        self.upload_latency_us.store(latency.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.uploads_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cleanup(&self) {
        self.cleanup_deletions.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            uploads_total: self.uploads_total.load(Ordering::Relaxed),
            uploads_failed: self.uploads_failed.load(Ordering::Relaxed),
            cleanup_deletions: self.cleanup_deletions.load(Ordering::Relaxed),
            bytes_uploaded: self.bytes_uploaded.load(Ordering::Relaxed),
            upload_latency_us: self.upload_latency_us.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug)]
pub struct MetricsSnapshot {
    pub uploads_total: u64,
    pub uploads_failed: u64,
    pub cleanup_deletions: u64,
    pub bytes_uploaded: u64,
    pub upload_latency_us: u64,
}

// ── Tests ───────────────────────────────────────────────────

#[tokio::test]
async fn test_metrics_upload_counters() {
    let metrics = VisionMetrics::new();
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(128);

    for i in 0..10 {
        let start = Instant::now();
        match server.upload(&png, &format!("metric_{}.png", i)) {
            Ok(_) => metrics.record_upload(png.len() as u64, start.elapsed()),
            Err(_) => metrics.record_failure(),
        }
    }

    let snap = metrics.snapshot();
    assert_eq!(snap.uploads_total, 10);
    assert_eq!(snap.uploads_failed, 0);
    assert_eq!(snap.bytes_uploaded, 128 * 10);
    assert!(snap.upload_latency_us > 0, "latency should be recorded");
}

#[test]
fn test_metrics_failure_counter() {
    let metrics = VisionMetrics::new();
    let (server, _tmp) = setup_test_server();

    // Try uploading invalid data
    let garbage = vec![0x00; 16];
    let start = Instant::now();
    match server.upload(&garbage, "bad.png") {
        Ok(_) => metrics.record_upload(garbage.len() as u64, start.elapsed()),
        Err(_) => metrics.record_failure(),
    }

    let snap = metrics.snapshot();
    assert_eq!(snap.uploads_total, 0);
    assert_eq!(snap.uploads_failed, 1);
}

#[test]
fn test_metrics_cleanup_counter() {
    let metrics = VisionMetrics::new();

    // Simulate 5 cleanup deletions
    for _ in 0..5 {
        metrics.record_cleanup();
    }

    let snap = metrics.snapshot();
    assert_eq!(snap.cleanup_deletions, 5);
}

#[tokio::test]
async fn test_upload_latency_tracking() {
    let metrics = VisionMetrics::new();
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(1024);

    let start = Instant::now();
    server.upload(&png, "latency_test.png").unwrap();
    let elapsed = start.elapsed();

    metrics.record_upload(png.len() as u64, elapsed);

    let snap = metrics.snapshot();

    // Latency should be reasonable (under 100ms for a local file write)
    assert!(
        snap.upload_latency_us < 100_000,
        "upload latency {} us seems too high",
        snap.upload_latency_us
    );
    println!(
        "[observability] Upload latency: {} µs",
        snap.upload_latency_us
    );
}
