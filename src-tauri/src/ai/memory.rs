// pattern: Mixed (unavoidable)
// Reason: 该文件同时包含记忆领域规则、SQLite 读写、嵌入计算与摘要状态机；Phase 1 先在现有集中实现上做低侵入扩展。
use anyhow::Result;
#[cfg(not(test))]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRetrievalObservation {
    pub character_id: String,
    pub query: String,
    pub semantic_candidates: i64,
    pub bm25_candidates: i64,
    pub fused_candidates: i64,
    pub injected_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::FromRow)]
pub struct MemoryRetrievalLogRecord {
    pub query: String,
    pub semantic_candidates: i64,
    pub bm25_candidates: i64,
    pub fused_candidates: i64,
    pub injected_count: i64,
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
}

fn build_retrieval_observation(
    character_id: &str,
    query: &str,
    stats: &RetrievalCandidateStats,
) -> MemoryRetrievalObservation {
    MemoryRetrievalObservation {
        character_id: character_id.to_string(),
        query: query.to_string(),
        semantic_candidates: stats.semantic_candidates as i64,
        bm25_candidates: stats.bm25_candidates as i64,
        fused_candidates: stats.fused_candidates as i64,
        injected_count: stats.injected_count as i64,
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

fn is_observability_enabled() -> bool {
    crate::config::load_memory_upgrade_config(&memory_upgrade_config_path()).observability_enabled
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

fn sanitize_write_observation(
    mut observation: MemoryWriteObservation,
) -> MemoryWriteObservation {
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

fn validate_retrieval_stats(stats: &RetrievalCandidateStats) -> std::result::Result<(), anyhow::Error> {
    if stats.injected_count > stats.fused_candidates {
        anyhow::bail!("retrieval injected_count cannot exceed fused_candidates");
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

fn build_memory_write_observation_from_counts(
    character_id: &str,
    source: &str,
    trigger: &str,
    extracted_count: i64,
    stored_count: i64,
    deduplicated_count: i64,
    invalidated_count: i64,
    duration_ms: i64,
) -> MemoryWriteObservation {
    MemoryWriteObservation {
        character_id: build_periodic_character_label(character_id),
        source: build_periodic_source_label(source),
        trigger: build_periodic_trigger_label(trigger),
        extracted_count,
        stored_count,
        deduplicated_count,
        invalidated_count,
        duration_ms,
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
    extracted_count: i64,
    stored_count: i64,
    deduplicated_count: i64,
    invalidated_count: i64,
    duration_ms: i64,
) -> std::result::Result<MemoryWriteObservation, anyhow::Error> {
    validate_write_result_counts(
        extracted_count,
        stored_count,
        deduplicated_count,
        invalidated_count,
    )?;
    let observation = build_memory_write_observation_from_counts(
        character_id,
        source,
        trigger,
        extracted_count,
        stored_count,
        deduplicated_count,
        invalidated_count,
        duration_ms,
    );
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
        0,
        0,
        0,
        0,
        duration_ms,
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
    Ok(Some(build_retrieval_observation_record(character_id, query, stats)?))
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

fn build_stats_for_current_results(
    semantic_results: &[MemorySnippet],
    bm25_results: &[(i64, f64)],
    fused_results: &[(f32, MemorySnippet)],
    injected_results: &[MemorySnippet],
) -> RetrievalCandidateStats {
    build_retrieval_stats_from_results(
        semantic_results,
        bm25_results,
        fused_results,
        injected_results,
    )
    .unwrap_or_default()
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

fn build_search_stats(
    semantic_results: &[MemorySnippet],
    bm25_results: &[(i64, f64)],
    fused_results: &[(f32, MemorySnippet)],
    injected_results: &[MemorySnippet],
) -> RetrievalCandidateStats {
    build_stats_for_current_results(
        semantic_results,
        bm25_results,
        fused_results,
        injected_results,
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

fn build_memory_observability_counts(
    manager: &MemoryManager,
) -> impl std::future::Future<Output = Result<MemoryObservabilitySummary>> + '_ {
    async move {
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
}

fn insert_memory_write_event(
    manager: &MemoryManager,
    observation: MemoryWriteObservation,
) -> impl std::future::Future<Output = Result<()>> + '_ {
    async move {
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
}

fn insert_memory_retrieval_log(
    manager: &MemoryManager,
    observation: MemoryRetrievalObservation,
) -> impl std::future::Future<Output = Result<()>> + '_ {
    async move {
        let record = build_memory_retrieval_record(&observation)?;
        sqlx::query(
            "INSERT INTO memory_retrieval_logs (character_id, query, semantic_candidates, bm25_candidates, fused_candidates, injected_count, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
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
}

fn fetch_latest_memory_write_event(
    manager: &MemoryManager,
) -> impl std::future::Future<Output = Result<Option<MemoryWriteEventRecord>>> + '_ {
    async move {
        let row = sqlx::query_as::<_, MemoryWriteEventRecord>(
            "SELECT source, trigger, extracted_count, stored_count, deduplicated_count, invalidated_count, duration_ms FROM memory_write_events ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&manager.db)
        .await?;
        Ok(row)
    }
}

fn fetch_latest_memory_retrieval_log(
    manager: &MemoryManager,
) -> impl std::future::Future<Output = Result<Option<MemoryRetrievalLogRecord>>> + '_ {
    async move {
        let row = sqlx::query_as::<_, MemoryRetrievalLogRecord>(
            "SELECT query, semantic_candidates, bm25_candidates, fused_candidates, injected_count FROM memory_retrieval_logs ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&manager.db)
        .await?;
        Ok(row)
    }
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
#[cfg(not(test))]
const MODEL_AUX_FILES: &[&str] = &[
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
    "vocab.txt",
];

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
        let embeddings = guard.embed(vec![text], None)?;
        Ok(embeddings[0].clone())
    }

    #[cfg(test)]
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(test_embedding(text))
    }

    pub async fn add_memory(&self, content: &str, character_id: &str) -> Result<()> {
        let embedding = self.embed(content).await?;
        let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
        let now = chrono::Utc::now().timestamp();

        // Deduplication: check if a very similar memory already exists
        if let Ok(true) = self
            .deduplicate_or_refresh(&embedding, character_id, now)
            .await
        {
            tracing::info!(
                target: "memory",
                "[Memory] Deduplicated: refreshed existing memory for '{}'",
                &content[..content.len().min(50)]
            );
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO memories (content, embedding, created_at, updated_at, importance, character_id, tier) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(now)
        .bind(now)
        .bind(0.5) // Default importance
        .bind(character_id)
        .bind("ephemeral")
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Return all memory content strings for a given character (used for dedup in extraction).
    pub async fn get_all_memory_contents(&self, character_id: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT content FROM memories WHERE character_id = ? ORDER BY importance DESC LIMIT 50",
        )
        .bind(character_id)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.iter().map(|r| r.get::<String, _>("content")).collect())
    }

    /// Check for duplicate memories. If a near-duplicate exists (similarity > threshold),
    /// refresh its timestamp instead of inserting a new row. Returns true if deduplicated.
    async fn deduplicate_or_refresh(
        &self,
        new_embedding: &[f32],
        character_id: &str,
        now: i64,
    ) -> Result<bool> {
        let rows = sqlx::query("SELECT id, embedding FROM memories WHERE character_id = ?")
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
                    "[Memory] Dedup: similarity={:.3} > {:.3}, refreshing id={}",
                    sim, DEDUP_THRESHOLD, id
                );
                // Refresh updated_at only — created_at must remain immutable
                sqlx::query("UPDATE memories SET updated_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(id)
                    .execute(&self.db)
                    .await?;
                return Ok(true);
            }
        }
        Ok(false)
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
            sqlx::query("SELECT id, embedding, importance FROM memories WHERE character_id = ?")
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
        fused.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let snippets: Vec<MemorySnippet> = fused
            .iter()
            .filter(|(score, _)| *score >= MIN_RRF_SCORE)
            .take(limit)
            .map(|(_, m)| m.clone())
            .collect();

        let stats = build_search_stats(&semantic_results, &bm25_results, &fused, &snippets);
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

    pub fn build_write_observation_for_test(
        character_id: &str,
        source: &str,
        trigger: &str,
        extracted_count: i64,
        stored_count: i64,
        deduplicated_count: i64,
        invalidated_count: i64,
        duration_ms: i64,
    ) -> Result<MemoryWriteObservation> {
        build_write_observation_record(
            character_id,
            source,
            trigger,
            extracted_count,
            stored_count,
            deduplicated_count,
            invalidated_count,
            duration_ms,
        )
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
            sqlx::query("SELECT id, content, embedding, created_at, importance, tier FROM memories WHERE character_id = ? AND tier != 'invalidated'")
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

        scored_memories.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

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
             WHERE memories_fts MATCH ? AND m.character_id = ? \
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
            "SELECT id, content, embedding, created_at, importance, tier FROM memories WHERE id = ?",
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
            ) {
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
        let embedding = self.embed(content).await?;
        let embedding_bytes: Vec<u8> = bincode::serialize(&embedding)?;
        let now = chrono::Utc::now().timestamp();

        // Deduplication check — also upgrades importance/tier if duplicate found
        if let Ok(true) = self
            .deduplicate_or_upgrade(&embedding, character_id, now, importance)
            .await
        {
            return Ok(());
        }

        let tier = if importance >= 0.8 {
            "core"
        } else {
            "ephemeral"
        };

        sqlx::query(
            "INSERT INTO memories (content, embedding, created_at, updated_at, importance, character_id, tier) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(now)
        .bind(now)
        .bind(importance.clamp(0.0, 1.0))
        .bind(character_id)
        .bind(tier)
        .execute(&self.db)
        .await?;

        // After inserting, check for contradiction with existing memories in the 0.70-0.95 band
        let _ = self
            .check_and_invalidate_contradictions(content, &embedding, character_id)
            .await;

        Ok(())
    }

    /// After storing a new memory, scan existing memories in the CONTRADICTION_BAND
    /// (similarity 0.70–0.95) and mark those that appear to contradict the new fact
    /// as 'invalidated' so they won't be retrieved in future searches.
    ///
    /// Uses a lightweight negation-keyword heuristic — no LLM call needed.
    pub async fn check_and_invalidate_contradictions(
        &self,
        new_content: &str,
        new_embedding: &[f32],
        character_id: &str,
    ) -> Result<usize> {
        let rows = sqlx::query(
            "SELECT id, content, embedding FROM memories \
             WHERE character_id = ? AND tier != 'invalidated' \
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
                    sqlx::query(
                        "UPDATE memories SET tier = 'invalidated', updated_at = ? WHERE id = ?",
                    )
                    .bind(chrono::Utc::now().timestamp())
                    .bind(id)
                    .execute(&self.db)
                    .await?;
                    invalidated += 1;
                    tracing::info!(
                        target: "memory",
                        "[Memory] Invalidated contradicting memory id={}: '{}'",
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
            "SELECT rowid AS rowid, content, created_at, importance, tier FROM memories WHERE character_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
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
                }
            })
            .collect())
    }

    /// Count total memories for a character.
    pub async fn count_memories(&self, character_id: &str) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM memories WHERE character_id = ?")
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

        sqlx::query(
            "UPDATE memories SET content = ?, embedding = ?, importance = ?, tier = ? WHERE rowid = ?",
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(clamped)
        .bind(tier)
        .bind(id)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Delete a memory by ID.
    pub async fn delete_memory(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM memories WHERE rowid = ?")
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    /// Update a memory's tier (e.g. "core" or "ephemeral").
    pub async fn update_memory_tier(&self, id: i64, tier: &str) -> Result<()> {
        sqlx::query("UPDATE memories SET tier = ? WHERE rowid = ?")
            .bind(tier)
            .bind(id)
            .execute(&self.db)
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
             WHERE character_id = ? AND tier = 'ephemeral' AND created_at < ?",
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
                sqlx::query("DELETE FROM memories WHERE rowid = ?")
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
                if sim > CONSOLIDATION_THRESHOLD && time_diff <= CONSOLIDATION_TIME_WINDOW_SECS {
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
            let merged = match merge_facts_via_llm(&facts, &provider).await {
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

/// Row type for paginated memory listing.
#[derive(sqlx::FromRow)]
struct MemoryRow {
    rowid: i64,
    content: String,
    created_at: i64,
    importance: f64,
    tier: String,
}

/// Public record type returned to frontend via Tauri commands.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryRecord {
    pub id: i64,
    pub content: String,
    pub created_at: i64,
    pub importance: f64,
    pub tier: String,
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
) -> Result<String> {
    use crate::llm::messages::user_text_message;

    let facts_list = facts
        .iter()
        .enumerate()
        .map(|(i, f)| format!("{}. {}", i + 1, f))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "You are a memory consolidation assistant. Merge the following related facts into a single, \
         concise, and complete memory entry. Preserve all important details. Do not add information \
         that is not present in the original facts. Output only the merged memory text, nothing else.\n\n\
         Facts:\n{}",
        facts_list
    );

    let messages = vec![user_text_message(prompt)];

    let result = provider
        .chat(messages, None)
        .await
        .map_err(|e| anyhow::anyhow!("LLM merge failed: {}", e))?;

    Ok(result.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_write_observation_for_event_trigger() {
        let observation = MemoryManager::build_write_observation_for_test(
            "char-1",
            "chat",
            "event_profile",
            1,
            1,
            0,
            0,
            10,
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
