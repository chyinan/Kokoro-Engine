// pattern: Imperative Shell

//! Experimental bridge to the locally installed Codex app-server.
//!
//! This module deliberately delegates authentication, model providers, custom endpoints,
//! and the Codex backend protocol to the user's installed Codex runtime. Kokoro only owns
//! the child process, JSONL RPC transport, a read-only ephemeral thread, and event mapping.

use crate::llm::codex_runtime_protocol::{
    build_initialize_params, build_rpc_notification, build_rpc_request,
    build_thread_inject_items_params, build_thread_start_params, build_turn_interrupt_params,
    build_turn_start_params, parse_model_list_page, parse_runtime_notification,
    prepare_inference_inputs, resolve_dynamic_tool_call_name, validate_tool_call, RuntimeEvent,
};
use crate::llm::provider::{
    LlmChatMessage, LlmParams, LlmProvider, LlmStreamEvent, LlmToolDefinition,
};
use async_trait::async_trait;
use futures::{channel::mpsc, Stream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
#[cfg(windows)]
use std::path::PathBuf;
use std::path::PathBuf as RuntimePathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{broadcast, oneshot, Mutex};

const DEFAULT_CODEX_BINARY: &str = "codex";
const CODEX_BINARY_ENV: &str = "KOKORO_CODEX_BINARY";
const NOTIFICATION_BUFFER: usize = 256;
const MAX_VERSION_OUTPUT_BYTES: usize = 256;
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const TURN_START_RESPONSE_TIMEOUT: Duration = Duration::from_secs(300);
const FIRST_OUTPUT_TIMEOUT: Duration = Duration::from_secs(300);
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const TOTAL_TURN_TIMEOUT: Duration = Duration::from_secs(900);
const MAX_MODEL_LIST_PAGES: usize = 64;

static NEXT_RUNTIME_CWD_ID: AtomicU64 = AtomicU64::new(1);

type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Debug, Clone, Serialize)]
pub struct CodexRuntimeInfo {
    pub status: String,
    pub binary: String,
    pub version: Option<String>,
}

pub struct CodexRuntimeProvider {
    provider_id: String,
    model_override: Option<String>,
    binary: String,
    server: Arc<Mutex<Option<Arc<CodexAppServer>>>>,
    drop_server: Arc<StdMutex<Option<Arc<CodexAppServer>>>>,
}

impl CodexRuntimeProvider {
    pub fn new(id: String, model_override: Option<String>) -> Self {
        Self {
            provider_id: id,
            model_override: model_override.and_then(non_empty_string),
            binary: resolve_codex_binary(),
            server: Arc::new(Mutex::new(None)),
            drop_server: Arc::new(StdMutex::new(None)),
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>, String> {
        let server = self.ensure_server().await?;
        let result = async {
            let mut models = Vec::new();
            let mut cursor: Option<String> = None;
            let mut seen_cursors = std::collections::HashSet::new();
            for _ in 0..MAX_MODEL_LIST_PAGES {
                let mut params = json!({ "includeHidden": false });
                if let Some(cursor_value) = cursor.as_deref() {
                    params["cursor"] = Value::String(cursor_value.to_string());
                }
                let response = server.request("model/list", params).await?;
                let (page, next_cursor) = parse_model_list_page(&response)?;
                for model in page {
                    if !models.contains(&model) {
                        models.push(model);
                    }
                }
                let Some(next_cursor) = next_cursor else {
                    return Ok(models);
                };
                if !seen_cursors.insert(next_cursor.clone()) {
                    return Err(
                        "Codex app-server model/list returned a repeated cursor".to_string()
                    );
                }
                cursor = Some(next_cursor);
            }
            Err(format!(
                "Codex app-server model/list exceeded the {} page safety limit",
                MAX_MODEL_LIST_PAGES
            ))
        }
        .await;
        // Model discovery is a short-lived operation. Do not leave a background
        // app-server process behind after the settings action completes.
        self.invalidate_server(&server).await;
        result
    }

    async fn ensure_server(&self) -> Result<Arc<CodexAppServer>, String> {
        let mut guard = self.server.lock().await;
        if let Some(server) = guard.as_ref() {
            if !server.is_exited() {
                return Ok(server.clone());
            }
        }

        let server = CodexAppServer::start(&self.binary).await?;
        *guard = Some(server.clone());
        if let Ok(mut drop_guard) = self.drop_server.lock() {
            *drop_guard = Some(server.clone());
        }
        Ok(server)
    }

    async fn invalidate_server(&self, server: &Arc<CodexAppServer>) {
        let should_remove = self
            .server
            .lock()
            .await
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, server));
        if should_remove {
            *self.server.lock().await = None;
            if let Ok(mut drop_guard) = self.drop_server.lock() {
                *drop_guard = None;
            }
            server.shutdown().await;
        }
    }

