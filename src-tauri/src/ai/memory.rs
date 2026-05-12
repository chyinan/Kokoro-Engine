// pattern: Mixed (unavoidable)
// Reason: 该文件同时包含记忆领域规则、SQLite 读写、嵌入计算与摘要状态机；Phase 1 先在现有集中实现上做低侵入扩展。
use anyhow::Result;
#[cfg(not(test))]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
#[cfg(not(test))]
use tokio::sync::Mutex;

use crate::ai::context::MemorySnippet;

pub struct MemoryManager {
    #[cfg(not(test))]
    embedder: tokio::sync::OnceCell<Mutex<TextEmbedding>>,
    db: SqlitePool,
}

/// Half-life in days for memory decay (memories lose 50% relevance every N days).
const MEMORY_HALF_LIFE_DAYS: f64 = 30.0;

/// Cosine similarity threshold above which a new memory is considered a duplicate.
/// 0.95+ = near-identical wording; 0.85 was too aggressive and collapsed distinct memories
/// about the same topic/person into false duplicates.
const DEDUP_THRESHOLD: f32 = 0.95;

/// Cosine similarity threshold for memory consolidation clustering.
/// 0.85 requires strong topical overlap; 0.75 was too loose and merged unrelated topics.
const CONSOLIDATION_THRESHOLD: f32 = 0.85;

/// Maximum time gap (in seconds) between memories that can be consolidated together.
/// Memories created more than N days apart are unlikely to be about the same event.
const CONSOLIDATION_TIME_WINDOW_SECS: i64 = 7 * 24 * 3600; // 7 days

/// Maximum number of memories in a single consolidation cluster.
const MAX_CLUSTER_SIZE: usize = 5;

/// Minimum RRF score to be included in search results.
/// Prevents injecting completely irrelevant memories into the prompt.
/// At k=60, rank-1 score ≈ 0.0164, rank-60 ≈ 0.0083.
/// 0.008 filters out memories that appear only at the very bottom of one list.
const MIN_RRF_SCORE: f32 = 0.008;

/// Minimum cosine similarity (after time decay) for semantic search candidates.
/// Prevents semantically unrelated memories from entering the RRF pool at all.
const MIN_COSINE_SIMILARITY: f32 = 0.30;

/// Similarity band where memories are topically related but not duplicates —
/// potential contradictions (new fact vs old fact about same topic) live here.
/// CONTRADICTION_BAND_HIGH equals DEDUP_THRESHOLD; the range is exclusive at the
/// high end, so sim=0.95 falls in neither band — this is intentional.
const CONTRADICTION_BAND_LOW: f32 = 0.70;
const CONTRADICTION_BAND_HIGH: f32 = 0.95; // exclusive upper bound = DEDUP_THRESHOLD
const CONVERSATION_SUMMARY_MIN_MESSAGES: usize = 8;
const CONVERSATION_SUMMARY_MAX_MESSAGES: usize = 12;
const CONVERSATION_SUMMARY_FAILURE_THRESHOLD: i64 = 3;
const CONVERSATION_SUMMARY_COOLDOWN_SECS: i64 = 15 * 60;
const DREAM_SEMANTIC_AUTO_MERGE_THRESHOLD: f32 = 0.97;
const DREAM_SEMANTIC_REVIEW_THRESHOLD: f32 = 0.90;
const DREAM_CONFIDENCE_AUTO_APPLY: f64 = 0.88;
const DREAM_LLM_DISCOVERY_BATCH_SIZE: usize = 48;
const DREAM_LLM_DISCOVERY_BATCH_OVERLAP: usize = 8;
const DREAM_LLM_DISCOVERY_MIN_CONFIDENCE: f64 = 0.70;
const DREAM_LLM_DISCOVERY_MAX_PROPOSALS_PER_RUN: i64 = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryMetadata {
    memory_type: String,
    entity_key: Option<String>,
    canonical_content: Option<String>,
}

struct NewMemoryCandidate<'a> {
    content: &'a str,
    character_id: &'a str,
    importance: f64,
    confidence: f64,
    metadata: &'a MemoryMetadata,
    canonical_hash: &'a str,
    now: i64,
}

struct MemoryOperationRecord<'a> {
    character_id: &'a str,
    operation_type: &'a str,
    actor: &'a str,
    memory_id: Option<i64>,
    proposal_id: Option<i64>,
    before_json: Option<String>,
    after_json: Option<String>,
}

struct EntitySlotUpsert<'a> {
    content: &'a str,
    embedding_bytes: &'a [u8],
    character_id: &'a str,
    importance: f64,
    metadata: &'a MemoryMetadata,
    canonical_hash: &'a str,
    now: i64,
}

struct ActiveMemoryInsert<'a> {
    content: &'a str,
    embedding_bytes: Vec<u8>,
    character_id: &'a str,
    importance: f64,
    metadata: &'a MemoryMetadata,
    canonical_hash: &'a str,
    now: i64,
}

struct DreamProposalInsert<'a> {
    character_id: &'a str,
    proposal_type: &'a str,
    status: &'a str,
    confidence: f64,
    title: &'a str,
    rationale: &'a str,
    source_memory_ids: &'a [i64],
    target_memory_id: Option<i64>,
    proposed_content: Option<&'a str>,
    proposed_memory_type: Option<&'a str>,
    proposed_entity_key: Option<&'a str>,
    impact: &'a str,
}

struct DreamAutoMergeRequest<'a> {
    character_id: &'a str,
    entries: &'a [&'a DreamCandidateEntry],
    proposal_type: &'a str,
    title: &'a str,
    rationale: &'a str,
    confidence: f64,
    proposed_content_override: Option<&'a str>,
    preferred_keeper_id: Option<i64>,
}

