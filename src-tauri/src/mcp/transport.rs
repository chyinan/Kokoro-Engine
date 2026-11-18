//! SDK transports, plus the legacy GET/SSE + POST binding removed from rmcp.

use super::manager::McpServerConfig;
use crate::error::KokoroError;
use eventsource_stream::{Event, Eventsource};
use futures::{stream::BoxStream, StreamExt, TryStreamExt};
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use reqwest_mcp::{header, redirect, Client, StatusCode, Url};
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{
        common::client_side_sse::NeverRetry,
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
        TokioChildProcess, Transport,
    },
    RoleClient,
};
use std::{future::Future, io, process::Stdio, sync::Arc, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransportMode {
    Stdio,
    StreamableHttp,
    LegacySse,
}

pub(super) fn transport_mode(config: &McpServerConfig) -> Result<TransportMode, KokoroError> {
    match config.transport_type.as_str() {
        "streamable_http" | "streamable-http" => Ok(TransportMode::StreamableHttp),
        "sse" => Ok(TransportMode::LegacySse),
        _ if !config.command.is_empty() => Ok(TransportMode::Stdio),
        _ => match config.url.as_deref() {
            Some(url) if url.trim_end_matches('/').ends_with("/sse") => {
                Ok(TransportMode::LegacySse)
            }
            Some(_) => Ok(TransportMode::StreamableHttp),
            None => Err(KokoroError::Config(
                "MCP server requires a command or URL".into(),
            )),
        },
    }
}

pub(super) fn server_url(config: &McpServerConfig) -> Result<Url, KokoroError> {
    config
        .url
        .as_deref()
        .and_then(|url| Url::parse(url).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
        .ok_or_else(|| KokoroError::Config("MCP server requires a valid HTTP or HTTPS URL".into()))
}

pub(super) fn stdio(config: &McpServerConfig) -> Result<StdioTransport, KokoroError> {
    #[cfg(windows)]
    let mut command = {
        // Resolve .cmd/.bat shims using the server's PATH without changing
        // the application process environment or interpreting shell expressions.
        let paths = config
            .env
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| std::ffi::OsString::from(value))
            .or_else(|| std::env::var_os("PATH"));
        let cwd = std::env::current_dir()
            .map_err(|err| KokoroError::Io(format!("MCP working directory unavailable: {err}")))?;
        let executable = which::which_in(&config.command, paths, cwd)
            .map_err(|_| KokoroError::Config("MCP executable was not found".into()))?;
        tokio::process::Command::new(executable)
    };
    #[cfg(not(windows))]
    let mut command = tokio::process::Command::new(&config.command);

    command.args(&config.args).envs(&config.env);
    let mut wrapped = CommandWrap::from(command);
    wrapped.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        let mut flags = process_wrap::tokio::CreationFlags(Default::default());
        flags.0 .0 = 0x08000000; // CREATE_NO_WINDOW
        wrapped.wrap(flags).wrap(process_wrap::tokio::JobObject);
    }
    #[cfg(unix)]
    wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());

    let (transport, stderr) = TokioChildProcess::builder(wrapped)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            KokoroError::ExternalService(format!("MCP process spawn failed: {error}"))
        })?;
    let stderr_drain = stderr.map(|mut stderr| {
        // Servers may write credentials to stderr. Drain it without logging it or retaining it.
        tokio::spawn(async move {
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
        })
    });
    Ok(StdioTransport {
        child: Some(transport),
        stderr_drain,
    })
}

pub(super) struct StdioTransport {
    child: Option<TokioChildProcess>,
    stderr_drain: Option<JoinHandle<()>>,
}

impl Transport<RoleClient> for StdioTransport {
    type Error = io::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.child.as_mut().expect("open MCP child").send(message)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.child.as_mut()?.receive().await
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let result = if let Some(mut child) = self.child.take() {
            match child.graceful_shutdown().await {
                Ok(()) => Ok(()),
                Err(error) => {
                    // A failed protocol close still owns the process. Reap it
                    // here instead of relying on the SDK's asynchronous Drop.
                    if let Some(mut process) = child.into_inner() {
                        let reap = Box::into_pin(process.kill()).await;
                        if reap.is_ok() {
                            tracing::warn!(target: "mcp", %error, "MCP child was reaped after graceful shutdown failed");
                        }
                        reap
                    } else {
                        Err(error)
                    }
                }
            }
        } else {
            Ok(())
        };
        if let Some(drain) = self.stderr_drain.take() {
            // The child's descendants can inherit stderr. Stop the drain
            // after the process wrapper has completed shutdown.
            drain.abort();
            if let Err(error) = drain.await {
                if !error.is_cancelled() {
                    return Err(io::Error::other(error));
                }
            }
        }
        result
    }
}

/// Return the resource to its owner even when the SDK drops its transport
/// during negotiation. Only the owner reports success after the close joins.
pub(super) struct RetainedTransport<T> {
    transport: Option<T>,
    returned: Option<oneshot::Sender<T>>,
}

impl<T> RetainedTransport<T> {
    pub(super) fn new(transport: T) -> (Self, oneshot::Receiver<T>) {
        let (returned, receiver) = oneshot::channel();
        (
            Self {
                transport: Some(transport),
                returned: Some(returned),
            },
            receiver,
        )
    }
}

impl<T: Transport<RoleClient>> Transport<RoleClient> for RetainedTransport<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.transport
            .as_mut()
            .expect("open MCP transport")
            .send(message)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.transport.as_mut()?.receive().await
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        // The owner closes the returned transport after joining the SDK.
        Ok(())
    }
}

impl<T> Drop for RetainedTransport<T> {
    fn drop(&mut self) {
        if let (Some(returned), Some(transport)) = (self.returned.take(), self.transport.take()) {
            let _ = returned.send(transport);
        }
    }
}

