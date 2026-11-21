use super::{
    bridge, client::McpRequestError, manager::build_connected_client, McpManager, McpServerConfig,
};
use crate::actions::ActionRegistry;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as BlockingMutex},
    time::Duration,
};
use tokio::{
    sync::{mpsc, Mutex, RwLock},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use warp::{
    http::{Response, StatusCode},
    hyper::Body,
    Filter,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(8);

fn config(name: &str, mode: &str, url: Option<String>) -> McpServerConfig {
    McpServerConfig {
        name: name.into(),
        transport_type: mode.into(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        url,
        enabled: true,
    }
}

#[tokio::test]
async fn stdio_launches_drains_stderr_pages_tools_and_refreshes_actions() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp-stdio.mjs");
    let dir = tempfile::tempdir().unwrap();
    let mut config = config("stdio-test", "stdio", None);
    let executable = format!(
        "kokoro-mcp-shim-{}",
        dir.path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .trim_start_matches('.')
    );
    #[cfg(windows)]
    {
        // Exercise actual PATHEXT lookup using only the configured child PATH.
        let shim = dir.path().join(format!("{executable}.cmd"));
        std::fs::write(&shim, "@echo off\r\nnode.exe %*\r\n").unwrap();
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let shim = dir.path().join(&executable);
        std::fs::write(&shim, "#!/bin/sh\nexec node \"$@\"\n").unwrap();
        std::fs::set_permissions(shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    config.command = executable;
    config.args = vec![path.to_string_lossy().into_owned()];
    let pid_file = dir.path().join("connected.pid");
    config.env.insert(
        "KOKORO_MCP_TEST_PID_FILE".into(),
        pid_file.to_string_lossy().into_owned(),
    );
    let parent_path = std::env::var_os("PATH").unwrap_or_default();
    let child_path = std::env::join_paths(
        std::iter::once(dir.path().to_path_buf()).chain(std::env::split_paths(&parent_path)),
    )
    .unwrap();
    let key = if cfg!(windows) { "Path" } else { "PATH" };
    config
        .env
        .insert(key.into(), child_path.to_string_lossy().into_owned());
    config.env.insert(
        "KOKORO_MCP_TEST_VALUE".into(),
        "configured-stdio-environment".into(),
    );
    let manager = Arc::new(Mutex::new(McpManager::new(
        dir.path().join("mcp.json").to_str().unwrap(),
    )));
    let registry = Arc::new(RwLock::new(ActionRegistry::new()));
    bridge::add_server(&manager, &registry, config)
        .await
        .unwrap();
    wait_for_action(&registry, "second").await;
    let status = manager.lock().await.list_status().pop().unwrap();
    assert!(status.connected);
    assert_eq!(status.server_version.as_deref(), Some("1"));
    assert_eq!(status.tool_count, 2);
    let (client, revision) = {
        let manager = manager.lock().await;
        let (_, client) = manager.connected_clients().next().unwrap();
        (client.clone(), client.tools().unwrap().0)
    };
    let result = client
        .call_tool(revision, "first", Default::default())
        .await
        .unwrap();
    assert_eq!(
        result.content,
        vec![json!({"type":"text","text":"configured-stdio-environment"})]
    );
    wait_for_action(&registry, "replacement").await;
    assert!(!registry
        .read()
        .await
        .list_actions()
        .iter()
        .any(|action| action.name == "first" || action.name == "second"));
    assert!(matches!(
        client
            .call_tool(revision, "first", Default::default())
            .await,
        Err(McpRequestError::ToolUnavailable)
    ));
    bridge::remove_server(&manager, &registry, "stdio-test")
        .await
        .unwrap();
    assert!(!client.is_connected());
    assert!(registry.read().await.list_actions().is_empty());
    assert!(fixture_process_has_exited(&pid_file).await);
}

async fn fixture_process_has_exited(pid_file: &std::path::Path) -> bool {
    let pid = std::fs::read_to_string(pid_file).unwrap();
    tokio::process::Command::new("node")
        .args(["-e", "try { process.kill(Number(process.argv[1]), 0); process.exitCode = 1 } catch (e) { process.exitCode = e.code === 'ESRCH' ? 0 : 2 }", &pid])
        .status()
        .await
        .unwrap()
        .success()
}

#[tokio::test]
async fn shutdown_during_stdio_negotiation_reaps_the_child() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp-stdio.mjs");
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("starting.pid");
    let mut config = config("starting child", "stdio", None);
    config.command = "node".into();
    config.args = vec![fixture.to_string_lossy().into_owned()];
    config.env.insert(
        "KOKORO_MCP_TEST_PID_FILE".into(),
        pid_file.to_string_lossy().into_owned(),
    );
    config
        .env
        .insert("KOKORO_MCP_TEST_HOLD_STARTUP".into(), "true".into());
    let manager = Arc::new(Mutex::new(McpManager::new(
        dir.path().join("mcp.json").to_str().unwrap(),
    )));
    let registry = Arc::new(RwLock::new(ActionRegistry::new()));
    bridge::add_server(&manager, &registry, config)
        .await
        .unwrap();
    tokio::time::timeout(TEST_TIMEOUT, async {
        while !pid_file.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(manager.lock().await.list_status()[0].status, "connecting");
    tokio::time::timeout(TEST_TIMEOUT, bridge::shutdown(&manager, &registry))
        .await
        .unwrap()
        .unwrap();
    assert!(fixture_process_has_exited(&pid_file).await);
    assert!(registry.read().await.list_actions().is_empty());
}

async fn wait_for_action(registry: &RwLock<ActionRegistry>, name: &str) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            if registry
                .read()
                .await
                .list_actions()
                .iter()
                .any(|action| action.name == name)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("MCP action publishing timed out");
}

fn tool_definition(name: &str) -> Value {
    json!({"name":name,"description":"test tool","inputSchema":{"type":"object"}})
}

fn rpc_reply(request: &Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":request["id"],"result":result})
}

fn initialized(request: &Value, version: &str, capabilities: Value) -> Value {
    rpc_reply(
        request,
        json!({"protocolVersion":version,"capabilities":capabilities,"serverInfo":{"name":"test-http","version":"2"}}),
    )
}

fn json_response(body: Value) -> Response<Body> {
    Response::builder()
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn empty_response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap()
}

struct Server {
    url: String,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn serve<F>(filter: F) -> Server
where
    F: Filter<Extract = (Response<Body>,), Error = warp::Rejection> + Clone + Send + Sync + 'static,
{
    let (addr, serve) = warp::serve(filter).bind_ephemeral(([127, 0, 0, 1], 0));
    Server {
        url: format!("http://{addr}"),
        task: tokio::spawn(serve),
    }
}

#[tokio::test]
async fn streamable_http_distinguishes_results_and_never_replays_expired_calls() {
    let requests = Arc::new(BlockingMutex::new(Vec::new()));
    let received = requests.clone();
    let post = warp::post().and(warp::header::headers_cloned()).and(warp::body::json::<Value>())
        .map(move |headers: warp::http::HeaderMap, request: Value| {
            let method = request["method"].as_str().unwrap();
            received.lock().unwrap().push((request.clone(), headers));
            match method {
                "server/discover" => json_response(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"legacy"}})),
                "initialize" => {
                    let mut response = json_response(initialized(&request, "2025-11-25", json!({"tools":{}})));
                    response.headers_mut().insert("mcp-session-id", warp::http::HeaderValue::from_static("test-session"));
                    response
                }
                "tools/list" => json_response(rpc_reply(&request, json!({"tools":[tool_definition("rich"), tool_definition("mixed"), tool_definition("protocol_error"), tool_definition("redirect"), tool_definition("expired")]}))),
                "tools/call" => match request["params"]["name"].as_str().unwrap() {
                    "rich" => json_response(rpc_reply(&request, json!({
                        "isError":true,
                        "content":[
                            {"type":"text","text":"problem"},
                            {"type":"audio","data":"YWJj","mimeType":"audio/wav"},
                            {"type":"future_kind","details":{"retained":true}},
                            {"type":"resource_link","uri":"file:///not-opened","name":"resource"}
                        ],
                        "structuredContent":{"details":[1,2]},
                    }))),
                    "mixed" => json_response(rpc_reply(&request, json!({
                        "content":[
                            {"type":"text","text":"mixed result"},
                            {"type":"image","data":"YWJj","mimeType":"image/png"},
                            {"type":"resource","resource":{"uri":"file:///not-opened","mimeType":"text/plain","text":"embedded"}}
                        ],
                        "structuredContent":null
                    }))),
                    "protocol_error" => json_response(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32602,"message":"invalid parameters"}})),
                    "redirect" => Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header("location", "/different-endpoint")
                        .body(Body::empty())
                        .unwrap(),
                    _ => empty_response(StatusCode::NOT_FOUND),
                },
                _ => empty_response(StatusCode::ACCEPTED),
            }
        });
    let other = warp::get()
        .or(warp::delete())
        .unify()
        .map(|| empty_response(StatusCode::METHOD_NOT_ALLOWED));
    let server = serve(post.or(other).unify());
    let mut config = config(
        "stream-http",
        "streamable_http",
        Some(format!("{}/mcp", server.url)),
    );
    config
        .env
        .insert("Authorization".into(), "mcp-test-stdio-only".into());
    let client = build_connected_client(&config, CancellationToken::new())
        .await
        .unwrap();
    let revision = client.tools().unwrap().0;
    let rich = client
        .call_tool(revision, "rich", Default::default())
        .await
        .unwrap();
    assert!(rich.is_error);
    assert_eq!(rich.structured_content, Some(json!({"details":[1,2]})));
    assert_eq!(
        rich.content
            .iter()
            .map(|v| v["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["text", "audio", "future_kind", "resource_link"]
    );
    let action = bridge::action_result(rich);
    assert!(!action.success);
    assert!(action.message.contains("Unrecognized MCP content"));
    assert_eq!(
        action.data.unwrap()["content"][2]["details"]["retained"],
        true
    );
    let mixed = client
        .call_tool(revision, "mixed", Default::default())
        .await
        .unwrap();
    let mixed = bridge::action_result(mixed);
    assert!(mixed.success);
    assert!(mixed
        .message
        .starts_with("mixed result\n[MCP image content attached]"));
    let data = mixed.data.unwrap();
    assert_eq!(
        data["content"][1],
        json!({"type":"image","data":"YWJj","mimeType":"image/png"})
    );
    assert_eq!(data["content"][2]["resource"]["text"], "embedded");
    assert_eq!(data.get("structuredContent"), Some(&Value::Null));
    assert!(matches!(
        client
            .call_tool(revision, "redirect", Default::default())
            .await,
        Err(McpRequestError::Transport)
    ));
    assert!(matches!(
        client
            .call_tool(revision, "protocol_error", Default::default())
            .await,
        Err(McpRequestError::Protocol { code: -32602, .. })
    ));
    assert!(matches!(
        client
            .call_tool(revision, "expired", Default::default())
            .await,
        Err(McpRequestError::Disconnected)
    ));
    assert!(!client.is_connected());
    assert!(matches!(
        client
            .call_tool(revision, "mixed", Default::default())
            .await,
        Err(McpRequestError::Disconnected)
    ));
    client.shutdown().await.unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests
        .iter()
        .all(|(_, headers)| !headers.contains_key("authorization")));
    let initializations = requests
        .iter()
        .filter(|(req, _)| req["method"] == "initialize")
        .count();
    assert_eq!(initializations, 1);
    assert_eq!(
        requests
            .iter()
            .filter(|(req, _)| req["params"]["name"] == "redirect")
            .count(),
        1
    );
    let expired = requests
        .iter()
        .filter(|(req, _)| req["params"]["name"] == "expired")
        .collect::<Vec<_>>();
    assert_eq!(expired.len(), 1);
    assert_eq!(
        requests
            .iter()
            .filter(|(req, _)| req["params"]["name"] == "mixed")
            .count(),
        1,
        "retiring the expired session must prevent further calls on that peer"
    );
    let headers = &expired[0].1;
    assert_eq!(headers["mcp-session-id"], "test-session");
    assert_eq!(headers["mcp-protocol-version"], "2025-11-25");
}

