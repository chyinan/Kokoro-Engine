// pattern: Imperative Shell
//! Vision Watcher — background loop that captures screen and analyzes with VLM.

use crate::ipc::{
    events::{ProactiveTrigger, VisionObservationEvent, VisionStatus, VisionStatusChanged},
    wire::emit,
};
use crate::lifecycle::{AppLifecycle, TaskHandle, TaskScope, TaskStage};
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::messages::user_message_with_images;
use crate::llm::provider::{LlmParams, LlmProvider};
use crate::llm::service::LlmService;
use crate::vision::capture::{
    capture_screen_with_options, has_significant_change, CaptureOptions, CapturedImage,
};
use crate::vision::config::VisionConfig;
use crate::vision::context::VisionContext;
use crate::vision::context::{AnalysisDispatch, VisionFrame};
use futures::FutureExt;
use reqwest::Client;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::{Mutex, OnceCell, RwLock};
use tokio_util::sync::CancellationToken;

struct VisionRun {
    stop: CancellationToken,
    producer: Mutex<TaskHandle<()>>,
    result: OnceCell<Result<(), String>>,
}

impl VisionRun {
    fn is_running(&self) -> bool {
        self.result.get().is_none()
            && self
                .producer
                .try_lock()
                .map_or(true, |task| !task.is_finished())
    }

    async fn joined(&self) -> Result<(), String> {
        self.result
            .get_or_init(|| async {
                self.producer
                    .lock()
                    .await
                    .join()
                    .await
                    .map_err(|error| error.to_string())
            })
            .await
            .clone()
    }
}

const STOP_WAIT: Duration = Duration::from_secs(2);

/// Shared handle to control the watcher loop.
#[derive(Clone)]
pub struct VisionWatcher {
    running: Arc<AtomicBool>,
    producer_generation: Arc<AtomicU64>,
    scope: TaskScope,
    run: Arc<Mutex<Option<Arc<VisionRun>>>>,
    pub config: Arc<RwLock<VisionConfig>>,
    pub context: VisionContext,
    pub llm_service: Option<LlmService>,
    pub client: Client,
}

impl VisionWatcher {
    pub fn new(config: VisionConfig) -> Self {
        Self::new_with_scope(config, AppLifecycle::new().scope(TaskStage::Work, "vision"))
    }

    pub(crate) fn new_with_scope(config: VisionConfig, scope: TaskScope) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            producer_generation: Arc::new(AtomicU64::new(0)),
            scope,
            run: Arc::new(Mutex::new(None)),
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

    async fn auto_capture_active(&self) -> bool {
        if !self.running.load(Ordering::Relaxed) {
            return false;
        }
        let config = self.config.read().await;
        config.vlm_enabled && config.auto_vision_enabled
    }

