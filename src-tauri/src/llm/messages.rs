use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestDeveloperMessageContent, ChatCompletionRequestMessage,
    ChatCompletionRequestMessageContentPartImageArgs,
    ChatCompletionRequestMessageContentPartTextArgs, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
    ChatCompletionRequestUserMessageContentPart, FunctionCall, ImageUrlArgs,
};
use std::collections::HashSet;

use crate::llm::provider::LlmChatMessage;

pub fn is_vision_context_metadata(metadata: Option<&serde_json::Value>) -> bool {
    metadata
        .and_then(|meta| meta.get("type"))
        .and_then(|value| value.as_str())
        == Some("vision_observation")
}

pub fn render_vision_context_user_message(
    content: impl AsRef<str>,
    metadata: Option<&serde_json::Value>,
) -> ChatCompletionRequestMessage {
    let _ = metadata;
    user_text_message(format!("[Screen context]\nSummary: {}", content.as_ref()))
}

pub fn role_text_message(
    role: &str,
    text: impl Into<String>,
) -> Result<ChatCompletionRequestMessage, String> {
    let text = text.into();
    match role {
        "system" | "developer" => Ok(system_message(text)),
        "user" => Ok(user_text_message(text)),
        "assistant" => Ok(assistant_text_message(text)),
        other => Err(format!("Unsupported chat role: {}", other)),
    }
}

