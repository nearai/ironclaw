mod analysis;
mod api_capacity;
mod capture;
mod child_io;
mod compare;
mod db_probe;
mod human;
mod process_metrics;
mod process_pressure;
mod progress;
mod ramp;
mod redaction;
mod report;
mod resource_ops;
mod secret_ops;
mod suite;
mod summary;
mod sweep;
mod synthetic;
#[cfg(test)]
mod tests;
mod trace;
mod user_turn;

use std::{
    any::Any,
    collections::BTreeMap,
    io::ErrorKind,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, mpsc},
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crate::redaction::redact_postgres_url;
use crate::{
    api_capacity::ApiCapacitySummary,
    capture::CapturedRun,
    child_io::{join_child_stderr_reader, spawn_child_stderr_reader},
    db_probe::DbProbeSummary,
    process_metrics::{ProcessMetrics, ProcessMetricsSampler},
    progress::{ProgressCounters, spawn_progress_reporter, stop_progress_reporter},
    redaction::redact_libsql_path,
    secret_ops::{build_secret_consume_workload, run_secret_consume_tasks},
    summary::{
        FailureCause, FailureCauseSummary, LatencySummary, latency_summary,
        summarize_failure_causes, summarize_user_turn_operation_attribution,
        summarize_user_turn_stages,
    },
    synthetic::SyntheticIds,
    user_turn::{
        UserTurnOperationAttributionSummary, UserTurnStageDurations, UserTurnStageLatencySummary,
        build_user_turn_workload, run_user_turn_tasks,
    },
};
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum, parser::ValueSource};
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, TenantId, VirtualPath,
};
use ironclaw_resources::{FilesystemResourceGovernor, ResourceAccount, ResourceGovernor};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "ironclaw_stress",
    about = "Stress IronClaw infrastructure workloads"
)]
pub(crate) struct Args {
    #[arg(long, value_enum)]
    pub(crate) backend: Backend,

    /// Named workload preset. Explicit CLI flags override preset defaults.
    #[arg(long, value_enum)]
    pub(crate) preset: Option<StressPreset>,

    /// Run a curated multi-scenario stress suite.
    #[arg(long, value_enum)]
    pub(crate) suite: Option<StressSuite>,

    /// OS processes to run against the same snapshot path. Use >1 to exercise
    /// cross-process CAS contention that the in-process lock cannot serialize.
    #[arg(long, default_value_t = 1)]
    pub(crate) processes: usize,

    /// Threads per process.
    #[arg(long, default_value_t = 8)]
    pub(crate) concurrency: usize,

    /// Operations per thread.
    #[arg(long, default_value_t = 200)]
    pub(crate) operations: usize,

    /// Run workers for this many measured seconds instead of a fixed operation count. Set to 0
    /// for fixed-operation mode.
    #[arg(long, default_value_t = 0)]
    pub(crate) duration_seconds: u64,

    /// Warm up for this many seconds before measuring. Requires --duration-seconds.
    #[arg(long, default_value_t = 0)]
    pub(crate) warmup_seconds: u64,

    /// Synthetic users distributed across operations.
    #[arg(long, default_value_t = 50)]
    pub(crate) users: usize,

    /// Distinct user threads targeted by user-turn scenarios. Set to 0 to use one thread per user.
    #[arg(long, default_value_t = 0)]
    pub(crate) active_thread_count: usize,

    /// Distinct threads per owner-user that share one `/turns/state.json`. Set
    /// above 1 to reproduce the production contention shape (a user's foreground
    /// turn plus routine turns on different threads concurrently writing the same
    /// per-user turn-state document). Default 1 = one thread per owner.
    #[arg(long, default_value_t = 1)]
    pub(crate) threads_per_owner: usize,

    /// Synthetic tenants distributed across users.
    #[arg(long, default_value_t = 1)]
    pub(crate) tenants: usize,

    /// Distinct synthetic user threads to prefill before measured user-turn runs.
    #[arg(long, default_value_t = 0)]
    pub(crate) prefill_threads: usize,

    /// Completed chat turns to prefill per synthetic thread.
    #[arg(long, default_value_t = 0)]
    pub(crate) prefill_turns_per_thread: usize,

    /// Synthetic threads to prefill concurrently.
    #[arg(long, default_value_t = 4)]
    pub(crate) prefill_concurrency: usize,

    /// Exercise the gate-blocked turn path: every Nth measured user-turn
    /// operation blocks its run on a gate (alternating approval/auth), resumes
    /// it, then re-claims and completes. 0 (default) = never block, the pure
    /// claim/complete hot path. Combine with
    /// `--turn-state-backend memory-persist-on-block` to drive persist-on-block
    /// writes under concurrency and confirm the durable sink does not
    /// reintroduce contention.
    #[arg(long, default_value_t = 0)]
    pub(crate) gate_blocked_every: usize,

    #[arg(long, value_enum, default_value_t = Scenario::ReserveRelease)]
    pub(crate) scenario: Scenario,

    /// Turn-state store backend for user-turn scenarios. `filesystem` = durable
    /// per-user state.json (CAS, current production path);
    /// `filesystem-row` = durable typed append-log deltas with a hot
    /// in-process row cache; `memory` = one shared in-process authority
    /// (runtime-wedge prototype). No effect on non-turn scenarios.
    #[arg(long, value_enum, default_value_t = TurnStateBackend::Filesystem)]
    pub(crate) turn_state_backend: TurnStateBackend,

    /// Override max retained terminal run records in the turn-state store.
    /// Useful for measuring filesystem snapshot growth sensitivity.
    #[arg(long)]
    pub(crate) turn_state_max_terminal_records: Option<usize>,

    /// Override max retained lifecycle events in the turn-state store.
    #[arg(long)]
    pub(crate) turn_state_max_events: Option<usize>,

    /// Override max retained idempotency records per operation family.
    #[arg(long)]
    pub(crate) turn_state_max_idempotency_records: Option<usize>,

    /// Shared run id. Defaults to a fresh UUID.
    #[arg(long)]
    pub(crate) run_id: Option<String>,

    /// libSQL database path. Defaults to a temp-file path printed in output.
    #[arg(long)]
    pub(crate) libsql_path: Option<PathBuf>,

    /// Postgres URL. Defaults to IRONCLAW_FILESYSTEM_POSTGRES_URL, then DATABASE_URL.
    #[arg(long)]
    pub(crate) postgres_url: Option<String>,

    /// Postgres pool size per process.
    #[arg(long, default_value_t = 4)]
    pub(crate) postgres_pool_size: usize,

    /// Base URL for API-level scenarios, for example https://host or http://127.0.0.1:8080.
    #[arg(long)]
    pub(crate) api_base_url: Option<String>,

    /// JSONL file of API test users. Each line may contain user_id and bearer_token.
    #[arg(long)]
    pub(crate) api_users_jsonl: Option<PathBuf>,

    /// Bearer token used for every generated API test user when --api-users-jsonl is omitted.
    #[arg(long)]
    pub(crate) api_bearer_token: Option<String>,

    /// Operator/admin bearer token used to provision API test users via WebUI admin CRUD.
    #[arg(long)]
    pub(crate) api_admin_bearer_token: Option<String>,

    /// Admin principals used to fan out API test-user provisioning. 1 uses the supplied admin token directly.
    #[arg(long, default_value_t = 1)]
    pub(crate) api_admin_provisioners: usize,

    /// Aggregate read pressure per virtual user for api-user-capacity.
    #[arg(long, default_value_t = 0.0)]
    pub(crate) api_read_qps_per_user: f64,

    /// Read worker count for api-user-capacity. 0 auto-scales from --users.
    #[arg(long, default_value_t = 0)]
    pub(crate) api_read_workers: usize,

    /// Read endpoint mix for api-user-capacity, e.g. list_threads=50,timeline=45,session=5.
    #[arg(long, default_value_t = api_capacity::default_read_mix())]
    pub(crate) api_read_mix: String,

    /// Page size for API list/timeline reads.
    #[arg(long, default_value_t = 50)]
    pub(crate) api_page_size: u32,

    /// Per-user delay after each completed full flow.
    #[arg(long, default_value_t = 0)]
    pub(crate) api_message_interval_ms: u64,

    /// Wait until the assistant message is visible in the timeline before completing a full flow.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub(crate) api_wait_for_assistant: bool,

    /// Full-flow timeout while waiting for assistant visibility.
    #[arg(long, default_value_t = 30_000)]
    pub(crate) api_terminal_timeout_ms: u64,

    /// Timeline polling interval while waiting for assistant visibility.
    #[arg(long, default_value_t = 250)]
    pub(crate) api_poll_interval_ms: u64,

    /// Per-request timeout for API calls.
    #[arg(long, default_value_t = 10_000)]
    pub(crate) api_request_timeout_ms: u64,

    /// Concurrent setup creates for API scenario user threads.
    #[arg(long, default_value_t = 16)]
    pub(crate) api_setup_concurrency: usize,

    /// Background long-running API users to run alongside foreground api-user-capacity sends.
    #[arg(long, default_value_t = 0)]
    pub(crate) api_background_users: usize,

    /// Max concurrently active background API users. 0 auto-scales from --api-background-users.
    #[arg(long, default_value_t = 0)]
    pub(crate) api_background_concurrency: usize,

    /// Full-flow operations per background API user.
    #[arg(long, default_value_t = 1)]
    pub(crate) api_background_operations: usize,

    /// Delay after launching background users before foreground sends start.
    #[arg(long, default_value_t = 250)]
    pub(crate) api_background_start_delay_ms: u64,

    /// Optional bind address for the built-in OpenAI-compatible mock LLM sidecar.
    #[arg(long)]
    pub(crate) mock_llm_bind: Option<SocketAddr>,

