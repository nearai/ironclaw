//! Wire schema for a trace contribution: the envelope, its consent,
//! contributor, privacy and outcome metadata, the event stream, and the card
//! and scorecard value types serialized alongside it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::*;

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
/// explicit limit. Bounds the initial WebUI/service slice so `None` never
/// requests unbounded history; full history is a future paginated flow.
pub(crate) const ACCOUNT_TRACES_DEFAULT_LIMIT: usize = 200;
/// Hard ceiling on the account-traces page size; larger requests are clamped so
/// a caller can never ask the server for an unbounded slice.
pub(crate) const ACCOUNT_TRACES_MAX_LIMIT: usize = 500;
/// Maximum accepted account-traces response body (256 KiB). A list response is
/// larger than a single claim but must still be bounded so the direct path
/// cannot buffer an unbounded body.
pub(crate) const ACCOUNT_TRACES_MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub(crate) const TRACE_UPLOAD_CLAIM_REFRESH_SKEW_SECONDS: i64 = 60;
pub(crate) const TRACE_CREDIT_NOTICE_OUTBOX_MAX_ATTEMPTS_STORED: usize = 10;

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
    /// The originating extension/surface id when `channel` is
    /// [`TraceChannel::Extension`] (generic origin data replacing the retired
    /// concrete variants).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceChannel {
    Web,
    Cli,
    Routine,
    Other,
    /// An extension-served channel. The concrete identity lives in
    /// `IronclawTraceMetadata::channel_origin` (data, never an enum variant).
    /// `#[serde(other)]` also absorbs historical concrete tags and any
    /// future unknown tag — persisted envelopes always deserialize (LLM data
    /// is never dropped to a parse quarantine over a channel name).
    #[serde(other)]
    Extension,
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
