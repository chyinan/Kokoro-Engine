//! OpenAI Responses API provider.
// pattern: Imperative Shell

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::ChatCompletionRequestMessage;
use async_openai::types::responses::{
    CreateResponse, OutputItem, OutputMessageContent, Response, ResponseStreamEvent, Status,
};
use async_openai::Client;
use async_trait::async_trait;
use futures::{channel::mpsc, Stream, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;

use crate::llm::provider::{
    build_openai_client, parse_tool_call_arguments, LlmChatMessage, LlmParams, LlmProvider,
    LlmStreamEvent, LlmToolCall, LlmToolDefinition,
};
use crate::llm::responses_protocol::build_responses_request;

#[derive(Default)]
struct PendingFunctionCall {
    call_id: String,
    name: String,
    arguments: String,
}

pub struct OpenAIResponsesProvider {
    client: Client<OpenAIConfig>,
    model: String,
    provider_id: String,
}

impl OpenAIResponsesProvider {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            client: build_openai_client(api_key, base_url),
            model: model.unwrap_or_else(|| "gpt-4o".to_string()),
            provider_id: "openai_responses".to_string(),
        }
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.provider_id = id;
        self
    }

    fn request(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
        stream: bool,
    ) -> Result<CreateResponse, String> {
        let value = build_responses_request(&self.model, messages, options, tools, stream)?;
        serde_json::from_value(value)
            .map_err(|error| format!("failed to build Responses API request: {}", error))
    }

    async fn create_chat_rich_inner(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        let request = self.request(messages, options, Vec::new(), false)?;
        let response = self
            .client
            .responses()
            .create(request)
            .await
            .map_err(format_openai_error)?;
        extract_response_text(&response)
    }

    async fn create_chat_stream_rich_inner(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        let request = self.request(messages, options, tools, true)?;
        let mut stream: Pin<
            Box<dyn Stream<Item = Result<Value, async_openai::error::OpenAIError>> + Send>,
        > = self
            .client
            .responses()
            .create_stream_byot(&request)
            .await
            .map_err(format_openai_error)?;
        let (mut tx, rx) = mpsc::unbounded::<Result<LlmStreamEvent, String>>();

        tokio::spawn(async move {
            let mut pending_calls: HashMap<u32, PendingFunctionCall> = HashMap::new();
            let mut ready_calls = Vec::new();
            let mut completed = false;

            while let Some(result) = stream.next().await {
                let value = match result {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = tx.start_send(Err(format_openai_error(error)));
                        return;
                    }
                };
                let Some(event_type) = value
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                else {
                    let _ = tx
                        .start_send(Err("Responses API stream event is missing type".to_string()));
                    return;
                };
                if !should_parse_event(&event_type) {
                    tracing::debug!(
                        target: "llm::responses",
                        event_type,
                        "Ignoring unsupported Responses stream event"
                    );
                    continue;
                }
                let event = match serde_json::from_value::<ResponseStreamEvent>(value) {
                    Ok(event) => event,
                    Err(error) => {
                        let _ = tx.start_send(Err(format!(
                            "failed to parse Responses stream event {}: {}",
                            event_type, error
                        )));
                        return;
                    }
                };

                match event {
                    ResponseStreamEvent::ResponseOutputTextDelta(event)
                        if !event.delta.is_empty() =>
                    {
                        let result = tx.start_send(Ok(LlmStreamEvent::Text(event.delta)));
                        if result.is_err() {
                            return;
                        }
                    }
                    ResponseStreamEvent::ResponseRefusalDelta(event) if !event.delta.is_empty() => {
                        let result = tx.start_send(Ok(LlmStreamEvent::Text(event.delta)));
                        if result.is_err() {
                            return;
                        }
                    }
                    ResponseStreamEvent::ResponseReasoningSummaryTextDelta(event)
                        if !event.delta.is_empty() =>
                    {
                        let result =
                            tx.start_send(Ok(LlmStreamEvent::ReasoningContent(event.delta)));
                        if result.is_err() {
                            return;
                        }
                    }
                    ResponseStreamEvent::ResponseReasoningTextDelta(event)
                        if !event.delta.is_empty() =>
                    {
                        let result =
                            tx.start_send(Ok(LlmStreamEvent::ReasoningContent(event.delta)));
                        if result.is_err() {
                            return;
                        }
                    }
                    ResponseStreamEvent::ResponseOutputItemAdded(event) => {
                        if let OutputItem::FunctionCall(call) = event.item {
                            pending_calls.insert(
                                event.output_index,
                                PendingFunctionCall {
                                    call_id: call.call_id,
                                    name: call.name,
                                    arguments: call.arguments,
                                },
                            );
                        }
                    }
                    ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(event) => {
                        pending_calls
                            .entry(event.output_index)
                            .or_default()
                            .arguments
                            .push_str(&event.delta);
                    }
                    ResponseStreamEvent::ResponseFunctionCallArgumentsDone(event) => {
                        let call = pending_calls.entry(event.output_index).or_default();
                        if let Some(name) = event.name {
                            call.name = name;
                        }
                        call.arguments = event.arguments;
                    }
                    ResponseStreamEvent::ResponseOutputItemDone(event) => match event.item {
                        OutputItem::FunctionCall(call) => {
                            let mut pending = pending_calls
                                .remove(&event.output_index)
                                .unwrap_or_default();
                            if pending.call_id.is_empty() {
                                pending.call_id = call.call_id;
                            }
                            if pending.name.is_empty() {
                                pending.name = call.name;
                            }
                            if pending.arguments.is_empty() {
                                pending.arguments = call.arguments;
                            }
                            ready_calls.push(pending);
                        }
                        OutputItem::Reasoning(item) => {
                            let value = serde_json::to_value(OutputItem::Reasoning(item));
                            match value {
                                Ok(value) => {
                                    if tx
                                        .start_send(Ok(LlmStreamEvent::ProviderData(value)))
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                Err(error) => {
                                    let _ = tx.start_send(Err(format!(
                                        "failed to preserve Responses reasoning item: {}",
                                        error
                                    )));
                                    return;
                                }
                            }
                        }
                        _ => {}
                    },
                    ResponseStreamEvent::ResponseFailed(event) => {
                        let message = event
                            .response
                            .error
                            .map(|error| format!("{}: {}", error.code, error.message))
                            .unwrap_or_else(|| "response failed".to_string());
                        let _ = tx.start_send(Err(format!("Responses API error: {}", message)));
                        return;
                    }
                    ResponseStreamEvent::ResponseIncomplete(event) => {
                        let reason = event
                            .response
                            .incomplete_details
                            .map(|details| details.reason)
                            .unwrap_or_else(|| "unknown reason".to_string());
                        let _ = tx.start_send(Err(format!(
                            "Responses API returned an incomplete response: {}",
                            reason
                        )));
                        return;
                    }
                    ResponseStreamEvent::ResponseError(event) => {
                        let code = event.code.unwrap_or_else(|| "unknown_error".to_string());
                        let _ = tx.start_send(Err(format!(
                            "Responses API error {}: {}",
                            code, event.message
                        )));
                        return;
                    }
                    ResponseStreamEvent::ResponseCompleted(_) => {
                        let mut indices = pending_calls.keys().copied().collect::<Vec<_>>();
                        indices.sort_unstable();
                        for index in indices {
                            if let Some(call) = pending_calls.remove(&index) {
                                ready_calls.push(call);
                            }
                        }
                        for call in ready_calls.drain(..) {
                            if emit_tool_call(&mut tx, call).is_err() {
                                return;
                            }
                        }
                        tracing::debug!(target: "llm::responses", "Responses stream completed");
                        completed = true;
                        break;
                    }
                    _ => {}
                }
            }

            if !completed {
                let _ = tx.start_send(Err(
                    "Responses API stream ended before response.completed".to_string()
                ));
            }
        });

        Ok(Box::pin(rx))
    }
}

