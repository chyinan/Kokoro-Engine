//! Vision Watcher — background loop that captures screen and analyzes with VLM.

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
}

impl VisionWatcher {
    pub fn new(config: VisionConfig) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            config: Arc::new(RwLock::new(config)),
            context: VisionContext::new(),
        }
    }

    /// Start the background vision loop.
    pub fn start(&self, app_handle: AppHandle) {
        if self.running.load(Ordering::Relaxed) {
            println!("[Vision] Watcher already running");
            return;
        }

        self.running.store(true, Ordering::Relaxed);
        let watcher = self.clone();

        tokio::spawn(async move {
            println!("[Vision] Watcher started");
            let _ = app_handle.emit("vision-status", "active");

            let client = Client::new();
            let mut prev_screenshot: Option<Vec<u8>> = None;

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
                        eprintln!("[Vision] Capture failed: {}", e);
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
                    println!("[Vision] Screen changed, analyzing with VLM...");

                    // 3. Send to VLM for analysis
                    match analyze_screenshot(&client, &config, &screenshot).await {
                        Ok(description) => {
                            println!(
                                "[Vision] Observation: {}",
                                &description[..description.len().min(100)]
                            );
                            watcher.context.update(description.clone()).await;
                            let _ = app_handle.emit("vision-observation", &description);
                        }
                        Err(e) => {
                            eprintln!("[Vision] VLM analysis failed: {}", e);
                        }
                    }

                    prev_screenshot = Some(screenshot);
                } else {
                    println!("[Vision] No significant change, skipping analysis");
                }

                // 4. Sleep for the configured interval
                tokio::time::sleep(std::time::Duration::from_secs(config.interval_secs as u64))
                    .await;
            }

            println!("[Vision] Watcher stopped");
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

/// Send a screenshot to the VLM for analysis using OpenAI-compatible vision API.
pub async fn analyze_screenshot(
    client: &Client,
    config: &VisionConfig,
    screenshot: &[u8],
) -> Result<String, String> {
    let base_url = config
        .vlm_base_url
        .as_deref()
        .unwrap_or("http://localhost:11434/v1");

    let url = format!("{}/chat/completions", base_url);

    // Encode screenshot as base64 data URL
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, screenshot);
    let data_url = format!("data:image/jpeg;base64,{}", b64);

    let body = serde_json::json!({
        "model": config.vlm_model,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "Describe what you see on this screen briefly (1-2 sentences). Focus on what the user is currently doing — what application is open, what content is visible. Be concise and factual."
                    },
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": data_url
                        }
                    }
                ]
            }
        ],
        "max_tokens": 150,
        "temperature": 0.3
    });

    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body);

    // Add API key if configured
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
