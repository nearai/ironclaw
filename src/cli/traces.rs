//! Trace contribution CLI commands.
//!
//! These commands are deliberately opt-in and local-first. `preview` creates a
//! redacted contribution envelope from an existing recorded trace. `submit`
//! only uploads when the user provides an explicit ingestion endpoint.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Subcommand, ValueEnum};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::trace_contribution::{
    ConsentScope, CreditSummary, DeterministicTraceRedactor, RecordedTraceContributionOptions,
    ResidualPiiRisk, StandingTraceContributionPolicy, TraceChannel, TraceContributionEnvelope,
    TraceCreditEvent, TraceCreditEventKind, TraceRedactor, TraceSubmissionStatusUpdate,
    estimate_initial_credit, fetch_trace_submission_statuses, privacy_filter_adapter_from_env,
    trace_submission_status_endpoint,
};

#[derive(Subcommand, Debug, Clone)]
pub enum TracesCommand {
    /// Enable autonomous trace contribution after local redaction
    OptIn {
        /// Explicit private ingestion endpoint URL
        #[arg(long)]
        endpoint: String,

        /// Environment variable containing the bearer token for the endpoint
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Include locally redacted user/assistant message text
        #[arg(long)]
        include_message_text: bool,

        /// Include locally redacted tool arguments, tool results, and HTTP bodies
        #[arg(long)]
        include_tool_payloads: bool,

        /// Consent scope to include in autonomous envelopes
        #[arg(long, value_enum, default_value_t = TraceScopeArg::DebuggingEvaluation)]
        scope: TraceScopeArg,

        /// Only auto-submit traces that use these tool names
        #[arg(long, value_delimiter = ',')]
        selected_tools: Vec<String>,

        /// Submit medium-risk traces without holding them for manual review
        #[arg(long)]
        allow_pii_review_bypass: bool,

        /// Minimum local score required for autonomous submission
        #[arg(long, default_value_t = 0.35)]
        min_submission_score: f32,
    },

    /// Disable autonomous trace contribution
    OptOut,

    /// Show local standing trace contribution policy
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Preview a redacted contribution envelope from a recorded trace file
    Preview {
        /// Recorded trace JSON file produced by IRONCLAW_RECORD_TRACE
        #[arg(long, value_name = "PATH")]
        recorded_trace: PathBuf,

        /// Include locally redacted user/assistant message text
        #[arg(long)]
        include_message_text: bool,

        /// Include locally redacted tool arguments, tool results, and HTTP bodies
        #[arg(long)]
        include_tool_payloads: bool,

        /// Consent scope to include in the envelope
        #[arg(long, value_enum, default_value_t = TraceScopeArg::DebuggingEvaluation)]
        scope: TraceScopeArg,

        /// Source channel for this trace
        #[arg(long, value_enum, default_value_t = TraceChannelArg::Cli)]
        channel: TraceChannelArg,

        /// Optional engine version metadata
        #[arg(long)]
        engine_version: Option<String>,

        /// Optional pseudonymous contributor ID
        #[arg(long)]
        contributor_id: Option<String>,

        /// Optional separate credit account reference
        #[arg(long)]
        credit_account_ref: Option<String>,

        /// Write envelope JSON to a file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        output: Option<PathBuf>,

        /// Add the redacted envelope to the autonomous submission queue
        #[arg(long)]
        enqueue: bool,
    },

    /// Add an already-previewed envelope to the autonomous submission queue
    Enqueue {
        /// Redacted contribution envelope JSON file
        #[arg(long, value_name = "PATH")]
        envelope: PathBuf,
    },

    /// Submit eligible queued envelopes using the standing opt-in policy
    FlushQueue {
        /// Maximum queued envelopes to submit
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },

