//! Privacy-preserving trace contribution envelopes.
//!
//! This module is intentionally separate from replay traces. Replay fixtures
//! capture enough behavior to drive tests; contribution envelopes capture the
//! consent, privacy, replayability, scoring, and revocation metadata needed
//! before a trace can leave a user's machine.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Context;
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

use crate::redaction::redact_sensitive_json;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_llm::recording::{TraceFile, TraceResponse};

pub const TRACE_CONTRIBUTION_SCHEMA_VERSION: &str = "ironclaw.trace_contribution.v1";
pub const TRACE_CONTRIBUTION_POLICY_VERSION: &str = "2026-04-24";
pub const DETERMINISTIC_REDACTION_PIPELINE_VERSION: &str = "ironclaw-deterministic-secret-path-v1";
pub const PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX: &str = "privacy-filter-sidecar-v1";
pub const SERVER_RESCRUB_PIPELINE_SUFFIX: &str = "server-rescrub-v1";
pub const PRIVACY_FILTER_CANARY_VERSION: &str = "trace-privacy-filter-canary-v1";
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES: usize = 1024 * 1024;
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES: usize = 64 * 1024;
pub const TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS: u32 = 24 * 365;
pub const TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS: u64 = 5_000;
pub const TRACE_REMOTE_REQUEST_DEFAULT_TIMEOUT_MS: u64 = 30_000;
pub const TRACE_REMOTE_REQUEST_TIMEOUT_ENV: &str = "IRONCLAW_TRACE_REMOTE_REQUEST_TIMEOUT_MS";
pub const TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Default page size for an account-traces fetch when the caller passes no
/// explicit limit. Bounds the initial WebUI/facade slice so `None` never
/// requests unbounded history; full history is a future paginated flow.
const ACCOUNT_TRACES_DEFAULT_LIMIT: usize = 200;
/// Hard ceiling on the account-traces page size; larger requests are clamped so
/// a caller can never ask the server for an unbounded slice.
const ACCOUNT_TRACES_MAX_LIMIT: usize = 500;
/// Maximum accepted account-traces response body (256 KiB). A list response is
/// larger than a single claim but must still be bounded so the direct path
/// cannot buffer an unbounded body.
const ACCOUNT_TRACES_MAX_RESPONSE_BYTES: usize = 256 * 1024;
const TRACE_UPLOAD_CLAIM_REFRESH_SKEW_SECONDS: i64 = 60;
const TRACE_CREDIT_NOTICE_OUTBOX_MAX_ATTEMPTS_STORED: usize = 10;

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
    /// Set when the user has explicitly authorized this held trace for
    /// submission past the manual-review (High residual-PII-risk) gate. The
    /// flag travels into the submitted trace so the server ledger records that
    /// the contribution was made under explicit higher-risk authorization.
    #[serde(default)]
    pub manual_review_authorized: bool,
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
    /// Contributor has explicitly consented to map their pseudonymous
    /// principal_ref to a publicly-visible handle via the community
    /// surface. Does NOT grant any trace-content allowed-uses on its
    /// own — it gates the /v1/community/profile endpoints. A claim
    /// scoped to ONLY public_attribution cannot submit traces.
    PublicAttribution,
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

impl OutcomeMetadata {
    pub fn set_user_feedback(mut self, user_feedback: UserFeedback) -> Self {
        self.user_feedback = user_feedback;
        self
    }

    pub fn set_task_success(mut self, task_success: TaskSuccess) -> Self {
        self.task_success = task_success;
        self
    }

    pub fn set_failure_modes(mut self, failure_modes: Vec<TraceFailureMode>) -> Self {
        self.failure_modes = failure_modes;
        self
    }

    pub fn set_human_correction(mut self, human_correction: impl Into<String>) -> Self {
        self.human_correction = Some(human_correction.into());
        self
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

impl ProcessEvaluationLabels {
    pub fn set_evaluator_version(mut self, evaluator_version: impl Into<String>) -> Self {
        self.evaluator_version = evaluator_version.into();
        self
    }

    pub fn set_labels(mut self, labels: Vec<ProcessEvaluatorLabel>) -> Self {
        self.labels = labels;
        self
    }

    pub fn set_tool_selection(mut self, tool_selection: ProcessEvalRating) -> Self {
        self.tool_selection = Some(tool_selection);
        self
    }

    pub fn set_tool_argument_quality(mut self, tool_argument_quality: ProcessEvalRating) -> Self {
        self.tool_argument_quality = Some(tool_argument_quality);
        self
    }

    pub fn set_tool_ordering(mut self, tool_ordering: ProcessEvalRating) -> Self {
        self.tool_ordering = Some(tool_ordering);
        self
    }

    pub fn set_verification(mut self, verification: ProcessEvalRating) -> Self {
        self.verification = Some(verification);
        self
    }

    pub fn set_side_effect_safety(mut self, side_effect_safety: ProcessEvalRating) -> Self {
        self.side_effect_safety = Some(side_effect_safety);
        self
    }

    pub fn set_overall_score(mut self, overall_score: f32) -> Self {
        self.overall_score = Some(overall_score);
        self
    }
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

impl HindsightRelabelingCandidate {
    pub fn set_achieved_subgoals(mut self, achieved_subgoals: Vec<String>) -> Self {
        self.achieved_subgoals = achieved_subgoals;
        self
    }

    pub fn set_failure_type(mut self, failure_type: TraceFailureMode) -> Self {
        self.failure_type = Some(failure_type);
        self
    }

    pub fn set_recoverability_score(mut self, recoverability_score: f32) -> Self {
        self.recoverability_score = Some(recoverability_score);
        self
    }

    pub fn set_benchmark_candidate(mut self, benchmark_candidate: bool) -> Self {
        self.benchmark_candidate = benchmark_candidate;
        self
    }

    pub fn set_relabeled_training_candidate(mut self, relabeled_training_candidate: bool) -> Self {
        self.relabeled_training_candidate = relabeled_training_candidate;
        self
    }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceUploadAuthMode {
    /// Operator-minted workload token read from env (legacy/back-compat path).
    #[default]
    WorkloadTokenEnv,
    /// Self-signed workload JWTs using the local device key (agent onboarding path).
    DeviceKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandingTraceContributionPolicy {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_endpoint: Option<String>,
    pub bearer_token_env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_issuer_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub upload_token_issuer_allowed_hosts: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_audience: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_workload_token_env: Option<String>,
    /// Operator-issued pilot invite code. When set, the trace-commons
    /// upload-claim request includes it (mirrored into the request body;
    /// the server-side issuer reads `WorkloadClaims.invite_code` today,
    /// the body field is forward-compat for a later server slice). The
    /// client surfaces the issuer's typed `PilotAllowlist*` refusals
    /// directly — there is no local JWT pre-flight that decodes the
    /// configured workload token to verify the embedded `invite_code`
    /// matches this value. Off by default; only required when the issuer
    /// is allowlist-gated. A follow-up may add the pre-flight check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_token_invite_code: Option<String>,
    #[serde(default = "default_trace_upload_claim_issuer_timeout_ms")]
    pub upload_token_issuer_timeout_ms: u64,
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
    #[serde(default)]
    pub auth_mode: TraceUploadAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_key_id: Option<String>,
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

pub fn normalize_trace_selected_tools<I, S>(selected_tools: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    selected_tools
        .into_iter()
        .map(|tool| tool.as_ref().trim().to_string())
        .filter(|tool| !tool.is_empty())
        .collect()
}

impl Default for StandingTraceContributionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            ingestion_endpoint: None,
            bearer_token_env: "IRONCLAW_TRACE_SUBMIT_TOKEN".to_string(),
            upload_token_issuer_url: None,
            upload_token_issuer_allowed_hosts: BTreeSet::new(),
            upload_token_audience: None,
            upload_token_tenant_id: None,
            upload_token_workload_token_env: None,
            upload_token_invite_code: None,
            upload_token_issuer_timeout_ms: TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS,
            include_message_text: false,
            include_tool_payloads: false,
            auto_submit_failed_traces: true,
            auto_submit_high_value_traces: true,
            selected_tools: BTreeSet::new(),
            require_manual_approval_when_pii_detected: true,
            min_submission_score: 0.35,
            credit_notice_interval_hours: 168,
            default_scope: ConsentScope::DebuggingEvaluation,
            auth_mode: TraceUploadAuthMode::default(),
            device_key_id: None,
        }
    }
}

impl StandingTraceContributionPolicy {
    pub fn set_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn set_ingestion_endpoint(mut self, ingestion_endpoint: impl Into<String>) -> Self {
        self.ingestion_endpoint = Some(ingestion_endpoint.into());
        self
    }

    pub fn set_bearer_token_env(mut self, bearer_token_env: impl Into<String>) -> Self {
        self.bearer_token_env = bearer_token_env.into();
        self
    }

    pub fn set_upload_token_issuer_url(
        mut self,
        upload_token_issuer_url: impl Into<String>,
    ) -> Self {
        self.upload_token_issuer_url = Some(upload_token_issuer_url.into());
        self
    }

    pub fn set_upload_token_issuer_allowed_hosts<I, S>(
        mut self,
        upload_token_issuer_allowed_hosts: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.upload_token_issuer_allowed_hosts = upload_token_issuer_allowed_hosts
            .into_iter()
            .map(Into::into)
            .collect();
        self
    }

    pub fn set_upload_token_audience(mut self, upload_token_audience: impl Into<String>) -> Self {
        self.upload_token_audience = Some(upload_token_audience.into());
        self
    }

    pub fn set_upload_token_tenant_id(mut self, upload_token_tenant_id: impl Into<String>) -> Self {
        self.upload_token_tenant_id = Some(upload_token_tenant_id.into());
        self
    }

    pub fn set_upload_token_workload_token_env(
        mut self,
        upload_token_workload_token_env: impl Into<String>,
    ) -> Self {
        self.upload_token_workload_token_env = Some(upload_token_workload_token_env.into());
        self
    }

    pub fn set_upload_token_invite_code(
        mut self,
        upload_token_invite_code: impl Into<String>,
    ) -> Self {
        self.upload_token_invite_code = Some(upload_token_invite_code.into());
        self
    }

    pub fn set_upload_token_issuer_timeout_ms(
        mut self,
        upload_token_issuer_timeout_ms: u64,
    ) -> Self {
        self.upload_token_issuer_timeout_ms = upload_token_issuer_timeout_ms;
        self
    }

    pub fn set_include_message_text(mut self, include_message_text: bool) -> Self {
        self.include_message_text = include_message_text;
        self
    }

    pub fn set_include_tool_payloads(mut self, include_tool_payloads: bool) -> Self {
        self.include_tool_payloads = include_tool_payloads;
        self
    }

    pub fn set_auto_submit_failed_traces(mut self, auto_submit_failed_traces: bool) -> Self {
        self.auto_submit_failed_traces = auto_submit_failed_traces;
        self
    }

    pub fn set_auto_submit_high_value_traces(
        mut self,
        auto_submit_high_value_traces: bool,
    ) -> Self {
        self.auto_submit_high_value_traces = auto_submit_high_value_traces;
        self
    }

    pub fn set_selected_tools<I, S>(mut self, selected_tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.selected_tools = selected_tools.into_iter().map(Into::into).collect();
        self
    }

    pub fn set_require_manual_approval_when_pii_detected(
        mut self,
        require_manual_approval_when_pii_detected: bool,
    ) -> Self {
        self.require_manual_approval_when_pii_detected = require_manual_approval_when_pii_detected;
        self
    }

    pub fn set_min_submission_score(mut self, min_submission_score: f32) -> Self {
        self.min_submission_score = min_submission_score;
        self
    }

    pub fn set_credit_notice_interval_hours(mut self, credit_notice_interval_hours: u32) -> Self {
        self.credit_notice_interval_hours = credit_notice_interval_hours;
        self
    }

    pub fn set_default_scope(mut self, default_scope: ConsentScope) -> Self {
        self.default_scope = default_scope;
        self
    }

    pub fn set_auth_mode(mut self, auth_mode: TraceUploadAuthMode) -> Self {
        self.auth_mode = auth_mode;
        self
    }

    pub fn set_device_key_id(mut self, device_key_id: impl Into<String>) -> Self {
        self.device_key_id = Some(device_key_id.into());
        self
    }
}

fn default_trace_upload_claim_issuer_timeout_ms() -> u64 {
    TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS
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

pub fn trace_credit_notice_message(summary: &CreditSummary) -> String {
    let mut message = format!(
        "Trace contribution credit update: {} submitted, {} expired ({} total), pending +{:.2}, final confirmed +{:.2}, delayed ledger {:+.2}. Delayed credit can change after privacy review, replay/eval, duplicate checks, and downstream utility scoring.",
        summary.submissions_submitted,
        summary.submissions_expired,
        summary.submissions_total,
        summary.pending_credit,
        summary.final_credit,
        summary.delayed_credit_delta
    );
    if summary.credit_events_total > 0 {
        message.push_str(&format!(
            " {} credit event(s) recorded.",
            summary.credit_events_total
        ));
    }
    if !summary.recent_explanations.is_empty() {
        let explanations = summary
            .recent_explanations
            .iter()
            .take(2)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        message.push_str(" Recent factors: ");
        message.push_str(&explanations);
    }
    message
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

// Below-High residual PII risk is treated as clean for scoring: the
// deterministic redactor has already scrubbed detected PII, and the 0.35
// submission gate leaves no headroom for a partial Medium discount on a
// minimal trace (it would zero an otherwise-valuable trace). Only High risk
// is penalized — `privacy_gate` zeros its score and `trace_autonomous_eligibility`
// holds it for manual review.
fn privacy_gate(risk: ResidualPiiRisk) -> f32 {
    match risk {
        ResidualPiiRisk::Low | ResidualPiiRisk::Medium => 1.0,
        ResidualPiiRisk::High => 0.0,
    }
}

fn privacy_risk_score(risk: ResidualPiiRisk) -> f32 {
    match risk {
        ResidualPiiRisk::Low | ResidualPiiRisk::Medium => 0.0,
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
    messages: &[crate::ConversationMessage],
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

impl RecordedTraceContributionOptions {
    pub fn set_include_message_text(mut self, include_message_text: bool) -> Self {
        self.include_message_text = include_message_text;
        self
    }

    pub fn set_include_tool_payloads(mut self, include_tool_payloads: bool) -> Self {
        self.include_tool_payloads = include_tool_payloads;
        self
    }

    pub fn set_consent_scopes(mut self, consent_scopes: Vec<ConsentScope>) -> Self {
        self.consent_scopes = consent_scopes;
        self
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
            outcome: OutcomeMetadata::default().set_task_success(task_success),
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
        "/tmp/trace_canary_private/path.txt".to_string(), // safety: canary text intentionally includes a synthetic private path.
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

impl DeterministicTraceRedactor {
    pub async fn redact_trace(
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
            manual_review_authorized: false,
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
        // Public attribution grants no trace-content allowed-uses; a claim
        // scoped to only public_attribution cannot submit traces.
        ConsentScope::PublicAttribution => Vec::new(),
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
        Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b")
            .expect("hardcoded private email regex must compile") // safety: hardcoded regex is covered by unit tests and should always compile.
    });
    &PRIVATE_EMAIL_REGEX
}

fn local_path_regex() -> &'static Regex {
    static LOCAL_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?x)(?:/Users|/home|/private/var|/tmp)/[^\s'"`<>{}\[\]]+"#)
            .expect("hardcoded local path regex must compile") // safety: hardcoded regex is covered by unit tests and should always compile.
    });
    &LOCAL_PATH_REGEX
}

fn trace_queue_secret_like_reason_regex() -> &'static Regex {
    static TRACE_QUEUE_SECRET_LIKE_REASON_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?ix)\b(?:sk|pk|rk|ghp|gho|ghu|glpat|xox[baprs])[-_a-z0-9]{8,}\b")
            .expect("hardcoded trace queue secret-like reason regex must compile") // safety: hardcoded regex is covered by queue diagnostics tests.
    });
    &TRACE_QUEUE_SECRET_LIKE_REASON_REGEX
}

fn remote_credit_explanation_url_regex() -> &'static Regex {
    static REMOTE_CREDIT_EXPLANATION_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\bhttps?://[^\s'"`<>{}\[\]]+"#)
            .expect("hardcoded remote credit explanation URL regex must compile") // safety: hardcoded regex is covered by local status-history safety tests.
    });
    &REMOTE_CREDIT_EXPLANATION_URL_REGEX
}

fn remote_credit_explanation_tenant_ref_regex() -> &'static Regex {
    static REMOTE_CREDIT_EXPLANATION_TENANT_REF_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\btenant[-_][a-z0-9][a-z0-9_-]{1,}\b")
            .expect("hardcoded remote credit explanation tenant-ref regex must compile") // safety: hardcoded regex is covered by local status-history safety tests.
    });
    &REMOTE_CREDIT_EXPLANATION_TENANT_REF_REGEX
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<LocalTraceSubmissionHistoryEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_credit_notice_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "TraceCreditNoticeState::is_empty")]
    pub credit_notice_state: TraceCreditNoticeState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalTraceSubmissionHistoryEvent {
    pub event_id: Uuid,
    pub kind: LocalTraceSubmissionHistoryKind,
    pub occurred_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_status: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub credit_delta: f32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub delayed_credit_explanation_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalTraceSubmissionHistoryKind {
    StatusSync,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditNoticeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_presented_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

impl TraceCreditNoticeState {
    pub fn is_empty(&self) -> bool {
        self.last_presented_at.is_none()
            && self.acknowledged_at.is_none()
            && self.snoozed_until.is_none()
            && self.fingerprint.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceCreditNoticeOutboxItem {
    pub notice_id: String,
    pub fingerprint: String,
    pub summary: CreditSummary,
    pub message: String,
    pub status: TraceCreditNoticeOutboxStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub attempt_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivery_attempts: Vec<TraceCreditNoticeDeliveryAttempt>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceCreditNoticeOutboxStatus {
    Pending,
    Delivered,
    Acknowledged,
    Snoozed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCreditNoticeDeliveryAttempt {
    pub channel: String,
    pub attempted_at: DateTime<Utc>,
    pub succeeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<TraceQueueTelemetryFailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalTraceSubmissionStatus {
    Submitted,
    Revoked,
    Expired,
    Purged,
}

impl LocalTraceSubmissionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Purged => "purged",
        }
    }
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
    pub kind: TraceQueueHoldKind,
    pub reason: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceQueueHoldKind {
    PolicyGate,
    ManualReview,
    RetryableSubmissionFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceQueueHoldSidecar {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    envelope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    held_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<TraceQueueHoldKind>,
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_retry_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueFlushReport {
    pub submitted: usize,
    pub held: usize,
    #[serde(default, skip_serializing_if = "TraceQueueCompactionReport::is_empty")]
    pub compaction: TraceQueueCompactionReport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<TraceQueueHold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_notice: Option<CreditSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueWorkerReport {
    pub scopes_checked: usize,
    pub submitted: usize,
    pub held: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_reports: Vec<TraceQueueWorkerScopeReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceQueueWorkerScopeReport {
    pub scope: String,
    pub submitted: usize,
    pub held: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<TraceQueueHold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_notice: Option<CreditSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueCompactionReport {
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub scanned_count: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub duplicate_envelopes_removed: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub orphan_hold_sidecars_removed: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub malformed_envelopes_quarantined: u32,
}

impl TraceQueueCompactionReport {
    pub fn set_scanned_count(mut self, scanned_count: u32) -> Self {
        self.scanned_count = scanned_count;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.scanned_count == 0
            && self.duplicate_envelopes_removed == 0
            && self.orphan_hold_sidecars_removed == 0
            && self.malformed_envelopes_quarantined == 0
    }

    fn reclaimed_count(&self) -> u32 {
        self.duplicate_envelopes_removed
            .saturating_add(self.orphan_hold_sidecars_removed)
            .saturating_add(self.malformed_envelopes_quarantined)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueTelemetry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_flush_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_flush_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failed_flush_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub consecutive_flush_failures: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub retryable_submission_failure_count: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub status_sync_failure_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_retryable_submission_failure_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status_sync_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status_sync_failed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction: Option<TraceQueueCompactionReport>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub compaction_reclaimed_items_total: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<TraceQueueTelemetryFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueTelemetryFailure {
    pub kind: TraceQueueTelemetryFailureKind,
    pub reason: String,
    pub error_hash: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceQueueTelemetryFailureKind {
    Policy,
    Endpoint,
    Credential,
    Network,
    NetworkOffline,
    NetworkDns,
    NetworkTimeout,
    NetworkConnectionRefused,
    HttpRejection,
    StatusSync,
    Submission,
    Queue,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceQueueWarning {
    pub kind: TraceQueueWarningKind,
    pub count: u32,
    pub severity: TraceQueueWarningSeverity,
    pub promotion_blocking: bool,
    pub message: String,
    pub recommended_action: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceQueueWarningSeverity {
    Warning,
    Blocking,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceQueueWarningKind {
    SchemaVersionMismatch,
    PolicyVersionMismatch,
    RedactionPipelineMismatch,
    TraceCardRedactionPipelineMismatch,
    MalformedEnvelope,
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
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub retry_scheduled_count: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub manual_review_hold_count: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub policy_hold_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub telemetry: TraceQueueTelemetry,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<TraceQueueWarning>,
    pub policy_enabled: bool,
    pub endpoint_configured: bool,
    pub ready_to_flush: bool,
}

pub enum TraceQueueEligibility {
    Submit,
    Hold {
        kind: TraceQueueHoldKind,
        reason: String,
    },
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

pub(crate) fn trace_contribution_dir_for_scope_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> PathBuf {
    let contributions = base.join("trace_contributions");
    match scope {
        Some(scope) if !scope.trim().is_empty() => {
            contributions.join("users").join(scope_hash(scope))
        }
        _ => contributions,
    }
}

pub fn trace_contribution_dir_for_scope(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

/// Canonical per-scope key for Trace Commons local state (policy, device keys,
/// credits, profile tokens).
///
/// Composite of `tenant_id` + `user_id` so the same user id in two tenants does
/// NOT share local state — Trace Commons state is tenant-scoped, and keying on
/// the user alone would collapse cross-tenant isolation. The returned string is
/// opaque: callers hand it to `trace_contribution_dir_for_scope` /
/// `read_*_for_scope`, which hash it, so only stability and cross-tenant
/// distinctness matter (the `/` separator can't collide because `TenantId`
/// validation forbids it).
pub fn trace_scope_key(tenant_id: &str, user_id: &str) -> String {
    format!("{tenant_id}/{user_id}")
}

pub fn local_pseudonymous_contributor_id(scope: &str) -> String {
    format!("sha256:{}", scope_hash(scope))
}

/// Read (or create on first use) the per-instance random salt used to derive
/// per-user pseudonymous subjects under instance enrollment. Persisted at the
/// instance trace dir (`0600` on Unix). Concurrent first-use races are settled
/// with `create_new`: exactly one writer wins and the loser re-reads.
fn instance_subject_salt_at(base: &std::path::Path) -> anyhow::Result<String> {
    use std::io::Write as _;

    let dir = trace_contribution_dir_for_scope_at(base, None);
    let path = dir.join("subject_salt");
    let read_existing = |path: &std::path::Path| -> anyhow::Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(salt) => {
                let salt = salt.trim().to_string();
                anyhow::ensure!(!salt.is_empty(), "instance subject salt file is empty");
                Ok(Some(salt))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow::anyhow!("failed to read instance subject salt: {e}")),
        }
    };
    if let Some(salt) = read_existing(&path)? {
        return Ok(salt);
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("failed to create instance trace dir: {e}"))?;
    // 32 random bytes, hex-encoded.
    let salt = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let open_new = || {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(&path)
    };
    match open_new() {
        Ok(mut file) => {
            file.write_all(salt.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|e| anyhow::anyhow!("failed to write instance subject salt: {e}"))?;
            Ok(salt)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_existing(&path)?
            .ok_or_else(|| anyhow::anyhow!("instance subject salt disappeared during creation")),
        Err(e) => Err(anyhow::anyhow!(
            "failed to create instance subject salt: {e}"
        )),
    }
}

/// Per-user pseudonymous subject for instance enrollment, salted with the
/// per-instance random salt. Unlike [`local_pseudonymous_contributor_id`]
/// (an unsalted scope hash used for local state keying and log refs), this
/// value is sent to the Trace Commons server as the claim subject — salting
/// prevents the server (or anyone with ledger access) from dictionary-matching
/// guessable tenant/user identifiers to de-pseudonymize contributors.
fn salted_pseudonymous_contributor_id_at(
    base: &std::path::Path,
    scope: &str,
) -> anyhow::Result<String> {
    let salt = instance_subject_salt_at(base)?;
    let digest = Sha256::digest(format!("{salt}:{scope}").as_bytes());
    // safety: slicing the fixed-size SHA-256 byte array.
    Ok(format!("sha256:{}", hex::encode(&digest[..16])))
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

/// Read the scoped policy only if its file exists: `Ok(None)` when absent.
/// The presence distinction matters for the instance-enrollment fallback —
/// a scoped policy file that EXISTS with `enabled = false` is an explicit
/// user opt-out (written by `traces opt-out`) and must not be treated like
/// "never configured".
fn read_trace_policy_for_scope_if_present_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> anyhow::Result<Option<StandingTraceContributionPolicy>> {
    let path = trace_policy_path_at(base, scope);
    // Fail loud on stat/permission errors: `Path::exists()` maps them to
    // `false`, which would silently treat an unreadable policy as
    // missing/default-disabled and flip enrollment/flush behavior. Only a
    // confirmed non-existent path reports absence.
    if !path
        .try_exists()
        .map_err(|e| anyhow::anyhow!("failed to stat trace policy {}: {}", path.display(), e))?
    {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read trace policy {}: {}", path.display(), e))?;
    serde_json::from_str(&body)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("failed to parse trace policy {}: {}", path.display(), e))
}

fn read_trace_policy_for_scope_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> anyhow::Result<StandingTraceContributionPolicy> {
    Ok(read_trace_policy_for_scope_if_present_at(base, scope)?.unwrap_or_default())
}

pub fn read_trace_policy_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<StandingTraceContributionPolicy> {
    read_trace_policy_for_scope_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

fn write_trace_policy_for_scope_at(
    base: &std::path::Path,
    scope: Option<&str>,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    write_json_file(&trace_policy_path_at(base, scope), policy, "trace policy")
}

pub fn write_trace_policy_for_scope(
    scope: Option<&str>,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    write_trace_policy_for_scope_at(&ironclaw_common::paths::ironclaw_base_dir(), scope, policy)
}

// ── Trace credential resolution (instance enrollment) ────────────────────────
//
// File-size justification (.claude/rules/architecture.md §5): this PR adds the
// instance-enrollment resolver, account login-link, and account-traces sections
// to an already-oversized module because they are tightly coupled to the
// policy-read/scope-dir/claim-mint machinery that lives here (every helper
// below calls into read_trace_policy_for_scope_at / trace_contribution_dir_* /
// DefaultTraceUploadCredentialProvider); splitting them out first would have
// meant exporting a wide private surface mid-feature. Decomposition of
// contribution.rs is tracked in issue #4088.

/// Resolved Trace Commons credentials for a (tenant, user): which local-state
/// scope to use and the per-user subject (if any) to send to the server.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceCredentialResolution {
    /// The scope string keying the user's local state (queued envelopes,
    /// records, credits). NOTE: it is NOT always where the device key and
    /// enrollment policy live — under instance enrollment (`subject` is
    /// `Some`) those come from the instance scope (`None`); callers select
    /// the device-key dir based on `subject.is_some()`.
    pub state_scope: String,
    /// Per-user subject to send in upload-claim / login-link requests.
    /// `None` for the personal-invite model (device key already 1:1 with user).
    pub subject: Option<String>,
    /// The resolved enrollment policy.
    pub policy: StandingTraceContributionPolicy,
}

/// Inner implementation that reads policies relative to an explicit base dir.
/// Used by `resolve_trace_credentials` (which supplies the real base) and by
/// tests (which supply an isolated tempdir).
fn resolve_trace_credentials_at(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
) -> anyhow::Result<Option<TraceCredentialResolution>> {
    let scope = trace_scope_key(tenant_id, user_id);

    match read_trace_policy_for_scope_if_present_at(base_dir, Some(scope.as_str()))
        .map_err(|e| anyhow::anyhow!("failed to read personal trace policy: {e}"))?
    {
        Some(personal) if personal.enabled => {
            return Ok(Some(TraceCredentialResolution {
                state_scope: scope,
                subject: None,
                policy: personal,
            }));
        }
        // A PRESENT scoped policy with enabled=false is an explicit user
        // opt-out (`traces opt-out`); it must win over the instance fallback.
        Some(_) => return Ok(None),
        // No scoped policy was ever written — the instance fallback applies.
        None => {}
    }

    let instance = read_trace_policy_for_scope_at(base_dir, None)
        .map_err(|e| anyhow::anyhow!("failed to read instance trace policy: {e}"))?;
    if instance.enabled {
        return Ok(Some(TraceCredentialResolution {
            subject: Some(salted_pseudonymous_contributor_id_at(base_dir, &scope)?),
            state_scope: scope,
            policy: instance,
        }));
    }

    Ok(None)
}

/// Explicit per-user Trace Commons opt-out: write (or update) the scope's
/// policy with `enabled = false`, which the resolver treats as an explicit
/// opt-out that blocks the instance fallback for this user — WITHOUT touching
/// the instance-level (scope-`None`) policy. This is the primitive a per-user
/// opt-out surface must use: flipping the root policy would disenroll the
/// entire instance.
pub fn opt_out_user_scope_at(base: &std::path::Path, scope: &str) -> anyhow::Result<()> {
    let mut policy =
        read_trace_policy_for_scope_if_present_at(base, Some(scope))?.unwrap_or_default();
    policy.enabled = false;
    write_trace_policy_for_scope_at(base, Some(scope), &policy)
}

/// [`opt_out_user_scope_at`] against the process base dir.
pub fn opt_out_user_scope(scope: &str) -> anyhow::Result<()> {
    opt_out_user_scope_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

/// Pick the user's own (personal-invite) enrollment when present and enabled,
/// else fall back to the admin-provisioned instance enrollment (scope `None`)
/// with a per-user pseudonymous subject. Returns `None` when neither is
/// enabled — and, importantly, when the user's scoped policy exists with
/// `enabled = false` (an explicit `traces opt-out`), which blocks the
/// instance fallback entirely.
pub fn resolve_trace_credentials(
    tenant_id: &TenantId,
    user_id: &UserId,
) -> anyhow::Result<Option<TraceCredentialResolution>> {
    // Typed at the public boundary so callers can't transpose tenant/user;
    // stringify only when handing off to the dir-parameterised core.
    resolve_trace_credentials_at(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
    )
}

/// The effective enrollment a scope contributes under during an autonomous
/// flush: the policy to gate/submit with, the directory that holds the device
/// key, and the per-user subject (if any) to attribute the upload to.
///
/// This consolidates the flush gate and per-user subject derivation into a
/// single policy-read/path-resolution pass so the two cannot drift — earlier
/// the gate and the subject derivation re-read the same policies independently.
struct EffectiveFlushTarget {
    policy: StandingTraceContributionPolicy,
    device_key_dir: PathBuf,
    subject: Option<String>,
}

/// Inner implementation that reads policies relative to an explicit base dir.
/// Used by `resolve_effective_flush_target` (real base) and by tests (tempdir).
fn resolve_effective_flush_target_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> anyhow::Result<Option<EffectiveFlushTarget>> {
    // Personal-invite enrollment: the per-scope policy is enabled and its device
    // key is already 1:1 with the user, so no explicit subject is needed.
    match read_trace_policy_for_scope_if_present_at(base, scope)
        .map_err(|e| anyhow::anyhow!("failed to read personal trace policy: {e}"))?
    {
        Some(personal) if personal.enabled => {
            return Ok(Some(EffectiveFlushTarget {
                policy: personal,
                device_key_dir: trace_contribution_dir_for_scope_at(base, scope),
                subject: None,
            }));
        }
        // A PRESENT scoped policy with enabled=false is an explicit user
        // opt-out (`traces opt-out`); capture/flush must NOT fall back to the
        // instance enrollment for this scope.
        Some(_) => return Ok(None),
        // No scoped policy was ever written — the instance fallback applies.
        None => {}
    }

    // Instance enrollment: no enabled per-scope policy, but the admin-provisioned
    // instance policy (scope None) is enabled. The device key lives at the shared
    // instance dir and uploads are attributed via a per-user pseudonymous subject.
    let instance = read_trace_policy_for_scope_at(base, None)
        .map_err(|e| anyhow::anyhow!("failed to read instance trace policy: {e}"))?;
    if instance.enabled {
        return Ok(Some(EffectiveFlushTarget {
            policy: instance,
            device_key_dir: trace_contribution_dir_for_scope_at(base, None),
            subject: scope
                .map(|s| salted_pseudonymous_contributor_id_at(base, s))
                .transpose()?,
        }));
    }

    Ok(None)
}

/// Resolve the enrollment a scope contributes under for the autonomous flush
/// path. See [`resolve_effective_flush_target_at`]. Returns `Ok(None)` when the
/// scope is enrolled in neither a personal-invite nor an instance enrollment.
fn resolve_effective_flush_target(
    scope: Option<&str>,
) -> anyhow::Result<Option<EffectiveFlushTarget>> {
    resolve_effective_flush_target_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

/// The effective trace-contribution policy a scope captures under: its own
/// personal-invite policy when enabled, else the admin-provisioned instance
/// policy (scope `None`). Returns `Ok(None)` when the scope is enrolled in
/// neither — i.e. capture must skip. This is the *capture-side* mirror of the
/// flush gate ([`resolve_effective_flush_target`]) so an instance-only-enrolled
/// user's turns are captured (and later flushed) instead of being dropped
/// because their per-user policy is absent/disabled. The returned policy is
/// always enabled.
pub fn resolve_effective_capture_policy(
    scope: Option<&str>,
) -> anyhow::Result<Option<StandingTraceContributionPolicy>> {
    resolve_effective_capture_policy_at(&ironclaw_common::paths::ironclaw_base_dir(), scope)
}

/// Dir-parameterised core for [`resolve_effective_capture_policy`] so tests can
/// use an isolated tempdir instead of the process-global instance scope.
fn resolve_effective_capture_policy_at(
    base: &std::path::Path,
    scope: Option<&str>,
) -> anyhow::Result<Option<StandingTraceContributionPolicy>> {
    Ok(resolve_effective_flush_target_at(base, scope)?.map(|target| target.policy))
}

pub fn mark_trace_credit_notice_due_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    mark_trace_credit_noticed_if_due_at_unlocked(
        scope,
        policy.credit_notice_interval_hours,
        Utc::now(),
    )
}

pub fn trace_credit_notice_due_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    trace_credit_notice_due_for_scope_at_unlocked(
        scope,
        policy.credit_notice_interval_hours,
        Utc::now(),
    )
}

pub fn acknowledge_trace_credit_notice_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    acknowledge_trace_credit_notice_for_scope_at_unlocked(scope, Utc::now())
}

pub fn snooze_trace_credit_notice_for_scope(
    scope: Option<&str>,
    duration: chrono::Duration,
) -> anyhow::Result<Option<CreditSummary>> {
    let now = Utc::now();
    if duration <= chrono::Duration::zero() {
        anyhow::bail!("trace credit notice snooze duration must be positive");
    }
    if duration > chrono::Duration::hours(i64::from(TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS)) {
        anyhow::bail!(
            "trace credit notice snooze duration must be at most {} hours",
            TRACE_CREDIT_NOTICE_MAX_SNOOZE_HOURS
        );
    }
    let snoozed_until = now
        .checked_add_signed(duration)
        .ok_or_else(|| anyhow::anyhow!("trace credit notice snooze deadline is out of range"))?;
    snooze_trace_credit_notice_for_scope_until_at(scope, snoozed_until, now)
}

pub fn snooze_trace_credit_notice_for_scope_until(
    scope: Option<&str>,
    snoozed_until: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    snooze_trace_credit_notice_for_scope_until_at(scope, snoozed_until, Utc::now())
}

pub fn read_trace_credit_notice_outbox_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    read_trace_credit_notice_outbox_for_scope_unlocked(scope)
}

pub fn pending_trace_credit_notice_outbox_items_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    pending_trace_credit_notice_outbox_items_for_scope_at_unlocked(scope, Utc::now())
}

pub fn record_trace_credit_notice_delivery_success_for_scope(
    scope: Option<&str>,
    fingerprint: &str,
    channel: &str,
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    record_trace_credit_notice_delivery_success_for_scope_at_unlocked(
        scope,
        fingerprint,
        channel,
        Utc::now(),
    )
}

pub fn record_trace_credit_notice_delivery_failure_for_scope(
    scope: Option<&str>,
    fingerprint: &str,
    channel: &str,
    error: &str,
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    record_trace_credit_notice_delivery_failure_for_scope_at_unlocked(
        scope,
        fingerprint,
        channel,
        error,
        Utc::now(),
    )
}

fn snooze_trace_credit_notice_for_scope_until_at(
    scope: Option<&str>,
    snoozed_until: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    snooze_trace_credit_notice_for_scope_until_at_unlocked(scope, snoozed_until, now)
}

pub fn queue_trace_envelope_for_scope(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<PathBuf> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    queue_trace_envelope_for_scope_unlocked(scope, envelope)
}

/// Queue an envelope as **held for manual review**: write the envelope into
/// the scope queue and a `ManualReview` hold sidecar carrying `reason`, under
/// a single scope lock. The flush worker skips envelopes that have a hold
/// sidecar, so the trace is durably retained — reviewable and authorizable —
/// without being submitted until the hold is cleared.
pub fn queue_trace_envelope_as_held_for_scope(
    scope: Option<&str>,
    envelope: &TraceContributionEnvelope,
    reason: &str,
) -> anyhow::Result<PathBuf> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let path = queue_trace_envelope_for_scope_unlocked(scope, envelope)?;
    let hold = TraceQueueHold {
        submission_id: envelope.submission_id,
        kind: TraceQueueHoldKind::ManualReview,
        reason: reason.to_string(),
        attempts: 0,
        next_retry_at: None,
    };
    write_trace_queue_hold_sidecar_for_path(&path, &hold)?;
    Ok(path)
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
        holds.push(trace_queue_hold_from_sidecar(submission_id, &sidecar));
    }
    holds.sort_by_key(|hold| hold.submission_id);
    Ok(holds)
}

/// The subset of queue holds that are awaiting user manual review (e.g. a High
/// residual-PII-risk hold). These are the only holds surfaced for the user to
/// authorize; policy/value gates (`PolicyGate`) and transient retry holds
/// (`RetryableSubmissionFailure`) are intentionally excluded.
pub fn manual_review_holds_for_scope(scope: Option<&str>) -> anyhow::Result<Vec<TraceQueueHold>> {
    Ok(retain_manual_review_holds(
        read_trace_queue_holds_for_scope(scope)?,
    ))
}

fn retain_manual_review_holds(holds: Vec<TraceQueueHold>) -> Vec<TraceQueueHold> {
    holds
        .into_iter()
        .filter(|hold| matches!(hold.kind, TraceQueueHoldKind::ManualReview))
        .collect()
}

/// Authorize a held manual-review trace for submission, promoting it as-is.
///
/// Stamps the queued envelope with `manual_review_authorized` (so
/// [`trace_autonomous_eligibility`] submits it past every gate) and removes
/// its `.held.json` sidecar. The envelope rewrite is the durable consent
/// record and happens BEFORE the sidecar removal, so any failure leaves the
/// trace held (fail closed). Returns `Ok(false)` when the submission has no
/// `ManualReview` hold (nothing to authorize); errors only on IO failure.
pub fn authorize_manual_review_hold_for_scope(
    scope: Option<&str>,
    submission_id: Uuid,
) -> anyhow::Result<bool> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let envelope_path = trace_queue_dir(scope).join(format!("{submission_id}.json"));
    if !envelope_path.exists() {
        return Ok(false);
    }
    let Some(sidecar) = read_trace_queue_hold_sidecar_for_envelope(&envelope_path)? else {
        return Ok(false);
    };
    if trace_queue_hold_from_sidecar(submission_id, &sidecar).kind
        != TraceQueueHoldKind::ManualReview
    {
        return Ok(false);
    }

    let mut envelope = load_trace_envelope(&envelope_path)?;
    envelope.manual_review_authorized = true;
    // Consent record first: persist the authorization before clearing the
    // hold, so a crash between the two leaves the trace held, not submitted.
    write_json_file(&envelope_path, &envelope, "authorized trace envelope")?;

    let hold_path = trace_queue_hold_path_for_envelope_path(&envelope_path);
    std::fs::remove_file(&hold_path).map_err(|error| {
        anyhow::anyhow!(
            "failed to remove trace hold sidecar {}: {}",
            hold_path.display(),
            error
        )
    })?;
    Ok(true)
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
    let telemetry = read_trace_queue_telemetry_for_scope_unlocked(scope)?;
    let warnings = trace_queue_warnings_for_scope_unlocked(scope)?;

    let mut held_reason_counts = BTreeMap::new();
    let mut retry_scheduled_count = 0;
    let mut manual_review_hold_count = 0;
    let mut policy_hold_count = 0;
    let mut next_retry_at = None;
    for hold in &holds {
        *held_reason_counts.entry(hold.reason.clone()).or_insert(0) += 1;
        match hold.kind {
            TraceQueueHoldKind::RetryableSubmissionFailure => {
                retry_scheduled_count += 1;
                if let Some(retry_at) = hold.next_retry_at {
                    next_retry_at = Some(
                        next_retry_at.map_or(retry_at, |current| std::cmp::min(current, retry_at)),
                    );
                }
            }
            TraceQueueHoldKind::ManualReview => manual_review_hold_count += 1,
            TraceQueueHoldKind::PolicyGate => policy_hold_count += 1,
        }
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
        retry_scheduled_count,
        manual_review_hold_count,
        policy_hold_count,
        next_retry_at,
        telemetry,
        warnings,
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

fn load_queued_trace_envelope_or_quarantine(
    scope: Option<&str>,
    path: &Path,
    phase: &str,
) -> anyhow::Result<Option<TraceContributionEnvelope>> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read envelope {}: {}", path.display(), e))?;
    match serde_json::from_str(&body) {
        Ok(envelope) => Ok(Some(envelope)),
        Err(error) => {
            let quarantine_path = quarantine_malformed_trace_queue_envelope(scope, path)?;
            tracing::debug!(
                %error,
                path = %path.display(),
                quarantine_path = %quarantine_path.display(),
                phase,
                "Quarantined malformed Trace Commons queue envelope"
            );
            Ok(None)
        }
    }
}

pub fn apply_credit_estimate_to_envelope(envelope: &mut TraceContributionEnvelope) {
    let estimate = estimate_initial_credit(envelope);
    envelope.value.submission_score = estimate.submission_score;
    envelope.value.credit_points_pending = estimate.credit_points_pending;
    envelope.value.explanation = estimate.explanation;
    envelope.value_card.scorecard = estimate.scorecard;
    envelope.value_card.user_visible_explanation = envelope.value.explanation.clone();
}

#[derive(Debug, Clone)]
struct TraceUploadClaimContext {
    trace_id: Option<Uuid>,
    submission_id: Option<Uuid>,
    consent_scopes: Vec<ConsentScope>,
    allowed_uses: Vec<TraceAllowedUse>,
    /// Base directory of the user scope (e.g. `trace_contribution_dir_for_scope(scope)`).
    /// Required for `TraceUploadAuthMode::DeviceKey` — the device key is loaded from
    /// this directory.  `None` for callers that do not have a scope context (legacy
    /// CLI paths, static-token paths) which is fine as long as `auth_mode` is
    /// `WorkloadTokenEnv`.
    scope_dir: Option<PathBuf>,
    /// Per-user pseudonymous subject (from `resolve_trace_credentials`). When
    /// set and auth_mode is DeviceKey, it is sent to the issuer so the minted
    /// claim's principal is per-user under the shared instance device key.
    /// `None` for the personal-invite model (device key already 1:1 with user).
    subject: Option<String>,
}

impl TraceUploadClaimContext {
    fn for_envelope(envelope: &TraceContributionEnvelope) -> Self {
        Self {
            trace_id: Some(envelope.trace_id),
            submission_id: Some(envelope.submission_id),
            consent_scopes: envelope.consent.scopes.clone(),
            allowed_uses: envelope.trace_card.allowed_uses.clone(),
            scope_dir: None,
            subject: None,
        }
    }

    fn for_status_sync() -> Self {
        Self {
            trace_id: None,
            submission_id: None,
            consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject: None,
        }
    }

    fn for_submission_id(submission_id: Uuid) -> Self {
        Self {
            trace_id: None,
            submission_id: Some(submission_id),
            consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject: None,
        }
    }

    /// Attach the scope's base directory so that `DeviceKey` auth mode can
    /// locate the per-tenant keypair.
    fn with_scope_dir(mut self, dir: PathBuf) -> Self {
        self.scope_dir = Some(dir);
        self
    }

    /// Attach the per-user pseudonymous subject from `resolve_trace_credentials`.
    /// For instance-enrolled users this is `local_pseudonymous_contributor_id(scope)`;
    /// for personal-invite enrollment and paths with no user context it is `None`.
    fn with_subject(mut self, subject: Option<String>) -> Self {
        self.subject = subject;
        self
    }

    /// Context for account-management calls (e.g. minting a one-time login
    /// link). No trace or submission identity, no consent scopes — the caller
    /// is not submitting a trace.  Callers should chain `.with_scope_dir()` to
    /// supply the tenant keypair directory when `DeviceKey` auth is active.
    fn for_account(subject: Option<String>) -> Self {
        Self {
            trace_id: None,
            submission_id: None,
            consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject,
        }
    }
}

#[async_trait]
trait TraceUploadCredentialProvider: Send + Sync {
    async fn bearer_token(
        &self,
        policy: &StandingTraceContributionPolicy,
        context: &TraceUploadClaimContext,
        force_refresh: bool,
    ) -> anyhow::Result<String>;
}

struct DefaultTraceUploadCredentialProvider;

struct StaticEnvTraceUploadCredentialProvider<'a> {
    bearer_token_env: &'a str,
}

#[derive(Debug, Clone)]
struct CachedTraceUploadClaim {
    token: String,
    refresh_after: DateTime<Utc>,
}

static TRACE_UPLOAD_CLAIM_CACHE: LazyLock<
    std::sync::Mutex<BTreeMap<String, CachedTraceUploadClaim>>,
> = LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));

#[derive(Debug, Serialize)]
struct TraceUploadClaimIssuerRequest {
    schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audience: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    consent_scopes: Vec<ConsentScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_uses: Vec<TraceAllowedUse>,
    requested_at: DateTime<Utc>,
    /// Pilot invite code mirrored from the standing policy. Server-side
    /// the canonical source is `WorkloadClaims.invite_code` (signed); this
    /// field is sent in the body as forward-compat for a future server
    /// slice that may accept it from either source. Omitted when the
    /// policy has no `upload_token_invite_code` set.
    #[serde(skip_serializing_if = "Option::is_none")]
    invite_code: Option<String>,
    /// Per-user subject; only sent in DeviceKey mode. The server (Slice 0)
    /// derives a per-user principal from it. Omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TraceUploadClaimIssuerResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug)]
struct TraceRemoteRequestFailure {
    status: Option<reqwest::StatusCode>,
    kind: TraceQueueTelemetryFailureKind,
    message: String,
    source: Option<reqwest::Error>,
}

impl TraceRemoteRequestFailure {
    fn request_failed(operation: &'static str, error: reqwest::Error) -> Self {
        let status = error.status();
        let kind = trace_remote_request_failure_kind_for_reqwest_error(&error);
        Self {
            status,
            kind,
            message: format!("{operation} request failed: {error}"),
            source: Some(error),
        }
    }

    fn http_rejection(operation: &'static str, status: reqwest::StatusCode, body: String) -> Self {
        let safe_body = safe_trace_remote_rejection_body(&body);
        let message = if safe_body.is_empty() {
            format!("{operation} rejected by {status}")
        } else {
            format!("{operation} rejected by {status}: {safe_body}")
        };
        let kind = if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            TraceQueueTelemetryFailureKind::Credential
        } else {
            TraceQueueTelemetryFailureKind::HttpRejection
        };
        Self {
            status: Some(status),
            kind,
            message,
            source: None,
        }
    }

    fn auth_rejection(&self) -> bool {
        matches!(
            self.status,
            Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
        )
    }

    fn endpoint_invalid(message: String) -> Self {
        Self {
            status: None,
            kind: TraceQueueTelemetryFailureKind::Endpoint,
            message,
            source: None,
        }
    }

    fn dns_rejected(message: String) -> Self {
        Self {
            status: None,
            kind: TraceQueueTelemetryFailureKind::NetworkDns,
            message,
            source: None,
        }
    }
}

impl std::fmt::Display for TraceRemoteRequestFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TraceRemoteRequestFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|error| error as &(dyn Error + 'static))
    }
}

const TRACE_REMOTE_REJECTION_BODY_MAX_CHARS: usize = 512;

fn safe_trace_remote_rejection_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let sanitized = serde_json::from_str::<Value>(trimmed)
        .map(|value| redact_sensitive_json(&value).to_string())
        .unwrap_or_else(|_| trimmed.split_whitespace().collect::<Vec<_>>().join(" "));
    let mut chars = sanitized.chars();
    let mut bounded = chars
        .by_ref()
        .take(TRACE_REMOTE_REJECTION_BODY_MAX_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        bounded.push_str("...");
    }
    bounded
}

fn trace_remote_request_failure_kind_for_reqwest_error(
    error: &reqwest::Error,
) -> TraceQueueTelemetryFailureKind {
    if let Some(kind) = trace_queue_telemetry_failure_kind_for_error_source(error) {
        return kind;
    }
    let message = error_chain_message(error).to_ascii_lowercase();
    if error.is_timeout()
        || message.contains("timed out")
        || message.contains("timeout")
        || message.contains("deadline elapsed")
    {
        TraceQueueTelemetryFailureKind::NetworkTimeout
    } else if message.contains("network is unreachable")
        || message.contains("no route to host")
        || message.contains("offline")
        || message.contains("internet connection appears to be offline")
    {
        TraceQueueTelemetryFailureKind::NetworkOffline
    } else if message.contains("dns")
        || message.contains("failed to lookup")
        || message.contains("failed to resolve")
        || message.contains("name or service not known")
        || message.contains("nodename nor servname")
    {
        TraceQueueTelemetryFailureKind::NetworkDns
    } else if message.contains("connection refused") || message.contains("refused") {
        TraceQueueTelemetryFailureKind::NetworkConnectionRefused
    } else {
        TraceQueueTelemetryFailureKind::Network
    }
}

fn trace_queue_telemetry_failure_kind_for_error_source(
    error: &(dyn Error + 'static),
) -> Option<TraceQueueTelemetryFailureKind> {
    let mut source = Some(error);
    while let Some(cause) = source {
        if let Some(reqwest_error) = cause.downcast_ref::<reqwest::Error>()
            && reqwest_error.is_timeout()
        {
            return Some(TraceQueueTelemetryFailureKind::NetworkTimeout);
        }
        if let Some(io_error) = cause.downcast_ref::<std::io::Error>()
            && let Some(kind) =
                trace_queue_telemetry_failure_kind_for_io_error_kind(io_error.kind())
        {
            return Some(kind);
        }
        source = cause.source();
    }
    None
}

fn trace_queue_telemetry_failure_kind_for_io_error_kind(
    kind: std::io::ErrorKind,
) -> Option<TraceQueueTelemetryFailureKind> {
    match kind {
        std::io::ErrorKind::TimedOut => Some(TraceQueueTelemetryFailureKind::NetworkTimeout),
        std::io::ErrorKind::ConnectionRefused => {
            Some(TraceQueueTelemetryFailureKind::NetworkConnectionRefused)
        }
        std::io::ErrorKind::AddrNotAvailable
        | std::io::ErrorKind::HostUnreachable
        | std::io::ErrorKind::NetworkDown
        | std::io::ErrorKind::NetworkUnreachable
        | std::io::ErrorKind::NotConnected => Some(TraceQueueTelemetryFailureKind::NetworkOffline),
        std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset => {
            Some(TraceQueueTelemetryFailureKind::Network)
        }
        _ => None,
    }
}

fn error_chain_message(error: &(dyn Error + 'static)) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source {
        messages.push(error.to_string());
        source = error.source();
    }
    messages.join("\n")
}

#[async_trait]
impl TraceUploadCredentialProvider for StaticEnvTraceUploadCredentialProvider<'_> {
    async fn bearer_token(
        &self,
        _policy: &StandingTraceContributionPolicy,
        _context: &TraceUploadClaimContext,
        _force_refresh: bool,
    ) -> anyhow::Result<String> {
        trace_upload_static_env_bearer_token(self.bearer_token_env)
    }
}

#[async_trait]
impl TraceUploadCredentialProvider for DefaultTraceUploadCredentialProvider {
    async fn bearer_token(
        &self,
        policy: &StandingTraceContributionPolicy,
        context: &TraceUploadClaimContext,
        force_refresh: bool,
    ) -> anyhow::Result<String> {
        trace_upload_bearer_token_via(policy, context, force_refresh, None).await
    }
}

/// Sink-aware bearer mint. `sink == Some`: AGENT path — the upload-claim
/// issuer request routes through host `RuntimeHttpEgress` like every other
/// agent-driven network effect. `sink == None`: WORKER/CLI path — the direct
/// hardened reqwest client (unchanged [`DefaultTraceUploadCredentialProvider`]
/// behavior). Sink-based entry points (login-link, account-traces) MUST pass
/// their sink here rather than minting through the default provider, or the
/// claim request silently bypasses the deployment's egress policy.
async fn trace_upload_bearer_token_via(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
    force_refresh: bool,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<String> {
    if policy
        .upload_token_issuer_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
    {
        return trace_upload_issuer_claim_bearer_token(policy, context, force_refresh, sink).await;
    }
    trace_upload_static_env_bearer_token(&policy.bearer_token_env)
}

fn trace_upload_static_env_bearer_token(bearer_token_env: &str) -> anyhow::Result<String> {
    std::env::var(bearer_token_env).map_err(|_| {
        anyhow::anyhow!(
            "{} is not set; refusing to call Trace Commons without explicit API credentials",
            bearer_token_env
        )
    })
}

async fn trace_upload_issuer_claim_bearer_token(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
    force_refresh: bool,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<String> {
    let cache_key = trace_upload_claim_cache_key(policy, context)?;
    if !force_refresh && let Some(cached) = trace_upload_cached_claim(&cache_key, Utc::now()) {
        return Ok(cached);
    }

    let claim = fetch_trace_upload_claim_from_issuer(policy, context, sink).await?;
    if let Some(refresh_after) = trace_upload_claim_refresh_after(&claim, Utc::now()) {
        let mut cache = match TRACE_UPLOAD_CLAIM_CACHE.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        cache.insert(
            cache_key,
            CachedTraceUploadClaim {
                token: claim.access_token.clone(),
                refresh_after,
            },
        );
    }
    Ok(claim.access_token)
}

fn trace_upload_cached_claim(cache_key: &str, now: DateTime<Utc>) -> Option<String> {
    let cache = match TRACE_UPLOAD_CLAIM_CACHE.lock() {
        Ok(cache) => cache,
        Err(poisoned) => poisoned.into_inner(),
    };
    cache
        .get(cache_key)
        .filter(|cached| cached.refresh_after > now)
        .map(|cached| cached.token.clone())
}

fn trace_upload_claim_cache_key(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
) -> anyhow::Result<String> {
    let issuer = policy
        .upload_token_issuer_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Trace Commons upload token issuer URL is not configured"))?
        .trim();
    // Include invite_code in the cache key so rotating it forces a fresh
    // claim fetch (the issuer's mint binds a `policy_label` claim derived
    // from the active allowlist policy; serving a cached token after the
    // operator changed the user's invite_code would mis-attribute traces).
    // Hash the invite code to keep the operator-secret out of the in-memory
    // cache key; distinct raw codes still produce distinct hashes, so cache
    // separation is preserved.
    let invite_code_key = policy
        .upload_token_invite_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|code| format!("sha256:{}", hex::encode(Sha256::digest(code.as_bytes()))))
        .unwrap_or_default();
    // In DeviceKey mode, different scopes within the same tenant would otherwise
    // collide on the same cache key (they share the same issuer/tenant/audience).
    // Include a hash of the scope_dir path to ensure each scope gets its own
    // cached claim.  WorkloadTokenEnv mode has no scope concept so scope_dir is
    // always None there — no change in that path.
    let scope_dir_key = match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => context
            .scope_dir
            .as_ref()
            .map(|p| {
                format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(p.to_string_lossy().as_bytes()))
                )
            })
            .unwrap_or_default(),
        TraceUploadAuthMode::WorkloadTokenEnv => String::new(),
    };
    // Under instance enrollment every user shares the SAME instance device-key
    // dir (scope `None`), so `scope_dir_key` is identical across users — the
    // per-user `subject` is what distinguishes their minted claims. Omitting it
    // would let a claim minted for one subject be served from cache to another,
    // mis-attributing traces / leaking across users.
    //
    // The key MUST hash the EXACT bytes the issuer request sends (see
    // `build_trace_upload_claim_issuer_request`: DeviceKey carries `subject`,
    // WorkloadTokenEnv never does) with a `None`/`Some` discriminator. Trimming
    // or collapsing empties here (the old behavior) let `None`, `Some("")`, and
    // whitespace variants share a key while minting different payloads.
    let payload_subject = match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => context.subject.as_deref(),
        TraceUploadAuthMode::WorkloadTokenEnv => None,
    };
    let subject_key = match payload_subject {
        Some(subject) => format!(
            "some:sha256:{}",
            hex::encode(Sha256::digest(subject.as_bytes()))
        ),
        None => "none".to_string(),
    };
    Ok(format!(
        "{}|tenant={}|audience={}|scopes={}|uses={}|workload_env={}|invite_code={}|scope_dir={}|subject={}",
        issuer,
        policy.upload_token_tenant_id.as_deref().unwrap_or_default(),
        policy.upload_token_audience.as_deref().unwrap_or_default(),
        trace_upload_claim_scope_key(&context.consent_scopes),
        trace_upload_claim_use_key(&context.allowed_uses),
        policy
            .upload_token_workload_token_env
            .as_deref()
            .unwrap_or_default(),
        invite_code_key,
        scope_dir_key,
        subject_key,
    ))
}

fn trace_upload_claim_scope_key(scopes: &[ConsentScope]) -> String {
    scopes
        .iter()
        .map(|scope| format!("{scope:?}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn trace_upload_claim_use_key(uses: &[TraceAllowedUse]) -> String {
    uses.iter()
        .map(|allowed_use| format!("{allowed_use:?}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn trace_upload_claim_refresh_after(
    response: &TraceUploadClaimIssuerResponse,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let expires_at = match response.expires_at {
        Some(expires_at) => expires_at,
        None => {
            let seconds = response.expires_in?;
            if seconds <= 0 {
                return None;
            }
            now.checked_add_signed(chrono::Duration::seconds(seconds))?
        }
    };
    let refresh_after =
        expires_at - chrono::Duration::seconds(TRACE_UPLOAD_CLAIM_REFRESH_SKEW_SECONDS);
    (refresh_after > now).then_some(refresh_after)
}

/// Typed PilotAllowlist refusal labels returned by the upload-claim issuer
/// when its `pilot_allowlist` gate refuses to mint a claim. Parsing into an
/// enum keeps the diagnostic mapping closed: any unknown label falls through
/// to the generic HTTP-status diagnostic, which is what we want for
/// future-extension safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PilotAllowlistRefusal {
    NotMatched,
    InviteCodeMissing,
    Stale,
    Malformed,
}

impl PilotAllowlistRefusal {
    fn from_label(label: &str) -> Option<Self> {
        match label {
            "PilotAllowlistNotMatched" => Some(Self::NotMatched),
            "PilotAllowlistInviteCodeMissing" => Some(Self::InviteCodeMissing),
            "PilotAllowlistStale" => Some(Self::Stale),
            "PilotAllowlistMalformed" => Some(Self::Malformed),
            _ => None,
        }
    }

    fn label_str(&self) -> &'static str {
        match self {
            Self::NotMatched => "PilotAllowlistNotMatched",
            Self::InviteCodeMissing => "PilotAllowlistInviteCodeMissing",
            Self::Stale => "PilotAllowlistStale",
            Self::Malformed => "PilotAllowlistMalformed",
        }
    }

    fn diagnostic(&self) -> &'static str {
        match self {
            Self::InviteCodeMissing => {
                "the workload token did not carry an invite_code claim. \
                 Re-run `ironclaw traces opt-in --upload-token-invite-code <CODE> ...` with the operator-issued code, \
                 or have your operator reissue a workload token that includes it."
            }
            Self::NotMatched => {
                "the invite code hash was not in the issuer's active allowlist. \
                 Confirm the code with your operator; it may have been rotated or revoked."
            }
            Self::Stale => {
                "the issuer's allowlist snapshot is stale and the source has not reloaded successfully. \
                 This is transient on the issuer side — retry after the operator confirms recovery."
            }
            Self::Malformed => {
                "the issuer's allowlist source is failing to parse. \
                 This is an operator-side problem — escalate to the issuer admin."
            }
        }
    }
}

/// Build the `anyhow` error returned when the issuer rejects an upload-claim
/// request with a non-success HTTP status. Factored out so the
/// label-dispatch logic (typed PilotAllowlist diagnostics vs. generic HTTP
/// fallback) is unit-testable without spinning up a full HTTPS issuer.
fn build_trace_upload_claim_http_error(
    issuer_label: &str,
    status: u16,
    body_text: &str,
) -> anyhow::Error {
    let label = parse_trace_upload_claim_error_label(body_text);
    let refusal = label.as_deref().and_then(PilotAllowlistRefusal::from_label);
    if let Some(refusal) = refusal {
        return anyhow::anyhow!(
            "Trace Commons upload claim refused by {} ({}): {} — {}",
            issuer_label,
            status,
            refusal.label_str(),
            refusal.diagnostic(),
        );
    }
    anyhow::anyhow!(
        "failed to fetch Trace Commons upload claim from {}: HTTP {}{}",
        issuer_label,
        status,
        label
            .as_deref()
            .map(|l| format!(" ({l})"))
            .unwrap_or_default(),
    )
}

/// Returns the bearer credential to present to the upload-claim issuer.
///
/// - `TraceUploadAuthMode::DeviceKey`: self-signs a short-lived workload JWT
///   with the local device keypair for the tenant.  The context must carry a
///   `scope_dir`.
/// - `TraceUploadAuthMode::WorkloadTokenEnv`: reads the workload token from
///   the environment variable named in the policy (existing behavior, byte-for-byte
///   identical to the inline block this replaces).
///
/// Returns `Ok(None)` when the policy has no `upload_token_workload_token_env`
/// configured and `auth_mode` is `WorkloadTokenEnv`, which means the caller
/// should proceed without a bearer credential (unauthenticated issuer).
async fn issuer_request_bearer(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
) -> anyhow::Result<Option<String>> {
    match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => {
            let tenant = policy.upload_token_tenant_id.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "device-key auth requires upload_token_tenant_id in the trace policy"
                )
            })?;
            let audience = policy.upload_token_audience.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "device-key auth requires upload_token_audience in the trace policy"
                )
            })?;
            let scope_dir = context.scope_dir.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "device-key auth requires a scope directory on the claim context; \
                     ensure the caller threads the user scope"
                )
            })?;
            let key = crate::onboarding::DeviceKeypair::load_for_tenant(scope_dir, tenant)
                .map_err(|e| anyhow::anyhow!("failed to load device key: {e}"))?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "trace policy is in device-key auth mode but no device key exists \
                         for tenant {tenant}; re-run onboarding"
                    )
                })?;
            Ok(Some(key.sign_workload_jwt(audience).map_err(|e| {
                anyhow::anyhow!("failed to sign workload JWT: {e}")
            })?))
        }
        TraceUploadAuthMode::WorkloadTokenEnv => {
            let Some(env_name) = policy.upload_token_workload_token_env.as_deref() else {
                return Ok(None);
            };
            if env_name.trim().is_empty() {
                return Ok(None);
            }
            let workload_token = std::env::var(env_name).map_err(|_| {
                anyhow::anyhow!(
                    "{} is not set; refusing to fetch Trace Commons upload claim without \
                     workload credentials",
                    env_name
                )
            })?;
            Ok(Some(workload_token))
        }
    }
}

/// Build the JSON body sent to the upload-claim issuer for a claim context.
/// Factored out of `fetch_trace_upload_claim_from_issuer` so the wire shape
/// (skip-serialized empty/None fields) is unit-testable without an HTTPS
/// issuer.
fn build_trace_upload_claim_issuer_request(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
) -> TraceUploadClaimIssuerRequest {
    // In DeviceKey mode the registered device key is the post-invite credential —
    // the server does not expect (and must not receive) an invite_code in the body.
    let invite_code = match policy.auth_mode {
        TraceUploadAuthMode::WorkloadTokenEnv => policy
            .upload_token_invite_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        TraceUploadAuthMode::DeviceKey => None,
    };
    // Per-user subject only applies to the device-key (instance) path; in
    // WorkloadTokenEnv mode the workload token already identifies the principal.
    let subject = match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => context.subject.clone(),
        TraceUploadAuthMode::WorkloadTokenEnv => None,
    };
    TraceUploadClaimIssuerRequest {
        schema_version: "ironclaw.trace_upload_claim_request.v1",
        tenant_id: policy.upload_token_tenant_id.clone(),
        audience: policy.upload_token_audience.clone(),
        trace_id: context.trace_id,
        submission_id: context.submission_id,
        consent_scopes: context.consent_scopes.clone(),
        allowed_uses: context.allowed_uses.clone(),
        requested_at: Utc::now(),
        invite_code,
        subject,
    }
}

/// Host-injected HTTP transport for AGENT-INVOKED Trace Commons contribution
/// writes (upload-claim mint, community-profile PUT/DELETE). When `Some`, these
/// run through the host `RuntimeHttpEgress` pipeline (private-IP filtering,
/// redaction, byte accounting). The background flush/sync worker and the CLI
/// pass `None` and keep their crate-local client (see `trace_remote_http_client`,
/// whose comment justifies why the worker lane intentionally bypasses egress).
#[async_trait]
pub trait ContributionHttpSink: Send + Sync {
    async fn execute(
        &self,
        request: ContributionHttpRequest,
    ) -> Result<ContributionHttpResponse, ContributionHttpError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributionHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

pub struct ContributionHttpRequest {
    pub method: ContributionHttpMethod,
    pub url: String,
    pub bearer_token: Option<String>,
    pub json_body: Option<Vec<u8>>,
    pub response_body_limit: u64,
    pub timeout_ms: u32,
}

#[derive(Debug)]
pub struct ContributionHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub struct ContributionHttpError {
    message: String,
}

impl ContributionHttpError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ContributionHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ContributionHttpError {}

/// Direct-transport [`ContributionHttpSink`] for trusted non-agent surfaces
/// (WebUI facades, CLI). Applies the same hardening as the other direct
/// clients in this module: per-request pinned DNS resolution with
/// private/internal-IP rejection (`resolve_trace_upload_claim_issuer_host`),
/// no redirects, the request's own timeout, and a body read bounded DURING
/// streaming by the request's `response_body_limit`. Agent-path callers must
/// keep using the host-egress sink instead.
/// INVARIANT: request URLs handed to this sink must be derived from the
/// enrolled policy's trust-anchored endpoints (`account_login_links_url`,
/// `account_traces_url`, …) — never from caller/request input. The sink
/// attaches the caller's bearer to whatever URL it is given; keeping it
/// crate-private confines that to the vetted derivations in this module.
pub(crate) struct DirectPinnedContributionSink;

#[async_trait]
impl ContributionHttpSink for DirectPinnedContributionSink {
    async fn execute(
        &self,
        request: ContributionHttpRequest,
    ) -> Result<ContributionHttpResponse, ContributionHttpError> {
        let url = reqwest::Url::parse(&request.url)
            .map_err(|e| ContributionHttpError::new(format!("invalid request URL: {e}")))?;
        let host = url
            .host_str()
            .ok_or_else(|| ContributionHttpError::new("request URL requires a host"))?
            .to_ascii_lowercase();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| ContributionHttpError::new("request URL requires a known port"))?;
        let resolved_addrs = resolve_trace_upload_claim_issuer_host(&host, port)
            .await
            .map_err(|e| ContributionHttpError::new(format!("host resolution rejected: {e}")))?;
        let timeout = Duration::from_millis(u64::from(request.timeout_ms));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(3)))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ironclaw-trace-commons-client")
            .resolve_to_addrs(&host, &resolved_addrs)
            .build()
            .map_err(|e| ContributionHttpError::new(format!("failed to build client: {e}")))?;

        let method = match request.method {
            ContributionHttpMethod::Get => reqwest::Method::GET,
            ContributionHttpMethod::Post => reqwest::Method::POST,
            ContributionHttpMethod::Put => reqwest::Method::PUT,
            ContributionHttpMethod::Delete => reqwest::Method::DELETE,
        };
        let mut builder = client
            .request(method, url)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(token) = request.bearer_token {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = request.json_body {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let mut response = builder
            .send()
            .await
            .map_err(|e| ContributionHttpError::new(format!("request failed: {e}")))?;
        let status = response.status().as_u16();
        // Enforce the cap DURING the chunked read so a hostile server cannot
        // force a large allocation by streaming an oversized body.
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ContributionHttpError::new(format!("response read failed: {e}")))?
        {
            if body.len() as u64 + chunk.len() as u64 > request.response_body_limit {
                return Err(ContributionHttpError::new(format!(
                    "response body exceeds the {} byte limit",
                    request.response_body_limit
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(ContributionHttpResponse { status, body })
    }
}

/// Decode a host-egress response body into a bounded UTF-8 string, capping at
/// `TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES` (the host egress already enforced the
/// limit, but truncating defensively keeps a hostile body bounded). Lossy
/// decoding is acceptable — the body is parsed as JSON or scanned for an error
/// label, never echoed verbatim.
fn bounded_utf8_from_egress_body(mut body: Vec<u8>) -> String {
    body.truncate(TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES);
    String::from_utf8_lossy(&body).into_owned()
}

async fn fetch_trace_upload_claim_from_issuer(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<TraceUploadClaimIssuerResponse> {
    let issuer_url = policy.upload_token_issuer_url.as_deref().ok_or_else(|| {
        anyhow::anyhow!("Trace Commons upload token issuer URL is not configured")
    })?;
    let parsed =
        reqwest::Url::parse(issuer_url).context("invalid Trace Commons upload token issuer URL")?;
    validate_trace_upload_claim_issuer_url(&parsed, &policy.upload_token_issuer_allowed_hosts)?;
    let timeout = trace_upload_claim_issuer_timeout(policy)?;
    let request_body = build_trace_upload_claim_issuer_request(policy, context);
    let issuer_bearer = issuer_request_bearer(policy, context).await?;

    // Both branches converge on `(status, body_text)`, then share the status
    // check + JSON parse + response validation below.
    let (status, body_text): (u16, String) = if let Some(sink) = sink {
        // AGENT path: route through the host RuntimeHttpEgress pipeline. The
        // egress performs its own private-IP filtering and DNS resolution, so
        // this branch does NOT build a reqwest client / resolve_to_addrs.
        let json_body = serde_json::to_vec(&request_body)
            .context("failed to serialize Trace Commons upload claim request body")?;
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let response = sink
            .execute(ContributionHttpRequest {
                method: ContributionHttpMethod::Post,
                url: parsed.to_string(),
                bearer_token: issuer_bearer,
                json_body: Some(json_body),
                response_body_limit: TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
                timeout_ms,
            })
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to fetch Trace Commons upload claim from {}: {}",
                    safe_trace_upload_claim_issuer_url_label(&parsed),
                    error
                )
            })?;
        (
            response.status,
            bounded_utf8_from_egress_body(response.body),
        )
    } else {
        // WORKER/CLI/TEST path: existing crate-local hardened reqwest client,
        // unchanged behavior (pinned DNS, bounded body, no redirects).
        let host = parsed
            .host_str()
            .ok_or_else(|| {
                anyhow::anyhow!("Trace Commons upload token issuer URL requires a host")
            })?
            .to_ascii_lowercase();
        let port = parsed.port_or_known_default().ok_or_else(|| {
            anyhow::anyhow!("Trace Commons upload token issuer URL requires a known port")
        })?;
        let resolved_addrs = resolve_trace_upload_claim_issuer_host(&host, port).await?;
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(3)))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("ironclaw-trace-commons-upload-claim/0.1")
            .resolve_to_addrs(&host, &resolved_addrs)
            .build()
            .context("failed to build Trace Commons upload token issuer HTTP client")?;
        let mut request = client
            .post(parsed.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&request_body);
        if let Some(bearer) = issuer_bearer {
            request = request.bearer_auth(bearer);
        }

        let response = request.send().await.with_context(|| {
            format!(
                "failed to fetch Trace Commons upload claim from {}",
                safe_trace_upload_claim_issuer_url_label(&parsed)
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            // Read the (tiny) error body so the typed-label path below can
            // surface a clear diagnostic; bounded by the shared reader.
            let body_text = read_bounded_trace_upload_claim_response(response, &parsed)
                .await
                .unwrap_or_default();
            (status.as_u16(), body_text)
        } else {
            if let Some(content_length) = response.content_length() {
                anyhow::ensure!(
                    content_length <= TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
                    "Trace Commons upload claim response from {} exceeded {} bytes",
                    safe_trace_upload_claim_issuer_url_label(&parsed),
                    TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES
                );
            }
            let body_text = read_bounded_trace_upload_claim_response(response, &parsed).await?;
            (status.as_u16(), body_text)
        }
    };

    // Shared handling: status check + typed-label error, JSON parse, validation.
    if !(200..300).contains(&status) {
        return Err(build_trace_upload_claim_http_error(
            &safe_trace_upload_claim_issuer_url_label(&parsed),
            status,
            &body_text,
        ));
    }
    let claim: TraceUploadClaimIssuerResponse =
        serde_json::from_str(&body_text).with_context(|| {
            format!(
                "Trace Commons upload claim response from {} was not valid JSON",
                safe_trace_upload_claim_issuer_url_label(&parsed)
            )
        })?;
    validate_trace_upload_claim_response(&claim)?;
    Ok(claim)
}

async fn read_bounded_trace_upload_claim_response(
    mut response: reqwest::Response,
    issuer_url: &reqwest::Url,
) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.with_context(|| {
        format!(
            "failed to read Trace Commons upload claim response from {}",
            safe_trace_upload_claim_issuer_url_label(issuer_url)
        )
    })? {
        // Check the cap BEFORE growing the buffer so an oversized chunk can't
        // push `bytes` past the hard ceiling before the error returns.
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow::anyhow!("Trace Commons upload claim response size overflow"))?;
        anyhow::ensure!(
            next_len <= TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES,
            "Trace Commons upload claim response from {} exceeded {} bytes",
            safe_trace_upload_claim_issuer_url_label(issuer_url),
            TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES
        );
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).with_context(|| {
        format!(
            "Trace Commons upload claim response from {} was not valid UTF-8",
            safe_trace_upload_claim_issuer_url_label(issuer_url)
        )
    })
}

/// Read an account-traces list response body with a hard byte ceiling so the
/// direct path cannot buffer an unbounded body even when the server omits a
/// `Content-Length` (chunked transfer). Mirrors
/// [`read_bounded_trace_upload_claim_response`] with the larger
/// [`ACCOUNT_TRACES_MAX_RESPONSE_BYTES`] cap.
async fn read_bounded_account_traces_response(
    mut response: reqwest::Response,
) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow::anyhow!("failed to read account traces response body: {e}"))?
    {
        // Check the cap BEFORE growing the buffer so an oversized chunk can't
        // push `bytes` past the hard ceiling before the error returns.
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| anyhow::anyhow!("account traces response size overflow"))?;
        anyhow::ensure!(
            next_len <= ACCOUNT_TRACES_MAX_RESPONSE_BYTES,
            "account traces response exceeded {} bytes",
            ACCOUNT_TRACES_MAX_RESPONSE_BYTES
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Parse the issuer's typed error label out of an error response body.
/// The issuer returns `{"error": "<Label>"}` for refusals (see
/// trace-commons-server `IssuerError::into_response`). Returns `None`
/// when the body is empty, not JSON, or doesn't carry an `error` string —
/// the caller falls back to a generic HTTP-status diagnostic in that case.
fn parse_trace_upload_claim_error_label(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    parsed
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn validate_trace_upload_claim_response(
    response: &TraceUploadClaimIssuerResponse,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !response.access_token.trim().is_empty(),
        "Trace Commons upload claim response did not include an access token"
    );
    if let Some(token_type) = response.token_type.as_deref() {
        anyhow::ensure!(
            token_type.eq_ignore_ascii_case("bearer"),
            "Trace Commons upload claim response token_type must be bearer"
        );
    }
    let header = jsonwebtoken::decode_header(response.access_token.trim())
        .context("Trace Commons upload claim access token is not a JWT")?;
    anyhow::ensure!(
        header.alg == jsonwebtoken::Algorithm::EdDSA,
        "Trace Commons upload claim access token must use EdDSA"
    );
    anyhow::ensure!(
        header
            .kid
            .as_deref()
            .is_some_and(|kid| !kid.trim().is_empty()),
        "Trace Commons upload claim access token must include a kid"
    );
    Ok(())
}

fn validate_trace_upload_claim_issuer_url(
    url: &reqwest::Url,
    allowed_hosts: &BTreeSet<String>,
) -> anyhow::Result<()> {
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| anyhow::anyhow!("Trace Commons upload token issuer URL requires a host"))?;
    // Literal-loopback hosts get the same dev exception as the loopback-HTTP
    // invite form in onboarding — otherwise a successful loopback onboarding
    // writes a policy whose claim endpoint can never be used.
    let loopback_dev = crate::onboarding::invite::is_loopback_host(&host);
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback_dev),
        "Trace Commons upload token issuer URL must use https (or http to a loopback host for local dev)"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Trace Commons upload token issuer URL must not include embedded credentials"
    );
    anyhow::ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "Trace Commons upload token issuer URL must not include query strings or fragments"
    );
    if !loopback_dev {
        anyhow::ensure!(
            !is_internal_trace_upload_claim_issuer_hostname(&host),
            "Trace Commons upload token issuer URL must not use localhost or internal hostnames"
        );
        if let Ok(ip) = host.parse::<IpAddr>() {
            anyhow::ensure!(
                !is_disallowed_trace_upload_claim_issuer_ip(ip),
                "Trace Commons upload token issuer URL must not use private, local, or reserved IP addresses"
            );
        }
    }
    anyhow::ensure!(
        !allowed_hosts.is_empty(),
        "Trace Commons upload token issuer URL requires an allowed-host list"
    );
    anyhow::ensure!(
        allowed_hosts.contains(&host),
        "Trace Commons upload token issuer URL host is not allowlisted"
    );
    Ok(())
}

async fn resolve_trace_upload_claim_issuer_host(
    host: &str,
    port: u16,
) -> anyhow::Result<Vec<SocketAddr>> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| {
            format!("failed to resolve Trace Commons upload token issuer host {host}")
        })?
        .collect();
    anyhow::ensure!(
        !addrs.is_empty(),
        "Trace Commons upload token issuer host {host} resolved to no addresses"
    );
    // For a literal-loopback host (the local-dev exception) the pinned
    // resolution must stay on loopback — anything else is DNS tampering.
    let loopback_dev = crate::onboarding::invite::is_loopback_host(host);
    for addr in &addrs {
        if loopback_dev {
            anyhow::ensure!(
                addr.ip().is_loopback(),
                "Trace Commons upload token issuer loopback host {host} resolved to non-loopback address"
            );
            continue;
        }
        anyhow::ensure!(
            !is_disallowed_trace_upload_claim_issuer_ip(addr.ip()),
            "Trace Commons upload token issuer host {host} resolved to disallowed address"
        );
    }
    Ok(addrs)
}

fn is_internal_trace_upload_claim_issuer_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host == "metadata.google.internal"
}

fn is_disallowed_trace_upload_claim_issuer_ip(ip: IpAddr) -> bool {
    match normalize_trace_upload_claim_issuer_ip(ip) {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 0
                || octets[0] == 10
                || octets[0] == 127
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 169 && octets[1] == 254)
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 198 && (18..=19).contains(&octets[1]))
                || octets[0] >= 224
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] & 0xff00) == 0xff00
        }
    }
}

fn normalize_trace_upload_claim_issuer_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
    }
}

fn trace_upload_claim_issuer_timeout(
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<Duration> {
    let timeout_ms = policy.upload_token_issuer_timeout_ms;
    anyhow::ensure!(
        (1..=30_000).contains(&timeout_ms),
        "Trace Commons upload token issuer timeout must be between 1 and 30000 milliseconds"
    );
    Ok(Duration::from_millis(timeout_ms))
}

fn safe_trace_upload_claim_issuer_url_label(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or("<unknown-host>");
    format!("{}://{}", url.scheme(), host)
}

pub const COMMUNITY_PROFILE_HANDLE_MIN_CHARS: usize = 3;
pub const COMMUNITY_PROFILE_HANDLE_MAX_CHARS: usize = 32;
pub const COMMUNITY_PROFILE_BIO_MAX_BYTES: usize = 280;
const COMMUNITY_PROFILE_PATH: &str = "/v1/community/profile";

/// Short-lived claim minted from the upload-claim issuer that authorizes
/// community-profile management only (consent scope `public_attribution`,
/// empty allowed-uses). A claim scoped to only `public_attribution` cannot
/// submit traces — it gates the `/v1/community/profile` endpoints.
#[derive(Debug, Clone)]
pub struct ProfileAttributionToken {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub expires_in: Option<i64>,
}

/// Claim context for the community-profile second opt-in: no trace ids, the
/// `public_attribution` consent scope only, and no allowed uses.
fn profile_attribution_claim_context(scope: Option<&str>) -> TraceUploadClaimContext {
    TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::PublicAttribution],
        allowed_uses: Vec::new(),
        scope_dir: Some(trace_contribution_dir_for_scope(scope)),
        subject: None,
    }
}

/// Build a profile-attribution claim context from an instance-aware
/// [`TraceCredentialResolution`]. The device key lives at the instance scope
/// dir when a pseudonymous `subject` is present (instance enrollment) and at the
/// user scope dir otherwise (personal-invite enrollment) — the same scope_dir /
/// subject selection as `mint_account_login_link_inner`.
fn profile_attribution_claim_context_from_resolution(
    base_dir: &std::path::Path,
    resolution: &TraceCredentialResolution,
) -> TraceUploadClaimContext {
    let scope_dir = if resolution.subject.is_some() {
        trace_contribution_dir_for_scope_at(base_dir, None)
    } else {
        trace_contribution_dir_for_scope_at(base_dir, Some(resolution.state_scope.as_str()))
    };
    TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::PublicAttribution],
        allowed_uses: Vec::new(),
        scope_dir: Some(scope_dir),
        subject: resolution.subject.clone(),
    }
}

/// True when the resolved enrollment is missing the upload-claim issuer URL — a
/// local *precondition* failure (enrollment incomplete), distinct from the
/// transport/backend failures that surface later from the claim mint. Callers
/// check this so a missing URL maps to `EnrollmentIncomplete` while post-check
/// failures map to `Backend`, instead of collapsing everything into one code.
fn upload_claim_issuer_missing(policy: &StandingTraceContributionPolicy) -> bool {
    policy
        .upload_token_issuer_url
        .as_deref()
        .is_none_or(|url| url.trim().is_empty())
}

/// Mint a short-lived profile-attribution token from the configured Trace
/// Commons upload-claim issuer. The token authorizes community-profile
/// management only and cannot submit traces.
pub async fn mint_profile_attribution_token_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<ProfileAttributionToken> {
    let policy = read_trace_policy_for_scope(scope)?;
    mint_profile_attribution_token_with_policy(&policy, scope, None).await
}

/// Agent-invoked variant: routes the upload-claim mint through the host
/// `RuntimeHttpEgress` pipeline via the injected `sink`.
pub async fn mint_profile_attribution_token_for_scope_via_sink(
    scope: Option<&str>,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<ProfileAttributionToken> {
    let policy = read_trace_policy_for_scope(scope)?;
    mint_profile_attribution_token_with_policy(&policy, scope, Some(sink)).await
}

/// Instance-aware variant of [`mint_profile_attribution_token_for_scope_via_sink`]:
/// resolves the caller's enrollment (personal invite OR admin-provisioned
/// instance enrollment) via [`resolve_trace_credentials`], so an instance-only
/// contributor mints under the shared instance device key with a per-user
/// pseudonymous subject rather than being falsely rejected as not enrolled.
pub async fn mint_profile_attribution_token_for_user_via_sink(
    tenant_id: &TenantId,
    user_id: &UserId,
    sink: &dyn ContributionHttpSink,
) -> Result<ProfileAttributionToken, ProfileAttributionError> {
    // Typed at the public boundary; stringify only for the dir-parameterised core.
    mint_profile_attribution_token_for_user_inner(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
        sink,
    )
    .await
}

/// Dir-parameterised core for [`mint_profile_attribution_token_for_user_via_sink`].
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
async fn mint_profile_attribution_token_for_user_inner(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    sink: &dyn ContributionHttpSink,
) -> Result<ProfileAttributionToken, ProfileAttributionError> {
    let resolution = resolve_trace_credentials_at(base_dir, tenant_id, user_id)
        .map_err(ProfileAttributionError::PolicyRead)?
        .ok_or(ProfileAttributionError::NotEnrolled)?;
    // Local precondition: a missing issuer URL is EnrollmentIncomplete.
    if upload_claim_issuer_missing(&resolution.policy) {
        return Err(ProfileAttributionError::EnrollmentIncomplete(
            anyhow::anyhow!("Trace Commons upload-claim issuer URL is not configured"),
        ));
    }
    let context = profile_attribution_claim_context_from_resolution(base_dir, &resolution);
    // Post-precondition failures (issuer transport/status, serde, device-key)
    // are Backend — not "re-run onboarding".
    mint_profile_attribution_token_with_context(&resolution.policy, &context, Some(sink))
        .await
        .map_err(ProfileAttributionError::Backend)
}

async fn mint_profile_attribution_token_with_policy(
    policy: &StandingTraceContributionPolicy,
    scope: Option<&str>,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<ProfileAttributionToken> {
    let context = profile_attribution_claim_context(scope);
    mint_profile_attribution_token_with_context(policy, &context, sink).await
}

/// Mint a profile-attribution token using a prebuilt claim context. Shared by
/// the scope-based (`*_for_scope_*`) and instance-aware (`*_for_user_*`) entry
/// points so the enabled/issuer gates and the issuer round-trip stay in one
/// place regardless of how the context (scope_dir + subject) was derived.
async fn mint_profile_attribution_token_with_context(
    policy: &StandingTraceContributionPolicy,
    context: &TraceUploadClaimContext,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<ProfileAttributionToken> {
    anyhow::ensure!(
        policy.enabled,
        "not enrolled in Trace Commons — onboard first (run the Trace Commons onboarding \
         or `ironclaw traces opt-in`)"
    );
    anyhow::ensure!(
        policy
            .upload_token_issuer_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty()),
        "Trace Commons upload token issuer URL is not configured; re-run onboarding"
    );
    let claim = fetch_trace_upload_claim_from_issuer(policy, context, sink).await?;
    Ok(ProfileAttributionToken {
        access_token: claim.access_token,
        expires_at: claim.expires_at,
        expires_in: claim.expires_in,
    })
}

/// Create or update the public community profile for this scope. Mints a
/// fresh profile-attribution token and PUTs the profile to the Trace Commons
/// community endpoint derived from the policy's ingest URL.
pub async fn set_community_profile_for_scope(
    scope: Option<&str>,
    display_handle: &str,
    bio: Option<&str>,
) -> anyhow::Result<()> {
    set_community_profile_for_scope_inner(scope, display_handle, bio, None).await
}

/// Agent-invoked variant: routes BOTH the upload-claim mint AND the profile PUT
/// through the host `RuntimeHttpEgress` pipeline via the injected `sink`.
pub async fn set_community_profile_for_scope_via_sink(
    scope: Option<&str>,
    display_handle: &str,
    bio: Option<&str>,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<()> {
    set_community_profile_for_scope_inner(scope, display_handle, bio, Some(sink)).await
}

/// Instance-aware variant of [`set_community_profile_for_scope_via_sink`]:
/// resolves the caller's enrollment via [`resolve_trace_credentials`] so an
/// instance-only contributor can publish a community profile under the shared
/// instance device key with a per-user pseudonymous subject.
pub async fn set_community_profile_for_user_via_sink(
    tenant_id: &TenantId,
    user_id: &UserId,
    display_handle: &str,
    bio: Option<&str>,
    sink: &dyn ContributionHttpSink,
) -> Result<(), CommunityProfileError> {
    // Typed at the public boundary; stringify only for the dir-parameterised core.
    set_community_profile_for_user_inner(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
        display_handle,
        bio,
        Some(sink),
    )
    .await
}

/// Dir-parameterised core for [`set_community_profile_for_user_via_sink`].
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
async fn set_community_profile_for_user_inner(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    display_handle: &str,
    bio: Option<&str>,
    sink: Option<&dyn ContributionHttpSink>,
) -> Result<(), CommunityProfileError> {
    let handle = validate_community_profile_handle(display_handle)
        .map_err(|e| CommunityProfileError::InvalidProfile(format!("{e:#}")))?;
    if let Some(bio) = bio {
        validate_community_profile_bio(bio)
            .map_err(|e| CommunityProfileError::InvalidProfile(format!("{e:#}")))?;
    }
    let resolution = resolve_trace_credentials_at(base_dir, tenant_id, user_id)
        .map_err(ProfileAttributionError::PolicyRead)?
        .ok_or(ProfileAttributionError::NotEnrolled)?;
    // Local preconditions (missing ingest URL or issuer URL) are
    // EnrollmentIncomplete; the mint/PUT transport failures below are Backend.
    let url = community_profile_url_from_policy(&resolution.policy)
        .map_err(ProfileAttributionError::EnrollmentIncomplete)?;
    if upload_claim_issuer_missing(&resolution.policy) {
        return Err(
            ProfileAttributionError::EnrollmentIncomplete(anyhow::anyhow!(
                "Trace Commons upload-claim issuer URL is not configured"
            ))
            .into(),
        );
    }
    let context = profile_attribution_claim_context_from_resolution(base_dir, &resolution);
    let token = mint_profile_attribution_token_with_context(&resolution.policy, &context, sink)
        .await
        .map_err(ProfileAttributionError::Backend)?;
    let body = serde_json::json!({
        "display_handle": handle,
        "bio": bio,
    });
    execute_community_profile_request(
        &resolution.policy,
        ContributionHttpMethod::Put,
        url,
        &token.access_token,
        Some(&body),
        sink,
    )
    .await
    .map_err(ProfileAttributionError::Backend)?;
    Ok(())
}

async fn set_community_profile_for_scope_inner(
    scope: Option<&str>,
    display_handle: &str,
    bio: Option<&str>,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<()> {
    let handle = validate_community_profile_handle(display_handle)?;
    if let Some(bio) = bio {
        validate_community_profile_bio(bio)?;
    }
    let policy = read_trace_policy_for_scope(scope)?;
    let url = community_profile_url_from_policy(&policy)?;
    let token = mint_profile_attribution_token_with_policy(&policy, scope, sink).await?;
    let body = serde_json::json!({
        "display_handle": handle,
        "bio": bio,
    });
    execute_community_profile_request(
        &policy,
        ContributionHttpMethod::Put,
        url,
        &token.access_token,
        Some(&body),
        sink,
    )
    .await
}

/// Withdraw the public community profile for this scope (DELETE the profile
/// resource). Mints a fresh profile-attribution token like
/// [`set_community_profile_for_scope`].
pub async fn withdraw_community_profile_for_scope(scope: Option<&str>) -> anyhow::Result<()> {
    let policy = read_trace_policy_for_scope(scope)?;
    let url = community_profile_url_from_policy(&policy)?;
    let token = mint_profile_attribution_token_with_policy(&policy, scope, None).await?;
    execute_community_profile_request(
        &policy,
        ContributionHttpMethod::Delete,
        url,
        &token.access_token,
        None,
        None,
    )
    .await
}

/// Derive the community-profile endpoint from the policy's ingest endpoint,
/// keeping scheme/host/port and replacing only the path. Profile writes are
/// handled by ingest, while upload-claim minting remains issuer-owned.
fn community_profile_url_from_policy(
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<reqwest::Url> {
    let ingest_url = policy
        .ingestion_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("Trace Commons ingest endpoint is not configured; re-run onboarding")
        })?;
    let mut url =
        reqwest::Url::parse(ingest_url).context("invalid Trace Commons ingest endpoint")?;
    validate_trace_commons_ingest_url(&url)?;
    // Preserve any mount prefix on the ingest path (e.g. `/api/v1/traces`),
    // mirroring `trace_submission_status_endpoint`, instead of clobbering the
    // whole path — otherwise a prefixed deployment 404s on profile PUT/DELETE.
    let path = url.path().trim_end_matches('/');
    let new_path = if let Some(prefix) = path.strip_suffix("/v1/traces") {
        format!("{}/v1/community/profile", prefix.trim_end_matches('/'))
    } else if let Some(prefix) = path.strip_suffix("/traces") {
        format!("{}/community/profile", prefix.trim_end_matches('/'))
    } else {
        COMMUNITY_PROFILE_PATH.to_string()
    };
    url.set_path(&new_path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn validate_trace_commons_ingest_url(url: &reqwest::Url) -> anyhow::Result<()> {
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| anyhow::anyhow!("Trace Commons ingest endpoint requires a host"))?;
    // Same literal-loopback dev exception as the claim-issuer validator: a
    // loopback-HTTP invite stores a loopback ingest endpoint in the policy.
    let loopback_dev = crate::onboarding::invite::is_loopback_host(&host);
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback_dev),
        "Trace Commons ingest endpoint must use https (or http to a loopback host for local dev)"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Trace Commons ingest endpoint must not include embedded credentials"
    );
    anyhow::ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "Trace Commons ingest endpoint must not include query strings or fragments"
    );
    if !loopback_dev {
        anyhow::ensure!(
            !is_internal_trace_upload_claim_issuer_hostname(&host),
            "Trace Commons ingest endpoint must not use localhost or internal hostnames"
        );
        if let Ok(ip) = host.parse::<IpAddr>() {
            anyhow::ensure!(
                !is_disallowed_trace_upload_claim_issuer_ip(ip),
                "Trace Commons ingest endpoint must not use private, local, or reserved IP addresses"
            );
        }
    }
    Ok(())
}

fn validate_community_profile_handle(handle: &str) -> anyhow::Result<String> {
    let trimmed = handle.trim();
    anyhow::ensure!(
        trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "community profile handle may only contain ASCII letters, digits, '-' and '_'"
    );
    // All-ASCII at this point, so byte length == character length.
    anyhow::ensure!(
        trimmed.len() >= COMMUNITY_PROFILE_HANDLE_MIN_CHARS,
        "community profile handle must be at least {COMMUNITY_PROFILE_HANDLE_MIN_CHARS} characters"
    );
    anyhow::ensure!(
        trimmed.len() <= COMMUNITY_PROFILE_HANDLE_MAX_CHARS,
        "community profile handle must be at most {COMMUNITY_PROFILE_HANDLE_MAX_CHARS} characters"
    );
    Ok(trimmed.to_string())
}

fn validate_community_profile_bio(bio: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        bio.len() <= COMMUNITY_PROFILE_BIO_MAX_BYTES,
        "community profile bio must be at most {COMMUNITY_PROFILE_BIO_MAX_BYTES} bytes"
    );
    Ok(())
}

/// Build a hardened HTTP client for Trace Commons account-surface requests:
/// pinned DNS resolution against the validated host (private/internal IPs
/// rejected via `resolve_trace_upload_claim_issuer_host`), policy-derived
/// bounded timeouts, and no redirect following — mirroring
/// `fetch_trace_upload_claim_from_issuer`. The pinned resolution closes the
/// DNS-rebinding window between claim validation and the follow-up request.
async fn pinned_trace_commons_http_client(
    policy: &StandingTraceContributionPolicy,
    url: &reqwest::Url,
    user_agent: &str,
) -> anyhow::Result<reqwest::Client> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Trace Commons request URL requires a host"))?
        .to_ascii_lowercase();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("Trace Commons request URL requires a known port"))?;
    let resolved_addrs = resolve_trace_upload_claim_issuer_host(&host, port).await?;
    let timeout = trace_upload_claim_issuer_timeout(policy)?;
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(3)))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(user_agent)
        .resolve_to_addrs(&host, &resolved_addrs)
        .build()
        .context("failed to build pinned Trace Commons HTTP client")
}

/// Build the hardened HTTP client for community-profile requests. See
/// [`pinned_trace_commons_http_client`].
async fn community_profile_http_client(
    policy: &StandingTraceContributionPolicy,
    url: &reqwest::Url,
) -> anyhow::Result<reqwest::Client> {
    pinned_trace_commons_http_client(policy, url, "ironclaw-trace-commons-community-profile/0.1")
        .await
}

fn community_profile_method_label(method: ContributionHttpMethod) -> &'static str {
    match method {
        ContributionHttpMethod::Get => "GET",
        ContributionHttpMethod::Post => "POST",
        ContributionHttpMethod::Put => "PUT",
        ContributionHttpMethod::Delete => "DELETE",
    }
}

/// Send a community-profile request and map non-success statuses to a bounded
/// diagnostic. The bearer token and raw response bodies never appear in
/// errors or logs — only the bounded JSON `error` field, when present.
///
/// `sink == Some`: AGENT path — route through the host `RuntimeHttpEgress`
/// pipeline. `sink == None`: WORKER/CLI path — build the crate-local hardened
/// reqwest client via [`community_profile_http_client`] (unchanged behavior).
async fn execute_community_profile_request(
    policy: &StandingTraceContributionPolicy,
    method: ContributionHttpMethod,
    url: reqwest::Url,
    access_token: &str,
    body: Option<&Value>,
    sink: Option<&dyn ContributionHttpSink>,
) -> anyhow::Result<()> {
    let method_label = community_profile_method_label(method);

    let (status, body_text): (u16, String) = if let Some(sink) = sink {
        let json_body = match body {
            Some(body) => Some(
                serde_json::to_vec(body)
                    .context("failed to serialize Trace Commons community profile request body")?,
            ),
            None => None,
        };
        let timeout = trace_upload_claim_issuer_timeout(policy)?;
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let response = sink
            .execute(ContributionHttpRequest {
                method,
                url: url.to_string(),
                bearer_token: Some(access_token.to_string()),
                json_body,
                response_body_limit: TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
                timeout_ms,
            })
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Trace Commons community profile {} request to {} failed: {}",
                    method_label,
                    safe_trace_upload_claim_issuer_url_label(&url),
                    error
                )
            })?;
        (
            response.status,
            bounded_utf8_from_egress_body(response.body),
        )
    } else {
        let client = community_profile_http_client(policy, &url).await?;
        let reqwest_method = match method {
            ContributionHttpMethod::Get => reqwest::Method::GET,
            ContributionHttpMethod::Post => reqwest::Method::POST,
            ContributionHttpMethod::Put => reqwest::Method::PUT,
            ContributionHttpMethod::Delete => reqwest::Method::DELETE,
        };
        let mut request = client
            .request(reqwest_method, url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .bearer_auth(access_token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.with_context(|| {
            format!(
                "Trace Commons community profile {} request to {} failed",
                method_label,
                safe_trace_upload_claim_issuer_url_label(&url)
            )
        })?;
        let status = response.status();
        if status.is_success() {
            (status.as_u16(), String::new())
        } else {
            let body_text = read_bounded_trace_upload_claim_response(response, &url)
                .await
                .unwrap_or_default(); // silent-ok: error-body read best-effort, status alone is diagnostic
            (status.as_u16(), body_text)
        }
    };

    if !(200..300).contains(&status) {
        let label = parse_trace_upload_claim_error_label(&body_text);
        return Err(anyhow::anyhow!(
            "Trace Commons community profile {} request to {} rejected: HTTP {}{}",
            method_label,
            safe_trace_upload_claim_issuer_url_label(&url),
            status,
            label
                .as_deref()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default(),
        ));
    }
    Ok(())
}

// ── Trace Commons account login links ────────────────────────────────────────

/// A one-time browser login link that lands the contributor in their Trace
/// Commons account.
#[derive(Debug, Clone)]
pub struct AccountLoginLink {
    /// The Trace Commons account identifier the link is scoped to.
    pub account_id: String,
    /// The one-time login URL; typically an `/account/login?code=…` path.
    pub url: String,
}

/// Typed classification of an account login-link failure. The host maps these
/// variants to the user-facing `error_code` contract, so that contract no
/// longer depends on substring-matching upstream error wording. The mint path
/// returns the specific variant at each failure site.
#[derive(Debug, thiserror::Error)]
pub enum AccountLoginLinkError {
    /// No enrollment (personal invite or instance) resolved for the caller.
    #[error("not enrolled in Trace Commons")]
    NotEnrolled,
    /// The local enrollment policy could not be read or parsed.
    #[error("could not read Trace Commons enrollment policy")]
    PolicyRead(#[source] anyhow::Error),
    /// Enrollment is incomplete — the upload-claim issuer URL or the local
    /// device-key state is missing/invalid (both surface from the bearer mint).
    #[error("Trace Commons enrollment is incomplete (issuer URL or device-key state)")]
    EnrollmentIncomplete(#[source] anyhow::Error),
    /// The issuer refused to mint the login link (non-2xx HTTP response).
    #[error("Trace Commons issuer refused the login-link request (HTTP {status})")]
    IssuerRefused { status: u16 },
    /// Any other failure — transport, serialization, or a malformed response.
    #[error("Trace Commons login-link request failed")]
    Backend(#[source] anyhow::Error),
    /// The host could not persist the minted link to local state (host-side
    /// write failure; carried here so the host maps one typed contract).
    #[error("could not write the account login link to local state")]
    LocalStateWrite,
}

/// Typed classification of a profile-attribution token mint failure, shared by
/// the `profile_token` and `profile_set` flows (both mint the same token). The
/// host maps these variants to the user-facing `error_code` contract, so it no
/// longer substring-matches upstream error wording.
#[derive(Debug, thiserror::Error)]
pub enum ProfileAttributionError {
    /// No enrollment (personal invite or instance) resolved for the caller.
    #[error("not enrolled in Trace Commons")]
    NotEnrolled,
    /// The local enrollment policy could not be read or parsed.
    #[error("could not read Trace Commons enrollment policy")]
    PolicyRead(#[source] anyhow::Error),
    /// Enrollment is incomplete — the upload-claim issuer URL or the local
    /// device-key state is missing/invalid (both surface from the token mint).
    #[error("Trace Commons enrollment is incomplete (issuer URL or device-key state)")]
    EnrollmentIncomplete(#[source] anyhow::Error),
    /// Any other failure — transport, serialization, or a rejected request.
    #[error("Trace Commons profile request failed")]
    Backend(#[source] anyhow::Error),
    /// The host could not persist minted state locally (host-side write).
    #[error("could not write the profile token to local state")]
    LocalStateWrite,
}

/// Typed classification of a community-profile publish failure: either the
/// caller-supplied handle/bio is invalid, or the underlying attribution mint /
/// request failed (see [`ProfileAttributionError`]).
#[derive(Debug, thiserror::Error)]
pub enum CommunityProfileError {
    /// The display handle or bio failed validation.
    #[error("invalid community profile: {0}")]
    InvalidProfile(String),
    /// The attribution token mint or the profile request failed.
    #[error(transparent)]
    Attribution(#[from] ProfileAttributionError),
}

/// Extract the API base URL (origin) from the configured upload-claim issuer
/// URL by stripping the `/v1/trace-upload-claim` suffix. Other account API
/// endpoints (`/v1/account/login-links`, `/v1/account/traces`, …) are built on
/// top of this shared origin so the derivation is not duplicated.
fn account_api_base_url(policy: &StandingTraceContributionPolicy) -> anyhow::Result<String> {
    let issuer_url = policy.upload_token_issuer_url.as_deref().ok_or_else(|| {
        anyhow::anyhow!("Trace Commons upload token issuer URL is not configured")
    })?;
    let base = issuer_url
        .trim_end_matches('/')
        .strip_suffix("/v1/trace-upload-claim")
        .ok_or_else(|| {
            anyhow::anyhow!(
                "upload_token_issuer_url does not end in /v1/trace-upload-claim: {issuer_url}"
            )
        })?;
    Ok(base.to_string())
}

/// Derive the account-login-links URL from the configured upload-claim issuer
/// URL. The login-links service lives at the same origin as the issuer; only
/// the path differs: strip `/v1/trace-upload-claim`, append
/// `/v1/account/login-links`.
fn account_login_links_url(policy: &StandingTraceContributionPolicy) -> anyhow::Result<String> {
    Ok(format!(
        "{}/v1/account/login-links",
        account_api_base_url(policy)?
    ))
}

/// Derive the account-traces URL from the configured upload-claim issuer URL.
/// Strip `/v1/trace-upload-claim`, append `/v1/account/traces`.
fn account_traces_url(
    policy: &StandingTraceContributionPolicy,
    limit: Option<usize>,
) -> anyhow::Result<String> {
    let base = account_api_base_url(policy)?;
    // Always send a bounded limit: `None` defaults to ACCOUNT_TRACES_DEFAULT_LIMIT
    // and any explicit value is clamped to [1, ACCOUNT_TRACES_MAX_LIMIT], so no
    // caller can trigger an unbounded server-side history fetch.
    let effective = limit
        .unwrap_or(ACCOUNT_TRACES_DEFAULT_LIMIT)
        .clamp(1, ACCOUNT_TRACES_MAX_LIMIT);
    Ok(format!("{base}/v1/account/traces?limit={effective}"))
}

/// Mint a one-time account login link for the given `(tenant_id, user_id)`.
/// Routes the POST through the caller-supplied `sink` (host egress on the
/// agent path) so the request obeys the deployment's network-egress policy.
///
/// - Resolves the user's Trace Commons credentials; returns an error if the
///   user is not enrolled.
/// - Mints the per-user bearer via `DefaultTraceUploadCredentialProvider`
///   (identical to how submission and profile-attribution flows do it).
/// - POSTs `{ "subject": <subject> }` (field omitted when `subject` is
///   `None`, i.e. personal-invite enrollment) to `/v1/account/login-links`.
/// - Parses the `{ account_id, url }` response into [`AccountLoginLink`].
pub async fn mint_account_login_link_via_sink(
    tenant_id: &TenantId,
    user_id: &UserId,
    sink: &dyn ContributionHttpSink,
) -> Result<AccountLoginLink, AccountLoginLinkError> {
    // Typed at the public boundary so callers can't transpose tenant/user;
    // stringify only when handing off to the dir-parameterised core.
    mint_account_login_link_inner(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
        sink,
    )
    .await
}

/// Direct (non-agent) counterpart to [`mint_account_login_link_via_sink`]
/// for WebUI facades and other trusted product surfaces: mints the one-time
/// login link through the [`DirectPinnedContributionSink`] (pinned DNS,
/// private-IP filtering) instead of a host-egress sink.
///
/// Delivery contract: the link is returned ONLY in the result — it is never
/// persisted to a local delivery file. Hosted multi-tenant users cannot read
/// host files; the caller (an authenticated WebUI response) is the delivery
/// channel. The URL must never be logged or placed on any model-visible
/// surface.
pub async fn mint_account_login_link(
    tenant_id: &TenantId,
    user_id: &UserId,
) -> Result<AccountLoginLink, AccountLoginLinkError> {
    // Typed at the public boundary so callers can't transpose tenant/user;
    // stringify only when handing off to the dir-parameterised core.
    mint_account_login_link_direct(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id.as_str(),
        user_id.as_str(),
    )
    .await
}

/// Dir-parameterised core for [`mint_account_login_link`] (direct path).
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
async fn mint_account_login_link_direct(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
) -> Result<AccountLoginLink, AccountLoginLinkError> {
    mint_account_login_link_inner(base_dir, tenant_id, user_id, &DirectPinnedContributionSink).await
}

/// Dir-parameterised core for [`mint_account_login_link_via_sink`].
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
async fn mint_account_login_link_inner(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    sink: &dyn ContributionHttpSink,
) -> Result<AccountLoginLink, AccountLoginLinkError> {
    let resolution = resolve_trace_credentials_at(base_dir, tenant_id, user_id)
        .map_err(AccountLoginLinkError::PolicyRead)?
        .ok_or(AccountLoginLinkError::NotEnrolled)?;

    // Device key location depends on enrollment type:
    // - Instance enrollment (`subject` is `Some`): the shared device key is at
    //   the instance scope dir (None scope).
    // - Personal-invite enrollment (`subject` is `None`): the user's device key
    //   is at the user scope dir.
    let scope_dir = if resolution.subject.is_some() {
        trace_contribution_dir_for_scope_at(base_dir, None)
    } else {
        trace_contribution_dir_for_scope_at(base_dir, Some(resolution.state_scope.as_str()))
    };

    // Local preconditions FIRST, before any secret/egress work, so incomplete
    // enrollment fails closed with no side effects: a missing issuer URL and a
    // malformed login-links URL are both EnrollmentIncomplete; the claim mint's
    // transport/status/device-key failures below are Backend.
    if upload_claim_issuer_missing(&resolution.policy) {
        return Err(AccountLoginLinkError::EnrollmentIncomplete(
            anyhow::anyhow!("Trace Commons upload-claim issuer URL is not configured"),
        ));
    }
    let url = account_login_links_url(&resolution.policy)
        .map_err(AccountLoginLinkError::EnrollmentIncomplete)?;
    // Parsed once up front: the join base for a relative `url` in the response
    // (its origin is the trust-anchored issuer origin).
    let endpoint_url = reqwest::Url::parse(&url).map_err(|e| {
        AccountLoginLinkError::EnrollmentIncomplete(
            anyhow::Error::new(e).context("login-links URL is not a valid URL"),
        )
    })?;
    let context =
        TraceUploadClaimContext::for_account(resolution.subject.clone()).with_scope_dir(scope_dir);
    // Mint the bearer THROUGH the sink: on the agent path the upload-claim
    // issuer request must route via host RuntimeHttpEgress like the login-link
    // POST below, not the direct reqwest path.
    let bearer = trace_upload_bearer_token_via(&resolution.policy, &context, false, Some(sink))
        .await
        .map_err(AccountLoginLinkError::Backend)?;
    let body = match &resolution.subject {
        Some(s) => serde_json::json!({ "subject": s }),
        None => serde_json::json!({}),
    };
    let body_bytes = serde_json::to_vec(&body).map_err(|e| {
        AccountLoginLinkError::Backend(anyhow::Error::new(e).context("serialize login-link body"))
    })?;
    // Honor the operator-tuned issuer timeout rather than a hardcoded value,
    // matching `execute_community_profile_request`.
    let timeout = trace_upload_claim_issuer_timeout(&resolution.policy)
        .map_err(AccountLoginLinkError::Backend)?;
    let response = sink
        .execute(ContributionHttpRequest {
            method: ContributionHttpMethod::Post,
            url,
            bearer_token: Some(bearer),
            json_body: Some(body_bytes),
            response_body_limit: TRACE_UPLOAD_CLAIM_MAX_RESPONSE_BYTES as u64,
            timeout_ms: u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX),
        })
        .await
        .map_err(|e| {
            AccountLoginLinkError::Backend(anyhow::anyhow!("login-link request failed: {e}"))
        })?;
    if !(200..300).contains(&response.status) {
        return Err(AccountLoginLinkError::IssuerRefused {
            status: response.status,
        });
    }
    let parsed: serde_json::Value = serde_json::from_slice(&response.body).map_err(|e| {
        AccountLoginLinkError::Backend(anyhow::Error::new(e).context("login-link response JSON"))
    })?;
    let account_id = parsed["account_id"]
        .as_str()
        .ok_or_else(|| {
            AccountLoginLinkError::Backend(anyhow::anyhow!(
                "login-link response missing account_id"
            ))
        })?
        .to_string();
    let link_url = parsed["url"]
        .as_str()
        .ok_or_else(|| {
            AccountLoginLinkError::Backend(anyhow::anyhow!("login-link response missing url"))
        })?
        .to_string();
    // The server may return a relative path (e.g. `/account/login?code=…`).
    // Resolve it against the login-links endpoint — whose origin is the
    // trust-anchored issuer origin — so every delivery channel (browser
    // navigation, local delivery file) receives an absolute URL instead of
    // one that would resolve against the WRONG origin (e.g. the IronClaw
    // WebUI's own host).
    let resolved = match reqwest::Url::parse(&link_url) {
        Ok(absolute) => absolute,
        Err(_) => endpoint_url.join(&link_url).map_err(|e| {
            AccountLoginLinkError::Backend(
                anyhow::Error::new(e).context("login-link response url is not resolvable"),
            )
        })?,
    };
    // ORIGIN PIN: the caller navigates an authenticated user's browser to this
    // URL. A hostile or compromised issuer response must not be able to steer
    // that navigation anywhere else — the final URL must stay on the
    // trust-anchored issuer origin (same scheme + host + port as the
    // login-links endpoint; this also excludes non-HTTP(S) schemes such as
    // `javascript:`) and must carry no userinfo.
    let same_origin = resolved.scheme() == endpoint_url.scheme()
        && resolved.host_str() == endpoint_url.host_str()
        && resolved.port_or_known_default() == endpoint_url.port_or_known_default();
    if !same_origin || !resolved.username().is_empty() || resolved.password().is_some() {
        return Err(AccountLoginLinkError::Backend(anyhow::anyhow!(
            "login-link response url is not on the issuer origin"
        )));
    }
    Ok(AccountLoginLink {
        account_id,
        url: resolved.to_string(),
    })
}

// ── Trace Commons account traces ──────────────────────────────────────────────

/// A single submitted trace record as returned by `GET /v1/account/traces`.
/// Only the fields the UI needs are projected here; unknown server fields are
/// ignored via `#[serde(default)]` and `#[serde(deny_unknown_fields)]` is
/// deliberately omitted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccountTraceItem {
    #[serde(default)]
    pub submission_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub credit_points_pending: f32,
    #[serde(default)]
    pub credit_points_final: Option<f32>,
    #[serde(default)]
    pub received_at: Option<String>,
}

/// Agent-path (host-egress sink) counterpart to the direct `fetch_account_traces`.
/// Not yet wired to a first-party capability — retained as the sink-based entry
/// for a future `trace_commons.account_traces` agent capability, mirroring
/// `mint_account_login_link_via_sink`. Covered by unit tests.
///
/// Fetch the list of submitted traces for the given `(tenant_id, user_id)` via
/// the caller-supplied `sink` (host egress on the agent path).
///
/// - Resolves the user's Trace Commons credentials; returns `Ok(vec![])` when
///   the user is not enrolled (lenient zero-state, not an error).
/// - Mints the per-user bearer via `DefaultTraceUploadCredentialProvider`
///   (identical to how submission and profile-attribution flows do it).
/// - GETs `<origin>/v1/account/traces?limit=N` and parses the JSON array into
///   `Vec<AccountTraceItem>`. Non-2xx for an unenrolled/empty case also
///   returns `Ok(vec![])`. Transport failures return `Err`.
pub async fn fetch_account_traces_via_sink(
    tenant_id: &str,
    user_id: &str,
    limit: Option<usize>,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<Vec<AccountTraceItem>> {
    fetch_account_traces_inner(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id,
        user_id,
        limit,
        sink,
    )
    .await
}

/// Fetch the list of submitted traces for the given `(tenant_id, user_id)` using
/// the crate-local hardened reqwest client (the direct/CLI path, no host-egress
/// sink required).
///
/// This is the facade-safe counterpart to [`fetch_account_traces_via_sink`]: it
/// uses the [`pinned_trace_commons_http_client`] (private-IP-filtered, pinned
/// DNS resolution — the same hardening as the upload-claim issuer request), so
/// a rebinding host cannot redirect this bearer-authenticated GET to an
/// internal address, without coupling the caller to a host-egress
/// `ContributionHttpSink`. Use this from WebUI facades and any non-agent
/// surface. Use [`fetch_account_traces_via_sink`] from the agent runtime where
/// all egress must flow through `RuntimeHttpEgress`.
///
/// Returns `Ok(vec![])` when the user is not enrolled, or when the server
/// returns 404 (an enrolled principal with no account/traces yet). Any other
/// non-2xx status and all transport failures return `Err`.
pub async fn fetch_account_traces(
    tenant_id: &str,
    user_id: &str,
    limit: Option<usize>,
) -> anyhow::Result<Vec<AccountTraceItem>> {
    fetch_account_traces_direct(
        ironclaw_common::paths::ironclaw_base_dir().as_path(),
        tenant_id,
        user_id,
        limit,
    )
    .await
}

/// Dir-parameterised core for [`fetch_account_traces`] (direct/CLI path).
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
async fn fetch_account_traces_direct(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    limit: Option<usize>,
) -> anyhow::Result<Vec<AccountTraceItem>> {
    let resolution = match resolve_trace_credentials_at(base_dir, tenant_id, user_id)? {
        Some(r) => r,
        None => return Ok(vec![]),
    };

    let scope_dir = if resolution.subject.is_some() {
        trace_contribution_dir_for_scope_at(base_dir, None)
    } else {
        trace_contribution_dir_for_scope_at(base_dir, Some(resolution.state_scope.as_str()))
    };

    let context =
        TraceUploadClaimContext::for_account(resolution.subject.clone()).with_scope_dir(scope_dir);
    let provider = DefaultTraceUploadCredentialProvider;
    let bearer = provider
        .bearer_token(&resolution.policy, &context, false)
        .await?;
    let url = account_traces_url(&resolution.policy, limit)?;
    let url = reqwest::Url::parse(&url).context("account traces URL is not a valid URL")?;
    // Pinned-DNS, private-IP-filtered client: the bearer minted above must not
    // be attachable to an internal address via DNS rebinding between the claim
    // request and this GET.
    let client =
        pinned_trace_commons_http_client(&resolution.policy, &url, "ironclaw-trace-commons-client")
            .await?;
    let response = client
        .get(url)
        .bearer_auth(&bearer)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("account traces request failed: {e}"))?;
    let status = response.status();
    // 404 means this enrolled principal has no account/traces yet — a legitimate
    // empty state. Any OTHER non-2xx (401/403/429/5xx) is a real failure and must
    // surface as an error so the WebUI boundary renders a sanitized unavailable
    // state rather than a misleading "no traces".
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(vec![]);
    }
    anyhow::ensure!(
        status.is_success(),
        "account traces request returned status {}",
        status.as_u16()
    );
    let body = read_bounded_account_traces_response(response).await?;
    let items: Vec<AccountTraceItem> = serde_json::from_slice(&body)
        .context("account traces response was not a valid JSON array")?;
    Ok(items)
}

/// Dir-parameterised core for [`fetch_account_traces_via_sink`].
/// Accepts an explicit `base_dir` so tests can supply an isolated tempdir.
async fn fetch_account_traces_inner(
    base_dir: &std::path::Path,
    tenant_id: &str,
    user_id: &str,
    limit: Option<usize>,
    sink: &dyn ContributionHttpSink,
) -> anyhow::Result<Vec<AccountTraceItem>> {
    let resolution = match resolve_trace_credentials_at(base_dir, tenant_id, user_id)? {
        Some(r) => r,
        None => return Ok(vec![]),
    };

    let scope_dir = if resolution.subject.is_some() {
        trace_contribution_dir_for_scope_at(base_dir, None)
    } else {
        trace_contribution_dir_for_scope_at(base_dir, Some(resolution.state_scope.as_str()))
    };

    let context =
        TraceUploadClaimContext::for_account(resolution.subject.clone()).with_scope_dir(scope_dir);
    // Mint the bearer THROUGH the sink: on the agent path the upload-claim
    // issuer request must route via host RuntimeHttpEgress like the traces
    // GET below, not the direct reqwest path.
    let bearer =
        trace_upload_bearer_token_via(&resolution.policy, &context, false, Some(sink)).await?;
    let url = account_traces_url(&resolution.policy, limit)?;
    // Honor the operator-tuned issuer timeout rather than a hardcoded value,
    // and cap the body at the account-traces ceiling (a legitimate trace list
    // can exceed the smaller claim-response cap the mint paths use).
    let timeout = trace_upload_claim_issuer_timeout(&resolution.policy)?;
    let response = sink
        .execute(ContributionHttpRequest {
            method: ContributionHttpMethod::Get,
            url,
            bearer_token: Some(bearer),
            json_body: None,
            response_body_limit: ACCOUNT_TRACES_MAX_RESPONSE_BYTES as u64,
            timeout_ms: u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX),
        })
        .await
        .map_err(|e| anyhow::anyhow!("account traces request failed: {e}"))?;
    // 404 = no account/traces yet for this enrolled principal (legitimate empty);
    // any other non-2xx is a real failure and propagates as an error. The host
    // egress already bounded the body via `response_body_limit` above.
    if response.status == 404 {
        return Ok(vec![]);
    }
    anyhow::ensure!(
        (200..300).contains(&response.status),
        "account traces request returned status {}",
        response.status
    );
    let items: Vec<AccountTraceItem> = serde_json::from_slice(&response.body)
        .context("account traces response was not a valid JSON array")?;
    Ok(items)
}

#[cfg(test)]
tokio::task_local! {
    /// Test-only, task-scoped override for the remote-request timeout.
    ///
    /// The timeout was historically configured for tests by setting the
    /// process-global `IRONCLAW_TRACE_REMOTE_REQUEST_TIMEOUT_MS` env var via a
    /// `set_var`/`remove_var` guard. That is a process-global mutation: under
    /// parallel test execution, the short (e.g. 50ms) value set by one timing
    /// test leaked into every other test that built a trace HTTP client on
    /// another thread, causing spurious `operation timed out` failures against
    /// fast local mock servers (and the reverse: the guard's `remove_var`
    /// reverting an in-flight request to the 30s default). A task-local
    /// override is visible only within the awaiting test's own task tree —
    /// `trace_remote_http_client` is called from the same task that runs the
    /// submit `.await` — so it is fully isolated across parallel tests with no
    /// process-global state and no change to production behavior.
    ///
    /// CAVEAT: the override only propagates within the awaiting task's tree. If
    /// a future refactor wraps the HTTP call in `tokio::spawn` (a new task that
    /// does not inherit task-locals), the spawned request would silently bypass
    /// this override and fall back to the env/default timeout.
    static TEST_REMOTE_REQUEST_TIMEOUT_OVERRIDE: Duration;
}

fn trace_remote_request_timeout() -> Duration {
    // Test-only, task-scoped override takes precedence (see task-local docs).
    #[cfg(test)]
    if let Ok(override_timeout) = TEST_REMOTE_REQUEST_TIMEOUT_OVERRIDE.try_with(|t| *t) {
        return override_timeout;
    }
    std::env::var(TRACE_REMOTE_REQUEST_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(TRACE_REMOTE_REQUEST_DEFAULT_TIMEOUT_MS))
}

// Justification — why the background trace-upload / status-sync lane does NOT
// route through host `RuntimeHttpEgress` (unlike the agent-invoked onboard /
// profile_token / profile_set paths, which do):
//
// The host egress pipeline exists to gate *model-driven* external writes — it
// attaches a per-request capability identity + resource scope, an approval-gate
// obligation, and a host-derived credential-injection plan. The trace queue
// flush/sync worker has none of those by construction: it is a durable runtime
// task (`spawn_trace_queue_flush_worker`) that drains the local contribution
// queue for many scopes on a fixed interval, with no model input, no
// per-request capability, and no approval gate. Forcing it through egress would
// require synthesizing a fake capability id + scope and a credential model for a
// gate-less task — added complexity with no security benefit, because this lane
// already: (1) sends only trace envelopes that passed the safety/redaction
// pipeline at capture time (scan-before-storage); (2) targets the user's own
// operator-enrolled ingest endpoint, validated for SSRF/private-IP via
// `validate_trace_commons_ingest_url` with pinned `resolve_to_addrs`; and
// (3) authenticates with the enrolled-policy bearer token, never a model-
// supplied value. So this is an intentional trusted internal lane, not an
// un-gated external-write hole. See PR #4559 discussion.
// In addition to the enrollment-time endpoint validation described above, each
// background request pins its own DNS resolution below
// (`pinned_trace_remote_http_client`), so a host that passed validation at
// enrollment cannot later rebind to a private/internal address and receive the
// bearer-authenticated submit/status/revoke requests.
async fn pinned_trace_remote_http_client(
    endpoint: &str,
) -> Result<reqwest::Client, TraceRemoteRequestFailure> {
    let url = reqwest::Url::parse(endpoint).map_err(|error| {
        TraceRemoteRequestFailure::endpoint_invalid(format!(
            "trace remote endpoint is not a valid URL: {error}"
        ))
    })?;
    let host = url
        .host_str()
        .ok_or_else(|| {
            TraceRemoteRequestFailure::endpoint_invalid(
                "trace remote endpoint requires a host".to_string(),
            )
        })?
        .to_ascii_lowercase();
    let port = url.port_or_known_default().ok_or_else(|| {
        TraceRemoteRequestFailure::endpoint_invalid(
            "trace remote endpoint requires a known port".to_string(),
        )
    })?;
    let resolved_addrs = resolve_trace_upload_claim_issuer_host(&host, port)
        .await
        .map_err(|error| {
            TraceRemoteRequestFailure::dns_rejected(format!(
                "trace remote endpoint host resolution rejected: {error}"
            ))
        })?;
    let timeout = trace_remote_request_timeout();
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("ironclaw-trace-commons-client")
        .resolve_to_addrs(&host, &resolved_addrs)
        .build()
        .map_err(|error| {
            TraceRemoteRequestFailure::request_failed("trace remote HTTP client", error)
        })
}

pub async fn submit_trace_envelope_to_endpoint(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    bearer_token_env: &str,
) -> anyhow::Result<TraceSubmissionReceipt> {
    let provider = StaticEnvTraceUploadCredentialProvider { bearer_token_env };
    let policy = StandingTraceContributionPolicy::default().set_bearer_token_env(bearer_token_env);
    submit_trace_envelope_to_endpoint_with_credential_provider(
        envelope, endpoint, &policy, &provider, None, None,
    )
    .await
}

pub async fn submit_trace_envelope_to_endpoint_with_policy(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<TraceSubmissionReceipt> {
    submit_trace_envelope_to_endpoint_with_credential_provider(
        envelope,
        endpoint,
        policy,
        &DefaultTraceUploadCredentialProvider,
        None,
        None,
    )
    .await
}

async fn submit_trace_envelope_to_endpoint_with_credential_provider(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    policy: &StandingTraceContributionPolicy,
    provider: &dyn TraceUploadCredentialProvider,
    scope_dir: Option<&Path>,
    subject: Option<String>,
) -> anyhow::Result<TraceSubmissionReceipt> {
    let context = {
        let ctx = TraceUploadClaimContext::for_envelope(envelope);
        let ctx = if let Some(dir) = scope_dir {
            ctx.with_scope_dir(dir.to_path_buf())
        } else {
            ctx
        };
        ctx.with_subject(subject)
    };
    let token = provider.bearer_token(policy, &context, false).await?;
    match submit_trace_envelope_to_endpoint_with_token(envelope, endpoint, &token).await {
        Ok(receipt) => Ok(receipt),
        Err(error) if error.auth_rejection() => {
            let refreshed = provider.bearer_token(policy, &context, true).await?;
            submit_trace_envelope_to_endpoint_with_token(envelope, endpoint, &refreshed)
                .await
                .map_err(anyhow::Error::from)
        }
        Err(error) => Err(anyhow::Error::from(error)),
    }
}

async fn submit_trace_envelope_to_endpoint_with_token(
    envelope: &TraceContributionEnvelope,
    endpoint: &str,
    token: &str,
) -> Result<TraceSubmissionReceipt, TraceRemoteRequestFailure> {
    let response = pinned_trace_remote_http_client(endpoint)
        .await?
        .post(endpoint)
        .bearer_auth(token)
        .header("Idempotency-Key", envelope.submission_id.to_string())
        .json(envelope)
        .send()
        .await
        .map_err(|error| TraceRemoteRequestFailure::request_failed("trace submission", error))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(TraceRemoteRequestFailure::http_rejection(
            "trace submission",
            status,
            body,
        ));
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
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        },
    )
}

pub async fn flush_trace_contribution_queue_for_scope(
    scope: Option<&str>,
    limit: usize,
) -> anyhow::Result<TraceQueueFlushReport> {
    flush_trace_contribution_queue_for_scope_with_credential_provider(
        scope,
        limit,
        &DefaultTraceUploadCredentialProvider,
    )
    .await
}

async fn flush_trace_contribution_queue_for_scope_with_credential_provider(
    scope: Option<&str>,
    limit: usize,
    provider: &dyn TraceUploadCredentialProvider,
) -> anyhow::Result<TraceQueueFlushReport> {
    let _guard = lock_trace_scope_for_mutation(scope).await;
    let flush_started_at = Utc::now();
    record_trace_queue_flush_attempt_for_scope_unlocked(scope, flush_started_at)?;

    // Resolve which enrollment this scope contributes under in a single
    // policy-read/path pass. A personal-invite enrollment uses the per-scope
    // policy + per-scope device-key dir + no subject; an instance enrollment
    // (no enabled per-scope policy, but the admin-provisioned instance policy at
    // scope None is enabled) uses the instance policy + instance device-key dir
    // + a per-user pseudonymous subject. `Ok(None)` means unenrolled and the
    // flush aborts, exactly as before.
    let target = match resolve_effective_flush_target(scope) {
        Ok(target) => target,
        Err(error) => {
            record_trace_queue_flush_failure_for_scope_unlocked(scope, &error, flush_started_at)?;
            return Err(error);
        }
    };
    let Some(EffectiveFlushTarget {
        policy,
        device_key_dir: scope_dir,
        subject,
    }) = target
    else {
        let error = anyhow::anyhow!("trace contribution opt-in is disabled");
        record_trace_queue_flush_failure_for_scope_unlocked(scope, &error, flush_started_at)?;
        return Err(error);
    };
    let Some(endpoint) = policy.ingestion_endpoint.as_deref() else {
        let error = anyhow::anyhow!("trace contribution endpoint is not configured");
        record_trace_queue_flush_failure_for_scope_unlocked(scope, &error, flush_started_at)?;
        return Err(error);
    };

    let compaction = match compact_trace_queue_for_scope_unlocked(scope) {
        Ok(report) => report,
        Err(error) => {
            record_trace_queue_flush_failure_for_scope_unlocked(scope, &error, flush_started_at)?;
            return Err(error);
        }
    };
    let mut submitted = 0usize;
    let mut holds = Vec::new();
    let mut had_nonfatal_failure = false;
    for path in queued_trace_envelope_paths_for_scope(scope)?
        .into_iter()
        .take(limit)
    {
        let Some(mut envelope) = load_queued_trace_envelope_or_quarantine(scope, &path, "flush")?
        else {
            had_nonfatal_failure = true;
            continue;
        };
        apply_credit_estimate_to_envelope(&mut envelope);

        match trace_autonomous_eligibility(&envelope, &policy) {
            TraceQueueEligibility::Submit => {
                if let Some(hold) = retry_hold_if_not_due(&path, Utc::now())? {
                    holds.push(hold);
                    continue;
                }
                let receipt = match submit_trace_envelope_to_endpoint_with_credential_provider(
                    &envelope,
                    endpoint,
                    &policy,
                    provider,
                    Some(&scope_dir),
                    subject.clone(),
                )
                .await
                {
                    Ok(receipt) => receipt,
                    Err(error) => {
                        record_trace_queue_retryable_submission_failure_for_scope_unlocked(
                            scope,
                            &error,
                            Utc::now(),
                        )?;
                        had_nonfatal_failure = true;
                        let hold = retry_hold_after_submission_failure(
                            &path,
                            envelope.submission_id,
                            &error,
                            Utc::now(),
                        )?;
                        if let Err(hold_error) =
                            write_trace_queue_hold_sidecar_for_path(&path, &hold)
                        {
                            tracing::debug!(
                                error = %hold_error,
                                submission_id = %envelope.submission_id,
                                "Failed to write retry hold reason for trace submission"
                            );
                        }
                        holds.push(hold);
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
            TraceQueueEligibility::Hold { kind, reason } => {
                let hold = TraceQueueHold {
                    submission_id: envelope.submission_id,
                    kind,
                    reason: safe_trace_queue_hold_reason(&reason),
                    attempts: 0,
                    next_retry_at: None,
                };
                write_trace_queue_hold_sidecar_for_path(&path, &hold)?;
                holds.push(hold);
            }
        }
    }

    // Flush keeps the scoped lock through submission and status-sync network calls
    // so another same-scope flush cannot submit or remove the same queue file.
    // Sync with the SAME resolved target (policy, device-key dir, subject) the
    // submissions above used, so instance-enrolled scopes get their final
    // credit status instead of a per-scope re-read that resolves to a disabled
    // personal policy.
    match sync_remote_trace_submission_records_for_scope_unlocked_with_target(
        scope,
        &policy,
        &scope_dir,
        subject.as_deref(),
        provider,
    )
    .await
    {
        Ok(_) => record_trace_queue_status_sync_success_for_scope_unlocked(scope, Utc::now())?,
        Err(error) => {
            record_trace_queue_status_sync_failure_for_scope_unlocked(scope, &error, Utc::now())?;
            had_nonfatal_failure = true;
            tracing::debug!(%error, "Failed to sync remote Trace Commons credit status");
        }
    }

    let credit_notice =
        mark_trace_credit_noticed_if_due_unlocked(scope, policy.credit_notice_interval_hours)?;
    record_trace_queue_flush_success_for_scope_unlocked(scope, Utc::now(), !had_nonfatal_failure)?;
    Ok(TraceQueueFlushReport {
        submitted,
        held: holds.len(),
        compaction,
        holds,
        credit_notice,
    })
}

pub async fn flush_trace_contribution_queue_worker_tick<I, S>(
    scopes: I,
    limit_per_scope: usize,
) -> anyhow::Result<TraceQueueWorkerReport>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut report = TraceQueueWorkerReport {
        scopes_checked: 0,
        submitted: 0,
        held: 0,
        scope_reports: Vec::new(),
    };
    let mut seen = BTreeSet::new();

    for scope in scopes {
        let scope = scope.as_ref().trim();
        if scope.is_empty() || !seen.insert(scope.to_string()) {
            continue;
        }
        report.scopes_checked += 1;
        let scope_report =
            match flush_trace_contribution_queue_for_scope(Some(scope), limit_per_scope).await {
                Ok(flush) => TraceQueueWorkerScopeReport {
                    scope: scope.to_string(),
                    submitted: flush.submitted,
                    held: flush.held,
                    holds: flush.holds,
                    credit_notice: flush.credit_notice,
                },
                Err(error) => {
                    tracing::debug!(
                        %error,
                        scope_hash = %scope_hash(scope),
                        "Trace Commons queue worker skipped scope"
                    );
                    continue;
                }
            };
        report.submitted += scope_report.submitted;
        report.held += scope_report.held;
        report.scope_reports.push(scope_report);
    }

    Ok(report)
}

pub async fn sync_remote_trace_submission_records_for_scope(
    scope: Option<&str>,
) -> anyhow::Result<usize> {
    sync_remote_trace_submission_records_for_scope_with_credential_provider(
        scope,
        &DefaultTraceUploadCredentialProvider,
    )
    .await
}

async fn sync_remote_trace_submission_records_for_scope_with_credential_provider(
    scope: Option<&str>,
    provider: &dyn TraceUploadCredentialProvider,
) -> anyhow::Result<usize> {
    // Resolve the effective enrollment (personal-invite or instance) the same
    // way the flush does, so instance-enrolled scopes sync with the instance
    // policy + device key + per-user subject instead of a disabled per-scope
    // policy read that would silently return Ok(0).
    let Some(EffectiveFlushTarget {
        policy,
        device_key_dir,
        subject,
    }) = resolve_effective_flush_target(scope)?
    else {
        return Ok(0);
    };
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
    let updates = fetch_trace_submission_statuses_with_credential_provider(
        &status_endpoint,
        &policy,
        provider,
        &submission_ids,
        Some(&device_key_dir),
        subject.as_deref(),
    )
    .await?;
    let _guard = lock_trace_scope_for_mutation(scope).await;
    apply_remote_trace_submission_statuses_for_scope_unlocked(scope, &updates)
}

/// Status-sync core used by the queue flush: syncs the local records of
/// `scope` against the remote, authenticating with the caller-resolved
/// effective flush target (`policy` + `device_key_dir` + `subject`) rather
/// than re-reading the per-scope policy. An instance-enrolled user has no
/// enabled per-scope policy and its device key lives at the instance dir, so
/// re-reading here would silently sync nothing (or with the wrong credential
/// context) right after a successful instance-attributed submission.
async fn sync_remote_trace_submission_records_for_scope_unlocked_with_target(
    scope: Option<&str>,
    policy: &StandingTraceContributionPolicy,
    device_key_dir: &Path,
    subject: Option<&str>,
    provider: &dyn TraceUploadCredentialProvider,
) -> anyhow::Result<usize> {
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
    let updates = fetch_trace_submission_statuses_with_credential_provider(
        &status_endpoint,
        policy,
        provider,
        &submission_ids,
        Some(device_key_dir),
        subject,
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
    let provider = StaticEnvTraceUploadCredentialProvider { bearer_token_env };
    let policy = StandingTraceContributionPolicy::default().set_bearer_token_env(bearer_token_env);
    fetch_trace_submission_statuses_with_credential_provider(
        status_endpoint,
        &policy,
        &provider,
        submission_ids,
        None,
        None,
    )
    .await
}

pub async fn fetch_trace_submission_statuses_with_policy(
    status_endpoint: &str,
    policy: &StandingTraceContributionPolicy,
    submission_ids: &[Uuid],
) -> anyhow::Result<Vec<TraceSubmissionStatusUpdate>> {
    fetch_trace_submission_statuses_with_credential_provider(
        status_endpoint,
        policy,
        &DefaultTraceUploadCredentialProvider,
        submission_ids,
        None,
        None,
    )
    .await
}

async fn fetch_trace_submission_statuses_with_credential_provider(
    status_endpoint: &str,
    policy: &StandingTraceContributionPolicy,
    provider: &dyn TraceUploadCredentialProvider,
    submission_ids: &[Uuid],
    scope_dir: Option<&Path>,
    subject: Option<&str>,
) -> anyhow::Result<Vec<TraceSubmissionStatusUpdate>> {
    let context = {
        let ctx =
            TraceUploadClaimContext::for_status_sync().with_subject(subject.map(str::to_string));
        if let Some(dir) = scope_dir {
            ctx.with_scope_dir(dir.to_path_buf())
        } else {
            ctx
        }
    };
    let mut updates = Vec::new();

    for chunk in submission_ids.chunks(200) {
        let token = provider.bearer_token(policy, &context, false).await?;
        let body =
            match fetch_trace_submission_statuses_chunk_with_token(status_endpoint, chunk, &token)
                .await
            {
                Ok(body) => body,
                Err(error) if error.auth_rejection() => {
                    let refreshed = provider.bearer_token(policy, &context, true).await?;
                    fetch_trace_submission_statuses_chunk_with_token(
                        status_endpoint,
                        chunk,
                        &refreshed,
                    )
                    .await
                    .map_err(anyhow::Error::from)?
                }
                Err(error) => return Err(anyhow::Error::from(error)),
            };
        let mut page: Vec<TraceSubmissionStatusUpdate> = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("failed to parse trace status sync response: {}", e))?;
        updates.append(&mut page);
    }

    Ok(updates)
}

async fn fetch_trace_submission_statuses_chunk_with_token(
    status_endpoint: &str,
    submission_ids: &[Uuid],
    token: &str,
) -> Result<String, TraceRemoteRequestFailure> {
    let response = pinned_trace_remote_http_client(status_endpoint)
        .await?
        .post(status_endpoint)
        .bearer_auth(token)
        .json(&TraceSubmissionStatusRequest {
            submission_ids: submission_ids.to_vec(),
        })
        .send()
        .await
        .map_err(|error| TraceRemoteRequestFailure::request_failed("trace status sync", error))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(TraceRemoteRequestFailure::http_rejection(
            "trace status sync",
            status,
            body,
        ));
    }
    Ok(body)
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
        let explanation = safe_remote_credit_explanation_lines(update);
        let credit_changed = (old_effective_credit - new_effective_credit).abs() > f32::EPSILON;
        let explanation_changed =
            !explanation.is_empty() && record.credit_explanation != explanation;

        let status_changed = record.server_status.as_deref() != Some(update.status.as_str());
        let credit_delta = new_effective_credit - old_effective_credit;

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
            record.credit_notice_state = TraceCreditNoticeState::default();
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
                points_delta: credit_delta,
                reason: sync_reason,
                created_at: now,
            });
            let history_event = LocalTraceSubmissionHistoryEvent {
                event_id: Uuid::new_v4(),
                kind: LocalTraceSubmissionHistoryKind::StatusSync,
                occurred_at: now,
                server_status: Some(update.status.clone()),
                credit_delta,
                delayed_credit_explanation_count: update
                    .delayed_credit_explanations
                    .len()
                    .try_into()
                    .unwrap_or(u32::MAX),
            };
            if !record.history.iter().any(|event| {
                event.kind == history_event.kind
                    && event.server_status == history_event.server_status
                    && (event.credit_delta - history_event.credit_delta).abs() <= f32::EPSILON
                    && event.delayed_credit_explanation_count
                        == history_event.delayed_credit_explanation_count
            }) {
                record.history.push(history_event);
            }
            changed += 1;
        }
    }

    if changed > 0 {
        write_local_trace_records_for_scope(scope, &records)?;
    }
    Ok(changed)
}

fn safe_remote_credit_explanation_lines(update: &TraceSubmissionStatusUpdate) -> Vec<String> {
    update
        .explanation
        .iter()
        .chain(update.delayed_credit_explanations.iter())
        .filter_map(|line| {
            let line = safe_remote_credit_explanation_line(line);
            (!line.is_empty()).then_some(line)
        })
        .take(16)
        .collect()
}

fn safe_remote_credit_explanation_line(line: &str) -> String {
    let normalized = line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if normalized.is_empty() {
        return String::new();
    }
    let (redacted, _) = DeterministicTraceRedactor::default().redact_text(&normalized);
    let redacted = trace_queue_secret_like_reason_regex().replace_all(&redacted, "[REDACTED]");
    let redacted =
        remote_credit_explanation_url_regex().replace_all(&redacted, "[REDACTED:private_url]");
    let redacted = remote_credit_explanation_tenant_ref_regex()
        .replace_all(&redacted, "[REDACTED:tenant_ref]");
    redacted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .chars()
        .take(240)
        .collect()
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

/// The credit projection for one scope: the aggregate report plus the
/// manual-review holds awaiting authorization. This is what the WebUI credits
/// surfaces poll for.
#[derive(Debug, Clone)]
pub struct ScopedCreditView {
    pub report: TraceCreditReport,
    pub manual_review_holds: Vec<TraceQueueHold>,
}

/// Cheap change-detection signature of a scope's on-disk credit inputs.
/// `None` for an absent file. Computing the signature is a couple of `stat`s;
/// reading + parsing the full submissions history is what we avoid.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CreditViewSignature {
    records: Option<(std::time::SystemTime, u64)>,
    holds: u64,
}

struct CreditViewCacheEntry {
    signature: CreditViewSignature,
    view: ScopedCreditView,
}

/// Per-scope memoization of the computed credit view, keyed by the on-disk
/// signature of the inputs. Bounds polling cost to O(new submissions): when the
/// submissions file and held-trace sidecars are unchanged since the last
/// computation (the steady-state polling case), the request is a couple of
/// `stat`s + a clone, NOT a full-history read/parse/aggregate.
static CREDIT_VIEW_CACHE: LazyLock<std::sync::Mutex<HashMap<String, CreditViewCacheEntry>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Hard cap so the cache can't grow one entry per historical caller forever
/// (same bound the trace-queue flush worker observes). Cleared wholesale on
/// overflow — entries are pure memoization and recompute on demand.
const CREDIT_VIEW_CACHE_MAX_SCOPES: usize = 4096;

fn path_change_signature(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    Some((mtime, meta.len()))
}

/// Cheap signature over the scope's `*.held.json` sidecars (manual-review
/// holds): a hash of each sidecar's (name, len, mtime). Scanning the directory
/// entries' metadata is far cheaper than reading + parsing each sidecar.
fn holds_change_signature(scope: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let dir = trace_queue_dir(Some(scope));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut items: Vec<(String, u64, Option<std::time::SystemTime>)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if !name.ends_with(".held.json") {
                return None;
            }
            let meta = entry.metadata().ok();
            Some((
                name,
                meta.as_ref().map(|m| m.len()).unwrap_or(0),
                meta.and_then(|m| m.modified().ok()),
            ))
        })
        .collect();
    // Sort so the signature is order-independent across `read_dir` orderings.
    items.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    items.hash(&mut hasher);
    hasher.finish()
}

fn current_credit_view_signature(scope: &str) -> CreditViewSignature {
    CreditViewSignature {
        records: path_change_signature(&trace_records_path(Some(scope))),
        holds: holds_change_signature(scope),
    }
}

/// Build the scope's credit view, memoized by the on-disk input signature.
///
/// On a cache hit (inputs unchanged since the last computation) this clones the
/// cached view after a couple of `stat`s. On a miss it reads + aggregates the
/// records and re-scans holds once, then caches the result. Genuine read/parse
/// failures propagate (a missing file is already softened to the zero/empty
/// state inside the underlying readers).
pub fn scoped_credit_view(scope: &str) -> anyhow::Result<ScopedCreditView> {
    let signature = current_credit_view_signature(scope);
    {
        let cache = match CREDIT_VIEW_CACHE.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(entry) = cache.get(scope)
            && entry.signature == signature
        {
            return Ok(entry.view.clone());
        }
    }

    // Miss: recompute once.
    let records = read_local_trace_records_for_scope(Some(scope))?;
    let report = trace_credit_report(&records);
    let manual_review_holds = manual_review_holds_for_scope(Some(scope))?;
    let view = ScopedCreditView {
        report,
        manual_review_holds,
    };

    let mut cache = match CREDIT_VIEW_CACHE.lock() {
        Ok(cache) => cache,
        Err(poisoned) => poisoned.into_inner(),
    };
    if cache.len() >= CREDIT_VIEW_CACHE_MAX_SCOPES && !cache.contains_key(scope) {
        cache.clear();
    }
    cache.insert(
        scope.to_string(),
        CreditViewCacheEntry {
            signature,
            view: view.clone(),
        },
    );
    Ok(view)
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
    let provider = StaticEnvTraceUploadCredentialProvider { bearer_token_env };
    let policy = StandingTraceContributionPolicy::default().set_bearer_token_env(bearer_token_env);
    revoke_trace_submission_for_scope_with_credential_provider(
        scope,
        submission_id,
        endpoint,
        &policy,
        &provider,
    )
    .await
}

pub async fn revoke_trace_submission_for_scope_with_policy(
    scope: Option<&str>,
    submission_id: Uuid,
    endpoint: Option<&str>,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    revoke_trace_submission_for_scope_with_credential_provider(
        scope,
        submission_id,
        endpoint,
        policy,
        &DefaultTraceUploadCredentialProvider,
    )
    .await
}

async fn revoke_trace_submission_for_scope_with_credential_provider(
    scope: Option<&str>,
    submission_id: Uuid,
    endpoint: Option<&str>,
    policy: &StandingTraceContributionPolicy,
    provider: &dyn TraceUploadCredentialProvider,
) -> anyhow::Result<()> {
    if let Some(endpoint) = endpoint {
        // Compute the scope's base directory so DeviceKey auth mode can locate
        // the per-tenant keypair when self-signing the revoke request bearer.
        let scope_dir = trace_contribution_dir_for_scope(scope);
        revoke_trace_submission_at_endpoint_with_credential_provider(
            submission_id,
            endpoint,
            policy,
            provider,
            Some(&scope_dir),
        )
        .await?;
    }

    let _guard = lock_trace_scope_for_mutation(scope).await;
    mark_local_trace_revoked_for_scope_unlocked(scope, submission_id)
}

pub async fn revoke_trace_submission_at_endpoint_with_policy(
    submission_id: Uuid,
    endpoint: &str,
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<()> {
    revoke_trace_submission_at_endpoint_with_credential_provider(
        submission_id,
        endpoint,
        policy,
        &DefaultTraceUploadCredentialProvider,
        None,
    )
    .await
}

async fn revoke_trace_submission_at_endpoint_with_credential_provider(
    submission_id: Uuid,
    endpoint: &str,
    policy: &StandingTraceContributionPolicy,
    provider: &dyn TraceUploadCredentialProvider,
    scope_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let context = {
        let ctx = TraceUploadClaimContext::for_submission_id(submission_id);
        if let Some(dir) = scope_dir {
            ctx.with_scope_dir(dir.to_path_buf())
        } else {
            ctx
        }
    };
    let token = provider.bearer_token(policy, &context, false).await?;
    match revoke_trace_submission_at_endpoint_with_token(submission_id, endpoint, &token).await {
        Ok(()) => Ok(()),
        Err(error) if error.auth_rejection() => {
            let refreshed = provider.bearer_token(policy, &context, true).await?;
            revoke_trace_submission_at_endpoint_with_token(submission_id, endpoint, &refreshed)
                .await
                .map_err(anyhow::Error::from)
        }
        Err(error) => Err(anyhow::Error::from(error)),
    }
}

async fn revoke_trace_submission_at_endpoint_with_token(
    submission_id: Uuid,
    endpoint: &str,
    token: &str,
) -> Result<(), TraceRemoteRequestFailure> {
    let response = pinned_trace_remote_http_client(endpoint)
        .await?
        .delete(endpoint)
        .bearer_auth(token)
        .json(&serde_json::json!({ "submission_id": submission_id }))
        .send()
        .await
        .map_err(|error| TraceRemoteRequestFailure::request_failed("trace revocation", error))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(TraceRemoteRequestFailure::http_rejection(
            "trace revocation",
            status,
            body,
        ));
    }
    Ok(())
}

pub fn trace_autonomous_eligibility(
    envelope: &TraceContributionEnvelope,
    policy: &StandingTraceContributionPolicy,
) -> TraceQueueEligibility {
    // Fail closed before any submit path: an envelope with no trace-content
    // allowed-uses (e.g. a `public_attribution`-only consent scope, which
    // grants `allowed_uses = []`) is not submittable — there is no use the
    // remote side would accept it for. Reject it here rather than relying on
    // the remote to bounce it, even ahead of the manual-review bypass (an
    // authorized hold with no allowed-uses still has nothing to submit).
    if envelope.trace_card.allowed_uses.is_empty() {
        return TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::PolicyGate,
            reason: "trace grants no allowed-uses and is not submittable".to_string(),
        };
    }

    // An explicitly user-authorized held trace submits as-is, bypassing every
    // gate (PII manual-review, score, tool-allowlist). The user reviewed the
    // already-redacted trace and accepted its residual risk.
    if envelope.manual_review_authorized {
        return TraceQueueEligibility::Submit;
    }

    if policy.require_manual_approval_when_pii_detected
        && envelope.privacy.residual_pii_risk == ResidualPiiRisk::High
    {
        return TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::ManualReview,
            reason: "manual review required because residual privacy risk is high".to_string(),
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
            kind: TraceQueueHoldKind::PolicyGate,
            reason: "trace does not use any selected auto-submit tools".to_string(),
        };
    }

    if envelope.value.submission_score < policy.min_submission_score {
        return TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::PolicyGate,
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
        kind: TraceQueueHoldKind::PolicyGate,
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
            record.credit_notice_state = TraceCreditNoticeState::default();
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
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
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
    mark_trace_credit_noticed_if_due_at_unlocked(scope, interval_hours, Utc::now())
}

fn mark_trace_credit_noticed_if_due_unlocked(
    scope: Option<&str>,
    interval_hours: u32,
) -> anyhow::Result<Option<CreditSummary>> {
    mark_trace_credit_noticed_if_due_at_unlocked(scope, interval_hours, Utc::now())
}

#[cfg(test)]
fn trace_credit_notice_due_for_scope_at(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    let policy = read_trace_policy_for_scope(scope)?;
    if !policy.enabled || policy.credit_notice_interval_hours == 0 {
        return Ok(None);
    }
    trace_credit_notice_due_for_scope_at_unlocked(scope, policy.credit_notice_interval_hours, now)
}

fn trace_credit_notice_due_for_scope_at_unlocked(
    scope: Option<&str>,
    interval_hours: u32,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    if interval_hours == 0 {
        return Ok(None);
    }

    let records = read_local_trace_records_for_scope(scope)?;
    Ok(
        trace_credit_notice_due_for_records(&records, interval_hours, now)
            .map(|(summary, _fingerprint)| summary),
    )
}

fn mark_trace_credit_noticed_if_due_at_unlocked(
    scope: Option<&str>,
    interval_hours: u32,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    if interval_hours == 0 {
        return Ok(None);
    }

    let mut records = read_local_trace_records_for_scope(scope)?;
    let Some((summary, fingerprint)) =
        trace_credit_notice_due_for_records(&records, interval_hours, now)
    else {
        return Ok(None);
    };
    upsert_trace_credit_notice_outbox_item_unlocked(scope, &summary, &fingerprint, now)?;

    for record in &mut records {
        if trace_record_noticeable(record) {
            record.last_credit_notice_at = Some(now);
            record.credit_notice_state = TraceCreditNoticeState {
                last_presented_at: Some(now),
                acknowledged_at: None,
                snoozed_until: None,
                fingerprint: Some(fingerprint.clone()),
            };
        }
    }
    write_local_trace_records_for_scope(scope, &records)?;
    Ok(Some(summary))
}

fn acknowledge_trace_credit_notice_for_scope_at_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    let mut records = read_local_trace_records_for_scope(scope)?;
    let Some(fingerprint) = trace_credit_notice_fingerprint(&records) else {
        return Ok(None);
    };
    let summary = trace_credit_summary(&records);
    for record in &mut records {
        if trace_record_noticeable(record) {
            record.credit_notice_state = TraceCreditNoticeState {
                last_presented_at: record
                    .credit_notice_state
                    .last_presented_at
                    .or(record.last_credit_notice_at)
                    .or(Some(now)),
                acknowledged_at: Some(now),
                snoozed_until: None,
                fingerprint: Some(fingerprint.clone()),
            };
        }
    }
    mark_trace_credit_notice_outbox_acknowledged_unlocked(scope, &fingerprint, now)?;
    write_local_trace_records_for_scope(scope, &records)?;
    Ok(Some(summary))
}

fn snooze_trace_credit_notice_for_scope_until_at_unlocked(
    scope: Option<&str>,
    snoozed_until: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<CreditSummary>> {
    let mut records = read_local_trace_records_for_scope(scope)?;
    let Some(fingerprint) = trace_credit_notice_fingerprint(&records) else {
        return Ok(None);
    };
    let summary = trace_credit_summary(&records);
    for record in &mut records {
        if trace_record_noticeable(record) {
            record.credit_notice_state = TraceCreditNoticeState {
                last_presented_at: record
                    .credit_notice_state
                    .last_presented_at
                    .or(record.last_credit_notice_at)
                    .or(Some(now)),
                acknowledged_at: None,
                snoozed_until: Some(snoozed_until),
                fingerprint: Some(fingerprint.clone()),
            };
        }
    }
    mark_trace_credit_notice_outbox_snoozed_unlocked(scope, &fingerprint, snoozed_until, now)?;
    write_local_trace_records_for_scope(scope, &records)?;
    Ok(Some(summary))
}

fn trace_credit_notice_due_for_records(
    records: &[LocalTraceSubmissionRecord],
    interval_hours: u32,
    now: DateTime<Utc>,
) -> Option<(CreditSummary, String)> {
    let fingerprint = trace_credit_notice_fingerprint(records)?;
    let noticeable = records
        .iter()
        .filter(|record| trace_record_noticeable(record))
        .collect::<Vec<_>>();
    if noticeable.is_empty() {
        return None;
    }

    let all_acknowledged = noticeable.iter().all(|record| {
        record.credit_notice_state.fingerprint.as_deref() == Some(fingerprint.as_str())
            && record.credit_notice_state.acknowledged_at.is_some()
    });
    if all_acknowledged {
        return None;
    }

    let all_snoozed = noticeable.iter().all(|record| {
        record.credit_notice_state.fingerprint.as_deref() == Some(fingerprint.as_str())
            && record
                .credit_notice_state
                .snoozed_until
                .is_some_and(|snoozed_until| snoozed_until > now)
    });
    if all_snoozed {
        return None;
    }

    let interval = chrono::Duration::hours(i64::from(interval_hours));
    let notice_due = noticeable.iter().any(|record| {
        if record.credit_notice_state.fingerprint.as_deref() != Some(fingerprint.as_str()) {
            return record
                .last_credit_notice_at
                .map(|last_notice| now.signed_duration_since(last_notice) >= interval)
                .unwrap_or(true);
        }
        if record
            .credit_notice_state
            .snoozed_until
            .is_some_and(|snoozed_until| snoozed_until <= now)
        {
            return true;
        }
        record
            .credit_notice_state
            .last_presented_at
            .or(record.last_credit_notice_at)
            .map(|last_notice| now.signed_duration_since(last_notice) >= interval)
            .unwrap_or(true)
    });

    if notice_due {
        Some((trace_credit_summary(records), fingerprint))
    } else {
        None
    }
}

fn trace_credit_notice_fingerprint(records: &[LocalTraceSubmissionRecord]) -> Option<String> {
    let mut parts = Vec::new();
    for record in records
        .iter()
        .filter(|record| trace_record_noticeable(record))
    {
        let mut events = record
            .credit_events
            .iter()
            .map(|event| {
                format!(
                    "{}:{:?}:{:.6}:{}",
                    event.event_id,
                    event.kind,
                    event.points_delta,
                    event.created_at.timestamp_millis()
                )
            })
            .collect::<Vec<_>>();
        events.sort();
        parts.push(format!(
            "{}|{}|{}|{:.6}|{}|{}",
            record.submission_id,
            record.status.as_str(),
            record.server_status.as_deref().unwrap_or_default(),
            record.credit_points_pending,
            record
                .credit_points_final
                .map(|points| format!("{points:.6}"))
                .unwrap_or_default(),
            events.join(",")
        ));
    }
    if parts.is_empty() {
        return None;
    }
    parts.sort();
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    Some(format!("sha256:{}", hex::encode(&hasher.finalize()[..16]))) // safety: slicing the fixed-size SHA-256 byte array.
}

fn upsert_trace_credit_notice_outbox_item_unlocked(
    scope: Option<&str>,
    summary: &CreditSummary,
    fingerprint: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let mut outbox = read_trace_credit_notice_outbox_for_scope_unlocked(scope)?;
    let message = trace_credit_notice_message(summary);
    if let Some(item) = outbox
        .iter_mut()
        .find(|item| item.fingerprint == fingerprint)
    {
        item.summary = summary.clone();
        item.message = message;
        item.updated_at = now;
        if item.status != TraceCreditNoticeOutboxStatus::Acknowledged {
            item.status = TraceCreditNoticeOutboxStatus::Pending;
            item.next_attempt_at = None;
            item.snoozed_until = None;
        }
    } else {
        outbox.push(TraceCreditNoticeOutboxItem {
            notice_id: trace_credit_notice_outbox_id(fingerprint),
            fingerprint: fingerprint.to_string(),
            summary: summary.clone(),
            message,
            status: TraceCreditNoticeOutboxStatus::Pending,
            created_at: now,
            updated_at: now,
            last_attempt_at: None,
            delivered_at: None,
            next_attempt_at: None,
            snoozed_until: None,
            attempt_count: 0,
            delivery_attempts: Vec::new(),
        });
    }
    write_trace_credit_notice_outbox_for_scope_unlocked(scope, &outbox)
}

fn mark_trace_credit_notice_outbox_acknowledged_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    update_trace_credit_notice_outbox_item_unlocked(scope, fingerprint, |item| {
        item.status = TraceCreditNoticeOutboxStatus::Acknowledged;
        item.updated_at = now;
        item.next_attempt_at = None;
        item.snoozed_until = None;
    })
    .map(|_| ())
}

fn mark_trace_credit_notice_outbox_snoozed_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    snoozed_until: DateTime<Utc>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    update_trace_credit_notice_outbox_item_unlocked(scope, fingerprint, |item| {
        item.status = TraceCreditNoticeOutboxStatus::Snoozed;
        item.updated_at = now;
        item.next_attempt_at = Some(snoozed_until);
        item.snoozed_until = Some(snoozed_until);
    })
    .map(|_| ())
}

fn read_trace_credit_notice_outbox_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    let path = trace_credit_notice_outbox_path(scope);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "failed to read trace credit notice outbox {}: {}",
            path.display(),
            e
        )
    })?;
    serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse trace credit notice outbox {}: {}",
            path.display(),
            e
        )
    })
}

fn write_trace_credit_notice_outbox_for_scope_unlocked(
    scope: Option<&str>,
    outbox: &[TraceCreditNoticeOutboxItem],
) -> anyhow::Result<()> {
    write_json_file(
        &trace_credit_notice_outbox_path(scope),
        outbox,
        "trace credit notice outbox",
    )
}

#[cfg(test)]
fn pending_trace_credit_notice_outbox_items_for_scope_at(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    let _guard = lock_trace_scope_for_mutation_blocking(scope);
    pending_trace_credit_notice_outbox_items_for_scope_at_unlocked(scope, now)
}

fn pending_trace_credit_notice_outbox_items_for_scope_at_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<TraceCreditNoticeOutboxItem>> {
    Ok(read_trace_credit_notice_outbox_for_scope_unlocked(scope)?
        .into_iter()
        .filter(|item| trace_credit_notice_outbox_item_due(item, now))
        .collect())
}

fn trace_credit_notice_outbox_item_due(
    item: &TraceCreditNoticeOutboxItem,
    now: DateTime<Utc>,
) -> bool {
    match item.status {
        TraceCreditNoticeOutboxStatus::Pending => item
            .next_attempt_at
            .map(|next_attempt_at| next_attempt_at <= now)
            .unwrap_or(true),
        TraceCreditNoticeOutboxStatus::Snoozed => item
            .snoozed_until
            .map(|snoozed_until| snoozed_until <= now)
            .unwrap_or_else(|| {
                item.next_attempt_at
                    .map(|next_attempt_at| next_attempt_at <= now)
                    .unwrap_or(true)
            }),
        TraceCreditNoticeOutboxStatus::Delivered | TraceCreditNoticeOutboxStatus::Acknowledged => {
            false
        }
    }
}

fn record_trace_credit_notice_delivery_success_for_scope_at_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    channel: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    update_trace_credit_notice_outbox_item_unlocked(scope, fingerprint, |item| {
        push_trace_credit_notice_delivery_attempt(
            item,
            TraceCreditNoticeDeliveryAttempt {
                channel: safe_trace_credit_notice_channel(channel),
                attempted_at: now,
                succeeded: true,
                error_kind: None,
                error_hash: None,
            },
        );
        item.status = TraceCreditNoticeOutboxStatus::Delivered;
        item.updated_at = now;
        item.last_attempt_at = Some(now);
        item.delivered_at = Some(now);
        item.next_attempt_at = None;
        item.snoozed_until = None;
        item.attempt_count = item.attempt_count.saturating_add(1);
    })
}

fn record_trace_credit_notice_delivery_failure_for_scope_at_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    channel: &str,
    error: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    let error_hash = trace_credit_notice_delivery_error_hash(error);
    let error_kind = trace_credit_notice_delivery_error_kind(error);
    update_trace_credit_notice_outbox_item_unlocked(scope, fingerprint, |item| {
        let next_attempt_count = item.attempt_count.saturating_add(1);
        push_trace_credit_notice_delivery_attempt(
            item,
            TraceCreditNoticeDeliveryAttempt {
                channel: safe_trace_credit_notice_channel(channel),
                attempted_at: now,
                succeeded: false,
                error_kind: Some(error_kind),
                error_hash: Some(error_hash.clone()),
            },
        );
        item.status = TraceCreditNoticeOutboxStatus::Pending;
        item.updated_at = now;
        item.last_attempt_at = Some(now);
        item.delivered_at = None;
        item.next_attempt_at = Some(trace_queue_next_retry_at(now, next_attempt_count));
        item.snoozed_until = None;
        item.attempt_count = next_attempt_count;
    })
}

fn update_trace_credit_notice_outbox_item_unlocked(
    scope: Option<&str>,
    fingerprint: &str,
    mut update: impl FnMut(&mut TraceCreditNoticeOutboxItem),
) -> anyhow::Result<Option<TraceCreditNoticeOutboxItem>> {
    let mut outbox = read_trace_credit_notice_outbox_for_scope_unlocked(scope)?;
    let mut updated = None;
    if let Some(item) = outbox
        .iter_mut()
        .find(|item| item.fingerprint == fingerprint)
    {
        update(item);
        updated = Some(item.clone());
    }
    if updated.is_some() {
        write_trace_credit_notice_outbox_for_scope_unlocked(scope, &outbox)?;
    }
    Ok(updated)
}

fn push_trace_credit_notice_delivery_attempt(
    item: &mut TraceCreditNoticeOutboxItem,
    attempt: TraceCreditNoticeDeliveryAttempt,
) {
    item.delivery_attempts.push(attempt);
    let excess = item
        .delivery_attempts
        .len()
        .saturating_sub(TRACE_CREDIT_NOTICE_OUTBOX_MAX_ATTEMPTS_STORED);
    if excess > 0 {
        item.delivery_attempts.drain(0..excess);
    }
}

fn trace_credit_notice_outbox_id(fingerprint: &str) -> String {
    canonical_hash(&format!("trace_credit_notice:{fingerprint}"))
}

fn safe_trace_credit_notice_channel(channel: &str) -> String {
    let sanitized = channel
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_' | '.'))
        .take(64)
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn trace_credit_notice_delivery_error_hash(error: &str) -> String {
    let digest = Sha256::digest(error.as_bytes());
    format!("sha256:{}", hex::encode(&digest[..8])) // safety: slicing the fixed-size SHA-256 byte array.
}

fn trace_credit_notice_delivery_error_kind(error: &str) -> TraceQueueTelemetryFailureKind {
    let error = anyhow::anyhow!(error.to_string());
    trace_queue_telemetry_failure_kind(&error)
}

struct TraceQueueCompactionCandidate {
    path: PathBuf,
    envelope: TraceContributionEnvelope,
    hold: Option<TraceQueueHold>,
}

fn compact_trace_queue_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<TraceQueueCompactionReport> {
    let paths = queued_trace_envelope_paths_for_scope(scope)?;
    let mut report = TraceQueueCompactionReport::default().set_scanned_count(paths.len() as u32);
    let mut candidates = Vec::new();
    for path in paths {
        let Some(envelope) = load_queued_trace_envelope_or_quarantine(scope, &path, "compaction")?
        else {
            report.malformed_envelopes_quarantined =
                report.malformed_envelopes_quarantined.saturating_add(1);
            continue;
        };
        let hold = read_trace_queue_hold_sidecar_for_envelope(&path)
            .ok()
            .flatten()
            .and_then(|sidecar| {
                trace_queue_submission_id_from_envelope_path(&path)
                    .map(|submission_id| trace_queue_hold_from_sidecar(submission_id, &sidecar))
            });
        candidates.push(TraceQueueCompactionCandidate {
            path,
            envelope,
            hold,
        });
    }

    let mut by_key: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        by_key
            .entry(trace_queue_dedupe_key(&candidate.envelope))
            .or_default()
            .push(index);
    }

    let mut remove_paths = BTreeSet::new();
    for indexes in by_key.values() {
        if indexes.len() < 2 {
            continue;
        }
        let Some(keep) = indexes
            .iter()
            .copied()
            .max_by_key(|index| trace_queue_compaction_rank(&candidates[*index]))
        else {
            continue;
        };
        for index in indexes.iter().copied() {
            if index != keep {
                remove_paths.insert(candidates[index].path.clone());
            }
        }
    }

    for path in &remove_paths {
        let hold_path = trace_queue_hold_path_for_envelope_path(path);
        if hold_path.exists() {
            std::fs::remove_file(&hold_path).map_err(|e| {
                anyhow::anyhow!(
                    "failed to remove duplicate queue hold {}: {}",
                    hold_path.display(),
                    e
                )
            })?;
        }
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| {
                anyhow::anyhow!(
                    "failed to remove duplicate queue envelope {}: {}",
                    path.display(),
                    e
                )
            })?;
            report.duplicate_envelopes_removed =
                report.duplicate_envelopes_removed.saturating_add(1);
        }
    }

    let dir = trace_queue_dir(scope);
    if dir.exists() {
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
                continue;
            };
            let envelope_path = dir.join(format!("{submission_id}.json"));
            if !envelope_path.exists() {
                std::fs::remove_file(&path).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to remove orphan queue hold {}: {}",
                        path.display(),
                        e
                    )
                })?;
                report.orphan_hold_sidecars_removed =
                    report.orphan_hold_sidecars_removed.saturating_add(1);
            }
        }
    }

    record_trace_queue_compaction_for_scope_unlocked(scope, &report, Utc::now())?;
    Ok(report)
}

fn trace_queue_dedupe_key(envelope: &TraceContributionEnvelope) -> String {
    let mut value = match serde_json::to_value(envelope) {
        Ok(value) => value,
        Err(_) => {
            return canonical_hash(&format!(
                "unserializable:{}:{}",
                envelope.trace_id, envelope.submission_id
            ));
        }
    };
    if let Value::Object(object) = &mut value {
        object.remove("submission_id");
        object.remove("created_at");
    }
    match serde_json::to_string(&value) {
        Ok(canonical) => canonical_hash(&canonical),
        Err(_) => canonical_hash(&format!(
            "unserializable:{}:{}",
            envelope.trace_id, envelope.submission_id
        )),
    }
}

fn trace_queue_compaction_rank(candidate: &TraceQueueCompactionCandidate) -> (u8, u32, i64, i64) {
    let hold_rank = candidate.hold.as_ref().map_or(0, |_| 1);
    let attempts = candidate.hold.as_ref().map_or(0, |hold| hold.attempts);
    let next_retry = candidate
        .hold
        .as_ref()
        .and_then(|hold| hold.next_retry_at)
        .map(|at| at.timestamp_millis())
        .unwrap_or(0);
    (
        hold_rank,
        attempts,
        next_retry,
        candidate.envelope.created_at.timestamp_millis(),
    )
}

fn trace_queue_warnings_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<Vec<TraceQueueWarning>> {
    let mut counts: BTreeMap<TraceQueueWarningKind, u32> = BTreeMap::new();
    for path in queued_trace_envelope_paths_for_scope(scope)? {
        let envelope = match load_trace_envelope(&path) {
            Ok(envelope) => envelope,
            Err(error) => {
                tracing::debug!(
                    %error,
                    path = %path.display(),
                    "Trace Commons queue diagnostics found malformed envelope"
                );
                *counts
                    .entry(TraceQueueWarningKind::MalformedEnvelope)
                    .or_default() += 1;
                continue;
            }
        };
        if envelope.schema_version != TRACE_CONTRIBUTION_SCHEMA_VERSION {
            *counts
                .entry(TraceQueueWarningKind::SchemaVersionMismatch)
                .or_default() += 1;
        }
        if envelope.consent.policy_version != TRACE_CONTRIBUTION_POLICY_VERSION {
            *counts
                .entry(TraceQueueWarningKind::PolicyVersionMismatch)
                .or_default() += 1;
        }
        if !trace_queue_redaction_pipeline_supported(&envelope.privacy.redaction_pipeline_version) {
            *counts
                .entry(TraceQueueWarningKind::RedactionPipelineMismatch)
                .or_default() += 1;
        }
        if envelope.trace_card.redaction_pipeline_version
            != envelope.privacy.redaction_pipeline_version
        {
            *counts
                .entry(TraceQueueWarningKind::TraceCardRedactionPipelineMismatch)
                .or_default() += 1;
        }
    }
    Ok(counts
        .into_iter()
        .map(|(kind, count)| TraceQueueWarning {
            kind,
            count,
            severity: trace_queue_warning_severity(kind),
            promotion_blocking: trace_queue_warning_promotion_blocking(kind),
            message: trace_queue_warning_message(kind, count),
            recommended_action: trace_queue_warning_recommended_action(kind).to_string(),
        })
        .collect())
}

fn trace_queue_warning_severity(kind: TraceQueueWarningKind) -> TraceQueueWarningSeverity {
    match kind {
        TraceQueueWarningKind::MalformedEnvelope => TraceQueueWarningSeverity::Blocking,
        TraceQueueWarningKind::SchemaVersionMismatch
        | TraceQueueWarningKind::PolicyVersionMismatch
        | TraceQueueWarningKind::RedactionPipelineMismatch
        | TraceQueueWarningKind::TraceCardRedactionPipelineMismatch => {
            TraceQueueWarningSeverity::Warning
        }
    }
}

fn trace_queue_warning_promotion_blocking(kind: TraceQueueWarningKind) -> bool {
    matches!(
        kind,
        TraceQueueWarningKind::SchemaVersionMismatch
            | TraceQueueWarningKind::PolicyVersionMismatch
            | TraceQueueWarningKind::RedactionPipelineMismatch
            | TraceQueueWarningKind::TraceCardRedactionPipelineMismatch
            | TraceQueueWarningKind::MalformedEnvelope
    )
}

fn trace_queue_warning_message(kind: TraceQueueWarningKind, count: u32) -> String {
    let label = match kind {
        TraceQueueWarningKind::SchemaVersionMismatch => "schema version mismatch",
        TraceQueueWarningKind::PolicyVersionMismatch => "policy version mismatch",
        TraceQueueWarningKind::RedactionPipelineMismatch => "redaction pipeline mismatch",
        TraceQueueWarningKind::TraceCardRedactionPipelineMismatch => {
            "trace card redaction pipeline mismatch"
        }
        TraceQueueWarningKind::MalformedEnvelope => "malformed queued envelope",
    };
    format!("{count} queued trace(s) have {label}")
}

fn trace_queue_warning_recommended_action(kind: TraceQueueWarningKind) -> &'static str {
    match kind {
        TraceQueueWarningKind::SchemaVersionMismatch => {
            "Re-preview or regenerate queued traces with the current contribution schema before production promotion."
        }
        TraceQueueWarningKind::PolicyVersionMismatch => {
            "Refresh user consent for queued traces under the current Trace Commons policy before production promotion."
        }
        TraceQueueWarningKind::RedactionPipelineMismatch => {
            "Re-run local redaction with an approved redaction pipeline before allowing autonomous promotion."
        }
        TraceQueueWarningKind::TraceCardRedactionPipelineMismatch => {
            "Rebuild trace-card metadata so it matches the envelope redaction pipeline before promotion."
        }
        TraceQueueWarningKind::MalformedEnvelope => {
            "Remove, quarantine, or regenerate malformed queue files before enabling production autonomous uploads."
        }
    }
}

fn trace_queue_redaction_pipeline_supported(version: &str) -> bool {
    let parts = version
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    !parts.is_empty()
        && parts.contains(&DETERMINISTIC_REDACTION_PIPELINE_VERSION)
        && parts.iter().all(|part| {
            matches!(
                *part,
                DETERMINISTIC_REDACTION_PIPELINE_VERSION
                    | PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX
                    | SERVER_RESCRUB_PIPELINE_SUFFIX
            )
        })
}

fn read_trace_queue_telemetry_for_scope_unlocked(
    scope: Option<&str>,
) -> anyhow::Result<TraceQueueTelemetry> {
    let path = trace_queue_telemetry_path(scope);
    if !path.exists() {
        return Ok(TraceQueueTelemetry::default());
    }
    let body = std::fs::read_to_string(&path).map_err(|e| {
        anyhow::anyhow!(
            "failed to read trace queue telemetry {}: {}",
            path.display(),
            e
        )
    })?;
    serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse trace queue telemetry {}: {}",
            path.display(),
            e
        )
    })
}

fn write_trace_queue_telemetry_for_scope_unlocked(
    scope: Option<&str>,
    telemetry: &TraceQueueTelemetry,
) -> anyhow::Result<()> {
    write_json_file(
        &trace_queue_telemetry_path(scope),
        telemetry,
        "trace queue telemetry",
    )
}

fn mutate_trace_queue_telemetry_for_scope_unlocked(
    scope: Option<&str>,
    mut mutate: impl FnMut(&mut TraceQueueTelemetry),
) -> anyhow::Result<()> {
    let mut telemetry = read_trace_queue_telemetry_for_scope_unlocked(scope)?;
    mutate(&mut telemetry);
    write_trace_queue_telemetry_for_scope_unlocked(scope, &telemetry)
}

fn record_trace_queue_flush_attempt_for_scope_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_flush_attempt_at = Some(now);
    })
}

fn record_trace_queue_flush_success_for_scope_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
    clear_failure: bool,
) -> anyhow::Result<()> {
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_successful_flush_at = Some(now);
        telemetry.consecutive_flush_failures = 0;
        if clear_failure {
            telemetry.last_failure = None;
        }
    })
}

fn record_trace_queue_compaction_for_scope_unlocked(
    scope: Option<&str>,
    report: &TraceQueueCompactionReport,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_compaction_at = Some(now);
        telemetry.last_compaction = Some(report.clone());
        telemetry.compaction_reclaimed_items_total = telemetry
            .compaction_reclaimed_items_total
            .saturating_add(report.reclaimed_count());
    })
}

fn record_trace_queue_flush_failure_for_scope_unlocked(
    scope: Option<&str>,
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let failure = trace_queue_telemetry_failure(error, now);
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_failed_flush_at = Some(now);
        telemetry.consecutive_flush_failures =
            telemetry.consecutive_flush_failures.saturating_add(1);
        telemetry.last_failure = Some(failure.clone());
    })
}

fn record_trace_queue_retryable_submission_failure_for_scope_unlocked(
    scope: Option<&str>,
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let failure =
        trace_queue_telemetry_failure_with_label(error, now, "submission retry scheduled");
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.retryable_submission_failure_count = telemetry
            .retryable_submission_failure_count
            .saturating_add(1);
        telemetry.last_retryable_submission_failure_at = Some(now);
        telemetry.last_failure = Some(failure.clone());
    })
}

fn record_trace_queue_status_sync_success_for_scope_unlocked(
    scope: Option<&str>,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.last_status_sync_at = Some(now);
    })
}

fn record_trace_queue_status_sync_failure_for_scope_unlocked(
    scope: Option<&str>,
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let kind = match trace_queue_telemetry_failure_kind(error) {
        TraceQueueTelemetryFailureKind::Unknown => TraceQueueTelemetryFailureKind::StatusSync,
        kind => kind,
    };
    let failure = trace_queue_telemetry_failure_with_kind(error, now, kind, "status sync failed");
    mutate_trace_queue_telemetry_for_scope_unlocked(scope, |telemetry| {
        telemetry.status_sync_failure_count = telemetry.status_sync_failure_count.saturating_add(1);
        telemetry.last_status_sync_failed_at = Some(now);
        telemetry.last_failure = Some(failure.clone());
    })
}

fn trace_queue_telemetry_failure(
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> TraceQueueTelemetryFailure {
    let kind = trace_queue_telemetry_failure_kind(error);
    trace_queue_telemetry_failure_with_kind(error, now, kind, "flush failed")
}

fn trace_queue_telemetry_failure_with_label(
    error: &anyhow::Error,
    now: DateTime<Utc>,
    label: &str,
) -> TraceQueueTelemetryFailure {
    let kind = trace_queue_telemetry_failure_kind(error);
    trace_queue_telemetry_failure_with_kind(error, now, kind, label)
}

fn trace_queue_telemetry_failure_kind(error: &anyhow::Error) -> TraceQueueTelemetryFailureKind {
    for cause in error.chain() {
        if let Some(remote_failure) = cause.downcast_ref::<TraceRemoteRequestFailure>() {
            return remote_failure.kind;
        }
        if let Some(llm_error) = cause.downcast_ref::<ironclaw_llm::error::LlmError>()
            && let Some(kind) = trace_queue_telemetry_failure_kind_for_llm_error(llm_error)
        {
            return kind;
        }
        if let Some(kind) = trace_queue_telemetry_failure_kind_for_error_source(cause) {
            return kind;
        }
        if let Some(reqwest_error) = cause.downcast_ref::<reqwest::Error>() {
            return trace_remote_request_failure_kind_for_reqwest_error(reqwest_error);
        }
    }
    let message = error
        .chain()
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    if message.contains("endpoint") || message.contains("invalid trace contribution") {
        TraceQueueTelemetryFailureKind::Endpoint
    } else if message.contains("rejected by 401")
        || message.contains("rejected by 403")
        || message.contains("unauthorized")
        || message.contains("forbidden")
    {
        TraceQueueTelemetryFailureKind::Credential
    } else if message.contains("rejected by") {
        TraceQueueTelemetryFailureKind::HttpRejection
    } else if message.contains("not set")
        || message.contains("credentials")
        || message.contains("credential")
        || message.contains("token")
    {
        TraceQueueTelemetryFailureKind::Credential
    } else if message.contains("network is unreachable")
        || message.contains("no route to host")
        || message.contains("offline")
        || message.contains("internet connection appears to be offline")
    {
        TraceQueueTelemetryFailureKind::NetworkOffline
    } else if message.contains("dns")
        || message.contains("failed to lookup")
        || message.contains("failed to resolve")
        || message.contains("name or service not known")
        || message.contains("nodename nor servname")
    {
        TraceQueueTelemetryFailureKind::NetworkDns
    } else if message.contains("timed out")
        || message.contains("timeout")
        || message.contains("deadline elapsed")
    {
        TraceQueueTelemetryFailureKind::NetworkTimeout
    } else if message.contains("connection refused") || message.contains("refused") {
        TraceQueueTelemetryFailureKind::NetworkConnectionRefused
    } else if message.contains("request failed")
        || message.contains("connection")
        || message.contains("tcp")
        || message.contains("error trying to connect")
    {
        TraceQueueTelemetryFailureKind::Network
    } else if message.contains("opt-in") || message.contains("policy") {
        TraceQueueTelemetryFailureKind::Policy
    } else if message.contains("queue") || message.contains("envelope") {
        TraceQueueTelemetryFailureKind::Queue
    } else {
        TraceQueueTelemetryFailureKind::Unknown
    }
}

fn trace_queue_telemetry_failure_kind_for_llm_error(
    error: &ironclaw_llm::error::LlmError,
) -> Option<TraceQueueTelemetryFailureKind> {
    match error {
        ironclaw_llm::error::LlmError::AuthFailed { .. }
        | ironclaw_llm::error::LlmError::SessionExpired { .. }
        | ironclaw_llm::error::LlmError::SessionRenewalFailed { .. } => {
            Some(TraceQueueTelemetryFailureKind::Credential)
        }
        ironclaw_llm::error::LlmError::RateLimited { .. } => {
            Some(TraceQueueTelemetryFailureKind::HttpRejection)
        }
        ironclaw_llm::error::LlmError::RequestFailed { .. } => {
            Some(TraceQueueTelemetryFailureKind::Network)
        }
        _ => None,
    }
}

fn trace_queue_telemetry_failure_with_kind(
    error: &anyhow::Error,
    now: DateTime<Utc>,
    kind: TraceQueueTelemetryFailureKind,
    label: &str,
) -> TraceQueueTelemetryFailure {
    let error_hash = trace_queue_error_hash(error);
    TraceQueueTelemetryFailure {
        kind,
        reason: format!("{label}; error_hash={error_hash}"),
        error_hash,
        at: now,
    }
}

fn trace_queue_error_hash(error: &anyhow::Error) -> String {
    let mut hasher = Sha256::new();
    hasher.update(error.to_string().as_bytes());
    let digest = hasher.finalize();
    format!("sha256:{}", hex::encode(&digest[..8])) // safety: slicing the fixed-size SHA-256 byte array.
}

fn sanitized_trace_submission_failure_reason(error: &anyhow::Error) -> (String, String) {
    let error_hash = trace_queue_error_hash(error);
    (
        format!("submission failed; retained for retry (error_hash={error_hash})"),
        error_hash,
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

#[cfg(test)]
fn write_trace_queue_hold_reason(path: &Path, reason: &str) -> anyhow::Result<()> {
    write_trace_queue_hold_sidecar_for_path(
        path,
        &TraceQueueHold {
            submission_id: trace_queue_submission_id_from_envelope_path(path)
                .unwrap_or_else(Uuid::nil),
            kind: trace_queue_hold_kind_for_policy_reason(reason),
            reason: safe_trace_queue_hold_reason(reason),
            attempts: 0,
            next_retry_at: None,
        },
    )
}

fn write_trace_queue_hold_sidecar_for_path(
    path: &Path,
    hold: &TraceQueueHold,
) -> anyhow::Result<()> {
    let hold_path = trace_queue_hold_path_for_envelope_path(path);
    let body = TraceQueueHoldSidecar {
        envelope: path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
        held_at: Some(Utc::now()),
        kind: Some(hold.kind),
        reason: Some(safe_trace_queue_hold_reason(&hold.reason)),
        attempts: (hold.attempts > 0).then_some(hold.attempts),
        next_retry_at: hold.next_retry_at,
        error_hash: trace_queue_error_hash_from_reason(&hold.reason),
    };
    write_json_file(&hold_path, &body, "trace queue hold reason")
}

fn retry_hold_if_not_due(
    path: &Path,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<TraceQueueHold>> {
    let Some(sidecar) = read_trace_queue_hold_sidecar_for_envelope(path).unwrap_or_else(|error| {
        tracing::debug!(
            %error,
            path = %path.display(),
            "Ignoring unreadable Trace Commons retry sidecar"
        );
        None
    }) else {
        return Ok(None);
    };
    let Some(submission_id) = trace_queue_submission_id_from_envelope_path(path) else {
        return Ok(None);
    };
    let hold = trace_queue_hold_from_sidecar(submission_id, &sidecar);
    if hold.kind == TraceQueueHoldKind::RetryableSubmissionFailure
        && hold
            .next_retry_at
            .is_some_and(|next_retry_at| next_retry_at > now)
    {
        return Ok(Some(hold));
    }
    Ok(None)
}

fn retry_hold_after_submission_failure(
    path: &Path,
    submission_id: Uuid,
    error: &anyhow::Error,
    now: DateTime<Utc>,
) -> anyhow::Result<TraceQueueHold> {
    let previous = read_trace_queue_hold_sidecar_for_envelope(path).unwrap_or_else(|error| {
        tracing::debug!(
            %error,
            path = %path.display(),
            "Ignoring unreadable Trace Commons retry sidecar before rescheduling"
        );
        None
    });
    let attempts = previous.and_then(|sidecar| sidecar.attempts).unwrap_or(0) + 1;
    let next_retry_at = trace_queue_next_retry_at(now, attempts);
    let (reason, _) = sanitized_trace_submission_failure_reason(error);
    Ok(TraceQueueHold {
        submission_id,
        kind: TraceQueueHoldKind::RetryableSubmissionFailure,
        reason,
        attempts,
        next_retry_at: Some(next_retry_at),
    })
}

fn trace_queue_next_retry_at(now: DateTime<Utc>, attempts: u32) -> DateTime<Utc> {
    let exponent = attempts.saturating_sub(1).min(8);
    let multiplier = 1u64 << exponent;
    let seconds = 300u64.saturating_mul(multiplier).min(86_400);
    now + chrono::Duration::seconds(seconds as i64)
}

fn read_trace_queue_hold_sidecar_for_envelope(
    path: &Path,
) -> anyhow::Result<Option<TraceQueueHoldSidecar>> {
    let hold_path = trace_queue_hold_path_for_envelope_path(path);
    if !hold_path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&hold_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to read trace queue hold {}: {}",
            hold_path.display(),
            e
        )
    })?;
    let sidecar = serde_json::from_str::<TraceQueueHoldSidecar>(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse trace queue hold {}: {}",
            hold_path.display(),
            e
        )
    })?;
    Ok(Some(sidecar))
}

fn trace_queue_hold_from_sidecar(
    submission_id: Uuid,
    sidecar: &TraceQueueHoldSidecar,
) -> TraceQueueHold {
    let reason = safe_trace_queue_hold_reason(sidecar.reason.as_deref().unwrap_or("held"));
    TraceQueueHold {
        submission_id,
        kind: sidecar
            .kind
            .unwrap_or_else(|| trace_queue_hold_kind_for_policy_reason(&reason)),
        reason,
        attempts: sidecar.attempts.unwrap_or(0),
        next_retry_at: sidecar.next_retry_at,
    }
}

fn trace_queue_hold_kind_for_policy_reason(reason: &str) -> TraceQueueHoldKind {
    if reason.to_ascii_lowercase().contains("manual review") {
        TraceQueueHoldKind::ManualReview
    } else if reason.to_ascii_lowercase().contains("retained for retry") {
        TraceQueueHoldKind::RetryableSubmissionFailure
    } else {
        TraceQueueHoldKind::PolicyGate
    }
}

fn trace_queue_submission_id_from_envelope_path(path: &Path) -> Option<Uuid> {
    let raw = path.file_stem()?.to_str()?;
    Uuid::parse_str(raw).ok()
}

fn trace_queue_hold_path_for_envelope_path(path: &Path) -> PathBuf {
    path.with_extension("held.json")
}

fn trace_queue_error_hash_from_reason(reason: &str) -> Option<String> {
    reason
        .split("error_hash=")
        .nth(1)
        .map(|suffix| suffix.trim_end_matches(')').trim().to_string())
        .filter(|hash| hash.starts_with("sha256:"))
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

fn trace_policy_path_at(base: &std::path::Path, scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope_at(base, scope).join("policy.json")
}

fn trace_queue_dir(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("queue")
}

/// Whether `scope` has any pending (flushable) queue entries on disk.
///
/// Lets the periodic flush worker prune drained scopes from its in-memory
/// observed-scope set instead of retaining one entry per historical caller
/// forever. A `.held.json` sidecar is a manual-review hold (not flushable until
/// authorized) and does not count; only a queued envelope (`<id>.json` with no
/// `.held.json` peer that is awaiting authorization) keeps a scope "pending".
pub fn trace_scope_has_pending_queue(scope: &str) -> bool {
    let dir = trace_queue_dir(Some(scope));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(".json") && !name.ends_with(".held.json"))
    })
}

fn trace_queue_malformed_dir(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("queue_malformed")
}

fn trace_records_path(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("submissions.json")
}

fn trace_queue_telemetry_path(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("queue_telemetry.json")
}

fn trace_credit_notice_outbox_path(scope: Option<&str>) -> PathBuf {
    trace_contribution_dir_for_scope(scope).join("credit_notice_outbox.json")
}

/// Atomic, durable, 0o600 JSON write (create_new + uuid temp name + sync_all +
/// best-effort parent-dir sync). Reused by `onboarding::device_key` so the
/// Ed25519 secret is never world-readable at any point and concurrent writers
/// don't race on a fixed temp name.
pub(crate) fn write_json_file<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    label: &str,
) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| {
        anyhow::anyhow!(
            "failed to create {} directory {}: {}",
            label,
            parent.display(),
            e
        )
    })?;
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| anyhow::anyhow!("failed to serialize {}: {}", label, e))?;
    let temp_path = parent.join(format!(
        "{}{}.tmp",
        trace_json_temp_prefix(path),
        Uuid::new_v4()
    ));
    let mut temp = {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Trace policy files now potentially carry an operator-issued pilot
        // invite code; mirror the CLI's 0o600 stance for atomic policy
        // writes too so the rename-into-place step doesn't widen perms.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&temp_path).map_err(|e| {
            anyhow::anyhow!(
                "failed to create temporary {} {}: {}",
                label,
                temp_path.display(),
                e
            )
        })?
    };
    if let Err(error) = temp.write_all(body.as_bytes()) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(anyhow::anyhow!(
            "failed to write temporary {} for {}: {}",
            label,
            path.display(),
            error
        ));
    }
    if let Err(error) = temp.sync_all() {
        let _ = std::fs::remove_file(&temp_path);
        return Err(anyhow::anyhow!(
            "failed to sync temporary {} for {}: {}",
            label,
            path.display(),
            error
        ));
    }
    drop(temp);
    std::fs::rename(&temp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        anyhow::anyhow!("failed to install {} {}: {}", label, path.display(), e)
    })?;
    sync_directory_best_effort(parent, label);
    Ok(())
}

fn trace_json_temp_prefix(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!(".{name}."))
        .unwrap_or_else(|| ".trace-json.".to_string())
}

fn quarantine_malformed_trace_queue_envelope(
    scope: Option<&str>,
    path: &Path,
) -> anyhow::Result<PathBuf> {
    let quarantine_dir = trace_queue_malformed_dir(scope);
    std::fs::create_dir_all(&quarantine_dir).map_err(|e| {
        anyhow::anyhow!(
            "failed to create malformed trace queue directory {}: {}",
            quarantine_dir.display(),
            e
        )
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "failed to quarantine malformed trace queue envelope without file name: {}",
            path.display()
        )
    })?;
    let mut quarantine_path = quarantine_dir.join(file_name);
    if quarantine_path.exists() {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("queued-envelope");
        quarantine_path = quarantine_dir.join(format!("{stem}.{}.json", Uuid::new_v4()));
    }
    std::fs::rename(path, &quarantine_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to quarantine malformed trace queue envelope {} to {}: {}",
            path.display(),
            quarantine_path.display(),
            e
        )
    })?;
    if let Some(active_dir) = path.parent() {
        sync_directory_best_effort(active_dir, "trace queue directory");
    }
    sync_directory_best_effort(&quarantine_dir, "malformed trace queue directory");
    Ok(quarantine_path)
}

fn sync_directory_best_effort(path: &Path, label: &str) {
    match std::fs::File::open(path) {
        Ok(file) => {
            if let Err(error) = file.sync_all() {
                tracing::debug!(
                    %error,
                    path = %path.display(),
                    label,
                    "Directory sync is not supported or failed"
                );
            }
        }
        Err(error) => {
            tracing::debug!(
                %error,
                path = %path.display(),
                label,
                "Directory sync is not supported or failed"
            );
        }
    }
}

fn scope_hash(scope: &str) -> String {
    let digest = Sha256::digest(scope.as_bytes());
    hex::encode(&digest[..16]) // safety: slicing the fixed-size SHA-256 byte array.
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ironclaw_llm::recording::{TraceStep, TraceToolCall};

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

        let mut missing_endpoint = StandingTraceContributionPolicy::default().set_enabled(true);
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

    struct RefreshingTestUploadCredentialProvider {
        current: std::sync::Mutex<String>,
        fresh: String,
    }

    impl RefreshingTestUploadCredentialProvider {
        fn new(stale: &str, fresh: &str) -> Self {
            Self {
                current: std::sync::Mutex::new(stale.to_string()),
                fresh: fresh.to_string(),
            }
        }
    }

    #[async_trait]
    impl TraceUploadCredentialProvider for RefreshingTestUploadCredentialProvider {
        async fn bearer_token(
            &self,
            _policy: &StandingTraceContributionPolicy,
            _context: &TraceUploadClaimContext,
            force_refresh: bool,
        ) -> anyhow::Result<String> {
            let mut current = self.current.lock().expect("test provider lock");
            if force_refresh {
                *current = self.fresh.clone();
            }
            Ok(current.clone())
        }
    }

    /// Records the (subject, scope_dir) of every claim context it is asked to
    /// mint for, so tests can assert the credential context that status sync
    /// actually used.
    #[derive(Default)]
    struct CapturingUploadCredentialProvider {
        contexts: std::sync::Mutex<Vec<(Option<String>, Option<PathBuf>)>>,
    }

    #[async_trait]
    impl TraceUploadCredentialProvider for CapturingUploadCredentialProvider {
        async fn bearer_token(
            &self,
            _policy: &StandingTraceContributionPolicy,
            context: &TraceUploadClaimContext,
            _force_refresh: bool,
        ) -> anyhow::Result<String> {
            self.contexts
                .lock()
                .expect("capturing provider lock")
                .push((context.subject.clone(), context.scope_dir.clone()));
            Ok("captured-token".to_string())
        }
    }

    struct FailingTestUploadCredentialProvider {
        kind: std::io::ErrorKind,
    }

    #[async_trait]
    impl TraceUploadCredentialProvider for FailingTestUploadCredentialProvider {
        async fn bearer_token(
            &self,
            _policy: &StandingTraceContributionPolicy,
            _context: &TraceUploadClaimContext,
            _force_refresh: bool,
        ) -> anyhow::Result<String> {
            Err(std::io::Error::new(
                self.kind,
                "credential provider failed while using super-secret-token",
            )
            .into())
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
        let options = RecordedTraceContributionOptions::default()
            .set_include_message_text(true)
            .set_include_tool_payloads(true);
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
            RecordedTraceContributionOptions::default().set_include_message_text(true),
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
            RecordedTraceContributionOptions::default().set_include_message_text(true),
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
        let options = RecordedTraceContributionOptions::default()
            .set_include_message_text(true)
            .set_include_tool_payloads(true);
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
            RecordedTraceContributionOptions::default()
                .set_include_message_text(true)
                .set_include_tool_payloads(true)
                .set_consent_scopes(vec![ConsentScope::ModelTraining]),
        );
        raw.outcome = OutcomeMetadata::default()
            .set_user_feedback(UserFeedback::Correction)
            .set_task_success(TaskSuccess::Partial)
            .set_failure_modes(vec![TraceFailureMode::UserIntentMisread])
            .set_human_correction(
                "Use alice@example.com and /Users/alice/project/fix.md as the correction",
            );
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
            RecordedTraceContributionOptions::default()
                .set_consent_scopes(vec![ConsentScope::ModelTraining]),
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
    async fn medium_pii_tool_trace_auto_submits_while_high_is_held() {
        // Below-High residual PII risk must auto-submit: the manual-approval
        // eligibility gate fires only on High, and the value scorecard no
        // longer crushes a Medium tool trace below the 0.35 submission gate.
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");

        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_require_manual_approval_when_pii_detected(true);
        assert_eq!(policy.min_submission_score, 0.35, "default gate is 0.35");

        // Medium: clears the score gate and auto-submits (no manual review).
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
        apply_credit_estimate_to_envelope(&mut envelope);
        assert!(
            envelope.value.submission_score >= policy.min_submission_score,
            "medium-risk tool trace must clear the score gate, got {}",
            envelope.value.submission_score
        );
        assert!(
            matches!(
                trace_autonomous_eligibility(&envelope, &policy),
                TraceQueueEligibility::Submit
            ),
            "medium-risk tool trace must auto-submit, not hold for manual review"
        );

        // High: still held (and its score collapses to zero via the gate).
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
        apply_credit_estimate_to_envelope(&mut envelope);
        assert!(
            matches!(
                trace_autonomous_eligibility(&envelope, &policy),
                TraceQueueEligibility::Hold { .. }
            ),
            "high-risk trace must remain held"
        );
    }

    #[tokio::test]
    async fn empty_allowed_uses_envelope_fails_closed_not_submitted() {
        // A public_attribution-only consent scope grants no trace-content
        // allowed-uses; such an envelope must never be submitted, even with an
        // otherwise-permissive auto-submit policy or an explicit manual-review
        // authorization (there is nothing to submit it for).
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        envelope.trace_card.allowed_uses = Vec::new();

        let permissive = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0);
        assert!(
            matches!(
                trace_autonomous_eligibility(&envelope, &permissive),
                TraceQueueEligibility::Hold {
                    kind: TraceQueueHoldKind::PolicyGate,
                    ..
                }
            ),
            "empty allowed-uses must fail closed under a permissive auto-submit policy"
        );

        // Even an explicit manual-review authorization cannot submit it.
        envelope.manual_review_authorized = true;
        assert!(
            matches!(
                trace_autonomous_eligibility(&envelope, &permissive),
                TraceQueueEligibility::Hold { .. }
            ),
            "empty allowed-uses must fail closed even when manual_review_authorized"
        );
    }

    #[tokio::test]
    async fn eligibility_hold_kind_separates_manual_review_from_policy_gate() {
        // The hold kind must distinguish a PII manual-review hold (which is
        // retained for the user to authorize) from a policy/value gate (which
        // is not review-worthy), so the held-review surface is not polluted
        // with low-value traces.
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);

        // High residual PII risk + manual-approval policy => ManualReview.
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
        let manual_policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_require_manual_approval_when_pii_detected(true);
        assert!(matches!(
            trace_autonomous_eligibility(&envelope, &manual_policy),
            TraceQueueEligibility::Hold {
                kind: TraceQueueHoldKind::ManualReview,
                ..
            }
        ));

        // Below-threshold score (no PII concern) => PolicyGate, not review.
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
        let strict_policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_min_submission_score(1.0);
        assert!(matches!(
            trace_autonomous_eligibility(&envelope, &strict_policy),
            TraceQueueEligibility::Hold {
                kind: TraceQueueHoldKind::PolicyGate,
                ..
            }
        ));
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
            crate::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Please inspect the build".to_string(),
                created_at: now,
            },
            crate::ConversationMessage {
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
            crate::ConversationMessage {
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
            history: Vec::new(),
            last_credit_notice_at,
            credit_notice_state: TraceCreditNoticeState::default(),
        }
    }

    #[test]
    fn scoped_credit_view_reflects_record_changes_via_signature() {
        let scope = format!("scoped-credit-view-{}", Uuid::new_v4());

        // No records yet -> zero report.
        let view = scoped_credit_view(&scope).expect("empty view");
        assert_eq!(view.report.submissions_total, 0);
        assert!(view.manual_review_holds.is_empty());

        // One submitted record -> view reflects it (cache miss, recompute).
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(1.0, None, None, Vec::new())],
        )
        .expect("write one record");
        let view = scoped_credit_view(&scope).expect("one-record view");
        assert_eq!(view.report.submissions_total, 1);
        assert_eq!(view.report.submissions_submitted, 1);

        // A repeated call with no change returns the same view (cache hit path).
        assert_eq!(
            scoped_credit_view(&scope).expect("cache-hit view").report,
            view.report
        );

        // Changing the records file changes its signature -> the cached view is
        // invalidated and recomputed, so the new total is reflected (a stale
        // cache would still report 1).
        write_local_trace_records_for_scope(
            Some(&scope),
            &[
                submitted_credit_record(1.0, None, None, Vec::new()),
                submitted_credit_record(2.0, None, None, Vec::new()),
            ],
        )
        .expect("write two records");
        let view = scoped_credit_view(&scope).expect("two-record view");
        assert_eq!(view.report.submissions_total, 2);

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn queue_diagnostics_are_scoped_to_one_user_queue_and_records() {
        let scope_a = format!("trace-queue-diagnostics-a-{}", Uuid::new_v4());
        let scope_b = format!("trace-queue-diagnostics-b-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string());
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
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string());
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
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(0),
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
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_credit_notice_interval_hours(168);
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
    fn credit_notice_acknowledge_suppresses_same_fingerprint_until_credit_changes() {
        let scope = format!("trace-credit-ack-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(168),
        )
        .expect("policy writes");
        let record = submitted_credit_record(
            1.0,
            Some(1.0),
            None,
            vec!["Accepted after privacy checks.".to_string()],
        );
        let submission_id = record.submission_id;
        let trace_id = record.trace_id;
        write_local_trace_records_for_scope(Some(&scope), &[record]).expect("record writes");

        let due = trace_credit_notice_due_for_scope(Some(&scope))
            .expect("notice due check succeeds")
            .expect("notice starts due");
        assert_eq!(due.final_credit, 1.0);
        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert!(records[0].last_credit_notice_at.is_none());
        assert!(records[0].credit_notice_state.is_empty());

        let acknowledged = acknowledge_trace_credit_notice_for_scope(Some(&scope))
            .expect("acknowledge succeeds")
            .expect("acknowledge returns the current summary");
        assert_eq!(acknowledged.final_credit, 1.0);

        let after_ack =
            trace_credit_notice_due_for_scope(Some(&scope)).expect("notice due check succeeds");
        assert_eq!(after_ack, None);
        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert!(records[0].credit_notice_state.acknowledged_at.is_some());

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
                delayed_credit_explanations: vec!["Benchmark conversion bonus: +1.0.".to_string()],
            }],
        )
        .expect("status sync applies");
        assert_eq!(changed, 1);

        let after_change = trace_credit_notice_due_for_scope(Some(&scope))
            .expect("notice due check succeeds")
            .expect("changed credit is due again");
        assert_eq!(after_change.final_credit, 2.0);
        assert_eq!(after_change.delayed_credit_delta, 1.0);

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn credit_notice_snooze_suppresses_until_deadline() {
        let scope = format!("trace-credit-snooze-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(168),
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(
                1.0,
                Some(1.5),
                None,
                vec!["Delayed utility credit posted.".to_string()],
            )],
        )
        .expect("record writes");
        let now = Utc::now();
        let snoozed_until = now + chrono::Duration::hours(24);

        assert!(
            trace_credit_notice_due_for_scope_at(Some(&scope), now)
                .expect("notice due check succeeds")
                .is_some()
        );
        let snoozed =
            snooze_trace_credit_notice_for_scope_until_at(Some(&scope), snoozed_until, now)
                .expect("snooze succeeds")
                .expect("snooze returns the current summary");
        assert_eq!(snoozed.final_credit, 1.5);

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert_eq!(
            records[0].credit_notice_state.snoozed_until,
            Some(snoozed_until)
        );
        assert_eq!(
            trace_credit_notice_due_for_scope_at(Some(&scope), now + chrono::Duration::hours(1))
                .expect("notice due check succeeds"),
            None
        );
        assert!(
            trace_credit_notice_due_for_scope_at(Some(&scope), now + chrono::Duration::hours(25))
                .expect("notice due check succeeds")
                .is_some()
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn legacy_credit_notice_timestamp_suppresses_until_interval() {
        let scope = format!("trace-credit-legacy-notice-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(168),
        )
        .expect("policy writes");
        let now = Utc::now();
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(
                1.0,
                Some(1.5),
                Some(now),
                vec!["Previously noticed before the state field existed.".to_string()],
            )],
        )
        .expect("record writes");

        assert_eq!(
            trace_credit_notice_due_for_scope_at(Some(&scope), now + chrono::Duration::hours(1))
                .expect("notice due check succeeds"),
            None
        );
        assert!(
            trace_credit_notice_due_for_scope_at(Some(&scope), now + chrono::Duration::hours(169))
                .expect("notice due check succeeds")
                .is_some()
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn trace_credit_notice_outbox_enqueue_is_idempotent_per_fingerprint() {
        let scope = format!("trace-credit-outbox-idempotent-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(168),
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(
                1.0,
                Some(1.5),
                None,
                vec!["Delayed utility credit posted.".to_string()],
            )],
        )
        .expect("record writes");

        let now = Utc::now();
        let first = mark_trace_credit_noticed_if_due_at_unlocked(Some(&scope), 168, now)
            .expect("first notice check succeeds")
            .expect("first notice is due");
        assert_eq!(first.final_credit, 1.5);
        let second = mark_trace_credit_noticed_if_due_at_unlocked(
            Some(&scope),
            168,
            now + chrono::Duration::hours(169),
        )
        .expect("second notice check succeeds")
        .expect("same fingerprint is due again after interval");
        assert_eq!(second.final_credit, 1.5);

        let outbox = read_trace_credit_notice_outbox_for_scope(Some(&scope))
            .expect("credit notice outbox reads");
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].status, TraceCreditNoticeOutboxStatus::Pending);
        assert_eq!(outbox[0].attempt_count, 0);
        assert!(outbox[0].message.contains("pending +1.00"));

        let pending = pending_trace_credit_notice_outbox_items_for_scope_at(Some(&scope), now)
            .expect("pending outbox reads");
        assert_eq!(pending.len(), 1);

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn credit_notice_delivery_success_marks_outbox_delivered() {
        let scope = format!("trace-credit-outbox-delivered-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(168),
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(
                2.0,
                Some(3.0),
                None,
                vec!["Benchmark conversion bonus posted.".to_string()],
            )],
        )
        .expect("record writes");
        mark_trace_credit_noticed_if_due_at_unlocked(Some(&scope), 168, Utc::now())
            .expect("notice check succeeds")
            .expect("notice is due");
        let fingerprint = read_trace_credit_notice_outbox_for_scope(Some(&scope))
            .expect("outbox reads")[0]
            .fingerprint
            .clone();

        let delivered = record_trace_credit_notice_delivery_success_for_scope(
            Some(&scope),
            &fingerprint,
            "test",
        )
        .expect("delivery success records")
        .expect("outbox item exists");

        assert_eq!(delivered.status, TraceCreditNoticeOutboxStatus::Delivered);
        assert_eq!(delivered.attempt_count, 1);
        assert!(delivered.delivered_at.is_some());
        assert_eq!(delivered.delivery_attempts.len(), 1);
        assert!(delivered.delivery_attempts[0].succeeded);
        assert!(
            pending_trace_credit_notice_outbox_items_for_scope_at(Some(&scope), Utc::now())
                .expect("pending outbox reads")
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn credit_notice_delivery_failure_keeps_pending_with_safe_error_hash() {
        let scope = format!("trace-credit-outbox-failure-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(168),
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(
                4.0,
                Some(5.0),
                None,
                vec!["Regression catch bonus posted.".to_string()],
            )],
        )
        .expect("record writes");
        let now = Utc::now();
        mark_trace_credit_noticed_if_due_at_unlocked(Some(&scope), 168, now)
            .expect("notice check succeeds")
            .expect("notice is due");
        let fingerprint = read_trace_credit_notice_outbox_for_scope(Some(&scope))
            .expect("outbox reads")[0]
            .fingerprint
            .clone();

        let failed = record_trace_credit_notice_delivery_failure_for_scope(
            Some(&scope),
            &fingerprint,
            "test",
            "failed for alice@example.com using sk-test-secret in /Users/alice/private",
        )
        .expect("delivery failure records")
        .expect("outbox item exists");

        assert_eq!(failed.status, TraceCreditNoticeOutboxStatus::Pending);
        assert_eq!(failed.attempt_count, 1);
        assert!(failed.next_attempt_at.is_some());
        assert_eq!(failed.delivery_attempts.len(), 1);
        let attempt = &failed.delivery_attempts[0];
        assert!(!attempt.succeeded);
        assert_eq!(attempt.channel, "test");
        assert!(
            attempt
                .error_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:"))
        );
        assert!(attempt.error_kind.is_some());
        let serialized = serde_json::to_string(&failed).expect("outbox serializes");
        assert!(!serialized.contains("alice@example.com"));
        assert!(!serialized.contains("sk-test-secret"));
        assert!(!serialized.contains("/Users/alice/private"));

        assert!(
            pending_trace_credit_notice_outbox_items_for_scope_at(Some(&scope), now)
                .expect("pending before retry reads")
                .is_empty(),
            "failed delivery should wait until next_attempt_at before retry"
        );
        assert_eq!(
            pending_trace_credit_notice_outbox_items_for_scope_at(
                Some(&scope),
                failed.next_attempt_at.expect("next attempt exists")
            )
            .expect("pending after retry reads")
            .len(),
            1
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn credit_notice_acknowledge_and_snooze_suppress_pending_outbox_items() {
        let ack_scope = format!("trace-credit-outbox-ack-test-{}", Uuid::new_v4());
        let snooze_scope = format!("trace-credit-outbox-snooze-test-{}", Uuid::new_v4());
        for scope in [&ack_scope, &snooze_scope] {
            write_trace_policy_for_scope(
                Some(scope),
                &StandingTraceContributionPolicy::default()
                    .set_enabled(true)
                    .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                    .set_credit_notice_interval_hours(168),
            )
            .expect("policy writes");
            write_local_trace_records_for_scope(
                Some(scope),
                &[submitted_credit_record(
                    1.0,
                    Some(2.0),
                    None,
                    vec!["Delayed utility credit posted.".to_string()],
                )],
            )
            .expect("record writes");
            mark_trace_credit_noticed_if_due_at_unlocked(Some(scope), 168, Utc::now())
                .expect("notice check succeeds")
                .expect("notice is due");
        }

        acknowledge_trace_credit_notice_for_scope_at_unlocked(Some(&ack_scope), Utc::now())
            .expect("ack succeeds")
            .expect("ack returns summary");
        let ack_outbox =
            read_trace_credit_notice_outbox_for_scope(Some(&ack_scope)).expect("outbox reads");
        assert_eq!(
            ack_outbox[0].status,
            TraceCreditNoticeOutboxStatus::Acknowledged
        );
        assert!(
            pending_trace_credit_notice_outbox_items_for_scope_at(Some(&ack_scope), Utc::now())
                .expect("pending ack outbox reads")
                .is_empty()
        );

        let now = Utc::now();
        let snoozed_until = now + chrono::Duration::hours(4);
        snooze_trace_credit_notice_for_scope_until_at_unlocked(
            Some(&snooze_scope),
            snoozed_until,
            now,
        )
        .expect("snooze succeeds")
        .expect("snooze returns summary");
        let snooze_outbox =
            read_trace_credit_notice_outbox_for_scope(Some(&snooze_scope)).expect("outbox reads");
        assert_eq!(
            snooze_outbox[0].status,
            TraceCreditNoticeOutboxStatus::Snoozed
        );
        assert_eq!(snooze_outbox[0].snoozed_until, Some(snoozed_until));
        assert!(
            pending_trace_credit_notice_outbox_items_for_scope_at(
                Some(&snooze_scope),
                now + chrono::Duration::hours(1)
            )
            .expect("pending snoozed outbox reads")
            .is_empty()
        );
        assert_eq!(
            pending_trace_credit_notice_outbox_items_for_scope_at(
                Some(&snooze_scope),
                snoozed_until
            )
            .expect("pending after snooze reads")
            .len(),
            1
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&ack_scope)));
        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&snooze_scope)));
    }

    #[test]
    fn local_trace_records_load_legacy_json_without_history() {
        let scope = format!("trace-local-history-legacy-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        let path = trace_records_path(Some(&scope));
        std::fs::create_dir_all(path.parent().expect("trace records path has a parent"))
            .expect("trace records dir exists");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&serde_json::json!([
                {
                    "submission_id": submission_id,
                    "trace_id": trace_id,
                    "endpoint": "https://trace.example.com/v1/traces",
                    "status": "submitted",
                    "server_status": "accepted",
                    "submitted_at": Utc::now(),
                    "privacy_risk": "low",
                    "redaction_counts": {},
                    "credit_points_pending": 1.0,
                    "credit_points_final": 1.0,
                    "credit_explanation": ["Accepted locally."]
                }
            ]))
            .expect("legacy records serialize"),
        )
        .expect("legacy records write");

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].submission_id, submission_id);
        assert_eq!(records[0].trace_id, trace_id);
        let serialized = serde_json::to_value(&records[0]).expect("record serializes");
        assert!(serialized.get("history").is_none());

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn status_sync_appends_safe_local_history_event() {
        let scope = format!("trace-local-history-sync-test-{}", Uuid::new_v4());
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
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
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
        let serialized = serde_json::to_value(&records[0]).expect("record serializes");
        let history = serialized["history"]
            .as_array()
            .expect("history is present");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["kind"], "status_sync");
        assert_eq!(history[0]["server_status"], "accepted");
        assert_eq!(history[0]["credit_delta"], 1.0);
        assert_eq!(history[0]["delayed_credit_explanation_count"], 1);
        assert!(history[0].get("message").is_none());

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn duplicate_status_sync_does_not_append_duplicate_history() {
        let scope = format!("trace-local-history-duplicate-test-{}", Uuid::new_v4());
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
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
            }],
        )
        .expect("local record writes");
        let update = TraceSubmissionStatusUpdate {
            submission_id,
            trace_id,
            status: "accepted".to_string(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.0),
            credit_points_ledger: 1.0,
            credit_points_total: Some(2.0),
            explanation: vec!["Accepted after privacy checks.".to_string()],
            delayed_credit_explanations: vec!["Regression coverage bonus: +1.0.".to_string()],
        };

        apply_remote_trace_submission_statuses_for_scope(
            Some(&scope),
            std::slice::from_ref(&update),
        )
        .expect("first status sync applies");
        apply_remote_trace_submission_statuses_for_scope(Some(&scope), &[update])
            .expect("duplicate status sync applies");

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        let serialized = serde_json::to_value(&records[0]).expect("record serializes");
        let history = serialized["history"]
            .as_array()
            .expect("history is present");
        assert_eq!(history.len(), 1);
        assert_eq!(records[0].credit_events.len(), 1);

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn local_status_history_does_not_persist_unsafe_remote_fields() {
        let scope = format!("trace-local-history-safety-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        write_local_trace_records_for_scope(
            Some(&scope),
            &[LocalTraceSubmissionRecord {
                submission_id,
                trace_id,
                endpoint: Some("https://private.trace.example.com/v1/traces".to_string()),
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
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
            }],
        )
        .expect("local record writes");

        apply_remote_trace_submission_statuses_for_scope(
            Some(&scope),
            &[TraceSubmissionStatusUpdate {
                submission_id,
                trace_id,
                status: "accepted".to_string(),
                credit_points_pending: 1.0,
                credit_points_final: Some(2.0),
                credit_points_ledger: 1.0,
                credit_points_total: Some(2.0),
                explanation: vec![
                    "Accepted for alice@example.com under tenant-raw-alpha at https://private.trace.example.com/v1/traces".to_string(),
                ],
                delayed_credit_explanations: vec![
                    "Read /Users/alice/private/token.txt with sk-test-raw-token-123456789".to_string(),
                ],
            }],
        )
        .expect("status sync applies");

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        let safe_local_credit_projection = serde_json::json!({
            "credit_explanation": &records[0].credit_explanation,
            "credit_events": &records[0].credit_events,
            "history": &records[0].history,
        });
        let serialized =
            serde_json::to_string(&safe_local_credit_projection).expect("records serialize");
        assert!(!serialized.contains("alice@example.com"));
        assert!(!serialized.contains("tenant-raw-alpha"));
        assert!(!serialized.contains("https://private.trace.example.com"));
        assert!(!serialized.contains("/Users/alice/private"));
        assert!(!serialized.contains("sk-test-raw-token"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn delayed_credit_sync_resets_notice_and_notice_marks_records() {
        let scope = format!("trace-credit-sync-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let trace_id = Uuid::new_v4();
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_credit_notice_interval_hours(168),
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
                history: Vec::new(),
                last_credit_notice_at: Some(Utc::now()),
                credit_notice_state: TraceCreditNoticeState::default(),
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
                history: Vec::new(),
                last_credit_notice_at: Some(Utc::now()),
                credit_notice_state: TraceCreditNoticeState::default(),
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
                history: Vec::new(),
                last_credit_notice_at: Some(Utc::now()),
                credit_notice_state: TraceCreditNoticeState::default(),
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
                history: Vec::new(),
                last_credit_notice_at: Some(Utc::now()),
                credit_notice_state: TraceCreditNoticeState::default(),
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
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
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
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
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
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
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
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
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
    async fn queue_trace_envelope_as_held_retains_envelope_and_manual_review_sidecar() {
        // A held (e.g. PII-gated) trace must be retained for review, not
        // dropped: the envelope is queued AND a ManualReview hold sidecar is
        // written so the flush worker skips it until it is authorized.
        let scope = format!("trace-held-retain-test-{}", Uuid::new_v4());
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);

        let reason = "manual review required because residual privacy risk is high";
        let queue_path = queue_trace_envelope_as_held_for_scope(Some(&scope), &envelope, reason)
            .expect("held envelope queues");

        assert!(
            queue_path.exists(),
            "held envelope must be retained on disk"
        );

        let holds = read_trace_queue_holds_for_scope(Some(&scope)).expect("read holds");
        assert_eq!(holds.len(), 1, "exactly one hold sidecar");
        assert_eq!(holds[0].submission_id, envelope.submission_id);
        assert_eq!(holds[0].kind, TraceQueueHoldKind::ManualReview);
        assert!(
            holds[0].reason.contains("residual privacy risk is high"),
            "hold reason preserved, got {}",
            holds[0].reason
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[test]
    fn retain_manual_review_holds_excludes_policy_and_retry_holds() {
        let holds = vec![
            TraceQueueHold {
                submission_id: Uuid::new_v4(),
                kind: TraceQueueHoldKind::ManualReview,
                reason: "residual privacy risk is high".to_string(),
                attempts: 0,
                next_retry_at: None,
            },
            TraceQueueHold {
                submission_id: Uuid::new_v4(),
                kind: TraceQueueHoldKind::PolicyGate,
                reason: "submission score below minimum".to_string(),
                attempts: 0,
                next_retry_at: None,
            },
            TraceQueueHold {
                submission_id: Uuid::new_v4(),
                kind: TraceQueueHoldKind::RetryableSubmissionFailure,
                reason: "retained for retry".to_string(),
                attempts: 1,
                next_retry_at: None,
            },
        ];
        let kept = retain_manual_review_holds(holds);
        assert_eq!(kept.len(), 1, "only the ManualReview hold is surfaced");
        assert_eq!(kept[0].kind, TraceQueueHoldKind::ManualReview);
    }

    #[tokio::test]
    async fn authorize_manual_review_hold_promotes_envelope_past_all_gates() {
        let scope = format!("trace-authorize-test-{}", Uuid::new_v4());
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
        apply_credit_estimate_to_envelope(&mut envelope);

        let manual_policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_require_manual_approval_when_pii_detected(true);

        // Precondition: the High-PII trace is held for manual review.
        queue_trace_envelope_as_held_for_scope(
            Some(&scope),
            &envelope,
            "manual review required because residual privacy risk is high",
        )
        .expect("held envelope queues");
        assert_eq!(
            manual_review_holds_for_scope(Some(&scope)).unwrap().len(),
            1
        );
        assert!(matches!(
            trace_autonomous_eligibility(&envelope, &manual_policy),
            TraceQueueEligibility::Hold {
                kind: TraceQueueHoldKind::ManualReview,
                ..
            }
        ));

        // Authorize -> promotes as-is.
        let authorized =
            authorize_manual_review_hold_for_scope(Some(&scope), envelope.submission_id)
                .expect("authorize succeeds");
        assert!(authorized, "the held trace is authorized");

        // Hold cleared, envelope stamped, eligibility now submits.
        assert!(
            manual_review_holds_for_scope(Some(&scope))
                .unwrap()
                .is_empty(),
            "hold sidecar removed"
        );
        let reloaded_path =
            trace_queue_dir(Some(&scope)).join(format!("{}.json", envelope.submission_id));
        let reloaded = load_trace_envelope(&reloaded_path).expect("reload envelope");
        assert!(
            reloaded.manual_review_authorized,
            "envelope stamped authorized"
        );
        assert!(
            matches!(
                trace_autonomous_eligibility(&reloaded, &manual_policy),
                TraceQueueEligibility::Submit
            ),
            "authorized trace now submits despite High PII"
        );

        // Authorizing an unknown submission is a no-op, not an error.
        assert!(
            !authorize_manual_review_hold_for_scope(Some(&scope), Uuid::new_v4())
                .expect("unknown submission is Ok(false)")
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn manual_review_holds_for_scope_returns_only_manual_review_holds() {
        let scope = format!("trace-manual-holds-test-{}", Uuid::new_v4());
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);

        queue_trace_envelope_as_held_for_scope(
            Some(&scope),
            &envelope,
            "manual review required because residual privacy risk is high",
        )
        .expect("held envelope queues");

        let holds = manual_review_holds_for_scope(Some(&scope)).expect("read manual-review holds");
        assert_eq!(holds.len(), 1, "the one ManualReview hold is returned");
        assert_eq!(holds[0].submission_id, envelope.submission_id);
        assert_eq!(holds[0].kind, TraceQueueHoldKind::ManualReview);
        assert!(holds[0].reason.contains("residual privacy risk is high"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_flush_holds_failed_submission_and_still_returns_due_credit_notice() {
        let scope = format!("trace-flush-submit-failure-test-{}", Uuid::new_v4());
        let token_env = "TRACE_COMMONS_FLUSH_HOLD_TEST_TOKEN";
        let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
            .set_bearer_token_env(token_env.to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0)
            .set_credit_notice_interval_hours(168);
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
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
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

    #[tokio::test]
    async fn queue_flush_records_typed_retry_state_and_defers_until_backoff_expires() {
        let scope = format!("trace-flush-typed-retry-state-test-{}", Uuid::new_v4());
        let token_env = "TRACE_COMMONS_TYPED_RETRY_TEST_TOKEN";
        let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
            .set_bearer_token_env(token_env.to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0);
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

        let first = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
            .await
            .expect("first flush should hold failed submission");
        assert_eq!(first.submitted, 0);
        assert_eq!(first.held, 1);
        assert_eq!(
            first.holds[0].kind,
            TraceQueueHoldKind::RetryableSubmissionFailure
        );
        assert_eq!(first.holds[0].attempts, 1);
        let first_retry_at = first.holds[0]
            .next_retry_at
            .expect("retry failure gets a next retry time");
        assert!(first_retry_at > Utc::now());

        let second = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
            .await
            .expect("second flush should respect retry backoff");
        assert_eq!(second.submitted, 0);
        assert_eq!(second.held, 1);
        assert_eq!(
            second.holds[0].kind,
            TraceQueueHoldKind::RetryableSubmissionFailure
        );
        assert_eq!(
            second.holds[0].attempts, 1,
            "a backoff-held envelope must not consume another retry attempt"
        );
        assert_eq!(second.holds[0].next_retry_at, Some(first_retry_at));

        let holds = read_trace_queue_holds_for_scope(Some(&scope)).expect("holds read");
        assert_eq!(
            holds[0].kind,
            TraceQueueHoldKind::RetryableSubmissionFailure
        );
        assert_eq!(holds[0].attempts, 1);
        assert_eq!(holds[0].next_retry_at, Some(first_retry_at));

        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        assert_eq!(diagnostics.retry_scheduled_count, 1);
        assert_eq!(diagnostics.manual_review_hold_count, 0);
        assert_eq!(diagnostics.policy_hold_count, 0);
        assert_eq!(diagnostics.next_retry_at, Some(first_retry_at));

        let hold_body = std::fs::read_to_string(queue_path.with_extension("held.json"))
            .expect("hold reason writes");
        assert!(hold_body.contains("\"kind\": \"retryable_submission_failure\""));
        assert!(hold_body.contains("\"attempts\": 1"));
        assert!(!hold_body.contains("127.0.0.1"));
        assert!(!hold_body.contains("super-secret-token"));
    }

    #[tokio::test]
    async fn queue_flush_uses_refreshed_upload_claim_for_submit_and_status_sync() {
        let scope = format!("trace-issuer-refresh-test-{}", Uuid::new_v4());
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_for_submit = seen.clone();
        let seen_for_status = seen.clone();
        let app = axum::Router::new()
            .route(
                "/v1/traces",
                axum::routing::post(
                    move |headers: axum::http::HeaderMap,
                          axum::Json(_body): axum::Json<TraceContributionEnvelope>| {
                        let seen = seen_for_submit.clone();
                        async move {
                            let authorization = headers
                                .get(axum::http::header::AUTHORIZATION)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or("<missing>")
                                .to_string();
                            seen.lock().expect("seen lock").push(authorization.clone());
                            if authorization == "Bearer stale-upload-claim" {
                                return (
                                    axum::http::StatusCode::UNAUTHORIZED,
                                    axum::Json(serde_json::json!({"error": "expired"})),
                                );
                            }
                            (
                                axum::http::StatusCode::OK,
                                axum::Json(serde_json::json!({
                                    "status": "accepted",
                                    "credit_points_pending": 1.0,
                                    "explanation": ["accepted"]
                                })),
                            )
                        }
                    },
                ),
            )
            .route(
                "/v1/contributors/me/submission-status",
                axum::routing::post(move |headers: axum::http::HeaderMap| {
                    let seen = seen_for_status.clone();
                    async move {
                        let authorization = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("<missing>")
                            .to_string();
                        seen.lock().expect("seen lock").push(authorization);
                        axum::Json(Vec::<TraceSubmissionStatusUpdate>::new())
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint(endpoint)
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
        )
        .expect("policy writes");
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);
        queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

        let provider =
            RefreshingTestUploadCredentialProvider::new("stale-upload-claim", "fresh-upload-claim");
        let report = flush_trace_contribution_queue_for_scope_with_credential_provider(
            Some(&scope),
            10,
            &provider,
        )
        .await
        .expect("flush retries with refreshed claim");

        assert_eq!(report.submitted, 1);
        assert_eq!(report.held, 0);
        assert_eq!(
            *seen.lock().expect("seen lock"),
            vec![
                "Bearer stale-upload-claim".to_string(),
                "Bearer fresh-upload-claim".to_string(),
                "Bearer fresh-upload-claim".to_string()
            ]
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    /// Regression: an instance-only-enrolled scope has NO enabled per-scope
    /// policy — its policy, device key, and per-user subject come from the
    /// resolved effective flush target. Status sync must run off that resolved
    /// target instead of re-reading the per-scope policy (which would silently
    /// return Ok(0) right after a successful instance-attributed submission,
    /// so final credit status never lands locally).
    #[tokio::test]
    async fn status_sync_with_target_uses_resolved_instance_credential_context() {
        let scope = format!("trace-instance-status-sync-test-{}", Uuid::new_v4());

        // Seed a Submitted record for the scope. Deliberately do NOT write a
        // per-scope policy: the old per-scope re-read would bail with Ok(0).
        let record = submitted_credit_record(1.0, None, None, Vec::new());
        let submission_id = record.submission_id;
        let trace_id = record.trace_id;
        write_local_trace_records_for_scope(Some(&scope), &[record]).expect("record writes");

        let app = axum::Router::new().route(
            "/v1/contributors/me/submission-status",
            axum::routing::post(move || async move {
                axum::Json(vec![TraceSubmissionStatusUpdate {
                    submission_id,
                    trace_id,
                    status: "accepted".to_string(),
                    credit_points_pending: 1.0,
                    credit_points_final: Some(2.0),
                    credit_points_ledger: 0.0,
                    credit_points_total: Some(2.0),
                    explanation: Vec::new(),
                    delayed_credit_explanations: Vec::new(),
                }])
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let instance_policy = StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some(endpoint),
            ..Default::default()
        };
        let instance_dir = tempfile::tempdir().expect("instance device-key dir");
        let provider = CapturingUploadCredentialProvider::default();

        let synced = sync_remote_trace_submission_records_for_scope_unlocked_with_target(
            Some(&scope),
            &instance_policy,
            instance_dir.path(),
            Some("subject-abc"),
            &provider,
        )
        .await
        .expect("instance-target status sync succeeds");
        assert_eq!(synced, 1, "the submitted record must sync its final status");

        let contexts = provider.contexts.lock().expect("contexts lock");
        assert_eq!(contexts.len(), 1, "one bearer mint for one status chunk");
        assert_eq!(
            contexts[0].0.as_deref(),
            Some("subject-abc"),
            "claim context must carry the resolved per-user subject"
        );
        assert_eq!(
            contexts[0].1.as_deref(),
            Some(instance_dir.path()),
            "claim context must use the resolved instance device-key dir"
        );
        drop(contexts);

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert_eq!(
            records[0].credit_points_final,
            Some(2.0),
            "final credit from the remote update must land on the local record"
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_flush_classifies_upload_http_rejection_through_submit_call_site() {
        let scope = format!("trace-upload-http-classification-test-{}", Uuid::new_v4());
        let token_env = "TRACE_COMMONS_UPLOAD_HTTP_CLASSIFICATION_TOKEN";
        let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
        let app = axum::Router::new().route(
            "/v1/traces",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({"error": "token expired"})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint(endpoint)
                .set_bearer_token_env(token_env.to_string())
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
        )
        .expect("policy writes");
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);
        queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

        flush_trace_contribution_queue_for_scope(Some(&scope), 10)
            .await
            .expect("upload HTTP rejection is held for retry");
        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        let failure = diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("upload rejection recorded");
        assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::HttpRejection);
        assert!(failure.reason.contains("error_hash="));
        assert!(!failure.reason.contains("token expired"));
        assert!(!failure.reason.contains("super-secret-token"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_flush_classifies_status_sync_request_failure_through_call_site() {
        let scope = format!("trace-status-sync-classification-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(1.0, None, None, Vec::new())],
        )
        .expect("local record writes");

        flush_trace_contribution_queue_for_scope_with_credential_provider(
            Some(&scope),
            10,
            &RefreshingTestUploadCredentialProvider::new(
                "super-secret-token",
                "super-secret-token",
            ),
        )
        .await
        .expect("status sync failure is nonfatal during flush");
        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        let failure = diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("status sync failure recorded");
        assert_eq!(
            failure.kind,
            TraceQueueTelemetryFailureKind::NetworkConnectionRefused
        );
        assert!(failure.reason.contains("status sync failed"));
        assert!(!failure.reason.contains("127.0.0.1"));
        assert!(!failure.reason.contains("super-secret-token"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_flush_classifies_status_sync_auth_rejection_as_credential() {
        let scope = format!(
            "trace-status-sync-auth-classification-test-{}",
            Uuid::new_v4()
        );
        let token_env = "TRACE_COMMONS_STATUS_SYNC_AUTH_CLASSIFICATION_TOKEN";
        let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
        let app = axum::Router::new().route(
            "/v1/contributors/me/submission-status",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": "not authorized"})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint(endpoint)
                .set_bearer_token_env(token_env.to_string())
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(1.0, None, None, Vec::new())],
        )
        .expect("local record writes");

        flush_trace_contribution_queue_for_scope(Some(&scope), 10)
            .await
            .expect("status sync auth failure is nonfatal during flush");
        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        let failure = diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("status sync auth failure recorded");
        assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::Credential);
        assert!(failure.reason.contains("status sync failed"));
        assert!(!failure.reason.contains("super-secret-token"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_flush_classifies_typed_submission_provider_connection_loss_before_text() {
        let cases = [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted,
        ];

        for io_kind in cases {
            let scope = format!(
                "trace-submission-provider-connection-loss-classification-test-{}",
                Uuid::new_v4()
            );
            write_trace_policy_for_scope(
                Some(&scope),
                &StandingTraceContributionPolicy::default()
                    .set_enabled(true)
                    .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                    .set_auto_submit_high_value_traces(true)
                    .set_min_submission_score(0.0),
            )
            .expect("policy writes");
            let raw = RawTraceContribution::from_recorded_trace(
                &sample_trace(),
                RecordedTraceContributionOptions::default(),
            );
            let mut envelope = DeterministicTraceRedactor::default()
                .redact_trace(raw)
                .await
                .expect("redaction should succeed");
            apply_credit_estimate_to_envelope(&mut envelope);
            queue_trace_envelope_for_scope(Some(&scope), &envelope)
                .expect("queued envelope writes");

            let report = flush_trace_contribution_queue_for_scope_with_credential_provider(
                Some(&scope),
                10,
                &FailingTestUploadCredentialProvider { kind: io_kind },
            )
            .await
            .expect("submission provider failure is held for retry");
            assert_eq!(report.submitted, 0);
            assert_eq!(report.held, 1);

            let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
            let failure = diagnostics
                .telemetry
                .last_failure
                .as_ref()
                .expect("submission provider failure recorded");
            assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::Network);
            assert!(failure.reason.contains("submission retry scheduled"));
            assert!(failure.reason.contains("error_hash="));
            assert!(!failure.reason.contains("super-secret-token"));

            let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
        }
    }

    #[tokio::test]
    async fn queue_flush_classifies_typed_status_sync_provider_io_errors_before_text() {
        let cases = [
            (
                std::io::ErrorKind::TimedOut,
                TraceQueueTelemetryFailureKind::NetworkTimeout,
            ),
            (
                std::io::ErrorKind::NotConnected,
                TraceQueueTelemetryFailureKind::NetworkOffline,
            ),
            (
                std::io::ErrorKind::AddrNotAvailable,
                TraceQueueTelemetryFailureKind::NetworkOffline,
            ),
            (
                std::io::ErrorKind::NetworkDown,
                TraceQueueTelemetryFailureKind::NetworkOffline,
            ),
            (
                std::io::ErrorKind::NetworkUnreachable,
                TraceQueueTelemetryFailureKind::NetworkOffline,
            ),
            (
                std::io::ErrorKind::HostUnreachable,
                TraceQueueTelemetryFailureKind::NetworkOffline,
            ),
            (
                std::io::ErrorKind::ConnectionRefused,
                TraceQueueTelemetryFailureKind::NetworkConnectionRefused,
            ),
        ];

        for (io_kind, expected_kind) in cases {
            let scope = format!(
                "trace-status-sync-provider-io-classification-test-{}",
                Uuid::new_v4()
            );
            write_trace_policy_for_scope(
                Some(&scope),
                &StandingTraceContributionPolicy::default()
                    .set_enabled(true)
                    .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
                    .set_auto_submit_high_value_traces(true)
                    .set_min_submission_score(0.0),
            )
            .expect("policy writes");
            write_local_trace_records_for_scope(
                Some(&scope),
                &[submitted_credit_record(1.0, None, None, Vec::new())],
            )
            .expect("local record writes");

            flush_trace_contribution_queue_for_scope_with_credential_provider(
                Some(&scope),
                10,
                &FailingTestUploadCredentialProvider { kind: io_kind },
            )
            .await
            .expect("status sync provider failure is nonfatal during flush");
            let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
            let failure = diagnostics
                .telemetry
                .last_failure
                .as_ref()
                .expect("status sync provider failure recorded");
            assert_eq!(failure.kind, expected_kind);
            assert!(failure.reason.contains("status sync failed"));
            assert!(failure.reason.contains("error_hash="));
            assert!(
                !failure
                    .reason
                    .contains("credential provider failed while using super-secret-token")
            );
            assert!(!failure.reason.contains("super-secret-token"));

            let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
        }
    }

    #[tokio::test]
    async fn revoke_trace_submission_uses_refreshed_upload_claim() {
        let scope = format!("trace-revoke-refresh-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_for_revoke = seen.clone();
        let app = axum::Router::new().route(
            "/v1/traces/revoke",
            axum::routing::delete(move |headers: axum::http::HeaderMap| {
                let seen = seen_for_revoke.clone();
                async move {
                    let authorization = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("<missing>")
                        .to_string();
                    seen.lock().expect("seen lock").push(authorization.clone());
                    if authorization == "Bearer stale-upload-claim" {
                        return (
                            axum::http::StatusCode::UNAUTHORIZED,
                            axum::Json(serde_json::json!({"error": "expired"})),
                        );
                    }
                    (
                        axum::http::StatusCode::NO_CONTENT,
                        axum::Json(serde_json::json!({})),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces/revoke",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        write_local_trace_records_for_scope(
            Some(&scope),
            &[LocalTraceSubmissionRecord {
                submission_id,
                trace_id: Uuid::new_v4(),
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("accepted".to_string()),
                submitted_at: Some(Utc::now()),
                revoked_at: None,
                privacy_risk: "low".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 1.0,
                credit_points_final: None,
                credit_explanation: Vec::new(),
                credit_events: Vec::new(),
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
            }],
        )
        .expect("local record writes");

        let provider =
            RefreshingTestUploadCredentialProvider::new("stale-upload-claim", "fresh-upload-claim");
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint(endpoint.clone());
        revoke_trace_submission_for_scope_with_credential_provider(
            Some(&scope),
            submission_id,
            Some(&endpoint),
            &policy,
            &provider,
        )
        .await
        .expect("revoke retries with refreshed claim");

        assert_eq!(
            *seen.lock().expect("seen lock"),
            vec![
                "Bearer stale-upload-claim".to_string(),
                "Bearer fresh-upload-claim".to_string()
            ]
        );
        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert_eq!(records[0].status, LocalTraceSubmissionStatus::Revoked);
        assert!(records[0].revoked_at.is_some());

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn revoke_trace_submission_classifies_http_rejection_through_call_site() {
        let scope = format!("trace-revoke-http-classification-test-{}", Uuid::new_v4());
        let submission_id = Uuid::new_v4();
        let app = axum::Router::new().route(
            "/v1/traces/revoke",
            axum::routing::delete(|| async {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({"error": "token expired"})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces/revoke",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        write_local_trace_records_for_scope(
            Some(&scope),
            &[LocalTraceSubmissionRecord {
                submission_id,
                trace_id: Uuid::new_v4(),
                endpoint: Some("https://trace.example.com/v1/traces".to_string()),
                status: LocalTraceSubmissionStatus::Submitted,
                server_status: Some("accepted".to_string()),
                submitted_at: Some(Utc::now()),
                revoked_at: None,
                privacy_risk: "low".to_string(),
                redaction_counts: BTreeMap::new(),
                credit_points_pending: 1.0,
                credit_points_final: None,
                credit_explanation: Vec::new(),
                credit_events: Vec::new(),
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
            }],
        )
        .expect("local record writes");

        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint(endpoint.clone());
        let provider =
            RefreshingTestUploadCredentialProvider::new("stale-upload-claim", "fresh-upload-claim");
        let error = revoke_trace_submission_for_scope_with_credential_provider(
            Some(&scope),
            submission_id,
            Some(&endpoint),
            &policy,
            &provider,
        )
        .await
        .expect_err("revoke HTTP rejection should surface to caller");

        assert_eq!(
            trace_queue_telemetry_failure_kind(&error),
            TraceQueueTelemetryFailureKind::HttpRejection
        );
        assert!(!error.to_string().contains("stale-upload-claim"));
        assert!(!error.to_string().contains("fresh-upload-claim"));
        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert_eq!(records[0].status, LocalTraceSubmissionStatus::Submitted);
        assert!(records[0].revoked_at.is_none());

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn policy_aware_submit_rejects_redirects_without_resending_bearer_token() {
        let token_env = "TRACE_COMMONS_REDIRECT_TEST_TOKEN";
        let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
        let redirected_authorizations = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let redirected_authorizations_for_handler = redirected_authorizations.clone();
        let app = axum::Router::new()
            .route(
                "/v1/traces",
                axum::routing::post(|| async {
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [(axum::http::header::LOCATION, "/redirected-trace-ingest")],
                    )
                }),
            )
            .route(
                "/redirected-trace-ingest",
                axum::routing::post(move |headers: axum::http::HeaderMap| {
                    let redirected_authorizations = redirected_authorizations_for_handler.clone();
                    async move {
                        let authorization = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("<missing>")
                            .to_string();
                        redirected_authorizations
                            .lock()
                            .expect("redirected authorizations lock")
                            .push(authorization);
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "status": "accepted",
                                "credit_points_pending": 1.0,
                                "explanation": ["accepted after redirect"]
                            })),
                        )
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);

        let error = submit_trace_envelope_to_endpoint_with_policy(
            &envelope,
            &endpoint,
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint(endpoint.clone())
                .set_bearer_token_env(token_env.to_string()),
        )
        .await
        .expect_err("credentialed trace submission redirects should be rejected");

        assert!(error.to_string().contains("307"));
        assert!(
            redirected_authorizations
                .lock()
                .expect("redirected authorizations lock")
                .is_empty(),
            "bearer token must not be resent to redirected trace endpoints"
        );
    }

    #[tokio::test]
    async fn policy_aware_submit_uses_bounded_request_timeout() {
        let _token_guard = EnvVarRestore::set("TRACE_COMMONS_TEST_TOKEN", "super-secret-token");
        // The 50ms remote-request timeout is supplied via a task-scoped
        // override rather than the process-global
        // `IRONCLAW_TRACE_REMOTE_REQUEST_TIMEOUT_MS` env var, so it cannot leak
        // into other tests' HTTP clients under parallel execution. See the
        // `TEST_REMOTE_REQUEST_TIMEOUT_OVERRIDE` task-local docs.
        // Regression detection is decoupled from a tight wall-clock race: the
        // mock sleeps 10s (>> the 200ms request timeout) and then returns 200
        // OK. A submit that HONORS its bounded timeout returns an
        // `is_timeout()` reqwest error in ~200ms; a submit that IGNORES it
        // sleeps the full 10s and returns the mock's success body, tripping the
        // `Ok(Ok(_))` arm below. The outer 30s watchdog exists ONLY to fail a
        // genuine infinite hang, not to time the request — so it never flakes
        // under an oversubscribed test runtime where reqwest's timer is merely
        // delayed by a few seconds.
        let app = axum::Router::new().route(
            "/v1/traces",
            axum::routing::post(|| async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                (
                    axum::http::StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "status": "accepted",
                        "credit_points_pending": 1.0,
                        "explanation": ["accepted slowly"]
                    })),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);

        let result = TEST_REMOTE_REQUEST_TIMEOUT_OVERRIDE
            .scope(
                Duration::from_millis(200),
                tokio::time::timeout(
                    Duration::from_secs(30),
                    submit_trace_envelope_to_endpoint_with_policy(
                        &envelope,
                        &endpoint,
                        &StandingTraceContributionPolicy::default()
                            .set_enabled(true)
                            .set_ingestion_endpoint(endpoint.clone())
                            .set_bearer_token_env("TRACE_COMMONS_TEST_TOKEN".to_string()),
                    ),
                ),
            )
            .await;
        let error = match result {
            Ok(Err(error)) => error,
            // The submit returned the mock's success body, so it slept the full
            // 10s instead of honoring the 200ms request timeout.
            Ok(Ok(_)) => {
                panic!("slow trace submission should time out via the bounded request timeout")
            }
            // The 30s anti-hang watchdog tripped: the submit neither honored
            // its request timeout nor received the (10s-delayed) response.
            Err(_) => panic!("trace submission hung past the 30s anti-hang watchdog"),
        };

        assert!(
            error.chain().any(|cause| cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|error| error.is_timeout())),
            "unexpected timeout error: {error}"
        );
    }

    #[tokio::test]
    async fn policy_aware_direct_submit_uses_default_credential_provider() {
        let _token_env = EnvVarRestore::set(
            "IRONCLAW_TRACE_COMMONS_DIRECT_SUBMIT_TEST_TOKEN",
            "direct-submit-token",
        );
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_for_submit = seen.clone();
        let app = axum::Router::new().route(
            "/v1/traces",
            axum::routing::post(
                move |headers: axum::http::HeaderMap,
                      axum::Json(_body): axum::Json<TraceContributionEnvelope>| {
                    let seen = seen_for_submit.clone();
                    async move {
                        let authorization = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("<missing>")
                            .to_string();
                        seen.lock().expect("seen lock").push(authorization);
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "status": "accepted",
                                "credit_points_pending": 1.0,
                                "explanation": ["accepted"]
                            })),
                        )
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);
        let policy = StandingTraceContributionPolicy::default()
            .set_bearer_token_env("IRONCLAW_TRACE_COMMONS_DIRECT_SUBMIT_TEST_TOKEN".to_string());

        let receipt = submit_trace_envelope_to_endpoint_with_policy(&envelope, &endpoint, &policy)
            .await
            .expect("direct submit uses policy credentials");

        assert_eq!(receipt.status, "accepted");
        assert_eq!(
            *seen.lock().expect("seen lock"),
            vec!["Bearer direct-submit-token".to_string()]
        );
    }

    #[test]
    fn upload_claim_issuer_url_validation_rejects_unsafe_targets() {
        let allowed_hosts = BTreeSet::from(["issuer.example.com".to_string()]);
        assert!(
            validate_trace_upload_claim_issuer_url(
                &reqwest::Url::parse("https://issuer.example.com/v1/claims").expect("url"),
                &allowed_hosts,
            )
            .is_ok()
        );

        for unsafe_url in [
            "http://issuer.example.com/v1/claims",
            "https://user:secret@issuer.example.com/v1/claims",
            "https://issuer.example.com/v1/claims?token=secret",
            "https://issuer.example.com/v1/claims#fragment",
            "https://metadata.google.internal/v1/claims",
        ] {
            assert!(
                validate_trace_upload_claim_issuer_url(
                    &reqwest::Url::parse(unsafe_url).expect("url"),
                    &allowed_hosts,
                )
                .is_err(),
                "{unsafe_url} should be rejected"
            );
        }
    }

    #[test]
    fn upload_claim_issuer_url_validation_allows_literal_loopback_dev() {
        // The loopback-HTTP dev invite form writes a loopback claim endpoint
        // into the policy; the validator must accept the same exception or a
        // successful loopback onboarding can never mint a claim.
        for (url, host) in [
            ("http://127.0.0.1:3917/v1/trace-upload-claim", "127.0.0.1"),
            ("http://localhost:3917/v1/trace-upload-claim", "localhost"),
            ("http://[::1]:3917/v1/trace-upload-claim", "[::1]"),
            ("https://127.0.0.1/v1/trace-upload-claim", "127.0.0.1"),
        ] {
            let allowed_hosts = BTreeSet::from([host.to_string()]);
            assert!(
                validate_trace_upload_claim_issuer_url(
                    &reqwest::Url::parse(url).expect("url"),
                    &allowed_hosts,
                )
                .is_ok(),
                "{url} should be accepted under the loopback dev exception"
            );
        }

        // The exception is literal loopback only: plain-HTTP private/internal
        // hosts and loopback-suffixed hostnames stay rejected, and loopback
        // still has to pass the allowlist.
        let allowed = BTreeSet::from(["10.0.0.5".to_string(), "foo.localhost".to_string()]);
        for unsafe_url in [
            "http://10.0.0.5/v1/trace-upload-claim",
            "http://192.168.1.10/v1/trace-upload-claim",
            "http://foo.localhost/v1/trace-upload-claim",
            "https://foo.localhost/v1/trace-upload-claim",
        ] {
            assert!(
                validate_trace_upload_claim_issuer_url(
                    &reqwest::Url::parse(unsafe_url).expect("url"),
                    &allowed,
                )
                .is_err(),
                "{unsafe_url} should be rejected"
            );
        }
        assert!(
            validate_trace_upload_claim_issuer_url(
                &reqwest::Url::parse("http://127.0.0.1:3917/v1/trace-upload-claim").expect("url"),
                &BTreeSet::from(["issuer.example.com".to_string()]),
            )
            .is_err(),
            "loopback host not on the allowlist should be rejected"
        );
    }

    #[test]
    fn ingest_url_validation_allows_literal_loopback_dev() {
        assert!(
            validate_trace_commons_ingest_url(
                &reqwest::Url::parse("http://127.0.0.1:3917/v1/traces").expect("url")
            )
            .is_ok()
        );
        assert!(
            validate_trace_commons_ingest_url(
                &reqwest::Url::parse("http://10.0.0.5/v1/traces").expect("url")
            )
            .is_err()
        );
        assert!(
            validate_trace_commons_ingest_url(
                &reqwest::Url::parse("https://ingest.example.com/v1/traces").expect("url")
            )
            .is_ok()
        );
    }

    #[tokio::test]
    async fn fetch_trace_upload_claim_from_issuer_accepts_loopback_dev_issuer() {
        // Regression: a loopback-HTTP dev onboarding writes
        // `http://127.0.0.1:<port>/v1/trace-upload-claim` into the policy. The
        // real claim fetch must honor the same loopback exception end-to-end
        // (URL validator + pinned DNS resolution), or a successfully onboarded
        // loopback enrollment can never mint a claim.
        let token = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "dev-key-1"}));
        let claim_token = token.clone();
        let app = axum::Router::new().route(
            "/v1/trace-upload-claim",
            axum::routing::post(move || {
                let token = claim_token.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "access_token": token,
                        "token_type": "Bearer",
                        "expires_in": 300,
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock issuer listener binds");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let scope_dir = tempfile::tempdir().expect("tempdir");
        crate::onboarding::DeviceKeypair::load_or_generate_pending(
            scope_dir.path(),
            "loopback-invite-hash",
        )
        .expect("generate pending device key")
        .promote(scope_dir.path(), "tenant-dev")
        .expect("promote device key");

        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_upload_token_issuer_url(format!("http://{addr}/v1/trace-upload-claim"))
            .set_upload_token_issuer_allowed_hosts(BTreeSet::from(["127.0.0.1".to_string()]))
            .set_upload_token_tenant_id("tenant-dev".to_string())
            .set_upload_token_audience("trace-commons".to_string());
        let context = TraceUploadClaimContext {
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: Vec::new(),
            scope_dir: Some(scope_dir.path().to_path_buf()),
            subject: None,
        };
        let claim = fetch_trace_upload_claim_from_issuer(&policy, &context, None)
            .await
            .expect("loopback dev issuer mints a claim");
        assert_eq!(claim.access_token, token);
    }

    #[tokio::test]
    async fn fetch_claim_sends_subject_when_present() {
        use std::sync::{Arc, Mutex};
        let captured: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        let token = test_jwt_with_header(serde_json::json!({"alg":"EdDSA","kid":"dev-key-1"}));
        let claim_token = token.clone();
        let app = axum::Router::new().route(
            "/v1/trace-upload-claim",
            axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let cap = cap.clone();
                let token = claim_token.clone();
                async move {
                    cap.lock().unwrap().push(body);
                    axum::Json(serde_json::json!({
                        "access_token": token, "token_type": "Bearer", "expires_in": 300
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let scope_dir = tempfile::tempdir().unwrap();
        crate::onboarding::DeviceKeypair::load_or_generate_pending(scope_dir.path(), "h")
            .unwrap()
            .promote(scope_dir.path(), "tenant-dev")
            .unwrap();

        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
            upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
                "127.0.0.1".to_string()
            ]),
            upload_token_tenant_id: Some("tenant-dev".to_string()),
            upload_token_audience: Some("trace-commons".to_string()),
            ..Default::default()
        };
        let context = TraceUploadClaimContext {
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: Vec::new(),
            scope_dir: Some(scope_dir.path().to_path_buf()),
            subject: Some("sha256:alice".to_string()),
        };
        let _ = fetch_trace_upload_claim_from_issuer(&policy, &context, None)
            .await
            .unwrap();

        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["subject"], "sha256:alice");
    }

    #[test]
    fn upload_claim_response_requires_eddsa_jwt_with_kid() {
        let token = test_jwt_with_header(serde_json::json!({
            "alg": "EdDSA",
            "kid": "managed-key-1"
        }));
        validate_trace_upload_claim_response(&TraceUploadClaimIssuerResponse {
            access_token: token,
            token_type: Some("Bearer".to_string()),
            expires_at: None,
            expires_in: Some(300),
        })
        .expect("EdDSA token with kid is accepted for client-side transport");

        let non_eddsa = test_jwt_with_header(serde_json::json!({
            "alg": "HS256",
            "kid": "managed-key-1"
        }));
        let error = validate_trace_upload_claim_response(&TraceUploadClaimIssuerResponse {
            access_token: non_eddsa,
            token_type: Some("Bearer".to_string()),
            expires_at: None,
            expires_in: Some(300),
        })
        .expect_err("non-EdDSA upload claims are rejected");
        assert!(error.to_string().contains("EdDSA"));

        let missing_kid = test_jwt_with_header(serde_json::json!({
            "alg": "EdDSA"
        }));
        let error = validate_trace_upload_claim_response(&TraceUploadClaimIssuerResponse {
            access_token: missing_kid,
            token_type: Some("Bearer".to_string()),
            expires_at: None,
            expires_in: Some(300),
        })
        .expect_err("managed upload claims require kid");
        assert!(error.to_string().contains("kid"));
    }

    fn test_jwt_with_header(header: serde_json::Value) -> String {
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.signature",
            engine.encode(header.to_string().as_bytes()),
            engine.encode(b"{}")
        )
    }

    #[tokio::test]
    async fn queue_flush_compacts_duplicate_envelopes_and_orphan_holds_before_submit() {
        let scope = format!("trace-queue-compaction-test-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_bearer_token_env("TRACE_COMMONS_MISSING_TOKEN".to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0);
        write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut older = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut older);
        older.created_at = Utc::now() - chrono::Duration::minutes(5);
        let older_path =
            queue_trace_envelope_for_scope(Some(&scope), &older).expect("older queued");

        let mut newer = older.clone();
        newer.submission_id = Uuid::new_v4();
        newer.created_at = Utc::now();
        let newer_path =
            queue_trace_envelope_for_scope(Some(&scope), &newer).expect("newer queued");

        let orphan_id = Uuid::new_v4();
        let orphan_path = trace_queue_dir(Some(&scope)).join(format!("{orphan_id}.held.json"));
        std::fs::write(
            &orphan_path,
            serde_json::json!({ "reason": "old sidecar" }).to_string(),
        )
        .expect("orphan hold writes");

        let report = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
            .await
            .expect("flush handles retryable submit failure after compaction");

        assert_eq!(report.compaction.duplicate_envelopes_removed, 1);
        assert_eq!(report.compaction.orphan_hold_sidecars_removed, 1);
        assert!(!older_path.exists(), "older duplicate should be removed");
        assert!(newer_path.exists(), "newest duplicate should remain queued");
        assert!(
            !orphan_path.exists(),
            "orphan hold sidecar should be removed"
        );

        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        assert_eq!(diagnostics.queued_count, 1);
        assert_eq!(
            diagnostics
                .telemetry
                .last_compaction
                .as_ref()
                .expect("last compaction is recorded")
                .duplicate_envelopes_removed,
            1
        );
        assert_eq!(diagnostics.telemetry.compaction_reclaimed_items_total, 2);

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_flush_quarantines_malformed_envelope_and_submits_later_valid_envelope() {
        let scope = format!("trace-queue-malformed-recovery-test-{}", Uuid::new_v4());
        let submitted_ids = Arc::new(std::sync::Mutex::new(Vec::<Uuid>::new()));
        let submitted_ids_for_route = submitted_ids.clone();
        let app = axum::Router::new()
            .route(
                "/v1/traces",
                axum::routing::post(
                    move |axum::Json(body): axum::Json<TraceContributionEnvelope>| {
                        let submitted_ids = submitted_ids_for_route.clone();
                        async move {
                            submitted_ids
                                .lock()
                                .expect("submitted ids lock")
                                .push(body.submission_id);
                            axum::Json(serde_json::json!({
                                "status": "accepted",
                                "credit_points_pending": 1.0,
                                "explanation": ["accepted"]
                            }))
                        }
                    },
                ),
            )
            .route(
                "/v1/contributors/me/submission-status",
                axum::routing::post(|| async {
                    axum::Json(Vec::<TraceSubmissionStatusUpdate>::new())
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock trace commons listener binds");
        let endpoint = format!(
            "http://{}/v1/traces",
            listener.local_addr().expect("local addr")
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint(endpoint)
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
        )
        .expect("policy writes");

        let queue_dir = trace_queue_dir(Some(&scope));
        std::fs::create_dir_all(&queue_dir).expect("queue dir exists");
        let malformed_path = queue_dir.join(format!("{}.json", Uuid::nil()));
        let malformed_body = r#"{"redacted_content":"[REDACTED local-only body]","#;
        std::fs::write(&malformed_path, malformed_body).expect("malformed fixture writes");

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);
        queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("valid envelope queues");

        let provider = RefreshingTestUploadCredentialProvider::new("trace-token", "trace-token");
        let report = flush_trace_contribution_queue_for_scope_with_credential_provider(
            Some(&scope),
            10,
            &provider,
        )
        .await
        .expect("flush should skip malformed envelope and submit valid envelope");

        assert_eq!(report.submitted, 1);
        assert_eq!(report.compaction.malformed_envelopes_quarantined, 1);
        assert!(
            !malformed_path.exists(),
            "malformed envelope should leave active queue"
        );
        let quarantine_path =
            trace_queue_malformed_dir(Some(&scope)).join(format!("{}.json", Uuid::nil()));
        assert!(
            quarantine_path.exists(),
            "malformed envelope should be quarantined"
        );
        assert_eq!(
            std::fs::read_to_string(&quarantine_path).expect("quarantine body reads"),
            malformed_body
        );
        assert_eq!(
            *submitted_ids.lock().expect("submitted ids lock"),
            vec![envelope.submission_id]
        );
        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        assert_eq!(diagnostics.queued_count, 0);

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_compaction_keeps_same_trace_when_semantic_metadata_differs() {
        let scope = format!("trace-queue-exact-compaction-test-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_bearer_token_env("TRACE_COMMONS_MISSING_TOKEN".to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0);
        write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut base = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut base);
        base.created_at = Utc::now() - chrono::Duration::minutes(5);
        let base_path = queue_trace_envelope_for_scope(Some(&scope), &base).expect("base queued");

        let mut changed = base.clone();
        changed.submission_id = Uuid::new_v4();
        changed.created_at = Utc::now();
        changed.outcome.task_success = TaskSuccess::Failure;
        changed
            .value_card
            .limitations
            .push("Different replay utility metadata.".to_string());
        changed.trace_card.redaction_pipeline_version = "legacy-trace-card-redactor".to_string();
        let changed_path =
            queue_trace_envelope_for_scope(Some(&scope), &changed).expect("changed queued");

        let report = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
            .await
            .expect("flush handles retryable submit failures");

        assert_eq!(report.compaction.duplicate_envelopes_removed, 0);
        assert!(base_path.exists(), "base envelope should remain queued");
        assert!(
            changed_path.exists(),
            "semantically different envelope should remain queued"
        );

        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        assert_eq!(diagnostics.queued_count, 2);
        assert!(
            diagnostics.warnings.iter().any(|warning| {
                warning.kind == TraceQueueWarningKind::TraceCardRedactionPipelineMismatch
            }),
            "warning from changed envelope should not be hidden by compaction"
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_compaction_failure_records_sanitized_queue_telemetry() {
        let scope = format!("trace-queue-compaction-failure-test-{}", Uuid::new_v4());
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_bearer_token_env("TRACE_COMMONS_MISSING_TOKEN".to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0);
        write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut older = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut older);
        older.created_at = Utc::now() - chrono::Duration::minutes(5);
        let older_path =
            queue_trace_envelope_for_scope(Some(&scope), &older).expect("older queued");

        let mut newer = older.clone();
        newer.submission_id = Uuid::new_v4();
        newer.created_at = Utc::now();
        let _newer_path =
            queue_trace_envelope_for_scope(Some(&scope), &newer).expect("newer queued");

        let older_hold_path = trace_queue_hold_path_for_envelope_path(&older_path);
        std::fs::create_dir_all(&older_hold_path).expect("hold directory fixture creates");

        let error = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
            .await
            .expect_err("compaction hold removal failure should fail flush");
        assert!(error.to_string().contains("duplicate queue hold"));

        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        let failure = diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("compaction failure is recorded");
        assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::Queue);
        assert!(failure.reason.contains("flush failed"));
        assert!(!failure.reason.contains(&scope));
        assert!(!failure.reason.contains("TRACE_COMMONS_MISSING_TOKEN"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_diagnostics_reports_schema_policy_and_redaction_warnings() {
        let scope = format!("trace-queue-warning-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string()),
        )
        .expect("policy writes");

        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        envelope.schema_version = "ironclaw.trace_contribution.v0".to_string();
        envelope.consent.policy_version = "2025-01-01".to_string();
        envelope.privacy.redaction_pipeline_version = "legacy-redactor".to_string();
        envelope.trace_card.redaction_pipeline_version = "legacy-redactor".to_string();
        queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        assert!(
            diagnostics
                .warnings
                .iter()
                .any(|warning| warning.kind == TraceQueueWarningKind::SchemaVersionMismatch)
        );
        assert!(
            diagnostics
                .warnings
                .iter()
                .any(|warning| warning.kind == TraceQueueWarningKind::PolicyVersionMismatch)
        );
        assert!(
            diagnostics
                .warnings
                .iter()
                .any(|warning| warning.kind == TraceQueueWarningKind::RedactionPipelineMismatch)
        );
        assert!(
            diagnostics
                .warnings
                .iter()
                .all(|warning| warning.promotion_blocking)
        );
        assert!(
            diagnostics
                .warnings
                .iter()
                .all(|warning| !warning.recommended_action.trim().is_empty())
        );
        let serialized = serde_json::to_string(&diagnostics).expect("diagnostics serialize");
        assert!(!serialized.contains("legacy-redactor"));
        assert!(!serialized.contains("2025-01-01"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn queue_telemetry_classifies_endpoint_credential_and_network_failures() {
        let endpoint_scope = format!("trace-queue-endpoint-classification-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&endpoint_scope),
            &StandingTraceContributionPolicy::default().set_enabled(true),
        )
        .expect("endpoint policy writes");
        let endpoint_result =
            flush_trace_contribution_queue_for_scope(Some(&endpoint_scope), 10).await;
        assert!(endpoint_result.is_err());
        let endpoint_diagnostics =
            trace_queue_diagnostics_for_scope(Some(&endpoint_scope)).expect("diagnostics");
        assert_eq!(
            endpoint_diagnostics
                .telemetry
                .last_failure
                .as_ref()
                .expect("endpoint failure recorded")
                .kind,
            TraceQueueTelemetryFailureKind::Endpoint
        );

        let credential_scope = format!("trace-queue-credential-classification-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&credential_scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_bearer_token_env("TRACE_COMMONS_MISSING_TOKEN".to_string())
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
        )
        .expect("credential policy writes");
        let raw = RawTraceContribution::from_recorded_trace(
            &sample_trace(),
            RecordedTraceContributionOptions::default(),
        );
        let mut envelope = DeterministicTraceRedactor::default()
            .redact_trace(raw)
            .await
            .expect("redaction should succeed");
        apply_credit_estimate_to_envelope(&mut envelope);
        queue_trace_envelope_for_scope(Some(&credential_scope), &envelope)
            .expect("queued envelope writes");
        flush_trace_contribution_queue_for_scope(Some(&credential_scope), 10)
            .await
            .expect("credential submission failure is held for retry");
        let credential_diagnostics =
            trace_queue_diagnostics_for_scope(Some(&credential_scope)).expect("diagnostics");
        assert_eq!(
            credential_diagnostics
                .telemetry
                .last_failure
                .as_ref()
                .expect("credential failure recorded")
                .kind,
            TraceQueueTelemetryFailureKind::Credential
        );

        let network_scope = format!("trace-queue-network-classification-{}", Uuid::new_v4());
        let token_env = "TRACE_COMMONS_QUEUE_NETWORK_CLASSIFICATION_TOKEN";
        let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
        write_trace_policy_for_scope(
            Some(&network_scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
                .set_bearer_token_env(token_env.to_string())
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
        )
        .expect("network policy writes");
        let mut envelope = envelope.clone();
        envelope.submission_id = Uuid::new_v4();
        queue_trace_envelope_for_scope(Some(&network_scope), &envelope)
            .expect("queued envelope writes");
        flush_trace_contribution_queue_for_scope(Some(&network_scope), 10)
            .await
            .expect("network submission failure is held for retry");
        let network_diagnostics =
            trace_queue_diagnostics_for_scope(Some(&network_scope)).expect("diagnostics");
        assert_eq!(
            network_diagnostics
                .telemetry
                .last_failure
                .as_ref()
                .expect("network failure recorded")
                .kind,
            TraceQueueTelemetryFailureKind::NetworkConnectionRefused
        );

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&endpoint_scope)));
        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&credential_scope)));
        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&network_scope)));
    }

    #[test]
    fn queue_telemetry_classifies_network_subtypes_without_raw_error_details() {
        let now = Utc::now();
        let cases = [
            (
                anyhow::anyhow!(
                    "request failed: DNS lookup failed for https://private.example/v1/traces"
                ),
                TraceQueueTelemetryFailureKind::NetworkDns,
            ),
            (
                anyhow::anyhow!("request failed: operation timed out contacting trace service"),
                TraceQueueTelemetryFailureKind::NetworkTimeout,
            ),
            (
                anyhow::anyhow!("request failed: connection refused by 127.0.0.1:9"),
                TraceQueueTelemetryFailureKind::NetworkConnectionRefused,
            ),
            (
                anyhow::anyhow!("request failed: network is unreachable while offline"),
                TraceQueueTelemetryFailureKind::NetworkOffline,
            ),
        ];

        for (error, expected) in cases {
            let failure =
                trace_queue_telemetry_failure_with_label(&error, now, "submission retry scheduled");
            assert_eq!(failure.kind, expected);
            assert!(failure.reason.contains("error_hash="));
            assert!(!failure.reason.contains("private.example"));
            assert!(!failure.reason.contains("127.0.0.1"));
        }
    }

    #[test]
    fn queue_telemetry_classifies_typed_llm_auth_and_session_failures_without_raw_details() {
        let now = Utc::now();
        let cases = [
            (
                anyhow::Error::from(ironclaw_llm::error::LlmError::AuthFailed {
                    provider: "trace-secret-provider".to_string(),
                }),
                TraceQueueTelemetryFailureKind::Credential,
            ),
            (
                anyhow::Error::from(ironclaw_llm::error::LlmError::SessionExpired {
                    provider: "trace-secret-provider".to_string(),
                }),
                TraceQueueTelemetryFailureKind::Credential,
            ),
        ];

        for (error, expected) in cases {
            let failure =
                trace_queue_telemetry_failure_with_label(&error, now, "provider boundary failed");
            assert_eq!(failure.kind, expected);
            assert!(failure.reason.contains("error_hash="));
            assert!(!failure.reason.contains("trace-secret-provider"));
        }
    }

    #[tokio::test]
    async fn trace_queue_worker_tick_flushes_scopes_and_returns_credit_notices_for_delivery() {
        let scope = format!("trace-worker-tick-test-{}", Uuid::new_v4());
        let token_env = "TRACE_COMMONS_WORKER_TICK_TEST_TOKEN";
        let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
        let policy = StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
            .set_bearer_token_env(token_env.to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0)
            .set_credit_notice_interval_hours(168);
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
        queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

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
                credit_points_pending: 1.0,
                credit_points_final: Some(1.5),
                credit_explanation: vec!["Delayed utility credit posted.".to_string()],
                credit_events: Vec::new(),
                history: Vec::new(),
                last_credit_notice_at: None,
                credit_notice_state: TraceCreditNoticeState::default(),
            }],
        )
        .expect("local record writes");

        let report = flush_trace_contribution_queue_worker_tick(vec![scope.clone()], 10)
            .await
            .expect("worker tick succeeds");

        assert_eq!(report.scopes_checked, 1);
        assert_eq!(report.submitted, 0);
        assert_eq!(report.held, 1);
        assert_eq!(report.scope_reports[0].scope, scope);
        let notice = report.scope_reports[0]
            .credit_notice
            .as_ref()
            .expect("worker returns due credit notice for caller delivery");
        assert_eq!(notice.pending_credit, 1.0);
        assert_eq!(notice.final_credit, 1.5);

        let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
        assert!(
            records[0].last_credit_notice_at.is_some(),
            "worker tick marks due notices only when it returns them for delivery"
        );
    }

    #[tokio::test]
    async fn trace_queue_worker_tick_records_durable_failure_and_success_telemetry() {
        let scope = format!("trace-worker-telemetry-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default().set_enabled(true),
        )
        .expect("failure policy writes");

        let failed = flush_trace_contribution_queue_worker_tick(vec![scope.clone()], 10)
            .await
            .expect("worker tick handles scoped failure");
        assert_eq!(failed.scopes_checked, 1);
        assert!(failed.scope_reports.is_empty());

        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        assert!(diagnostics.telemetry.last_flush_attempt_at.is_some());
        assert!(diagnostics.telemetry.last_failed_flush_at.is_some());
        assert_eq!(diagnostics.telemetry.consecutive_flush_failures, 1);
        let failure = diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("failure metadata is stored");
        assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::Endpoint);
        assert!(failure.reason.contains("flush failed"));
        assert!(!failure.reason.contains(&scope));

        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string()),
        )
        .expect("success policy writes");

        let succeeded = flush_trace_contribution_queue_worker_tick(vec![scope.clone()], 10)
            .await
            .expect("worker tick handles scoped success");
        assert_eq!(succeeded.scopes_checked, 1);
        assert_eq!(succeeded.scope_reports.len(), 1);

        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        assert!(diagnostics.telemetry.last_successful_flush_at.is_some());
        assert_eq!(diagnostics.telemetry.consecutive_flush_failures, 0);
        assert!(diagnostics.telemetry.last_failure.is_none());

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
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
            RecordedTraceContributionOptions::default()
                .set_include_message_text(true)
                .set_include_tool_payloads(true),
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
        envelope.process_evaluation = Some(
            ProcessEvaluationLabels::default()
                .set_evaluator_version("process-evaluator-v1")
                .set_labels(vec![
                    ProcessEvaluatorLabel::CorrectToolSelection,
                    ProcessEvaluatorLabel::MissingVerification,
                ])
                .set_tool_selection(ProcessEvalRating::Pass)
                .set_tool_argument_quality(ProcessEvalRating::Unknown)
                .set_tool_ordering(ProcessEvalRating::Partial)
                .set_verification(ProcessEvalRating::Fail)
                .set_side_effect_safety(ProcessEvalRating::Pass)
                .set_overall_score(0.72),
        );
        envelope.hindsight = Some(
            HindsightRelabelingCandidate::default()
                .set_achieved_subgoals(vec!["redacted_subgoal:diagnosed_tool_failure".to_string()])
                .set_failure_type(TraceFailureMode::MissingVerification)
                .set_recoverability_score(0.8)
                .set_benchmark_candidate(true)
                .set_relabeled_training_candidate(true),
        );
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

    // ----- pilot allowlist invite_code integration ----------------------

    #[test]
    fn standing_policy_serde_back_compat_when_invite_code_missing() {
        // Existing policy files written before the invite_code field landed
        // must continue to parse unchanged.
        let legacy_json = r#"{
            "enabled": true,
            "ingestion_endpoint": "https://example/v1/traces",
            "bearer_token_env": "IRONCLAW_TRACE_SUBMIT_TOKEN",
            "upload_token_issuer_url": "https://issuer.example/v1/trace-upload-claim",
            "upload_token_issuer_allowed_hosts": ["issuer.example"],
            "upload_token_audience": "trace-commons",
            "upload_token_tenant_id": "tenant-a",
            "upload_token_workload_token_env": "IRONCLAW_TRACE_WORKLOAD_TOKEN",
            "upload_token_issuer_timeout_ms": 7000,
            "include_message_text": false,
            "include_tool_payloads": false,
            "auto_submit_failed_traces": true,
            "auto_submit_high_value_traces": true,
            "selected_tools": [],
            "require_manual_approval_when_pii_detected": true,
            "min_submission_score": 0.35,
            "credit_notice_interval_hours": 168,
            "default_scope": "debugging_evaluation"
        }"#;
        let policy: StandingTraceContributionPolicy =
            serde_json::from_str(legacy_json).expect("legacy policy parses");
        assert!(policy.upload_token_invite_code.is_none());
        assert!(policy.enabled);
    }

    #[test]
    fn standing_policy_serde_round_trips_invite_code_when_set() {
        let policy = StandingTraceContributionPolicy::default()
            .set_upload_token_invite_code("INV-PILOT-001");
        let serialized = serde_json::to_string(&policy).expect("serializes");
        assert!(
            serialized.contains("\"upload_token_invite_code\":\"INV-PILOT-001\""),
            "serialized policy carries invite code: {serialized}"
        );
        let round: StandingTraceContributionPolicy =
            serde_json::from_str(&serialized).expect("round trips");
        assert_eq!(
            round.upload_token_invite_code.as_deref(),
            Some("INV-PILOT-001")
        );
    }

    #[test]
    fn standing_policy_serde_omits_invite_code_when_none() {
        // skip_serializing_if keeps existing-shape policies byte-identical
        // for deployments that never configured an invite code.
        let policy = StandingTraceContributionPolicy::default();
        let serialized = serde_json::to_string(&policy).expect("serializes");
        assert!(
            !serialized.contains("upload_token_invite_code"),
            "default policy must not emit upload_token_invite_code: {serialized}"
        );
    }

    #[test]
    fn cache_key_distinguishes_different_invite_codes() {
        let make_policy = |invite: Option<&str>| {
            let policy = StandingTraceContributionPolicy::default()
                .set_upload_token_issuer_url("https://issuer.example/v1/trace-upload-claim");
            if let Some(invite) = invite {
                policy.set_upload_token_invite_code(invite)
            } else {
                policy
            }
        };
        let context = TraceUploadClaimContext::for_status_sync();
        let key_a = trace_upload_claim_cache_key(&make_policy(Some("INV-A")), &context).unwrap();
        let key_b = trace_upload_claim_cache_key(&make_policy(Some("INV-B")), &context).unwrap();
        let key_none = trace_upload_claim_cache_key(&make_policy(None), &context).unwrap();
        assert_ne!(
            key_a, key_b,
            "different invite codes => different cache keys"
        );
        assert_ne!(key_a, key_none, "with-invite vs no-invite must differ");
    }

    #[test]
    fn cache_key_isolates_scopes_in_device_key_mode() {
        // Security property: in DeviceKey mode a claim minted for scope A must
        // not be servable from cache for scope B. Same tenant/audience/issuer,
        // different scope_dir => different cache key.
        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_upload_token_issuer_url("https://issuer.example/v1/trace-upload-claim")
            .set_upload_token_tenant_id("tenant-shared")
            .set_upload_token_audience("trace-commons-ingest");
        let ctx_a = TraceUploadClaimContext::for_status_sync()
            .with_scope_dir(std::path::PathBuf::from("/scopes/user-a"));
        let ctx_b = TraceUploadClaimContext::for_status_sync()
            .with_scope_dir(std::path::PathBuf::from("/scopes/user-b"));

        let key_a = trace_upload_claim_cache_key(&policy, &ctx_a).unwrap();
        let key_b = trace_upload_claim_cache_key(&policy, &ctx_b).unwrap();
        assert_ne!(
            key_a, key_b,
            "DeviceKey mode: different scope_dir must yield different cache keys"
        );
    }

    #[test]
    fn cache_key_ignores_scope_dir_in_workload_token_env_mode() {
        // In WorkloadTokenEnv mode there is no scope concept; adding a scope_dir
        // to the context must not change the key (preserves pre-change behavior).
        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::WorkloadTokenEnv)
            .set_upload_token_issuer_url("https://issuer.example/v1/trace-upload-claim");
        let ctx_no_scope = TraceUploadClaimContext::for_status_sync();
        let ctx_with_scope = TraceUploadClaimContext::for_status_sync()
            .with_scope_dir(std::path::PathBuf::from("/scopes/user-a"));

        let key_no_scope = trace_upload_claim_cache_key(&policy, &ctx_no_scope).unwrap();
        let key_with_scope = trace_upload_claim_cache_key(&policy, &ctx_with_scope).unwrap();
        assert_eq!(
            key_no_scope, key_with_scope,
            "WorkloadTokenEnv mode: scope_dir must not affect the cache key"
        );
    }

    #[test]
    fn parse_trace_upload_claim_error_label_handles_known_shapes() {
        assert_eq!(
            parse_trace_upload_claim_error_label(r#"{"error":"PilotAllowlistNotMatched"}"#)
                .as_deref(),
            Some("PilotAllowlistNotMatched")
        );
        assert_eq!(
            parse_trace_upload_claim_error_label(
                r#"  {"error": "  PilotAllowlistStale  ", "extra": 1}"#
            )
            .as_deref(),
            Some("PilotAllowlistStale")
        );
        // Body with no `error` field => None (caller falls back to HTTP status).
        assert!(parse_trace_upload_claim_error_label(r#"{"message":"oops"}"#).is_none());
        // Empty / whitespace / non-JSON => None, never panics.
        assert!(parse_trace_upload_claim_error_label("").is_none());
        assert!(parse_trace_upload_claim_error_label("   ").is_none());
        assert!(parse_trace_upload_claim_error_label("not json").is_none());
        // `error` present but empty/whitespace-only => None (not a usable label).
        assert!(parse_trace_upload_claim_error_label(r#"{"error":"   "}"#).is_none());
    }

    #[test]
    fn parse_trace_upload_claim_error_label_returns_none_for_non_string_error() {
        // Non-string error fields must not panic and must return None so the
        // caller falls back to the generic HTTP-status diagnostic rather than
        // formatting a label like "42" or "[1,2,3]" into the user-facing
        // message.
        assert!(parse_trace_upload_claim_error_label(r#"{"error":42}"#).is_none());
        assert!(parse_trace_upload_claim_error_label(r#"{"error":{"detail":"x"}}"#).is_none());
        assert!(parse_trace_upload_claim_error_label(r#"{"error":[1,2,3]}"#).is_none());
        assert!(parse_trace_upload_claim_error_label(r#"{"error":true}"#).is_none());
        assert!(parse_trace_upload_claim_error_label(r#"{"error":null}"#).is_none());
    }

    #[tokio::test]
    async fn fetch_trace_upload_claim_from_issuer_returns_typed_pilot_allowlist_error() {
        // Spin up a mock HTTP server that returns the issuer's typed
        // PilotAllowlistNotMatched refusal, then drive the factored-out
        // error-formatting helper directly with that body to assert the
        // user-actionable diagnostic (the helper is the unit under test;
        // the mock confirms the body shape the real issuer emits).
        let app = axum::Router::new().route(
            "/v1/trace-upload-claim",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({"error": "PilotAllowlistNotMatched"})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock issuer listener binds");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let client = reqwest::Client::builder()
            .build()
            .expect("reqwest client builds");
        let response = client
            .post(format!("http://{addr}/v1/trace-upload-claim"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .expect("mock issuer responds");
        let status = response.status().as_u16();
        let body_text = response.text().await.expect("mock issuer body");
        assert_eq!(status, 400);

        let error = build_trace_upload_claim_http_error("issuer.example", status, &body_text);
        let chain = format!("{error:#}");
        assert!(
            chain.contains("PilotAllowlistNotMatched"),
            "diagnostic chain must surface the typed label: {chain}"
        );
        assert!(
            chain.contains("invite code hash was not in the issuer's active allowlist"),
            "diagnostic chain must surface the user-actionable diagnostic text: {chain}"
        );
    }

    #[tokio::test]
    async fn fetch_trace_upload_claim_from_issuer_generic_http_error_when_label_unknown() {
        // Issuer returns a non-JSON 500 — the helper must fall back to the
        // generic "HTTP 500" diagnostic without naming any PilotAllowlist
        // refusal label.
        let app = axum::Router::new().route(
            "/v1/trace-upload-claim",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error",
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock issuer listener binds");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let client = reqwest::Client::builder()
            .build()
            .expect("reqwest client builds");
        let response = client
            .post(format!("http://{addr}/v1/trace-upload-claim"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .expect("mock issuer responds");
        let status = response.status().as_u16();
        let body_text = response.text().await.expect("mock issuer body");
        assert_eq!(status, 500);

        let error = build_trace_upload_claim_http_error("issuer.example", status, &body_text);
        let chain = format!("{error:#}");
        assert!(
            chain.contains("HTTP 500"),
            "generic fallback must surface the HTTP status: {chain}"
        );
        assert!(
            !chain.contains("PilotAllowlist"),
            "generic fallback must not name any PilotAllowlist label: {chain}"
        );
    }

    #[test]
    fn cache_key_hashes_invite_code_with_sha256_prefix() {
        let policy = StandingTraceContributionPolicy::default()
            .set_upload_token_issuer_url("https://issuer.example/v1/trace-upload-claim")
            .set_upload_token_invite_code("INV-PILOT-001");
        let context = TraceUploadClaimContext::for_status_sync();
        let key = trace_upload_claim_cache_key(&policy, &context).expect("cache key");
        assert!(
            !key.contains("INV-PILOT-001"),
            "raw invite code must not appear in cache key: {key}"
        );
        let expected_hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest("INV-PILOT-001".as_bytes()))
        );
        assert!(
            key.contains(&expected_hash),
            "cache key must include sha256-hashed invite code: {key}"
        );
    }

    #[test]
    fn legacy_policy_json_defaults_to_workload_token_env_auth() {
        // Take the default policy's JSON and strip the two NEW fields to simulate
        // a pre-upgrade policy file on disk.
        let mut legacy = serde_json::to_value(StandingTraceContributionPolicy::default()).unwrap();
        let obj = legacy.as_object_mut().unwrap();
        obj.remove("auth_mode");
        obj.remove("device_key_id");
        let policy: StandingTraceContributionPolicy = serde_json::from_value(legacy).unwrap();
        assert_eq!(policy.auth_mode, TraceUploadAuthMode::WorkloadTokenEnv);
        assert!(policy.device_key_id.is_none());
    }

    #[test]
    fn device_key_policy_round_trips() {
        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_device_key_id("sha256:abc".to_string());
        let json = serde_json::to_value(&policy).unwrap();
        assert_eq!(json["auth_mode"], "device_key");
        let back: StandingTraceContributionPolicy = serde_json::from_value(json).unwrap();
        assert_eq!(back.auth_mode, TraceUploadAuthMode::DeviceKey);
        assert_eq!(back.device_key_id.as_deref(), Some("sha256:abc"));
    }

    // --- DeviceKey auth mode tests for issuer_request_bearer ---

    #[tokio::test]
    async fn device_key_auth_mode_self_signs_workload_jwt() {
        let dir = tempfile::tempdir().unwrap();
        let pending =
            crate::onboarding::DeviceKeypair::load_or_generate_pending(dir.path(), "h").unwrap();
        let promoted = pending.promote(dir.path(), "tenant-a").unwrap();

        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_upload_token_tenant_id("tenant-a".to_string())
            .set_upload_token_audience("trace-commons-ingest".to_string());
        let context =
            TraceUploadClaimContext::for_status_sync().with_scope_dir(dir.path().to_path_buf());

        let result = issuer_request_bearer(&policy, &context).await.unwrap();
        let bearer = result.expect("DeviceKey mode must return a bearer token");

        // The JWT must be EdDSA and carry the device key id as kid.
        let header = jsonwebtoken::decode_header(&bearer).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::EdDSA);
        assert_eq!(header.kid.as_deref(), Some(promoted.device_key_id.as_str()));
    }

    #[tokio::test]
    async fn device_key_auth_mode_without_local_key_errors_clearly() {
        // Empty dir — no key has ever been generated or promoted.
        let dir = tempfile::tempdir().unwrap();

        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_upload_token_tenant_id("tenant-a".to_string())
            .set_upload_token_audience("trace-commons-ingest".to_string());
        let context =
            TraceUploadClaimContext::for_status_sync().with_scope_dir(dir.path().to_path_buf());

        let err = issuer_request_bearer(&policy, &context).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("re-run onboarding"),
            "error should mention re-run onboarding, got: {msg}"
        );
    }

    #[tokio::test]
    async fn device_key_auth_mode_without_scope_dir_errors_clearly() {
        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_upload_token_tenant_id("tenant-a".to_string())
            .set_upload_token_audience("trace-commons-ingest".to_string());
        // No scope_dir — context constructed without with_scope_dir().
        let context = TraceUploadClaimContext::for_status_sync();

        let err = issuer_request_bearer(&policy, &context).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("scope"),
            "error should mention scope directory, got: {msg}"
        );
    }

    /// Regression for the revoke path: a DeviceKey-mode revoke context built
    /// from `for_submission_id` plus a real `scope_dir` must reach the signing
    /// path and resolve a bearer, rather than hard-erroring on missing scope.
    /// This is a focused test on the bearer/context construction the revoke
    /// path now performs (wiring the full revoke HTTP path is heavier; the
    /// scope_dir threading is what regressed).
    #[tokio::test]
    async fn device_key_auth_mode_revoke_context_self_signs_workload_jwt() {
        let dir = tempfile::tempdir().unwrap();
        let pending =
            crate::onboarding::DeviceKeypair::load_or_generate_pending(dir.path(), "h").unwrap();
        let promoted = pending.promote(dir.path(), "tenant-a").unwrap();

        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_upload_token_tenant_id("tenant-a".to_string())
            .set_upload_token_audience("trace-commons-ingest".to_string());
        // Mirror the revoke path: context from for_submission_id + scope_dir.
        let context = TraceUploadClaimContext::for_submission_id(Uuid::new_v4())
            .with_scope_dir(dir.path().to_path_buf());

        let bearer = issuer_request_bearer(&policy, &context)
            .await
            .unwrap()
            .expect("DeviceKey revoke context must resolve a bearer token");

        let header = jsonwebtoken::decode_header(&bearer).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::EdDSA);
        assert_eq!(header.kid.as_deref(), Some(promoted.device_key_id.as_str()));
    }

    #[tokio::test]
    async fn workload_token_env_mode_reads_env_unchanged() {
        // Use a uniquely named env var so other tests cannot interfere.
        let env_var = "IRONCLAW_TEST_WORKLOAD_TOKEN_UNIQUE_9f3a2b1c";
        // SAFETY: test-only; uniquely named var not read by any other test.
        unsafe {
            std::env::set_var(env_var, "test-bearer-xyz");
        }

        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::WorkloadTokenEnv)
            .set_upload_token_workload_token_env(env_var.to_string());
        let context = TraceUploadClaimContext::for_status_sync();

        let result = issuer_request_bearer(&policy, &context).await.unwrap();
        assert_eq!(result.as_deref(), Some("test-bearer-xyz"));

        // SAFETY: same as set above — cleanup.
        unsafe {
            std::env::remove_var(env_var);
        }
    }

    /// Focused unit test on request construction: verify that DeviceKey mode
    /// sets invite_code = None while WorkloadTokenEnv mode uses the policy value.
    #[test]
    fn invite_code_gated_by_auth_mode() {
        // DeviceKey mode — invite_code must be None regardless of policy field.
        let policy_device_key = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::DeviceKey)
            .set_upload_token_invite_code("should-not-appear".to_string());
        let invite_code_device_key = match policy_device_key.auth_mode {
            TraceUploadAuthMode::WorkloadTokenEnv => policy_device_key
                .upload_token_invite_code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            TraceUploadAuthMode::DeviceKey => None,
        };
        assert!(
            invite_code_device_key.is_none(),
            "DeviceKey mode must not send invite_code"
        );

        // WorkloadTokenEnv mode — invite_code from policy should be forwarded.
        let policy_env = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::WorkloadTokenEnv)
            .set_upload_token_invite_code("invite-abc".to_string());
        let invite_code_env = match policy_env.auth_mode {
            TraceUploadAuthMode::WorkloadTokenEnv => policy_env
                .upload_token_invite_code
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            TraceUploadAuthMode::DeviceKey => None,
        };
        assert_eq!(
            invite_code_env.as_deref(),
            Some("invite-abc"),
            "WorkloadTokenEnv mode must forward invite_code from policy"
        );
    }

    // ── community profile (public_attribution second opt-in) ────────────────

    #[test]
    fn community_profile_handle_validation_rejects_bad_handles() {
        let error = validate_community_profile_handle("ab").expect_err("too short rejected");
        assert!(error.to_string().contains("at least 3"));

        let long = "a".repeat(33);
        let error = validate_community_profile_handle(&long).expect_err("too long rejected");
        assert!(error.to_string().contains("at most 32"));

        for bad in ["bad handle!", "naïve", "pilot.zaki", "pilot/zaki"] {
            let error =
                validate_community_profile_handle(bad).expect_err("bad characters rejected");
            assert!(
                error.to_string().contains("ASCII letters"),
                "{bad} should fail the character-set check: {error}"
            );
        }

        assert_eq!(
            validate_community_profile_handle("  pilot_zaki  ").expect("trimmed handle accepted"),
            "pilot_zaki"
        );
        assert_eq!(
            validate_community_profile_handle("Pilot-Zaki_42").expect("alnum/-/_ accepted"),
            "Pilot-Zaki_42"
        );
    }

    #[test]
    fn community_profile_bio_validation_bounds_bytes() {
        validate_community_profile_bio(&"x".repeat(280)).expect("280 bytes accepted");
        let error =
            validate_community_profile_bio(&"x".repeat(281)).expect_err("281 bytes rejected");
        assert!(error.to_string().contains("at most 280 bytes"));
        // Byte-length, not char-length: 141 two-byte chars = 282 bytes.
        assert!(validate_community_profile_bio(&"é".repeat(141)).is_err());
    }

    #[test]
    fn community_profile_url_derives_from_ingest_url() {
        let policy = StandingTraceContributionPolicy::default()
            .set_ingestion_endpoint("https://ingest.example.com:8443/v1/traces".to_string())
            .set_upload_token_issuer_url(
                "https://issuer.example.com/v1/trace-upload-claim".to_string(),
            )
            .set_upload_token_issuer_allowed_hosts(BTreeSet::from([
                "issuer.example.com".to_string()
            ]));
        let url = community_profile_url_from_policy(&policy).expect("profile URL derives");
        assert_eq!(
            url.as_str(),
            "https://ingest.example.com:8443/v1/community/profile",
            "scheme/host/port preserved, path replaced"
        );

        // Profile routing must not depend on issuer host compatibility.
        let split_hosts = StandingTraceContributionPolicy::default()
            .set_ingestion_endpoint("https://ingest.tracecommons.ai/v1/traces".to_string())
            .set_upload_token_issuer_url(
                "https://issuer.tracecommons.ai/v1/trace-upload-claim".to_string(),
            )
            .set_upload_token_issuer_allowed_hosts(BTreeSet::from([
                "issuer.tracecommons.ai".to_string()
            ]));
        let split_url =
            community_profile_url_from_policy(&split_hosts).expect("split hosts derive");
        assert_eq!(
            split_url.as_str(),
            "https://ingest.tracecommons.ai/v1/community/profile"
        );

        // Plain HTTP ingest endpoints are rejected.
        let insecure = StandingTraceContributionPolicy::default()
            .set_ingestion_endpoint("http://ingest.example.com/v1/traces".to_string());
        assert!(community_profile_url_from_policy(&insecure).is_err());

        // Internal (non-loopback) ingest hosts are rejected.
        let internal = StandingTraceContributionPolicy::default()
            .set_ingestion_endpoint("https://ingest.corp.internal/v1/traces".to_string());
        assert!(community_profile_url_from_policy(&internal).is_err());

        // Literal loopback gets the dev exception (loopback-HTTP onboarding
        // stores a loopback ingest endpoint).
        let loopback = StandingTraceContributionPolicy::default()
            .set_ingestion_endpoint("http://127.0.0.1:3917/v1/traces".to_string());
        let loopback_url =
            community_profile_url_from_policy(&loopback).expect("loopback dev ingest derives");
        assert_eq!(
            loopback_url.as_str(),
            "http://127.0.0.1:3917/v1/community/profile"
        );

        // A mounted prefix on the ingest path must be preserved (mirrors
        // trace_submission_status_endpoint), not clobbered to the bare path.
        let prefixed = StandingTraceContributionPolicy::default()
            .set_ingestion_endpoint("https://ingest.example.com/api/v1/traces".to_string());
        assert_eq!(
            community_profile_url_from_policy(&prefixed)
                .expect("prefixed ingest derives")
                .as_str(),
            "https://ingest.example.com/api/v1/community/profile"
        );
    }

    #[tokio::test]
    async fn mint_profile_attribution_token_requires_enrollment() {
        let scope = format!("trace-profile-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(Some(&scope), &StandingTraceContributionPolicy::default())
            .expect("policy writes");
        let error = mint_profile_attribution_token_for_scope(Some(&scope))
            .await
            .expect_err("disabled policy must refuse to mint");
        assert!(
            error.to_string().contains("not enrolled in Trace Commons"),
            "error must point at onboarding: {error}"
        );
        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn mint_profile_attribution_token_requires_issuer_url() {
        let scope = format!("trace-profile-test-{}", Uuid::new_v4());
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default().set_enabled(true),
        )
        .expect("policy writes");
        let error = mint_profile_attribution_token_for_scope(Some(&scope))
            .await
            .expect_err("missing issuer URL must refuse to mint");
        assert!(
            error.to_string().contains("issuer URL is not configured"),
            "error must name the missing issuer URL: {error}"
        );
        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }

    #[tokio::test]
    async fn profile_attribution_claim_request_wire_shape_and_mock_issuer_roundtrip() {
        // Like the PilotAllowlist tests above, we assert the wire shape via
        // the factored-out request builder and confirm a real issuer
        // round-trip of exactly that body against a mock that rejects any
        // drift (the full fetch path is covered separately by
        // fetch_trace_upload_claim_from_issuer_accepts_loopback_dev_issuer).
        let scope = format!("trace-profile-test-{}", Uuid::new_v4());
        let context = profile_attribution_claim_context(Some(&scope));
        let policy = StandingTraceContributionPolicy::default()
            .set_auth_mode(TraceUploadAuthMode::WorkloadTokenEnv)
            .set_upload_token_tenant_id("tenant-a".to_string())
            .set_upload_token_audience("trace-commons".to_string());
        let request = build_trace_upload_claim_issuer_request(&policy, &context);
        let body = serde_json::to_value(&request).expect("request serializes");
        assert_eq!(
            body["consent_scopes"],
            serde_json::json!(["public_attribution"])
        );
        let obj = body.as_object().expect("request body is an object");
        assert!(
            !obj.contains_key("allowed_uses"),
            "empty allowed_uses must be skip-serialized"
        );
        assert!(
            !obj.contains_key("trace_id"),
            "profile claims carry no trace_id"
        );
        assert!(
            !obj.contains_key("submission_id"),
            "profile claims carry no submission_id"
        );

        let mint_token = test_jwt_with_header(serde_json::json!({
            "alg": "EdDSA",
            "kid": "managed-key-1"
        }));
        let mint_token_for_route = mint_token.clone();
        let app = axum::Router::new().route(
            "/v1/trace-upload-claim",
            axum::routing::post(
                move |axum::Json(request_body): axum::Json<serde_json::Value>| {
                    let mint_token = mint_token_for_route.clone();
                    async move {
                        let obj = request_body.as_object().cloned().unwrap_or_default();
                        if request_body["consent_scopes"]
                            != serde_json::json!(["public_attribution"])
                            || obj.contains_key("allowed_uses")
                            || obj.contains_key("trace_id")
                            || obj.contains_key("submission_id")
                        {
                            return (
                                axum::http::StatusCode::BAD_REQUEST,
                                axum::Json(serde_json::json!({"error": "unexpected claim body"})),
                            );
                        }
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "access_token": mint_token,
                                "token_type": "Bearer",
                                "expires_in": 300
                            })),
                        )
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock issuer listener binds");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::builder()
            .build()
            .expect("reqwest client builds");
        let response = client
            .post(format!("http://{addr}/v1/trace-upload-claim"))
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&request)
            .send()
            .await
            .expect("mock issuer responds");
        assert_eq!(
            response.status().as_u16(),
            200,
            "mock issuer must accept the profile claim body"
        );
        let claim: TraceUploadClaimIssuerResponse =
            response.json().await.expect("claim response parses");
        validate_trace_upload_claim_response(&claim).expect("mock claim passes validation");
        assert_eq!(claim.access_token, mint_token);
        assert_eq!(claim.expires_in, Some(300));
    }

    #[tokio::test]
    async fn community_profile_put_sends_bearer_and_body() {
        let token = test_jwt_with_header(serde_json::json!({
            "alg": "EdDSA",
            "kid": "managed-key-1"
        }));
        let seen: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_for_route = seen.clone();
        let app = axum::Router::new().route(
            "/v1/community/profile",
            axum::routing::put(
                move |headers: axum::http::HeaderMap,
                      axum::Json(body): axum::Json<serde_json::Value>| {
                    let seen = seen_for_route.clone();
                    async move {
                        let authorization = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("<missing>")
                            .to_string();
                        seen.lock().expect("seen lock").push((authorization, body));
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({"display_handle": "pilot_zaki"})),
                        )
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock profile listener binds");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let url = reqwest::Url::parse(&format!("http://{addr}/v1/community/profile"))
            .expect("profile url parses");
        // None sink exercises the worker/CLI crate-local reqwest path
        // (community_profile_http_client builds against the loopback mock).
        let policy = StandingTraceContributionPolicy::default();
        execute_community_profile_request(
            &policy,
            ContributionHttpMethod::Put,
            url,
            &token,
            Some(&serde_json::json!({"display_handle": "pilot_zaki", "bio": null})),
            None,
        )
        .await
        .expect("profile PUT succeeds");

        let seen = seen.lock().expect("seen lock");
        assert_eq!(seen.len(), 1);
        let (authorization, body) = &seen[0];
        assert_eq!(authorization, &format!("Bearer {token}"));
        assert_eq!(
            body,
            &serde_json::json!({"display_handle": "pilot_zaki", "bio": null})
        );
    }

    #[tokio::test]
    async fn community_profile_delete_sends_bearer_without_body() {
        let token = test_jwt_with_header(serde_json::json!({
            "alg": "EdDSA",
            "kid": "managed-key-1"
        }));
        let seen: Arc<std::sync::Mutex<Vec<(String, usize)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_for_route = seen.clone();
        let app = axum::Router::new().route(
            "/v1/community/profile",
            axum::routing::delete(
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let seen = seen_for_route.clone();
                    async move {
                        let authorization = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("<missing>")
                            .to_string();
                        seen.lock()
                            .expect("seen lock")
                            .push((authorization, body.len()));
                        axum::http::StatusCode::NO_CONTENT
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock profile listener binds");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let url = reqwest::Url::parse(&format!("http://{addr}/v1/community/profile"))
            .expect("profile url parses");
        let policy = StandingTraceContributionPolicy::default();
        execute_community_profile_request(
            &policy,
            ContributionHttpMethod::Delete,
            url,
            &token,
            None,
            None,
        )
        .await
        .expect("profile DELETE succeeds");

        let seen = seen.lock().expect("seen lock");
        assert_eq!(seen.len(), 1);
        let (authorization, body_len) = &seen[0];
        assert_eq!(authorization, &format!("Bearer {token}"));
        assert_eq!(*body_len, 0, "withdraw must send no body");
    }

    #[tokio::test]
    async fn community_profile_error_surfaces_bounded_error_field_without_token() {
        let token = test_jwt_with_header(serde_json::json!({
            "alg": "EdDSA",
            "kid": "managed-key-1"
        }));
        let app = axum::Router::new().route(
            "/v1/community/profile",
            axum::routing::put(|| async {
                (
                    axum::http::StatusCode::CONFLICT,
                    axum::Json(serde_json::json!({"error": "display handle already taken"})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock profile listener binds");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let url = reqwest::Url::parse(&format!("http://{addr}/v1/community/profile"))
            .expect("profile url parses");
        let policy = StandingTraceContributionPolicy::default();
        let error = execute_community_profile_request(
            &policy,
            ContributionHttpMethod::Put,
            url,
            &token,
            Some(&serde_json::json!({"display_handle": "pilot_zaki", "bio": null})),
            None,
        )
        .await
        .expect_err("conflict must surface as an error");

        let chain = format!("{error:#}");
        assert!(chain.contains("HTTP 409"), "status surfaces: {chain}");
        assert!(
            chain.contains("display handle already taken"),
            "bounded error field surfaces: {chain}"
        );
        assert!(
            !chain.contains(&token),
            "bearer token must never appear in errors: {chain}"
        );
    }

    #[tokio::test]
    async fn set_community_profile_rejects_invalid_handle_before_any_network_call() {
        // Test through the caller: the public entrypoint must refuse a bad
        // handle before reading policy or touching the network.
        let scope = format!("trace-profile-test-{}", Uuid::new_v4());
        let error = set_community_profile_for_scope(Some(&scope), "x", None)
            .await
            .expect_err("short handle must be rejected");
        assert!(error.to_string().contains("at least 3"));

        let error =
            set_community_profile_for_scope(Some(&scope), "ok_handle", Some(&"x".repeat(281)))
                .await
                .expect_err("oversized bio must be rejected");
        assert!(error.to_string().contains("at most 280 bytes"));
    }

    // --- resolve_trace_credentials tests ---
    // Isolation: each test uses its own tempdir passed to the private
    // `resolve_trace_credentials_at` core, so tests are fully isolated from
    // the global IRONCLAW_BASE_DIR and from each other (no shared state,
    // no cleanup needed).  The public `resolve_trace_credentials` is a thin
    // wrapper that supplies the real base dir — the core logic is tested here.

    fn write_policy_at(
        base: &std::path::Path,
        scope: Option<&str>,
        policy: &StandingTraceContributionPolicy,
    ) {
        write_trace_policy_for_scope_at(base, scope, policy).expect("write_policy_at");
    }

    #[test]
    fn resolver_prefers_personal_invite_enrollment_with_no_subject() {
        let dir = tempfile::tempdir().unwrap();
        let scope = trace_scope_key("tenant-a", "alice");
        let personal = StandingTraceContributionPolicy {
            enabled: true,
            ..Default::default()
        };
        write_policy_at(dir.path(), Some(scope.as_str()), &personal);

        let r = resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
            .unwrap()
            .unwrap();
        assert_eq!(r.state_scope, scope);
        assert_eq!(r.subject, None, "personal invite carries no subject");
        assert!(r.policy.enabled);
    }

    #[test]
    fn resolver_falls_back_to_instance_enrollment_with_per_user_subject() {
        let dir = tempfile::tempdir().unwrap();
        // No personal policy; only the instance-level (scope None) policy.
        let instance = StandingTraceContributionPolicy {
            enabled: true,
            ..Default::default()
        };
        write_policy_at(dir.path(), None, &instance);

        let r = resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
            .unwrap()
            .unwrap();
        let expected_scope = trace_scope_key("tenant-a", "alice");
        let subject = r
            .subject
            .clone()
            .expect("instance fallback carries a subject");
        assert!(r.policy.enabled);

        // The subject must be SALTED with per-instance random state: an
        // unsalted hash of the raw scope lets the server dictionary-match
        // guessable tenant/user ids and de-pseudonymize contributors.
        assert_ne!(
            subject,
            local_pseudonymous_contributor_id(&expected_scope),
            "instance subject must not be the unsalted scope hash"
        );

        // Stable within an instance: same (base, scope) → same subject.
        let again = resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
            .unwrap()
            .unwrap();
        assert_eq!(again.subject.as_deref(), Some(subject.as_str()));

        // Distinct across instances: a different base dir has a different salt.
        let other = tempfile::tempdir().unwrap();
        write_policy_at(other.path(), None, &instance);
        let other_subject = resolve_trace_credentials_at(other.path(), "tenant-a", "alice")
            .unwrap()
            .unwrap()
            .subject
            .expect("other instance resolves a subject");
        assert_ne!(
            other_subject, subject,
            "different instances must derive different subjects for the same scope"
        );
    }

    #[test]
    fn capture_policy_resolves_personal_then_instance_then_none() {
        let dir = tempfile::tempdir().unwrap();
        let scope = trace_scope_key("tenant-a", "alice");

        // Neither enrolled → capture must skip.
        assert!(
            resolve_effective_capture_policy_at(dir.path(), Some(scope.as_str()))
                .unwrap()
                .is_none(),
            "unenrolled scope must yield no capture policy"
        );

        // Instance-only enrollment (scope None), no per-user policy: capture must
        // resolve the instance policy — the P1 the per-user-only gate dropped.
        let instance = StandingTraceContributionPolicy {
            enabled: true,
            upload_token_tenant_id: Some("instance-tenant".to_string()),
            ..Default::default()
        };
        write_policy_at(dir.path(), None, &instance);
        let resolved = resolve_effective_capture_policy_at(dir.path(), Some(scope.as_str()))
            .unwrap()
            .expect("instance-only scope must capture under the instance policy");
        assert!(resolved.enabled);
        assert_eq!(
            resolved.upload_token_tenant_id.as_deref(),
            Some("instance-tenant"),
            "instance-only capture must use the instance policy"
        );

        // A user's own enabled personal-invite policy takes precedence.
        let personal = StandingTraceContributionPolicy {
            enabled: true,
            upload_token_tenant_id: Some("personal-tenant".to_string()),
            ..Default::default()
        };
        write_policy_at(dir.path(), Some(scope.as_str()), &personal);
        let resolved = resolve_effective_capture_policy_at(dir.path(), Some(scope.as_str()))
            .unwrap()
            .expect("personal enrollment resolves");
        assert_eq!(
            resolved.upload_token_tenant_id.as_deref(),
            Some("personal-tenant"),
            "personal-invite policy must take precedence over the instance policy"
        );
    }

    #[test]
    fn resolver_returns_none_when_unenrolled() {
        let dir = tempfile::tempdir().unwrap();
        // Empty dir — no policy files at all.
        assert!(
            resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn opt_out_user_scope_blocks_only_that_user_never_the_instance() {
        // Regression (PR #5858 review): the CLI's opt-out used to flip the
        // ROOT policy too — which, under instance enrollment, disenrolled the
        // ENTIRE instance when one user opted out. The per-user opt-out
        // primitive must write only the user's scoped policy.
        let dir = tempfile::tempdir().unwrap();
        write_policy_at(
            dir.path(),
            None,
            &StandingTraceContributionPolicy {
                enabled: true,
                ..Default::default()
            },
        );

        let alice = trace_scope_key("tenant-a", "alice");
        opt_out_user_scope_at(dir.path(), &alice).expect("opt-out writes");

        // The instance policy is untouched on disk.
        let instance = read_trace_policy_for_scope_at(dir.path(), None).expect("instance reads");
        assert!(
            instance.enabled,
            "per-user opt-out must never disable the instance enrollment"
        );
        // Alice is out on every resolution surface…
        assert!(
            resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
                .unwrap()
                .is_none(),
            "opted-out user must not resolve instance credentials"
        );
        // …while other users still inherit the instance enrollment.
        assert!(
            resolve_trace_credentials_at(dir.path(), "tenant-a", "bob")
                .unwrap()
                .is_some(),
            "other users must keep inheriting the instance enrollment"
        );
    }

    #[test]
    fn resolver_explicit_user_opt_out_blocks_instance_fallback() {
        // `traces opt-out` writes the user's scoped policy with enabled=false.
        // That explicit opt-out must win over an enabled instance policy on
        // EVERY resolution surface (credentials, flush, capture) — a disabled
        // scoped policy file is not the same as "never configured".
        let dir = tempfile::tempdir().unwrap();
        let scope = trace_scope_key("tenant-a", "alice");
        write_policy_at(
            dir.path(),
            None,
            &StandingTraceContributionPolicy {
                enabled: true,
                ..Default::default()
            },
        );
        write_policy_at(
            dir.path(),
            Some(scope.as_str()),
            &StandingTraceContributionPolicy {
                enabled: false,
                ..Default::default()
            },
        );

        assert!(
            resolve_trace_credentials_at(dir.path(), "tenant-a", "alice")
                .unwrap()
                .is_none(),
            "explicit per-user opt-out must not resolve to instance credentials"
        );
        assert!(
            resolve_effective_flush_target_at(dir.path(), Some(scope.as_str()))
                .unwrap()
                .is_none(),
            "explicit per-user opt-out must not flush under instance enrollment"
        );
        assert!(
            resolve_effective_capture_policy_at(dir.path(), Some(scope.as_str()))
                .unwrap()
                .is_none(),
            "explicit per-user opt-out must not capture under the instance policy"
        );
    }

    // --- resolve_effective_flush_target tests ---
    // Same isolation contract as the resolver tests: each uses its own tempdir
    // passed to the private `_at` core, so they never touch the global
    // IRONCLAW_BASE_DIR. These prove the autonomous flush gate is resolver-aware:
    // an instance-only enrollment resolves to a contributing target (so the gate
    // no longer aborts) carrying the per-user pseudonymous subject and the
    // INSTANCE device-key dir.

    #[test]
    fn effective_flush_target_personal_enabled_uses_scope_dir_and_no_subject() {
        let dir = tempfile::tempdir().unwrap();
        let scope = trace_scope_key("tenant-a", "alice");
        let personal = StandingTraceContributionPolicy {
            enabled: true,
            ..Default::default()
        };
        write_policy_at(dir.path(), Some(scope.as_str()), &personal);

        let target = resolve_effective_flush_target_at(dir.path(), Some(scope.as_str()))
            .unwrap()
            .expect("personal-enabled scope is a contributing target");
        assert!(target.policy.enabled);
        assert_eq!(target.subject, None, "personal invite carries no subject");
        assert_eq!(
            target.device_key_dir,
            trace_contribution_dir_for_scope_at(dir.path(), Some(scope.as_str())),
            "personal enrollment loads its device key from the per-scope dir"
        );
    }

    #[test]
    fn effective_flush_target_instance_only_uses_instance_dir_and_subject() {
        let dir = tempfile::tempdir().unwrap();
        let scope = trace_scope_key("tenant-a", "alice");
        // No personal policy for the scope; only the instance-level (None) policy.
        let instance = StandingTraceContributionPolicy {
            enabled: true,
            ..Default::default()
        };
        write_policy_at(dir.path(), None, &instance);

        let target = resolve_effective_flush_target_at(dir.path(), Some(scope.as_str()))
            .unwrap()
            .expect("instance-enrolled scope is a contributing target (gate must not abort)");
        assert!(target.policy.enabled);
        assert_eq!(
            target.subject,
            Some(salted_pseudonymous_contributor_id_at(dir.path(), &scope).unwrap()),
            "instance enrollment attributes the user via a salted per-user pseudonymous subject"
        );
        assert_eq!(
            target.device_key_dir,
            trace_contribution_dir_for_scope_at(dir.path(), None),
            "instance enrollment loads the shared device key from the instance (None) dir"
        );
    }

    #[test]
    fn effective_flush_target_none_when_unenrolled() {
        let dir = tempfile::tempdir().unwrap();
        let scope = trace_scope_key("tenant-a", "alice");
        // Empty dir — neither a personal nor an instance policy is enabled.
        assert!(
            resolve_effective_flush_target_at(dir.path(), Some(scope.as_str()))
                .unwrap()
                .is_none(),
            "unenrolled scope has no contributing target"
        );
    }

    #[test]
    fn upload_claim_request_includes_subject_in_device_key_mode() {
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            upload_token_tenant_id: Some("tenant-a".to_string()),
            ..Default::default()
        };
        let ctx = TraceUploadClaimContext {
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject: Some("sha256:deadbeef".to_string()),
        };
        let req = build_trace_upload_claim_issuer_request(&policy, &ctx);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["subject"], "sha256:deadbeef");
    }

    #[test]
    fn upload_claim_cache_key_separates_subjects_sharing_a_scope_dir() {
        // Instance enrollment: all users share the SAME instance device-key dir
        // (scope None), distinguished only by their per-user subject. The cache
        // key MUST differ per subject, or a claim minted for one user would be
        // served from cache to another (cross-user trace mis-attribution).
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            upload_token_issuer_url: Some(
                "https://issuer.example/v1/trace-upload-claim".to_string(),
            ),
            upload_token_tenant_id: Some("tenant-a".to_string()),
            upload_token_audience: Some("trace-commons".to_string()),
            ..Default::default()
        };
        let shared_dir = std::path::PathBuf::from("/instance/trace_contributions");
        let ctx_for = |subject: &str| TraceUploadClaimContext {
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: Vec::new(),
            scope_dir: Some(shared_dir.clone()),
            subject: Some(subject.to_string()),
        };

        let alice = trace_upload_claim_cache_key(&policy, &ctx_for("sha256:alice")).unwrap();
        let bob = trace_upload_claim_cache_key(&policy, &ctx_for("sha256:bob")).unwrap();
        let alice_again = trace_upload_claim_cache_key(&policy, &ctx_for("sha256:alice")).unwrap();

        assert_ne!(
            alice, bob,
            "distinct subjects sharing a scope_dir must get distinct cache keys"
        );
        assert_eq!(
            alice, alice_again,
            "same subject must produce a stable cache key"
        );

        // A no-subject context (personal-invite path) must also differ from the
        // subject-bearing keys so the two models never collide on cache.
        let no_subject = TraceUploadClaimContext {
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: Vec::new(),
            scope_dir: Some(shared_dir.clone()),
            subject: None,
        };
        let none_key = trace_upload_claim_cache_key(&policy, &no_subject).unwrap();
        assert_ne!(alice, none_key);
        assert_ne!(bob, none_key);

        // The key hashes the exact optional bytes with a None/Some discriminator,
        // so `Some("")` and whitespace variants never collide with `None` or with
        // each other (which would let one payload's claim serve another's).
        let empty_key = trace_upload_claim_cache_key(&policy, &ctx_for("")).unwrap();
        assert_ne!(
            empty_key, none_key,
            "Some(\"\") must not share a key with None"
        );
        let padded = trace_upload_claim_cache_key(&policy, &ctx_for("  sha256:alice  ")).unwrap();
        assert_ne!(
            padded, alice,
            "whitespace-padded subject must not collide with its trimmed form"
        );
    }

    #[test]
    fn context_with_subject_sets_field() {
        let ctx = TraceUploadClaimContext {
            trace_id: None,
            submission_id: None,
            consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject: None,
        }
        .with_subject(Some("sha256:abc".to_string()));
        assert_eq!(ctx.subject.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn upload_claim_request_omits_subject_when_none() {
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            ..Default::default()
        };
        let ctx = TraceUploadClaimContext {
            trace_id: None,
            submission_id: None,
            consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            scope_dir: None,
            subject: None,
        };
        let req = build_trace_upload_claim_issuer_request(&policy, &ctx);
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("subject").is_none(), "subject omitted when None");
    }

    // --- mint_account_login_link_via_sink tests ---

    /// Minimal reqwest-backed ContributionHttpSink for use in unit tests that
    /// need to exercise the sink path against a local mock server.
    struct ReqwestContributionSink;

    #[async_trait]
    impl ContributionHttpSink for ReqwestContributionSink {
        async fn execute(
            &self,
            req: ContributionHttpRequest,
        ) -> Result<ContributionHttpResponse, ContributionHttpError> {
            let method = match req.method {
                ContributionHttpMethod::Get => reqwest::Method::GET,
                ContributionHttpMethod::Post => reqwest::Method::POST,
                ContributionHttpMethod::Put => reqwest::Method::PUT,
                ContributionHttpMethod::Delete => reqwest::Method::DELETE,
            };
            let client = reqwest::Client::new();
            let mut builder = client.request(method, &req.url);
            if let Some(token) = req.bearer_token {
                builder = builder.bearer_auth(token);
            }
            if let Some(body) = req.json_body {
                builder = builder
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body);
            }
            let response = builder
                .send()
                .await
                .map_err(|e| ContributionHttpError::new(e.to_string()))?;
            let status = response.status().as_u16();
            let body = response
                .bytes()
                .await
                .map_err(|e| ContributionHttpError::new(e.to_string()))?
                .to_vec();
            Ok(ContributionHttpResponse { status, body })
        }
    }

    /// Sink wrapper that records every request URL it executes. Used to pin
    /// the egress invariant: on the agent (sink) path, EVERY network call —
    /// including the upload-claim mint — must route through the sink, not a
    /// direct reqwest client.
    struct RecordingSink {
        inner: ReqwestContributionSink,
        urls: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                inner: ReqwestContributionSink,
                urls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ContributionHttpSink for RecordingSink {
        async fn execute(
            &self,
            req: ContributionHttpRequest,
        ) -> Result<ContributionHttpResponse, ContributionHttpError> {
            self.urls
                .lock()
                .expect("recording sink lock")
                .push(req.url.clone());
            self.inner.execute(req).await
        }
    }

    #[tokio::test]
    async fn mint_account_login_link_posts_subject_and_returns_url() {
        use std::sync::{Arc, Mutex};

        // A syntactically valid JWT that passes validate_trace_upload_claim_response.
        let claim_jwt =
            test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "test-key-1"}));
        let claim_jwt_for_mock = claim_jwt.clone();

        // ── mock server ──────────────────────────────────────────────────────
        // Two endpoints:
        //   /v1/trace-upload-claim  — upload-claim issuer (reqwest, DeviceKey mode)
        //   /v1/account/login-links — the endpoint under test (via sink)
        let captured: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();

        let app = axum::Router::new()
            .route(
                "/v1/trace-upload-claim",
                axum::routing::post(move || {
                    let jwt = claim_jwt_for_mock.clone();
                    async move {
                        // Return a syntactically valid JWT so
                        // fetch_trace_upload_claim_from_issuer is satisfied.
                        axum::Json(serde_json::json!({
                            "access_token": jwt,
                            "token_type": "Bearer",
                            "expires_in": 300
                        }))
                    }
                }),
            )
            .route(
                "/v1/account/login-links",
                axum::routing::post(move |axum::Json(b): axum::Json<serde_json::Value>| {
                    let cap = cap.clone();
                    async move {
                        cap.lock().unwrap().push(b);
                        axum::Json(serde_json::json!({
                            "account_id": "11111111-1111-1111-1111-111111111111",
                            "url": "/account/login?code=abc"
                        }))
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        // ── isolated tempdir ─────────────────────────────────────────────────
        let base = tempfile::tempdir().unwrap();

        // Instance policy (scope None) — enables instance enrollment so
        // resolve_trace_credentials_at returns a per-user subject.
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
            upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
                "127.0.0.1".to_string()
            ]),
            upload_token_tenant_id: Some("tenant-dev".to_string()),
            upload_token_audience: Some("trace-commons-ingest".to_string()),
            ..Default::default()
        };
        write_trace_policy_for_scope_at(base.path(), None, &policy)
            .expect("instance policy writes");

        // Generate and promote a device key at the instance scope dir so
        // DeviceKey auth mode can sign the workload JWT without a network call.
        let instance_dir = trace_contribution_dir_for_scope_at(base.path(), None);
        let pending =
            crate::onboarding::DeviceKeypair::load_or_generate_pending(&instance_dir, "testhash")
                .unwrap();
        pending.promote(&instance_dir, "tenant-dev").unwrap();

        // ── call under test ──────────────────────────────────────────────────
        let sink = RecordingSink::new();
        let link = mint_account_login_link_inner(base.path(), "tenant-dev", "alice", &sink)
            .await
            .unwrap();

        // ── assertions ───────────────────────────────────────────────────────
        // The server returned a RELATIVE url; it must come back absolutized
        // against the trust-anchored issuer origin, never left relative (a
        // relative URL would resolve against the consuming surface's origin).
        assert_eq!(link.url, format!("http://{addr}/account/login?code=abc"));
        assert_eq!(link.account_id, "11111111-1111-1111-1111-111111111111");

        // Egress invariant: on the agent path BOTH network calls — the
        // upload-claim mint and the login-link POST — must route through the
        // sink; a direct-reqwest claim mint would bypass RuntimeHttpEgress.
        {
            let sink_urls = sink.urls.lock().unwrap();
            assert_eq!(
                sink_urls.len(),
                2,
                "claim mint + login-link POST must both go through the sink; got {sink_urls:?}"
            );
            assert!(
                sink_urls[0].ends_with("/v1/trace-upload-claim"),
                "first sink request must be the upload-claim mint; got {sink_urls:?}"
            );
            assert!(
                sink_urls[1].ends_with("/v1/account/login-links"),
                "second sink request must be the login-link POST; got {sink_urls:?}"
            );
        }

        {
            let bodies = captured.lock().unwrap();
            assert_eq!(bodies.len(), 1, "exactly one POST to login-links");
            let expected_subject = salted_pseudonymous_contributor_id_at(
                base.path(),
                &trace_scope_key("tenant-dev", "alice"),
            )
            .unwrap();
            assert_eq!(
                bodies[0]["subject"],
                serde_json::Value::String(expected_subject),
                "posted subject must be per-user pseudonymous id for instance enrollment"
            );
        }

        // ── direct (WebUI facade) variant ────────────────────────────────────
        // Same enrollment, no sink: the hosted-WebUI path mints through the
        // pinned direct client. The link is delivered ONLY in the return value
        // (the authenticated HTTP response) — it must never be persisted to a
        // local delivery file, which hosted users cannot read.
        let direct = mint_account_login_link_direct(base.path(), "tenant-dev", "alice")
            .await
            .expect("direct login-link mint succeeds");
        assert_eq!(direct.url, format!("http://{addr}/account/login?code=abc"));
        assert_eq!(direct.account_id, "11111111-1111-1111-1111-111111111111");
        let mut delivery_files = Vec::new();
        let mut stack = vec![base.path().to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("account_login_link."))
                {
                    delivery_files.push(path);
                }
            }
        }
        assert!(
            delivery_files.is_empty(),
            "direct mint must not write a local delivery file; found {delivery_files:?}"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "direct mint must POST to login-links too"
        );
    }

    /// Write an instance policy (scope `None`) and promote a device key at the
    /// instance scope dir, so `resolve_trace_credentials_at` returns a per-user
    /// pseudonymous subject and DeviceKey auth can sign without a network call.
    /// Returns nothing — the caller reads back via the resolver.
    fn enroll_instance_with_device_key(base: &std::path::Path, addr: std::net::SocketAddr) {
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
            upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
                "127.0.0.1".to_string()
            ]),
            upload_token_tenant_id: Some("tenant-dev".to_string()),
            upload_token_audience: Some("trace-commons-ingest".to_string()),
            ingestion_endpoint: Some(format!("http://{addr}/v1/traces")),
            ..Default::default()
        };
        write_trace_policy_for_scope_at(base, None, &policy).expect("instance policy writes");
        let instance_dir = trace_contribution_dir_for_scope_at(base, None);
        let pending =
            crate::onboarding::DeviceKeypair::load_or_generate_pending(&instance_dir, "testhash")
                .unwrap();
        pending.promote(&instance_dir, "tenant-dev").unwrap();
    }

    /// The instance-aware profile-token mint (`*_for_user_*`) must resolve the
    /// shared instance enrollment for a user with no personal-invite policy, and
    /// carry that user's pseudonymous subject to the upload-claim issuer — else
    /// instance-only contributors are falsely rejected as not enrolled.
    #[tokio::test]
    async fn mint_profile_attribution_token_for_user_uses_instance_subject() {
        use std::sync::{Arc, Mutex};

        let claim_jwt =
            test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "test-key-1"}));
        let claim_jwt_for_mock = claim_jwt.clone();
        let claim_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let claim_cap = claim_bodies.clone();

        let app = axum::Router::new().route(
            "/v1/trace-upload-claim",
            axum::routing::post(move |axum::Json(b): axum::Json<serde_json::Value>| {
                let jwt = claim_jwt_for_mock.clone();
                let claim_cap = claim_cap.clone();
                async move {
                    claim_cap.lock().unwrap().push(b);
                    axum::Json(serde_json::json!({
                        "access_token": jwt,
                        "token_type": "Bearer",
                        "expires_in": 300
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let base = tempfile::tempdir().unwrap();
        enroll_instance_with_device_key(base.path(), addr);

        let sink = ReqwestContributionSink;
        let token = mint_profile_attribution_token_for_user_inner(
            base.path(),
            "tenant-dev",
            "alice",
            &sink,
        )
        .await
        .expect("instance-enrolled user mints a profile-attribution token");
        assert_eq!(token.access_token, claim_jwt);

        let bodies = claim_bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1, "exactly one claim request");
        let expected_subject = salted_pseudonymous_contributor_id_at(
            base.path(),
            &trace_scope_key("tenant-dev", "alice"),
        )
        .unwrap();
        assert_eq!(
            bodies[0]["subject"],
            serde_json::Value::String(expected_subject),
            "claim request must carry the per-user pseudonymous subject for instance enrollment"
        );
    }

    /// The instance-aware community-profile publish (`*_for_user_*`) must resolve
    /// the shared instance enrollment, mint under the per-user subject, and PUT
    /// the profile — proving instance-only contributors can publish a profile.
    #[tokio::test]
    async fn set_community_profile_for_user_publishes_under_instance_subject() {
        use std::sync::{Arc, Mutex};

        let claim_jwt =
            test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "test-key-1"}));
        let claim_jwt_for_mock = claim_jwt.clone();
        let claim_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let claim_cap = claim_bodies.clone();
        let profile_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let profile_cap = profile_bodies.clone();

        let app = axum::Router::new()
            .route(
                "/v1/trace-upload-claim",
                axum::routing::post(move |axum::Json(b): axum::Json<serde_json::Value>| {
                    let jwt = claim_jwt_for_mock.clone();
                    let claim_cap = claim_cap.clone();
                    async move {
                        claim_cap.lock().unwrap().push(b);
                        axum::Json(serde_json::json!({
                            "access_token": jwt,
                            "token_type": "Bearer",
                            "expires_in": 300
                        }))
                    }
                }),
            )
            .route(
                "/v1/community/profile",
                axum::routing::put(move |axum::Json(b): axum::Json<serde_json::Value>| {
                    let profile_cap = profile_cap.clone();
                    async move {
                        profile_cap.lock().unwrap().push(b);
                        axum::Json(serde_json::json!({ "ok": true }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let base = tempfile::tempdir().unwrap();
        enroll_instance_with_device_key(base.path(), addr);

        let sink = ReqwestContributionSink;
        set_community_profile_for_user_inner(
            base.path(),
            "tenant-dev",
            "alice",
            "pilot_alice",
            Some("Trace Commons pilot"),
            Some(&sink),
        )
        .await
        .expect("instance-enrolled user publishes a community profile");

        let expected_subject = salted_pseudonymous_contributor_id_at(
            base.path(),
            &trace_scope_key("tenant-dev", "alice"),
        )
        .unwrap();
        let claims = claim_bodies.lock().unwrap();
        assert_eq!(claims.len(), 1, "exactly one claim request");
        assert_eq!(
            claims[0]["subject"],
            serde_json::Value::String(expected_subject),
            "claim request must carry the per-user pseudonymous subject for instance enrollment"
        );
        let profiles = profile_bodies.lock().unwrap();
        assert_eq!(profiles.len(), 1, "exactly one community-profile PUT");
        assert_eq!(
            profiles[0]["display_handle"],
            serde_json::json!("pilot_alice")
        );
    }

    /// Helper: instance-enroll a tempdir against a mock whose
    /// `/v1/account/login-links` returns `link_url`, then mint via the direct
    /// path. Pins the origin-anchoring contract for hostile response URLs.
    async fn mint_login_link_with_response_url(
        link_url: &str,
    ) -> (Result<AccountLoginLink, AccountLoginLinkError>, String) {
        let claim_jwt = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "k1"}));
        let claim_jwt_for_mock = claim_jwt.clone();
        let link_url = link_url.to_string();
        let app = axum::Router::new()
            .route(
                "/v1/trace-upload-claim",
                axum::routing::post(move || {
                    let jwt = claim_jwt_for_mock.clone();
                    async move {
                        axum::Json(serde_json::json!({
                            "access_token": jwt, "token_type": "Bearer", "expires_in": 300
                        }))
                    }
                }),
            )
            .route(
                "/v1/account/login-links",
                axum::routing::post(move || {
                    let url = link_url.clone();
                    async move {
                        axum::Json(serde_json::json!({
                            "account_id": "11111111-1111-1111-1111-111111111111",
                            "url": url
                        }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let base = tempfile::tempdir().unwrap();
        enroll_instance_with_device_key(base.path(), addr);
        let result = mint_account_login_link_direct(base.path(), "tenant-dev", "alice").await;
        (result, format!("http://{addr}"))
    }

    #[tokio::test]
    async fn mint_account_login_link_pins_url_to_issuer_origin() {
        // Relative url: absolutized against the trust-anchored issuer origin.
        let (result, origin) = mint_login_link_with_response_url("/account/login?code=rel").await;
        let link = result.expect("relative url absolutizes");
        assert_eq!(link.url, format!("{origin}/account/login?code=rel"));

        // Cross-origin ABSOLUTE url: a hostile issuer response must not steer
        // the authenticated browser to another origin.
        let (result, _) =
            mint_login_link_with_response_url("https://attacker.example/account/login").await;
        let error = result.expect_err("cross-origin absolute url must be rejected");
        assert!(
            error.to_string().to_lowercase().contains("login link")
                || matches!(error, AccountLoginLinkError::Backend(_)),
            "cross-origin rejection surfaces as a Backend error: {error}"
        );

        // Non-HTTP(S) scheme: must be rejected (javascript: would execute in
        // the opened tab's context).
        let (result, _) =
            mint_login_link_with_response_url("javascript:alert(document.domain)").await;
        result.expect_err("non-http scheme must be rejected");

        // Userinfo smuggling: rejected even when the host would match. (The
        // mock's port isn't knowable before it binds, so a same-host+userinfo
        // URL can't be fabricated exactly — but userinfo is rejected before
        // the origin comparison, which cross-host coverage above pins anyway.)
        let (result, _) =
            mint_login_link_with_response_url("http://user:pass@127.0.0.1/account/login").await;
        result.expect_err("userinfo in the login-link url must be rejected");
    }

    #[tokio::test]
    async fn direct_pinned_sink_rejects_private_hosts_and_bounds_bodies() {
        // Disallowed (link-local/metadata) host: rejected at resolution, before
        // any request is built.
        let error = DirectPinnedContributionSink
            .execute(ContributionHttpRequest {
                method: ContributionHttpMethod::Get,
                url: "http://169.254.169.254/v1/anything".to_string(),
                bearer_token: Some("secret".to_string()),
                json_body: None,
                response_body_limit: 1024,
                timeout_ms: 2_000,
            })
            .await
            .expect_err("link-local host must be rejected");
        assert!(
            error.to_string().contains("resolution rejected")
                || error.to_string().contains("rejected"),
            "rejection must come from host resolution: {error}"
        );

        // Redirects are NOT followed: the 3xx surfaces as the response status
        // and the Location target is never contacted.
        let hit = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hit_for_target = hit.clone();
        let target_app = axum::Router::new().route(
            "/stolen",
            axum::routing::get(move || {
                let hit = hit_for_target.clone();
                async move {
                    hit.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    "leaked"
                }
            }),
        );
        let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(target_listener, target_app).await;
        });
        let redirect_to = format!("http://{target_addr}/stolen");
        let redirect_app = axum::Router::new().route(
            "/hop",
            axum::routing::get(move || {
                let location = redirect_to.clone();
                async move {
                    (
                        axum::http::StatusCode::FOUND,
                        [(axum::http::header::LOCATION, location)],
                        "",
                    )
                }
            }),
        );
        let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_addr = redirect_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(redirect_listener, redirect_app).await;
        });
        let response = DirectPinnedContributionSink
            .execute(ContributionHttpRequest {
                method: ContributionHttpMethod::Get,
                url: format!("http://{redirect_addr}/hop"),
                bearer_token: Some("secret".to_string()),
                json_body: None,
                response_body_limit: 1024,
                timeout_ms: 2_000,
            })
            .await
            .expect("redirect response surfaces, not followed");
        assert_eq!(response.status, 302, "3xx must surface as the status");
        assert_eq!(
            hit.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the redirect target must never be contacted"
        );

        // Oversized body: rejected DURING the read, not buffered.
        let big_app = axum::Router::new().route(
            "/big",
            axum::routing::get(|| async { "x".repeat(64 * 1024) }),
        );
        let big_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let big_addr = big_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(big_listener, big_app).await;
        });
        let error = DirectPinnedContributionSink
            .execute(ContributionHttpRequest {
                method: ContributionHttpMethod::Get,
                url: format!("http://{big_addr}/big"),
                bearer_token: None,
                json_body: None,
                response_body_limit: 1024,
                timeout_ms: 5_000,
            })
            .await
            .expect_err("oversized body must be rejected");
        assert!(
            error.to_string().contains("exceeds"),
            "rejection names the byte limit: {error}"
        );
    }

    #[tokio::test]
    async fn mint_account_login_link_errors_when_not_enrolled() {
        let base = tempfile::tempdir().unwrap();
        // No policy written — resolver returns None.
        let sink = ReqwestContributionSink;
        let err = mint_account_login_link_inner(base.path(), "tenant-dev", "alice", &sink)
            .await
            .expect_err("unenrolled user must error");
        assert!(
            err.to_string().contains("not enrolled"),
            "error must mention enrollment: {err}"
        );
    }

    #[test]
    fn account_login_links_url_errors_on_wrong_suffix() {
        // URL that does NOT end in /v1/trace-upload-claim — must error, not silently misroute.
        let policy = StandingTraceContributionPolicy {
            upload_token_issuer_url: Some(
                "https://api.example.com/v2/trace-upload-claim".to_string(),
            ),
            ..Default::default()
        };
        let err = account_login_links_url(&policy).expect_err("wrong suffix must be an error");
        assert!(
            err.to_string()
                .contains("does not end in /v1/trace-upload-claim"),
            "error must name the expected suffix: {err}"
        );
    }

    #[test]
    fn account_login_links_url_correct_on_valid_issuer() {
        let policy = StandingTraceContributionPolicy {
            upload_token_issuer_url: Some(
                "https://api.example.com/v1/trace-upload-claim".to_string(),
            ),
            ..Default::default()
        };
        let url = account_login_links_url(&policy).expect("valid issuer must succeed");
        assert_eq!(url, "https://api.example.com/v1/account/login-links");
    }

    #[test]
    fn account_traces_url_correct_with_and_without_limit() {
        let policy = StandingTraceContributionPolicy {
            upload_token_issuer_url: Some(
                "https://api.example.com/v1/trace-upload-claim".to_string(),
            ),
            ..Default::default()
        };
        // None defaults to the bounded page size (never an unbounded fetch).
        let url_no_limit = account_traces_url(&policy, None).expect("no-limit must succeed");
        assert_eq!(
            url_no_limit,
            format!(
                "https://api.example.com/v1/account/traces?limit={}",
                ACCOUNT_TRACES_DEFAULT_LIMIT
            )
        );
        let url_with_limit = account_traces_url(&policy, Some(50)).expect("limit=50 must succeed");
        assert_eq!(
            url_with_limit,
            "https://api.example.com/v1/account/traces?limit=50"
        );
        // An over-large limit is clamped to the hard ceiling.
        let url_clamped =
            account_traces_url(&policy, Some(100_000)).expect("large limit must succeed");
        assert_eq!(
            url_clamped,
            format!(
                "https://api.example.com/v1/account/traces?limit={}",
                ACCOUNT_TRACES_MAX_LIMIT
            )
        );
    }

    #[tokio::test]
    async fn fetch_account_traces_returns_user_submissions() {
        // A syntactically valid JWT that passes validate_trace_upload_claim_response.
        let claim_jwt =
            test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "test-key-1"}));
        let claim_jwt_for_mock = claim_jwt.clone();

        // ── mock server ──────────────────────────────────────────────────────
        // Two endpoints:
        //   /v1/trace-upload-claim  — upload-claim issuer (DeviceKey mode)
        //   /v1/account/traces      — the endpoint under test (via sink)
        let app = axum::Router::new()
            .route(
                "/v1/trace-upload-claim",
                axum::routing::post(move || {
                    let jwt = claim_jwt_for_mock.clone();
                    async move {
                        axum::Json(serde_json::json!({
                            "access_token": jwt,
                            "token_type": "Bearer",
                            "expires_in": 300
                        }))
                    }
                }),
            )
            .route(
                "/v1/account/traces",
                axum::routing::get(|| async {
                    axum::Json(serde_json::json!([
                        {
                            "submission_id": "s1",
                            "status": "accepted",
                            "credit_points_pending": 1.0,
                            "credit_points_final": 1.0,
                            "received_at": "2026-06-25T00:00:00Z"
                        }
                    ]))
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        // ── isolated tempdir ─────────────────────────────────────────────────
        let base = tempfile::tempdir().unwrap();

        // Instance policy (scope None) — enables instance enrollment so
        // resolve_trace_credentials_at returns a per-user subject.
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
            upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
                "127.0.0.1".to_string()
            ]),
            upload_token_tenant_id: Some("tenant-dev".to_string()),
            upload_token_audience: Some("trace-commons-ingest".to_string()),
            ..Default::default()
        };
        write_trace_policy_for_scope_at(base.path(), None, &policy)
            .expect("instance policy writes");

        // Generate and promote a device key at the instance scope dir so
        // DeviceKey auth mode can sign the workload JWT without a network call.
        let instance_dir = trace_contribution_dir_for_scope_at(base.path(), None);
        let pending =
            crate::onboarding::DeviceKeypair::load_or_generate_pending(&instance_dir, "testhash")
                .unwrap();
        pending.promote(&instance_dir, "tenant-dev").unwrap();

        // ── call under test ──────────────────────────────────────────────────
        let sink = RecordingSink::new();
        let items = fetch_account_traces_inner(base.path(), "tenant-dev", "alice", Some(50), &sink)
            .await
            .unwrap();

        // ── assertions ───────────────────────────────────────────────────────
        // Egress invariant: claim mint + traces GET must both route through
        // the sink on the agent path (no direct-reqwest claim mint).
        {
            let sink_urls = sink.urls.lock().unwrap();
            assert_eq!(
                sink_urls.len(),
                2,
                "claim mint + traces GET must both go through the sink; got {sink_urls:?}"
            );
            assert!(
                sink_urls[0].ends_with("/v1/trace-upload-claim"),
                "first sink request must be the upload-claim mint; got {sink_urls:?}"
            );
            assert!(
                sink_urls[1].contains("/v1/account/traces"),
                "second sink request must be the traces GET; got {sink_urls:?}"
            );
        }
        assert_eq!(items.len(), 1, "expected exactly one trace item");
        assert_eq!(items[0].submission_id, "s1");
        assert_eq!(items[0].status, "accepted");
        assert!(
            (items[0].credit_points_pending - 1.0).abs() < f32::EPSILON,
            "credit_points_pending must be 1.0"
        );
        assert_eq!(
            items[0].credit_points_final,
            Some(1.0),
            "credit_points_final must be Some(1.0)"
        );
        assert_eq!(
            items[0].received_at.as_deref(),
            Some("2026-06-25T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn fetch_account_traces_returns_empty_when_not_enrolled() {
        let base = tempfile::tempdir().unwrap();
        // No policy written — resolver returns None → lenient Ok(vec![]).
        let sink = ReqwestContributionSink;
        let items = fetch_account_traces_inner(base.path(), "tenant-dev", "alice", None, &sink)
            .await
            .unwrap();
        assert!(items.is_empty(), "unenrolled user must return empty list");
    }

    /// Helper: enroll an instance scope at `base` against a mock that serves the
    /// claim issuer plus `/v1/account/traces` returning `status`/`body`. Returns
    /// the results of BOTH fetch paths — the sink-backed
    /// `fetch_account_traces_inner` (agent path) and the direct
    /// `fetch_account_traces_direct` (WebUI/CLI path, pinned reqwest client) —
    /// so status-handling regressions in either path are caught.
    async fn fetch_account_traces_with_status(
        status: axum::http::StatusCode,
        body: serde_json::Value,
    ) -> (
        anyhow::Result<Vec<AccountTraceItem>>,
        anyhow::Result<Vec<AccountTraceItem>>,
    ) {
        let claim_jwt = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "k1"}));
        let claim_jwt_for_mock = claim_jwt.clone();
        let app = axum::Router::new()
            .route(
                "/v1/trace-upload-claim",
                axum::routing::post(move || {
                    let jwt = claim_jwt_for_mock.clone();
                    async move {
                        axum::Json(serde_json::json!({
                            "access_token": jwt, "token_type": "Bearer", "expires_in": 300
                        }))
                    }
                }),
            )
            .route(
                "/v1/account/traces",
                axum::routing::get(move || {
                    let body = body.clone();
                    async move { (status, axum::Json(body)) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let base = tempfile::tempdir().unwrap();
        let policy = StandingTraceContributionPolicy {
            enabled: true,
            auth_mode: TraceUploadAuthMode::DeviceKey,
            upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
            upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
                "127.0.0.1".to_string()
            ]),
            upload_token_tenant_id: Some("tenant-dev".to_string()),
            upload_token_audience: Some("trace-commons-ingest".to_string()),
            ..Default::default()
        };
        write_trace_policy_for_scope_at(base.path(), None, &policy).unwrap();
        let instance_dir = trace_contribution_dir_for_scope_at(base.path(), None);
        crate::onboarding::DeviceKeypair::load_or_generate_pending(&instance_dir, "h")
            .unwrap()
            .promote(&instance_dir, "tenant-dev")
            .unwrap();

        let sink = ReqwestContributionSink;
        let via_sink =
            fetch_account_traces_inner(base.path(), "tenant-dev", "alice", None, &sink).await;
        let direct = fetch_account_traces_direct(base.path(), "tenant-dev", "alice", None).await;
        (via_sink, direct)
    }

    #[tokio::test]
    async fn fetch_account_traces_errors_on_server_error() {
        // A 5xx must NOT be swallowed as an empty list — it surfaces as Err so
        // the WebUI boundary renders a sanitized unavailable state. Both the
        // sink-backed (agent) and direct (WebUI/CLI) paths must agree.
        let (via_sink, direct) = fetch_account_traces_with_status(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": "boom"}),
        )
        .await;
        assert!(
            via_sink.is_err(),
            "sink path: 5xx must surface as an error, not empty"
        );
        assert!(
            direct.is_err(),
            "direct path: 5xx must surface as an error, not empty"
        );
    }

    #[tokio::test]
    async fn fetch_account_traces_404_is_empty() {
        // 404 = no account/traces yet for this enrolled principal → legitimate
        // empty state, not an error. Both fetch paths must agree.
        let (via_sink, direct) = fetch_account_traces_with_status(
            axum::http::StatusCode::NOT_FOUND,
            serde_json::json!({"error": "no account"}),
        )
        .await;
        assert!(
            via_sink
                .expect("sink path: 404 must be the empty zero-state")
                .is_empty()
        );
        assert!(
            direct
                .expect("direct path: 404 must be the empty zero-state")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn pinned_trace_remote_client_rejects_private_endpoint_hosts() {
        // The background submit/status/revoke lane pins DNS per request: a host
        // resolving to a private/link-local address must be rejected before any
        // bearer-authenticated request is built (DNS-rebinding defense).
        let error = pinned_trace_remote_http_client("http://169.254.169.254/v1/traces")
            .await
            .expect_err("link-local endpoint host must be rejected");
        assert_eq!(error.kind, TraceQueueTelemetryFailureKind::NetworkDns);

        // The literal-loopback local-dev exception still applies.
        pinned_trace_remote_http_client("http://127.0.0.1:8080/v1/traces")
            .await
            .expect("literal loopback endpoint builds (local-dev exception)");
    }
}
