// pattern: Functional Core

use async_openai::types::chat::ChatCompletionRequestMessage;
use std::collections::HashMap;

use crate::llm::provider::{LlmChatMessage, LlmStreamEvent, LlmToolCall, LlmToolDefinition};
use crate::llm::responses_protocol::build_responses_request;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub(crate) struct InferenceInputs {
    pub(crate) history: Vec<Value>,
    pub(crate) turn_input: Vec<Value>,
    pub(crate) tool_output: Option<Value>,
}

#[derive(Debug, Clone)]
pub(crate) enum RuntimeEvent {
    Stream(LlmStreamEvent),
    Completed {
        status: String,
        text: Option<String>,
    },
    Status,
    Failed(String),
}

pub(crate) fn build_rpc_request(id: u64, method: &str, params: Value) -> Value {
    json!({ "id": id, "method": method, "params": params })
}

pub(crate) fn build_rpc_notification(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": params })
}

pub(crate) fn build_initialize_params(client_version: &str) -> Value {
    json!({
        "clientInfo": {
            "name": "kokoro_engine",
            "title": "Kokoro Engine",
            "version": client_version,
        },
        "capabilities": { "experimentalApi": true }
    })
}

pub(crate) fn build_thread_start_params(
    model_override: Option<&str>,
    tools: &[LlmToolDefinition],
) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("ephemeral".to_string(), Value::Bool(true));
    params.insert(
        "approvalPolicy".to_string(),
        Value::String("never".to_string()),
    );
    params.insert(
        "sandbox".to_string(),
        Value::String("read-only".to_string()),
    );
    params.insert("environments".to_string(), Value::Array(Vec::new()));
    params.insert("baseInstructions".to_string(), Value::String(String::new()));
    params.insert(
        "developerInstructions".to_string(),
        Value::String(String::new()),
    );
    params.insert(
        "config".to_string(),
        json!({
            "mcp_servers": {},
            "include_permissions_instructions": false,
            "include_apps_instructions": false,
            "include_collaboration_mode_instructions": false,
            "features": {
                "apps": false,
                "browser_use": false,
                "code_mode": false,
                "connectors": false,
                "memories": false,
                "multi_agent": false,
                "plugins": false,
                "request_permissions": false,
                "request_permissions_tool": false,
                "search_tool": false,
                "shell_tool": false,
                "skip_host_skill_discovery": true,
                "skill_mcp_dependency_install": false,
                "skill_search": false,
                "tool_search": false,
                "web_search": false,
            },
        }),
    );

    if !tools.is_empty() {
        params.insert(
            "dynamicTools".to_string(),
            Value::Array(build_dynamic_tools(tools)),
        );
    }

    if let Some(model) = model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        params.insert("model".to_string(), Value::String(model.to_string()));
    }

    Value::Object(params)
}

fn build_dynamic_tools(tools: &[LlmToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();
            for parameter in &tool.parameters {
                properties.insert(
                    parameter.name.clone(),
                    json!({
                        "type": "string",
                        "description": parameter.description,
                    }),
                );
                if parameter.required {
                    required.push(parameter.name.clone());
                }
            }

            json!({
                "type": "function",
                // Codex reserves names such as `mcp__...`. Keep the Codex-facing
                // name separate from Kokoro's internal/MCP name and resolve it
                // back before the host tool loop executes.
                "name": codex_dynamic_tool_name(index, &tool.name),
                "description": tool.description,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                },
            })
        })
        .collect()
}

fn codex_dynamic_tool_name(index: usize, original_name: &str) -> String {
    const MAX_NAME_LENGTH: usize = 64;
    let mut normalized = String::new();
    let mut previous_separator = false;
    for character in original_name.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character);
            previous_separator = false;
        } else if !previous_separator {
            normalized.push('_');
            previous_separator = true;
        }
    }
    let normalized = normalized.trim_matches('_');
    let normalized = if normalized.is_empty() {
        "tool"
    } else {
        normalized
    };
    let prefix = format!("kokoro_tool_{index}_");
    let suffix_budget = MAX_NAME_LENGTH.saturating_sub(prefix.len());
    format!(
        "{}{}",
        prefix,
        normalized.chars().take(suffix_budget).collect::<String>()
    )
}

