// pattern: Functional Core

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QQConversationKind {
    C2c,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QQInboundMessage {
    pub kind: QQConversationKind,
    pub message_id: String,
    pub content: String,
    pub user_openid: String,
    pub group_openid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QQAccessToken {
    pub value: String,
    pub expires_in: u64,
}

pub fn parse_access_token_response(body: &str) -> Result<QQAccessToken, String> {
    let payload = serde_json::from_str::<Value>(body)
        .map_err(|error| format!("QQ access token endpoint returned invalid JSON: {}", error))?;

    if let Some(value) = payload
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let expires_in = match payload.get("expires_in") {
            None => 7200,
            Some(Value::Number(value)) => value
                .as_u64()
                .ok_or("QQ access token response contained an invalid expires_in")?,
            Some(Value::String(value)) => value
                .parse::<u64>()
                .map_err(|_| "QQ access token response contained an invalid expires_in")?,
            Some(_) => {
                return Err("QQ access token response contained an invalid expires_in".into())
            }
        };
        return Ok(QQAccessToken {
            value: value.to_string(),
            expires_in,
        });
    }

    let code = payload
        .get("code")
        .or_else(|| payload.get("err_code"))
        .map(|value| match value {
            Value::String(value) => value.clone(),
            value => value.to_string(),
        })
        .unwrap_or_else(|| "unknown".to_string());
    let message = payload
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("access_token missing from response");
    Err(format!(
        "QQ access token rejected (code {}): {}",
        code, message
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayDispatchTransition {
    Ready(String),
    Resumed,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSessionAction {
    Resume,
    Reidentify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatTickAction {
    Send,
    Reconnect,
}

pub fn gateway_dispatch_transition(
    event_type: &str,
    data: &Value,
) -> Result<GatewayDispatchTransition, String> {
    match event_type {
        "READY" => Ok(GatewayDispatchTransition::Ready(required_string(
            data,
            "session_id",
        )?)),
        "RESUMED" => Ok(GatewayDispatchTransition::Resumed),
        _ => Ok(GatewayDispatchTransition::Other),
    }
}

pub fn invalid_session_action(can_resume: bool) -> InvalidSessionAction {
    if can_resume {
        InvalidSessionAction::Resume
    } else {
        InvalidSessionAction::Reidentify
    }
}

pub fn heartbeat_tick_action(awaiting_acknowledgement: bool) -> HeartbeatTickAction {
    if awaiting_acknowledgement {
        HeartbeatTickAction::Reconnect
    } else {
        HeartbeatTickAction::Send
    }
}

pub fn parse_dispatch(event_type: &str, data: &Value) -> Result<Option<QQInboundMessage>, String> {
    match event_type {
        "C2C_MESSAGE_CREATE" => Ok(Some(QQInboundMessage {
            kind: QQConversationKind::C2c,
            message_id: required_string(data, "id")?,
            content: normalize_message_content(&required_string(data, "content")?),
            user_openid: required_nested_string(data, "author", "user_openid")?,
            group_openid: None,
        })),
        "GROUP_AT_MESSAGE_CREATE" => Ok(Some(QQInboundMessage {
            kind: QQConversationKind::Group,
            message_id: required_string(data, "id")?,
            content: normalize_message_content(&required_string(data, "content")?),
            user_openid: required_nested_string(data, "author", "member_openid")?,
            group_openid: Some(required_string(data, "group_openid")?),
        })),
        _ => Ok(None),
    }
}

pub fn is_message_allowed(
    message: &QQInboundMessage,
    allow_c2c: bool,
    allowed_user_openids: &[String],
    allowed_group_openids: &[String],
) -> bool {
    match message.kind {
        QQConversationKind::C2c => {
            allow_c2c
                && !allowed_user_openids.is_empty()
                && allowed_user_openids
                    .iter()
                    .any(|openid| openid == &message.user_openid)
        }
        QQConversationKind::Group => message.group_openid.as_ref().is_some_and(|group_openid| {
            !allowed_group_openids.is_empty()
                && allowed_group_openids
                    .iter()
                    .any(|allowed| allowed == group_openid)
                && (allowed_user_openids.is_empty()
                    || allowed_user_openids
                        .iter()
                        .any(|openid| openid == &message.user_openid))
        }),
    }
}

pub fn conversation_key(message: &QQInboundMessage) -> String {
    match (&message.kind, &message.group_openid) {
        (QQConversationKind::C2c, _) => format!("qq:c2c:{}", message.user_openid),
        (QQConversationKind::Group, Some(group_openid)) => {
            format!("qq:group:{}:{}", group_openid, message.user_openid)
        }
        (QQConversationKind::Group, None) => format!("qq:group:unknown:{}", message.user_openid),
    }
}

pub fn build_text_reply_body(content: &str, message_id: &str, message_sequence: u32) -> Value {
    serde_json::json!({
        "content": content,
        "msg_type": 0,
        "msg_id": message_id,
        "msg_seq": message_sequence,
    })
}

fn required_string(data: &Value, field: &str) -> Result<String, String> {
    data.get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing or invalid {}", field))
}

fn normalize_message_content(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.starts_with("<@") {
        if let Some(end) = trimmed.find('>') {
            return trimmed[end + 1..].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn required_nested_string(data: &Value, parent: &str, field: &str) -> Result<String, String> {
    data.get(parent)
        .and_then(|value| value.get(field))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing or invalid {}.{}", parent, field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_c2c_message_create() {
        let data = json!({
            "id": "message-1",
            "content": " hello ",
            "author": { "user_openid": "user-1" }
        });

        let message = parse_dispatch("C2C_MESSAGE_CREATE", &data)
            .expect("event should parse")
            .expect("event should be supported");

        assert_eq!(message.kind, QQConversationKind::C2c);
        assert_eq!(message.message_id, "message-1");
        assert_eq!(message.content, "hello");
        assert_eq!(message.user_openid, "user-1");
        assert_eq!(message.group_openid, None);
    }

    #[test]
    fn parses_group_at_message_create() {
        let data = json!({
            "id": "message-2",
            "content": "  <@!bot-openid> hi group  ",
            "group_openid": "group-1",
            "author": { "member_openid": "member-1" }
        });

        let message = parse_dispatch("GROUP_AT_MESSAGE_CREATE", &data)
            .expect("event should parse")
            .expect("event should be supported");

        assert_eq!(message.kind, QQConversationKind::Group);
        assert_eq!(message.content, "hi group");
        assert_eq!(message.user_openid, "member-1");
        assert_eq!(message.group_openid.as_deref(), Some("group-1"));
    }

    #[test]
    fn ignores_unsupported_dispatch_events() {
        let message = parse_dispatch("READY", &json!({ "session_id": "session-1" }))
            .expect("unsupported events should not fail");

        assert_eq!(message, None);
    }

    #[test]
    fn validates_required_event_fields() {
        let error = parse_dispatch(
            "C2C_MESSAGE_CREATE",
            &json!({ "id": "message-1", "content": "hello", "author": {} }),
        )
        .expect_err("missing user_openid should fail");

        assert!(error.contains("user_openid"));
    }

    #[test]
    fn applies_c2c_and_group_allowlists() {
        let c2c = QQInboundMessage {
            kind: QQConversationKind::C2c,
            message_id: "message-1".to_string(),
            content: "hello".to_string(),
            user_openid: "user-1".to_string(),
            group_openid: None,
        };
        let group = QQInboundMessage {
            kind: QQConversationKind::Group,
            message_id: "message-2".to_string(),
            content: "hello".to_string(),
            user_openid: "member-1".to_string(),
            group_openid: Some("group-1".to_string()),
        };

        assert!(!is_message_allowed(&c2c, true, &[], &[]));
        assert!(!is_message_allowed(&c2c, false, &[], &[]));
        assert!(is_message_allowed(&c2c, true, &["user-1".to_string()], &[]));
        assert!(!is_message_allowed(&c2c, true, &["other".to_string()], &[]));
        assert!(!is_message_allowed(&group, true, &[], &[]));
        assert!(is_message_allowed(
            &group,
            true,
            &[],
            &["group-1".to_string()]
        ));
        assert!(!is_message_allowed(
            &group,
            true,
            &[],
            &["other".to_string()]
        ));
    }

    #[test]
    fn builds_isolated_conversation_keys() {
        let c2c = QQInboundMessage {
            kind: QQConversationKind::C2c,
            message_id: "message-1".to_string(),
            content: "hello".to_string(),
            user_openid: "user-1".to_string(),
            group_openid: None,
        };
        let group = QQInboundMessage {
            kind: QQConversationKind::Group,
            message_id: "message-2".to_string(),
            content: "hello".to_string(),
            user_openid: "member-1".to_string(),
            group_openid: Some("group-1".to_string()),
        };

        assert_eq!(conversation_key(&c2c), "qq:c2c:user-1");
        assert_eq!(conversation_key(&group), "qq:group:group-1:member-1");
    }

    #[test]
    fn builds_passive_text_reply_body() {
        assert_eq!(
            build_text_reply_body("hello", "message-1", 1),
            json!({
                "content": "hello",
                "msg_type": 0,
                "msg_id": "message-1",
                "msg_seq": 1
            })
        );
    }

    #[test]
    fn parses_ready_and_resumed_gateway_transitions() {
        assert_eq!(
            gateway_dispatch_transition("READY", &json!({ "session_id": "session-1" }))
                .expect("READY should parse"),
            GatewayDispatchTransition::Ready("session-1".to_string())
        );
        assert_eq!(
            gateway_dispatch_transition("RESUMED", &json!({})).expect("RESUMED should parse"),
            GatewayDispatchTransition::Resumed
        );
        assert_eq!(
            gateway_dispatch_transition("C2C_MESSAGE_CREATE", &json!({}))
                .expect("other events should parse"),
            GatewayDispatchTransition::Other
        );
        assert!(gateway_dispatch_transition("READY", &json!({})).is_err());
    }

    #[test]
    fn selects_invalid_session_recovery_from_resumability() {
        assert_eq!(invalid_session_action(true), InvalidSessionAction::Resume);
        assert_eq!(
            invalid_session_action(false),
            InvalidSessionAction::Reidentify
        );
    }

    #[test]
    fn reconnects_when_a_heartbeat_is_still_unacknowledged() {
        assert_eq!(heartbeat_tick_action(false), HeartbeatTickAction::Send);
        assert_eq!(heartbeat_tick_action(true), HeartbeatTickAction::Reconnect);
    }

    #[test]
    fn parses_successful_access_token_responses() {
        let token =
            parse_access_token_response(r#"{"access_token":"token-value","expires_in":7200}"#)
                .expect("valid token response should parse");

        assert_eq!(token.value, "token-value");
        assert_eq!(token.expires_in, 7200);
    }

    #[test]
    fn accepts_string_access_token_expiry() {
        let token =
            parse_access_token_response(r#"{"access_token":"token-value","expires_in":"7200"}"#)
                .expect("string expiry should parse");

        assert_eq!(token.expires_in, 7200);
    }

    #[test]
    fn reports_qq_access_token_business_errors() {
        let error = parse_access_token_response(r#"{"code":100007,"message":"appid invalid"}"#)
            .expect_err("QQ business error should be rejected");

        assert_eq!(
            error,
            "QQ access token rejected (code 100007): appid invalid"
        );
    }
}
