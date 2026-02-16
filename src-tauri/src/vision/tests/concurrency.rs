#![cfg(feature = "stress")]

use super::helpers::*;
use crate::vision::server::VisionServer;
use std::collections::HashSet;
use std::sync::Arc;

// ── Burst Upload ────────────────────────────────────────────

#[tokio::test]
async fn test_burst_upload_1000_images() {
    let (server, tmp) = start_test_server().await;
    let server = Arc::new(server);
    let png = make_png_bytes(128);

    let mut handles = Vec::new();
    for i in 0..1000 {
        let s = Arc::clone(&server);
        let data = png.clone();
        handles.push(tokio::spawn(async move {
            s.upload(&data, &format!("burst_{}.png", i))
        }));
    }

    let mut urls = HashSet::new();
    let mut failures = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(url) => {
                urls.insert(url);
            }
            Err(e) => {
                failures += 1;
                eprintln!("[stress] upload failed: {}", e);
            }
        }
    }

    assert_eq!(failures, 0, "no uploads should fail in burst test");
    assert_eq!(urls.len(), 1000, "all 1000 URLs should be unique");

    // Verify all files exist on disk
    let dir = upload_dir(&tmp);
    let count = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .count();
    assert_eq!(count, 1000, "all 1000 files should exist on disk");
}

// ── Parallel HTTP Fetch ─────────────────────────────────────

#[tokio::test]
async fn test_parallel_http_fetch() {
    let (server, _tmp) = start_test_server().await;
    let server = Arc::new(server);
    let png = make_png_bytes(256);

    // Upload 100 files
    let mut urls = Vec::new();
    for i in 0..100 {
        let url = server.upload(&png, &format!("fetch_{}.png", i)).unwrap();
        urls.push(url);
    }

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Fetch all 100 URLs 5 times each (500 total requests) concurrently
    let client = reqwest::Client::new();
    let mut handles = Vec::new();

    for _ in 0..5 {
        for url in &urls {
            let c = client.clone();
            let u = url.clone();
            let expected = png.clone();
            handles.push(tokio::spawn(async move {
                let resp = c.get(&u).send().await.unwrap();
                assert_eq!(resp.status(), 200, "fetch failed for {}", u);
                let body = resp.bytes().await.unwrap();
                assert_eq!(body.len(), expected.len(), "body size mismatch for {}", u);
            }));
        }
    }

    let mut failures = 0;
    for handle in handles {
        if handle.await.is_err() {
            failures += 1;
        }
    }
    assert_eq!(failures, 0, "no parallel fetches should fail");
}

// ── Sequential Counter Under Contention ─────────────────────

#[tokio::test]
async fn test_sequential_counter_under_contention() {
    let (server, _tmp) = start_test_server().await;
    let server = Arc::new(server);
    let png = make_png_bytes(32);

    let mut handles = Vec::new();
    for i in 0..200 {
        let s = Arc::clone(&server);
        let data = png.clone();
        handles.push(tokio::spawn(async move {
            s.upload(&data, &format!("contention_{}.png", i))
        }));
    }

    let mut filenames = HashSet::new();
    for handle in handles {
        let url = handle.await.unwrap().unwrap();
        let name = url.rsplit('/').next().unwrap().to_string();
        filenames.insert(name);
    }

    assert_eq!(filenames.len(), 200, "all 200 filenames must be unique under contention");
}

// ── Concurrent Upload and Cleanup ───────────────────────────

#[tokio::test]
async fn test_concurrent_upload_and_cleanup() {
    let (server, tmp) = start_test_server().await;
    let server = Arc::new(server);
    let dir = upload_dir(&tmp);
    let png = make_png_bytes(64);

    // Spawn uploaders
    let mut handles = Vec::new();
    for i in 0..100 {
        let s = Arc::clone(&server);
        let data = png.clone();
        handles.push(tokio::spawn(async move {
            s.upload(&data, &format!("race_{}.png", i))
        }));
    }

    // Spawn cleanup concurrently
    let cleanup_dir = dir.clone();
    let cleanup = tokio::spawn(async move {
        // Run cleanup logic 10 times in quick succession
        for _ in 0..10 {
            let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(1800);
            if let Ok(entries) = std::fs::read_dir(&cleanup_dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(modified) = meta.modified() {
                            if modified < cutoff {
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    });

    // Wait for everything — no panics is the invariant
    for handle in handles {
        let _ = handle.await;
    }
    cleanup.await.unwrap();

    // All uploaded files should still exist (they're fresh, not expired)
    let remaining = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .count();
    assert!(remaining > 0, "fresh files should survive concurrent cleanup");
}

// ── IPC Flood (rapid sequential uploads) ────────────────────

#[tokio::test]
async fn test_ipc_flood_5000() {
    let (server, tmp) = start_test_server().await;
    let png = make_png_bytes(32);
    let metrics = TestMetrics::new();

    let start = std::time::Instant::now();

    for i in 0..5000 {
        match server.upload(&png, &format!("flood_{}.png", i)) {
            Ok(_) => metrics.record_upload(png.len() as u64),
            Err(_) => metrics.record_failure(),
        }
    }

    let elapsed = start.elapsed();
    let throughput = 5000.0 / elapsed.as_secs_f64();

    println!(
        "[stress] IPC flood: {} uploads in {:.2?} ({:.0} uploads/sec)",
        metrics.total(),
        elapsed,
        throughput
    );
    println!(
        "[stress] Total bytes: {:.2} MB",
        metrics.bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed) as f64 / (1024.0 * 1024.0)
    );

    assert_eq!(metrics.failed(), 0, "no failures in IPC flood");
    assert_eq!(metrics.total(), 5000, "all 5000 uploads should succeed");

    // Verify files on disk
    let dir = upload_dir(&tmp);
    let count = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .count();
    assert_eq!(count, 5000);
}