pub(crate) fn build_thread_inject_items_params(thread_id: &str, history: Vec<Value>) -> Value {
    json!({ "threadId": thread_id, "items": history })
}

pub(crate) fn build_turn_start_params(
    thread_id: &str,
    turn_input: Vec<Value>,
    tool_output: Option<Value>,
) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("threadId".to_string(), Value::String(thread_id.to_string()));
    params.insert("input".to_string(), Value::Array(turn_input));
    if let Some(tool_output) = tool_output {
        params.insert("toolOutput".to_string(), tool_output);
    }
    Value::Object(params)
}

pub(crate) fn build_turn_interrupt_params(thread_id: &str, turn_id: &str) -> Value {
    json!({ "threadId": thread_id, "turnId": turn_id })
}

pub(crate) fn prepare_inference_inputs(
    messages: Vec<LlmChatMessage>,
    tools: &[LlmToolDefinition],
) -> Result<InferenceInputs, String> {
    let latest_user_index = messages
        .iter()
        .rposition(|message| message_role(&message.message).as_deref() == Some("user"))
        .ok_or_else(|| "Codex runtime requires at least one user message".to_string())?;

    let tool_continuation_index = messages
        .iter()
        .rposition(|message| message_role(&message.message).as_deref() == Some("tool"))
        .filter(|index| *index == messages.len().saturating_sub(1));
    let history_end = tool_continuation_index.unwrap_or(latest_user_index);
    let history = build_responses_request(
        "",
        messages[..history_end].to_vec(),
        None,
        Vec::new(),
        false,
    )?;
    let (turn_input, tool_output) = if let Some(tool_index) = tool_continuation_index {
        (
            Vec::new(),
            Some(build_tool_output(&messages, tool_index, tools)?),
        )
    } else {
        let latest = build_responses_request(
            "",
            vec![messages[latest_user_index].clone()],
            None,
            Vec::new(),
            false,
        )?;
        let latest = latest
            .get("input")
            .and_then(Value::as_array)
            .ok_or_else(|| "Codex runtime turn was not an input array".to_string())?;
        (responses_items_to_user_input(latest)?, None)
    };

    let history = history
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "Codex runtime history was not an input array".to_string())?;
    let history = history
        .into_iter()
        .map(|item| normalize_history_item(item, tools))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(InferenceInputs {
        history,
        turn_input,
        tool_output,
    })
}

fn normalize_history_item(mut item: Value, tools: &[LlmToolDefinition]) -> Result<Value, String> {
    if item.get("type").and_then(Value::as_str) == Some("function_call") {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex runtime function call is missing a name".to_string())?;
        item["name"] = Value::String(codex_tool_alias_for_name(name, tools)?);
        return Ok(item);
    }
    if item.get("type").is_some() {
        return Ok(item);
    }
    if item.get("role").and_then(Value::as_str).is_some() {
        item["type"] = Value::String("message".to_string());
        if let Some(Value::String(text)) = item.get("content").cloned() {
            item["content"] = json!([{
                "type": "input_text",
                "text": text,
            }]);
        }
        return Ok(item);
    }
    Err("Codex runtime history item is missing a Responses API type".to_string())
}

fn build_tool_output(
    messages: &[LlmChatMessage],
    tool_index: usize,
    tools: &[LlmToolDefinition],
) -> Result<Value, String> {
    let tool_message = serde_json::to_value(&messages[tool_index].message)
        .map_err(|error| format!("failed to serialize Codex runtime tool output: {error}"))?;
    let call_id = tool_message
        .get("tool_call_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Codex runtime tool message is missing tool_call_id".to_string())?;
    let output = match tool_message.get("content") {
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
        None => String::new(),
    };

    let name = messages[..tool_index]
        .iter()
        .rev()
        .filter_map(|message| serde_json::to_value(&message.message).ok())
        .find_map(|message| {
            message
                .get("tool_calls")
                .and_then(Value::as_array)
                .and_then(|calls| {
                    calls.iter().find_map(|call| {
                        (call.get("id").and_then(Value::as_str) == Some(call_id))
                            .then(|| {
                                call.get("function")
                                    .and_then(|function| function.get("name"))
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                            })
                            .flatten()
                    })
                })
        })
        .ok_or_else(|| "Codex runtime tool message has no matching assistant call".to_string())?;

    Ok(json!({
        "name": codex_tool_alias_for_name(&name, tools)?,
        "namespace": null,
        "output": output,
    }))
}

