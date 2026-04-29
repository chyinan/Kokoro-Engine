//! LLM Provider trait and async-openai-backed provider implementation.

use async_openai::config::OpenAIConfig;
use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    ChatCompletionMessageToolCallChunk, ChatCompletionRequestMessage, ChatCompletionTool,
    ChatCompletionToolChoiceOption, ChatCompletionTools, CreateChatCompletionRequest,
    CreateChatCompletionRequestArgs, FinishReason, FunctionObjectArgs, ToolChoiceOptions,
};
use async_openai::Client;
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::{channel::mpsc, Stream, StreamExt};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::collections::HashMap;
use std::pin::Pin;

// ── Common Parameters ──────────────────────────────────
#[derive(Debug, Clone, Default)]
pub struct LlmParams {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct LlmToolParam {
    pub name: String,
    pub description: String,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct LlmToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Vec<LlmToolParam>,
}

#[derive(Debug, Clone)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub args: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum LlmStreamEvent {
    Text(String),
    ReasoningContent(String),
    ToolCall(LlmToolCall),
}

#[derive(Debug, Clone)]
pub struct LlmChatMessage {
    pub message: ChatCompletionRequestMessage,
    pub reasoning_content: Option<String>,
}

impl From<ChatCompletionRequestMessage> for LlmChatMessage {
    fn from(message: ChatCompletionRequestMessage) -> Self {
        Self {
            message,
            reasoning_content: None,
        }
    }
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Common interface for LLM providers (OpenAI, Ollama, etc.)
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String>;

    async fn chat_stream(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String>;

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        _tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        let stream = self.chat_stream(messages, options).await?;
        let mapped = stream.map(|item| item.map(LlmStreamEvent::Text));
        Ok(Box::pin(mapped))
    }

    fn supports_native_tools(&self) -> bool {
        false
    }

    async fn chat_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        self.chat(
            messages
                .into_iter()
                .map(|message| message.message)
                .collect(),
            options,
        )
        .await
    }

    async fn chat_stream_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        let stream = self
            .chat_stream(
                messages
                    .into_iter()
                    .map(|message| message.message)
                    .collect(),
                options,
            )
            .await?;
        Ok(Box::pin(stream.map(|item| item.map(LlmStreamEvent::Text))))
    }

    async fn chat_stream_with_tools_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        self.chat_stream_with_tools(
            messages
                .into_iter()
                .map(|message| message.message)
                .collect(),
            options,
            tools,
        )
        .await
    }

    fn id(&self) -> &str;
}

pub fn build_openai_client(api_key: String, base_url: Option<String>) -> Client<OpenAIConfig> {
    let mut config = OpenAIConfig::new().with_api_key(api_key);
    if let Some(base_url) = base_url {
        config = config.with_api_base(base_url);
    }
    Client::with_config(config)
}

pub async fn list_model_ids(client: &Client<OpenAIConfig>) -> Result<Vec<String>, String> {
    let response = client.models().list().await.map_err(format_openai_error)?;
    Ok(response.data.into_iter().map(|model| model.id).collect())
}

pub async fn create_chat(
    client: &Client<OpenAIConfig>,
    model: &str,
    messages: Vec<ChatCompletionRequestMessage>,
    options: Option<LlmParams>,
) -> Result<String, String> {
    let request = build_request(model, messages, options, None, false)?;
    let response = client
        .chat()
        .create(request)
        .await
        .map_err(format_openai_error)?;

    Ok(response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .unwrap_or_default())
}

pub async fn create_chat_stream(
    client: &Client<OpenAIConfig>,
    model: &str,
    messages: Vec<ChatCompletionRequestMessage>,
    options: Option<LlmParams>,
) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
    let request = build_request(model, messages, options, None, true)?;
    let mut stream = client
        .chat()
        .create_stream(request)
        .await
        .map_err(format_openai_error)?;

    let (mut tx, rx) = mpsc::unbounded::<Result<String, String>>();
    tokio::spawn(async move {
        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    for choice in chunk.choices {
                        if let Some(content) = choice.delta.content {
                            if tx.start_send(Ok(content)).is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.start_send(Err(format_openai_error(error)));
                    return;
                }
            }
        }
    });

    Ok(Box::pin(rx))
}

