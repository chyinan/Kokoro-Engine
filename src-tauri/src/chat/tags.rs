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
    strip_markdown_emphasis_markers(result.trim())
}

/// Remove paired Markdown emphasis delimiters from text rendered as plain text.
///
/// The chat UI intentionally does not render Markdown. Some providers still
/// return escaped or regular emphasis delimiters, such as `\\*\\*today\\*\\*`,
/// `**today**`, or `*today*`, which would otherwise leak into the visible
/// conversation. Keep unmatched delimiters and word-like exponent/identifier
/// forms intact.
pub(crate) fn strip_markdown_emphasis_markers(text: &str) -> String {
    let mut masked = String::with_capacity(text.len());
    let mut replacements: Vec<(String, &str)> = Vec::new();
    let mut segment_start = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'`' || (index > 0 && bytes[index - 1] == b'\\') {
            index += 1;
            continue;
        }

        let mut opening_end = index + 1;
        while opening_end < bytes.len() && bytes[opening_end] == b'`' {
            opening_end += 1;
        }
        let delimiter_len = opening_end - index;
        let mut cursor = opening_end;
        let mut closing_start = None;

        while cursor < bytes.len() {
            if bytes[cursor] != b'`' || (cursor > 0 && bytes[cursor - 1] == b'\\') {
                cursor += 1;
                continue;
            }

            let mut closing_end = cursor + 1;
            while closing_end < bytes.len() && bytes[closing_end] == b'`' {
                closing_end += 1;
            }
            if closing_end - cursor == delimiter_len {
                closing_start = Some(cursor);
                break;
            }
            cursor = closing_end;
        }

        let code_end = closing_start
            .map(|start| start + delimiter_len)
            .unwrap_or(text.len());
        let mut token_index = replacements.len();
        let token = loop {
            let candidate = format!("\u{e000}KOKORO_CODE_{token_index}\u{e001}");
            if !text.contains(&candidate)
                && !replacements.iter().any(|(token, _)| token == &candidate)
            {
                break candidate;
            }
            token_index += 1;
        };
        masked.push_str(&text[segment_start..index]);
        masked.push_str(&token);
        replacements.push((token, &text[index..code_end]));
        index = code_end;
        segment_start = index;

        if closing_start.is_none() {
            break;
        }
    }

    masked.push_str(&text[segment_start..]);
    let mut result = strip_markdown_emphasis_segment(&masked);
    for (token, source) in replacements {
        result = result.replace(&token, source);
    }
    result
}

fn strip_markdown_emphasis_segment(text: &str) -> String {
    let mut result = text.to_string();
    for marker in [r"\*\*", "**", r"\*", "*"] {
        let mut search_from = 0usize;
        while let Some(open_rel) = result[search_from..].find(marker) {
            let open = search_from + open_rel;
            let content_start = open + marker.len();
            if marker == "*"
                && (open > 0
                    && (result.as_bytes()[open - 1] == b'*'
                        || result.as_bytes()[open - 1] == b'\\')
                    || result.as_bytes().get(content_start) == Some(&b'*'))
            {
                search_from = content_start;
                continue;
            }
            let Some(close_rel) = result[content_start..].find(marker) else {
                break;
            };
            let close = content_start + close_rel;
            let content = &result[content_start..close];

            if content.trim().is_empty() {
                search_from = content_start;
                continue;
            }

            if looks_like_regex_literal(&result, open, close, marker)
                || looks_like_glob_pattern(&result, open, close, marker)
            {
                search_from = content_start;
                continue;
            }

            let previous = result[..open].chars().next_back();
            let first_content = content.chars().next();
            let last_content = content.chars().next_back();
            let has_content_boundary_whitespace = first_content
                .map(char::is_whitespace)
                .unwrap_or(true)
                || last_content.map(char::is_whitespace).unwrap_or(true);
            let looks_like_word_operator = previous
                .zip(first_content)
                .map(|(left, right)| left.is_ascii_alphanumeric() && right.is_ascii_alphanumeric())
                .unwrap_or(false);
            if has_content_boundary_whitespace || looks_like_word_operator {
                search_from = content_start;
                continue;
            }

            result.replace_range(close..close + marker.len(), "");
            result.replace_range(open..open + marker.len(), "");
            search_from = open;
        }
    }
    result
}

