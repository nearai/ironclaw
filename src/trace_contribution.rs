//! Privacy-preserving trace contribution envelopes.
//!
//! This module is intentionally separate from replay traces. Replay fixtures
//! capture enough behavior to drive tests; contribution envelopes capture the
//! consent, privacy, replayability, scoring, and revocation metadata needed
//! before a trace can leave a user's machine.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use regex::Regex;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::OwnedMutexGuard;
use uuid::Uuid;

use crate::llm::recording::{TraceFile, TraceResponse};
use crate::tools::redaction::redact_sensitive_json;

pub const TRACE_CONTRIBUTION_SCHEMA_VERSION: &str = "ironclaw.trace_contribution.v1";
pub const TRACE_CONTRIBUTION_POLICY_VERSION: &str = "2026-04-24";
pub const DETERMINISTIC_REDACTION_PIPELINE_VERSION: &str = "ironclaw-deterministic-secret-path-v1";
pub const PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX: &str = "privacy-filter-sidecar-v1";
pub const SERVER_RESCRUB_PIPELINE_SUFFIX: &str = "server-rescrub-v1";
pub const PRIVACY_FILTER_CANARY_VERSION: &str = "trace-privacy-filter-canary-v1";
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES: usize = 1024 * 1024;
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceContributionEnvelope {
    pub schema_version: String,
    pub trace_id: Uuid,
    pub submission_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub ironclaw: IronclawTraceMetadata,
    pub consent: ConsentMetadata,
    pub contributor: ContributorMetadata,
    pub privacy: PrivacyMetadata,
    pub events: Vec<TraceContributionEvent>,
    pub outcome: OutcomeMetadata,
    pub replay: ReplayMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_analysis: Option<EmbeddingAnalysisMetadata>,
    pub value: ValueMetadata,
    #[serde(default)]
    pub trace_card: TraceCard,
    #[serde(default)]
    pub value_card: TraceValueCard,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hindsight: Option<HindsightRelabelingCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_dynamics: Option<TrainingDynamicsSignals>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_evaluation: Option<ProcessEvaluationLabels>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IronclawTraceMetadata {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub feature_flags: BTreeMap<String, String>,
    pub channel: TraceChannel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceChannel {
    Web,
    Cli,
    Telegram,
    Slack,
    Routine,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsentMetadata {
    pub policy_version: String,
    pub scopes: Vec<ConsentScope>,
    pub message_text_included: bool,
    pub tool_payloads_included: bool,
    pub revocable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConsentScope {
    DebuggingEvaluation,
    BenchmarkOnly,
    RankingTraining,
    ModelTraining,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContributorMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pseudonymous_contributor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_scope_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_account_ref: Option<String>,
    pub revocation_handle: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyMetadata {
    pub redaction_pipeline_version: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub redaction_counts: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_filter_summary: Option<SafePrivacyFilterSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pii_labels_present: Vec<String>,
    pub residual_pii_risk: ResidualPiiRisk,
    pub redaction_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ResidualPiiRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceContributionEvent {
    pub event_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<Uuid>,
    pub event_type: TraceContributionEventType,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_content: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub structured_payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_counts: Option<TokenCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_modes: Vec<TraceFailureMode>,
    #[serde(default)]
    pub side_effect: SideEffectLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceContributionEventType {
    UserMessage,
    AssistantMessage,
    ToolCall,
    ToolResult,
    RoutingDecision,
    Feedback,
    HttpExchange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceFailureMode {
    ToolSelectionError,
    ToolArgumentError,
    ToolOrderingError,
    MissingVerification,
    PrematureTermination,
    LoopingOrRepetition,
    ContextLoss,
    PrivacyPolicyViolation,
    SecretExposureAttempt,
    UserIntentMisread,
    UnrecoverableToolFailure,
    BadMemoryRetrieval,
    BadRoutingDecision,
    UnsafeSideEffect,
    SpecificationAmbiguity,
    EnvironmentOrAuthFailure,
    Other(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectLevel {
    #[default]
    None,
    ReadOnly,
    LocalWrite,
    ExternalWrite,
    CredentialUse,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenCounts {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutcomeMetadata {
    pub user_feedback: UserFeedback,
    pub task_success: TaskSuccess,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub error_taxonomy: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_modes: Vec<TraceFailureMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_correction: Option<String>,
}

impl Default for OutcomeMetadata {
    fn default() -> Self {
        Self {
            user_feedback: UserFeedback::None,
            task_success: TaskSuccess::Unknown,
            error_taxonomy: Vec::new(),
            failure_modes: Vec::new(),
            human_correction: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserFeedback {
    ThumbsUp,
    ThumbsDown,
    Correction,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskSuccess {
    Success,
    Partial,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayMetadata {
    pub replayable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_manifest_hashes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_assertions: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replay_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingAnalysisMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub canonical_summary_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_vector_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nearest_trace_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest_cluster_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub novelty_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SafePrivacyFilterSummary {
    pub schema_version: u16,
    pub output_mode: String,
    pub span_count: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_label: BTreeMap<String, u32>,
    pub decoded_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SafePrivacyFilterRedaction {
    pub redacted_text: String,
    pub summary: SafePrivacyFilterSummary,
    pub report: RedactionReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyFilterSidecarRequest {
    pub text: String,
}

impl PrivacyFilterSidecarRequest {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyFilterCanaryReport {
    pub canary_version: String,
    pub healthy: bool,
    pub canary_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted_output_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SafePrivacyFilterSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceAllowedUse {
    Debugging,
    Evaluation,
    BenchmarkGeneration,
    RankingModelTraining,
    ModelTraining,
    AggregateAnalytics,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceRetentionClass {
    LocalQueue,
    PrivateCorpusRevocable,
    BenchmarkRevocable,
    TrainingRevocable,
    AggregateOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRetentionPolicy {
    pub name: String,
    pub class: TraceRetentionClass,
    pub revocable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    pub allows_derived_artifacts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivedArtifactInvalidationMarker {
    pub schema_version: String,
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub revocation_handle_hash: String,
    pub redaction_hash: String,
    pub artifact_prefixes: Vec<String>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceDatasetEligibility {
    pub eligible: bool,
    pub requested_use: TraceAllowedUse,
    pub retention_policy: TraceRetentionPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCard {
    pub consent_scope: ConsentScope,
    pub redaction_pipeline_version: String,
    pub source_channel: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_uses: Vec<TraceAllowedUse>,
    pub retention_policy: String,
    pub revocation_handle: String,
}

impl Default for TraceCard {
    fn default() -> Self {
        Self {
            consent_scope: ConsentScope::DebuggingEvaluation,
            redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
            source_channel: "unknown".to_string(),
            tool_categories: Vec::new(),
            allowed_uses: default_allowed_uses_for_scope(ConsentScope::DebuggingEvaluation),
            retention_policy: "private_corpus_revocable".to_string(),
            revocation_handle: Uuid::nil().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceValueCard {
    pub score_version: String,
    pub scorecard: TraceValueScorecard,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_visible_explanation: Vec<String>,
}

impl Default for TraceValueCard {
    fn default() -> Self {
        Self {
            score_version: "trace-value-scorecard-v1".to_string(),
            scorecard: TraceValueScorecard::default(),
            limitations: vec![
                "Initial score uses local heuristics only; delayed utility credit is assigned by downstream evaluation, benchmark, and training jobs.".to_string(),
            ],
            user_visible_explanation: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TraceValueScorecard {
    pub schema_validity: f32,
    pub privacy_risk: f32,
    pub quality: f32,
    pub replayability: f32,
    pub novelty: f32,
    pub duplicate_penalty: f32,
    pub coverage_bonus: f32,
    pub difficulty: f32,
    pub dependability: f32,
    pub user_correction_value: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_eval_value: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downstream_utility: Option<f32>,
    pub online_score: f32,
    pub credit_points_estimate: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TrainingDynamicsSignals {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variability: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correctness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cartography_bucket: Option<CartographyBucket>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CartographyBucket {
    Easy,
    Ambiguous,
    Hard,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProcessEvaluationLabels {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_name: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evaluator_version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<ProcessEvaluatorLabel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_selection: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_argument_quality: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_ordering: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effect_safety: Option<ProcessEvalRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_score: Option<f32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProcessEvalRating {
    Pass,
    Partial,
    Fail,
    NotApplicable,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProcessEvaluatorLabel {
    CorrectToolSelection,
    IncorrectToolSelection,
    CorrectToolArguments,
    IncorrectToolArguments,
    CorrectToolOrdering,
    ToolOrderingIssue,
    RetryLoop,
    MissingVerification,
    ProperVerification,
    SafeSideEffects,
    UnsafeSideEffectAttempt,
    UserCorrectionHandled,
    RecoverableFailure,
    BenchmarkCandidate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalRepresentationKind {
    WholeTrace,
    Turn,
    ToolSequence,
    ErrorOutcome,
    Correction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalTraceRepresentation {
    pub kind: CanonicalRepresentationKind,
    pub vector_key: String,
    pub canonical_hash: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HindsightRelabelingCandidate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_goal_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub achieved_subgoals: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_type: Option<TraceFailureMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoverability_score: Option<f32>,
    #[serde(default)]
    pub benchmark_candidate: bool,
    #[serde(default)]
    pub relabeled_training_candidate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TraceCreditEventKind {
    Accepted,
    RejectedPrivacy,
    RejectedDuplicate,
    CreditSynced,
    Replayable,
    NovelCluster,
    UnderrepresentedCoverage,
    UserCorrectionIncluded,
    ConvertedToBenchmark,
    CaughtRegression,
    UsedForTrainingOrRanking,
    ReviewerBonus,
    AbusePenalty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditEvent {
    pub event_id: Uuid,
    pub submission_id: Uuid,
    pub contributor_pseudonym: String,
    pub kind: TraceCreditEventKind,
    pub points_delta: f32,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValueMetadata {
    pub submission_score: f32,
    pub credit_points_pending: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

impl Default for ValueMetadata {
    fn default() -> Self {
        Self {
            submission_score: 0.0,
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanation: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandingTraceContributionPolicy {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_endpoint: Option<String>,
    pub bearer_token_env: String,
    pub include_message_text: bool,
    pub include_tool_payloads: bool,
    pub auto_submit_failed_traces: bool,
    pub auto_submit_high_value_traces: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub selected_tools: BTreeSet<String>,
    pub require_manual_approval_when_pii_detected: bool,
    pub min_submission_score: f32,
    pub credit_notice_interval_hours: u32,
    pub default_scope: ConsentScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceContributionAcceptance {
    PreviewOnly,
    QueueFromPreview,
    ManualSubmit,
    AutonomousSubmit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceContributionPolicyRejection {
    OptInDisabled,
    EndpointMissing,
}

impl std::fmt::Display for TraceContributionPolicyRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OptInDisabled => write!(f, "trace contribution opt-in is disabled"),
            Self::EndpointMissing => write!(f, "trace contribution endpoint is not configured"),
        }
    }
}

impl std::error::Error for TraceContributionPolicyRejection {}

pub fn preflight_trace_contribution_policy(
    policy: &StandingTraceContributionPolicy,
    intent: TraceContributionAcceptance,
) -> Result<(), TraceContributionPolicyRejection> {
    if intent == TraceContributionAcceptance::PreviewOnly {
        return Ok(());
    }
    if !policy.enabled {
        return Err(TraceContributionPolicyRejection::OptInDisabled);
    }
    if policy.ingestion_endpoint.is_none() {
        return Err(TraceContributionPolicyRejection::EndpointMissing);
    }
    Ok(())
}

impl Default for StandingTraceContributionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            ingestion_endpoint: None,
            bearer_token_env: "IRONCLAW_TRACE_SUBMIT_TOKEN".to_string(),
            include_message_text: false,
            include_tool_payloads: false,
            auto_submit_failed_traces: true,
            auto_submit_high_value_traces: true,
            selected_tools: BTreeSet::new(),
            require_manual_approval_when_pii_detected: true,
            min_submission_score: 0.35,
            credit_notice_interval_hours: 168,
            default_scope: ConsentScope::DebuggingEvaluation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreditEstimate {
    pub submission_score: f32,
    pub credit_points_pending: f32,
    pub scorecard: TraceValueScorecard,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreditSummary {
    pub submissions_total: u32,
    pub submissions_submitted: u32,
    pub submissions_revoked: u32,
    #[serde(default)]
    pub submissions_expired: u32,
    pub pending_credit: f32,
    pub final_credit: f32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub delayed_credit_delta: f32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub credit_events_total: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_explanations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditReport {
    pub submissions_total: u32,
    pub submissions_submitted: u32,
    pub submissions_revoked: u32,
    #[serde(default)]
    pub submissions_expired: u32,
    #[serde(default)]
    pub submissions_accepted: u32,
    #[serde(default)]
    pub submissions_quarantined: u32,
    #[serde(default)]
    pub submissions_rejected: u32,
    pub pending_credit: f32,
    pub final_credit: f32,
    #[serde(default)]
    pub credit_events_total: u32,
    #[serde(default)]
    pub delayed_credit_delta: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submission_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation_lines: Vec<String>,
}

pub fn estimate_initial_credit(envelope: &TraceContributionEnvelope) -> CreditEstimate {
    let scorecard = compute_value_scorecard(envelope);
    let submission_score = scorecard.online_score;
    let credit_points_pending = scorecard.credit_points_estimate;
    let explanation = scorecard.explanation.clone();

    CreditEstimate {
        submission_score,
        credit_points_pending,
        scorecard,
        explanation,
    }
}

pub fn compute_value_scorecard(envelope: &TraceContributionEnvelope) -> TraceValueScorecard {
    let schema_validity = if envelope.schema_version == TRACE_CONTRIBUTION_SCHEMA_VERSION {
        1.0
    } else {
        0.0
    };
    let privacy_risk = privacy_risk_score(envelope.privacy.residual_pii_risk);
    let gate = privacy_gate(envelope.privacy.residual_pii_risk);
    let event_count = envelope.events.len() as f32;
    let quality = (event_count / 8.0).clamp(0.15, 1.0);
    let replayability = if envelope.replay.replayable { 1.0 } else { 0.0 };
    let novelty = envelope
        .embedding_analysis
        .as_ref()
        .and_then(|analysis| analysis.novelty_score)
        .unwrap_or_else(|| (event_count / 12.0).clamp(0.15, 0.6))
        .min(0.85);
    let duplicate_penalty = envelope
        .embedding_analysis
        .as_ref()
        .and_then(|analysis| analysis.duplicate_score)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let coverage_bonus = (envelope.replay.required_tools.len() as f32 / 5.0).clamp(0.0, 1.0);
    let failed_or_partial = matches!(
        envelope.outcome.task_success,
        TaskSuccess::Failure | TaskSuccess::Partial
    );
    let difficulty = if failed_or_partial { 0.65 } else { 0.35 };
    let dependability = if envelope.events.is_empty() {
        0.0
    } else if envelope.privacy.redaction_hash.starts_with("sha256:") {
        1.0
    } else {
        0.5
    };
    let user_correction_value = if envelope.outcome.human_correction.is_some()
        || envelope.outcome.user_feedback == UserFeedback::Correction
    {
        1.0
    } else {
        0.0
    };
    let process_eval_value = envelope
        .process_evaluation
        .as_ref()
        .and_then(|labels| labels.overall_score)
        .map(|score| score.clamp(0.0, 1.0));

    let raw = gate
        * schema_validity
        * (0.25 * quality
            + 0.20 * replayability
            + 0.20 * novelty
            + 0.15 * coverage_bonus
            + 0.10 * difficulty
            + 0.10 * user_correction_value)
        - 0.40 * duplicate_penalty
        - 0.60 * privacy_risk;
    let online_score = raw.clamp(0.0, 1.0);
    let credit_points_estimate =
        if matches!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::High) {
            0.0
        } else {
            (10.0 * online_score * 100.0).round() / 100.0
        };

    let mut explanation = Vec::new();
    if gate > 0.0 {
        explanation.push(format!(
            "Privacy gate passed with {:?} residual risk.",
            envelope.privacy.residual_pii_risk
        ));
    } else {
        explanation.push("Residual privacy risk is high; credit is held for review.".to_string());
    }
    if envelope.replay.replayable {
        explanation.push("Replay metadata is present.".to_string());
    }
    if !envelope.replay.required_tools.is_empty() {
        explanation.push(format!(
            "Covers {} tool(s).",
            envelope.replay.required_tools.len()
        ));
    }
    if user_correction_value > 0.0 {
        explanation.push("Includes a redacted user correction signal.".to_string());
    }
    if duplicate_penalty > 0.0 {
        explanation.push(format!(
            "Duplicate penalty applied at {:.2}.",
            duplicate_penalty
        ));
    }
    if !envelope.privacy.redaction_counts.is_empty() {
        explanation.push("Deterministic redactions were applied before submission.".to_string());
    }

    TraceValueScorecard {
        schema_validity,
        privacy_risk,
        quality,
        replayability,
        novelty,
        duplicate_penalty,
        coverage_bonus,
        difficulty,
        dependability,
        user_correction_value,
        process_eval_value,
        downstream_utility: None,
        online_score,
        credit_points_estimate,
        explanation,
    }
}

fn privacy_gate(risk: ResidualPiiRisk) -> f32 {
    match risk {
        ResidualPiiRisk::Low => 1.0,
        ResidualPiiRisk::Medium => 0.5,
        ResidualPiiRisk::High => 0.0,
    }
}

fn privacy_risk_score(risk: ResidualPiiRisk) -> f32 {
    match risk {
        ResidualPiiRisk::Low => 0.0,
        ResidualPiiRisk::Medium => 0.5,
        ResidualPiiRisk::High => 1.0,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceContribution {
    pub trace_id: Uuid,
    pub submission_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub ironclaw: IronclawTraceMetadata,
    pub consent: ConsentMetadata,
    pub contributor: ContributorMetadata,
    pub events: Vec<RawTraceContributionEvent>,
    pub outcome: OutcomeMetadata,
    pub replay: ReplayMetadata,
    pub embedding_analysis: Option<EmbeddingAnalysisMetadata>,
    pub value: ValueMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceContributionEvent {
    pub event_id: Uuid,
    pub event_type: TraceContributionEventType,
    pub timestamp: DateTime<Utc>,
    pub content: Option<String>,
    pub structured_payload: Value,
    pub tool_name: Option<String>,
    pub latency_ms: Option<u64>,
    pub token_counts: Option<TokenCounts>,
    pub cost_usd: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceCaptureTurn {
    pub user_input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<RawTraceCaptureToolCall>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawTraceCaptureToolCall {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

pub fn capture_turns_from_conversation_messages(
    messages: &[crate::history::ConversationMessage],
) -> Vec<RawTraceCaptureTurn> {
    let mut turns = Vec::new();
    let mut iter = messages.iter().peekable();

    while let Some(message) = iter.next() {
        if message.role == "user" {
            let mut turn = RawTraceCaptureTurn {
                user_input: message.content.clone(),
                response: None,
                tool_calls: Vec::new(),
                started_at: message.created_at,
                completed_at: None,
                state: Some("Completed".to_string()),
            };

            if let Some(next) = iter.peek()
                && next.role == "tool_calls"
                && let Some(tool_message) = iter.next()
            {
                turn.tool_calls = parse_capture_tool_calls(&tool_message.content);
            }

            if let Some(next) = iter.peek()
                && next.role == "assistant"
                && let Some(assistant_message) = iter.next()
            {
                turn.response = Some(assistant_message.content.clone());
                turn.completed_at = Some(assistant_message.created_at);
            }

            if turn.response.is_none() {
                turn.state = Some("Failed".to_string());
            }
            turns.push(turn);
        } else if message.role == "assistant" {
            turns.push(RawTraceCaptureTurn {
                user_input: String::new(),
                response: Some(message.content.clone()),
                tool_calls: Vec::new(),
                started_at: message.created_at,
                completed_at: Some(message.created_at),
                state: Some("Completed".to_string()),
            });
        }
    }

    turns
}

fn parse_capture_tool_calls(content: &str) -> Vec<RawTraceCaptureToolCall> {
    let value = match serde_json::from_str::<Value>(content) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let calls = match value {
        Value::Array(calls) => calls,
        Value::Object(mut obj) => match obj.remove("calls") {
            Some(Value::Array(calls)) => calls,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };

    calls
        .into_iter()
        .map(|call| RawTraceCaptureToolCall {
            name: call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            result_preview: call
                .get("result_preview")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            error: call
                .get("error")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            rationale: call
                .get("rationale")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct RecordedTraceContributionOptions {
    pub include_message_text: bool,
    pub include_tool_payloads: bool,
    pub consent_scopes: Vec<ConsentScope>,
    pub channel: TraceChannel,
    pub engine_version: Option<String>,
    pub feature_flags: BTreeMap<String, String>,
    pub pseudonymous_contributor_id: Option<String>,
    pub tenant_scope_ref: Option<String>,
    pub credit_account_ref: Option<String>,
}

impl Default for RecordedTraceContributionOptions {
    fn default() -> Self {
        Self {
            include_message_text: false,
            include_tool_payloads: false,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            channel: TraceChannel::Cli,
            engine_version: None,
            feature_flags: BTreeMap::new(),
            pseudonymous_contributor_id: None,
            tenant_scope_ref: None,
            credit_account_ref: None,
        }
    }
}

impl RawTraceContribution {
    pub fn from_recorded_trace(
        trace: &TraceFile,
        options: RecordedTraceContributionOptions,
    ) -> Self {
        let created_at = Utc::now();
        let mut events = Vec::new();
        let mut required_tools = BTreeSet::new();

        for step in &trace.steps {
            match &step.response {
                TraceResponse::UserInput { content } => {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        event_type: TraceContributionEventType::UserMessage,
                        timestamp: created_at,
                        content: options.include_message_text.then(|| content.clone()),
                        structured_payload: Value::Null,
                        tool_name: None,
                        latency_ms: None,
                        token_counts: None,
                        cost_usd: None,
                    });
                }
                TraceResponse::Text {
                    content,
                    input_tokens,
                    output_tokens,
                } => {
                    events.push(RawTraceContributionEvent {
                        event_id: Uuid::new_v4(),
                        event_type: TraceContributionEventType::AssistantMessage,
                        timestamp: created_at,
                        content: options.include_message_text.then(|| content.clone()),
                        structured_payload: Value::Null,
                        tool_name: None,
                        latency_ms: None,
                        token_counts: Some(TokenCounts {
                            input_tokens: *input_tokens,
                            output_tokens: *output_tokens,
                        }),
                        cost_usd: None,
                    });
                }
                TraceResponse::ToolCalls {
                    tool_calls,
                    input_tokens,
                    output_tokens,
                } => {
                    for tool_call in tool_calls {
                        required_tools.insert(tool_call.name.clone());
                        let structured_payload = if options.include_tool_payloads {
                            serde_json::json!({
                                "tool_call_id": tool_call.id,
                                "arguments": tool_call.arguments,
                            })
                        } else {
                            serde_json::json!({
                                "tool_call_id": tool_call.id,
                            })
                        };

                        events.push(RawTraceContributionEvent {
                            event_id: Uuid::new_v4(),
                            event_type: TraceContributionEventType::ToolCall,
                            timestamp: created_at,
                            content: None,
                            structured_payload,
                            tool_name: Some(tool_call.name.clone()),
                            latency_ms: None,
                            token_counts: Some(TokenCounts {
                                input_tokens: *input_tokens,
                                output_tokens: *output_tokens,
                            }),
                            cost_usd: None,
                        });
                    }
                }
            }

            for expected in &step.expected_tool_results {
                required_tools.insert(expected.name.clone());
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::ToolResult,
                    timestamp: created_at,
                    content: options
                        .include_tool_payloads
                        .then(|| expected.content.clone()),
                    structured_payload: serde_json::json!({
                        "tool_call_id": expected.tool_call_id,
                    }),
                    tool_name: Some(expected.name.clone()),
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }
        }

        for exchange in &trace.http_exchanges {
            let structured_payload = if options.include_tool_payloads {
                serde_json::json!({
                    "request": {
                        "method": exchange.request.method,
                        "url": exchange.request.url,
                        "headers": exchange.request.headers,
                        "body": exchange.request.body,
                    },
                    "response": {
                        "status": exchange.response.status,
                        "headers": exchange.response.headers,
                    },
                })
            } else {
                serde_json::json!({
                    "request": {
                        "method": exchange.request.method,
                    },
                    "response": {
                        "status": exchange.response.status,
                    },
                })
            };

            events.push(RawTraceContributionEvent {
                event_id: Uuid::new_v4(),
                event_type: TraceContributionEventType::HttpExchange,
                timestamp: created_at,
                content: options
                    .include_tool_payloads
                    .then(|| exchange.response.body.clone()),
                structured_payload,
                tool_name: Some("http".to_string()),
                latency_ms: None,
                token_counts: None,
                cost_usd: None,
            });
        }

        let required_tools: Vec<String> = required_tools.into_iter().collect();

        Self {
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at,
            ironclaw: IronclawTraceMetadata {
                version: env!("CARGO_PKG_VERSION").to_string(),
                engine_version: options.engine_version,
                feature_flags: options.feature_flags,
                channel: options.channel,
                model_name: Some(trace.model_name.clone()),
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: options.consent_scopes,
                message_text_included: options.include_message_text,
                tool_payloads_included: options.include_tool_payloads,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: options.pseudonymous_contributor_id,
                tenant_scope_ref: options.tenant_scope_ref,
                credit_account_ref: options.credit_account_ref,
                revocation_handle: Uuid::new_v4(),
            },
            events,
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: !trace.steps.is_empty(),
                required_tools,
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
        }
    }

    pub fn from_capture_turns(
        turns: &[RawTraceCaptureTurn],
        options: RecordedTraceContributionOptions,
    ) -> Self {
        let created_at = Utc::now();
        let mut events = Vec::new();
        let mut required_tools = BTreeSet::new();
        let mut task_success = TaskSuccess::Unknown;

        for turn in turns {
            if !turn.user_input.is_empty() {
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::UserMessage,
                    timestamp: turn.started_at,
                    content: options
                        .include_message_text
                        .then(|| turn.user_input.clone()),
                    structured_payload: serde_json::json!({
                        "state": turn.state,
                    }),
                    tool_name: None,
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }

            for tool_call in &turn.tool_calls {
                required_tools.insert(tool_call.name.clone());
                let structured_payload = if options.include_tool_payloads {
                    serde_json::json!({
                        "result_preview": tool_call.result_preview,
                        "error": tool_call.error,
                        "rationale": tool_call.rationale,
                    })
                } else {
                    serde_json::json!({
                        "has_result": tool_call.result_preview.is_some(),
                        "has_error": tool_call.error.is_some(),
                    })
                };
                let content = options
                    .include_tool_payloads
                    .then(|| {
                        tool_call
                            .result_preview
                            .as_deref()
                            .or(tool_call.error.as_deref())
                            .unwrap_or("")
                            .to_string()
                    })
                    .filter(|content| !content.is_empty());

                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::ToolCall,
                    timestamp: turn.completed_at.unwrap_or(turn.started_at),
                    content,
                    structured_payload,
                    tool_name: Some(tool_call.name.clone()),
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }

            if let Some(response) = &turn.response {
                events.push(RawTraceContributionEvent {
                    event_id: Uuid::new_v4(),
                    event_type: TraceContributionEventType::AssistantMessage,
                    timestamp: turn.completed_at.unwrap_or(turn.started_at),
                    content: options.include_message_text.then(|| response.clone()),
                    structured_payload: Value::Null,
                    tool_name: None,
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                });
            }

            if matches!(turn.state.as_deref(), Some("Failed" | "failed")) {
                task_success = TaskSuccess::Failure;
            } else if task_success == TaskSuccess::Unknown && turn.response.is_some() {
                task_success = TaskSuccess::Success;
            }
        }

        let required_tools: Vec<String> = required_tools.into_iter().collect();

        Self {
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at,
            ironclaw: IronclawTraceMetadata {
                version: env!("CARGO_PKG_VERSION").to_string(),
                engine_version: options.engine_version,
                feature_flags: options.feature_flags,
                channel: options.channel,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: options.consent_scopes,
                message_text_included: options.include_message_text,
                tool_payloads_included: options.include_tool_payloads,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: options.pseudonymous_contributor_id,
                tenant_scope_ref: options.tenant_scope_ref,
                credit_account_ref: options.credit_account_ref,
                revocation_handle: Uuid::new_v4(),
            },
            events,
            outcome: OutcomeMetadata {
                task_success,
                ..OutcomeMetadata::default()
            },
            replay: ReplayMetadata {
                replayable: !turns.is_empty(),
                required_tools,
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: vec![
                    "Captured from web conversation history; exact tool arguments may be omitted by consent policy.".to_string(),
                ],
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactionReport {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pii_labels_present: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub blocked_secret_detected: bool,
}

impl RedactionReport {
    fn increment(&mut self, label: impl Into<String>) {
        *self.counts.entry(label.into()).or_insert(0) += 1;
    }

    fn add_pii_label(&mut self, label: impl Into<String>) {
        let label = label.into();
        if !self.pii_labels_present.contains(&label) {
            self.pii_labels_present.push(label);
        }
    }

    fn add_warning(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn merge(&mut self, other: RedactionReport) {
        for (key, value) in other.counts {
            *self.counts.entry(key).or_insert(0) += value;
        }
        for label in other.pii_labels_present {
            if !self.pii_labels_present.contains(&label) {
                self.pii_labels_present.push(label);
            }
        }
        for warning in other.warnings {
            self.add_warning(warning);
        }
        self.blocked_secret_detected |= other.blocked_secret_detected;
    }
}

pub fn safe_privacy_filter_redaction_from_output(
    output: &Value,
) -> Result<SafePrivacyFilterRedaction, TraceContributionError> {
    let redacted_text = output
        .get("redacted_text")
        .and_then(Value::as_str)
        .ok_or_else(|| TraceContributionError::RedactionFailed {
            reason: "privacy filter output is missing redacted_text".to_string(),
        })?
        .to_string();
    let detected_spans = output
        .get("detected_spans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut by_label = BTreeMap::new();
    let mut report = RedactionReport::default();
    for span in &detected_spans {
        let raw_label = span
            .get("label")
            .or_else(|| span.get("type"))
            .or_else(|| span.get("entity_type"))
            .and_then(Value::as_str);
        let label = safe_privacy_filter_label(raw_label, &mut report);
        *by_label.entry(label.clone()).or_insert(0) += 1;
        report.increment(format!("privacy_filter:{label}"));
        if label.eq_ignore_ascii_case("secret") {
            report.blocked_secret_detected = true;
        }
        if !report.pii_labels_present.contains(&label) {
            report.pii_labels_present.push(label);
        }
    }

    let schema_version = output
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(1);
    let decoded_mismatch = output
        .get("decoded_mismatch")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(SafePrivacyFilterRedaction {
        redacted_text,
        summary: SafePrivacyFilterSummary {
            schema_version,
            output_mode: "redacted_text_only".to_string(),
            span_count: detected_spans.len() as u32,
            by_label,
            decoded_mismatch,
        },
        report,
    })
}

fn safe_privacy_filter_label(raw_label: Option<&str>, report: &mut RedactionReport) -> String {
    let Some(raw_label) = raw_label else {
        return "unknown".to_string();
    };
    let normalized = raw_label
        .trim()
        .chars()
        .map(|character| {
            if character == '-' {
                '_'
            } else {
                character.to_ascii_lowercase()
            }
        })
        .collect::<String>();
    let allowed = matches!(
        normalized.as_str(),
        "account_number"
            | "credit_card"
            | "ip_address"
            | "private_address"
            | "private_date"
            | "private_email"
            | "private_location"
            | "private_name"
            | "private_person"
            | "private_phone"
            | "private_url"
            | "secret"
            | "secret_like"
            | "ssn"
    );
    if allowed {
        return normalized;
    }

    report.add_warning("Privacy Filter sidecar emitted unsupported span label; mapped to unknown.");
    "unknown".to_string()
}

pub fn synthetic_privacy_filter_canary_text() -> String {
    synthetic_privacy_filter_canary_values().join(" ")
}

pub fn synthetic_privacy_filter_canary_values() -> Vec<String> {
    vec![
        "trace-canary.person@example.invalid".to_string(),
        "tc_canary_secret_0123456789abcdef".to_string(),
        "/tmp/trace_canary_private/path.txt".to_string(),
    ]
}

pub async fn run_configured_privacy_filter_canary_from_env()
-> Result<Option<PrivacyFilterCanaryReport>, TraceContributionError> {
    let Some(adapter) = privacy_filter_adapter_from_env() else {
        return Ok(None);
    };
    run_privacy_filter_canary(adapter.as_ref()).await.map(Some)
}

pub async fn run_privacy_filter_canary(
    adapter: &dyn PrivacyFilterAdapter,
) -> Result<PrivacyFilterCanaryReport, TraceContributionError> {
    let raw_values = synthetic_privacy_filter_canary_values();
    let canary_text = raw_values.join(" ");
    let canary_hash = canonical_hash(&canary_text);
    let redaction = adapter.redact_text(&canary_text).await?;

    let Some(redaction) = redaction else {
        return Ok(PrivacyFilterCanaryReport {
            canary_version: PRIVACY_FILTER_CANARY_VERSION.to_string(),
            healthy: false,
            canary_hash,
            redacted_output_hash: None,
            summary: None,
            failures: vec!["privacy filter returned no redaction for synthetic canary".to_string()],
        });
    };

    let mut failures = Vec::new();
    let summary_json = serde_json::to_string(&redaction.summary).unwrap_or_default();
    let report_json = serde_json::to_string(&redaction.report).unwrap_or_default();
    for raw_value in &raw_values {
        if redaction.redacted_text.contains(raw_value) {
            failures.push(format!(
                "privacy filter redacted_text retained canary value hash {}",
                canonical_hash(raw_value)
            ));
        }
        if summary_json.contains(raw_value) || report_json.contains(raw_value) {
            failures.push(format!(
                "privacy filter safe summary retained canary value hash {}",
                canonical_hash(raw_value)
            ));
        }
    }

    Ok(PrivacyFilterCanaryReport {
        canary_version: PRIVACY_FILTER_CANARY_VERSION.to_string(),
        healthy: failures.is_empty(),
        canary_hash,
        redacted_output_hash: Some(canonical_hash(&redaction.redacted_text)),
        summary: Some(redaction.summary),
        failures,
    })
}

fn merge_privacy_filter_summary(
    target: &mut Option<SafePrivacyFilterSummary>,
    next: &SafePrivacyFilterSummary,
) {
    let target = target.get_or_insert_with(|| SafePrivacyFilterSummary {
        schema_version: next.schema_version,
        output_mode: "redacted_text_only".to_string(),
        span_count: 0,
        by_label: BTreeMap::new(),
        decoded_mismatch: false,
    });
    target.schema_version = target.schema_version.max(next.schema_version);
    target.span_count = target.span_count.saturating_add(next.span_count);
    target.decoded_mismatch |= next.decoded_mismatch;
    for (label, count) in &next.by_label {
        *target.by_label.entry(label.clone()).or_insert(0) += count;
    }
}

fn redaction_pipeline_version(privacy_filter_used: bool) -> String {
    if privacy_filter_used {
        format!(
            "{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+{PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX}"
        )
    } else {
        DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string()
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum TraceContributionError {
    #[error("trace contribution redaction failed: {reason}")]
    RedactionFailed { reason: String },
}

#[async_trait]
pub trait TraceRedactor: Send + Sync {
    async fn redact_trace(
        &self,
        trace: RawTraceContribution,
    ) -> Result<TraceContributionEnvelope, TraceContributionError>;
}

#[async_trait]
pub trait PrivacyFilterAdapter: Send + Sync {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError>;
}

pub struct NoopPrivacyFilterAdapter;

#[async_trait]
impl PrivacyFilterAdapter for NoopPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        _text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct CommandPrivacyFilterAdapter {
    command: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    max_input_bytes: usize,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
}

impl CommandPrivacyFilterAdapter {
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            timeout: Duration::from_secs(10),
            max_input_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
            max_stdout_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES,
            max_stderr_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES,
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_input_limit(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    pub fn with_output_limits(mut self, max_stdout_bytes: usize, max_stderr_bytes: usize) -> Self {
        self.max_stdout_bytes = max_stdout_bytes;
        self.max_stderr_bytes = max_stderr_bytes;
        self
    }
}

#[async_trait]
impl PrivacyFilterAdapter for CommandPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        if text.len() > self.max_input_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar input exceeded limit: input_len={} max_input_bytes={}",
                    text.len(),
                    self.max_input_bytes
                ),
            });
        }

        let mut command = tokio::process::Command::new(&self.command);
        command.env_clear();
        for name in ["PATH", "LANG", "LC_ALL"] {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child =
            command
                .spawn()
                .map_err(|error| TraceContributionError::RedactionFailed {
                    reason: format!(
                        "failed to spawn privacy filter sidecar {}: {}",
                        self.command.display(),
                        error
                    ),
                })?;

        let mut stdin =
            child
                .stdin
                .take()
                .ok_or_else(|| TraceContributionError::RedactionFailed {
                    reason: "privacy filter sidecar stdin was not available".to_string(),
                })?;
        let request = PrivacyFilterSidecarRequest::new(text);
        let request_body = serde_json::to_vec(&request).map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to serialize privacy filter request: {error}"),
            }
        })?;
        stdin.write_all(&request_body).await.map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to write privacy filter request: {error}"),
            }
        })?;
        drop(stdin);

        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar timed out after {}ms",
                    self.timeout.as_millis()
                ),
            })?
            .map_err(|error| TraceContributionError::RedactionFailed {
                reason: format!("privacy filter sidecar failed: {error}"),
            })?;

        if output.stdout.len() > self.max_stdout_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "stdout exceeded privacy filter sidecar limit: stdout_len={} max_stdout_bytes={}",
                    output.stdout.len(),
                    self.max_stdout_bytes
                ),
            });
        }
        if output.stderr.len() > self.max_stderr_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "stderr exceeded privacy filter sidecar limit: stderr_len={} stderr_hash={} max_stderr_bytes={}",
                    output.stderr.len(),
                    privacy_filter_bytes_hash(&output.stderr),
                    self.max_stderr_bytes
                ),
            });
        }

        if !output.status.success() {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "privacy filter sidecar exited with {}; stderr_len={} stderr_hash={}",
                    output.status,
                    output.stderr.len(),
                    privacy_filter_bytes_hash(&output.stderr)
                ),
            });
        }

        let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            TraceContributionError::RedactionFailed {
                reason: format!("failed to parse privacy filter output: {error}"),
            }
        })?;
        safe_privacy_filter_redaction_from_output(&value).map(Some)
    }
}

fn privacy_filter_bytes_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

pub fn privacy_filter_adapter_from_env() -> Option<Arc<dyn PrivacyFilterAdapter>> {
    let command = std::env::var("IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND").ok()?;
    let command = command.trim();
    if command.is_empty() {
        return None;
    }

    let args = std::env::var("IRONCLAW_TRACE_PRIVACY_FILTER_ARGS")
        .ok()
        .map(|raw| {
            raw.split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut adapter = CommandPrivacyFilterAdapter::new(command).with_args(args);
    if let Some(timeout_ms) = parse_usize_env("IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS") {
        adapter = adapter.with_timeout(Duration::from_millis(timeout_ms as u64));
    }
    if let Some(max_input_bytes) = parse_usize_env("IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES")
    {
        adapter = adapter.with_input_limit(max_input_bytes);
    }
    let max_stdout = parse_usize_env("IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES");
    let max_stderr = parse_usize_env("IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES");
    if max_stdout.is_some() || max_stderr.is_some() {
        adapter = adapter.with_output_limits(
            max_stdout.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES),
            max_stderr.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES),
        );
    }
    Some(Arc::new(adapter))
}

fn parse_usize_env(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

pub struct DeterministicTraceRedactor {
    leak_detector: ironclaw_safety::LeakDetector,
    known_path_prefixes: Vec<String>,
    privacy_filter: Option<Arc<dyn PrivacyFilterAdapter>>,
}

impl Default for DeterministicTraceRedactor {
    fn default() -> Self {
        let mut known_path_prefixes = Vec::new();
        if let Some(home) = dirs::home_dir() {
            known_path_prefixes.push(path_to_string(home));
        }
        if let Ok(current_dir) = std::env::current_dir() {
            known_path_prefixes.push(path_to_string(current_dir));
        }
        Self::new(known_path_prefixes)
    }
}

impl DeterministicTraceRedactor {
    pub fn new(known_path_prefixes: Vec<String>) -> Self {
        let mut known_path_prefixes: Vec<String> = known_path_prefixes
            .into_iter()
            .filter(|prefix| !prefix.trim().is_empty())
            .collect();
        known_path_prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        known_path_prefixes.dedup();

        Self {
            leak_detector: ironclaw_safety::LeakDetector::new(),
            known_path_prefixes,
            privacy_filter: privacy_filter_adapter_from_env(),
        }
    }

    pub fn with_privacy_filter(mut self, adapter: Arc<dyn PrivacyFilterAdapter>) -> Self {
        self.privacy_filter = Some(adapter);
        self
    }

    async fn apply_privacy_filter_to_text(
        &self,
        text: String,
        report: &mut RedactionReport,
        privacy_filter_summary: &mut Option<SafePrivacyFilterSummary>,
    ) -> Result<String, TraceContributionError> {
        let Some(adapter) = self.privacy_filter.as_ref() else {
            return Ok(text);
        };
        let redaction = match adapter.redact_text(&text).await {
            Ok(Some(redaction)) => redaction,
            Ok(None) => return Ok(text),
            Err(error) => {
                let error_text = error.to_string();
                report.increment("privacy_filter:sidecar_failure");
                report.add_warning(format!(
                    "Privacy Filter sidecar failed; deterministic redaction fallback was used. error_hash={}",
                    canonical_hash(&error_text)
                ));
                return Ok(text);
            }
        };

        merge_privacy_filter_summary(privacy_filter_summary, &redaction.summary);
        report.merge(redaction.report);
        Ok(redaction.redacted_text)
    }

    pub fn with_known_path_prefixes(prefixes: impl IntoIterator<Item = PathBuf>) -> Self {
        Self::new(prefixes.into_iter().map(path_to_string).collect())
    }

    pub fn redact_text(&self, input: &str) -> (String, RedactionReport) {
        let mut state = RedactionState::default();
        self.redact_text_with_state(input, &mut state)
    }

    fn redact_text_with_state(
        &self,
        input: &str,
        state: &mut RedactionState,
    ) -> (String, RedactionReport) {
        let mut report = RedactionReport::default();
        let mut redacted = self.redact_private_emails(input, state, &mut report);
        redacted = self.redact_generic_paths(&redacted, state, &mut report);
        redacted = self.redact_known_paths(&redacted, state, &mut report);

        let scan = self.leak_detector.scan(&redacted);
        if scan.is_clean() {
            return (redacted, report);
        }

        let ranges = scan
            .matches
            .iter()
            .map(|m| {
                report.increment("secret");
                report.increment(format!("secret:{}", m.pattern_name));
                if matches!(
                    m.severity,
                    ironclaw_safety::LeakSeverity::High | ironclaw_safety::LeakSeverity::Critical
                ) {
                    report.blocked_secret_detected = true;
                }
                m.location.clone()
            })
            .collect::<Vec<_>>();

        (apply_redaction_ranges(&redacted, &ranges), report)
    }

    fn redact_json_value(
        &self,
        tool_name: Option<&str>,
        value: &Value,
        state: &mut RedactionState,
    ) -> (Value, RedactionReport) {
        let mut report = RedactionReport::default();
        let tool_redacted = redact_tool_specific_payload(tool_name, value, &mut report);
        let keyed_redaction = redact_sensitive_json(&tool_redacted);
        count_sensitive_field_redactions(&tool_redacted, &keyed_redaction, &mut report);
        let redacted = self.redact_json_strings(keyed_redaction, state, &mut report);
        (redacted, report)
    }

    fn redact_json_strings(
        &self,
        value: Value,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> Value {
        match value {
            Value::String(s) => {
                let (redacted, child_report) = self.redact_text_with_state(&s, state);
                report.merge(child_report);
                Value::String(redacted)
            }
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| self.redact_json_strings(item, state, report))
                    .collect(),
            ),
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| (key, self.redact_json_strings(value, state, report)))
                    .collect(),
            ),
            other => other,
        }
    }

    fn redact_private_emails(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        apply_placeholder_regex(input, private_email_regex(), "private_email", state, report)
    }

    fn redact_known_paths(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        let mut output = input.to_string();
        for prefix in &self.known_path_prefixes {
            let count = output.matches(prefix).count();
            if count == 0 {
                continue;
            }
            let placeholder = state.placeholders.placeholder_for("local_path", prefix);
            output = output.replace(prefix, &placeholder);
            for _ in 0..count {
                report.increment("local_path");
                report.add_pii_label("local_path");
            }
        }
        output
    }

    fn redact_generic_paths(
        &self,
        input: &str,
        state: &mut RedactionState,
        report: &mut RedactionReport,
    ) -> String {
        apply_placeholder_regex(input, local_path_regex(), "local_path", state, report)
    }
}

#[derive(Debug, Default)]
struct RedactionState {
    placeholders: PlaceholderMap,
}

#[derive(Debug, Default)]
struct PlaceholderMap {
    by_label_and_value: BTreeMap<(String, String), String>,
    next_by_label: BTreeMap<String, u32>,
}

impl PlaceholderMap {
    fn placeholder_for(&mut self, label: &str, value: &str) -> String {
        let key = (label.to_string(), value.to_string());
        if let Some(existing) = self.by_label_and_value.get(&key) {
            return existing.clone();
        }

        let next = self.next_by_label.entry(label.to_string()).or_insert(0);
        *next += 1;
        let token = format!("<PRIVATE_{}_{}>", placeholder_label_fragment(label), *next);
        self.by_label_and_value.insert(key, token.clone());
        token
    }
}

#[async_trait]
impl TraceRedactor for DeterministicTraceRedactor {
    async fn redact_trace(
        &self,
        trace: RawTraceContribution,
    ) -> Result<TraceContributionEnvelope, TraceContributionError> {
        let mut report = RedactionReport::default();
        let mut state = RedactionState::default();
        let mut privacy_filter_summary = None;
        let mut events = Vec::with_capacity(trace.events.len());
        let trace_card_scopes = trace.consent.scopes.clone();
        let trace_card_channel = trace.ironclaw.channel;
        let trace_card_revocation_handle = trace.contributor.revocation_handle;

        for raw_event in trace.events {
            let redacted_content = match raw_event.content {
                Some(content) => {
                    let (mut redacted, child_report) =
                        self.redact_text_with_state(&content, &mut state);
                    report.merge(child_report);
                    redacted = self
                        .apply_privacy_filter_to_text(
                            redacted,
                            &mut report,
                            &mut privacy_filter_summary,
                        )
                        .await?;
                    Some(redacted)
                }
                None => None,
            };

            let (structured_payload, payload_report) = self.redact_json_value(
                raw_event.tool_name.as_deref(),
                &raw_event.structured_payload,
                &mut state,
            );
            report.merge(payload_report);

            let tool_call_id = raw_event
                .structured_payload
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let tool_category = raw_event.tool_name.as_deref().map(tool_category_for);
            let side_effect = side_effect_for(raw_event.event_type, raw_event.tool_name.as_deref());

            events.push(TraceContributionEvent {
                event_id: raw_event.event_id,
                parent_event_id: None,
                event_type: raw_event.event_type,
                timestamp: raw_event.timestamp,
                redacted_content,
                structured_payload,
                tool_name: raw_event.tool_name,
                tool_category,
                tool_call_id,
                latency_ms: raw_event.latency_ms,
                token_counts: raw_event.token_counts,
                cost_usd: raw_event.cost_usd,
                success: None,
                failure_modes: Vec::new(),
                side_effect,
            });
        }

        let mut outcome = trace.outcome;
        if let Some(correction) = outcome.human_correction.take() {
            let (mut redacted, child_report) = self.redact_text_with_state(&correction, &mut state);
            report.merge(child_report);
            redacted = self
                .apply_privacy_filter_to_text(redacted, &mut report, &mut privacy_filter_summary)
                .await?;
            outcome.human_correction = Some(redacted);
        }

        let residual_pii_risk = residual_risk(&trace.consent, &report);
        let redaction_hash = redaction_hash(&events, &report.counts);
        let mut warnings = privacy_warnings(residual_pii_risk);
        warnings.extend(report.warnings.clone());
        let privacy = PrivacyMetadata {
            redaction_pipeline_version: redaction_pipeline_version(
                privacy_filter_summary.is_some(),
            ),
            redaction_counts: report.counts,
            privacy_filter_summary,
            pii_labels_present: report.pii_labels_present,
            residual_pii_risk,
            redaction_hash,
            warnings,
        };

        let trace_card = build_trace_card(
            &trace_card_scopes,
            trace_card_channel,
            trace_card_revocation_handle,
            &events,
        );
        let value_card = TraceValueCard::default();
        Ok(TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: trace.trace_id,
            submission_id: trace.submission_id,
            created_at: trace.created_at,
            ironclaw: trace.ironclaw,
            consent: trace.consent,
            contributor: trace.contributor,
            privacy,
            events,
            outcome,
            replay: trace.replay,
            embedding_analysis: trace.embedding_analysis,
            value: trace.value,
            trace_card,
            value_card,
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        })
    }
}

pub fn rescrub_trace_envelope(envelope: &mut TraceContributionEnvelope) {
    let redactor = DeterministicTraceRedactor::default();
    rescrub_trace_envelope_with(&redactor, envelope);
}

pub fn rescrub_trace_envelope_with(
    redactor: &DeterministicTraceRedactor,
    envelope: &mut TraceContributionEnvelope,
) {
    let mut report = RedactionReport::default();
    let mut state = RedactionState::default();

    for event in &mut envelope.events {
        if let Some(content) = event.redacted_content.take() {
            let (redacted, child_report) = redactor.redact_text_with_state(&content, &mut state);
            report.merge(child_report);
            event.redacted_content = Some(redacted);
        }

        if !event.structured_payload.is_null() {
            let (redacted_payload, child_report) = redactor.redact_json_value(
                event.tool_name.as_deref(),
                &event.structured_payload,
                &mut state,
            );
            report.merge(child_report);
            event.structured_payload = redacted_payload;
        }
    }

    if let Some(correction) = envelope.outcome.human_correction.take() {
        let (redacted, child_report) = redactor.redact_text_with_state(&correction, &mut state);
        report.merge(child_report);
        envelope.outcome.human_correction = Some(redacted);
    }

    let blocked_secret_detected = report.blocked_secret_detected;
    for (label, count) in report.counts {
        *envelope.privacy.redaction_counts.entry(label).or_insert(0) += count;
    }
    for label in report.pii_labels_present {
        if !envelope.privacy.pii_labels_present.contains(&label) {
            envelope.privacy.pii_labels_present.push(label);
        }
    }

    let server_pass_risk = residual_risk(
        &envelope.consent,
        &RedactionReport {
            counts: BTreeMap::new(),
            pii_labels_present: Vec::new(),
            warnings: Vec::new(),
            blocked_secret_detected,
        },
    );
    envelope.privacy.residual_pii_risk =
        max_residual_risk(envelope.privacy.residual_pii_risk, server_pass_risk);
    if !envelope
        .privacy
        .redaction_pipeline_version
        .contains(SERVER_RESCRUB_PIPELINE_SUFFIX)
    {
        envelope.privacy.redaction_pipeline_version.push('+');
        envelope
            .privacy
            .redaction_pipeline_version
            .push_str(SERVER_RESCRUB_PIPELINE_SUFFIX);
    }
    envelope.trace_card.redaction_pipeline_version =
        envelope.privacy.redaction_pipeline_version.clone();
    merge_privacy_warnings(
        &mut envelope.privacy.warnings,
        privacy_warnings(envelope.privacy.residual_pii_risk),
    );
    merge_privacy_warnings(
        &mut envelope.privacy.warnings,
        vec!["Server-side trace re-scrub was applied before corpus storage.".to_string()],
    );
    envelope.privacy.redaction_hash =
        redaction_hash(&envelope.events, &envelope.privacy.redaction_counts);
}

fn residual_risk(consent: &ConsentMetadata, report: &RedactionReport) -> ResidualPiiRisk {
    if report.blocked_secret_detected {
        return ResidualPiiRisk::High;
    }

    if consent.message_text_included || consent.tool_payloads_included {
        return ResidualPiiRisk::Medium;
    }

    ResidualPiiRisk::Low
}

fn max_residual_risk(left: ResidualPiiRisk, right: ResidualPiiRisk) -> ResidualPiiRisk {
    use ResidualPiiRisk::{High, Low, Medium};
    match (left, right) {
        (High, _) | (_, High) => High,
        (Medium, _) | (_, Medium) => Medium,
        (Low, Low) => Low,
    }
}

fn merge_privacy_warnings(existing: &mut Vec<String>, new_warnings: Vec<String>) {
    for warning in new_warnings {
        if !existing.contains(&warning) {
            existing.push(warning);
        }
    }
}

fn privacy_warnings(risk: ResidualPiiRisk) -> Vec<String> {
    match risk {
        ResidualPiiRisk::Low => Vec::new(),
        ResidualPiiRisk::Medium => vec![
            "Message text or tool payloads were included after local redaction; server-side re-scrub is still required.".to_string(),
        ],
        ResidualPiiRisk::High => vec![
            "Secret-like content was detected after deterministic scrubbing; keep this trace quarantined until reviewed.".to_string(),
        ],
    }
}

fn build_trace_card(
    consent_scopes: &[ConsentScope],
    channel: TraceChannel,
    revocation_handle: Uuid,
    events: &[TraceContributionEvent],
) -> TraceCard {
    let consent_scope = consent_scopes
        .first()
        .copied()
        .unwrap_or(ConsentScope::DebuggingEvaluation);
    let tool_categories = events
        .iter()
        .filter_map(|event| event.tool_category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    TraceCard {
        consent_scope,
        redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
        source_channel: channel_label(channel).to_string(),
        tool_categories,
        allowed_uses: allowed_uses_for_scopes(consent_scopes),
        retention_policy: "private_corpus_revocable".to_string(),
        revocation_handle: revocation_handle.to_string(),
    }
}

fn allowed_uses_for_scopes(scopes: &[ConsentScope]) -> Vec<TraceAllowedUse> {
    if scopes.is_empty() {
        return default_allowed_uses_for_scope(ConsentScope::DebuggingEvaluation);
    }

    scopes
        .iter()
        .flat_map(|scope| default_allowed_uses_for_scope(*scope))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn default_allowed_uses_for_scope(scope: ConsentScope) -> Vec<TraceAllowedUse> {
    match scope {
        ConsentScope::DebuggingEvaluation => vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::AggregateAnalytics,
        ],
        ConsentScope::BenchmarkOnly => vec![
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::BenchmarkGeneration,
            TraceAllowedUse::AggregateAnalytics,
        ],
        ConsentScope::RankingTraining => vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::RankingModelTraining,
            TraceAllowedUse::AggregateAnalytics,
        ],
        ConsentScope::ModelTraining => vec![
            TraceAllowedUse::Debugging,
            TraceAllowedUse::Evaluation,
            TraceAllowedUse::RankingModelTraining,
            TraceAllowedUse::ModelTraining,
            TraceAllowedUse::AggregateAnalytics,
        ],
    }
}

pub fn retention_policy_for_allowed_use(allowed_use: TraceAllowedUse) -> TraceRetentionPolicy {
    match allowed_use {
        TraceAllowedUse::Debugging | TraceAllowedUse::Evaluation => TraceRetentionPolicy {
            name: "private_corpus_revocable".to_string(),
            class: TraceRetentionClass::PrivateCorpusRevocable,
            revocable: true,
            max_age_days: Some(730),
            allows_derived_artifacts: true,
        },
        TraceAllowedUse::BenchmarkGeneration => TraceRetentionPolicy {
            name: "benchmark_revocable".to_string(),
            class: TraceRetentionClass::BenchmarkRevocable,
            revocable: true,
            max_age_days: Some(1095),
            allows_derived_artifacts: true,
        },
        TraceAllowedUse::RankingModelTraining | TraceAllowedUse::ModelTraining => {
            TraceRetentionPolicy {
                name: "training_revocable".to_string(),
                class: TraceRetentionClass::TrainingRevocable,
                revocable: true,
                max_age_days: Some(1095),
                allows_derived_artifacts: true,
            }
        }
        TraceAllowedUse::AggregateAnalytics => TraceRetentionPolicy {
            name: "aggregate_only".to_string(),
            class: TraceRetentionClass::AggregateOnly,
            revocable: false,
            max_age_days: None,
            allows_derived_artifacts: false,
        },
    }
}

pub fn retention_policy_for_trace(envelope: &TraceContributionEnvelope) -> TraceRetentionPolicy {
    let strongest = envelope
        .trace_card
        .allowed_uses
        .iter()
        .copied()
        .max_by_key(|allowed_use| match allowed_use {
            TraceAllowedUse::ModelTraining => 5,
            TraceAllowedUse::RankingModelTraining => 4,
            TraceAllowedUse::BenchmarkGeneration => 3,
            TraceAllowedUse::Evaluation => 2,
            TraceAllowedUse::Debugging => 1,
            TraceAllowedUse::AggregateAnalytics => 0,
        })
        .unwrap_or(TraceAllowedUse::Debugging);
    let mut policy = retention_policy_for_allowed_use(strongest);
    if !envelope.consent.revocable {
        policy.revocable = false;
    }
    policy
}

pub fn derived_artifact_invalidation_marker(
    envelope: &TraceContributionEnvelope,
    reason: impl Into<String>,
) -> DerivedArtifactInvalidationMarker {
    DerivedArtifactInvalidationMarker {
        schema_version: "ironclaw.trace_derived_artifact_invalidation.v1".to_string(),
        submission_id: envelope.submission_id,
        trace_id: envelope.trace_id,
        revocation_handle_hash: canonical_hash(&envelope.contributor.revocation_handle.to_string()),
        redaction_hash: envelope.privacy.redaction_hash.clone(),
        artifact_prefixes: derived_artifact_prefixes(envelope),
        reason: reason.into(),
        created_at: Utc::now(),
    }
}

pub fn derived_artifact_prefixes(envelope: &TraceContributionEnvelope) -> Vec<String> {
    let trace_id = envelope.trace_id;
    let submission_id = envelope.submission_id;
    vec![
        format!("trace:{trace_id}"),
        format!("submission:{submission_id}"),
        format!("summary:{trace_id}"),
        format!("embedding:{trace_id}"),
        format!("benchmark:{trace_id}"),
        format!("training_example:{trace_id}"),
    ]
}

pub fn trace_dataset_eligibility(
    envelope: &TraceContributionEnvelope,
    requested_use: TraceAllowedUse,
    revoked: bool,
) -> TraceDatasetEligibility {
    let retention_policy = retention_policy_for_allowed_use(requested_use);
    let mut reasons = Vec::new();

    if revoked {
        reasons.push("submission has been revoked".to_string());
    }
    if !envelope.trace_card.allowed_uses.contains(&requested_use) {
        reasons.push("requested use is outside consent scope".to_string());
    }
    if !envelope.consent.revocable && retention_policy.revocable {
        reasons.push("trace consent is not revocable for a revocable dataset class".to_string());
    }
    match envelope.privacy.residual_pii_risk {
        ResidualPiiRisk::Low => {}
        ResidualPiiRisk::Medium => {
            if matches!(
                requested_use,
                TraceAllowedUse::BenchmarkGeneration
                    | TraceAllowedUse::RankingModelTraining
                    | TraceAllowedUse::ModelTraining
            ) {
                reasons.push(
                    "medium residual privacy risk is limited to debugging, evaluation, or aggregate analytics"
                        .to_string(),
                );
            }
        }
        ResidualPiiRisk::High => {
            reasons.push("high residual privacy risk is not dataset eligible".to_string());
        }
    }
    if envelope
        .privacy
        .warnings
        .iter()
        .any(|warning| warning.to_ascii_lowercase().contains("quarantined"))
    {
        reasons.push("trace is quarantined by privacy warning".to_string());
    }

    TraceDatasetEligibility {
        eligible: reasons.is_empty(),
        requested_use,
        retention_policy,
        reasons,
    }
}

fn channel_label(channel: TraceChannel) -> &'static str {
    match channel {
        TraceChannel::Web => "web",
        TraceChannel::Cli => "cli",
        TraceChannel::Telegram => "telegram",
        TraceChannel::Slack => "slack",
        TraceChannel::Routine => "routine",
        TraceChannel::Other => "other",
    }
}

fn tool_category_for(tool_name: &str) -> String {
    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("http") || lower.contains("browser") || lower.contains("web") {
        "network".to_string()
    } else if lower.contains("file")
        || lower.contains("fs")
        || lower.contains("workspace")
        || lower.contains("shell")
        || lower.contains("exec")
    {
        "workspace".to_string()
    } else if lower.contains("memory") || lower.contains("search") {
        "retrieval".to_string()
    } else if lower.contains("calendar") || lower.contains("email") || lower.contains("slack") {
        "external_app".to_string()
    } else {
        "other".to_string()
    }
}

fn side_effect_for(
    event_type: TraceContributionEventType,
    tool_name: Option<&str>,
) -> SideEffectLevel {
    match event_type {
        TraceContributionEventType::UserMessage
        | TraceContributionEventType::AssistantMessage
        | TraceContributionEventType::Feedback => SideEffectLevel::None,
        TraceContributionEventType::RoutingDecision => SideEffectLevel::None,
        TraceContributionEventType::ToolResult => SideEffectLevel::None,
        TraceContributionEventType::HttpExchange => SideEffectLevel::ReadOnly,
        TraceContributionEventType::ToolCall => tool_name
            .map(classify_tool_side_effect)
            .unwrap_or(SideEffectLevel::Unknown),
    }
}

fn classify_tool_side_effect(tool_name: &str) -> SideEffectLevel {
    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("write")
        || lower.contains("create")
        || lower.contains("delete")
        || lower.contains("send")
        || lower.contains("post")
    {
        if lower.contains("email") || lower.contains("calendar") || lower.contains("slack") {
            SideEffectLevel::ExternalWrite
        } else {
            SideEffectLevel::LocalWrite
        }
    } else if lower.contains("auth") || lower.contains("credential") || lower.contains("token") {
        SideEffectLevel::CredentialUse
    } else {
        SideEffectLevel::ReadOnly
    }
}

pub fn canonical_summary_for_embedding(envelope: &TraceContributionEnvelope) -> String {
    canonical_whole_trace_representation(envelope)
}

pub fn canonical_representations_for_embedding(
    envelope: &TraceContributionEnvelope,
) -> Vec<CanonicalTraceRepresentation> {
    let mut representations = Vec::new();
    push_canonical_representation(
        &mut representations,
        envelope,
        CanonicalRepresentationKind::WholeTrace,
        0,
        canonical_whole_trace_representation(envelope),
    );

    for (index, content) in canonical_turn_representations(envelope)
        .into_iter()
        .enumerate()
    {
        push_canonical_representation(
            &mut representations,
            envelope,
            CanonicalRepresentationKind::Turn,
            index,
            content,
        );
    }

    let tool_sequence = canonical_tool_sequence_representation(envelope);
    if !tool_sequence.is_empty() {
        push_canonical_representation(
            &mut representations,
            envelope,
            CanonicalRepresentationKind::ToolSequence,
            0,
            tool_sequence,
        );
    }

    let error_outcome = canonical_error_outcome_representation(envelope);
    if !error_outcome.is_empty() {
        push_canonical_representation(
            &mut representations,
            envelope,
            CanonicalRepresentationKind::ErrorOutcome,
            0,
            error_outcome,
        );
    }

    if let Some(correction) = canonical_correction_representation(envelope) {
        push_canonical_representation(
            &mut representations,
            envelope,
            CanonicalRepresentationKind::Correction,
            0,
            correction,
        );
    }

    representations
}

fn push_canonical_representation(
    representations: &mut Vec<CanonicalTraceRepresentation>,
    envelope: &TraceContributionEnvelope,
    kind: CanonicalRepresentationKind,
    index: usize,
    content: String,
) {
    let canonical_hash = canonical_hash(&content);
    let hash_fragment = canonical_hash
        .strip_prefix("sha256:")
        .unwrap_or(&canonical_hash)
        .chars()
        .take(16)
        .collect::<String>();
    representations.push(CanonicalTraceRepresentation {
        kind,
        vector_key: format!(
            "trace:{}:{:?}:{}:{}",
            envelope.trace_id, kind, index, hash_fragment
        )
        .to_ascii_lowercase(),
        canonical_hash,
        content,
    });
}

fn canonical_whole_trace_representation(envelope: &TraceContributionEnvelope) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Outcome: {:?}", envelope.outcome.task_success));
    if !envelope.replay.required_tools.is_empty() {
        lines.push(format!(
            "Tools used: {}",
            envelope.replay.required_tools.join(", ")
        ));
    }
    let failure_modes = envelope
        .outcome
        .failure_modes
        .iter()
        .map(|mode| format!("{mode:?}"))
        .collect::<Vec<_>>();
    if !failure_modes.is_empty() {
        lines.push(format!("Failure modes: {}", failure_modes.join(", ")));
    }
    lines.push(format!(
        "User correction included: {}",
        envelope.outcome.human_correction.is_some()
    ));
    lines.push("Redacted summary:".to_string());

    for event in envelope.events.iter().take(12) {
        let mut line = format!("  {:?}:", event.event_type);
        if let Some(tool_name) = &event.tool_name {
            line.push_str(&format!(" tool={tool_name}"));
        }
        if let Some(content) = &event.redacted_content {
            line.push(' ');
            line.push_str(content);
        } else if !event.structured_payload.is_null() {
            line.push_str(" payload=");
            line.push_str(&safe_payload_summary(&event.structured_payload));
        }
        lines.push(line);
    }

    lines.join("\n")
}

fn canonical_turn_representations(envelope: &TraceContributionEnvelope) -> Vec<String> {
    let mut turns = Vec::new();
    let mut current = Vec::new();
    let mut turn_index = 0usize;

    for event in &envelope.events {
        if event.event_type == TraceContributionEventType::UserMessage && !current.is_empty() {
            turns.push(canonical_turn_content(turn_index, &current));
            current.clear();
            turn_index += 1;
        }
        current.push(event);
    }
    if !current.is_empty() {
        turns.push(canonical_turn_content(turn_index, &current));
    }

    turns
}

fn canonical_turn_content(turn_index: usize, events: &[&TraceContributionEvent]) -> String {
    let mut lines = vec![format!("Turn: {turn_index}")];
    for event in events {
        lines.push(canonical_event_line(event));
    }
    lines.join("\n")
}

fn canonical_tool_sequence_representation(envelope: &TraceContributionEnvelope) -> String {
    let mut lines = Vec::new();
    for event in envelope
        .events
        .iter()
        .filter(|event| event.event_type == TraceContributionEventType::ToolCall)
    {
        let tool_name = event.tool_name.as_deref().unwrap_or("unknown");
        let category = event.tool_category.as_deref().unwrap_or("unknown");
        lines.push(format!(
            "Tool: name={tool_name} category={category} side_effect={:?} success={}",
            event.side_effect,
            event
                .success
                .map(|success| success.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }
    lines.join("\n")
}

fn canonical_error_outcome_representation(envelope: &TraceContributionEnvelope) -> String {
    let has_error_signal = !envelope.outcome.error_taxonomy.is_empty()
        || !envelope.outcome.failure_modes.is_empty()
        || matches!(
            envelope.outcome.task_success,
            TaskSuccess::Failure | TaskSuccess::Partial
        )
        || envelope
            .events
            .iter()
            .any(|event| !event.failure_modes.is_empty() || event.success == Some(false));
    if !has_error_signal {
        return String::new();
    }

    let mut lines = vec![format!("Task success: {:?}", envelope.outcome.task_success)];
    if !envelope.outcome.error_taxonomy.is_empty() {
        lines.push(format!(
            "Error taxonomy: {}",
            envelope.outcome.error_taxonomy.join(", ")
        ));
    }
    if !envelope.outcome.failure_modes.is_empty() {
        lines.push(format!(
            "Outcome failure modes: {}",
            envelope
                .outcome
                .failure_modes
                .iter()
                .map(|mode| format!("{mode:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for event in envelope
        .events
        .iter()
        .filter(|event| !event.failure_modes.is_empty() || event.success == Some(false))
    {
        lines.push(canonical_event_line(event));
    }
    lines.join("\n")
}

fn canonical_correction_representation(envelope: &TraceContributionEnvelope) -> Option<String> {
    let correction = envelope.outcome.human_correction.as_ref()?;
    let mut lines = vec![format!("Correction: {correction}")];
    if !envelope.outcome.failure_modes.is_empty() {
        lines.push(format!(
            "Failure modes: {}",
            envelope
                .outcome
                .failure_modes
                .iter()
                .map(|mode| format!("{mode:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Some(lines.join("\n"))
}

fn canonical_event_line(event: &TraceContributionEvent) -> String {
    let mut line = format!("{:?}:", event.event_type);
    if let Some(tool_name) = &event.tool_name {
        line.push_str(&format!(" tool={tool_name}"));
    }
    if let Some(content) = &event.redacted_content {
        line.push(' ');
        line.push_str(content);
    } else if !event.structured_payload.is_null() {
        line.push_str(" payload=");
        line.push_str(&safe_payload_summary(&event.structured_payload));
    }
    if !event.failure_modes.is_empty() {
        line.push_str(" failure_modes=");
        line.push_str(
            &event
                .failure_modes
                .iter()
                .map(|mode| format!("{mode:?}"))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    line
}

fn safe_payload_summary(payload: &Value) -> String {
    match payload {
        Value::Object(map) => {
            let keys = map.keys().take(8).cloned().collect::<Vec<_>>();
            format!("keys({})", keys.join(","))
        }
        Value::Array(items) => format!("array(len={})", items.len()),
        Value::String(_) => "redacted_string".to_string(),
        Value::Null => "null".to_string(),
        Value::Bool(_) | Value::Number(_) => "scalar".to_string(),
    }
}

fn canonical_hash(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

fn redaction_hash(events: &[TraceContributionEvent], counts: &BTreeMap<String, u32>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(events).unwrap_or_default());
    hasher.update(serde_json::to_vec(counts).unwrap_or_default());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn redact_tool_specific_payload(
    tool_name: Option<&str>,
    value: &Value,
    report: &mut RedactionReport,
) -> Value {
    let Some(profile) = tool_name.and_then(tool_payload_profile) else {
        return value.clone();
    };
    redact_tool_specific_value(value, profile, None, report)
}

fn redact_tool_specific_value(
    value: &Value,
    profile: ToolPayloadProfile,
    field_name: Option<&str>,
    report: &mut RedactionReport,
) -> Value {
    if let Some(action) = field_name.and_then(|field| tool_redaction_action(profile, field)) {
        report.increment("tool_sensitive_field");
        report.increment(format!("tool_sensitive_field:{}", action.label()));
        report.add_pii_label(action.label());
        return apply_tool_redaction_action(value, action);
    }

    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| {
                    (
                        key.clone(),
                        redact_tool_specific_value(child, profile, Some(key), report),
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|child| redact_tool_specific_value(child, profile, None, report))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolPayloadProfile {
    Browser,
    Calendar,
    Database,
    Email,
    Filesystem,
    IssueTracker,
    Messaging,
}

fn tool_payload_profile(tool_name: &str) -> Option<ToolPayloadProfile> {
    let lower = tool_name.to_ascii_lowercase();
    if lower.contains("email") || lower.contains("gmail") {
        Some(ToolPayloadProfile::Email)
    } else if lower.contains("calendar") {
        Some(ToolPayloadProfile::Calendar)
    } else if lower.contains("slack")
        || lower.contains("telegram")
        || lower.contains("signal")
        || lower.contains("discord")
    {
        Some(ToolPayloadProfile::Messaging)
    } else if lower.contains("github")
        || lower.contains("gitlab")
        || lower.contains("linear")
        || lower.contains("issue")
        || lower.contains("pull_request")
        || lower.contains("pr_")
    {
        Some(ToolPayloadProfile::IssueTracker)
    } else if lower.contains("browser")
        || lower.contains("http")
        || lower.contains("fetch")
        || lower.contains("url")
        || lower.contains("web")
    {
        Some(ToolPayloadProfile::Browser)
    } else if lower.contains("sql")
        || lower.contains("db")
        || lower.contains("database")
        || lower.contains("postgres")
        || lower.contains("libsql")
        || lower.contains("mysql")
    {
        Some(ToolPayloadProfile::Database)
    } else if lower.contains("file")
        || lower.contains("fs")
        || lower.contains("workspace")
        || lower.contains("shell")
        || lower.contains("exec")
    {
        Some(ToolPayloadProfile::Filesystem)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolRedactionAction {
    Replace(&'static str),
    SanitizeUrl(&'static str),
    RedactObjectValues(&'static str),
    SummarizeCollection(&'static str),
}

impl ToolRedactionAction {
    fn label(self) -> &'static str {
        match self {
            ToolRedactionAction::Replace(label)
            | ToolRedactionAction::SanitizeUrl(label)
            | ToolRedactionAction::RedactObjectValues(label)
            | ToolRedactionAction::SummarizeCollection(label) => label,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ToolSensitiveFieldRule {
    matcher: ToolFieldMatcher,
    action: ToolRedactionAction,
}

#[derive(Debug, Clone, Copy)]
enum ToolFieldMatcher {
    Exact(&'static [&'static str]),
    Contains(&'static [&'static str]),
}

const EMAIL_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "to",
            "cc",
            "bcc",
            "from",
            "reply_to",
            "replyto",
            "recipient",
            "recipients",
            "sender",
        ]),
        action: ToolRedactionAction::SummarizeCollection("email_participant"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "subject", "body", "text", "html", "snippet", "message", "raw", "mime",
        ]),
        action: ToolRedactionAction::Replace("email_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["headers", "header"]),
        action: ToolRedactionAction::RedactObjectValues("email_header"),
    },
];

const CALENDAR_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&["attendee", "participant", "organizer", "creator"]),
        action: ToolRedactionAction::SummarizeCollection("calendar_participant"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "summary",
            "title",
            "description",
            "location",
            "notes",
            "calendar_id",
            "hangout_link",
            "conference_data",
            "conference_uri",
        ]),
        action: ToolRedactionAction::Replace("calendar_content"),
    },
];

const MESSAGING_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&[
            "channel",
            "conversation",
            "user",
            "member",
            "team",
            "workspace",
            "chat",
        ]),
        action: ToolRedactionAction::Replace("message_identity"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "text",
            "message",
            "body",
            "blocks",
            "attachments",
            "permalink",
            "thread",
            "thread_ts",
        ]),
        action: ToolRedactionAction::Replace("message_content"),
    },
];

const ISSUE_TRACKER_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "title",
            "body",
            "description",
            "comment",
            "comments",
            "summary",
            "content",
        ]),
        action: ToolRedactionAction::Replace("issue_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["url", "html_url", "api_url", "web_url", "href"]),
        action: ToolRedactionAction::SanitizeUrl("private_url"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&[
            "author",
            "assignee",
            "reviewer",
            "requester",
            "creator",
            "owner",
            "repo",
            "repository",
            "org",
            "organization",
            "project",
            "team",
            "user",
        ]),
        action: ToolRedactionAction::Replace("issue_identity"),
    },
];

const BROWSER_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["url", "href", "referrer", "referer", "current_url"]),
        action: ToolRedactionAction::SanitizeUrl("private_url"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["headers", "header", "cookies", "cookie"]),
        action: ToolRedactionAction::RedactObjectValues("browser_header"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&["body", "html", "text", "title", "content", "dom"]),
        action: ToolRedactionAction::Replace("browser_content"),
    },
];

const DATABASE_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "query",
            "sql",
            "statement",
            "prepared_statement",
            "connection_string",
            "database_url",
        ]),
        action: ToolRedactionAction::Replace("database_content"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "params",
            "parameters",
            "args",
            "arguments",
            "values",
            "binds",
            "bindings",
            "query_params",
        ]),
        action: ToolRedactionAction::SummarizeCollection("database_query_param"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "row", "rows", "record", "records", "result", "results", "data",
        ]),
        action: ToolRedactionAction::SummarizeCollection("database_row"),
    },
];

const FILESYSTEM_RULES: &[ToolSensitiveFieldRule] = &[
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Contains(&["path", "file", "filename", "cwd", "directory"]),
        action: ToolRedactionAction::Replace("local_path"),
    },
    ToolSensitiveFieldRule {
        matcher: ToolFieldMatcher::Exact(&[
            "content", "contents", "command", "stdout", "stderr", "diff", "patch",
        ]),
        action: ToolRedactionAction::Replace("workspace_content"),
    },
];

fn tool_redaction_action(
    profile: ToolPayloadProfile,
    field_name: &str,
) -> Option<ToolRedactionAction> {
    let lower = field_name.to_ascii_lowercase();

    profile_rules(profile)
        .iter()
        .find(|rule| field_matches(&lower, rule.matcher))
        .map(|rule| rule.action)
}

fn profile_rules(profile: ToolPayloadProfile) -> &'static [ToolSensitiveFieldRule] {
    match profile {
        ToolPayloadProfile::Email => EMAIL_RULES,
        ToolPayloadProfile::Calendar => CALENDAR_RULES,
        ToolPayloadProfile::Messaging => MESSAGING_RULES,
        ToolPayloadProfile::IssueTracker => ISSUE_TRACKER_RULES,
        ToolPayloadProfile::Browser => BROWSER_RULES,
        ToolPayloadProfile::Database => DATABASE_RULES,
        ToolPayloadProfile::Filesystem => FILESYSTEM_RULES,
    }
}

fn field_matches(lower_field_name: &str, matcher: ToolFieldMatcher) -> bool {
    match matcher {
        ToolFieldMatcher::Exact(names) => names.contains(&lower_field_name),
        ToolFieldMatcher::Contains(fragments) => fragments
            .iter()
            .any(|fragment| lower_field_name.contains(fragment)),
    }
}

fn apply_tool_redaction_action(value: &Value, action: ToolRedactionAction) -> Value {
    match action {
        ToolRedactionAction::Replace(label) => redacted_scalar_or_summary(label, value),
        ToolRedactionAction::SanitizeUrl(label) => sanitize_url_value(value, label),
        ToolRedactionAction::RedactObjectValues(label) => redact_object_values(value, label),
        ToolRedactionAction::SummarizeCollection(label) => summarize_collection(label, value),
    }
}

fn redacted_scalar_or_summary(label: &str, value: &Value) -> Value {
    match value {
        Value::Array(_) | Value::Object(_) => summarize_collection(label, value),
        _ => Value::String(redacted_marker(label)),
    }
}

fn redact_object_values(value: &Value, label: &str) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.keys()
                .map(|key| (key.clone(), Value::String(redacted_marker(label))))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_object_values(item, label))
                .collect(),
        ),
        _ => Value::String(redacted_marker(label)),
    }
}

fn summarize_collection(label: &str, value: &Value) -> Value {
    match value {
        Value::Array(items) => serde_json::json!({
            "redacted": redacted_marker(label),
            "count": items.len(),
        }),
        Value::Object(map) => serde_json::json!({
            "redacted": redacted_marker(label),
            "field_count": map.len(),
        }),
        _ => Value::String(redacted_marker(label)),
    }
}

fn sanitize_url_value(value: &Value, label: &str) -> Value {
    match value {
        Value::String(url) => Value::String(sanitize_private_url(url, label)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_url_value(item, label))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| (key.clone(), sanitize_url_value(child, label)))
                .collect(),
        ),
        _ => Value::String(redacted_marker(label)),
    }
}

fn sanitize_private_url(raw_url: &str, label: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(raw_url) else {
        return redacted_marker(label);
    };

    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return redacted_marker(label);
    }

    let has_private_components =
        url.path() != "/" || url.query().is_some() || url.fragment().is_some();
    if has_private_components {
        url.set_path("/[REDACTED_PATH]");
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.username().is_empty() {
        let _ = url.set_username("");
    }
    let _ = url.set_password(None);
    url.to_string()
}

fn redacted_marker(label: &str) -> String {
    format!("[REDACTED:{label}]")
}

fn count_sensitive_field_redactions(before: &Value, after: &Value, report: &mut RedactionReport) {
    match (before, after) {
        (Value::Object(before_map), Value::Object(after_map)) => {
            for (key, before_value) in before_map {
                if let Some(after_value) = after_map.get(key) {
                    count_sensitive_field_redactions(before_value, after_value, report);
                }
            }
        }
        (Value::Array(before_items), Value::Array(after_items)) => {
            for (before_value, after_value) in before_items.iter().zip(after_items.iter()) {
                count_sensitive_field_redactions(before_value, after_value, report);
            }
        }
        (before_value, Value::String(redacted))
            if redacted == "[REDACTED]" && before_value != after =>
        {
            report.increment("sensitive_field");
        }
        _ => {}
    }
}

fn apply_redaction_ranges(input: &str, ranges: &[std::ops::Range<usize>]) -> String {
    apply_labeled_ranges(input, ranges, "[REDACTED]")
}

fn apply_placeholder_regex(
    input: &str,
    regex: &Regex,
    label: &str,
    state: &mut RedactionState,
    report: &mut RedactionReport,
) -> String {
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;
    let mut changed = false;

    for mat in regex.find_iter(input) {
        let candidate = mat.as_str();
        if candidate.contains("<PRIVATE_") || candidate.contains("[REDACTED") {
            continue;
        }
        result.push_str(&input[last_end..mat.start()]);
        let placeholder = state.placeholders.placeholder_for(label, candidate);
        result.push_str(&placeholder);
        last_end = mat.end();
        report.increment(label);
        report.add_pii_label(label);
        changed = true;
    }

    if !changed {
        return input.to_string();
    }
    result.push_str(&input[last_end..]);
    result
}

fn apply_labeled_ranges(
    input: &str,
    ranges: &[std::ops::Range<usize>],
    replacement: &str,
) -> String {
    if ranges.is_empty() {
        return input.to_string();
    }

    let mut ranges = ranges.to_vec();
    ranges.sort_by_key(|range| range.start);

    let mut result = String::with_capacity(input.len());
    let mut last_end = 0;
    for range in ranges {
        if range.start < last_end {
            continue;
        }
        result.push_str(&input[last_end..range.start]);
        result.push_str(replacement);
        last_end = range.end;
    }
    result.push_str(&input[last_end..]);
    result
}

fn private_email_regex() -> &'static Regex {
    static PRIVATE_EMAIL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by unit tests and should always compile.
        Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b")
            .expect("hardcoded private email regex must compile")
    });
    &PRIVATE_EMAIL_REGEX
}

fn local_path_regex() -> &'static Regex {
    static LOCAL_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by unit tests and should always compile.
        Regex::new(r#"(?x)(?:/Users|/home|/private/var|/tmp)/[^\s'"`<>{}\[\]]+"#)
            .expect("hardcoded local path regex must compile")
    });
    &LOCAL_PATH_REGEX
}

fn trace_queue_secret_like_reason_regex() -> &'static Regex {
    static TRACE_QUEUE_SECRET_LIKE_REASON_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        // safety: hardcoded regex is covered by queue diagnostics tests.
        Regex::new(r"(?ix)\b(?:sk|pk|rk|ghp|gho|ghu|glpat|xox[baprs])[-_a-z0-9]{8,}\b")
            .expect("hardcoded trace queue secret-like reason regex must compile")
    });
    &TRACE_QUEUE_SECRET_LIKE_REASON_REGEX
}

fn placeholder_label_fragment(label: &str) -> String {
    let raw = label
        .strip_prefix("private_")
        .unwrap_or(label)
        .to_ascii_uppercase();
    raw.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalTraceSubmissionRecord {
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub status: LocalTraceSubmissionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    pub privacy_risk: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub redaction_counts: BTreeMap<String, u32>,
    #[serde(default)]
    pub credit_points_pending: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credit_explanation: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credit_events: Vec<TraceCreditEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_notice_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalTraceSubmissionStatus {
    Submitted,
    Revoked,
    Expired,
    Purged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionReceipt {
    #[serde(default = "default_submission_status")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_pending: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceSubmissionStatusRequest {
    pub submission_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSubmissionStatusUpdate {
    pub submission_id: Uuid,
    pub trace_id: Uuid,
    pub status: String,
    pub credit_points_pending: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub credit_points_ledger: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_points_total: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explanation: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delayed_credit_explanations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueHold {
    pub submission_id: Uuid,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
struct TraceQueueHoldSidecar {
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueFlushReport {
    pub submitted: usize,
    pub held: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<TraceQueueHold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_notice: Option<CreditSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueDiagnostics {
    pub queued_count: u32,
    pub held_count: u32,
    pub submitted_count: u32,
    pub revoked_count: u32,
    pub expired_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submission_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub held_reason_counts: BTreeMap<String, u32>,
    pub policy_enabled: bool,
    pub endpoint_configured: bool,
    pub ready_to_flush: bool,
}

pub enum TraceQueueEligibility {
    Submit,
    Hold { reason: String },
}

fn default_submission_status() -> String {
    "submitted".to_string()
}

fn is_zero_f32(value: &f32) -> bool {
    value.abs() <= f32::EPSILON
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

pub fn trace_contribution_dir_for_scope(scope: Option<&str>) -> PathBuf {
    let base = crate::bootstrap::ironclaw_base_dir().join("trace_contributions");
    match scope {
        Some(scope) if !scope.trim().is_empty() => base.join("users").join(scope_hash(scope)),
        _ => base,
    }
}

pub fn local_pseudonymous_contributor_id(scope: &str) -> String {
    format!("sha256:{}", scope_hash(scope))
}

pub fn local_pseudonymous_tenant_scope_ref(scope: &str) -> String {
    format!("tenant_sha256:{}", scope_hash(scope))
}

static TRACE_SCOPE_MUTATION_LOCKS: LazyLock<
    std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn trace_scope_mutation_lock_key(scope: Option<&str>) -> String {
    match scope {
        Some(scope) if !scope.trim().is_empty() => format!("scope:{}", scope_hash(scope)),
        _ => "global".to_string(),
    }
}

fn trace_scope_mutation_lock(scope: Option<&str>) -> Arc<tokio::sync::Mutex<()>> {
    let key = trace_scope_mutation_lock_key(scope);
    let mut locks = match TRACE_SCOPE_MUTATION_LOCKS.lock() {
        Ok(locks) => locks,
        Err(poisoned) => poisoned.into_inner(),
    };
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

async fn lock_trace_scope_for_mutation(scope: Option<&str>) -> OwnedMutexGuard<()> {
    trace_scope_mutation_lock(scope).lock_owned().await
}

fn lock_trace_scope_for_mutation_blocking(scope: Option<&str>) -> OwnedMutexGuard<()> {
    let lock = trace_scope_mutation_lock(scope);
    loop {
        if let Ok(guard) = lock.clone().try_lock_owned() {
            return guard;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub fn read_trace_policy_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<StandingTraceContributionPolicy> {
    let path = trace_policy_path(scope);
    if !path.exists() {
        return Ok(StandingTraceContributionPolicy::default());
    }
    let body = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read trace policy {}: {}", path.display(), e))?;
    serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("failed to parse trace policy {}: {}", path.display(), e))
}

pub fn write_trace_policy_for_scope(
    scope: Option<&str>,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    write_json_file(&trace_policy_path(scope), policy, "trace policy")
}

pub fn mark_trace_credit_notice_due_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    mark_trace_credit_noticed_if_due_unlocked(scope, policy.credit_notice_interval_hours)
}

pub fn queue_trace_envelope_for_scope(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<PathBuf> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    queue_trace_envelope_for_scope_unlocked(scope, envelope)
}

fn queue_trace_envelope_for_scope_unlocked(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<PathBuf> {
    let path = trace_queue_dir(scope).join(format!("{}.json", envelope.submission_id));
    write_json_file(&path, envelope, "queued trace envelope")?;
    Ok(path)
}

pub fn queued_trace_envelope_paths_for_scope(scope: Option<&str>) -> anyhow::Result<Vec<PathBuf>> {
    let dir = trace_queue_dir(scope);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .map_err(|e| anyhow::anyhow!("failed to read queue {}: {}", dir.display(), e))?
    {
        let entry = entry.map_err(|e| anyhow::anyhow!("failed to read queue entry: {}", e))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".held.json"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

pub fn read_trace_queue_holds_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceQueueHold>> {
    let dir = trace_queue_dir(scope);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut holds = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .map_err(|e| anyhow::anyhow!("failed to read queue {}: {}", dir.display(), e))?
    {
        let entry = entry.map_err(|e| anyhow::anyhow!("failed to read queue entry: {}", e))?;
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".held.json"))
        {
            continue;
        }

        let Some(submission_id) = trace_queue_hold_submission_id(&path) else {
            tracing::debug!(path = %path.display(), "Ignoring Trace Commons queue hold without a valid submission id");
            continue;
        };
        let Ok(body) = std::fs::read_to_string(&path) else {
            tracing::debug!(path = %path.display(), "Ignoring unreadable Trace Commons queue hold");
            continue;
        };
        let Ok(sidecar) = serde_json::from_str::<TraceQueueHoldSidecar>(&body) else {
            tracing::debug!(path = %path.display(), "Ignoring malformed Trace Commons queue hold");
            continue;
        };

        holds.push(TraceQueueHold {
            submission_id,
            reason: safe_trace_queue_hold_reason(sidecar.reason.as_deref().unwrap_or("held")),
        });
    }
    holds.sort_by_key(|hold| hold.submission_id);
    Ok(holds)
}

pub fn trace_queue_diagnostics_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<TraceQueueDiagnostics> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    let queued_count = queued_trace_envelope_paths_for_scope(scope)?.len() as u32;
    let holds = read_trace_queue_holds_for_scope(scope)?;
    let records = read_local_trace_records_for_scope(scope)?;
    let credit_report = trace_credit_report(&records);

    let mut held_reason_counts = BTreeMap::new();
    for hold in &holds {
        *held_reason_counts.entry(hold.reason.clone()).or_insert(0) += 1;
    }

    let endpoint_configured = policy
        .ingestion_endpoint
        .as_deref()
        .is_some_and(|endpoint| !endpoint.trim().is_empty());

    Ok(TraceQueueDiagnostics {
        queued_count,
        held_count: holds.len() as u32,
        submitted_count: credit_report.submissions_submitted,
        revoked_count: credit_report.submissions_revoked,
        expired_count: credit_report.submissions_expired,
        last_submission_at: credit_report.last_submission_at,
        last_credit_sync_at: credit_report.last_credit_sync_at,
        held_reason_counts,
        policy_enabled: policy.enabled,
        endpoint_configured,
        ready_to_flush: policy.enabled && endpoint_configured && queued_count > 0,
    })
}

pub fn load_trace_envelope(path: &Path) -> anyhow::Result<TraceContributionEnvelope> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read envelope {}: {}", path.display(), e))?;
    serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("failed to parse envelope {}: {}", path.display(), e))
}

pub fn apply_credit_estimate_to_envelope(envelope: &mut TraceContributionEnvelope) {
    let estimate = estimate_initial_credit(envelope);
    envelope.value.submission_score = estimate.submission_score;
    envelope.value.credit_points_pending = estimate.credit_points_pending;
    envelope.value.explanation = estimate.explanation;
    envelope.value_card.scorecard = estimate.scorecard;
    envelope.value_card.user_visible_explanation = envelope.value.explanation.clone();
}

pub async fn submit_trace_envelope_to_endpoint(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    bearer_token_env: &str,
) -> anyhow::Result<TraceSubmissionReceipt> {
    let token = std::env::var(bearer_token_env).map_err(|_| {
        anyhow::anyhow!(
            "{} is not set; refusing to submit without explicit API credentials",
            bearer_token_env
        )
    })?;

    let response = reqwest::Client::new()
        .post(endpoint)
        .bearer_auth(token)
        .header("Idempotency-Key", envelope.submission_id.to_string())
        .json(envelope)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("trace submission request failed: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("trace submission rejected by {}: {}", status, body);
    }

    Ok(
        parse_trace_submission_receipt(&body).unwrap_or_else(|| TraceSubmissionReceipt {
            status: "submitted".to_string(),
            credit_points_pending: Some(envelope.value.credit_points_pending),
            credit_points_final: None,
            explanation: envelope.value.explanation.clone(),
        }),
    )
}

pub fn record_submitted_trace_envelope_for_scope(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    receipt: TraceSubmissionReceipt,
) -> anyhow::Result<()> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    record_submitted_trace_envelope_for_scope_unlocked(scope, envelope, endpoint, receipt)
}

fn record_submitted_trace_envelope_for_scope_unlocked(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    receipt: TraceSubmissionReceipt,
) -> anyhow::Result<()> {
    let credit_points_pending = receipt
        .credit_points_pending
        .unwrap_or(envelope.value.credit_points_pending);
    let credit_points_final = receipt.credit_points_final;
    let credit_explanation = if receipt.explanation.is_empty() {
        envelope.value.explanation.clone()
    } else {
        receipt.explanation
    };

    upsert_local_trace_record_for_scope(
        scope,
        LocalTraceSubmissionRecord {
            submission_id: envelope.submission_id,
            trace_id: envelope.trace_id,
            endpoint: Some(endpoint.to_string()),
            status: LocalTraceSubmissionStatus::Submitted,
            server_status: Some(receipt.status),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: format!("{:?}", envelope.privacy.residual_pii_risk),
            redaction_counts: envelope.privacy.redaction_counts.clone(),
            credit_points_pending,
            credit_points_final,
            credit_explanation,
            credit_events: vec![TraceCreditEvent {
                event_id: Uuid::new_v4(),
                submission_id: envelope.submission_id,
                contributor_pseudonym: envelope
                    .contributor
                    .pseudonymous_contributor_id
                    .clone()
                    .unwrap_or_else(|| "anonymous".to_string()),
                kind: TraceCreditEventKind::Accepted,
                points_delta: credit_points_pending,
                reason: "Accepted for private Trace Commons processing; delayed utility credit may be added later.".to_string(),
                created_at: Utc::now(),
            }],
            last_credit_notice_at: None,
        },
    )
}

pub async fn flush_trace_contribution_queue_for_scope(
    scope: Option<&str>,
    limit: usize,
) -> anyhow::Result<TraceQueueFlushReport> {
    let _guard = lock_trace_scope_for_mutation(scope).await;
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled {
        anyhow::bail!("trace contribution opt-in is disabled");
    }
    let endpoint = policy
        .ingestion_endpoint
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("trace contribution endpoint is not configured"))?;

    let mut submitted = 0usize;
    let mut holds = Vec::new();
    for path in queued_trace_envelope_paths_for_scope(scope)?
        .into_iter()
        .take(limit)
    {
        let mut envelope = load_trace_envelope(&path)?;
        apply_credit_estimate_to_envelope(&mut envelope);

        match trace_autonomous_eligibility(&envelope, &policy) {
            TraceQueueEligibility::Submit => {
                let receipt = match submit_trace_envelope_to_endpoint(
                    &envelope,
                    endpoint,
                    &policy.bearer_token_env,
                )
                .await
                {
                    Ok(receipt) => receipt,
                    Err(error) => {
                        let reason = sanitized_trace_submission_failure_reason(&error);
                        if let Err(hold_error) = write_trace_queue_hold_reason(&path, &reason) {
                            tracing::debug!(
                                error = %hold_error,
                                submission_id = %envelope.submission_id,
                                "Failed to write retry hold reason for trace submission"
                            );
                        }
                        holds.push(TraceQueueHold {
                            submission_id: envelope.submission_id,
                            reason,
                        });
                        continue;
                    }
                };
                record_submitted_trace_envelope_for_scope_unlocked(
                    scope, &envelope, endpoint, receipt,
                )?;
                std::fs::remove_file(&path).map_err(|e| {
                    anyhow::anyhow!("failed to remove queued envelope {}: {}", path.display(), e)
                })?;
                submitted += 1;
            }
            TraceQueueEligibility::Hold { reason } => {
                write_trace_queue_hold_reason(&path, &reason)?;
                holds.push(TraceQueueHold {
                    submission_id: envelope.submission_id,
                    reason,
                });
            }
        }
    }

    // Flush keeps the scoped lock through submission and status-sync network calls
    // so another same-scope flush cannot submit or remove the same queue file.
    if let Err(error) = sync_remote_trace_submission_records_for_scope_unlocked(scope).await {
        tracing::debug!(%error, "Failed to sync remote Trace Commons credit status");
    }

    let credit_notice =
        mark_trace_credit_noticed_if_due_unlocked(scope, policy.credit_notice_interval_hours)?;
    Ok(TraceQueueFlushReport {
        submitted,
        held: holds.len(),
        holds,
        credit_notice,
    })
}

pub async fn sync_remote_trace_submission_records_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<usize> {
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled {
        return Ok(0);
    }
    let Some(endpoint) = policy.ingestion_endpoint.as_deref() else {
        return Ok(0);
    };

    let submission_ids = {
        let _guard = lock_trace_scope_for_mutation(scope).await;
        let records = read_local_trace_records_for_scope(scope)?;
        records
            .iter()
            .filter(|record| record.status == LocalTraceSubmissionStatus::Submitted)
            .map(|record| record.submission_id)
            .collect::<Vec<_>>()
    };
    if submission_ids.is_empty() {
        return Ok(0);
    }

    let status_endpoint = trace_submission_status_endpoint(endpoint)?;
    let updates = fetch_trace_submission_statuses(
        &status_endpoint,
        &policy.bearer_token_env,
        &submission_ids,
    )
    .await?;
    let _guard = lock_trace_scope_for_mutation(scope).await;
    apply_remote_trace_submission_statuses_for_scope_unlocked(scope, &updates)
}

async fn sync_remote_trace_submission_records_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<usize> {
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled {
        return Ok(0);
    }
    let Some(endpoint) = policy.ingestion_endpoint.as_deref() else {
        return Ok(0);
    };

    let records = read_local_trace_records_for_scope(scope)?;
    let submission_ids = records
        .iter()
        .filter(|record| record.status == LocalTraceSubmissionStatus::Submitted)
        .map(|record| record.submission_id)
        .collect::<Vec<_>>();
    if submission_ids.is_empty() {
        return Ok(0);
    }

    let status_endpoint = trace_submission_status_endpoint(endpoint)?;
    let updates = fetch_trace_submission_statuses(
        &status_endpoint,
        &policy.bearer_token_env,
        &submission_ids,
    )
    .await?;
    apply_remote_trace_submission_statuses_for_scope_unlocked(scope, &updates)
}

pub fn trace_submission_status_endpoint(submission_endpoint: &str) -> anyhow::Result<String> {
    let mut url = reqwest::Url::parse(submission_endpoint).map_err(|e| {
        anyhow::anyhow!(
            "invalid trace contribution endpoint {}: {}",
            submission_endpoint,
            e
        )
    })?;
    let path = url.path().trim_end_matches('/');
    let replacement = if let Some(prefix) = path.strip_suffix("/v1/traces") {
        format!(
            "{}/v1/contributors/me/submission-status",
            prefix.trim_end_matches('/')
        )
    } else if let Some(prefix) = path.strip_suffix("/traces") {
        format!(
            "{}/contributors/me/submission-status",
            prefix.trim_end_matches('/')
        )
    } else {
        format!(
            "{}/v1/contributors/me/submission-status",
            path.trim_end_matches('/')
        )
    };
    url.set_path(if replacement.starts_with('/') {
        &replacement
    } else {
        "/v1/contributors/me/submission-status"
    });
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

pub async fn fetch_trace_submission_statuses(
    status_endpoint: &str,
    bearer_token_env: &str,
    submission_ids: &[Uuid],
) -> anyhow::Result<Vec<TraceSubmissionStatusUpdate>> {
    let token = std::env::var(bearer_token_env).map_err(|_| {
        anyhow::anyhow!(
            "{} is not set; refusing to sync Trace Commons credit without credentials",
            bearer_token_env
        )
    })?;
    let client = reqwest::Client::new();
    let mut updates = Vec::new();

    for chunk in submission_ids.chunks(200) {
        let response = client
            .post(status_endpoint)
            .bearer_auth(&token)
            .json(&TraceSubmissionStatusRequest {
                submission_ids: chunk.to_vec(),
            })
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("trace status sync request failed: {}", e))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("trace status sync rejected by {}: {}", status, body);
        }

        let mut page: Vec<TraceSubmissionStatusUpdate> = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("failed to parse trace status sync response: {}", e))?;
        updates.append(&mut page);
    }

    Ok(updates)
}

pub fn apply_remote_trace_submission_statuses_for_scope(
    scope: Option<&str>,
    updates: &[TraceSubmissionStatusUpdate],
) -> anyhow::Result<usize> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    apply_remote_trace_submission_statuses_for_scope_unlocked(scope, updates)
}

fn apply_remote_trace_submission_statuses_for_scope_unlocked(
    scope: Option<&str>,
    updates: &[TraceSubmissionStatusUpdate],
) -> anyhow::Result<usize> {
    if updates.is_empty() {
        return Ok(0);
    }

    let mut records = read_local_trace_records_for_scope(scope)?;
    let mut changed = 0usize;
    let now = Utc::now();
    for update in updates {
        let Some(record) = records
            .iter_mut()
            .find(|record| record.submission_id == update.submission_id)
        else {
            continue;
        };

        let old_effective_credit = record
            .credit_points_final
            .unwrap_or(record.credit_points_pending);
        let new_effective_credit = update
            .credit_points_total
            .or(update.credit_points_final)
            .unwrap_or(update.credit_points_pending);
        let new_stored_final = update.credit_points_total.or(update.credit_points_final);
        let mut explanation = update.explanation.clone();
        explanation.extend(update.delayed_credit_explanations.clone());
        let credit_changed = (old_effective_credit - new_effective_credit).abs() > f32::EPSILON;
        let explanation_changed =
            !explanation.is_empty() && record.credit_explanation != explanation;

        let status_changed = record.server_status.as_deref() != Some(update.status.as_str());

        record.trace_id = update.trace_id;
        record.server_status = Some(update.status.clone());
        record.credit_points_pending = update.credit_points_pending;
        record.credit_points_final = new_stored_final;
        if !explanation.is_empty() {
            record.credit_explanation = explanation;
        }
        if update.status == "revoked" {
            record.status = LocalTraceSubmissionStatus::Revoked;
            record.revoked_at.get_or_insert(now);
        } else if update.status == "expired" {
            record.status = LocalTraceSubmissionStatus::Expired;
        } else if update.status == "purged" {
            record.status = LocalTraceSubmissionStatus::Purged;
        }

        if status_changed || credit_changed || explanation_changed {
            record.last_credit_notice_at = None;
            let sync_reason = if update.credit_points_ledger.abs() > f32::EPSILON {
                format!(
                    "Server status synced as {}; delayed ledger credit now {:+.2}.",
                    update.status, update.credit_points_ledger
                )
            } else {
                format!("Server status synced as {}.", update.status)
            };
            record.credit_events.push(TraceCreditEvent {
                event_id: Uuid::new_v4(),
                submission_id: update.submission_id,
                contributor_pseudonym: "local-sync".to_string(),
                kind: TraceCreditEventKind::CreditSynced,
                points_delta: new_effective_credit - old_effective_credit,
                reason: sync_reason,
                created_at: now,
            });
            changed += 1;
        }
    }

    if changed > 0 {
        write_local_trace_records_for_scope(scope, &records)?;
    }
    Ok(changed)
}

pub fn read_local_trace_records_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Vec<LocalTraceSubmissionRecord>> {
    let path = trace_records_path(scope);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "failed to read local trace submission records {}: {}",
            path.display(),
            e
        )
    })?;
    let records = serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse local trace submission records {}: {}",
            path.display(),
            e
        )
    })?;
    Ok(records)
}

pub fn trace_credit_summary(records: &[LocalTraceSubmissionRecord]) -> CreditSummary {
    let report = trace_credit_report(records);
    CreditSummary {
        submissions_total: report.submissions_total,
        submissions_submitted: report.submissions_submitted,
        submissions_revoked: report.submissions_revoked,
        submissions_expired: report.submissions_expired,
        pending_credit: report.pending_credit,
        final_credit: report.final_credit,
        delayed_credit_delta: report.delayed_credit_delta,
        credit_events_total: report.credit_events_total,
        recent_explanations: recent_trace_credit_explanations(records, 6),
    }
}

pub fn trace_credit_report(records: &[LocalTraceSubmissionRecord]) -> TraceCreditReport {
    let submissions_submitted = records
        .iter()
        .filter(|record| record.status == LocalTraceSubmissionStatus::Submitted)
        .count() as u32;
    let submissions_revoked = records
        .iter()
        .filter(|record| record.status == LocalTraceSubmissionStatus::Revoked)
        .count() as u32;
    let submissions_expired = records
        .iter()
        .filter(|record| {
            matches!(
                record.status,
                LocalTraceSubmissionStatus::Expired | LocalTraceSubmissionStatus::Purged
            )
        })
        .count() as u32;

    let submissions_accepted = records
        .iter()
        .filter(|record| local_trace_server_status_matches(record, "accepted"))
        .count() as u32;
    let submissions_quarantined = records
        .iter()
        .filter(|record| local_trace_server_status_matches(record, "quarantined"))
        .count() as u32;
    let submissions_rejected = records
        .iter()
        .filter(|record| local_trace_server_status_matches(record, "rejected"))
        .count() as u32;

    let pending_credit = records
        .iter()
        .map(|record| record.credit_points_pending)
        .sum();
    let final_credit = records
        .iter()
        .filter_map(|record| record.credit_points_final)
        .sum();
    let credit_events_total = records
        .iter()
        .map(|record| record.credit_events.len() as u32)
        .sum();
    let delayed_credit_delta = records
        .iter()
        .flat_map(|record| record.credit_events.iter())
        .filter(|event| event.kind != TraceCreditEventKind::Accepted)
        .map(|event| event.points_delta)
        .sum();
    let last_submission_at = records
        .iter()
        .filter_map(|record| record.submitted_at)
        .max();
    let last_credit_sync_at = records
        .iter()
        .flat_map(|record| record.credit_events.iter())
        .filter(|event| event.kind == TraceCreditEventKind::CreditSynced)
        .map(|event| event.created_at)
        .max();

    let explanation_lines = trace_credit_report_explanation_lines(
        records,
        submissions_accepted,
        submissions_quarantined,
        submissions_rejected,
        pending_credit,
        final_credit,
        delayed_credit_delta,
    );

    TraceCreditReport {
        submissions_total: records.len() as u32,
        submissions_submitted,
        submissions_revoked,
        submissions_expired,
        submissions_accepted,
        submissions_quarantined,
        submissions_rejected,
        pending_credit,
        final_credit,
        credit_events_total,
        delayed_credit_delta,
        last_submission_at,
        last_credit_sync_at,
        explanation_lines,
    }
}

fn local_trace_server_status_matches(record: &LocalTraceSubmissionRecord, expected: &str) -> bool {
    record
        .server_status
        .as_deref()
        .map(|status| status.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn trace_credit_report_explanation_lines(
    records: &[LocalTraceSubmissionRecord],
    submissions_accepted: u32,
    submissions_quarantined: u32,
    submissions_rejected: u32,
    pending_credit: f32,
    final_credit: f32,
    delayed_credit_delta: f32,
) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "{} submitted trace(s): {} accepted, {} quarantined, {} rejected.",
        records.len(),
        submissions_accepted,
        submissions_quarantined,
        submissions_rejected
    ));
    lines.push(format!(
        "Credit totals: pending +{:.2}, final confirmed +{:.2}.",
        pending_credit, final_credit
    ));
    if delayed_credit_delta.abs() > f32::EPSILON {
        lines.push(format!(
            "Delayed ledger adjustments currently total {:+.2}.",
            delayed_credit_delta
        ));
    }
    lines.extend(recent_trace_credit_explanations(records, 6));
    lines
}

fn recent_trace_credit_explanations(
    records: &[LocalTraceSubmissionRecord],
    limit: usize,
) -> Vec<String> {
    records
        .iter()
        .rev()
        .flat_map(|record| record.credit_explanation.iter().cloned())
        .take(limit)
        .collect()
}

pub async fn revoke_trace_submission_for_scope(
    scope: Option<&str>,
    submission_id: Uuid,
    endpoint: Option<&str>,
    bearer_token_env: &str,
) -> anyhow::Result<()> {
    if let Some(endpoint) = endpoint {
        let token = std::env::var(bearer_token_env).map_err(|_| {
            anyhow::anyhow!(
                "{} is not set; refusing to call revocation API without credentials",
                bearer_token_env
            )
        })?;
        let response = reqwest::Client::new()
            .delete(endpoint)
            .bearer_auth(token)
            .json(&serde_json::json!({ "submission_id": submission_id }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("trace revocation request failed: {}", e))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("trace revocation rejected by {}: {}", status, body);
        }
    }

    let _guard = lock_trace_scope_for_mutation(scope).await;
    mark_local_trace_revoked_for_scope_unlocked(scope, submission_id)
}

pub fn trace_autonomous_eligibility(
    envelope: &TraceContributionEnvelope,
    policy: &StandingTraceContributionPolicy,
) -> TraceQueueEligibility {
    if policy.require_manual_approval_when_pii_detected
        && envelope.privacy.residual_pii_risk != ResidualPiiRisk::Low
    {
        return TraceQueueEligibility::Hold {
            reason: "manual review required because residual privacy risk is not low".to_string(),
        };
    }

    if !policy.selected_tools.is_empty()
        && envelope
            .replay
            .required_tools
            .iter()
            .all(|tool| !policy.selected_tools.contains(tool))
    {
        return TraceQueueEligibility::Hold {
            reason: "trace does not use any selected auto-submit tools".to_string(),
        };
    }

    if envelope.value.submission_score < policy.min_submission_score {
        return TraceQueueEligibility::Hold {
            reason: format!(
                "submission score {:.2} is below policy minimum {:.2}",
                envelope.value.submission_score, policy.min_submission_score
            ),
        };
    }

    let failed_trace = matches!(
        envelope.outcome.task_success,
        TaskSuccess::Failure | TaskSuccess::Partial
    );
    if failed_trace && policy.auto_submit_failed_traces {
        return TraceQueueEligibility::Submit;
    }
    if policy.auto_submit_high_value_traces {
        return TraceQueueEligibility::Submit;
    }

    TraceQueueEligibility::Hold {
        reason: "policy does not allow this autonomous submission class".to_string(),
    }
}

fn parse_trace_submission_receipt(body: &str) -> Option<TraceSubmissionReceipt> {
    if body.trim().is_empty() {
        return None;
    }
    serde_json::from_str(body).ok()
}

fn upsert_local_trace_record_for_scope(
    scope: Option<&str>,
    record: LocalTraceSubmissionRecord,
) -> anyhow::Result<()> {
    let mut records = read_local_trace_records_for_scope(scope)?;
    if let Some(existing) = records
        .iter_mut()
        .find(|existing| existing.submission_id == record.submission_id)
    {
        *existing = record;
    } else {
        records.push(record);
    }
    write_local_trace_records_for_scope(scope, &records)
}

fn mark_local_trace_revoked_for_scope_unlocked(
    scope: Option<&str>,
    submission_id: Uuid,
) -> anyhow::Result<()> {
    let mut records = read_local_trace_records_for_scope(scope)?;
    let now = Utc::now();
    let mut found = false;
    for record in &mut records {
        if record.submission_id == submission_id {
            record.status = LocalTraceSubmissionStatus::Revoked;
            record.revoked_at = Some(now);
            found = true;
        }
    }
    if !found {
        records.push(LocalTraceSubmissionRecord {
            submission_id,
            trace_id: Uuid::nil(),
            endpoint: None,
            status: LocalTraceSubmissionStatus::Revoked,
            server_status: None,
            submitted_at: None,
            revoked_at: Some(now),
            privacy_risk: "unknown".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 0.0,
            credit_points_final: None,
            credit_explanation: Vec::new(),
            credit_events: Vec::new(),
            last_credit_notice_at: None,
        });
    }
    write_local_trace_records_for_scope(scope, &records)
}

#[cfg(test)]
fn mark_trace_credit_noticed_if_due(
    scope: Option<&str>,
    interval_hours: u32,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    mark_trace_credit_noticed_if_due_unlocked(scope, interval_hours)
}

fn mark_trace_credit_noticed_if_due_unlocked(
    scope: Option<&str>,
    interval_hours: u32,
) -> anyhow::Result<Option<CreditSummary>> {
    if interval_hours == 0 {
        return Ok(None);
    }

    let mut records = read_local_trace_records_for_scope(scope)?;
    if records
        .iter()
        .all(|record| !trace_record_noticeable(record))
    {
        return Ok(None);
    }

    let now = Utc::now();
    let interval = chrono::Duration::hours(i64::from(interval_hours));
    let notice_due = records
        .iter()
        .filter(|record| trace_record_noticeable(record))
        .any(|record| {
            record
                .last_credit_notice_at
                .map(|last_notice| now.signed_duration_since(last_notice) >= interval)
                .unwrap_or(true)
        });

    if !notice_due {
        return Ok(None);
    }

    let summary = trace_credit_summary(&records);
    for record in &mut records {
        if trace_record_noticeable(record) {
            record.last_credit_notice_at = Some(now);
        }
    }
    write_local_trace_records_for_scope(scope, &records)?;
    Ok(Some(summary))
}

fn sanitized_trace_submission_failure_reason(error: &anyhow::Error) -> String {
    let mut hasher = Sha256::new();
    hasher.update(error.to_string().as_bytes());
    let digest = hasher.finalize();
    format!(
        "submission failed; retained for retry (error_hash=sha256:{})",
        hex::encode(&digest[..8])
    )
}

fn trace_record_noticeable(record: &LocalTraceSubmissionRecord) -> bool {
    record.status == LocalTraceSubmissionStatus::Submitted || !record.credit_events.is_empty()
}

fn write_local_trace_records_for_scope(
    scope: Option<&str>,
    records: &[LocalTraceSubmissionRecord],
) -> anyhow::Result<()> {
    write_json_file(
        &trace_records_path(scope),
        records,
        "local trace submission records",
    )
}

fn write_trace_queue_hold_reason(path: &Path, reason: &str) -> anyhow::Result<()> {
    let hold_path = path.with_extension("held.json");
    let body = serde_json::json!({
        "envelope": path.file_name().and_then(|name| name.to_str()).unwrap_or("unknown"),
        "held_at": Utc::now(),
        "reason": safe_trace_queue_hold_reason(reason),
    });
    write_json_file(&hold_path, &body, "trace queue hold reason")
}

fn trace_queue_hold_submission_id(path: &Path) -> Option<Uuid> {
    let file_name = path.file_name()?.to_str()?;
    let raw = file_name.strip_suffix(".held.json")?;
    Uuid::parse_str(raw).ok()
}

fn safe_trace_queue_hold_reason(reason: &str) -> String {
    let normalized = reason
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if normalized.is_empty() {
        return "held".to_string();
    }
    let (redacted, _) = DeterministicTraceRedactor::default().redact_text(&normalized);
    let redacted = trace_queue_secret_like_reason_regex().replace_all(&redacted, "[REDACTED]");
    let redacted = redacted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if redacted.is_empty() {
        return "held".to_string();
    }
    redacted.chars().take(240).collect()
}

fn trace_policy_path(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("policy.json")
}

fn trace_queue_dir(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("queue")
}

fn trace_records_path(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("submissions.json")
}

fn write_json_file<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    label: &str,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!(
                "failed to create {} directory {}: {}",
                label,
                parent.display(),
                e
            )
        })?;
    }
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| anyhow::anyhow!("failed to serialize {}: {}", label, e))?;
    std::fs::write(path, body)
        .map_err(|e| anyhow::anyhow!("failed to write {} {}: {}", label, path.display(), e))
}

fn scope_hash(scope: &str) -> String {
    let digest = Sha256::digest(scope.as_bytes());
    hex::encode(&digest[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::recording::{TraceStep, TraceToolCall};

    #[test]
    fn trace_policy_preflight_gates_queue_and_submit_intents() {
        let disabled = StandingTraceContributionPolicy::default();
        assert_eq!(
            preflight_trace_contribution_policy(
                &disabled,
                TraceContributionAcceptance::PreviewOnly
            ),
            Ok(())
        );
        assert_eq!(
            preflight_trace_contribution_policy(
                &disabled,
                TraceContributionAcceptance::QueueFromPreview
            ),
            Err(TraceContributionPolicyRejection::OptInDisabled)
        );
        assert_eq!(
            preflight_trace_contribution_policy(
                &disabled,
                TraceContributionAcceptance::ManualSubmit
            ),
            Err(TraceContributionPolicyRejection::OptInDisabled)
        );
        assert_eq!(
            preflight_trace_contribution_policy(
                &disabled,
                TraceContributionAcceptance::AutonomousSubmit
            ),
            Err(TraceContributionPolicyRejection::OptInDisabled)
        );

        let mut missing_endpoint = StandingTraceContributionPolicy {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(
            preflight_trace_contribution_policy(
                &missing_endpoint,
                TraceContributionAcceptance::ManualSubmit
            ),
            Err(TraceContributionPolicyRejection::EndpointMissing)
        );

        missing_endpoint.ingestion_endpoint = Some("https://trace.example/v1/traces".to_string());
        assert_eq!(
            preflight_trace_contribution_policy(
                &missing_endpoint,
                TraceContributionAcceptance::ManualSubmit
            ),
            Ok(())
        );
    }

    struct FakePrivacyFilterAdapter;

    #[async_trait]
    impl PrivacyFilterAdapter for FakePrivacyFilterAdapter {
        async fn redact_text(
            &self,
            text: &str,
        ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
            if !text.contains("Alice") {
                return Ok(None);
            }
            let mut report = RedactionReport::default();
            report.increment("privacy_filter:private_person");
            report.add_pii_label("private_person");
            Ok(Some(SafePrivacyFilterRedaction {
                redacted_text: text.replace("Alice", "<PRIVATE_PERSON_1>"),
                summary: SafePrivacyFilterSummary {
                    schema_version: 1,
                    output_mode: "redacted_text_only".to_string(),
                    span_count: 1,
                    by_label: BTreeMap::from([("private_person".to_string(), 1)]),
                    decoded_mismatch: false,
                },
                report,
            }))
        }
    }

    struct CanaryPrivacyFilterAdapter;

    #[async_trait]
    impl PrivacyFilterAdapter for CanaryPrivacyFilterAdapter {
        async fn redact_text(
            &self,
            text: &str,
        ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
            let values = synthetic_privacy_filter_canary_values();
            let mut redacted = text.to_string();
            for (index, value) in values.iter().enumerate() {
                redacted = redacted.replace(value, &format!("<CANARY_REDACTED_{}>", index + 1));
            }
            let output = serde_json::json!({
                "schema_version": 1,
                "text": text,
                "redacted_text": redacted,
                "detected_spans": [
                    {"label": "private_email", "text": values[0]},
                    {"label": "secret", "text": values[1]},
                    {"label": "local_path", "text": values[2]}
                ]
            });
            safe_privacy_filter_redaction_from_output(&output).map(Some)
        }
    }

    struct FailingPrivacyFilterAdapter;

    #[async_trait]
    impl PrivacyFilterAdapter for FailingPrivacyFilterAdapter {
        async fn redact_text(
            &self,
            _text: &str,
        ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
            Err(TraceContributionError::RedactionFailed {
                reason: "sidecar stderr mentioned tc_canary_secret_0123456789abcdef".to_string(),
            })
        }
    }

    struct EnvVarRestore {
        name: &'static str,
        previous: Option<String>,
    }

    impl EnvVarRestore {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var(name).ok();
            // SAFETY: This test-scoped guard restores the variable in Drop.
            // The sidecar isolation test needs a real process environment
            // value to prove `CommandPrivacyFilterAdapter` clears child env.
            unsafe {
                std::env::set_var(name, value);
            }
            Self { name, previous }
        }
    }

    impl Drop for EnvVarRestore {
        fn drop(&mut self) {
            // SAFETY: Restoring the exact test-scoped variable keeps process
            // environment mutation bounded to this guard's lifetime.
            unsafe {
                if let Some(previous) = self.previous.as_ref() {
                    std::env::set_var(self.name, previous);
                } else {
                    std::env::remove_var(self.name);
                }
            }
        }
    }

    fn sample_trace() -> TraceFile {
        TraceFile {
            model_name: "test-model".to_string(),
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            steps: vec![
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::UserInput {
                        content: "Email alice@example.com about /Users/alice/project/secrets.txt"
                            .to_string(),
                    },
                    expected_tool_results: Vec::new(),
                },
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::ToolCalls {
                        tool_calls: vec![TraceToolCall {
                            id: "call_1".to_string(),
                            name: "http".to_string(),
                            arguments: serde_json::json!({
                                "url": "https://api.example.com",
                                "Authorization": "Bearer abcdefghijklmnopqrstuvwxyz123456",
                                "path": "/Users/alice/project/secrets.txt"
                            }),
                        }],
                        input_tokens: 10,
                        output_tokens: 3,
                    },
                    expected_tool_results: Vec::new(),
                },
            ],
        }
    }

    #[tokio::test]
    async fn metadata_only_recorded_trace_omits_message_text_and_tool_arguments() {
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
            "/Users/alice/project",
        )])
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

        let json = serde_json::to_string(&envelope).expect("envelope serializes");
        assert!(!json.contains("alice@example.com"));
        assert!(!json.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(!json.contains("/Users/alice/project"));
        assert!(json.contains("\"tool_name\":\"http\""));
        assert!(!envelope.consent.message_text_included);
        assert!(!envelope.consent.tool_payloads_included);
        assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Low);
    }

    #[tokio::test]
    async fn text_and_payload_preview_redacts_paths_and_sensitive_fields() {
        let options = RecordedTraceContributionOptions {
            include_message_text: true,
            include_tool_payloads: true,
            ..Default::default()
        };
        let raw = RawTraceContribution::from_recorded_trace(&sample_trace(), options);
        let envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
            "/Users/alice/project",
        )])
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

        let json = serde_json::to_string(&envelope).expect("envelope serializes");
        assert!(json.contains("<PRIVATE_LOCAL_PATH_"));
        assert!(json.contains("<PRIVATE_EMAIL_"));
        assert!(json.contains("[REDACTED]"));
        assert!(!json.contains("/Users/alice/project"));
        assert!(!json.contains("alice@example.com"));
        assert!(!json.contains("abcdefghijklmnopqrstuvwxyz"));
        assert_eq!(
            envelope.privacy.redaction_counts.get("local_path"),
            Some(&2)
        );
        assert_eq!(
            envelope.privacy.redaction_counts.get("sensitive_field"),
            Some(&1)
        );
        assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Medium);
    }

    #[test]
    fn deterministic_text_redactor_redacts_generic_local_paths() {
        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let (redacted, report) =
            redactor.redact_text("read /tmp/ironclaw/private/token.txt before upload");

        assert_eq!(redacted, "read <PRIVATE_LOCAL_PATH_1> before upload");
        assert_eq!(report.counts.get("local_path"), Some(&1));
    }

    #[test]
    fn stable_placeholders_preserve_entity_distinctions() {
        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let (redacted, report) = redactor.redact_text(
            "Email alice@example.com, copy bob@example.com, then follow up with alice@example.com.",
        );

        assert!(redacted.contains("<PRIVATE_EMAIL_1>"));
        assert!(redacted.contains("<PRIVATE_EMAIL_2>"));
        assert_eq!(redacted.matches("<PRIVATE_EMAIL_1>").count(), 2);
        assert_eq!(redacted.matches("<PRIVATE_EMAIL_2>").count(), 1);
        assert!(!redacted.contains("alice@example.com"));
        assert!(!redacted.contains("bob@example.com"));
        assert_eq!(report.counts.get("private_email"), Some(&3));
        assert!(
            report
                .pii_labels_present
                .contains(&"private_email".to_string())
        );
    }

    #[test]
    fn privacy_filter_summary_shape_cannot_serialize_original_span_text() {
        let summary = SafePrivacyFilterSummary {
            schema_version: 1,
            output_mode: "redacted_text_only".to_string(),
            span_count: 2,
            by_label: BTreeMap::from([("private_email".to_string(), 2)]),
            decoded_mismatch: false,
        };

        let json = serde_json::to_string(&summary).expect("summary serializes");
        assert!(json.contains("private_email"));
        assert!(!json.contains("alice@example.com"));
        assert!(!json.contains("detected_spans"));
        assert!(!json.contains("\"text\""));
    }

    #[test]
    fn privacy_filter_output_adapter_strips_raw_span_text() {
        let output = serde_json::json!({
            "schema_version": 1,
            "text": "Email alice@example.com with secret sk-test",
            "redacted_text": "Email <PRIVATE_EMAIL> with <SECRET>",
            "detected_spans": [
                {"label": "private_email", "start": 6, "end": 23, "text": "alice@example.com"},
                {"label": "secret", "start": 36, "end": 43, "text": "sk-test"}
            ]
        });

        let safe =
            safe_privacy_filter_redaction_from_output(&output).expect("privacy output parses");
        let json = serde_json::to_string(&safe).expect("safe output serializes");

        assert_eq!(safe.redacted_text, "Email <PRIVATE_EMAIL> with <SECRET>");
        assert_eq!(safe.summary.span_count, 2);
        assert_eq!(safe.summary.by_label.get("private_email"), Some(&1));
        assert!(safe.report.blocked_secret_detected);
        assert!(!json.contains("alice@example.com"));
        assert!(!json.contains("sk-test"));
        assert!(!json.contains("detected_spans"));
    }

    #[test]
    fn privacy_filter_output_adapter_maps_unsafe_labels_without_leaking_them() {
        let output = serde_json::json!({
            "schema_version": 1,
            "redacted_text": "Email <PRIVATE_EMAIL> with <SECRET>",
            "detected_spans": [
                {"label": "alice@example.com", "text": "alice@example.com"},
                {"type": "/Users/alice/.ssh/id_rsa", "text": "/Users/alice/.ssh/id_rsa"},
                {"entity_type": "sk-test-raw-token", "text": "sk-test-raw-token"}
            ]
        });

        let safe =
            safe_privacy_filter_redaction_from_output(&output).expect("privacy output parses");
        let json = serde_json::to_string(&safe).expect("safe output serializes");

        assert_eq!(safe.summary.by_label.get("unknown"), Some(&3));
        assert_eq!(safe.report.counts.get("privacy_filter:unknown"), Some(&3));
        for raw in [
            "alice@example.com",
            "/Users/alice/.ssh/id_rsa",
            "sk-test-raw-token",
        ] {
            assert!(!json.contains(raw), "safe output leaked {raw}");
        }
        assert!(safe.report.warnings.iter().any(|warning| {
            warning == "Privacy Filter sidecar emitted unsupported span label; mapped to unknown."
        }));
    }

    #[tokio::test]
    async fn privacy_filter_sidecar_summary_is_integrated_without_raw_text() {
        let trace = TraceFile {
            model_name: "test-model".to_string(),
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            steps: vec![TraceStep {
                request_hint: None,
                response: TraceResponse::UserInput {
                    content: "Alice asked for a project update".to_string(),
                },
                expected_tool_results: Vec::new(),
            }],
        };
        let raw = RawTraceContribution::from_recorded_trace(
            &trace,
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..Default::default()
            },
        );
        let envelope = DeterministicTraceRedactor::new(Vec::new())
            .with_privacy_filter(Arc::new(FakePrivacyFilterAdapter))
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");

        let json = serde_json::to_string(&envelope).expect("envelope serializes");
        assert!(json.contains(PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX));
        assert!(json.contains("<PRIVATE_PERSON_1>"));
        assert!(!json.contains("Alice asked"));
        assert_eq!(
            envelope
                .privacy
                .privacy_filter_summary
                .as_ref()
                .and_then(|summary| summary.by_label.get("private_person"))
                .copied(),
            Some(1)
        );
        assert_eq!(
            envelope
                .privacy
                .redaction_counts
                .get("privacy_filter:private_person"),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn privacy_filter_canary_report_keeps_raw_canary_values_out() {
        let report = run_privacy_filter_canary(&CanaryPrivacyFilterAdapter)
            .await
            .expect("canary should run");
        let json = serde_json::to_string(&report).expect("report serializes");

        assert!(report.healthy);
        assert_eq!(
            report
                .summary
                .as_ref()
                .and_then(|summary| summary.by_label.get("secret")),
            Some(&1)
        );
        for raw_value in synthetic_privacy_filter_canary_values() {
            assert!(!json.contains(&raw_value));
        }
        assert!(json.contains("sha256:"));
        assert!(!json.contains("tc_canary_secret_0123456789abcdef"));
    }

    #[tokio::test]
    async fn privacy_filter_sidecar_failure_falls_back_without_raw_error_text() {
        let trace = TraceFile {
            model_name: "test-model".to_string(),
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            steps: vec![TraceStep {
                request_hint: None,
                response: TraceResponse::UserInput {
                    content: "Alice asked for a status update".to_string(),
                },
                expected_tool_results: Vec::new(),
            }],
        };
        let raw = RawTraceContribution::from_recorded_trace(
            &trace,
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..Default::default()
            },
        );

        let envelope = DeterministicTraceRedactor::new(Vec::new())
            .with_privacy_filter(Arc::new(FailingPrivacyFilterAdapter))
            .redact_trace(raw)
            .await
            .expect("deterministic fallback should keep redaction non-fatal");
        let json = serde_json::to_string(&envelope).expect("envelope serializes");

        assert!(json.contains("Privacy Filter sidecar failed"));
        assert!(json.contains("sha256:"));
        assert!(!json.contains("tc_canary_secret_0123456789abcdef"));
        assert!(envelope.privacy.privacy_filter_summary.is_none());
        assert!(
            envelope
                .privacy
                .redaction_counts
                .contains_key("privacy_filter:sidecar_failure")
        );
    }

    #[tokio::test]
    async fn command_privacy_filter_error_does_not_echo_stderr() {
        if !Path::new("/bin/sh").exists() {
            return;
        }
        let adapter = CommandPrivacyFilterAdapter::new("/bin/sh").with_args([
            "-c",
            "cat >/dev/null; printf '%s' 'raw-secret-from-stderr' >&2; exit 7",
        ]);

        let error = adapter
            .redact_text("hello")
            .await
            .expect_err("non-zero sidecar exit should fail")
            .to_string();

        assert!(error.contains("stderr_len="));
        assert!(error.contains("stderr_hash="));
        assert!(!error.contains("raw-secret-from-stderr"));
    }

    #[tokio::test]
    async fn command_privacy_filter_adapter_does_not_inherit_trace_commons_tokens() {
        if !Path::new("/bin/sh").exists() {
            return;
        }
        let _env_guard =
            EnvVarRestore::set("TRACE_COMMONS_TENANT_TOKENS", "tenant-a:super-secret-token");

        let adapter = CommandPrivacyFilterAdapter::new("/bin/sh").with_args([
            "-c",
            "cat >/dev/null; printf '{\"redacted_text\":\"%s\"}' \"${TRACE_COMMONS_TENANT_TOKENS-unset}\"",
        ]);
        let redaction = adapter
            .redact_text("hello")
            .await
            .expect("sidecar should run")
            .expect("sidecar should return redaction");

        assert_eq!(redaction.redacted_text, "unset");
    }

    #[tokio::test]
    async fn command_privacy_filter_rejects_oversized_stdout() {
        if !Path::new("/bin/sh").exists() {
            return;
        }
        let adapter = CommandPrivacyFilterAdapter::new("/bin/sh")
            .with_args([
                "-c",
                "cat >/dev/null; printf '%s' '{\"redacted_text\":\"012345678901234567890123456789\"}'",
            ])
            .with_output_limits(16, 16);

        let error = adapter
            .redact_text("hello")
            .await
            .expect_err("oversized stdout should fail")
            .to_string();

        assert!(error.contains("stdout exceeded privacy filter sidecar limit"));
        assert!(!error.contains("0123456789"));
    }

    #[test]
    fn tool_specific_payload_redaction_removes_email_content_fields() {
        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let payload = serde_json::json!({
            "to": ["alice@example.com"],
            "subject": "Project launch",
            "body": "Please review /tmp/ironclaw/private.txt",
            "public_id": "message-1"
        });

        let mut state = RedactionState::default();
        let (redacted, report) =
            redactor.redact_json_value(Some("gmail_send"), &payload, &mut state);
        let json = serde_json::to_string(&redacted).expect("payload serializes");

        assert!(json.contains("[REDACTED:email_participant]"));
        assert!(json.contains("[REDACTED:email_content]"));
        assert!(json.contains("message-1"));
        assert!(!json.contains("alice@example.com"));
        assert!(!json.contains("Project launch"));
        assert!(!json.contains("/tmp/ironclaw/private.txt"));
        assert_eq!(report.counts.get("tool_sensitive_field"), Some(&3));
    }

    #[test]
    fn tool_specific_payload_redaction_preserves_browser_replay_metadata() {
        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let payload = serde_json::json!({
            "method": "GET",
            "url": "https://example.com/private/customer-123?token=secret-token#frag",
            "headers": {
                "authorization": "Bearer secret-token",
                "accept": "application/json"
            },
            "response": {
                "status": 204,
                "event_id": "evt_public_123"
            }
        });

        let mut state = RedactionState::default();
        let (redacted, report) =
            redactor.redact_json_value(Some("browser_fetch"), &payload, &mut state);
        let json = serde_json::to_string(&redacted).expect("payload serializes");

        assert_eq!(redacted["method"], "GET");
        assert_eq!(redacted["response"]["status"], 204);
        assert_eq!(redacted["response"]["event_id"], "evt_public_123");
        assert!(json.contains("https://example.com/[REDACTED_PATH]"));
        assert!(!json.contains("customer-123"));
        assert!(!json.contains("secret-token"));
        assert!(json.contains("[REDACTED:browser_header]"));
        assert_eq!(report.counts.get("tool_sensitive_field"), Some(&2));
    }

    #[test]
    fn tool_specific_payload_redaction_preserves_issue_tracker_numbers() {
        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let payload = serde_json::json!({
            "issue_number": 42,
            "number": 42,
            "state": "open",
            "status": "triaged",
            "event_id": "evt_issue_public",
            "title": "Customer Acme reported a private failure",
            "body": "Stack trace includes /Users/alice/project/secrets.txt",
            "html_url": "https://github.com/private-org/private-repo/issues/42?auth=secret",
            "assignee": "alice@example.com",
            "repository": "private-org/private-repo"
        });

        let mut state = RedactionState::default();
        let (redacted, report) =
            redactor.redact_json_value(Some("github_issue_create"), &payload, &mut state);
        let json = serde_json::to_string(&redacted).expect("payload serializes");

        assert_eq!(redacted["issue_number"], 42);
        assert_eq!(redacted["number"], 42);
        assert_eq!(redacted["state"], "open");
        assert_eq!(redacted["status"], "triaged");
        assert_eq!(redacted["event_id"], "evt_issue_public");
        assert!(json.contains("https://github.com/[REDACTED_PATH]"));
        assert!(!json.contains("Acme"));
        assert!(!json.contains("alice@example.com"));
        assert!(!json.contains("private-org/private-repo"));
        assert!(!json.contains("/Users/alice/project"));
        assert_eq!(report.counts.get("tool_sensitive_field"), Some(&5));
    }

    #[test]
    fn tool_specific_payload_redaction_covers_calendar_and_messaging_payloads() {
        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let calendar_payload = serde_json::json!({
            "event_id": "evt_calendar_public",
            "status": "confirmed",
            "summary": "Interview with Alice",
            "location": "Alice home office",
            "attendees": [{"email": "alice@example.com"}]
        });
        let slack_payload = serde_json::json!({
            "event_id": "evt_slack_public",
            "ok": true,
            "channel_id": "C123PRIVATE",
            "user_id": "U123PRIVATE",
            "text": "Alice's private launch note"
        });

        let mut state = RedactionState::default();
        let (calendar_redacted, calendar_report) = redactor.redact_json_value(
            Some("calendar_create_event"),
            &calendar_payload,
            &mut state,
        );
        let (slack_redacted, slack_report) =
            redactor.redact_json_value(Some("slack_post_message"), &slack_payload, &mut state);
        let json = serde_json::to_string(&(calendar_redacted.clone(), slack_redacted.clone()))
            .expect("payloads serialize");

        assert_eq!(calendar_redacted["event_id"], "evt_calendar_public");
        assert_eq!(calendar_redacted["status"], "confirmed");
        assert_eq!(slack_redacted["event_id"], "evt_slack_public");
        assert_eq!(slack_redacted["ok"], true);
        assert!(json.contains("[REDACTED:calendar_content]"));
        assert!(json.contains("[REDACTED:calendar_participant]"));
        assert!(json.contains("[REDACTED:message_identity]"));
        assert!(json.contains("[REDACTED:message_content]"));
        assert!(!json.contains("Alice"));
        assert!(!json.contains("alice@example.com"));
        assert!(!json.contains("C123PRIVATE"));
        assert_eq!(calendar_report.counts.get("tool_sensitive_field"), Some(&3));
        assert_eq!(slack_report.counts.get("tool_sensitive_field"), Some(&3));
    }

    #[test]
    fn tool_specific_payload_redaction_summarizes_database_rows_and_params() {
        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let payload = serde_json::json!({
            "operation": "select",
            "status_code": 200,
            "query": "select * from customers where email = $1",
            "params": ["alice@example.com"],
            "rows": [
                {"email": "alice@example.com", "token": "secret-token"},
                {"email": "bob@example.com", "token": "other-secret"}
            ]
        });

        let mut state = RedactionState::default();
        let (redacted, report) =
            redactor.redact_json_value(Some("postgres_query"), &payload, &mut state);
        let json = serde_json::to_string(&redacted).expect("payload serializes");

        assert_eq!(redacted["operation"], "select");
        assert_eq!(redacted["status_code"], 200);
        assert_eq!(redacted["params"]["count"], 1);
        assert_eq!(redacted["rows"]["count"], 2);
        assert!(json.contains("[REDACTED:database_content]"));
        assert!(json.contains("[REDACTED:database_query_param]"));
        assert!(json.contains("[REDACTED:database_row]"));
        assert!(!json.contains("alice@example.com"));
        assert!(!json.contains("secret-token"));
        assert!(!json.contains("select * from customers"));
        assert_eq!(report.counts.get("tool_sensitive_field"), Some(&3));
    }

    #[tokio::test]
    async fn canonical_summary_uses_redacted_content_only() {
        let options = RecordedTraceContributionOptions {
            include_message_text: true,
            include_tool_payloads: true,
            ..Default::default()
        };
        let raw = RawTraceContribution::from_recorded_trace(&sample_trace(), options);
        let envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
            "/Users/alice/project",
        )])
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

        let summary = canonical_summary_for_embedding(&envelope);
        assert!(summary.contains("<PRIVATE_LOCAL_PATH_"));
        assert!(!summary.contains("/Users/alice/project"));
        assert!(!summary.contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[tokio::test]
    async fn canonical_representations_use_only_redacted_private_values() {
        let mut raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions {
                include_message_text: true,
                include_tool_payloads: true,
                consent_scopes: vec![ConsentScope::ModelTraining],
                ..Default::default()
            },
        );
        raw.outcome = OutcomeMetadata {
            user_feedback: UserFeedback::Correction,
            task_success: TaskSuccess::Partial,
            failure_modes: vec![TraceFailureMode::UserIntentMisread],
            human_correction: Some(
                "Use alice@example.com and /Users/alice/project/fix.md as the correction"
                    .to_string(),
            ),
            ..OutcomeMetadata::default()
        };
        let envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
            "/Users/alice/project",
        )])
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

        let representations = canonical_representations_for_embedding(&envelope);
        let joined = representations
            .iter()
            .map(|representation| representation.content.as_str())
            .collect::<Vec<_>>()
            .join("\n---\n");

        assert!(
            representations.iter().any(
                |representation| representation.kind == CanonicalRepresentationKind::WholeTrace
            )
        );
        assert!(
            representations
                .iter()
                .any(|representation| representation.kind == CanonicalRepresentationKind::Turn)
        );
        assert!(
            representations
                .iter()
                .any(|representation| representation.kind
                    == CanonicalRepresentationKind::ToolSequence)
        );
        assert!(
            representations
                .iter()
                .any(|representation| representation.kind
                    == CanonicalRepresentationKind::ErrorOutcome)
        );
        assert!(
            representations.iter().any(
                |representation| representation.kind == CanonicalRepresentationKind::Correction
            )
        );
        assert!(joined.contains("<PRIVATE_EMAIL_"));
        assert!(joined.contains("<PRIVATE_LOCAL_PATH_"));
        assert!(!joined.contains("alice@example.com"));
        assert!(!joined.contains("/Users/alice/project"));
        assert!(!joined.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(
            representations
                .iter()
                .all(|representation| representation.canonical_hash.starts_with("sha256:"))
        );
        assert!(
            representations
                .iter()
                .all(|representation| representation.vector_key.starts_with("trace:"))
        );
    }

    #[tokio::test]
    async fn dataset_eligibility_gates_consent_revocation_and_privacy_risk() {
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions {
                consent_scopes: vec![ConsentScope::ModelTraining],
                ..Default::default()
            },
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");

        let eligible = trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, false);
        assert!(eligible.eligible);
        assert_eq!(
            eligible.retention_policy.class,
            TraceRetentionClass::TrainingRevocable
        );

        let revoked = trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, true);
        assert!(!revoked.eligible);
        assert!(
            revoked
                .reasons
                .iter()
                .any(|reason| reason.contains("revoked"))
        );

        let outside_scope =
            trace_dataset_eligibility(&envelope, TraceAllowedUse::BenchmarkGeneration, false);
        assert!(!outside_scope.eligible);
        assert!(
            outside_scope
                .reasons
                .iter()
                .any(|reason| reason.contains("outside consent"))
        );

        envelope.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
        let medium_training =
            trace_dataset_eligibility(&envelope, TraceAllowedUse::ModelTraining, false);
        assert!(!medium_training.eligible);
        assert!(
            medium_training
                .reasons
                .iter()
                .any(|reason| reason.contains("medium residual privacy risk"))
        );

        envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
        let high_eval = trace_dataset_eligibility(&envelope, TraceAllowedUse::Evaluation, false);
        assert!(!high_eval.eligible);
        assert!(
            high_eval
                .reasons
                .iter()
                .any(|reason| reason.contains("high residual privacy risk"))
        );
    }

    #[tokio::test]
    async fn derived_artifact_invalidation_marker_uses_hashes_not_raw_handles() {
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        let marker = derived_artifact_invalidation_marker(&envelope, "user revoked consent");
        let json = serde_json::to_string(&marker).expect("marker serializes");

        assert_eq!(marker.submission_id, envelope.submission_id);
        assert!(marker.revocation_handle_hash.starts_with("sha256:"));
        assert!(!json.contains(&envelope.contributor.revocation_handle.to_string()));
        assert!(
            marker
                .artifact_prefixes
                .contains(&format!("embedding:{}", envelope.trace_id))
        );
    }

    #[test]
    fn capture_turns_reconstructs_tool_calls_from_conversation_messages() {
        let now = Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Please inspect the build".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "tool_calls".to_string(),
                content: serde_json::json!({
                    "calls": [{
                        "name": "shell",
                        "result_preview": "build succeeded",
                        "rationale": "run the project check"
                    }]
                })
                .to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "The build is clean.".to_string(),
                created_at: now,
            },
        ];

        let turns = capture_turns_from_conversation_messages(&messages);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_input, "Please inspect the build");
        assert_eq!(turns[0].response.as_deref(), Some("The build is clean."));
        assert_eq!(turns[0].tool_calls.len(), 1);
        assert_eq!(turns[0].tool_calls[0].name, "shell");
        assert_eq!(
            turns[0].tool_calls[0].result_preview.as_deref(),
            Some("build succeeded")
        );
    }

    #[test]
    fn scoped_trace_state_uses_hashed_isolated_paths_and_refs() {
        let alice = trace_contribution_dir_for_scope(Some("tenant-a:user-alice"));
        let bob = trace_contribution_dir_for_scope(Some("tenant-b:user-bob"));
        let alice_path = alice.to_string_lossy();

        assert_ne!(alice, bob);
        assert!(!alice_path.contains("tenant-a"));
        assert!(!alice_path.contains("user-alice"));
        assert_eq!(
            local_pseudonymous_contributor_id("tenant-a:user-alice"),
            local_pseudonymous_contributor_id("tenant-a:user-alice")
        );
        assert_ne!(
            local_pseudonymous_contributor_id("tenant-a:user-alice"),
            local_pseudonymous_contributor_id("tenant-b:user-bob")
        );
        assert!(local_pseudonymous_tenant_scope_ref("tenant-a").starts_with("tenant_sha256:"));
    }

    #[tokio::test]
    async fn trace_scope_flushes_serialize_same_scope_without_blocking_other_scopes() {
        let scope = format!("trace-lock-test-{}", Uuid::new_v4());
        let other_scope = format!("trace-lock-other-test-{}", Uuid::new_v4());
        let first_guard = lock_trace_scope_for_mutation(Some(&scope)).await;

        let same_scope = scope.clone();
        let mut same_scope_waiter = Box::pin(tokio::spawn(async move {
            flush_trace_contribution_queue_for_scope(Some(&same_scope), 1).await
        }));

        let other_scope_waiter = tokio::spawn(async move {
            flush_trace_contribution_queue_for_scope(Some(&other_scope), 1).await
        });

        let other_scope_result =
            tokio::time::timeout(Duration::from_millis(200), other_scope_waiter)
                .await
                .expect("different scope should not be blocked")
                .expect("different scope waiter should complete");
        assert!(
            other_scope_result
                .expect_err("default disabled policy should make flush exit")
                .to_string()
                .contains("opt-in is disabled")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), same_scope_waiter.as_mut())
                .await
                .is_err(),
            "same scope waiter should remain serialized behind the first guard"
        );

        drop(first_guard);
        let same_scope_result =
            tokio::time::timeout(Duration::from_millis(200), same_scope_waiter.as_mut())
                .await
                .expect("same scope waiter should complete after the first guard is dropped")
                .expect("same scope waiter should not panic");
        assert!(
            same_scope_result
                .expect_err("default disabled policy should make flush exit")
                .to_string()
                .contains("opt-in is disabled")
        );
    }

    #[test]
    fn status_sync_endpoint_is_derived_from_submission_endpoint() {
        assert_eq!(
            trace_submission_status_endpoint("https://trace.example.com/v1/traces")
                .expect("endpoint parses"),
            "https://trace.example.com/v1/contributors/me/submission-status"
        );
        assert_eq!(
            trace_submission_status_endpoint("https://trace.example.com/api/v1/traces?x=1")
                .expect("endpoint parses"),
            "https://trace.example.com/api/v1/contributors/me/submission-status"
        );
    }

    fn submitted_credit_record(
        credit_points_pending: f32,
        credit_points_final: Option<f32>,
        last_credit_notice_at: Option<DateTime<Utc>>,
        credit_explanation: Vec<String>,
    ) -> LocalTraceSubmissionRecord {
        LocalTraceSubmissionRecord {
            submission_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: LocalTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending,
            credit_points_final,
            credit_explanation,
            credit_events: Vec::new(),
            last_credit_notice_at,
        }
    }

    #[test]
    fn queue_diagnostics_are_scoped_to_one_user_queue_and_records() {
        let scope_a = format!("trace-queue-diagnostics-a-{}", Uuid::new_v4());
        let scope_b = format!("trace-queue-diagnostics-b-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&scope_a), &policy).expect("scope a policy writes");
        write_trace_policy_for_scope(Some(&scope_b), &policy).expect("scope b policy writes");

        let queue_a = trace_queue_dir(Some(&scope_a));
        let queue_b = trace_queue_dir(Some(&scope_b));
        std::fs::create_dir_all(&queue_a).expect("scope a queue exists");
        std::fs::create_dir_all(&queue_b).expect("scope b queue exists");
        std::fs::write(queue_a.join(format!("{}.json", Uuid::new_v4())), "{}")
            .expect("scope a queued fixture writes");
        std::fs::write(queue_b.join(format!("{}.json", Uuid::new_v4())), "{}")
            .expect("scope b queued fixture writes");

        let sync_at = Utc::now();
        let mut scope_a_record = submitted_credit_record(
            1.0,
            Some(1.5),
            None,
            vec!["Accepted for scope a.".to_string()],
        );
        scope_a_record.credit_events.push(TraceCreditEvent {
            event_id: Uuid::new_v4(),
            submission_id: scope_a_record.submission_id,
            contributor_pseudonym: "local-sync".to_string(),
            kind: TraceCreditEventKind::CreditSynced,
            points_delta: 0.5,
            reason: "Server status synced as accepted.".to_string(),
            created_at: sync_at,
        });
        write_local_trace_records_for_scope(Some(&scope_a), &[scope_a_record])
            .expect("scope a records write");
        write_local_trace_records_for_scope(
            Some(&scope_b),
            &[LocalTraceSubmissionRecord {
                status: LocalTraceSubmissionStatus::Revoked,
                revoked_at: Some(Utc::now()),
                ..submitted_credit_record(
                    0.0,
                    Some(0.0),
                    None,
                    vec!["Revoked for scope b.".to_string()],
                )
            }],
        )
        .expect("scope b records write");

        let diagnostics_a =
            trace_queue_diagnostics_for_scope(Some(&scope_a)).expect("scope a diagnostics read");
        let diagnostics_b =
            trace_queue_diagnostics_for_scope(Some(&scope_b)).expect("scope b diagnostics read");

        assert_eq!(diagnostics_a.queued_count, 1);
        assert_eq!(diagnostics_a.submitted_count, 1);
        assert_eq!(diagnostics_a.revoked_count, 0);
        assert!(diagnostics_a.policy_enabled);
        assert!(diagnostics_a.endpoint_configured);
        assert!(diagnostics_a.ready_to_flush);
        assert!(diagnostics_a.last_submission_at.is_some());
        assert_eq!(diagnostics_a.last_credit_sync_at, Some(sync_at));

        assert_eq!(diagnostics_b.queued_count, 1);
        assert_eq!(diagnostics_b.submitted_count, 0);
        assert_eq!(diagnostics_b.revoked_count, 1);
        assert!(diagnostics_b.ready_to_flush);

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope_a)));
        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope_b)));
    }

    #[test]
    fn queue_diagnostics_aggregates_sanitized_hold_reasons() {
        let scope = format!("trace-queue-diagnostics-holds-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");
        let dir = trace_queue_dir(Some(&scope));
        std::fs::create_dir_all(&dir).expect("queue dir exists");

        let raw_reason =
            "manual review for alice@example.com in /Users/alice/private with sk-test-raw-token";
        for _ in 0..2 {
            let queue_path = dir.join(format!("{}.json", Uuid::new_v4()));
            std::fs::write(&queue_path, "{}").expect("queued fixture writes");
            write_trace_queue_hold_reason(&queue_path, raw_reason).expect("hold reason writes");
        }

        let diagnostics =
            trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics read");

        assert_eq!(diagnostics.queued_count, 2);
        assert_eq!(diagnostics.held_count, 2);
        assert_eq!(
            diagnostics
                .held_reason_counts
                .values()
                .copied()
                .sum::<u32>(),
            2
        );
        assert_eq!(diagnostics.held_reason_counts.len(), 1);
        let aggregated_reason = diagnostics
            .held_reason_counts
            .keys()
            .next()
            .expect("held reason is present");
        assert!(!aggregated_reason.contains("alice@example.com"));
        assert!(!aggregated_reason.contains("/Users/alice/private"));
        assert!(!aggregated_reason.contains("sk-test-raw-token"));

        let serialized = serde_json::to_string(&diagnostics).expect("diagnostics serialize");
        assert!(!serialized.contains("alice@example.com"));
        assert!(!serialized.contains("/Users/alice/private"));
        assert!(!serialized.contains("sk-test-raw-token"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn credit_notice_snapshot_returns_none_when_policy_disabled_or_interval_zero() {
        let disabled_scope = format!("trace-credit-disabled-notice-test-{}", Uuid::new_v4());
        write_local_trace_records_for_scope(
            Some(&disabled_scope),
            &[submitted_credit_record(
                1.0,
                Some(1.0),
                None,
                vec!["Accepted locally.".to_string()],
            )],
        )
        .expect("disabled scope record writes");

        let disabled_notice = mark_trace_credit_notice_due_for_scope(Some(&disabled_scope))
            .expect("disabled notice check succeeds");
        assert_eq!(disabled_notice, None);
        let disabled_records =
            read_local_trace_records_for_scope(Some(&disabled_scope)).expect("records read");
        assert!(
            disabled_records[0].last_credit_notice_at.is_none(),
            "disabled policy must not mark the local notice as seen"
        );

        let zero_interval_scope =
            format!("trace-credit-zero-interval-notice-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&zero_interval_scope),
            &StandingTraceContributionPolicy {
                enabled: true,
                ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                credit_notice_interval_hours: 0,
                ..Default::default()
            },
        )
        .expect("zero interval policy writes");
        write_local_trace_records_for_scope(
            Some(&zero_interval_scope),
            &[submitted_credit_record(
                2.0,
                Some(2.5),
                None,
                vec!["Delayed utility credit posted.".to_string()],
            )],
        )
        .expect("zero interval scope record writes");

        let zero_interval_notice =
            mark_trace_credit_notice_due_for_scope(Some(&zero_interval_scope))
                .expect("zero interval notice check succeeds");
        assert_eq!(zero_interval_notice, None);
        let zero_interval_records =
            read_local_trace_records_for_scope(Some(&zero_interval_scope)).expect("records read");
        assert!(
            zero_interval_records[0].last_credit_notice_at.is_none(),
            "zero interval policy must leave the notice unmarked"
        );
    }

    #[test]
    fn scoped_credit_notice_snapshot_marks_only_that_scope() {
        let due_scope = format!("trace-credit-due-scope-test-{}", Uuid::new_v4());
        let untouched_scope = format!("trace-credit-untouched-scope-test-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            credit_notice_interval_hours: 168,
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&due_scope), &policy).expect("due policy writes");
        write_trace_policy_for_scope(Some(&untouched_scope), &policy)
            .expect("untouched policy writes");
        write_local_trace_records_for_scope(
            Some(&due_scope),
            &[submitted_credit_record(
                1.5,
                Some(2.0),
                None,
                vec!["Accepted after privacy checks.".to_string()],
            )],
        )
        .expect("due record writes");
        write_local_trace_records_for_scope(
            Some(&untouched_scope),
            &[submitted_credit_record(
                9.0,
                Some(10.0),
                None,
                vec!["Should not be marked by another scope.".to_string()],
            )],
        )
        .expect("untouched record writes");

        let notice = mark_trace_credit_notice_due_for_scope(Some(&due_scope))
            .expect("scoped notice check succeeds")
            .expect("due scope should produce a notice");

        assert_eq!(notice.submissions_submitted, 1);
        assert_eq!(notice.pending_credit, 1.5);
        assert_eq!(notice.final_credit, 2.0);

        let due_records = read_local_trace_records_for_scope(Some(&due_scope)).expect("records");
        assert!(due_records[0].last_credit_notice_at.is_some());
        let untouched_records =
            read_local_trace_records_for_scope(Some(&untouched_scope)).expect("records");
        assert!(
            untouched_records[0].last_credit_notice_at.is_none(),
            "checking one scope must not mark another scope's local credit notice"
        );
    }

    #[test]
    fn delayed_credit_sync_resets_notice_and_notice_marks_records() {
        let scope = format!("trace-credit-sync-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy {
                enabled: true,
                ingestion_endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                credit_notice_interval_hours: 168,
                ..Default::default()
            },
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(&scope),
            &[LocalTraceSubmissionRecord {
                submission_id,
                trace_id,
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("accepted".to_string()),
                submitted_at: Some(Utc::now()),
                revoked_at: None,
                privacy_risk: "low".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 1.0,
                credit_points_final: None,
                credit_explanation: vec!["Accepted locally.".to_string()],
                credit_events: Vec::new(),
                last_credit_notice_at: Some(Utc::now()),
            }],
        )
        .expect("local record writes");

        let changed = apply_remote_trace_submission_statuses_for_scope(
            Some(&scope),
            &[TraceSubmissionStatusUpdate {
                submission_id,
                trace_id,
                status: "accepted".to_string(),
                credit_points_pending: 1.0,
                credit_points_final: Some(2.0),
                credit_points_ledger: 1.0,
                credit_points_total: Some(2.0),
                explanation: vec!["Accepted after privacy checks.".to_string()],
                delayed_credit_explanations: vec!["Regression coverage bonus: +1.0.".to_string()],
            }],
        )
        .expect("status sync applies");
        assert_eq!(changed, 1);

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert_eq!(records[0].credit_points_final, Some(2.0));
        assert!(records[0].last_credit_notice_at.is_none());
        assert_eq!(records[0].credit_events.len(), 1);

        let notice = mark_trace_credit_notice_due_for_scope(Some(&scope))
            .expect("notice check succeeds")
            .expect("notice should be due after changed credit");
        assert_eq!(notice.pending_credit, 1.0);
        assert_eq!(notice.final_credit, 2.0);
        assert_eq!(notice.delayed_credit_delta, 1.0);
        assert_eq!(notice.credit_events_total, 1);
        assert!(
            notice
                .recent_explanations
                .iter()
                .any(|reason| reason.contains("Regression coverage bonus"))
        );

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert!(records[0].last_credit_notice_at.is_some());
    }

    #[test]
    fn delayed_credit_explanation_change_resets_notice_without_net_credit_delta() {
        let scope = format!("trace-credit-explanation-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        write_local_trace_records_for_scope(
            Some(&scope),
            &[LocalTraceSubmissionRecord {
                submission_id,
                trace_id,
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("accepted".to_string()),
                submitted_at: Some(Utc::now()),
                revoked_at: None,
                privacy_risk: "low".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 1.0,
                credit_points_final: Some(2.0),
                credit_explanation: vec!["Previous credit explanation.".to_string()],
                credit_events: Vec::new(),
                last_credit_notice_at: Some(Utc::now()),
            }],
        )
        .expect("local record writes");

        let changed = apply_remote_trace_submission_statuses_for_scope(
            Some(&scope),
            &[TraceSubmissionStatusUpdate {
                submission_id,
                trace_id,
                status: "accepted".to_string(),
                credit_points_pending: 1.0,
                credit_points_final: Some(2.0),
                credit_points_ledger: 1.0,
                credit_points_total: Some(2.0),
                explanation: vec!["Accepted after privacy checks.".to_string()],
                delayed_credit_explanations: vec![
                    "Process evaluation utility credited without changing total.".to_string(),
                ],
            }],
        )
        .expect("status sync applies");
        assert_eq!(changed, 1);

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert!(records[0].last_credit_notice_at.is_none());
        assert_eq!(records[0].credit_events.len(), 1);
        assert_eq!(records[0].credit_events[0].points_delta, 0.0);
        assert!(
            records[0]
                .credit_explanation
                .iter()
                .any(|explanation| { explanation.contains("Process evaluation utility credited") })
        );
    }

    #[test]
    fn revoked_credit_change_still_produces_a_notice() {
        let scope = format!("trace-credit-revoked-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        write_local_trace_records_for_scope(
            Some(&scope),
            &[LocalTraceSubmissionRecord {
                submission_id,
                trace_id,
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("accepted".to_string()),
                submitted_at: Some(Utc::now()),
                revoked_at: None,
                privacy_risk: "low".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 1.0,
                credit_points_final: Some(1.0),
                credit_explanation: vec!["Accepted locally.".to_string()],
                credit_events: Vec::new(),
                last_credit_notice_at: Some(Utc::now()),
            }],
        )
        .expect("local record writes");

        apply_remote_trace_submission_statuses_for_scope(
            Some(&scope),
            &[TraceSubmissionStatusUpdate {
                submission_id,
                trace_id,
                status: "revoked".to_string(),
                credit_points_pending: 0.0,
                credit_points_final: Some(0.0),
                credit_points_ledger: 0.0,
                credit_points_total: Some(0.0),
                explanation: vec!["Submission revoked.".to_string()],
                delayed_credit_explanations: Vec::new(),
            }],
        )
        .expect("status sync applies");

        let notice = mark_trace_credit_noticed_if_due(Some(&scope), 168)
            .expect("notice check succeeds")
            .expect("revoked credit delta should still be noticeable");
        assert_eq!(notice.submissions_revoked, 1);
        assert_eq!(notice.final_credit, 0.0);
        assert!(
            notice
                .recent_explanations
                .iter()
                .any(|reason| reason.contains("Submission revoked"))
        );
    }

    #[test]
    fn expired_status_sync_stops_resubmission_and_reports_expired_credit() {
        let scope = format!("trace-credit-expired-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        write_local_trace_records_for_scope(
            Some(&scope),
            &[LocalTraceSubmissionRecord {
                submission_id,
                trace_id,
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("accepted".to_string()),
                submitted_at: Some(Utc::now()),
                revoked_at: None,
                privacy_risk: "low".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 1.0,
                credit_points_final: Some(1.0),
                credit_explanation: vec!["Accepted locally.".to_string()],
                credit_events: Vec::new(),
                last_credit_notice_at: Some(Utc::now()),
            }],
        )
        .expect("local record writes");

        apply_remote_trace_submission_statuses_for_scope(
            Some(&scope),
            &[TraceSubmissionStatusUpdate {
                submission_id,
                trace_id,
                status: "expired".to_string(),
                credit_points_pending: 1.0,
                credit_points_final: Some(1.0),
                credit_points_ledger: 0.0,
                credit_points_total: Some(1.0),
                explanation: vec!["Expired under retention policy.".to_string()],
                delayed_credit_explanations: Vec::new(),
            }],
        )
        .expect("status sync applies");

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert_eq!(records[0].status, LocalTraceSubmissionStatus::Expired);
        assert_eq!(trace_credit_summary(&records).submissions_expired, 1);
        assert!(records[0].last_credit_notice_at.is_none());
    }

    #[test]
    fn trace_credit_report_groups_remote_status_and_delayed_credit_events() {
        let submitted_at = Utc::now();
        let accepted_id = Uuid::new_v4();
        let quarantined_id = Uuid::new_v4();
        let rejected_id = Uuid::new_v4();
        let sync_event_at = submitted_at + chrono::Duration::minutes(5);
        let records = vec![
            LocalTraceSubmissionRecord {
                submission_id: accepted_id,
                trace_id: Uuid::new_v4(),
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("accepted".to_string()),
                submitted_at: Some(submitted_at),
                revoked_at: None,
                privacy_risk: "Low".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 2.0,
                credit_points_final: Some(3.5),
                credit_explanation: vec![
                    "Accepted after privacy checks.".to_string(),
                    "Regression coverage bonus: +1.5.".to_string(),
                ],
                credit_events: vec![
                    TraceCreditEvent {
                        event_id: Uuid::new_v4(),
                        submission_id: accepted_id,
                        contributor_pseudonym: "local".to_string(),
                        kind: TraceCreditEventKind::Accepted,
                        points_delta: 2.0,
                        reason: "Accepted for private Trace Commons processing.".to_string(),
                        created_at: submitted_at,
                    },
                    TraceCreditEvent {
                        event_id: Uuid::new_v4(),
                        submission_id: accepted_id,
                        contributor_pseudonym: "local-sync".to_string(),
                        kind: TraceCreditEventKind::CreditSynced,
                        points_delta: 1.5,
                        reason:
                            "Server status synced as accepted; delayed ledger credit now +1.50."
                                .to_string(),
                        created_at: sync_event_at,
                    },
                ],
                last_credit_notice_at: None,
            },
            LocalTraceSubmissionRecord {
                submission_id: quarantined_id,
                trace_id: Uuid::new_v4(),
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("quarantined".to_string()),
                submitted_at: Some(submitted_at + chrono::Duration::minutes(2)),
                revoked_at: None,
                privacy_risk: "Medium".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 0.0,
                credit_points_final: None,
                credit_explanation: vec![
                    "Submission is quarantined until privacy review completes.".to_string(),
                ],
                credit_events: Vec::new(),
                last_credit_notice_at: None,
            },
            LocalTraceSubmissionRecord {
                submission_id: rejected_id,
                trace_id: Uuid::new_v4(),
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("rejected".to_string()),
                submitted_at: Some(submitted_at + chrono::Duration::minutes(1)),
                revoked_at: None,
                privacy_risk: "High".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 0.0,
                credit_points_final: Some(0.0),
                credit_explanation: vec!["Rejected during privacy review.".to_string()],
                credit_events: Vec::new(),
                last_credit_notice_at: None,
            },
        ];

        let report = trace_credit_report(&records);

        assert_eq!(report.submissions_total, 3);
        assert_eq!(report.submissions_submitted, 3);
        assert_eq!(report.submissions_accepted, 1);
        assert_eq!(report.submissions_quarantined, 1);
        assert_eq!(report.submissions_rejected, 1);
        assert_eq!(report.pending_credit, 2.0);
        assert_eq!(report.final_credit, 3.5);
        assert_eq!(report.credit_events_total, 2);
        assert_eq!(report.delayed_credit_delta, 1.5);
        assert_eq!(
            report.last_submission_at,
            Some(submitted_at + chrono::Duration::minutes(2))
        );
        assert_eq!(report.last_credit_sync_at, Some(sync_event_at));
        assert!(
            report
                .explanation_lines
                .iter()
                .any(|line| line.contains("1 accepted"))
        );
        assert!(
            report
                .explanation_lines
                .iter()
                .any(|line| line.contains("1 quarantined"))
        );
        assert!(
            report
                .explanation_lines
                .iter()
                .any(|line| line.contains("1 rejected"))
        );
        assert!(
            report
                .explanation_lines
                .iter()
                .any(|line| line.contains("Regression coverage bonus"))
        );
    }

    #[test]
    fn trace_credit_summary_uses_richer_report_totals_without_changing_shape() {
        let record = LocalTraceSubmissionRecord {
            submission_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: LocalTraceSubmissionStatus::Purged,
            server_status: Some("expired".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "Low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 4.0,
            credit_points_final: Some(4.0),
            credit_explanation: vec!["Expired under retention policy.".to_string()],
            credit_events: Vec::new(),
            last_credit_notice_at: None,
        };

        let summary = trace_credit_summary(&[record]);

        assert_eq!(summary.submissions_total, 1);
        assert_eq!(summary.submissions_expired, 1);
        assert_eq!(summary.pending_credit, 4.0);
        assert_eq!(summary.final_credit, 4.0);
        assert_eq!(
            summary.recent_explanations,
            vec!["Expired under retention policy.".to_string()]
        );
    }

    #[tokio::test]
    async fn queue_flush_holds_failed_submission_and_still_returns_due_credit_notice() {
        let scope = format!("trace-flush-submit-failure-test-{}", Uuid::new_v4());
        let _token_guard = EnvVarRestore::set("TRACE_COMMONS_TEST_TOKEN", "super-secret-token");
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("http://127.0.0.1:9/v1/traces".to_string()),
            bearer_token_env: "TRACE_COMMONS_TEST_TOKEN".to_string(),
            auto_submit_high_value_traces: true,
            min_submission_score: 0.0,
            credit_notice_interval_hours: 168,
            ..Default::default()
        };
        write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);
        let queue_path = queue_trace_envelope_for_scope(Some(&scope), &envelope)
            .expect("queued envelope writes");

        write_local_trace_records_for_scope(
            Some(&scope),
            &[LocalTraceSubmissionRecord {
                submission_id: Uuid::new_v4(),
                trace_id: Uuid::new_v4(),
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("accepted".to_string()),
                submitted_at: Some(Utc::now()),
                revoked_at: None,
                privacy_risk: "low".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 1.5,
                credit_points_final: Some(2.5),
                credit_explanation: vec!["Delayed utility credit posted.".to_string()],
                credit_events: Vec::new(),
                last_credit_notice_at: None,
            }],
        )
        .expect("local record writes");

        let report = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
            .await
            .expect("flush should not abort on one failed submission");

        assert_eq!(report.submitted, 0);
        assert_eq!(report.held, 1);
        assert_eq!(report.holds[0].submission_id, envelope.submission_id);
        assert!(queue_path.exists(), "failed envelope should stay queued");
        assert!(report.holds[0].reason.contains("retained for retry"));
        assert!(!report.holds[0].reason.contains("127.0.0.1"));
        assert!(!report.holds[0].reason.contains("super-secret-token"));

        let hold_path = queue_path.with_extension("held.json");
        let hold_body = std::fs::read_to_string(&hold_path).expect("hold reason writes");
        assert!(hold_body.contains("retained for retry"));
        assert!(!hold_body.contains("127.0.0.1"));
        assert!(!hold_body.contains("super-secret-token"));

        let notice = report
            .credit_notice
            .expect("due credit notice should still be evaluated");
        assert_eq!(notice.submissions_submitted, 1);
        assert_eq!(notice.pending_credit, 1.5);
        assert_eq!(notice.final_credit, 2.5);
    }

    #[test]
    fn read_trace_queue_holds_for_scope_returns_sidecars_without_envelope_bodies() {
        let scope = format!("trace-queue-holds-test-{}", Uuid::new_v4());
        let dir = trace_queue_dir(Some(&scope));
        std::fs::create_dir_all(&dir).expect("queue dir exists");
        let submission_id = Uuid::new_v4();
        let queue_path = dir.join(format!("{submission_id}.json"));
        std::fs::write(&queue_path, "raw envelope body should not be exposed")
            .expect("queued envelope fixture writes");
        write_trace_queue_hold_reason(&queue_path, "requires manual review")
            .expect("hold reason writes");

        std::fs::write(
            dir.join(format!("{}.held.json", Uuid::new_v4())),
            "{not-json",
        )
        .expect("malformed hold fixture writes");
        std::fs::write(
            dir.join("not-a-submission.held.json"),
            serde_json::json!({ "reason": "should be ignored" }).to_string(),
        )
        .expect("invalid id hold fixture writes");

        let holds = read_trace_queue_holds_for_scope(Some(&scope)).expect("holds read");

        assert_eq!(holds.len(), 1);
        assert_eq!(holds[0].submission_id, submission_id);
        assert_eq!(holds[0].reason, "requires manual review");
        let serialized = serde_json::to_string(&holds).expect("holds serialize");
        assert!(!serialized.contains("raw envelope body should not be exposed"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn server_rescrub_redacts_late_leaks_before_storage() {
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions {
                include_message_text: true,
                include_tool_payloads: true,
                ..Default::default()
            },
        );
        let mut envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
            "/Users/alice/project",
        )])
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

        envelope.events[0].redacted_content =
            Some("late leak at /tmp/ironclaw/private/token.txt".to_string());
        envelope.events[1].structured_payload = serde_json::json!({
            "Authorization": "Bearer abcdefghijklmnopqrstuvwxyz123456",
            "path": "/tmp/ironclaw/private/token.txt"
        });
        rescrub_trace_envelope_with(&DeterministicTraceRedactor::new(Vec::new()), &mut envelope);

        let json = serde_json::to_string(&envelope).expect("envelope serializes");
        assert!(json.contains("<PRIVATE_LOCAL_PATH_"));
        assert!(json.contains(SERVER_RESCRUB_PIPELINE_SUFFIX));
        assert!(!json.contains("/tmp/ironclaw/private/token.txt"));
        assert!(!json.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(
            envelope
                .privacy
                .redaction_counts
                .get("local_path")
                .copied()
                .unwrap_or_default()
                >= 3
        );
    }

    #[tokio::test]
    async fn value_score_caps_novelty_and_records_scorecard() {
        let mut raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        raw.embedding_analysis = Some(EmbeddingAnalysisMetadata {
            embedding_model: Some("test-embedding".to_string()),
            canonical_summary_hash: "sha256:test".to_string(),
            trace_vector_id: Some("vector-1".to_string()),
            nearest_trace_ids: Vec::new(),
            cluster_id: Some("cluster-1".to_string()),
            nearest_cluster_id: Some("cluster-1".to_string()),
            novelty_score: Some(99.0),
            duplicate_score: Some(0.0),
            coverage_tags: Vec::new(),
        });
        let envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");

        let estimate = estimate_initial_credit(&envelope);
        assert_eq!(estimate.scorecard.novelty, 0.85);
        assert_eq!(
            estimate.credit_points_pending,
            estimate.scorecard.credit_points_estimate
        );
    }

    #[tokio::test]
    async fn research_scorecard_extension_fields_default_for_older_envelopes() {
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        let mut json = serde_json::to_value(&envelope).expect("envelope serializes");
        let object = json.as_object_mut().expect("envelope is a json object");
        object.remove("hindsight");
        object.remove("training_dynamics");
        object.remove("process_evaluation");

        let decoded: TraceContributionEnvelope =
            serde_json::from_value(json).expect("older envelope deserializes");

        assert_eq!(decoded.hindsight, None);
        assert_eq!(decoded.training_dynamics, None);
        assert_eq!(decoded.process_evaluation, None);
    }

    #[tokio::test]
    async fn process_evaluator_labels_allow_partial_future_payloads() {
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        let mut json = serde_json::to_value(&envelope).expect("envelope serializes");
        let object = json.as_object_mut().expect("envelope is a json object");
        object.insert(
            "process_evaluation".to_string(),
            serde_json::json!({
                "overall_score": 0.66,
                "labels": ["proper_verification"]
            }),
        );

        let decoded: TraceContributionEnvelope =
            serde_json::from_value(json).expect("partial process evaluation deserializes");

        let process_evaluation = decoded
            .process_evaluation
            .expect("process evaluation should be preserved");
        assert_eq!(process_evaluation.evaluator_version, "");
        assert_eq!(process_evaluation.overall_score, Some(0.66));
        assert_eq!(
            process_evaluation.labels,
            vec![ProcessEvaluatorLabel::ProperVerification]
        );
    }

    #[tokio::test]
    async fn process_evaluator_labels_do_not_require_raw_content() {
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        envelope.process_evaluation = Some(ProcessEvaluationLabels {
            evaluator_version: "process-evaluator-v1".to_string(),
            labels: vec![
                ProcessEvaluatorLabel::CorrectToolSelection,
                ProcessEvaluatorLabel::MissingVerification,
            ],
            tool_selection: Some(ProcessEvalRating::Pass),
            tool_argument_quality: Some(ProcessEvalRating::Unknown),
            tool_ordering: Some(ProcessEvalRating::Partial),
            verification: Some(ProcessEvalRating::Fail),
            side_effect_safety: Some(ProcessEvalRating::Pass),
            overall_score: Some(0.72),
            ..ProcessEvaluationLabels::default()
        });
        envelope.hindsight = Some(HindsightRelabelingCandidate {
            achieved_subgoals: vec!["redacted_subgoal:diagnosed_tool_failure".to_string()],
            failure_type: Some(TraceFailureMode::MissingVerification),
            recoverability_score: Some(0.8),
            benchmark_candidate: true,
            relabeled_training_candidate: true,
            ..HindsightRelabelingCandidate::default()
        });
        envelope.training_dynamics = Some(TrainingDynamicsSignals {
            mean_confidence: Some(0.61),
            variability: Some(0.29),
            correctness: Some(0.5),
            cartography_bucket: Some(CartographyBucket::Ambiguous),
        });

        let json = serde_json::to_string(&envelope).expect("envelope serializes");
        let decoded: TraceContributionEnvelope =
            serde_json::from_str(&json).expect("envelope deserializes");

        assert!(json.contains("process_evaluation"));
        assert!(json.contains("training_dynamics"));
        assert!(json.contains("hindsight"));
        assert!(!json.contains("raw_content"));
        assert!(!json.contains("raw_tool"));
        assert!(!json.contains("hidden_reasoning"));
        assert_eq!(
            decoded
                .process_evaluation
                .as_ref()
                .expect("process labels present")
                .labels,
            vec![
                ProcessEvaluatorLabel::CorrectToolSelection,
                ProcessEvaluatorLabel::MissingVerification,
            ]
        );
    }
}