pub async fn create_chat_stream_with_tools(
    client: &Client<OpenAIConfig>,
    model: &str,
    messages: Vec<ChatCompletionRequestMessage>,
    options: Option<LlmParams>,
    tools: Vec<LlmToolDefinition>,
) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
    let request = build_request(model, messages, options, Some(tools), true)?;
    let mut stream = client
        .chat()
        .create_stream(request)
        .await
        .map_err(format_openai_error)?;

    let (mut tx, rx) = mpsc::unbounded::<Result<LlmStreamEvent, String>>();

    tokio::spawn(async move {
        let mut pending_tool_calls: HashMap<u32, PartialToolCall> = HashMap::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    for choice in chunk.choices {
                        if let Some(content) = choice.delta.content.clone() {
                            if tx.start_send(Ok(LlmStreamEvent::Text(content))).is_err() {
                                return;
                            }
                        }

                        if let Some(tool_calls) = choice.delta.tool_calls.clone() {
                            apply_tool_call_chunks(&mut pending_tool_calls, tool_calls);
                        }

                        if matches!(choice.finish_reason, Some(FinishReason::ToolCalls))
                            && emit_pending_tool_calls(&mut tx, &mut pending_tool_calls).is_err()
                        {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.start_send(Err(format_openai_error(error)));
                    return;
                }
            }
        }

        let _ = emit_pending_tool_calls(&mut tx, &mut pending_tool_calls);
    });

    Ok(Box::pin(rx))
}

fn build_request(
    model: &str,
    messages: Vec<ChatCompletionRequestMessage>,
    options: Option<LlmParams>,
    tools: Option<Vec<LlmToolDefinition>>,
    stream: bool,
) -> Result<CreateChatCompletionRequest, String> {
    let opts = options.unwrap_or_default();
    let converted_tools = tools
        .filter(|tools| !tools.is_empty())
        .map(convert_tools)
        .transpose()?;

    let mut builder = CreateChatCompletionRequestArgs::default();
    builder.model(model);
    builder.messages(messages);
    builder.stream(stream);

    if let Some(value) = opts.temperature {
        builder.temperature(value);
    }
    if let Some(value) = opts.max_tokens {
        builder.max_tokens(value);
    }
    if let Some(value) = opts.top_p {
        builder.top_p(value);
    }
    if let Some(value) = opts.frequency_penalty {
        builder.frequency_penalty(value);
    }
    if let Some(value) = opts.presence_penalty {
        builder.presence_penalty(value);
    }
    if let Some(stop) = opts.stop {
        builder.stop(stop);
    }
    if let Some(tools) = converted_tools {
        builder.tools(tools);
        builder.tool_choice(ChatCompletionToolChoiceOption::Mode(
            ToolChoiceOptions::Auto,
        ));
        builder.parallel_tool_calls(false);
    }

    builder.build().map_err(|error| error.to_string())
}

fn build_rich_request_json(
    model: &str,
    messages: Vec<LlmChatMessage>,
    options: Option<LlmParams>,
    tools: Option<Vec<LlmToolDefinition>>,
    stream: bool,
) -> Result<serde_json::Value, String> {
    let reasoning_content = messages
        .iter()
        .map(|message| message.reasoning_content.clone())
        .collect::<Vec<_>>();
    let plain_messages = messages
        .into_iter()
        .map(|message| message.message)
        .collect::<Vec<_>>();
    let request = build_request(model, plain_messages, options, tools, stream)?;
    let mut value = serde_json::to_value(request).map_err(|error| error.to_string())?;

    if let Some(messages_value) = value
        .get_mut("messages")
        .and_then(|value| value.as_array_mut())
    {
        for (message_value, reasoning) in messages_value.iter_mut().zip(reasoning_content) {
            let Some(reasoning) = reasoning.filter(|value| !value.trim().is_empty()) else {
                continue;
            };
            let Some(message_object) = message_value.as_object_mut() else {
                continue;
            };
            if message_object.get("role").and_then(|value| value.as_str()) == Some("assistant") {
                message_object.insert(
                    "reasoning_content".to_string(),
                    serde_json::Value::String(reasoning),
                );
            }
        }
    }

    Ok(value)
}

