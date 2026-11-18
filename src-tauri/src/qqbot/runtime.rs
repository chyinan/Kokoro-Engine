// pattern: Imperative Shell

use crate::bot::lifecycle::BotIngress;
use crate::bot::{
    generate_bot_text_reply, BotChatServices, BotConfig, BotEvent, BotEvents, QqAuthorizationPrompt,
};
use crate::qqbot::protocol::{
    build_text_reply_body, conversation_key, gateway_dispatch_transition, heartbeat_tick_action,
    invalid_session_action, is_message_allowed, parse_access_token_response, parse_dispatch,
    GatewayDispatchTransition, HeartbeatTickAction, InvalidSessionAction, QQConversationKind,
    QQInboundMessage,
};
use futures::{FutureExt, SinkExt, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex, RwLock, Semaphore};
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_util::sync::CancellationToken;

const TOKEN_URL: &str = "https://bots.qq.com/app/getAppAccessToken";
const API_BASE: &str = "https://api.sgroup.qq.com";
const GROUP_AND_C2C_INTENT: u32 = 1 << 25;
const TOKEN_REFRESH_AHEAD: Duration = Duration::from_secs(300);
const MAX_DEDUPLICATED_MESSAGES: usize = 512;
const MAX_CONCURRENT_REPLIES: usize = 4;
const MAX_QUEUED_REPLY_TASKS: usize = 32;
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_PENDING_AUTHORIZATIONS: usize = 16;

#[derive(Debug, Deserialize)]
struct GatewayResponse {
    url: String,
}

#[derive(Clone)]
struct TokenProvider {
    client: Client,
    cached: Arc<RwLock<Option<CachedToken>>>,
}

#[derive(Clone)]
struct CachedToken {
    value: String,
    expires_at: Instant,
}

impl TokenProvider {
    fn new(client: Client) -> Self {
        Self {
            client,
            cached: Arc::new(RwLock::new(None)),
        }
    }

    async fn access_token(&self, app_id: &str, app_secret: &str) -> Result<String, String> {
        if let Some(token) = self.cached.read().await.clone() {
            if token.expires_at > Instant::now() + TOKEN_REFRESH_AHEAD {
                return Ok(token.value);
            }
        }

        let response = self
            .client
            .post(TOKEN_URL)
            .json(&json!({ "appId": app_id, "clientSecret": app_secret }))
            .send()
            .await
            .map_err(|error| format!("failed to request QQ access token: {}", error))?;
        let status = response.status();
        let trace_id = response
            .headers()
            .get("x-tps-trace-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response
            .text()
            .await
            .map_err(|error| format!("failed to read QQ access token response: {}", error))?;
        let payload = parse_access_token_response(&body).map_err(|error| {
            let trace = trace_id
                .as_deref()
                .map(|value| format!("; TraceId {}", value))
                .unwrap_or_default();
            format!("{} (HTTP {}{})", error, status, trace)
        })?;
        if !status.is_success() {
            return Err(format!("QQ access token endpoint returned {}", status));
        }

        let cached = CachedToken {
            value: payload.value.clone(),
            expires_at: Instant::now() + Duration::from_secs(payload.expires_in.max(60)),
        };
        *self.cached.write().await = Some(cached);
        Ok(payload.value)
    }

    async fn clear(&self) {
        *self.cached.write().await = None;
    }
}

#[derive(Default)]
struct MessageDeduplicator {
    message_ids: VecDeque<String>,
}

struct PendingAuthorization {
    request_id: String,
    user_openid: String,
    group_openid: Option<String>,
    created_at: Instant,
    decision_tx: oneshot::Sender<bool>,
}

#[derive(Clone, Default)]
pub struct QQAuthorizationBroker {
    pending_by_openid: Arc<Mutex<HashMap<String, PendingAuthorization>>>,
    rejected_openids: Arc<Mutex<HashSet<String>>>,
}

impl QQAuthorizationBroker {
    async fn request(
        &self,
        events: &dyn BotEvents,
        message: &QQInboundMessage,
        stop: &CancellationToken,
    ) -> bool {
        let authorization_key = authorization_key(message);
        if self
            .rejected_openids
            .lock()
            .await
            .contains(&authorization_key)
        {
            return false;
        }
        let request_id = message.message_id.clone();
        let (decision_tx, mut decision_rx) = oneshot::channel();
        {
            let mut pending = self.pending_by_openid.lock().await;
            if pending
                .get(&authorization_key)
                .is_some_and(|request| request.created_at.elapsed() < AUTHORIZATION_TIMEOUT)
            {
                return false;
            }
            if pending.len() >= MAX_PENDING_AUTHORIZATIONS {
                tracing::warn!(target: "bot::qq", "ignored QQ authorization request because the pending queue is full");
                return false;
            }
            pending.insert(
                authorization_key.clone(),
                PendingAuthorization {
                    request_id: request_id.clone(),
                    user_openid: message.user_openid.clone(),
                    group_openid: message.group_openid.clone(),
                    created_at: Instant::now(),
                    decision_tx,
                },
            );
        }

        let payload = QqAuthorizationPrompt {
            request_id: request_id.clone(),
            user_openid: message.user_openid.clone(),
            conversation_kind: match &message.kind {
                QQConversationKind::C2c => "c2c",
                QQConversationKind::Group => "group",
            }
            .to_string(),
            group_openid: message.group_openid.clone(),
        };
        if let Err(error) = events.emit(BotEvent::QqAuthorizationRequested(payload.clone())) {
            tracing::error!(target: "bot::qq", "failed to show QQ authorization request: {}", error);
        }

        let decision = tokio::select! {
            _ = stop.cancelled() => None,
            result = tokio::time::timeout(AUTHORIZATION_TIMEOUT, &mut decision_rx) => result.ok(),
        };
        match decision {
            Some(Ok(decision)) => decision,
            Some(Err(_)) => false,
            None => {
                let expired = {
                    let mut pending = self.pending_by_openid.lock().await;
                    if pending
                        .get(&authorization_key)
                        .is_some_and(|request| request.request_id == request_id)
                    {
                        pending.remove(&authorization_key);
                        true
                    } else {
                        false
                    }
                };
                if expired {
                    let _ = events.emit(BotEvent::QqAuthorizationExpired(payload));
                    false
                } else {
                    tokio::select! {
                        _ = stop.cancelled() => false,
                        decision = decision_rx => matches!(decision, Ok(true)),
                    }
                }
            }
        }
    }

