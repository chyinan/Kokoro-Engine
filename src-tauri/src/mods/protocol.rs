use std::fs;
use std::path::PathBuf;

/// The mod SDK script that gets auto-injected into HTML files served by mod://
/// This provides the `Kokoro` global API inside MOD component iframes.
const MOD_SDK_SCRIPT: &str = include_str!("../../../public/mod-sdk.js");

/// Handler for the `mod://` custom protocol.
/// Serves static files from the `mods/` directory.
/// For HTML files, automatically injects the Kokoro mod SDK.
pub fn handle_mod_request<R: tauri::Runtime>(
    _ctx: tauri::UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let uri = request.uri();
    let path_str = uri.path();

    // Security: block directory traversal
    if path_str.contains("..") {
        return tauri::http::Response::builder()
            .status(403)
            .body(b"Forbidden".to_vec())
            .unwrap();
    }

    let clean_path = path_str.strip_prefix('/').unwrap_or(path_str);

    // Resolve the mods directory — during `cargo tauri dev` the CWD is
    // `src-tauri/`, so we fall back to `../mods` if `mods/` doesn't exist.
    let mods_base = {
        let direct = PathBuf::from("mods");
        if direct.exists() {
            direct
        } else {
            let parent = PathBuf::from("../mods");
            if parent.exists() {
                parent
            } else {
                direct
            }
        }
    };
    let file_path = mods_base.join(clean_path);

    // Security: 验证规范路径在 mods 目录内，防止符号链接绕过
    if let (Ok(canonical_base), Ok(canonical_file)) = (
        mods_base.canonicalize(),
        file_path.canonicalize(),
    ) {
        if !canonical_file.starts_with(&canonical_base) {
            return tauri::http::Response::builder()
                .status(403)
                .body(b"Forbidden".to_vec())
                .unwrap();
        }
    }

    if !file_path.exists() {
        return tauri::http::Response::builder()
            .status(404)
            .body(b"Not Found".to_vec())
            .unwrap();
    }

    let mime_type = match file_path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("gif") => "image/gif",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        _ => "application/octet-stream",
    };

    match fs::read(&file_path) {
        Ok(content) => {
            // Auto-inject the mod SDK script into HTML files
            let body = if mime_type == "text/html" {
                let html = String::from_utf8_lossy(&content);
                let injected = inject_sdk_into_html(&html);
                injected.into_bytes()
            } else {
                content
            };

            tauri::http::Response::builder()
                .header("Content-Type", mime_type)
                .header("Access-Control-Allow-Origin", "*")
                .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
                .header("Access-Control-Allow-Headers", "*")
                // Permissive CSP for mod HTML — mods need full flexibility
                .header(
                    "Content-Security-Policy",
                    "default-src * 'unsafe-inline' 'unsafe-eval' data: blob:; \
                     img-src * data: blob:; \
                     media-src * data: blob:; \
                     script-src * 'unsafe-inline' 'unsafe-eval'; \
                     style-src * 'unsafe-inline';",
                )
                .body(body)
                .unwrap()
        }
        Err(_) => tauri::http::Response::builder()
            .status(500)
            .body(b"Internal Server Error".to_vec())
            .unwrap(),
    }
}

/// Inject the Kokoro Mod SDK into an HTML document.
/// Inserts a `<script>` tag just before `</head>` or at the start of `<body>`.
fn inject_sdk_into_html(html: &str) -> String {
    let sdk_tag = format!("<script>{}</script>", MOD_SDK_SCRIPT);

    // Try to inject before </head>
    if let Some(pos) = html.to_lowercase().find("</head>") {
        let mut result = String::with_capacity(html.len() + sdk_tag.len());
        result.push_str(&html[..pos]);
        result.push_str(&sdk_tag);
        result.push_str(&html[pos..]);
        return result;
    }

    // Fallback: inject after <body> or at the beginning
    if let Some(pos) = html.to_lowercase().find("<body") {
        // Find the end of the <body ...> tag
        if let Some(end) = html[pos..].find('>') {
            let insert_pos = pos + end + 1;
            let mut result = String::with_capacity(html.len() + sdk_tag.len());
            result.push_str(&html[..insert_pos]);
            result.push_str(&sdk_tag);
            result.push_str(&html[insert_pos..]);
            return result;
        }
    }

    // Last resort: prepend the SDK script
    format!("{}{}", sdk_tag, html)
}