    /// Show local credit totals and recent credit explanations
    Credit {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Submit an already-previewed redacted contribution envelope
    Submit {
        /// Redacted contribution envelope JSON file
        #[arg(long, value_name = "PATH")]
        envelope: PathBuf,

        /// Explicit private ingestion endpoint URL
        #[arg(long)]
        endpoint: String,

        /// Environment variable containing the bearer token for the endpoint
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,
    },

    /// List local trace contribution submission records
    ListSubmissions {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Include aggregate submission and credit totals
        #[arg(long)]
        summary: bool,
    },

    /// Revoke a trace contribution locally and, optionally, at an ingestion API
    Revoke {
        /// Submission ID to revoke
        submission_id: Uuid,

        /// Optional private revocation endpoint URL
        #[arg(long)]
        endpoint: Option<String>,

        /// Environment variable containing the bearer token for the endpoint
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,
    },

    /// Check a Trace Commons ingestion service /health endpoint
    IngestHealth {
        /// Trace Commons ingestion base URL, /health URL, or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List centrally quarantined traces for reviewer/admin triage
    QuarantineList {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List active-learning central trace review work
    ActiveLearningReviewQueue {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Maximum review work items to return
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by residual privacy risk
        #[arg(long, value_enum)]
        privacy_risk: Option<TracePrivacyRiskArg>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Approve or reject a quarantined central trace
    ReviewDecision {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Submission ID to review
        submission_id: Uuid,

        /// Review decision
        #[arg(long, value_enum)]
        decision: TraceReviewDecisionArg,

        /// Optional reviewer/admin rationale
        #[arg(long)]
        reason: Option<String>,

        /// Optional pending credit override for approved traces
        #[arg(long)]
        credit_points_pending: Option<f32>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Append a delayed central credit ledger event
    AppendCreditEvent {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Submission ID receiving delayed credit
        submission_id: Uuid,

        /// Delayed credit event type
        #[arg(long, value_enum)]
        event_type: TraceCreditEventTypeArg,

        /// Credit point delta; use a negative value for penalties
        #[arg(long)]
        credit_points_delta: f32,

        /// Explanation for the contributor/reviewer ledger
        #[arg(long)]
        reason: String,

        /// Optional benchmark/job/external reference
        #[arg(long)]
        external_ref: Option<String>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show central Trace Commons analytics summary
    AnalyticsSummary {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run central retention/revocation and DB reconciliation maintenance
    MaintenanceRun {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Explicit maintenance purpose recorded in central audit metadata
        #[arg(long)]
        purpose: String,

        /// Run without mutating central state when the service supports it
        #[arg(long)]
        dry_run: bool,

        /// Backfill file-backed Trace Commons metadata into the configured DB mirror
        #[arg(long)]
        backfill_db_mirror: bool,

        /// Index accepted canonical summaries into DB vector metadata rows
        #[arg(long)]
        index_vectors: bool,

        /// RFC3339 cutoff; expired submissions at or before this time are purged
        #[arg(long)]
        purge_expired_before: Option<String>,

        /// Maximum age in hours for export-cache pruning
        #[arg(long)]
        max_export_age_hours: Option<i64>,

        /// Leave invalid export-cache files in place
        #[arg(long)]
        skip_export_cache_prune: bool,

        /// Include DB mirror reconciliation and reader parity diagnostics
        #[arg(long)]
        reconcile_db_mirror: bool,

        /// Include file-backed audit hash-chain verification diagnostics
        #[arg(long)]
        verify_audit_chain: bool,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run scoped central retention maintenance through the worker route
    WorkerRetentionMaintenance {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Optional retention purpose recorded in central audit metadata
        #[arg(long)]
        purpose: Option<String>,

        /// Run without mutating central state when the service supports it
        #[arg(long)]
        dry_run: bool,

        /// RFC3339 cutoff; expired submissions at or before this time are purged
        #[arg(long)]
        purge_expired_before: Option<String>,

        /// Maximum age in hours for export-cache pruning
        #[arg(long)]
        max_export_age_hours: Option<i64>,

        /// Leave invalid export-cache files in place
        #[arg(long)]
        skip_export_cache_prune: bool,

        /// Environment variable containing a retention-worker bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Index scoped vector metadata through the worker route
    WorkerVectorIndex {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Optional vector indexing purpose recorded in central audit metadata
        #[arg(long)]
        purpose: Option<String>,

        /// Run without mutating central state when the service supports it
        #[arg(long)]
        dry_run: bool,

        /// Environment variable containing a vector-worker bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Start central benchmark conversion for approved replayable traces
    BenchmarkConvert {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Explicit conversion purpose recorded in central audit metadata
        #[arg(long)]
        purpose: String,

        /// Maximum traces to convert
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by consent scope
        #[arg(long, value_enum)]
        consent_scope: Option<TraceScopeArg>,

        /// Filter by central corpus status
        #[arg(long, value_enum)]
        status: Option<TraceCorpusStatusArg>,

        /// Filter by residual privacy risk
        #[arg(long, value_enum)]
        privacy_risk: Option<TracePrivacyRiskArg>,

        /// Optional benchmark/job/external reference
        #[arg(long)]
        external_ref: Option<String>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Start benchmark conversion through the worker route
    WorkerBenchmarkConvert {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Explicit conversion purpose recorded in central audit metadata
        #[arg(long)]
        purpose: String,

        /// Maximum traces to convert
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by consent scope
        #[arg(long, value_enum)]
        consent_scope: Option<TraceScopeArg>,

        /// Filter by central corpus status
        #[arg(long, value_enum)]
        status: Option<TraceCorpusStatusArg>,

        /// Filter by residual privacy risk
        #[arg(long, value_enum)]
        privacy_risk: Option<TracePrivacyRiskArg>,

        /// Optional benchmark/job/external reference
        #[arg(long)]
        external_ref: Option<String>,

        /// Environment variable containing a benchmark-worker bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Update benchmark registry/evaluator lifecycle metadata
    BenchmarkLifecycleUpdate {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Benchmark conversion id to update
        #[arg(long)]
        conversion_id: Uuid,

        /// Registry lifecycle status
        #[arg(long, value_enum)]
        registry_status: Option<TraceBenchmarkRegistryStatusArg>,

        /// Registry artifact/reference id
        #[arg(long)]
        registry_ref: Option<String>,

        /// Published timestamp in RFC3339 format
        #[arg(long)]
        published_at: Option<String>,

        /// Evaluator lifecycle status
        #[arg(long, value_enum)]
        evaluation_status: Option<TraceBenchmarkEvaluationStatusArg>,

        /// Evaluator run/reference id
        #[arg(long)]
        evaluator_ref: Option<String>,

        /// Evaluated timestamp in RFC3339 format
        #[arg(long)]
        evaluated_at: Option<String>,

        /// Evaluator score from 0.0 to 1.0
        #[arg(long)]
        score: Option<f32>,

        /// Number of passed evaluator checks
        #[arg(long)]
        pass_count: Option<u32>,

        /// Number of failed evaluator checks
        #[arg(long)]
        fail_count: Option<u32>,

        /// Human/operator reason for the lifecycle update
        #[arg(long)]
        reason: Option<String>,

        /// Environment variable containing a reviewer/admin/benchmark-worker bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Export an approved replay dataset slice from the central corpus
    ReplayDatasetExport {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Explicit export purpose for services that enforce export manifests
        #[arg(long)]
        purpose: Option<String>,

        /// Filter by consent scope
        #[arg(long, value_enum)]
        consent_scope: Option<TraceScopeArg>,

        /// Filter by central corpus status
        #[arg(long, value_enum)]
        status: Option<TraceCorpusStatusArg>,

        /// Filter by residual privacy risk
        #[arg(long, value_enum)]
        privacy_risk: Option<TracePrivacyRiskArg>,

        /// Maximum exported dataset items
        #[arg(long)]
        limit: Option<usize>,

        /// Write export JSON to a file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        output: Option<PathBuf>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List replay export manifest metadata from the central corpus
    ReplayExportManifests {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show safe non-secret Trace Commons config cutover/status booleans
    ConfigStatus {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,
    },

    /// Read the DB-backed tenant contribution policy
    TenantPolicyGet {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Write the DB-backed tenant contribution policy
    TenantPolicySet {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Policy version string recorded with tenant policy audit metadata
        #[arg(long)]
        policy_version: String,

        /// Allowed consent scopes, comma separated
        #[arg(long, value_enum, value_delimiter = ',')]
        allowed_consent_scopes: Vec<TraceScopeArg>,

        /// Allowed trace-card uses, comma separated
        #[arg(long, value_enum, value_delimiter = ',')]
        allowed_uses: Vec<TraceAllowedUseArg>,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Export approved ranker training candidates
    RankerTrainingCandidates {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Explicit export purpose for services that enforce ranker manifests
        #[arg(long)]
        purpose: Option<String>,

        /// Filter by consent scope
        #[arg(long, value_enum)]
        consent_scope: Option<TraceScopeArg>,

        /// Filter by corpus status
        #[arg(long, value_enum)]
        status: Option<TraceCorpusStatusArg>,

        /// Filter by residual privacy risk
        #[arg(long, value_enum)]
        privacy_risk: Option<TracePrivacyRiskArg>,

        /// Maximum exported training candidates
        #[arg(long)]
        limit: Option<usize>,

        /// Write export JSON to a file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        output: Option<PathBuf>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Export approved ranker training pairs
    RankerTrainingPairs {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Explicit export purpose for services that enforce ranker manifests
        #[arg(long)]
        purpose: Option<String>,

        /// Filter by consent scope
        #[arg(long, value_enum)]
        consent_scope: Option<TraceScopeArg>,

        /// Filter by corpus status
        #[arg(long, value_enum)]
        status: Option<TraceCorpusStatusArg>,

        /// Filter by residual privacy risk
        #[arg(long, value_enum)]
        privacy_risk: Option<TracePrivacyRiskArg>,

        /// Maximum exported training pairs
        #[arg(long)]
        limit: Option<usize>,

        /// Write export JSON to a file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        output: Option<PathBuf>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List central Trace Commons audit events
    AuditEvents {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Maximum audit events to return
        #[arg(long)]
        limit: Option<usize>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List central trace metadata with optional reviewer filters
    ListTraces {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Compatibility purpose hint for services that support purpose-indexed trace lists
        #[arg(long)]
        purpose: Option<String>,

        /// Filter by consent scope
        #[arg(long, value_enum)]
        consent_scope: Option<TraceScopeArg>,

        /// Filter by central corpus status
        #[arg(long, value_enum)]
        status: Option<TraceCorpusStatusArg>,

        /// Maximum traces to return
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by coverage tag
        #[arg(long)]
        coverage_tag: Option<String>,

        /// Filter by required tool name
        #[arg(long)]
        tool: Option<String>,

        /// Filter by residual privacy risk
        #[arg(long, value_enum)]
        privacy_risk: Option<TracePrivacyRiskArg>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run a local Privacy Filter sidecar canary check
    PrivacyFilterCanary {
        /// Synthetic canary text to send to the configured sidecar
        #[arg(
            long,
            default_value = "Canary email alice@example.com with key sk-test-123 and path /Users/alice/private.txt"
        )]
        text: String,

        /// Sidecar timeout in seconds
        #[arg(long, default_value_t = 10)]
        timeout_seconds: u64,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceScopeArg {
    DebuggingEvaluation,
    BenchmarkOnly,
    RankingTraining,
    ModelTraining,
}

impl std::fmt::Display for TraceScopeArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::DebuggingEvaluation => "debugging-evaluation",
            Self::BenchmarkOnly => "benchmark-only",
            Self::RankingTraining => "ranking-training",
            Self::ModelTraining => "model-training",
        };
        write!(f, "{value}")
    }
}

impl From<TraceScopeArg> for ConsentScope {
    fn from(value: TraceScopeArg) -> Self {
        match value {
            TraceScopeArg::DebuggingEvaluation => ConsentScope::DebuggingEvaluation,
            TraceScopeArg::BenchmarkOnly => ConsentScope::BenchmarkOnly,
            TraceScopeArg::RankingTraining => ConsentScope::RankingTraining,
            TraceScopeArg::ModelTraining => ConsentScope::ModelTraining,
        }
    }
}

fn trace_scope_server_value(scope: TraceScopeArg) -> &'static str {
    match scope {
        TraceScopeArg::DebuggingEvaluation => "debugging_evaluation",
        TraceScopeArg::BenchmarkOnly => "benchmark_only",
        TraceScopeArg::RankingTraining => "ranking_training",
        TraceScopeArg::ModelTraining => "model_training",
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceAllowedUseArg {
    Debugging,
    Evaluation,
    BenchmarkGeneration,
    RankingModelTraining,
    ModelTraining,
    AggregateAnalytics,
}

impl TraceAllowedUseArg {
    fn server_value(self) -> &'static str {
        match self {
            Self::Debugging => "debugging",
            Self::Evaluation => "evaluation",
            Self::BenchmarkGeneration => "benchmark_generation",
            Self::RankingModelTraining => "ranking_model_training",
            Self::ModelTraining => "model_training",
            Self::AggregateAnalytics => "aggregate_analytics",
        }
    }
}

impl std::fmt::Display for TraceAllowedUseArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.server_value())
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceChannelArg {
    Web,
    Cli,
    Telegram,
    Slack,
    Routine,
    Other,
}

impl std::fmt::Display for TraceChannelArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Web => "web",
            Self::Cli => "cli",
            Self::Telegram => "telegram",
            Self::Slack => "slack",
            Self::Routine => "routine",
            Self::Other => "other",
        };
        write!(f, "{value}")
    }
}

impl From<TraceChannelArg> for TraceChannel {
    fn from(value: TraceChannelArg) -> Self {
        match value {
            TraceChannelArg::Web => TraceChannel::Web,
            TraceChannelArg::Cli => TraceChannel::Cli,
            TraceChannelArg::Telegram => TraceChannel::Telegram,
            TraceChannelArg::Slack => TraceChannel::Slack,
            TraceChannelArg::Routine => TraceChannel::Routine,
            TraceChannelArg::Other => TraceChannel::Other,
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceReviewDecisionArg {
    Approve,
    Reject,
}

impl std::fmt::Display for TraceReviewDecisionArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceCreditEventTypeArg {
    BenchmarkConversion,
    RegressionCatch,
    TrainingUtility,
    RankingUtility,
    ReviewerBonus,
    AbusePenalty,
}

impl std::fmt::Display for TraceCreditEventTypeArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::BenchmarkConversion => "benchmark_conversion",
            Self::RegressionCatch => "regression_catch",
            Self::TrainingUtility => "training_utility",
            Self::RankingUtility => "ranking_utility",
            Self::ReviewerBonus => "reviewer_bonus",
            Self::AbusePenalty => "abuse_penalty",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceBenchmarkRegistryStatusArg {
    Candidate,
    Published,
    Revoked,
}

impl std::fmt::Display for TraceBenchmarkRegistryStatusArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Candidate => "candidate",
            Self::Published => "published",
            Self::Revoked => "revoked",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceBenchmarkEvaluationStatusArg {
    NotRun,
    Queued,
    Running,
    Passed,
    Failed,
    Inconclusive,
}

impl std::fmt::Display for TraceBenchmarkEvaluationStatusArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::NotRun => "not_run",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Inconclusive => "inconclusive",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceCorpusStatusArg {
    Accepted,
    Quarantined,
    Rejected,
    Revoked,
    Expired,
    Purged,
}

impl std::fmt::Display for TraceCorpusStatusArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Accepted => "accepted",
            Self::Quarantined => "quarantined",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Purged => "purged",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracePrivacyRiskArg {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for TracePrivacyRiskArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        };
        write!(f, "{value}")
    }
}

pub async fn run_traces_command(cmd: TracesCommand) -> anyhow::Result<()> {
    match cmd {
        TracesCommand::OptIn {
            endpoint,
            bearer_token_env,
            include_message_text,
            include_tool_payloads,
            scope,
            selected_tools,
            allow_pii_review_bypass,
            min_submission_score,
        } => opt_in(OptInOptions {
            endpoint,
            bearer_token_env,
            include_message_text,
            include_tool_payloads,
            scope,
            selected_tools,
            allow_pii_review_bypass,
            min_submission_score,
        }),
        TracesCommand::OptOut => opt_out(),
        TracesCommand::Status { json } => show_policy_status(json),
        TracesCommand::Preview {
            recorded_trace,
            include_message_text,
            include_tool_payloads,
            scope,
            channel,
            engine_version,
            contributor_id,
            credit_account_ref,
            output,
            enqueue,
        } => {
            preview_recorded_trace(PreviewOptions {
                recorded_trace,
                include_message_text,
                include_tool_payloads,
                scope,
                channel,
                engine_version,
                contributor_id,
                credit_account_ref,
                output,
                enqueue,
            })
            .await
        }
        TracesCommand::Enqueue { envelope } => {
            let envelope = load_envelope(&envelope)?;
            enqueue_envelope(&envelope)?;
            println!(
                "Queued redacted trace contribution {}",
                envelope.submission_id
            );
            Ok(())
        }
        TracesCommand::FlushQueue { limit } => flush_queue(limit).await,
        TracesCommand::Credit { json } => show_credit(json).await,
        TracesCommand::Submit {
            envelope,
            endpoint,
            bearer_token_env,
        } => submit_envelope(&envelope, &endpoint, &bearer_token_env).await,
        TracesCommand::ListSubmissions { json, summary } => list_submissions(json, summary).await,
        TracesCommand::Revoke {
            submission_id,
            endpoint,
            bearer_token_env,
        } => revoke_submission(submission_id, endpoint.as_deref(), &bearer_token_env).await,
        TracesCommand::IngestHealth { endpoint, json } => {
            trace_commons_ingest_health(&endpoint, json).await
        }
        TracesCommand::QuarantineList {
            endpoint,
            bearer_token_env,
            json,
        } => trace_commons_quarantine_list(&endpoint, &bearer_token_env, json).await,
        TracesCommand::ActiveLearningReviewQueue {
            endpoint,
            limit,
            privacy_risk,
            bearer_token_env,
            json,
        } => {
            trace_commons_active_learning_review_queue(
                &endpoint,
                &bearer_token_env,
                limit,
                privacy_risk,
                json,
            )
            .await
        }
        TracesCommand::ReviewDecision {
            endpoint,
            submission_id,
            decision,
            reason,
            credit_points_pending,
            bearer_token_env,
            json,
        } => {
            trace_commons_review_decision(TraceCommonsReviewDecisionOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                submission_id,
                decision,
                reason,
                credit_points_pending,
                json,
            })
            .await
        }
        TracesCommand::AppendCreditEvent {
            endpoint,
            submission_id,
            event_type,
            credit_points_delta,
            reason,
            external_ref,
            bearer_token_env,
            json,
        } => {
            trace_commons_append_credit_event(TraceCommonsAppendCreditEventOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                submission_id,
                event_type,
                credit_points_delta,
                reason,
                external_ref,
                json,
            })
            .await
        }
        TracesCommand::AnalyticsSummary {
            endpoint,
            bearer_token_env,
            json,
        } => trace_commons_analytics_summary(&endpoint, &bearer_token_env, json).await,
        TracesCommand::MaintenanceRun {
            endpoint,
            purpose,
            dry_run,
            backfill_db_mirror,
            index_vectors,
            purge_expired_before,
            max_export_age_hours,
            skip_export_cache_prune,
            reconcile_db_mirror,
            verify_audit_chain,
            bearer_token_env,
            json,
        } => {
            let options = TraceCommonsMaintenanceOptions {
                purpose,
                dry_run,
                backfill_db_mirror,
                index_vectors,
                reconcile_db_mirror,
                verify_audit_chain,
                prune_export_cache: !skip_export_cache_prune,
                max_export_age_hours,
                purge_expired_before,
            };
            trace_commons_maintenance_run(&endpoint, &bearer_token_env, options, json).await
        }
        TracesCommand::WorkerRetentionMaintenance {
            endpoint,
            purpose,
            dry_run,
            purge_expired_before,
            max_export_age_hours,
            skip_export_cache_prune,
            bearer_token_env,
            json,
        } => {
            trace_commons_retention_maintenance_run(TraceCommonsRetentionMaintenanceOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                dry_run,
                prune_export_cache: !skip_export_cache_prune,
                max_export_age_hours,
                purge_expired_before,
                json,
            })
            .await
        }
        TracesCommand::WorkerVectorIndex {
            endpoint,
            purpose,
            dry_run,
            bearer_token_env,
            json,
        } => {
            trace_commons_vector_index_run(&endpoint, &bearer_token_env, purpose, dry_run, json)
                .await
        }
        TracesCommand::BenchmarkConvert {
            endpoint,
            purpose,
            limit,
            consent_scope,
            status,
            privacy_risk,
            external_ref,
            bearer_token_env,
            json,
        } => {
            trace_commons_benchmark_convert(TraceCommonsBenchmarkConvertOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                limit,
                consent_scope,
                status,
                privacy_risk,
                external_ref,
                json,
                path: "/v1/benchmarks/convert",
            })
            .await
        }
        TracesCommand::WorkerBenchmarkConvert {
            endpoint,
            purpose,
            limit,
            consent_scope,
            status,
            privacy_risk,
            external_ref,
            bearer_token_env,
            json,
        } => {
            trace_commons_benchmark_convert(TraceCommonsBenchmarkConvertOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                limit,
                consent_scope,
                status,
                privacy_risk,
                external_ref,
                json,
                path: "/v1/workers/benchmark-convert",
            })
            .await
        }
        TracesCommand::BenchmarkLifecycleUpdate {
            endpoint,
            conversion_id,
            registry_status,
            registry_ref,
            published_at,
            evaluation_status,
            evaluator_ref,
            evaluated_at,
            score,
            pass_count,
            fail_count,
            reason,
            bearer_token_env,
            json,
        } => {
            trace_commons_benchmark_lifecycle_update(TraceCommonsBenchmarkLifecycleUpdateOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                conversion_id,
                registry_status,
                registry_ref,
                published_at,
                evaluation_status,
                evaluator_ref,
                evaluated_at,
                score,
                pass_count,
                fail_count,
                reason,
                json,
            })
            .await
        }
        TracesCommand::ReplayDatasetExport {
            endpoint,
            purpose,
            consent_scope,
            status,
            privacy_risk,
            limit,
            output,
            bearer_token_env,
            json,
        } => {
            trace_commons_replay_dataset_export(TraceCommonsReplayDatasetExportOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                consent_scope,
                status,
                privacy_risk,
                limit,
                output,
                json,
            })
            .await
        }
        TracesCommand::ReplayExportManifests {
            endpoint,
            bearer_token_env,
            json,
        } => trace_commons_replay_export_manifests(&endpoint, &bearer_token_env, json).await,
        TracesCommand::ConfigStatus {
            endpoint,
            bearer_token_env,
        } => trace_commons_config_status(&endpoint, &bearer_token_env).await,
        TracesCommand::TenantPolicyGet {
            endpoint,
            bearer_token_env,
            json,
        } => trace_commons_tenant_policy_get(&endpoint, &bearer_token_env, json).await,
        TracesCommand::TenantPolicySet {
            endpoint,
            policy_version,
            allowed_consent_scopes,
            allowed_uses,
            bearer_token_env,
            json,
        } => {
            trace_commons_tenant_policy_set(
                &endpoint,
                &bearer_token_env,
                policy_version,
                allowed_consent_scopes,
                allowed_uses,
                json,
            )
            .await
        }
        TracesCommand::RankerTrainingCandidates {
            endpoint,
            purpose,
            consent_scope,
            status,
            privacy_risk,
            limit,
            output,
            bearer_token_env,
            json,
        } => {
            trace_commons_ranker_training_export(TraceCommonsRankerTrainingExportOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                consent_scope,
                status,
                privacy_risk,
                limit,
                output,
                json,
                path: "/v1/ranker/training-candidates",
                output_label: "ranker training candidates",
                item_field: "candidates",
            })
            .await
        }
        TracesCommand::RankerTrainingPairs {
            endpoint,
            purpose,
            consent_scope,
            status,
            privacy_risk,
            limit,
            output,
            bearer_token_env,
            json,
        } => {
            trace_commons_ranker_training_export(TraceCommonsRankerTrainingExportOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                consent_scope,
                status,
                privacy_risk,
                limit,
                output,
                json,
                path: "/v1/ranker/training-pairs",
                output_label: "ranker training pairs",
                item_field: "pairs",
            })
            .await
        }
        TracesCommand::AuditEvents {
            endpoint,
            limit,
            bearer_token_env,
            json,
        } => trace_commons_audit_events(&endpoint, &bearer_token_env, limit, json).await,
        TracesCommand::ListTraces {
            endpoint,
            purpose,
            consent_scope,
            status,
            limit,
            coverage_tag,
            tool,
            privacy_risk,
            bearer_token_env,
            json,
        } => {
            trace_commons_list_traces(TraceCommonsListTracesOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                consent_scope,
                status,
                limit,
                coverage_tag,
                tool,
                privacy_risk,
                json,
            })
            .await
        }
        TracesCommand::PrivacyFilterCanary {
            text,
            timeout_seconds,
            json,
        } => privacy_filter_canary(&text, timeout_seconds, json).await,
    }
}

struct PreviewOptions {
    recorded_trace: PathBuf,
    include_message_text: bool,
    include_tool_payloads: bool,
    scope: TraceScopeArg,
    channel: TraceChannelArg,
    engine_version: Option<String>,
    contributor_id: Option<String>,
    credit_account_ref: Option<String>,
    output: Option<PathBuf>,
    enqueue: bool,
}

struct OptInOptions {
    endpoint: String,
    bearer_token_env: String,
    include_message_text: bool,
    include_tool_payloads: bool,
    scope: TraceScopeArg,
    selected_tools: Vec<String>,
    allow_pii_review_bypass: bool,
    min_submission_score: f32,
}

fn opt_in(options: OptInOptions) -> anyhow::Result<()> {
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        ingestion_endpoint: Some(options.endpoint),
        bearer_token_env: options.bearer_token_env,
        include_message_text: options.include_message_text,
        include_tool_payloads: options.include_tool_payloads,
        selected_tools: options
            .selected_tools
            .into_iter()
            .filter(|tool| !tool.trim().is_empty())
            .collect::<BTreeSet<_>>(),
        require_manual_approval_when_pii_detected: !options.allow_pii_review_bypass,
        min_submission_score: options.min_submission_score.clamp(0.0, 1.0),
        default_scope: options.scope.into(),
        ..StandingTraceContributionPolicy::default()
    };

    write_policy(&policy)?;
    println!("Trace contribution opt-in enabled.");
    println!("Autonomous submissions will use local redaction and the configured endpoint.");
    Ok(())
}

fn opt_out() -> anyhow::Result<()> {
    let mut policy = read_policy()?;
    policy.enabled = false;
    write_policy(&policy)?;
    println!("Trace contribution opt-in disabled. Queued envelopes remain local.");
    Ok(())
}

fn show_policy_status(json: bool) -> anyhow::Result<()> {
    let policy = read_policy()?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&policy)
                .map_err(|e| anyhow::anyhow!("failed to serialize trace policy: {}", e))?
        );
        return Ok(());
    }

    println!("Trace contribution policy:");
    println!("  enabled: {}", policy.enabled);
    println!(
        "  endpoint: {}",
        policy
            .ingestion_endpoint
            .as_deref()
            .unwrap_or("not configured")
    );
    println!("  bearer token env: {}", policy.bearer_token_env);
    println!("  include message text: {}", policy.include_message_text);
    println!("  include tool payloads: {}", policy.include_tool_payloads);
    println!(
        "  manual review when PII detected: {}",
        policy.require_manual_approval_when_pii_detected
    );
    println!("  min submission score: {:.2}", policy.min_submission_score);
    println!(
        "  credit notice interval: {} hour(s)",
        policy.credit_notice_interval_hours
    );
    println!("  queued envelopes: {}", queued_envelope_paths()?.len());
    Ok(())
}

async fn preview_recorded_trace(options: PreviewOptions) -> anyhow::Result<()> {
    let raw_json = std::fs::read_to_string(&options.recorded_trace).map_err(|e| {
        anyhow::anyhow!(
            "failed to read recorded trace {}: {}",
            options.recorded_trace.display(),
            e
        )
    })?;
    let recorded_trace: crate::llm::recording::TraceFile = serde_json::from_str(&raw_json)
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to parse recorded trace {}: {}",
                options.recorded_trace.display(),
                e
            )
        })?;

    let raw_contribution = crate::trace_contribution::RawTraceContribution::from_recorded_trace(
        &recorded_trace,
        RecordedTraceContributionOptions {
            include_message_text: options.include_message_text,
            include_tool_payloads: options.include_tool_payloads,
            consent_scopes: vec![options.scope.into()],
            channel: options.channel.into(),
            engine_version: options.engine_version,
            feature_flags: BTreeMap::new(),
            pseudonymous_contributor_id: options.contributor_id,
            tenant_scope_ref: None,
            credit_account_ref: options.credit_account_ref,
        },
    );

    let redactor = DeterministicTraceRedactor::default();
    let mut envelope = redactor.redact_trace(raw_contribution).await?;
    apply_credit_estimate(&mut envelope);
    let envelope_json = serde_json::to_string_pretty(&envelope)
        .map_err(|e| anyhow::anyhow!("failed to serialize contribution envelope: {}", e))?;

    if let Some(output) = options.output {
        std::fs::write(&output, envelope_json)
            .map_err(|e| anyhow::anyhow!("failed to write envelope {}: {}", output.display(), e))?;
        println!(
            "Wrote redacted trace contribution preview to {}",
            output.display()
        );
        println!(
            "Redaction summary: {}",
            redaction_summary(&envelope.privacy.redaction_counts)
        );
    } else {
        println!("{envelope_json}");
    }

    if options.enqueue {
        enqueue_envelope(&envelope)?;
        println!(
            "Queued redacted trace contribution {} for autonomous submission.",
            envelope.submission_id
        );
    }

    Ok(())
}

