use std::future::Future;

use tokio::sync::watch;

use crate::error::KokoroError;
use crate::llm::provider::{LlmChatMessage, LlmProvider, LlmStreamEvent, LlmToolDefinition};

use super::application::{
    poll_stream_with_cancellation_and_timeout, stream_chunk_idle_timeout,
    stream_first_chunk_timeout, wait_for_cancel_event, StreamPollResult,
    TURN_CANCELLED_BY_USER_MESSAGE,
};
use super::tags::{find_safe_emit_boundary, ToolCall};

pub(super) struct ModelFailure {
    pub stage: &'static str,
    pub message: String,
    pub timed_out: bool,
}

#[derive(Default)]
pub(super) struct RoundOutput {
    pub text: String,
    pub reasoning: String,
    pub provider_data: Vec<serde_json::Value>,
    pub native_tool_calls: Vec<ToolCall>,
    pub remainder: String,
    pub failure: Option<ModelFailure>,
}

fn cancelled() -> KokoroError {
    KokoroError::Chat(TURN_CANCELLED_BY_USER_MESSAGE.into())
}

/// Read one provider round. The caller owns persistence, cleanup and tool execution.
pub(super) async fn run<Fut>(
    provider: &dyn LlmProvider,
    messages: Vec<LlmChatMessage>,
    tools: Option<&[LlmToolDefinition]>,
    cancellation: &mut Option<watch::Receiver<bool>>,
    mut publish_delta: impl FnMut(String) -> Fut,
) -> Result<RoundOutput, KokoroError>
where
    Fut: Future<Output = Result<(), KokoroError>>,
{
    let first_chunk_timeout = stream_first_chunk_timeout();
    let idle_timeout = stream_chunk_idle_timeout();
    let creation = async {
        match tools {
            Some(tools) => {
                provider
                    .chat_stream_with_tools_rich(messages, None, tools.to_vec())
                    .await
            }
            None => provider.chat_stream_rich(messages, None).await,
        }
    };
    let connection = tokio::select! {
        biased;
        _ = wait_for_cancel_event(cancellation) => return Err(cancelled()),
        result = tokio::time::timeout(first_chunk_timeout, creation) => result,
    };

    let mut result = RoundOutput::default();
    let mut stream = match connection {
        Ok(Ok(stream)) => stream,
        Ok(Err(message)) => {
            result.failure = Some(ModelFailure {
                stage: "stream_connect",
                message,
                timed_out: false,
            });
            return Ok(result);
        }
        Err(_) => {
            result.failure = Some(ModelFailure {
                stage: "stream_connect",
                message: format!(
                    "LLM provider connection timed out after {}s",
                    first_chunk_timeout.as_secs()
                ),
                timed_out: true,
            });
            return Ok(result);
        }
    };

    let mut received_chunk = false;
    loop {
        let timeout = if received_chunk {
            idle_timeout
        } else {
            first_chunk_timeout
        };
        match poll_stream_with_cancellation_and_timeout(&mut stream, cancellation, timeout).await {
            StreamPollResult::Cancelled => return Err(cancelled()),
            StreamPollResult::StreamEnded => break,
            StreamPollResult::TimedOut(_) => {
                let kind = if received_chunk {
                    "stream chunk idle"
                } else {
                    "initial response"
                };
                result.failure = Some(ModelFailure {
                    stage: "stream_timeout",
                    message: format!("LLM {kind} timeout after {}s", timeout.as_secs()),
                    timed_out: true,
                });
                break;
            }
            StreamPollResult::Item(Err(message)) => {
                result.failure = Some(ModelFailure {
                    stage: "stream_receive",
                    message,
                    timed_out: false,
                });
                break;
            }
            StreamPollResult::Item(Ok(event)) => {
                received_chunk = true;
                match event {
                    LlmStreamEvent::Text(content) => {
                        result.text.push_str(&content);
                        result.remainder.push_str(&content);
                        let safe = find_safe_emit_boundary(&result.remainder);
                        if safe > 0 {
                            let remainder = result.remainder.split_off(safe);
                            let delta = std::mem::replace(&mut result.remainder, remainder);
                            publish_delta(delta).await?;
                        }
                    }
                    LlmStreamEvent::ReasoningContent(content) => {
                        result.reasoning.push_str(&content)
                    }
                    LlmStreamEvent::ProviderData(value) => result.provider_data.push(value),
                    LlmStreamEvent::ToolCall(call) => result.native_tool_calls.push(ToolCall {
                        tool_call_id: Some(call.id),
                        name: call.name,
                        args: call.args,
                    }),
                }
            }
        }
    }
    if result.failure.is_some() {
        result.native_tool_calls.clear();
    }
    Ok(result)
}