fn convert_tools(tools: Vec<LlmToolDefinition>) -> Result<Vec<ChatCompletionTools>, String> {
    tools
        .into_iter()
        .map(|tool| {
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();

            for param in &tool.parameters {
                properties.insert(
                    param.name.clone(),
                    serde_json::json!({
                        "type": "string",
                        "description": param.description,
                    }),
                );

                if param.required {
                    required.push(param.name.clone());
                }
            }

            let parameters = serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            });

            let function = FunctionObjectArgs::default()
                .name(tool.name)
                .description(tool.description)
                .parameters(parameters)
                .build()
                .map_err(|error| error.to_string())?;

            Ok(ChatCompletionTools::Function(ChatCompletionTool {
                function,
            }))
        })
        .collect()
}

fn apply_tool_call_chunks(
    pending_tool_calls: &mut HashMap<u32, PartialToolCall>,
    chunks: Vec<ChatCompletionMessageToolCallChunk>,
) {
    for chunk in chunks {
        let entry = pending_tool_calls.entry(chunk.index).or_default();
        if let Some(id) = chunk.id {
            entry.id = id;
        }
        if let Some(function) = chunk.function {
            if let Some(name) = function.name {
                entry.name = name;
            }
            if let Some(arguments) = function.arguments {
                entry.arguments.push_str(&arguments);
            }
        }
    }
}

fn emit_pending_tool_calls(
    tx: &mut mpsc::UnboundedSender<Result<LlmStreamEvent, String>>,
    pending_tool_calls: &mut HashMap<u32, PartialToolCall>,
) -> Result<(), ()> {
    let mut indices = pending_tool_calls.keys().copied().collect::<Vec<_>>();
    indices.sort_unstable();

    for index in indices {
        if let Some(call) = pending_tool_calls.remove(&index) {
            if call.name.trim().is_empty() {
                continue;
            }
            let parsed_args = parse_tool_call_arguments(&call.arguments);
            tx.start_send(Ok(LlmStreamEvent::ToolCall(LlmToolCall {
                id: call.id,
                name: call.name,
                args: parsed_args,
            })))
            .map_err(|_| ())?;
        }
    }

    Ok(())
}

fn parse_tool_call_arguments(raw: &str) -> HashMap<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return HashMap::new();
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Object(map)) => map
            .into_iter()
            .map(|(key, value)| {
                let rendered = match value {
                    serde_json::Value::String(value) => value,
                    other => other.to_string(),
                };
                (key, rendered)
            })
            .collect(),
        _ => HashMap::new(),
    }
}

fn format_openai_error(error: OpenAIError) -> String {
    error.to_string()
}