fn should_parse_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.output_text.delta"
            | "response.refusal.delta"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta"
            | "response.output_item.added"
            | "response.function_call_arguments.delta"
            | "response.function_call_arguments.done"
            | "response.output_item.done"
            | "response.failed"
            | "response.incomplete"
            | "response.completed"
            | "error"
    )
}

fn emit_tool_call(
    tx: &mut mpsc::UnboundedSender<Result<LlmStreamEvent, String>>,
    call: PendingFunctionCall,
) -> Result<(), ()> {
    if call.call_id.trim().is_empty() || call.name.trim().is_empty() {
        return Ok(());
    }
    tx.start_send(Ok(LlmStreamEvent::ToolCall(LlmToolCall {
        id: call.call_id,
        name: call.name,
        args: parse_tool_call_arguments(&call.arguments),
    })))
    .map_err(|_| ())
}

fn extract_response_text(response: &Response) -> Result<String, String> {
    match response.status {
        Status::Completed => {}
        Status::Failed => {
            let message = response
                .error
                .as_ref()
                .map(|error| format!("{}: {}", error.code, error.message))
                .unwrap_or_else(|| "response failed".to_string());
            return Err(format!("Responses API error: {}", message));
        }
        Status::Incomplete => {
            let reason = response
                .incomplete_details
                .as_ref()
                .map(|details| details.reason.as_str())
                .unwrap_or("unknown reason");
            return Err(format!(
                "Responses API returned an incomplete response: {}",
                reason
            ));
        }
        ref status => {
            return Err(format!(
                "Responses API returned unexpected status: {:?}",
                status
            ));
        }
    }

    let mut text = String::new();
    for item in &response.output {
        if let OutputItem::Message(message) = item {
            for content in &message.content {
                match content {
                    OutputMessageContent::OutputText(output) => text.push_str(&output.text),
                    OutputMessageContent::Refusal(refusal) => text.push_str(&refusal.refusal),
                }
            }
        }
    }
    Ok(text)
}

fn format_openai_error(error: async_openai::error::OpenAIError) -> String {
    error.to_string()
}

