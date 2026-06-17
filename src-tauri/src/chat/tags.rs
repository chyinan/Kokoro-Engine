use crate::actions::ToolInvocation;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const TOOL_CALL_TAG_PREFIX: &str = "[TOOL_CALL:";
const TRANSLATE_TAG_PREFIX: &str = "[TRANSLATE:";

/// Tag prefixes that should be buffered (not emitted to frontend mid-stream).
const BUFFERED_TAG_PREFIXES: &[&str] = &[TOOL_CALL_TAG_PREFIX, TRANSLATE_TAG_PREFIX];

/// Returns the byte position up to which it's safe to emit text to the frontend.
/// Holds back any suffix that could be the start of a known tag prefix.
pub(crate) fn find_safe_emit_boundary(text: &str) -> usize {
    if let Some(last_bracket) = text.rfind('[') {
        let suffix = &text[last_bracket..];
        for prefix in BUFFERED_TAG_PREFIXES {
            if suffix.len() < prefix.len() {
                // Partial match — could still become a full tag
                if prefix.starts_with(suffix) {
                    return last_bracket;
                }
            } else if suffix.starts_with(prefix) {
                // Full prefix match — definitely a tag, hold it
                return last_bracket;
            }
        }
    }
    text.len()
}

/// Strip any `<tool_result>...</tool_result>` blocks or stray tags that the LLM may echo back.
pub(crate) fn strip_leaked_tags(text: &str) -> String {
    let mut result = text.to_string();
    // Remove <tool_result>...</tool_result> blocks (greedy within single block)
    while let Some(start) = result.find("<tool_result>") {
        if let Some(end) = result[start..].find("</tool_result>") {
            let tag_end = start + end + "</tool_result>".len();
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            // Unclosed tag — remove from <tool_result> to end of line
            let line_end = result[start..]
                .find('\n')
                .map(|i| start + i)
                .unwrap_or(result.len());
            result = format!("{}{}", result[..start].trim_end(), &result[line_end..]);
        }
    }
    result.trim().to_string()
}

/// Strip `[TRANSLATE:...]` tags from text.
pub(crate) fn strip_translate_tags(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find(TRANSLATE_TAG_PREFIX) {
        if let Some(end_bracket) = result[start..].find(']') {
            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            // Unclosed tag — remove from [TRANSLATE: to end
            result = result[..start].trim_end().to_string();
        }
    }
    result.trim().to_string()
}

pub(crate) fn merge_continuation_text(accumulated: &mut String, next: &str) {
    if next.is_empty() {
        return;
    }
    if accumulated.is_empty() {
        accumulated.push_str(next);
        return;
    }
    if next.starts_with(accumulated.as_str()) {
        *accumulated = next.to_string();
        return;
    }
    if accumulated.ends_with(next) {
        return;
    }

    let mut overlap = 0usize;
    let max_overlap = accumulated.len().min(next.len());
    for candidate in (1..=max_overlap).rev() {
        if accumulated.is_char_boundary(accumulated.len() - candidate)
            && next.is_char_boundary(candidate)
            && accumulated[accumulated.len() - candidate..] == next[..candidate]
        {
            overlap = candidate;
            break;
        }
    }

    if overlap > 0 {
        accumulated.push_str(&next[overlap..]);
    } else {
        if !accumulated.ends_with(char::is_whitespace) && !next.starts_with(char::is_whitespace) {
            accumulated.push(' ');
        }
        accumulated.push_str(next);
    }
}

/// Extract the content inside `[TRANSLATE:...]` tags, then strip them from text.
/// Returns (cleaned_text, Option<translation>).
pub(crate) fn extract_translate_tags(text: &str) -> (String, Option<String>) {
    let mut translations = Vec::new();
    let mut result = text.to_string();
    while let Some(start) = result.find(TRANSLATE_TAG_PREFIX) {
        if let Some(end_bracket) = result[start..].find(']') {
            let inner = &result[start + TRANSLATE_TAG_PREFIX.len()..start + end_bracket];
            let trimmed = inner.trim();
            if !trimmed.is_empty() {
                translations.push(trimmed.to_string());
            }
            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                result[tag_end..].trim_start()
            );
        } else {
            // Unclosed tag — extract what we can
            let inner = &result[start + TRANSLATE_TAG_PREFIX.len()..];
            let trimmed = inner.trim();
            if !trimmed.is_empty() {
                translations.push(trimmed.to_string());
            }
            result = result[..start].trim_end().to_string();
        }
    }
    let translation = if translations.is_empty() {
        None
    } else {
        Some(translations.join(" "))
    };
    (result.trim().to_string(), translation)
}