async fn submit_envelope(
    envelope_path: &Path,
    endpoint: &str,
    bearer_token_env: &str,
) -> anyhow::Result<()> {
    let mut envelope = load_envelope(envelope_path)?;
    apply_credit_estimate(&mut envelope);
    let receipt = submit_envelope_to_endpoint(&envelope, endpoint, bearer_token_env).await?;

    record_submitted_envelope(&envelope, endpoint, receipt)?;

    println!(
        "Submitted redacted trace contribution {}",
        envelope.submission_id
    );
    Ok(())
}

async fn submit_envelope_to_endpoint(
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

    let client = reqwest::Client::new();
    let response = client
        .post(endpoint)
        .bearer_auth(token)
        .header("Idempotency-Key", envelope.submission_id.to_string())
        .json(&envelope)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("trace submission request failed: {}", e))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("trace submission rejected by {}: {}", status, body);
    }

    Ok(
        parse_submission_receipt(&body).unwrap_or_else(|| TraceSubmissionReceipt {
            status: "submitted".to_string(),
            credit_points_pending: Some(envelope.value.credit_points_pending),
            credit_points_final: None,
            explanation: envelope.value.explanation.clone(),
        }),
    )
}

fn record_submitted_envelope(
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

    upsert_local_record(LocalSubmissionRecord {
        submission_id: envelope.submission_id,
        trace_id: envelope.trace_id,
        endpoint: Some(endpoint.to_string()),
        status: LocalSubmissionStatus::Submitted,
        server_status: Some(receipt.status),
        submitted_at: Some(chrono::Utc::now()),
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
            created_at: chrono::Utc::now(),
        }],
        last_credit_notice_at: None,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct TraceSubmissionReceipt {
    #[serde(default = "default_submitted_status")]
    status: String,
    credit_points_pending: Option<f32>,
    credit_points_final: Option<f32>,
    #[serde(default)]
    explanation: Vec<String>,
}

fn default_submitted_status() -> String {
    "submitted".to_string()
}

fn parse_submission_receipt(body: &str) -> Option<TraceSubmissionReceipt> {
    if body.trim().is_empty() {
        return None;
    }
    serde_json::from_str(body).ok()
}

async fn revoke_submission(
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

        let client = reqwest::Client::new();
        let response = client
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

    mark_local_revoked(submission_id)?;
    println!("Marked trace contribution {submission_id} revoked.");
    Ok(())
}

async fn trace_commons_ingest_health(endpoint: &str, json: bool) -> anyhow::Result<()> {
    let response =
        trace_commons_api_request(Method::GET, endpoint, "/health", &[], None, None).await?;
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    let Some(value) = response.json.as_ref() else {
        println!("Trace Commons ingest health: {}", response.body.trim());
        return Ok(());
    };
    println!("Trace Commons ingest health:");
    println!("  endpoint: {}", response.url);
    println!(
        "  status: {}",
        value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
    );
    if let Some(schema_version) = value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
    {
        println!("  schema version: {schema_version}");
    }
    Ok(())
}

async fn trace_commons_quarantine_list(
    endpoint: &str,
    bearer_token_env: &str,
    json: bool,
) -> anyhow::Result<()> {
    let response = trace_commons_api_request(
        Method::GET,
        endpoint,
        "/v1/review/quarantine",
        &[],
        Some(bearer_token_env),
        None,
    )
    .await?;
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }
    print_trace_commons_items(
        "Central quarantined traces",
        response.json.as_ref(),
        &[
            "submission_id",
            "privacy_risk",
            "submission_score",
            "received_at",
            "canonical_summary",
        ],
    );
    Ok(())
}

async fn trace_commons_active_learning_review_queue(
    endpoint: &str,
    bearer_token_env: &str,
    limit: Option<usize>,
    privacy_risk: Option<TracePrivacyRiskArg>,
    json: bool,
) -> anyhow::Result<()> {
    let mut query = optional_usize_query("limit", limit);
    if let Some(privacy_risk) = privacy_risk {
        query.push(("privacy_risk", privacy_risk.to_string()));
    }
    let Some(response) = trace_commons_optional_api_request(
        Method::GET,
        endpoint,
        "/v1/review/active-learning",
        &query,
        Some(bearer_token_env),
        None,
    )
    .await?
    else {
        print_trace_commons_unsupported("/v1/review/active-learning");
        return Ok(());
    };
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }
    print_trace_commons_items(
        "Central active-learning review queue",
        response.json.as_ref().and_then(|value| value.get("items")),
        &[
            "submission_id",
            "status",
            "privacy_risk",
            "priority_score",
            "priority_reasons",
            "received_at",
        ],
    );
    Ok(())
}

struct TraceCommonsReviewDecisionOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    submission_id: Uuid,
    decision: TraceReviewDecisionArg,
    reason: Option<String>,
    credit_points_pending: Option<f32>,
    json: bool,
}

async fn trace_commons_review_decision(
    options: TraceCommonsReviewDecisionOptions<'_>,
) -> anyhow::Result<()> {
    let mut body = serde_json::json!({
        "decision": options.decision.to_string(),
    });
    if let Some(reason) = options.reason {
        body["reason"] = serde_json::Value::String(reason);
    }
    if let Some(credit_points_pending) = options.credit_points_pending {
        body["credit_points_pending"] = serde_json::json!(credit_points_pending);
    }

    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        &format!("/v1/review/{}/decision", options.submission_id),
        &[],
        Some(options.bearer_token_env),
        Some(body),
    )
    .await?;
    if options.json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    println!(
        "Recorded central review decision for {}",
        options.submission_id
    );
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  status", value, "status");
        print_optional_json_field("  pending credit", value, "credit_points_pending");
        print_optional_json_field("  final credit", value, "credit_points_final");
    }
    Ok(())
}

struct TraceCommonsAppendCreditEventOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    submission_id: Uuid,
    event_type: TraceCreditEventTypeArg,
    credit_points_delta: f32,
    reason: String,
    external_ref: Option<String>,
    json: bool,
}

async fn trace_commons_append_credit_event(
    options: TraceCommonsAppendCreditEventOptions<'_>,
) -> anyhow::Result<()> {
    let mut body = serde_json::json!({
        "event_type": options.event_type.to_string(),
        "credit_points_delta": options.credit_points_delta,
        "reason": options.reason,
    });
    if let Some(external_ref) = options.external_ref {
        body["external_ref"] = serde_json::Value::String(external_ref);
    }

    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        &format!("/v1/review/{}/credit-events", options.submission_id),
        &[],
        Some(options.bearer_token_env),
        Some(body),
    )
    .await?;
    if options.json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    println!(
        "Appended central delayed credit event for {}",
        options.submission_id
    );
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  event id", value, "event_id");
        print_optional_json_field("  event type", value, "event_type");
        print_optional_json_field("  delta", value, "credit_points_delta");
        print_optional_json_field("  reason", value, "reason");
    }
    Ok(())
}

async fn trace_commons_analytics_summary(
    endpoint: &str,
    bearer_token_env: &str,
    json: bool,
) -> anyhow::Result<()> {
    let response = trace_commons_api_request(
        Method::GET,
        endpoint,
        "/v1/analytics/summary",
        &[],
        Some(bearer_token_env),
        None,
    )
    .await?;
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    let Some(value) = response.json.as_ref() else {
        println!("{}", response.body.trim());
        return Ok(());
    };
    println!("Trace Commons analytics summary:");
    print_optional_json_field("  tenant", value, "tenant_id");
    print_optional_json_field("  submissions", value, "submissions_total");
    print_optional_json_field("  duplicate groups", value, "duplicate_groups");
    print_optional_json_field("  average novelty", value, "average_novelty_score");
    print_json_map("  by status", value.get("by_status"));
    print_json_map("  by privacy risk", value.get("by_privacy_risk"));
    print_json_map("  by task success", value.get("by_task_success"));
    Ok(())
}

async fn trace_commons_maintenance_run(
    endpoint: &str,
    bearer_token_env: &str,
    options: TraceCommonsMaintenanceOptions,
    json: bool,
) -> anyhow::Result<()> {
    require_non_empty_purpose(&options.purpose)?;
    let body = trace_commons_maintenance_body(&options);
    let response = trace_commons_api_request(
        Method::POST,
        endpoint,
        "/v1/admin/maintenance",
        &[],
        Some(bearer_token_env),
        Some(body),
    )
    .await?;
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    println!("Trace Commons maintenance run requested.");
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  audit event id", value, "audit_event_id");
        print_optional_json_field("  purpose", value, "purpose");
        print_optional_json_field("  dry run", value, "dry_run");
        print_optional_json_field("  revoked submissions", value, "revoked_submission_count");
        print_optional_json_field("  expired submissions", value, "expired_submission_count");
        print_optional_json_field("  records marked revoked", value, "records_marked_revoked");
        print_optional_json_field("  records marked expired", value, "records_marked_expired");
        print_optional_json_field("  records marked purged", value, "records_marked_purged");
        print_optional_json_field("  derived marked revoked", value, "derived_marked_revoked");
        print_optional_json_field("  derived marked expired", value, "derived_marked_expired");
        print_optional_json_field(
            "  export cache files pruned",
            value,
            "export_cache_files_pruned",
        );
        print_optional_json_field(
            "  export provenance invalidated",
            value,
            "export_provenance_invalidated",
        );
        print_optional_json_field(
            "  trace object files deleted",
            value,
            "trace_object_files_deleted",
        );
        print_optional_json_field(
            "  encrypted artifacts deleted",
            value,
            "encrypted_artifacts_deleted",
        );
        print_optional_json_field("  DB mirror backfilled", value, "db_mirror_backfilled");
        print_optional_json_field("  vectors indexed", value, "vector_entries_indexed");
        for line in maintenance_audit_chain_lines(value) {
            println!("{line}");
        }
        for line in maintenance_reconciliation_lines(value) {
            println!("{line}");
        }
    }
    Ok(())
}

struct TraceCommonsMaintenanceOptions {
    purpose: String,
    dry_run: bool,
    backfill_db_mirror: bool,
    index_vectors: bool,
    reconcile_db_mirror: bool,
    verify_audit_chain: bool,
    prune_export_cache: bool,
    max_export_age_hours: Option<i64>,
    purge_expired_before: Option<String>,
}

fn trace_commons_maintenance_body(options: &TraceCommonsMaintenanceOptions) -> serde_json::Value {
    let mut body = serde_json::json!({
        "purpose": &options.purpose,
        "dry_run": options.dry_run,
    });
    if options.backfill_db_mirror {
        body["backfill_db_mirror"] = serde_json::Value::Bool(true);
    }
    if options.index_vectors {
        body["index_vectors"] = serde_json::Value::Bool(true);
    }
    if options.reconcile_db_mirror {
        body["reconcile_db_mirror"] = serde_json::Value::Bool(true);
    }
    if options.verify_audit_chain {
        body["verify_audit_chain"] = serde_json::Value::Bool(true);
    }
    if !options.prune_export_cache {
        body["prune_export_cache"] = serde_json::Value::Bool(false);
    }
    if let Some(max_export_age_hours) = options.max_export_age_hours {
        body["max_export_age_hours"] = serde_json::Value::Number(max_export_age_hours.into());
    }
    if let Some(purge_expired_before) = options.purge_expired_before.as_ref() {
        body["purge_expired_before"] = serde_json::Value::String(purge_expired_before.clone());
    }
    body
}

struct TraceCommonsRetentionMaintenanceOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    purpose: Option<String>,
    dry_run: bool,
    prune_export_cache: bool,
    max_export_age_hours: Option<i64>,
    purge_expired_before: Option<String>,
    json: bool,
}

async fn trace_commons_retention_maintenance_run(
    options: TraceCommonsRetentionMaintenanceOptions<'_>,
) -> anyhow::Result<()> {
    if let Some(purpose) = options.purpose.as_deref() {
        require_non_empty_purpose(purpose)?;
    }
    let body = trace_commons_retention_maintenance_body(
        options.purpose,
        options.dry_run,
        options.prune_export_cache,
        options.max_export_age_hours,
        options.purge_expired_before,
    );
    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        "/v1/workers/retention-maintenance",
        &[],
        Some(options.bearer_token_env),
        Some(body),
    )
    .await?;
    if options.json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }
    println!("Trace Commons retention maintenance requested.");
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  audit event id", value, "audit_event_id");
        print_optional_json_field("  purpose", value, "purpose");
        print_optional_json_field("  dry run", value, "dry_run");
        print_optional_json_field("  records marked revoked", value, "records_marked_revoked");
        print_optional_json_field("  records marked expired", value, "records_marked_expired");
        print_optional_json_field("  records marked purged", value, "records_marked_purged");
        print_optional_json_field(
            "  export cache files pruned",
            value,
            "export_cache_files_pruned",
        );
        print_optional_json_field(
            "  export provenance invalidated",
            value,
            "export_provenance_invalidated",
        );
    }
    Ok(())
}

fn trace_commons_retention_maintenance_body(
    purpose: Option<String>,
    dry_run: bool,
    prune_export_cache: bool,
    max_export_age_hours: Option<i64>,
    purge_expired_before: Option<String>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "dry_run": dry_run,
    });
    if let Some(purpose) = purpose {
        body["purpose"] = serde_json::Value::String(purpose);
    }
    if !prune_export_cache {
        body["prune_export_cache"] = serde_json::Value::Bool(false);
    }
    if let Some(max_export_age_hours) = max_export_age_hours {
        body["max_export_age_hours"] = serde_json::Value::Number(max_export_age_hours.into());
    }
    if let Some(purge_expired_before) = purge_expired_before {
        body["purge_expired_before"] = serde_json::Value::String(purge_expired_before);
    }
    body
}

async fn trace_commons_vector_index_run(
    endpoint: &str,
    bearer_token_env: &str,
    purpose: Option<String>,
    dry_run: bool,
    json: bool,
) -> anyhow::Result<()> {
    if let Some(purpose) = purpose.as_deref() {
        require_non_empty_purpose(purpose)?;
    }
    let body = trace_commons_vector_index_body(purpose, dry_run);
    let response = trace_commons_api_request(
        Method::POST,
        endpoint,
        "/v1/workers/vector-index",
        &[],
        Some(bearer_token_env),
        Some(body),
    )
    .await?;
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }
    println!("Trace Commons vector index requested.");
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  audit event id", value, "audit_event_id");
        print_optional_json_field("  purpose", value, "purpose");
        print_optional_json_field("  dry run", value, "dry_run");
        print_optional_json_field("  vectors indexed", value, "vector_entries_indexed");
    }
    Ok(())
}

fn trace_commons_vector_index_body(purpose: Option<String>, dry_run: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "dry_run": dry_run,
    });
    if let Some(purpose) = purpose {
        body["purpose"] = serde_json::Value::String(purpose);
    }
    body
}

struct TraceCommonsBenchmarkConvertOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    purpose: String,
    limit: Option<usize>,
    consent_scope: Option<TraceScopeArg>,
    status: Option<TraceCorpusStatusArg>,
    privacy_risk: Option<TracePrivacyRiskArg>,
    external_ref: Option<String>,
    json: bool,
    path: &'a str,
}

async fn trace_commons_benchmark_convert(
    options: TraceCommonsBenchmarkConvertOptions<'_>,
) -> anyhow::Result<()> {
    require_non_empty_purpose(&options.purpose)?;
    let mut body = serde_json::json!({
        "purpose": options.purpose,
    });
    if let Some(limit) = options.limit {
        body["limit"] = serde_json::json!(limit);
    }
    if let Some(consent_scope) = options.consent_scope {
        body["consent_scope"] = serde_json::Value::String(consent_scope.to_string());
    }
    if let Some(status) = options.status {
        body["status"] = serde_json::Value::String(status.to_string());
    }
    if let Some(privacy_risk) = options.privacy_risk {
        body["privacy_risk"] = serde_json::Value::String(privacy_risk.to_string());
    }
    if let Some(external_ref) = options.external_ref {
        body["external_ref"] = serde_json::Value::String(external_ref);
    }

    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        options.path,
        &[],
        Some(options.bearer_token_env),
        Some(body),
    )
    .await?;
    if options.json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    println!("Trace Commons benchmark conversion requested.");
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  conversion id", value, "conversion_id");
        print_optional_json_field("  audit event id", value, "audit_event_id");
        print_optional_json_field("  item count", value, "item_count");
        print_optional_json_field("  purpose", value, "purpose");
    }
    Ok(())
}

struct TraceCommonsBenchmarkLifecycleUpdateOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    conversion_id: Uuid,
    registry_status: Option<TraceBenchmarkRegistryStatusArg>,
    registry_ref: Option<String>,
    published_at: Option<String>,
    evaluation_status: Option<TraceBenchmarkEvaluationStatusArg>,
    evaluator_ref: Option<String>,
    evaluated_at: Option<String>,
    score: Option<f32>,
    pass_count: Option<u32>,
    fail_count: Option<u32>,
    reason: Option<String>,
    json: bool,
}

async fn trace_commons_benchmark_lifecycle_update(
    options: TraceCommonsBenchmarkLifecycleUpdateOptions<'_>,
) -> anyhow::Result<()> {
    let body = trace_commons_benchmark_lifecycle_body(
        options.registry_status,
        options.registry_ref,
        options.published_at,
        options.evaluation_status,
        options.evaluator_ref,
        options.evaluated_at,
        options.score,
        options.pass_count,
        options.fail_count,
        options.reason,
    )?;
    let path = format!("/v1/benchmarks/{}/lifecycle", options.conversion_id);
    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        &path,
        &[],
        Some(options.bearer_token_env),
        Some(body),
    )
    .await?;
    if options.json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    println!("Trace Commons benchmark lifecycle updated.");
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  conversion id", value, "conversion_id");
        print_optional_json_field("  purpose", value, "purpose");
        if let Some(registry) = value.get("registry") {
            print_optional_json_field("  registry status", registry, "status");
            print_optional_json_field("  registry ref", registry, "registry_ref");
        }
        if let Some(evaluation) = value.get("evaluation") {
            print_optional_json_field("  evaluation status", evaluation, "status");
            print_optional_json_field("  evaluator ref", evaluation, "evaluator_ref");
            print_optional_json_field("  score", evaluation, "score");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn trace_commons_benchmark_lifecycle_body(
    registry_status: Option<TraceBenchmarkRegistryStatusArg>,
    registry_ref: Option<String>,
    published_at: Option<String>,
    evaluation_status: Option<TraceBenchmarkEvaluationStatusArg>,
    evaluator_ref: Option<String>,
    evaluated_at: Option<String>,
    score: Option<f32>,
    pass_count: Option<u32>,
    fail_count: Option<u32>,
    reason: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    if let Some(score) = score
        && !(0.0..=1.0).contains(&score)
    {
        anyhow::bail!("--score must be between 0.0 and 1.0");
    }

    let mut body = serde_json::json!({});
    let mut registry = serde_json::Map::new();
    if let Some(status) = registry_status {
        registry.insert(
            "status".to_string(),
            serde_json::Value::String(status.to_string()),
        );
    }
    if let Some(registry_ref) = registry_ref {
        registry.insert(
            "registry_ref".to_string(),
            serde_json::Value::String(registry_ref),
        );
    }
    if let Some(published_at) = published_at {
        registry.insert(
            "published_at".to_string(),
            serde_json::Value::String(published_at),
        );
    }
    if !registry.is_empty() {
        body["registry"] = serde_json::Value::Object(registry);
    }

    let mut evaluation = serde_json::Map::new();
    if let Some(status) = evaluation_status {
        evaluation.insert(
            "status".to_string(),
            serde_json::Value::String(status.to_string()),
        );
    }
    if let Some(evaluator_ref) = evaluator_ref {
        evaluation.insert(
            "evaluator_ref".to_string(),
            serde_json::Value::String(evaluator_ref),
        );
    }
    if let Some(evaluated_at) = evaluated_at {
        evaluation.insert(
            "evaluated_at".to_string(),
            serde_json::Value::String(evaluated_at),
        );
    }
    if let Some(score) = score {
        evaluation.insert("score".to_string(), serde_json::json!(score));
    }
    if let Some(pass_count) = pass_count {
        evaluation.insert("pass_count".to_string(), serde_json::json!(pass_count));
    }
    if let Some(fail_count) = fail_count {
        evaluation.insert("fail_count".to_string(), serde_json::json!(fail_count));
    }
    if !evaluation.is_empty() {
        body["evaluation"] = serde_json::Value::Object(evaluation);
    }

    if body.as_object().is_some_and(serde_json::Map::is_empty) {
        anyhow::bail!(
            "benchmark lifecycle update requires at least one registry or evaluation field"
        );
    }
    if let Some(reason) = reason {
        body["reason"] = serde_json::Value::String(reason);
    }
    Ok(body)
}

struct TraceCommonsReplayDatasetExportOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    purpose: Option<String>,
    consent_scope: Option<TraceScopeArg>,
    status: Option<TraceCorpusStatusArg>,
    privacy_risk: Option<TracePrivacyRiskArg>,
    limit: Option<usize>,
    output: Option<PathBuf>,
    json: bool,
}

async fn trace_commons_replay_dataset_export(
    options: TraceCommonsReplayDatasetExportOptions<'_>,
) -> anyhow::Result<()> {
    let mut query = Vec::new();
    if let Some(limit) = options.limit {
        query.push(("limit", limit.to_string()));
    }
    if let Some(purpose) = options.purpose {
        require_non_empty_purpose(&purpose)?;
        query.push(("purpose", purpose));
    }
    if let Some(consent_scope) = options.consent_scope {
        query.push(("consent_scope", consent_scope.to_string()));
    }
    if let Some(status) = options.status {
        query.push(("status", status.to_string()));
    }
    if let Some(privacy_risk) = options.privacy_risk {
        query.push(("privacy_risk", privacy_risk.to_string()));
    }

    let response = trace_commons_api_request(
        Method::GET,
        options.endpoint,
        "/v1/datasets/replay",
        &query,
        Some(options.bearer_token_env),
        None,
    )
    .await?;

    if let Some(output) = options.output {
        let body = pretty_trace_commons_body(&response)?;
        std::fs::write(&output, body).map_err(|e| {
            anyhow::anyhow!(
                "failed to write replay dataset export {}: {}",
                output.display(),
                e
            )
        })?;
        if options.json {
            print_trace_commons_json(&response)?;
        } else {
            println!(
                "Wrote central replay dataset export to {}",
                output.display()
            );
            if let Some(value) = response.json.as_ref() {
                print_optional_json_field("  export id", value, "export_id");
                print_optional_json_field("  manifest id", value, "manifest_id");
                print_optional_json_field("  audit event id", value, "audit_event_id");
                print_optional_json_field("  item count", value, "item_count");
            }
        }
        return Ok(());
    }

    print_trace_commons_json(&response)
}

async fn trace_commons_replay_export_manifests(
    endpoint: &str,
    bearer_token_env: &str,
    json: bool,
) -> anyhow::Result<()> {
    let Some(response) = trace_commons_optional_api_request(
        Method::GET,
        endpoint,
        "/v1/datasets/replay/manifests",
        &[],
        Some(bearer_token_env),
        None,
    )
    .await?
    else {
        print_trace_commons_unsupported("/v1/datasets/replay/manifests");
        return Ok(());
    };

    if json {
        print_trace_commons_json(&response)
    } else {
        print_trace_commons_items(
            "Central replay export manifests",
            response.json.as_ref(),
            &[
                "export_manifest_id",
                "purpose_code",
                "item_count",
                "source_submission_ids_hash",
                "generated_at",
                "invalidated_at",
            ],
        );
        Ok(())
    }
}

async fn trace_commons_tenant_policy_get(
    endpoint: &str,
    bearer_token_env: &str,
    json: bool,
) -> anyhow::Result<()> {
    let response = trace_commons_api_request(
        Method::GET,
        endpoint,
        "/v1/admin/tenant-policy",
        &[],
        Some(bearer_token_env),
        None,
    )
    .await?;
    print_trace_commons_tenant_policy_response(response, json)
}

async fn trace_commons_config_status(endpoint: &str, bearer_token_env: &str) -> anyhow::Result<()> {
    let response = trace_commons_api_request(
        Method::GET,
        endpoint,
        "/v1/admin/config-status",
        &[],
        Some(bearer_token_env),
        None,
    )
    .await?;
    print_trace_commons_json(&response)
}

async fn trace_commons_tenant_policy_set(
    endpoint: &str,
    bearer_token_env: &str,
    policy_version: String,
    allowed_consent_scopes: Vec<TraceScopeArg>,
    allowed_uses: Vec<TraceAllowedUseArg>,
    json: bool,
) -> anyhow::Result<()> {
    require_non_empty_policy_version(&policy_version)?;
    let body =
        trace_commons_tenant_policy_body(policy_version, allowed_consent_scopes, allowed_uses);
    let response = trace_commons_api_request(
        Method::PUT,
        endpoint,
        "/v1/admin/tenant-policy",
        &[],
        Some(bearer_token_env),
        Some(body),
    )
    .await?;
    print_trace_commons_tenant_policy_response(response, json)
}

fn trace_commons_tenant_policy_body(
    policy_version: String,
    allowed_consent_scopes: Vec<TraceScopeArg>,
    allowed_uses: Vec<TraceAllowedUseArg>,
) -> serde_json::Value {
    serde_json::json!({
        "policy_version": policy_version,
        "allowed_consent_scopes": allowed_consent_scopes
            .into_iter()
            .map(trace_scope_server_value)
            .collect::<Vec<_>>(),
        "allowed_uses": allowed_uses
            .into_iter()
            .map(TraceAllowedUseArg::server_value)
            .collect::<Vec<_>>(),
    })
}

fn print_trace_commons_tenant_policy_response(
    response: TraceCommonsApiResponse,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }
    let Some(value) = response.json.as_ref() else {
        println!("{}", response.body.trim());
        return Ok(());
    };
    println!("Trace Commons tenant policy:");
    print_optional_json_field("  tenant", value, "tenant_id");
    print_optional_json_field("  policy version", value, "policy_version");
    print_optional_json_field("  allowed consent scopes", value, "allowed_consent_scopes");
    print_optional_json_field("  allowed uses", value, "allowed_uses");
    print_optional_json_field("  updated by", value, "updated_by_principal_ref");
    print_optional_json_field("  updated at", value, "updated_at");
    Ok(())
}

struct TraceCommonsRankerTrainingExportOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    purpose: Option<String>,
    consent_scope: Option<TraceScopeArg>,
    status: Option<TraceCorpusStatusArg>,
    privacy_risk: Option<TracePrivacyRiskArg>,
    limit: Option<usize>,
    output: Option<PathBuf>,
    json: bool,
    path: &'a str,
    output_label: &'a str,
    item_field: &'a str,
}

async fn trace_commons_ranker_training_export(
    options: TraceCommonsRankerTrainingExportOptions<'_>,
) -> anyhow::Result<()> {
    let mut query = Vec::new();
    if let Some(limit) = options.limit {
        query.push(("limit", limit.to_string()));
    }
    if let Some(purpose) = options.purpose {
        require_non_empty_purpose(&purpose)?;
        query.push(("purpose", purpose));
    }
    if let Some(consent_scope) = options.consent_scope {
        query.push(("consent_scope", consent_scope.to_string()));
    }
    if let Some(status) = options.status {
        query.push(("status", status.to_string()));
    }
    if let Some(privacy_risk) = options.privacy_risk {
        query.push(("privacy_risk", privacy_risk.to_string()));
    }

    let Some(response) = trace_commons_optional_api_request(
        Method::GET,
        options.endpoint,
        options.path,
        &query,
        Some(options.bearer_token_env),
        None,
    )
    .await?
    else {
        print_trace_commons_unsupported(options.path);
        return Ok(());
    };

    if let Some(output) = options.output {
        let body = pretty_trace_commons_body(&response)?;
        std::fs::write(&output, body).map_err(|e| {
            anyhow::anyhow!(
                "failed to write ranker training export {}: {}",
                output.display(),
                e
            )
        })?;
        if options.json {
            print_trace_commons_json(&response)?;
        } else {
            println!(
                "Wrote central {} export to {}",
                options.output_label,
                output.display()
            );
            if let Some(value) = response.json.as_ref() {
                print_optional_json_field("  export id", value, "export_id");
                print_optional_json_field("  audit event id", value, "audit_event_id");
                print_optional_json_field("  purpose", value, "purpose");
                print_optional_json_field("  item count", value, "item_count");
            }
        }
        return Ok(());
    }

    if options.json {
        print_trace_commons_json(&response)
    } else {
        print_trace_commons_items(
            &format!("Central {}", options.output_label),
            response
                .json
                .as_ref()
                .and_then(|value| value.get(options.item_field)),
            &[
                "submission_id",
                "trace_id",
                "ranker_score",
                "preferred_submission_id",
                "rejected_submission_id",
                "reason",
            ],
        );
        Ok(())
    }
}

async fn trace_commons_audit_events(
    endpoint: &str,
    bearer_token_env: &str,
    limit: Option<usize>,
    json: bool,
) -> anyhow::Result<()> {
    let query = optional_usize_query("limit", limit);
    let response = trace_commons_api_request(
        Method::GET,
        endpoint,
        "/v1/audit/events",
        &query,
        Some(bearer_token_env),
        None,
    )
    .await?;
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }
    print_trace_commons_items(
        "Central audit events",
        response.json.as_ref(),
        &["created_at", "kind", "submission_id", "status", "reason"],
    );
    Ok(())
}

struct TraceCommonsListTracesOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    purpose: Option<String>,
    consent_scope: Option<TraceScopeArg>,
    status: Option<TraceCorpusStatusArg>,
    limit: Option<usize>,
    coverage_tag: Option<String>,
    tool: Option<String>,
    privacy_risk: Option<TracePrivacyRiskArg>,
    json: bool,
}

async fn trace_commons_list_traces(
    options: TraceCommonsListTracesOptions<'_>,
) -> anyhow::Result<()> {
    let mut query = Vec::new();
    if let Some(purpose) = options.purpose {
        require_non_empty_purpose(&purpose)?;
        query.push(("purpose", purpose));
    }
    if let Some(consent_scope) = options.consent_scope {
        query.push(("consent_scope", consent_scope.to_string()));
    }
    if let Some(status) = options.status {
        query.push(("status", status.to_string()));
    }
    if let Some(limit) = options.limit {
        query.push(("limit", limit.to_string()));
    }
    if let Some(coverage_tag) = options.coverage_tag {
        query.push(("coverage_tag", coverage_tag));
    }
    if let Some(tool) = options.tool {
        query.push(("tool", tool));
    }
    if let Some(privacy_risk) = options.privacy_risk {
        query.push(("privacy_risk", privacy_risk.to_string()));
    }

    let response = trace_commons_api_request(
        Method::GET,
        options.endpoint,
        "/v1/traces",
        &query,
        Some(options.bearer_token_env),
        None,
    )
    .await?;
    if options.json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }
    print_trace_commons_items(
        "Central traces",
        response.json.as_ref(),
        &[
            "submission_id",
            "status",
            "privacy_risk",
            "submission_score",
            "credit_points_pending",
            "received_at",
        ],
    );
    Ok(())
}

async fn privacy_filter_canary(text: &str, timeout_seconds: u64, json: bool) -> anyhow::Result<()> {
    let adapter = privacy_filter_adapter_from_env().ok_or_else(|| {
        anyhow::anyhow!(
            "IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND is not set; no local privacy filter sidecar is configured"
        )
    })?;
    let timeout = Duration::from_secs(timeout_seconds.max(1));
    let redaction = tokio::time::timeout(timeout, adapter.redact_text(text))
        .await
        .map_err(|_| anyhow::anyhow!("privacy filter canary timed out after {:?}", timeout))?
        .map_err(|e| anyhow::anyhow!("privacy filter canary failed: {}", e))?;

    let Some(redaction) = redaction else {
        anyhow::bail!("privacy filter sidecar returned no redaction for canary text");
    };
    let leaked_tokens = canary_leaked_tokens(text, &redaction.redacted_text);
    let passed = leaked_tokens.is_empty();
    let report = serde_json::json!({
        "passed": passed,
        "span_count": redaction.summary.span_count,
        "by_label": redaction.summary.by_label,
        "decoded_mismatch": redaction.summary.decoded_mismatch,
        "leaked_tokens": leaked_tokens,
    });
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| anyhow::anyhow!("failed to serialize canary report: {}", e))?
        );
    } else {
        println!("Privacy Filter sidecar canary:");
        println!("  passed: {passed}");
        println!("  spans: {}", redaction.summary.span_count);
        print_json_map("  by label", report.get("by_label"));
        if !leaked_tokens.is_empty() {
            println!("  leaked canary tokens: {}", leaked_tokens.join(", "));
        }
    }
    if !passed {
        anyhow::bail!("privacy filter canary failed: sidecar output retained canary token(s)");
    }
    Ok(())
}

struct TraceCommonsApiResponse {
    url: String,
    body: String,
    json: Option<serde_json::Value>,
}

async fn trace_commons_api_request(
    method: Method,
    endpoint: &str,
    path: &str,
    query: &[(&str, String)],
    bearer_token_env: Option<&str>,
    request_body: Option<serde_json::Value>,
) -> anyhow::Result<TraceCommonsApiResponse> {
    let url = trace_commons_api_url(endpoint, path, query)?;
    let client = reqwest::Client::new();
    let mut request = client.request(method, &url);
    if let Some(bearer_token_env) = bearer_token_env {
        let token = std::env::var(bearer_token_env).map_err(|_| {
            anyhow::anyhow!(
                "{} is not set; refusing to call Trace Commons API without credentials",
                bearer_token_env
            )
        })?;
        request = request.bearer_auth(token);
    }
    if let Some(request_body) = request_body {
        request = request.json(&request_body);
    }

    let response = request
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Trace Commons API request to {url} failed: {e}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "Trace Commons API request to {} failed with {}: {}",
            url,
            status,
            compact_response_body(&body)
        );
    }
    let json = if body.trim().is_empty() {
        None
    } else {
        serde_json::from_str(&body).ok()
    };
    Ok(TraceCommonsApiResponse { url, body, json })
}

async fn trace_commons_optional_api_request(
    method: Method,
    endpoint: &str,
    path: &str,
    query: &[(&str, String)],
    bearer_token_env: Option<&str>,
    request_body: Option<serde_json::Value>,
) -> anyhow::Result<Option<TraceCommonsApiResponse>> {
    let url = trace_commons_api_url(endpoint, path, query)?;
    let client = reqwest::Client::new();
    let mut request = client.request(method, &url);
    if let Some(bearer_token_env) = bearer_token_env {
        let token = std::env::var(bearer_token_env).map_err(|_| {
            anyhow::anyhow!(
                "{} is not set; refusing to call Trace Commons API without credentials",
                bearer_token_env
            )
        })?;
        request = request.bearer_auth(token);
    }
    if let Some(request_body) = request_body {
        request = request.json(&request_body);
    }

    let response = request
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Trace Commons API request to {url} failed: {e}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::NOT_IMPLEMENTED {
        return Ok(None);
    }
    if !status.is_success() {
        anyhow::bail!(
            "Trace Commons API request to {} failed with {}: {}",
            url,
            status,
            compact_response_body(&body)
        );
    }
    let json = if body.trim().is_empty() {
        None
    } else {
        serde_json::from_str(&body).ok()
    };
    Ok(Some(TraceCommonsApiResponse { url, body, json }))
}

fn trace_commons_api_url(
    endpoint: &str,
    path: &str,
    query: &[(&str, String)],
) -> anyhow::Result<String> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|e| anyhow::anyhow!("invalid Trace Commons endpoint {endpoint}: {e}"))?;
    let desired_path = normalize_url_path(path);
    let current_path = url.path().trim_end_matches('/');
    if current_path != desired_path.trim_end_matches('/') {
        let prefix = trace_commons_endpoint_prefix(current_path);
        url.set_path(&join_url_paths(&prefix, &desired_path));
    }
    url.set_query(None);
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    Ok(url.to_string())
}

fn trace_commons_endpoint_prefix(path: &str) -> String {
    let path = path.trim_end_matches('/');
    for suffix in ["/v1/traces", "/health", "/v1"] {
        if let Some(prefix) = path.strip_suffix(suffix) {
            return prefix.to_string();
        }
    }
    if path == "/" {
        String::new()
    } else {
        path.to_string()
    }
}

fn normalize_url_path(path: &str) -> String {
    format!("/{}", path.trim_start_matches('/'))
        .trim_end_matches('/')
        .to_string()
}

fn join_url_paths(prefix: &str, path: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        normalize_url_path(path)
    } else {
        format!("{prefix}{}", normalize_url_path(path))
    }
}

fn optional_usize_query(key: &'static str, value: Option<usize>) -> Vec<(&'static str, String)> {
    value
        .map(|value| vec![(key, value.to_string())])
        .unwrap_or_default()
}

fn require_non_empty_purpose(purpose: &str) -> anyhow::Result<()> {
    if purpose.trim().is_empty() {
        anyhow::bail!("--purpose must not be empty");
    }
    Ok(())
}

fn require_non_empty_policy_version(policy_version: &str) -> anyhow::Result<()> {
    if policy_version.trim().is_empty() {
        anyhow::bail!("--policy-version must not be empty");
    }
    Ok(())
}

fn canary_leaked_tokens(original: &str, redacted: &str) -> Vec<String> {
    let candidates = original
        .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';')
        .map(|token| token.trim_matches(|ch: char| ch == '"' || ch == '\'' || ch == '.'))
        .filter(|token| {
            token.contains('@')
                || token.starts_with("sk-")
                || token.starts_with('/')
                || token.contains("TOKEN")
                || token.contains("tenant")
                || token.contains("user")
        });
    let mut leaked = Vec::new();
    for token in candidates {
        if !token.is_empty() && redacted.contains(token) && !leaked.iter().any(|v| v == token) {
            leaked.push(token.to_string());
        }
    }
    leaked
}

fn print_trace_commons_json(response: &TraceCommonsApiResponse) -> anyhow::Result<()> {
    println!("{}", pretty_trace_commons_body(response)?);
    Ok(())
}

fn print_trace_commons_unsupported(path: &str) {
    println!(
        "Trace Commons endpoint {path} is not exposed by this ingestion service yet (404/501)."
    );
}

fn pretty_trace_commons_body(response: &TraceCommonsApiResponse) -> anyhow::Result<String> {
    if let Some(value) = response.json.as_ref() {
        serde_json::to_string_pretty(value)
            .map_err(|e| anyhow::anyhow!("failed to serialize Trace Commons response: {}", e))
    } else {
        Ok(response.body.clone())
    }
}

fn print_trace_commons_items(heading: &str, value: Option<&serde_json::Value>, fields: &[&str]) {
    let Some(items) = value.and_then(serde_json::Value::as_array) else {
        println!("{heading}: response was not a JSON array.");
        return;
    };
    if items.is_empty() {
        println!("{heading}: none.");
        return;
    }
    println!("{heading}: {}", items.len());
    for item in items {
        let details = fields
            .iter()
            .filter_map(|field| json_field_display(item, field).map(|value| (*field, value)))
            .map(|(field, value)| format!("{field}={value}"))
            .collect::<Vec<_>>()
            .join("  ");
        println!("  {details}");
    }
}

fn print_optional_json_field(label: &str, value: &serde_json::Value, field: &str) {
    if let Some(display) = json_field_display(value, field) {
        println!("{label}: {display}");
    }
}