fn codex_tool_alias_for_name(
    original_name: &str,
    tools: &[LlmToolDefinition],
) -> Result<String, String> {
    let index = find_tool_index_for_input(original_name, tools).map_err(|_| {
        format!("Codex runtime history references an unregistered Kokoro tool: {original_name}")
    })?;
    Ok(codex_dynamic_tool_name(index, &tools[index].name))
}

fn message_role(message: &ChatCompletionRequestMessage) -> Option<String> {
    serde_json::to_value(message).ok().and_then(|value| {
        value
            .get("role")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn responses_items_to_user_input(items: &[Value]) -> Result<Vec<Value>, String> {
    let mut input = Vec::new();
    for item in items {
        let role = item.get("role").and_then(Value::as_str);
        if role != Some("user") {
            return Err("Codex runtime latest input must be a user message".to_string());
        }

        match item.get("content") {
            Some(Value::String(text)) => input.push(json!({
                "type": "text",
                "text": text,
                "text_elements": [],
            })),
            Some(Value::Array(parts)) => {
                for part in parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("input_text") => input.push(json!({
                            "type": "text",
                            "text": part.get("text").and_then(Value::as_str).unwrap_or_default(),
                            "text_elements": [],
                        })),
                        Some("input_image") => {
                            let url = part
                                .get("image_url")
                                .and_then(Value::as_str)
                                .filter(|value| !value.trim().is_empty())
                                .ok_or_else(|| {
                                    "Codex runtime image input is missing image_url".to_string()
                                })?;
                            let mut image = json!({ "type": "image", "url": url });
                            if let Some(detail) = part.get("detail") {
                                image["detail"] = detail.clone();
                            }
                            input.push(image);
                        }
                        Some(kind) => {
                            return Err(format!("unsupported Codex runtime user input: {kind}"));
                        }
                        None => return Err("Codex runtime user input is missing type".to_string()),
                    }
                }
            }
            Some(_) => {
                return Err("Codex runtime user content has an unsupported shape".to_string())
            }
            None => return Err("Codex runtime user message is missing content".to_string()),
        }
    }

    if input.is_empty() {
        return Err("Codex runtime latest user message is empty".to_string());
    }
    Ok(input)
}

pub(crate) fn parse_runtime_notification(
    value: &Value,
    thread_id: &str,
    turn_id: &str,
) -> Option<RuntimeEvent> {
    let method = value.get("method").and_then(Value::as_str)?;
    let params = value.get("params")?;
    if method == "error" {
        let message = params
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Codex app-server reported an error")
            .to_string();
        if is_transient_transport_status(&message) {
            return Some(RuntimeEvent::Status);
        }
        return Some(RuntimeEvent::Failed(message));
    }
    let event_thread_id = params.get("threadId").and_then(Value::as_str)?;
    if event_thread_id != thread_id {
        return None;
    }
    if is_disallowed_capability_event(method, params) {
        return Some(RuntimeEvent::Failed(
            "Codex app-server attempted to use a disabled filesystem, shell, or MCP capability"
                .to_string(),
        ));
    }

    match method {
        "item/tool/call" if params.get("turnId").and_then(Value::as_str) == Some(turn_id) => {
            let id = params.get("callId").and_then(Value::as_str)?;
            let name = params.get("tool").and_then(Value::as_str)?;
            let args = match parse_tool_arguments(params.get("arguments")?) {
                Ok(args) => args,
                Err(error) => return Some(RuntimeEvent::Failed(error)),
            };
            Some(RuntimeEvent::Stream(LlmStreamEvent::ToolCall(
                LlmToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    args,
                },
            )))
        }
        "item/agentMessage/delta"
            if params.get("turnId").and_then(Value::as_str) == Some(turn_id) =>
        {
            let delta = params.get("delta").and_then(Value::as_str)?;
            if delta.is_empty() {
                None
            } else {
                Some(RuntimeEvent::Stream(LlmStreamEvent::Text(
                    delta.to_string(),
                )))
            }
        }
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta"
            if params.get("turnId").and_then(Value::as_str) == Some(turn_id) =>
        {
            let delta = params.get("delta").and_then(Value::as_str)?;
            if delta.is_empty() {
                None
            } else {
                Some(RuntimeEvent::Stream(LlmStreamEvent::ReasoningContent(
                    delta.to_string(),
                )))
            }
        }
        "turn/completed" => {
            let turn = params.get("turn")?;
            if turn.get("id").and_then(Value::as_str) != Some(turn_id) {
                return None;
            }
            let status = turn.get("status").and_then(Value::as_str)?.to_string();
            let text = extract_completed_agent_text(turn);
            if status == "failed" {
                Some(RuntimeEvent::Failed(
                    turn.get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("Codex turn failed")
                        .to_string(),
                ))
            } else {
                Some(RuntimeEvent::Completed { status, text })
            }
        }
        _ => None,
    }
}