/// Parsed tool call from `[TOOL_CALL:name|key=val|key=val]`
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToolCall {
    pub(crate) tool_call_id: Option<String>,
    pub(crate) name: String,
    pub(crate) args: HashMap<String, String>,
}

fn tool_call_fingerprint(tool_call: &ToolCall) -> String {
    let mut args = tool_call.args.iter().collect::<Vec<_>>();
    args.sort_by(|(left_key, left_value), (right_key, right_value)| {
        left_key
            .cmp(right_key)
            .then_with(|| left_value.cmp(right_value))
    });

    let serialized_args = args
        .into_iter()
        .map(|(key, value)| format!("{}={}", key, value))
        .collect::<Vec<_>>()
        .join("&");

    format!("{}|{}", tool_call.name, serialized_args)
}

pub(crate) fn merge_round_tool_calls(
    parsed_tool_calls: Vec<ToolCall>,
    native_tool_calls: Vec<ToolCall>,
) -> (Vec<ToolCall>, usize) {
    if parsed_tool_calls.is_empty() {
        return (native_tool_calls, 0);
    }
    if native_tool_calls.is_empty() {
        return (parsed_tool_calls, 0);
    }

    let native_fingerprints = native_tool_calls
        .iter()
        .map(tool_call_fingerprint)
        .collect::<HashSet<_>>();
    let mut deduped_textual_tool_call_count = 0usize;
    let mut merged = parsed_tool_calls
        .into_iter()
        .filter(|tool_call| {
            let is_duplicate = native_fingerprints.contains(&tool_call_fingerprint(tool_call));
            if is_duplicate {
                deduped_textual_tool_call_count += 1;
            }
            !is_duplicate
        })
        .collect::<Vec<_>>();

    merged.extend(native_tool_calls);
    (merged, deduped_textual_tool_call_count)
}

impl From<ToolCall> for ToolInvocation {
    fn from(value: ToolCall) -> Self {
        Self {
            tool_call_id: value.tool_call_id,
            name: value.name,
            args: value.args,
        }
    }
}