    pub async fn take(
        &self,
        request_id: &str,
        user_openid: &str,
        group_openid: Option<&str>,
    ) -> Option<oneshot::Sender<bool>> {
        let authorization_key = match group_openid {
            Some(group_openid) => format!("group:{}", group_openid),
            None => format!("user:{}", user_openid),
        };
        let mut pending = self.pending_by_openid.lock().await;
        let matches_request = pending.get(&authorization_key).is_some_and(|request| {
            request.request_id == request_id
                && request.user_openid == user_openid
                && request.group_openid.as_deref() == group_openid
        });
        if !matches_request {
            return None;
        }
        pending
            .remove(&authorization_key)
            .map(|request| request.decision_tx)
    }

    pub async fn mark_rejected(&self, user_openid: &str, group_openid: Option<&str>) {
        let authorization_key = match group_openid {
            Some(group_openid) => format!("group:{}", group_openid),
            None => format!("user:{}", user_openid),
        };
        self.rejected_openids.lock().await.insert(authorization_key);
    }
}

fn authorization_key(message: &QQInboundMessage) -> String {
    match (&message.kind, &message.group_openid) {
        (QQConversationKind::Group, Some(group_openid)) => format!("group:{}", group_openid),
        _ => format!("user:{}", message.user_openid),
    }
}

impl MessageDeduplicator {
    fn insert_if_new(&mut self, message_id: &str) -> bool {
        if self
            .message_ids
            .iter()
            .any(|existing| existing == message_id)
        {
            return false;
        }
        self.message_ids.push_back(message_id.to_string());
        while self.message_ids.len() > MAX_DEDUPLICATED_MESSAGES {
            self.message_ids.pop_front();
        }
        true
    }
}

enum SessionExit {
    Shutdown,
    Reconnect,
    RefreshToken,
}

#[derive(Default)]
struct GatewaySessionState {
    session_id: Option<String>,
    sequence: Option<i64>,
}

pub(crate) async fn run_qq_gateway(
    config: Arc<RwLock<BotConfig>>,
    authorization_broker: QQAuthorizationBroker,
    services: BotChatServices,
    connected: Arc<AtomicBool>,
    ingress: BotIngress,
) -> Result<(), String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("Could not create QQ HTTP client: {error}"))?;
    let token_provider = TokenProvider::new(client.clone());
    let deduplicator = Arc::new(Mutex::new(MessageDeduplicator::default()));
    let reply_limit = Arc::new(Semaphore::new(MAX_CONCURRENT_REPLIES));
    let task_limit = Arc::new(Semaphore::new(MAX_QUEUED_REPLY_TASKS));
    let reconnect_delays = [1_u64, 2, 5, 10, 30, 60];
    let mut reconnect_attempt = 0_usize;
    let mut session_state = GatewaySessionState::default();
    // A reconnect replaces the socket, while admitted replies use the HTTP API.
    let mut reply_tasks = JoinSet::new();
    let stop = ingress.token();

    tracing::info!(target: "bot::qq", "QQ bot started");
    let result = loop {
        let exit = AssertUnwindSafe(run_session(
            &client,
            &token_provider,
            config.clone(),
            services.clone(),
            deduplicator.clone(),
            reply_limit.clone(),
            task_limit.clone(),
            authorization_broker.clone(),
            &mut session_state,
            &connected,
            &ingress,
            &mut reply_tasks,
        ))
        .catch_unwind()
        .await;
        connected.store(false, Ordering::SeqCst);
        let Ok(exit) = exit else {
            break Err("QQ gateway panicked".into());
        };

        match exit {
            SessionExit::Shutdown => break Ok(()),
            SessionExit::RefreshToken => {
                token_provider.clear().await;
                session_state = GatewaySessionState::default();
            }
            SessionExit::Reconnect => {}
        }

        if session_state.session_id.is_some() {
            reconnect_attempt = 0;
        }

        let delay = reconnect_delays[reconnect_attempt.min(reconnect_delays.len() - 1)];
        reconnect_attempt = reconnect_attempt.saturating_add(1);
        tracing::warn!(target: "bot::qq", "QQ gateway disconnected; reconnecting in {}s", delay);
        tokio::select! {
            _ = stop.cancelled() => break Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
        }
    };
    // Even a failed gateway must join every admitted reply before it exits.
    if result.is_err() {
        stop.cancel();
    }
    while let Some(completed) = reply_tasks.join_next().await {
        if let Err(error) = completed {
            tracing::warn!(target: "bot::qq", "QQ reply task ended unexpectedly: {error}");
        }
    }
    tracing::info!(target: "bot::qq", "QQ bot stopped");
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    client: &Client,
    token_provider: &TokenProvider,
    config: Arc<RwLock<BotConfig>>,
    services: BotChatServices,
    deduplicator: Arc<Mutex<MessageDeduplicator>>,
    reply_limit: Arc<Semaphore>,
    task_limit: Arc<Semaphore>,
    authorization_broker: QQAuthorizationBroker,
    session_state: &mut GatewaySessionState,
    connected: &AtomicBool,
    ingress: &BotIngress,
    reply_tasks: &mut JoinSet<()>,
) -> SessionExit {
    let qq_config = config.read().await.qq.clone();
    if !qq_config.enabled {
        return SessionExit::Shutdown;
    }
    let Some(app_id) = resolve_config_value(&qq_config.app_id, &qq_config.app_id_env) else {
        tracing::error!(target: "bot::qq", "QQ AppID is not configured");
        return SessionExit::Shutdown;
    };
    let Some(app_secret) = resolve_config_value(&qq_config.app_secret, &qq_config.app_secret_env)
    else {
        tracing::error!(target: "bot::qq", "QQ AppSecret is not configured");
        return SessionExit::Shutdown;
    };
    let stop = ingress.token();
    let access_token = match tokio::select! {
        _ = stop.cancelled() => return SessionExit::Shutdown,
        token = token_provider.access_token(&app_id, &app_secret) => token,
    } {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(target: "bot::qq", "{}", error);
            return SessionExit::RefreshToken;
        }
    };
    let gateway_url = match tokio::select! {
        _ = stop.cancelled() => return SessionExit::Shutdown,
        gateway = fetch_gateway_url(client, &access_token) => gateway,
    } {
        Ok(url) => url,
        Err(error) => {
            tracing::error!(target: "bot::qq", "{}", error);
            return SessionExit::RefreshToken;
        }
    };

    let (socket, _) = match tokio::select! {
        _ = stop.cancelled() => return SessionExit::Shutdown,
        socket = tokio_tungstenite::connect_async(&gateway_url) => socket,
    } {
        Ok(socket) => socket,
        Err(error) => {
            tracing::error!(target: "bot::qq", "failed to connect QQ gateway: {}", error);
            return SessionExit::Reconnect;
        }
    };
    let (mut write, mut read) = socket.split();
    let mut sequence = session_state.sequence;

    let heartbeat_interval_ms = loop {
        tokio::select! {
            _ = stop.cancelled() => return SessionExit::Shutdown,
            message = read.next() => {
                let Some(Ok(WsMessage::Text(text))) = message else {
                    return SessionExit::Reconnect;
                };
                let Ok(payload) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if payload.get("op").and_then(Value::as_i64) == Some(10) {
                    break payload
                        .get("d")
                        .and_then(|data| data.get("heartbeat_interval"))
                        .and_then(Value::as_u64)
                        .unwrap_or(45_000);
                }
            }
        }
    };

    let identify = match (&session_state.session_id, session_state.sequence) {
        (Some(session_id), Some(resume_sequence)) => json!({
            "op": 6,
            "d": {
                "token": format!("QQBot {}", access_token),
                "session_id": session_id,
                "seq": resume_sequence
            }
        }),
        _ => json!({
            "op": 2,
            "d": {
                "token": format!("QQBot {}", access_token),
                "intents": GROUP_AND_C2C_INTENT,
                "shard": [0, 1]
            }
        }),
    };
    let sent = tokio::select! {
        _ = stop.cancelled() => return SessionExit::Shutdown,
        sent = write.send(WsMessage::Text(identify.to_string().into())) => sent,
    };
    if sent.is_err() {
        return SessionExit::Reconnect;
    }

    tracing::info!(target: "bot::qq", "QQ gateway connected");
    let mut heartbeat = tokio::time::interval(Duration::from_millis(heartbeat_interval_ms.max(1)));
    heartbeat.tick().await;
    let mut awaiting_heartbeat_ack = false;

    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => {
                let _ = tokio::time::timeout(Duration::from_secs(2), write.close()).await;
                return SessionExit::Shutdown;
            }
            message = read.next() => {
                let Some(Ok(WsMessage::Text(text))) = message else {
                    return SessionExit::Reconnect;
                };
                let Ok(payload) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if let Some(next_sequence) = payload.get("s").and_then(Value::as_i64) {
                    sequence = Some(next_sequence);
                    session_state.sequence = Some(next_sequence);
                }
                match payload.get("op").and_then(Value::as_i64) {
                    Some(0) => {
                        let Some(event_type) = payload.get("t").and_then(Value::as_str) else {
                            continue;
                        };
                        let data = payload.get("d").cloned().unwrap_or(Value::Null);
                        match gateway_dispatch_transition(event_type, &data) {
                            Ok(GatewayDispatchTransition::Ready(session_id)) => {
                                session_state.session_id = Some(session_id);
                                connected.store(true, Ordering::SeqCst);
                                continue;
                            }
                            Ok(GatewayDispatchTransition::Resumed) => {
                                connected.store(true, Ordering::SeqCst);
                                continue;
                            }
                            Ok(GatewayDispatchTransition::Other) => {}
                            Err(error) => {
                                tracing::warn!(target: "bot::qq", "ignored invalid QQ gateway event {}: {}", event_type, error);
                                continue;
                            }
                        }
                        let message = match parse_dispatch(event_type, &data) {
                            Ok(Some(message)) => message,
                            Ok(None) => continue,
                            Err(error) => {
                                tracing::warn!(target: "bot::qq", "ignored invalid QQ event {}: {}", event_type, error);
                                continue;
                            }
                        };
                        tracing::info!(
                            target: "bot::qq",
                            "received QQ {} message: user_openid={}, group_openid={}",
                            match &message.kind {
                                QQConversationKind::C2c => "C2C",
                                QQConversationKind::Group => "group",
                            },
                            message.user_openid,
                            message.group_openid.as_deref().unwrap_or("-")
                        );
                        let Some(work) = ingress.admit("QQ message") else {
                            continue;
                        };
                        if !deduplicator.lock().await.insert_if_new(&message.message_id) {
                            tracing::debug!(target: "bot::qq", "ignored duplicate QQ message {}", message.message_id);
                            continue;
                        }
                        let Ok(task_admission) = task_limit.clone().try_acquire_owned() else {
                            tracing::warn!(target: "bot::qq", "dropped QQ message because the reply queue is full");
                            continue;
                        };
                        let task_client = client.clone();
                        let task_token_provider = token_provider.clone();
                        let task_config = config.clone();
                        let task_services = services.clone();
                        let task_app_id = app_id.clone();
                        let task_app_secret = app_secret.clone();
                        let task_reply_limit = reply_limit.clone();
                        let task_authorization_broker = authorization_broker.clone();
                        let stop = stop.clone();
                        reply_tasks.spawn(async move {
                            let _task_admission = task_admission;
                            let _work = work;
                            if let Err(error) = handle_inbound_message(
                                &task_client,
                                &task_token_provider,
                                &task_app_id,
                                &task_app_secret,
                                &task_config,
                                &task_services,
                                &task_authorization_broker,
                                &task_reply_limit,
                                message,
                                &stop,
                            )
                            .await
                            {
                                tracing::error!(target: "bot::qq", "failed to handle QQ message: {}", error);
                            }
                        });
                    }
                    Some(7) => return SessionExit::Reconnect,
                    Some(9) => {
                        return match invalid_session_action(
                            payload.get("d").and_then(Value::as_bool).unwrap_or(false),
                        ) {
                            InvalidSessionAction::Resume => SessionExit::Reconnect,
                            InvalidSessionAction::Reidentify => SessionExit::RefreshToken,
                        };
                    }
                    Some(11) => awaiting_heartbeat_ack = false,
                    _ => {}
                }
            }
            completed = reply_tasks.join_next(), if !reply_tasks.is_empty() => {
                if let Some(Err(error)) = completed {
                    tracing::warn!(target: "bot::qq", "QQ reply task ended unexpectedly: {}", error);
                }
            }
            _ = heartbeat.tick() => {
                if heartbeat_tick_action(awaiting_heartbeat_ack) == HeartbeatTickAction::Reconnect {
                    tracing::warn!(target: "bot::qq", "QQ heartbeat acknowledgement timed out");
                    return SessionExit::Reconnect;
                }
                let sent = tokio::select! {
                    _ = stop.cancelled() => return SessionExit::Shutdown,
                    sent = write.send(WsMessage::Text(json!({ "op": 1, "d": sequence }).to_string().into())) => sent,
                };
                if sent.is_err() {
                    return SessionExit::Reconnect;
                }
                awaiting_heartbeat_ack = true;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_inbound_message(
    client: &Client,
    token_provider: &TokenProvider,
    app_id: &str,
    app_secret: &str,
    config: &Arc<RwLock<BotConfig>>,
    services: &BotChatServices,
    authorization_broker: &QQAuthorizationBroker,
    reply_limit: &Arc<Semaphore>,
    message: QQInboundMessage,
    stop: &CancellationToken,
) -> Result<(), String> {
    let mut qq_config = config.read().await.qq.clone();
    if !qq_config.enabled {
        return Ok(());
    }
    let mut allowed = is_message_allowed(
        &message,
        qq_config.allow_c2c,
        &qq_config.allowed_user_openids,
        &qq_config.allowed_group_openids,
    );
    let can_request_authorization = match &message.kind {
        QQConversationKind::C2c => qq_config.allow_c2c,
        QQConversationKind::Group => message.group_openid.is_some(),
    };
    if !allowed
        && can_request_authorization
        && authorization_broker
            .request(services.events.as_ref(), &message, stop)
            .await
    {
        qq_config = config.read().await.qq.clone();
        allowed = is_message_allowed(
            &message,
            qq_config.allow_c2c,
            &qq_config.allowed_user_openids,
            &qq_config.allowed_group_openids,
        );
    }
    if !allowed {
        tracing::warn!(
            target: "bot::qq",
            "rejected QQ {} message by allowlist: user_openid={}, group_openid={}; add these OpenIDs in Bot settings if this sender/group should be allowed",
            match &message.kind {
                QQConversationKind::C2c => "C2C",
                QQConversationKind::Group => "group",
            },
            message.user_openid,
            message.group_openid.as_deref().unwrap_or("-")
        );
        return Ok(());
    }
    if message.content.is_empty() {
        tracing::debug!(target: "bot::qq", "ignored empty QQ message {}", message.message_id);
        return Ok(());
    }

    let _permit = reply_limit
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "QQ reply service is shutting down".to_string())?;

    let key = conversation_key(&message);
    let reply = generate_bot_text_reply(
        services,
        "qq",
        &message.content,
        qq_config.character_id.as_deref(),
        &key,
        &message.message_id,
    )
    .await?;
    if reply.trim().is_empty() {
        return Ok(());
    }

    let access_token = token_provider.access_token(app_id, app_secret).await?;
    send_text_reply(client, &access_token, &message, &reply).await
}

