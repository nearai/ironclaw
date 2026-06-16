//! Trace contribution CLI commands.
//!
//! These commands are deliberately opt-in and local-first. `preview` creates a
//! redacted contribution envelope from an existing recorded trace. `submit`
//! only uploads when the user provides an explicit ingestion endpoint.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ironclaw_reborn_traces::client::{TraceClientHost, TraceClientScope};
use ironclaw_reborn_traces::contribution::{
    ConsentScope, CreditSummary, RecordedTraceContributionOptions, StandingTraceContributionPolicy,
    TraceChannel, TraceContributionAcceptance, TraceContributionEnvelope, TraceCreditEvent,
    TraceCreditEventKind, TraceSubmissionReceipt, TraceSubmissionStatusUpdate,
    acknowledge_trace_credit_notice_for_scope, estimate_initial_credit,
    fetch_trace_submission_statuses_with_policy, mark_trace_credit_notice_due_for_scope,
    mint_profile_attribution_token_for_scope, normalize_trace_selected_tools,
    preflight_trace_contribution_policy, read_trace_policy_for_scope,
    revoke_trace_submission_at_endpoint_with_policy, set_community_profile_for_scope,
    snooze_trace_credit_notice_for_scope, submit_trace_envelope_to_endpoint_with_policy,
    trace_credit_summary, trace_queue_diagnostics_for_scope, trace_submission_status_endpoint,
    withdraw_community_profile_for_scope, write_trace_policy_for_scope,
};

mod contributor;
mod shared;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// `traces` subcommand surface for the standalone Reborn CLI.
#[derive(Debug, Args)]
pub(crate) struct TracesCommand {
    #[command(subcommand)]
    command: TracesSubcommand,
}

impl TracesCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(run_traces(self.command))
    }
}