fn normalize_memory_text(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn stable_hash64(input: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

fn canonical_hash(content: &str) -> String {
    stable_hash64(&normalize_memory_text(content))
}

fn strip_structured_memory_prefix(content: &str) -> &str {
    let trimmed = content.trim();
    if trimmed.starts_with('[') {
        if let Some(end) = trimmed.find("] ") {
            return trimmed[(end + 2)..].trim();
        }
    }
    trimmed
}

fn parse_structured_memory_metadata(content: &str) -> (Option<String>, Option<String>) {
    let trimmed = content.trim();
    if !trimmed.starts_with('[') {
        return (None, None);
    }
    let Some(end) = trimmed.find(']') else {
        return (None, None);
    };
    let tags = &trimmed[1..end];
    let mut memory_type = None;
    let mut entity_key = None;
    for tag in tags.split('|') {
        let Some((key, value)) = tag.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "type" => memory_type = Some(value.to_string()),
            "key" => entity_key = Some(value.to_string()),
            _ => {}
        }
    }
    (memory_type, entity_key)
}

fn short_key_fragment(value: &str) -> String {
    normalize_memory_text(value)
        .chars()
        .filter(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '-' || *ch == '.')
        .take(48)
        .collect::<String>()
}

fn infer_memory_metadata(content: &str) -> MemoryMetadata {
    let (structured_type, structured_key) = parse_structured_memory_metadata(content);
    let plain = strip_structured_memory_prefix(content);
    let normalized = normalize_memory_text(plain);

    if let Some(memory_type) = structured_type {
        return MemoryMetadata {
            memory_type,
            entity_key: structured_key,
            canonical_content: None,
        };
    }

    if normalized.contains("用户的名字")
        || normalized.contains("用户名字")
        || normalized.contains("user's name")
        || normalized.contains("user name")
        || normalized.contains("my name is")
        || normalized.contains("call me")
    {
        return MemoryMetadata {
            memory_type: "profile".to_string(),
            entity_key: Some("user.name".to_string()),
            canonical_content: None,
        };
    }

    if normalized.contains("喜欢")
        || normalized.contains("不喜欢")
        || normalized.contains("更喜欢")
        || normalized.contains("讨厌")
        || normalized.contains("prefer")
        || normalized.contains("i like")
        || normalized.contains("i dislike")
    {
        return MemoryMetadata {
            memory_type: "preference".to_string(),
            entity_key: Some(format!("preference.{}", short_key_fragment(plain))),
            canonical_content: None,
        };
    }

    if normalized.contains("计划")
        || normalized.contains("打算")
        || normalized.contains("明天")
        || normalized.contains("下周")
        || normalized.contains("plan to")
        || normalized.contains("i will")
    {
        return MemoryMetadata {
            memory_type: "plan".to_string(),
            entity_key: None,
            canonical_content: None,
        };
    }

    MemoryMetadata {
        memory_type: "fact".to_string(),
        entity_key: None,
        canonical_content: None,
    }
}

fn proposal_ids_json(ids: &[i64]) -> Result<String> {
    Ok(serde_json::to_string(ids)?)
}

fn parse_proposal_ids(raw: &str) -> Vec<i64> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn strip_code_fences_for_memory_json(response: &str) -> &str {
    let trimmed = response.trim();
    if trimmed.starts_with("```") {
        let after_open = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed)
            .trim_start_matches('\n')
            .trim_start();
        if let Some(end) = after_open.rfind("```") {
            return after_open[..end].trim();
        }
    }
    trimmed
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationSummaryStatus {
    Pending,
    Running,
    Ready,
    Failed,
    CircuitOpen,
}

impl ConversationSummaryStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::CircuitOpen => "circuit_open",
        }
    }

    fn from_db(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            "circuit_open" => Self::CircuitOpen,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConversationSummaryTask {
    pub record_id: i64,
    pub conversation_id: String,
    pub character_id: String,
    pub version: i64,
    pub start_message_id: i64,
    pub end_message_id: i64,
    pub transcript: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummaryRecord {
    pub conversation_id: String,
    pub version: i64,
    pub start_message_id: i64,
    pub end_message_id: i64,
    pub summary: String,
    pub status: ConversationSummaryStatus,
    pub failure_count: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryWriteObservation {
    pub character_id: String,
    pub source: String,
    pub trigger: String,
    pub extracted_count: i64,
    pub stored_count: i64,
    pub deduplicated_count: i64,
    pub invalidated_count: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalEvalMetrics {
    pub overlap_count: usize,
    pub semantic_only_count: usize,
    pub bm25_only_count: usize,
    pub filtered_out_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRetrievalObservation {
    pub character_id: String,
    pub query: String,
    pub semantic_candidates: i64,
    pub bm25_candidates: i64,
    pub fused_candidates: i64,
    pub injected_count: i64,
    pub overlap_count: Option<i64>,
    pub semantic_only_count: Option<i64>,
    pub bm25_only_count: Option<i64>,
    pub filtered_out_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::FromRow)]
pub struct MemoryRetrievalLogRecord {
    pub query: String,
    pub semantic_candidates: i64,
    pub bm25_candidates: i64,
    pub fused_candidates: i64,
    pub injected_count: i64,
    pub overlap_count: Option<i64>,
    pub semantic_only_count: Option<i64>,
    pub bm25_only_count: Option<i64>,
    pub filtered_out_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRetrievalEvalSummary {
    pub retrieval_eval_enabled: bool,
    pub query_length: i64,
    pub candidate_efficiency_pct: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryObservabilitySummary {
    pub write_event_count: i64,
    pub retrieval_log_count: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetrievalCandidateStats {
    pub semantic_candidates: usize,
    pub bm25_candidates: usize,
    pub fused_candidates: usize,
    pub injected_count: usize,
    pub eval_metrics: Option<RetrievalEvalMetrics>,
}

fn build_retrieval_eval_summary(log: &MemoryRetrievalLogRecord) -> MemoryRetrievalEvalSummary {
    let fused_candidates = log.fused_candidates.max(0);
    let injected_count = log.injected_count.max(0);
    let candidate_efficiency_pct = if fused_candidates == 0 {
        0
    } else {
        ((injected_count as f64 / fused_candidates as f64) * 100.0).round() as i64
    };

    MemoryRetrievalEvalSummary {
        retrieval_eval_enabled: is_retrieval_eval_enabled(),
        query_length: log.query.chars().count() as i64,
        candidate_efficiency_pct,
    }
}

fn build_retrieval_observation(
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> MemoryRetrievalObservation {
    let eval_metrics = stats.eval_metrics.as_ref();
    MemoryRetrievalObservation {
        character_id: character_id.to_string(),
        query: query.to_string(),
        semantic_candidates: stats.semantic_candidates as i64,
        bm25_candidates: stats.bm25_candidates as i64,
        fused_candidates: stats.fused_candidates as i64,
        injected_count: stats.injected_count as i64,
        overlap_count: eval_metrics.map(|metrics| metrics.overlap_count as i64),
        semantic_only_count: eval_metrics.map(|metrics| metrics.semantic_only_count as i64),
        bm25_only_count: eval_metrics.map(|metrics| metrics.bm25_only_count as i64),
        filtered_out_count: eval_metrics.map(|metrics| metrics.filtered_out_count as i64),
    }
}

fn validate_memory_write_observation(
    observation: &MemoryWriteObservation,
) -> std::result::Result<(), anyhow::Error> {
    if observation.source.trim().is_empty() {
        anyhow::bail!("memory write observation source cannot be empty");
    }
    if observation.trigger.trim().is_empty() {
        anyhow::bail!("memory write observation trigger cannot be empty");
    }
    if observation.character_id.trim().is_empty() {
        anyhow::bail!("memory write observation character_id cannot be empty");
    }
    if observation.extracted_count < 0
        || observation.stored_count < 0
        || observation.deduplicated_count < 0
        || observation.invalidated_count < 0
        || observation.duration_ms < 0
    {
        anyhow::bail!("memory write observation counts must be non-negative");
    }

    Ok(())
}

fn validate_memory_retrieval_observation(
    observation: &MemoryRetrievalObservation,
) -> std::result::Result<(), anyhow::Error> {
    if observation.character_id.trim().is_empty() {
        anyhow::bail!("memory retrieval observation character_id cannot be empty");
    }
    if observation.query.trim().is_empty() {
        anyhow::bail!("memory retrieval observation query cannot be empty");
    }
    if observation.semantic_candidates < 0
        || observation.bm25_candidates < 0
        || observation.fused_candidates < 0
        || observation.injected_count < 0
    {
        anyhow::bail!("memory retrieval observation counts must be non-negative");
    }
    assert_optional_non_negative_i64(observation.overlap_count, "overlap_count")?;
    assert_optional_non_negative_i64(observation.semantic_only_count, "semantic_only_count")?;
    assert_optional_non_negative_i64(observation.bm25_only_count, "bm25_only_count")?;
    assert_optional_non_negative_i64(observation.filtered_out_count, "filtered_out_count")?;

    if let Some(overlap_count) = observation.overlap_count {
        if overlap_count > observation.semantic_candidates
            || overlap_count > observation.bm25_candidates
        {
            anyhow::bail!("memory retrieval overlap_count cannot exceed semantic/bm25 candidates");
        }
    }
    if let (Some(overlap_count), Some(semantic_only_count)) =
        (observation.overlap_count, observation.semantic_only_count)
    {
        if semantic_only_count + overlap_count > observation.semantic_candidates {
            anyhow::bail!(
                "memory retrieval semantic eval counts cannot exceed semantic_candidates"
            );
        }
    }
    if let (Some(overlap_count), Some(bm25_only_count)) =
        (observation.overlap_count, observation.bm25_only_count)
    {
        if bm25_only_count + overlap_count > observation.bm25_candidates {
            anyhow::bail!("memory retrieval bm25 eval counts cannot exceed bm25_candidates");
        }
    }
    if let Some(filtered_out_count) = observation.filtered_out_count {
        if filtered_out_count > observation.fused_candidates {
            anyhow::bail!("memory retrieval filtered_out_count cannot exceed fused_candidates");
        }
    }

    Ok(())
}

fn merge_retrieval_candidate_stats(
    semantic_candidates: usize,
    bm25_candidates: usize,
    fused_candidates: usize,
    injected_count: usize,
) -> RetrievalCandidateStats {
    RetrievalCandidateStats {
        semantic_candidates,
        bm25_candidates,
        fused_candidates,
        injected_count,
        eval_metrics: None,
    }
}

#[derive(Debug, Clone)]
struct SearchMemoriesOutcome {
    snippets: Vec<MemorySnippet>,
}

impl SearchMemoriesOutcome {
    fn new(snippets: Vec<MemorySnippet>) -> Self {
        Self { snippets }
    }
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct MemoryWriteEventRecord {
    pub source: String,
    pub trigger: String,
    pub extracted_count: i64,
    pub stored_count: i64,
    pub deduplicated_count: i64,
    pub invalidated_count: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct MemoryDreamJobRecord {
    pub id: i64,
    pub character_id: String,
    pub phase: String,
    pub status: String,
    pub trigger: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub auto_applied_count: i64,
    pub proposal_count: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct MemoryDreamSourceRecord {
    pub id: i64,
    pub content: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub importance: f64,
    pub tier: String,
    pub memory_type: String,
    pub entity_key: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct MemoryDreamProposalRow {
    pub id: i64,
    pub character_id: String,
    pub proposal_type: String,
    pub status: String,
    pub confidence: f64,
    pub title: String,
    pub rationale: String,
    pub source_memory_ids: String,
    pub target_memory_id: Option<i64>,
    pub proposed_content: Option<String>,
    pub proposed_memory_type: Option<String>,
    pub proposed_entity_key: Option<String>,
    pub impact: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub applied_at: Option<i64>,
}

impl MemoryDreamProposalRow {
    fn into_record(
        self,
        source_memories: Vec<MemoryDreamSourceRecord>,
    ) -> MemoryDreamProposalRecord {
        MemoryDreamProposalRecord {
            id: self.id,
            character_id: self.character_id,
            proposal_type: self.proposal_type,
            status: self.status,
            confidence: self.confidence,
            title: self.title,
            rationale: self.rationale,
            source_memory_ids: self.source_memory_ids,
            source_memories,
            target_memory_id: self.target_memory_id,
            proposed_content: self.proposed_content,
            proposed_memory_type: self.proposed_memory_type,
            proposed_entity_key: self.proposed_entity_key,
            impact: self.impact,
            created_at: self.created_at,
            updated_at: self.updated_at,
            applied_at: self.applied_at,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryDreamProposalRecord {
    pub id: i64,
    pub character_id: String,
    pub proposal_type: String,
    pub status: String,
    pub confidence: f64,
    pub title: String,
    pub rationale: String,
    pub source_memory_ids: String,
    pub source_memories: Vec<MemoryDreamSourceRecord>,
    pub target_memory_id: Option<i64>,
    pub proposed_content: Option<String>,
    pub proposed_memory_type: Option<String>,
    pub proposed_entity_key: Option<String>,
    pub impact: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub applied_at: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryDreamingSummary {
    pub latest_job: Option<MemoryDreamJobRecord>,
    pub pending_proposal_count: i64,
    pub auto_applied_proposal_count: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryDreamRunResult {
    pub job: MemoryDreamJobRecord,
    pub auto_applied_count: i64,
    pub proposal_count: i64,
}

#[derive(Debug, Clone)]
struct DreamCandidateEntry {
    id: i64,
    content: String,
    embedding: Vec<f32>,
    created_at: i64,
    updated_at: i64,
    importance: f64,
    tier: String,
    memory_type: String,
    entity_key: Option<String>,
    canonical_hash: Option<String>,
    evidence_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct DreamPairAssessment {
    decision: String,
    confidence: f64,
    #[serde(default)]
    canonical_memory_id: Option<i64>,
    #[serde(default)]
    merged_memory: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    entity_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DreamDiscoveryProposal {
    #[serde(default)]
    source_memory_ids: Vec<i64>,
    #[serde(default)]
    decision: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    canonical_memory_id: Option<i64>,
    #[serde(default)]
    proposed_content: Option<String>,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    entity_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DreamDiscoveryResponse {
    #[serde(default)]
    proposals: Vec<DreamDiscoveryProposal>,
}

fn is_observability_enabled() -> bool {
    crate::config::load_memory_upgrade_config(&memory_upgrade_config_path()).observability_enabled
}

fn is_retrieval_eval_enabled() -> bool {
    crate::config::load_memory_upgrade_config(&memory_upgrade_config_path()).retrieval_eval_enabled
}

pub fn memory_upgrade_config_path() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("memory_upgrade_config.json")
}

pub fn build_periodic_memory_write_observation(
    character_id: &str,
    source: &str,
    trigger: &str,
    duration_ms: i64,
) -> MemoryWriteObservation {
    MemoryWriteObservation {
        character_id: character_id.to_string(),
        source: source.to_string(),
        trigger: trigger.to_string(),
        extracted_count: 0,
        stored_count: 0,
        deduplicated_count: 0,
        invalidated_count: 0,
        duration_ms,
    }
}

pub fn build_periodic_retrieval_observation(
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> MemoryRetrievalObservation {
    build_retrieval_observation(character_id, query, stats)
}

fn build_memory_observability_summary(
    write_event_count: i64,
    retrieval_log_count: i64,
) -> MemoryObservabilitySummary {
    MemoryObservabilitySummary {
        write_event_count,
        retrieval_log_count,
    }
}

fn normalize_trigger_value(trigger: &str) -> String {
    trigger.trim().to_string()
}

fn normalize_source_value(source: &str) -> String {
    source.trim().to_string()
}

fn normalize_query_value(query: &str) -> String {
    query.trim().to_string()
}

fn normalize_character_id_value(character_id: &str) -> String {
    character_id.trim().to_string()
}

fn sanitize_write_observation(mut observation: MemoryWriteObservation) -> MemoryWriteObservation {
    observation.source = normalize_source_value(&observation.source);
    observation.trigger = normalize_trigger_value(&observation.trigger);
    observation.character_id = normalize_character_id_value(&observation.character_id);
    observation
}

fn sanitize_retrieval_observation(
    mut observation: MemoryRetrievalObservation,
) -> MemoryRetrievalObservation {
    observation.query = normalize_query_value(&observation.query);
    observation.character_id = normalize_character_id_value(&observation.character_id);
    observation
}

fn build_memory_write_event_record(observation: &MemoryWriteObservation) -> MemoryWriteEventRecord {
    MemoryWriteEventRecord {
        source: observation.source.clone(),
        trigger: observation.trigger.clone(),
        extracted_count: observation.extracted_count,
        stored_count: observation.stored_count,
        deduplicated_count: observation.deduplicated_count,
        invalidated_count: observation.invalidated_count,
        duration_ms: observation.duration_ms,
    }
}

fn build_memory_retrieval_log_record(
    observation: &MemoryRetrievalObservation,
) -> MemoryRetrievalLogRecord {
    MemoryRetrievalLogRecord {
        query: observation.query.clone(),
        semantic_candidates: observation.semantic_candidates,
        bm25_candidates: observation.bm25_candidates,
        fused_candidates: observation.fused_candidates,
        injected_count: observation.injected_count,
        overlap_count: observation.overlap_count,
        semantic_only_count: observation.semantic_only_count,
        bm25_only_count: observation.bm25_only_count,
        filtered_out_count: observation.filtered_out_count,
    }
}

fn should_record_memory_observability() -> bool {
    is_observability_enabled()
}

fn assert_non_negative_i64(value: i64, field_name: &str) -> std::result::Result<(), anyhow::Error> {
    if value < 0 {
        anyhow::bail!("{} must be non-negative", field_name);
    }
    Ok(())
}

fn assert_optional_non_negative_i64(
    value: Option<i64>,
    field_name: &str,
) -> std::result::Result<(), anyhow::Error> {
    if let Some(value) = value {
        assert_non_negative_i64(value, field_name)?;
    }
    Ok(())
}

fn validate_retrieval_stats(
    stats: &RetrievalCandidateStats,
) -> std::result::Result<(), anyhow::Error> {
    if stats.injected_count > stats.fused_candidates {
        anyhow::bail!("retrieval injected_count cannot exceed fused_candidates");
    }
    if let Some(eval_metrics) = &stats.eval_metrics {
        if eval_metrics.overlap_count > stats.semantic_candidates
            || eval_metrics.overlap_count > stats.bm25_candidates
        {
            anyhow::bail!("retrieval overlap_count cannot exceed semantic/bm25 candidates");
        }
        if eval_metrics.semantic_only_count + eval_metrics.overlap_count > stats.semantic_candidates
        {
            anyhow::bail!("retrieval semantic eval counts cannot exceed semantic_candidates");
        }
        if eval_metrics.bm25_only_count + eval_metrics.overlap_count > stats.bm25_candidates {
            anyhow::bail!("retrieval bm25 eval counts cannot exceed bm25_candidates");
        }
        if eval_metrics.filtered_out_count > stats.fused_candidates {
            anyhow::bail!("retrieval filtered_out_count cannot exceed fused_candidates");
        }
    }
    Ok(())
}

fn build_retrieval_stats_from_lengths(
    semantic_candidates: usize,
    bm25_candidates: usize,
    fused_candidates: usize,
    injected_count: usize,
) -> RetrievalCandidateStats {
    merge_retrieval_candidate_stats(
        semantic_candidates,
        bm25_candidates,
        fused_candidates,
        injected_count,
    )
}

fn validate_write_result_counts(
    extracted_count: i64,
    stored_count: i64,
    deduplicated_count: i64,
    invalidated_count: i64,
) -> std::result::Result<(), anyhow::Error> {
    assert_non_negative_i64(extracted_count, "extracted_count")?;
    assert_non_negative_i64(stored_count, "stored_count")?;
    assert_non_negative_i64(deduplicated_count, "deduplicated_count")?;
    assert_non_negative_i64(invalidated_count, "invalidated_count")?;
    Ok(())
}

fn build_observation_duration(started_at: std::time::Instant) -> i64 {
    started_at.elapsed().as_millis() as i64
}

fn build_periodic_source_label(source: &str) -> String {
    normalize_source_value(source)
}

fn build_periodic_trigger_label(trigger: &str) -> String {
    normalize_trigger_value(trigger)
}

fn build_periodic_character_label(character_id: &str) -> String {
    normalize_character_id_value(character_id)
}

#[derive(Debug, Clone, Copy)]
struct WriteObservationCounts {
    extracted_count: i64,
    stored_count: i64,
    deduplicated_count: i64,
    invalidated_count: i64,
    duration_ms: i64,
}

fn build_memory_write_observation_from_counts(
    character_id: &str,
    source: &str,
    trigger: &str,
    counts: WriteObservationCounts,
) -> MemoryWriteObservation {
    MemoryWriteObservation {
        character_id: build_periodic_character_label(character_id),
        source: build_periodic_source_label(source),
        trigger: build_periodic_trigger_label(trigger),
        extracted_count: counts.extracted_count,
        stored_count: counts.stored_count,
        deduplicated_count: counts.deduplicated_count,
        invalidated_count: counts.invalidated_count,
        duration_ms: counts.duration_ms,
    }
}

fn retrieval_stats_injected_count(snippets: &[MemorySnippet]) -> usize {
    snippets.len()
}

fn bm25_candidate_count(matches: &[(i64, f64)]) -> usize {
    matches.len()
}

fn semantic_candidate_count(snippets: &[MemorySnippet]) -> usize {
    snippets.len()
}

fn fused_candidate_count(fused: &[(f32, MemorySnippet)]) -> usize {
    fused.len()
}

fn clamp_observation_duration(duration_ms: i64) -> i64 {
    duration_ms.max(0)
}

fn build_clamped_memory_write_observation(
    observation: MemoryWriteObservation,
) -> MemoryWriteObservation {
    let mut observation = observation;
    observation.duration_ms = clamp_observation_duration(observation.duration_ms);
    observation
}

fn build_clamped_memory_retrieval_observation(
    observation: MemoryRetrievalObservation,
) -> MemoryRetrievalObservation {
    observation
}

fn validate_memory_observability_summary(
    summary: &MemoryObservabilitySummary,
) -> std::result::Result<(), anyhow::Error> {
    assert_non_negative_i64(summary.write_event_count, "write_event_count")?;
    assert_non_negative_i64(summary.retrieval_log_count, "retrieval_log_count")?;
    Ok(())
}

fn summarize_memory_observability_counts(
    write_event_count: i64,
    retrieval_log_count: i64,
) -> std::result::Result<MemoryObservabilitySummary, anyhow::Error> {
    let summary = build_memory_observability_summary(write_event_count, retrieval_log_count);
    validate_memory_observability_summary(&summary)?;
    Ok(summary)
}

fn build_observation_trigger_for_periodic_extraction() -> &'static str {
    "periodic_extraction"
}

fn build_observation_trigger_for_periodic_consolidation() -> &'static str {
    "periodic_consolidation"
}

fn build_observation_source_for_chat() -> &'static str {
    "chat"
}

fn build_observation_source_for_telegram() -> &'static str {
    "telegram"
}

fn build_retrieval_observation_record(
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> std::result::Result<MemoryRetrievalObservation, anyhow::Error> {
    validate_retrieval_stats(stats)?;
    let observation = build_retrieval_observation(character_id, query, stats);
    let observation = sanitize_retrieval_observation(observation);
    validate_memory_retrieval_observation(&observation)?;
    Ok(build_clamped_memory_retrieval_observation(observation))
}

fn build_write_observation_record(
    character_id: &str,
    source: &str,
    trigger: &str,
    counts: WriteObservationCounts,
) -> std::result::Result<MemoryWriteObservation, anyhow::Error> {
    validate_write_result_counts(
        counts.extracted_count,
        counts.stored_count,
        counts.deduplicated_count,
        counts.invalidated_count,
    )?;
    let observation =
        build_memory_write_observation_from_counts(character_id, source, trigger, counts);
    let observation = sanitize_write_observation(observation);
    validate_memory_write_observation(&observation)?;
    Ok(build_clamped_memory_write_observation(observation))
}

fn build_observation_summary_from_rows(
    write_event_count: i64,
    retrieval_log_count: i64,
) -> std::result::Result<MemoryObservabilitySummary, anyhow::Error> {
    summarize_memory_observability_counts(write_event_count, retrieval_log_count)
}

fn build_retrieval_stats_for_search(
    semantic_candidates: usize,
    bm25_candidates: usize,
    fused_candidates: usize,
    injected_count: usize,
) -> std::result::Result<RetrievalCandidateStats, anyhow::Error> {
    let stats = build_retrieval_stats_from_lengths(
        semantic_candidates,
        bm25_candidates,
        fused_candidates,
        injected_count,
    );
    validate_retrieval_stats(&stats)?;
    Ok(stats)
}

fn observation_duration_from_start(started_at: std::time::Instant) -> i64 {
    clamp_observation_duration(build_observation_duration(started_at))
}

fn build_memory_write_record(
    observation: &MemoryWriteObservation,
) -> std::result::Result<MemoryWriteEventRecord, anyhow::Error> {
    validate_memory_write_observation(observation)?;
    Ok(build_memory_write_event_record(observation))
}

fn build_memory_retrieval_record(
    observation: &MemoryRetrievalObservation,
) -> std::result::Result<MemoryRetrievalLogRecord, anyhow::Error> {
    validate_memory_retrieval_observation(observation)?;
    Ok(build_memory_retrieval_log_record(observation))
}

fn build_memory_observability_summary_checked(
    write_event_count: i64,
    retrieval_log_count: i64,
) -> std::result::Result<MemoryObservabilitySummary, anyhow::Error> {
    build_observation_summary_from_rows(write_event_count, retrieval_log_count)
}

fn build_retrieval_stats_from_results(
    semantic_results: &[MemorySnippet],
    bm25_results: &[(i64, f64)],
    fused_results: &[(f32, MemorySnippet)],
    injected_results: &[MemorySnippet],
) -> std::result::Result<RetrievalCandidateStats, anyhow::Error> {
    build_retrieval_stats_for_search(
        semantic_candidate_count(semantic_results),
        bm25_candidate_count(bm25_results),
        fused_candidate_count(fused_results),
        retrieval_stats_injected_count(injected_results),
    )
}

fn build_observability_duration_ms(started_at: std::time::Instant) -> i64 {
    observation_duration_from_start(started_at)
}

fn build_default_memory_observability_summary() -> MemoryObservabilitySummary {
    MemoryObservabilitySummary {
        write_event_count: 0,
        retrieval_log_count: 0,
    }
}

fn maybe_build_periodic_write_observation(
    character_id: &str,
    source: &str,
    trigger: &str,
    duration_ms: i64,
) -> std::result::Result<Option<MemoryWriteObservation>, anyhow::Error> {
    if !should_record_memory_observability() {
        return Ok(None);
    }
    Ok(Some(build_write_observation_record(
        character_id,
        source,
        trigger,
        WriteObservationCounts {
            extracted_count: 0,
            stored_count: 0,
            deduplicated_count: 0,
            invalidated_count: 0,
            duration_ms,
        },
    )?))
}

fn maybe_build_retrieval_observation(
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> std::result::Result<Option<MemoryRetrievalObservation>, anyhow::Error> {
    if !should_record_memory_observability() {
        return Ok(None);
    }
    Ok(Some(build_retrieval_observation_record(
        character_id,
        query,
        stats,
    )?))
}

fn build_summary_from_counts_or_default(
    write_event_count: i64,
    retrieval_log_count: i64,
) -> MemoryObservabilitySummary {
    build_memory_observability_summary_checked(write_event_count, retrieval_log_count)
        .unwrap_or_else(|_| build_default_memory_observability_summary())
}

fn is_empty_query(query: &str) -> bool {
    query.trim().is_empty()
}

fn is_empty_label(label: &str) -> bool {
    label.trim().is_empty()
}

fn build_memory_retrieval_log(
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> std::result::Result<Option<MemoryRetrievalObservation>, anyhow::Error> {
    if is_empty_query(query) {
        return Ok(None);
    }
    maybe_build_retrieval_observation(character_id, query, stats)
}

fn build_memory_write_log(
    character_id: &str,
    source: &str,
    trigger: &str,
    duration_ms: i64,
) -> std::result::Result<Option<MemoryWriteObservation>, anyhow::Error> {
    if is_empty_label(source) || is_empty_label(trigger) {
        return Ok(None);
    }
    maybe_build_periodic_write_observation(character_id, source, trigger, duration_ms)
}

fn observation_summary_from_manager_counts(
    write_event_count: i64,
    retrieval_log_count: i64,
) -> MemoryObservabilitySummary {
    build_summary_from_counts_or_default(write_event_count, retrieval_log_count)
}

fn build_retrieval_eval_metrics(
    semantic_results: &[MemorySnippet],
    bm25_results: &[(i64, f64)],
    fused_results: &[(f32, MemorySnippet)],
    injected_results: &[MemorySnippet],
) -> RetrievalEvalMetrics {
    let semantic_ids: std::collections::HashSet<i64> =
        semantic_results.iter().map(|memory| memory.id).collect();
    let bm25_ids: std::collections::HashSet<i64> = bm25_results.iter().map(|(id, _)| *id).collect();

    RetrievalEvalMetrics {
        overlap_count: semantic_ids.intersection(&bm25_ids).count(),
        semantic_only_count: semantic_ids.difference(&bm25_ids).count(),
        bm25_only_count: bm25_ids.difference(&semantic_ids).count(),
        filtered_out_count: fused_results.len().saturating_sub(injected_results.len()),
    }
}

fn build_stats_for_current_results(
    semantic_results: &[MemorySnippet],
    bm25_results: &[(i64, f64)],
    fused_results: &[(f32, MemorySnippet)],
    injected_results: &[MemorySnippet],
    retrieval_eval_enabled: bool,
) -> RetrievalCandidateStats {
    let mut stats = build_retrieval_stats_from_results(
        semantic_results,
        bm25_results,
        fused_results,
        injected_results,
    )
    .unwrap_or_default();

    if retrieval_eval_enabled {
        stats.eval_metrics = Some(build_retrieval_eval_metrics(
            semantic_results,
            bm25_results,
            fused_results,
            injected_results,
        ));
    }

    stats
}

fn periodic_source_for_chat() -> &'static str {
    build_observation_source_for_chat()
}

fn periodic_source_for_telegram() -> &'static str {
    build_observation_source_for_telegram()
}

fn periodic_trigger_for_extraction() -> &'static str {
    build_observation_trigger_for_periodic_extraction()
}

fn periodic_trigger_for_consolidation() -> &'static str {
    build_observation_trigger_for_periodic_consolidation()
}

fn build_periodic_write_duration(started_at: std::time::Instant) -> i64 {
    build_observability_duration_ms(started_at)
}

fn build_observability_summary_or_default(
    write_event_count: i64,
    retrieval_log_count: i64,
) -> MemoryObservabilitySummary {
    observation_summary_from_manager_counts(write_event_count, retrieval_log_count)
}

fn validate_observability_log_inputs(
    character_id: &str,
    source: &str,
    trigger: &str,
) -> std::result::Result<(), anyhow::Error> {
    if is_empty_label(character_id) {
        anyhow::bail!("memory observability character_id cannot be empty");
    }
    if is_empty_label(source) {
        anyhow::bail!("memory observability source cannot be empty");
    }
    if is_empty_label(trigger) {
        anyhow::bail!("memory observability trigger cannot be empty");
    }
    Ok(())
}

fn validate_observability_query_inputs(
    character_id: &str,
    query: &str,
) -> std::result::Result<(), anyhow::Error> {
    if is_empty_label(character_id) {
        anyhow::bail!("memory observability character_id cannot be empty");
    }
    if is_empty_query(query) {
        anyhow::bail!("memory observability query cannot be empty");
    }
    Ok(())
}

fn should_skip_observability_insert() -> bool {
    !should_record_memory_observability()
}

fn build_write_observation_if_enabled(
    character_id: &str,
    source: &str,
    trigger: &str,
    duration_ms: i64,
) -> std::result::Result<Option<MemoryWriteObservation>, anyhow::Error> {
    validate_observability_log_inputs(character_id, source, trigger)?;
    if should_skip_observability_insert() {
        return Ok(None);
    }
    build_memory_write_log(character_id, source, trigger, duration_ms)
}

fn build_retrieval_observation_if_enabled(
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> std::result::Result<Option<MemoryRetrievalObservation>, anyhow::Error> {
    validate_observability_query_inputs(character_id, query)?;
    if should_skip_observability_insert() {
        return Ok(None);
    }
    build_memory_retrieval_log(character_id, query, stats)
}

fn build_periodic_write_event(
    character_id: &str,
    source: &str,
    trigger: &str,
    started_at: std::time::Instant,
) -> std::result::Result<Option<MemoryWriteObservation>, anyhow::Error> {
    build_write_observation_if_enabled(
        character_id,
        source,
        trigger,
        build_periodic_write_duration(started_at),
    )
}

fn build_observability_summary_from_counts(
    write_event_count: i64,
    retrieval_log_count: i64,
) -> MemoryObservabilitySummary {
    build_observability_summary_or_default(write_event_count, retrieval_log_count)
}

fn build_search_outcome(snippets: Vec<MemorySnippet>) -> SearchMemoriesOutcome {
    SearchMemoriesOutcome::new(snippets)
}

fn compare_scored_memory(
    left_score: f32,
    left: &MemorySnippet,
    right_score: f32,
    right: &MemorySnippet,
) -> Ordering {
    right_score
        .partial_cmp(&left_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            right
                .importance
                .partial_cmp(&left.importance)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| right.created_at.cmp(&left.created_at))
        .then_with(|| left.id.cmp(&right.id))
}

fn build_search_stats(
    semantic_results: &[MemorySnippet],
    bm25_results: &[(i64, f64)],
    fused_results: &[(f32, MemorySnippet)],
    injected_results: &[MemorySnippet],
    retrieval_eval_enabled: bool,
) -> RetrievalCandidateStats {
    build_stats_for_current_results(
        semantic_results,
        bm25_results,
        fused_results,
        injected_results,
        retrieval_eval_enabled,
    )
}

fn build_current_write_observation(
    character_id: &str,
    source: &str,
    trigger: &str,
    started_at: std::time::Instant,
) -> std::result::Result<Option<MemoryWriteObservation>, anyhow::Error> {
    build_periodic_write_event(character_id, source, trigger, started_at)
}

fn build_current_retrieval_observation(
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> std::result::Result<Option<MemoryRetrievalObservation>, anyhow::Error> {
    build_retrieval_observation_if_enabled(character_id, query, stats)
}

async fn build_memory_observability_counts(
    manager: &MemoryManager,
) -> Result<MemoryObservabilitySummary> {
    let write_event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_write_events")
        .fetch_one(&manager.db)
        .await?;
    let retrieval_log_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_retrieval_logs")
        .fetch_one(&manager.db)
        .await?;
    Ok(build_observability_summary_from_counts(
        write_event_count,
        retrieval_log_count,
    ))
}

async fn insert_memory_write_event(
    manager: &MemoryManager,
    observation: MemoryWriteObservation,
) -> Result<()> {
    let record = build_memory_write_record(&observation)?;
    sqlx::query(
        "INSERT INTO memory_write_events (character_id, source, trigger, extracted_count, stored_count, deduplicated_count, invalidated_count, duration_ms, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(observation.character_id)
    .bind(record.source)
    .bind(record.trigger)
    .bind(record.extracted_count)
    .bind(record.stored_count)
    .bind(record.deduplicated_count)
    .bind(record.invalidated_count)
    .bind(record.duration_ms)
    .bind(chrono::Utc::now().timestamp())
    .execute(&manager.db)
    .await?;
    Ok(())
}

async fn insert_memory_retrieval_log(
    manager: &MemoryManager,
    observation: MemoryRetrievalObservation,
) -> Result<()> {
    let record = build_memory_retrieval_record(&observation)?;
    sqlx::query(
        "INSERT INTO memory_retrieval_logs (character_id, query, semantic_candidates, bm25_candidates, fused_candidates, injected_count, overlap_count, semantic_only_count, bm25_only_count, filtered_out_count, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(observation.character_id)
    .bind(record.query)
    .bind(record.semantic_candidates)
    .bind(record.bm25_candidates)
    .bind(record.fused_candidates)
    .bind(record.injected_count)
    .bind(chrono::Utc::now().timestamp())
    .execute(&manager.db)
    .await?;
    Ok(())
}

async fn fetch_latest_memory_write_event(
    manager: &MemoryManager,
) -> Result<Option<MemoryWriteEventRecord>> {
    let row = sqlx::query_as::<_, MemoryWriteEventRecord>(
        "SELECT source, trigger, extracted_count, stored_count, deduplicated_count, invalidated_count, duration_ms FROM memory_write_events ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&manager.db)
    .await?;
    Ok(row)
}

async fn fetch_latest_memory_retrieval_log(
    manager: &MemoryManager,
) -> Result<Option<MemoryRetrievalLogRecord>> {
    let row = sqlx::query_as::<_, MemoryRetrievalLogRecord>(
        "SELECT query, semantic_candidates, bm25_candidates, fused_candidates, injected_count, overlap_count, semantic_only_count, bm25_only_count, filtered_out_count FROM memory_retrieval_logs ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&manager.db)
    .await?;
    Ok(row)
}

async fn record_memory_write_if_enabled(
    manager: &MemoryManager,
    character_id: &str,
    source: &str,
    trigger: &str,
    started_at: std::time::Instant,
) -> Result<()> {
    if let Some(observation) =
        build_current_write_observation(character_id, source, trigger, started_at)?
    {
        insert_memory_write_event(manager, observation).await?;
    }
    Ok(())
}

async fn record_memory_retrieval_if_enabled(
    manager: &MemoryManager,
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> Result<()> {
    if let Some(observation) = build_current_retrieval_observation(character_id, query, stats)? {
        insert_memory_retrieval_log(manager, observation).await?;
    }
    Ok(())
}

/// Local model directory path (relative to working dir).
#[cfg(not(test))]
const LOCAL_MODEL_DIR: &str = "models/models--Qdrant--all-MiniLM-L6-v2-onnx";
#[cfg(not(test))]
const MODEL_REPO: &str = "Qdrant/all-MiniLM-L6-v2-onnx";
const MODEL_PAGE_URL: &str = "https://huggingface.co/Qdrant/all-MiniLM-L6-v2-onnx";
#[cfg(not(test))]
const MODEL_AUX_FILES: &[&str] = &[
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
    "vocab.txt",
];
#[cfg(not(test))]
const MODEL_REF_NAME: &str = "main";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryEmbeddingModelStatus {
    pub installed: bool,
    pub repo_id: String,
    pub download_url: String,
    pub install_dir: String,
    pub model_path: String,
    pub required_files: Vec<String>,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryEmbeddingModelDownloadProgress {
    pub stage: String,
    pub message: String,
    pub current_file: String,
    pub file_index: usize,
    pub file_count: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[cfg(not(test))]
fn required_model_files() -> Vec<&'static str> {
    let mut files = vec!["model.onnx"];
    files.extend(MODEL_AUX_FILES.iter().copied());
    files
}

#[cfg(not(test))]
fn default_model_cache_dir() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.chyin.kokoro")
        .join("models")
}

#[cfg(not(test))]
fn default_model_repo_dir() -> std::path::PathBuf {
    default_model_cache_dir().join("models--Qdrant--all-MiniLM-L6-v2-onnx")
}

#[cfg(not(test))]
fn default_model_snapshot_dir() -> std::path::PathBuf {
    default_model_repo_dir()
        .join("snapshots")
        .join(MODEL_REF_NAME)
}

#[cfg(not(test))]
fn missing_required_model_files(snapshot_dir: &std::path::Path) -> Vec<String> {
    required_model_files()
        .into_iter()
        .filter(|file| !snapshot_dir.join(file).is_file())
        .map(str::to_string)
        .collect()
}

#[cfg(not(test))]
fn find_existing_model_snapshot_dir(require_complete: bool) -> Option<std::path::PathBuf> {
    for base in MemoryManager::model_search_roots() {
        let repo_dir = base.join(LOCAL_MODEL_DIR);
        let Some(snapshot_dir) = MemoryManager::resolve_snapshot_dir(&repo_dir) else {
            continue;
        };
        if !require_complete || missing_required_model_files(&snapshot_dir).is_empty() {
            return Some(snapshot_dir);
        }
    }

    None
}

#[cfg(not(test))]
fn ensure_default_model_repo_layout() -> Result<()> {
    let repo_dir = default_model_repo_dir();
    let snapshot_dir = default_model_snapshot_dir();
    std::fs::create_dir_all(&snapshot_dir)?;
    std::fs::create_dir_all(repo_dir.join("refs"))?;
    std::fs::write(repo_dir.join("refs").join(MODEL_REF_NAME), MODEL_REF_NAME)?;
    Ok(())
}

#[cfg(not(test))]
fn build_download_progress(
    stage: &str,
    message: String,
    current_file: String,
    file_index: usize,
    file_count: usize,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> MemoryEmbeddingModelDownloadProgress {
    MemoryEmbeddingModelDownloadProgress {
        stage: stage.to_string(),
        message,
        current_file,
        file_index,
        file_count,
        downloaded_bytes,
        total_bytes,
    }
}

#[cfg(not(test))]
pub fn memory_embedding_model_status() -> MemoryEmbeddingModelStatus {
    let snapshot_dir = find_existing_model_snapshot_dir(true)
        .or_else(|| find_existing_model_snapshot_dir(false))
        .unwrap_or_else(default_model_snapshot_dir);
    let missing_files = missing_required_model_files(&snapshot_dir);
    let model_path = snapshot_dir.join("model.onnx");

    MemoryEmbeddingModelStatus {
        installed: missing_files.is_empty(),
        repo_id: MODEL_REPO.to_string(),
        download_url: MODEL_PAGE_URL.to_string(),
        install_dir: snapshot_dir.to_string_lossy().into_owned(),
        model_path: model_path.to_string_lossy().into_owned(),
        required_files: required_model_files()
            .into_iter()
            .map(str::to_string)
            .collect(),
        missing_files,
    }
}

#[cfg(test)]
pub fn memory_embedding_model_status() -> MemoryEmbeddingModelStatus {
    MemoryEmbeddingModelStatus {
        installed: true,
        repo_id: "Qdrant/all-MiniLM-L6-v2-onnx".to_string(),
        download_url: MODEL_PAGE_URL.to_string(),
        install_dir: String::new(),
        model_path: String::new(),
        required_files: vec!["model.onnx".to_string()],
        missing_files: Vec::new(),
    }
}

#[cfg(not(test))]
fn memory_model_endpoint() -> String {
    std::env::var("HF_ENDPOINT")
        .unwrap_or_else(|_| "https://huggingface.co".to_string())
        .trim_end_matches('/')
        .to_string()
}

#[cfg(not(test))]
fn memory_model_file_url(endpoint: &str, file_name: &str) -> String {
    format!(
        "{}/{}/resolve/{}/{}",
        endpoint, MODEL_REPO, MODEL_REF_NAME, file_name
    )
}

#[cfg(not(test))]
fn memory_model_file_path(
    snapshot_dir: &std::path::Path,
    file_name: &str,
) -> std::result::Result<std::path::PathBuf, String> {
    let path = std::path::Path::new(file_name);
    if path.components().any(|component| {
        !matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) {
        return Err(format!("Invalid model file path: {}", file_name));
    }
    Ok(snapshot_dir.join(path))
}

#[cfg(not(test))]
fn emit_memory_model_progress(
    emit_progress: &std::sync::Arc<
        dyn Fn(MemoryEmbeddingModelDownloadProgress) -> Result<(), String> + Send + Sync,
    >,
    file_name: &str,
    file_index: usize,
    file_count: usize,
    progress: crate::utils::download::DownloadProgress,
) -> std::result::Result<(), String> {
    emit_progress(build_download_progress(
        "downloading",
        format!("Downloading {} ({}/{})", file_name, file_index, file_count),
        file_name.to_string(),
        file_index,
        file_count,
        progress.downloaded_bytes,
        progress.total_bytes,
    ))
}

#[cfg(not(test))]
async fn download_memory_model_file(
    client: &reqwest::Client,
    endpoint: &str,
    snapshot_dir: &std::path::Path,
    file_name: &str,
    file_index: usize,
    file_count: usize,
    emit_progress: &std::sync::Arc<
        dyn Fn(MemoryEmbeddingModelDownloadProgress) -> Result<(), String> + Send + Sync,
    >,
) -> std::result::Result<(), String> {
    let target_path = memory_model_file_path(snapshot_dir, file_name)?;
    let url = memory_model_file_url(endpoint, file_name);
    let download_progress = emit_progress.clone();
    let file_name_for_progress = file_name.to_string();

    let final_progress = crate::utils::download::download_file_with_progress(
        client,
        &url,
        &target_path,
        crate::utils::download::DownloadOptions::default(),
        std::sync::Arc::new(move |progress| {
            emit_memory_model_progress(
                &download_progress,
                &file_name_for_progress,
                file_index,
                file_count,
                progress,
            )
        }),
    )
    .await
    .map_err(|error| format!("Failed to download {}: {}", file_name, error))?;

    emit_progress(build_download_progress(
        "complete",
        format!("Finished {} ({}/{})", file_name, file_index, file_count),
        file_name.to_string(),
        file_index,
        file_count,
        final_progress.downloaded_bytes,
        final_progress.total_bytes,
    ))?;

    Ok(())
}

#[cfg(not(test))]
pub async fn download_memory_embedding_model<F>(
    emit_progress: F,
) -> std::result::Result<MemoryEmbeddingModelStatus, String>
where
    F: Fn(MemoryEmbeddingModelDownloadProgress) -> Result<(), String> + Send + Sync + 'static,
{
    let status = memory_embedding_model_status();
    if status.installed {
        emit_progress(build_download_progress(
            "ready",
            "Memory embedding model is already installed".to_string(),
            "model.onnx".to_string(),
            0,
            0,
            0,
            None,
        ))?;
        return Ok(status);
    }

    ensure_default_model_repo_layout().map_err(|error| error.to_string())?;

    let snapshot_dir = default_model_snapshot_dir();
    let missing_files = missing_required_model_files(&snapshot_dir);
    let file_count = missing_files.len();
    let emit_progress: std::sync::Arc<
        dyn Fn(MemoryEmbeddingModelDownloadProgress) -> Result<(), String> + Send + Sync,
    > = std::sync::Arc::new(emit_progress);

    emit_progress(build_download_progress(
        "checking",
        "Checking required memory model files".to_string(),
        String::new(),
        0,
        file_count,
        0,
        None,
    ))?;

    let endpoint = memory_model_endpoint();
    let client = reqwest::Client::builder()
        .user_agent("kokoro-engine/0.2.7")
        .build()
        .map_err(|error| format!("Failed to initialize model downloader: {}", error))?;

    for (index, file_name) in missing_files.iter().enumerate() {
        download_memory_model_file(
            &client,
            &endpoint,
            &snapshot_dir,
            file_name,
            index + 1,
            file_count,
            &emit_progress,
        )
        .await?;
    }

    emit_progress(build_download_progress(
        "verifying",
        "Verifying downloaded memory model".to_string(),
        "model.onnx".to_string(),
        file_count,
        file_count,
        0,
        None,
    ))?;

    if MemoryManager::try_load_local().is_none() {
        return Err(
            "Model files were downloaded, but local verification failed. Please retry.".to_string(),
        );
    }

    let final_status = memory_embedding_model_status();
    if !final_status.installed {
        return Err("Model download finished, but required files are still missing.".to_string());
    }

    emit_progress(build_download_progress(
        "ready",
        "Memory embedding model is ready".to_string(),
        "model.onnx".to_string(),
        file_count,
        file_count,
        0,
        None,
    ))?;

    Ok(final_status)
}

#[cfg(test)]
pub async fn download_memory_embedding_model<F>(
    _emit_progress: F,
) -> std::result::Result<MemoryEmbeddingModelStatus, String>
where
    F: Fn(MemoryEmbeddingModelDownloadProgress) -> Result<(), String> + Send + Sync + 'static,
{
    Ok(memory_embedding_model_status())
}

impl MemoryManager {
    /// Creates a new MemoryManager without downloading any models.
    /// The embedding model is lazy-loaded on first use.
    pub fn new(db: SqlitePool) -> Self {
        Self {
            #[cfg(not(test))]
            embedder: tokio::sync::OnceCell::new(),
            db,
        }
    }

    pub async fn mark_interrupted_dream_jobs(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE memory_dream_jobs \
             SET status = 'interrupted', finished_at = ?, error = ? \
             WHERE status = 'running'",
        )
        .bind(now_ts())
        .bind("Dream job was interrupted before completion.")
        .execute(&self.db)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn has_dream_job_since(
        &self,
        character_id: &str,
        trigger: &str,
        since_ts: i64,
    ) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_dream_jobs \
             WHERE character_id = ? AND trigger = ? AND started_at >= ?",
        )
        .bind(character_id)
        .bind(trigger)
        .bind(since_ts)
        .fetch_one(&self.db)
        .await?;
        Ok(count > 0)
    }

    #[cfg(not(test))]
    fn resolve_snapshot_dir(repo_dir: &std::path::Path) -> Option<std::path::PathBuf> {
        use std::fs;
        let refs_main = repo_dir.join("refs").join("main");
        if let Ok(rev) = fs::read_to_string(&refs_main) {
            let rev = rev.trim();
            if !rev.is_empty() {
                let snapshot = repo_dir.join("snapshots").join(rev);
                if snapshot.exists() {
                    return Some(snapshot);
                }
            }
        }

        let snapshots = repo_dir.join("snapshots");
        fs::read_dir(&snapshots)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.is_dir())
    }

    #[cfg(not(test))]
    fn model_search_roots() -> Vec<std::path::PathBuf> {
        use std::path::PathBuf;

        // Search multiple candidate base directories
        let mut candidates: Vec<PathBuf> = Vec::new();

        // 1. Current working directory
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.clone());
            // 2. Parent of CWD (handles `src-tauri/` → project root)
            if let Some(parent) = cwd.parent() {
                candidates.push(parent.to_path_buf());
            }
        }

        // 3. Directory of the executable itself
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.to_path_buf());
                // 4. Resources dir bundled alongside the exe (Tauri build layout)
                candidates.push(exe_dir.join("_up_").join(".."));
            }
        }

        // 5. App data dir (com.chyin.kokoro) — models copied here on first run
        if let Some(app_data) = dirs_next::data_dir() {
            candidates.push(app_data.join("com.chyin.kokoro"));
        }

        candidates
    }

    #[cfg(not(test))]
    async fn hydrate_missing_local_files(snapshot_dir: &std::path::Path) -> Result<bool> {
        let missing: Vec<&str> = MODEL_AUX_FILES
            .iter()
            .copied()
            .filter(|name| !snapshot_dir.join(name).exists())
            .collect();

        if missing.is_empty() {
            return Ok(false);
        }

        tracing::info!(
            target: "memory",
            "[Memory] Hydrating missing tokenizer/config files in {}: {:?}",
            snapshot_dir.display(),
            missing
        );

        let client = reqwest::Client::builder()
            .user_agent("kokoro-engine/0.1.4")
            .build()?;

        tokio::fs::create_dir_all(snapshot_dir).await?;

        for file in &missing {
            let url = format!(
                "https://huggingface.co/{}/resolve/main/{}",
                MODEL_REPO, file
            );
            let bytes = client
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            tokio::fs::write(snapshot_dir.join(file), &bytes).await?;
        }

        Ok(true)
    }

    /// Try to load the embedding model from local files (no network required).
    #[cfg(not(test))]
    fn try_load_local() -> Option<TextEmbedding> {
        use fastembed::{InitOptionsUserDefined, TokenizerFiles, UserDefinedEmbeddingModel};
        use std::fs;

        let candidates = Self::model_search_roots();

        for base in &candidates {
            let repo_dir = base.join(LOCAL_MODEL_DIR);
            let Some(dir) = Self::resolve_snapshot_dir(&repo_dir) else {
                continue;
            };
            let onnx = dir.join("model.onnx");

            if onnx.exists() {
                tracing::info!(target: "memory", "[Memory] Found local model at: {}", dir.display());

                let tokenizer = dir.join("tokenizer.json");
                let config = dir.join("config.json");
                let special = dir.join("special_tokens_map.json");
                let tok_config = dir.join("tokenizer_config.json");

                if !tokenizer.exists() || !config.exists() {
                    tracing::error!(
                        target: "memory",
                        "[Memory] model.onnx found but tokenizer/config missing in {}, skipping.",
                        dir.display()
                    );
                    continue;
                }

                let model_def = UserDefinedEmbeddingModel::new(
                    fs::read(&onnx).ok()?,
                    TokenizerFiles {
                        tokenizer_file: fs::read(&tokenizer).ok()?,
                        config_file: fs::read(&config).ok()?,
                        special_tokens_map_file: fs::read(&special).unwrap_or_default(),
                        tokenizer_config_file: fs::read(&tok_config).unwrap_or_default(),
                    },
                );

                match TextEmbedding::try_new_from_user_defined(
                    model_def,
                    InitOptionsUserDefined::default(),
                ) {
                    Ok(model) => {
                        tracing::info!(target: "memory", "[Memory] Embedding model loaded successfully from local files.");
                        return Some(model);
                    }
                    Err(e) => {
                        tracing::error!(target: "memory", "[Memory] Failed to load local model: {}", e);
                    }
                }
            }
        }

        tracing::info!(
            target: "memory",
            "[Memory] No local model found. Searched: {:?}",
            candidates
                .iter()
                .map(|c| c.display().to_string())
                .collect::<Vec<_>>()
        );
        None
    }

    /// Lazily initializes the embedding model on first call.
    /// Tries local files first, then falls back to HuggingFace download.
    #[cfg(not(test))]
    async fn get_embedder(&self) -> Result<&Mutex<TextEmbedding>> {
        self.embedder
            .get_or_try_init(|| async {
                // 1. Try local files (no network)
                if let Some(model) = Self::try_load_local() {
                    return Ok(Mutex::new(model));
                }

                // 1.5 If a partial local cache exists, fill in the small tokenizer/config files
                // and retry local loading before falling back to fastembed's downloader again.
                for base in Self::model_search_roots() {
                    let repo_dir = base.join(LOCAL_MODEL_DIR);
                    let Some(snapshot_dir) = Self::resolve_snapshot_dir(&repo_dir) else {
                        continue;
                    };
                    if !snapshot_dir.join("model.onnx").exists() {
                        continue;
                    }
                    if Self::hydrate_missing_local_files(&snapshot_dir).await? {
                        if let Some(model) = Self::try_load_local() {
                            return Ok(Mutex::new(model));
                        }
                    }
                }

                // 2. Fall back to HF download — cache into app data so build can find it next time
                tracing::info!(target: "memory", "[Memory] Local model not found, downloading from HuggingFace...");
                let cache_dir = dirs_next::data_dir()
                    .map(|d| d.join("com.chyin.kokoro").join("models"))
                    .unwrap_or_else(|| std::path::PathBuf::from("models"));
                let _ = std::fs::create_dir_all(&cache_dir);
                let model = TextEmbedding::try_new(
                    InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_cache_dir(cache_dir),
                )?;

                // Persist any missing small files so the next startup can load locally
                // instead of repeatedly treating the cache as incomplete.
                let app_repo_dir = dirs_next::data_dir()
                    .map(|d| d.join("com.chyin.kokoro").join(LOCAL_MODEL_DIR))
                    .unwrap_or_else(|| std::path::PathBuf::from(LOCAL_MODEL_DIR));
                if let Some(snapshot_dir) = Self::resolve_snapshot_dir(&app_repo_dir) {
                    let _ = Self::hydrate_missing_local_files(&snapshot_dir).await;
                }

                tracing::info!(target: "memory", "[Memory] Embedding model downloaded and loaded successfully.");
                Ok(Mutex::new(model))
            })
            .await
    }

    #[cfg(not(test))]
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let embedder = self.get_embedder().await?;
        let mut guard = embedder.lock().await;
        let text_owned = text.to_owned();
        // ORT 内部使用 std::sync::Mutex，若初始化时曾发生 panic 则 mutex 被污染
        // 后续调用会以 panic 形式传播，必须用 catch_unwind 捕获转为 Err
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            guard.embed(vec![text_owned.as_str()], None)
        }));
        let embeddings = result
            .map_err(|_| anyhow::anyhow!("ORT embedding panicked — local embedding unavailable"))?
            .map_err(|e| anyhow::anyhow!("ORT embed error: {e}"))?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }

    #[cfg(test)]
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(test_embedding(text))
    }

    pub async fn add_memory(&self, content: &str, character_id: &str) -> Result<()> {
        self.add_memory_with_importance(content, character_id, 0.5)
            .await
    }

    async fn insert_memory_candidate(&self, candidate: NewMemoryCandidate<'_>) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO memory_candidates \
             (character_id, content, memory_type, entity_key, importance, confidence, canonical_hash, source_kind, source_refs, decision, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'extractor', '[]', 'pending', ?)",
        )
        .bind(candidate.character_id)
        .bind(candidate.content)
        .bind(&candidate.metadata.memory_type)
        .bind(candidate.metadata.entity_key.as_deref())
        .bind(candidate.importance.clamp(0.0, 1.0))
        .bind(candidate.confidence.clamp(0.0, 1.0))
        .bind(candidate.canonical_hash)
        .bind(candidate.now)
        .execute(&self.db)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn mark_candidate_decision(
        &self,
        candidate_id: i64,
        decision: &str,
        applied_memory_id: Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE memory_candidates SET decision = ?, applied_memory_id = ?, decided_at = ? WHERE id = ?",
        )
        .bind(decision)
        .bind(applied_memory_id)
        .bind(now_ts())
        .bind(candidate_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    async fn record_memory_operation(&self, operation: MemoryOperationRecord<'_>) -> Result<()> {
        sqlx::query(
            "INSERT INTO memory_operations \
             (character_id, operation_type, actor, memory_id, proposal_id, before_json, after_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(operation.character_id)
        .bind(operation.operation_type)
        .bind(operation.actor)
        .bind(operation.memory_id)
        .bind(operation.proposal_id)
        .bind(operation.before_json)
        .bind(operation.after_json)
        .bind(now_ts())
        .execute(&self.db)
        .await?;
        Ok(())
    }

    async fn refresh_exact_duplicate_by_hash(
        &self,
        character_id: &str,
        canonical_hash: &str,
        now: i64,
        importance: f64,
    ) -> Result<Option<i64>> {
        let row = sqlx::query(
            "SELECT id, importance, evidence_count FROM memories \
             WHERE character_id = ? AND canonical_hash = ? AND status = 'active' \
             ORDER BY importance DESC, updated_at DESC LIMIT 1",
        )
        .bind(character_id)
        .bind(canonical_hash)
        .fetch_optional(&self.db)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let id: i64 = row.get("id");
        let existing_importance: f64 = row.get("importance");
        let evidence_count: i64 = row.get("evidence_count");
        let best_importance = existing_importance.max(importance.clamp(0.0, 1.0));
        let tier = if best_importance >= 0.8 {
            "core"
        } else {
            "ephemeral"
        };

        sqlx::query(
            "UPDATE memories \
             SET updated_at = ?, last_seen_at = ?, evidence_count = ?, importance = ?, tier = ? \
             WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(evidence_count.saturating_add(1))
        .bind(best_importance)
        .bind(tier)
        .bind(id)
        .execute(&self.db)
        .await?;

        self.record_memory_operation(MemoryOperationRecord {
            character_id,
            operation_type: "deduplicate_exact",
            actor: "memory_pipeline",
            memory_id: Some(id),
            proposal_id: None,
            before_json: None,
            after_json: Some(
                serde_json::json!({ "importance": best_importance, "evidence_count": evidence_count + 1 })
                    .to_string(),
            ),
        })
        .await?;
        Ok(Some(id))
    }

    async fn upsert_entity_slot_memory(&self, upsert: EntitySlotUpsert<'_>) -> Result<Option<i64>> {
        let Some(entity_key) = upsert.metadata.entity_key.as_deref() else {
            return Ok(None);
        };

        let row = sqlx::query(
            "SELECT id, content, importance, evidence_count, first_seen_at FROM memories \
             WHERE character_id = ? AND memory_type = ? AND entity_key = ? AND status = 'active' \
             ORDER BY importance DESC, updated_at DESC LIMIT 1",
        )
        .bind(upsert.character_id)
        .bind(&upsert.metadata.memory_type)
        .bind(entity_key)
        .fetch_optional(&self.db)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let id: i64 = row.get("id");
        let existing_content: String = row.get("content");
        let existing_importance: f64 = row.get("importance");
        let evidence_count: i64 = row.get("evidence_count");
        let first_seen_at: i64 = row.get("first_seen_at");
        let best_importance = existing_importance.max(upsert.importance.clamp(0.0, 1.0));
        let tier = if best_importance >= 0.8 {
            "core"
        } else {
            "ephemeral"
        };
        let merged_content = upsert
            .metadata
            .canonical_content
            .as_deref()
            .unwrap_or(upsert.content)
            .trim()
            .to_string();
        let update_embedding = if merged_content == existing_content {
            None
        } else {
            Some(upsert.embedding_bytes.to_vec())
        };

        if let Some(bytes) = update_embedding {
            sqlx::query(
                "UPDATE memories \
                 SET content = ?, embedding = ?, updated_at = ?, importance = ?, tier = ?, confidence = ?, \
                     last_seen_at = ?, evidence_count = ?, canonical_hash = ? \
                 WHERE id = ?",
            )
            .bind(&merged_content)
            .bind(bytes)
            .bind(upsert.now)
            .bind(best_importance)
            .bind(tier)
            .bind(DREAM_CONFIDENCE_AUTO_APPLY)
            .bind(upsert.now)
            .bind(evidence_count.saturating_add(1))
            .bind(upsert.canonical_hash)
            .bind(id)
            .execute(&self.db)
            .await?;
        } else {
            sqlx::query(
                "UPDATE memories \
                 SET updated_at = ?, importance = ?, tier = ?, confidence = ?, last_seen_at = ?, evidence_count = ? \
                 WHERE id = ?",
            )
            .bind(upsert.now)
            .bind(best_importance)
            .bind(tier)
            .bind(DREAM_CONFIDENCE_AUTO_APPLY)
            .bind(upsert.now)
            .bind(evidence_count.saturating_add(1))
            .bind(id)
            .execute(&self.db)
            .await?;
        }

        self.record_memory_operation(MemoryOperationRecord {
            character_id: upsert.character_id,
            operation_type: "entity_slot_upsert",
            actor: "memory_pipeline",
            memory_id: Some(id),
            proposal_id: None,
            before_json: Some(
                serde_json::json!({ "content": existing_content, "first_seen_at": first_seen_at })
                    .to_string(),
            ),
            after_json: Some(
                serde_json::json!({ "content": merged_content, "last_seen_at": upsert.now, "evidence_count": evidence_count + 1 })
                    .to_string(),
            ),
        })
        .await?;
        Ok(Some(id))
    }

    async fn insert_active_memory(&self, insert: ActiveMemoryInsert<'_>) -> Result<i64> {
        let clamped = insert.importance.clamp(0.0, 1.0);
        let tier = if clamped >= 0.8 { "core" } else { "ephemeral" };
        let storage_content = insert
            .metadata
            .canonical_content
            .as_deref()
            .unwrap_or(insert.content)
            .trim()
            .to_string();

        let result = sqlx::query(
            "INSERT INTO memories \
             (content, embedding, created_at, updated_at, importance, character_id, tier, \
              memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, evidence_count, \
              source_kind, source_refs, canonical_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, 1, 'extractor', '[]', ?)",
        )
        .bind(&storage_content)
        .bind(insert.embedding_bytes)
        .bind(insert.now)
        .bind(insert.now)
        .bind(clamped)
        .bind(insert.character_id)
        .bind(tier)
        .bind(&insert.metadata.memory_type)
        .bind(insert.metadata.entity_key.as_deref())
        .bind(DREAM_CONFIDENCE_AUTO_APPLY)
        .bind(insert.now)
        .bind(insert.now)
        .bind(insert.canonical_hash)
        .execute(&self.db)
        .await?;

        let id = result.last_insert_rowid();
        self.record_memory_operation(MemoryOperationRecord {
            character_id: insert.character_id,
            operation_type: "insert_active",
            actor: "memory_pipeline",
            memory_id: Some(id),
            proposal_id: None,
            before_json: None,
            after_json: Some(
                serde_json::json!({ "content": storage_content, "memory_type": insert.metadata.memory_type, "entity_key": insert.metadata.entity_key })
                    .to_string(),
            ),
        })
        .await?;
        Ok(id)
    }

    /// Return all memory content strings for a given character (used for dedup in extraction).
    pub async fn get_all_memory_contents(&self, character_id: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT content FROM memories WHERE character_id = ? AND tier != 'invalidated' AND status = 'active' ORDER BY importance DESC LIMIT 50",
        )
        .bind(character_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.iter().map(|r| r.get::<String, _>("content")).collect())
    }

    /// Like `deduplicate_or_refresh`, but also upgrades importance and tier if the
    /// new extraction has higher importance than the existing duplicate.
    async fn deduplicate_or_upgrade(
        &self,
        new_embedding: &[f32],
        character_id: &str,
        now: i64,
        new_importance: f64,
    ) -> Result<bool> {
        let rows =
            sqlx::query("SELECT id, embedding, importance FROM memories WHERE character_id = ? AND tier != 'invalidated' AND status = 'active'")
                .bind(character_id)
                .fetch_all(&self.db)
                .await?;

        for row in rows {
            let existing_bytes: Vec<u8> = row.get("embedding");
            let existing: Vec<f32> = bincode::deserialize(&existing_bytes)?;
            let sim = cosine_similarity(new_embedding, &existing);
            if sim > DEDUP_THRESHOLD {
                let id: i64 = row.get("id");
                tracing::info!(
                    target: "memory",
                    "[Memory] Dedup-upgrade: similarity={:.3} > {:.3}, upgrading id={}",
                    sim, DEDUP_THRESHOLD, id
                );
                let existing_importance: f64 = row.get("importance");
                let best_importance = existing_importance.max(new_importance);
                let tier = if best_importance >= 0.8 {
                    "core"
                } else {
                    "ephemeral"
                };
                sqlx::query(
                    "UPDATE memories SET updated_at = ?, importance = ?, tier = ? WHERE id = ?",
                )
                .bind(now)
                .bind(best_importance)
                .bind(tier)
                .bind(id)
                .execute(&self.db)
                .await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn search_memories(
        &self,
        query: &str,
        limit: usize,
        character_id: &str,
    ) -> Result<Vec<MemorySnippet>> {
        let outcome = self
            .search_memories_with_observability(query, limit, character_id)
            .await?;
        Ok(outcome.snippets)
    }

    async fn search_memories_with_observability(
        &self,
        query: &str,
        limit: usize,
        character_id: &str,
    ) -> Result<SearchMemoriesOutcome> {
        let semantic_results = self.semantic_search(query, limit * 2, character_id).await?;
        let bm25_results = self
            .bm25_search(query, character_id, limit * 2)
            .await
            .unwrap_or_default();

        let k = 60.0_f32;
        let mut rrf_scores: std::collections::HashMap<i64, (f32, MemorySnippet)> =
            std::collections::HashMap::new();

        for (rank, mem) in semantic_results.iter().enumerate() {
            let score = 1.0 / (k + rank as f32 + 1.0);
            rrf_scores.entry(mem.id).or_insert((0.0, mem.clone())).0 += score;
        }

        for (rank, (id, _bm25_score)) in bm25_results.iter().enumerate() {
            let score = 1.0 / (k + rank as f32 + 1.0);
            if let Some(entry) = rrf_scores.get_mut(id) {
                entry.0 += score;
            } else if let Ok(Some(snippet)) = self.fetch_memory_snippet(*id).await {
                rrf_scores.insert(*id, (score, snippet));
            }
        }

        let mut fused: Vec<(f32, MemorySnippet)> = rrf_scores.into_values().collect();
        fused.sort_by(|a, b| compare_scored_memory(a.0, &a.1, b.0, &b.1));

        let snippets: Vec<MemorySnippet> = fused
            .iter()
            .filter(|(score, _)| *score >= MIN_RRF_SCORE)
            .take(limit)
            .map(|(_, m)| m.clone())
            .collect();

        let stats = build_search_stats(
            &semantic_results,
            &bm25_results,
            &fused,
            &snippets,
            is_retrieval_eval_enabled(),
        );
        let _ = record_memory_retrieval_if_enabled(self, character_id, query, &stats).await;

        Ok(build_search_outcome(snippets))
    }

    pub async fn record_memory_write_observation(
        &self,
        observation: MemoryWriteObservation,
    ) -> Result<()> {
        insert_memory_write_event(self, observation).await
    }

    pub async fn record_memory_retrieval_observation(
        &self,
        observation: MemoryRetrievalObservation,
    ) -> Result<()> {
        insert_memory_retrieval_log(self, observation).await
    }

    pub async fn latest_memory_write_event(&self) -> Result<Option<MemoryWriteEventRecord>> {
        fetch_latest_memory_write_event(self).await
    }

    pub async fn latest_memory_retrieval_log(&self) -> Result<Option<MemoryRetrievalLogRecord>> {
        fetch_latest_memory_retrieval_log(self).await
    }

    pub async fn latest_memory_retrieval_eval_summary(
        &self,
    ) -> Result<Option<MemoryRetrievalEvalSummary>> {
        if !is_retrieval_eval_enabled() {
            return Ok(None);
        }

        Ok(self
            .latest_memory_retrieval_log()
            .await?
            .as_ref()
            .map(build_retrieval_eval_summary))
    }

    pub async fn memory_observability_summary(&self) -> Result<MemoryObservabilitySummary> {
        build_memory_observability_counts(self).await
    }

    pub async fn record_periodic_write_if_enabled(
        &self,
        character_id: &str,
        source: &str,
        trigger: &str,
        started_at: std::time::Instant,
    ) -> Result<()> {
        record_memory_write_if_enabled(self, character_id, source, trigger, started_at).await
    }

    pub async fn periodic_write_observation_for_chat(
        &self,
        character_id: &str,
        started_at: std::time::Instant,
    ) -> Result<()> {
        self.record_periodic_write_if_enabled(
            character_id,
            periodic_source_for_chat(),
            periodic_trigger_for_extraction(),
            started_at,
        )
        .await
    }

    pub async fn periodic_write_observation_for_telegram(
        &self,
        character_id: &str,
        started_at: std::time::Instant,
    ) -> Result<()> {
        self.record_periodic_write_if_enabled(
            character_id,
            periodic_source_for_telegram(),
            periodic_trigger_for_extraction(),
            started_at,
        )
        .await
    }

    pub async fn periodic_consolidation_observation(
        &self,
        character_id: &str,
        source: &str,
        started_at: std::time::Instant,
    ) -> Result<()> {
        self.record_periodic_write_if_enabled(
            character_id,
            source,
            periodic_trigger_for_consolidation(),
            started_at,
        )
        .await
    }

    pub fn build_retrieval_observation_for_test(
        character_id: &str,
        query: &str,
        stats: &RetrievalCandidateStats,
    ) -> Result<MemoryRetrievalObservation> {
        build_retrieval_observation_record(character_id, query, stats)
    }

    pub fn retrieval_stats_for_test(
        semantic_candidates: usize,
        bm25_candidates: usize,
        fused_candidates: usize,
        injected_count: usize,
    ) -> Result<RetrievalCandidateStats> {
        build_retrieval_stats_for_search(
            semantic_candidates,
            bm25_candidates,
            fused_candidates,
            injected_count,
        )
    }

    /// Pure semantic (embedding) search with time decay, respecting tier.
    async fn semantic_search(
        &self,
        query: &str,
        limit: usize,
        character_id: &str,
    ) -> Result<Vec<MemorySnippet>> {
        let query_embedding = self.embed(query).await?;

        let rows =
            sqlx::query("SELECT id, content, embedding, created_at, importance, tier FROM memories WHERE character_id = ? AND tier != 'invalidated' AND status = 'active'")
                .bind(character_id)
                .fetch_all(&self.db)
                .await?;

        let mut scored_memories: Vec<(MemorySnippet, f32)> = Vec::new();
        let now = chrono::Utc::now().timestamp();

        for row in rows {
            let embedding_bytes: Vec<u8> = row.get("embedding");
            let embedding: Vec<f32> = bincode::deserialize(&embedding_bytes)?;

            let similarity = cosine_similarity(&query_embedding, &embedding);

            let created_at: i64 = row.get("created_at");
            let tier: String = row.get("tier");

            // Core memories never decay; ephemeral memories use time decay
            let decay = if tier == "core" {
                1.0_f32
            } else {
                let age_days = (now - created_at) as f64 / 86400.0;
                (0.5_f64).powf(age_days / MEMORY_HALF_LIFE_DAYS) as f32
            };
            let final_score = similarity * decay;

            // Skip memories with negligible relevance before RRF pooling
            if final_score < MIN_COSINE_SIMILARITY {
                continue;
            }

            let memory = MemorySnippet {
                id: row.get("id"),
                content: row.get("content"),
                embedding: embedding_bytes,
                created_at,
                importance: row.get("importance"),
                tier,
            };

            scored_memories.push((memory, final_score));
        }

        scored_memories.sort_by(|a, b| compare_scored_memory(a.1, &a.0, b.1, &b.0));

        Ok(scored_memories
            .into_iter()
            .take(limit)
            .map(|(m, _)| m)
            .collect())
    }

    /// BM25 keyword search via FTS5. Returns (memory_id, bm25_score) pairs.
    async fn bm25_search(
        &self,
        query: &str,
        character_id: &str,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>> {
        let fts_query = escape_fts5_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query(
            "SELECT m.id, bm25(memories_fts) AS score \
             FROM memories_fts f \
             JOIN memories m ON m.id = f.rowid \
             WHERE memories_fts MATCH ? AND m.character_id = ? AND m.tier != 'invalidated' AND m.status = 'active' \
             ORDER BY score \
             LIMIT ?",
        )
        .bind(&fts_query)
        .bind(character_id)
        .bind(limit as i64)
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let id: i64 = r.get("id");
                let score: f64 = r.get("score");
                (id, score)
            })
            .collect())
    }

    /// Fetch a single memory snippet by ID.
    async fn fetch_memory_snippet(&self, id: i64) -> Result<Option<MemorySnippet>> {
        let row = sqlx::query(
            "SELECT id, content, embedding, created_at, importance, tier FROM memories WHERE id = ? AND tier != 'invalidated' AND status = 'active'",
        )
        .bind(id)
        .fetch_optional(&self.db)
        .await?;

        Ok(row.map(|r| MemorySnippet {
            id: r.get("id"),
            content: r.get("content"),
            embedding: r.get("embedding"),
            created_at: r.get("created_at"),
            importance: r.get("importance"),
            tier: r.get("tier"),
        }))
    }
}

#[cfg(test)]
fn test_embedding(text: &str) -> Vec<f32> {
    const DIM: usize = 64;

    let normalized = text.to_lowercase();
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    let mut vector = vec![0.0_f32; DIM];

    if tokens.is_empty() {
        return vector;
    }

    for token in tokens {
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in token.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }

        let bucket = (hash as usize) % DIM;
        vector[bucket] += 1.0;
    }

    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }

    vector
}

// ── Session Summaries / Conversation Summaries ─────────────────────────────

impl MemoryManager {
    /// Save a session summary for a character.
    pub async fn save_session_summary(&self, character_id: &str, summary: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_summaries (character_id, summary, created_at) VALUES (?, ?, ?)",
        )
        .bind(character_id)
        .bind(summary)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Get the most recent N session summaries for a character.
    pub async fn get_recent_summaries(
        &self,
        character_id: &str,
        limit: usize,
    ) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT summary FROM session_summaries WHERE character_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(character_id)
        .bind(limit as i64)
        .fetch_all(&self.db)
        .await?;

        Ok(rows.iter().map(|r| r.get("summary")).collect())
    }

    pub async fn get_latest_conversation_summary(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationSummaryRecord>> {
        let row = sqlx::query(
            "SELECT conversation_id, version, start_message_id, end_message_id, summary, status, failure_count, updated_at
             FROM conversation_summaries
             WHERE conversation_id = ? AND status = 'ready'
             ORDER BY version DESC
             LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.db)
        .await?;

        Ok(row.map(|r| ConversationSummaryRecord {
            conversation_id: r.get("conversation_id"),
            version: r.get("version"),
            start_message_id: r.get("start_message_id"),
            end_message_id: r.get("end_message_id"),
            summary: r.get("summary"),
            status: ConversationSummaryStatus::from_db(r.get::<String, _>("status").as_str()),
            failure_count: r.get("failure_count"),
            updated_at: r.get("updated_at"),
        }))
    }

    pub async fn get_conversation_summary_task(
        &self,
        conversation_id: &str,
        character_id: &str,
    ) -> Result<Option<ConversationSummaryTask>> {
        let now = chrono::Utc::now().timestamp();

        if let Some(circuit_row) = sqlx::query(
            "SELECT updated_at, failure_count FROM conversation_summaries
             WHERE conversation_id = ? AND status = 'circuit_open'
             ORDER BY version DESC
             LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.db)
        .await?
        {
            let updated_at: i64 = circuit_row.get("updated_at");
            let failure_count: i64 = circuit_row.get("failure_count");
            if now - updated_at < CONVERSATION_SUMMARY_COOLDOWN_SECS {
                return Ok(None);
            }

            sqlx::query(
                "UPDATE conversation_summaries
                 SET status = 'failed', updated_at = ?
                 WHERE conversation_id = ? AND status = 'circuit_open'",
            )
            .bind(now)
            .bind(conversation_id)
            .execute(&self.db)
            .await?;

            tracing::info!(
                target: "context",
                "[Context] Reopening conversation summary circuit for '{}' after cooldown (failure_count={})",
                conversation_id, failure_count
            );
        }

        if sqlx::query(
            "SELECT id FROM conversation_summaries
             WHERE conversation_id = ? AND status IN ('pending', 'running')
             LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.db)
        .await?
        .is_some()
        {
            return Ok(None);
        }

        let latest_ready_end: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(end_message_id), 0)
             FROM conversation_summaries
             WHERE conversation_id = ? AND status = 'ready'",
        )
        .bind(conversation_id)
        .fetch_one(&self.db)
        .await?;

        let message_rows = sqlx::query(
            "SELECT id, role, content, metadata
             FROM conversation_messages
             WHERE conversation_id = ? AND id > ?
             ORDER BY id ASC",
        )
        .bind(conversation_id)
        .bind(latest_ready_end)
        .fetch_all(&self.db)
        .await?;

        let mut visible = Vec::new();
        for row in message_rows {
            let metadata_raw: Option<String> = row.get("metadata");
            let metadata_json = metadata_raw
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let technical_type = metadata_json
                .as_ref()
                .and_then(|meta| meta.get("type"))
                .and_then(|value| value.as_str());
            if matches!(
                technical_type,
                Some("assistant_tool_calls")
                    | Some("tool_result")
                    | Some("translation_instruction")
                    | Some("vision_observation")
            ) {
                continue;
            }
            if row.get::<String, _>("role") == "context" {
                continue;
            }

            visible.push((
                row.get::<i64, _>("id"),
                row.get::<String, _>("role"),
                row.get::<String, _>("content"),
            ));
        }

        if visible.len() < CONVERSATION_SUMMARY_MIN_MESSAGES {
            return Ok(None);
        }

        let chunk = &visible[..visible.len().min(CONVERSATION_SUMMARY_MAX_MESSAGES)];
        let start_message_id = chunk.first().map(|(id, _, _)| *id).unwrap_or(0);
        let end_message_id = chunk.last().map(|(id, _, _)| *id).unwrap_or(0);
        if start_message_id == 0 || end_message_id == 0 {
            return Ok(None);
        }

        let version: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM conversation_summaries WHERE conversation_id = ?",
        )
        .bind(conversation_id)
        .fetch_one(&self.db)
        .await?;

        let record_id = sqlx::query(
            "INSERT INTO conversation_summaries
             (conversation_id, character_id, version, start_message_id, end_message_id, summary, status, failure_count, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, '', 'pending', 0, ?, ?)",
        )
        .bind(conversation_id)
        .bind(character_id)
        .bind(version)
        .bind(start_message_id)
        .bind(end_message_id)
        .bind(now)
        .bind(now)
        .execute(&self.db)
        .await?
        .last_insert_rowid();

        let transcript = chunk
            .iter()
            .map(|(_, role, content)| format!("{}: {}", role, content))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(Some(ConversationSummaryTask {
            record_id,
            conversation_id: conversation_id.to_string(),
            character_id: character_id.to_string(),
            version,
            start_message_id,
            end_message_id,
            transcript,
        }))
    }

    pub async fn mark_conversation_summary_running(&self, record_id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE conversation_summaries
             SET status = 'running', updated_at = ?
             WHERE id = ?",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(record_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn complete_conversation_summary(&self, record_id: i64, summary: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "UPDATE conversation_summaries
             SET summary = ?, status = 'ready', failure_count = 0, updated_at = ?
             WHERE id = ?",
        )
        .bind(summary)
        .bind(now)
        .bind(record_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn fail_conversation_summary(&self, record_id: i64, error: &str) -> Result<()> {
        let row = sqlx::query(
            "SELECT conversation_id, failure_count FROM conversation_summaries WHERE id = ?",
        )
        .bind(record_id)
        .fetch_one(&self.db)
        .await?;

        let conversation_id: String = row.get("conversation_id");
        let previous_failure_count: i64 = row.get("failure_count");
        let failure_count = previous_failure_count + 1;
        let status = if failure_count >= CONVERSATION_SUMMARY_FAILURE_THRESHOLD {
            ConversationSummaryStatus::CircuitOpen
        } else {
            ConversationSummaryStatus::Failed
        };
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            "UPDATE conversation_summaries
             SET status = ?, failure_count = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(failure_count)
        .bind(now)
        .bind(record_id)
        .execute(&self.db)
        .await?;

        tracing::error!(
            target: "context",
            "[Context] Conversation summary failed for '{}' (record={}, failures={}): {}",
            conversation_id, record_id, failure_count, error
        );
        Ok(())
    }

    // ── Smart Memory Importance ────────────────────────────

    /// Add a memory with an explicit importance score (0.0-1.0).
    /// Higher importance memories decay slower during search.
    pub async fn add_memory_with_importance(
        &self,
        content: &str,
        character_id: &str,
        importance: f64,
    ) -> Result<()> {
        let metadata = infer_memory_metadata(content);
        let storage_probe = metadata.canonical_content.as_deref().unwrap_or(content);
        let hash = canonical_hash(storage_probe);
        let embedding = self.embed(storage_probe).await?;
        let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
        let now = now_ts();
        let candidate_id = self
            .insert_memory_candidate(NewMemoryCandidate {
                content,
                character_id,
                importance,
                confidence: DREAM_CONFIDENCE_AUTO_APPLY,
                metadata: &metadata,
                canonical_hash: &hash,
                now,
            })
            .await?;

        if let Some(id) = self
            .refresh_exact_duplicate_by_hash(character_id, &hash, now, importance)
            .await?
        {
            self.mark_candidate_decision(candidate_id, "duplicate", Some(id))
                .await?;
            return Ok(());
        }

        if let Some(id) = self
            .upsert_entity_slot_memory(EntitySlotUpsert {
                content,
                embedding_bytes: &embedding_bytes,
                character_id,
                importance,
                metadata: &metadata,
                canonical_hash: &hash,
                now,
            })
            .await?
        {
            self.mark_candidate_decision(candidate_id, "updated", Some(id))
                .await?;
            return Ok(());
        }

        // Deduplication check — also upgrades importance/tier if duplicate found
        if let Ok(true) = self
            .deduplicate_or_upgrade(&embedding, character_id, now, importance)
            .await
        {
            self.mark_candidate_decision(candidate_id, "semantic_duplicate", None)
                .await?;
            return Ok(());
        }

        let memory_id = self
            .insert_active_memory(ActiveMemoryInsert {
                content,
                embedding_bytes,
                character_id,
                importance,
                metadata: &metadata,
                canonical_hash: &hash,
                now,
            })
            .await?;
        self.mark_candidate_decision(candidate_id, "inserted", Some(memory_id))
            .await?;

        // After inserting, check for contradiction with existing memories in the 0.70-0.95 band.
        // The v2 path records review proposals instead of hiding old memories immediately.
        let _ = self
            .check_and_invalidate_contradictions(content, &embedding, character_id)
            .await;

        Ok(())
    }

    /// After storing a new memory, scan existing memories in the CONTRADICTION_BAND
    /// (similarity 0.70–0.95) and create review proposals for likely contradictions.
    ///
    /// Uses a lightweight negation-keyword heuristic — no LLM call needed. Dream Memory v2
    /// keeps the old memory active until a proposal is reviewed.
    pub async fn check_and_invalidate_contradictions(
        &self,
        new_content: &str,
        new_embedding: &[f32],
        character_id: &str,
    ) -> Result<usize> {
        let rows = sqlx::query(
            "SELECT id, content, embedding FROM memories \
             WHERE character_id = ? AND tier != 'invalidated' AND status = 'active' \
             AND created_at >= ? \
             ORDER BY created_at DESC LIMIT 200",
        )
        .bind(character_id)
        .bind(chrono::Utc::now().timestamp() - 90 * 24 * 3600) // last 90 days
        .fetch_all(&self.db)
        .await?;

        let mut invalidated = 0usize;
        for row in rows {
            let bytes: Vec<u8> = row.get("embedding");
            let Ok(existing_emb): Result<Vec<f32>, _> = bincode::deserialize(&bytes) else {
                continue;
            };
            let sim = cosine_similarity(new_embedding, &existing_emb);

            if (CONTRADICTION_BAND_LOW..CONTRADICTION_BAND_HIGH).contains(&sim) {
                let existing_content: String = row.get("content");
                if is_likely_contradiction(new_content, &existing_content) {
                    let id: i64 = row.get("id");
                    self.create_dream_proposal(DreamProposalInsert {
                        character_id,
                        proposal_type: "conflict_review",
                        status: "pending",
                        confidence: f64::from(sim),
                        title: "Review possible memory conflict",
                        rationale: "A new memory appears to contradict an existing active memory.",
                        source_memory_ids: &[id],
                        target_memory_id: Some(id),
                        proposed_content: Some(new_content),
                        proposed_memory_type: None,
                        proposed_entity_key: None,
                        impact: "Manual review required before invalidating or replacing the older memory.",
                    })
                    .await?;
                    invalidated += 1;
                    tracing::info!(
                        target: "memory",
                        "[Memory] Created conflict proposal for memory id={}: '{}'",
                        id,
                        &existing_content[..existing_content.len().min(60)]
                    );
                }
            }
        }
        Ok(invalidated)
    }

    // ── Memory CRUD (for viewer/editor UI) ────────────────

    /// List all memories for a character, paginated, ordered by creation time desc.
    pub async fn list_memories(
        &self,
        character_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<MemoryRecord>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            "SELECT rowid AS rowid, content, created_at, importance, tier, memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, evidence_count \
             FROM memories WHERE character_id = ? AND status = 'active' ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(character_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.db)
        .await?;

        let now = chrono::Utc::now().timestamp();
        Ok(rows
            .into_iter()
            .map(|r| {
                let effective_importance = if r.tier == "core" {
                    r.importance
                } else {
                    let age_days = (now - r.created_at) as f64 / 86400.0;
                    let decay = (0.5_f64).powf(age_days / MEMORY_HALF_LIFE_DAYS);
                    r.importance * decay
                };
                MemoryRecord {
                    id: r.rowid,
                    content: r.content,
                    created_at: r.created_at,
                    importance: effective_importance,
                    tier: r.tier,
                    memory_type: r.memory_type,
                    entity_key: r.entity_key,
                    status: r.status,
                    confidence: r.confidence,
                    first_seen_at: r.first_seen_at,
                    last_seen_at: r.last_seen_at,
                    evidence_count: r.evidence_count,
                }
            })
            .collect())
    }

    /// Count total memories for a character.
    pub async fn count_memories(&self, character_id: &str) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM memories WHERE character_id = ? AND status = 'active'",
        )
        .bind(character_id)
        .fetch_one(&self.db)
        .await?;
        Ok(row.0)
    }

    /// Update a memory's content and importance. Re-embeds the content.
    /// Automatically syncs tier based on new importance.
    pub async fn update_memory(&self, id: i64, content: &str, importance: f64) -> Result<()> {
        let embedding = self.embed(content).await?;
        let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
        let clamped = importance.clamp(0.0, 1.0);
        let tier = if clamped >= 0.8 { "core" } else { "ephemeral" };
        let metadata = infer_memory_metadata(content);
        let hash = canonical_hash(metadata.canonical_content.as_deref().unwrap_or(content));
        let now = now_ts();

        sqlx::query(
            "UPDATE memories SET content = ?, embedding = ?, importance = ?, tier = ?, memory_type = ?, entity_key = ?, canonical_hash = ?, updated_at = ?, last_seen_at = ? WHERE rowid = ?",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(clamped)
        .bind(tier)
        .bind(metadata.memory_type)
        .bind(metadata.entity_key)
        .bind(hash)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Delete a memory by ID.
    pub async fn delete_memory(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE memories SET status = 'archived', updated_at = ? WHERE rowid = ?")
            .bind(now_ts())
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Update a memory's tier (e.g. "core" or "ephemeral").
    pub async fn update_memory_tier(&self, id: i64, tier: &str) -> Result<()> {
        sqlx::query("UPDATE memories SET tier = ?, updated_at = ? WHERE rowid = ?")
            .bind(tier)
            .bind(now_ts())
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    async fn create_dream_proposal(&self, proposal: DreamProposalInsert<'_>) -> Result<i64> {
        let now = now_ts();
        let applied_at = if matches!(proposal.status, "auto_applied" | "approved") {
            Some(now)
        } else {
            None
        };
        let result = sqlx::query(
            "INSERT INTO memory_dream_proposals \
             (character_id, proposal_type, status, confidence, title, rationale, source_memory_ids, \
              target_memory_id, proposed_content, proposed_memory_type, proposed_entity_key, impact, \
              created_at, updated_at, applied_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(proposal.character_id)
        .bind(proposal.proposal_type)
        .bind(proposal.status)
        .bind(proposal.confidence.clamp(0.0, 1.0))
        .bind(proposal.title)
        .bind(proposal.rationale)
        .bind(proposal_ids_json(proposal.source_memory_ids)?)
        .bind(proposal.target_memory_id)
        .bind(proposal.proposed_content)
        .bind(proposal.proposed_memory_type)
        .bind(proposal.proposed_entity_key)
        .bind(proposal.impact)
        .bind(now)
        .bind(now)
        .bind(applied_at)
        .execute(&self.db)
        .await?;
        Ok(result.last_insert_rowid())
    }

    async fn load_dream_entries(&self, character_id: &str) -> Result<Vec<DreamCandidateEntry>> {
        let rows = sqlx::query(
            "SELECT id, content, embedding, created_at, updated_at, importance, tier, memory_type, \
                    entity_key, canonical_hash, evidence_count \
             FROM memories \
             WHERE character_id = ? AND tier != 'invalidated' AND status = 'active'",
        )
        .bind(character_id)
        .fetch_all(&self.db)
        .await?;

        let mut entries = Vec::new();
        for row in rows {
            let bytes: Vec<u8> = row.get("embedding");
            let Ok(embedding) = bincode::deserialize::<Vec<f32>>(&bytes) else {
                continue;
            };
            entries.push(DreamCandidateEntry {
                id: row.get("id"),
                content: row.get("content"),
                embedding,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                importance: row.get("importance"),
                tier: row.get("tier"),
                memory_type: row.get("memory_type"),
                entity_key: row.get("entity_key"),
                canonical_hash: row.get("canonical_hash"),
                evidence_count: row.get("evidence_count"),
            });
        }
        Ok(entries)
    }

    fn choose_dream_keeper<'a>(
        &self,
        entries: &'a [&DreamCandidateEntry],
    ) -> &'a DreamCandidateEntry {
        entries
            .iter()
            .copied()
            .max_by(|a, b| {
                let a_core = if a.tier == "core" { 1 } else { 0 };
                let b_core = if b.tier == "core" { 1 } else { 0 };
                a_core
                    .cmp(&b_core)
                    .then_with(|| {
                        a.importance
                            .partial_cmp(&b.importance)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| a.evidence_count.cmp(&b.evidence_count))
                    .then_with(|| a.updated_at.cmp(&b.updated_at))
            })
            .expect("dream keeper requires non-empty entries")
    }

    fn choose_dream_keeper_with_preference<'a>(
        &self,
        entries: &'a [&DreamCandidateEntry],
        preferred_id: Option<i64>,
    ) -> &'a DreamCandidateEntry {
        if let Some(preferred_id) = preferred_id {
            if let Some(entry) = entries
                .iter()
                .copied()
                .find(|entry| entry.id == preferred_id)
            {
                return entry;
            }
        }
        self.choose_dream_keeper(entries)
    }

    async fn auto_merge_dream_group(&self, request: DreamAutoMergeRequest<'_>) -> Result<i64> {
        let DreamAutoMergeRequest {
            character_id,
            entries,
            proposal_type,
            title,
            rationale,
            confidence,
            proposed_content_override,
            preferred_keeper_id,
        } = request;

        if entries.len() < 2 {
            return Ok(0);
        }

        let keeper = self.choose_dream_keeper_with_preference(entries, preferred_keeper_id);
        let source_ids: Vec<i64> = entries.iter().map(|entry| entry.id).collect();
        let superseded_ids: Vec<i64> = source_ids
            .iter()
            .copied()
            .filter(|id| *id != keeper.id)
            .collect();
        if superseded_ids.is_empty() {
            return Ok(0);
        }

        let evidence_count: i64 = entries
            .iter()
            .map(|entry| entry.evidence_count.max(1))
            .sum();
        let max_importance = entries
            .iter()
            .map(|entry| entry.importance)
            .fold(0.0_f64, f64::max);
        let first_seen_at = entries
            .iter()
            .map(|entry| entry.created_at)
            .min()
            .unwrap_or(keeper.created_at);
        let last_seen_at = entries
            .iter()
            .map(|entry| entry.updated_at.max(entry.created_at))
            .max()
            .unwrap_or(keeper.updated_at);
        let tier = if entries.iter().any(|entry| entry.tier == "core") {
            "core"
        } else {
            "ephemeral"
        };
        let metadata = infer_memory_metadata(&keeper.content);
        let merged_content = proposed_content_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| metadata.canonical_content.clone())
            .unwrap_or_else(|| keeper.content.clone());

        sqlx::query(
            "UPDATE memories \
             SET content = ?, importance = ?, tier = ?, confidence = ?, first_seen_at = ?, \
                 last_seen_at = ?, evidence_count = ?, updated_at = ?, supersedes = ? \
             WHERE id = ?",
        )
        .bind(&merged_content)
        .bind(max_importance)
        .bind(tier)
        .bind(confidence.clamp(0.0, 1.0))
        .bind(first_seen_at)
        .bind(last_seen_at)
        .bind(evidence_count)
        .bind(now_ts())
        .bind(proposal_ids_json(&superseded_ids)?)
        .bind(keeper.id)
        .execute(&self.db)
        .await?;

        for superseded_id in &superseded_ids {
            sqlx::query(
                "UPDATE memories SET status = 'superseded', updated_at = ?, supersedes = ? WHERE id = ?",
            )
            .bind(now_ts())
            .bind(proposal_ids_json(&[keeper.id])?)
            .bind(superseded_id)
            .execute(&self.db)
            .await?;
        }

        let proposal_id = self
            .create_dream_proposal(DreamProposalInsert {
                character_id,
                proposal_type,
                status: "auto_applied",
                confidence,
                title,
                rationale,
                source_memory_ids: &source_ids,
                target_memory_id: Some(keeper.id),
                proposed_content: Some(&merged_content),
                proposed_memory_type: Some(&keeper.memory_type),
                proposed_entity_key: keeper.entity_key.as_deref(),
                impact:
                    "Auto-merged duplicate active memories; source rows were marked superseded.",
            })
            .await?;

        self.record_memory_operation(MemoryOperationRecord {
            character_id,
            operation_type: proposal_type,
            actor: "dreaming",
            memory_id: Some(keeper.id),
            proposal_id: Some(proposal_id),
            before_json: Some(serde_json::json!({ "source_memory_ids": source_ids }).to_string()),
            after_json: Some(
                serde_json::json!({ "keeper_id": keeper.id, "superseded_ids": superseded_ids })
                    .to_string(),
            ),
        })
        .await?;
        Ok(1)
    }

    async fn create_semantic_review_proposal(
        &self,
        character_id: &str,
        a: &DreamCandidateEntry,
        b: &DreamCandidateEntry,
        similarity: f32,
    ) -> Result<i64> {
        let pair_entries = [a, b];
        let keeper = self.choose_dream_keeper(&pair_entries);
        self.create_dream_proposal(DreamProposalInsert {
            character_id,
            proposal_type: "semantic_review",
            status: "pending",
            confidence: f64::from(similarity),
            title: "Review similar memories",
            rationale:
                "These memories are semantically similar but below the automatic merge threshold.",
            source_memory_ids: &[a.id, b.id],
            target_memory_id: Some(keeper.id),
            proposed_content: Some(&keeper.content),
            proposed_memory_type: Some(&keeper.memory_type),
            proposed_entity_key: keeper.entity_key.as_deref(),
            impact: "Manual review required before superseding either memory.",
        })
        .await
    }

    fn build_llm_discovery_prompt(
        &self,
        entries: &[&DreamCandidateEntry],
        target_language: Option<&str>,
    ) -> String {
        let language_rule = target_language
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|language| {
                format!(
                    "Write proposed_content in {language}. Preserve proper nouns and exact user-provided identifiers."
                )
            })
            .unwrap_or_else(|| {
                "Write proposed_content in the same language as the relevant memories.".to_string()
            });

        let mut list = String::new();
        for entry in entries {
            let created = entry.created_at;
            let key = entry.entity_key.as_deref().unwrap_or("");
            let content = strip_structured_memory_prefix(&entry.content);
            list.push_str(&format!(
                "- id: {}\n  created_at: {}\n  type: {}\n  key: {}\n  importance: {:.2}\n  content: {}\n",
                entry.id, created, entry.memory_type, key, entry.importance, content
            ));
        }

        format!(
            "You are the Dream Discovery pass for a long-term memory system.\n\
             Your job is to discover non-obvious duplicate or mergeable memories that simple embeddings may miss.\n\
             Look for equivalent meaning across different wording, languages, abstraction levels, aliases, and time phrasing.\n\
             General preservation rule: when a group represents the same durable history, origin, identity, commitment, preference origin, relationship state, or other personally meaningful milestone, choose the earliest source as canonical_memory_id and treat later repeats as evidence. Use a newer source as canonical only when it is a correction or a clearly better updated fact.\n\
             Do not propose merges for memories that are merely about the same broad topic.\n\
             Do not delete or decide final state; only propose review groups.\n\
             {language_rule}\n\n\
             Return ONLY JSON in this schema:\n\
             {{\"proposals\":[{{\"source_memory_ids\":[1,2],\"canonical_memory_id\":1,\"decision\":\"duplicate|merge|conflict|update\",\"confidence\":0.0,\"proposed_content\":\"optional canonical memory\",\"memory_type\":\"optional\",\"entity_key\":\"optional\"}}]}}\n\n\
             Memory entries:\n{list}"
        )
    }

    async fn run_llm_discovery_batch(
        &self,
        character_id: &str,
        entries: &[&DreamCandidateEntry],
        provider: &std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
        target_language: Option<&str>,
    ) -> Result<i64> {
        use crate::llm::messages::user_text_message;

        if entries.len() < 2 {
            return Ok(0);
        }

        let prompt = self.build_llm_discovery_prompt(entries, target_language);
        let response = provider
            .chat(vec![user_text_message(prompt)], None)
            .await
            .map_err(|error| anyhow::anyhow!("Dream discovery LLM scan failed: {}", error))?;
        let json = strip_code_fences_for_memory_json(&response);
        let discovery: DreamDiscoveryResponse = serde_json::from_str(json)
            .map_err(|error| anyhow::anyhow!("Dream discovery JSON parse failed: {}", error))?;

        let by_id: HashMap<i64, &DreamCandidateEntry> =
            entries.iter().map(|entry| (entry.id, *entry)).collect();
        let mut proposals = 0i64;
        for proposal in discovery.proposals {
            if proposals >= DREAM_LLM_DISCOVERY_MAX_PROPOSALS_PER_RUN {
                break;
            }
            let confidence = proposal.confidence.clamp(0.0, 1.0);
            if confidence < DREAM_LLM_DISCOVERY_MIN_CONFIDENCE {
                continue;
            }

            let mut source_ids = proposal.source_memory_ids;
            source_ids.sort_unstable();
            source_ids.dedup();
            if source_ids.len() < 2 {
                continue;
            }
            if source_ids.iter().any(|id| !by_id.contains_key(id)) {
                continue;
            }

            let source_json = proposal_ids_json(&source_ids)?;
            let existing: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM memory_dream_proposals \
                 WHERE character_id = ? AND status = 'pending' AND source_memory_ids = ? \
                 LIMIT 1",
            )
            .bind(character_id)
            .bind(&source_json)
            .fetch_optional(&self.db)
            .await?;
            if existing.is_some() {
                continue;
            }

            let source_entries: Vec<&DreamCandidateEntry> = source_ids
                .iter()
                .filter_map(|id| by_id.get(id).copied())
                .collect();
            let target = self
                .choose_dream_keeper_with_preference(&source_entries, proposal.canonical_memory_id);
            let decision = proposal.decision.trim().to_lowercase();
            let proposal_type = match decision.as_str() {
                "conflict" => "llm_discovery_conflict_review",
                "update" => "llm_discovery_update_review",
                _ => "llm_discovery_review",
            };
            let (title, rationale) = match proposal_type {
                "llm_discovery_conflict_review" => (
                    "Review LLM-discovered memory conflict",
                    "LLM Dream Discovery found a non-obvious possible conflict between these memories.",
                ),
                "llm_discovery_update_review" => (
                    "Review LLM-discovered memory update",
                    "LLM Dream Discovery found a non-obvious possible update between these memories.",
                ),
                _ => (
                    "Review LLM-discovered memory relation",
                    "LLM Dream Discovery found a non-obvious relationship between these memories.",
                ),
            };
            let fallback_content = strip_structured_memory_prefix(&target.content).to_string();

            self.create_dream_proposal(DreamProposalInsert {
                character_id,
                proposal_type,
                status: "pending",
                confidence,
                title,
                rationale,
                source_memory_ids: &source_ids,
                target_memory_id: Some(target.id),
                proposed_content: proposal
                    .proposed_content
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .or(Some(fallback_content.as_str())),
                proposed_memory_type: proposal.memory_type.as_deref().or(Some(&target.memory_type)),
                proposed_entity_key: proposal.entity_key.as_deref().or(target.entity_key.as_deref()),
                impact: "Manual review required because this relation was discovered by LLM rather than deterministic similarity.",
            })
            .await?;
            proposals += 1;
        }

        Ok(proposals)
    }

    async fn run_llm_discovery_pass(
        &self,
        character_id: &str,
        entries: &[DreamCandidateEntry],
        provider: Option<&std::sync::Arc<dyn crate::llm::provider::LlmProvider>>,
        target_language: Option<&str>,
    ) -> Result<i64> {
        let Some(provider) = provider else {
            return Ok(0);
        };
        if entries.len() < 2 {
            return Ok(0);
        }

        let mut total = 0i64;
        let step = DREAM_LLM_DISCOVERY_BATCH_SIZE
            .saturating_sub(DREAM_LLM_DISCOVERY_BATCH_OVERLAP)
            .max(1);
        let mut start = 0usize;
        while start < entries.len() && total < DREAM_LLM_DISCOVERY_MAX_PROPOSALS_PER_RUN {
            let end = (start + DREAM_LLM_DISCOVERY_BATCH_SIZE).min(entries.len());
            let batch: Vec<&DreamCandidateEntry> = entries[start..end].iter().collect();
            match self
                .run_llm_discovery_batch(character_id, &batch, provider, target_language)
                .await
            {
                Ok(count) => total += count,
                Err(error) => tracing::warn!(
                    target: "memory",
                    "[Memory] Dream discovery batch failed: {}",
                    error
                ),
            }
            if end == entries.len() {
                break;
            }
            start += step;
        }

        Ok(total)
    }

    fn build_dream_pair_prompt(
        &self,
        a: &DreamCandidateEntry,
        b: &DreamCandidateEntry,
        target_language: Option<&str>,
    ) -> String {
        let language_rule = target_language
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|language| {
                format!(
                    "Write merged_memory in {language}. Preserve proper nouns and exact user-provided identifiers."
                )
            })
            .unwrap_or_else(|| "Write merged_memory in the same language as the existing memories.".to_string());

        format!(
            "You are reviewing two long-term memory entries for a desktop companion app.\n\
             Decide whether they are the same durable fact, a safe merge, a conflict, or distinct.\n\
             Never mark memories as duplicate merely because they are about the same broad topic.\n\
             General preservation rule: when the pair represents the same durable history, origin, identity, commitment, preference origin, relationship state, or other personally meaningful milestone, choose the earliest source as canonical_memory_id and preserve that history in merged_memory. Treat later repeats as evidence. Use the newer source as canonical only when it is a correction or clearly better updated fact.\n\
             {language_rule}\n\n\
             Return ONLY JSON in this schema:\n\
             {{\"decision\":\"duplicate|merge|conflict|distinct\",\"confidence\":0.0,\"canonical_memory_id\":1,\"merged_memory\":\"optional\",\"rationale\":\"short\",\"memory_type\":\"optional\",\"entity_key\":\"optional\"}}\n\n\
             Memory A: {}\n\
             id: {}\n\
             type: {}, key: {}\n\n\
             Memory B: {}\n\
             id: {}\n\
             type: {}, key: {}",
            a.content,
            a.id,
            a.memory_type,
            a.entity_key.as_deref().unwrap_or(""),
            b.content,
            b.id,
            b.memory_type,
            b.entity_key.as_deref().unwrap_or("")
        )
    }

    async fn assess_dream_pair_via_llm(
        &self,
        a: &DreamCandidateEntry,
        b: &DreamCandidateEntry,
        provider: &std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
        target_language: Option<&str>,
    ) -> Result<DreamPairAssessment> {
        use crate::llm::messages::user_text_message;

        let prompt = self.build_dream_pair_prompt(a, b, target_language);
        let response = provider
            .chat(vec![user_text_message(prompt)], None)
            .await
            .map_err(|error| anyhow::anyhow!("Dream pair LLM assessment failed: {}", error))?;
        let json = strip_code_fences_for_memory_json(&response);
        let assessment: DreamPairAssessment = serde_json::from_str(json).map_err(|error| {
            anyhow::anyhow!("Dream pair LLM assessment JSON parse failed: {}", error)
        })?;
        Ok(assessment)
    }

    async fn run_dream_pass(
        &self,
        character_id: &str,
        provider: Option<&std::sync::Arc<dyn crate::llm::provider::LlmProvider>>,
        target_language: Option<&str>,
    ) -> Result<(i64, i64)> {
        let entries = self.load_dream_entries(character_id).await?;
        let mut processed: HashSet<i64> = HashSet::new();
        let mut auto_applied = 0i64;
        let mut proposals = 0i64;

        let mut by_hash: HashMap<String, Vec<&DreamCandidateEntry>> = HashMap::new();
        for entry in &entries {
            let hash = entry
                .canonical_hash
                .clone()
                .unwrap_or_else(|| canonical_hash(&entry.content));
            by_hash.entry(hash).or_default().push(entry);
        }
        for group in by_hash.values() {
            if group.len() < 2 || group.iter().any(|entry| processed.contains(&entry.id)) {
                continue;
            }
            auto_applied += self
                .auto_merge_dream_group(DreamAutoMergeRequest {
                    character_id,
                    entries: group,
                    proposal_type: "canonical_duplicate",
                    title: "Merged exact duplicate memories",
                    rationale: "Dream Light found duplicate canonical hashes.",
                    confidence: 1.0,
                    proposed_content_override: None,
                    preferred_keeper_id: None,
                })
                .await?;
            for entry in group {
                processed.insert(entry.id);
            }
        }

        let mut by_entity: HashMap<String, Vec<&DreamCandidateEntry>> = HashMap::new();
        for entry in &entries {
            if processed.contains(&entry.id) {
                continue;
            }
            if let Some(key) = &entry.entity_key {
                by_entity
                    .entry(format!("{}:{}", entry.memory_type, key))
                    .or_default()
                    .push(entry);
            }
        }
        for group in by_entity.values() {
            if group.len() < 2 {
                continue;
            }
            auto_applied += self
                .auto_merge_dream_group(DreamAutoMergeRequest {
                    character_id,
                    entries: group,
                    proposal_type: "entity_slot_merge",
                    title: "Merged duplicate slot memories",
                    rationale:
                        "Dream REM found multiple active memories for the same structured slot.",
                    confidence: DREAM_CONFIDENCE_AUTO_APPLY,
                    proposed_content_override: None,
                    preferred_keeper_id: None,
                })
                .await?;
            for entry in group {
                processed.insert(entry.id);
            }
        }

        let mut reviewed_pairs: HashSet<(i64, i64)> = HashSet::new();
        for (i, a) in entries.iter().enumerate() {
            if processed.contains(&a.id) {
                continue;
            }
            for b in entries.iter().skip(i + 1) {
                if processed.contains(&b.id) {
                    continue;
                }
                let pair = if a.id < b.id {
                    (a.id, b.id)
                } else {
                    (b.id, a.id)
                };
                if !reviewed_pairs.insert(pair) {
                    continue;
                }
                let sim = cosine_similarity(&a.embedding, &b.embedding);
                if sim >= DREAM_SEMANTIC_AUTO_MERGE_THRESHOLD {
                    if let Some(provider) = provider {
                        match self
                            .assess_dream_pair_via_llm(a, b, provider, target_language)
                            .await
                        {
                            Ok(assessment) => {
                                let decision = assessment.decision.trim().to_lowercase();
                                let confidence = assessment.confidence.clamp(0.0, 1.0);
                                if matches!(decision.as_str(), "duplicate" | "merge" | "update")
                                    && confidence >= DREAM_CONFIDENCE_AUTO_APPLY
                                {
                                    let pair_entries = [a, b];
                                    auto_applied += self
                                        .auto_merge_dream_group(DreamAutoMergeRequest {
                                            character_id,
                                            entries: &pair_entries,
                                            proposal_type: "llm_semantic_auto_merge",
                                            title: "Merged LLM-confirmed memories",
                                            rationale: assessment
                                                .rationale
                                                .as_deref()
                                                .unwrap_or("LLM confirmed these memories are safely mergeable."),
                                            confidence,
                                            proposed_content_override: assessment.merged_memory.as_deref(),
                                            preferred_keeper_id: assessment.canonical_memory_id,
                                        })
                                        .await?;
                                    processed.insert(a.id);
                                    processed.insert(b.id);
                                    break;
                                }

                                if decision == "conflict" {
                                    let pair_entries = [a, b];
                                    let keeper = self.choose_dream_keeper(&pair_entries);
                                    self.create_dream_proposal(DreamProposalInsert {
                                        character_id,
                                        proposal_type: "llm_conflict_review",
                                        status: "pending",
                                        confidence,
                                        title: "Review LLM-detected memory conflict",
                                        rationale: assessment
                                            .rationale
                                            .as_deref()
                                            .unwrap_or("LLM detected a possible contradiction."),
                                        source_memory_ids: &[a.id, b.id],
                                        target_memory_id: Some(keeper.id),
                                        proposed_content: assessment.merged_memory.as_deref(),
                                        proposed_memory_type: assessment.memory_type.as_deref(),
                                        proposed_entity_key: assessment.entity_key.as_deref(),
                                        impact: "Manual review required before changing either active memory.",
                                    })
                                    .await?;
                                    proposals += 1;
                                    continue;
                                }

                                if decision == "distinct" {
                                    continue;
                                }

                                let pair_entries = [a, b];
                                let keeper = self.choose_dream_keeper(&pair_entries);
                                self.create_dream_proposal(DreamProposalInsert {
                                    character_id,
                                    proposal_type: "llm_semantic_review",
                                    status: "pending",
                                    confidence,
                                    title: "Review LLM memory merge suggestion",
                                    rationale: assessment
                                        .rationale
                                        .as_deref()
                                        .unwrap_or("LLM suggested reviewing these similar memories."),
                                    source_memory_ids: &[a.id, b.id],
                                    target_memory_id: Some(keeper.id),
                                    proposed_content: assessment.merged_memory.as_deref(),
                                    proposed_memory_type: assessment.memory_type.as_deref(),
                                    proposed_entity_key: assessment.entity_key.as_deref(),
                                    impact: "Manual review required because confidence was below the auto-apply threshold.",
                                })
                                .await?;
                                proposals += 1;
                                continue;
                            }
                            Err(error) => {
                                tracing::warn!(
                                    target: "memory",
                                    "[Memory] Dream LLM pair assessment failed; falling back to deterministic merge: {}",
                                    error
                                );
                            }
                        }
                    }
                    let pair_entries = [a, b];
                    auto_applied += self
                        .auto_merge_dream_group(DreamAutoMergeRequest {
                            character_id,
                            entries: &pair_entries,
                            proposal_type: "semantic_auto_merge",
                            title: "Merged highly similar memories",
                            rationale: "Dream Deep found a high-confidence semantic duplicate.",
                            confidence: f64::from(sim),
                            proposed_content_override: None,
                            preferred_keeper_id: None,
                        })
                        .await?;
                    processed.insert(a.id);
                    processed.insert(b.id);
                    break;
                } else if sim >= DREAM_SEMANTIC_REVIEW_THRESHOLD {
                    let existing: Option<i64> = sqlx::query_scalar(
                        "SELECT id FROM memory_dream_proposals \
                         WHERE character_id = ? AND status = 'pending' AND source_memory_ids = ? \
                         LIMIT 1",
                    )
                    .bind(character_id)
                    .bind(proposal_ids_json(&[a.id, b.id])?)
                    .fetch_optional(&self.db)
                    .await?;
                    if existing.is_none() {
                        self.create_semantic_review_proposal(character_id, a, b, sim)
                            .await?;
                        proposals += 1;
                    }
                }
            }
        }

        proposals += self
            .run_llm_discovery_pass(character_id, &entries, provider, target_language)
            .await?;

        Ok((auto_applied, proposals))
    }

    pub async fn run_dream_now(
        &self,
        character_id: &str,
        trigger: &str,
    ) -> Result<MemoryDreamRunResult> {
        self.run_dream_now_with_provider(character_id, trigger, None, None)
            .await
    }

    pub async fn run_dream_now_with_provider(
        &self,
        character_id: &str,
        trigger: &str,
        provider: Option<std::sync::Arc<dyn crate::llm::provider::LlmProvider>>,
        target_language: Option<String>,
    ) -> Result<MemoryDreamRunResult> {
        let started_at = now_ts();
        let job_id = sqlx::query(
            "INSERT INTO memory_dream_jobs (character_id, phase, status, trigger, started_at) \
             VALUES (?, 'full', 'running', ?, ?)",
        )
        .bind(character_id)
        .bind(trigger)
        .bind(started_at)
        .execute(&self.db)
        .await?
        .last_insert_rowid();

        match self
            .run_dream_pass(character_id, provider.as_ref(), target_language.as_deref())
            .await
        {
            Ok((auto_applied_count, proposal_count)) => {
                sqlx::query(
                    "UPDATE memory_dream_jobs \
                     SET status = 'ready', finished_at = ?, auto_applied_count = ?, proposal_count = ? \
                     WHERE id = ?",
                )
                .bind(now_ts())
                .bind(auto_applied_count)
                .bind(proposal_count)
                .bind(job_id)
                .execute(&self.db)
                .await?;
                let job = self
                    .latest_dream_job(character_id)
                    .await?
                    .expect("dream job should exist after update");
                Ok(MemoryDreamRunResult {
                    job,
                    auto_applied_count,
                    proposal_count,
                })
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE memory_dream_jobs SET status = 'failed', finished_at = ?, error = ? WHERE id = ?",
                )
                .bind(now_ts())
                .bind(error.to_string())
                .bind(job_id)
                .execute(&self.db)
                .await?;
                Err(error)
            }
        }
    }

    pub async fn latest_dream_job(
        &self,
        character_id: &str,
    ) -> Result<Option<MemoryDreamJobRecord>> {
        Ok(sqlx::query_as::<_, MemoryDreamJobRecord>(
            "SELECT id, character_id, phase, status, trigger, started_at, finished_at, \
                    auto_applied_count, proposal_count, error \
             FROM memory_dream_jobs WHERE character_id = ? ORDER BY started_at DESC LIMIT 1",
        )
        .bind(character_id)
        .fetch_optional(&self.db)
        .await?)
    }

    pub async fn list_dream_jobs(
        &self,
        character_id: &str,
        limit: i64,
    ) -> Result<Vec<MemoryDreamJobRecord>> {
        Ok(sqlx::query_as::<_, MemoryDreamJobRecord>(
            "SELECT id, character_id, phase, status, trigger, started_at, finished_at, \
                    auto_applied_count, proposal_count, error \
             FROM memory_dream_jobs WHERE character_id = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(character_id)
        .bind(limit)
        .fetch_all(&self.db)
        .await?)
    }

    pub async fn list_dream_proposals(
        &self,
        character_id: &str,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<MemoryDreamProposalRecord>> {
        let status = status.unwrap_or("pending");
        let rows = sqlx::query_as::<_, MemoryDreamProposalRow>(
            "SELECT id, character_id, proposal_type, status, confidence, title, rationale, \
                    source_memory_ids, target_memory_id, proposed_content, proposed_memory_type, \
                    proposed_entity_key, impact, created_at, updated_at, applied_at \
             FROM memory_dream_proposals \
             WHERE character_id = ? AND status = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(character_id)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.db)
        .await?;

        let mut proposals = Vec::with_capacity(rows.len());
        for row in rows {
            let source_ids = parse_proposal_ids(&row.source_memory_ids);
            let source_memories = self.load_dream_source_memories(&source_ids).await?;
            proposals.push(row.into_record(source_memories));
        }

        Ok(proposals)
    }

    async fn load_dream_source_memories(
        &self,
        source_ids: &[i64],
    ) -> Result<Vec<MemoryDreamSourceRecord>> {
        let mut memories = Vec::with_capacity(source_ids.len());
        for id in source_ids {
            if let Some(memory) = sqlx::query_as::<_, MemoryDreamSourceRecord>(
                "SELECT id, content, created_at, updated_at, importance, tier, memory_type, \
                        entity_key, status \
                 FROM memories WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.db)
            .await?
            {
                memories.push(memory);
            }
        }
        Ok(memories)
    }

    pub async fn dreaming_summary(&self, character_id: &str) -> Result<MemoryDreamingSummary> {
        let latest_job = self.latest_dream_job(character_id).await?;
        let pending_proposal_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_dream_proposals WHERE character_id = ? AND status = 'pending'",
        )
        .bind(character_id)
        .fetch_one(&self.db)
        .await?;
        let auto_applied_proposal_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_dream_proposals WHERE character_id = ? AND status = 'auto_applied'",
        )
        .bind(character_id)
        .fetch_one(&self.db)
        .await?;
        Ok(MemoryDreamingSummary {
            latest_job,
            pending_proposal_count,
            auto_applied_proposal_count,
        })
    }

    pub async fn reject_dream_proposal(&self, proposal_id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE memory_dream_proposals SET status = 'rejected', updated_at = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(now_ts())
        .bind(proposal_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn approve_dream_proposal(&self, proposal_id: i64) -> Result<()> {
        let proposal = sqlx::query_as::<_, MemoryDreamProposalRow>(
            "SELECT id, character_id, proposal_type, status, confidence, title, rationale, \
                    source_memory_ids, target_memory_id, proposed_content, proposed_memory_type, \
                    proposed_entity_key, impact, created_at, updated_at, applied_at \
             FROM memory_dream_proposals WHERE id = ?",
        )
        .bind(proposal_id)
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("dream proposal not found"))?;

        if proposal.status != "pending" {
            return Ok(());
        }

        let source_ids = parse_proposal_ids(&proposal.source_memory_ids);
        let target_id = proposal
            .target_memory_id
            .or_else(|| source_ids.first().copied())
            .ok_or_else(|| anyhow::anyhow!("dream proposal has no target memory"))?;
        let now = now_ts();

        if let Some(content) = proposal.proposed_content.as_deref() {
            let embedding = self.embed(content).await?;
            let embedding_bytes = bincode::serialize(&embedding)?;
            let metadata = MemoryMetadata {
                memory_type: proposal
                    .proposed_memory_type
                    .clone()
                    .unwrap_or_else(|| "fact".to_string()),
                entity_key: proposal.proposed_entity_key.clone(),
                canonical_content: None,
            };
            sqlx::query(
                "UPDATE memories SET content = ?, embedding = ?, memory_type = ?, entity_key = ?, \
                    canonical_hash = ?, confidence = ?, updated_at = ?, last_seen_at = ?, status = 'active' \
                 WHERE id = ?",
            )
            .bind(content)
            .bind(embedding_bytes)
            .bind(metadata.memory_type)
            .bind(metadata.entity_key)
            .bind(canonical_hash(content))
            .bind(proposal.confidence)
            .bind(now)
            .bind(now)
            .bind(target_id)
            .execute(&self.db)
            .await?;
        }

        let superseded_ids: Vec<i64> = source_ids
            .iter()
            .copied()
            .filter(|id| *id != target_id)
            .collect();
        if !superseded_ids.is_empty() {
            for superseded_id in &superseded_ids {
                sqlx::query(
                    "UPDATE memories SET status = 'superseded', updated_at = ?, supersedes = ? WHERE id = ?",
                )
                .bind(now)
                .bind(proposal_ids_json(&[target_id])?)
                .bind(superseded_id)
                .execute(&self.db)
                .await?;
            }
        }

        sqlx::query(
            "UPDATE memory_dream_proposals SET status = 'approved', updated_at = ?, applied_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(proposal_id)
        .execute(&self.db)
        .await?;

        self.record_memory_operation(MemoryOperationRecord {
            character_id: &proposal.character_id,
            operation_type: &proposal.proposal_type,
            actor: "user",
            memory_id: Some(target_id),
            proposal_id: Some(proposal_id),
            before_json: Some(serde_json::json!({ "source_memory_ids": source_ids }).to_string()),
            after_json: Some(
                serde_json::json!({ "target_id": target_id, "superseded_ids": superseded_ids })
                    .to_string(),
            ),
        })
        .await?;
        Ok(())
    }

    /// Delete ephemeral memories whose effective importance (original × decay) has fallen
    /// below `threshold`. Core memories are never pruned.
    /// Returns the number of deleted rows.
    pub async fn prune_decayed_memories(&self, character_id: &str, threshold: f64) -> Result<u64> {
        let now = chrono::Utc::now().timestamp();
        // Compute the minimum age (in seconds) at which a memory with max importance (1.0)
        // would decay below the threshold:
        //   threshold = importance * 0.5^(age_days / half_life)
        // Worst case importance = 1.0, so:
        //   age_days > half_life * log2(1 / threshold)
        let min_age_days = MEMORY_HALF_LIFE_DAYS * (1.0_f64 / threshold).log2();
        let cutoff_ts = now - (min_age_days * 86400.0) as i64;

        // Among records old enough to potentially be below threshold, check each one.
        // We do the exact per-row check in Rust to handle varying importance values.
        let rows = sqlx::query(
            "SELECT rowid, created_at, importance FROM memories \
             WHERE character_id = ? AND tier = 'ephemeral' AND status = 'active' AND created_at < ?",
        )
        .bind(character_id)
        .bind(cutoff_ts)
        .fetch_all(&self.db)
        .await?;

        let mut deleted = 0u64;
        for row in rows {
            let id: i64 = row.get("rowid");
            let created_at: i64 = row.get("created_at");
            let importance: f64 = row.get("importance");
            let age_days = (now - created_at) as f64 / 86400.0;
            let decay = (0.5_f64).powf(age_days / MEMORY_HALF_LIFE_DAYS);
            if importance * decay < threshold {
                sqlx::query(
                    "UPDATE memories SET status = 'archived', updated_at = ? WHERE rowid = ?",
                )
                .bind(now)
                .bind(id)
                .execute(&self.db)
                .await?;
                deleted += 1;
            }
        }

        if deleted > 0 {
            tracing::info!(
                target: "memory",
                "[Memory] Pruned {} decayed ephemeral memories for '{}'",
                deleted, character_id
            );
        }
        Ok(deleted)
    }
}

// ── Memory Consolidation ──────────────────────────────────────

impl MemoryManager {
    /// Find clusters of similar memories and merge them via LLM.
    /// Inserts consolidated memories and deletes the source fragments.
    pub async fn consolidate_memories(
        &self,
        character_id: &str,
        provider: std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
    ) -> Result<usize> {
        self.consolidate_memories_with_language(character_id, provider, None)
            .await
    }

    /// Find clusters of similar memories and merge them via LLM, preserving the
    /// configured assistant response language for newly written entries.
    pub async fn consolidate_memories_with_language(
        &self,
        character_id: &str,
        provider: std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
        target_language: Option<String>,
    ) -> Result<usize> {
        let result = self
            .run_dream_now_with_provider(
                character_id,
                "periodic_consolidation",
                Some(provider.clone()),
                target_language.clone(),
            )
            .await?;
        return Ok((result.auto_applied_count + result.proposal_count) as usize);

        #[allow(unreachable_code)]
        {
            // 1. Load all memories with embeddings for this character
            let rows = sqlx::query(
            "SELECT id, content, embedding, created_at, importance, tier FROM memories WHERE character_id = ?",
        )
        .bind(character_id)
        .fetch_all(&self.db)
        .await?;

            if rows.len() < 2 {
                return Ok(0);
            }

            // Parse into (id, content, embedding, importance, tier, created_at)
            let mut entries: Vec<(i64, String, Vec<f32>, f64, String, i64)> = Vec::new();
            for row in &rows {
                let embedding_bytes: Vec<u8> = row.get("embedding");
                let embedding: Vec<f32> = bincode::deserialize(&embedding_bytes)?;
                entries.push((
                    row.get("id"),
                    row.get("content"),
                    embedding,
                    row.get("importance"),
                    row.get("tier"),
                    row.get("created_at"),
                ));
            }

            // 2. Greedy clustering: group similar memories
            let mut used = vec![false; entries.len()];
            let mut clusters: Vec<Vec<usize>> = Vec::new();

            for i in 0..entries.len() {
                if used[i] {
                    continue;
                }
                let mut cluster = vec![i];
                used[i] = true;

                for j in (i + 1)..entries.len() {
                    if used[j] || cluster.len() >= MAX_CLUSTER_SIZE {
                        break;
                    }
                    let sim = cosine_similarity(&entries[i].2, &entries[j].2);
                    let time_diff = (entries[i].5 - entries[j].5).abs();
                    if sim > CONSOLIDATION_THRESHOLD && time_diff <= CONSOLIDATION_TIME_WINDOW_SECS
                    {
                        cluster.push(j);
                        used[j] = true;
                    }
                }

                // Only consolidate clusters with 2+ memories
                if cluster.len() >= 2 {
                    clusters.push(cluster);
                }
            }

            if clusters.is_empty() {
                return Ok(0);
            }

            let mut consolidated_count = 0;

            // 3. For each cluster, merge via LLM
            for cluster in &clusters {
                let facts: Vec<&str> = cluster.iter().map(|&idx| entries[idx].1.as_str()).collect();
                let source_ids: Vec<i64> = cluster.iter().map(|&idx| entries[idx].0).collect();

                // Inherit max importance; if any is core, result is core
                let max_importance = cluster
                    .iter()
                    .map(|&idx| entries[idx].3)
                    .fold(0.0_f64, f64::max);
                let tier = if cluster.iter().any(|&idx| entries[idx].4 == "core") {
                    "core"
                } else {
                    "ephemeral"
                };
                // 保留最早的 created_at，避免整合后记忆时间被重置
                let earliest_created_at = cluster
                    .iter()
                    .map(|&idx| entries[idx].5)
                    .min()
                    .unwrap_or_else(|| chrono::Utc::now().timestamp());

                // Call LLM to merge facts
                let merged = match merge_facts_via_llm(
                    &facts,
                    &provider,
                    target_language.as_deref(),
                )
                .await
                {
                    Ok(text) => text,
                    Err(e) => {
                        tracing::error!(target: "memory", "[Memory] Consolidation LLM call failed: {}", e);
                        continue;
                    }
                };

                if merged.trim().is_empty() {
                    continue;
                }

                // 4. Insert consolidated memory
                let embedding = match self.embed(&merged).await {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::error!(target: "memory", "[Memory] Failed to embed consolidated memory: {}", e);
                        continue;
                    }
                };
                let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
                let now = chrono::Utc::now().timestamp();
                let consolidated_from_json = serde_json::to_string(&source_ids)?;

                sqlx::query(
                "INSERT INTO memories (content, embedding, created_at, updated_at, importance, character_id, tier, consolidated_from) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&merged)
            .bind(&embedding_bytes)
            .bind(earliest_created_at)
            .bind(now)
            .bind(max_importance)
            .bind(character_id)
            .bind(tier)
            .bind(&consolidated_from_json)
            .execute(&self.db)
            .await?;

                // 5. Delete source memories
                for id in &source_ids {
                    sqlx::query("DELETE FROM memories WHERE id = ?")
                        .bind(id)
                        .execute(&self.db)
                        .await?;
                }

                consolidated_count += 1;
                tracing::info!(
                    target: "memory",
                    "[Memory] Consolidated {} memories into: {}",
                    source_ids.len(),
                    &merged[..merged.len().min(80)]
                );
            }

            Ok(consolidated_count)
        }
    }
}

/// Row type for paginated memory listing.
#[derive(sqlx::FromRow)]
struct MemoryRow {
    rowid: i64,
    content: String,
    created_at: i64,
    importance: f64,
    tier: String,
    memory_type: String,
    entity_key: Option<String>,
    status: String,
    confidence: f64,
    first_seen_at: i64,
    last_seen_at: i64,
    evidence_count: i64,
}

/// Public record type returned to frontend via Tauri commands.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryRecord {
    pub id: i64,
    pub content: String,
    pub created_at: i64,
    pub importance: f64,
    pub tier: String,
    pub memory_type: String,
    pub entity_key: Option<String>,
    pub status: String,
    pub confidence: f64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub evidence_count: i64,
}

/// Escape user input for FTS5 MATCH syntax.
/// Wraps each word in double quotes and joins with OR.
pub(crate) fn escape_fts5_query(query: &str) -> String {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            // Remove any double quotes from the word to prevent injection
            w.replace('"', "")
        })
        .filter(|w| !w.is_empty())
        .map(|clean| format!("\"{}\"", clean))
        .collect();
    words.join(" OR ")
}

pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot_product: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a * norm_b)
    }
}

/// Lightweight contradiction heuristic: checks if one statement has negation markers
/// the other lacks. Catches "likes cats" vs "doesn't like cats" without an LLM call.
/// False-positive rate is acceptable — only triggers inside the 0.70–0.95 similarity band.
fn is_likely_contradiction(a: &str, b: &str) -> bool {
    const NEGATIONS: &[&str] = &[
        "not",
        "no",
        "never",
        "don't",
        "dont",
        "doesn't",
        "doesnt",
        "won't",
        "wont",
        "can't",
        "cant",
        "hate",
        "dislike",
        "不",
        "没",
        "从不",
        "不喜欢",
        "讨厌",
        "不想",
        "不会",
        "没有",
    ];
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    let a_neg = NEGATIONS.iter().any(|n| a_lower.contains(n));
    let b_neg = NEGATIONS.iter().any(|n| b_lower.contains(n));
    // Contradiction = exactly one side has a negation marker
    a_neg != b_neg
}

/// Use LLM to merge multiple related facts into a single consolidated memory.
async fn merge_facts_via_llm(
    facts: &[&str],
    provider: &std::sync::Arc<dyn crate::llm::provider::LlmProvider>,
    target_language: Option<&str>,
) -> Result<String> {
    use crate::llm::messages::user_text_message;

    let prompt = build_merge_facts_prompt(facts, target_language);
    let messages = vec![user_text_message(prompt)];

    let result = provider
        .chat(messages, None)
        .await
        .map_err(|e| anyhow::anyhow!("LLM merge failed: {}", e))?;

    Ok(result.trim().to_string())
}

fn build_memory_language_requirement(target_language: Option<&str>) -> String {
    let Some(language) = target_language
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return String::new();
    };

    format!(
        " Write the merged memory entry in {language}. \
         If the source facts use another language, translate or summarize them into {language}. \
         Preserve proper nouns, code identifiers, product names, and exact quoted phrases only when necessary."
    )
}

fn build_merge_facts_prompt(facts: &[&str], target_language: Option<&str>) -> String {
    let facts_list = facts
        .iter()
        .enumerate()
        .map(|(i, f)| format!("{}. {}", i + 1, f))
        .collect::<Vec<_>>()
        .join("\n");
    let language_requirement = build_memory_language_requirement(target_language);

    format!(
        "You are a memory consolidation assistant. Merge the following related facts into a single, \
         concise, and complete memory entry. Preserve all important details. Do not add information \
         that is not present in the original facts.{language_requirement} \
         Output only the merged memory text, nothing else.\n\n\
         Facts:\n{}",
        facts_list
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_snippet_for_sort(id: i64, importance: f64, created_at: i64) -> MemorySnippet {
        MemorySnippet {
            id,
            content: format!("memory {id}"),
            embedding: Vec::new(),
            created_at,
            importance,
            tier: "ephemeral".to_string(),
        }
    }

    #[test]
    fn compare_scored_memory_uses_stable_tie_breaks() {
        let mut items = vec![
            (0.50, memory_snippet_for_sort(3, 0.4, 100)),
            (0.70, memory_snippet_for_sort(5, 0.1, 10)),
            (0.50, memory_snippet_for_sort(4, 0.8, 90)),
            (0.50, memory_snippet_for_sort(2, 0.8, 120)),
            (0.50, memory_snippet_for_sort(1, 0.8, 120)),
        ];

        items.sort_by(|left, right| compare_scored_memory(left.0, &left.1, right.0, &right.1));

        let ids = items
            .into_iter()
            .map(|(_, memory)| memory.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![5, 1, 2, 4, 3]);
    }

    #[test]
    fn retrieval_eval_summary_is_hidden_when_flag_disabled() {
        let summary = if is_retrieval_eval_enabled() {
            Some(build_retrieval_eval_summary(&MemoryRetrievalLogRecord {
                query: "hello".to_string(),
                semantic_candidates: 1,
                bm25_candidates: 1,
                fused_candidates: 2,
                injected_count: 1,
                overlap_count: None,
                semantic_only_count: None,
                bm25_only_count: None,
                filtered_out_count: None,
            }))
        } else {
            None
        };

        if is_retrieval_eval_enabled() {
            assert!(summary.is_some());
        } else {
            assert!(summary.is_none());
        }
    }

    #[test]
    fn merge_facts_prompt_includes_target_memory_language() {
        let prompt = build_merge_facts_prompt(&["The user likes cats."], Some("日本語"));

        assert!(prompt.contains("Write the merged memory entry in 日本語"));
        assert!(prompt.contains("translate or summarize them into 日本語"));
    }

    #[test]
    fn build_write_observation_for_event_trigger() {
        let observation = build_write_observation_record(
            "char-1",
            "chat",
            "event_profile",
            WriteObservationCounts {
                extracted_count: 1,
                stored_count: 1,
                deduplicated_count: 0,
                invalidated_count: 0,
                duration_ms: 10,
            },
        )
        .expect("observation");

        assert_eq!(observation.trigger, "event_profile");
    }

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 0.0001,
            "Identical vectors should have similarity 1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 0.0001,
            "Orthogonal vectors should have similarity ~0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - (-1.0)).abs() < 0.0001,
            "Opposite vectors should have similarity -1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_zero_vector_a() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(
            sim, 0.0,
            "Zero vector should produce similarity 0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_zero_vector_b() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(
            sim, 0.0,
            "Zero vector should produce similarity 0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_both_zero_vectors() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(
            sim, 0.0,
            "Both zero vectors should produce similarity 0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_similarity_scaled_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![2.0, 4.0, 6.0]; // b = 2 * a
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 0.0001,
            "Scaled vectors should have similarity 1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_escape_fts5_query_empty_string() {
        let result = escape_fts5_query("");
        assert_eq!(result, "", "Empty string should produce empty result");
    }

    #[test]
    fn test_escape_fts5_query_single_word() {
        let result = escape_fts5_query("hello");
        assert_eq!(
            result, "\"hello\"",
            "Single word should be wrapped in quotes"
        );
    }

    #[test]
    fn test_escape_fts5_query_multiple_words() {
        let result = escape_fts5_query("hello world test");
        assert_eq!(
            result, "\"hello\" OR \"world\" OR \"test\"",
            "Multiple words should be joined with OR"
        );
    }

    #[test]
    fn test_escape_fts5_query_with_embedded_quotes() {
        let result = escape_fts5_query("hello \"world\" test");
        assert_eq!(
            result, "\"hello\" OR \"world\" OR \"test\"",
            "Embedded quotes should be removed for injection prevention"
        );
    }

    #[test]
    fn test_escape_fts5_query_whitespace_only() {
        let result = escape_fts5_query("   \t  \n  ");
        assert_eq!(
            result, "",
            "Whitespace-only input should produce empty result"
        );
    }

    #[test]
    fn test_escape_fts5_query_mixed_whitespace() {
        let result = escape_fts5_query("  hello   world  ");
        assert_eq!(
            result, "\"hello\" OR \"world\"",
            "Extra whitespace should be normalized"
        );
    }

    #[test]
    fn test_escape_fts5_query_only_quotes() {
        let result = escape_fts5_query("\"\"\"");
        assert_eq!(
            result, "",
            "String with only quotes should produce empty result after filtering"
        );
    }

    #[test]
    fn test_escape_fts5_query_word_with_quotes() {
        let result = escape_fts5_query("hel\"lo wor\"ld");
        assert_eq!(
            result, "\"hello\" OR \"world\"",
            "Quotes within words should be removed"
        );
    }

    async fn setup_test_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory database");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");
        pool
    }

    #[tokio::test]
    async fn test_memory_manager_add_and_retrieve() {
        let pool = setup_test_pool().await;
        let manager = MemoryManager::new(pool);

        // Add a memory
        let content = "Test memory content";
        let char_id = "test_char";
        manager
            .add_memory(content, char_id)
            .await
            .expect("Failed to add memory");

        // Retrieve all memories for this character
        let memories = manager
            .get_all_memory_contents(char_id)
            .await
            .expect("Failed to retrieve memories");

        assert_eq!(
            memories.len(),
            1,
            "Should have exactly one memory after adding one"
        );
        assert_eq!(
            memories[0], content,
            "Retrieved memory content should match what was added"
        );
    }

    #[tokio::test]
    async fn test_memory_manager_character_isolation() {
        let pool = setup_test_pool().await;
        let manager = MemoryManager::new(pool);

        // Add memories for different characters
        manager
            .add_memory("Alice's memory", "alice")
            .await
            .expect("Failed to add Alice's memory");
        manager
            .add_memory("Bob's memory", "bob")
            .await
            .expect("Failed to add Bob's memory");

        // Retrieve memories for Alice
        let alice_memories = manager
            .get_all_memory_contents("alice")
            .await
            .expect("Failed to retrieve Alice's memories");

        // Retrieve memories for Bob
        let bob_memories = manager
            .get_all_memory_contents("bob")
            .await
            .expect("Failed to retrieve Bob's memories");

        assert_eq!(
            alice_memories.len(),
            1,
            "Alice should have exactly one memory"
        );
        assert_eq!(bob_memories.len(), 1, "Bob should have exactly one memory");
        assert_eq!(alice_memories[0], "Alice's memory");
        assert_eq!(bob_memories[0], "Bob's memory");
    }

    #[tokio::test]
    async fn test_memory_manager_empty_character() {
        let pool = setup_test_pool().await;
        let manager = MemoryManager::new(pool);

        // Retrieve memories for a character with no memories
        let memories = manager
            .get_all_memory_contents("nonexistent_char")
            .await
            .expect("Failed to retrieve memories");

        assert_eq!(
            memories.len(),
            0,
            "Should return empty list for character with no memories"
        );
    }

    #[tokio::test]
    async fn test_memory_manager_multiple_memories() {
        let pool = setup_test_pool().await;
        let manager = MemoryManager::new(pool);

        let char_id = "test_char";

        // Add multiple distinct memories (with unique content to avoid deduplication)
        let memories_to_add = vec![
            "The user likes to play chess on weekends",
            "The user works as a software engineer in San Francisco",
            "The user has a cat named Whiskers",
            "The user enjoys reading science fiction novels",
            "The user prefers coffee over tea in the morning",
        ];

        for content in &memories_to_add {
            manager
                .add_memory(content, char_id)
                .await
                .expect("Failed to add memory");
        }

        // Retrieve all memories
        let memories = manager
            .get_all_memory_contents(char_id)
            .await
            .expect("Failed to retrieve memories");

        assert_eq!(
            memories.len(),
            5,
            "Should have exactly 5 distinct memories after adding 5"
        );
    }

    // ── is_likely_contradiction unit tests ────────────────────────────────

    #[test]
    fn test_contradiction_one_side_negation_en() {
        // Exactly one side has negation → contradiction
        assert!(is_likely_contradiction(
            "The user likes cats",
            "The user doesn't like cats"
        ));
    }

    #[test]
    fn test_contradiction_one_side_negation_zh() {
        // Chinese negation keyword
        assert!(is_likely_contradiction("用户喜欢猫", "用户不喜欢猫"));
    }

    #[test]
    fn test_no_contradiction_both_have_negation() {
        // Both sides have negation → not a contradiction
        assert!(!is_likely_contradiction(
            "The user doesn't like dogs",
            "The user never eats meat"
        ));
    }

    #[test]
    fn test_no_contradiction_neither_has_negation() {
        // Neither side has negation → not a contradiction
        assert!(!is_likely_contradiction(
            "The user likes coffee",
            "The user drinks tea every morning"
        ));
    }

    #[test]
    fn test_contradiction_hate_keyword() {
        // 'hate' counts as negation
        assert!(is_likely_contradiction(
            "The user loves jazz music",
            "The user hates jazz music"
        ));
    }
}
