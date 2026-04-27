//! Trace contribution CLI commands.
//!
//! These commands are deliberately opt-in and local-first. `preview` creates a
//! redacted contribution envelope from an existing recorded trace. `submit`
//! only uploads when the user provides an explicit ingestion endpoint.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand, ValueEnum};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::trace_contribution::{
    ConsentScope, CreditSummary, DeterministicTraceRedactor, RecordedTraceContributionOptions,
    StandingTraceContributionPolicy, TraceChannel, TraceContributionAcceptance,
    TraceContributionEnvelope, TraceCreditEvent, TraceCreditEventKind, TraceRedactor,
    TraceSubmissionReceipt, TraceSubmissionStatusUpdate, acknowledge_trace_credit_notice_for_scope,
    estimate_initial_credit, fetch_trace_submission_statuses_with_policy,
    flush_trace_contribution_queue_for_scope, mark_trace_credit_notice_due_for_scope,
    preflight_trace_contribution_policy, privacy_filter_adapter_from_env,
    read_local_trace_records_for_scope, read_trace_policy_for_scope,
    revoke_trace_submission_at_endpoint_with_policy, snooze_trace_credit_notice_for_scope,
    submit_trace_envelope_to_endpoint_with_policy, trace_credit_summary,
    trace_queue_diagnostics_for_scope, trace_submission_status_endpoint,
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

        /// HTTPS issuer URL that returns short-lived EdDSA upload claims
        #[arg(long)]
        upload_token_issuer_url: Option<String>,

        /// Exact allowed issuer hostnames for upload claim refresh
        #[arg(long, value_delimiter = ',')]
        upload_token_issuer_allowed_hosts: Vec<String>,

        /// Audience to request from the upload claim issuer
        #[arg(long)]
        upload_token_audience: Option<String>,

        /// Tenant ID to request from the upload claim issuer
        #[arg(long)]
        upload_token_tenant_id: Option<String>,

        /// Environment variable containing workload credentials for the issuer
        #[arg(long)]
        upload_token_workload_token_env: Option<String>,

        /// Upload claim issuer timeout in milliseconds
        #[arg(long, default_value_t = crate::trace_contribution::TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS)]
        upload_token_issuer_timeout_ms: u64,

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

    /// Show local autonomous trace queue diagnostics
    QueueStatus {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Local tenant/user trace scope to inspect
        #[arg(long)]
        scope: Option<String>,
    },

    /// Show local credit totals and recent credit explanations
    Credit {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Print and mark a due periodic credit notice instead of the full credit report
        #[arg(long)]
        notice: bool,

        /// Local tenant/user trace scope to check for a due periodic notice
        #[arg(long, requires = "notice")]
        notice_scope: Option<String>,

        /// Acknowledge the current credit notice until credit changes again
        #[arg(long, requires = "notice", conflicts_with = "snooze_hours")]
        ack: bool,

        /// Snooze the current credit notice for this many hours
        #[arg(long, requires = "notice", conflicts_with = "ack")]
        snooze_hours: Option<u32>,
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

        /// Filter by review lease state
        #[arg(long, value_enum)]
        lease_filter: Option<TraceReviewLeaseFilterArg>,

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

        /// Filter by review lease state
        #[arg(long, value_enum)]
        lease_filter: Option<TraceReviewLeaseFilterArg>,

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

    /// Claim a DB-backed central trace review lease
    ReviewLeaseClaim {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Submission ID to lease for review
        #[arg(long)]
        submission_id: Uuid,

        /// Optional lease TTL in seconds
        #[arg(long)]
        lease_ttl_seconds: Option<i64>,

        /// Optional RFC3339 review due timestamp
        #[arg(long)]
        review_due_at: Option<String>,

        /// Environment variable containing a reviewer/admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Release a DB-backed central trace review lease
    ReviewLeaseRelease {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Submission ID to release from review
        #[arg(long)]
        submission_id: Uuid,

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

    /// Append idempotent delayed utility credit through the worker route
    WorkerUtilityCredit {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Utility credit event type
        #[arg(long, value_enum)]
        event_type: TraceUtilityCreditEventTypeArg,

        /// Credit point delta
        #[arg(long)]
        credit_points_delta: f32,

        /// Explanation for the worker credit ledger
        #[arg(long)]
        reason: String,

        /// Benchmark/job/external reference used for idempotency
        #[arg(long)]
        external_ref: String,

        /// Accepted submission IDs receiving delayed utility credit
        #[arg(value_name = "SUBMISSION_ID", num_args = 1..)]
        submission_ids: Vec<Uuid>,

        /// Environment variable containing a utility-credit-worker bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Submit process-evaluation metadata through the worker route
    ProcessEvaluationSubmit(ProcessEvaluationSubmitArgs),

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

    /// List DB-backed retention maintenance jobs from the central corpus
    RetentionJobsList {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Maximum retention jobs to return
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by retention job status
        #[arg(long, value_enum)]
        status: Option<TraceRetentionJobStatusArg>,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List DB-backed retention maintenance job items from the central corpus
    RetentionJobItems {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Retention job ID
        retention_job_id: Uuid,

        /// Maximum retention job items to return
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by lifecycle action
        #[arg(long, value_enum)]
        action: Option<TraceRetentionJobItemActionArg>,

        /// Filter by item status
        #[arg(long, value_enum)]
        status: Option<TraceRetentionJobItemStatusArg>,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List DB-backed export access grants from the central corpus
    ExportAccessGrantsList {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Maximum export access grants to return
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by export grant status
        #[arg(long, value_enum)]
        status: Option<TraceExportAccessGrantStatusArg>,

        /// Filter by dataset kind, such as replay_dataset or ranker_training_pairs
        #[arg(long)]
        dataset_kind: Option<String>,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List DB-backed export jobs from the central corpus
    ExportJobsList {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Maximum export jobs to return
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by export job status
        #[arg(long, value_enum)]
        status: Option<TraceExportJobStatusArg>,

        /// Filter by dataset kind, such as replay_dataset or ranker_training_pairs
        #[arg(long)]
        dataset_kind: Option<String>,

        /// Environment variable containing an admin bearer token
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

    /// Export an approved replay dataset slice through the worker route
    WorkerReplayDatasetExport {
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

        /// Environment variable containing an export-worker bearer token
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

    /// List DB-backed tenant access grants for hosted-agent permissioning
    TenantAccessGrantsList {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Maximum tenant access grants to return
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by grant status
        #[arg(long, value_enum)]
        status: Option<TraceTenantAccessGrantStatusArg>,

        /// Filter by grant role
        #[arg(long, value_enum)]
        role: Option<TraceTenantAccessGrantRoleArg>,

        /// Filter by stored principal_ref
        #[arg(long)]
        principal_ref: Option<String>,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a DB-backed tenant access grant for an issuer-authorized principal
    TenantAccessGrantCreate {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Stored principal_ref to authorize
        #[arg(long)]
        principal_ref: String,

        /// Role this grant authorizes
        #[arg(long, value_enum)]
        role: TraceTenantAccessGrantRoleArg,

        /// Optional explicit grant id
        #[arg(long)]
        grant_id: Option<Uuid>,

        /// Allowed consent scopes, comma separated
        #[arg(long, value_enum, value_delimiter = ',')]
        allowed_consent_scopes: Vec<TraceScopeArg>,

        /// Allowed trace-card uses, comma separated
        #[arg(long, value_enum, value_delimiter = ',')]
        allowed_uses: Vec<TraceAllowedUseArg>,

        /// Issuer that authorized the principal
        #[arg(long)]
        issuer: Option<String>,

        /// Audience bound by the issuer
        #[arg(long)]
        audience: Option<String>,

        /// Subject bound by the issuer
        #[arg(long)]
        subject: Option<String>,

        /// RFC3339 issued-at timestamp
        #[arg(long)]
        issued_at: Option<String>,

        /// RFC3339 expiration timestamp
        #[arg(long)]
        expires_at: Option<String>,

        /// Required audit reason
        #[arg(long)]
        reason: String,

        /// Safe grant metadata as key=value, repeatable
        #[arg(long = "metadata", value_name = "KEY=VALUE")]
        metadata: Vec<String>,

        /// Environment variable containing an admin bearer token
        #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
        bearer_token_env: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Revoke a DB-backed tenant access grant
    TenantAccessGrantRevoke {
        /// Trace Commons ingestion base URL or /v1/traces URL
        #[arg(long)]
        endpoint: String,

        /// Grant ID to revoke
        grant_id: Uuid,

        /// Required audit reason
        #[arg(long)]
        reason: String,

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

    /// Export approved ranker training candidates through the worker route
    WorkerRankerTrainingCandidates {
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

        /// Environment variable containing an export-worker bearer token
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

    /// Export approved ranker training pairs through the worker route
    WorkerRankerTrainingPairs {
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

        /// Environment variable containing an export-worker bearer token
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

#[derive(Args, Debug, Clone)]
pub struct ProcessEvaluationSubmitArgs {
    /// Trace Commons ingestion base URL or /v1/traces URL
    #[arg(long)]
    endpoint: String,

    /// Submission ID to attach process-evaluation metadata to
    #[arg(long)]
    submission_id: Uuid,

    /// Non-empty operator reason for the process-evaluation submission
    #[arg(long)]
    reason: String,

    /// Optional evaluator name
    #[arg(long)]
    evaluator_name: Option<String>,

    /// Evaluator version
    #[arg(long)]
    evaluator_version: String,

    /// Process-evaluation labels; repeat for multiple labels
    #[arg(long = "label")]
    labels: Vec<String>,

    /// Rating for tool selection quality
    #[arg(long, value_parser = parse_process_evaluation_rating)]
    tool_selection: Option<TraceProcessEvaluationRatingArg>,

    /// Rating for tool argument quality
    #[arg(long, value_parser = parse_process_evaluation_rating)]
    tool_argument_quality: Option<TraceProcessEvaluationRatingArg>,

    /// Rating for tool ordering
    #[arg(long, value_parser = parse_process_evaluation_rating)]
    tool_ordering: Option<TraceProcessEvaluationRatingArg>,

    /// Rating for verification quality
    #[arg(long, value_parser = parse_process_evaluation_rating)]
    verification: Option<TraceProcessEvaluationRatingArg>,

    /// Rating for side-effect safety
    #[arg(long, value_parser = parse_process_evaluation_rating)]
    side_effect_safety: Option<TraceProcessEvaluationRatingArg>,

    /// Overall process-evaluation score from 0.0 to 1.0
    #[arg(long)]
    overall_score: Option<f32>,

    /// Optional training utility credit delta to append for the evaluated accepted submission
    #[arg(long)]
    utility_credit_points_delta: Option<f32>,

    /// External idempotency reference required when appending process-evaluation utility credit
    #[arg(long)]
    utility_external_ref: Option<String>,

    /// Environment variable containing a process-evaluation-worker bearer token
    #[arg(long, default_value = "IRONCLAW_TRACE_SUBMIT_TOKEN")]
    bearer_token_env: String,

    /// Output as JSON
    #[arg(long)]
    json: bool,
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
pub enum TraceReviewLeaseFilterArg {
    All,
    Mine,
    Available,
    Active,
    Expired,
}

impl std::fmt::Display for TraceReviewLeaseFilterArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::All => "all",
            Self::Mine => "mine",
            Self::Available => "available",
            Self::Active => "active",
            Self::Expired => "expired",
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
pub enum TraceUtilityCreditEventTypeArg {
    RegressionCatch,
    TrainingUtility,
    RankingUtility,
}

impl std::fmt::Display for TraceUtilityCreditEventTypeArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::RegressionCatch => "regression_catch",
            Self::TrainingUtility => "training_utility",
            Self::RankingUtility => "ranking_utility",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceProcessEvaluationRatingArg {
    Pass,
    Partial,
    Fail,
    NotApplicable,
    Unknown,
}

fn parse_process_evaluation_rating(value: &str) -> Result<TraceProcessEvaluationRatingArg, String> {
    match value {
        "pass" => Ok(TraceProcessEvaluationRatingArg::Pass),
        "partial" => Ok(TraceProcessEvaluationRatingArg::Partial),
        "fail" => Ok(TraceProcessEvaluationRatingArg::Fail),
        "not_applicable" => Ok(TraceProcessEvaluationRatingArg::NotApplicable),
        "unknown" => Ok(TraceProcessEvaluationRatingArg::Unknown),
        _ => Err(format!(
            "invalid process evaluation rating {value:?}; expected pass, partial, fail, not_applicable, or unknown"
        )),
    }
}

impl std::fmt::Display for TraceProcessEvaluationRatingArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Pass => "pass",
            Self::Partial => "partial",
            Self::Fail => "fail",
            Self::NotApplicable => "not_applicable",
            Self::Unknown => "unknown",
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

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceRetentionJobStatusArg {
    Planned,
    Running,
    DryRun,
    Complete,
    Failed,
    Paused,
}

impl std::fmt::Display for TraceRetentionJobStatusArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Planned => "planned",
            Self::Running => "running",
            Self::DryRun => "dry_run",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Paused => "paused",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceExportAccessGrantStatusArg {
    Active,
    Consumed,
    Revoked,
    Expired,
}

impl std::fmt::Display for TraceExportAccessGrantStatusArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Active => "active",
            Self::Consumed => "consumed",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceExportJobStatusArg {
    Queued,
    Running,
    Complete,
    Failed,
    Cancelled,
    Expired,
}

impl std::fmt::Display for TraceExportJobStatusArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceTenantAccessGrantRoleArg {
    Contributor,
    Reviewer,
    Admin,
    ExportWorker,
    RetentionWorker,
    VectorWorker,
    BenchmarkWorker,
    UtilityWorker,
    ProcessEvalWorker,
    RevocationWorker,
}

impl TraceTenantAccessGrantRoleArg {
    fn server_value(self) -> &'static str {
        match self {
            Self::Contributor => "contributor",
            Self::Reviewer => "reviewer",
            Self::Admin => "admin",
            Self::ExportWorker => "export_worker",
            Self::RetentionWorker => "retention_worker",
            Self::VectorWorker => "vector_worker",
            Self::BenchmarkWorker => "benchmark_worker",
            Self::UtilityWorker => "utility_worker",
            Self::ProcessEvalWorker => "process_eval_worker",
            Self::RevocationWorker => "revocation_worker",
        }
    }
}

impl std::fmt::Display for TraceTenantAccessGrantRoleArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.server_value())
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceTenantAccessGrantStatusArg {
    Active,
    Revoked,
    Expired,
}

impl std::fmt::Display for TraceTenantAccessGrantStatusArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceRetentionJobItemActionArg {
    Revoke,
    Expire,
    Purge,
}

impl std::fmt::Display for TraceRetentionJobItemActionArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Revoke => "revoke",
            Self::Expire => "expire",
            Self::Purge => "purge",
        };
        write!(f, "{value}")
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceRetentionJobItemStatusArg {
    Pending,
    Done,
    Failed,
    Skipped,
}

impl std::fmt::Display for TraceRetentionJobItemStatusArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        };
        write!(f, "{value}")
    }
}

pub async fn run_traces_command(cmd: TracesCommand) -> anyhow::Result<()> {
    match cmd {
        TracesCommand::OptIn {
            endpoint,
            bearer_token_env,
            upload_token_issuer_url,
            upload_token_issuer_allowed_hosts,
            upload_token_audience,
            upload_token_tenant_id,
            upload_token_workload_token_env,
            upload_token_issuer_timeout_ms,
            include_message_text,
            include_tool_payloads,
            scope,
            selected_tools,
            allow_pii_review_bypass,
            min_submission_score,
        } => opt_in(OptInOptions {
            endpoint,
            bearer_token_env,
            upload_token_issuer_url,
            upload_token_issuer_allowed_hosts,
            upload_token_audience,
            upload_token_tenant_id,
            upload_token_workload_token_env,
            upload_token_issuer_timeout_ms,
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
            let policy = read_policy()?;
            enqueue_envelope_with_policy(
                &envelope,
                &policy,
                TraceContributionAcceptance::QueueFromPreview,
            )?;
            println!(
                "Queued redacted trace contribution {}",
                envelope.submission_id
            );
            Ok(())
        }
        TracesCommand::FlushQueue { limit } => flush_queue(limit).await,
        TracesCommand::QueueStatus { json, scope } => show_queue_status(json, scope.as_deref()),
        TracesCommand::Credit {
            json,
            notice,
            notice_scope,
            ack,
            snooze_hours,
        } => show_credit(json, notice, notice_scope.as_deref(), ack, snooze_hours).await,
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
            lease_filter,
            bearer_token_env,
            json,
        } => trace_commons_quarantine_list(&endpoint, &bearer_token_env, lease_filter, json).await,
        TracesCommand::ActiveLearningReviewQueue {
            endpoint,
            limit,
            privacy_risk,
            lease_filter,
            bearer_token_env,
            json,
        } => {
            trace_commons_active_learning_review_queue(
                &endpoint,
                &bearer_token_env,
                limit,
                privacy_risk,
                lease_filter,
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
        TracesCommand::ReviewLeaseClaim {
            endpoint,
            submission_id,
            lease_ttl_seconds,
            review_due_at,
            bearer_token_env,
            json,
        } => {
            trace_commons_review_lease_claim(TraceCommonsReviewLeaseClaimOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                submission_id,
                lease_ttl_seconds,
                review_due_at,
                json,
            })
            .await
        }
        TracesCommand::ReviewLeaseRelease {
            endpoint,
            submission_id,
            bearer_token_env,
            json,
        } => {
            trace_commons_review_lease_release(&endpoint, &bearer_token_env, submission_id, json)
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
        TracesCommand::WorkerUtilityCredit {
            endpoint,
            event_type,
            credit_points_delta,
            reason,
            external_ref,
            submission_ids,
            bearer_token_env,
            json,
        } => {
            trace_commons_worker_utility_credit(TraceCommonsWorkerUtilityCreditOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                event_type,
                credit_points_delta,
                reason,
                external_ref,
                submission_ids,
                json,
            })
            .await
        }
        TracesCommand::ProcessEvaluationSubmit(args) => {
            trace_commons_process_evaluation_submit(TraceCommonsProcessEvaluationSubmitOptions {
                endpoint: &args.endpoint,
                bearer_token_env: &args.bearer_token_env,
                submission_id: args.submission_id,
                reason: args.reason,
                evaluator_name: args.evaluator_name,
                evaluator_version: args.evaluator_version,
                labels: args.labels,
                tool_selection: args.tool_selection,
                tool_argument_quality: args.tool_argument_quality,
                tool_ordering: args.tool_ordering,
                verification: args.verification,
                side_effect_safety: args.side_effect_safety,
                overall_score: args.overall_score,
                utility_credit_points_delta: args.utility_credit_points_delta,
                utility_external_ref: args.utility_external_ref,
                json: args.json,
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
        TracesCommand::RetentionJobsList {
            endpoint,
            limit,
            status,
            bearer_token_env,
            json,
        } => {
            trace_commons_retention_jobs_list(&endpoint, &bearer_token_env, limit, status, json)
                .await
        }
        TracesCommand::RetentionJobItems {
            endpoint,
            retention_job_id,
            limit,
            action,
            status,
            bearer_token_env,
            json,
        } => {
            trace_commons_retention_job_items(
                &endpoint,
                &bearer_token_env,
                retention_job_id,
                limit,
                action,
                status,
                json,
            )
            .await
        }
        TracesCommand::ExportAccessGrantsList {
            endpoint,
            limit,
            status,
            dataset_kind,
            bearer_token_env,
            json,
        } => {
            trace_commons_export_access_grants_list(
                &endpoint,
                &bearer_token_env,
                limit,
                status,
                dataset_kind,
                json,
            )
            .await
        }
        TracesCommand::ExportJobsList {
            endpoint,
            limit,
            status,
            dataset_kind,
            bearer_token_env,
            json,
        } => {
            trace_commons_export_jobs_list(
                &endpoint,
                &bearer_token_env,
                limit,
                status,
                dataset_kind,
                json,
            )
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
                method: Method::GET,
                path: "/v1/datasets/replay",
            })
            .await
        }
        TracesCommand::WorkerReplayDatasetExport {
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
                method: Method::GET,
                path: "/v1/workers/replay-export",
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
        TracesCommand::TenantAccessGrantsList {
            endpoint,
            limit,
            status,
            role,
            principal_ref,
            bearer_token_env,
            json,
        } => {
            trace_commons_tenant_access_grants_list(
                &endpoint,
                &bearer_token_env,
                limit,
                status,
                role,
                principal_ref,
                json,
            )
            .await
        }
        TracesCommand::TenantAccessGrantCreate {
            endpoint,
            principal_ref,
            role,
            grant_id,
            allowed_consent_scopes,
            allowed_uses,
            issuer,
            audience,
            subject,
            issued_at,
            expires_at,
            reason,
            metadata,
            bearer_token_env,
            json,
        } => {
            trace_commons_tenant_access_grant_create(TraceCommonsTenantAccessGrantCreateOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                principal_ref,
                role,
                grant_id,
                allowed_consent_scopes,
                allowed_uses,
                issuer,
                audience,
                subject,
                issued_at,
                expires_at,
                reason,
                metadata,
                json,
            })
            .await
        }
        TracesCommand::TenantAccessGrantRevoke {
            endpoint,
            grant_id,
            reason,
            bearer_token_env,
            json,
        } => {
            trace_commons_tenant_access_grant_revoke(
                &endpoint,
                &bearer_token_env,
                grant_id,
                reason,
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
        TracesCommand::WorkerRankerTrainingCandidates {
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
                path: "/v1/workers/ranker/training-candidates",
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
        TracesCommand::WorkerRankerTrainingPairs {
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
                path: "/v1/workers/ranker/training-pairs",
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
    upload_token_issuer_url: Option<String>,
    upload_token_issuer_allowed_hosts: Vec<String>,
    upload_token_audience: Option<String>,
    upload_token_tenant_id: Option<String>,
    upload_token_workload_token_env: Option<String>,
    upload_token_issuer_timeout_ms: u64,
    include_message_text: bool,
    include_tool_payloads: bool,
    scope: TraceScopeArg,
    selected_tools: Vec<String>,
    allow_pii_review_bypass: bool,
    min_submission_score: f32,
}

#[derive(Debug, Clone, Serialize)]
struct TraceQueueStatusDiagnostics {
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    policy_ready: bool,
    bearer_token_env: String,
    bearer_token_present: bool,
    upload_token_issuer_configured: bool,
    upload_token_issuer_allowed_hosts_count: usize,
    upload_token_audience_configured: bool,
    upload_token_tenant_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_token_workload_token_env: Option<String>,
    upload_token_workload_token_present: bool,
    include_message_text: bool,
    include_tool_payloads: bool,
    require_manual_approval_when_pii_detected: bool,
    min_submission_score: f32,
    credit_notice_interval_hours: u32,
    selected_tools_count: usize,
    queue: crate::trace_contribution::TraceQueueDiagnostics,
    credit_summary: CreditSummary,
}

fn opt_in(options: OptInOptions) -> anyhow::Result<()> {
    let issuer_url = options
        .upload_token_issuer_url
        .and_then(|url| (!url.trim().is_empty()).then_some(url));
    let workload_token_env = options
        .upload_token_workload_token_env
        .and_then(|env| (!env.trim().is_empty()).then_some(env));
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        ingestion_endpoint: Some(options.endpoint),
        bearer_token_env: options.bearer_token_env,
        upload_token_issuer_url: issuer_url,
        upload_token_issuer_allowed_hosts: options
            .upload_token_issuer_allowed_hosts
            .into_iter()
            .map(|host| host.trim().to_ascii_lowercase())
            .filter(|host| !host.is_empty())
            .collect::<BTreeSet<_>>(),
        upload_token_audience: options
            .upload_token_audience
            .and_then(|audience| (!audience.trim().is_empty()).then_some(audience)),
        upload_token_tenant_id: options
            .upload_token_tenant_id
            .and_then(|tenant_id| (!tenant_id.trim().is_empty()).then_some(tenant_id)),
        upload_token_workload_token_env: workload_token_env,
        upload_token_issuer_timeout_ms: options.upload_token_issuer_timeout_ms,
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
    println!(
        "  upload claim issuer configured: {}",
        policy.upload_token_issuer_url.is_some()
    );
    if policy.upload_token_issuer_url.is_some() {
        println!(
            "  upload claim allowed hosts: {}",
            policy.upload_token_issuer_allowed_hosts.len()
        );
        println!(
            "  upload claim audience configured: {}",
            policy.upload_token_audience.is_some()
        );
        println!(
            "  upload claim tenant configured: {}",
            policy.upload_token_tenant_id.is_some()
        );
        println!(
            "  upload claim workload token env: {}",
            policy
                .upload_token_workload_token_env
                .as_deref()
                .unwrap_or("not configured")
        );
    }
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

fn show_queue_status(json: bool, scope: Option<&str>) -> anyhow::Result<()> {
    let diagnostics = trace_queue_status_diagnostics(scope)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&diagnostics).map_err(|e| {
                anyhow::anyhow!("failed to serialize trace queue diagnostics: {}", e)
            })?
        );
        return Ok(());
    }

    println!("Trace contribution queue status:");
    println!(
        "  scope: {}",
        diagnostics.scope.as_deref().unwrap_or("default")
    );
    println!("  policy ready: {}", diagnostics.policy_ready);
    println!("  opt-in enabled: {}", diagnostics.queue.policy_enabled);
    println!(
        "  endpoint configured: {}",
        diagnostics.queue.endpoint_configured
    );
    println!(
        "  bearer token env: {} ({})",
        diagnostics.bearer_token_env,
        if diagnostics.bearer_token_present {
            "set"
        } else {
            "not set"
        }
    );
    println!(
        "  upload claim issuer configured: {}",
        diagnostics.upload_token_issuer_configured
    );
    if diagnostics.upload_token_issuer_configured {
        println!(
            "  upload claim allowed hosts: {}",
            diagnostics.upload_token_issuer_allowed_hosts_count
        );
        println!(
            "  upload claim audience configured: {}",
            diagnostics.upload_token_audience_configured
        );
        println!(
            "  upload claim tenant configured: {}",
            diagnostics.upload_token_tenant_configured
        );
        println!(
            "  upload claim workload token env: {} ({})",
            diagnostics
                .upload_token_workload_token_env
                .as_deref()
                .unwrap_or("not configured"),
            if diagnostics.upload_token_workload_token_present {
                "set"
            } else {
                "not set"
            }
        );
    }
    println!(
        "  include message text: {}",
        diagnostics.include_message_text
    );
    println!(
        "  include tool payloads: {}",
        diagnostics.include_tool_payloads
    );
    println!(
        "  manual review when PII detected: {}",
        diagnostics.require_manual_approval_when_pii_detected
    );
    println!(
        "  min submission score: {:.2}",
        diagnostics.min_submission_score
    );
    println!(
        "  credit notice interval: {} hour(s)",
        diagnostics.credit_notice_interval_hours
    );
    println!("  selected tools: {}", diagnostics.selected_tools_count);
    println!("  queued envelopes: {}", diagnostics.queue.queued_count);
    println!("  held envelopes: {}", diagnostics.queue.held_count);
    println!(
        "  retry scheduled: {}",
        diagnostics.queue.retry_scheduled_count
    );
    println!(
        "  manual review holds: {}",
        diagnostics.queue.manual_review_hold_count
    );
    println!("  policy holds: {}", diagnostics.queue.policy_hold_count);
    if let Some(next_retry_at) = diagnostics.queue.next_retry_at {
        println!("  next retry at: {}", next_retry_at.to_rfc3339());
    }
    if let Some(compaction) = &diagnostics.queue.telemetry.last_compaction {
        println!(
            "  last compaction reclaimed: {} item(s)",
            compaction
                .duplicate_envelopes_removed
                .saturating_add(compaction.orphan_hold_sidecars_removed)
        );
        println!(
            "    duplicate envelopes removed: {}",
            compaction.duplicate_envelopes_removed
        );
        println!(
            "    orphan hold sidecars removed: {}",
            compaction.orphan_hold_sidecars_removed
        );
    }
    if let Some(at) = diagnostics.queue.telemetry.last_flush_attempt_at {
        println!("  last flush attempt: {}", at.to_rfc3339());
    }
    if let Some(at) = diagnostics.queue.telemetry.last_successful_flush_at {
        println!("  last successful flush: {}", at.to_rfc3339());
    }
    if let Some(at) = diagnostics.queue.telemetry.last_failed_flush_at {
        println!("  last failed flush: {}", at.to_rfc3339());
    }
    if diagnostics.queue.telemetry.consecutive_flush_failures > 0 {
        println!(
            "  consecutive flush failures: {}",
            diagnostics.queue.telemetry.consecutive_flush_failures
        );
    }
    if diagnostics
        .queue
        .telemetry
        .retryable_submission_failure_count
        > 0
    {
        println!(
            "  retryable submission failures: {}",
            diagnostics
                .queue
                .telemetry
                .retryable_submission_failure_count
        );
    }
    if diagnostics.queue.telemetry.status_sync_failure_count > 0 {
        println!(
            "  status sync failures: {}",
            diagnostics.queue.telemetry.status_sync_failure_count
        );
    }
    if let Some(failure) = &diagnostics.queue.telemetry.last_failure {
        println!(
            "  last telemetry failure: {:?} {}",
            failure.kind, failure.reason
        );
    }
    if !diagnostics.queue.warnings.is_empty() {
        println!("  queue warnings:");
        for warning in &diagnostics.queue.warnings {
            println!(
                "    {:?} ({:?}, promotion blocking: {}): {}",
                warning.kind, warning.severity, warning.promotion_blocking, warning.message
            );
            println!("      action: {}", warning.recommended_action);
        }
    }
    println!("  submitted records: {}", diagnostics.queue.submitted_count);
    println!("  revoked records: {}", diagnostics.queue.revoked_count);
    println!("  expired records: {}", diagnostics.queue.expired_count);
    println!("  ready to flush: {}", diagnostics.queue.ready_to_flush);
    if !diagnostics.queue.held_reason_counts.is_empty() {
        println!("  held reasons:");
        for (reason, count) in &diagnostics.queue.held_reason_counts {
            println!("    {reason}: {count}");
        }
    }
    println!("  local submissions and credit:");
    print_credit_summary_fields(&diagnostics.credit_summary, "    ");
    Ok(())
}

fn trace_queue_status_diagnostics(
    scope: Option<&str>,
) -> anyhow::Result<TraceQueueStatusDiagnostics> {
    let normalized_scope = scope.and_then(|scope| {
        let trimmed = scope.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let scope_ref = normalized_scope.as_deref();
    let policy = read_trace_policy_for_scope(scope_ref)?;
    let bearer_token_present = !policy.bearer_token_env.trim().is_empty()
        && std::env::var_os(&policy.bearer_token_env).is_some();
    let upload_token_issuer_configured = policy
        .upload_token_issuer_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty());
    let upload_token_workload_token_present = policy
        .upload_token_workload_token_env
        .as_deref()
        .filter(|env| !env.trim().is_empty())
        .is_some_and(|env| std::env::var_os(env).is_some());
    let issuer_credentials_ready = upload_token_issuer_configured
        && (!policy.upload_token_issuer_allowed_hosts.is_empty())
        && policy
            .upload_token_workload_token_env
            .as_deref()
            .is_none_or(|env| env.trim().is_empty() || std::env::var_os(env).is_some());
    let queue = trace_queue_diagnostics_for_scope(scope_ref)?;
    let local_records = read_local_trace_records_for_scope(scope_ref)?;

    Ok(TraceQueueStatusDiagnostics {
        scope: normalized_scope,
        policy_ready: queue.ready_to_flush && (bearer_token_present || issuer_credentials_ready),
        bearer_token_env: policy.bearer_token_env,
        bearer_token_present,
        upload_token_issuer_configured,
        upload_token_issuer_allowed_hosts_count: policy.upload_token_issuer_allowed_hosts.len(),
        upload_token_audience_configured: policy.upload_token_audience.is_some(),
        upload_token_tenant_configured: policy.upload_token_tenant_id.is_some(),
        upload_token_workload_token_env: policy.upload_token_workload_token_env,
        upload_token_workload_token_present,
        include_message_text: policy.include_message_text,
        include_tool_payloads: policy.include_tool_payloads,
        require_manual_approval_when_pii_detected: policy.require_manual_approval_when_pii_detected,
        min_submission_score: policy.min_submission_score,
        credit_notice_interval_hours: policy.credit_notice_interval_hours,
        selected_tools_count: policy.selected_tools.len(),
        queue,
        credit_summary: trace_credit_summary(&local_records),
    })
}

async fn preview_recorded_trace(options: PreviewOptions) -> anyhow::Result<()> {
    let queue_policy = if options.enqueue {
        let policy = read_policy()?;
        preflight_cli_trace_upload(
            &policy,
            TraceContributionAcceptance::QueueFromPreview,
            options.include_message_text,
            options.include_tool_payloads,
        )?;
        Some(policy)
    } else {
        None
    };

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
        let policy = queue_policy.as_ref().ok_or_else(|| {
            anyhow::anyhow!("trace contribution queue policy was not initialized")
        })?;
        enqueue_envelope_with_policy(
            &envelope,
            policy,
            TraceContributionAcceptance::QueueFromPreview,
        )?;
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
    let mut policy = read_policy()?;
    policy.ingestion_endpoint = Some(endpoint.to_string());
    policy.bearer_token_env = bearer_token_env.to_string();
    let receipt =
        submit_trace_envelope_to_endpoint_with_policy(&envelope, endpoint, &policy).await?;

    record_submitted_envelope(&envelope, endpoint, receipt)?;

    println!(
        "Submitted redacted trace contribution {}",
        envelope.submission_id
    );
    Ok(())
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

async fn revoke_submission(
    submission_id: Uuid,
    endpoint: Option<&str>,
    bearer_token_env: &str,
) -> anyhow::Result<()> {
    if let Some(endpoint) = endpoint {
        let mut policy = read_policy()?;
        policy.bearer_token_env = bearer_token_env.to_string();
        revoke_trace_submission_at_endpoint_with_policy(submission_id, endpoint, &policy).await?;
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
    lease_filter: Option<TraceReviewLeaseFilterArg>,
    json: bool,
) -> anyhow::Result<()> {
    let mut query = Vec::new();
    if let Some(lease_filter) = lease_filter {
        query.push(("lease_filter", lease_filter.to_string()));
    }
    let response = trace_commons_api_request(
        Method::GET,
        endpoint,
        "/v1/review/quarantine",
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
        "Central quarantined traces",
        response.json.as_ref(),
        &[
            "submission_id",
            "privacy_risk",
            "submission_score",
            "review_escalation_state",
            "review_age_hours",
            "review_escalation_reasons",
            "review_assigned_to_principal_ref",
            "review_lease_expires_at",
            "review_due_at",
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
    lease_filter: Option<TraceReviewLeaseFilterArg>,
    json: bool,
) -> anyhow::Result<()> {
    let mut query = optional_usize_query("limit", limit);
    if let Some(privacy_risk) = privacy_risk {
        query.push(("privacy_risk", privacy_risk.to_string()));
    }
    if let Some(lease_filter) = lease_filter {
        query.push(("lease_filter", lease_filter.to_string()));
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
            "review_escalation_state",
            "review_age_hours",
            "review_escalation_reasons",
            "review_assigned_to_principal_ref",
            "review_lease_expires_at",
            "review_due_at",
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

struct TraceCommonsReviewLeaseClaimOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    submission_id: Uuid,
    lease_ttl_seconds: Option<i64>,
    review_due_at: Option<String>,
    json: bool,
}

async fn trace_commons_review_lease_claim(
    options: TraceCommonsReviewLeaseClaimOptions<'_>,
) -> anyhow::Result<()> {
    let body =
        trace_commons_review_lease_claim_body(options.lease_ttl_seconds, options.review_due_at)?;
    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        &format!("/v1/review/{}/lease", options.submission_id),
        &[],
        Some(options.bearer_token_env),
        Some(body),
    )
    .await?;
    if options.json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    println!("Claimed central review lease for {}", options.submission_id);
    print_trace_commons_review_lease_fields(response.json.as_ref());
    Ok(())
}

fn trace_commons_review_lease_claim_body(
    lease_ttl_seconds: Option<i64>,
    review_due_at: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    let mut body = serde_json::Map::new();
    if let Some(lease_ttl_seconds) = lease_ttl_seconds {
        if lease_ttl_seconds <= 0 {
            anyhow::bail!("--lease-ttl-seconds must be greater than 0");
        }
        body.insert(
            "lease_ttl_seconds".to_string(),
            serde_json::json!(lease_ttl_seconds),
        );
    }
    if let Some(review_due_at) = review_due_at
        .map(|timestamp| timestamp.trim().to_string())
        .filter(|timestamp| !timestamp.is_empty())
    {
        chrono::DateTime::parse_from_rfc3339(&review_due_at)
            .map_err(|error| anyhow::anyhow!("--review-due-at must be RFC3339: {error}"))?;
        body.insert(
            "review_due_at".to_string(),
            serde_json::Value::String(review_due_at),
        );
    }
    Ok(serde_json::Value::Object(body))
}

async fn trace_commons_review_lease_release(
    endpoint: &str,
    bearer_token_env: &str,
    submission_id: Uuid,
    json: bool,
) -> anyhow::Result<()> {
    let response = trace_commons_api_request(
        Method::DELETE,
        endpoint,
        &format!("/v1/review/{submission_id}/lease"),
        &[],
        Some(bearer_token_env),
        None,
    )
    .await?;
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    println!("Released central review lease for {submission_id}");
    print_trace_commons_review_lease_fields(response.json.as_ref());
    Ok(())
}

fn print_trace_commons_review_lease_fields(value: Option<&serde_json::Value>) {
    if let Some(value) = value {
        print_optional_json_field("  tenant id", value, "tenant_id");
        print_optional_json_field("  tenant storage ref", value, "tenant_storage_ref");
        print_optional_json_field("  trace id", value, "trace_id");
        print_optional_json_field("  status", value, "status");
        print_optional_json_field(
            "  assigned principal",
            value,
            "review_assigned_to_principal_ref",
        );
        print_optional_json_field("  assigned at", value, "review_assigned_at");
        print_optional_json_field("  lease expires at", value, "review_lease_expires_at");
        print_optional_json_field("  review due at", value, "review_due_at");
    }
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

struct TraceCommonsWorkerUtilityCreditOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    event_type: TraceUtilityCreditEventTypeArg,
    credit_points_delta: f32,
    reason: String,
    external_ref: String,
    submission_ids: Vec<Uuid>,
    json: bool,
}

async fn trace_commons_worker_utility_credit(
    options: TraceCommonsWorkerUtilityCreditOptions<'_>,
) -> anyhow::Result<()> {
    let body = trace_commons_worker_utility_credit_body(
        options.event_type,
        options.credit_points_delta,
        options.reason,
        options.external_ref,
        options.submission_ids,
    );

    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        "/v1/workers/utility-credit",
        &[],
        Some(options.bearer_token_env),
        Some(body),
    )
    .await?;
    if options.json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }

    println!("Trace Commons utility credit worker complete.");
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  appended", value, "appended_count");
        print_optional_json_field("  skipped existing", value, "skipped_existing_count");
    }
    Ok(())
}

fn trace_commons_worker_utility_credit_body(
    event_type: TraceUtilityCreditEventTypeArg,
    credit_points_delta: f32,
    reason: String,
    external_ref: String,
    submission_ids: Vec<Uuid>,
) -> serde_json::Value {
    serde_json::json!({
        "event_type": event_type.to_string(),
        "credit_points_delta": credit_points_delta,
        "reason": reason,
        "external_ref": external_ref,
        "submission_ids": submission_ids,
    })
}

struct TraceCommonsProcessEvaluationSubmitOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    submission_id: Uuid,
    reason: String,
    evaluator_name: Option<String>,
    evaluator_version: String,
    labels: Vec<String>,
    tool_selection: Option<TraceProcessEvaluationRatingArg>,
    tool_argument_quality: Option<TraceProcessEvaluationRatingArg>,
    tool_ordering: Option<TraceProcessEvaluationRatingArg>,
    verification: Option<TraceProcessEvaluationRatingArg>,
    side_effect_safety: Option<TraceProcessEvaluationRatingArg>,
    overall_score: Option<f32>,
    utility_credit_points_delta: Option<f32>,
    utility_external_ref: Option<String>,
    json: bool,
}

async fn trace_commons_process_evaluation_submit(
    options: TraceCommonsProcessEvaluationSubmitOptions<'_>,
) -> anyhow::Result<()> {
    let body = trace_commons_process_evaluation_body(
        options.submission_id,
        options.reason,
        options.evaluator_name,
        options.evaluator_version,
        options.labels,
        options.tool_selection,
        options.tool_argument_quality,
        options.tool_ordering,
        options.verification,
        options.side_effect_safety,
        options.overall_score,
        options.utility_credit_points_delta,
        options.utility_external_ref,
    )?;

    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        "/v1/workers/process-evaluation",
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
        "Trace Commons process evaluation submitted for {}.",
        options.submission_id
    );
    if let Some(value) = response.json.as_ref() {
        print_optional_json_field("  tenant", value, "tenant_id");
        print_optional_json_field("  tenant storage ref", value, "tenant_storage_ref");
        print_optional_json_field("  trace id", value, "trace_id");
        print_optional_json_field("  status", value, "status");
        print_optional_json_field("  process eval value", value, "process_eval_value");
        print_optional_json_field("  review scorecard", value, "review_scorecard");
        print_optional_json_field("  output object ref", value, "output_object_ref_id");
        print_optional_json_field(
            "  utility credit appended",
            value,
            "utility_credit_appended_count",
        );
        print_optional_json_field(
            "  utility credit skipped existing",
            value,
            "utility_credit_skipped_existing_count",
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn trace_commons_process_evaluation_body(
    submission_id: Uuid,
    reason: String,
    evaluator_name: Option<String>,
    evaluator_version: String,
    labels: Vec<String>,
    tool_selection: Option<TraceProcessEvaluationRatingArg>,
    tool_argument_quality: Option<TraceProcessEvaluationRatingArg>,
    tool_ordering: Option<TraceProcessEvaluationRatingArg>,
    verification: Option<TraceProcessEvaluationRatingArg>,
    side_effect_safety: Option<TraceProcessEvaluationRatingArg>,
    overall_score: Option<f32>,
    utility_credit_points_delta: Option<f32>,
    utility_external_ref: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    require_non_empty_reason(&reason)?;
    if evaluator_version.trim().is_empty() {
        anyhow::bail!("--evaluator-version must not be empty");
    }
    if let Some(overall_score) = overall_score
        && !(0.0..=1.0).contains(&overall_score)
    {
        anyhow::bail!("--overall-score must be between 0.0 and 1.0");
    }
    let utility_external_ref = if let Some(delta) = utility_credit_points_delta {
        if !delta.is_finite() || delta.abs() > 100.0 {
            anyhow::bail!("--utility-credit-points-delta must be finite with abs <= 100.0");
        }
        let external_ref = utility_external_ref
            .as_deref()
            .map(str::trim)
            .filter(|external_ref| !external_ref.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--utility-external-ref must be non-empty when --utility-credit-points-delta is set"
                )
        })?;
        Some(external_ref.to_string())
    } else {
        if utility_external_ref
            .as_deref()
            .is_some_and(|external_ref| !external_ref.trim().is_empty())
        {
            anyhow::bail!("--utility-external-ref requires --utility-credit-points-delta");
        }
        None
    };

    let mut process_evaluation = serde_json::json!({
        "evaluator_version": evaluator_version.trim(),
        "labels": labels
            .into_iter()
            .map(|label| label.trim().to_string())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>(),
    });
    if let Some(evaluator_name) = evaluator_name
        && !evaluator_name.trim().is_empty()
    {
        process_evaluation["evaluator_name"] =
            serde_json::Value::String(evaluator_name.trim().to_string());
    }
    if let Some(tool_selection) = tool_selection {
        process_evaluation["tool_selection"] =
            serde_json::Value::String(tool_selection.to_string());
    }
    if let Some(tool_argument_quality) = tool_argument_quality {
        process_evaluation["tool_argument_quality"] =
            serde_json::Value::String(tool_argument_quality.to_string());
    }
    if let Some(tool_ordering) = tool_ordering {
        process_evaluation["tool_ordering"] = serde_json::Value::String(tool_ordering.to_string());
    }
    if let Some(verification) = verification {
        process_evaluation["verification"] = serde_json::Value::String(verification.to_string());
    }
    if let Some(side_effect_safety) = side_effect_safety {
        process_evaluation["side_effect_safety"] =
            serde_json::Value::String(side_effect_safety.to_string());
    }
    if let Some(overall_score) = overall_score {
        process_evaluation["overall_score"] = serde_json::json!(overall_score);
    }

    let mut body = serde_json::json!({
        "submission_id": submission_id,
        "process_evaluation": process_evaluation,
        "reason": reason.trim(),
    });
    if let (Some(delta), Some(external_ref)) = (utility_credit_points_delta, utility_external_ref) {
        body["utility_credit_points_delta"] = serde_json::json!(delta);
        body["utility_external_ref"] = serde_json::Value::String(external_ref);
    }
    Ok(body)
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
    print_optional_json_field("  min cell count", value, "min_cell_count");
    print_optional_json_field("  suppressed cells", value, "suppressed_cell_count");
    print_optional_json_field("  duplicate groups", value, "duplicate_groups");
    print_optional_json_field("  average novelty", value, "average_novelty_score");
    print_json_map("  by status", value.get("by_status"));
    print_json_map("  by privacy risk", value.get("by_privacy_risk"));
    print_json_map("  by task success", value.get("by_task_success"));
    if let Some(process_evaluation) = value.get("process_evaluation") {
        print_optional_json_field(
            "  process evaluated traces",
            process_evaluation,
            "evaluated_traces",
        );
        print_json_map("  process labels", process_evaluation.get("by_label"));
        print_json_map("  process ratings", process_evaluation.get("by_rating"));
        print_json_map(
            "  process score bands",
            process_evaluation.get("by_score_band"),
        );
    }
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
        for line in trace_commons_maintenance_summary_lines(value) {
            println!("{line}");
        }
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

fn trace_commons_maintenance_summary_lines(value: &serde_json::Value) -> Vec<String> {
    json_field_lines(
        value,
        &[
            ("  audit event id", "audit_event_id"),
            ("  purpose", "purpose"),
            ("  dry run", "dry_run"),
            ("  revoked submissions", "revoked_submission_count"),
            ("  expired submissions", "expired_submission_count"),
            ("  records marked revoked", "records_marked_revoked"),
            ("  records marked expired", "records_marked_expired"),
            ("  records marked purged", "records_marked_purged"),
            ("  derived marked revoked", "derived_marked_revoked"),
            ("  derived marked expired", "derived_marked_expired"),
            ("  export cache files pruned", "export_cache_files_pruned"),
            (
                "  export provenance invalidated",
                "export_provenance_invalidated",
            ),
            (
                "  benchmark artifacts invalidated",
                "benchmark_artifacts_invalidated",
            ),
            ("  trace object files deleted", "trace_object_files_deleted"),
            (
                "  encrypted artifacts deleted",
                "encrypted_artifacts_deleted",
            ),
            ("  DB mirror backfilled", "db_mirror_backfilled"),
            ("  vectors indexed", "vector_entries_indexed"),
        ],
    )
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
        for line in trace_commons_retention_maintenance_summary_lines(value) {
            println!("{line}");
        }
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

fn trace_commons_retention_maintenance_summary_lines(value: &serde_json::Value) -> Vec<String> {
    json_field_lines(
        value,
        &[
            ("  audit event id", "audit_event_id"),
            ("  purpose", "purpose"),
            ("  dry run", "dry_run"),
            ("  records marked revoked", "records_marked_revoked"),
            ("  records marked expired", "records_marked_expired"),
            ("  records marked purged", "records_marked_purged"),
            ("  export cache files pruned", "export_cache_files_pruned"),
            (
                "  export provenance invalidated",
                "export_provenance_invalidated",
            ),
            (
                "  benchmark artifacts invalidated",
                "benchmark_artifacts_invalidated",
            ),
        ],
    )
}

async fn trace_commons_retention_jobs_list(
    endpoint: &str,
    bearer_token_env: &str,
    limit: Option<usize>,
    status: Option<TraceRetentionJobStatusArg>,
    json: bool,
) -> anyhow::Result<()> {
    let mut query = optional_usize_query("limit", limit);
    if let Some(status) = status {
        query.push(("status", status.to_string()));
    }
    let Some(response) = trace_commons_optional_api_request(
        Method::GET,
        endpoint,
        "/v1/admin/retention/jobs",
        &query,
        Some(bearer_token_env),
        None,
    )
    .await?
    else {
        print_trace_commons_unsupported("/v1/admin/retention/jobs");
        return Ok(());
    };

    if json {
        print_trace_commons_json(&response)
    } else {
        print_trace_commons_items(
            "Central retention maintenance jobs",
            response.json.as_ref(),
            &[
                "retention_job_id",
                "status",
                "purpose",
                "dry_run",
                "selected_revoked_count",
                "selected_expired_count",
                "started_at",
                "completed_at",
                "created_at",
            ],
        );
        Ok(())
    }
}

async fn trace_commons_retention_job_items(
    endpoint: &str,
    bearer_token_env: &str,
    retention_job_id: Uuid,
    limit: Option<usize>,
    action: Option<TraceRetentionJobItemActionArg>,
    status: Option<TraceRetentionJobItemStatusArg>,
    json: bool,
) -> anyhow::Result<()> {
    let mut query = optional_usize_query("limit", limit);
    if let Some(action) = action {
        query.push(("action", action.to_string()));
    }
    if let Some(status) = status {
        query.push(("status", status.to_string()));
    }
    let path = format!("/v1/admin/retention/jobs/{retention_job_id}/items");
    let Some(response) = trace_commons_optional_api_request(
        Method::GET,
        endpoint,
        &path,
        &query,
        Some(bearer_token_env),
        None,
    )
    .await?
    else {
        print_trace_commons_unsupported(&path);
        return Ok(());
    };

    if json {
        print_trace_commons_json(&response)
    } else {
        print_trace_commons_items(
            "Central retention maintenance job items",
            response.json.as_ref(),
            &[
                "submission_id",
                "action",
                "status",
                "reason",
                "verified_at",
                "updated_at",
            ],
        );
        Ok(())
    }
}

async fn trace_commons_export_access_grants_list(
    endpoint: &str,
    bearer_token_env: &str,
    limit: Option<usize>,
    status: Option<TraceExportAccessGrantStatusArg>,
    dataset_kind: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let mut query = optional_usize_query("limit", limit);
    if let Some(status) = status {
        query.push(("status", status.to_string()));
    }
    if let Some(dataset_kind) = dataset_kind {
        query.push(("dataset_kind", dataset_kind));
    }
    let Some(response) = trace_commons_optional_api_request(
        Method::GET,
        endpoint,
        "/v1/admin/export/access-grants",
        &query,
        Some(bearer_token_env),
        None,
    )
    .await?
    else {
        print_trace_commons_unsupported("/v1/admin/export/access-grants");
        return Ok(());
    };

    if json {
        print_trace_commons_json(&response)
    } else {
        print_trace_commons_items(
            "Central export access grants",
            response.json.as_ref(),
            &[
                "grant_id",
                "export_job_id",
                "status",
                "requested_dataset_kind",
                "purpose",
                "max_item_cap",
                "requested_at",
                "expires_at",
            ],
        );
        Ok(())
    }
}

async fn trace_commons_export_jobs_list(
    endpoint: &str,
    bearer_token_env: &str,
    limit: Option<usize>,
    status: Option<TraceExportJobStatusArg>,
    dataset_kind: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let mut query = optional_usize_query("limit", limit);
    if let Some(status) = status {
        query.push(("status", status.to_string()));
    }
    if let Some(dataset_kind) = dataset_kind {
        query.push(("dataset_kind", dataset_kind));
    }
    let Some(response) = trace_commons_optional_api_request(
        Method::GET,
        endpoint,
        "/v1/admin/export/jobs",
        &query,
        Some(bearer_token_env),
        None,
    )
    .await?
    else {
        print_trace_commons_unsupported("/v1/admin/export/jobs");
        return Ok(());
    };

    if json {
        print_trace_commons_json(&response)
    } else {
        print_trace_commons_items(
            "Central export jobs",
            response.json.as_ref(),
            &[
                "export_job_id",
                "status",
                "requested_dataset_kind",
                "purpose",
                "item_count",
                "result_manifest_id",
                "started_at",
                "finished_at",
            ],
        );
        Ok(())
    }
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
    method: Method,
    path: &'a str,
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
        options.method,
        options.endpoint,
        options.path,
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

async fn trace_commons_tenant_access_grants_list(
    endpoint: &str,
    bearer_token_env: &str,
    limit: Option<usize>,
    status: Option<TraceTenantAccessGrantStatusArg>,
    role: Option<TraceTenantAccessGrantRoleArg>,
    principal_ref: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let mut query = optional_usize_query("limit", limit);
    if let Some(status) = status {
        query.push(("status", status.to_string()));
    }
    if let Some(role) = role {
        query.push(("role", role.to_string()));
    }
    if let Some(principal_ref) = principal_ref
        .map(|principal_ref| principal_ref.trim().to_string())
        .filter(|principal_ref| !principal_ref.is_empty())
    {
        query.push(("principal_ref", principal_ref));
    }
    let Some(response) = trace_commons_optional_api_request(
        Method::GET,
        endpoint,
        "/v1/admin/tenant-access-grants",
        &query,
        Some(bearer_token_env),
        None,
    )
    .await?
    else {
        print_trace_commons_unsupported("/v1/admin/tenant-access-grants");
        return Ok(());
    };

    if json {
        print_trace_commons_json(&response)
    } else {
        print_trace_commons_items(
            "Central tenant access grants",
            response.json.as_ref(),
            &[
                "grant_id",
                "principal_ref",
                "role",
                "status",
                "allowed_consent_scopes",
                "allowed_uses",
                "issuer",
                "subject",
                "expires_at",
                "revoked_at",
            ],
        );
        Ok(())
    }
}

struct TraceCommonsTenantAccessGrantCreateOptions<'a> {
    endpoint: &'a str,
    bearer_token_env: &'a str,
    principal_ref: String,
    role: TraceTenantAccessGrantRoleArg,
    grant_id: Option<Uuid>,
    allowed_consent_scopes: Vec<TraceScopeArg>,
    allowed_uses: Vec<TraceAllowedUseArg>,
    issuer: Option<String>,
    audience: Option<String>,
    subject: Option<String>,
    issued_at: Option<String>,
    expires_at: Option<String>,
    reason: String,
    metadata: Vec<String>,
    json: bool,
}

async fn trace_commons_tenant_access_grant_create(
    options: TraceCommonsTenantAccessGrantCreateOptions<'_>,
) -> anyhow::Result<()> {
    let body = trace_commons_tenant_access_grant_body(&options)?;
    let response = trace_commons_api_request(
        Method::POST,
        options.endpoint,
        "/v1/admin/tenant-access-grants",
        &[],
        Some(options.bearer_token_env),
        Some(body),
    )
    .await?;
    print_trace_commons_tenant_access_grant_response(
        response,
        options.json,
        "Trace Commons tenant access grant created.",
    )
}

fn trace_commons_tenant_access_grant_body(
    options: &TraceCommonsTenantAccessGrantCreateOptions<'_>,
) -> anyhow::Result<serde_json::Value> {
    let principal_ref = options.principal_ref.trim();
    if principal_ref.is_empty() {
        anyhow::bail!("--principal-ref must not be empty");
    }
    require_non_empty_reason(&options.reason)?;
    if let Some(issued_at) = options.issued_at.as_deref() {
        require_rfc3339_timestamp("--issued-at", issued_at)?;
    }
    if let Some(expires_at) = options.expires_at.as_deref() {
        require_rfc3339_timestamp("--expires-at", expires_at)?;
    }
    let mut body = serde_json::json!({
        "principal_ref": principal_ref,
        "role": options.role.server_value(),
        "allowed_consent_scopes": options
            .allowed_consent_scopes
            .iter()
            .copied()
            .map(trace_scope_server_value)
            .collect::<Vec<_>>(),
        "allowed_uses": options
            .allowed_uses
            .iter()
            .copied()
            .map(TraceAllowedUseArg::server_value)
            .collect::<Vec<_>>(),
        "reason": options.reason.trim(),
    });
    if let Some(grant_id) = options.grant_id {
        body["grant_id"] = serde_json::Value::String(grant_id.to_string());
    }
    if let Some(issuer) = trimmed_optional_cli_string(options.issuer.as_deref()) {
        body["issuer"] = serde_json::Value::String(issuer);
    }
    if let Some(audience) = trimmed_optional_cli_string(options.audience.as_deref()) {
        body["audience"] = serde_json::Value::String(audience);
    }
    if let Some(subject) = trimmed_optional_cli_string(options.subject.as_deref()) {
        body["subject"] = serde_json::Value::String(subject);
    }
    if let Some(issued_at) = trimmed_optional_cli_string(options.issued_at.as_deref()) {
        body["issued_at"] = serde_json::Value::String(issued_at);
    }
    if let Some(expires_at) = trimmed_optional_cli_string(options.expires_at.as_deref()) {
        body["expires_at"] = serde_json::Value::String(expires_at);
    }
    let metadata = parse_trace_commons_metadata(&options.metadata)?;
    if !metadata.is_empty() {
        body["metadata"] = serde_json::to_value(metadata)
            .map_err(|error| anyhow::anyhow!("failed to encode grant metadata: {error}"))?;
    }
    Ok(body)
}

async fn trace_commons_tenant_access_grant_revoke(
    endpoint: &str,
    bearer_token_env: &str,
    grant_id: Uuid,
    reason: String,
    json: bool,
) -> anyhow::Result<()> {
    require_non_empty_reason(&reason)?;
    let response = trace_commons_api_request(
        Method::POST,
        endpoint,
        &format!("/v1/admin/tenant-access-grants/{grant_id}/revoke"),
        &[],
        Some(bearer_token_env),
        Some(serde_json::json!({ "reason": reason.trim() })),
    )
    .await?;
    print_trace_commons_tenant_access_grant_response(
        response,
        json,
        "Trace Commons tenant access grant revoked.",
    )
}

fn print_trace_commons_tenant_access_grant_response(
    response: TraceCommonsApiResponse,
    json: bool,
    heading: &str,
) -> anyhow::Result<()> {
    if json {
        print_trace_commons_json(&response)?;
        return Ok(());
    }
    let Some(value) = response.json.as_ref() else {
        println!("{}", response.body.trim());
        return Ok(());
    };
    println!("{heading}");
    print_optional_json_field("  tenant", value, "tenant_id");
    print_optional_json_field("  grant id", value, "grant_id");
    print_optional_json_field("  principal", value, "principal_ref");
    print_optional_json_field("  role", value, "role");
    print_optional_json_field("  status", value, "status");
    print_optional_json_field("  allowed consent scopes", value, "allowed_consent_scopes");
    print_optional_json_field("  allowed uses", value, "allowed_uses");
    print_optional_json_field("  issuer", value, "issuer");
    print_optional_json_field("  audience", value, "audience");
    print_optional_json_field("  subject", value, "subject");
    print_optional_json_field("  expires at", value, "expires_at");
    print_optional_json_field("  revoked at", value, "revoked_at");
    Ok(())
}

fn parse_trace_commons_metadata(values: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut metadata = BTreeMap::new();
    for value in values {
        let Some((key, raw_value)) = value.split_once('=') else {
            anyhow::bail!("--metadata values must use KEY=VALUE");
        };
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("--metadata keys must not be empty");
        }
        metadata.insert(key.to_string(), raw_value.trim().to_string());
    }
    Ok(metadata)
}

fn require_rfc3339_timestamp(label: &str, value: &str) -> anyhow::Result<()> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{label} must not be empty");
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|error| anyhow::anyhow!("{label} must be RFC3339: {error}"))?;
    Ok(())
}

fn trimmed_optional_cli_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
            "review_assigned_to_principal_ref",
            "review_lease_expires_at",
            "review_due_at",
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

fn require_non_empty_reason(reason: &str) -> anyhow::Result<()> {
    if reason.trim().is_empty() {
        anyhow::bail!("--reason must not be empty");
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
    if let Some(line) = json_field_line(label, value, field) {
        println!("{line}");
    }
}

fn json_field_lines(value: &serde_json::Value, fields: &[(&str, &str)]) -> Vec<String> {
    fields
        .iter()
        .filter_map(|(label, field)| json_field_line(label, value, field))
        .collect()
}

fn json_field_line(label: &str, value: &serde_json::Value, field: &str) -> Option<String> {
    json_field_display(value, field).map(|display| format!("{label}: {display}"))
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
            ("db_retention_job_count", "db_retention_jobs"),
            ("db_retention_job_item_count", "db_retention_items"),
            (
                "missing_current_retention_job_ids_in_db",
                "missing_current_retention_jobs",
            ),
            (
                "current_retention_job_item_count_mismatches",
                "retention_item_mismatches",
            ),
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
    let report = flush_trace_contribution_queue_for_scope(None, limit).await?;
    println!(
        "Autonomous trace queue flush complete: {} submitted, {} held.",
        report.submitted, report.held
    );
    for hold in report.holds.iter().take(5) {
        println!(
            "  held {} ({:?}, attempts {}): {}",
            hold.submission_id, hold.kind, hold.attempts, hold.reason
        );
    }
    if let Some(summary) = report.credit_notice {
        println!("{}", credit_notice_message(&summary));
        for explanation in summary.recent_explanations.iter().take(3) {
            println!("  - {explanation}");
        }
    }
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
    let updates =
        fetch_trace_submission_statuses_with_policy(&status_endpoint, policy, &submission_ids)
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
    let changed = apply_cli_submission_status_updates_to_records(&mut records, updates, now);

    if changed > 0 {
        write_local_records(&records)?;
    }
    Ok(changed)
}

fn apply_cli_submission_status_updates_to_records(
    records: &mut [LocalSubmissionRecord],
    updates: &[TraceSubmissionStatusUpdate],
    now: chrono::DateTime<chrono::Utc>,
) -> usize {
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
        let explanation_changed =
            !explanation.is_empty() && record.credit_explanation != explanation;

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
                contributor_pseudonym: "cli-sync".to_string(),
                kind: TraceCreditEventKind::CreditSynced,
                points_delta: new_effective_credit - old_effective_credit,
                reason: sync_reason,
                created_at: now,
            });
            changed += 1;
        }
    }

    changed
}

fn credit_notice_message(summary: &CreditSummary) -> String {
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
    message
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

async fn show_credit(
    json: bool,
    notice: bool,
    notice_scope: Option<&str>,
    ack: bool,
    snooze_hours: Option<u32>,
) -> anyhow::Result<()> {
    if notice {
        return show_credit_notice(json, notice_scope, ack, snooze_hours);
    }

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

fn show_credit_notice(
    json: bool,
    scope: Option<&str>,
    ack: bool,
    snooze_hours: Option<u32>,
) -> anyhow::Result<()> {
    let notice = if ack {
        acknowledge_trace_credit_notice_for_scope(scope)?
    } else if let Some(hours) = snooze_hours {
        snooze_trace_credit_notice_for_scope(scope, chrono::Duration::hours(i64::from(hours)))?
    } else {
        mark_trace_credit_notice_due_for_scope(scope)?
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&notice)
                .map_err(|e| anyhow::anyhow!("failed to serialize credit notice: {}", e))?
        );
        return Ok(());
    }

    let Some(summary) = notice else {
        println!("No trace contribution credit notice due.");
        return Ok(());
    };

    if ack {
        println!("Acknowledged trace contribution credit notice.");
        print_credit_summary_fields(&summary, "  ");
    } else if let Some(hours) = snooze_hours {
        println!("Snoozed trace contribution credit notice for {hours} hour(s).");
        print_credit_summary_fields(&summary, "  ");
    } else {
        println!("{}", credit_notice_message(&summary));
        for explanation in summary.recent_explanations.iter().take(3) {
            println!("  - {explanation}");
        }
    }
    Ok(())
}

fn print_credit_summary_fields(summary: &CreditSummary, indent: &str) {
    println!("{indent}submissions: {}", summary.submissions_total);
    println!("{indent}submitted: {}", summary.submissions_submitted);
    println!("{indent}revoked: {}", summary.submissions_revoked);
    println!("{indent}expired: {}", summary.submissions_expired);
    println!("{indent}pending credit: +{:.2}", summary.pending_credit);
    println!("{indent}final credit: +{:.2}", summary.final_credit);
    println!(
        "{indent}delayed ledger: {:+.2}",
        summary.delayed_credit_delta
    );
    if summary.credit_events_total > 0 {
        println!("{indent}credit events: {}", summary.credit_events_total);
    }
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
        delayed_credit_delta: records
            .iter()
            .flat_map(|record| record.credit_events.iter())
            .filter(|event| event.kind != TraceCreditEventKind::Accepted)
            .map(|event| event.points_delta)
            .sum(),
        credit_events_total: records
            .iter()
            .map(|record| record.credit_events.len() as u32)
            .sum(),
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

fn preflight_cli_trace_upload(
    policy: &StandingTraceContributionPolicy,
    intent: TraceContributionAcceptance,
    include_message_text: bool,
    include_tool_payloads: bool,
) -> anyhow::Result<()> {
    preflight_trace_contribution_policy(policy, intent)
        .map_err(|rejection| anyhow::anyhow!("{rejection}"))?;
    if intent != TraceContributionAcceptance::PreviewOnly {
        if include_message_text && !policy.include_message_text {
            anyhow::bail!("trace contribution policy does not allow message text upload");
        }
        if include_tool_payloads && !policy.include_tool_payloads {
            anyhow::bail!("trace contribution policy does not allow tool payload upload");
        }
    }
    Ok(())
}

fn preflight_cli_trace_envelope_upload(
    policy: &StandingTraceContributionPolicy,
    intent: TraceContributionAcceptance,
    envelope: &TraceContributionEnvelope,
) -> anyhow::Result<()> {
    preflight_cli_trace_upload(
        policy,
        intent,
        envelope.consent.message_text_included,
        envelope.consent.tool_payloads_included,
    )
}

fn enqueue_envelope_with_policy(
    envelope: &TraceContributionEnvelope,
    policy: &StandingTraceContributionPolicy,
    intent: TraceContributionAcceptance,
) -> anyhow::Result<PathBuf> {
    enqueue_envelope_to_dir_with_policy(envelope, policy, intent, &queue_dir())
}

fn enqueue_envelope_to_dir_with_policy(
    envelope: &TraceContributionEnvelope,
    policy: &StandingTraceContributionPolicy,
    intent: TraceContributionAcceptance,
    dir: &Path,
) -> anyhow::Result<PathBuf> {
    preflight_cli_trace_envelope_upload(policy, intent, envelope)?;
    enqueue_envelope_to_dir(envelope, dir)
}

fn enqueue_envelope_to_dir(
    envelope: &TraceContributionEnvelope,
    dir: &Path,
) -> anyhow::Result<PathBuf> {
    let path = dir.join(format!("{}.json", envelope.submission_id));
    std::fs::create_dir_all(dir).map_err(|e| {
        anyhow::anyhow!(
            "failed to create trace contribution queue {}: {}",
            dir.display(),
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
    use crate::trace_contribution::{
        ConsentMetadata, ContributorMetadata, DETERMINISTIC_REDACTION_PIPELINE_VERSION,
        IronclawTraceMetadata, OutcomeMetadata, PrivacyMetadata, ReplayMetadata, ResidualPiiRisk,
        TRACE_CONTRIBUTION_POLICY_VERSION, TRACE_CONTRIBUTION_SCHEMA_VERSION, TraceCard,
        TraceValueCard, ValueMetadata,
    };
    use clap::Parser;

    fn unwrap_traces_command(cli: Cli) -> TracesCommand {
        let Some(Command::Traces(command)) = cli.command else {
            panic!("expected traces command");
        };
        *command
    }

    fn parse_cli_result<const N: usize>(args: [&'static str; N]) -> Result<Cli, clap::Error> {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || Cli::try_parse_from(args))
            .expect("parser thread should spawn")
            .join()
            .expect("parser thread should not panic")
    }

    fn parse_cli<const N: usize>(args: [&'static str; N]) -> Cli {
        parse_cli_result(args).expect("CLI args should parse")
    }

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

    fn trace_queue_policy_fixture() -> StandingTraceContributionPolicy {
        StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example/internal/v1/traces".to_string()),
            ..StandingTraceContributionPolicy::default()
        }
    }

    fn trace_queue_envelope_fixture(
        message_text_included: bool,
        tool_payloads_included: bool,
    ) -> TraceContributionEnvelope {
        TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at: chrono::Utc::now(),
            ironclaw: IronclawTraceMetadata {
                version: env!("CARGO_PKG_VERSION").to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: vec![ConsentScope::DebuggingEvaluation],
                message_text_included,
                tool_payloads_included,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: None,
                tenant_scope_ref: None,
                credit_account_ref: None,
                revocation_handle: Uuid::new_v4(),
            },
            privacy: PrivacyMetadata {
                redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
                redaction_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: ResidualPiiRisk::Low,
                redaction_hash: "sha256:test".to_string(),
                warnings: Vec::new(),
            },
            events: Vec::new(),
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: false,
                required_tools: Vec::new(),
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
            trace_card: TraceCard::default(),
            value_card: TraceValueCard::default(),
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        }
    }

    #[test]
    fn cli_trace_upload_preflight_keeps_preview_local_but_gates_queue_intents() {
        let envelope = trace_queue_envelope_fixture(true, true);
        let policy = StandingTraceContributionPolicy::default();

        preflight_cli_trace_envelope_upload(
            &policy,
            TraceContributionAcceptance::PreviewOnly,
            &envelope,
        )
        .expect("local preview remains available before opt-in");
        let error = preflight_cli_trace_envelope_upload(
            &policy,
            TraceContributionAcceptance::QueueFromPreview,
            &envelope,
        )
        .expect_err("queueing requires standing opt-in");

        assert!(error.to_string().contains("opt-in is disabled"));
    }

    #[test]
    fn cli_enqueue_rejects_capture_fields_disallowed_by_policy_before_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let policy = trace_queue_policy_fixture();
        let envelope = trace_queue_envelope_fixture(true, false);
        let path = dir.path().join(format!("{}.json", envelope.submission_id));

        let error = enqueue_envelope_to_dir_with_policy(
            &envelope,
            &policy,
            TraceContributionAcceptance::QueueFromPreview,
            dir.path(),
        )
        .expect_err("message text requires standing opt-in capture permission");

        assert!(error.to_string().contains("message text upload"));
        assert!(!path.exists(), "rejected envelopes must not be queued");
    }

    #[test]
    fn cli_enqueue_rejects_tool_payloads_disallowed_by_policy_before_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let policy = trace_queue_policy_fixture();
        let envelope = trace_queue_envelope_fixture(false, true);
        let path = dir.path().join(format!("{}.json", envelope.submission_id));

        let error = enqueue_envelope_to_dir_with_policy(
            &envelope,
            &policy,
            TraceContributionAcceptance::QueueFromPreview,
            dir.path(),
        )
        .expect_err("tool payloads require standing opt-in capture permission");

        assert!(error.to_string().contains("tool payload upload"));
        assert!(!path.exists(), "rejected envelopes must not be queued");
    }

    #[test]
    fn cli_enqueue_accepts_policy_matching_envelope_capture() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut policy = trace_queue_policy_fixture();
        policy.include_message_text = true;
        let envelope = trace_queue_envelope_fixture(true, false);

        let path = enqueue_envelope_to_dir_with_policy(
            &envelope,
            &policy,
            TraceContributionAcceptance::QueueFromPreview,
            dir.path(),
        )
        .expect("matching standing policy should queue envelope");

        assert!(path.exists());
    }

    #[test]
    fn list_submissions_summary_flag_parses_through_cli() {
        let cli = parse_cli(["ironclaw", "traces", "list-submissions", "--summary"]);

        let TracesCommand::ListSubmissions { json, summary } = unwrap_traces_command(cli) else {
            panic!("expected traces list-submissions command");
        };

        assert!(!json);
        assert!(summary);
    }

    #[test]
    fn credit_notice_flags_parse_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "credit",
            "--notice",
            "--notice-scope",
            "tenant-a:user-alice",
        ]);

        let TracesCommand::Credit {
            json,
            notice,
            notice_scope,
            ack,
            snooze_hours,
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces credit command");
        };

        assert!(!json);
        assert!(notice);
        assert_eq!(notice_scope.as_deref(), Some("tenant-a:user-alice"));
        assert!(!ack);
        assert_eq!(snooze_hours, None);
    }

    #[test]
    fn credit_notice_action_flags_parse_through_cli() {
        let ack_cli = parse_cli(["ironclaw", "traces", "credit", "--notice", "--ack"]);
        let TracesCommand::Credit {
            notice: ack_notice,
            ack,
            snooze_hours,
            ..
        } = unwrap_traces_command(ack_cli)
        else {
            panic!("expected traces credit command");
        };
        assert!(ack_notice);
        assert!(ack);
        assert_eq!(snooze_hours, None);

        let snooze_cli = parse_cli([
            "ironclaw",
            "traces",
            "credit",
            "--notice",
            "--snooze-hours",
            "24",
        ]);
        let TracesCommand::Credit {
            notice: snooze_notice,
            ack,
            snooze_hours,
            ..
        } = unwrap_traces_command(snooze_cli)
        else {
            panic!("expected traces credit command");
        };
        assert!(snooze_notice);
        assert!(!ack);
        assert_eq!(snooze_hours, Some(24));
    }

    #[test]
    fn queue_status_flags_parse_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "queue-status",
            "--json",
            "--scope",
            "tenant-a:user-alice",
        ]);

        let TracesCommand::QueueStatus { json, scope } = unwrap_traces_command(cli) else {
            panic!("expected traces queue-status command");
        };

        assert!(json);
        assert_eq!(scope.as_deref(), Some("tenant-a:user-alice"));
    }

    #[test]
    fn opt_in_upload_claim_issuer_flags_parse_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "opt-in",
            "--endpoint",
            "https://trace.example/v1/traces",
            "--upload-token-issuer-url",
            "https://issuer.example/v1/trace-upload-claim",
            "--upload-token-issuer-allowed-hosts",
            "issuer.example",
            "--upload-token-audience",
            "trace-commons",
            "--upload-token-tenant-id",
            "tenant-a",
            "--upload-token-workload-token-env",
            "IRONCLAW_TRACE_WORKLOAD_TOKEN",
            "--upload-token-issuer-timeout-ms",
            "7000",
        ]);

        let TracesCommand::OptIn {
            upload_token_issuer_url,
            upload_token_issuer_allowed_hosts,
            upload_token_audience,
            upload_token_tenant_id,
            upload_token_workload_token_env,
            upload_token_issuer_timeout_ms,
            ..
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces opt-in command");
        };

        assert_eq!(
            upload_token_issuer_url.as_deref(),
            Some("https://issuer.example/v1/trace-upload-claim")
        );
        assert_eq!(upload_token_issuer_allowed_hosts, vec!["issuer.example"]);
        assert_eq!(upload_token_audience.as_deref(), Some("trace-commons"));
        assert_eq!(upload_token_tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(
            upload_token_workload_token_env.as_deref(),
            Some("IRONCLAW_TRACE_WORKLOAD_TOKEN")
        );
        assert_eq!(upload_token_issuer_timeout_ms, 7000);
    }

    #[test]
    fn active_learning_review_queue_uses_lease_filter_query() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/review/active-learning",
            &[
                ("limit", "25".to_string()),
                ("lease_filter", "available".to_string()),
            ],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/review/active-learning?limit=25&lease_filter=available"
        );
    }

    #[test]
    fn quarantine_list_lease_filter_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "quarantine-list",
            "--endpoint",
            "https://trace.example/v1/traces",
            "--lease-filter",
            "mine",
        ]);

        let TracesCommand::QuarantineList { lease_filter, .. } = unwrap_traces_command(cli) else {
            panic!("expected traces quarantine-list command");
        };

        assert_eq!(lease_filter, Some(TraceReviewLeaseFilterArg::Mine));
    }

    #[test]
    fn review_lease_claim_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "review-lease-claim",
            "--endpoint",
            "https://trace.example/internal",
            "--submission-id",
            "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
            "--lease-ttl-seconds",
            "900",
            "--review-due-at",
            "2026-04-26T12:00:00Z",
            "--bearer-token-env",
            "TRACE_COMMONS_REVIEWER_TOKEN",
            "--json",
        ]);

        let TracesCommand::ReviewLeaseClaim {
            endpoint,
            submission_id,
            lease_ttl_seconds,
            review_due_at,
            bearer_token_env,
            json,
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces review-lease-claim command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert_eq!(
            submission_id,
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses")
        );
        assert_eq!(lease_ttl_seconds, Some(900));
        assert_eq!(review_due_at.as_deref(), Some("2026-04-26T12:00:00Z"));
        assert_eq!(bearer_token_env, "TRACE_COMMONS_REVIEWER_TOKEN");
        assert!(json);
    }

    #[test]
    fn review_lease_release_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "review-lease-release",
            "--endpoint",
            "https://trace.example/internal",
            "--submission-id",
            "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
            "--bearer-token-env",
            "TRACE_COMMONS_REVIEWER_TOKEN",
        ]);

        let TracesCommand::ReviewLeaseRelease {
            endpoint,
            submission_id,
            bearer_token_env,
            json,
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces review-lease-release command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert_eq!(
            submission_id,
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses")
        );
        assert_eq!(bearer_token_env, "TRACE_COMMONS_REVIEWER_TOKEN");
        assert!(!json);
    }

    #[test]
    fn review_lease_uses_submission_route_from_ingest_endpoint() {
        let submission_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses");
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            &format!("/v1/review/{submission_id}/lease"),
            &[],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/review/018f2b7b-0c11-72fd-95c4-1f9f98feac01/lease"
        );
    }

    #[test]
    fn review_lease_claim_body_omits_absent_optional_fields() {
        let body = trace_commons_review_lease_claim_body(None, None).expect("body builds");

        assert!(body.as_object().expect("body is object").is_empty());
    }

    #[test]
    fn review_lease_claim_body_includes_optional_fields() {
        let body = trace_commons_review_lease_claim_body(
            Some(900),
            Some("2026-04-26T12:00:00Z".to_string()),
        )
        .expect("body builds");

        assert_eq!(body["lease_ttl_seconds"], 900);
        assert_eq!(body["review_due_at"], "2026-04-26T12:00:00Z");
    }

    #[test]
    fn review_lease_claim_body_rejects_invalid_optional_fields() {
        let ttl_error = trace_commons_review_lease_claim_body(Some(0), None)
            .expect_err("non-positive TTL is rejected");
        assert!(ttl_error.to_string().contains("greater than 0"));

        let due_at_error =
            trace_commons_review_lease_claim_body(None, Some("tomorrow-ish".to_string()))
                .expect_err("non-RFC3339 due timestamp is rejected");
        assert!(due_at_error.to_string().contains("RFC3339"));
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
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "ranker-training-candidates",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-ranker-candidates",
        ]);

        let TracesCommand::RankerTrainingCandidates { purpose, .. } = unwrap_traces_command(cli)
        else {
            panic!("expected traces ranker-training-candidates command");
        };

        assert_eq!(purpose.as_deref(), Some("nightly-ranker-candidates"));
    }

    #[test]
    fn worker_ranker_training_candidates_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "worker-ranker-training-candidates",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-worker-candidates",
            "--bearer-token-env",
            "TRACE_COMMONS_EXPORT_WORKER_TOKEN",
        ]);

        let TracesCommand::WorkerRankerTrainingCandidates {
            purpose,
            bearer_token_env,
            ..
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces worker-ranker-training-candidates command");
        };

        assert_eq!(purpose.as_deref(), Some("nightly-worker-candidates"));
        assert_eq!(bearer_token_env, "TRACE_COMMONS_EXPORT_WORKER_TOKEN");
    }

    #[test]
    fn ranker_training_pairs_purpose_flag_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "ranker-training-pairs",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-ranker-pairs",
        ]);

        let TracesCommand::RankerTrainingPairs { purpose, .. } = unwrap_traces_command(cli) else {
            panic!("expected traces ranker-training-pairs command");
        };

        assert_eq!(purpose.as_deref(), Some("nightly-ranker-pairs"));
    }

    #[test]
    fn worker_ranker_training_pairs_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "worker-ranker-training-pairs",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-worker-pairs",
            "--bearer-token-env",
            "TRACE_COMMONS_EXPORT_WORKER_TOKEN",
        ]);

        let TracesCommand::WorkerRankerTrainingPairs {
            purpose,
            bearer_token_env,
            ..
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces worker-ranker-training-pairs command");
        };

        assert_eq!(purpose.as_deref(), Some("nightly-worker-pairs"));
        assert_eq!(bearer_token_env, "TRACE_COMMONS_EXPORT_WORKER_TOKEN");
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
    fn worker_ranker_training_candidates_use_dedicated_route() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1",
            "/v1/workers/ranker/training-candidates",
            &[("purpose", "nightly-worker-candidates".to_string())],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/workers/ranker/training-candidates?purpose=nightly-worker-candidates"
        );
    }

    #[test]
    fn worker_ranker_training_pairs_use_dedicated_route() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1",
            "/v1/workers/ranker/training-pairs",
            &[("privacy_risk", "low".to_string())],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/workers/ranker/training-pairs?privacy_risk=low"
        );
    }

    #[test]
    fn tenant_policy_get_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "tenant-policy-get",
            "--endpoint",
            "https://trace.example/internal",
            "--json",
        ]);

        let TracesCommand::TenantPolicyGet { endpoint, json, .. } = unwrap_traces_command(cli)
        else {
            panic!("expected traces tenant-policy-get command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert!(json);
    }

    #[test]
    fn config_status_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "config-status",
            "--endpoint",
            "https://trace.example/internal",
            "--bearer-token-env",
            "TRACE_COMMONS_ADMIN_TOKEN",
        ]);

        let TracesCommand::ConfigStatus {
            endpoint,
            bearer_token_env,
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces config-status command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
    }

    #[test]
    fn tenant_policy_set_parses_through_cli() {
        let cli = parse_cli([
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
        ]);

        let TracesCommand::TenantPolicySet {
            policy_version,
            allowed_consent_scopes,
            allowed_uses,
            ..
        } = unwrap_traces_command(cli)
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
    fn tenant_access_grants_list_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "tenant-access-grants-list",
            "--endpoint",
            "https://trace.example/internal",
            "--limit",
            "25",
            "--status",
            "active",
            "--role",
            "contributor",
            "--principal-ref",
            "principal_sha256:abc",
            "--bearer-token-env",
            "TRACE_COMMONS_ADMIN_TOKEN",
            "--json",
        ]);

        let TracesCommand::TenantAccessGrantsList {
            endpoint,
            limit,
            status,
            role,
            principal_ref,
            bearer_token_env,
            json,
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces tenant-access-grants-list command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert_eq!(limit, Some(25));
        assert_eq!(status, Some(TraceTenantAccessGrantStatusArg::Active));
        assert_eq!(role, Some(TraceTenantAccessGrantRoleArg::Contributor));
        assert_eq!(principal_ref.as_deref(), Some("principal_sha256:abc"));
        assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
        assert!(json);
    }

    #[test]
    fn tenant_access_grant_create_parses_through_cli() {
        let grant_id = "018f2b7b-0c11-72fd-95c4-1f9f98feac01";
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "tenant-access-grant-create",
            "--endpoint",
            "https://trace.example/internal",
            "--principal-ref",
            "principal_sha256:abc",
            "--role",
            "process-eval-worker",
            "--grant-id",
            grant_id,
            "--allowed-consent-scopes",
            "debugging-evaluation,ranking-training",
            "--allowed-uses",
            "debugging,ranking-model-training",
            "--issuer",
            "https://issuer.near.com",
            "--audience",
            "trace-commons",
            "--subject",
            "tenant-a-agent",
            "--issued-at",
            "2026-04-27T00:00:00Z",
            "--expires-at",
            "2026-04-28T00:00:00Z",
            "--reason",
            "hosted tenant verified",
            "--metadata",
            "hosted_surface=near.com",
            "--bearer-token-env",
            "TRACE_COMMONS_ADMIN_TOKEN",
        ]);

        let TracesCommand::TenantAccessGrantCreate {
            principal_ref,
            role,
            grant_id: parsed_grant_id,
            allowed_consent_scopes,
            allowed_uses,
            issuer,
            audience,
            subject,
            issued_at,
            expires_at,
            reason,
            metadata,
            bearer_token_env,
            ..
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces tenant-access-grant-create command");
        };

        assert_eq!(principal_ref, "principal_sha256:abc");
        assert_eq!(role, TraceTenantAccessGrantRoleArg::ProcessEvalWorker);
        assert_eq!(parsed_grant_id, Some(Uuid::parse_str(grant_id).unwrap()));
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
        assert_eq!(issuer.as_deref(), Some("https://issuer.near.com"));
        assert_eq!(audience.as_deref(), Some("trace-commons"));
        assert_eq!(subject.as_deref(), Some("tenant-a-agent"));
        assert_eq!(issued_at.as_deref(), Some("2026-04-27T00:00:00Z"));
        assert_eq!(expires_at.as_deref(), Some("2026-04-28T00:00:00Z"));
        assert_eq!(reason, "hosted tenant verified");
        assert_eq!(metadata, vec!["hosted_surface=near.com"]);
        assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
    }

    #[test]
    fn tenant_access_grant_revoke_parses_through_cli() {
        let grant_id = "018f2b7b-0c11-72fd-95c4-1f9f98feac01";
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "tenant-access-grant-revoke",
            "--endpoint",
            "https://trace.example/internal",
            grant_id,
            "--reason",
            "tenant deprovisioned",
            "--bearer-token-env",
            "TRACE_COMMONS_ADMIN_TOKEN",
            "--json",
        ]);

        let TracesCommand::TenantAccessGrantRevoke {
            grant_id: parsed_grant_id,
            reason,
            bearer_token_env,
            json,
            ..
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces tenant-access-grant-revoke command");
        };

        assert_eq!(parsed_grant_id, Uuid::parse_str(grant_id).unwrap());
        assert_eq!(reason, "tenant deprovisioned");
        assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
        assert!(json);
    }

    #[test]
    fn tenant_access_grants_use_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/admin/tenant-access-grants",
            &[
                ("status", "active".to_string()),
                ("role", "export_worker".to_string()),
            ],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/admin/tenant-access-grants?status=active&role=export_worker"
        );
    }

    #[test]
    fn tenant_access_grant_revoke_uses_ingest_endpoint() {
        let grant_id = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").unwrap();
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            &format!("/v1/admin/tenant-access-grants/{grant_id}/revoke"),
            &[],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/admin/tenant-access-grants/018f2b7b-0c11-72fd-95c4-1f9f98feac01/revoke"
        );
    }

    #[test]
    fn tenant_access_grant_body_uses_server_enum_names_and_metadata() {
        let options = TraceCommonsTenantAccessGrantCreateOptions {
            endpoint: "https://trace.example/internal",
            bearer_token_env: "TRACE_COMMONS_ADMIN_TOKEN",
            principal_ref: "principal_sha256:abc".to_string(),
            role: TraceTenantAccessGrantRoleArg::ExportWorker,
            grant_id: Some(Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").unwrap()),
            allowed_consent_scopes: vec![
                TraceScopeArg::DebuggingEvaluation,
                TraceScopeArg::BenchmarkOnly,
            ],
            allowed_uses: vec![
                TraceAllowedUseArg::Debugging,
                TraceAllowedUseArg::AggregateAnalytics,
                TraceAllowedUseArg::RankingModelTraining,
            ],
            issuer: Some(" https://issuer.near.com ".to_string()),
            audience: Some(" trace-commons ".to_string()),
            subject: Some(" tenant-a-agent ".to_string()),
            issued_at: Some("2026-04-27T00:00:00Z".to_string()),
            expires_at: Some("2026-04-28T00:00:00Z".to_string()),
            reason: " hosted tenant verified ".to_string(),
            metadata: vec![
                "hosted_surface=near.com".to_string(),
                "region=iad".to_string(),
            ],
            json: false,
        };

        let body = trace_commons_tenant_access_grant_body(&options).expect("body builds");
        assert_eq!(body["principal_ref"], "principal_sha256:abc");
        assert_eq!(body["role"], "export_worker");
        assert_eq!(
            body["allowed_consent_scopes"],
            serde_json::json!(["debugging_evaluation", "benchmark_only"])
        );
        assert_eq!(
            body["allowed_uses"],
            serde_json::json!(["debugging", "aggregate_analytics", "ranking_model_training"])
        );
        assert_eq!(body["issuer"], "https://issuer.near.com");
        assert_eq!(body["audience"], "trace-commons");
        assert_eq!(body["subject"], "tenant-a-agent");
        assert_eq!(body["reason"], "hosted tenant verified");
        assert_eq!(body["metadata"]["hosted_surface"], "near.com");
        assert_eq!(body["metadata"]["region"], "iad");
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
    fn worker_replay_dataset_export_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "worker-replay-dataset-export",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-worker-replay",
            "--bearer-token-env",
            "TRACE_COMMONS_EXPORT_WORKER_TOKEN",
        ]);

        let TracesCommand::WorkerReplayDatasetExport {
            purpose,
            bearer_token_env,
            ..
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces worker-replay-dataset-export command");
        };

        assert_eq!(purpose.as_deref(), Some("nightly-worker-replay"));
        assert_eq!(bearer_token_env, "TRACE_COMMONS_EXPORT_WORKER_TOKEN");
    }

    #[test]
    fn worker_replay_dataset_export_uses_dedicated_route() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/workers/replay-export",
            &[("purpose", "nightly-worker-replay".to_string())],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/workers/replay-export?purpose=nightly-worker-replay"
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
        let cli = parse_cli([
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
        ]);

        let TracesCommand::BenchmarkLifecycleUpdate {
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
        } = unwrap_traces_command(cli)
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
        let benchmark = parse_cli([
            "ironclaw",
            "traces",
            "worker-benchmark-convert",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "nightly-worker-benchmark",
            "--status",
            "accepted",
        ]);
        let TracesCommand::WorkerBenchmarkConvert {
            purpose, status, ..
        } = unwrap_traces_command(benchmark)
        else {
            panic!("expected traces worker-benchmark-convert command");
        };
        assert_eq!(purpose, "nightly-worker-benchmark");
        assert_eq!(status, Some(TraceCorpusStatusArg::Accepted));

        let retention = parse_cli([
            "ironclaw",
            "traces",
            "worker-retention-maintenance",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "retention-worker",
            "--dry-run",
        ]);
        let TracesCommand::WorkerRetentionMaintenance {
            purpose, dry_run, ..
        } = unwrap_traces_command(retention)
        else {
            panic!("expected traces worker-retention-maintenance command");
        };
        assert_eq!(purpose.as_deref(), Some("retention-worker"));
        assert!(dry_run);

        let vector = parse_cli([
            "ironclaw",
            "traces",
            "worker-vector-index",
            "--endpoint",
            "https://trace.example/internal",
            "--purpose",
            "vector-worker",
        ]);
        let TracesCommand::WorkerVectorIndex { purpose, .. } = unwrap_traces_command(vector) else {
            panic!("expected traces worker-vector-index command");
        };
        assert_eq!(purpose.as_deref(), Some("vector-worker"));

        let utility_submission_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
        let utility = parse_cli([
            "ironclaw",
            "traces",
            "worker-utility-credit",
            "--endpoint",
            "https://trace.example/internal",
            "--event-type",
            "ranking-utility",
            "--credit-points-delta",
            "1.25",
            "--reason",
            "ranking eval utility",
            "--external-ref",
            "ranker:nightly-42",
            "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
        ]);
        let TracesCommand::WorkerUtilityCredit {
            event_type,
            credit_points_delta,
            external_ref,
            submission_ids,
            ..
        } = unwrap_traces_command(utility)
        else {
            panic!("expected traces worker-utility-credit command");
        };
        assert_eq!(event_type, TraceUtilityCreditEventTypeArg::RankingUtility);
        assert_eq!(credit_points_delta, 1.25);
        assert_eq!(external_ref, "ranker:nightly-42");
        assert_eq!(submission_ids, vec![utility_submission_id]);
    }

    #[test]
    fn worker_utility_credit_rejects_reviewer_and_abuse_events() {
        for event_type in ["reviewer-bonus", "abuse-penalty"] {
            let parsed = parse_cli_result([
                "ironclaw",
                "traces",
                "worker-utility-credit",
                "--endpoint",
                "https://trace.example/internal",
                "--event-type",
                event_type,
                "--credit-points-delta",
                "1.0",
                "--reason",
                "not a utility event",
                "--external-ref",
                "job:bad",
                "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
            ]);
            assert!(parsed.is_err(), "{event_type} should not parse");
        }
    }

    #[test]
    fn worker_utility_credit_body_uses_narrow_fields() {
        let first = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
        let second = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac02").expect("valid uuid");
        let body = trace_commons_worker_utility_credit_body(
            TraceUtilityCreditEventTypeArg::TrainingUtility,
            2.0,
            "offline training utility".to_string(),
            "training:batch-7".to_string(),
            vec![first, second],
        );

        assert_eq!(body["event_type"], "training_utility");
        assert_eq!(body["credit_points_delta"], 2.0);
        assert_eq!(body["reason"], "offline training utility");
        assert_eq!(body["external_ref"], "training:batch-7");
        assert_eq!(
            body["submission_ids"],
            serde_json::json!([
                "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
                "018f2b7b-0c11-72fd-95c4-1f9f98feac02"
            ])
        );
        assert!(body.get("reviewer_bonus").is_none());
        assert!(body.get("abuse_penalty").is_none());
    }

    #[test]
    fn worker_utility_credit_uses_dedicated_route() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/workers/utility-credit",
            &[],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/workers/utility-credit"
        );
    }

    #[test]
    fn process_evaluation_submit_parses_through_cli() {
        let submission_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "process-evaluation-submit",
            "--endpoint",
            "https://trace.example/internal",
            "--submission-id",
            "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
            "--reason",
            "nightly evaluator pass",
            "--evaluator-name",
            "process-quality",
            "--evaluator-version",
            "2026-04-26.1",
            "--label",
            "proper_verification",
            "--label",
            "safe_tool_ordering",
            "--tool-selection",
            "pass",
            "--tool-argument-quality",
            "partial",
            "--tool-ordering",
            "not_applicable",
            "--verification",
            "fail",
            "--side-effect-safety",
            "unknown",
            "--overall-score",
            "0.75",
            "--utility-credit-points-delta",
            "2.5",
            "--utility-external-ref",
            "process-eval:nightly:42",
            "--bearer-token-env",
            "TRACE_COMMONS_PROCESS_EVALUATION_WORKER_TOKEN",
            "--json",
        ]);

        let TracesCommand::ProcessEvaluationSubmit(args) = unwrap_traces_command(cli) else {
            panic!("expected traces process-evaluation-submit command");
        };

        assert_eq!(args.endpoint, "https://trace.example/internal");
        assert_eq!(args.submission_id, submission_id);
        assert_eq!(args.reason, "nightly evaluator pass");
        assert_eq!(args.evaluator_name.as_deref(), Some("process-quality"));
        assert_eq!(args.evaluator_version, "2026-04-26.1");
        assert_eq!(
            args.labels,
            vec!["proper_verification", "safe_tool_ordering"]
        );
        assert_eq!(
            args.tool_selection,
            Some(TraceProcessEvaluationRatingArg::Pass)
        );
        assert_eq!(
            args.tool_argument_quality,
            Some(TraceProcessEvaluationRatingArg::Partial)
        );
        assert_eq!(
            args.tool_ordering,
            Some(TraceProcessEvaluationRatingArg::NotApplicable)
        );
        assert_eq!(
            args.verification,
            Some(TraceProcessEvaluationRatingArg::Fail)
        );
        assert_eq!(
            args.side_effect_safety,
            Some(TraceProcessEvaluationRatingArg::Unknown)
        );
        assert_eq!(args.overall_score, Some(0.75));
        assert_eq!(args.utility_credit_points_delta, Some(2.5));
        assert_eq!(
            args.utility_external_ref.as_deref(),
            Some("process-eval:nightly:42")
        );
        assert_eq!(
            args.bearer_token_env,
            "TRACE_COMMONS_PROCESS_EVALUATION_WORKER_TOKEN"
        );
        assert!(args.json);
    }

    #[test]
    fn process_evaluation_body_uses_expected_shape() {
        let submission_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
        let body = trace_commons_process_evaluation_body(
            submission_id,
            " nightly evaluator pass ".to_string(),
            Some(" process-quality ".to_string()),
            " 2026-04-26.1 ".to_string(),
            vec![
                "proper_verification".to_string(),
                " ".to_string(),
                "safe_tool_ordering".to_string(),
            ],
            Some(TraceProcessEvaluationRatingArg::Pass),
            Some(TraceProcessEvaluationRatingArg::Partial),
            Some(TraceProcessEvaluationRatingArg::NotApplicable),
            Some(TraceProcessEvaluationRatingArg::Fail),
            Some(TraceProcessEvaluationRatingArg::Unknown),
            Some(0.75),
            Some(2.5),
            Some(" process-eval:nightly:42 ".to_string()),
        )
        .expect("body builds");

        assert_eq!(
            body,
            serde_json::json!({
                "submission_id": "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
                "process_evaluation": {
                    "evaluator_name": "process-quality",
                    "evaluator_version": "2026-04-26.1",
                    "labels": ["proper_verification", "safe_tool_ordering"],
                    "tool_selection": "pass",
                    "tool_argument_quality": "partial",
                    "tool_ordering": "not_applicable",
                    "verification": "fail",
                    "side_effect_safety": "unknown",
                    "overall_score": 0.75
                },
                "utility_credit_points_delta": 2.5,
                "utility_external_ref": "process-eval:nightly:42",
                "reason": "nightly evaluator pass"
            })
        );
    }

    #[test]
    fn process_evaluation_body_omits_utility_credit_when_delta_absent() {
        let submission_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
        let body = trace_commons_process_evaluation_body(
            submission_id,
            "operator reason".to_string(),
            None,
            "2026-04-26.1".to_string(),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("body builds");

        assert!(body.get("utility_credit_points_delta").is_none());
        assert!(body.get("utility_external_ref").is_none());
    }

    #[test]
    fn process_evaluation_body_rejects_invalid_inputs() {
        let submission_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");

        let empty_reason = trace_commons_process_evaluation_body(
            submission_id,
            " ".to_string(),
            None,
            "2026-04-26.1".to_string(),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(empty_reason.is_err());

        let empty_version = trace_commons_process_evaluation_body(
            submission_id,
            "operator reason".to_string(),
            None,
            " ".to_string(),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(empty_version.is_err());

        let invalid_score = trace_commons_process_evaluation_body(
            submission_id,
            "operator reason".to_string(),
            None,
            "2026-04-26.1".to_string(),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            Some(1.25),
            None,
            None,
        );
        assert!(invalid_score.is_err());

        let non_finite_delta = trace_commons_process_evaluation_body(
            submission_id,
            "operator reason".to_string(),
            None,
            "2026-04-26.1".to_string(),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(f32::NAN),
            Some("process-eval:nightly:42".to_string()),
        );
        assert!(non_finite_delta.is_err());

        let too_large_delta = trace_commons_process_evaluation_body(
            submission_id,
            "operator reason".to_string(),
            None,
            "2026-04-26.1".to_string(),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(100.01),
            Some("process-eval:nightly:42".to_string()),
        );
        assert!(too_large_delta.is_err());

        let empty_external_ref = trace_commons_process_evaluation_body(
            submission_id,
            "operator reason".to_string(),
            None,
            "2026-04-26.1".to_string(),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(2.5),
            Some(" ".to_string()),
        );
        assert!(empty_external_ref.is_err());

        let dangling_external_ref = trace_commons_process_evaluation_body(
            submission_id,
            "operator reason".to_string(),
            None,
            "2026-04-26.1".to_string(),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("process-eval:nightly:42".to_string()),
        );
        assert!(dangling_external_ref.is_err());
    }

    #[test]
    fn process_evaluation_submit_uses_dedicated_route() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/workers/process-evaluation",
            &[],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/workers/process-evaluation"
        );
    }

    #[test]
    fn local_credit_summary_includes_delayed_ledger_totals() {
        let submission_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
        let trace_id = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac02").expect("valid uuid");
        let summary = credit_summary(&[LocalSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://trace.example/internal/v1/traces".to_string()),
            status: LocalSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(chrono::Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.25),
            credit_explanation: vec!["Accepted after privacy checks.".to_string()],
            credit_events: vec![
                TraceCreditEvent {
                    event_id: Uuid::new_v4(),
                    submission_id,
                    contributor_pseudonym: "contributor-a".to_string(),
                    kind: TraceCreditEventKind::Accepted,
                    points_delta: 1.0,
                    reason: "Accepted".to_string(),
                    created_at: chrono::Utc::now(),
                },
                TraceCreditEvent {
                    event_id: Uuid::new_v4(),
                    submission_id,
                    contributor_pseudonym: "contributor-a".to_string(),
                    kind: TraceCreditEventKind::UsedForTrainingOrRanking,
                    points_delta: 1.25,
                    reason: "Process evaluation utility.".to_string(),
                    created_at: chrono::Utc::now(),
                },
            ],
            last_credit_notice_at: None,
        }]);

        assert_eq!(summary.pending_credit, 1.0);
        assert_eq!(summary.final_credit, 2.25);
        assert_eq!(summary.delayed_credit_delta, 1.25);
        assert_eq!(summary.credit_events_total, 2);
    }

    #[test]
    fn cli_credit_sync_resets_notice_when_explanation_changes_without_credit_delta() {
        let submission_id =
            Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac11").expect("valid uuid");
        let trace_id = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac12").expect("valid uuid");
        let now = chrono::Utc::now();
        let mut records = vec![LocalSubmissionRecord {
            submission_id,
            trace_id,
            endpoint: Some("https://trace.example/internal/v1/traces".to_string()),
            status: LocalSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(now),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.0),
            credit_explanation: vec!["Previous explanation.".to_string()],
            credit_events: Vec::new(),
            last_credit_notice_at: Some(now),
        }];
        let update = TraceSubmissionStatusUpdate {
            submission_id,
            trace_id,
            status: "accepted".to_string(),
            credit_points_pending: 1.0,
            credit_points_final: Some(2.0),
            credit_points_ledger: 0.0,
            credit_points_total: Some(2.0),
            explanation: vec!["Accepted after privacy checks.".to_string()],
            delayed_credit_explanations: vec![
                "Process evaluation changed the utility explanation.".to_string(),
            ],
        };

        let changed = apply_cli_submission_status_updates_to_records(
            &mut records,
            std::slice::from_ref(&update),
            now,
        );

        assert_eq!(changed, 1);
        assert!(records[0].last_credit_notice_at.is_none());
        assert_eq!(
            records[0].credit_explanation,
            vec![
                "Accepted after privacy checks.".to_string(),
                "Process evaluation changed the utility explanation.".to_string(),
            ]
        );
        assert_eq!(records[0].credit_events.len(), 1);
        assert_eq!(
            records[0].credit_events[0].kind,
            TraceCreditEventKind::CreditSynced
        );
        assert_eq!(records[0].credit_events[0].points_delta, 0.0);

        let unchanged = apply_cli_submission_status_updates_to_records(
            &mut records,
            std::slice::from_ref(&update),
            now,
        );

        assert_eq!(unchanged, 0);
        assert_eq!(records[0].credit_events.len(), 1);
    }

    #[test]
    fn cli_credit_notice_message_includes_delayed_and_event_totals() {
        let message = credit_notice_message(&CreditSummary {
            submissions_total: 4,
            submissions_submitted: 2,
            submissions_revoked: 1,
            submissions_expired: 1,
            pending_credit: 3.5,
            final_credit: 2.0,
            delayed_credit_delta: 0.75,
            credit_events_total: 5,
            recent_explanations: Vec::new(),
        });

        assert!(message.contains("2 submitted"));
        assert!(message.contains("1 expired (4 total)"));
        assert!(message.contains("pending +3.50"));
        assert!(message.contains("final confirmed +2.00"));
        assert!(message.contains("delayed ledger +0.75"));
        assert!(message.contains("5 credit event(s) recorded"));
        assert!(message.contains("Delayed credit can change"));
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
    fn maintenance_summary_lines_include_benchmark_artifact_invalidation() {
        let value = serde_json::json!({
            "audit_event_id": "audit-123",
            "purpose": "source-invalidation",
            "dry_run": false,
            "benchmark_artifacts_invalidated": 3
        });

        let lines = trace_commons_maintenance_summary_lines(&value);

        assert!(lines.contains(&"  benchmark artifacts invalidated: 3".to_string()));
    }

    #[test]
    fn retention_maintenance_summary_lines_include_benchmark_artifact_invalidation() {
        let value = serde_json::json!({
            "audit_event_id": "audit-123",
            "purpose": "retention-sweep",
            "dry_run": false,
            "benchmark_artifacts_invalidated": 2
        });

        let lines = trace_commons_retention_maintenance_summary_lines(&value);

        assert!(lines.contains(&"  benchmark artifacts invalidated: 2".to_string()));
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
    fn retention_jobs_list_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "retention-jobs-list",
            "--endpoint",
            "https://trace.example/internal",
            "--limit",
            "25",
            "--status",
            "dry-run",
            "--bearer-token-env",
            "TRACE_COMMONS_ADMIN_TOKEN",
        ]);

        let TracesCommand::RetentionJobsList {
            endpoint,
            limit,
            status,
            bearer_token_env,
            json,
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces retention-jobs-list command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert_eq!(limit, Some(25));
        assert_eq!(status, Some(TraceRetentionJobStatusArg::DryRun));
        assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
        assert!(!json);
    }

    #[test]
    fn retention_job_items_use_ingest_endpoint() {
        let job_id =
            Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("static uuid parses");
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            &format!("/v1/admin/retention/jobs/{job_id}/items"),
            &[
                ("action", "purge".to_string()),
                ("status", "done".to_string()),
            ],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/admin/retention/jobs/11111111-1111-1111-1111-111111111111/items?action=purge&status=done"
        );
    }

    #[test]
    fn export_access_grants_list_parses_through_cli() {
        let cli = parse_cli([
            "ironclaw",
            "traces",
            "export-access-grants-list",
            "--endpoint",
            "https://trace.example/internal",
            "--limit",
            "25",
            "--status",
            "active",
            "--dataset-kind",
            "replay-dataset",
            "--bearer-token-env",
            "TRACE_COMMONS_ADMIN_TOKEN",
        ]);

        let TracesCommand::ExportAccessGrantsList {
            endpoint,
            limit,
            status,
            dataset_kind,
            bearer_token_env,
            json,
        } = unwrap_traces_command(cli)
        else {
            panic!("expected traces export-access-grants-list command");
        };

        assert_eq!(endpoint, "https://trace.example/internal");
        assert_eq!(limit, Some(25));
        assert_eq!(status, Some(TraceExportAccessGrantStatusArg::Active));
        assert_eq!(dataset_kind.as_deref(), Some("replay-dataset"));
        assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
        assert!(!json);
    }

    #[test]
    fn export_jobs_list_uses_ingest_endpoint() {
        let url = trace_commons_api_url(
            "https://trace.example/internal/v1/traces",
            "/v1/admin/export/jobs",
            &[
                ("status", "complete".to_string()),
                ("dataset_kind", "ranker_training_pairs".to_string()),
            ],
        )
        .expect("url builds");

        assert_eq!(
            url,
            "https://trace.example/internal/v1/admin/export/jobs?status=complete&dataset_kind=ranker_training_pairs"
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
        let value: serde_json::Value = serde_json::from_str(
            r#"{
            "db_reconciliation": {
                "file_submission_count": 3,
                "db_submission_count": 2,
                "missing_submission_ids_in_db": ["missing-a", "missing-b"],
                "missing_submission_ids_in_files": [],
                "status_mismatches": ["status-mismatch"],
                "file_derived_count": 4,
                "db_derived_count": 3,
                "missing_derived_submission_ids_in_db": ["derived-missing-db"],
                "missing_derived_submission_ids_in_files": ["derived-missing-files"],
                "derived_status_mismatches": ["derived-status"],
                "derived_hash_mismatches": ["derived-hash"],
                "db_object_ref_count": 2,
                "accepted_without_active_envelope_object_ref": ["missing-object-ref"],
                "unreadable_active_envelope_object_refs": ["unreadable-object-ref"],
                "hash_mismatched_active_envelope_object_refs": ["hash-mismatch"],
                "file_credit_event_count": 5,
                "db_credit_event_count": 4,
                "file_audit_event_count": 6,
                "db_audit_event_count": 6,
                "db_retention_job_count": 2,
                "db_retention_job_item_count": 4,
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
                "db_reader_parity_failures": ["reviewer_metadata: failed"],
                "active_vector_entries": 7,
                "accepted_current_derived_without_active_vector_entry": ["vector-a", "vector-b"],
                "invalid_active_vector_entries": 1,
                "blocking_gaps": [
                    "missing_submission_ids_in_db=2",
                    "reviewer_metadata_reader_parity=failed"
                ]
            }
        }"#,
        )
        .expect("valid reconciliation fixture");

        let lines = maintenance_reconciliation_lines(&value);

        assert_eq!(
            lines,
            vec![
                "  db reconciliation:".to_string(),
                "    submissions: files=3 db=2 missing_in_db=2 missing_in_files=0 status_mismatches=1".to_string(),
                "    derived: files=4 db=3 missing_in_db=1 missing_in_files=1 status_mismatches=1 hash_mismatches=1".to_string(),
                "    object refs: db=2 accepted_without_active_envelope=1 unreadable_active_envelope=1 hash_mismatched_active_envelope=1".to_string(),
                "    ledger/audit: file_credit_events=5 db_credit_events=4 file_audit_events=6 db_audit_events=6 db_retention_jobs=2 db_retention_items=4".to_string(),
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
