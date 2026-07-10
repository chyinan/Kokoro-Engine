//! Pure request translation for the OpenAI Responses API.
// pattern: Functional Core

use serde_json::{json, Map, Value};

use crate::llm::messages::sanitize_llm_tool_message_sequence;
use crate::llm::provider::{LlmChatMessage, LlmParams, LlmToolDefinition};

pub(crate) fn build_responses_request(
    model: &str,
    messages: Vec<LlmChatMessage>,
    options: Option<LlmParams>,
    tools: Vec<LlmToolDefinition>,
    stream: bool,
) -> Result<Value, String> {
    let options = options.unwrap_or_default();
    validate_options(&options)?;

    let mut request = Map::new();
    request.insert("model".to_string(), Value::String(model.to_string()));
    request.insert(
        "input".to_string(),
        Value::Array(convert_messages(messages)?),
    );
    request.insert("store".to_string(), Value::Bool(false));
    request.insert("stream".to_string(), Value::Bool(stream));

    if let Some(value) = options.temperature {
        request.insert("temperature".to_string(), json!(value));
    }
    if let Some(value) = options.max_tokens {
        request.insert("max_output_tokens".to_string(), json!(value));
    }
    if let Some(value) = options.top_p {
        request.insert("top_p".to_string(), json!(value));
    }

    if !tools.is_empty() {
        request.insert("tools".to_string(), Value::Array(convert_tools(tools)));
        request.insert("tool_choice".to_string(), Value::String("auto".to_string()));
        request.insert("parallel_tool_calls".to_string(), Value::Bool(false));
        request.insert(
            "include".to_string(),
            json!(["reasoning.encrypted_content"]),
        );
    }

    Ok(Value::Object(request))
}

fn validate_options(options: &LlmParams) -> Result<(), String> {
    let mut unsupported = Vec::new();
    if options.frequency_penalty.is_some() {
        unsupported.push("frequency_penalty");
    }
    if options.presence_penalty.is_some() {
        unsupported.push("presence_penalty");
    }
    if options.stop.is_some() {
        unsupported.push("stop");
    }

    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "unsupported Responses API options: {}",
            unsupported.join(", ")
        ))
    }
}

fn convert_messages(messages: Vec<LlmChatMessage>) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();

    for rich_message in sanitize_llm_tool_message_sequence(messages) {
        items.extend(rich_message.provider_data);
        let value = serde_json::to_value(&rich_message.message)
            .map_err(|error| format!("failed to serialize LLM message: {}", error))?;
        let object = value
            .as_object()
            .ok_or_else(|| "failed to serialize LLM message as an object".to_string())?;
        let role = object
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| "LLM message is missing role".to_string())?;

        if role == "tool" {
            let call_id = object
                .get("tool_call_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "tool message is missing tool_call_id".to_string())?;
            items.push(json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": extract_text_content(object.get("content")),
            }));
            continue;
        }

        let content = convert_content(role, object.get("content"))?;
        if !is_empty_content(&content) {
            items.push(json!({
                "role": role,
                "content": content,
            }));
        }

        if role == "assistant" {
            if let Some(tool_calls) = object.get("tool_calls").and_then(Value::as_array) {
                for tool_call in tool_calls {
                    let call = tool_call
                        .as_object()
                        .ok_or_else(|| "assistant tool call is not an object".to_string())?;
                    let function = call
                        .get("function")
                        .and_then(Value::as_object)
                        .ok_or_else(|| "assistant tool call is missing function".to_string())?;
                    let call_id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| "assistant tool call is missing id".to_string())?;
                    let name = function
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| "assistant tool call is missing name".to_string())?;
                    let arguments = function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    items.push(json!({
                        "type": "function_call",
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments,
                    }));
                }
            }
        }
    }

    Ok(items)
}

fn convert_content(role: &str, content: Option<&Value>) -> Result<Value, String> {
    let Some(content) = content else {
        return Ok(Value::String(String::new()));
    };

    if let Some(text) = content.as_str() {
        return Ok(Value::String(text.to_string()));
    }

    let Some(parts) = content.as_array() else {
        return Err(format!("unsupported {} message content", role));
    };

    let mut converted = Vec::new();
    for part in parts {
        let object = part
            .as_object()
            .ok_or_else(|| format!("unsupported {} message content part", role))?;
        match object.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = object.get("text").and_then(Value::as_str) {
                    converted.push(json!({ "type": "input_text", "text": text }));
                }
            }
            Some("image_url") => {
                let image = object
                    .get("image_url")
                    .and_then(Value::as_object)
                    .ok_or_else(|| "image content is missing image_url".to_string())?;
                let url = image
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "image content is missing url".to_string())?;
                let mut item = json!({ "type": "input_image", "image_url": url });
                if let Some(detail) = image.get("detail").and_then(Value::as_str) {
                    item["detail"] = Value::String(detail.to_string());
                }
                converted.push(item);
            }
            Some(kind) => {
                return Err(format!(
                    "unsupported {} message content part type: {}",
                    role, kind
                ));
            }
            None => return Err(format!("{} message content part is missing type", role)),
        }
    }

    Ok(Value::Array(converted))
}

