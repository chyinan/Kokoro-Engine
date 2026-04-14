//! Automatic memory extraction from conversation history.
//!
//! Every N conversation turns, the recent history is sent to the LLM
//! with a special prompt that asks it to extract noteworthy facts.
//! Extracted memories are stored via MemoryManager for future RAG retrieval.

use crate::ai::context::Message;
use crate::ai::memory::MemoryManager;
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
}

/// A scored memory fact from the LLM.
#[derive(serde::Deserialize)]
struct ScoredFact {
    fact: String,
    importance: f64,
}

#[derive(serde::Deserialize)]
struct StructuredFact {
    fact: String,
    importance: f64,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    entity_key: Option<String>,
}

fn extraction_prompt(options: MemoryExtractionOptions) -> String {
    if options.structured_memory_enabled {
        concat!(
            "You are a memory extraction assistant. Analyze the following conversation and extract noteworthy facts worth remembering.\n\n",
            "Respond with ONLY a JSON array of objects in this schema:\n",
            "[{\"fact\":\"...\",\"importance\":0.8,\"memory_type\":\"profile|preference|plan|fact|constraint\",\"entity_key\":\"optional.entity.key\"}]\n",
            "If nothing noteworthy was said, respond with [].\n",
            "IMPORTANT: Output ONLY the JSON array, no explanation or markdown."
        )
        .to_string()
    } else {
        EXTRACTION_PROMPT.to_string()
    }
}

fn build_storage_content_from_structured_fact(fact: &StructuredFact) -> String {
    let mut tags = Vec::new();
    if let Some(memory_type) = &fact.memory_type {
        if !memory_type.trim().is_empty() {
            tags.push(format!("type:{}", memory_type.trim()));
        }
    }
    if let Some(entity_key) = &fact.entity_key {
        if !entity_key.trim().is_empty() {
            tags.push(format!("key:{}", entity_key.trim()));
        }
    }

    if tags.is_empty() {
        fact.fact.trim().to_string()
    } else {
        format!("[{}] {}", tags.join("|"), fact.fact.trim())
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
) {
    extract_and_store_memories_with_options(
        recent_history,
        memory_manager,
        provider,
        character_id,
        MemoryExtractionOptions::default(),
    )
    .await;
}

pub async fn extract_and_store_memories_with_options(
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
        "[Memory] Starting extraction for '{}' with {} history messages",
        character_id,
        recent_history.len()
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
            .map(|m| format!("- {}", m))
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
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let messages = vec![
        system_message(format!("{}{}", extraction_prompt(options), existing_block)),
        user_text_message(format!("Conversation to analyze:\n\n{}", transcript)),
    ];

    match provider.chat(messages, None).await {
        Ok(response) => {
            if options.structured_memory_enabled {
                let structured = parse_structured_response(&response);
                if !structured.is_empty() {
                    let count = structured.len();
                    for fact in structured {
                        let content = build_storage_content_from_structured_fact(&fact);
                        if let Err(e) = memory_manager
                            .add_memory_with_importance(&content, &character_id, fact.importance)
                            .await
                        {
                            tracing::error!(
                                target: "memory",
                                "[Memory] Failed to store structured memory '{}': {}",
                                content,
                                e
                            );
                        }
                    }
                    tracing::info!(
                        target: "memory",
                        "[Memory] Extracted {} structured memories for '{}'.",
                        count,
                        character_id
                    );
                    return;
                }
            }

            // Try scored format first, fall back to plain string array
            let scored = parse_scored_response(&response);
            if scored.is_empty() {
                // Fallback: try parsing as plain string array
                let plain = parse_plain_response(&response);
                if plain.is_empty() {
                    tracing::info!(target: "memory", "[Memory] No noteworthy facts extracted this round.");
                    return;
                }
                let count = plain.len();
                for memory in plain {
                    if let Err(e) = memory_manager.add_memory(&memory, &character_id).await {
                        tracing::error!(target: "memory", "[Memory] Failed to store memory '{}': {}", memory, e);
                    }
                }
                tracing::info!(
                    target: "memory",
                    "[Memory] Extracted {} memories (plain format) for '{}'.",
                    count, character_id
                );
            } else {
                let count = scored.len();
                for sf in scored {
                    if let Err(e) = memory_manager
                        .add_memory_with_importance(&sf.fact, &character_id, sf.importance)
                        .await
                    {
                        tracing::error!(
                            target: "memory",
                            "[Memory] Failed to store scored memory '{}': {}",
                            sf.fact, e
                        );
                    }
                }
                tracing::info!(
                    target: "memory",
                    "[Memory] Extracted {} scored memories for '{}'.",
                    count, character_id
                );
            }
        }
        Err(e) => {
            tracing::error!(target: "memory", "[Memory] Extraction LLM call failed: {}", e);
        }
    }
}

fn parse_structured_response(response: &str) -> Vec<StructuredFact> {
    let json_str = strip_code_fences(response);
    match serde_json::from_str::<Vec<StructuredFact>>(json_str) {
        Ok(items) => items
            .into_iter()
            .filter(|s| !s.fact.trim().is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Parse the LLM response as scored facts: [{"fact": "...", "importance": 0.8}]
fn parse_scored_response(response: &str) -> Vec<ScoredFact> {
    let json_str = strip_code_fences(response);
    match serde_json::from_str::<Vec<ScoredFact>>(json_str) {
        Ok(items) => items
            .into_iter()
            .filter(|s| !s.fact.trim().is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Parse the LLM response as plain strings (backward compatible).
fn parse_plain_response(response: &str) -> Vec<String> {
    let json_str = strip_code_fences(response);
    match serde_json::from_str::<Vec<String>>(json_str) {
        Ok(items) => items.into_iter().filter(|s| !s.trim().is_empty()).collect(),
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
