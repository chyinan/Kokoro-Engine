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
            tracing::info!(target = "vision", "Watcher already running");
            return;
        }
        let watcher = self.clone();

        tokio::spawn(async move {
            tracing::info!(target = "vision", "Watcher started");
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
                        tracing::error!(target = "vision", "Capture failed: {}", e);
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
                    tracing::info!(target = "vision", "Screen changed, analyzing with VLM...");

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
                                target = "vision",
                                "Observation: {}",
                                &description[..description.len().min(100)]
                            );
                            watcher.context.update(description.clone()).await;
                            let _ = app_handle.emit("vision-observation", &description);

                            // 冷却检查：距上次 proactive 至少间隔 interval_secs
                            let cooldown =
                                std::time::Duration::from_secs(config.interval_secs as u64);
                            let ready =
                                !proactive_initialized || last_proactive_ts.elapsed() >= cooldown;
                            if ready {
                                proactive_initialized = true;
                                last_proactive_ts = std::time::Instant::now();
                                let instruction = format!(
                                    "You just noticed the user's screen changed. Current observation: {}. \
                                    React naturally — comment, ask, or just say something relevant. Keep it brief.",
                                    description
                                );
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
                        Err(e) => {
                            tracing::error!(target = "vision", "VLM analysis failed: {}", e);
                        }
                    }

                    prev_screenshot = Some(screenshot);
                } else {
                    tracing::info!(target = "vision", "No significant change, skipping analysis");
                }

                // 4. Sleep for the configured interval
                tokio::time::sleep(std::time::Duration::from_secs(config.interval_secs as u64))
                    .await;
            }

            tracing::info!(target = "vision", "Watcher stopped");
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

const VISION_PROMPT: &str = "Describe what you see on this screen briefly (1-2 sentences). Focus on what the user is currently doing — what application is open, what content is visible. Be concise and factual.";

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
        let base_url = config
            .vlm_base_url
            .as_deref()
            .unwrap_or("http://localhost:11434/v1");

        let url = format!("{}/chat/completions", base_url);

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