    async fn start_turn_on_server(
        &self,
        server: Arc<CodexAppServer>,
        messages: Vec<LlmChatMessage>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<
        (
            Arc<CodexAppServer>,
            broadcast::Receiver<Value>,
            String,
            String,
            Instant,
        ),
        String,
    > {
        let inputs = prepare_inference_inputs(messages, &tools)?;
        let notifications = server.subscribe();

        let thread_response = server
            .request(
                "thread/start",
                build_thread_start_params(self.model_override.as_deref(), &tools),
            )
            .await?;
        let thread_id = thread_response
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "Codex app-server thread/start returned no thread id".to_string())?
            .to_string();

        if !inputs.history.is_empty() {
            server
                .request(
                    "thread/inject_items",
                    build_thread_inject_items_params(&thread_id, inputs.history),
                )
                .await?;
        }

        let turn_started_at = Instant::now();
        let turn_response = server
            .request_with_timeout(
                "turn/start",
                build_turn_start_params(&thread_id, inputs.turn_input, inputs.tool_output),
                TURN_START_RESPONSE_TIMEOUT,
            )
            .await?;
        let turn_id = turn_response
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "Codex app-server turn/start returned no turn id".to_string())?
            .to_string();

        Ok((server, notifications, thread_id, turn_id, turn_started_at))
    }

    async fn start_turn(
        &self,
        messages: Vec<LlmChatMessage>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<
        (
            Arc<CodexAppServer>,
            broadcast::Receiver<Value>,
            String,
            String,
            Instant,
        ),
        String,
    > {
        let mut last_error = None;
        for attempt in 0..2 {
            let server = self.ensure_server().await?;
            match self
                .start_turn_on_server(server.clone(), messages.clone(), tools.clone())
                .await
            {
                Ok(result) => return Ok(result),
                Err(error) => {
                    last_error = Some(error);
                    self.invalidate_server(&server).await;
                    if attempt == 0 {
                        continue;
                    }
                }
            }
        }

        Err(format!(
            "Codex runtime request failed: {}",
            last_error.unwrap_or_else(|| "unknown app-server error".to_string())
        ))
    }

    async fn create_stream(
        &self,
        messages: Vec<LlmChatMessage>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        let (server, mut notifications, thread_id, turn_id, turn_started_at) =
            self.start_turn(messages, tools.clone()).await?;
        let (mut tx, rx) = mpsc::unbounded::<Result<LlmStreamEvent, String>>();
        let finished = Arc::new(AtomicBool::new(false));
        let task_finished = finished.clone();
        let event_thread_id = thread_id.clone();
        let event_turn_id = turn_id.clone();
        let allowed_tools = tools;
        let interrupt_server = server.clone();
        let interrupt_thread_id = thread_id.clone();
        let interrupt_turn_id = turn_id.clone();

        tokio::spawn(async move {
            let mut emitted_text = false;
            let mut received_first_output = false;
            let mut last_output_at = None;
            loop {
                let elapsed = turn_started_at.elapsed();
                if elapsed >= TOTAL_TURN_TIMEOUT {
                    spawn_turn_interrupt(
                        interrupt_server.clone(),
                        &interrupt_thread_id,
                        &interrupt_turn_id,
                    );
                    let _ = tx.start_send(Err(format!(
                        "Codex turn exceeded the total timeout of {} seconds",
                        TOTAL_TURN_TIMEOUT.as_secs()
                    )));
                    task_finished.store(true, Ordering::Release);
                    return;
                }
                let phase_timeout = if received_first_output {
                    STREAM_IDLE_TIMEOUT
                } else {
                    FIRST_OUTPUT_TIMEOUT
                };
                let phase_elapsed = last_output_at
                    .map(|last_output_at: Instant| last_output_at.elapsed())
                    .unwrap_or(elapsed);
                let wait_timeout = phase_timeout
                    .saturating_sub(phase_elapsed)
                    .min(TOTAL_TURN_TIMEOUT.saturating_sub(elapsed));
                let notification =
                    match tokio::time::timeout(wait_timeout, notifications.recv()).await {
                        Ok(result) => match result {
                            Ok(notification) => notification,
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => {
                                let _ = tx.start_send(Err(
                                    "Codex app-server notification stream closed unexpectedly"
                                        .to_string(),
                                ));
                                task_finished.store(true, Ordering::Release);
                                return;
                            }
                        },
                        Err(_) => {
                            let message = if received_first_output {
                                format!(
                                    "Codex stream was idle for {} seconds",
                                    STREAM_IDLE_TIMEOUT.as_secs()
                                )
                            } else {
                                format!(
                                    "Codex turn produced no first output within {} seconds",
                                    FIRST_OUTPUT_TIMEOUT.as_secs()
                                )
                            };
                            spawn_turn_interrupt(
                                interrupt_server.clone(),
                                &interrupt_thread_id,
                                &interrupt_turn_id,
                            );
                            let _ = tx.start_send(Err(message));
                            task_finished.store(true, Ordering::Release);
                            return;
                        }
                    };

                let Some(event) =
                    parse_runtime_notification(&notification, &event_thread_id, &event_turn_id)
                else {
                    continue;
                };

                match event {
                    RuntimeEvent::Stream(mut event) => {
                        received_first_output = true;
                        last_output_at = Some(Instant::now());
                        if matches!(event, LlmStreamEvent::Text(_)) {
                            emitted_text = true;
                        }
                        if let LlmStreamEvent::ToolCall(call) = &mut event {
                            if let Err(error) = resolve_dynamic_tool_call_name(call, &allowed_tools)
                                .and_then(|_| validate_tool_call(call, &allowed_tools))
                            {
                                spawn_turn_interrupt(
                                    interrupt_server.clone(),
                                    &interrupt_thread_id,
                                    &interrupt_turn_id,
                                );
                                let _ = tx.start_send(Err(error));
                                task_finished.store(true, Ordering::Release);
                                return;
                            }
                            task_finished.store(true, Ordering::Release);
                            if tx.start_send(Ok(event)).is_err() {
                                return;
                            }
                            return;
                        }
                        if tx.start_send(Ok(event)).is_err() {
                            task_finished.store(true, Ordering::Release);
                            return;
                        }
                    }
                    RuntimeEvent::Completed { status, text } => {
                        if status == "completed" {
                            if !emitted_text {
                                if let Some(text) = text.filter(|value| !value.is_empty()) {
                                    if tx.start_send(Ok(LlmStreamEvent::Text(text))).is_err() {
                                        task_finished.store(true, Ordering::Release);
                                        return;
                                    }
                                }
                            }
                            task_finished.store(true, Ordering::Release);
                            return;
                        }
                        let message = if status == "interrupted" {
                            "Codex turn was interrupted".to_string()
                        } else {
                            format!("Codex turn ended with status: {status}")
                        };
                        spawn_turn_interrupt(
                            interrupt_server.clone(),
                            &interrupt_thread_id,
                            &interrupt_turn_id,
                        );
                        let _ = tx.start_send(Err(message));
                        task_finished.store(true, Ordering::Release);
                        return;
                    }
                    RuntimeEvent::Status => {}
                    RuntimeEvent::Failed(error) => {
                        spawn_turn_interrupt(
                            interrupt_server.clone(),
                            &interrupt_thread_id,
                            &interrupt_turn_id,
                        );
                        let _ = tx.start_send(Err(error));
                        task_finished.store(true, Ordering::Release);
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(CodexEventStream {
            receiver: rx,
            server,
            thread_id,
            turn_id,
            finished,
        }))
    }
}

fn spawn_turn_interrupt(server: Arc<CodexAppServer>, thread_id: &str, turn_id: &str) {
    let thread_id = thread_id.to_string();
    let turn_id = turn_id.to_string();
    tokio::spawn(async move {
        let _ = server
            .request(
                "turn/interrupt",
                build_turn_interrupt_params(&thread_id, &turn_id),
            )
            .await;
    });
}

impl Drop for CodexRuntimeProvider {
    fn drop(&mut self) {
        // The stdout reader owns another Arc so async Drop cannot be relied on
        // to terminate the child. Best-effort synchronous kill prevents a
        // provider switch or app shutdown from orphaning app-server.
        if let Ok(guard) = self.drop_server.lock() {
            if let Some(server) = guard.as_ref() {
                server.terminate();
            }
        }
    }
}

#[async_trait]
impl LlmProvider for CodexRuntimeProvider {
    async fn chat(
        &self,
        messages: Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        self.chat_rich(
            messages.into_iter().map(LlmChatMessage::from).collect(),
            options,
        )
        .await
    }

    async fn chat_stream(
        &self,
        messages: Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
        let stream = self
            .chat_stream_rich(
                messages.into_iter().map(LlmChatMessage::from).collect(),
                options,
            )
            .await?;
        Ok(Box::pin(stream.filter_map(|event| async move {
            match event {
                Ok(LlmStreamEvent::Text(text)) => Some(Ok(text)),
                Ok(LlmStreamEvent::ReasoningContent(_))
                | Ok(LlmStreamEvent::ToolCall(_))
                | Ok(LlmStreamEvent::ProviderData(_)) => None,
                Err(error) => Some(Err(error)),
            }
        })))
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<async_openai::types::chat::ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        _tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        self.chat_stream_with_tools_rich(
            messages.into_iter().map(LlmChatMessage::from).collect(),
            options,
            _tools,
        )
        .await
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn chat_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        _options: Option<LlmParams>,
    ) -> Result<String, String> {
        let mut stream = self.create_stream(messages, Vec::new()).await?;
        let mut response = String::new();
        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::Text(text) => response.push_str(&text),
                LlmStreamEvent::ReasoningContent(_)
                | LlmStreamEvent::ToolCall(_)
                | LlmStreamEvent::ProviderData(_) => {}
            }
        }
        Ok(response)
    }

    async fn chat_stream_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        _options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        self.create_stream(messages, Vec::new()).await
    }

    async fn chat_stream_with_tools_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        let _ = options;
        self.create_stream(messages, tools).await
    }

    fn id(&self) -> &str {
        &self.provider_id
    }
}

