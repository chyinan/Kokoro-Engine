// pattern: Mixed (unavoidable)
// Reason: This module combines pure extraction prompt/candidate helpers with the provider call and memory persistence shell.
//! Automatic memory extraction from conversation history.
//!
//! Every N conversation turns, the recent history is sent to the LLM
//! with a special prompt that asks it to extract noteworthy facts.
//! Extracted memories are stored via MemoryManager for future RAG retrieval.

use crate::ai::context::Message;
use crate::ai::memory::MemoryManager;
use crate::ai::memory_event_ingress::MemoryEventType;
use crate::llm::messages::{system_message, user_text_message};
use crate::llm::provider::LlmProvider;
use std::sync::Arc;

/// System prompt for the memory extraction LLM call.
const EXTRACTION_PROMPT: &str = concat!(
    "You are a memory extraction assistant. Analyze the following conversation ",
    "and extract any noteworthy facts worth remembering for future conversations.\n\n",
    "Extract facts such as:\n",
    "- User's name, preferences, hobbies, or personal details\n",
    "- Important events, dates, or plans mentioned\n",
    "- User's opinions or feelings about specific topics\n",
    "- Any commitments or promises made\n",
    "- Technical preferences or project details\n\n",
    "For each fact, assign an importance score from 0.0 to 1.0:\n",
    "- 0.9-1.0: Critical personal info (name, birthday, major life events)\n",
    "- 0.7-0.8: Strong preferences or important plans\n",
    "- 0.5-0.6: Interesting details or opinions\n",
    "- 0.3-0.4: Minor observations or casual mentions\n\n",
    "Respond with ONLY a JSON array of objects: [{\"fact\": \"...\", \"importance\": 0.8}]\n",
    "If nothing noteworthy was said, respond with [].\n\n",
    "IMPORTANT: Output ONLY the JSON array, no explanation or markdown."
);

#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryExtractionOptions {
    pub structured_memory_enabled: bool,
    pub focus_event: Option<MemoryEventType>,
}

/// A scored memory fact from the LLM.
#[derive(serde::Deserialize)]
struct ScoredFact {
    fact: String,
    importance: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct MemoryWriteCandidate {
    content: String,
    importance: Option<f64>,
}

pub fn build_memory_extraction_options(
    config: &crate::config::MemoryUpgradeConfig,
    focus_event: Option<MemoryEventType>,
) -> MemoryExtractionOptions {
    MemoryExtractionOptions {
        structured_memory_enabled: config.structured_memory_enabled,
        focus_event,
    }
}

fn build_extraction_prompt(existing_block: &str, options: MemoryExtractionOptions) -> String {
    let structured_block = if options.structured_memory_enabled {
        "\n\nKeep each memory candidate compact, standalone, and canonical. Prefer one fact per item."
    } else {
        ""
    };
    let focus_block = match options.focus_event {
        Some(event_type) => format!(
            "\n\nPrioritize {} and discard unrelated low-signal facts.",
            event_focus_instruction(event_type)
        ),
        None => String::new(),
    };

    format!(
        "{}{}{}{}",
        EXTRACTION_PROMPT, structured_block, focus_block, existing_block
    )
}

fn event_focus_instruction(event_type: MemoryEventType) -> &'static str {
    match event_type {
        MemoryEventType::Preference => "stable preferences and likes/dislikes",
        MemoryEventType::Correction => "corrections that update an existing memory",
        MemoryEventType::Plan => "future plans, commitments, and upcoming actions",
        MemoryEventType::Profile => "profile facts, identity, background, and role information",
    }
}

fn build_memory_write_candidates(
    response: &str,
    options: MemoryExtractionOptions,
) -> Vec<MemoryWriteCandidate> {
    let scored = parse_scored_response(response);
    if !scored.is_empty() {
        return scored
            .into_iter()
            .map(|item| MemoryWriteCandidate {
                content: item.fact,
                importance: options
                    .structured_memory_enabled
                    .then_some(item.importance.clamp(0.0, 1.0)),
            })
            .collect();
    }

    parse_plain_response(response)
        .into_iter()
        .map(|content| MemoryWriteCandidate {
            content,
            importance: None,
        })
        .collect()
}

