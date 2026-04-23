//! Vision Watcher — background loop that captures screen and analyzes with VLM.

use crate::llm::messages::user_message_with_images;
use crate::llm::provider::LlmParams;
use crate::llm::service::LlmService;
use crate::vision::capture::{capture_screen, has_significant_change};
use crate::vision::config::VisionConfig;
use crate::vision::context::VisionContext;
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

    /// Start the background vision loop.
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

            let client = watcher.client.clone();
            let mut prev_screenshot: Option<Vec<u8>> = None;
            // 用一个足够久远的时间初始化，确保第一次触发不受冷却限制
            let mut last_proactive_ts = std::time::Instant::now();
            let mut proactive_initialized = false;

            loop {
                if !watcher.running.load(Ordering::Relaxed) {
                    break;
                }

                let config = watcher.config.read().await.clone();
                if !config.enabled {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }

                // 1. Capture screen
                let screenshot = match capture_screen() {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::error!(target: "vision", "Capture failed: {}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(
                            config.interval_secs as u64,
                        ))
                        .await;
                        continue;
                    }
                };

                // 2. Check for significant change
                let changed = match &prev_screenshot {
                    Some(prev) => {
                        has_significant_change(prev, &screenshot, config.change_threshold)
                    }
                    None => true, // First capture is always "changed"
                };

                if changed {
                    tracing::info!(target: "vision", "Screen changed, analyzing with VLM...");

                    // 3. Send to VLM for analysis
                    match analyze_screenshot(
                        &client,
                        &config,
                        &screenshot,
                        watcher.llm_service.as_ref(),
                    )
                    .await
                    {
                        Ok(description) => {
                            tracing::info!(
                                target: "vision",
                                "Observation: {}",
                                &description[..description.len().min(100)]
                            );
                            watcher.context.update(description.clone()).await;
                            let _ = app_handle.emit("vision-observation", &description);

                            if config.proactive_enabled {
                                // 冷却检查：距上次 proactive 至少间隔 interval_secs
                                let cooldown =
                                    std::time::Duration::from_secs(config.interval_secs as u64);
                                let ready = !proactive_initialized
                                    || last_proactive_ts.elapsed() >= cooldown;
                                if ready {
                                    proactive_initialized = true;
                                    last_proactive_ts = std::time::Instant::now();
                                    let instruction =
                                        build_proactive_vision_instruction(&description);
                                    let _ = app_handle.emit(
                                        "proactive-trigger",
                                        serde_json::json!({
                                            "trigger": "vision",
                                            "idle_seconds": 0,
                                            "instruction": instruction,
                                        }),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(target: "vision", "VLM analysis failed: {}", e);
                        }
                    }

                    prev_screenshot = Some(screenshot);
                } else {
                    tracing::info!(target: "vision", "No significant change, skipping analysis");
                }

                // 4. Sleep for the configured interval
                tokio::time::sleep(std::time::Duration::from_secs(config.interval_secs as u64))
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
        tokio::spawn(async move { ctx.clear().await });
    }
}

const VISION_PROMPT: &str = "Describe the screenshot in 2-3 concise, information-rich sentences. Include the active application/window, important visible UI text, and the most visually prominent non-UI content such as characters, artwork, background, objects, colors, and scene details. If a fictional/anime character is clearly recognizable, you may name them; do not identify real people from appearance alone. Do not infer authorship, private intent, emotions, or anything off-screen. If something is unclear, say that briefly.";
const DEFAULT_OLLAMA_VLM_BASE_URL: &str = "http://localhost:11434/v1";
const DEFAULT_OPENAI_VLM_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_LLAMA_CPP_VLM_BASE_URL: &str = "http://127.0.0.1:8080";

fn build_proactive_vision_instruction(description: &str) -> String {
    format!(
        "用户的电脑屏幕上目前正在显示的是：{}。请结合当前角色的人设和性格，对屏幕内容做一句自然、简短、轻量的评论。\
        只评论屏幕上直接可见的内容，不要表现得像在监视用户，也不要声称知道隐藏信息、用户想法、代码是谁写的或谁改的，或屏幕外发生的事。\
        避免重复、恐怖、威胁、夸张或令人不适的措辞；如果内容不清楚或可能敏感，就保持中性温和。",
        description
    )
}

fn default_vlm_base_url(provider: &str) -> &'static str {
    match provider {
        "llama_cpp" => DEFAULT_LLAMA_CPP_VLM_BASE_URL,
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
/// Otherwise uses the independently configured VLM endpoint (ollama / openai).
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
    } else {
        // ── Independent VLM endpoint (ollama / openai) ─────────────────────
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
    use super::{build_proactive_vision_instruction, normalize_vlm_chat_base_url};

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
        let instruction = build_proactive_vision_instruction("VS Code with Rust source open");

        assert!(instruction.contains("用户的电脑屏幕上目前正在显示的是"));
        assert!(instruction.contains("不要表现得像在监视用户"));
        assert!(instruction.contains("代码是谁写的或谁改的"));
    }
}