struct CodexAppServer {
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<StdMutex<Child>>,
    next_request_id: AtomicU64,
    pending: PendingRequests,
    notifications: broadcast::Sender<Value>,
    exited: AtomicBool,
    working_dir: RuntimePathBuf,
}

struct CodexEventStream {
    receiver: mpsc::UnboundedReceiver<Result<LlmStreamEvent, String>>,
    server: Arc<CodexAppServer>,
    thread_id: String,
    turn_id: String,
    finished: Arc<AtomicBool>,
}

impl Stream for CodexEventStream {
    type Item = Result<LlmStreamEvent, String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}

impl Drop for CodexEventStream {
    fn drop(&mut self) {
        if self.finished.load(Ordering::Acquire) {
            return;
        }

        let server = self.server.clone();
        let thread_id = self.thread_id.clone();
        let turn_id = self.turn_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = server
                    .request(
                        "turn/interrupt",
                        build_turn_interrupt_params(&thread_id, &turn_id),
                    )
                    .await;
            });
        }
    }
}

impl CodexAppServer {
    async fn start(binary: &str) -> Result<Arc<Self>, String> {
        let working_dir = create_runtime_working_dir()?;
        let mut command = Command::new(binary);
        command
            .args(["app-server", "--listen", "stdio://"])
            // Keep the runtime cwd outside the Kokoro checkout so its AGENTS.md and project
            // instructions are not accidentally injected into Codex's model context.
            .current_dir(&working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
            let _ = std::fs::remove_dir(&working_dir);
            format!(
                "failed to start Codex app-server using '{}': {}",
                binary, error
            )
        })?;
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let _ = child.start_kill();
                let _ = std::fs::remove_dir(&working_dir);
                return Err("Codex app-server did not expose stdin".to_string());
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = child.start_kill();
                let _ = std::fs::remove_dir(&working_dir);
                return Err("Codex app-server did not expose stdout".to_string());
            }
        };
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(read_stderr(stderr));
        }

        let (notifications, _) = broadcast::channel(NOTIFICATION_BUFFER);
        let server = Arc::new(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            child: Arc::new(StdMutex::new(child)),
            next_request_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            notifications,
            exited: AtomicBool::new(false),
            working_dir,
        });

        let reader_server = server.clone();
        tokio::spawn(async move { reader_server.read_stdout(stdout).await });

        if let Err(error) = server
            .request(
                "initialize",
                build_initialize_params(env!("CARGO_PKG_VERSION")),
            )
            .await
        {
            server.shutdown().await;
            return Err(error);
        }
        if let Err(error) = server
            .notify("initialized", Value::Object(serde_json::Map::new()))
            .await
        {
            server.shutdown().await;
            return Err(error);
        }

        Ok(server)
    }

    fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.notifications.subscribe()
    }

    fn is_exited(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_with_timeout(method, params, RPC_REQUEST_TIMEOUT)
            .await
    }

    async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        if self.is_exited() {
            return Err("Codex app-server process is not running".to_string());
        }

        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        if let Err(error) = self
            .write_value(&build_rpc_request(id, method, params))
            .await
        {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }

        let started_at = Instant::now();
        let result = match tokio::time::timeout(timeout, rx).await {
            Ok(result) => match result {
                Ok(result) => result,
                Err(_) => Err("Codex app-server request channel closed".to_string()),
            },
            Err(_) => {
                self.pending.lock().await.remove(&id);
                let message = if method == "turn/start" {
                    format!(
                        "Codex app-server turn/start acknowledgement timed out after {} seconds",
                        timeout.as_secs()
                    )
                } else {
                    format!(
                        "Codex app-server {method} RPC timed out after {} seconds",
                        timeout.as_secs()
                    )
                };
                tracing::warn!(
                    target: "llm.codex_runtime",
                    method,
                    timeout_kind = if method == "turn/start" { "turn_start_ack" } else { "json_rpc" },
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "Codex app-server request timeout"
                );
                Err(message)
            }
        };
        if result.is_ok() {
            tracing::debug!(
                target: "llm.codex_runtime",
                method,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "Codex app-server request completed"
            );
        } else if !result
            .as_ref()
            .err()
            .is_some_and(|error| error.contains("timed out"))
        {
            tracing::warn!(
                target: "llm.codex_runtime",
                method,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "Codex app-server request failed"
            );
        }
        result
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write_value(&build_rpc_notification(method, params))
            .await
    }

    async fn write_value(&self, value: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().await;
        let mut line = serde_json::to_vec(value)
            .map_err(|error| format!("failed to encode Codex app-server message: {error}"))?;
        line.push(b'\n');
        stdin
            .write_all(&line)
            .await
            .map_err(|error| format!("failed to write to Codex app-server: {error}"))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("failed to flush Codex app-server request: {error}"))
    }

    async fn read_stdout(self: Arc<Self>, stdout: ChildStdout) {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                self.fail_protocol("Codex app-server emitted invalid JSON")
                    .await;
                self.terminate_and_cleanup().await;
                return;
            };
            record_protocol_notification(&value);

            if let Some(id_value) = value.get("id").cloned() {
                if value.get("method").is_some() {
                    self.respond_to_server_request(id_value, value).await;
                } else if let Some(id) = id_value.as_u64() {
                    if let Some(sender) = self.pending.lock().await.remove(&id) {
                        let result = if let Some(error) = value.get("error") {
                            Err(format_rpc_error(error))
                        } else {
                            Ok(value.get("result").cloned().unwrap_or(Value::Null))
                        };
                        let _ = sender.send(result);
                    }
                }
            } else {
                let _ = self.notifications.send(value);
            }
        }

        self.fail_protocol("Codex app-server process exited").await;
        self.terminate_and_cleanup().await;
    }

    async fn respond_to_server_request(self: &Arc<Self>, id: Value, request: Value) {
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if method == "item/tool/call" {
            let _ = self.notifications.send(request.clone());
            let response = json!({
                "id": id,
                "result": {
                    "contentItems": [{
                        "type": "inputText",
                        "text": "Kokoro is handling this tool call in its host tool loop."
                    }],
                    "success": false
                }
            });
            let _ = self.write_value(&response).await;

            if let (Some(thread_id), Some(turn_id)) = (
                request
                    .get("params")
                    .and_then(|params| params.get("threadId"))
                    .and_then(Value::as_str),
                request
                    .get("params")
                    .and_then(|params| params.get("turnId"))
                    .and_then(Value::as_str),
            ) {
                let server = self.clone();
                let thread_id = thread_id.to_string();
                let turn_id = turn_id.to_string();
                tokio::spawn(async move {
                    let _ = server
                        .request(
                            "turn/interrupt",
                            build_turn_interrupt_params(&thread_id, &turn_id),
                        )
                        .await;
                });
            }
            return;
        }

        let lower_method = method.to_ascii_lowercase();
        if lower_method.contains("commandexecution")
            || lower_method.contains("filechange")
            || lower_method.contains("mcptoolcall")
            || lower_method.contains("skill")
            || lower_method.contains("plugin")
            || lower_method.contains("connector")
            || lower_method.contains("search")
            || lower_method.contains("web")
        {
            let _ = self.notifications.send(json!({
                "method": "error",
                "params": {
                    "message": "Codex app-server attempted to use a disabled filesystem, shell, or MCP capability"
                }
            }));
        }
        let response = if method.ends_with("requestApproval") {
            json!({ "id": id, "result": { "decision": "decline" } })
        } else {
            json!({
                "id": id,
                "error": {
                    "code": -32601,
                    "message": "Kokoro does not expose this Codex app-server capability"
                }
            })
        };
        let _ = self.write_value(&response).await;
    }

    async fn shutdown(&self) {
        self.terminate();
    }

    fn terminate(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.start_kill();
        }
    }

    async fn fail_protocol(&self, message: &str) {
        if self.exited.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.notifications.send(json!({
            "method": "error",
            "params": { "message": message }
        }));
        let mut pending = self.pending.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(message.to_string()));
        }
    }

    async fn terminate_and_cleanup(&self) {
        self.terminate();
        for _ in 0..20 {
            let exited = self
                .child
                .lock()
                .ok()
                .and_then(|mut child| child.try_wait().ok())
                .flatten()
                .is_some();
            if exited {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let _ = std::fs::remove_dir(&self.working_dir);
    }
}

fn create_runtime_working_dir() -> Result<RuntimePathBuf, String> {
    let root = std::env::temp_dir();
    for _ in 0..8 {
        let id = NEXT_RUNTIME_CWD_ID.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("kokoro-codex-runtime-{}-{id}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create private Codex runtime working directory: {error}"
                ));
            }
        }
    }
    Err("failed to allocate a unique Codex runtime working directory".to_string())
}

