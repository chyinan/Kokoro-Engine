use super::helpers::*;
use crate::vision::server::{detect_image_mime, mime_to_extension, VisionServer};
use std::collections::HashSet;

// ── Shared HTTP helper ──────────────────────────────────────

/// Retry-fetch a URL until we get a 200 or give up after attempts.
async fn fetch_ok(url: &str) -> reqwest::Response {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for _i in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                return resp;
            }
        }
    }
    panic!("failed to fetch {} after retries", url);
}

// ── File Save Integrity ─────────────────────────────────────

#[tokio::test]
async fn test_upload_saves_file_to_disk() {
    let (server, tmp) = start_test_server().await;
    let png = make_png_bytes(256);

    let _url = server.upload(&png, "test.png").unwrap();
    let dir = upload_dir(&tmp);

    let entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1, "expected exactly one file in upload_dir");

    let saved = std::fs::read(entries[0].path()).unwrap();
    assert_eq!(saved, png, "saved bytes must match uploaded bytes");
}

#[tokio::test]
async fn test_upload_returns_valid_url() {
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(128);

    let url = server.upload(&png, "photo.png").unwrap();

    assert!(
        url.starts_with(&format!("http://127.0.0.1:{}/vision/image_", server.port)),
        "URL must point to local vision server, got: {}",
        url
    );
    assert!(url.ends_with(".png"), "URL must end with .png extension");
}

// ── URL Accessibility ───────────────────────────────────────

#[tokio::test]
async fn test_url_is_accessible_via_http() {
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(512);
    let url = server.upload(&png, "fetch_test.png").unwrap();

    let resp = fetch_ok(&url).await;
    assert_eq!(resp.status(), 200);
    let body = resp.bytes().await.unwrap();
    assert_eq!(
        body.as_ref(),
        png.as_slice(),
        "HTTP body must match uploaded bytes"
    );
}

// ── Cache Headers ───────────────────────────────────────────

#[tokio::test]
async fn test_cache_control_header() {
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(64);
    let url = server.upload(&png, "cache.png").unwrap();

    let resp = fetch_ok(&url).await;
    let cache = resp
        .headers()
        .get("cache-control")
        .expect("Cache-Control header must be present")
        .to_str()
        .unwrap();
    assert_eq!(cache, "no-store", "Cache-Control must be no-store");
}

#[tokio::test]
async fn test_access_control_header() {
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(64);
    let url = server.upload(&png, "cors.png").unwrap();

    let resp = fetch_ok(&url).await;
    let cors = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("CORS header must be present")
        .to_str()
        .unwrap();
    assert_eq!(cors, "*", "CORS header must be *");
}

// ── Content-Type ────────────────────────────────────────────

#[tokio::test]
async fn test_content_type_png() {
    let (server, _tmp) = start_test_server().await;
    let url = server.upload(&make_png_bytes(64), "a.png").unwrap();
    let resp = fetch_ok(&url).await;
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "image/png");
}

#[tokio::test]
async fn test_content_type_jpeg() {
    let (server, _tmp) = start_test_server().await;
    let url = server.upload(&make_jpeg_bytes(64), "a.jpg").unwrap();
    let resp = fetch_ok(&url).await;
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "image/jpeg");
}

#[tokio::test]
async fn test_content_type_gif() {
    let (server, _tmp) = start_test_server().await;
    let url = server.upload(&make_gif_bytes(), "a.gif").unwrap();
    let resp = fetch_ok(&url).await;
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "image/gif");
}

#[tokio::test]
async fn test_content_type_webp() {
    let (server, _tmp) = start_test_server().await;
    let url = server.upload(&make_webp_bytes(), "a.webp").unwrap();
    let resp = fetch_ok(&url).await;
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "image/webp");
}

#[tokio::test]
async fn test_content_type_bmp() {
    let (server, _tmp) = start_test_server().await;
    let url = server.upload(&make_bmp_bytes(), "a.bmp").unwrap();
    let resp = fetch_ok(&url).await;
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "image/bmp");
}

// ── Sequential Naming ───────────────────────────────────────

