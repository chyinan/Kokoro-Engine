//! Vision Watcher — background loop that captures screen and analyzes with VLM.

use crate::llm::anthropic::AnthropicProvider;
use crate::llm::messages::user_message_with_images;
use crate::llm::provider::{LlmParams, LlmProvider};
use crate::llm::service::LlmService;
use crate::vision::capture::{capture_screen_with_options, has_significant_change, CaptureOptions};
use crate::vision::config::VisionConfig;
use crate::vision::context::VisionContext;
use crate::vision::context::{AnalysisDispatch, VisionFrame};
use reqwest::Client;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

/// Shared handle to control the watcher loop.
#[derive(Clone)]
pub struct VisionWatcher {
    pub running: Arc<AtomicBool>,
    pub config: Arc<RwLock<VisionConfig>>,
    pub context: VisionContext,
    pub llm_service: Option<LlmService>,
    pub client: Client,
}

impl VisionWatcher {
    pub fn new(config: VisionConfig) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            config: Arc::new(RwLock::new(config)),
            context: VisionContext::new(),
            llm_service: None,
            client: Client::new(),
        }
    }

    pub fn with_llm_service(mut self, llm_service: LlmService) -> Self {
        self.llm_service = Some(llm_service);
        self
    }

    /// Start the background producer loop. VLM analysis is dispatched asynchronously
    /// with at most one in-flight request and newest-only pending frame state.
    pub fn start(&self, app_handle: AppHandle) {
        if self
            .running
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            tracing::info!(target: "vision", "Watcher already running");
            return;
        }
        let watcher = self.clone();

        tokio::spawn(async move {
            tracing::info!(target: "vision", "Watcher started");
            let _ = app_handle.emit("vision-status", "active");
            let mut prev_screenshot: Option<Vec<u8>> = None;
            let mut last_capture_warning: Option<String> = None;

            loop {
                if !watcher.running.load(Ordering::Relaxed) {
                    break;
                }

                let config = watcher.config.read().await.clone();
                if !config.vlm_enabled || !config.auto_vision_enabled {
                    watcher.context.clear_auto_state_on_disable().await;
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }

                if watcher.context.should_pause_auto_capture().await {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }

                let capture_options = CaptureOptions {
                    display_id: config.display_id.clone(),
                    region: config.vlm_region,
                };
                let captured = match capture_screen_with_options(&capture_options) {
                    Ok(image) => image,
                    Err(error) => {
                        tracing::error!(target: "vision", "Capture failed: {}", error);
                        watcher.context.set_last_error(error).await;
                        tokio::time::sleep(std::time::Duration::from_secs(
                            config.capture_interval_secs as u64,
                        ))
                        .await;
                        continue;
                    }
                };
                if let Some(warning) = captured.warning.clone() {
                    if last_capture_warning.as_deref() != Some(warning.as_str()) {
                        tracing::warn!(target: "vision", "{}", warning);
                        watcher.context.set_last_error(warning.clone()).await;
                        last_capture_warning = Some(warning);
                    } else {
                        tracing::debug!(target: "vision", "{}", warning);
                    }
                } else {
                    last_capture_warning = None;
                }

                let changed = prev_screenshot
                    .as_ref()
                    .map(|prev| {
                        has_significant_change(prev, &captured.jpeg_bytes, config.change_threshold)
                    })
                    .unwrap_or(true);

                if !changed {
                    tracing::info!(target: "vision", "No significant change, skipping analysis");
                    tokio::time::sleep(std::time::Duration::from_secs(
                        config.capture_interval_secs as u64,
                    ))
                    .await;
                    continue;
                }

                prev_screenshot = Some(captured.jpeg_bytes.clone());
                let frame = VisionFrame {
                    id: uuid::Uuid::new_v4().to_string(),
                    captured_at: chrono::Utc::now(),
                    jpeg_bytes: captured.jpeg_bytes,
                    display_id: captured.display_id,
                    region: captured.region,
                    image_hash: captured.image_hash,
                };

                if let AnalysisDispatch::Dispatch(frame) =
                    watcher.context.submit_auto_frame(frame).await
                {
                    let analysis_watcher = watcher.clone();
                    let analysis_app = app_handle.clone();
                    tokio::spawn(async move {
                        run_analysis_chain(analysis_watcher, analysis_app, frame).await;
                    });
                }

                tokio::time::sleep(std::time::Duration::from_secs(
                    config.capture_interval_secs as u64,
                ))
                .await;
            }

            tracing::info!(target: "vision", "Watcher stopped");
            let _ = app_handle.emit("vision-status", "inactive");
        });
    }

    /// Stop the background vision loop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        let ctx = self.context.clone();
        tokio::spawn(async move { ctx.clear_auto_state_on_disable().await });
    }
}