    /// Start the background producer loop. VLM analysis is dispatched asynchronously
    /// with at most one in-flight request and newest-only pending frame state.
    pub(crate) async fn start(&self, app_handle: AppHandle) -> Result<(), String> {
        let _start = self
            .scope
            .lease("watcher-start")
            .map_err(|error| error.to_string())?;
        let mut slot = self.run.lock().await;
        if let Some(run) = slot.as_ref().filter(|run| run.is_running()) {
            if run.stop.is_cancelled() {
                return Err("Vision watcher is still stopping".into());
            }
            tracing::info!(target: "vision", "Watcher already running");
            return Ok(());
        }
        self.running.store(true, Ordering::Release);
        let producer_generation = self.producer_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let watcher = self.clone();
        let stop = self.scope.token().child_token();
        let worker_stop = stop.clone();

        let result = self.scope.spawn_durable("watcher", async move {
            watcher.context.clear_auto_state_on_disable().await;
            if worker_stop.is_cancelled() || !watcher.is_producer_current(producer_generation) {
                return;
            }
            tracing::info!(target: "vision", "Watcher started");
            let _ = emit(&app_handle, VisionStatusChanged(VisionStatus::Active));
            let mut prev_screenshot: Option<Vec<u8>> = None;
            let mut last_capture_warning: Option<String> = None;
            let mut analyses = Vec::new();

            while !worker_stop.is_cancelled() && watcher.is_producer_current(producer_generation) {
                analyses.retain(|task: &TaskHandle<()>| !task.is_finished());
                let config = watcher.config.read().await.clone();
                if !config.vlm_enabled || !config.auto_vision_enabled {
                    watcher.context.clear_auto_state_on_disable().await;
                    sleep_or_stop(&worker_stop, Duration::from_secs(2)).await;
                    continue;
                }

                if watcher.context.should_pause_auto_capture().await {
                    sleep_or_stop(&worker_stop, Duration::from_millis(250)).await;
                    continue;
                }

                let interval = Duration::from_secs(config.capture_interval_secs as u64);
                let capture_options = CaptureOptions {
                    display_id: config.display_id.clone(),
                    region: config.vlm_region,
                };
                let capture = watcher.capture(capture_options).await;
                if worker_stop.is_cancelled() {
                    break;
                }
                let captured = match capture {
                    Ok(image) => image,
                    Err(error) => {
                        tracing::error!(target: "vision", "Capture failed: {}", error);
                        watcher.context.set_last_error(error).await;
                        sleep_or_stop(&worker_stop, interval).await;
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
                    sleep_or_stop(&worker_stop, interval).await;
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

                if let AnalysisDispatch::Dispatch { frame, generation } =
                    watcher.context.submit_auto_frame(frame).await
                {
                    let analysis_watcher = watcher.clone();
                    let analysis_app = app_handle.clone();
                    let analysis_stop = worker_stop.clone();
                    let Ok(task) = watcher.scope.spawn("analysis", async move {
                        run_analysis_chain(
                            analysis_watcher,
                            analysis_app,
                            frame,
                            generation,
                            producer_generation,
                            analysis_stop,
                        )
                        .await;
                    }) else {
                        break;
                    };
                    analyses.push(task);
                }

                sleep_or_stop(&worker_stop, interval).await;
            }

            worker_stop.cancel();
            for mut task in analyses {
                let _ = task.join().await;
            }
            tracing::info!(target: "vision", "Watcher stopped");
            if watcher.is_producer_current(producer_generation) {
                watcher.running.store(false, Ordering::Release);
                watcher.context.clear_auto_state_on_disable().await;
            }
            let _ = emit(&app_handle, VisionStatusChanged(VisionStatus::Inactive));
        });
        let producer = match result {
            Ok(task) => task,
            Err(error) => {
                self.running.store(false, Ordering::Release);
                return Err(error.to_string());
            }
        };
        *slot = Some(Arc::new(VisionRun {
            stop,
            producer: Mutex::new(producer),
            result: OnceCell::new(),
        }));
        Ok(())
    }

    fn is_producer_current(&self, generation: u64) -> bool {
        self.running.load(Ordering::Acquire)
            && self.producer_generation.load(Ordering::Acquire) == generation
    }

    /// Stop capture and inference and confirm their run has finished before restarting.
    pub(crate) async fn stop(&self) -> Result<(), String> {
        tokio::time::timeout(STOP_WAIT, self.stop_and_wait())
            .await
            .map_err(|_| "Vision watcher is still stopping; capture has not finished".to_string())?
    }

    async fn stop_and_wait(&self) -> Result<(), String> {
        let (run, context_generation) = {
            let slot = self.run.lock().await;
            self.running.store(false, Ordering::Release);
            self.producer_generation.fetch_add(1, Ordering::SeqCst);
            let context_generation = self.context.invalidate_auto_generation();
            if let Some(run) = slot.as_ref() {
                run.stop.cancel();
            }
            (slot.clone(), context_generation)
        };
        self.context
            .clear_auto_state_if_current(context_generation)
            .await;
        let Some(run) = run else {
            return Ok(());
        };
        let outcome = run.joined().await;
        let mut slot = self.run.lock().await;
        if slot
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, &run))
        {
            *slot = None;
        }
        outcome
    }

    pub(crate) async fn capture(&self, options: CaptureOptions) -> Result<CapturedImage, String> {
        let mut task = self
            .scope
            .spawn_blocking("screen-capture", move || {
                capture_screen_with_options(&options)
            })
            .map_err(|error| error.to_string())?;
        task.join().await.map_err(|error| error.to_string())?
    }

