use super::helpers::*;
use crate::vision::server::MAX_UPLOAD_SIZE;

// ── Path Traversal ──────────────────────────────────────────

#[tokio::test]
async fn test_path_traversal_dot_dot_in_url() {
    let (server, _tmp) = start_test_server().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Attempt to traverse using encoded slashes — warp will likely 404 or reject
    let url = format!("http://127.0.0.1:{}/vision/..%5C..%5Csecrets", server.port);
    let resp = client.get(&url).send().await.unwrap();
    assert_ne!(resp.status(), 200, "path traversal must not succeed (got {})", resp.status());
}

#[tokio::test]
async fn test_path_traversal_direct() {
    let (server, _tmp) = start_test_server().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Direct .. in path — warp normalizes this before routing
    let url = format!("http://127.0.0.1:{}/vision/..%2fCargo.toml", server.port);
    let resp = client.get(&url).send().await.unwrap();
    assert_ne!(resp.status(), 200, "path traversal via encoded / must not succeed");
}

#[tokio::test]
async fn test_path_traversal_backslash_in_url() {
    let (server, _tmp) = start_test_server().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Literal backslash in URL
    let url = format!("http://127.0.0.1:{}/vision/test%5C..%5Csecrets", server.port);
    let resp = client.get(&url).send().await.unwrap();
    assert_ne!(resp.status(), 200, "backslash path traversal must not succeed");
}

// ── Oversized Upload ────────────────────────────────────────

#[test]
fn test_oversized_upload_rejected() {
    let (server, _tmp) = setup_test_server();
    let big = make_png_bytes(6 * 1024 * 1024);

    let result = server.upload(&big, "huge.png");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("File too large"), "error: {}", err);
}

#[test]
fn test_exactly_max_size_accepted() {
    let (server, _tmp) = setup_test_server();
    let exact = make_png_bytes(MAX_UPLOAD_SIZE);

    let result = server.upload(&exact, "exactly5mb.png");
    assert!(result.is_ok(), "exactly max size should be accepted");
}

#[test]
fn test_one_byte_over_max_rejected() {
    let (server, _tmp) = setup_test_server();
    let over = make_png_bytes(MAX_UPLOAD_SIZE + 1);

    let result = server.upload(&over, "over.png");
    assert!(result.is_err(), "1 byte over max should be rejected");
}

// ── Malformed / Invalid Input ───────────────────────────────

#[test]
fn test_zero_byte_upload() {
    let (server, _tmp) = setup_test_server();
    let result = server.upload(&[], "empty.png");
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("Invalid image file"),
        "empty upload should be rejected as invalid image"
    );
}

#[test]
fn test_malformed_non_image_bytes() {
    let (server, _tmp) = setup_test_server();
    let garbage = vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
    let result = server.upload(&garbage, "garbage.bin");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid image file"));
}

#[test]
fn test_text_file_as_image() {
    let (server, _tmp) = setup_test_server();
    let text = b"This is not an image file at all!";
    let result = server.upload(text, "readme.txt");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid image file"));
}

#[test]
fn test_three_byte_input() {
    let (server, _tmp) = setup_test_server();
    let result = server.upload(&[0x89, 0x50, 0x4E], "tiny.png");
    assert!(result.is_err(), "3-byte input should fail MIME detection");
}

// ── URL Reuse After Deletion ────────────────────────────────

#[tokio::test]
async fn test_url_reuse_after_deletion() {
    let (server, tmp) = start_test_server().await;
    let png = make_png_bytes(64);
    let url = server.upload(&png, "reuse.png").unwrap();

    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // Wait and verify accessible
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status() == 200 {
                break;
            }
        }
    }

    // Delete file
    let dir = upload_dir(&tmp);
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        std::fs::remove_file(entry.path()).unwrap();
    }

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Re-fetch same URL — must be 404
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 404, "deleted file URL must return 404");
}

// ── Filename Sanitization ───────────────────────────────────

#[test]
fn test_upload_ignores_user_filename() {
    let (server, _tmp) = setup_test_server();
    let png = make_png_bytes(64);

    let url = server.upload(&png, "../../attack.png").unwrap();
    let generated_name = url.rsplit('/').next().unwrap();

    assert!(
        generated_name.starts_with("image_"),
        "generated name should be safe, got: {}",
        generated_name
    );
    assert!(
        !generated_name.contains(".."),
        "generated name must not contain path traversal"
    );
    assert!(
        !generated_name.contains('/') && !generated_name.contains('\\'),
        "generated name must not contain path separators"
    );
}