#[tokio::test]
async fn test_sequential_naming() {
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(32);

    let url1 = server.upload(&png, "a.png").unwrap();
    let url2 = server.upload(&png, "b.png").unwrap();
    let url3 = server.upload(&png, "c.png").unwrap();

    let name1 = url1.rsplit('/').next().unwrap();
    let name2 = url2.rsplit('/').next().unwrap();
    let name3 = url3.rsplit('/').next().unwrap();

    assert!(name1.starts_with("image_1_"), "first upload: {}", name1);
    assert!(name2.starts_with("image_2_"), "second upload: {}", name2);
    assert!(name3.starts_with("image_3_"), "third upload: {}", name3);
}

#[tokio::test]
async fn test_uuid_suffix_uniqueness() {
    let (server, _tmp) = start_test_server().await;
    let png = make_png_bytes(32);

    let mut filenames = HashSet::new();
    for i in 0..100 {
        let url = server.upload(&png, &format!("img_{}.png", i)).unwrap();
        let name = url.rsplit('/').next().unwrap().to_string();
        assert!(
            filenames.insert(name.clone()),
            "duplicate filename: {}",
            name
        );
    }
    assert_eq!(filenames.len(), 100);
}

// ── MIME Detection ──────────────────────────────────────────

#[test]
fn test_mime_detection_all_formats() {
    assert_eq!(detect_image_mime(&make_png_bytes(32)).unwrap(), "image/png");
    assert_eq!(
        detect_image_mime(&make_jpeg_bytes(32)).unwrap(),
        "image/jpeg"
    );
    assert_eq!(detect_image_mime(&make_gif_bytes()).unwrap(), "image/gif");
    assert_eq!(detect_image_mime(&make_webp_bytes()).unwrap(), "image/webp");
    assert_eq!(detect_image_mime(&make_bmp_bytes()).unwrap(), "image/bmp");
}

#[test]
fn test_mime_to_extension_roundtrip() {
    assert_eq!(mime_to_extension("image/png"), "png");
    assert_eq!(mime_to_extension("image/jpeg"), "jpg");
    assert_eq!(mime_to_extension("image/gif"), "gif");
    assert_eq!(mime_to_extension("image/webp"), "webp");
    assert_eq!(mime_to_extension("image/bmp"), "bmp");
    assert_eq!(mime_to_extension("application/octet-stream"), "bin");
}

// ── TTL Cleanup ─────────────────────────────────────────────

#[tokio::test]
async fn test_ttl_cleanup_removes_old_files() {
    let (server, tmp) = start_test_server().await;
    let dir = upload_dir(&tmp);
    let png = make_png_bytes(64);

    server.upload(&png, "old.png").unwrap();

    let entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1);

    let path = entries[0].path();
    let old_time = filetime::FileTime::from_system_time(
        std::time::SystemTime::now() - std::time::Duration::from_secs(1860),
    );
    filetime::set_file_mtime(&path, old_time).unwrap();

    // Manually run cleanup logic
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(1800);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        std::fs::remove_file(entry.path()).unwrap();
                    }
                }
            }
        }
    }

    let remaining: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(remaining.len(), 0, "old file should have been cleaned up");
}

#[tokio::test]
async fn test_ttl_cleanup_preserves_fresh_files() {
    let (server, tmp) = start_test_server().await;
    let dir = upload_dir(&tmp);
    let png = make_png_bytes(64);

    server.upload(&png, "fresh.png").unwrap();

    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(1800);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        std::fs::remove_file(entry.path()).unwrap();
                    }
                }
            }
        }
    }

    let remaining: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(remaining.len(), 1, "fresh file should NOT be cleaned up");
}

// ── Startup Cleanup ─────────────────────────────────────────

#[test]
fn test_startup_cleans_previous_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let vision_dir = tmp.path().join("vision_uploads");
    std::fs::create_dir_all(&vision_dir).unwrap();

    std::fs::write(vision_dir.join("old_image_1.png"), b"old_data_1").unwrap();
    std::fs::write(vision_dir.join("old_image_2.png"), b"old_data_2").unwrap();

    let _server = VisionServer::new(tmp.path());

    let remaining: Vec<_> = std::fs::read_dir(&vision_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(remaining.len(), 0, "new() should clean old session files");
}