fn is_transient_transport_status(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    (message.contains("falling back")
        && (message.contains("websocket") || message.contains("https")))
        || message.starts_with("reconnecting")
        || message.contains("retrying")
        || message.contains("request timed out")
        || message.contains("connection timed out")
}

fn is_disallowed_capability_event(method: &str, params: &Value) -> bool {
    let method = method.to_ascii_lowercase();
    if method.contains("commandexecution")
        || method.contains("filechange")
        || method.contains("mcptoolcall")
        || method.contains("skill")
        || method.contains("plugin")
        || method.contains("connector")
        || method.contains("search")
        || method.contains("web")
    {
        return true;
    }
    params
        .get("item")
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str)
        .map(|item_type| {
            let item_type = item_type.to_ascii_lowercase();
            item_type.contains("command")
                || item_type.contains("file")
                || item_type.contains("mcp")
                || item_type.contains("skill")
                || item_type.contains("plugin")
                || item_type.contains("app")
                || item_type.contains("connector")
                || item_type.contains("search")
                || item_type.contains("web")
        })
        .unwrap_or(false)
}

fn parse_tool_arguments(value: &Value) -> Result<HashMap<String, String>, String> {
    let object = if let Some(object) = value.as_object() {
        object.clone()
    } else if let Some(raw) = value.as_str() {
        serde_json::from_str::<Value>(raw)
            .map_err(|_| "Codex dynamic tool arguments were not valid JSON".to_string())?
            .as_object()
            .cloned()
            .ok_or_else(|| "Codex dynamic tool arguments must be a JSON object".to_string())?
    } else {
        return Err("Codex dynamic tool arguments must be an object".to_string());
    };

    Ok(object
        .into_iter()
        .map(|(key, value)| {
            let rendered = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            (key, rendered)
        })
        .collect())
}

fn extract_completed_agent_text(turn: &Value) -> Option<String> {
    turn.get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .rev()
                .find(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage"))
        })
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub(crate) fn parse_model_list_page(
    value: &Value,
) -> Result<(Vec<String>, Option<String>), String> {
    let models = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Codex app-server model/list returned no data array".to_string())?;
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for model in models {
        if model.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let id = model
            .get("model")
            .and_then(Value::as_str)
            .or_else(|| model.get("id").and_then(Value::as_str))
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                "Codex app-server model/list returned a model without an id".to_string()
            })?;
        if seen.insert(id.to_string()) {
            result.push(id.to_string());
        }
    }
    let next_cursor = match value.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) if cursor.trim().is_empty() => None,
        Some(Value::String(cursor)) => Some(cursor.to_string()),
        Some(_) => {
            return Err("Codex app-server model/list returned an invalid nextCursor".to_string())
        }
    };
    Ok((result, next_cursor))
}