    pub(crate) fn work_scope(&self) -> &TaskScope {
        &self.scope
    }
}

async fn sleep_or_stop(stop: &CancellationToken, duration: Duration) {
    tokio::select! {
        biased;
        _ = stop.cancelled() => {},
        _ = tokio::time::sleep(duration) => {},
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn stop_keeps_the_old_run_owned_until_capture_ends() {
        let watcher = VisionWatcher::new(VisionConfig::default());
        let stop = watcher.scope.token().child_token();
        let (release, capture_end) = tokio::sync::oneshot::channel();
        let producer = watcher
            .scope
            .spawn_durable("simulated native capture", async move {
                capture_end.await.unwrap();
            })
            .unwrap();
        let run = Arc::new(VisionRun {
            stop: stop.clone(),
            producer: Mutex::new(producer),
            result: OnceCell::new(),
        });
        *watcher.run.lock().await = Some(run.clone());

        let error = watcher.stop().await.unwrap_err();
        assert!(error.contains("still stopping"));
        assert!(stop.is_cancelled());
        assert!(run.is_running());
        assert!(watcher.run.lock().await.is_some());

        release.send(()).unwrap();
        watcher.stop().await.unwrap();
        assert!(!run.is_running());
        assert!(watcher.run.lock().await.is_none());
    }

    #[tokio::test]
    async fn provider_panic_releases_analysis_slot_and_keeps_newest_frame() {
        let context = VisionContext::new();
        let frame = |id: &str| VisionFrame {
            id: id.into(),
            captured_at: chrono::Utc::now(),
            jpeg_bytes: Vec::new(),
            display_id: None,
            region: None,
            image_hash: id.into(),
        };
        let first = frame("first");
        let AnalysisDispatch::Dispatch { generation, .. } =
            context.submit_auto_frame(first.clone()).await
        else {
            panic!("the first analysis must be admitted");
        };
        context.submit_auto_frame(frame("newest")).await;

        let failed = observe_provider_result(async { panic!("controlled provider failure") }).await;
        assert_eq!(failed, Err("Vision analysis provider panicked".into()));
        let completion = context
            .finish_auto_analysis(generation, &first, failed)
            .await;
        assert!(!completion.recorded);
        let newest = completion.next_frame.unwrap();
        assert_eq!(newest.id, "newest");
        context
            .finish_auto_analysis(generation, &newest, Ok("next observation".into()))
            .await;
        assert!(matches!(
            context.submit_auto_frame(frame("later")).await,
            AnalysisDispatch::Dispatch { .. }
        ));
    }
}

// Internal provider failure must still go through finish_auto_analysis;
// otherwise the frame queue would keep its in-flight marker indefinitely.
async fn observe_provider_result<F>(analysis: F) -> Result<String, String>
where
    F: Future<Output = Result<String, String>>,
{
    AssertUnwindSafe(analysis)
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err("Vision analysis provider panicked".into()))
}

async fn run_analysis_chain(
    watcher: VisionWatcher,
    app_handle: AppHandle,
    first_frame: VisionFrame,
    generation: crate::vision::context::AutoAnalysisGeneration,
    producer_generation: u64,
    stop: CancellationToken,
) {
    let mut current = first_frame;
    loop {
        if stop.is_cancelled() {
            break;
        }
        if !watcher.is_producer_current(producer_generation) || !watcher.auto_capture_active().await
        {
            if !watcher.is_producer_current(producer_generation) {
                break;
            }
            watcher.context.clear_auto_state_on_disable().await;
            break;
        }

        tracing::info!(target: "vision", "Screen changed, analyzing with VLM...");
        let config = watcher.config.read().await.clone();
        let result = tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            result = observe_provider_result(analyze_screenshot(
                &watcher.client,
                &config,
                &current.jpeg_bytes,
                watcher.llm_service.as_ref(),
            )) => result,
        };

        match &result {
            Ok(description) => {
                tracing::debug!(target: "vision", "Observation: {}", description);
            }
            Err(error) => tracing::error!(target: "vision", "VLM analysis failed: {}", error),
        }

        let completion_config = watcher.config.read().await.clone();
        if !watcher.is_producer_current(producer_generation)
            || !completion_config.vlm_enabled
            || !completion_config.auto_vision_enabled
        {
            if !watcher.is_producer_current(producer_generation) {
                break;
            }
            watcher.context.clear_auto_state_on_disable().await;
            break;
        }

        let should_emit_proactive = result.is_ok() && completion_config.proactive_vision_enabled;
        let completion = watcher
            .context
            .finish_auto_analysis(generation, &current, result)
            .await;

        if should_emit_proactive && completion.recorded {
            if let Some(observation) = watcher
                .context
                .latest_completed_observation(chrono::Utc::now())
                .await
            {
                let _ = emit(
                    &app_handle,
                    VisionObservationEvent {
                        summary: observation.summary,
                        captured_at: observation.captured_at.to_rfc3339(),
                        source: observation.source,
                    },
                );
            }
        }

        if should_emit_proactive && completion.recorded {
            emit_proactive_vision_comment(&app_handle).await;
        }

        match completion.next_frame {
            Some(next) => current = next,
            None => break,
        }
    }
}

