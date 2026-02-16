use std::fs;
use std::path::PathBuf;

/// Handler for the `live2d://` custom protocol.
///
/// Serves files from `{app_data_dir}/live2d_models/` so pixi-live2d-display
/// can resolve relative URLs (textures, moc3, motions) correctly.
///
/// URL pattern: `http://live2d.localhost/{model_name}/runtime/file.ext`
/// Maps to:     `{app_data_dir}/live2d_models/{model_name}/runtime/file.ext`
pub fn handle_live2d_request(
    models_dir: PathBuf,
) -> impl Fn(
    tauri::UriSchemeContext<'_, tauri::Wry>,
    tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>>
       + Send
       + Sync
       + 'static {
    move |_ctx, request| {
        let uri = request.uri();
        let path_str = percent_decode(uri.path());

        // Security: block directory traversal
        if path_str.contains("..") {
            return tauri::http::Response::builder()
                .status(403)
                .body(b"Forbidden".to_vec())
                .unwrap();
        }

        let clean_path = path_str.strip_prefix('/').unwrap_or(&path_str);
        let file_path = models_dir.join(clean_path);

        if !file_path.exists() || !file_path.is_file() {
            eprintln!(
                "[live2d protocol] 404 Not Found: {} (resolved to {:?})",
                clean_path, file_path
            );
            return tauri::http::Response::builder()
                .status(404)
                .body(format!("Not Found: {}", clean_path).into_bytes())
                .unwrap();
        }

        let mime_type = match file_path.extension().and_then(|e| e.to_str()) {
            Some("json") => "application/json",
            Some("moc3") => "application/octet-stream",
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            _ => "application/octet-stream",
        };

        match fs::read(&file_path) {
            Ok(content) => tauri::http::Response::builder()
                .header("Content-Type", mime_type)
                .header("Access-Control-Allow-Origin", "*")
                .body(content)
                .unwrap(),
            Err(e) => {
                eprintln!("[live2d protocol] Read error for {:?}: {}", file_path, e);
                tauri::http::Response::builder()
                    .status(500)
                    .body(b"Internal Server Error".to_vec())
                    .unwrap()
            }
        }
    }
}

/// Decode percent-encoded characters in a URL path.
fn percent_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}
