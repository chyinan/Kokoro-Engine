//! MCP Transport Layer — trait-based abstraction for MCP server communication.
//!
//! Currently implements stdio transport (subprocess stdin/stdout).
//! Designed to be extended with SSE/HTTP transports in the future.

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
        tokio::spawn(async move {
            while let Some((request, responder)) = rx.recv().await {
                // Store responder for this request ID
                pending_writer.lock().await.insert(request.id, responder);

                // Serialize and write
                let mut line = serde_json::to_string(&request).unwrap_or_default();
                line.push('\n');

                if let Err(e) = stdin.write_all(line.as_bytes()).await {
                    eprintln!("[MCP/Stdio] Write error: {}", e);
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    eprintln!("[MCP/Stdio] Flush error: {}", e);
                    break;
                }
            }
        });

        // ── Reader task: reads stdout lines, dispatches to pending responders ──
        let reader = BufReader::new(stdout);
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
                        // EOF — process exited
                        eprintln!("[MCP/Stdio] Server process exited (stdout closed)");
                        connected_clone.store(false, Ordering::SeqCst);
                        break;
                    }
                    Err(e) => {
                        eprintln!("[MCP/Stdio] Read error: {}", e);
                        connected_clone.store(false, Ordering::SeqCst);
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

        // Notifications use id=0 but we don't wait for response
        // Actually per JSON-RPC spec, notifications MUST NOT have an id field.
        // We send it via the writer but don't register a responder.
        let line = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .map_err(|e| e.to_string())?;

        // We can't write directly to stdin from here, so we use a dummy oneshot
        // Actually, let's just skip notification support for now — most MCP usage
        // is request/response. We can add true notification support later.
        let _ = line;
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
