use serde::{Deserialize, Serialize};
use thiserror::Error;

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