async fn read_stderr(stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        record_transport_signal("stderr", &line);
    }
}

fn record_transport_signal(source: &str, message: &str) {
    let signal = classify_transport_signal(message);
    let bytes = message.len();
    if signal == "other" {
        tracing::debug!(
            target: "llm.codex_runtime",
            source,
            signal,
            bytes,
            "Codex runtime status"
        );
    } else {
        tracing::warn!(
            target: "llm.codex_runtime",
            source,
            signal,
            bytes,
            "Codex runtime transport status"
        );
    }
}

fn record_protocol_notification(value: &Value) {
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let lower_method = method.to_ascii_lowercase();
    let relevant = method == "error"
        || lower_method.contains("transport")
        || lower_method.contains("warning")
        || lower_method.contains("status")
        || lower_method.contains("fallback")
        || lower_method.contains("reconnect")
        || lower_method.contains("websocket")
        || lower_method.contains("https")
        || lower_method.contains("timeout")
        || lower_method.contains("commandexecution")
        || lower_method.contains("filechange")
        || lower_method.contains("mcptoolcall")
        || lower_method.contains("skill")
        || lower_method.contains("plugin")
        || lower_method.contains("connector")
        || lower_method.contains("search")
        || lower_method.contains("web");
    if !relevant {
        return;
    }
    let message = value
        .get("params")
        .and_then(|params| params.get("message"))
        .and_then(Value::as_str)
        .unwrap_or(method);
    record_transport_signal(method, message);
}