    /// Model name returned by the built-in mock LLM.
    #[arg(long, default_value = "stress-mock")]
    pub(crate) mock_llm_model: String,

    /// Base latency added by the built-in mock LLM.
    #[arg(long, default_value_t = 0)]
    pub(crate) mock_llm_latency_ms: u64,

    /// Latency for mock LLM requests tagged as API background load. 0 uses --mock-llm-latency-ms.
    #[arg(long, default_value_t = 0)]
    pub(crate) mock_llm_background_latency_ms: u64,

    /// Deterministic jitter ceiling added by the built-in mock LLM.
    #[arg(long, default_value_t = 0)]
    pub(crate) mock_llm_jitter_ms: u64,

    /// Assistant content bytes returned by the built-in mock LLM.
    #[arg(long, default_value_t = 128)]
    pub(crate) mock_llm_output_bytes: usize,

    /// Deterministic failure rate for the built-in mock LLM, from 0.0 to 1.0.
    #[arg(long, default_value_t = 0.0)]
    pub(crate) mock_llm_failure_rate: f64,

    /// Emit live progress to stderr every N seconds. Set to 0 to disable.
    #[arg(long, default_value_t = 1)]
    pub(crate) progress_interval_seconds: u64,

    /// Emit a human-readable summary table to stderr after the JSON summary.
    #[arg(long, default_value_t = false)]
    pub(crate) human_read: bool,

    /// Emit a heuristic bottleneck report to stderr after the JSON summary.
    #[arg(long, default_value_t = false)]
    pub(crate) bottleneck_report: bool,

    /// Compare this run's JSON summary against a previous JSON or JSONL output file.
    #[arg(long)]
    pub(crate) compare_json: Option<PathBuf>,

    /// Ramp --concurrency from the current value to this maximum, stopping at the first threshold breach.
    #[arg(long)]
    pub(crate) ramp_concurrency: Option<usize>,

    /// Ramp --users from the current value to this maximum, stopping at the first threshold breach.
    #[arg(long)]
    pub(crate) ramp_users: Option<usize>,

    /// Multiplier between ramp points.
    #[arg(long, default_value_t = 2)]
    pub(crate) ramp_factor: usize,

    /// Comma-separated concurrency values to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_concurrency: Vec<usize>,

    /// Comma-separated synthetic user counts to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_users: Vec<usize>,

    /// Comma-separated active thread counts to sweep. Use 0 for one thread per user.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_active_thread_count: Vec<usize>,

    /// Comma-separated model latency values to sweep for mixed-user-session.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_model_latency_ms: Vec<u64>,

    /// Comma-separated user message byte sizes to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_user_message_bytes: Vec<usize>,

    /// Comma-separated assistant message byte sizes to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_assistant_message_bytes: Vec<usize>,

    /// Comma-separated context load message counts to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_context_max_messages: Vec<usize>,

    /// Comma-separated context-growth turns per operation to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_context_growth_turns_per_operation: Vec<usize>,

    /// Comma-separated tool-call counts per turn to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_tool_calls_per_turn: Vec<usize>,

    /// Comma-separated tool output byte sizes to sweep.
    #[arg(long, value_delimiter = ',')]
    pub(crate) sweep_tool_output_bytes: Vec<usize>,

    /// Repetitions per sweep point.
    #[arg(long, default_value_t = 1)]
    pub(crate) repetitions: usize,

    /// Write one JSON object per sweep run to this file.
    #[arg(long)]
    pub(crate) output_jsonl: Option<PathBuf>,

    /// Write interval trace samples as JSONL. Multi-process runs write child-specific files.
    #[arg(long)]
    pub(crate) trace_jsonl: Option<PathBuf>,

    /// Seconds between trace JSONL samples.
    #[arg(long, default_value_t = 1)]
    pub(crate) trace_interval_seconds: u64,

    /// Fail when any run's failure rate is above this value, e.g. 0.01.
    #[arg(long)]
    pub(crate) max_failure_rate: Option<f64>,

    /// Fail when any run's p95 latency is above this many milliseconds.
    #[arg(long)]
    pub(crate) max_p95_ms: Option<u64>,

    /// Fail when any run's throughput is below this operations/sec value.
    #[arg(long)]
    pub(crate) min_throughput: Option<f64>,

    /// Fail when any run's peak RSS is above this many MiB.
    #[arg(long)]
    pub(crate) max_rss_mb: Option<u64>,

    /// Fail when any run's CPU time is above this many milliseconds.
    #[arg(long)]
    pub(crate) max_cpu_ms: Option<u128>,

    /// Synthetic model latency for mixed-user-session operations.
    #[arg(long, default_value_t = 0)]
    pub(crate) model_latency_ms: u64,

    /// Source for mixed-user-session model waits. `provider` sends real LLM requests.
    #[arg(long, value_enum, default_value_t = ModelLatencySource::Synthetic)]
    pub(crate) model_latency_source: ModelLatencySource,

    /// Optional per-request model override for provider-backed model latency.
    #[arg(long)]
    pub(crate) provider_model: Option<String>,

    /// Max output tokens for provider-backed model latency requests.
    #[arg(long, default_value_t = 16)]
    pub(crate) provider_max_tokens: u32,

    /// Synthetic model latency profile for mixed-user-session operations.
    #[arg(long, value_enum, default_value_t = ModelLatencyProfile::Fixed)]
    pub(crate) model_latency_profile: ModelLatencyProfile,

    /// Additional deterministic jitter ceiling for uniform model latency.
    #[arg(long, default_value_t = 0)]
    pub(crate) model_latency_jitter_ms: u64,

    /// Every Nth model wait uses the spike latency for tail-spike profile. Set to 0 to disable.
    #[arg(long, default_value_t = 0)]
    pub(crate) model_latency_spike_every: usize,

    /// Spike latency for tail-spike profile. Defaults to base latency when 0.
    #[arg(long, default_value_t = 0)]
    pub(crate) model_latency_spike_ms: u64,

    /// Minimum user message payload bytes written per turn. Set to 0 for compact default text.
    #[arg(long, default_value_t = 0)]
    pub(crate) user_message_bytes: usize,

    /// Minimum assistant message payload bytes written per turn. Set to 0 for compact default text.
    #[arg(long, default_value_t = 0)]
    pub(crate) assistant_message_bytes: usize,

    /// Messages requested during context-window loads.
    #[arg(long, default_value_t = 20)]
    pub(crate) context_max_messages: usize,

    /// Threads to seed under one scope for the thread-list read workload.
    #[arg(long, default_value_t = 1000)]
    pub(crate) thread_list_threads: usize,

    /// Page size used while walking the thread-list workload.
    #[arg(long, default_value_t = 50)]
    pub(crate) thread_list_page_size: usize,

    /// Sequential chat turns written per context-growth operation.
    #[arg(long, default_value_t = 4)]
    pub(crate) context_growth_turns_per_operation: usize,

    /// Synthetic tool calls written per tool-session turn.
    #[arg(long, default_value_t = 2)]
    pub(crate) tool_calls_per_turn: usize,

    /// Synthetic latency per tool call in tool-session.
    #[arg(long, default_value_t = 0)]
    pub(crate) tool_latency_ms: u64,

    /// Bytes written into each synthetic capability preview output.
    #[arg(long, default_value_t = 1024)]
    pub(crate) tool_output_bytes: usize,

    /// Every Nth synthetic tool result is recorded as failed. Set to 0 to disable.
    #[arg(long, default_value_t = 0)]
    pub(crate) tool_failure_every: usize,

    /// Emit structured stderr spans for failed user-turn operations.
    #[arg(long, default_value_t = false)]
    pub(crate) span_log_failures: bool,

    /// Emit structured stderr spans for user-turn operations at or above this latency.
    /// Set to 0 to disable.
    #[arg(long, default_value_t = 0)]
    pub(crate) slow_span_threshold_ms: u64,

    /// Max structured spans to emit per process. Set to 0 for unlimited.
    #[arg(long, default_value_t = 100)]
    pub(crate) span_sample_limit: usize,

    /// CPU loop iterations per cpu-burn operation.
    #[arg(long, default_value_t = 250_000)]
    pub(crate) cpu_work_units: u64,

    /// Bytes allocated and touched per memory-churn operation.
    #[arg(long, default_value_t = 1_048_576)]
    pub(crate) memory_bytes: usize,

    /// Milliseconds to hold each memory allocation before dropping it.
    #[arg(long, default_value_t = 0)]
    pub(crate) memory_hold_ms: u64,

    #[arg(long, hide = true)]
    pub(crate) child_index: Option<usize>,

    #[arg(long, hide = true, default_value_t = false)]
    pub(crate) warmup_phase: bool,

    #[arg(long, hide = true)]
    pub(crate) suite_case_label: Option<String>,
}

impl Args {
    pub(crate) fn uses_duration_mode(&self) -> bool {
        self.duration_seconds > 0
    }

    pub(crate) fn operation_target(&self) -> OperationTarget {
        if self.uses_duration_mode() {
            OperationTarget::Duration {
                duration: Duration::from_secs(self.duration_seconds),
            }
        } else {
            OperationTarget::Fixed {
                operations_per_worker: self.operations,
                total_operations: self.concurrency.saturating_mul(self.operations),
            }
        }
    }

    pub(crate) fn initial_worker_sample_capacity(&self) -> usize {
        if self.uses_duration_mode() {
            self.operations.clamp(1, 1024)
        } else {
            self.operations
        }
    }

    pub(crate) fn turn_state_store_limits(&self) -> ironclaw_turns::TurnStateStoreLimits {
        let defaults = ironclaw_turns::TurnStateStoreLimits::default();
        ironclaw_turns::TurnStateStoreLimits {
            max_events: self.turn_state_max_events.unwrap_or(defaults.max_events),
            max_terminal_records: self
                .turn_state_max_terminal_records
                .unwrap_or(defaults.max_terminal_records),
            max_idempotency_records: self
                .turn_state_max_idempotency_records
                .unwrap_or(defaults.max_idempotency_records),
            ..defaults
        }
    }