fn sse_response(first: Option<&str>, receiver: mpsc::UnboundedReceiver<String>) -> Response<Body> {
    let subsequent = futures::stream::unfold(receiver, |mut receiver| async {
        receiver
            .recv()
            .await
            .map(|event| (Ok::<_, std::io::Error>(event), receiver))
    });
    let body = Body::wrap_stream(futures::StreamExt::chain(
        futures::stream::iter(first.map(|event| Ok(event.to_string()))),
        subsequent,
    ));
    Response::builder()
        .header("content-type", "text/event-stream; charset=utf-8")
        .body(body)
        .unwrap()
}

#[tokio::test]
async fn legacy_sse_uses_endpoint_and_cancels_an_abandoned_call() {
    let stream = Arc::new(BlockingMutex::new(None::<mpsc::UnboundedSender<String>>));
    let controls = stream.clone();
    let (sent, mut received) = mpsc::unbounded_channel::<Value>();
    let for_get = stream.clone();
    let get = warp::get().and(warp::path!("nested" / "sse")).map(move || {
        let (sender, receiver) = mpsc::unbounded_channel();
        *for_get.lock().unwrap() = Some(sender);
        sse_response(
            Some("event: endpoint\r\ndata: ./messages?session=legacy\r\n\r\n"),
            receiver,
        )
    });
    let post = warp::post()
        .and(warp::path!("nested" / "messages"))
        .and(warp::body::json::<Value>())
        .map(move |request: Value| {
            let _ = sent.send(request.clone());
            let output = match request["method"].as_str().unwrap() {
                "initialize" => Some(initialized(&request, "2024-11-05", json!({"tools":{}}))),
                "tools/list" => Some(rpc_reply(
                    &request,
                    json!({"tools":[tool_definition("wait")]}),
                )),
                _ => None,
            };
            if let Some(output) = output {
                let _ = stream
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .send(format!("event: message\ndata: {output}\n\n"));
            }
            empty_response(StatusCode::ACCEPTED)
        });
    let redirect = warp::get().and(warp::path!("sse")).map(|| {
        Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header("location", "/nested/sse")
            .body(Body::empty())
            .unwrap()
    });
    let server = serve(get.or(post).unify().or(redirect).unify());
    let config = config("legacy", "sse", Some(format!("{}/sse", server.url)));
    let client = Arc::new(
        build_connected_client(&config, CancellationToken::new())
            .await
            .unwrap(),
    );
    let revision = client.tools().unwrap().0;
    let init = received.recv().await.unwrap();
    assert_eq!(init["method"], "initialize");
    assert_eq!(init["params"]["protocolVersion"], "2024-11-05");
    let caller = client.clone();
    let call =
        tokio::spawn(async move { caller.call_tool(revision, "wait", Default::default()).await });
    let tool_request = get_rpc(&mut received, "tools/call").await;
    call.abort();
    let cancel = get_rpc(&mut received, "notifications/cancelled").await;
    assert_eq!(cancel["params"]["requestId"], tool_request["id"]);

    let admitted = client.clone();
    let reply = tokio::spawn(async move {
        admitted
            .call_tool(revision, "wait", Default::default())
            .await
    });
    let request = get_rpc(&mut received, "tools/call").await;
    let closing = client.clone();
    let stop = tokio::spawn(async move { closing.shutdown().await });
    tokio::time::timeout(TEST_TIMEOUT, async {
        while client.is_connected() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!stop.is_finished());
    assert!(matches!(
        client.call_tool(revision, "wait", Default::default()).await,
        Err(McpRequestError::Stopping)
    ));
    let result = rpc_reply(
        &request,
        json!({"content":[{"type":"text","text":"final output"}]}),
    );
    let sse = controls.lock().unwrap().as_ref().unwrap().clone();
    sse.send(format!("event: message\ndata: {result}\n\n"))
        .unwrap();
    assert_eq!(
        reply.await.unwrap().unwrap().content,
        [json!({"type":"text","text":"final output"})]
    );
    stop.await.unwrap().unwrap();
    tokio::time::timeout(TEST_TIMEOUT, sse.closed())
        .await
        .unwrap();
}

async fn get_rpc(receiver: &mut mpsc::UnboundedReceiver<Value>, method: &str) -> Value {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let message = receiver.recv().await.expect("fixture stopped");
            if message["method"] == method {
                return message;
            }
        }
    })
    .await
    .expect("expected MCP message missing")
}