/// Parse all `[TOOL_CALL:name|key=val|...]` tags from the text.
/// Returns (cleaned_text, Vec<ToolCall>).
pub(crate) fn parse_tool_call_tags(text: &str) -> (String, Vec<ToolCall>) {
    let mut result = text.to_string();
    let mut calls = Vec::new();

    while let Some(start) = result.rfind(TOOL_CALL_TAG_PREFIX) {
        let rest = &result[start..];
        if let Some(end_bracket) = rest.find(']') {
            let inner = &rest[TOOL_CALL_TAG_PREFIX.len()..end_bracket];
            let parts: Vec<&str> = inner.split('|').collect();

            if let Some(name) = parts.first() {
                let name = name.trim().to_string();
                let mut args = HashMap::new();

                for part in parts.iter().skip(1) {
                    if let Some(eq_pos) = part.find('=') {
                        let key = part[..eq_pos].trim().to_string();
                        let val = part[eq_pos + 1..].trim().to_string();
                        args.insert(key, val);
                    }
                }

                calls.push(ToolCall {
                    tool_call_id: None,
                    name,
                    args,
                });
            }

            let tag_end = start + end_bracket + 1;
            result = format!(
                "{}{}",
                result[..start].trim_end(),
                if tag_end < result.len() {
                    &result[tag_end..]
                } else {
                    ""
                }
            );
        } else {
            break;
        }
    }

    // 额外支持简化格式: [action_name|key=val|key=val]
    // 例: [play_cue|cue=shy]
    let mut extra_calls = Vec::new();
    let mut cleaned = result.clone();
    let mut offset = 0;
    while offset < cleaned.len() {
        let Some(rel_start) = cleaned[offset..].find('[') else {
            break;
        };
        let start = offset + rel_start;
        let rest = &cleaned[start..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[1..end];

        let mut matched = false;
        if let Some(pipe_pos) = inner.find('|') {
            let name_part = &inner[..pipe_pos];
            let is_identifier =
                !name_part.is_empty() && name_part.chars().all(|c| c.is_alphanumeric() || c == '_');
            let has_kv = inner[pipe_pos + 1..].contains('=');

            if is_identifier && has_kv {
                let parts: Vec<&str> = inner.split('|').collect();
                let name = parts[0].trim().to_string();
                let mut args = HashMap::new();
                for part in parts.iter().skip(1) {
                    if let Some(eq_pos) = part.find('=') {
                        let key = part[..eq_pos].trim().to_string();
                        let val = part[eq_pos + 1..].trim().to_string();
                        args.insert(key, val);
                    }
                }
                extra_calls.push(ToolCall {
                    tool_call_id: None,
                    name,
                    args,
                });
                let tag_end = start + end + 1;
                cleaned = format!(
                    "{}{}",
                    cleaned[..start].trim_end(),
                    if tag_end < cleaned.len() {
                        &cleaned[tag_end..]
                    } else {
                        ""
                    }
                );
                // offset 不变，继续从同一位置扫描（内容已缩短）
                matched = true;
            }
        }
        if !matched {
            // 跳过这个 [ 继续往后找
            offset = start + 1;
        }
    }
    calls.extend(extra_calls);

    // 支持冒号格式: [action_name:value]
    // 例: [play_cue:happy]、[set_background:beach]
    // 将 value 映射到该 action 的主参数名
    let primary_arg_map: &[(&str, &str)] = &[("play_cue", "cue"), ("set_background", "prompt")];
    let mut colon_calls = Vec::new();
    let mut cleaned2 = cleaned.clone();
    let mut offset2 = 0;
    while offset2 < cleaned2.len() {
        let Some(rel_start) = cleaned2[offset2..].find('[') else {
            break;
        };
        let start = offset2 + rel_start;
        let rest = &cleaned2[start..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[1..end];

        let mut matched = false;
        if let Some(colon_pos) = inner.find(':') {
            let name_part = inner[..colon_pos].trim();
            let val_part = inner[colon_pos + 1..].trim();
            let is_identifier =
                !name_part.is_empty() && name_part.chars().all(|c| c.is_alphanumeric() || c == '_');

            if is_identifier && !val_part.is_empty() {
                if let Some(&(_, arg_key)) = primary_arg_map.iter().find(|&&(n, _)| n == name_part)
                {
                    let mut args = HashMap::new();
                    args.insert(arg_key.to_string(), val_part.to_string());
                    colon_calls.push(ToolCall {
                        tool_call_id: None,
                        name: name_part.to_string(),
                        args,
                    });
                    let tag_end = start + end + 1;
                    cleaned2 = format!(
                        "{}{}",
                        cleaned2[..start].trim_end(),
                        if tag_end < cleaned2.len() {
                            &cleaned2[tag_end..]
                        } else {
                            ""
                        }
                    );
                    matched = true;
                }
            }
        }
        if !matched {
            offset2 = start + 1;
        }
    }
    calls.extend(colon_calls);

    calls.reverse();
    (cleaned2.trim().to_string(), calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_translate_tags_basic() {
        let input = "こんにちは[TRANSLATE:你好]";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "こんにちは");
        assert_eq!(translation, Some("你好".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_none() {
        let input = "こんにちは";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "こんにちは");
        assert_eq!(translation, None);
    }

    #[test]
    fn test_extract_translate_tags_multiple() {
        let input = "A[TRANSLATE:甲] B[TRANSLATE:乙]";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "AB");
        assert_eq!(translation, Some("甲 乙".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_unclosed() {
        let input = "hello[TRANSLATE:world";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "hello");
        assert_eq!(translation, Some("world".to_string()));
    }

    #[test]
    fn test_extract_translate_tags_empty_content() {
        let input = "hello[TRANSLATE:]world";
        let (text, translation) = extract_translate_tags(input);
        assert_eq!(text, "helloworld");
        assert_eq!(translation, None);
    }

    #[test]
    fn test_strip_translate_tags() {
        let input = "こんにちは[TRANSLATE:你好]";
        assert_eq!(strip_translate_tags(input), "こんにちは");
    }

    #[test]
    fn test_strip_translate_tags_no_tag() {
        let input = "こんにちは";
        assert_eq!(strip_translate_tags(input), "こんにちは");
    }

    #[test]
    fn test_strip_leaked_tags_removes_tool_result() {
        let input = "before<tool_result>leaked data</tool_result>after";
        assert_eq!(strip_leaked_tags(input), "beforeafter");
    }

    #[test]
    fn test_strip_leaked_tags_unclosed() {
        let input = "before<tool_result>leaked\nafter";
        assert_eq!(strip_leaked_tags(input), "before\nafter");
    }

    #[test]
    fn test_strip_leaked_tags_no_tag() {
        let input = "clean text";
        assert_eq!(strip_leaked_tags(input), "clean text");
    }

    #[test]
    fn test_safe_emit_boundary_no_bracket() {
        let text = "hello world";
        assert_eq!(find_safe_emit_boundary(text), text.len());
    }

    #[test]
    fn test_safe_emit_boundary_partial_tool_call() {
        let text = "hello [TOOL_CA";
        let boundary = find_safe_emit_boundary(text);
        assert_eq!(boundary, "hello ".len());
    }

    #[test]
    fn test_safe_emit_boundary_partial_translate() {
        let text = "hello [TRANS";
        let boundary = find_safe_emit_boundary(text);
        assert_eq!(boundary, "hello ".len());
    }

    #[test]
    fn test_safe_emit_boundary_unrelated_bracket() {
        let text = "hello [world]";
        assert_eq!(find_safe_emit_boundary(text), text.len());
    }

    #[test]
    fn test_parse_tool_call_basic() {
        let input = "text[TOOL_CALL:play_cue|cue=happy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"happy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_no_tag() {
        let input = "just text";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "just text");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_tool_call_multiple_args() {
        let input = "[TOOL_CALL:set_background|prompt=beach|style=anime]";
        let (_, calls) = parse_tool_call_tags(input);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args.get("prompt"), Some(&"beach".to_string()));
        assert_eq!(calls[0].args.get("style"), Some(&"anime".to_string()));
    }

    #[test]
    fn test_parse_tool_call_simplified_format() {
        let input = "text[play_cue|cue=shy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"shy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_simplified_multiple() {
        let input = "hello[play_cue|cue=happy]world[play_cue|cue=sad]end";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "helloworldend");
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_parse_tool_call_simplified_no_false_positive() {
        // 普通方括号内容不应被误识别
        let input = "text [some words] more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "text [some words] more");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_parse_tool_call_colon_format() {
        let input = "text[play_cue:happy]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned.trim(), "textmore");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "play_cue");
        assert_eq!(calls[0].args.get("cue"), Some(&"happy".to_string()));
    }

    #[test]
    fn test_parse_tool_call_colon_unknown_action_no_match() {
        // 未在映射表中的 action 不应被识别为工具调用
        let input = "text[unknown_action:value]more";
        let (cleaned, calls) = parse_tool_call_tags(input);
        assert_eq!(cleaned, "text[unknown_action:value]more");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_merge_round_tool_calls_deduplicates_matching_textual_calls() {
        let parsed_tool_calls = vec![
            ToolCall {
                tool_call_id: None,
                name: "play_cue".to_string(),
                args: HashMap::from([("cue".to_string(), "happy".to_string())]),
            },
            ToolCall {
                tool_call_id: None,
                name: "store_memory".to_string(),
                args: HashMap::from([("fact".to_string(), "promise".to_string())]),
            },
        ];
        let native_tool_calls = vec![
            ToolCall {
                tool_call_id: Some("call-1".to_string()),
                name: "play_cue".to_string(),
                args: HashMap::from([("cue".to_string(), "happy".to_string())]),
            },
            ToolCall {
                tool_call_id: Some("call-2".to_string()),
                name: "store_memory".to_string(),
                args: HashMap::from([("fact".to_string(), "promise".to_string())]),
            },
        ];

        let (merged, deduped_count) = merge_round_tool_calls(parsed_tool_calls, native_tool_calls);

        assert_eq!(deduped_count, 2);
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().all(|call| call.tool_call_id.is_some()));
    }

    #[test]
    fn test_merge_round_tool_calls_keeps_non_matching_textual_calls() {
        let parsed_tool_calls = vec![
            ToolCall {
                tool_call_id: None,
                name: "play_cue".to_string(),
                args: HashMap::from([("cue".to_string(), "happy".to_string())]),
            },
            ToolCall {
                tool_call_id: None,
                name: "store_memory".to_string(),
                args: HashMap::from([("fact".to_string(), "promise".to_string())]),
            },
        ];
        let native_tool_calls = vec![ToolCall {
            tool_call_id: Some("call-1".to_string()),
            name: "play_cue".to_string(),
            args: HashMap::from([("cue".to_string(), "happy".to_string())]),
        }];

        let (merged, deduped_count) = merge_round_tool_calls(parsed_tool_calls, native_tool_calls);

        assert_eq!(deduped_count, 1);
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|call| {
            call.tool_call_id.is_none()
                && call.name == "store_memory"
                && call.args.get("fact") == Some(&"promise".to_string())
        }));
        assert!(merged.iter().any(|call| {
            call.tool_call_id.as_deref() == Some("call-1") && call.name == "play_cue"
        }));
    }
}
