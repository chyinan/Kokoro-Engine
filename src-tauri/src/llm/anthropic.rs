//! Anthropic provider backed by the native Messages API.
//!
//! Unlike OpenAI-compatible providers, Anthropic uses a different message
//! schema, SSE event format, and tool-calling protocol. This adapter converts
//! the app's internal OpenAI-style message representation into Anthropic's
//! request/response format.

use async_openai::types::chat::{
    ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContent,
    ChatCompletionRequestUserMessageContentPart,
};
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{channel::mpsc, Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;

use crate::llm::messages::extract_message_text;
use crate::llm::provider::{
    LlmParams, LlmProvider, LlmStreamEvent, LlmToolCall, LlmToolDefinition,
};

const DEFAULT_ANTHROPIC_SERVER_BASE_URL: &str = "https://api.anthropic.com";
const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicModelInfo {
    pub id: String,
}

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
    provider_id: String,
    server_base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: model.unwrap_or_else(|| "claude-sonnet-4-20250514".to_string()),
            provider_id: "anthropic".to_string(),
            server_base_url: normalize_anthropic_server_base_url(
                base_url
                    .as_deref()
                    .unwrap_or(DEFAULT_ANTHROPIC_SERVER_BASE_URL),
            ),
        }
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.provider_id = id;
        self
    }

    pub async fn list_models(
        base_url: &str,
        api_key: &str,
    ) -> Result<Vec<AnthropicModelInfo>, String> {
        let client = Client::new();
        let response = client
            .get(format!(
                "{}/v1/models",
                normalize_anthropic_server_base_url(base_url)
            ))
            .header("x-api-key", api_key)
            .header("anthropic-version", DEFAULT_ANTHROPIC_VERSION)
            .send()
            .await
            .map_err(|error| format!("Failed to query Anthropic models: {}", error))?;

        let response = ensure_success(response).await?;
        let mut payload: AnthropicModelListResponse = response
            .json()
            .await
            .map_err(|error| format!("Failed to parse Anthropic model list: {}", error))?;
        payload.data.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(payload.data)
    }

    async fn create_message(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        tools: Option<Vec<LlmToolDefinition>>,
    ) -> Result<AnthropicMessageResponse, String> {
        let request = build_anthropic_request(&self.model, messages, options, tools, false)?;
        let response = self
            .request_builder(self.messages_endpoint())
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("Failed to call Anthropic Messages API: {}", error))?;

        let response = ensure_success(response).await?;
        response
            .json::<AnthropicMessageResponse>()
            .await
            .map_err(|error| format!("Failed to parse Anthropic response JSON: {}", error))
    }

    async fn create_message_stream(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        tools: Option<Vec<LlmToolDefinition>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        let request = build_anthropic_request(&self.model, messages, options, tools, true)?;
        let response = self
            .request_builder(self.messages_endpoint())
            .header("Accept", "text/event-stream")
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("Failed to start Anthropic stream: {}", error))?;

        let response = ensure_success(response).await?;
        let mut stream = response.bytes_stream().eventsource();
        let (mut tx, rx) = mpsc::unbounded::<Result<LlmStreamEvent, String>>();

        tokio::spawn(async move {
            let mut pending_tool_calls: HashMap<usize, PendingAnthropicToolCall> = HashMap::new();

            while let Some(event_result) = stream.next().await {
                let event = match event_result {
                    Ok(event) => event,
                    Err(error) => {
                        let _ =
                            tx.start_send(Err(format!("Anthropic SSE stream error: {}", error)));
                        return;
                    }
                };

                if event.data == "[DONE]" {
                    break;
                }

                let parsed = match serde_json::from_str::<AnthropicStreamEvent>(&event.data) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        let _ = tx.start_send(Err(format!(
                            "Failed to parse Anthropic stream event JSON: {}",
                            error
                        )));
                        return;
                    }
                };

                match parsed.event_type.as_str() {
                    "content_block_start" => {
                        if let Some(index) = parsed.index {
                            if let Some(block) = parsed.content_block {
                                if block.block_type == "text" {
                                    if let Some(text) = block.text.filter(|text| !text.is_empty()) {
                                        if tx.start_send(Ok(LlmStreamEvent::Text(text))).is_err() {
                                            return;
                                        }
                                    }
                                } else if block.block_type == "tool_use" {
                                    pending_tool_calls.insert(
                                        index,
                                        PendingAnthropicToolCall {
                                            id: block.id.unwrap_or_default(),
                                            name: block.name.unwrap_or_default(),
                                            input_json: String::new(),
                                            initial_input: block.input,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    "content_block_delta" => {
                        if let Some(delta) = parsed.delta {
                            match delta.delta_type.as_deref() {
                                Some("text_delta") => {
                                    if let Some(text) = delta.text.filter(|text| !text.is_empty()) {
                                        if tx.start_send(Ok(LlmStreamEvent::Text(text))).is_err() {
                                            return;
                                        }
                                    }
                                }
                                Some("input_json_delta") => {
                                    if let Some(index) = parsed.index {
                                        if let Some(call) = pending_tool_calls.get_mut(&index) {
                                            if let Some(partial_json) = delta.partial_json {
                                                call.input_json.push_str(&partial_json);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "content_block_stop" => {
                        if let Some(index) = parsed.index {
                            if let Some(call) = pending_tool_calls.remove(&index) {
                                match finalize_tool_call(call) {
                                    Ok(tool_call) => {
                                        if tx
                                            .start_send(Ok(LlmStreamEvent::ToolCall(tool_call)))
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                    Err(error) => {
                                        let _ = tx.start_send(Err(error));
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    "error" => {
                        let message =
                            parsed.error.map(|error| error.message).unwrap_or_else(|| {
                                "Anthropic stream returned an error event".to_string()
                            });
                        let _ = tx.start_send(Err(message));
                        return;
                    }
                    _ => {}
                }
            }

            let _ = emit_pending_tool_calls(&mut tx, pending_tool_calls);
        });

        Ok(Box::pin(rx))
    }

    fn request_builder(&self, url: String) -> reqwest::RequestBuilder {
        self.client
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", DEFAULT_ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
    }

    fn messages_endpoint(&self) -> String {
        format!("{}/v1/messages", self.server_base_url)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        let response = self.create_message(messages, options, None).await?;
        Ok(extract_text_blocks(&response.content))
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
        let stream = self.create_message_stream(messages, options, None).await?;
        let mapped = stream.filter_map(|item| async move {
            match item {
                Ok(LlmStreamEvent::Text(text)) => Some(Ok(text)),
                Ok(LlmStreamEvent::ReasoningContent(_)) => None,
                Ok(LlmStreamEvent::ToolCall(_)) => None,
                Err(error) => Some(Err(error)),
            }
        });
        Ok(Box::pin(mapped))
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        self.create_message_stream(messages, options, Some(tools))
            .await
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    fn id(&self) -> &str {
        &self.provider_id
    }
}

#[derive(Debug)]
struct PendingAnthropicToolCall {
    id: String,
    name: String,
    input_json: String,
    initial_input: Option<Value>,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        text: String,
    },
    Image {
        source: AnthropicImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageResponse {
    content: Vec<AnthropicResponseContentBlock>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponseContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelListResponse {
    data: Vec<AnthropicModelInfo>,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    index: Option<usize>,
    delta: Option<AnthropicStreamDelta>,
    content_block: Option<AnthropicResponseContentBlock>,
    error: Option<AnthropicErrorInfo>,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamDelta {
    #[serde(rename = "type")]
    delta_type: Option<String>,
    text: Option<String>,
    partial_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorEnvelope {
    error: Option<AnthropicErrorInfo>,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorInfo {
    #[serde(rename = "type")]
    _error_type: Option<String>,
    message: String,
}

fn build_anthropic_request(
    model: &str,
    messages: Vec<ChatCompletionRequestMessage>,
    options: Option<LlmParams>,
    tools: Option<Vec<LlmToolDefinition>>,
    stream: bool,
) -> Result<AnthropicRequest, String> {
    let converted = convert_messages(messages)?;
    let options = options.unwrap_or_default();
    let stop_sequences = options.stop.filter(|values| !values.is_empty());
    let tools = tools
        .filter(|tools| !tools.is_empty())
        .map(convert_tools)
        .transpose()?;

    Ok(AnthropicRequest {
        model: model.to_string(),
        max_tokens: options.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        messages: converted.messages,
        system: converted.system,
        temperature: options.temperature,
        // Anthropic warns that some models reject simultaneous temperature + top_p.
        top_p: if options.temperature.is_some() {
            None
        } else {
            options.top_p
        },
        stop_sequences,
        stream: stream.then_some(true),
        tools,
    })
}

struct ConvertedAnthropicMessages {
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
}

fn convert_messages(
    messages: Vec<ChatCompletionRequestMessage>,
) -> Result<ConvertedAnthropicMessages, String> {
    let mut system_parts = Vec::new();
    let mut converted = Vec::new();
    let mut pending_tool_results = Vec::new();

    for message in messages {
        match message {
            ChatCompletionRequestMessage::Developer(message) => {
                let text = extract_message_text(&ChatCompletionRequestMessage::Developer(message));
                if !text.trim().is_empty() {
                    system_parts.push(text);
                }
            }
            ChatCompletionRequestMessage::System(message) => {
                let text = extract_message_text(&ChatCompletionRequestMessage::System(message));
                if !text.trim().is_empty() {
                    system_parts.push(text);
                }
            }
            ChatCompletionRequestMessage::User(message) => {
                flush_tool_results(&mut converted, &mut pending_tool_results);
                let content = convert_user_content(&message.content)?;
                if !content.is_empty() {
                    converted.push(AnthropicMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
            }
            ChatCompletionRequestMessage::Assistant(message) => {
                flush_tool_results(&mut converted, &mut pending_tool_results);
                let content = convert_assistant_content(&message)?;
                if !content.is_empty() {
                    converted.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                }
            }
            ChatCompletionRequestMessage::Tool(message) => {
                let text =
                    extract_message_text(&ChatCompletionRequestMessage::Tool(message.clone()));
                pending_tool_results.push(AnthropicContentBlock::ToolResult {
                    tool_use_id: message.tool_call_id,
                    content: text,
                });
            }
            ChatCompletionRequestMessage::Function(message) => {
                flush_tool_results(&mut converted, &mut pending_tool_results);
                let mut text = String::new();
                if !message.name.is_empty() {
                    text.push_str("Function ");
                    text.push_str(&message.name);
                    text.push_str(": ");
                }
                text.push_str(&message.content.unwrap_or_default());
                if !text.trim().is_empty() {
                    converted.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: vec![AnthropicContentBlock::Text { text }],
                    });
                }
            }
        }
    }

    flush_tool_results(&mut converted, &mut pending_tool_results);

    Ok(ConvertedAnthropicMessages {
        system: if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        },
        messages: converted,
    })
}

fn flush_tool_results(
    converted: &mut Vec<AnthropicMessage>,
    pending_tool_results: &mut Vec<AnthropicContentBlock>,
) {
    if pending_tool_results.is_empty() {
        return;
    }

    converted.push(AnthropicMessage {
        role: "user".to_string(),
        content: std::mem::take(pending_tool_results),
    });
}

fn convert_user_content(
    content: &ChatCompletionRequestUserMessageContent,
) -> Result<Vec<AnthropicContentBlock>, String> {
    match content {
        ChatCompletionRequestUserMessageContent::Text(text) => Ok(if text.trim().is_empty() {
            vec![]
        } else {
            vec![AnthropicContentBlock::Text { text: text.clone() }]
        }),
        ChatCompletionRequestUserMessageContent::Array(parts) => parts
            .iter()
            .map(convert_user_part)
            .collect::<Result<Vec<_>, _>>()
            .map(|parts| {
                parts
                    .into_iter()
                    .flatten()
                    .collect::<Vec<AnthropicContentBlock>>()
            }),
    }
}

fn convert_user_part(
    part: &ChatCompletionRequestUserMessageContentPart,
) -> Result<Vec<AnthropicContentBlock>, String> {
    match part {
        ChatCompletionRequestUserMessageContentPart::Text(text_part) => {
            if text_part.text.trim().is_empty() {
                Ok(vec![])
            } else {
                Ok(vec![AnthropicContentBlock::Text {
                    text: text_part.text.clone(),
                }])
            }
        }
        ChatCompletionRequestUserMessageContentPart::ImageUrl(image_part) => {
            Ok(vec![AnthropicContentBlock::Image {
                source: convert_image_url(&image_part.image_url.url)?,
            }])
        }
        other => Err(format!(
            "Anthropic provider does not support this user content part: {:?}",
            other
        )),
    }
}

fn convert_assistant_content(
    message: &async_openai::types::chat::ChatCompletionRequestAssistantMessage,
) -> Result<Vec<AnthropicContentBlock>, String> {
    let mut content = Vec::new();

    if let Some(text) = extract_assistant_text(message) {
        content.push(AnthropicContentBlock::Text { text });
    }

    if let Some(tool_calls) = &message.tool_calls {
        for tool_call in tool_calls {
            match tool_call {
                ChatCompletionMessageToolCalls::Function(tool_call) => {
                    content.push(AnthropicContentBlock::ToolUse {
                        id: tool_call.id.clone(),
                        name: tool_call.function.name.clone(),
                        input: parse_tool_call_json(&tool_call.function.arguments)?,
                    });
                }
                ChatCompletionMessageToolCalls::Custom(tool_call) => {
                    return Err(format!(
                        "Anthropic provider does not support custom tool call payloads: {}",
                        tool_call.custom_tool.name
                    ));
                }
            }
        }
    }

    Ok(content)
}

fn extract_assistant_text(
    message: &async_openai::types::chat::ChatCompletionRequestAssistantMessage,
) -> Option<String> {
    match &message.content {
        Some(ChatCompletionRequestAssistantMessageContent::Text(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(ChatCompletionRequestAssistantMessageContent::Array(parts)) => {
            let combined = parts
                .iter()
                .filter_map(|part| match part {
                    async_openai::types::chat::ChatCompletionRequestAssistantMessageContentPart::Text(
                        text_part,
                    ) => Some(text_part.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            if combined.trim().is_empty() {
                None
            } else {
                Some(combined)
            }
        }
        None => None,
    }
}

fn convert_image_url(url: &str) -> Result<AnthropicImageSource, String> {
    if let Some(parsed) = parse_data_url(url) {
        return Ok(AnthropicImageSource::Base64 {
            media_type: parsed.media_type,
            data: parsed.data,
        });
    }

    Ok(AnthropicImageSource::Url {
        url: url.to_string(),
    })
}

struct ParsedDataUrl {
    media_type: String,
    data: String,
}

fn parse_data_url(url: &str) -> Option<ParsedDataUrl> {
    let rest = url.strip_prefix("data:")?;
    let (metadata, data) = rest.split_once(",")?;
    let metadata = metadata.trim();
    let media_type = metadata.strip_suffix(";base64")?;
    if media_type.is_empty() || data.trim().is_empty() {
        return None;
    }

    Some(ParsedDataUrl {
        media_type: media_type.to_string(),
        data: data.to_string(),
    })
}

fn convert_tools(tools: Vec<LlmToolDefinition>) -> Result<Vec<AnthropicTool>, String> {
    tools
        .into_iter()
        .map(|tool| {
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();

            for param in &tool.parameters {
                properties.insert(
                    param.name.clone(),
                    json!({
                        "type": "string",
                        "description": param.description,
                    }),
                );
                if param.required {
                    required.push(param.name.clone());
                }
            }

            Ok(AnthropicTool {
                name: tool.name,
                description: tool.description,
                input_schema: json!({
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                }),
            })
        })
        .collect()
}

fn parse_tool_call_json(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }

    serde_json::from_str(trimmed)
        .map_err(|error| format!("Failed to parse tool call JSON arguments: {}", error))
}

fn finalize_tool_call(pending: PendingAnthropicToolCall) -> Result<LlmToolCall, String> {
    let input = if pending.input_json.trim().is_empty() {
        pending.initial_input.unwrap_or_else(|| json!({}))
    } else {
        serde_json::from_str::<Value>(&pending.input_json).map_err(|error| {
            format!(
                "Failed to parse Anthropic streamed tool input JSON for '{}': {}",
                pending.name, error
            )
        })?
    };

    Ok(LlmToolCall {
        id: pending.id,
        name: pending.name,
        args: anthropic_input_to_args(&input),
    })
}

fn emit_pending_tool_calls(
    tx: &mut mpsc::UnboundedSender<Result<LlmStreamEvent, String>>,
    pending_tool_calls: HashMap<usize, PendingAnthropicToolCall>,
) -> Result<(), ()> {
    let mut indexed = pending_tool_calls.into_iter().collect::<Vec<_>>();
    indexed.sort_by_key(|(index, _)| *index);

    for (_, pending) in indexed {
        let tool_call = finalize_tool_call(pending).map_err(|_| ())?;
        tx.start_send(Ok(LlmStreamEvent::ToolCall(tool_call)))
            .map_err(|_| ())?;
    }

    Ok(())
}

fn anthropic_input_to_args(input: &Value) -> HashMap<String, String> {
    match input {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                let rendered = match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                (key.clone(), rendered)
            })
            .collect(),
        _ => HashMap::new(),
    }
}

fn extract_text_blocks(blocks: &[AnthropicResponseContentBlock]) -> String {
    blocks
        .iter()
        .filter(|block| block.block_type == "text")
        .filter_map(|block| block.text.as_deref())
        .collect::<Vec<_>>()
        .join("")
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if let Ok(parsed) = serde_json::from_str::<AnthropicErrorEnvelope>(&body) {
        if let Some(error) = parsed.error {
            return Err(format!(
                "Anthropic API request failed (HTTP {}): {}",
                status, error.message
            ));
        }
    }

    let body = body.trim();
    if body.is_empty() {
        Err(format!("Anthropic API request failed with HTTP {}", status))
    } else {
        Err(format!(
            "Anthropic API request failed (HTTP {}): {}",
            status, body
        ))
    }
}

fn normalize_anthropic_server_base_url(base_url: &str) -> String {
    let mut normalized = base_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        normalized = DEFAULT_ANTHROPIC_SERVER_BASE_URL.to_string();
    }

    for suffix in ["/v1/messages", "/messages", "/v1"] {
        if let Some(stripped) = normalized.strip_suffix(suffix) {
            normalized = stripped.to_string();
            break;
        }
    }

    normalized.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        anthropic_input_to_args, build_anthropic_request, normalize_anthropic_server_base_url,
        AnthropicProvider,
    };
    use crate::llm::messages::{
        assistant_tool_calls_message, system_message, tool_result_message, user_message_with_images,
    };
    use crate::llm::provider::{LlmParams, LlmToolDefinition, LlmToolParam};
    use async_openai::types::chat::ChatCompletionRequestMessage;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn normalizes_anthropic_urls() {
        assert_eq!(
            normalize_anthropic_server_base_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com"
        );
        assert_eq!(
            normalize_anthropic_server_base_url("https://example.com/v1/messages"),
            "https://example.com"
        );
        assert_eq!(
            normalize_anthropic_server_base_url("https://example.com/messages"),
            "https://example.com"
        );
    }

    #[test]
    fn request_conversion_maps_system_images_tools_and_results() {
        let messages = vec![
            system_message("You are helpful."),
            user_message_with_images(
                "Describe this image",
                vec!["data:image/png;base64,QUJD".to_string()],
            ),
            assistant_tool_calls_message(
                Some("Let me check.".to_string()),
                vec![(
                    "toolu_1".to_string(),
                    "lookup_weather".to_string(),
                    "{\"city\":\"Shanghai\"}".to_string(),
                )],
            ),
            tool_result_message("toolu_1", "Sunny"),
        ];

        let request = build_anthropic_request(
            "claude-sonnet-4-20250514",
            messages,
            Some(LlmParams {
                temperature: Some(0.2),
                top_p: Some(0.9),
                max_tokens: Some(512),
                ..Default::default()
            }),
            Some(vec![LlmToolDefinition {
                name: "lookup_weather".to_string(),
                description: "Lookup weather".to_string(),
                parameters: vec![LlmToolParam {
                    name: "city".to_string(),
                    description: "Target city".to_string(),
                    required: true,
                }],
            }]),
            true,
        )
        .expect("request should build");

        assert_eq!(request.system.as_deref(), Some("You are helpful."));
        assert_eq!(request.messages.len(), 3);
        assert_eq!(request.messages[0].role, "user");
        assert_eq!(request.messages[1].role, "assistant");
        assert_eq!(request.messages[2].role, "user");
        assert_eq!(request.max_tokens, 512);
        assert_eq!(request.temperature, Some(0.2));
        assert_eq!(request.top_p, None);
        assert_eq!(request.stream, Some(true));

        let serialized = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(
            serialized["messages"][0]["content"][1]["type"],
            json!("image")
        );
        assert_eq!(
            serialized["messages"][0]["content"][1]["source"]["type"],
            json!("base64")
        );
        assert_eq!(
            serialized["messages"][1]["content"][1]["type"],
            json!("tool_use")
        );
        assert_eq!(
            serialized["messages"][2]["content"][0]["type"],
            json!("tool_result")
        );
        assert_eq!(serialized["tools"][0]["name"], json!("lookup_weather"));
    }

    #[test]
    fn anthropic_tool_input_preserves_non_string_values() {
        let args = anthropic_input_to_args(&json!({
            "query": "kokoro",
            "limit": 3,
            "filters": ["a", "b"],
        }));

        assert_eq!(args.get("query").map(String::as_str), Some("kokoro"));
        assert_eq!(args.get("limit").map(String::as_str), Some("3"));
        assert_eq!(
            args.get("filters").map(String::as_str),
            Some("[\"a\",\"b\"]")
        );
    }

    #[tokio::test]
    async fn list_models_uses_required_headers() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("x-api-key", "test-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    { "id": "claude-sonnet-4-20250514" },
                    { "id": "claude-haiku-3-5-20241022" }
                ]
            })))
            .mount(&server)
            .await;

        let models = AnthropicProvider::list_models(&server.uri(), "test-key")
            .await
            .expect("model listing should succeed");

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-haiku-3-5-20241022");
        assert_eq!(models[1].id, "claude-sonnet-4-20250514");
    }

    #[test]
    fn request_conversion_accepts_explicit_openai_history_type() {
        let request = build_anthropic_request(
            "claude-sonnet-4-20250514",
            vec![ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessageArgs::default()
                    .content("hello")
                    .build()
                    .expect("user message should build"),
            )],
            None,
            None,
            false,
        )
        .expect("request should build");

        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "user");
    }
}
