use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

/// 统一的 Kokoro 错误类型，支持结构化序列化
#[derive(Debug, Error, Serialize, Deserialize, Clone)]
#[serde(tag = "code", content = "message")]
pub enum KokoroError {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("数据库错误: {0}")]
    Database(String),

    #[error("LLM 错误: {0}")]
    Llm(String),

    #[error("TTS 错误: {0}")]
    Tts(String),

    #[error("STT 错误: {0}")]
    Stt(String),

    #[error("IO 错误: {0}")]
    Io(String),

    #[error("外部服务错误: {0}")]
    ExternalService(String),

    #[error("MOD 错误: {0}")]
    Mod(String),

    #[error("未找到: {0}")]
    NotFound(String),

    #[error("未授权: {0}")]
    Unauthorized(String),

    #[error("内部错误: {0}")]
    Internal(String),

    #[error("聊天错误: {0}")]
    Chat(String),

    #[error("校验错误: {0}")]
    Validation(String),
}

/// 便捷类型别名
pub type KokoroResult<T> = Result<T, KokoroError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailureEvent {
    pub event_id: String,
    pub timestamp: String,
    pub domain: String,
    pub stage: String,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub trace_id: String,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub character_id: Option<String>,
    pub context: Option<Value>,
}

impl FailureEvent {
    pub fn new(options: FailureEventOptions) -> Self {
        Self {
            event_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now().to_rfc3339(),
            domain: options.domain,
            stage: options.stage,
            code: options.code,
            message: options.message,
            retryable: options.retryable,
            trace_id: options.trace_id,
            conversation_id: options.conversation_id,
            turn_id: options.turn_id,
            character_id: options.character_id,
            context: options.context,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FailureEventOptions {
    pub domain: String,
    pub stage: String,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub trace_id: String,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub character_id: Option<String>,
    pub context: Option<Value>,
}

impl FailureEventOptions {
    pub fn new(
        domain: impl Into<String>,
        stage: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            domain: domain.into(),
            stage: stage.into(),
            code: code.into(),
            message: message.into(),
            retryable,
            trace_id: trace_id.into(),
            conversation_id: None,
            turn_id: None,
            character_id: None,
            context: None,
        }
    }

    pub fn with_conversation_id(mut self, conversation_id: Option<String>) -> Self {
        self.conversation_id = conversation_id;
        self
    }

    pub fn with_turn_id(mut self, turn_id: Option<String>) -> Self {
        self.turn_id = turn_id;
        self
    }

    pub fn with_character_id(mut self, character_id: Option<String>) -> Self {
        self.character_id = character_id;
        self
    }

    pub fn with_context(mut self, context: Option<Value>) -> Self {
        self.context = context;
        self
    }
}

impl From<KokoroError> for FailureEvent {
    fn from(error: KokoroError) -> Self {
        let (code, retryable) = match &error {
            KokoroError::Config(_) => ("CONFIG_ERROR", false),
            KokoroError::Database(_) => ("DATABASE_ERROR", true),
            KokoroError::Llm(_) => ("LLM_ERROR", true),
            KokoroError::Tts(_) => ("TTS_ERROR", true),
            KokoroError::Stt(_) => ("STT_ERROR", true),
            KokoroError::Io(_) => ("IO_ERROR", true),
            KokoroError::ExternalService(_) => ("EXTERNAL_SERVICE_ERROR", true),
            KokoroError::Mod(_) => ("MOD_ERROR", false),
            KokoroError::NotFound(_) => ("NOT_FOUND", false),
            KokoroError::Unauthorized(_) => ("UNAUTHORIZED", false),
            KokoroError::Internal(_) => ("INTERNAL_ERROR", false),
            KokoroError::Chat(_) => ("CHAT_ERROR", true),
            KokoroError::Validation(_) => ("VALIDATION_ERROR", false),
        };

        FailureEvent::new(FailureEventOptions::new(
            "system",
            "unknown",
            code,
            error.to_string(),
            retryable,
            "",
        ))
    }
}

impl KokoroError {
    pub fn into_failure_event(
        self,
        stage: impl Into<String>,
        trace_id: impl Into<String>,
        conversation_id: Option<String>,
        turn_id: Option<String>,
        character_id: Option<String>,
        context: Option<Value>,
    ) -> FailureEvent {
        let mut event = FailureEvent::from(self);
        event.stage = stage.into();
        event.trace_id = trace_id.into();
        event.conversation_id = conversation_id;
        event.turn_id = turn_id;
        event.character_id = character_id;
        event.context = context;
        event
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatErrorEvent {
    pub code: String,
    pub stage: String,
    pub retryable: bool,
    pub trace_id: String,
    pub message: String,
}

impl From<ChatErrorEvent> for FailureEvent {
    fn from(error: ChatErrorEvent) -> Self {
        FailureEvent::new(FailureEventOptions::new(
            "chat",
            error.stage,
            error.code,
            error.message,
            error.retryable,
            error.trace_id,
        ))
    }
}

impl ChatErrorEvent {
    pub fn into_failure_event(
        self,
        conversation_id: Option<String>,
        turn_id: Option<String>,
        character_id: Option<String>,
        context: Option<Value>,
    ) -> FailureEvent {
        let mut event = FailureEvent::from(self);
        event.conversation_id = conversation_id;
        event.turn_id = turn_id;
        event.character_id = character_id;
        event.context = context;
        event
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatErrorEvent, FailureEvent, FailureEventOptions, KokoroError};

    #[test]
    fn failure_event_contains_required_fields() {
        let event = FailureEvent::new(FailureEventOptions::new(
            "chat",
            "llm_stream",
            "CHAT_STREAM_ERROR",
            "failed to stream",
            true,
            "turn-1",
        ));

        assert!(!event.event_id.is_empty());
        assert!(!event.timestamp.is_empty());
        assert_eq!(event.stage, "llm_stream");
        assert_eq!(event.retryable, true);
        assert_eq!(event.trace_id, "turn-1");
    }

    #[test]
    fn kokoro_error_maps_to_failure_event() {
        let event = KokoroError::Validation("invalid cue".to_string()).into_failure_event(
            "play_cue",
            "turn-2",
            None,
            Some("turn-2".to_string()),
            None,
            None,
        );

        assert_eq!(event.code, "VALIDATION_ERROR");
        assert_eq!(event.stage, "play_cue");
        assert_eq!(event.retryable, false);
        assert_eq!(event.trace_id, "turn-2");
    }

    #[test]
    fn chat_error_event_maps_to_failure_event() {
        let chat_error = ChatErrorEvent {
            code: "CHAT_STREAM_ERROR".to_string(),
            stage: "llm_stream".to_string(),
            retryable: true,
            trace_id: "turn-3".to_string(),
            message: "provider timeout".to_string(),
        };

        let event = chat_error.into_failure_event(
            Some("conv-1".to_string()),
            Some("turn-3".to_string()),
            Some("char-1".to_string()),
            None,
        );

        assert_eq!(event.code, "CHAT_STREAM_ERROR");
        assert_eq!(event.stage, "llm_stream");
        assert_eq!(event.retryable, true);
        assert_eq!(event.trace_id, "turn-3");
        assert_eq!(event.conversation_id, Some("conv-1".to_string()));
    }
}

/// 将 KokoroError 序列化为 JSON 字符串，供 Tauri IPC 返回 Result<T, String> 使用。

/// 将 KokoroError 序列化为 JSON 字符串，供 Tauri IPC 返回 Result<T, String> 使用。
///
/// 注意：`.to_string()` 调用 Display，输出人类可读字符串（如 "配置错误: ..."），不是 JSON。
/// 迁移模块时应使用 `.map_err(Into::into)` 或 `String::from(e)` 以获得结构化 JSON 输出。
impl From<KokoroError> for String {
    fn from(e: KokoroError) -> String {
        serde_json::to_string(&e).unwrap_or_else(|_| e.to_string())
    }
}

/// 自动转换 std::io::Error
impl From<std::io::Error> for KokoroError {
    fn from(e: std::io::Error) -> Self {
        KokoroError::Io(e.to_string())
    }
}

/// 自动转换 serde_json::Error
impl From<serde_json::Error> for KokoroError {
    fn from(e: serde_json::Error) -> Self {
        KokoroError::Internal(format!("JSON 序列化错误: {}", e))
    }
}

/// 自动转换 sqlx::Error
impl From<sqlx::Error> for KokoroError {
    fn from(e: sqlx::Error) -> Self {
        KokoroError::Database(e.to_string())
    }
}

/// 自动转换 anyhow::Error
impl From<anyhow::Error> for KokoroError {
    fn from(e: anyhow::Error) -> Self {
        KokoroError::Internal(e.to_string())
    }
}

/// 自动转换 reqwest::Error（HTTP 请求失败）
impl From<reqwest::Error> for KokoroError {
    fn from(e: reqwest::Error) -> Self {
        KokoroError::ExternalService(e.to_string())
    }
}

/// 自动转换 zip::result::ZipError（ZIP 解压失败）
impl From<zip::result::ZipError> for KokoroError {
    fn from(e: zip::result::ZipError) -> Self {
        KokoroError::Io(e.to_string())
    }
}

/// 自动转换 TtsError
impl From<crate::tts::interface::TtsError> for KokoroError {
    fn from(e: crate::tts::interface::TtsError) -> Self {
        KokoroError::Tts(e.to_string())
    }
}

/// 自动转换 SttError
impl From<crate::stt::interface::SttError> for KokoroError {
    fn from(e: crate::stt::interface::SttError) -> Self {
        KokoroError::Stt(e.to_string())
    }
}

/// 自动转换 ImageGenError
impl From<crate::imagegen::interface::ImageGenError> for KokoroError {
    fn from(e: crate::imagegen::interface::ImageGenError) -> Self {
        KokoroError::ExternalService(e.to_string())
    }
}

/// 自动转换 String（用于兼容返回 Result<T, String> 的函数）
impl From<String> for KokoroError {
    fn from(e: String) -> Self {
        KokoroError::Internal(e)
    }
}