fn focus_event_label(focus_event: Option<MemoryEventType>) -> &'static str {
    match focus_event {
        Some(event_type) => event_type.as_str(),
        None => "generic",
    }
}

/// Extracts memories from recent conversation history and stores them.
///
/// This function is designed to be called in a background task (fire-and-forget).
pub async fn extract_and_store_memories(
    recent_history: &[Message],
    memory_manager: &Arc<MemoryManager>,
    provider: Arc<dyn LlmProvider>,
    character_id: String,
    options: MemoryExtractionOptions,
) {
    if recent_history.is_empty() {
        tracing::info!(target: "memory", "[Memory] extract_and_store_memories called but history is empty");
        return;
    }

    tracing::info!(
        target: "memory",
        "[Memory] Starting extraction for '{}' with {} history messages (structured={}, focus={})",
        character_id,
        recent_history.len(),
        options.structured_memory_enabled,
        focus_event_label(options.focus_event)
    );

    // Fetch existing memories so the LLM can avoid duplicates
    let existing_memories = match memory_manager.get_all_memory_contents(&character_id).await {
        Ok(mems) => mems,
        Err(e) => {
            tracing::error!(target: "memory", "[Memory] Failed to fetch existing memories: {}", e);
            Vec::new()
        }
    };

    let existing_block = if existing_memories.is_empty() {
        String::new()
    } else {
        let list = existing_memories
            .iter()
            .map(|memory| format!("- {}", memory))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\nYou already have these memories stored. Do NOT extract facts that are already covered below (even if worded differently):\n{}",
            list
        )
    };

    // Build the conversation transcript for the LLM
    let transcript = recent_history
        .iter()
        .map(|message| format!("{}: {}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n");

    let messages = vec![
        system_message(build_extraction_prompt(&existing_block, options)),
        user_text_message(format!("Conversation to analyze:\n\n{}", transcript)),
    ];

    match provider.chat(messages, None).await {
        Ok(response) => {
            let candidates = build_memory_write_candidates(&response, options);
            if candidates.is_empty() {
                tracing::info!(target: "memory", "[Memory] No noteworthy facts extracted this round.");
                return;
            }

            let candidate_count = candidates.len();
            for candidate in candidates {
                let result = match candidate.importance {
                    Some(importance) => {
                        memory_manager
                            .add_memory_with_importance(&candidate.content, &character_id, importance)
                            .await
                    }
                    None => memory_manager.add_memory(&candidate.content, &character_id).await,
                };

                if let Err(error) = result {
                    tracing::error!(
                        target: "memory",
                        "[Memory] Failed to store memory '{}': {}",
                        candidate.content,
                        error
                    );
                }
            }

            tracing::info!(
                target: "memory",
                "[Memory] Extracted {} memories for '{}' (structured={}, focus={}).",
                candidate_count,
                character_id,
                options.structured_memory_enabled,
                focus_event_label(options.focus_event)
            );
        }
        Err(e) => {
            tracing::error!(target: "memory", "[Memory] Extraction LLM call failed: {}", e);
        }
    }
}

/// Parse the LLM response as scored facts: [{"fact": "...", "importance": 0.8}]
fn parse_scored_response(response: &str) -> Vec<ScoredFact> {
    let json_str = strip_code_fences(response);
    match serde_json::from_str::<Vec<ScoredFact>>(json_str) {
        Ok(items) => items
            .into_iter()
            .filter(|item| !item.fact.trim().is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Parse the LLM response as plain strings (backward compatible).
fn parse_plain_response(response: &str) -> Vec<String> {
    let json_str = strip_code_fences(response);
    match serde_json::from_str::<Vec<String>>(json_str) {
        Ok(items) => items.into_iter().filter(|item| !item.trim().is_empty()).collect(),
        Err(e) => {
            tracing::error!(
                target: "memory",
                "[Memory] Failed to parse extraction response: {}. Raw: {}",
                e,
                &response[..response.len().min(200)]
            );
            Vec::new()
        }
    }
}

/// Strip markdown code fences if present.
fn strip_code_fences(response: &str) -> &str {
    let trimmed = response.trim();
    if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    }
}
