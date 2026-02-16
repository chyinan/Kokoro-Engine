use super::helpers::*;
use crate::vision::server::VisionServer;

// ── Deleted File Returns 404 ────────────────────────────────

#[tokio::test]
async fn test_deleted_file_returns_404() {
    let (server, tmp) = start_test_server().await;
    let png = make_png_bytes(128);
    let url = server.upload(&png, "delete_me.png").unwrap();

    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // Wait for server to be ready and verify accessible
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status() == 200 {
                break;
            }
        }
    }

    // Delete the file from disk
    let dir = upload_dir(&tmp);
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        std::fs::remove_file(entry.path()).unwrap();
    }

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Should now return 404
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 404, "deleted file should return 404");
}

// ── Missing Upload Dir ──────────────────────────────────────

#[test]
fn test_missing_upload_dir_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let server = VisionServer::new(tmp.path());
    let dir = upload_dir(&tmp);

    // Delete the upload directory entirely
    std::fs::remove_dir_all(&dir).unwrap();

    let png = make_png_bytes(64);
    let result = server.upload(&png, "orphan.png");
    assert!(result.is_err(), "upload with missing dir should fail");
}

// ── Nonexistent File 404 ────────────────────────────────────

#[tokio::test]
async fn test_nonexistent_file_returns_404() {
    let (server, _tmp) = start_test_server().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // Give server time to start
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let url = format!("http://127.0.0.1:{}/vision/nonexistent_abc123.png", server.port);
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 404);
}

// ── Corrupted File Still Serves ─────────────────────────────

#[tokio::test]
async fn test_corrupted_file_still_served() {
    let (server, tmp) = start_test_server().await;
    let png = make_png_bytes(256);
    let url = server.upload(&png, "corrupt.png").unwrap();

    // Overwrite the file with garbage (simulating corruption)
    let dir = upload_dir(&tmp);
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        std::fs::write(entry.path(), b"CORRUPTED_DATA").unwrap();
    }

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    // Wait for server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Server should still serve the (corrupted) content, not panic
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200, "corrupted file should still be served (status={})", resp.status());
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.as_ref(), b"CORRUPTED_DATA");
}