async fn fetch_gateway_url(client: &Client, access_token: &str) -> Result<String, String> {
    let response = client
        .get(format!("{}/gateway", API_BASE))
        .header("Authorization", format!("QQBot {}", access_token))
        .send()
        .await
        .map_err(|error| format!("failed to request QQ gateway URL: {}", error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("QQ gateway endpoint returned {}", status));
    }
    let payload = response
        .json::<GatewayResponse>()
        .await
        .map_err(|error| format!("failed to parse QQ gateway response: {}", error))?;
    if payload.url.trim().is_empty() {
        return Err("QQ gateway response did not include a URL".to_string());
    }
    Ok(payload.url)
}

async fn send_text_reply(
    client: &Client,
    access_token: &str,
    message: &QQInboundMessage,
    content: &str,
) -> Result<(), String> {
    let path = match (&message.kind, &message.group_openid) {
        (QQConversationKind::C2c, _) => format!("/v2/users/{}/messages", message.user_openid),
        (QQConversationKind::Group, Some(group_openid)) => {
            format!("/v2/groups/{}/messages", group_openid)
        }
        (QQConversationKind::Group, None) => {
            return Err("QQ group message is missing group_openid".to_string())
        }
    };
    let response = client
        .post(format!("{}{}", API_BASE, path))
        .header("Authorization", format!("QQBot {}", access_token))
        .json(&build_text_reply_body(content, &message.message_id, 1))
        .send()
        .await
        .map_err(|error| format!("QQ reply delivery is uncertain: {}", error.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "QQ message endpoint returned {}",
            response.status()
        ));
    }
    Ok(())
}