#[tokio::test]
async fn disabling_and_removing_a_pending_legacy_connection_closes_its_stream() {
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let server = serve(warp::get().map(move || {
        let (sender, receiver) = mpsc::unbounded_channel();
        observed_tx.send(sender).unwrap();
        sse_response(Some(": connecting\n\n"), receiver)
    }));
    let config = config("held", "sse", Some(format!("{}/sse", server.url)));
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(Mutex::new(McpManager::new(
        dir.path().join("mcp.json").to_str().unwrap(),
    )));
    let registry = Arc::new(RwLock::new(ActionRegistry::new()));
    bridge::add_server(&manager, &registry, config.clone())
        .await
        .unwrap();
    let first = tokio::time::timeout(TEST_TIMEOUT, observed_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(manager.lock().await.list_status()[0].status, "connecting");
    bridge::toggle_server(&manager, &registry, "held", false)
        .await
        .unwrap();
    tokio::time::timeout(TEST_TIMEOUT, first.closed())
        .await
        .unwrap();
    assert_eq!(manager.lock().await.list_status()[0].status, "disconnected");
    bridge::add_server(&manager, &registry, config)
        .await
        .unwrap();
    let second = tokio::time::timeout(TEST_TIMEOUT, observed_rx.recv())
        .await
        .unwrap()
        .unwrap();
    bridge::remove_server(&manager, &registry, "held")
        .await
        .unwrap();
    tokio::time::timeout(TEST_TIMEOUT, second.closed())
        .await
        .unwrap();
    assert!(manager.lock().await.list_status().is_empty());
    assert!(registry.read().await.list_actions().is_empty());
}

#[tokio::test]
async fn modern_http_negotiates_stateless_tool_notifications() {
    let (subscribed, mut subscriptions) = mpsc::unbounded_channel();
    let current = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let listed = current.clone();
    let server = serve(warp::post().and(warp::header::headers_cloned()).and(warp::body::json::<Value>())
        .map(move |headers: warp::http::HeaderMap, request: Value| {
            assert!(!headers.contains_key("mcp-session-id"));
            assert_eq!(headers["mcp-protocol-version"], "2026-07-28");
            match request["method"].as_str().unwrap() {
                "server/discover" => json_response(rpc_reply(&request, json!({
                    "resultType":"complete","supportedVersions":["2026-07-28"],
                    "capabilities":{"tools":{"listChanged":true}},"ttlMs":0,"cacheScope":"private",
                }))),
                "subscriptions/listen" => {
                    assert_eq!(request["params"]["notifications"]["toolsListChanged"], true);
                    let (tx, rx) = mpsc::unbounded_channel();
                    let meta = json!({"io.modelcontextprotocol/subscriptionId":request["id"]});
                    let ack = json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged","params":{"notifications":{"toolsListChanged":true},"_meta":meta}});
                    let changed = json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed","params":{"_meta":meta}});
                    subscribed.send((tx, format!("event: message\ndata: {changed}\n\n"))).unwrap();
                    sse_response(Some(&format!("event: message\ndata: {ack}\n\n")), rx)
                }
                "tools/list" => {
                    let name = if listed.load(std::sync::atomic::Ordering::Acquire) { "updated" } else { "initial" };
                    json_response(rpc_reply(&request, json!({"tools":[tool_definition(name)],"resultType":"complete"})))
                }
                _ => empty_response(StatusCode::ACCEPTED),
            }
        }));
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(Mutex::new(McpManager::new(
        dir.path().join("mcp.json").to_str().unwrap(),
    )));
    let registry = Arc::new(RwLock::new(ActionRegistry::new()));
    let config = config(
        "modern",
        "streamable-http",
        Some(format!("{}/mcp", server.url)),
    );
    bridge::add_server(&manager, &registry, config)
        .await
        .unwrap();
    wait_for_action(&registry, "initial").await;
    let (sender, event) = tokio::time::timeout(TEST_TIMEOUT, subscriptions.recv())
        .await
        .unwrap()
        .unwrap();
    current.store(true, std::sync::atomic::Ordering::Release);
    sender.send(event).unwrap();
    wait_for_action(&registry, "updated").await;
    assert!(!registry
        .read()
        .await
        .list_actions()
        .iter()
        .any(|tool| tool.name == "initial"));
    bridge::shutdown(&manager, &registry).await.unwrap();
}

#[tokio::test]
async fn an_unsupported_version_fails_but_a_peer_without_tools_can_connect() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let supported = Arc::new(AtomicBool::new(false));
    let choose_version = supported.clone();
    let tool_lists = Arc::new(AtomicUsize::new(0));
    let listed = tool_lists.clone();
    let server = serve(warp::post().and(warp::body::json::<Value>()).map(move |request: Value| {
        match request["method"].as_str().unwrap() {
            "server/discover" => json_response(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"legacy"}})),
            "initialize" => {
                let version = if choose_version.load(Ordering::Acquire) { "2025-11-25" } else { "2099-01-01" };
                json_response(initialized(&request, version, json!({})))
            }
            "tools/list" => {
                listed.fetch_add(1, Ordering::Relaxed);
                json_response(rpc_reply(&request, json!({"tools":[]})))
            }
            _ => empty_response(StatusCode::ACCEPTED),
        }
    }));
    let config = config(
        "version",
        "streamable_http",
        Some(format!("{}/mcp", server.url)),
    );
    let error = build_connected_client(&config, CancellationToken::new())
        .await
        .err()
        .expect("unsupported version should fail");
    assert!(
        error.to_string().contains("unsupported protocol version"),
        "{error}"
    );
    supported.store(true, Ordering::Release);
    let client = build_connected_client(&config, CancellationToken::new())
        .await
        .unwrap();
    assert!(client.is_connected());
    assert_eq!(client.tool_count(), 0);
    assert_eq!(tool_lists.load(Ordering::Relaxed), 0);
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_completed_old_generation_cannot_replace_live_actions() {
    let server = serve(
        warp::post()
            .and(warp::path::param::<String>())
            .and(warp::body::json::<Value>())
            .map(
                |tool: String, request: Value| match request["method"].as_str().unwrap() {
                    "server/discover" => json_response(rpc_reply(
                        &request,
                        json!({
                            "resultType":"complete", "supportedVersions":["2026-07-28"],
                            "capabilities":{"tools":{}}, "ttlMs":0,"cacheScope":"private",
                        }),
                    )),
                    "tools/list" => json_response(rpc_reply(
                        &request,
                        json!({"tools":[tool_definition(&tool)]}),
                    )),
                    _ => empty_response(StatusCode::ACCEPTED),
                },
            ),
    );
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(Mutex::new(McpManager::new(
        dir.path().join("mcp.json").to_str().unwrap(),
    )));
    let registry = Arc::new(RwLock::new(ActionRegistry::new()));
    let old_config = config(
        "named",
        "streamable_http",
        Some(format!("{}/old-tool", server.url)),
    );
    let old_attempt = {
        let mut manager = manager.lock().await;
        manager.upsert_server_config(old_config).unwrap();
        manager.prepare_connection("named").unwrap()
    };
    // Complete the old handshake, then hold its result as if it were waiting
    // for the manager lock. The new generation is committed before the old one.
    let late = Arc::new(old_attempt.connect().await.unwrap());
    assert_eq!(late.tools().unwrap().1[0].name, "old-tool");
    let new_config = config(
        "named",
        "streamable_http",
        Some(format!("{}/new-tool", server.url)),
    );
    bridge::add_server(&manager, &registry, new_config)
        .await
        .unwrap();
    wait_for_action(&registry, "new-tool").await;
    {
        let mut manager = manager.lock().await;
        assert!(!manager.commit_client(&old_attempt, late.clone()));
        assert_eq!(manager.list_status()[0].tool_count, 1);
        let (_, live) = manager.connected_clients().next().unwrap();
        assert_eq!(live.tools().unwrap().1[0].name, "new-tool");
    }
    assert!(!late.is_connected());
    assert_eq!(registry.read().await.list_actions()[0].name, "new-tool");
    let generation = registry.read().await.generation();
    bridge::publish_catalog(&manager, &registry).await;
    bridge::publish_catalog(&manager, &registry).await;
    assert_eq!(registry.read().await.generation(), generation);
    late.shutdown().await.unwrap();
    bridge::shutdown(&manager, &registry).await.unwrap();
}