async fn emit_proactive_vision_comment(app_handle: &AppHandle) {
    tracing::info!(target: "vision", "Vision screen-comment trigger fired");
    let Some(chat) = app_handle.try_state::<crate::chat::application::ChatService>() else {
        return;
    };
    let Some(state) = app_handle.try_state::<crate::ai::context::AIOrchestrator>() else {
        return;
    };
    let Ok(scope) = state.desktop_admission_scope().await else {
        return;
    };
    let Some(request_id) =
        chat.submit_proactive_bound(build_proactive_vision_instruction().into(), scope.clone())
    else {
        return;
    };
    let _ = emit(
        app_handle,
        ProactiveTrigger {
            trigger: "vision".into(),
            character_id: scope.0,
            conversation_id: scope.1,
            client_request_id: request_id,
            instruction: build_proactive_vision_instruction().into(),
            idle_seconds: None,
        },
    );
}

const VISION_PROMPT: &str = "Describe the screenshot in 2-3 concise, information-rich sentences. Include the active application/window, important visible UI text, and the most visually prominent non-UI content such as characters, artwork, background, objects, colors, and scene details. If a fictional/anime character is clearly recognizable, you may name them; do not identify real people from appearance alone. Do not infer authorship, private intent, emotions, or anything off-screen. If something is unclear, say that briefly.";
const DEFAULT_OLLAMA_VLM_BASE_URL: &str = "http://localhost:11434/v1";
const DEFAULT_OPENAI_VLM_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_ANTHROPIC_VLM_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_LLAMA_CPP_VLM_BASE_URL: &str = "http://127.0.0.1:8080";

fn build_proactive_vision_instruction() -> &'static str {
    "结合上方屏幕上下文和当前角色的人设，判断这次屏幕变化是否值得轻声回应。\
    如果有新页面、新应用、醒目的视觉内容、明显状态变化或适合角色自然吐槽的点，就说一句简短评论。\
    如果只是同一界面、普通滚动、日志/聊天内容变化，或评论会和最近说过的内容相似，请只回复 PASS。"
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

fn should_bypass_proxy_for_vlm(provider: &str) -> bool {
    matches!(provider, "ollama" | "llama_cpp")
}

fn no_proxy_client_for_vlm() -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| format!("Failed to build local VLM HTTP client: {}", e))
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
        let local_no_proxy_client;
        let request_client = if should_bypass_proxy_for_vlm(&config.vlm_provider) {
            local_no_proxy_client = no_proxy_client_for_vlm()?;
            &local_no_proxy_client
        } else {
            client
        };

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

        let mut req = request_client
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
    fn proactive_vision_instruction_defaults_to_pass() {
        let instruction = build_proactive_vision_instruction();

        assert!(!instruction.contains("用户的电脑屏幕上目前正在显示的是"));
        assert!(!instruction.contains("VS Code with Rust source open"));
        assert!(instruction.contains("上方屏幕上下文"));
        assert!(instruction.contains("值得轻声回应"));
        assert!(instruction.contains("新页面、新应用"));
        assert!(instruction.contains("同一界面"));
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