fn print_json_map(label: &str, value: Option<&serde_json::Value>) {
    let Some(map) = value.and_then(serde_json::Value::as_object) else {
        return;
    };
    if map.is_empty() {
        return;
    }
    let items = map
        .iter()
        .map(|(key, value)| {
            let value = json_value_display(value);
            format!("{key}={value}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    println!("{label}: {items}");
}

fn maintenance_audit_chain_lines(value: &serde_json::Value) -> Vec<String> {
    let Some(audit_chain) = value
        .get("audit_chain")
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };

    let mut lines = vec!["  audit chain:".to_string()];
    if let Some(line) = compact_json_items(
        audit_chain,
        "    status",
        &[
            ("verified", "verified"),
            ("event_count", "events"),
            ("legacy_event_count", "legacy"),
            ("mismatch_count", "mismatches"),
        ],
    ) {
        lines.push(line);
    }
    if let Some(failures) = audit_chain
        .get("failures")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        && failures > 0
    {
        lines.push(format!("    failures: {failures}"));
    }
    if let Some(db_mirror) = audit_chain
        .get("db_mirror")
        .and_then(serde_json::Value::as_object)
    {
        lines.push("    db mirror:".to_string());
        if let Some(line) = compact_json_items(
            db_mirror,
            "      status",
            &[
                ("verified", "verified"),
                ("event_count", "events"),
                ("legacy_event_count", "legacy"),
                ("payload_verified_event_count", "payload_verified"),
                ("payload_unverified_event_count", "payload_unverified"),
                ("mismatch_count", "mismatches"),
            ],
        ) {
            lines.push(line);
        }
        if let Some(failures) = db_mirror
            .get("failures")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            && failures > 0
        {
            lines.push(format!("      failures: {failures}"));
        }
    }
    lines
}

fn maintenance_reconciliation_lines(value: &serde_json::Value) -> Vec<String> {
    let Some(reconciliation) = value
        .get("db_reconciliation")
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };

    let mut lines = vec!["  db reconciliation:".to_string()];

    if let Some(line) = compact_json_items(
        reconciliation,
        "    submissions",
        &[
            ("file_submission_count", "files"),
            ("db_submission_count", "db"),
            ("missing_submission_ids_in_db", "missing_in_db"),
            ("missing_submission_ids_in_files", "missing_in_files"),
            ("status_mismatches", "status_mismatches"),
        ],
    ) {
        lines.push(line);
    }
    if let Some(line) = compact_json_items(
        reconciliation,
        "    derived",
        &[
            ("file_derived_count", "files"),
            ("db_derived_count", "db"),
            ("missing_derived_submission_ids_in_db", "missing_in_db"),
            (
                "missing_derived_submission_ids_in_files",
                "missing_in_files",
            ),
            ("derived_status_mismatches", "status_mismatches"),
            ("derived_hash_mismatches", "hash_mismatches"),
        ],
    ) {
        lines.push(line);
    }
    if let Some(line) = compact_json_items(
        reconciliation,
        "    object refs",
        &[
            ("db_object_ref_count", "db"),
            (
                "accepted_without_active_envelope_object_ref",
                "accepted_without_active_envelope",
            ),
            (
                "unreadable_active_envelope_object_refs",
                "unreadable_active_envelope",
            ),
            (
                "hash_mismatched_active_envelope_object_refs",
                "hash_mismatched_active_envelope",
            ),
        ],
    ) {
        lines.push(line);
    }
    if let Some(line) = compact_json_items(
        reconciliation,
        "    ledger/audit",
        &[
            ("file_credit_event_count", "file_credit_events"),
            ("db_credit_event_count", "db_credit_events"),
            ("missing_credit_event_ids_in_db", "missing_credit_in_db"),
            (
                "missing_credit_event_ids_in_files",
                "missing_credit_in_files",
            ),
            ("file_audit_event_count", "file_audit_events"),
            ("db_audit_event_count", "db_audit_events"),
            ("missing_audit_event_ids_in_db", "missing_audit_in_db"),
            ("missing_audit_event_ids_in_files", "missing_audit_in_files"),
        ],
    ) {
        lines.push(line);
    }
    if let Some(line) = compact_json_items(
        reconciliation,
        "    exports/tombstones",
        &[
            ("file_replay_export_manifest_count", "file_replay_manifests"),
            ("db_export_manifest_count", "db_export_manifests"),
            ("db_replay_export_manifest_count", "db_replay_manifests"),
            (
                "db_benchmark_export_manifest_count",
                "db_benchmark_manifests",
            ),
            ("db_ranker_export_manifest_count", "db_ranker_manifests"),
            ("db_other_export_manifest_count", "db_other_manifests"),
            ("db_export_manifest_item_count", "db_export_items"),
            (
                "db_export_manifest_item_missing_object_ref_count",
                "db_export_items_missing_object_refs",
            ),
            (
                "db_export_manifest_ids_with_missing_object_refs",
                "db_export_manifests_missing_object_refs",
            ),
            (
                "file_revocation_tombstone_count",
                "file_revocation_tombstones",
            ),
            ("db_tombstone_count", "db_tombstones"),
        ],
    ) {
        lines.push(line);
    }
    if let Some(line) = compact_json_items(
        reconciliation,
        "    reader parity",
        &[
            ("contributor_credit_reader_parity_ok", "contributor_credit"),
            ("reviewer_metadata_reader_parity_ok", "reviewer_metadata"),
            ("analytics_reader_parity_ok", "analytics"),
            ("audit_reader_parity_ok", "audit"),
            (
                "replay_export_manifest_reader_parity_ok",
                "replay_export_manifests",
            ),
            ("db_reader_parity_failures", "failures"),
        ],
    ) {
        lines.push(line);
    }
    if let Some(line) = compact_json_items(
        reconciliation,
        "    vectors",
        &[
            ("active_vector_entries", "active"),
            (
                "accepted_current_derived_without_active_vector_entry",
                "eligible_without_active",
            ),
            ("invalid_active_vector_entries", "invalid_active"),
        ],
    ) {
        lines.push(line);
    }
    if let Some(line) =
        compact_json_items(reconciliation, "    blocking", &[("blocking_gaps", "gaps")])
    {
        lines.push(line);
    }

    if lines.len() == 1 { Vec::new() } else { lines }
}

fn compact_json_items(
    map: &serde_json::Map<String, serde_json::Value>,
    label: &str,
    fields: &[(&str, &str)],
) -> Option<String> {
    let items = fields
        .iter()
        .filter_map(|(field, display_name)| {
            map.get(*field)
                .map(compact_json_count_display)
                .map(|value| format!("{display_name}={value}"))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        None
    } else {
        Some(format!("{label}: {}", items.join(" ")))
    }
}

fn compact_json_count_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(values) => values.len().to_string(),
        _ => json_value_display(value),
    }
}

fn json_field_display(value: &serde_json::Value, field: &str) -> Option<String> {
    value.get(field).map(json_value_display)
}

fn json_value_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(values) => values
            .iter()
            .map(json_value_display)
            .collect::<Vec<_>>()
            .join(","),
        serde_json::Value::Object(_) => value.to_string(),
    }
}

fn compact_response_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "(empty response body)".to_string();
    }
    const MAX_ERROR_BODY_CHARS: usize = 1000;
    let mut compact = trimmed.replace('\n', " ");
    if compact.chars().count() > MAX_ERROR_BODY_CHARS {
        compact = compact.chars().take(MAX_ERROR_BODY_CHARS).collect();
        compact.push_str("...");
    }
    compact
}

async fn flush_queue(limit: usize) -> anyhow::Result<()> {
    let policy = read_policy()?;
    if !policy.enabled {
        anyhow::bail!("trace contribution opt-in is disabled; run `ironclaw traces opt-in` first");
    }
    let endpoint = policy
        .ingestion_endpoint
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("trace contribution endpoint is not configured"))?;

    let mut submitted = 0usize;
    let mut held = 0usize;
    for path in queued_envelope_paths()?.into_iter().take(limit) {
        let mut envelope = load_envelope(&path)?;
        apply_credit_estimate(&mut envelope);

        match autonomous_eligibility(&envelope, &policy) {
            QueueEligibility::Submit => {
                let receipt =
                    submit_envelope_to_endpoint(&envelope, endpoint, &policy.bearer_token_env)
                        .await?;
                record_submitted_envelope(&envelope, endpoint, receipt)?;
                std::fs::remove_file(&path).map_err(|e| {
                    anyhow::anyhow!("failed to remove queued envelope {}: {}", path.display(), e)
                })?;
                submitted += 1;
            }
            QueueEligibility::Hold { reason } => {
                write_queue_hold_reason(&path, &reason)?;
                held += 1;
            }
        }
    }

    if let Err(error) = sync_cli_submission_records(&policy).await {
        eprintln!("Warning: failed to sync remote trace credit status: {error}");
    }
    println!("Autonomous trace queue flush complete: {submitted} submitted, {held} held.");
    maybe_print_credit_notice(&policy)?;
    Ok(())
}

async fn sync_cli_submission_records(
    policy: &StandingTraceContributionPolicy,
) -> anyhow::Result<usize> {
    if !policy.enabled {
        return Ok(0);
    }
    let Some(endpoint) = policy.ingestion_endpoint.as_deref() else {
        return Ok(0);
    };
    let submission_ids = read_local_records()?
        .into_iter()
        .filter(|record| record.status == LocalSubmissionStatus::Submitted)
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
    apply_cli_submission_status_updates(&updates)
}

fn apply_cli_submission_status_updates(
    updates: &[TraceSubmissionStatusUpdate],
) -> anyhow::Result<usize> {
    if updates.is_empty() {
        return Ok(0);
    }

    let mut records = read_local_records()?;
    let now = chrono::Utc::now();
    let mut changed = 0usize;
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
        let status_changed = record.server_status.as_deref() != Some(update.status.as_str());
        let credit_changed = (old_effective_credit - new_effective_credit).abs() > f32::EPSILON;

        record.trace_id = update.trace_id;
        record.server_status = Some(update.status.clone());
        record.credit_points_pending = update.credit_points_pending;
        record.credit_points_final = new_stored_final;
        if !explanation.is_empty() {
            record.credit_explanation = explanation;
        }
        if update.status == "revoked" {
            record.status = LocalSubmissionStatus::Revoked;
            record.revoked_at.get_or_insert(now);
        } else if update.status == "expired" {
            record.status = LocalSubmissionStatus::Expired;
        } else if update.status == "purged" {
            record.status = LocalSubmissionStatus::Purged;
        }

        if status_changed || credit_changed {
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
                contributor_pseudonym: "cli-sync".to_string(),
                kind: TraceCreditEventKind::CreditSynced,
                points_delta: new_effective_credit - old_effective_credit,
                reason: sync_reason,
                created_at: now,
            });
            changed += 1;
        }
    }

    if changed > 0 {
        write_local_records(&records)?;
    }
    Ok(changed)
}

fn maybe_print_credit_notice(policy: &StandingTraceContributionPolicy) -> anyhow::Result<()> {
    if policy.credit_notice_interval_hours == 0 {
        return Ok(());
    }

    let mut records = read_local_records()?;
    if records
        .iter()
        .all(|record| !local_record_noticeable_for_credit(record))
    {
        return Ok(());
    }

    let now = chrono::Utc::now();
    let interval = chrono::Duration::hours(i64::from(policy.credit_notice_interval_hours));
    let notice_due = records
        .iter()
        .filter(|record| local_record_noticeable_for_credit(record))
        .any(|record| {
            record
                .last_credit_notice_at
                .map(|last_notice| now.signed_duration_since(last_notice) >= interval)
                .unwrap_or(true)
        });

    if !notice_due {
        return Ok(());
    }

    let summary = credit_summary(&records);
    println!(
        "Trace contribution credit update: {} submitted, pending +{:.2}, final +{:.2}.",
        summary.submissions_submitted, summary.pending_credit, summary.final_credit
    );
    for explanation in summary.recent_explanations.iter().take(3) {
        println!("  - {explanation}");
    }

    for record in &mut records {
        if local_record_noticeable_for_credit(record) {
            record.last_credit_notice_at = Some(now);
        }
    }
    write_local_records(&records)
}

fn local_record_noticeable_for_credit(record: &LocalSubmissionRecord) -> bool {
    record.status == LocalSubmissionStatus::Submitted || !record.credit_events.is_empty()
}

enum QueueEligibility {
    Submit,
    Hold { reason: String },
}

fn autonomous_eligibility(
    envelope: &TraceContributionEnvelope,
    policy: &StandingTraceContributionPolicy,
) -> QueueEligibility {
    if policy.require_manual_approval_when_pii_detected
        && envelope.privacy.residual_pii_risk != ResidualPiiRisk::Low
    {
        return QueueEligibility::Hold {
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
        return QueueEligibility::Hold {
            reason: "trace does not use any selected auto-submit tools".to_string(),
        };
    }

    if envelope.value.submission_score < policy.min_submission_score {
        return QueueEligibility::Hold {
            reason: format!(
                "submission score {:.2} is below policy minimum {:.2}",
                envelope.value.submission_score, policy.min_submission_score
            ),
        };
    }

    let failed_trace = matches!(
        envelope.outcome.task_success,
        crate::trace_contribution::TaskSuccess::Failure
            | crate::trace_contribution::TaskSuccess::Partial
    );
    if failed_trace && policy.auto_submit_failed_traces {
        return QueueEligibility::Submit;
    }
    if policy.auto_submit_high_value_traces {
        return QueueEligibility::Submit;
    }

    QueueEligibility::Hold {
        reason: "policy does not allow this autonomous submission class".to_string(),
    }
}

fn write_queue_hold_reason(path: &Path, reason: &str) -> anyhow::Result<()> {
    let hold_path = path.with_extension("held.json");
    let body = serde_json::json!({
        "envelope": path.file_name().and_then(|name| name.to_str()).unwrap_or("unknown"),
        "held_at": chrono::Utc::now(),
        "reason": reason,
    });
    std::fs::write(
        &hold_path,
        serde_json::to_string_pretty(&body)
            .map_err(|e| anyhow::anyhow!("failed to serialize queue hold reason: {}", e))?,
    )
    .map_err(|e| {
        anyhow::anyhow!(
            "failed to write queue hold reason {}: {}",
            hold_path.display(),
            e
        )
    })
}

async fn list_submissions(json: bool, summary: bool) -> anyhow::Result<()> {
    let policy = read_policy()?;
    if let Err(error) = sync_cli_submission_records(&policy).await {
        eprintln!("Warning: failed to sync remote trace credit status: {error}");
    }
    let records = read_local_records()?;
    if json {
        if summary {
            let body = serde_json::json!({
                "summary": credit_summary(&records),
                "submissions": records,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&body).map_err(|e| {
                    anyhow::anyhow!("failed to serialize submission records: {}", e)
                })?
            );
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&records).map_err(|e| {
                    anyhow::anyhow!("failed to serialize submission records: {}", e)
                })?
            );
        }
        return Ok(());
    }

    if records.is_empty() {
        println!("No local trace contribution submissions recorded.");
        if summary {
            println!("Summary:");
            print_credit_summary_fields(&credit_summary(&records), "  ");
        }
        return Ok(());
    }

    println!("Trace contribution submissions:");
    for record in &records {
        let local_status = match record.status {
            LocalSubmissionStatus::Submitted => "submitted",
            LocalSubmissionStatus::Revoked => "revoked",
            LocalSubmissionStatus::Expired => "expired",
            LocalSubmissionStatus::Purged => "purged",
        };
        let status = record.server_status.as_deref().unwrap_or(local_status);
        let submitted = record
            .submitted_at
            .as_ref()
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "not submitted".to_string());
        println!(
            "  {}  {}  {}  pending +{:.2}  {}",
            record.submission_id,
            status,
            record.privacy_risk,
            record.credit_points_pending,
            submitted
        );
    }
    if summary {
        println!("Summary:");
        print_credit_summary_fields(&credit_summary(&records), "  ");
    }
    Ok(())
}

async fn show_credit(json: bool) -> anyhow::Result<()> {
    let policy = read_policy()?;
    if let Err(error) = sync_cli_submission_records(&policy).await {
        eprintln!("Warning: failed to sync remote trace credit status: {error}");
    }
    let summary = credit_summary(&read_local_records()?);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary)
                .map_err(|e| anyhow::anyhow!("failed to serialize credit summary: {}", e))?
        );
        return Ok(());
    }

    println!("Trace contribution credit:");
    print_credit_summary_fields(&summary, "  ");
    Ok(())
}