#[tokio::test]
async fn request_timeout_cancels_the_original_id_without_replay() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (client_wire, server_wire) = tokio::io::duplex(16 * 1024);
    let (input, output) = tokio::io::split(client_wire);
    let transport = rmcp::transport::async_rw::AsyncRwTransport::new_client(input, output);
    let (input, mut output) = tokio::io::split(server_wire);
    let (sent, mut received) = mpsc::unbounded_channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let server = tokio::spawn(async move {
        let mut lines = BufReader::new(input).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            sent.send(request.clone()).unwrap();
            let reply = match request["method"].as_str().unwrap() {
                "initialize" => Some(initialized(&request, "2024-11-05", json!({"tools":{}}))),
                "tools/list" => Some(rpc_reply(
                    &request,
                    json!({"tools":[tool_definition("wait")]}),
                )),
                "tools/call" => {
                    counted.fetch_add(1, Ordering::Relaxed);
                    None
                }
                _ => None,
            };
            if let Some(reply) = reply {
                output
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .unwrap();
            }
        }
    });
    let client = Arc::new(
        super::client::McpClient::start(transport, true, CancellationToken::new())
            .await
            .unwrap(),
    );
    let revision = client.tools().unwrap().0;
    let caller = client.clone();
    let call =
        tokio::spawn(async move { caller.call_tool(revision, "wait", Default::default()).await });
    let original = get_rpc(&mut received, "tools/call").await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
    assert!(matches!(call.await.unwrap(), Err(McpRequestError::Timeout)));
    let cancel = get_rpc(&mut received, "notifications/cancelled").await;
    assert_eq!(cancel["params"]["requestId"], original["id"]);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    client.shutdown().await.unwrap();
    tokio::time::timeout(TEST_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap();
}
