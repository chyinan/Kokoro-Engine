use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

const BUILTIN_MODEL_PREFIX: &str = "__builtin__/";

/// Handler for the `live2d://` custom protocol.
///
/// Serves imported models from `{app_data_dir}/live2d_models/` and builtin
/// models from bundled/dev `live2d/` assets so pixi-live2d-display can
/// resolve relative URLs (textures, moc3, motions) correctly.
///
/// URL pattern (Windows):      `http://live2d.localhost/{model_name}/runtime/file.ext`
/// URL pattern (macOS/Linux):  `live2d://localhost/{model_name}/runtime/file.ext`
/// Maps to: `{app_data_dir}/live2d_models/{model_name}/runtime/file.ext`
///
/// Uses `ctx.app_handle().path().app_data_dir()` at request time so the path
/// is resolved correctly on macOS sandboxed apps (DMG installs).
pub fn handle_live2d_request() -> impl Fn(
    tauri::UriSchemeContext<'_, tauri::Wry>,
    tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>>
       + Send
       + Sync
       + 'static {
    move |ctx, request| {
        let models_dir = match ctx.app_handle().path().app_data_dir() {
            Ok(app_data) => app_data.join("live2d_models"),
            Err(e) => {
                tracing::error!(target: "live2d", "[live2d protocol] Cannot resolve app data dir: {}", e);
                return tauri::http::Response::builder()
                    .status(500)
                    .body(b"Internal Server Error".to_vec())
                    .unwrap();
            }
        };
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
        let file_path = match resolve_live2d_file(ctx.app_handle(), &models_dir, clean_path) {
            Ok(Some(path)) => path,
            Ok(None) => {
                tracing::error!(
                    target: "live2d",
                    "[live2d protocol] 404 Not Found: {}",
                    clean_path
                );
                return tauri::http::Response::builder()
                    .status(404)
                    .body(format!("Not Found: {}", clean_path).into_bytes())
                    .unwrap();
            }
            Err(error) => {
                tracing::error!(
                    target: "live2d",
                    "[live2d protocol] Failed to resolve {}: {}",
                    clean_path,
                    error
                );
                return tauri::http::Response::builder()
                    .status(500)
                    .body(b"Internal Server Error".to_vec())
                    .unwrap();
            }
        };

        if !file_path.exists() || !file_path.is_file() {
            tracing::error!(
                target: "live2d",
                "[live2d protocol] 404 Not Found: {} (resolved to {:?})",
                clean_path,
                file_path
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
                tracing::error!(target: "live2d", "[live2d protocol] Read error for {:?}: {}", file_path, e);
                tauri::http::Response::builder()
                    .status(500)
                    .body(b"Internal Server Error".to_vec())
                    .unwrap()
            }
        }
    }
}

fn resolve_live2d_file(
    app_handle: &tauri::AppHandle,
    models_dir: &Path,
    clean_path: &str,
) -> Result<Option<PathBuf>, String> {
    if let Some(builtin_path) = clean_path.strip_prefix(BUILTIN_MODEL_PREFIX) {
        return resolve_builtin_live2d_file(app_handle, builtin_path);
    }

    let file_path = models_dir.join(clean_path);
    if !file_path.exists() {
        return Ok(None);
    }

    ensure_within_root(models_dir, &file_path)?;
    Ok(Some(file_path))
}

fn resolve_builtin_live2d_file(
    app_handle: &tauri::AppHandle,
    clean_path: &str,
) -> Result<Option<PathBuf>, String> {
    for root in builtin_live2d_roots(app_handle) {
        let candidate = root.join(clean_path);
        if !candidate.exists() {
            continue;
        }

        ensure_within_root(&root, &candidate)?;
        return Ok(Some(candidate));
    }

    Ok(None)
}

fn builtin_live2d_roots(app_handle: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        roots.push(resource_dir.join("live2d"));
    }

    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("public").join("live2d"));
        roots.push(cwd.join("dist").join("live2d"));

        if let Some(parent) = cwd.parent() {
            roots.push(parent.join("public").join("live2d"));
            roots.push(parent.join("dist").join("live2d"));
        }
    }

    roots
}

fn ensure_within_root(root: &Path, candidate: &Path) -> Result<(), String> {
    let canonical_root = root.canonicalize().map_err(|error| {
        format!(
            "Failed to canonicalize root '{}': {}",
            root.display(),
            error
        )
    })?;
    let canonical_candidate = candidate.canonicalize().map_err(|error| {
        format!(
            "Failed to canonicalize file '{}': {}",
            candidate.display(),
            error
        )
    })?;

    if canonical_candidate.starts_with(&canonical_root) {
        Ok(())
    } else {
        Err(format!(
            "Resolved path '{}' escapes root '{}'",
            canonical_candidate.display(),
            canonical_root.display()
        ))
    }
}

/// Decode percent-encoded characters in a URL path.
fn percent_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
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