async fn run_analysis_chain(
    watcher: VisionWatcher,
    app_handle: AppHandle,
    first_frame: VisionFrame,
) {
    let mut current = first_frame;
    loop {
        tracing::info!(target: "vision", "Screen changed, analyzing with VLM...");
        let config = watcher.config.read().await.clone();
        let result = analyze_screenshot(
            &watcher.client,
            &config,
            &current.jpeg_bytes,
            watcher.llm_service.as_ref(),
        )
        .await;

        match &result {
            Ok(description) => {
                tracing::debug!(target: "vision", "Observation: {}", description);
            }
            Err(error) => tracing::error!(target: "vision", "VLM analysis failed: {}", error),
        }

        let should_emit_proactive = result.is_ok() && config.proactive_vision_enabled;
        let next_frame = watcher.context.finish_auto_analysis(&current, result).await;

        if should_emit_proactive {
            if let Some(observation) = watcher
                .context
                .latest_completed_observation(chrono::Utc::now())
                .await
            {
                let _ = app_handle.emit(
                    "vision-observation",
                    serde_json::json!({
                        "summary": observation.summary,
                        "captured_at": observation.captured_at.to_rfc3339(),
                        "source": observation.source.as_str(),
                    }),
                );
            }
        }

        if should_emit_proactive {
            emit_proactive_vision_comment(&app_handle);
        }

        match next_frame {
            Some(next) => current = next,
            None => break,
        }
    }
}

fn emit_proactive_vision_comment(app_handle: &AppHandle) {
    tracing::info!(target: "vision", "Vision screen-comment trigger fired");
    let _ = app_handle.emit(
        "proactive-trigger",
        serde_json::json!({
            "trigger": "vision",
            "instruction": build_proactive_vision_instruction(),
        }),
    );
}

const VISION_PROMPT: &str = "Describe the screenshot in 2-3 concise, information-rich sentences. Include the active application/window, important visible UI text, and the most visually prominent non-UI content such as characters, artwork, background, objects, colors, and scene details. If a fictional/anime character is clearly recognizable, you may name them; do not identify real people from appearance alone. Do not infer authorship, private intent, emotions, or anything off-screen. If something is unclear, say that briefly.";
const DEFAULT_OLLAMA_VLM_BASE_URL: &str = "http://localhost:11434/v1";
const DEFAULT_OPENAI_VLM_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_ANTHROPIC_VLM_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_LLAMA_CPP_VLM_BASE_URL: &str = "http://127.0.0.1:8080";

fn build_proactive_vision_instruction() -> &'static str {
    "请结合上方的屏幕上下文、当前角色的人设和性格，对屏幕内容做一句自然、简短、轻量的评论。\
    只评论屏幕上直接可见的内容，不要表现得像在监视用户，也不要声称知道隐藏信息、用户想法、代码是谁写的或谁改的，或屏幕外发生的事。\
    避免重复、恐怖、威胁、夸张或令人不适的措辞；如果内容不清楚或可能敏感，就保持中性温和。\
    如果这个屏幕变化不值得评论，请只回复 PASS。"
}

fn default_vlm_base_url(provider: &str) -> &'static str {
    match provider {
        "llama_cpp" => DEFAULT_LLAMA_CPP_VLM_BASE_URL,
        "anthropic" => DEFAULT_ANTHROPIC_VLM_BASE_URL,
        "openai" => DEFAULT_OPENAI_VLM_BASE_URL,
        _ => DEFAULT_OLLAMA_VLM_BASE_URL,
    }
}

fn normalize_openai_compatible_chat_base_url(base_url: &str, default_base_url: &str) -> String {
    let mut normalized = base_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        normalized = default_base_url.trim_end_matches('/').to_string();
    }

    for suffix in ["/v1/chat/completions", "/chat/completions"] {
        if let Some(stripped) = normalized.strip_suffix(suffix) {
            normalized = stripped.to_string();
            break;
        }
    }

    if !normalized.ends_with("/v1") {
        normalized.push_str("/v1");
    }

    normalized
}

fn normalize_vlm_chat_base_url(provider: &str, base_url: Option<&str>) -> String {
    let default_base_url = default_vlm_base_url(provider);
    normalize_openai_compatible_chat_base_url(
        base_url.unwrap_or(default_base_url),
        default_base_url,
    )
}

