//! MCP Transport Layer — trait-based abstraction for MCP server communication.
//!
//! Implements stdio transport (subprocess stdin/stdout) and
//! Streamable HTTP transport (POST JSON-RPC to an HTTP endpoint).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

// ── JSON-RPC 2.0 Types ─────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC Error {}: {}", self.code, self.message)
    }
}

// ── Transport Trait ─────────────────────────────────────

/// Abstract transport for MCP communication.
/// Implementations handle the wire protocol (stdio, SSE, HTTP, etc.)
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a JSON-RPC request and wait for the response.
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, String>;

    /// Send a notification (no response expected).
    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), String>;

    /// Check if the transport is currently connected.
    fn is_connected(&self) -> bool;

    /// Gracefully shut down the transport.
    async fn shutdown(&self) -> Result<(), String>;
}

// ── Stdio Transport ─────────────────────────────────────

/// Spawns MCP server as subprocess, communicates via stdin/stdout JSON-RPC.
pub struct StdioTransport {
    sender: mpsc::Sender<(JsonRpcRequest, oneshot::Sender<Result<Value, String>>)>,
    next_id: AtomicU64,
    connected: Arc<std::sync::atomic::AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl StdioTransport {
    /// Spawn an MCP server process and set up communication channels.
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: Option<&HashMap<String, String>>,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Merge extra env vars
        if let Some(env_vars) = env {
            for (k, v) in env_vars {
                cmd.env(k, v);
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn MCP server '{}': {}", command, e))?;

        let stdin = child.stdin.take().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;

        let connected = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let connected_clone = connected.clone();

        // Channel for sending requests from any thread
        let (tx, mut rx) =
            mpsc::channel::<(JsonRpcRequest, oneshot::Sender<Result<Value, String>>)>(64);

        // Pending response map — shared between writer and reader
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_writer = pending.clone();
        let pending_reader = pending.clone();

        // ── Writer task: receives requests from channel, writes to stdin ──
        let mut stdin = stdin;
        let pending_cleanup = pending.clone();
        tokio::spawn(async move {
            while let Some((request, responder)) = rx.recv().await {
                // Store responder for this request ID
                pending_writer.lock().await.insert(request.id, responder);

                // Serialize and write
                let mut line = serde_json::to_string(&request).unwrap_or_default();
                line.push('\n');

                if let Err(e) = stdin.write_all(line.as_bytes()).await {
                    eprintln!("[MCP/Stdio] Write error: {}", e);
                    // 清理所有 pending 请求，通知等待者连接已断开
                    let mut pending = pending_cleanup.lock().await;
                    for (_, responder) in pending.drain() {
                        let _ = responder.send(Err("Transport write error: connection lost".to_string()));
                    }
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    eprintln!("[MCP/Stdio] Flush error: {}", e);
                    let mut pending = pending_cleanup.lock().await;
                    for (_, responder) in pending.drain() {
                        let _ = responder.send(Err("Transport flush error: connection lost".to_string()));
                    }
                    break;
                }
            }
        });

        // ── Reader task: reads stdout lines, dispatches to pending responders ──
        let reader = BufReader::new(stdout);
        let pending_reader_cleanup = pending.clone();
        tokio::spawn(async move {
            let mut lines = reader.lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let line = line.trim().to_string();
                        if line.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<JsonRpcResponse>(&line) {
                            Ok(response) => {
                                if let Some(id) = response.id {
                                    // Match response to pending request
                                    if let Some(responder) = pending_reader.lock().await.remove(&id)
                                    {
                                        let result = if let Some(error) = response.error {
                                            Err(error.to_string())
                                        } else {
                                            Ok(response.result.unwrap_or(Value::Null))
                                        };
                                        let _ = responder.send(result);
                                    }
                                }
                                // Notifications (id=null) are silently ignored for now
                            }
                            Err(e) => {
                                eprintln!(
                                    "[MCP/Stdio] Parse error: {} — line: {}",
                                    e,
                                    &line[..line.len().min(200)]
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        // EOF — process exited, 清理所有 pending 请求
                        eprintln!("[MCP/Stdio] Server process exited (stdout closed)");
                        connected_clone.store(false, Ordering::SeqCst);
                        let mut pending = pending_reader_cleanup.lock().await;
                        for (_, responder) in pending.drain() {
                            let _ = responder.send(Err("MCP server process exited".to_string()));
                        }
                        break;
                    }
                    Err(e) => {
                        eprintln!("[MCP/Stdio] Read error: {}", e);
                        connected_clone.store(false, Ordering::SeqCst);
                        let mut pending = pending_reader_cleanup.lock().await;
                        for (_, responder) in pending.drain() {
                            let _ = responder.send(Err(format!("MCP transport read error: {}", e)));
                        }
                        break;
                    }
                }
            }
        });

        Ok(Self {
            sender: tx,
            next_id: AtomicU64::new(1),
            connected,
            child: Arc::new(Mutex::new(Some(child))),
        })
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        if !self.is_connected() {
            return Err("Transport disconnected".to_string());
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let (tx, rx) = oneshot::channel();
        self.sender
            .send((request, tx))
            .await
            .map_err(|_| "Transport channel closed".to_string())?;

        // Timeout after 30 seconds
        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| format!("MCP request '{}' timed out", method))?
            .map_err(|_| "Response channel dropped".to_string())?
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), String> {
        if !self.is_connected() {
            return Err("Transport disconnected".to_string());
        }

        // JSON-RPC 通知没有 id 字段，也不需要等待响应
        // 通过 sender channel 发送，使用一个不会被读取的 responder
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let (tx, _rx) = oneshot::channel();
        self.sender
            .send((request, tx))
            .await
            .map_err(|_| "Transport channel closed".to_string())?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn shutdown(&self) -> Result<(), String> {
        self.connected.store(false, Ordering::SeqCst);

        // Try to kill the child process
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }

        Ok(())
    }
}

// ── Streamable HTTP Transport ──────────────────────────

/// Communicates with an MCP server over HTTP POST (Streamable HTTP transport).
/// Each JSON-RPC request is sent as a POST to the endpoint URL.
pub struct StreamableHttpTransport {
    url: String,
    client: reqwest::Client,
    next_id: AtomicU64,
    connected: Arc<std::sync::atomic::AtomicBool>,
    /// MCP session ID returned by the server via `Mcp-Session-Id` header.
    session_id: Arc<Mutex<Option<String>>>,
}

impl StreamableHttpTransport {
    /// Create a new HTTP transport pointing at the given URL.
    pub fn new(url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self {
            url: url.trim_end_matches('/').to_string(),
            client,
            next_id: AtomicU64::new(1),
            connected: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            session_id: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl McpTransport for StreamableHttpTransport {
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        if !self.is_connected() {
            return Err("Transport disconnected".to_string());
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        // Only include "params" when provided — some servers reject null params
        if let Some(p) = params {
            body["params"] = p;
        }

        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        // Attach session ID if we have one
        if let Some(ref sid) = *self.session_id.lock().await {
            req = req.header("Mcp-Session-Id", sid.clone());
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        // Capture session ID from response header
        if let Some(sid) = resp.headers().get("mcp-session-id") {
            if let Ok(s) = sid.to_str() {
                *self.session_id.lock().await = Some(s.to_string());
            }
        }

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            self.connected.store(false, Ordering::SeqCst);
            return Err(format!("HTTP {}: {}", status, text));
        }

        // Check content type — may be JSON or SSE
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // SSE: read events until we find the JSON-RPC response
            let text = resp.text().await.map_err(|e| format!("SSE read error: {}", e))?;
            parse_sse_response(&text, id)
        } else {
            // Plain JSON response
            let json_resp: JsonRpcResponse = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse JSON response: {}", e))?;

            if let Some(error) = json_resp.error {
                Err(error.to_string())
            } else {
                Ok(json_resp.result.unwrap_or(Value::Null))
            }
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), String> {
        if !self.is_connected() {
            return Err("Transport disconnected".to_string());
        }

        // JSON-RPC notification: no `id` field
        let mut body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if let Some(p) = params {
            body["params"] = p;
        }

        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        if let Some(ref sid) = *self.session_id.lock().await {
            req = req.header("Mcp-Session-Id", sid.clone());
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP notify failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP notify error {}: {}", status, text));
        }

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn shutdown(&self) -> Result<(), String> {
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// Parse a JSON-RPC response from an SSE text stream.
/// Looks for `data:` lines containing JSON with a matching `id`.
fn parse_sse_response(sse_text: &str, expected_id: u64) -> Result<Value, String> {
    for line in sse_text.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(data) {
                if resp.id == Some(expected_id) {
                    if let Some(error) = resp.error {
                        return Err(error.to_string());
                    }
                    return Ok(resp.result.unwrap_or(Value::Null));
                }
            }
        }
    }
    Err("No matching JSON-RPC response found in SSE stream".to_string())
}