pub(crate) fn validate_tool_call(
    call: &LlmToolCall,
    tools: &[LlmToolDefinition],
) -> Result<(), String> {
    let definition = tools
        .iter()
        .find(|tool| tool.name == call.name)
        .ok_or_else(|| format!("Codex requested an unregistered Kokoro tool: {}", call.name))?;
    let allowed = definition
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    if let Some(unexpected) = call.args.keys().find(|key| !allowed.contains(key.as_str())) {
        return Err(format!(
            "Codex supplied an unexpected argument for tool '{}': {}",
            call.name, unexpected
        ));
    }
    for parameter in &definition.parameters {
        if parameter.required && !call.args.contains_key(&parameter.name) {
            return Err(format!(
                "Codex omitted required argument '{}' for tool '{}'",
                parameter.name, call.name
            ));
        }
    }
    Ok(())
}

pub(crate) fn resolve_dynamic_tool_call_name(
    call: &mut LlmToolCall,
    tools: &[LlmToolDefinition],
) -> Result<(), String> {
    let index = find_tool_index_for_input(&call.name, tools).map_err(|_| {
        format!(
            "Codex requested an unknown or ambiguous Kokoro tool alias: {}",
            call.name
        )
    })?;
    call.name = tools[index].name.clone();
    Ok(())
}