fn resolve_config_value(
    value: &Option<String>,
    environment_name: &Option<String>,
) -> Option<String> {
    value
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            environment_name
                .as_ref()
                .and_then(|name| std::env::var(name).ok())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

#[cfg(test)]
mod authorization_tests {
    use super::*;

    #[tokio::test]
    async fn takes_only_the_matching_pending_authorization() {
        let broker = QQAuthorizationBroker::default();
        let (decision_tx, decision_rx) = oneshot::channel();
        broker.pending_by_openid.lock().await.insert(
            "user:user-1".to_string(),
            PendingAuthorization {
                request_id: "request-1".to_string(),
                user_openid: "user-1".to_string(),
                group_openid: None,
                created_at: Instant::now(),
                decision_tx,
            },
        );

        assert!(broker.take("other", "user-1", None).await.is_none());
        let sender = broker
            .take("request-1", "user-1", None)
            .await
            .expect("matching request should be taken");
        sender.send(true).expect("decision receiver should exist");
        assert!(decision_rx.await.expect("decision should arrive"));
        assert!(broker.take("request-1", "user-1", None).await.is_none());
    }

    #[tokio::test]
    async fn remembers_rejected_openids_for_the_runtime_session() {
        let broker = QQAuthorizationBroker::default();

        broker.mark_rejected("user-1", None).await;

        assert!(broker.rejected_openids.lock().await.contains("user:user-1"));
    }
}
