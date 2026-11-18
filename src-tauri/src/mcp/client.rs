//! Kokoro policy and a tool catalog over the official MCP client's peer.

use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, CancelledNotification, CancelledNotificationParam,
        ClientCapabilities, ClientConfig, ClientRequest, Implementation, JsonObject,
        ListToolsRequest, PaginatedRequestParams, ProtocolVersion, RequestId, ServerNotification,
        ServerResult, SubscriptionFilter,
    },
    service::{
        serve_client_with_lifecycle_and_ct, ClientInitializeError, ClientLifecycleMode,
        NotificationContext, Peer, PeerRequestOptions, ServiceError,
    },
    transport::{streamable_http_client::StreamableHttpError, Transport},
    ClientHandler, RoleClient,
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashSet, sync::RwLock, time::Duration};
use tokio::sync::{watch, Mutex};
use tokio_util::sync::CancellationToken;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TOOL_PAGES: usize = 128;
const MAX_TOOLS: usize = 4096;

#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: std::sync::Arc<JsonObject>,
}

/// Preserve ordered, original JSON content on the action result even when a
/// block cannot be displayed by Kokoro or recognized by the SDK.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolResult {
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    result_type: Option<String>,
    #[serde(default, deserialize_with = "present_value")]
    pub structured_content: Option<Value>,
}

fn present_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