fn looks_like_regex_literal(text: &str, open: usize, close: usize, marker: &str) -> bool {
    let Some(opening_slash) = text[..open].rfind('/') else {
        return false;
    };
    if opening_slash > 0 && text.as_bytes()[opening_slash - 1] == b'\\' {
        return false;
    }

    let before_slash = text[..opening_slash].chars().next_back();
    if before_slash.is_some_and(|character| !character.is_whitespace() && !"([{=:;,!?".contains(character)) {
        return false;
    }

    let closing_slash = close + marker.len();
    if text.as_bytes().get(closing_slash) != Some(&b'/') {
        return false;
    }

    !text[opening_slash + 1..closing_slash].contains('\n')
}

fn looks_like_glob_pattern(text: &str, open: usize, close: usize, marker: &str) -> bool {
    if marker != "*" {
        return false;
    }

    let content_start = open + marker.len();
    let content = &text[content_start..close];
    let after_close = text[close + marker.len()..].chars().next();
    let previous = text[..open].chars().next_back();
    let before_previous = text[..open].chars().rev().nth(1);
    let after_separator = text[close + marker.len()..].chars().nth(1);
    let has_path_prefix = matches!(previous, Some('/' | '\\'))
        && before_previous.is_some_and(|character| !character.is_whitespace());
    let has_path_suffix = matches!(after_close, Some('/' | '\\'))
        && after_separator.is_some_and(|character| !character.is_whitespace());
    let has_path_wildcard = (content.contains('/') || content.contains('\\'))
        && (content.contains('?') || content.contains('*'));
    let has_character_class_range = content
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .map(|value| {
            value
                .strip_prefix('!')
                .or_else(|| value.strip_prefix('^'))
                .unwrap_or(value)
                .contains('-')
        })
        .unwrap_or(false);

    content.starts_with('.')
        || content.ends_with('.')
        || matches!(previous, Some('.'))
        || matches!(after_close, Some('.'))
        || has_path_prefix
        || has_path_suffix
        || has_path_wildcard
        || has_character_class_range
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
        let continues_open_emphasis = continues_cross_round_emphasis(accumulated, next);
        if !continues_open_emphasis
            && !accumulated.ends_with(char::is_whitespace)
            && !next.starts_with(char::is_whitespace)
        {
            accumulated.push(' ');
        }
        accumulated.push_str(next);
    }
}

fn continues_cross_round_emphasis(accumulated: &str, next: &str) -> bool {
    let accumulated = accumulated.trim_end();
    [r"\*\*", "**", r"\*", "*"].into_iter().any(|marker| {
        has_unmatched_trailing_emphasis_marker(accumulated, marker)
            && contains_closing_emphasis_marker(next, marker)
    })
}

fn contains_closing_emphasis_marker(text: &str, marker: &str) -> bool {
    let mut search_from = 0usize;
    while let Some(relative) = text[search_from..].find(marker) {
        let index = search_from + relative;
        let content_before = text[..index].chars().next_back();
        let after = text[index + marker.len()..].chars().next();
        let is_single_unescaped_star = marker == "*"
            && (text[..index].ends_with(['*', '\\'])
                || text[index + marker.len()..].starts_with('*'));
        if !is_single_unescaped_star
            && content_before.is_some_and(|character| !character.is_whitespace())
            && after.is_none_or(|character| !character.is_alphanumeric())
        {
            return true;
        }
        search_from = index + marker.len();
    }
    false
}

