use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestDeveloperMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestMessageContentPartImageArgs,
    ChatCompletionRequestMessageContentPartTextArgs, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestSystemMessageContent,
    ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionRequestUserMessageContent, ChatCompletionRequestUserMessageContentPart,
    ImageUrlArgs, FunctionCall,
};

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
                    .get("name")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "Tool call history missing name".to_string())?;
                let arguments = tool_call
                    .get("arguments")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| "Tool call history missing arguments".to_string())?;
                Ok((id.to_string(), name.to_string(), arguments.to_string()))
            })
            .collect::<Result<Vec<_>, String>>()?;

        return Ok(assistant_tool_calls_message(
            None,
            tool_calls,
        ));
    }

    role_text_message(role, content)
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
        .content(ChatCompletionRequestAssistantMessageContent::Text(text.into()))
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
        parts.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(image_part));
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