    pub(crate) fn warmup_args(&self) -> Option<Self> {
        if self.warmup_seconds == 0 {
            return None;
        }
        let mut args = self.clone();
        args.duration_seconds = self.warmup_seconds;
        args.warmup_seconds = 0;
        args.warmup_phase = true;
        Some(args)
    }

    pub(crate) fn prefill_enabled(&self) -> bool {
        self.prefill_threads > 0 || self.prefill_turns_per_thread > 0
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum OperationTarget {
    Fixed {
        operations_per_worker: usize,
        total_operations: usize,
    },
    Duration {
        duration: Duration,
    },
}

impl OperationTarget {
    pub(crate) fn progress_total(self) -> Option<usize> {
        match self {
            Self::Fixed {
                total_operations, ..
            } => Some(total_operations),
            Self::Duration { .. } => None,
        }
    }

    pub(crate) fn label(self) -> String {
        match self {
            Self::Fixed {
                total_operations, ..
            } => format!("total_operations={total_operations}"),
            Self::Duration { duration } => {
                format!("duration_seconds={}", duration.as_secs())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Backend {
    Libsql,
    Postgres,
}

impl Backend {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Libsql => "libsql",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TurnStateBackend {
    /// Durable per-user `state.json` via the filesystem store (per-step CAS
    /// read-modify-write). The current production path; livelocks under
    /// concurrent same-user writers.
    Filesystem,
    /// Durable typed append-log deltas with one hot in-process store per
    /// tenant/user. Candidate filesystem fix for the blob growth curve.
    FilesystemRow,
    /// The row store over an in-memory `RootFilesystem` backend — the in-memory
    /// turn-state authority: coordination in memory with the durable backend
    /// cost removed. Measures the row-store mechanism's overhead in isolation.
    RowMemory,
}

impl TurnStateBackend {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Filesystem => "filesystem",
            Self::FilesystemRow => "filesystem-row",
            Self::RowMemory => "row-memory",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ModelLatencySource {
    Synthetic,
    Provider,
}

impl ModelLatencySource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Synthetic => "synthetic",
            Self::Provider => "provider",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ModelLatencyProfile {
    Fixed,
    Uniform,
    TailSpike,
}

impl ModelLatencyProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Uniform => "uniform",
            Self::TailSpike => "tail-spike",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StressPreset {
    ChatBaseline,
    HotThread,
    LargeContext,
    ToolHeavy,
    ModelTail,
    ResourceContention,
    CpuBurn,
    MemoryChurn,
    SoakUserSession,
}

impl StressPreset {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ChatBaseline => "chat-baseline",
            Self::HotThread => "hot-thread",
            Self::LargeContext => "large-context",
            Self::ToolHeavy => "tool-heavy",
            Self::ModelTail => "model-tail",
            Self::ResourceContention => "resource-contention",
            Self::CpuBurn => "cpu-burn",
            Self::MemoryChurn => "memory-churn",
            Self::SoakUserSession => "soak-user-session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StressSuite {
    BottleneckFinder,
    PostgresPoolPressure,
}

impl StressSuite {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BottleneckFinder => "bottleneck-finder",
            Self::PostgresPoolPressure => "postgres-pool-pressure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Scenario {
    ReserveRelease,
    ReserveReconcile,
    ChatTurn,
    TurnLifecycleChurn,
    ThreadList,
    MixedUserSession,
    ContextGrowth,
    ToolSession,
    ApiUserCapacity,
    SecretConsume,
    CpuBurn,
    MemoryChurn,
}

impl Scenario {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReserveRelease => "reserve-release",
            Self::ReserveReconcile => "reserve-reconcile",
            Self::ChatTurn => "chat-turn",
            Self::TurnLifecycleChurn => "turn-lifecycle-churn",
            Self::ThreadList => "thread-list",
            Self::MixedUserSession => "mixed-user-session",
            Self::ContextGrowth => "context-growth",
            Self::ToolSession => "tool-session",
            Self::ApiUserCapacity => "api-user-capacity",
            Self::SecretConsume => "secret-consume",
            Self::CpuBurn => "cpu-burn",
            Self::MemoryChurn => "memory-churn",
        }
    }

    pub(crate) fn is_resource_governor(self) -> bool {
        matches!(self, Self::ReserveRelease | Self::ReserveReconcile)
    }

    pub(crate) fn is_user_turn(self) -> bool {
        matches!(
            self,
            Self::ChatTurn
                | Self::TurnLifecycleChurn
                | Self::ThreadList
                | Self::MixedUserSession
                | Self::ContextGrowth
                | Self::ToolSession
        )
    }

    pub(crate) fn is_api_capacity(self) -> bool {
        matches!(self, Self::ApiUserCapacity)
    }

    pub(crate) fn is_secret_control_plane(self) -> bool {
        matches!(self, Self::SecretConsume)
    }

    pub(crate) fn is_process_local(self) -> bool {
        matches!(self, Self::CpuBurn | Self::MemoryChurn)
    }

    fn prewarm_target(self) -> PrewarmTarget {
        if self.is_process_local() {
            PrewarmTarget::None
        } else if self.is_resource_governor() {
            PrewarmTarget::ResourceGovernor
        } else if self.is_secret_control_plane() {
            PrewarmTarget::SecretControlPlane
        } else {
            PrewarmTarget::UserTurn
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrewarmTarget {
    None,
    ResourceGovernor,
    SecretControlPlane,
    UserTurn,
}

struct BackendHandle {
    governor: Arc<dyn ResourceGovernor>,
    target: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Sample {
    pub(crate) latency: Duration,
    pub(crate) error: Option<String>,
    pub(crate) failure: Option<FailureCause>,
    pub(crate) stages: Option<UserTurnStageDurations>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunSummary {
    backend: Backend,
    preset: Option<StressPreset>,
    scenario: Scenario,
    run_id: String,
    target: String,
    child_index: Option<usize>,
    processes: usize,
    concurrency: usize,
    operations_per_thread: usize,
    duration_seconds: u64,
    warmup_seconds: u64,
    trace_jsonl_enabled: bool,
    trace_interval_seconds: u64,
    users: usize,
    active_thread_count: usize,
    threads_per_owner: usize,
    turn_state_backend: TurnStateBackend,
    turn_state_max_terminal_records: Option<usize>,
    turn_state_max_events: Option<usize>,
    turn_state_max_idempotency_records: Option<usize>,
    gate_blocked_every: usize,
    tenants: usize,
    prefill_threads: usize,
    prefill_turns_per_thread: usize,
    prefill_concurrency: usize,
    model_latency_ms: u64,
    model_latency_source: ModelLatencySource,
    provider_model: Option<String>,
    provider_max_tokens: u32,
    model_latency_profile: ModelLatencyProfile,
    model_latency_jitter_ms: u64,
    model_latency_spike_every: usize,
    model_latency_spike_ms: u64,
    user_message_bytes: usize,
    assistant_message_bytes: usize,
    context_max_messages: usize,
    thread_list_threads: usize,
    thread_list_page_size: usize,
    context_growth_turns_per_operation: usize,
    tool_calls_per_turn: usize,
    tool_latency_ms: u64,
    tool_output_bytes: usize,
    tool_failure_every: usize,
    attempted: u64,
    succeeded: u64,
    failed: u64,
    duration_ms: u128,
    throughput_ops_sec: f64,
    latency: LatencySummary,
    process: ProcessMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    db_probe: Option<DbProbeSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prefill: Option<user_turn::PrefillSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_attribution: Option<UserTurnOperationAttributionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage_latency: Option<UserTurnStageLatencySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_capacity: Option<ApiCapacitySummary>,
    errors: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    failure_causes: BTreeMap<String, FailureCauseSummary>,
}

struct SummaryInput {
    target: String,
    elapsed: Duration,
    samples: Vec<Sample>,
    process: ProcessMetrics,
    db_probe: Option<DbProbeSummary>,
    prefill: Option<user_turn::PrefillSummary>,
    api_capacity: Option<ApiCapacitySummary>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = parse_args()?;
    validate_args(&args)?;

    let run_id = args
        .run_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    args.run_id = Some(run_id.clone());
    let generated_libsql_path = if args.child_index.is_none()
        && matches!(args.backend, Backend::Libsql)
        && !args.scenario.is_api_capacity()
        && args.libsql_path.is_none()
    {
        let path = default_libsql_path();
        args.libsql_path = Some(path.clone());
        Some(path)
    } else {
        None
    };

    let result = if args.child_index.is_none() && args.suite.is_some() {
        suite::run(&args, &run_id).await
    } else if args.child_index.is_none() && ramp::is_enabled(&args) {
        ramp::run(&args, &run_id).await
    } else if args.child_index.is_none() && sweep::is_enabled(&args) {
        trace::prepare_trace_outputs(&args).await?;
        sweep::run(&args, &run_id).await
    } else {
        trace::prepare_trace_outputs(&args).await?;
        run_once(&args, &run_id)
            .await
            .and_then(|captured| {
                report::print_captured_run(&args, &run_id, &captured).map(|_| captured)
            })
            .and_then(|captured| {
                let metrics = captured.metrics();
                sweep::enforce_thresholds(&args, &[("run".to_string(), metrics)])
            })
    };

    if let Some(path) = generated_libsql_path {
        cleanup_generated_libsql_path(&path).await;
    }

    result
}

fn parse_args() -> Result<Args, String> {
    let matches = Args::command().get_matches();
    parse_args_from_matches(&matches)
}

fn parse_args_from_matches(matches: &ArgMatches) -> Result<Args, String> {
    let mut args = Args::from_arg_matches(matches).map_err(|error| error.to_string())?;
    apply_preset(&mut args, matches);
    Ok(args)
}

fn apply_preset(args: &mut Args, matches: &ArgMatches) {
    let Some(preset) = args.preset else {
        return;
    };

    macro_rules! set_default {
        ($field:ident = $value:expr) => {
            if arg_is_defaulted(matches, stringify!($field)) {
                args.$field = $value;
            }
        };
    }

    match preset {
        StressPreset::ChatBaseline => {
            set_default!(scenario = Scenario::ChatTurn);
            set_default!(concurrency = 4);
            set_default!(operations = 100);
            set_default!(users = 100);
            set_default!(active_thread_count = 0);
        }
        StressPreset::HotThread => {
            set_default!(scenario = Scenario::ChatTurn);
            set_default!(concurrency = 8);
            set_default!(operations = 100);
            set_default!(users = 100);
            set_default!(active_thread_count = 1);
            set_default!(span_log_failures = true);
        }
        StressPreset::LargeContext => {
            set_default!(scenario = Scenario::MixedUserSession);
            set_default!(concurrency = 4);
            set_default!(operations = 50);
            set_default!(users = 100);
            set_default!(active_thread_count = 0);
            set_default!(prefill_threads = args.users);
            set_default!(prefill_turns_per_thread = 50);
            set_default!(prefill_concurrency = 8);
            set_default!(context_max_messages = 100);
            set_default!(user_message_bytes = 512);
            set_default!(assistant_message_bytes = 1024);
        }
        StressPreset::ToolHeavy => {
            set_default!(scenario = Scenario::ToolSession);
            set_default!(concurrency = 4);
            set_default!(operations = 50);
            set_default!(users = 50);
            set_default!(active_thread_count = 0);
            set_default!(tool_calls_per_turn = 8);
            set_default!(tool_output_bytes = 4096);
            set_default!(assistant_message_bytes = 1024);
        }
        StressPreset::ModelTail => {
            set_default!(scenario = Scenario::MixedUserSession);
            set_default!(concurrency = 6);
            set_default!(operations = 50);
            set_default!(users = 100);
            set_default!(model_latency_ms = 100);
            set_default!(model_latency_profile = ModelLatencyProfile::TailSpike);
            set_default!(model_latency_spike_every = 10);
            set_default!(model_latency_spike_ms = 2000);
        }
        StressPreset::ResourceContention => {
            set_default!(scenario = Scenario::ReserveReconcile);
            set_default!(concurrency = 8);
            set_default!(operations = 200);
            set_default!(users = 100);
        }
        StressPreset::CpuBurn => {
            set_default!(scenario = Scenario::CpuBurn);
            set_default!(concurrency = 4);
            set_default!(operations = 100);
            set_default!(cpu_work_units = 1_000_000);
        }
        StressPreset::MemoryChurn => {
            set_default!(scenario = Scenario::MemoryChurn);
            set_default!(concurrency = 4);
            set_default!(operations = 100);
            set_default!(memory_bytes = 16 * 1024 * 1024);
            set_default!(memory_hold_ms = 10);
        }
        StressPreset::SoakUserSession => {
            set_default!(scenario = Scenario::MixedUserSession);
            set_default!(concurrency = 6);
            set_default!(operations = 200);
            set_default!(duration_seconds = 900);
            set_default!(warmup_seconds = 60);
            set_default!(users = 200);
            set_default!(active_thread_count = 0);
            set_default!(prefill_threads = args.users.min(100));
            set_default!(prefill_turns_per_thread = 20);
            set_default!(prefill_concurrency = 8);
            set_default!(context_max_messages = 80);
            set_default!(user_message_bytes = 512);
            set_default!(assistant_message_bytes = 1024);
            set_default!(tool_calls_per_turn = 2);
            set_default!(tool_output_bytes = 1024);
            set_default!(trace_interval_seconds = 30);
        }
    }
}

fn arg_is_defaulted(matches: &ArgMatches, id: &str) -> bool {
    !matches!(matches.value_source(id), Some(ValueSource::CommandLine))
}

pub(crate) async fn run_once(args: &Args, run_id: &str) -> Result<CapturedRun, String> {
    if args.child_index.is_none() && args.processes > 1 {
        prewarm(args, run_id)
            .await
            .and_then(|_| run_child_processes(args, run_id))
            .map(|summaries| CapturedRun::Parent {
                aggregate: report::parent_summary_value(args, run_id, &summaries),
                summaries,
            })
    } else {
        run_in_process(args, run_id)
            .await
            .map(|summary| CapturedRun::Single(Box::new(summary)))
    }
}

fn validate_args(args: &Args) -> Result<(), String> {
    if args.processes == 0 {
        return Err("--processes must be greater than 0".to_string());
    }
    if args.concurrency == 0 {
        return Err("--concurrency must be greater than 0".to_string());
    }
    if args.operations == 0 {
        return Err("--operations must be greater than 0".to_string());
    }
    if args.warmup_seconds > 0 && args.duration_seconds == 0 {
        return Err("--warmup-seconds requires --duration-seconds".to_string());
    }
    if args.users == 0 {
        return Err("--users must be greater than 0".to_string());
    }
    if args.tenants == 0 {
        return Err("--tenants must be greater than 0".to_string());
    }
    if args.postgres_pool_size == 0 {
        return Err("--postgres-pool-size must be greater than 0".to_string());
    }
    if args.scenario.is_api_capacity() {
        if args.api_base_url.is_none() {
            return Err("--api-base-url is required for --scenario api-user-capacity".to_string());
        }
        if args.processes > 1 {
            return Err("--scenario api-user-capacity requires --processes 1".to_string());
        }
    } else if args.api_admin_bearer_token.is_some() {
        return Err(
            "--api-admin-bearer-token is only valid for --scenario api-user-capacity".to_string(),
        );
    }
    if args.api_read_qps_per_user < 0.0 {
        return Err("--api-read-qps-per-user must be greater than or equal to 0".to_string());
    }
    if args.api_page_size == 0 {
        return Err("--api-page-size must be greater than 0".to_string());
    }
    if args.api_terminal_timeout_ms == 0 {
        return Err("--api-terminal-timeout-ms must be greater than 0".to_string());
    }
    if args.api_poll_interval_ms == 0 {
        return Err("--api-poll-interval-ms must be greater than 0".to_string());
    }
    if args.api_request_timeout_ms == 0 {
        return Err("--api-request-timeout-ms must be greater than 0".to_string());
    }
    if args.api_setup_concurrency == 0 {
        return Err("--api-setup-concurrency must be greater than 0".to_string());
    }
    if args.api_admin_provisioners == 0 {
        return Err("--api-admin-provisioners must be greater than 0".to_string());
    }
    if args.api_background_users > 0 && args.api_background_operations == 0 {
        return Err(
            "--api-background-operations must be greater than 0 when background users are enabled"
                .to_string(),
        );
    }
    api_capacity::validate_read_mix(&args.api_read_mix)?;
    if !(0.0..=1.0).contains(&args.mock_llm_failure_rate) {
        return Err("--mock-llm-failure-rate must be between 0.0 and 1.0".to_string());
    }
    if matches!(args.model_latency_source, ModelLatencySource::Provider) {
        if !matches!(args.scenario, Scenario::MixedUserSession) {
            return Err(
                "--model-latency-source provider requires --scenario mixed-user-session"
                    .to_string(),
            );
        }
        if args.provider_max_tokens == 0 {
            return Err("--provider-max-tokens must be greater than 0".to_string());
        }
        if args.suite.is_some()
            || ramp::is_enabled(args)
            || sweep::is_enabled(args)
            || args.repetitions > 1
        {
            return Err(
                "--model-latency-source provider cannot be combined with suite, ramp, sweep, or repeated runs"
                    .to_string(),
            );
        }
    }
    if args.repetitions == 0 {
        return Err("--repetitions must be greater than 0".to_string());
    }
    if args.ramp_concurrency.is_some() && args.ramp_users.is_some() {
        return Err("use only one of --ramp-concurrency or --ramp-users".to_string());
    }
    if args.suite.is_some() {
        if args.preset.is_some() {
            return Err("--suite cannot be combined with --preset".to_string());
        }
        if matches!(args.suite, Some(StressSuite::PostgresPoolPressure))
            && !matches!(args.backend, Backend::Postgres)
        {
            return Err("--suite postgres-pool-pressure requires --backend postgres".to_string());
        }
        if args.processes > 1 {
            return Err(
                "--suite requires --processes 1 because it includes user-turn scenarios"
                    .to_string(),
            );
        }
        if args.ramp_concurrency.is_some()
            || args.ramp_users.is_some()
            || !args.sweep_concurrency.is_empty()
            || !args.sweep_users.is_empty()
            || !args.sweep_active_thread_count.is_empty()
            || !args.sweep_model_latency_ms.is_empty()
            || !args.sweep_user_message_bytes.is_empty()
            || !args.sweep_assistant_message_bytes.is_empty()
            || !args.sweep_context_max_messages.is_empty()
            || !args.sweep_context_growth_turns_per_operation.is_empty()
            || !args.sweep_tool_calls_per_turn.is_empty()
            || !args.sweep_tool_output_bytes.is_empty()
            || args.repetitions > 1
        {
            return Err(
                "--suite cannot be combined with ramp flags, sweep flags, or --repetitions > 1"
                    .to_string(),
            );
        }
    }
    if args.ramp_factor < 2 {
        return Err("--ramp-factor must be greater than 1".to_string());
    }
    if let Some(max_concurrency) = args.ramp_concurrency {
        if max_concurrency == 0 {
            return Err("--ramp-concurrency must be greater than 0".to_string());
        }
        if max_concurrency < args.concurrency {
            return Err(
                "--ramp-concurrency must be greater than or equal to --concurrency".to_string(),
            );
        }
    }
    if let Some(max_users) = args.ramp_users {
        if max_users == 0 {
            return Err("--ramp-users must be greater than 0".to_string());
        }
        if max_users < args.users {
            return Err("--ramp-users must be greater than or equal to --users".to_string());
        }
    }
    if ramp::is_enabled(args)
        && (!args.sweep_concurrency.is_empty()
            || !args.sweep_users.is_empty()
            || !args.sweep_active_thread_count.is_empty()
            || !args.sweep_model_latency_ms.is_empty()
            || !args.sweep_user_message_bytes.is_empty()
            || !args.sweep_assistant_message_bytes.is_empty()
            || !args.sweep_context_max_messages.is_empty()
            || !args.sweep_context_growth_turns_per_operation.is_empty()
            || !args.sweep_tool_calls_per_turn.is_empty()
            || !args.sweep_tool_output_bytes.is_empty()
            || args.repetitions > 1)
    {
        return Err(
            "ramp mode cannot be combined with sweep flags or --repetitions > 1".to_string(),
        );
    }
    if args.sweep_concurrency.contains(&0) {
        return Err("--sweep-concurrency values must be greater than 0".to_string());
    }
    if args.sweep_users.contains(&0) {
        return Err("--sweep-users values must be greater than 0".to_string());
    }
    let max_concurrency = args
        .sweep_concurrency
        .iter()
        .copied()
        .max()
        .unwrap_or(args.concurrency);
    let max_concurrency = args
        .ramp_concurrency
        .map_or(max_concurrency, |ramp_max| max_concurrency.max(ramp_max));
    let min_user_count = args.sweep_users.iter().copied().min().unwrap_or(args.users);
    let max_active_thread_count = args
        .sweep_active_thread_count
        .iter()
        .copied()
        .max()
        .unwrap_or(args.active_thread_count);
    if args.active_thread_count > args.users {
        return Err("--active-thread-count must be less than or equal to --users".to_string());
    }
    if max_active_thread_count > min_user_count {
        return Err(
            "--active-thread-count and --sweep-active-thread-count values must be less than or equal to every --sweep-users value"
                .to_string(),
        );
    }
    if args.scenario.is_user_turn()
        && args
            .sweep_active_thread_count
            .iter()
            .copied()
            .chain(std::iter::once(args.active_thread_count))
            .any(|active_thread_count| active_thread_count == 0)
        && max_concurrency > min_user_count
    {
        return Err(
            "user-turn scenarios with --active-thread-count 0 require --users to be greater than or equal to --concurrency"
                .to_string(),
        );
    }
    if args.sweep_context_max_messages.contains(&0) {
        return Err("--sweep-context-max-messages values must be greater than 0".to_string());
    }
    if args.sweep_context_growth_turns_per_operation.contains(&0) {
        return Err(
            "--sweep-context-growth-turns-per-operation values must be greater than 0".to_string(),
        );
    }
    if args.sweep_tool_calls_per_turn.contains(&0) {
        return Err("--sweep-tool-calls-per-turn values must be greater than 0".to_string());
    }
    if args
        .sweep_tool_output_bytes
        .iter()
        .any(|bytes| *bytes > 16 * 1024)
    {
        return Err("--sweep-tool-output-bytes values must be at most 16384".to_string());
    }
    if let Some(max_failure_rate) = args.max_failure_rate
        && !(0.0..=1.0).contains(&max_failure_rate)
    {
        return Err("--max-failure-rate must be between 0.0 and 1.0".to_string());
    }
    if let Some(min_throughput) = args.min_throughput
        && min_throughput < 0.0
    {
        return Err("--min-throughput must be greater than or equal to 0".to_string());
    }
    if matches!(args.max_rss_mb, Some(0)) {
        return Err("--max-rss-mb must be greater than 0".to_string());
    }
    if matches!(args.max_cpu_ms, Some(0)) {
        return Err("--max-cpu-ms must be greater than 0".to_string());
    }
    if args.cpu_work_units == 0 {
        return Err("--cpu-work-units must be greater than 0".to_string());
    }
    if args.memory_bytes == 0 {
        return Err("--memory-bytes must be greater than 0".to_string());
    }
    if args.context_max_messages == 0 {
        return Err("--context-max-messages must be greater than 0".to_string());
    }
    if args.thread_list_threads == 0 {
        return Err("--thread-list-threads must be greater than 0".to_string());
    }
    if args.thread_list_page_size == 0 || args.thread_list_page_size > 200 {
        return Err("--thread-list-page-size must be between 1 and 200".to_string());
    }
    if args.context_growth_turns_per_operation == 0 {
        return Err("--context-growth-turns-per-operation must be greater than 0".to_string());
    }
    if args.tool_calls_per_turn == 0 {
        return Err("--tool-calls-per-turn must be greater than 0".to_string());
    }
    if args.tool_output_bytes > 16 * 1024 {
        return Err("--tool-output-bytes must be at most 16384".to_string());
    }
    if args.prefill_concurrency == 0 {
        return Err("--prefill-concurrency must be greater than 0".to_string());
    }
    if args.prefill_enabled() {
        if args.prefill_threads == 0 || args.prefill_turns_per_thread == 0 {
            return Err(
                "--prefill-threads and --prefill-turns-per-thread must both be greater than 0"
                    .to_string(),
            );
        }
        if !args.scenario.is_user_turn() {
            return Err(format!(
                "--prefill-threads requires a user-turn scenario, got {}",
                args.scenario.as_str()
            ));
        }
        if args.prefill_threads > args.users {
            return Err("--prefill-threads must be less than or equal to --users".to_string());
        }
        // Prefill warms one thread per owner (`user_turn_context_for_user_index`,
        // no slot), while measured runs spread across `threads_per_owner`
        // slot-suffixed threads. Combining the two would warm different thread
        // IDs than the workload benchmarks, so reject it rather than silently
        // prefilling the wrong threads.
        if args.threads_per_owner > 1 {
            return Err(
                "--prefill-threads is incompatible with --threads-per-owner > 1 (prefill would \
                 warm different thread IDs than the measured slotted threads)"
                    .to_string(),
            );
        }
        if let Some(min_sweep_users) = args.sweep_users.iter().min()
            && args.prefill_threads > *min_sweep_users
        {
            return Err(
                "--prefill-threads must be less than or equal to every --sweep-users value"
                    .to_string(),
            );
        }
    }
    if args.scenario.is_user_turn() && args.processes > 1 {
        return Err(format!(
            "--scenario {} requires --processes 1",
            args.scenario.as_str()
        ));
    }
    Ok(())
}

async fn prewarm(args: &Args, run_id: &str) -> Result<(), String> {
    eprintln!(
        "{} prewarming backend={} scenario={} run_id={}",
        log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        run_id
    );
    match args.scenario.prewarm_target() {
        PrewarmTarget::None => {
            eprintln!(
                "{} prewarm skipped for process-local scenario",
                log_prefix(args)
            );
        }
        PrewarmTarget::ResourceGovernor => {
            let backend = build_backend(args, run_id).await?;
            let account =
                ResourceAccount::tenant(TenantId::new("stress-prewarm").map_err(display_err)?);
            backend
                .governor
                .account_snapshot(&account)
                .map_err(|error| format!("prewarm failed: {error:?}"))?;
        }
        PrewarmTarget::SecretControlPlane => {
            let workload = Arc::new(build_secret_consume_workload(args, run_id).await?);
            let identities = Arc::new(SyntheticIds::new(args)?);
            secret_ops::prefill_secrets(Arc::clone(&workload), args, identities).await?;
            eprintln!(
                "{} prewarmed target={}",
                log_prefix(args),
                workload.target()
            );
        }
        PrewarmTarget::UserTurn => {
            let workload = build_user_turn_workload(args, run_id).await?;
            eprintln!(
                "{} prewarmed target={}",
                log_prefix(args),
                workload.target()
            );
        }
    }
    Ok(())
}

fn run_child_processes(args: &Args, run_id: &str) -> Result<Vec<RunSummary>, String> {
    let current_exe =
        std::env::current_exe().map_err(|error| format!("resolve current executable: {error}"))?;
    let libsql_path = args.libsql_path.clone().unwrap_or_else(default_libsql_path);

    eprintln!(
        "{} spawning {} child processes total_operations_per_child={} progress_interval_seconds={}",
        log_prefix(args),
        args.processes,
        args.concurrency.saturating_mul(args.operations),
        args.progress_interval_seconds
    );

    let mut children = Vec::with_capacity(args.processes);
    for child_index in 0..args.processes {
        let mut command = Command::new(&current_exe);
        command
            .arg("--backend")
            .arg(args.backend.as_str())
            .arg("--processes")
            .arg("1")
            .arg("--concurrency")
            .arg(args.concurrency.to_string())
            .arg("--operations")
            .arg(args.operations.to_string())
            .arg("--duration-seconds")
            .arg(args.duration_seconds.to_string())
            .arg("--warmup-seconds")
            .arg(args.warmup_seconds.to_string())
            .arg("--users")
            .arg(args.users.to_string())
            .arg("--active-thread-count")
            .arg(args.active_thread_count.to_string())
            .arg("--tenants")
            .arg(args.tenants.to_string())
            .arg("--prefill-threads")
            .arg(args.prefill_threads.to_string())
            .arg("--prefill-turns-per-thread")
            .arg(args.prefill_turns_per_thread.to_string())
            .arg("--prefill-concurrency")
            .arg(args.prefill_concurrency.to_string())
            .arg("--scenario")
            .arg(args.scenario.as_str())
            .arg("--turn-state-backend")
            .arg(args.turn_state_backend.as_str())
            .arg("--postgres-pool-size")
            .arg(args.postgres_pool_size.to_string())
            .arg("--progress-interval-seconds")
            .arg(args.progress_interval_seconds.to_string())
            .arg("--trace-interval-seconds")
            .arg(args.trace_interval_seconds.to_string())
            .arg("--model-latency-ms")
            .arg(args.model_latency_ms.to_string())
            .arg("--model-latency-source")
            .arg(args.model_latency_source.as_str())
            .arg("--provider-max-tokens")
            .arg(args.provider_max_tokens.to_string())
            .arg("--model-latency-profile")
            .arg(args.model_latency_profile.as_str())
            .arg("--model-latency-jitter-ms")
            .arg(args.model_latency_jitter_ms.to_string())
            .arg("--model-latency-spike-every")
            .arg(args.model_latency_spike_every.to_string())
            .arg("--model-latency-spike-ms")
            .arg(args.model_latency_spike_ms.to_string())
            .arg("--user-message-bytes")
            .arg(args.user_message_bytes.to_string())
            .arg("--assistant-message-bytes")
            .arg(args.assistant_message_bytes.to_string())
            .arg("--context-max-messages")
            .arg(args.context_max_messages.to_string())
            .arg("--thread-list-threads")
            .arg(args.thread_list_threads.to_string())
            .arg("--thread-list-page-size")
            .arg(args.thread_list_page_size.to_string())
            .arg("--context-growth-turns-per-operation")
            .arg(args.context_growth_turns_per_operation.to_string())
            .arg("--tool-calls-per-turn")
            .arg(args.tool_calls_per_turn.to_string())
            .arg("--tool-latency-ms")
            .arg(args.tool_latency_ms.to_string())
            .arg("--tool-output-bytes")
            .arg(args.tool_output_bytes.to_string())
            .arg("--tool-failure-every")
            .arg(args.tool_failure_every.to_string())
            .arg("--slow-span-threshold-ms")
            .arg(args.slow_span_threshold_ms.to_string())
            .arg("--span-sample-limit")
            .arg(args.span_sample_limit.to_string())
            .arg("--run-id")
            .arg(run_id)
            .arg("--child-index")
            .arg(child_index.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(preset) = args.preset {
            command.arg("--preset").arg(preset.as_str());
        }
        if let Some(model) = &args.provider_model {
            command.arg("--provider-model").arg(model);
        }
        if let Some(label) = &args.suite_case_label {
            command.arg("--suite-case-label").arg(label);
        }
        if args.span_log_failures {
            command.arg("--span-log-failures");
        }
        if let Some(max_terminal_records) = args.turn_state_max_terminal_records {
            command
                .arg("--turn-state-max-terminal-records")
                .arg(max_terminal_records.to_string());
        }
        if let Some(max_events) = args.turn_state_max_events {
            command
                .arg("--turn-state-max-events")
                .arg(max_events.to_string());
        }
        if let Some(max_idempotency_records) = args.turn_state_max_idempotency_records {
            command
                .arg("--turn-state-max-idempotency-records")
                .arg(max_idempotency_records.to_string());
        }
        if let Some(path) = &args.trace_jsonl {
            command
                .arg("--trace-jsonl")
                .arg(trace::child_trace_path(path, child_index));
        }
        if matches!(args.backend, Backend::Libsql) {
            command.arg("--libsql-path").arg(&libsql_path);
        }
        if let Some(url) = &args.postgres_url {
            command.env("IRONCLAW_FILESYSTEM_POSTGRES_URL", url);
        }
        match command.spawn() {
            Ok(mut child) => {
                let stderr_reader = child
                    .stderr
                    .take()
                    .and_then(|stderr| spawn_child_stderr_reader(child_index, stderr));
                children.push((child_index, child, stderr_reader));
            }
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!("spawn child {child_index}: {error}"));
            }
        }
    }

    let mut summaries = Vec::with_capacity(children.len());
    while !children.is_empty() {
        let (child_index, child, stderr_reader) = children.remove(0);
        let output = match child.wait_with_output() {
            Ok(output) => output,
            Err(error) => {
                join_child_stderr_reader(child_index, stderr_reader);
                terminate_children(&mut children);
                return Err(format!("wait for child {child_index}: {error}"));
            }
        };
        join_child_stderr_reader(child_index, stderr_reader);
        if !output.status.success() {
            terminate_children(&mut children);
            return Err(format!(
                "child {child_index} failed with status {}; see stderr above for child logs",
                output.status
            ));
        }
        let stdout = match String::from_utf8(output.stdout) {
            Ok(stdout) => stdout,
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!(
                    "child {child_index} emitted non-utf8 stdout: {error}"
                ));
            }
        };
        let summary: RunSummary = match serde_json::from_str(stdout.trim()) {
            Ok(summary) => summary,
            Err(error) => {
                terminate_children(&mut children);
                return Err(format!(
                    "parse child {child_index} summary: {error}: {stdout}"
                ));
            }
        };
        summaries.push(summary);
    }
    summaries.sort_by_key(|summary| summary.child_index.unwrap_or(usize::MAX));
    Ok(summaries)
}

async fn run_in_process(args: &Args, run_id: &str) -> Result<RunSummary, String> {
    eprintln!(
        "{} preparing backend={} scenario={} run_id={}",
        log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        run_id
    );
    let operation_target = args.operation_target();
    if args.scenario.is_process_local() {
        return run_process_pressure_in_process(args, run_id, operation_target).await;
    }
    if args.scenario.is_api_capacity() {
        return run_api_capacity_in_process(args, run_id, operation_target).await;
    }
    let identities = Arc::new(SyntheticIds::new(args)?);

    if args.scenario.is_resource_governor() {
        return run_resource_governor_in_process(args, run_id, operation_target, identities).await;
    }
    if args.scenario.is_secret_control_plane() {
        return run_secret_consume_in_process(args, run_id, operation_target, identities).await;
    }

    run_user_turn_in_process(args, run_id, operation_target, identities).await
}

async fn run_process_pressure_in_process(
    args: &Args,
    run_id: &str,
    operation_target: OperationTarget,
) -> Result<RunSummary, String> {
    eprintln!(
        "{} running target=process://local concurrency={} operations_per_thread={} {} warmup_seconds={} progress_interval_seconds={}",
        log_prefix(args),
        args.concurrency,
        args.operations,
        operation_target.label(),
        args.warmup_seconds,
        args.progress_interval_seconds
    );
    if let Some(warmup_args) = args.warmup_args() {
        eprintln!(
            "{} warming up target=process://local duration_seconds={}",
            log_prefix(args),
            warmup_args.duration_seconds
        );
        let _ = tokio::task::spawn_blocking(move || process_pressure::run(&warmup_args))
            .await
            .map_err(|error| {
                if error.is_panic() {
                    eprintln!("process pressure warmup task panicked: {error:?}");
                    "process pressure warmup task panicked".to_string()
                } else {
                    eprintln!("process pressure warmup task cancelled: {error:?}");
                    "process pressure warmup task cancelled".to_string()
                }
            })??;
    }
    let metrics = ProcessMetricsSampler::start(Duration::from_millis(100));
    let started = Instant::now();
    let args_clone = args.clone();
    let samples = tokio::task::spawn_blocking(move || process_pressure::run(&args_clone))
        .await
        .map_err(|error| {
            if error.is_panic() {
                eprintln!("process pressure task panicked: {error:?}");
                "process pressure task panicked".to_string()
            } else {
                eprintln!("process pressure task cancelled: {error:?}");
                "process pressure task cancelled".to_string()
            }
        })??;
    let elapsed = started.elapsed();
    let process = metrics.finish();
    let summary = summarize(
        args,
        run_id,
        SummaryInput {
            target: "process://local".to_string(),
            elapsed,
            samples,
            process,
            db_probe: None,
            prefill: None,
            api_capacity: None,
        },
    );
    eprintln!(
        "{} finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    Ok(summary)
}

async fn run_api_capacity_in_process(
    args: &Args,
    run_id: &str,
    operation_target: OperationTarget,
) -> Result<RunSummary, String> {
    let target = args
        .api_base_url
        .clone()
        .unwrap_or_else(|| "api://unconfigured".to_string());
    let progress_total = match operation_target {
        OperationTarget::Fixed { .. } => Some(args.users.saturating_mul(args.operations)),
        OperationTarget::Duration { .. } => None,
    };
    let workload_label = match operation_target {
        OperationTarget::Fixed { .. } => {
            format!(
                "total_operations={}",
                args.users.saturating_mul(args.operations)
            )
        }
        OperationTarget::Duration { .. } => operation_target.label(),
    };
    eprintln!(
        "{} running target={} virtual_users={} operations_per_user={} {} warmup_seconds={} read_qps_per_user={:.2} progress_interval_seconds={}",
        log_prefix(args),
        target,
        args.users,
        args.operations,
        workload_label,
        args.warmup_seconds,
        args.api_read_qps_per_user,
        args.progress_interval_seconds
    );
    let progress = Arc::new(ProgressCounters::new(args.trace_jsonl.is_some()));
    let progress_reporter = spawn_progress_reporter(
        log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        args.progress_interval_seconds,
        progress_total,
        Arc::clone(&progress),
    );
    let trace_reporter = trace::spawn_trace_reporter(args, &target, Arc::clone(&progress));
    let metrics = ProcessMetricsSampler::start(Duration::from_millis(100));
    let api_run = api_capacity::run(args, run_id, progress).await;
    trace::stop_trace_reporter(trace_reporter);
    stop_progress_reporter(progress_reporter);
    let api_run = api_run?;
    let process = metrics.finish();
    let summary = summarize(
        args,
        run_id,
        SummaryInput {
            target: api_run.target,
            elapsed: api_run.elapsed,
            samples: api_run.samples,
            process,
            db_probe: None,
            prefill: None,
            api_capacity: Some(api_run.summary),
        },
    );
    eprintln!(
        "{} finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    Ok(summary)
}

async fn run_resource_governor_in_process(
    args: &Args,
    run_id: &str,
    operation_target: OperationTarget,
    identities: Arc<SyntheticIds>,
) -> Result<RunSummary, String> {
    let backend = build_backend(args, run_id).await?;
    eprintln!(
        "{} running target={} concurrency={} operations_per_thread={} {} warmup_seconds={} users={} tenants={} progress_interval_seconds={}",
        log_prefix(args),
        backend.target,
        args.concurrency,
        args.operations,
        operation_target.label(),
        args.warmup_seconds,
        args.users,
        args.tenants,
        args.progress_interval_seconds
    );
    if let Some(warmup_args) = args.warmup_args() {
        eprintln!(
            "{} warming up target={} duration_seconds={}",
            log_prefix(args),
            backend.target,
            warmup_args.duration_seconds
        );
        let governor = Arc::clone(&backend.governor);
        let identities = Arc::clone(&identities);
        let target = backend.target.clone();
        let _ = tokio::task::spawn_blocking(move || {
            run_threads(&governor, &warmup_args, &identities, &target)
        })
        .await
        .map_err(|error| {
            if error.is_panic() {
                eprintln!("run_threads warmup task panicked: {error:?}");
                "run_threads warmup task panicked".to_string()
            } else {
                eprintln!("run_threads warmup task cancelled: {error:?}");
                "run_threads warmup task cancelled".to_string()
            }
        })??;
    }
    let metrics = ProcessMetricsSampler::start(Duration::from_millis(100));
    let db_probe_before = db_probe::capture(args).await;
    let started = Instant::now();
    let governor = Arc::clone(&backend.governor);
    let args_clone = args.clone();
    let target_clone = backend.target.clone();
    let samples = tokio::task::spawn_blocking(move || {
        run_threads(&governor, &args_clone, &identities, &target_clone)
    })
    .await
    .map_err(|error| {
        if error.is_panic() {
            eprintln!("run_threads task panicked: {error:?}");
            "run_threads task panicked".to_string()
        } else {
            eprintln!("run_threads task cancelled: {error:?}");
            "run_threads task cancelled".to_string()
        }
    })??;
    let elapsed = started.elapsed();
    let process = metrics.finish();
    let db_probe = db_probe::summarize(db_probe_before, db_probe::capture(args).await);
    let summary = summarize(
        args,
        run_id,
        SummaryInput {
            target: backend.target,
            elapsed,
            samples,
            process,
            db_probe: Some(db_probe),
            prefill: None,
            api_capacity: None,
        },
    );
    eprintln!(
        "{} finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    Ok(summary)
}

async fn run_user_turn_in_process(
    args: &Args,
    run_id: &str,
    operation_target: OperationTarget,
    identities: Arc<SyntheticIds>,
) -> Result<RunSummary, String> {
    let workload = Arc::new(build_user_turn_workload(args, run_id).await?);
    eprintln!(
        "{} running target={} concurrency={} operations_per_task={} {} warmup_seconds={} users={} tenants={} progress_interval_seconds={}",
        log_prefix(args),
        workload.target(),
        args.concurrency,
        args.operations,
        operation_target.label(),
        args.warmup_seconds,
        args.users,
        args.tenants,
        args.progress_interval_seconds
    );
    let prefill = if matches!(args.scenario, Scenario::ThreadList) {
        user_turn::prefill_thread_list(Arc::clone(&workload), args, Arc::clone(&identities)).await?
    } else {
        user_turn::prefill_user_turn_history(Arc::clone(&workload), args, Arc::clone(&identities))
            .await?
    };
    if let Some(warmup_args) = args.warmup_args() {
        eprintln!(
            "{} warming up target={} duration_seconds={}",
            log_prefix(args),
            workload.target(),
            warmup_args.duration_seconds
        );
        let _ = run_user_turn_tasks(Arc::clone(&workload), &warmup_args, Arc::clone(&identities))
            .await?;
    }
    let metrics = ProcessMetricsSampler::start(Duration::from_millis(100));
    let db_probe_before = db_probe::capture(args).await;
    let started = Instant::now();
    let target = workload.target().to_string();
    let samples = run_user_turn_tasks(workload, args, identities).await?;
    let elapsed = started.elapsed();
    let process = metrics.finish();
    let db_probe = db_probe::summarize(db_probe_before, db_probe::capture(args).await);
    let summary = summarize(
        args,
        run_id,
        SummaryInput {
            target,
            elapsed,
            samples,
            process,
            db_probe: Some(db_probe),
            prefill,
            api_capacity: None,
        },
    );
    eprintln!(
        "{} finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    Ok(summary)
}

async fn run_secret_consume_in_process(
    args: &Args,
    run_id: &str,
    operation_target: OperationTarget,
    identities: Arc<SyntheticIds>,
) -> Result<RunSummary, String> {
    let workload = Arc::new(build_secret_consume_workload(args, run_id).await?);
    eprintln!(
        "{} running target={} concurrency={} operations_per_task={} {} warmup_seconds={} users={} tenants={} progress_interval_seconds={}",
        log_prefix(args),
        workload.target(),
        args.concurrency,
        args.operations,
        operation_target.label(),
        args.warmup_seconds,
        args.users,
        args.tenants,
        args.progress_interval_seconds
    );
    secret_ops::prefill_secrets(Arc::clone(&workload), args, Arc::clone(&identities)).await?;
    if let Some(warmup_args) = args.warmup_args() {
        eprintln!(
            "{} warming up target={} duration_seconds={}",
            log_prefix(args),
            workload.target(),
            warmup_args.duration_seconds
        );
        let _ =
            run_secret_consume_tasks(Arc::clone(&workload), &warmup_args, Arc::clone(&identities))
                .await?;
    }
    let metrics = ProcessMetricsSampler::start(Duration::from_millis(100));
    let db_probe_before = db_probe::capture(args).await;
    let started = Instant::now();
    let target = workload.target().to_string();
    let samples = run_secret_consume_tasks(workload, args, identities).await?;
    let elapsed = started.elapsed();
    let process = metrics.finish();
    let db_probe = db_probe::summarize(db_probe_before, db_probe::capture(args).await);
    let summary = summarize(
        args,
        run_id,
        SummaryInput {
            target,
            elapsed,
            samples,
            process,
            db_probe: Some(db_probe),
            prefill: None,
            api_capacity: None,
        },
    );
    eprintln!(
        "{} finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    Ok(summary)
}

fn run_threads(
    governor: &Arc<dyn ResourceGovernor>,
    args: &Args,
    identities: &Arc<SyntheticIds>,
    target: &str,
) -> Result<Vec<Sample>, String> {
    let operation_target = args.operation_target();
    let progress = Arc::new(ProgressCounters::new(args.trace_jsonl.is_some()));
    let progress_reporter = spawn_progress_reporter(
        log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        args.progress_interval_seconds,
        operation_target.progress_total(),
        Arc::clone(&progress),
    );
    let trace_reporter = trace::spawn_trace_reporter(args, target, Arc::clone(&progress));
    let result = run_threads_inner(governor, args, identities, &progress);
    trace::stop_trace_reporter(trace_reporter);
    stop_progress_reporter(progress_reporter);
    result
}

fn run_threads_inner(
    governor: &Arc<dyn ResourceGovernor>,
    args: &Args,
    identities: &Arc<SyntheticIds>,
    progress: &Arc<ProgressCounters>,
) -> Result<Vec<Sample>, String> {
    let (sender, receiver) = mpsc::channel();
    let mut handles = Vec::with_capacity(args.concurrency);

    for worker_index in 0..args.concurrency {
        let governor = Arc::clone(governor);
        let identities = Arc::clone(identities);
        let sender = sender.clone();
        let progress = Arc::clone(progress);
        let args = args.clone();
        let handle = match thread::Builder::new()
            .name(format!("ironclaw-stress-{worker_index}"))
            .spawn(move || -> Result<(), String> {
                let mut samples = Vec::with_capacity(args.initial_worker_sample_capacity());
                let started = Instant::now();
                let mut operation_index = 0;
                while should_run_operation(args.operation_target(), started, operation_index) {
                    let sample = run_one_operation(
                        &governor,
                        &args,
                        &identities,
                        worker_index,
                        operation_index,
                    );
                    progress.record(sample.error.is_some(), sample.latency);
                    samples.push(sample);
                    operation_index += 1;
                }
                sender
                    .send(samples)
                    .map_err(|_| "sample receiver dropped".to_string())
            }) {
            Ok(handle) => handle,
            Err(error) => {
                join_workers(handles)?;
                return Err(format!("spawn worker {worker_index}: {error}"));
            }
        };
        handles.push((worker_index, handle));
    }
    drop(sender);

    let mut samples = Vec::with_capacity(args.concurrency * args.operations);
    for worker_samples in receiver {
        samples.extend(worker_samples);
    }
    join_workers(handles)?;
    if let Some(expected) = args.operation_target().progress_total()
        && samples.len() != expected
    {
        return Err(format!(
            "collected {} samples but expected {expected}",
            samples.len()
        ));
    }
    Ok(samples)
}

fn should_run_operation(
    operation_target: OperationTarget,
    started: Instant,
    operation_index: usize,
) -> bool {
    match operation_target {
        OperationTarget::Fixed {
            operations_per_worker,
            ..
        } => operation_index < operations_per_worker,
        OperationTarget::Duration { duration } => started.elapsed() < duration,
    }
}

fn run_one_operation(
    governor: &Arc<dyn ResourceGovernor>,
    args: &Args,
    identities: &SyntheticIds,
    worker_index: usize,
    operation_index: usize,
) -> Sample {
    let scope = identities.scope(args, worker_index, operation_index);
    let estimate = resource_ops::estimate();
    let usage = resource_ops::usage();

    let started = Instant::now();
    let outcome = match args.scenario {
        Scenario::ReserveRelease => governor
            .reserve(scope, estimate)
            .and_then(|reservation| governor.release(reservation.id).map(|_| ())),
        Scenario::ReserveReconcile => governor
            .reserve(scope, estimate)
            .and_then(|reservation| governor.reconcile(reservation.id, usage).map(|_| ())),
        Scenario::ChatTurn
        | Scenario::TurnLifecycleChurn
        | Scenario::ThreadList
        | Scenario::ContextGrowth
        | Scenario::ToolSession
        | Scenario::SecretConsume => {
            unreachable!("user-turn scenarios use the async user-turn workload")
        }
        Scenario::MixedUserSession => {
            unreachable!("mixed-user-session uses the async user-turn workload")
        }
        Scenario::ApiUserCapacity => {
            unreachable!("api-user-capacity uses the async API workload")
        }
        Scenario::CpuBurn | Scenario::MemoryChurn => {
            unreachable!("process-only scenarios use the local pressure workload")
        }
    };
    let latency = started.elapsed();
    let failure = outcome
        .err()
        .map(|error| resource_ops::failure(args.scenario, error));
    let error = failure.as_ref().map(|cause| cause.bucket.clone());
    Sample {
        latency,
        error,
        failure,
        stages: None,
    }
}

fn summarize(args: &Args, run_id: &str, input: SummaryInput) -> RunSummary {
    let mut errors = BTreeMap::new();
    let mut latencies: Vec<u128> = input
        .samples
        .iter()
        .map(|sample| sample.latency.as_micros())
        .collect();
    latencies.sort_unstable();
    let failed = input
        .samples
        .iter()
        .filter_map(|sample| sample.error.as_ref())
        .map(|error| {
            *errors.entry(error.clone()).or_insert(0) += 1;
        })
        .count() as u64;
    let attempted = input.samples.len() as u64;
    let succeeded = attempted.saturating_sub(failed);
    let elapsed_secs = input.elapsed.as_secs_f64().max(f64::MIN_POSITIVE);

    RunSummary {
        backend: args.backend,
        preset: args.preset,
        scenario: args.scenario,
        run_id: run_id.to_string(),
        target: input.target,
        child_index: args.child_index,
        processes: args.processes,
        concurrency: args.concurrency,
        operations_per_thread: args.operations,
        duration_seconds: args.duration_seconds,
        warmup_seconds: args.warmup_seconds,
        trace_jsonl_enabled: args.trace_jsonl.is_some(),
        trace_interval_seconds: args.trace_interval_seconds,
        users: args.users,
        active_thread_count: args.active_thread_count,
        threads_per_owner: args.threads_per_owner,
        turn_state_backend: args.turn_state_backend,
        turn_state_max_terminal_records: args.turn_state_max_terminal_records,
        turn_state_max_events: args.turn_state_max_events,
        turn_state_max_idempotency_records: args.turn_state_max_idempotency_records,
        gate_blocked_every: args.gate_blocked_every,
        tenants: args.tenants,
        prefill_threads: args.prefill_threads,
        prefill_turns_per_thread: args.prefill_turns_per_thread,
        prefill_concurrency: args.prefill_concurrency,
        model_latency_ms: args.model_latency_ms,
        model_latency_source: args.model_latency_source,
        provider_model: args.provider_model.clone(),
        provider_max_tokens: args.provider_max_tokens,
        model_latency_profile: args.model_latency_profile,
        model_latency_jitter_ms: args.model_latency_jitter_ms,
        model_latency_spike_every: args.model_latency_spike_every,
        model_latency_spike_ms: args.model_latency_spike_ms,
        user_message_bytes: args.user_message_bytes,
        assistant_message_bytes: args.assistant_message_bytes,
        context_max_messages: args.context_max_messages,
        thread_list_threads: args.thread_list_threads,
        thread_list_page_size: args.thread_list_page_size,
        context_growth_turns_per_operation: args.context_growth_turns_per_operation,
        tool_calls_per_turn: args.tool_calls_per_turn,
        tool_latency_ms: args.tool_latency_ms,
        tool_output_bytes: args.tool_output_bytes,
        tool_failure_every: args.tool_failure_every,
        attempted,
        succeeded,
        failed,
        duration_ms: input.elapsed.as_millis(),
        throughput_ops_sec: attempted as f64 / elapsed_secs,
        latency: latency_summary(&latencies),
        process: input.process,
        db_probe: input.db_probe,
        prefill: input.prefill,
        operation_attribution: summarize_user_turn_operation_attribution(&input.samples),
        stage_latency: summarize_user_turn_stages(&input.samples),
        api_capacity: input.api_capacity,
        errors,
        failure_causes: summarize_failure_causes(&input.samples),
    }
}

pub(crate) fn log_prefix(args: &Args) -> String {
    match (args.child_index, args.suite_case_label.as_deref()) {
        (Some(child_index), Some(label)) => {
            format!("[ironclaw-stress child={child_index} case={label}]")
        }
        (Some(child_index), None) => format!("[ironclaw-stress child={child_index}]"),
        (None, Some(label)) => format!("[ironclaw-stress case={label}]"),
        (None, None) => "[ironclaw-stress]".to_string(),
    }
}

async fn build_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    match args.backend {
        Backend::Libsql => build_libsql_backend(args, run_id).await,
        Backend::Postgres => build_postgres_backend(args, run_id).await,
    }
}

async fn build_libsql_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    let (filesystem, target) = build_libsql_root(args).await?;
    Ok(BackendHandle {
        governor: governor_from_root(filesystem, run_id)?,
        target,
    })
}

pub(crate) async fn build_libsql_root(
    args: &Args,
) -> Result<(Arc<ironclaw_filesystem::LibSqlRootFilesystem>, String), String> {
    use ironclaw_filesystem::LibSqlRootFilesystem;

    let path = args.libsql_path.clone().unwrap_or_else(default_libsql_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(display_err)?;
    }
    let db = Arc::new(
        libsql::Builder::new_local(&path)
            .build()
            .await
            .map_err(display_err)?,
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await.map_err(display_err)?;
    Ok((filesystem, redact_libsql_path(&path)))
}

async fn build_postgres_backend(args: &Args, run_id: &str) -> Result<BackendHandle, String> {
    let (filesystem, _pool, target) = build_postgres_root_and_pool(args).await?;
    Ok(BackendHandle {
        governor: governor_from_root(filesystem, run_id)?,
        target,
    })
}

pub(crate) async fn build_postgres_root_and_pool(
    args: &Args,
) -> Result<
    (
        Arc<ironclaw_filesystem::PostgresRootFilesystem>,
        deadpool_postgres::Pool,
        String,
    ),
    String,
> {
    use ironclaw_filesystem::PostgresRootFilesystem;

    let url = resolve_postgres_url(args)?;
    let config = url
        .parse::<tokio_postgres::Config>()
        .map_err(|error| format!("parse Postgres URL: {error}"))?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(args.postgres_pool_size)
        .build()
        .map_err(display_err)?;
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool.clone()));
    filesystem.run_migrations().await.map_err(display_err)?;
    Ok((filesystem, pool, redact_postgres_url(&url)))
}

pub(crate) fn governor_from_root<F>(
    root: Arc<F>,
    run_id: &str,
) -> Result<Arc<dyn ResourceGovernor>, String>
where
    F: RootFilesystem + 'static,
{
    let view = resource_mount_view(run_id)?;
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(root, view));
    Ok(Arc::new(FilesystemResourceGovernor::new(scoped)))
}

fn resource_mount_view(run_id: &str) -> Result<MountView, String> {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").map_err(display_err)?,
        VirtualPath::new(format!("/resources/stress/{run_id}")).map_err(display_err)?,
        MountPermissions::read_write_list_delete(),
    )])
    .map_err(display_err)
}

pub(crate) fn default_libsql_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "ironclaw-stress-{}.db",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn cleanup_generated_libsql_path(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
    ] {
        match tokio::fs::remove_file(&candidate).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "failed to remove generated libSQL file {}: {error}",
                candidate.display()
            ),
        }
    }
}

fn terminate_children(children: &mut Vec<(usize, Child, Option<JoinHandle<()>>)>) {
    for (_, child, _) in children.iter_mut() {
        let _ = child.kill();
    }
    for (child_index, mut child, stderr_reader) in children.drain(..) {
        let _ = child.wait();
        join_child_stderr_reader(child_index, stderr_reader);
    }
}

fn join_workers(handles: Vec<(usize, JoinHandle<Result<(), String>>)>) -> Result<(), String> {
    for (worker_index, handle) in handles {
        match handle.join() {
            Ok(result) => result.map_err(|error| format!("worker {worker_index}: {error}"))?,
            Err(payload) => {
                return Err(format!(
                    "worker {worker_index} panicked: {}",
                    panic_payload_to_string(&payload)
                ));
            }
        }
    }
    Ok(())
}

fn panic_payload_to_string(payload: &Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}

pub(crate) fn resolve_postgres_url(args: &Args) -> Result<String, String> {
    if let Some(url) = args.postgres_url.clone() {
        return Ok(url);
    }
    if let Some(url) = optional_env_var("IRONCLAW_FILESYSTEM_POSTGRES_URL")? {
        return Ok(url);
    }
    // silent-ok: IRONCLAW_FILESYSTEM_POSTGRES_URL is optional; DATABASE_URL is the documented fallback.
    if let Some(url) = optional_env_var("DATABASE_URL")? {
        return Ok(url);
    }
    Err(
        "Postgres requires --postgres-url, IRONCLAW_FILESYSTEM_POSTGRES_URL, or DATABASE_URL"
            .to_string(),
    )
}

fn optional_env_var(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid Unicode")),
    }
}

fn display_err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