fn http_client(redirect: redirect::Policy) -> Result<Client, LegacySseError> {
    // A POST can perform a side effect. Do not let redirects or HTTP retries replay it.
    Client::builder()
        .redirect(redirect)
        .retry(reqwest_mcp::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| LegacySseError::Http(e.without_url()))
}

pub(super) fn streamable_http(
    url: Url,
) -> Result<StreamableHttpClientTransport<Client>, KokoroError> {
    let config = StreamableHttpClientTransportConfig::with_uri(url.as_str())
        .reinit_on_expired_session(false);
    let mut config = config;
    // Session loss is surfaced to Kokoro; only explicit reconnection can replace the peer.
    config.retry_config = Arc::new(NeverRetry::default());
    let client = http_client(redirect::Policy::none())
        .map_err(|error| KokoroError::ExternalService(error.to_string()))?;
    Ok(StreamableHttpClientTransport::with_client(client, config))
}

#[derive(Debug, thiserror::Error)]
pub(super) enum LegacySseError {
    #[error("MCP HTTP request failed: {0}")]
    Http(#[source] reqwest_mcp::Error),
    #[error("legacy MCP SSE returned HTTP {0}")]
    Status(StatusCode),
    #[error("legacy MCP SSE response is not text/event-stream")]
    ContentType,
    #[error("legacy MCP SSE did not provide a same-origin message endpoint")]
    Endpoint,
    #[error("legacy MCP SSE connection closed")]
    Closed,
}

type Events = BoxStream<'static, Result<Event, LegacySseError>>;

/// Only binds a legacy GET stream and its advertised POST address. The SDK owns
/// every JSON-RPC envelope, request ID, response, notification, and cancellation.
pub(super) struct LegacySse {
    client: Client,
    endpoint: Url,
    events: Option<Events>,
}

impl LegacySse {
    pub(super) async fn connect(url: Url) -> Result<Self, LegacySseError> {
        // Discovery GET has no tool side effects. Preserve same-origin redirects
        // there and resolve the endpoint relative to the effective stream URL.
        let origin = url.origin();
        let get_client = http_client(redirect::Policy::custom(move |attempt| {
            if attempt.url().origin() == origin && attempt.previous().len() < 10 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))?;
        let response = get_client
            .get(url.clone())
            .header(header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|e| LegacySseError::Http(e.without_url()))?;
        if !response.status().is_success() {
            return Err(LegacySseError::Status(response.status()));
        }
        let event_stream = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
        if !event_stream {
            return Err(LegacySseError::ContentType);
        }
        let effective_url = response.url().clone();
        let mut limit = SseFrameLimit::default();
        let mut events = response
            .bytes_stream()
            .map_err(io::Error::other)
            .and_then(move |chunk| std::future::ready(limit.check(&chunk).map(|()| chunk)))
            .eventsource()
            .map_err(|_| LegacySseError::Closed)
            .boxed();
        while let Some(event) = events.next().await {
            let event = event?;
            if event.event == "endpoint" {
                let endpoint = effective_url
                    .join(event.data.trim())
                    .ok()
                    .filter(|endpoint| {
                        !event.data.trim().is_empty()
                            && endpoint.origin() == url.origin()
                            && endpoint.username().is_empty()
                            && endpoint.password().is_none()
                    })
                    .ok_or(LegacySseError::Endpoint)?;
                // Keep credentials from the user configuration only, even
                // after the stream has redirected within its configured origin.
                let mut post_url = url.clone();
                post_url.set_path(endpoint.path());
                post_url.set_query(endpoint.query());
                post_url.set_fragment(None);
                return Ok(Self {
                    client: http_client(redirect::Policy::none())?,
                    endpoint: post_url,
                    events: Some(events),
                });
            }
        }
        Err(LegacySseError::Endpoint)
    }
}

impl Transport<RoleClient> for LegacySse {
    type Error = LegacySseError;

    fn send(
        &mut self,
        item: ClientJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let request = self.client.post(self.endpoint.clone()).json(&item);
        async move {
            let response = request
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| LegacySseError::Http(e.without_url()))?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(LegacySseError::Status(response.status()))
            }
        }
    }

    async fn receive(&mut self) -> Option<ServerJsonRpcMessage> {
        while let Some(Ok(event)) = self.events.as_mut()?.next().await {
            if event.event == "message" {
                // Malformed JSON-RPC is not silently converted into an empty tool result.
                return serde_json::from_str(&event.data).ok();
            }
        }
        None
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.events.take();
        Ok(())
    }
}

/// Bound input before the SSE parser buffers a possibly unfinished event.
/// CRLF is one delimiter; comment lines do not accumulate across keep-alives.
#[derive(Default)]
struct SseFrameLimit {
    event_bytes: usize,
    line_bytes: usize,
    comment: bool,
    after_cr: bool,
}

impl SseFrameLimit {
    fn check(&mut self, chunk: &[u8]) -> io::Result<()> {
        for &byte in chunk {
            if std::mem::take(&mut self.after_cr) && byte == b'\n' {
                continue;
            }
            if matches!(byte, b'\r' | b'\n') {
                self.after_cr = byte == b'\r';
                let size = std::mem::take(&mut self.line_bytes);
                if size == 0 {
                    self.event_bytes = 0;
                } else if !self.comment {
                    self.event_bytes += size + 1;
                }
            } else {
                if self.line_bytes == 0 {
                    self.comment = byte == b':';
                }
                self.line_bytes += 1;
            }
            if self.event_bytes + self.line_bytes > 16 * 1024 * 1024 {
                return Err(io::Error::other("MCP SSE event exceeded 16 MiB"));
            }
        }
        Ok(())
    }
}