fn classify_transport_signal(message: &str) -> &'static str {
    let message = message.to_ascii_lowercase();
    if message.contains("falling back") && message.contains("websocket") {
        "websocket_fallback"
    } else if message.contains("reconnect") {
        "reconnect"
    } else if message.contains("websocket") {
        "websocket"
    } else if message.contains("https") {
        "https"
    } else if message.contains("timeout") || message.contains("timed out") {
        "timeout"
    } else if message.contains("unauthorized")
        || message.contains("authentication")
        || message.contains("login")
    {
        "authentication"
    } else {
        "other"
    }
}

fn format_rpc_error(error: &Value) -> String {
    let code = error.get("code").and_then(Value::as_i64);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Codex app-server returned an error");
    match code {
        Some(code) => format!("Codex app-server error {code}: {message}"),
        None => format!("Codex app-server error: {message}"),
    }
}

pub fn detect_codex_runtime() -> CodexRuntimeInfo {
    let binary = resolve_codex_binary();
    match std::process::Command::new(&binary)
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => CodexRuntimeInfo {
            status: "available".to_string(),
            binary,
            version: bounded_output(&output.stdout),
        },
        Ok(output) => CodexRuntimeInfo {
            status: "error".to_string(),
            binary,
            version: bounded_output(&output.stderr),
        },
        Err(_) => CodexRuntimeInfo {
            status: "not_installed".to_string(),
            binary,
            version: None,
        },
    }
}