fn print_credit_summary_fields(summary: &CreditSummary, indent: &str) {
    println!("{indent}submissions: {}", summary.submissions_total);
    println!("{indent}submitted: {}", summary.submissions_submitted);
    println!("{indent}revoked: {}", summary.submissions_revoked);
    println!("{indent}expired: {}", summary.submissions_expired);
    println!("{indent}pending credit: +{:.2}", summary.pending_credit);
    println!("{indent}final credit: +{:.2}", summary.final_credit);
    if !summary.recent_explanations.is_empty() {
        println!("{indent}recent explanations:");
        for explanation in &summary.recent_explanations {
            println!("{indent}  - {explanation}");
        }
    }
}

fn credit_summary(records: &[LocalSubmissionRecord]) -> CreditSummary {
    let mut summary = CreditSummary {
        submissions_total: records.len() as u32,
        submissions_submitted: records
            .iter()
            .filter(|r| r.status == LocalSubmissionStatus::Submitted)
            .count() as u32,
        submissions_revoked: records
            .iter()
            .filter(|r| r.status == LocalSubmissionStatus::Revoked)
            .count() as u32,
        submissions_expired: records
            .iter()
            .filter(|r| {
                matches!(
                    r.status,
                    LocalSubmissionStatus::Expired | LocalSubmissionStatus::Purged
                )
            })
            .count() as u32,
        pending_credit: records.iter().map(|r| r.credit_points_pending).sum(),
        final_credit: records.iter().filter_map(|r| r.credit_points_final).sum(),
        recent_explanations: Vec::new(),
    };

    summary.recent_explanations = records
        .iter()
        .rev()
        .flat_map(|record| record.credit_explanation.iter().cloned())
        .take(6)
        .collect();
    summary
}

fn load_envelope(path: &Path) -> anyhow::Result<TraceContributionEnvelope> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read envelope {}: {}", path.display(), e))?;
    serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("failed to parse envelope {}: {}", path.display(), e))
}