fn has_unmatched_trailing_emphasis_marker(text: &str, marker: &str) -> bool {
    if !text.ends_with(marker) {
        return false;
    }

    if marker != "*" {
        return text.match_indices(marker).count() % 2 == 1;
    }

    let bytes = text.as_bytes();
    let standalone_count = bytes
        .iter()
        .enumerate()
        .filter(|(index, byte)| {
            **byte == b'*'
                && (*index == 0 || !matches!(bytes[*index - 1], b'*' | b'\\'))
                && bytes.get(*index + 1) != Some(&b'*')
        })
        .count();
    standalone_count % 2 == 1
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
#[derive(Debug, Clone, Serialize, PartialEq)]
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
    fn test_strip_markdown_emphasis_markers_for_date_and_weekday_replies() {
        assert_eq!(
            strip_markdown_emphasis_markers(r"今天是 \*\*2026年9月21日，星期一\*\*。"),
            "今天是 2026年9月21日，星期一。"
        );
        assert_eq!(
            strip_markdown_emphasis_markers(r"今天是 **2026年9月21日**，**星期一**。"),
            "今天是 2026年9月21日，星期一。"
        );
    }

    #[test]
    fn test_strip_markdown_emphasis_markers_handles_multiple_segments_and_punctuation() {
        assert_eq!(
            strip_markdown_emphasis_markers(r"\*\*日期\*\*：\*\*2026-09-21\*\*。"),
            "日期：2026-09-21。"
        );
    }

    #[test]
    fn test_strip_markdown_emphasis_markers_preserves_unmatched_and_word_operator_stars() {
        assert_eq!(strip_markdown_emphasis_markers(r"unfinished \*\*bold"), r"unfinished \*\*bold");
        assert_eq!(strip_markdown_emphasis_markers("2**3**"), "2**3**");
        assert_eq!(strip_markdown_emphasis_markers(r"literal \* star"), r"literal \* star");
    }

    #[test]
    fn test_strip_markdown_emphasis_markers_handles_italics_without_harming_math_or_lists() {
        assert_eq!(
            strip_markdown_emphasis_markers(r"今天是 \*星期一\*。"),
            "今天是 星期一。"
        );
        assert_eq!(
            strip_markdown_emphasis_markers("今天是 *星期一*。"),
            "今天是 星期一。"
        );
        assert_eq!(strip_markdown_emphasis_markers("2 * 3 = 6\n* 条目"), "2 * 3 = 6\n* 条目");
    }

    #[test]
    fn test_strip_markdown_emphasis_markers_preserves_inline_code() {
        assert_eq!(
            strip_markdown_emphasis_markers("请使用 `*foo*` 和 `**bar**` 匹配文件名"),
            "请使用 `*foo*` 和 `**bar**` 匹配文件名"
        );
        assert_eq!(
            strip_markdown_emphasis_markers("Use **`foo`** and **before `*literal*` after**"),
            "Use `foo` and before `*literal*` after"
        );
        assert_eq!(
            strip_markdown_emphasis_markers(r"Regex /\*foo\*/ and glob *.config.*"),
            r"Regex /\*foo\*/ and glob *.config.*"
        );
        assert_eq!(
            strip_markdown_emphasis_markers("Use glob foo.*bar* or src/*test*"),
            "Use glob foo.*bar* or src/*test*"
        );
        assert_eq!(
            strip_markdown_emphasis_markers("Use glob *a/b?* or *[0-9]*"),
            "Use glob *a/b?* or *[0-9]*"
        );
        assert_eq!(
            strip_markdown_emphasis_markers("Markdown *a/b* and *[today]*"),
            "Markdown a/b and [today]"
        );
    }

    #[test]
    fn test_merge_continuation_does_not_split_cross_round_emphasis() {
        let mut normal = "**hello".to_string();
        merge_continuation_text(&mut normal, "world**");
        assert_eq!(normal, "**hello world**");
        assert_eq!(strip_markdown_emphasis_markers(&normal), "hello world");

        let mut merged = "**".to_string();
        merge_continuation_text(&mut merged, "answer**。");
        assert_eq!(merged, "**answer**。");
        assert_eq!(strip_markdown_emphasis_markers(&merged), "answer。");

        let mut escaped = r"\*\*".to_string();
        merge_continuation_text(&mut escaped, r"answer\*\*");
        assert_eq!(strip_markdown_emphasis_markers(&escaped), "answer");

        let mut italic = "*".to_string();
        merge_continuation_text(&mut italic, "answer*。");
        assert_eq!(italic, "*answer*。");
        assert_eq!(strip_markdown_emphasis_markers(&italic), "answer。");

        let mut escaped_italic = r"\*".to_string();
        merge_continuation_text(&mut escaped_italic, r"answer\*");
        assert_eq!(strip_markdown_emphasis_markers(&escaped_italic), "answer");

        let mut unfinished = "prefix **".to_string();
        merge_continuation_text(&mut unfinished, "answer");
        assert_eq!(unfinished, "prefix ** answer");
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