/// Send a screenshot to the VLM for analysis.
/// When `vlm_provider` is "llm", delegates to the active LlmService provider.
/// Otherwise uses the independently configured VLM endpoint (ollama / openai /
/// anthropic / llama.cpp).
pub async fn analyze_screenshot(
    client: &Client,
    config: &VisionConfig,
    screenshot: &[u8],
    llm_service: Option<&LlmService>,
) -> Result<String, String> {
    // Encode screenshot as base64 data URL (used by both paths)
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, screenshot);
    let data_url = format!("data:image/jpeg;base64,{}", b64);

    if config.vlm_provider == "llm" {
        // ── Route through the active LLM provider ──────────────────────────
        let svc = llm_service.ok_or_else(|| "LLM service not available".to_string())?;
        let provider = svc.provider().await;

        let messages = vec![user_message_with_images(
            VISION_PROMPT.to_string(),
            vec![data_url],
        )];

        let params = LlmParams {
            max_tokens: Some(150),
            temperature: Some(0.3),
            ..Default::default()
        };

        provider.chat(messages, Some(params)).await
    } else if config.vlm_provider == "anthropic" {
        // ── Independent Anthropic Messages API endpoint ───────────────────
        let model = config.vlm_model.trim();
        let model = (!model.is_empty()).then(|| model.to_string());
        let provider = AnthropicProvider::new(
            config.vlm_api_key.clone().unwrap_or_default(),
            config.vlm_base_url.clone(),
            model,
        );

        let messages = vec![user_message_with_images(
            VISION_PROMPT.to_string(),
            vec![data_url],
        )];

        let params = LlmParams {
            max_tokens: Some(150),
            temperature: Some(0.3),
            ..Default::default()
        };

        provider.chat(messages, Some(params)).await
    } else {
        // ── Independent OpenAI-compatible VLM endpoint ─────────────────────
        let chat_base_url =
            normalize_vlm_chat_base_url(&config.vlm_provider, config.vlm_base_url.as_deref());
        let url = format!("{}/chat/completions", chat_base_url);

        let body = serde_json::json!({
            "model": config.vlm_model,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": VISION_PROMPT },
                    { "type": "image_url", "image_url": { "url": data_url } }
                ]
            }],
            "max_tokens": 150,
            "temperature": 0.3
        });

        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body);

        if let Some(api_key) = &config.vlm_api_key {
            if !api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", api_key));
            }
        }

        let response = req
            .send()
            .await
            .map_err(|e| format!("VLM request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("VLM API error ({}): {}", status, error_text));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse VLM response: {}", e))?;

        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            return Err("VLM returned empty response".to_string());
        }

        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        analyze_screenshot, build_proactive_vision_instruction, normalize_vlm_chat_base_url,
    };
    use crate::vision::config::VisionConfig;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn normalizes_llama_cpp_root_to_openai_compatible_chat_url() {
        assert_eq!(
            normalize_vlm_chat_base_url("llama_cpp", Some("http://127.0.0.1:8080")),
            "http://127.0.0.1:8080/v1"
        );
        assert_eq!(
            normalize_vlm_chat_base_url(
                "llama_cpp",
                Some("http://127.0.0.1:8080/v1/chat/completions")
            ),
            "http://127.0.0.1:8080/v1"
        );
    }

    #[test]
    fn normalizes_ollama_root_to_openai_compatible_chat_url() {
        assert_eq!(
            normalize_vlm_chat_base_url("ollama", Some("http://localhost:11434")),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn proactive_vision_instruction_contains_safety_constraints() {
        let instruction = build_proactive_vision_instruction();

        assert!(!instruction.contains("用户的电脑屏幕上目前正在显示的是"));
        assert!(!instruction.contains("VS Code with Rust source open"));
        assert!(instruction.contains("上方的屏幕上下文"));
        assert!(instruction.contains("不要表现得像在监视用户"));
        assert!(instruction.contains("代码是谁写的或谁改的"));
    }

    #[tokio::test]
    async fn analyze_screenshot_uses_anthropic_messages_api() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [
                    { "type": "text", "text": "A concise screen description." }
                ]
            })))
            .mount(&mock_server)
            .await;

        let mut config = VisionConfig::default();
        config.vlm_provider = "anthropic".to_string();
        config.vlm_base_url = Some(format!("{}/v1", mock_server.uri()));
        config.vlm_model = "claude-3-5-sonnet-20241022".to_string();
        config.vlm_api_key = Some("test-key".to_string());

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let description = analyze_screenshot(&client, &config, b"fake-jpeg", None)
            .await
            .expect("anthropic screenshot analysis should succeed");

        assert_eq!(description, "A concise screen description.");

        let requests = mock_server
            .received_requests()
            .await
            .expect("mock server should capture requests");
        assert_eq!(requests.len(), 1);

        let payload: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("request body should be JSON");
        assert_eq!(payload["model"], "claude-3-5-sonnet-20241022");
        assert_eq!(payload["max_tokens"], 150);
        assert_eq!(payload["temperature"], 0.3);
        assert_eq!(payload["messages"][0]["role"], "user");
        assert_eq!(payload["messages"][0]["content"][0]["type"], "text");
        assert_eq!(payload["messages"][0]["content"][1]["type"], "image");
        assert_eq!(
            payload["messages"][0]["content"][1]["source"]["type"],
            "base64"
        );
        assert_eq!(
            payload["messages"][0]["content"][1]["source"]["media_type"],
            "image/jpeg"
        );
        assert!(payload["messages"][0]["content"][1]["source"]["data"]
            .as_str()
            .is_some_and(|data| !data.is_empty()));
    }
}