pub fn history_message_to_chat_message(
    role: &str,
    content: impl Into<String>,
    metadata: Option<&serde_json::Value>,
) -> Result<ChatCompletionRequestMessage, String> {
    let content = content.into();

    if role == "context" && is_vision_context_metadata(metadata) {
        return Ok(render_vision_context_user_message(content, metadata));
    }

    if role == "tool" {
        let tool_call_id = metadata
            .and_then(|meta| meta.get("tool_call_id"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| "Tool history message missing tool_call_id".to_string())?;
        return Ok(tool_result_message(tool_call_id.to_string(), content));
    }

    if role == "assistant"
        && metadata
            .and_then(|meta| meta.get("type"))
            .and_then(|value| value.as_str())
            == Some("assistant_tool_calls")
    {
        let tool_calls = metadata
            .and_then(|meta| meta.get("tool_calls"))
            .and_then(|value| value.as_array())
            .ok_or_else(|| "Assistant tool-call history missing tool_calls".to_string())?
            .iter()
            .map(|tool_call| {
                let id = tool_call
                    .get("id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "Tool call history missing id".to_string())?;
                let name = tool_call
                    .get("tool_name")
                    .and_then(|value| value.as_str())
                    .or_else(|| tool_call.get("name").and_then(|value| value.as_str()))
                    .ok_or_else(|| "Tool call history missing name".to_string())?;
                let arguments = tool_call
                    .get("arguments")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "Tool call history missing arguments".to_string())?;
                Ok((id.to_string(), name.to_string(), arguments.to_string()))
            })
            .collect::<Result<Vec<_>, String>>()?;

        return Ok(assistant_tool_calls_message(None, tool_calls));
    }

    role_text_message(role, content)
}

pub fn history_message_to_llm_chat_message(
    role: &str,
    content: impl Into<String>,
    metadata: Option<&serde_json::Value>,
) -> Result<LlmChatMessage, String> {
    let message = history_message_to_chat_message(role, content, metadata)?;
    let reasoning_content = if role == "assistant" {
        metadata
            .and_then(|meta| meta.get("reasoning_content"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(ToString::to_string)
    } else {
        None
    };

    let provider_data = if role == "assistant" {
        metadata
            .and_then(|meta| meta.get("provider_data"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(LlmChatMessage {
        message,
        reasoning_content,
        provider_data,
    })
}

pub fn system_message(text: impl Into<String>) -> ChatCompletionRequestMessage {
    let message = ChatCompletionRequestSystemMessageArgs::default()
        .content(text.into())
        .build()
        .expect("system message build should not fail");
    ChatCompletionRequestMessage::System(message)
}

pub fn user_text_message(text: impl Into<String>) -> ChatCompletionRequestMessage {
    let message = ChatCompletionRequestUserMessageArgs::default()
        .content(ChatCompletionRequestUserMessageContent::Text(text.into()))
        .build()
        .expect("user message build should not fail");
    ChatCompletionRequestMessage::User(message)
}

pub fn assistant_text_message(text: impl Into<String>) -> ChatCompletionRequestMessage {
    let message = ChatCompletionRequestAssistantMessageArgs::default()
        .content(ChatCompletionRequestAssistantMessageContent::Text(
            text.into(),
        ))
        .build()
        .expect("assistant message build should not fail");
    ChatCompletionRequestMessage::Assistant(message)
}

pub fn assistant_tool_calls_message(
    text: Option<String>,
    tool_calls: Vec<(String, String, String)>,
) -> ChatCompletionRequestMessage {
    let tool_calls = tool_calls
        .into_iter()
        .map(|(id, name, arguments)| {
            ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                id,
                function: FunctionCall { name, arguments },
            })
        })
        .collect::<Vec<_>>();

    let mut builder = ChatCompletionRequestAssistantMessageArgs::default();
    if let Some(text) = text.filter(|text| !text.is_empty()) {
        builder.content(ChatCompletionRequestAssistantMessageContent::Text(text));
    }
    builder.tool_calls(tool_calls);

    let message = builder
        .build()
        .expect("assistant tool-calls message build should not fail");
    ChatCompletionRequestMessage::Assistant(message)
}

pub fn user_message_with_images(
    text: impl Into<String>,
    image_urls: Vec<String>,
) -> ChatCompletionRequestMessage {
    let mut parts = vec![ChatCompletionRequestUserMessageContentPart::Text(
        ChatCompletionRequestMessageContentPartTextArgs::default()
            .text(text.into())
            .build()
            .expect("user text content part build should not fail"),
    )];

    for url in image_urls {
        let image_url = ImageUrlArgs::default()
            .url(url)
            .build()
            .expect("image url build should not fail");
        let image_part = ChatCompletionRequestMessageContentPartImageArgs::default()
            .image_url(image_url)
            .build()
            .expect("image part build should not fail");
        parts.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(
            image_part,
        ));
    }

    let message = ChatCompletionRequestUserMessageArgs::default()
        .content(ChatCompletionRequestUserMessageContent::Array(parts))
        .build()
        .expect("multimodal user message build should not fail");
    ChatCompletionRequestMessage::User(message)
}

pub fn tool_result_message(
    tool_call_id: impl Into<String>,
    content: impl Into<String>,
) -> ChatCompletionRequestMessage {
    let message = ChatCompletionRequestToolMessageArgs::default()
        .tool_call_id(tool_call_id.into())
        .content(content.into())
        .build()
        .expect("tool result message build should not fail");
    ChatCompletionRequestMessage::Tool(message)
}

fn assistant_tool_call_ids(message: &ChatCompletionRequestMessage) -> Option<Vec<String>> {
    let ChatCompletionRequestMessage::Assistant(message) = message else {
        return None;
    };

    let tool_calls = message.tool_calls.as_ref()?;
    if tool_calls.is_empty() {
        return None;
    }

    let ids = tool_calls
        .iter()
        .map(|tool_call| match tool_call {
            ChatCompletionMessageToolCalls::Function(tool_call) => tool_call.id.clone(),
            ChatCompletionMessageToolCalls::Custom(tool_call) => tool_call.id.clone(),
        })
        .filter(|id| !id.trim().is_empty())
        .collect::<Vec<_>>();

    Some(ids)
}

fn tool_message_call_id(message: &ChatCompletionRequestMessage) -> Option<&str> {
    let ChatCompletionRequestMessage::Tool(message) = message else {
        return None;
    };
    Some(message.tool_call_id.as_str())
}

pub fn sanitize_chat_tool_message_sequence(
    messages: Vec<ChatCompletionRequestMessage>,
) -> Vec<ChatCompletionRequestMessage> {
    sanitize_llm_tool_message_sequence(messages.into_iter().map(LlmChatMessage::from).collect())
        .into_iter()
        .map(|message| message.message)
        .collect()
}

pub fn sanitize_llm_tool_message_sequence(messages: Vec<LlmChatMessage>) -> Vec<LlmChatMessage> {
    let mut sanitized = Vec::with_capacity(messages.len());
    let mut messages = messages.into_iter().peekable();

    while let Some(mut message) = messages.next() {
        let Some(required_ids) = assistant_tool_call_ids(&message.message) else {
            if tool_message_call_id(&message.message).is_none() {
                sanitized.push(message);
            }
            continue;
        };

        if required_ids.is_empty() {
            let text = extract_message_text(&message.message);
            if !text.trim().is_empty() {
                message.message = assistant_text_message(text);
                sanitized.push(message);
            }
            continue;
        }

        let required_ids = required_ids.into_iter().collect::<HashSet<_>>();
        let mut seen_ids = HashSet::new();
        let mut tool_messages = Vec::new();

        while let Some(next) = messages.peek() {
            let Some(tool_call_id) = tool_message_call_id(&next.message) else {
                break;
            };
            let tool_call_id = tool_call_id.to_string();
            let next = messages.next().expect("peeked message should exist");

            if required_ids.contains(&tool_call_id) && seen_ids.insert(tool_call_id) {
                tool_messages.push(next);
            }
        }

        if seen_ids.len() == required_ids.len() {
            sanitized.push(message);
            sanitized.extend(tool_messages);
            continue;
        }

        let text = extract_message_text(&message.message);
        if !text.trim().is_empty() {
            message.message = assistant_text_message(text);
            sanitized.push(message);
        }
    }

    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_tool_call_history_prefers_tool_name_and_keeps_backward_compatibility() {
        let metadata = serde_json::json!({
            "type": "assistant_tool_calls",
            "tool_calls": [
                {
                    "id": "call-1",
                    "tool_id": "mcp__filesystem__read_file",
                    "tool_name": "read_file",
                    "source": "mcp",
                    "server_name": "filesystem",
                    "needs_feedback": true,
                    "arguments": "{\"path\":\"README.md\"}",
                }
            ]
        });

        let message = history_message_to_chat_message("assistant", "", Some(&metadata))
            .expect("assistant tool history should deserialize");

        match message {
            ChatCompletionRequestMessage::Assistant(assistant) => {
                let tool_calls = assistant.tool_calls.expect("tool calls should exist");
                assert_eq!(tool_calls.len(), 1);
                let ChatCompletionMessageToolCalls::Function(tool_call) = &tool_calls[0] else {
                    panic!("expected function tool call");
                };
                assert_eq!(tool_call.id, "call-1");
                assert_eq!(tool_call.function.name, "read_file");
                assert_eq!(tool_call.function.arguments, "{\"path\":\"README.md\"}");
            }
            other => panic!("expected assistant tool-call message, got {other:?}"),
        }
    }

    #[test]
    fn assistant_tool_call_history_falls_back_to_legacy_name_field() {
        let metadata = serde_json::json!({
            "type": "assistant_tool_calls",
            "tool_calls": [
                {
                    "id": "call-legacy",
                    "name": "legacy_lookup",
                    "arguments": "{}",
                }
            ]
        });

        let message = history_message_to_chat_message("assistant", "", Some(&metadata))
            .expect("legacy assistant tool history should deserialize");

        match message {
            ChatCompletionRequestMessage::Assistant(assistant) => {
                let tool_calls = assistant.tool_calls.expect("tool calls should exist");
                let ChatCompletionMessageToolCalls::Function(tool_call) = &tool_calls[0] else {
                    panic!("expected function tool call");
                };
                assert_eq!(tool_call.id, "call-legacy");
                assert_eq!(tool_call.function.name, "legacy_lookup");
                assert_eq!(tool_call.function.arguments, "{}");
            }
            other => panic!("expected assistant tool-call message, got {other:?}"),
        }
    }

    #[test]
    fn assistant_history_keeps_reasoning_content_for_openai_compatible_replay() {
        let metadata = serde_json::json!({
            "turn_id": "turn-1",
            "reasoning_content": "private model reasoning token stream",
        });

        let message =
            history_message_to_llm_chat_message("assistant", "visible answer", Some(&metadata))
                .expect("assistant history should convert");

        assert_eq!(
            message.reasoning_content.as_deref(),
            Some("private model reasoning token stream")
        );
        assert_eq!(extract_message_text(&message.message), "visible answer");
    }

    #[test]
    fn vision_context_history_renders_as_user_message() {
        let metadata = serde_json::json!({
            "type": "vision_observation",
            "captured_at": "2026-05-11T06:00:00Z",
            "source": "auto",
        });

        let message =
            history_message_to_chat_message("context", "VS Code is visible", Some(&metadata))
                .expect("vision context should convert");

        match message {
            ChatCompletionRequestMessage::User(_) => {
                let text = extract_message_text(&message);
                assert!(text.contains("[Screen context]"));
                assert!(!text.contains("Captured at: 2026-05-11T06:00:00Z"));
                assert!(!text.contains("Source: auto"));
                assert!(text.contains("VS Code is visible"));
            }
            other => panic!("expected user-rendered context, got {other:?}"),
        }
    }

    #[test]
    fn sanitize_tool_message_sequence_keeps_complete_native_tool_exchange() {
        let messages = vec![
            LlmChatMessage::from(user_text_message("before")),
            LlmChatMessage::from(assistant_tool_calls_message(
                None,
                vec![(
                    "call-1".to_string(),
                    "lookup".to_string(),
                    "{\"q\":\"x\"}".to_string(),
                )],
            )),
            LlmChatMessage::from(tool_result_message("call-1", "result")),
            LlmChatMessage::from(user_text_message("after")),
        ];

        let sanitized = sanitize_llm_tool_message_sequence(messages);

        assert_eq!(sanitized.len(), 4);
        assert!(matches!(
            sanitized[1].message,
            ChatCompletionRequestMessage::Assistant(_)
        ));
        assert!(matches!(
            sanitized[2].message,
            ChatCompletionRequestMessage::Tool(_)
        ));
    }

    #[test]
    fn sanitize_tool_message_sequence_drops_empty_incomplete_tool_call_history() {
        let messages = vec![
            LlmChatMessage::from(user_text_message("before")),
            LlmChatMessage::from(assistant_tool_calls_message(
                None,
                vec![("call-1".to_string(), "lookup".to_string(), "{}".to_string())],
            )),
            LlmChatMessage::from(user_text_message("after")),
        ];

        let sanitized = sanitize_llm_tool_message_sequence(messages);

        assert_eq!(sanitized.len(), 2);
        assert_eq!(extract_message_text(&sanitized[0].message), "before");
        assert_eq!(extract_message_text(&sanitized[1].message), "after");
    }

    #[test]
    fn sanitize_tool_message_sequence_downgrades_textual_incomplete_tool_call_history() {
        let messages = vec![LlmChatMessage {
            message: assistant_tool_calls_message(
                Some("I will check.".to_string()),
                vec![("call-1".to_string(), "lookup".to_string(), "{}".to_string())],
            ),
            reasoning_content: Some("reasoning".to_string()),
            provider_data: Vec::new(),
        }];

        let sanitized = sanitize_llm_tool_message_sequence(messages);

        assert_eq!(sanitized.len(), 1);
        assert_eq!(extract_message_text(&sanitized[0].message), "I will check.");
        assert_eq!(sanitized[0].reasoning_content.as_deref(), Some("reasoning"));
        match &sanitized[0].message {
            ChatCompletionRequestMessage::Assistant(message) => {
                assert!(message.tool_calls.is_none());
            }
            other => panic!("expected assistant message, got {other:?}"),
        }
    }

    #[test]
    fn sanitize_tool_message_sequence_downgrades_tool_calls_with_empty_ids() {
        let messages = vec![LlmChatMessage::from(assistant_tool_calls_message(
            Some("checking".to_string()),
            vec![("".to_string(), "lookup".to_string(), "{}".to_string())],
        ))];

        let sanitized = sanitize_llm_tool_message_sequence(messages);

        assert_eq!(sanitized.len(), 1);
        assert_eq!(extract_message_text(&sanitized[0].message), "checking");
        match &sanitized[0].message {
            ChatCompletionRequestMessage::Assistant(message) => {
                assert!(message.tool_calls.is_none());
            }
            other => panic!("expected assistant message, got {other:?}"),
        }
    }

    #[test]
    fn sanitize_tool_message_sequence_drops_orphan_tool_messages() {
        let messages = vec![
            LlmChatMessage::from(user_text_message("before")),
            LlmChatMessage::from(tool_result_message("call-1", "orphaned")),
            LlmChatMessage::from(assistant_text_message("after")),
        ];

        let sanitized = sanitize_llm_tool_message_sequence(messages);

        assert_eq!(sanitized.len(), 2);
        assert_eq!(extract_message_text(&sanitized[0].message), "before");
        assert_eq!(extract_message_text(&sanitized[1].message), "after");
    }
}

pub fn is_user_message(message: &ChatCompletionRequestMessage) -> bool {
    matches!(message, ChatCompletionRequestMessage::User(_))
}

pub fn extract_message_text(message: &ChatCompletionRequestMessage) -> String {
    match message {
        ChatCompletionRequestMessage::Developer(message) => match &message.content {
            ChatCompletionRequestDeveloperMessageContent::Text(text) => text.clone(),
            ChatCompletionRequestDeveloperMessageContent::Array(parts) => parts
                .iter()
                .map(|part| match part {
                    async_openai::types::chat::ChatCompletionRequestDeveloperMessageContentPart::Text(
                        text_part,
                    ) => text_part.text.clone(),
                })
                .collect::<Vec<_>>()
                .join(""),
        },
        ChatCompletionRequestMessage::System(message) => match &message.content {
            ChatCompletionRequestSystemMessageContent::Text(text) => text.clone(),
            ChatCompletionRequestSystemMessageContent::Array(parts) => parts
                .iter()
                .map(|part| match part {
                    async_openai::types::chat::ChatCompletionRequestSystemMessageContentPart::Text(
                        text_part,
                    ) => text_part.text.clone(),
                })
                .collect::<Vec<_>>()
                .join(""),
        },
        ChatCompletionRequestMessage::User(message) => match &message.content {
            ChatCompletionRequestUserMessageContent::Text(text) => text.clone(),
            ChatCompletionRequestUserMessageContent::Array(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    ChatCompletionRequestUserMessageContentPart::Text(text_part) => {
                        Some(text_part.text.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        },
        ChatCompletionRequestMessage::Assistant(message) => match &message.content {
            Some(ChatCompletionRequestAssistantMessageContent::Text(text)) => text.clone(),
            _ => String::new(),
        },
        ChatCompletionRequestMessage::Tool(message) => match &message.content {
            async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(text) => {
                text.clone()
            }
            async_openai::types::chat::ChatCompletionRequestToolMessageContent::Array(parts) => {
                parts.iter().map(|part| match part {
                    async_openai::types::chat::ChatCompletionRequestToolMessageContentPart::Text(
                        text_part,
                    ) => text_part.text.clone(),
                }).collect::<Vec<_>>().join("")
            }
        },
        ChatCompletionRequestMessage::Function(message) => {
            message.content.clone().unwrap_or_default()
        }
    }
}

pub fn replace_user_message_with_images(
    message: &mut ChatCompletionRequestMessage,
    text: impl Into<String>,
    image_urls: Vec<String>,
) -> Result<(), String> {
    if !is_user_message(message) {
        return Err("replace_user_message_with_images requires a user message".to_string());
    }
    *message = user_message_with_images(text, image_urls);
    Ok(())
}