fn configured_codex_binary() -> String {
    std::env::var(CODEX_BINARY_ENV)
        .ok()
        .and_then(non_empty_string)
        .unwrap_or_else(|| DEFAULT_CODEX_BINARY.to_string())
}

fn resolve_codex_binary() -> String {
    let candidates = codex_binary_candidates();
    candidates
        .iter()
        .find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .cloned()
        .unwrap_or_else(configured_codex_binary)
}

fn codex_binary_candidates() -> Vec<String> {
    if std::env::var(CODEX_BINARY_ENV)
        .ok()
        .and_then(non_empty_string)
        .is_some()
    {
        return vec![configured_codex_binary()];
    }

    let mut candidates = vec![DEFAULT_CODEX_BINARY.to_string()];

    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let install_root = PathBuf::from(local_app_data)
                .join("OpenAI")
                .join("Codex")
                .join("bin");
            if let Ok(entries) = std::fs::read_dir(install_root) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("codex.exe");
                    if candidate.is_file() {
                        candidates.push(candidate.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }

    candidates
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn bounded_output(bytes: &[u8]) -> Option<String> {
    let end = bytes.len().min(MAX_VERSION_OUTPUT_BYTES);
    let value = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_installed_codex_cli_without_reading_credentials() {
        let info = detect_codex_runtime();

        assert!(matches!(
            info.status.as_str(),
            "available" | "error" | "not_installed"
        ));
        assert!(
            info.binary == DEFAULT_CODEX_BINARY || std::path::Path::new(&info.binary).is_absolute(),
            "unexpected resolved Codex binary: {}",
            info.binary
        );
        if info.status == "available" {
            assert!(info.version.is_some());
        }
    }

    #[test]
    fn provider_exposes_native_tool_capability_for_kokoro_dynamic_tools() {
        let provider = CodexRuntimeProvider::new("codex-runtime".to_string(), None);

        assert_eq!(provider.id(), "codex-runtime");
        assert!(provider.supports_native_tools());
    }
}