pub struct OpenAIProvider {
    client: Client<OpenAIConfig>,
    http_client: HttpClient,
    api_key: String,
    base_url: String,
    model: String,
    provider_id: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        let base_url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        Self {
            client: build_openai_client(api_key.clone(), Some(base_url.clone())),
            http_client: HttpClient::new(),
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.unwrap_or_else(|| "gpt-3.5-turbo".to_string()),
            provider_id: "openai".to_string(),
        }
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.provider_id = id;
        self
    }

    async fn post_chat_json(
        &self,
        request: serde_json::Value,
    ) -> Result<reqwest::Response, String> {
        let response = self
            .http_client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("Failed to call OpenAI-compatible chat API: {}", error))?;

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(format!("OpenAI-compatible API error {}: {}", status, text))
    }

    async fn create_chat_rich_inner(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        let request = build_rich_request_json(&self.model, messages, options, None, false)?;
        let response = self.post_chat_json(request).await?;
        let payload = response
            .json::<OpenAICompatChatResponse>()
            .await
            .map_err(|error| {
                format!("Failed to parse OpenAI-compatible response JSON: {}", error)
            })?;

        Ok(payload
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .unwrap_or_default())
    }

    async fn create_chat_stream_rich_inner(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
        tools: Option<Vec<LlmToolDefinition>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        let request = build_rich_request_json(&self.model, messages, options, tools, true)?;
        let response = self.post_chat_json(request).await?;
        let mut stream = response.bytes_stream().eventsource();
        let (mut tx, rx) = mpsc::unbounded::<Result<LlmStreamEvent, String>>();

        tokio::spawn(async move {
            let mut pending_tool_calls: HashMap<u32, PartialToolCall> = HashMap::new();

            while let Some(event_result) = stream.next().await {
                let event = match event_result {
                    Ok(event) => event,
                    Err(error) => {
                        let _ = tx.start_send(Err(format!(
                            "OpenAI-compatible SSE stream error: {}",
                            error
                        )));
                        return;
                    }
                };

                if event.data == "[DONE]" {
                    break;
                }

                let parsed = match serde_json::from_str::<OpenAICompatStreamChunk>(&event.data) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        let _ = tx.start_send(Err(format!(
                            "Failed to parse OpenAI-compatible stream event JSON: {}",
                            error
                        )));
                        return;
                    }
                };

                for choice in parsed.choices {
                    if let Some(reasoning) = choice.delta.reasoning_content {
                        if !reasoning.is_empty()
                            && tx
                                .start_send(Ok(LlmStreamEvent::ReasoningContent(reasoning)))
                                .is_err()
                        {
                            return;
                        }
                    }

                    if let Some(content) = choice.delta.content {
                        if tx.start_send(Ok(LlmStreamEvent::Text(content))).is_err() {
                            return;
                        }
                    }

                    if let Some(tool_calls) = choice.delta.tool_calls {
                        apply_tool_call_chunks(&mut pending_tool_calls, tool_calls);
                    }

                    if matches!(choice.finish_reason, Some(FinishReason::ToolCalls))
                        && emit_pending_tool_calls(&mut tx, &mut pending_tool_calls).is_err()
                    {
                        return;
                    }
                }
            }

            let _ = emit_pending_tool_calls(&mut tx, &mut pending_tool_calls);
        });

        Ok(Box::pin(rx))
    }
}

#[derive(Debug, Deserialize)]
struct OpenAICompatChatResponse {
    choices: Vec<OpenAICompatChatChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAICompatChatChoice {
    message: OpenAICompatChatMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAICompatChatMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAICompatStreamChunk {
    choices: Vec<OpenAICompatStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAICompatStreamChoice {
    delta: OpenAICompatStreamDelta,
    finish_reason: Option<FinishReason>,
}

#[derive(Debug, Deserialize)]
struct OpenAICompatStreamDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<ChatCompletionMessageToolCallChunk>>,
}

#[async_trait]
impl LlmProvider for OpenAIProvider {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        create_chat(&self.client, &self.model, messages, options).await
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
        create_chat_stream(&self.client, &self.model, messages, options).await
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        create_chat_stream_with_tools(&self.client, &self.model, messages, options, tools).await
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn chat_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        self.create_chat_rich_inner(messages, options).await
    }

    async fn chat_stream_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        self.create_chat_stream_rich_inner(messages, options, None)
            .await
    }

    async fn chat_stream_with_tools_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        self.create_chat_stream_rich_inner(messages, options, Some(tools))
            .await
    }

    fn id(&self) -> &str {
        &self.provider_id
    }
}
