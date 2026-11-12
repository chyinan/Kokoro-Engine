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

    // macOS WKWebView and Linux WebKitGTK parse custom scheme URLs as:
    //   mod://genshin-theme/chat.html → host="genshin-theme", path="/chat.html"
    // Windows WebView2 rewrites custom schemes to http://<scheme>.localhost/:
    //   http://mod.localhost/genshin-theme/chat.html → host="mod.localhost", path="/genshin-theme/chat.html"
    // So on Windows the mod-id is already in the path; on macOS/Linux we must prepend the host.
    #[cfg(target_os = "windows")]
    let clean_path = path_str.strip_prefix('/').unwrap_or(path_str).to_string();

    #[cfg(not(target_os = "windows"))]
    let clean_path = {
        let host = uri.host().unwrap_or("");
        let bare_path = path_str.strip_prefix('/').unwrap_or(path_str);
        if host.is_empty() {
            bare_path.to_string()
        } else {
            format!("{}/{}", host, bare_path)
        }
    };

    // In debug (dev) mode, fall back to the project-relative `mods/` directory.
    // In release builds, use the absolute app data path so macOS/Linux bundled
    // apps serve mod files regardless of the process working directory.
    #[cfg(debug_assertions)]
    let mods_base = {
        let direct = PathBuf::from("mods");
        if direct.exists() {
            direct
        } else {
            let parent = PathBuf::from("../mods");
            if parent.exists() {
                parent
            } else {
                crate::utils::paths::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("com.chyin.kokoro")
                    .join("mods")
            }
        }
    };
    #[cfg(not(debug_assertions))]
    let mods_base = crate::utils::paths::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("mods");
    let file_path = mods_base.join(clean_path);

    // Security: 验证规范路径在 mods 目录内，防止符号链接绕过
    // canonicalize 失败（文件不存在或符号链接断裂）时明确拒绝，而不是静默跳过
    let canonical_base = match mods_base.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return tauri::http::Response::builder()
                .status(403)
                .body(b"Forbidden".to_vec())
                .unwrap();
        }
    };
    let canonical_file = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return tauri::http::Response::builder()
                .status(404)
                .body(b"Not Found".to_vec())
                .unwrap();
        }
    };
    if !canonical_file.starts_with(&canonical_base) {
        return tauri::http::Response::builder()
            .status(403)
            .body(b"Forbidden".to_vec())
            .unwrap();
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
                // 限制 CORS 仅允许应用自身 origin，不对外开放
                .header("Access-Control-Allow-Origin", "tauri://localhost")
                .header("Access-Control-Allow-Methods", "GET, OPTIONS")
                .header("Access-Control-Allow-Headers", "Content-Type")
                // CSP: 移除宽泛的 localhost 通配，防止 MOD 探测本地服务
                // connect-src 仅允许 mod:// 协议自身资源，不允许任意 localhost 端口
                .header(
                    "Content-Security-Policy",
                    "default-src 'self' mod: data: blob: 'unsafe-inline'; \
                     img-src mod: data: blob:; \
                     media-src 'self' mod: data: blob:; \
                     script-src 'self' mod: 'unsafe-inline'; \
                     style-src 'self' mod: 'unsafe-inline'; \
                     connect-src 'self' mod:;",
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
