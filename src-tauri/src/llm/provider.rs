//! LLM Provider trait and async-openai-backed provider implementation.

use async_trait::async_trait;
use async_openai::config::OpenAIConfig;
use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    ChatCompletionMessageToolCallChunk, ChatCompletionRequestMessage, ChatCompletionTool,
    ChatCompletionToolChoiceOption, ChatCompletionTools, CreateChatCompletionRequest,
    CreateChatCompletionRequestArgs, FinishReason, FunctionObjectArgs, ToolChoiceOptions,
};
use async_openai::Client;
use futures::{Stream, StreamExt, channel::mpsc};
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
    ToolCall(LlmToolCall),
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
    let response = client.chat().create(request).await.map_err(format_openai_error)?;

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
        builder.tool_choice(ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::Auto));
        builder.parallel_tool_calls(false);
    }

    builder.build().map_err(|error| error.to_string())
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

            Ok(ChatCompletionTools::Function(ChatCompletionTool { function }))
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
    model: String,
    provider_id: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            client: build_openai_client(api_key, base_url),
            model: model.unwrap_or_else(|| "gpt-3.5-turbo".to_string()),
            provider_id: "openai".to_string(),
        }
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.provider_id = id;
        self
    }
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

    fn id(&self) -> &str {
        &self.provider_id
    }
}