#[async_trait]
impl LlmProvider for OpenAIResponsesProvider {
    async fn chat(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<String, String> {
        self.create_chat_rich_inner(messages.into_iter().map(Into::into).collect(), options)
            .await
    }

    async fn chat_stream(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>, String> {
        let stream = self
            .create_chat_stream_rich_inner(
                messages.into_iter().map(Into::into).collect(),
                options,
                Vec::new(),
            )
            .await?;
        Ok(Box::pin(stream.filter_map(|event| async move {
            match event {
                Ok(LlmStreamEvent::Text(text)) => Some(Ok(text)),
                Ok(LlmStreamEvent::ReasoningContent(_))
                | Ok(LlmStreamEvent::ToolCall(_))
                | Ok(LlmStreamEvent::ProviderData(_)) => None,
                Err(error) => Some(Err(error)),
            }
        })))
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        self.create_chat_stream_rich_inner(
            messages.into_iter().map(Into::into).collect(),
            options,
            tools,
        )
        .await
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
        self.create_chat_stream_rich_inner(messages, options, Vec::new())
            .await
    }

    async fn chat_stream_with_tools_rich(
        &self,
        messages: Vec<LlmChatMessage>,
        options: Option<LlmParams>,
        tools: Vec<LlmToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmStreamEvent, String>> + Send>>, String> {
        self.create_chat_stream_rich_inner(messages, options, tools)
            .await
    }

    fn id(&self) -> &str {
        &self.provider_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::messages::user_text_message;
    use futures::StreamExt;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn completed_response(output: serde_json::Value) -> serde_json::Value {
        json!({
            "created_at": 1,
            "id": "resp_test",
            "model": "gpt-4o",
            "object": "response",
            "output": output,
            "status": "completed"
        })
    }

    #[test]
    fn non_streaming_failed_and_incomplete_statuses_are_errors() {
        let mut failed = completed_response(json!([]));
        failed["status"] = json!("failed");
        failed["error"] = json!({ "code": "server_error", "message": "failed" });
        let failed: Response = serde_json::from_value(failed).unwrap();

        let mut incomplete = completed_response(json!([]));
        incomplete["status"] = json!("incomplete");
        incomplete["incomplete_details"] = json!({ "reason": "max_output_tokens" });
        let incomplete: Response = serde_json::from_value(incomplete).unwrap();

        assert_eq!(
            extract_response_text(&failed).unwrap_err(),
            "Responses API error: server_error: failed"
        );
        assert_eq!(
            extract_response_text(&incomplete).unwrap_err(),
            "Responses API returned an incomplete response: max_output_tokens"
        );
    }

    #[tokio::test]
    async fn chat_posts_to_responses_with_stateless_history() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(json!({
                "model": "gpt-4o",
                "store": false,
                "stream": false
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(completed_response(json!([{
                    "type": "message",
                    "id": "msg_test",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{
                        "type": "output_text",
                        "annotations": [],
                        "text": "hello from Responses"
                    }]
                }]))),
            )
            .mount(&server)
            .await;

        let provider = OpenAIResponsesProvider::new(
            "test-key".to_string(),
            Some(format!("{}/v1", server.uri())),
            Some("gpt-4o".to_string()),
        );
        let output = provider
            .chat(vec![user_text_message("hello")], None)
            .await
            .expect("Responses request should succeed");

        assert_eq!(output, "hello from Responses");
    }

    #[tokio::test]
    async fn stream_maps_text_and_function_call_events() {
        let server = MockServer::start().await;
        let function_call = json!({
            "type": "function_call",
            "arguments": "{\"query\":\"kokoro\"}",
            "call_id": "call-1",
            "name": "lookup",
            "id": "fc-1",
            "status": "completed"
        });
        let body = format!(
            "event: response.output_text.delta\ndata: {}\n\n\
             event: response.output_item.done\ndata: {}\n\n\
             event: response.completed\ndata: {}\n\n",
            json!({
                "type": "response.output_text.delta",
                "sequence_number": 1,
                "item_id": "msg-1",
                "output_index": 0,
                "content_index": 0,
                "delta": "hello"
            }),
            json!({
                "type": "response.output_item.done",
                "sequence_number": 2,
                "output_index": 1,
                "item": function_call
            }),
            json!({
                "type": "response.completed",
                "sequence_number": 3,
                "response": completed_response(json!([]))
            })
        );

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;

        let provider = OpenAIResponsesProvider::new(
            "test-key".to_string(),
            Some(format!("{}/v1", server.uri())),
            Some("gpt-4o".to_string()),
        );
        let stream = provider
            .chat_stream_with_tools(
                vec![user_text_message("hello")],
                None,
                vec![LlmToolDefinition {
                    name: "lookup".to_string(),
                    description: "Look up a value".to_string(),
                    parameters: Vec::new(),
                }],
            )
            .await
            .expect("Responses stream should start");
        let events = stream.collect::<Vec<_>>().await;

        assert!(matches!(
            events.first(),
            Some(Ok(LlmStreamEvent::Text(text))) if text == "hello"
        ));
        assert!(matches!(
            events.get(1),
            Some(Ok(LlmStreamEvent::ToolCall(call)))
                if call.id == "call-1"
                    && call.name == "lookup"
                    && call.args.get("query").map(String::as_str) == Some("kokoro")
        ));
    }
}