#[derive(Debug, thiserror::Error)]
pub enum McpRequestError {
    #[error("MCP protocol error {code}: {message}")]
    Protocol { code: i32, message: String },
    #[error("MCP protocol response was invalid: {0}")]
    InvalidResponse(&'static str),
    #[error("MCP transport disconnected; the outcome of an in-flight tool is unknown")]
    Disconnected,
    #[error("MCP transport request failed; the outcome of an in-flight tool is unknown")]
    Transport,
    #[error("MCP request timed out; the outcome of an in-flight tool is unknown")]
    Timeout,
    #[error("MCP request was cancelled; the outcome of an in-flight tool is unknown")]
    Cancelled,
    #[error("MCP tool changed or is no longer available; refresh the tool list")]
    ToolUnavailable,
}

impl From<ServiceError> for McpRequestError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::McpError(error) => Self::Protocol {
                code: error.code.0,
                message: error.message.into_owned(),
            },
            ServiceError::TransportClosed => Self::Disconnected,
            ServiceError::Timeout { .. } => Self::Timeout,
            ServiceError::Cancelled { .. } => Self::Cancelled,
            ServiceError::UnexpectedResponse => Self::InvalidResponse("unexpected result type"),
            // Underlying HTTP errors can include endpoint credentials or body text.
            _ => Self::Transport,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum McpConnectError {
    #[error("MCP connection cancelled")]
    Cancelled,
    #[error("MCP server did not complete initialization")]
    Initialization,
    #[error("MCP transport failed during initialization")]
    Transport,
    #[error("MCP server negotiated an unsupported protocol version")]
    Version,
    #[error("MCP tool discovery failed: {0}")]
    Discovery(#[from] McpRequestError),
}

impl From<ClientInitializeError> for McpConnectError {
    fn from(error: ClientInitializeError) -> Self {
        match error {
            ClientInitializeError::Cancelled => Self::Cancelled,
            ClientInitializeError::ConnectionClosed(_)
            | ClientInitializeError::TransportError { .. } => Self::Transport,
            ClientInitializeError::NoCompatibleProtocolVersion { .. } => Self::Version,
            _ => Self::Initialization,
        }
    }
}

struct CatalogHandler {
    initialize_version: ProtocolVersion,
    changed: watch::Sender<u64>,
}

impl ClientHandler for CatalogHandler {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            // Kokoro does not expose roots, sampling, or elicitation requests to servers.
            ClientCapabilities::default(),
            Implementation::new("kokoro-engine", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(self.initialize_version.clone())
    }

    async fn on_tool_list_changed(&self, context: NotificationContext<RoleClient>) {
        if declares_list_changed(&context.peer) {
            invalidate(&self.changed);
        }
    }
}

fn declares_list_changed(peer: &Peer<RoleClient>) -> bool {
    peer.peer_info().is_some_and(|info| {
        info.capabilities
            .tools
            .as_ref()
            .is_some_and(|tools| tools.list_changed == Some(true))
    })
}

fn invalidate(changed: &watch::Sender<u64>) {
    changed.send_modify(|rev| *rev = rev.wrapping_add(1));
}

/// A connection owns its SDK service; status and catalog reads never await network I/O.
pub struct McpClient {
    peer: Peer<RoleClient>,
    lifetime: CancellationToken,
    closed: watch::Receiver<bool>,
    changed: watch::Sender<u64>,
    catalog: RwLock<Option<(u64, Vec<McpToolInfo>)>>,
    refreshing: Mutex<()>,
}

impl McpClient {
    pub(super) async fn start<T: Transport<RoleClient> + 'static>(
        transport: T,
        legacy_sse: bool,
        lifetime: CancellationToken,
    ) -> Result<Self, McpConnectError> {
        // A service can close itself while its startup error is being returned.
        // Its child token must not cancel the manager's connection attempt.
        let lifetime = lifetime.child_token();
        let (changed, _) = watch::channel(0);
        let initialize_version = if legacy_sse {
            ProtocolVersion::V_2024_11_05
        } else {
            ProtocolVersion::LATEST_WITH_INITIALIZE
        };
        let lifecycle = if legacy_sse {
            ClientLifecycleMode::Initialize
        } else {
            ClientLifecycleMode::Auto {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                legacy_version: Some(initialize_version.clone()),
            }
        };
        let running = serve_client_with_lifecycle_and_ct(
            CatalogHandler {
                initialize_version,
                changed: changed.clone(),
            },
            transport,
            lifecycle,
            lifetime.clone(),
        )
        .await?;
        let peer = running.peer().clone();
        let info = peer.peer_info().ok_or(McpConnectError::Initialization)?;
        let max_version = if legacy_sse {
            ProtocolVersion::LATEST_WITH_INITIALIZE
        } else {
            ProtocolVersion::V_2026_07_28
        };
        if !ProtocolVersion::known_up_to(&max_version).contains(&info.protocol_version) {
            let _ = running.cancel().await;
            return Err(McpConnectError::Version);
        }

        let (finished, closed) = watch::channel(false);
        let session_lifetime = lifetime.clone();
        tokio::spawn(async move {
            let _ = running.waiting().await;
            session_lifetime.cancel();
            finished.send_replace(true);
        });
        let client = Self {
            peer,
            lifetime,
            closed,
            changed,
            catalog: RwLock::new(None),
            refreshing: Mutex::new(()),
        };
        if info.protocol_version == ProtocolVersion::V_2026_07_28
            && declares_list_changed(&client.peer)
        {
            client.subscribe_to_tool_changes();
        }
        client.refresh_tools().await?;
        Ok(client)
    }

    fn subscribe_to_tool_changes(&self) {
        let peer = self.peer.clone();
        let changed = self.changed.clone();
        let lifetime = self.lifetime.clone();
        tokio::spawn(async move {
            let subscribe = peer.listen(SubscriptionFilter::builder().tools_list_changed().build());
            let mut stream = tokio::select! {
                _ = lifetime.cancelled() => return,
                result = tokio::time::timeout(REQUEST_TIMEOUT, subscribe) => match result {
                    Ok(Ok(stream)) if stream.acknowledged().tools_list_changed == Some(true) => stream,
                    _ => {
                        tracing::debug!(target: "mcp", "Peer did not accept tool list notifications");
                        return;
                    }
                }
            };
            loop {
                tokio::select! {
                    _ = lifetime.cancelled() => return,
                    notification = stream.next() => match notification {
                        Ok(Some(ServerNotification::ToolListChangedNotification(_))) => invalidate(&changed),
                        Ok(Some(_)) => {},
                        _ => {
                            invalidate(&changed);
                            return;
                        }
                    }
                }
            }
        });
    }

    pub(super) fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub(super) fn lifetime(&self) -> CancellationToken {
        self.lifetime.clone()
    }

    pub(super) fn invalidate_tools(&self) {
        invalidate(&self.changed);
    }

    pub(super) fn tools(&self) -> Option<(u64, Vec<McpToolInfo>)> {
        if !self.is_connected() {
            return None;
        }
        self.catalog
            .read()
            .expect("MCP tool catalog lock")
            .as_ref()
            .filter(|(revision, _)| *revision == *self.changed.borrow())
            .cloned()
    }

    pub(super) fn tool_count(&self) -> u32 {
        self.catalog
            .read()
            .expect("MCP tool catalog lock")
            .as_ref()
            .filter(|(revision, _)| *revision == *self.changed.borrow())
            .and_then(|(_, tools)| u32::try_from(tools.len()).ok())
            .unwrap_or(0)
    }

    fn has_tool(&self, revision: Option<u64>, name: &str) -> bool {
        self.is_connected()
            && self
                .catalog
                .read()
                .expect("MCP tool catalog lock")
                .as_ref()
                .is_some_and(|(actual, tools)| {
                    *actual == *self.changed.borrow()
                        && revision.is_none_or(|expected| expected == *actual)
                        && tools.iter().any(|tool| tool.name == name)
                })
    }

    pub(super) async fn refresh_tools(&self) -> Result<(), McpRequestError> {
        let _guard = self.refreshing.lock().await;
        let revision = *self.changed.borrow();
        if self.tools().is_some() {
            return Ok(());
        }
        let tools = if self
            .peer
            .peer_info()
            .is_some_and(|info| info.capabilities.tools.is_some())
        {
            tokio::time::timeout(REQUEST_TIMEOUT, self.list_tools())
                .await
                .map_err(|_| McpRequestError::Timeout)??
        } else {
            Vec::new()
        };
        // A list_changed notification during pagination invalidates this snapshot.
        // The catalog comparison prevents publication even if a notification arrives
        // between this write and the subsequent registry update.
        *self.catalog.write().expect("MCP tool catalog lock") = Some((revision, tools));
        Ok(())
    }

    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpRequestError> {
        let mut tools = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
        for _ in 0..MAX_TOOL_PAGES {
            let request = ListToolsRequest {
                params: cursor
                    .take()
                    .map(|cursor| PaginatedRequestParams::default().with_cursor(Some(cursor))),
                ..Default::default()
            };
            let ServerResult::ListToolsResult(page) = self.request(request.into()).await? else {
                return Err(McpRequestError::InvalidResponse(
                    "expected tools/list result",
                ));
            };
            if page.tools.len().saturating_add(tools.len()) > MAX_TOOLS {
                return Err(McpRequestError::InvalidResponse("too many MCP tools"));
            }
            for tool in page.tools {
                if !names.insert(tool.name.to_string()) {
                    return Err(McpRequestError::InvalidResponse("duplicate MCP tool name"));
                }
                tools.push(McpToolInfo {
                    name: tool.name.into_owned(),
                    description: tool.description.map(|text| text.into_owned()),
                    input_schema: tool.input_schema,
                });
            }
            let Some(next) = page.next_cursor else {
                return Ok(tools);
            };
            if !cursors.insert(next.clone()) {
                return Err(McpRequestError::InvalidResponse(
                    "repeated tools/list cursor",
                ));
            }
            cursor = Some(next);
        }
        Err(McpRequestError::InvalidResponse(
            "too many tools/list pages",
        ))
    }

    pub(super) async fn call_tool(
        &self,
        revision: u64,
        name: &str,
        arguments: JsonObject,
    ) -> Result<McpToolResult, McpRequestError> {
        if !self.is_connected() {
            return Err(McpRequestError::Disconnected);
        }
        if !self.has_tool(Some(revision), name) {
            return Err(McpRequestError::ToolUnavailable);
        }
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(arguments);
        let result = self.request(CallToolRequest::new(params).into()).await?;
        decode_tool_result(result)
    }

    // Use typed SDK requests, but inspect its untagged result union so an opaque
    // content block (decoded as CustomResult) survives without invalidating valid siblings.
    async fn request(&self, request: ClientRequest) -> Result<ServerResult, McpRequestError> {
        let invoke = async {
            let handle = self
                .peer
                .send_cancellable_request(request, PeerRequestOptions::no_options())
                .await?;
            let mut guard = CancelOnDrop(Some((self.peer.clone(), handle.id.clone())));
            let result = handle.await_response().await;
            guard.0.take();
            if let Err(ServiceError::TransportSend(error)) = &result {
                if matches!(
                    error
                        .error
                        .downcast_ref::<StreamableHttpError<reqwest_mcp::Error>>(),
                    Some(StreamableHttpError::SessionExpired)
                ) {
                    // An expired session cannot safely serve new calls either.
                    // Drop the catalog and let the user reconnect explicitly.
                    self.cancel();
                    return Err(McpRequestError::Disconnected);
                }
            }
            result.map_err(Into::into)
        };
        tokio::select! {
            biased;
            _ = self.lifetime.cancelled() => Err(McpRequestError::Cancelled),
            result = tokio::time::timeout(REQUEST_TIMEOUT, invoke) => result.unwrap_or(Err(McpRequestError::Timeout)),
        }
    }

    pub(super) fn is_connected(&self) -> bool {
        !self.lifetime.is_cancelled() && !self.peer.is_transport_closed()
    }

    pub(super) fn server_version(&self) -> Option<String> {
        self.peer
            .peer_info()?
            .server_info
            .as_ref()
            .map(|info| info.version.clone())
    }

    pub(super) fn cancel(&self) {
        self.lifetime.cancel();
    }

    pub(super) async fn shutdown(&self) {
        self.cancel();
        let _ = tokio::time::timeout(
            Duration::from_secs(8),
            self.closed.clone().wait_for(|done| *done),
        )
        .await;
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn decode_tool_result(result: ServerResult) -> Result<McpToolResult, McpRequestError> {
    let raw = match result {
        ServerResult::CallToolResult(result) => serde_json::to_value(result)
            .map_err(|_| McpRequestError::InvalidResponse("could not convert tools/call result"))?,
        ServerResult::CustomResult(custom)
            if ["content", "structuredContent", "isError"]
                .iter()
                .any(|field| custom.0.get(field).is_some()) =>
        {
            custom.0
        }
        ServerResult::EmptyResult(_) => serde_json::json!({}),
        _ => {
            return Err(McpRequestError::InvalidResponse(
                "unsupported tools/call result type",
            ))
        }
    };
    let parsed: McpToolResult = serde_json::from_value(raw)
        .map_err(|_| McpRequestError::InvalidResponse("invalid tools/call fields"))?;
    if parsed
        .result_type
        .as_deref()
        .is_some_and(|kind| kind != "complete")
    {
        return Err(McpRequestError::InvalidResponse(
            "unsupported tools/call result type",
        ));
    }
    Ok(parsed)
}

struct CancelOnDrop(Option<(Peer<RoleClient>, RequestId)>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some((peer, id)) = self.0.take() {
            let notification = CancelledNotification::new(CancelledNotificationParam::new(
                Some(id),
                Some("Kokoro stopped waiting for the request".into()),
            ));
            // Drop is synchronous, but the SDK removes the pending request on the
            // cancellation notification. Do not hold a manager lock or retry the tool.
            tokio::spawn(async move {
                let _ = tokio::time::timeout(
                    Duration::from_secs(5),
                    peer.send_notification(notification.into()),
                )
                .await;
            });
        }
    }
}