fn apply_credit_estimate(envelope: &mut TraceContributionEnvelope) {
    let estimate = estimate_initial_credit(envelope);
    envelope.value.submission_score = estimate.submission_score;
    envelope.value.credit_points_pending = estimate.credit_points_pending;
    envelope.value.explanation = estimate.explanation;
    envelope.value_card.scorecard = estimate.scorecard;
    envelope.value_card.user_visible_explanation = envelope.value.explanation.clone();
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalSubmissionRecord {
    submission_id: Uuid,
    trace_id: Uuid,
    endpoint: Option<String>,
    status: LocalSubmissionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server_status: Option<String>,
    submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    privacy_risk: String,
    redaction_counts: BTreeMap<String, u32>,
    #[serde(default)]
    credit_points_pending: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credit_points_final: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    credit_explanation: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    credit_events: Vec<TraceCreditEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_credit_notice_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LocalSubmissionStatus {
    Submitted,
    Revoked,
    Expired,
    Purged,
}

fn upsert_local_record(record: LocalSubmissionRecord) -> anyhow::Result<()> {
    let mut records = read_local_records()?;
    if let Some(existing) = records
        .iter_mut()
        .find(|existing| existing.submission_id == record.submission_id)
    {
        *existing = record;
    } else {
        records.push(record);
    }
    write_local_records(&records)
}

fn mark_local_revoked(submission_id: Uuid) -> anyhow::Result<()> {
    let mut records = read_local_records()?;
    let now = chrono::Utc::now();
    let mut found = false;
    for record in &mut records {
        if record.submission_id == submission_id {
            record.status = LocalSubmissionStatus::Revoked;
            record.revoked_at = Some(now);
            found = true;
        }
    }

    if !found {
        records.push(LocalSubmissionRecord {
            submission_id,
            trace_id: Uuid::nil(),
            endpoint: None,
            status: LocalSubmissionStatus::Revoked,
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

    write_local_records(&records)
}

fn read_local_records() -> anyhow::Result<Vec<LocalSubmissionRecord>> {
    let path = local_records_path();
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
    serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse local trace submission records {}: {}",
            path.display(),
            e
        )
    })
}

fn write_local_records(records: &[LocalSubmissionRecord]) -> anyhow::Result<()> {
    let path = local_records_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!(
                "failed to create local trace submission directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    let body = serde_json::to_string_pretty(records)
        .map_err(|e| anyhow::anyhow!("failed to serialize local submission records: {}", e))?;
    std::fs::write(&path, body).map_err(|e| {
        anyhow::anyhow!(
            "failed to write local trace submission records {}: {}",
            path.display(),
            e
        )
    })
}

fn local_records_path() -> PathBuf {
    trace_contribution_dir().join("submissions.json")
}

fn policy_path() -> PathBuf {
    trace_contribution_dir().join("policy.json")
}

fn queue_dir() -> PathBuf {
    trace_contribution_dir().join("queue")
}

fn trace_contribution_dir() -> PathBuf {
    crate::bootstrap::ironclaw_base_dir().join("trace_contributions")
}

fn read_policy() -> anyhow::Result<StandingTraceContributionPolicy> {
    let path = policy_path();
    if !path.exists() {
        return Ok(StandingTraceContributionPolicy::default());
    }
    let body = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read trace policy {}: {}", path.display(), e))?;
    serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("failed to parse trace policy {}: {}", path.display(), e))
}

fn write_policy(policy: &StandingTraceContributionPolicy) -> anyhow::Result<()> {
    let path = policy_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!(
                "failed to create trace contribution directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    let body = serde_json::to_string_pretty(policy)
        .map_err(|e| anyhow::anyhow!("failed to serialize trace policy: {}", e))?;
    std::fs::write(&path, body)
        .map_err(|e| anyhow::anyhow!("failed to write trace policy {}: {}", path.display(), e))
}

fn enqueue_envelope(envelope: &TraceContributionEnvelope) -> anyhow::Result<PathBuf> {
    let path = queue_dir().join(format!("{}.json", envelope.submission_id));
    std::fs::create_dir_all(queue_dir()).map_err(|e| {
        anyhow::anyhow!(
            "failed to create trace contribution queue {}: {}",
            queue_dir().display(),
            e
        )
    })?;
    let body = serde_json::to_string_pretty(envelope)
        .map_err(|e| anyhow::anyhow!("failed to serialize queued envelope: {}", e))?;
    std::fs::write(&path, body).map_err(|e| {
        anyhow::anyhow!("failed to write queued envelope {}: {}", path.display(), e)
    })?;
    Ok(path)
}

fn queued_envelope_paths() -> anyhow::Result<Vec<PathBuf>> {
    let dir = queue_dir();
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

fn redaction_summary(counts: &BTreeMap<String, u32>) -> String {
    if counts.is_empty() {
        return "no deterministic redactions".to_string();
    }
    counts
        .iter()
        .map(|(label, count)| format!("{count} {label}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;

    #[test]
    fn trace_scope_arg_maps_to_consent_scope() {
        assert_eq!(
            ConsentScope::from(TraceScopeArg::BenchmarkOnly),
            ConsentScope::BenchmarkOnly
        );
    }

    #[test]
    fn redaction_summary_handles_empty_and_counts() {
        assert_eq!(
            redaction_summary(&BTreeMap::new()),
            "no deterministic redactions"
        );

        let mut counts = BTreeMap::new();
        counts.insert("local_path".to_string(), 2);
        counts.insert("secret".to_string(), 1);
        assert_eq!(redaction_summary(&counts), "2 local_path, 1 secret");
    }

    #[test]
    fn list_submissions_summary_flag_parses_through_cli() {
        let cli = Cli::try_parse_from(["ironclaw", "traces", "list-submissions", "--summary"])
            .expect("list-submissions --summary should parse");

        let Some(Command::Traces(TracesCommand::ListSubmissions { json, summary })) = cli.command
        else {
            panic!("expected traces list-submissions command");
        };

        assert!(!json);
        assert!(summary);
    }

    #[test]
    fn active_learning_review_queue_uses_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/review/active-learning",
            &[("limit", "25".to_string())],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/review/active-learning?limit=25"
        );
    }

    #[test]
    fn ranker_training_candidates_use_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal",
            "/v1/ranker/training-candidates",
            &[
                ("consent_scope", "ranking-training".to_string()),
                ("status", "accepted".to_string()),
            ],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/ranker/training-candidates?consent_scope=ranking-training&status=accepted"
        );
    }

    #[test]
    fn ranker_training_candidates_purpose_flag_parses_through_cli() {
        let cli = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "ranker-training-candidates",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-ranker-candidates",
        ])
        .expect("ranker candidates --purpose should parse");

        let Some(Command::Traces(TracesCommand::RankerTrainingCandidates { purpose, .. })) =
            cli.command
        else {
            panic!("expected traces ranker-training-candidates command");
        };

        assert_eq!(purpose.as_deref(), Some("nightly-ranker-candidates"));
    }

    #[test]
    fn ranker_training_pairs_purpose_flag_parses_through_cli() {
        let cli = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "ranker-training-pairs",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-ranker-pairs",
        ])
        .expect("ranker pairs --purpose should parse");

        let Some(Command::Traces(TracesCommand::RankerTrainingPairs { purpose, .. })) = cli.command
        else {
            panic!("expected traces ranker-training-pairs command");
        };

        assert_eq!(purpose.as_deref(), Some("nightly-ranker-pairs"));
    }

    #[test]
    fn ranker_training_pairs_use_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1",
            "/v1/ranker/training-pairs",
            &[("privacy_risk", "low".to_string())],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/ranker/training-pairs?privacy_risk=low"
        );
    }

    #[test]
    fn tenant_policy_get_parses_through_cli() {
        let cli = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "tenant-policy-get",
            "--endpoint",
            "https://trace.example/internal",
            "--json",
        ])
        .expect("tenant-policy-get should parse");

        let Some(Command::Traces(TracesCommand::TenantPolicyGet { endpoint, json, .. })) =
            cli.command
        else {
            panic!("expected traces tenant-policy-get command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert!(json);
    }

    #[test]
    fn config_status_parses_through_cli() {
        let cli = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "config-status",
            "--endpoint",
            "https://trace.example/internal",
            "--bearer-token-env",
            "TRACE_COMMONS_ADMIN_TOKEN",
        ])
        .expect("config-status should parse");

        let Some(Command::Traces(TracesCommand::ConfigStatus {
            endpoint,
            bearer_token_env,
        })) = cli.command
        else {
            panic!("expected traces config-status command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
    }

    #[test]
    fn tenant_policy_set_parses_through_cli() {
        let cli = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "tenant-policy-set",
            "--endpoint",
            "https://trace.example/internal",
            "--policy-version",
            "2026-04-26",
            "--allowed-consent-scopes",
            "debugging-evaluation,ranking-training",
            "--allowed-uses",
            "debugging,ranking-model-training",
        ])
        .expect("tenant-policy-set should parse");

        let Some(Command::Traces(TracesCommand::TenantPolicySet {
            policy_version,
            allowed_consent_scopes,
            allowed_uses,
            ..
        })) = cli.command
        else {
            panic!("expected traces tenant-policy-set command");
        };

        assert_eq!(policy_version, "2026-04-26");
        assert_eq!(
            allowed_consent_scopes,
            vec![
                TraceScopeArg::DebuggingEvaluation,
                TraceScopeArg::RankingTraining
            ]
        );
        assert_eq!(
            allowed_uses,
            vec![
                TraceAllowedUseArg::Debugging,
                TraceAllowedUseArg::RankingModelTraining
            ]
        );
    }

    #[test]
    fn tenant_policy_uses_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/admin/tenant-policy",
            &[],
        )
        .expect("url builds");

        assert_eq!(url, "https://trace.example/internal/v1/admin/tenant-policy");
    }

    #[test]
    fn config_status_uses_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/admin/config-status",
            &[],
        )
        .expect("url builds");

        assert_eq!(url, "https://trace.example/internal/v1/admin/config-status");
    }

    #[test]
    fn tenant_policy_body_uses_server_enum_names() {
        let body = trace_commons_tenant_policy_body(
            "2026-04-26".to_string(),
            vec![
                TraceScopeArg::DebuggingEvaluation,
                TraceScopeArg::BenchmarkOnly,
            ],
            vec![
                TraceAllowedUseArg::Debugging,
                TraceAllowedUseArg::AggregateAnalytics,
                TraceAllowedUseArg::RankingModelTraining,
            ],
        );

        assert_eq!(body["policy_version"], "2026-04-26");
        assert_eq!(
            body["allowed_consent_scopes"],
            serde_json::json!(["debugging_evaluation", "benchmark_only"])
        );
        assert_eq!(
            body["allowed_uses"],
            serde_json::json!(["debugging", "aggregate_analytics", "ranking_model_training"])
        );
    }

    #[test]
    fn worker_benchmark_convert_uses_dedicated_route() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/workers/benchmark-convert",
            &[],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/workers/benchmark-convert"
        );
    }

    #[test]
    fn benchmark_lifecycle_update_uses_conversion_route() {
        let conversion_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses");
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            &format!("/v1/benchmarks/{conversion_id}/lifecycle"),
            &[],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/benchmarks/018f2b7b-0c11-72fd-95c4-1f9f98feac01/lifecycle"
        );
    }

    #[test]
    fn benchmark_lifecycle_update_parses_through_cli() {
        let cli = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "benchmark-lifecycle-update",
            "--endpoint",
            "https://trace.example/internal",
            "--conversion-id",
            "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
            "--registry-status",
            "published",
            "--registry-ref",
            "benchmark:trace-018",
            "--published-at",
            "2026-04-26T12:00:00Z",
            "--evaluation-status",
            "passed",
            "--evaluator-ref",
            "eval:nightly-42",
            "--evaluated-at",
            "2026-04-26T12:05:00Z",
            "--score",
            "0.97",
            "--pass-count",
            "7",
            "--fail-count",
            "0",
            "--reason",
            "published after evaluator pass",
            "--bearer-token-env",
            "TRACE_COMMONS_BENCHMARK_WORKER_TOKEN",
        ])
        .expect("benchmark-lifecycle-update should parse");

        let Some(Command::Traces(TracesCommand::BenchmarkLifecycleUpdate {
            conversion_id,
            registry_status,
            registry_ref,
            published_at,
            evaluation_status,
            evaluator_ref,
            evaluated_at,
            score,
            pass_count,
            fail_count,
            reason,
            bearer_token_env,
            ..
        })) = cli.command
        else {
            panic!("expected traces benchmark-lifecycle-update command");
        };

        assert_eq!(
            conversion_id,
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses")
        );
        assert_eq!(
            registry_status,
            Some(TraceBenchmarkRegistryStatusArg::Published)
        );
        assert_eq!(registry_ref.as_deref(), Some("benchmark:trace-018"));
        assert_eq!(published_at.as_deref(), Some("2026-04-26T12:00:00Z"));
        assert_eq!(
            evaluation_status,
            Some(TraceBenchmarkEvaluationStatusArg::Passed)
        );
        assert_eq!(evaluator_ref.as_deref(), Some("eval:nightly-42"));
        assert_eq!(evaluated_at.as_deref(), Some("2026-04-26T12:05:00Z"));
        assert_eq!(score, Some(0.97));
        assert_eq!(pass_count, Some(7));
        assert_eq!(fail_count, Some(0));
        assert_eq!(reason.as_deref(), Some("published after evaluator pass"));
        assert_eq!(bearer_token_env, "TRACE_COMMONS_BENCHMARK_WORKER_TOKEN");
    }

    #[test]
    fn benchmark_lifecycle_body_uses_nested_server_fields() {
        let body = trace_commons_benchmark_lifecycle_body(
            Some(TraceBenchmarkRegistryStatusArg::Published),
            Some("benchmark:trace-018".to_string()),
            Some("2026-04-26T12:00:00Z".to_string()),
            Some(TraceBenchmarkEvaluationStatusArg::Passed),
            Some("eval:nightly-42".to_string()),
            Some("2026-04-26T12:05:00Z".to_string()),
            Some(0.5),
            Some(7),
            Some(0),
            Some("published after evaluator pass".to_string()),
        )
        .expect("body builds");

        assert_eq!(
            body,
            serde_json::json!({
                "registry": {
                    "status": "published",
                    "registry_ref": "benchmark:trace-018",
                    "published_at": "2026-04-26T12:00:00Z"
                },
                "evaluation": {
                    "status": "passed",
                    "evaluator_ref": "eval:nightly-42",
                    "evaluated_at": "2026-04-26T12:05:00Z",
                    "score": 0.5,
                    "pass_count": 7,
                    "fail_count": 0
                },
                "reason": "published after evaluator pass"
            })
        );
    }

    #[test]
    fn benchmark_lifecycle_body_rejects_empty_and_invalid_score() {
        let empty = trace_commons_benchmark_lifecycle_body(
            None, None, None, None, None, None, None, None, None, None,
        );
        assert!(empty.is_err());

        let invalid_score = trace_commons_benchmark_lifecycle_body(
            None,
            None,
            None,
            Some(TraceBenchmarkEvaluationStatusArg::Passed),
            None,
            None,
            Some(1.5),
            None,
            None,
            None,
        );
        assert!(invalid_score.is_err());
    }

    #[test]
    fn worker_commands_parse_through_cli() {
        let benchmark = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "worker-benchmark-convert",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-worker-benchmark",
            "--status",
            "accepted",
        ])
        .expect("worker-benchmark-convert should parse");
        let Some(Command::Traces(TracesCommand::WorkerBenchmarkConvert {
            purpose, status, ..
        })) = benchmark.command
        else {
            panic!("expected traces worker-benchmark-convert command");
        };
        assert_eq!(purpose, "nightly-worker-benchmark");
        assert_eq!(status, Some(TraceCorpusStatusArg::Accepted));

        let retention = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "worker-retention-maintenance",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "retention-worker",
            "--dry-run",
        ])
        .expect("worker-retention-maintenance should parse");
        let Some(Command::Traces(TracesCommand::WorkerRetentionMaintenance {
            purpose,
            dry_run,
            ..
        })) = retention.command
        else {
            panic!("expected traces worker-retention-maintenance command");
        };
        assert_eq!(purpose.as_deref(), Some("retention-worker"));
        assert!(dry_run);

        let vector = Cli::try_parse_from([
            "ironclaw",
            "traces",
            "worker-vector-index",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "vector-worker",
        ])
        .expect("worker-vector-index should parse");
        let Some(Command::Traces(TracesCommand::WorkerVectorIndex { purpose, .. })) =
            vector.command
        else {
            panic!("expected traces worker-vector-index command");
        };
        assert_eq!(purpose.as_deref(), Some("vector-worker"));
    }

    #[test]
    fn worker_retention_maintenance_body_uses_narrow_fields() {
        let body = trace_commons_retention_maintenance_body(
            Some("retention-cutover".to_string()),
            true,
            false,
            Some(24),
            Some("2026-04-26T00:00:00Z".to_string()),
        );

        assert_eq!(body["purpose"], "retention-cutover");
        assert_eq!(body["dry_run"], true);
        assert_eq!(body["prune_export_cache"], false);
        assert_eq!(body["max_export_age_hours"], 24);
        assert_eq!(body["purge_expired_before"], "2026-04-26T00:00:00Z");
        assert!(body.get("backfill_db_mirror").is_none());
        assert!(body.get("index_vectors").is_none());
        assert!(body.get("reconcile_db_mirror").is_none());
    }

    #[test]
    fn worker_vector_index_body_uses_narrow_fields() {
        let body = trace_commons_vector_index_body(Some("vector-refresh".to_string()), true);

        assert_eq!(body["purpose"], "vector-refresh");
        assert_eq!(body["dry_run"], true);
        assert!(body.get("backfill_db_mirror").is_none());
        assert!(body.get("reconcile_db_mirror").is_none());
    }

    #[test]
    fn replay_export_manifests_use_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/datasets/replay/manifests",
            &[],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/datasets/replay/manifests"
        );
    }

    #[test]
    fn list_traces_purpose_query_uses_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/traces",
            &[
                ("purpose", "ranker_training_pairs_export".to_string()),
                ("limit", "20".to_string()),
            ],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/traces?purpose=ranker_training_pairs_export&limit=20"
        );
    }

    #[test]
    fn maintenance_reconciliation_lines_summarize_counts_without_ids() {
        let value = serde_json::json!({
            "db_reconciliation": {
                "file_submission_count": 3,
                "db_submission_count": 2,
                "missing_submission_ids_in_db": [
                    "11111111-1111-1111-1111-111111111111",
                    "22222222-2222-2222-2222-222222222222"
                ],
                "missing_submission_ids_in_files": [],
                "status_mismatches": [
                    {
                        "submission_id": "33333333-3333-3333-3333-333333333333",
                        "file_status": "accepted",
                        "db_status": "revoked"
                    }
                ],
                "file_derived_count": 4,
                "db_derived_count": 3,
                "missing_derived_submission_ids_in_db": [
                    "44444444-4444-4444-4444-444444444444"
                ],
                "missing_derived_submission_ids_in_files": [
                    "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
                ],
                "derived_status_mismatches": [
                    {
                        "submission_id": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                        "file_status": "revoked",
                        "db_status": "current"
                    }
                ],
                "derived_hash_mismatches": [
                    {
                        "submission_id": "cccccccc-cccc-cccc-cccc-cccccccccccc",
                        "file_canonical_summary_hash": "sha256:file",
                        "db_canonical_summary_hash": "sha256:db"
                    }
                ],
                "db_object_ref_count": 2,
                "accepted_without_active_envelope_object_ref": [
                    "55555555-5555-5555-5555-555555555555"
                ],
                "unreadable_active_envelope_object_refs": [
                    "66666666-6666-6666-6666-666666666666"
                ],
                "hash_mismatched_active_envelope_object_refs": [
                    "77777777-7777-7777-7777-777777777777"
                ],
                "file_credit_event_count": 5,
                "db_credit_event_count": 4,
                "file_audit_event_count": 6,
                "db_audit_event_count": 6,
                "file_replay_export_manifest_count": 1,
                "db_export_manifest_count": 2,
                "db_replay_export_manifest_count": 1,
                "db_benchmark_export_manifest_count": 0,
                "db_ranker_export_manifest_count": 1,
                "db_other_export_manifest_count": 0,
                "db_export_manifest_item_count": 3,
                "file_revocation_tombstone_count": 1,
                "db_tombstone_count": 1,
                "contributor_credit_reader_parity_ok": true,
                "reviewer_metadata_reader_parity_ok": false,
                "analytics_reader_parity_ok": true,
                "audit_reader_parity_ok": true,
                "replay_export_manifest_reader_parity_ok": true,
                "db_reader_parity_failures": [
                    "reviewer_metadata: file_submissions=3 db_submissions=2 file_derived=4 db_derived=3"
                ],
                "active_vector_entries": 7,
                "accepted_current_derived_without_active_vector_entry": [
                    "88888888-8888-8888-8888-888888888888",
                    "99999999-9999-9999-9999-999999999999"
                ],
                "invalid_active_vector_entries": 1,
                "blocking_gaps": [
                    "missing_submission_ids_in_db=2",
                    "reviewer_metadata_reader_parity=failed"
                ]
            }
        });

        let lines = maintenance_reconciliation_lines(&value);

        assert_eq!(
            lines,
            vec![
                "  db reconciliation:".to_string(),
                "    submissions: files=3 db=2 missing_in_db=2 missing_in_files=0 status_mismatches=1".to_string(),
                "    derived: files=4 db=3 missing_in_db=1 missing_in_files=1 status_mismatches=1 hash_mismatches=1".to_string(),
                "    object refs: db=2 accepted_without_active_envelope=1 unreadable_active_envelope=1 hash_mismatched_active_envelope=1".to_string(),
                "    ledger/audit: file_credit_events=5 db_credit_events=4 file_audit_events=6 db_audit_events=6".to_string(),
                "    exports/tombstones: file_replay_manifests=1 db_export_manifests=2 db_replay_manifests=1 db_benchmark_manifests=0 db_ranker_manifests=1 db_other_manifests=0 db_export_items=3 file_revocation_tombstones=1 db_tombstones=1".to_string(),
                "    reader parity: contributor_credit=true reviewer_metadata=false analytics=true audit=true replay_export_manifests=true failures=1".to_string(),
                "    vectors: active=7 eligible_without_active=2 invalid_active=1".to_string(),
                "    blocking: gaps=2".to_string(),
            ]
        );
        let rendered = lines.join("\n");
        assert!(!rendered.contains("11111111-1111-1111-1111-111111111111"));
        assert!(!rendered.contains("33333333-3333-3333-3333-333333333333"));
    }

    #[test]
    fn maintenance_reconciliation_lines_summarize_ledger_audit_gap_counts_without_ids() {
        let value = serde_json::json!({
            "db_reconciliation": {
                "file_credit_event_count": 5,
                "db_credit_event_count": 4,
                "missing_credit_event_ids_in_db": [
                    "dddddddd-dddd-dddd-dddd-dddddddddddd"
                ],
                "missing_credit_event_ids_in_files": [],
                "file_audit_event_count": 6,
                "db_audit_event_count": 5,
                "missing_audit_event_ids_in_db": [
                    "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
                    "ffffffff-ffff-ffff-ffff-ffffffffffff"
                ],
                "missing_audit_event_ids_in_files": [
                    "12121212-1212-1212-1212-121212121212"
                ]
            }
        });

        let lines = maintenance_reconciliation_lines(&value);

        assert_eq!(
            lines,
            vec![
                "  db reconciliation:".to_string(),
                "    ledger/audit: file_credit_events=5 db_credit_events=4 missing_credit_in_db=1 missing_credit_in_files=0 file_audit_events=6 db_audit_events=5 missing_audit_in_db=2 missing_audit_in_files=1".to_string(),
            ]
        );
        let rendered = lines.join("\n");
        assert!(!rendered.contains("dddddddd-dddd-dddd-dddd-dddddddddddd"));
        assert!(!rendered.contains("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"));
        assert!(!rendered.contains("12121212-1212-1212-1212-121212121212"));
    }

    #[test]
    fn maintenance_reconciliation_lines_summarize_export_object_ref_gap_counts_without_ids() {
        let value = serde_json::json!({
            "db_reconciliation": {
                "file_replay_export_manifest_count": 1,
                "db_export_manifest_count": 2,
                "db_replay_export_manifest_count": 1,
                "db_benchmark_export_manifest_count": 0,
                "db_ranker_export_manifest_count": 1,
                "db_other_export_manifest_count": 0,
                "db_export_manifest_item_count": 3,
                "db_export_manifest_item_missing_object_ref_count": 2,
                "db_export_manifest_ids_with_missing_object_refs": [
                    "abababab-abab-abab-abab-abababababab",
                    "cdcdcdcd-cdcd-cdcd-cdcd-cdcdcdcdcdcd"
                ],
                "file_revocation_tombstone_count": 1,
                "db_tombstone_count": 1
            }
        });

        let lines = maintenance_reconciliation_lines(&value);

        assert_eq!(
            lines,
            vec![
                "  db reconciliation:".to_string(),
                "    exports/tombstones: file_replay_manifests=1 db_export_manifests=2 db_replay_manifests=1 db_benchmark_manifests=0 db_ranker_manifests=1 db_other_manifests=0 db_export_items=3 db_export_items_missing_object_refs=2 db_export_manifests_missing_object_refs=2 file_revocation_tombstones=1 db_tombstones=1".to_string(),
            ]
        );
        let rendered = lines.join("\n");
        assert!(!rendered.contains("abababab-abab-abab-abab-abababababab"));
        assert!(!rendered.contains("cdcdcdcd-cdcd-cdcd-cdcd-cdcdcdcdcdcd"));
    }

    #[test]
    fn maintenance_request_body_includes_reconcile_flag() {
        let options = TraceCommonsMaintenanceOptions {
            purpose: "db-read-cutover".to_string(),
            dry_run: true,
            backfill_db_mirror: true,
            index_vectors: true,
            reconcile_db_mirror: true,
            verify_audit_chain: true,
            prune_export_cache: false,
            max_export_age_hours: Some(48),
            purge_expired_before: Some("2026-04-25T00:00:00Z".to_string()),
        };
        let body = trace_commons_maintenance_body(&options);

        assert_eq!(body["purpose"], "db-read-cutover");
        assert_eq!(body["dry_run"], true);
        assert_eq!(body["backfill_db_mirror"], true);
        assert_eq!(body["index_vectors"], true);
        assert_eq!(body["reconcile_db_mirror"], true);
        assert_eq!(body["verify_audit_chain"], true);
        assert_eq!(body["prune_export_cache"], false);
        assert_eq!(body["max_export_age_hours"], 48);
        assert_eq!(body["purge_expired_before"], "2026-04-25T00:00:00Z");
    }

    #[test]
    fn maintenance_audit_chain_lines_summarize_without_hashes() {
        let value = serde_json::json!({
            "audit_chain": {
                "verified": false,
                "event_count": 4,
                "legacy_event_count": 1,
                "mismatch_count": 2,
                "last_event_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "failures": [
                    "line 2: event_hash mismatch",
                    "line 3: previous_event_hash mismatch"
                ],
                "db_mirror": {
                    "verified": false,
                    "event_count": 3,
                    "legacy_event_count": 1,
                    "payload_verified_event_count": 2,
                    "payload_unverified_event_count": 1,
                    "mismatch_count": 1,
                    "last_event_hash": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "failures": [
                        "db row 3: previous_event_hash mismatch"
                    ]
                }
            }
        });

        let lines = maintenance_audit_chain_lines(&value);
        assert_eq!(
            lines,
            vec![
                "  audit chain:".to_string(),
                "    status: verified=false events=4 legacy=1 mismatches=2".to_string(),
                "    failures: 2".to_string(),
                "    db mirror:".to_string(),
                "      status: verified=false events=3 legacy=1 payload_verified=2 payload_unverified=1 mismatches=1".to_string(),
                "      failures: 1".to_string(),
            ]
        );
        let rendered = lines.join("\n");
        assert!(!rendered.contains("sha256:aaaaaaaa"));
        assert!(!rendered.contains("sha256:bbbbbbbb"));
        assert!(!rendered.contains("line 2"));
        assert!(!rendered.contains("db row 3"));
    }
}