fn find_tool_index_for_input(input: &str, tools: &[LlmToolDefinition]) -> Result<usize, String> {
    let matches = tools
        .iter()
        .enumerate()
        .filter(|(index, tool)| {
            codex_dynamic_tool_name(*index, &tool.name) == input
                || tool.name == input
                || human_tool_name(&tool.name).as_deref() == Some(input)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => Err("unregistered tool".to_string()),
        _ => Err("ambiguous tool".to_string()),
    }
}

fn human_tool_name(canonical_name: &str) -> Option<String> {
    canonical_name
        .strip_prefix("builtin__")
        .map(str::to_string)
        .or_else(|| {
            canonical_name
                .strip_prefix("mcp__")
                .and_then(|value| value.rsplit_once("__"))
                .map(|(_, tool_name)| tool_name.to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::messages::{
        assistant_text_message, assistant_tool_calls_message, system_message, tool_result_message,
        user_text_message,
    };
    use crate::llm::provider::{LlmChatMessage, LlmStreamEvent};

    #[test]
    fn thread_start_uses_ephemeral_read_only_inference_defaults() {
        let params = build_thread_start_params(None, &[]);

        assert_eq!(params["ephemeral"], true);
        assert_eq!(params["approvalPolicy"], "never");
        assert_eq!(params["sandbox"], "read-only");
        assert_eq!(params["environments"], serde_json::json!([]));
        assert_eq!(params["baseInstructions"], "");
        assert_eq!(params["developerInstructions"], "");
        assert_eq!(params["config"]["include_permissions_instructions"], false);
        assert_eq!(params["config"]["mcp_servers"], serde_json::json!({}));
        assert_eq!(params["config"]["include_apps_instructions"], false);
        assert_eq!(
            params["config"]["include_collaboration_mode_instructions"],
            false
        );
        assert!(params.get("model").is_none());
        assert!(params.get("modelProvider").is_none());
        assert!(params.get("dynamicTools").is_none());
    }

    #[test]
    fn thread_start_model_override_is_scoped_to_the_ephemeral_thread() {
        let params = build_thread_start_params(Some("  gpt-5.4  "), &[]);

        assert_eq!(params["model"], "gpt-5.4");
        assert_eq!(params["config"]["include_permissions_instructions"], false);
        assert!(params.get("modelProvider").is_none());
    }

    #[test]
    fn exposes_kokoro_tools_as_dynamic_tools_without_enabling_codex_tools() {
        let params = build_thread_start_params(
            None,
            &[LlmToolDefinition {
                name: "lookup".to_string(),
                description: "Look up a value".to_string(),
                parameters: vec![crate::llm::provider::LlmToolParam {
                    name: "query".to_string(),
                    description: "Search query".to_string(),
                    required: true,
                }],
            }],
        );

        assert_eq!(params["dynamicTools"][0]["type"], "function");
        assert_eq!(params["dynamicTools"][0]["name"], "kokoro_tool_0_lookup");
        assert_eq!(
            params["dynamicTools"][0]["inputSchema"]["required"],
            serde_json::json!(["query"])
        );
        assert_eq!(params["config"]["features"]["shell_tool"], false);
        assert_eq!(
            params["config"]["features"]["skip_host_skill_discovery"],
            true
        );
    }

    #[test]
    fn splits_existing_history_from_the_latest_user_turn() {
        let inputs = prepare_inference_inputs(
            vec![
                LlmChatMessage::from(system_message("system")),
                LlmChatMessage::from(user_text_message("earlier")),
                LlmChatMessage::from(assistant_text_message("answer")),
                LlmChatMessage::from(user_text_message("latest")),
            ],
            &[],
        )
        .expect("messages should convert");

        assert_eq!(inputs.history.len(), 3);
        assert_eq!(inputs.history[0]["role"], "system");
        assert_eq!(inputs.history[0]["type"], "message");
        assert_eq!(inputs.history[0]["content"][0]["type"], "input_text");
        assert_eq!(inputs.history[1]["content"][0]["text"], "earlier");
        assert_eq!(inputs.history[1]["content"][0]["type"], "input_text");
        assert_eq!(inputs.history[1]["type"], "message");
        assert_eq!(inputs.history[2]["content"][0]["text"], "answer");
        assert_eq!(inputs.history[2]["content"][0]["type"], "input_text");
        assert_eq!(inputs.history[2]["type"], "message");
        assert_eq!(
            inputs.turn_input,
            vec![serde_json::json!({
                "type": "text",
                "text": "latest",
                "text_elements": []
            })]
        );
    }

    #[test]
    fn maps_only_authoritative_text_reasoning_and_turn_completion_events() {
        let text = parse_runtime_notification(
            &serde_json::json!({
                "method": "item/agentMessage/delta",
                "params": { "threadId": "thread-1", "turnId": "turn-1", "delta": "hello" }
            }),
            "thread-1",
            "turn-1",
        )
        .expect("text delta should be accepted");
        assert!(
            matches!(text, RuntimeEvent::Stream(LlmStreamEvent::Text(value)) if value == "hello")
        );

        let reasoning = parse_runtime_notification(
            &serde_json::json!({
                "method": "item/reasoning/summaryTextDelta",
                "params": { "threadId": "thread-1", "turnId": "turn-1", "delta": "think" }
            }),
            "thread-1",
            "turn-1",
        )
        .expect("reasoning delta should be accepted");
        assert!(
            matches!(reasoning, RuntimeEvent::Stream(LlmStreamEvent::ReasoningContent(value)) if value == "think")
        );

        let completed = parse_runtime_notification(
            &serde_json::json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": { "id": "turn-1", "status": "completed", "items": [] }
                }
            }),
            "thread-1",
            "turn-1",
        )
        .expect("turn completion should be accepted");
        assert!(
            matches!(completed, RuntimeEvent::Completed { status, .. } if status == "completed")
        );

        assert!(parse_runtime_notification(
            &serde_json::json!({
                "method": "item/agentMessage/delta",
                "params": { "threadId": "other", "turnId": "turn-1", "delta": "ignored" }
            }),
            "thread-1",
            "turn-1",
        )
        .is_none());
    }

    #[test]
    fn keeps_backend_transport_fallback_as_a_non_terminal_status() {
        let event = parse_runtime_notification(
            &serde_json::json!({
                "method": "error",
                "params": { "message": "Falling back from WebSockets to HTTPS transport" }
            }),
            "thread-1",
            "turn-1",
        )
        .expect("transport fallback should be observable");

        assert!(matches!(event, RuntimeEvent::Status));
    }

    #[test]
    fn maps_codex_dynamic_tool_requests_to_kokoro_tool_calls() {
        let event = parse_runtime_notification(
            &serde_json::json!({
                "method": "item/tool/call",
                "id": 42,
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "callId": "call-1",
                    "namespace": null,
                    "tool": "lookup",
                    "arguments": { "query": "hello", "limit": 3 }
                }
            }),
            "thread-1",
            "turn-1",
        )
        .expect("dynamic tool request should be accepted");

        match event {
            RuntimeEvent::Stream(LlmStreamEvent::ToolCall(call)) => {
                assert_eq!(call.id, "call-1");
                assert_eq!(call.name, "lookup");
                assert_eq!(call.args.get("query").map(String::as_str), Some("hello"));
                assert_eq!(call.args.get("limit").map(String::as_str), Some("3"));
            }
            other => panic!("expected a Kokoro tool call, got {other:?}"),
        }
    }

    #[test]
    fn resolves_codex_tool_aliases_back_to_mcp_names() {
        let mut call = LlmToolCall {
            id: "call-1".to_string(),
            name: "kokoro_tool_0_mcp_time_convert_time".to_string(),
            args: HashMap::new(),
        };
        let tools = vec![LlmToolDefinition {
            name: "mcp__time__convert_time".to_string(),
            description: "Convert a time".to_string(),
            parameters: Vec::new(),
        }];

        resolve_dynamic_tool_call_name(&mut call, &tools).expect("alias should resolve");
        assert_eq!(call.name, "mcp__time__convert_time");
    }

    #[test]
    fn parses_model_list_using_runtime_model_ids_without_hidden_entries() {
        let (models, next_cursor) = parse_model_list_page(&serde_json::json!({
            "data": [
                { "id": "gpt-5.4", "model": "gpt-5.4", "hidden": false },
                { "id": "gpt-5.4-mini", "model": "gpt-5.4-mini", "hidden": false },
                { "id": "internal", "model": "internal", "hidden": true },
                { "id": "gpt-5.4", "model": "gpt-5.4", "hidden": false }
            ],
            "nextCursor": "next-page"
        }))
        .expect("model list should parse");

        assert_eq!(models, vec!["gpt-5.4", "gpt-5.4-mini"]);
        assert_eq!(next_cursor.as_deref(), Some("next-page"));
    }

    #[test]
    fn represents_kokoro_tool_results_as_a_codex_tool_output_turn() {
        let tools = vec![LlmToolDefinition {
            name: "builtin__lookup".to_string(),
            description: "Look up a value".to_string(),
            parameters: Vec::new(),
        }];
        let inputs = prepare_inference_inputs(
            vec![
                LlmChatMessage::from(user_text_message("use lookup")),
                LlmChatMessage::from(assistant_tool_calls_message(
                    None,
                    vec![("call-1".to_string(), "lookup".to_string(), "{}".to_string())],
                )),
                LlmChatMessage::from(tool_result_message("call-1", "lookup result")),
            ],
            &tools,
        )
        .expect("tool continuation should convert");

        assert_eq!(inputs.turn_input, Vec::<serde_json::Value>::new());
        assert_eq!(
            inputs.tool_output,
            Some(serde_json::json!({
                "name": "kokoro_tool_0_builtin_lookup",
                "namespace": null,
                "output": "lookup result"
            }))
        );
        assert_eq!(
            inputs.history.last().expect("assistant call")["type"],
            "function_call"
        );
        assert_eq!(
            inputs.history.last().expect("assistant call")["name"],
            "kokoro_tool_0_builtin_lookup"
        );
    }

    #[test]
    fn rejects_malformed_dynamic_tool_arguments() {
        let event = parse_runtime_notification(
            &serde_json::json!({
                "method": "item/tool/call",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "callId": "call-1",
                    "tool": "lookup",
                    "arguments": "not-json"
                }
            }),
            "thread-1",
            "turn-1",
        )
        .expect("malformed tool arguments should become a failed runtime event");

        assert!(matches!(event, RuntimeEvent::Failed(message) if message.contains("valid JSON")));
    }

    #[test]
    fn rejects_disabled_codex_capability_events() {
        let event = parse_runtime_notification(
            &serde_json::json!({
                "method": "item/commandExecution/outputDelta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "delta": "unexpected"
                }
            }),
            "thread-1",
            "turn-1",
        )
        .expect("disabled capability should become a failed runtime event");

        assert!(matches!(event, RuntimeEvent::Failed(message) if message.contains("disabled")));
    }
}