#[derive(Subcommand, Debug, Clone)]
enum TracesSubcommand {
    /// Enable autonomous trace contribution after local redaction
    OptIn {
        /// Explicit private ingestion endpoint URL
        #[arg(long)]
        endpoint: String,

        /// Runtime/web user scope to configure; defaults to this instance's owner_id
        #[arg(long)]
        user_scope: Option<String>,

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

        /// Operator-issued pilot invite code. When set, included in upload-claim
        /// refresh requests so the issuer's allowlist gate can match it. Required
        /// only when the configured issuer runs with TRACE_COMMONS_ALLOWLIST_SOURCE
        /// — omit otherwise.
        #[arg(long)]
        upload_token_invite_code: Option<String>,

        /// Upload claim issuer timeout in milliseconds
        #[arg(long, default_value_t = ironclaw_reborn_traces::contribution::TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS)]
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
    OptOut {
        /// Runtime/web user scope to disable; defaults to this instance's owner_id
        #[arg(long)]
        user_scope: Option<String>,
    },

    /// Show local standing trace contribution policy
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show the runtime/web policy for this user scope instead of the global CLI policy
        #[arg(long)]
        user_scope: Option<String>,
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

    /// Manage the optional public community profile (second opt-in)
    Profile {
        #[command(subcommand)]
        command: TracesProfileSubcommand,
    },
}

/// Sub-subcommands for `traces profile`. The community profile is a second,
/// separate opt-in: a profile token carries only the `public_attribution`
/// consent scope with no allowed uses, so it cannot submit traces.
#[derive(Subcommand, Debug, Clone)]
enum TracesProfileSubcommand {
    /// Mint a short-lived profile token for the Trace Commons web profile page
    Token {
        /// Runtime/web user scope; defaults to this instance's owner_id
        #[arg(long)]
        user_scope: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create or update the public community profile
    Set {
        /// Pseudonymous display handle (3-32 chars: ASCII letters, digits, '-', '_')
        #[arg(long)]
        handle: String,

        /// Optional short bio (max 280 bytes)
        #[arg(long)]
        bio: Option<String>,

        /// Runtime/web user scope; defaults to this instance's owner_id
        #[arg(long)]
        user_scope: Option<String>,
    },

    /// Withdraw the public community profile
    Withdraw {
        /// Runtime/web user scope; defaults to this instance's owner_id
        #[arg(long)]
        user_scope: Option<String>,
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

async fn run_traces(cmd: TracesSubcommand) -> anyhow::Result<()> {
    match cmd {
        v @ TracesSubcommand::OptIn { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::OptOut { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::Status { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::Preview { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::Enqueue { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::FlushQueue { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::QueueStatus { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::Credit { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::Submit { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::ListSubmissions { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::Revoke { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::IngestHealth { .. } => contributor::dispatch(v).await,
        v @ TracesSubcommand::Profile { .. } => contributor::dispatch(v).await,
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
    user_scope: Option<String>,
    bearer_token_env: String,
    upload_token_issuer_url: Option<String>,
    upload_token_issuer_allowed_hosts: Vec<String>,
    upload_token_audience: Option<String>,
    upload_token_tenant_id: Option<String>,
    upload_token_workload_token_env: Option<String>,
    upload_token_invite_code: Option<String>,
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
    /// Hash-only: true when the standing policy has an `upload_token_invite_code`
    /// set, false otherwise. The raw code is never returned, surfaced only as
    /// a present/absent boolean to match the rest of the diagnostics surface.
    upload_token_invite_code_configured: bool,
    include_message_text: bool,
    include_tool_payloads: bool,
    require_manual_approval_when_pii_detected: bool,
    min_submission_score: f32,
    credit_notice_interval_hours: u32,
    selected_tools_count: usize,
    queue: ironclaw_reborn_traces::contribution::TraceQueueDiagnostics,
    credit_summary: CreditSummary,
}

fn opt_in(options: OptInOptions) -> anyhow::Result<()> {
    let runtime_scope = trace_runtime_user_scope(options.user_scope.as_deref())?;
    let issuer_url = options
        .upload_token_issuer_url
        .and_then(|url| (!url.trim().is_empty()).then_some(url));
    let workload_token_env = options
        .upload_token_workload_token_env
        .and_then(|env| (!env.trim().is_empty()).then_some(env));
    let invite_code = options
        .upload_token_invite_code
        .and_then(|code| (!code.trim().is_empty()).then_some(code));
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
        upload_token_invite_code: invite_code,
        upload_token_issuer_timeout_ms: options.upload_token_issuer_timeout_ms,
        include_message_text: options.include_message_text,
        include_tool_payloads: options.include_tool_payloads,
        selected_tools: normalize_trace_selected_tools(options.selected_tools),
        require_manual_approval_when_pii_detected: !options.allow_pii_review_bypass,
        min_submission_score: options.min_submission_score.clamp(0.0, 1.0),
        default_scope: options.scope.into(),
        ..StandingTraceContributionPolicy::default()
    };

    write_policy(&policy)?;
    write_trace_policy_for_scope(Some(&runtime_scope), &policy)?;
    println!("Trace contribution opt-in enabled.");
    println!("Autonomous submissions will use local redaction and the configured endpoint.");
    println!("Runtime/web trace scope configured: {runtime_scope}");
    Ok(())
}

fn opt_out(user_scope: Option<&str>) -> anyhow::Result<()> {
    let runtime_scope = trace_runtime_user_scope(user_scope)?;
    let mut policy = read_policy()?;
    policy.enabled = false;
    write_policy(&policy)?;
    let mut scoped_policy = read_trace_policy_for_scope(Some(&runtime_scope))?;
    scoped_policy.enabled = false;
    write_trace_policy_for_scope(Some(&runtime_scope), &scoped_policy)?;
    println!("Trace contribution opt-in disabled. Queued envelopes remain local.");
    println!("Runtime/web trace scope disabled: {runtime_scope}");
    Ok(())
}

fn show_policy_status(json: bool, user_scope: Option<&str>) -> anyhow::Result<()> {
    let normalized_scope = user_scope
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(str::to_string);
    let policy = match normalized_scope.as_deref() {
        Some(scope) => read_trace_policy_for_scope(Some(scope))?,
        None => read_policy()?,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&policy)
                .map_err(|e| anyhow::anyhow!("failed to serialize trace policy: {}", e))?
        );
        return Ok(());
    }

    println!("Trace contribution policy:");
    println!(
        "  scope: {}",
        normalized_scope.as_deref().unwrap_or("global CLI policy")
    );
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
        // Pilot invite code: present/absent only — never echo the raw code,
        // matching the rest of the file's hash-only stance toward
        // operator-secret material.
        println!(
            "  pilot invite code: {}",
            if policy
                .upload_token_invite_code
                .as_deref()
                .is_some_and(|code| !code.trim().is_empty())
            {
                "configured"
            } else {
                "not configured"
            }
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
    let queued_count = match normalized_scope.as_deref() {
        Some(scope) => ironclaw_reborn_traces::contribution::queued_trace_envelope_paths_for_scope(
            Some(scope),
        )?
        .len(),
        None => queued_envelope_paths()?.len(),
    };
    println!("  queued envelopes: {queued_count}");
    Ok(())
}

async fn profile_token(user_scope: Option<&str>, json: bool) -> anyhow::Result<()> {
    let runtime_scope = trace_runtime_user_scope(user_scope)?;
    let token = mint_profile_attribution_token_for_scope(Some(&runtime_scope)).await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "access_token": token.access_token,
                "expires_at": token.expires_at,
                "expires_in": token.expires_in,
            }))
            .map_err(|e| anyhow::anyhow!("failed to serialize profile token: {}", e))?
        );
        return Ok(());
    }
    println!("{}", token.access_token);
    if let Some(expires_at) = token.expires_at {
        println!("Expires at: {expires_at}");
    } else if let Some(expires_in) = token.expires_in {
        println!("Expires in: {expires_in} second(s)");
    }
    println!();
    println!(
        "Paste this token (without any 'Bearer ' prefix) into the Trace Commons profile page."
    );
    println!("It only authorizes community-profile management and cannot submit traces.");
    Ok(())
}

async fn profile_set(
    user_scope: Option<&str>,
    handle: &str,
    bio: Option<&str>,
) -> anyhow::Result<()> {
    let runtime_scope = trace_runtime_user_scope(user_scope)?;
    set_community_profile_for_scope(Some(&runtime_scope), handle, bio).await?;
    println!("Community profile set: display handle '{}'.", handle.trim());
    println!("Withdraw anytime with 'ironclaw-reborn traces profile withdraw'.");
    Ok(())
}

async fn profile_withdraw(user_scope: Option<&str>) -> anyhow::Result<()> {
    let runtime_scope = trace_runtime_user_scope(user_scope)?;
    withdraw_community_profile_for_scope(Some(&runtime_scope)).await?;
    println!(
        "Community profile withdrawn. Your handle no longer appears on the public community surface."
    );
    Ok(())
}

fn trace_runtime_user_scope(user_scope: Option<&str>) -> anyhow::Result<String> {
    if let Some(scope) = user_scope.map(str::trim).filter(|scope| !scope.is_empty()) {
        return Ok(scope.to_string());
    }

    Ok(resolve_runtime_owner_scope())
}

/// Minimal inline replacement for the legacy
/// `crate::config::{load_bootstrap_settings, resolve_owner_id}` pair. The
/// monolith's helper hydrated a full `Settings` from profile + TOML overlays;
/// for the CLI's scope-derivation path we only need the owner id, which
/// trims `IRONCLAW_OWNER_ID` and falls back to the legacy "default" scope.
fn resolve_runtime_owner_scope() -> String {
    let env_owner_id = std::env::var("IRONCLAW_OWNER_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    env_owner_id.unwrap_or_else(|| "default".to_string())
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
        println!(
            "  pilot invite code: {}",
            if diagnostics.upload_token_invite_code_configured {
                "configured"
            } else {
                "not configured"
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
    let upload_token_invite_code_configured = policy
        .upload_token_invite_code
        .as_deref()
        .is_some_and(|code| !code.trim().is_empty());
    let issuer_credentials_ready = upload_token_issuer_configured
        && (!policy.upload_token_issuer_allowed_hosts.is_empty())
        && policy
            .upload_token_workload_token_env
            .as_deref()
            .is_none_or(|env| env.trim().is_empty() || std::env::var_os(env).is_some());
    let queue = trace_queue_diagnostics_for_scope(scope_ref)?;
    let trace_host = TraceClientHost;
    let local_records = match scope_ref {
        Some(scope) => {
            let trace_scope = TraceClientScope::raw(scope);
            trace_host.read_local_records_for_scope(&trace_scope)?
        }
        None => trace_host.read_local_records_for_default()?,
    };

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
        upload_token_invite_code_configured,
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
    let recorded_trace: ironclaw_reborn_traces::recording::TraceFile =
        serde_json::from_str(&raw_json).map_err(|e| {
            anyhow::anyhow!(
                "failed to parse recorded trace {}: {}",
                options.recorded_trace.display(),
                e
            )
        })?;

    let trace_host = TraceClientHost;
    let envelope = trace_host
        .build_envelope_from_recorded_trace(
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
        )
        .await?;
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

fn print_trace_commons_json(response: &TraceCommonsApiResponse) -> anyhow::Result<()> {
    println!("{}", pretty_trace_commons_body(response)?);
    Ok(())
}

fn pretty_trace_commons_body(response: &TraceCommonsApiResponse) -> anyhow::Result<String> {
    if let Some(value) = response.json.as_ref() {
        serde_json::to_string_pretty(value)
            .map_err(|e| anyhow::anyhow!("failed to serialize Trace Commons response: {}", e))
    } else {
        Ok(response.body.clone())
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
    let report = TraceClientHost.flush_default_queue(limit).await?;
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
    ironclaw_reborn_traces::paths::ironclaw_base_dir().join("trace_contributions")
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
    // policy.json may now carry an operator-issued pilot invite code, so
    // restrict permissions on unix. Non-unix targets fall back to the
    // standard write — the surrounding directory layout already inherits
    // user-only access via ironclaw_base_dir.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| {
                anyhow::anyhow!("failed to write trace policy {}: {}", path.display(), e)
            })?;
        file.write_all(body.as_bytes()).map_err(|e| {
            anyhow::anyhow!("failed to write trace policy {}: {}", path.display(), e)
        })?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, body).map_err(|e| {
            anyhow::anyhow!("failed to write trace policy {}: {}", path.display(), e)
        })?;
    }
    Ok(())
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