fn extract_text_content(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn is_empty_content(content: &Value) -> bool {
    match content {
        Value::String(text) => text.is_empty(),
        Value::Array(parts) => parts.is_empty(),
        _ => false,
    }
}

fn convert_tools(tools: Vec<LlmToolDefinition>) -> Vec<Value> {
    tools
        .into_iter()
        .map(|tool| {
            let strict = tool.parameters.iter().all(|param| param.required);
            let properties = tool
                .parameters
                .iter()
                .map(|param| {
                    (
                        param.name.clone(),
                        json!({
                            "type": "string",
                            "description": param.description,
                        }),
                    )
                })
                .collect::<Map<String, Value>>();
            let required = tool
                .parameters
                .iter()
                .filter(|param| param.required)
                .map(|param| param.name.clone())
                .collect::<Vec<_>>();

            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                },
                "strict": strict,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::messages::{
        assistant_tool_calls_message, system_message, tool_result_message,
        user_message_with_images, user_text_message,
    };
    use crate::llm::provider::LlmToolParam;

    #[test]
    fn builds_stateless_text_request_without_changing_roles() {
        let request = build_responses_request(
            "gpt-4o",
            vec![
                system_message("system").into(),
                user_text_message("hello").into(),
            ],
            Some(LlmParams {
                temperature: Some(0.4),
                max_tokens: Some(128),
                top_p: Some(0.9),
                ..Default::default()
            }),
            vec![],
            false,
        )
        .unwrap();

        assert_eq!(request["store"], false);
        assert_eq!(request["stream"], false);
        assert_eq!(request["max_output_tokens"], 128);
        assert_eq!(request["input"][0]["role"], "system");
        assert_eq!(request["input"][1]["content"], "hello");
    }

    #[test]
    fn maps_images_and_native_tool_continuation_items() {
        let mut assistant = LlmChatMessage::from(assistant_tool_calls_message(
            None,
            vec![(
                "call-1".to_string(),
                "inspect".to_string(),
                "{}".to_string(),
            )],
        ));
        assistant.provider_data = vec![json!({
            "type": "reasoning",
            "id": "rs-1",
            "summary": [],
            "encrypted_content": "encrypted"
        })];
        let request = build_responses_request(
            "gpt-4o",
            vec![
                user_message_with_images("look", vec!["data:image/png;base64,abc".to_string()])
                    .into(),
                assistant,
                tool_result_message("call-1", "ok").into(),
            ],
            None,
            vec![],
            true,
        )
        .unwrap();

        assert_eq!(request["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(request["input"][1]["type"], "reasoning");
        assert_eq!(request["input"][2]["type"], "function_call");
        assert_eq!(request["input"][2]["call_id"], "call-1");
        assert_eq!(request["input"][3]["type"], "function_call_output");
        assert_eq!(request["input"][3]["output"], "ok");
    }

    #[test]
    fn builds_strict_function_tools_and_disables_parallel_calls() {
        let request = build_responses_request(
            "gpt-4o",
            vec![user_text_message("hello").into()],
            None,
            vec![LlmToolDefinition {
                name: "lookup".to_string(),
                description: "Look up a value".to_string(),
                parameters: vec![LlmToolParam {
                    name: "query".to_string(),
                    description: "Search query".to_string(),
                    required: true,
                }],
            }],
            true,
        )
        .unwrap();

        assert_eq!(request["parallel_tool_calls"], false);
        assert_eq!(request["tool_choice"], "auto");
        assert_eq!(request["tools"][0]["strict"], true);
        assert_eq!(request["tools"][0]["parameters"]["required"][0], "query");
        assert_eq!(request["include"][0], "reasoning.encrypted_content");
    }

    #[test]
    fn rejects_options_not_supported_by_responses() {
        let result = build_responses_request(
            "gpt-4o",
            vec![user_text_message("hello").into()],
            Some(LlmParams {
                stop: Some(vec!["END".to_string()]),
                ..Default::default()
            }),
            vec![],
            false,
        );

        assert_eq!(
            result.unwrap_err(),
            "unsupported Responses API options: stop"
        );
    }

    #[test]
    fn disables_strict_mode_when_a_tool_has_optional_parameters() {
        let request = build_responses_request(
            "gpt-4o",
            vec![user_text_message("hello").into()],
            None,
            vec![LlmToolDefinition {
                name: "lookup".to_string(),
                description: "Look up a value".to_string(),
                parameters: vec![LlmToolParam {
                    name: "limit".to_string(),
                    description: "Optional result limit".to_string(),
                    required: false,
                }],
            }],
            true,
        )
        .unwrap();

        assert_eq!(request["tools"][0]["strict"], false);
        assert_eq!(request["tools"][0]["parameters"]["required"], json!([]));
    }
}
