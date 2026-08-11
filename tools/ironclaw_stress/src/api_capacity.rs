use std::{
    collections::{BTreeMap, HashSet},
    net::SocketAddr,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinSet,
};

use crate::{
    Args, Sample,
    progress::ProgressCounters,
    scripted::{self, ScriptKey, ScriptedDecision, ScriptedMockCounters},
    summary::{FailureCause, LatencySummary, latency_summary, summarize_failure_causes},
};

const DEFAULT_READ_MIX: &str = "list_threads=50,timeline=45,session=5";
const BACKGROUND_FLOW_MARKER: &str = "ironclaw-stress-background";
const MAX_ERROR_BODY_CHARS: usize = 512;
// A 1 MiB scripted document is emitted across bounded tool calls, whose
// arguments and result references coexist in later model requests.
const MAX_MOCK_LLM_REQUEST_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_READ_WORKER_SLEEP: Duration = Duration::from_secs(3600);
const MOCK_COMPLETION_CREATED_AT: u64 = 1_700_000_000;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ApiCapacitySummary {
    pub(crate) base_url: String,
    pub(crate) virtual_users: usize,
    pub(crate) message_interval_ms: u64,
    pub(crate) read_qps_per_user: f64,
    pub(crate) read_workers: usize,
    pub(crate) read_mix: String,
    pub(crate) page_size: u32,
    pub(crate) background_users: usize,
    pub(crate) background_concurrency: usize,
    pub(crate) background_operations_per_user: usize,
    pub(crate) background_start_delay_ms: u64,
    pub(crate) wait_for_assistant: bool,
    pub(crate) terminal_timeout_ms: u64,
    pub(crate) poll_interval_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mock_llm: Option<MockLlmSummary>,
    pub(crate) endpoints: BTreeMap<String, ApiEndpointSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scripted: Option<ApiScriptedSummary>,
}

/// Driver-side summary of scripted tool-call workloads, bucketed by
/// document size.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ApiScriptedSummary {
    pub(crate) script: String,
    pub(crate) doc_sizes: Vec<usize>,
    pub(crate) hot_writers: usize,
    pub(crate) size_buckets: BTreeMap<String, ApiScriptedSizeSummary>,
}

/// Per-document-size scripted workload results.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct ApiScriptedSizeSummary {
    pub(crate) attempted: u64,
    pub(crate) succeeded: u64,
    pub(crate) failed: u64,
    #[serde(default)]
    pub(crate) verdicts: BTreeMap<String, u64>,
    #[serde(default)]
    pub(crate) failure_buckets: BTreeMap<String, u64>,
    /// Tool-result messages observed for this bucket's operations (read-back
    /// correlation evidence).
    #[serde(default)]
    pub(crate) tool_results: u64,
    /// Submit-to-first-tool-visible and submit-to-finalize latencies in
    /// microseconds, per stage.
    #[serde(default)]
    pub(crate) stage_latency_us: BTreeMap<String, LatencySummary>,
}

/// One scripted operation sample, carried from the writer tasks to the
/// summary.
#[derive(Debug, Clone)]
pub(crate) struct ScriptedOpSample {
    pub(crate) size_bytes: usize,
    pub(crate) verdict: Option<scripted::Verdict>,
    /// Tool-result messages observed for this operation, relative to the
    /// previous operation on the same thread.
    pub(crate) tools_executed: usize,
    /// Submit to first tool result visible in the timeline.
    pub(crate) tool_visible_latency: Option<Duration>,
    /// Submit to finalized assistant.
    pub(crate) finalize_latency: Option<Duration>,
    pub(crate) failure: Option<FailureCause>,
}

/// Scripted-mode task identity: the operation id prefix (empty for regular
/// users, `h{k}-` for hot writers) and the user identity part of markers.
#[derive(Debug, Clone)]
struct ScriptedTaskIdentity {
    op_prefix: String,
    marker_user: String,
}

/// Everything a scripted operation needs besides the harness and user.
#[derive(Debug, Clone)]
struct ScriptedFlowContext {
    key: scripted::ScriptKey,
    identity: ScriptedTaskIdentity,
    /// Document size for this operation, in bytes.
    size_bytes: usize,
    expected_tool_results: usize,
    /// Highest message sequence observed in the timeline before this
    /// operation's marker was submitted. Tool results with a sequence above
    /// the baseline belong to this operation, so per-op tool evidence stays
    /// correct even once the thread outgrows one timeline page.
    baseline_sequence: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MockLlmSummary {
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) latency_ms: u64,
    pub(crate) background_latency_ms: u64,
    pub(crate) jitter_ms: u64,
    pub(crate) output_bytes: usize,
    pub(crate) failure_rate: f64,
    pub(crate) requests: u64,
    pub(crate) foreground_requests: u64,
    pub(crate) background_requests: u64,
    pub(crate) max_in_flight: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_latency: Option<LatencySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) foreground_request_latency: Option<LatencySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) background_request_latency: Option<LatencySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_request_offset_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_request_offset_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_start_spread_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scripted: Option<ScriptedMockCounters>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ApiEndpointSummary {
    pub(crate) attempted: u64,
    pub(crate) succeeded: u64,
    pub(crate) failed: u64,
    pub(crate) throughput_ops_sec: f64,
    pub(crate) latency: LatencySummary,
    pub(crate) errors: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) failure_causes: BTreeMap<String, crate::summary::FailureCauseSummary>,
}

#[derive(Debug)]
pub(crate) struct ApiCapacityRun {
    pub(crate) target: String,
    pub(crate) elapsed: Duration,
    pub(crate) samples: Vec<Sample>,
    pub(crate) summary: ApiCapacitySummary,
}

#[derive(Debug, Clone)]
struct ApiUser {
    index: usize,
    label: String,
    bearer_token: Option<String>,
    thread_id: String,
    /// Additional threads of the same user. Scripted hot writers use these
    /// so concurrent operations run on distinct threads (no per-thread turn
    /// serialization) while still contending on the same per-user durable
    /// memory document.
    extra_thread_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiUserInput {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default, alias = "token")]
    bearer_token: Option<String>,
    #[serde(default, alias = "authorization")]
    authorization: Option<String>,
}

#[derive(Debug, Clone)]
struct ApiIdentity {
    label: String,
    bearer_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiAdminCreateUserRequest {
    email: String,
    display_name: String,
    role: &'static str,
}

#[derive(Debug, Deserialize)]
struct ApiAdminCreateUserResponse {
    user: ApiAdminCreatedUserRecord,
    api_token: String,
}

#[derive(Debug, Deserialize)]
struct ApiAdminCreatedUserRecord {
    user_id: String,
}

#[derive(Clone)]
struct ApiHarness {
    client: Client,
    base_url: String,
    page_size: u32,
    request_timeout: Duration,
}

#[derive(Debug, Clone)]
struct ApiRequestSample {
    name: &'static str,
    latency: Duration,
    failure: Option<FailureCause>,
}

#[derive(Debug)]
struct ApiCallResult {
    sample: ApiRequestSample,
    value: Result<Value, FailureCause>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadEndpoint {
    ListThreads,
    Timeline,
    Session,
}

#[derive(Debug, Clone)]
struct ReadMixEntry {
    endpoint: ReadEndpoint,
    cumulative_weight: u32,
}

#[derive(Debug, Clone)]
struct ReadMix {
    spec: String,
    total_weight: u32,
    entries: Vec<ReadMixEntry>,
}

#[derive(Debug)]
struct MockLlmHandle {
    base_url: String,
    state: Arc<MockLlmState>,
    stop_sender: Option<oneshot::Sender<()>>,
}

impl MockLlmHandle {
    fn summary(&self) -> MockLlmSummary {
        self.state.summary(self.base_url.clone())
    }
}

impl Drop for MockLlmHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }
    }
}

#[derive(Debug)]
struct MockLlmConfig {
    bind: SocketAddr,
    model: String,
    latency_ms: u64,
    background_latency_ms: u64,
    jitter_ms: u64,
    output_bytes: usize,
    failure_rate: f64,
    scripted: Option<ScriptKey>,
}

#[derive(Debug)]
struct MockLlmState {
    model: String,
    latency_ms: u64,
    background_latency_ms: u64,
    jitter_ms: u64,
    output_bytes: usize,
    failure_rate: f64,
    scripted: Option<ScriptKey>,
    scripted_counters: Mutex<ScriptedMockCounters>,
    /// Bounded stateful scripted driver: per-operation sessions survive
    /// production context compaction of long sequential plans.
    scripted_driver: Mutex<scripted::ScriptedDriver>,
    counter: AtomicU64,
    foreground_counter: AtomicU64,
    background_counter: AtomicU64,
    in_flight: AtomicU64,
    max_in_flight: AtomicU64,
    started_at: Instant,
    request_latencies: Mutex<Vec<Duration>>,
    foreground_request_latencies: Mutex<Vec<Duration>>,
    background_request_latencies: Mutex<Vec<Duration>>,
    request_start_offsets: Mutex<Vec<Duration>>,
}

impl MockLlmState {
    fn summary(&self, base_url: String) -> MockLlmSummary {
        let request_latencies = self
            .request_latencies
            .lock()
            .map(|latencies| latencies.clone())
            .unwrap_or_default();
        let foreground_request_latencies = self
            .foreground_request_latencies
            .lock()
            .map(|latencies| latencies.clone())
            .unwrap_or_default();
        let background_request_latencies = self
            .background_request_latencies
            .lock()
            .map(|latencies| latencies.clone())
            .unwrap_or_default();
        let request_start_offsets = self
            .request_start_offsets
            .lock()
            .map(|offsets| offsets.clone())
            .unwrap_or_default();
        let first_request_offset = request_start_offsets.iter().min().copied();
        let last_request_offset = request_start_offsets.iter().max().copied();
        let scripted = self
            .scripted_counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        MockLlmSummary {
            base_url,
            model: self.model.clone(),
            latency_ms: self.latency_ms,
            background_latency_ms: self.resolved_background_latency_ms(),
            jitter_ms: self.jitter_ms,
            output_bytes: self.output_bytes,
            failure_rate: self.failure_rate,
            requests: self.counter.load(Ordering::Relaxed),
            foreground_requests: self.foreground_counter.load(Ordering::Relaxed),
            background_requests: self.background_counter.load(Ordering::Relaxed),
            max_in_flight: self.max_in_flight.load(Ordering::Relaxed),
            request_latency: duration_latency_summary(&request_latencies),
            foreground_request_latency: duration_latency_summary(&foreground_request_latencies),
            background_request_latency: duration_latency_summary(&background_request_latencies),
            first_request_offset_ms: first_request_offset.map(|duration| duration.as_millis()),
            last_request_offset_ms: last_request_offset.map(|duration| duration.as_millis()),
            request_start_spread_ms: first_request_offset
                .zip(last_request_offset)
                .map(|(first, last)| last.saturating_sub(first).as_millis()),
            scripted: self.scripted.map(|_| scripted),
        }
    }

    fn resolved_background_latency_ms(&self) -> u64 {
        if self.background_latency_ms == 0 {
            self.latency_ms
        } else {
            self.background_latency_ms
        }
    }

    fn record_request_start(&self, offset: Duration) {
        if let Ok(mut offsets) = self.request_start_offsets.lock() {
            offsets.push(offset);
        }
    }

    fn record_request_latency(&self, kind: MockRequestKind, latency: Duration) {
        if let Ok(mut latencies) = self.request_latencies.lock() {
            latencies.push(latency);
        }
        match kind {
            MockRequestKind::Foreground => {
                if let Ok(mut latencies) = self.foreground_request_latencies.lock() {
                    latencies.push(latency);
                }
            }
            MockRequestKind::Background => {
                if let Ok(mut latencies) = self.background_request_latencies.lock() {
                    latencies.push(latency);
                }
            }
        }
    }

    fn enter_request(&self) {
        let in_flight = self.in_flight.fetch_add(1, Ordering::Relaxed) + 1;
        raise_atomic_max(&self.max_in_flight, in_flight);
    }

    fn exit_request(&self) {
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

fn duration_latency_summary(latencies: &[Duration]) -> Option<LatencySummary> {
    if latencies.is_empty() {
        return None;
    }
    let mut latency_us = latencies
        .iter()
        .map(Duration::as_micros)
        .collect::<Vec<_>>();
    latency_us.sort_unstable();
    Some(latency_summary(&latency_us))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MockRequestKind {
    Foreground,
    Background,
}

struct MockRequestGuard {
    state: Arc<MockLlmState>,
    kind: MockRequestKind,
    started_at: Instant,
}

impl MockRequestGuard {
    fn new(state: &Arc<MockLlmState>, kind: MockRequestKind) -> Self {
        state.record_request_start(state.started_at.elapsed());
        state.enter_request();
        Self {
            state: Arc::clone(state),
            kind,
            started_at: Instant::now(),
        }
    }
}

impl Drop for MockRequestGuard {
    fn drop(&mut self) {
        self.state
            .record_request_latency(self.kind, self.started_at.elapsed());
        self.state.exit_request();
    }
}

fn raise_atomic_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

pub(crate) fn default_read_mix() -> String {
    DEFAULT_READ_MIX.to_string()
}

pub(crate) fn validate_read_mix(spec: &str) -> Result<(), String> {
    ReadMix::parse(spec).map(|_| ())
}

pub(crate) async fn run(
    args: &Args,
    run_id: &str,
    progress: Arc<ProgressCounters>,
) -> Result<ApiCapacityRun, String> {
    let _mock_llm = start_mock_llm(args).await?;
    let harness = ApiHarness::new(args)?;
    let identity_count = args.users.saturating_add(args.api_background_users).max(1);
    let identities = load_identities(args, identity_count).await?;
    let identities = provision_missing_identities(args, run_id, &harness, identities).await?;
    let foreground_identities = select_identities(&identities, 0, args.users);
    let background_identities =
        select_identities(&identities, args.users, args.api_background_users);
    let users = setup_users(
        args,
        run_id,
        &harness,
        foreground_identities,
        args.users,
        "fg",
    )
    .await?;
    let background_users = if args.api_background_users == 0 {
        Vec::new()
    } else {
        setup_users(
            args,
            run_id,
            &harness,
            background_identities,
            args.api_background_users,
            "background",
        )
        .await?
    };
    let read_mix = ReadMix::parse(&args.api_read_mix)?;
    let read_workers = read_worker_count(args);
    let background_concurrency = background_concurrency_limit(args, background_users.len());

    if let Some(warmup_args) = args.warmup_args() {
        eprintln!(
            "{} warming up target={} duration_seconds={} users={}",
            crate::log_prefix(args),
            harness.base_url,
            warmup_args.duration_seconds,
            users.len()
        );
        let warmup_progress = Arc::new(ProgressCounters::new(false));
        let warmup_namespace = format!("{run_id}:warmup");
        let _ = run_window(
            &warmup_args,
            &warmup_namespace,
            &harness,
            Arc::new(users.clone()),
            Arc::new(background_users.clone()),
            &read_mix,
            warmup_progress,
        )
        .await?;
    }

    let started = Instant::now();
    let measured_namespace = format!("{run_id}:measured");
    let window = run_window(
        args,
        &measured_namespace,
        &harness,
        Arc::new(users),
        Arc::new(background_users),
        &read_mix,
        progress,
    )
    .await?;
    let elapsed = started.elapsed();
    let mock_summary = _mock_llm.as_ref().map(MockLlmHandle::summary);
    let mut endpoints = summarize_api_samples(&window.api_samples, elapsed);
    endpoints.insert(
        "full_flow".to_string(),
        summarize_flow_samples(&window.flow_samples, elapsed),
    );
    if !window.background_flow_samples.is_empty() {
        endpoints.insert(
            "background_full_flow".to_string(),
            summarize_flow_samples(&window.background_flow_samples, elapsed),
        );
    }
    let summary = ApiCapacitySummary {
        base_url: harness.base_url.clone(),
        virtual_users: args.users,
        message_interval_ms: args.api_message_interval_ms,
        read_qps_per_user: args.api_read_qps_per_user,
        read_workers,
        read_mix: read_mix.spec,
        page_size: args.api_page_size,
        background_users: args.api_background_users,
        background_concurrency,
        background_operations_per_user: args.api_background_operations,
        background_start_delay_ms: args.api_background_start_delay_ms,
        wait_for_assistant: args.api_wait_for_assistant,
        terminal_timeout_ms: args.api_terminal_timeout_ms,
        poll_interval_ms: args.api_poll_interval_ms,
        mock_llm: mock_summary,
        endpoints,
        scripted: summarize_scripted(&window.scripted_samples, args),
    };

    Ok(ApiCapacityRun {
        target: harness.base_url.clone(),
        elapsed,
        samples: window.flow_samples,
        summary,
    })
}

/// Aggregate scripted operation samples into per-document-size buckets.
fn summarize_scripted(samples: &[ScriptedOpSample], args: &Args) -> Option<ApiScriptedSummary> {
    let script = args.api_scripted_tool?;
    if samples.is_empty() {
        return None;
    }
    let mut size_buckets: BTreeMap<String, ApiScriptedSizeSummary> = BTreeMap::new();
    let mut stage_latencies: BTreeMap<String, BTreeMap<String, Vec<u128>>> = BTreeMap::new();
    for sample in samples {
        let size_key = sample.size_bytes.to_string();
        let bucket = size_buckets.entry(size_key.clone()).or_default();
        bucket.attempted += 1;
        bucket.tool_results += sample.tools_executed as u64;
        match &sample.failure {
            Some(failure) => {
                bucket.failed += 1;
                *bucket
                    .failure_buckets
                    .entry(failure.bucket.clone())
                    .or_insert(0) += 1;
            }
            None => {
                bucket.succeeded += 1;
                if let Some(verdict) = sample.verdict {
                    *bucket
                        .verdicts
                        .entry(verdict.as_str().to_string())
                        .or_insert(0) += 1;
                }
            }
        }
        let stages = stage_latencies.entry(size_key).or_default();
        if let Some(latency) = sample.tool_visible_latency {
            stages
                .entry("tool_visible".to_string())
                .or_default()
                .push(latency.as_micros());
        }
        if let Some(latency) = sample.finalize_latency {
            stages
                .entry("finalize".to_string())
                .or_default()
                .push(latency.as_micros());
        }
    }
    for (size_key, stages) in stage_latencies {
        let bucket = size_buckets.get_mut(&size_key);
        let Some(bucket) = bucket else {
            continue;
        };
        for (stage, mut values) in stages {
            // `latency_summary` reads percentiles and min/max by index, so
            // values must be sorted before aggregation.
            values.sort_unstable();
            bucket
                .stage_latency_us
                .insert(stage, latency_summary(&values));
        }
    }
    Some(ApiScriptedSummary {
        script: script.as_str().to_string(),
        doc_sizes: args.api_scripted_doc_sizes.clone(),
        hot_writers: args.api_hot_writers,
        size_buckets,
    })
}

struct WindowResult {
    flow_samples: Vec<Sample>,
    background_flow_samples: Vec<Sample>,
    api_samples: Vec<ApiRequestSample>,
    scripted_samples: Vec<ScriptedOpSample>,
}

struct ReadShutdown {
    stopped: AtomicBool,
    notify: tokio::sync::Notify,
}

impl ReadShutdown {
    fn new() -> Self {
        Self {
            stopped: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    async fn notified(&self) {
        self.notify.notified().await;
    }
}

async fn run_window(
    args: &Args,
    operation_namespace: &str,
    harness: &ApiHarness,
    users: Arc<Vec<ApiUser>>,
    background_users: Arc<Vec<ApiUser>>,
    read_mix: &ReadMix,
    progress: Arc<ProgressCounters>,
) -> Result<WindowResult, String> {
    let mut background_tasks = JoinSet::new();
    let mut writer_tasks = JoinSet::new();
    let mut hot_writer_tasks = JoinSet::new();
    let mut read_tasks = JoinSet::new();
    let stop_reads = Arc::new(ReadShutdown::new());

    let background_limit = background_concurrency_limit(args, background_users.len());
    let mut next_background_index = 0usize;
    while next_background_index < background_users.len()
        && background_tasks.len() < background_limit
    {
        let user = background_users[next_background_index].clone();
        next_background_index += 1;
        let harness = harness.clone();
        let args = args.clone();
        let operation_namespace = operation_namespace.to_string();
        background_tasks.spawn(async move {
            run_background_user(&args, &operation_namespace, harness, user).await
        });
    }
    if !background_tasks.is_empty() && args.api_background_start_delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(args.api_background_start_delay_ms)).await;
    }

    let writer_limit = writer_concurrency_limit(args, users.len());
    let writer_user_count = writer_user_count_for_window(args, users.len(), writer_limit);
    let mut next_writer_index = 0usize;
    while next_writer_index < writer_user_count && writer_tasks.len() < writer_limit {
        let user = users[next_writer_index].clone();
        next_writer_index += 1;
        let harness = harness.clone();
        let progress = Arc::clone(&progress);
        let args = args.clone();
        let operation_namespace = operation_namespace.to_string();
        writer_tasks.spawn(async move {
            run_virtual_user(&args, &operation_namespace, harness, user, progress).await
        });
    }

    // Scripted hot writers: extra concurrent writers on distinct threads of
    // the first user so several operations contend on the same per-user
    // durable memory document at once (CAS contention on one document).
    // They drain in their own JoinSet after the regular writer loop so a hot
    // writer finishing never triggers a `run_virtual_user` refill — hot
    // writers are deliberately extra concurrency beyond `--concurrency`,
    // not part of the writer accounting.
    if args.api_scripted_tool.is_some()
        && args.api_hot_writers > 0
        && let Some(hot_user) = users.first().cloned()
    {
        for hot_index in 0..args.api_hot_writers {
            let harness = harness.clone();
            let progress = Arc::clone(&progress);
            let args = args.clone();
            let operation_namespace = operation_namespace.to_string();
            let user = hot_user.clone();
            hot_writer_tasks.spawn(async move {
                run_hot_writer(
                    &args,
                    &operation_namespace,
                    harness,
                    user,
                    progress,
                    hot_index,
                )
                .await
            });
        }
    }

    if args.api_read_qps_per_user > 0.0 {
        let read_workers = read_worker_count(args);
        for worker_index in 0..read_workers {
            let harness = harness.clone();
            let users = Arc::clone(&users);
            let read_mix = read_mix.clone();
            let stop_reads = Arc::clone(&stop_reads);
            let args = args.clone();
            read_tasks.spawn(async move {
                run_read_worker(&args, harness, users, read_mix, worker_index, stop_reads).await
            });
        }
    }

    let mut flow_samples = Vec::new();
    let mut background_flow_samples = Vec::new();
    let mut api_samples = Vec::new();
    let mut scripted_samples = Vec::new();
    while let Some(joined) = writer_tasks.join_next().await {
        let result = joined.map_err(|error| format!("api task join failed: {error}"))??;
        flow_samples.extend(result.flow_samples);
        background_flow_samples.extend(result.background_flow_samples);
        api_samples.extend(result.api_samples);
        scripted_samples.extend(result.scripted_samples);

        if next_writer_index < writer_user_count {
            let user = users[next_writer_index].clone();
            next_writer_index += 1;
            let harness = harness.clone();
            let progress = Arc::clone(&progress);
            let args = args.clone();
            let operation_namespace = operation_namespace.to_string();
            writer_tasks.spawn(async move {
                run_virtual_user(&args, &operation_namespace, harness, user, progress).await
            });
        }
    }

    while let Some(joined) = hot_writer_tasks.join_next().await {
        let result =
            joined.map_err(|error| format!("api hot-writer task join failed: {error}"))??;
        flow_samples.extend(result.flow_samples);
        background_flow_samples.extend(result.background_flow_samples);
        api_samples.extend(result.api_samples);
        scripted_samples.extend(result.scripted_samples);
    }

    stop_reads.stop();
    while let Some(joined) = read_tasks.join_next().await {
        let result = joined.map_err(|error| format!("api read task join failed: {error}"))??;
        flow_samples.extend(result.flow_samples);
        background_flow_samples.extend(result.background_flow_samples);
        api_samples.extend(result.api_samples);
    }

    while let Some(joined) = background_tasks.join_next().await {
        let result =
            joined.map_err(|error| format!("api background task join failed: {error}"))??;
        background_flow_samples.extend(result.background_flow_samples);
        api_samples.extend(result.api_samples);
        scripted_samples.extend(result.scripted_samples);

        if next_background_index < background_users.len() {
            let user = background_users[next_background_index].clone();
            next_background_index += 1;
            let harness = harness.clone();
            let args = args.clone();
            let operation_namespace = operation_namespace.to_string();
            background_tasks.spawn(async move {
                run_background_user(&args, &operation_namespace, harness, user).await
            });
        }
    }

    Ok(WindowResult {
        flow_samples,
        background_flow_samples,
        api_samples,
        scripted_samples,
    })
}

fn writer_concurrency_limit(args: &Args, user_count: usize) -> usize {
    args.concurrency.max(1).min(user_count.max(1))
}

fn writer_user_count_for_window(args: &Args, user_count: usize, writer_limit: usize) -> usize {
    if args.uses_duration_mode() {
        writer_limit
    } else {
        user_count
    }
}

fn background_concurrency_limit(args: &Args, background_user_count: usize) -> usize {
    if background_user_count == 0 {
        0
    } else if args.api_background_concurrency == 0 {
        background_user_count.min(8)
    } else {
        args.api_background_concurrency
            .max(1)
            .min(background_user_count)
    }
}

struct TaskResult {
    flow_samples: Vec<Sample>,
    background_flow_samples: Vec<Sample>,
    api_samples: Vec<ApiRequestSample>,
    scripted_samples: Vec<ScriptedOpSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiFlowKind {
    Foreground,
    Background,
}

impl ApiFlowKind {
    fn send_sample_name(self) -> &'static str {
        match self {
            Self::Foreground => "send_message",
            Self::Background => "background_send_message",
        }
    }

    fn timeline_sample_name(self) -> &'static str {
        match self {
            Self::Foreground => "timeline",
            Self::Background => "background_timeline",
        }
    }

    fn wait_sample_name(self) -> &'static str {
        match self {
            Self::Foreground => "wait_for_assistant",
            Self::Background => "background_wait_for_assistant",
        }
    }

    fn payload_prefix(self, operation_ref: &str) -> String {
        match self {
            Self::Foreground => format!("api stress message {operation_ref}"),
            Self::Background => {
                format!("{BACKGROUND_FLOW_MARKER} api stress background message {operation_ref}")
            }
        }
    }
}

async fn run_virtual_user(
    args: &Args,
    operation_namespace: &str,
    harness: ApiHarness,
    user: ApiUser,
    progress: Arc<ProgressCounters>,
) -> Result<TaskResult, String> {
    run_virtual_user_with_identity(args, operation_namespace, harness, user, progress, None).await
}

/// A hot writer task: a scripted writer on its own thread of the first user
/// so several operations overlap (no per-thread turn serialization) while
/// contending on the same per-user durable memory document.
async fn run_hot_writer(
    args: &Args,
    operation_namespace: &str,
    harness: ApiHarness,
    user: ApiUser,
    progress: Arc<ProgressCounters>,
    hot_index: usize,
) -> Result<TaskResult, String> {
    let thread_id = user
        .extra_thread_ids
        .get(hot_index)
        .cloned()
        .unwrap_or_else(|| user.thread_id.clone());
    let user = ApiUser { thread_id, ..user };
    let identity = ScriptedTaskIdentity {
        op_prefix: format!("h{hot_index}-"),
        marker_user: format!("u{}", user.index),
    };
    run_virtual_user_with_identity(
        args,
        operation_namespace,
        harness,
        user,
        progress,
        Some(identity),
    )
    .await
}

async fn run_virtual_user_with_identity(
    args: &Args,
    operation_namespace: &str,
    harness: ApiHarness,
    user: ApiUser,
    progress: Arc<ProgressCounters>,
    scripted_identity: Option<ScriptedTaskIdentity>,
) -> Result<TaskResult, String> {
    let script_key = args.api_scripted_tool;
    let identity = scripted_identity.or_else(|| {
        script_key.map(|_| ScriptedTaskIdentity {
            op_prefix: String::new(),
            marker_user: format!("u{}", user.index),
        })
    });
    let target = args.operation_target();
    let started = Instant::now();
    let mut operation_index = 0;
    let mut expected_finalized_assistant_count = 0usize;
    let mut max_sequence_seen = 0u64;
    let mut flow_samples = Vec::with_capacity(args.initial_worker_sample_capacity());
    let mut api_samples = Vec::new();
    let mut scripted_samples = Vec::new();

    while crate::should_run_operation(target, started, operation_index) {
        // The op prefix (hot writers: `h{k}-`) is part of the client action
        // id so concurrent writers never collide on a duplicate-action
        // conflict even when they share the same user.
        let op_prefix = identity
            .as_ref()
            .map(|identity| identity.op_prefix.as_str())
            .unwrap_or("");
        let operation_ref = format!(
            "{operation_namespace}:{}:{}:{op_prefix}{}",
            user.label, user.index, operation_index
        );
        let (flow, mut operation_api_samples, updated_finalized_assistant_count, max_sequence) =
            match (script_key, identity.as_ref()) {
                (Some(key), Some(identity)) => {
                    let size_bytes =
                        scripted::doc_size_for(&args.api_scripted_doc_sizes, operation_index);
                    let flow_context = ScriptedFlowContext {
                        key,
                        identity: identity.clone(),
                        size_bytes,
                        expected_tool_results: key.expected_tool_results(size_bytes),
                        baseline_sequence: max_sequence_seen,
                    };
                    let (flow, api, finalized, sequence, op_sample) = run_scripted_full_flow(
                        args,
                        &harness,
                        &user,
                        operation_index,
                        &operation_ref,
                        ApiFlowKind::Foreground,
                        &flow_context,
                    )
                    .await;
                    scripted_samples.push(op_sample);
                    (flow, api, finalized, sequence)
                }
                _ => {
                    let (flow, api, finalized) = run_full_flow(
                        args,
                        &harness,
                        &user,
                        operation_index,
                        &operation_ref,
                        expected_finalized_assistant_count,
                        ApiFlowKind::Foreground,
                    )
                    .await;
                    (flow, api, finalized, max_sequence_seen)
                }
            };
        expected_finalized_assistant_count = updated_finalized_assistant_count;
        max_sequence_seen = max_sequence;
        progress.record(flow.error.is_some(), flow.latency);
        api_samples.append(&mut operation_api_samples);
        flow_samples.push(flow);
        operation_index += 1;
        if args.api_message_interval_ms > 0
            && crate::should_run_operation(target, started, operation_index)
        {
            tokio::time::sleep(Duration::from_millis(args.api_message_interval_ms)).await;
        }
    }

    Ok(TaskResult {
        flow_samples,
        background_flow_samples: Vec::new(),
        api_samples,
        scripted_samples,
    })
}

async fn run_background_user(
    args: &Args,
    operation_namespace: &str,
    harness: ApiHarness,
    user: ApiUser,
) -> Result<TaskResult, String> {
    let script_key = args.api_scripted_tool;
    // Background users are namespaced `b{index}` so their marker identities
    // cannot collide with foreground user `u{index}` operations: a real
    // cross-cohort isolation violation would otherwise read back as its own
    // token and be reported `confirmed`.
    let identity = script_key.map(|_| ScriptedTaskIdentity {
        op_prefix: String::new(),
        marker_user: format!("b{}", user.index),
    });
    let mut expected_finalized_assistant_count = 0usize;
    let mut max_sequence_seen = 0u64;
    let mut background_flow_samples = Vec::with_capacity(args.api_background_operations);
    let mut api_samples = Vec::new();
    let mut scripted_samples = Vec::new();

    for operation_index in 0..args.api_background_operations {
        let op_prefix = identity
            .as_ref()
            .map(|identity| identity.op_prefix.as_str())
            .unwrap_or("");
        let operation_ref = format!(
            "{operation_namespace}:background:{}:{}:{op_prefix}{}",
            user.label, user.index, operation_index
        );
        let (flow, mut operation_api_samples, updated_finalized_assistant_count, max_sequence) =
            match (script_key, identity.as_ref()) {
                (Some(key), Some(identity)) => {
                    let size_bytes =
                        scripted::doc_size_for(&args.api_scripted_doc_sizes, operation_index);
                    let flow_context = ScriptedFlowContext {
                        key,
                        identity: identity.clone(),
                        size_bytes,
                        expected_tool_results: key.expected_tool_results(size_bytes),
                        baseline_sequence: max_sequence_seen,
                    };
                    let (flow, api, finalized, sequence, op_sample) = run_scripted_full_flow(
                        args,
                        &harness,
                        &user,
                        operation_index,
                        &operation_ref,
                        ApiFlowKind::Background,
                        &flow_context,
                    )
                    .await;
                    scripted_samples.push(op_sample);
                    (flow, api, finalized, sequence)
                }
                _ => {
                    let (flow, api, finalized) = run_full_flow(
                        args,
                        &harness,
                        &user,
                        operation_index,
                        &operation_ref,
                        expected_finalized_assistant_count,
                        ApiFlowKind::Background,
                    )
                    .await;
                    (flow, api, finalized, max_sequence_seen)
                }
            };
        expected_finalized_assistant_count = updated_finalized_assistant_count;
        max_sequence_seen = max_sequence;
        api_samples.append(&mut operation_api_samples);
        background_flow_samples.push(flow);
    }

    Ok(TaskResult {
        flow_samples: Vec::new(),
        background_flow_samples,
        api_samples,
        scripted_samples,
    })
}

async fn run_full_flow(
    args: &Args,
    harness: &ApiHarness,
    user: &ApiUser,
    operation_index: usize,
    operation_ref: &str,
    expected_finalized_assistant_count: usize,
    flow_kind: ApiFlowKind,
) -> (Sample, Vec<ApiRequestSample>, usize) {
    let started = Instant::now();
    let mut api_samples = Vec::new();
    let body = json!({
        "client_action_id": format!("ironclaw-stress-api:{operation_ref}"),
        "content": stress_payload(
            flow_kind.payload_prefix(operation_ref),
            args.user_message_bytes,
        ),
    });
    let send = harness
        .request_json(
            user,
            flow_kind.send_sample_name(),
            Method::POST,
            &format!("/api/webchat/v2/threads/{}/messages", user.thread_id),
            Some(body),
        )
        .await;
    let send_value = send.value.clone();
    api_samples.push(send.sample);
    let failure = match send_value {
        Ok(value) => {
            let target_finalized_count = expected_finalized_assistant_count + 1;
            if args.api_wait_for_assistant {
                let wait_started = Instant::now();
                let wait = wait_for_assistant(
                    args,
                    harness,
                    user,
                    operation_index,
                    target_finalized_count,
                    flow_kind,
                    &mut api_samples,
                )
                .await;
                api_samples.push(ApiRequestSample {
                    name: flow_kind.wait_sample_name(),
                    latency: wait_started.elapsed(),
                    failure: wait.as_ref().err().cloned(),
                });
                match wait {
                    Ok(finalized_count) => {
                        let latency = started.elapsed();
                        return (
                            Sample {
                                latency,
                                error: None,
                                failure: None,
                                stages: None,
                            },
                            api_samples,
                            finalized_count.max(target_finalized_count),
                        );
                    }
                    Err(failure) => {
                        let latency = started.elapsed();
                        let error = Some(failure.bucket.clone());
                        return (
                            Sample {
                                latency,
                                error,
                                failure: Some(failure),
                                stages: None,
                            },
                            api_samples,
                            target_finalized_count,
                        );
                    }
                }
            } else if submitted_or_already_submitted(&value) {
                let latency = started.elapsed();
                return (
                    Sample {
                        latency,
                        error: None,
                        failure: None,
                        stages: None,
                    },
                    api_samples,
                    target_finalized_count,
                );
            } else {
                Some(FailureCause::new(
                    "api_submit_not_accepted",
                    flow_kind.send_sample_name(),
                    compact_json(&value),
                ))
            }
        }
        Err(failure) => Some(failure),
    };

    let latency = started.elapsed();
    let error = failure.as_ref().map(|cause| cause.bucket.clone());
    (
        Sample {
            latency,
            error,
            failure,
            stages: None,
        },
        api_samples,
        expected_finalized_assistant_count,
    )
}

/// Drive one scripted operation: submit the marker message, then poll the
/// timeline until the operation's verdict text is visible, recording the
/// submit-to-first-tool and submit-to-finalize stages and the read-back
/// verdict.
///
/// Completion is detected by verdict-text presence rather than assistant
/// counts because hot writers share one thread: several operations race on
/// the same timeline and counts cannot be attributed to a single op.
async fn run_scripted_full_flow(
    args: &Args,
    harness: &ApiHarness,
    user: &ApiUser,
    operation_index: usize,
    operation_ref: &str,
    flow_kind: ApiFlowKind,
    flow: &ScriptedFlowContext,
) -> (Sample, Vec<ApiRequestSample>, usize, u64, ScriptedOpSample) {
    let started = Instant::now();
    let mut api_samples = Vec::new();
    let key = flow.key;
    let op = scripted::ScriptedOp {
        key,
        user: flow.identity.marker_user.clone(),
        op: format!("{}{}", flow.identity.op_prefix, operation_index),
        size_bytes: flow.size_bytes,
    };
    let marker = scripted::marker_message(key, &op.user, &op.op, op.size_bytes);
    let body = json!({
        "client_action_id": format!("ironclaw-stress-api:{operation_ref}"),
        "content": stress_payload(marker, args.user_message_bytes),
    });
    let send = harness
        .request_json(
            user,
            flow_kind.send_sample_name(),
            Method::POST,
            &format!("/api/webchat/v2/threads/{}/messages", user.thread_id),
            Some(body),
        )
        .await;
    let send_value = send.value.clone();
    api_samples.push(send.sample);

    // Outcome of the submit step: verdict, failure, and the observed tool
    // count at completion.
    let mut verdict: Option<scripted::Verdict> = None;
    let mut failure: Option<FailureCause> = None;
    let mut tools_seen = 0usize;
    let mut latest_sequence = flow.baseline_sequence;
    let mut tool_visible_latency: Option<Duration> = None;
    let mut finalize_latency: Option<Duration> = None;

    match send_value {
        Ok(value) if submitted_or_already_submitted(&value) => {
            let verdict_prefix = format!("{} {}", scripted::RESULT_PREFIX, op.identity());
            let deadline = Instant::now() + Duration::from_millis(args.api_terminal_timeout_ms);
            let mut placeholder_seen = false;
            let mut my_final_content: Option<String> = None;
            loop {
                if Instant::now() >= deadline {
                    break;
                }
                let timeline = harness
                    .timeline_with_name(user, flow_kind.timeline_sample_name())
                    .await;
                let value = timeline.value.clone();
                api_samples.push(timeline.sample);
                match value {
                    Ok(value) => {
                        // Count only tool results that landed after this
                        // operation's baseline: subtracting page-limited
                        // absolute counts would under-count once the thread
                        // outgrows a single timeline page.
                        tools_seen = timeline_tool_results_after(&value, flow.baseline_sequence);
                        latest_sequence = timeline_max_sequence(&value).max(latest_sequence);
                        if tool_visible_latency.is_none() && tools_seen >= 1 {
                            tool_visible_latency = Some(started.elapsed());
                        }
                        placeholder_seen |= timeline_has_placeholder(&value);
                        if let Some(content) =
                            timeline_finalized_verdict_content(&value, &verdict_prefix)
                        {
                            finalize_latency = Some(started.elapsed());
                            my_final_content = Some(content);
                            break;
                        }
                    }
                    Err(timeline_failure) => {
                        failure = Some(timeline_failure);
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(args.api_poll_interval_ms)).await;
            }

            let final_verdict = my_final_content
                .as_ref()
                .and_then(|content| scripted::parse_result_verdict(content, &op));
            verdict = final_verdict;
            failure = failure.or_else(|| match &my_final_content {
                Some(_) => match final_verdict {
                    Some(parsed) if !parsed.is_failure() => None,
                    Some(parsed) => Some(FailureCause::new(
                        format!("scripted_verdict_{}", parsed.as_str()),
                        flow_kind.wait_sample_name(),
                        format!(
                            "operation {} finalized with failing verdict {}",
                            op.identity(),
                            parsed.as_str()
                        ),
                    )),
                    None => Some(FailureCause::new(
                        "scripted_no_verdict",
                        flow_kind.wait_sample_name(),
                        format!(
                            "operation {} finalized without a verdict text",
                            op.identity()
                        ),
                    )),
                },
                None => Some(FailureCause::new(
                    if placeholder_seen {
                        "scripted_tool_undisclosed"
                    } else {
                        "api_full_flow_timeout"
                    },
                    flow_kind.wait_sample_name(),
                    format!(
                        "operation {} verdict not visible after {}ms",
                        op.identity(),
                        args.api_terminal_timeout_ms
                    ),
                )),
            });
            if failure.is_none() && tools_seen < flow.expected_tool_results {
                failure = Some(FailureCause::new(
                    "scripted_tools_not_executed",
                    flow_kind.wait_sample_name(),
                    format!(
                        "operation {} finalized with {tools_seen}/{} tool results",
                        op.identity(),
                        flow.expected_tool_results
                    ),
                ));
            }
        }
        Ok(value) => {
            failure = Some(FailureCause::new(
                "api_submit_not_accepted",
                flow_kind.send_sample_name(),
                compact_json(&value),
            ));
        }
        Err(send_failure) => {
            failure = Some(send_failure);
        }
    }

    let latency = started.elapsed();
    let error = failure.as_ref().map(|cause| cause.bucket.clone());
    (
        Sample {
            latency,
            error,
            failure: failure.clone(),
            stages: None,
        },
        api_samples,
        operation_index + 1,
        latest_sequence,
        ScriptedOpSample {
            size_bytes: op.size_bytes,
            verdict,
            tools_executed: tools_seen,
            tool_visible_latency,
            finalize_latency,
            failure,
        },
    )
}

fn timeline_tool_results_after(value: &Value, baseline_sequence: u64) -> usize {
    value
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| {
            message.get("kind").and_then(Value::as_str) == Some("tool_result_reference")
                && message
                    .get("sequence")
                    .and_then(Value::as_u64)
                    .is_some_and(|sequence| sequence > baseline_sequence)
        })
        .count()
}

fn timeline_max_sequence(value: &Value) -> u64 {
    value
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get("sequence").and_then(Value::as_u64))
        .max()
        .unwrap_or(0)
}

fn timeline_has_placeholder(value: &Value) -> bool {
    value
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| {
            message.get("kind").and_then(Value::as_str) == Some("assistant")
                && message.get("status").and_then(Value::as_str) == Some("finalized")
        })
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .any(|content| content.contains(scripted::PLACEHOLDER_TEXT))
}

fn timeline_finalized_verdict_content(value: &Value, verdict_prefix: &str) -> Option<String> {
    // Match the prefix followed by a space so operation `1` cannot
    // terminate on operation `10`'s finalized message (`u0__1 ` is not a
    // substring of `...u0__10 confirmed`).
    let delimited = format!("{verdict_prefix} ");
    value
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| {
            message.get("kind").and_then(Value::as_str) == Some("assistant")
                && message.get("status").and_then(Value::as_str) == Some("finalized")
        })
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .find(|content| content.contains(&delimited))
        .map(str::to_string)
}

async fn wait_for_assistant(
    args: &Args,
    harness: &ApiHarness,
    user: &ApiUser,
    operation_index: usize,
    target_finalized_count: usize,
    flow_kind: ApiFlowKind,
    api_samples: &mut Vec<ApiRequestSample>,
) -> Result<usize, FailureCause> {
    let deadline = Instant::now() + Duration::from_millis(args.api_terminal_timeout_ms);
    loop {
        let timeline = harness
            .timeline_with_name(user, flow_kind.timeline_sample_name())
            .await;
        let value = timeline.value.clone();
        api_samples.push(timeline.sample);
        match value {
            Ok(value) => {
                let finalized_count = timeline_finalized_assistant_count(&value);
                if finalized_count >= target_finalized_count {
                    return Ok(finalized_count);
                }
            }
            Err(failure) => return Err(failure),
        }
        if Instant::now() >= deadline {
            return Err(FailureCause::new(
                "api_full_flow_timeout",
                "timeline",
                format!(
                    "assistant message not visible after {}ms for user={} op={}",
                    args.api_terminal_timeout_ms, user.label, operation_index
                ),
            ));
        }
        tokio::time::sleep(Duration::from_millis(args.api_poll_interval_ms)).await;
    }
}

async fn run_read_worker(
    args: &Args,
    harness: ApiHarness,
    users: Arc<Vec<ApiUser>>,
    read_mix: ReadMix,
    worker_index: usize,
    stop_reads: Arc<ReadShutdown>,
) -> Result<TaskResult, String> {
    let sleep = read_worker_interval(args);
    let mut interval = tokio::time::interval(sleep);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    let mut api_samples = Vec::new();
    let mut operation_index = 0_u64;
    loop {
        if stop_reads.is_stopped() {
            break;
        }
        tokio::select! {
            _ = stop_reads.notified() => break,
            _ = interval.tick() => {}
        }
        if stop_reads.is_stopped() {
            break;
        }
        let user = &users[(worker_index + operation_index as usize) % users.len()];
        let endpoint = read_mix.endpoint_for(operation_index);
        let call = match endpoint {
            ReadEndpoint::ListThreads => harness.list_threads(user).await,
            ReadEndpoint::Timeline => harness.timeline(user).await,
            ReadEndpoint::Session => harness.session(user).await,
        };
        api_samples.push(call.sample);
        operation_index += 1;
        if stop_reads.is_stopped() {
            break;
        }
    }

    Ok(TaskResult {
        flow_samples: Vec::new(),
        background_flow_samples: Vec::new(),
        api_samples,
        scripted_samples: Vec::new(),
    })
}

impl ApiHarness {
    fn new(args: &Args) -> Result<Self, String> {
        let base_url = args
            .api_base_url
            .as_ref()
            .ok_or_else(|| {
                "--api-base-url is required for --scenario api-user-capacity".to_string()
            })?
            .trim_end_matches('/')
            .to_string();
        let request_timeout = Duration::from_millis(args.api_request_timeout_ms);
        let client = Client::builder()
            .timeout(request_timeout)
            .build()
            .map_err(|error| format!("build api HTTP client: {error}"))?;
        Ok(Self {
            client,
            base_url,
            page_size: args.api_page_size,
            request_timeout,
        })
    }

    async fn create_thread(
        &self,
        identity: &ApiIdentity,
        requested_thread_id: &str,
    ) -> ApiCallResult {
        let user = ApiUser {
            index: 0,
            label: identity.label.clone(),
            bearer_token: identity.bearer_token.clone(),
            thread_id: requested_thread_id.to_string(),
            extra_thread_ids: Vec::new(),
        };
        self.request_json(
            &user,
            "create_thread",
            Method::POST,
            "/api/webchat/v2/threads",
            Some(json!({
                "client_action_id": format!("ironclaw-stress-api:create:{requested_thread_id}"),
                "requested_thread_id": requested_thread_id,
            })),
        )
        .await
    }

    async fn create_admin_user(
        &self,
        admin_bearer_token: &str,
        email: String,
        display_name: String,
        role: &'static str,
    ) -> ApiCallResult {
        let request = ApiAdminCreateUserRequest {
            email,
            display_name,
            role,
        };
        let body = match serde_json::to_value(request) {
            Ok(value) => value,
            Err(error) => {
                let detail = error.to_string();
                let failure =
                    FailureCause::new("api_json_encode", "admin_create_user", detail.clone());
                return ApiCallResult {
                    sample: ApiRequestSample {
                        name: "admin_create_user",
                        latency: Duration::ZERO,
                        failure: Some(failure.clone()),
                    },
                    value: Err(failure),
                };
            }
        };
        self.request_json_with_bearer(
            Some(admin_bearer_token),
            "admin_create_user",
            Method::POST,
            "/api/webchat/v2/admin/users",
            Some(body),
        )
        .await
    }

    async fn list_threads(&self, user: &ApiUser) -> ApiCallResult {
        self.request_json(
            user,
            "list_threads",
            Method::GET,
            &format!("/api/webchat/v2/threads?limit={}", self.page_size),
            None,
        )
        .await
    }

    async fn timeline(&self, user: &ApiUser) -> ApiCallResult {
        self.timeline_with_name(user, "timeline").await
    }

    async fn timeline_with_name(&self, user: &ApiUser, name: &'static str) -> ApiCallResult {
        self.request_json(
            user,
            name,
            Method::GET,
            &format!(
                "/api/webchat/v2/threads/{}/timeline?limit={}",
                user.thread_id, self.page_size
            ),
            None,
        )
        .await
    }

    async fn session(&self, user: &ApiUser) -> ApiCallResult {
        self.request_json(
            user,
            "session",
            Method::GET,
            "/api/webchat/v2/session",
            None,
        )
        .await
    }

    /// Enable the per-user Tools global auto-approve setting so scripted
    /// gated tool calls (`builtin.write_file`, memory writes) can execute
    /// without an interactive approver.
    async fn enable_tools_auto_approve(&self, user: &ApiUser) -> ApiCallResult {
        self.request_json(
            user,
            "enable_auto_approve",
            Method::POST,
            "/api/webchat/v2/settings/tools",
            Some(json!({ "enabled": true })),
        )
        .await
    }

    async fn request_json(
        &self,
        user: &ApiUser,
        name: &'static str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> ApiCallResult {
        self.request_json_with_bearer(user.bearer_token.as_deref(), name, method, path, body)
            .await
    }

    async fn request_json_with_bearer(
        &self,
        bearer_token: Option<&str>,
        name: &'static str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> ApiCallResult {
        let url = format!("{}{}", self.base_url, path);
        let started = Instant::now();
        let mut request = self.client.request(method, &url);
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let result = match tokio::time::timeout(self.request_timeout, request.send()).await {
            Ok(Ok(response)) => {
                let status = response.status();
                match response.text().await {
                    Ok(text) => {
                        if !status.is_success() {
                            Err(FailureCause::new(
                                format!("api_http_status_{}", status.as_u16()),
                                name,
                                truncate_detail(format!("{url}: {text}")),
                            ))
                        } else {
                            serde_json::from_str::<Value>(&text).map_err(|error| {
                                FailureCause::new(
                                    "api_json_decode",
                                    name,
                                    format!("{url}: {error}"),
                                )
                            })
                        }
                    }
                    Err(error) => Err(FailureCause::new(
                        "api_http_body",
                        name,
                        format!("{url}: {error}"),
                    )),
                }
            }
            Ok(Err(error)) => Err(FailureCause::new("api_http_request", name, error)),
            Err(_) => Err(FailureCause::new(
                "api_http_timeout",
                name,
                format!(
                    "{url}: request exceeded {}ms",
                    self.request_timeout.as_millis()
                ),
            )),
        };
        let latency = started.elapsed();
        ApiCallResult {
            sample: ApiRequestSample {
                name,
                latency,
                failure: result.as_ref().err().cloned(),
            },
            value: result,
        }
    }
}

async fn setup_users(
    args: &Args,
    run_id: &str,
    harness: &ApiHarness,
    identities: Vec<ApiIdentity>,
    user_count: usize,
    thread_label: &str,
) -> Result<Vec<ApiUser>, String> {
    let setup_concurrency = args.api_setup_concurrency.max(1);
    let mut next_index = 0;
    let mut join_set = JoinSet::new();
    let mut users = Vec::with_capacity(user_count);

    while next_index < user_count || !join_set.is_empty() {
        while next_index < user_count && join_set.len() < setup_concurrency {
            let harness = harness.clone();
            let identity = identities[next_index % identities.len()].clone();
            let thread_id = format!("stress-{run_id}-{thread_label}-{next_index}");
            let user_index = next_index;
            let threads_per_user = args.api_threads_per_user.max(1);
            let enable_auto_approve = args.api_scripted_tool.is_some();
            // The first user hosts the scripted hot writers: one thread per
            // hot writer in addition to the primary thread.
            let hot_extra_count = if enable_auto_approve && user_index == 0 {
                args.api_hot_writers
            } else {
                0
            };
            let extra_count = threads_per_user.saturating_sub(1).max(hot_extra_count);
            join_set.spawn(async move {
                let create = harness.create_thread(&identity, &thread_id).await;
                let primary = match create.value {
                    Ok(value) => extract_thread_id(&value).unwrap_or(thread_id),
                    Err(failure) => {
                        return Err(format!(
                            "create thread for api user {user_index}: {}: {}",
                            failure.bucket, failure.detail
                        ));
                    }
                };
                if enable_auto_approve {
                    // Scripted tools (`builtin.write_file`, memory writes) are
                    // gated_unless_granted for loop-run origins; enable the
                    // per-user Tools auto-approve setting through the same
                    // settings API a real user would use.
                    let user = ApiUser {
                        index: user_index,
                        label: identity.label.clone(),
                        bearer_token: identity.bearer_token.clone(),
                        thread_id: primary.clone(),
                        extra_thread_ids: Vec::new(),
                    };
                    let approve = harness.enable_tools_auto_approve(&user).await;
                    if let Err(failure) = approve.value {
                        return Err(format!(
                            "enable auto-approve for api user {user_index}: {}: {}",
                            failure.bucket, failure.detail
                        ));
                    }
                }
                // Extra threads grow the user's sidebar; scripted hot writers
                // additionally use them as distinct contention lanes. Sends
                // keep targeting the primary thread.
                let mut extra_thread_ids = Vec::with_capacity(extra_count);
                for extra in 1..=extra_count {
                    let extra_id = format!("{primary}-extra-{extra}");
                    let create = harness.create_thread(&identity, &extra_id).await;
                    match create.value {
                        Ok(value) => {
                            extra_thread_ids.push(extract_thread_id(&value).unwrap_or(extra_id));
                        }
                        Err(failure) => {
                            return Err(format!(
                                "create extra thread {extra} for api user {user_index}: {}: {}",
                                failure.bucket, failure.detail
                            ));
                        }
                    }
                }
                Ok(ApiUser {
                    index: user_index,
                    label: identity.label,
                    bearer_token: identity.bearer_token,
                    thread_id: primary,
                    extra_thread_ids,
                })
            });
            next_index += 1;
        }
        let Some(joined) = join_set.join_next().await else {
            break;
        };
        users.push(joined.map_err(|error| format!("setup task join failed: {error}"))??);
    }

    users.sort_by_key(|user| user.index);
    Ok(users)
}

fn select_identities(
    identities: &[ApiIdentity],
    start_index: usize,
    count: usize,
) -> Vec<ApiIdentity> {
    if count == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|offset| identities[(start_index + offset) % identities.len()].clone())
        .collect()
}

async fn provision_missing_identities(
    args: &Args,
    run_id: &str,
    harness: &ApiHarness,
    identities: Vec<ApiIdentity>,
) -> Result<Vec<ApiIdentity>, String> {
    let Some(admin_bearer_token) = args.api_admin_bearer_token.as_deref() else {
        return Ok(identities);
    };
    if admin_bearer_token.trim().is_empty() {
        return Err("--api-admin-bearer-token must not be empty".to_string());
    }
    let missing_tokens = identities
        .iter()
        .filter(|identity| identity.bearer_token.is_none())
        .count();
    if missing_tokens == 0 {
        return Ok(identities);
    }

    eprintln!(
        "{} provisioning {missing_tokens} API users through admin CRUD",
        crate::log_prefix(args)
    );
    let provisioner_tokens =
        prepare_admin_provisioners(args, run_id, harness, admin_bearer_token).await?;

    let setup_concurrency = args.api_setup_concurrency.max(1);
    let mut results = vec![None; identities.len()];
    let mut next_index = 0;
    let mut join_set = JoinSet::new();

    while next_index < identities.len() || !join_set.is_empty() {
        while next_index < identities.len() && join_set.len() < setup_concurrency {
            let identity = identities[next_index].clone();
            let identity_index = next_index;
            next_index += 1;

            if identity.bearer_token.is_some() {
                results[identity_index] = Some(identity);
                continue;
            }

            let harness = harness.clone();
            let provisioner_token =
                provisioner_tokens[identity_index % provisioner_tokens.len()].clone();
            let email = admin_user_email(run_id, identity_index);
            let display_name = format!("IronClaw Stress {}", identity.label);
            join_set.spawn(async move {
                let create = harness
                    .create_admin_user(&provisioner_token, email, display_name, "member")
                    .await;
                match create.value {
                    Ok(value) => {
                        let created: ApiAdminCreateUserResponse =
                            serde_json::from_value(value).map_err(|error| {
                                format!("parse admin create response for api identity {identity_index}: {error}")
                            })?;
                        if created.api_token.is_empty() {
                            return Err(format!(
                                "admin create response for api identity {identity_index} returned an empty api_token"
                            ));
                        }
                        Ok((
                            identity_index,
                            ApiIdentity {
                                label: created.user.user_id,
                                bearer_token: Some(created.api_token),
                            },
                        ))
                    }
                    Err(failure) => Err(format!(
                        "admin create user for api identity {identity_index}: {}: {}",
                        failure.bucket, failure.detail
                    )),
                }
            });
        }

        if join_set.is_empty() {
            continue;
        }

        let (identity_index, identity) = join_set
            .join_next()
            .await
            .ok_or_else(|| "admin provisioning join set ended unexpectedly".to_string())?
            .map_err(|error| format!("admin provisioning task join failed: {error}"))??;
        results[identity_index] = Some(identity);
    }

    results
        .into_iter()
        .enumerate()
        .map(|(index, identity)| {
            identity.ok_or_else(|| format!("api identity {index} was not initialized"))
        })
        .collect()
}

async fn prepare_admin_provisioners(
    args: &Args,
    run_id: &str,
    harness: &ApiHarness,
    admin_bearer_token: &str,
) -> Result<Vec<String>, String> {
    if args.api_admin_provisioners <= 1 {
        return Ok(vec![admin_bearer_token.to_string()]);
    }

    eprintln!(
        "{} creating {} admin provisioning principals",
        crate::log_prefix(args),
        args.api_admin_provisioners
    );

    let setup_concurrency = args.api_setup_concurrency.max(1);
    let mut next_index = 0;
    let mut join_set = JoinSet::new();
    let mut tokens = vec![None; args.api_admin_provisioners];

    while next_index < args.api_admin_provisioners || !join_set.is_empty() {
        while next_index < args.api_admin_provisioners && join_set.len() < setup_concurrency {
            let provisioner_index = next_index;
            next_index += 1;

            let harness = harness.clone();
            let admin_bearer_token = admin_bearer_token.to_string();
            let provisioner_run_id = format!("{run_id}-admin-provisioner");
            let email = admin_user_email(&provisioner_run_id, provisioner_index);
            let display_name = format!("IronClaw Stress Provisioner {provisioner_index}");
            join_set.spawn(async move {
                let create = harness
                    .create_admin_user(&admin_bearer_token, email, display_name, "admin")
                    .await;
                match create.value {
                    Ok(value) => {
                        let created: ApiAdminCreateUserResponse =
                            serde_json::from_value(value).map_err(|error| {
                                format!(
                                    "parse admin provisioner response {provisioner_index}: {error}"
                                )
                            })?;
                        if created.api_token.is_empty() {
                            return Err(format!(
                                "admin provisioner response {provisioner_index} returned an empty api_token"
                            ));
                        }
                        Ok((provisioner_index, created.api_token))
                    }
                    Err(failure) => Err(format!(
                        "create admin provisioner {provisioner_index}: {}: {}",
                        failure.bucket, failure.detail
                    )),
                }
            });
        }

        let (provisioner_index, token) = join_set
            .join_next()
            .await
            .ok_or_else(|| "admin provisioner join set ended unexpectedly".to_string())?
            .map_err(|error| format!("admin provisioner task join failed: {error}"))??;
        tokens[provisioner_index] = Some(token);
    }

    tokens
        .into_iter()
        .enumerate()
        .map(|(index, token)| {
            token.ok_or_else(|| format!("admin provisioner {index} was not initialized"))
        })
        .collect()
}

async fn load_identities(args: &Args, identity_count: usize) -> Result<Vec<ApiIdentity>, String> {
    if let Some(path) = &args.api_users_jsonl {
        return load_identity_file(path).await;
    }
    let bearer_token = generated_identity_bearer_token(args);
    Ok((0..identity_count)
        .map(|index| ApiIdentity {
            label: format!("user-{index}"),
            bearer_token: bearer_token.clone(),
        })
        .collect())
}

fn generated_identity_bearer_token(args: &Args) -> Option<String> {
    if args.api_admin_bearer_token.is_some() {
        None
    } else {
        args.api_bearer_token.clone()
    }
}

fn admin_user_email(run_id: &str, index: usize) -> String {
    let run_slug = sanitize_email_local_part(run_id);
    let mut local = if run_slug.is_empty() {
        format!("stress-{index}")
    } else {
        format!("stress-{index}-{run_slug}")
    };
    local.truncate(64);
    while local.ends_with('-') {
        local.pop();
    }
    if local.is_empty() {
        local.push_str("stress");
    }
    format!("{local}@ironclaw-stress.local")
}

fn sanitize_email_local_part(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len().min(64));
    let mut previous_dash = false;
    for ch in value.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            previous_dash = false;
            ch.to_ascii_lowercase()
        } else if previous_dash {
            continue;
        } else {
            previous_dash = true;
            '-'
        };
        sanitized.push(next);
    }
    sanitized.trim_matches('-').to_string()
}

async fn load_identity_file(path: &Path) -> Result<Vec<ApiIdentity>, String> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut identities = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let input: ApiUserInput = serde_json::from_str(line).map_err(|error| {
            format!("parse {} line {}: {error}", path.display(), line_index + 1)
        })?;
        let bearer_token = input
            .bearer_token
            .or_else(|| input.authorization.and_then(parse_authorization_value));
        identities.push(ApiIdentity {
            label: input
                .user_id
                .unwrap_or_else(|| format!("user-file-{line_index}")),
            bearer_token,
        });
    }
    if identities.is_empty() {
        return Err(format!("{} did not contain any api users", path.display()));
    }
    Ok(identities)
}

fn parse_authorization_value(value: String) -> Option<String> {
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::to_string)
        .or(Some(value))
}

fn summarize_api_samples(
    samples: &[ApiRequestSample],
    elapsed: Duration,
) -> BTreeMap<String, ApiEndpointSummary> {
    let mut grouped: BTreeMap<String, Vec<Sample>> = BTreeMap::new();
    for sample in samples {
        grouped
            .entry(sample.name.to_string())
            .or_default()
            .push(Sample {
                latency: sample.latency,
                error: sample
                    .failure
                    .as_ref()
                    .map(|failure| failure.bucket.clone()),
                failure: sample.failure.clone(),
                stages: None,
            });
    }
    let elapsed_secs = elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
    grouped
        .into_iter()
        .map(|(name, samples)| {
            let attempted = samples.len() as u64;
            let failed = samples
                .iter()
                .filter(|sample| sample.error.is_some())
                .count() as u64;
            let succeeded = attempted.saturating_sub(failed);
            let mut errors = BTreeMap::new();
            for error in samples.iter().filter_map(|sample| sample.error.as_ref()) {
                *errors.entry(error.clone()).or_insert(0) += 1;
            }
            let mut latencies: Vec<u128> = samples
                .iter()
                .map(|sample| sample.latency.as_micros())
                .collect();
            latencies.sort_unstable();
            (
                name,
                ApiEndpointSummary {
                    attempted,
                    succeeded,
                    failed,
                    throughput_ops_sec: attempted as f64 / elapsed_secs,
                    latency: latency_summary(&latencies),
                    errors,
                    failure_causes: summarize_failure_causes(&samples),
                },
            )
        })
        .collect()
}

fn summarize_flow_samples(samples: &[Sample], elapsed: Duration) -> ApiEndpointSummary {
    let attempted = samples.len() as u64;
    let failed = samples
        .iter()
        .filter(|sample| sample.error.is_some())
        .count() as u64;
    let succeeded = attempted.saturating_sub(failed);
    let mut errors = BTreeMap::new();
    for error in samples.iter().filter_map(|sample| sample.error.as_ref()) {
        *errors.entry(error.clone()).or_insert(0) += 1;
    }
    let mut latencies: Vec<u128> = samples
        .iter()
        .map(|sample| sample.latency.as_micros())
        .collect();
    latencies.sort_unstable();
    ApiEndpointSummary {
        attempted,
        succeeded,
        failed,
        throughput_ops_sec: attempted as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE),
        latency: latency_summary(&latencies),
        errors,
        failure_causes: summarize_failure_causes(samples),
    }
}

impl ReadMix {
    fn parse(spec: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        let mut total_weight = 0_u32;
        for raw in spec.split(',') {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let (name, weight) = raw.split_once('=').ok_or_else(|| {
                format!("invalid --api-read-mix entry {raw:?}; expected name=weight")
            })?;
            let weight: u32 = weight
                .parse()
                .map_err(|error| format!("invalid --api-read-mix weight {weight:?}: {error}"))?;
            if weight == 0 {
                continue;
            }
            total_weight = total_weight.saturating_add(weight);
            let endpoint = match name.trim() {
                "list_threads" | "threads" => ReadEndpoint::ListThreads,
                "timeline" => ReadEndpoint::Timeline,
                "session" => ReadEndpoint::Session,
                other => {
                    return Err(format!(
                        "unknown --api-read-mix endpoint {other:?}; expected list_threads, timeline, or session"
                    ));
                }
            };
            entries.push(ReadMixEntry {
                endpoint,
                cumulative_weight: total_weight,
            });
        }
        if entries.is_empty() || total_weight == 0 {
            return Err("--api-read-mix must include at least one positive weight".to_string());
        }
        Ok(Self {
            spec: spec.to_string(),
            total_weight,
            entries,
        })
    }

    fn endpoint_for(&self, operation_index: u64) -> ReadEndpoint {
        let slot = (operation_index % self.total_weight as u64) as u32;
        self.entries
            .iter()
            .find(|entry| slot < entry.cumulative_weight)
            .map(|entry| entry.endpoint)
            .unwrap_or(ReadEndpoint::Timeline)
    }
}

fn read_worker_count(args: &Args) -> usize {
    if args.api_read_workers > 0 {
        args.api_read_workers
    } else {
        args.users.clamp(1, 64)
    }
}

fn read_worker_interval(args: &Args) -> Duration {
    read_worker_interval_for(
        args.api_read_qps_per_user,
        args.users,
        read_worker_count(args),
    )
}

fn read_worker_interval_for(
    api_read_qps_per_user: f64,
    users: usize,
    read_workers: usize,
) -> Duration {
    let aggregate_qps = api_read_qps_per_user * users as f64;
    if !aggregate_qps.is_finite() || aggregate_qps <= 0.0 {
        return MAX_READ_WORKER_SLEEP;
    }
    let worker_count = read_workers.max(1) as f64;
    let sleep_secs = (worker_count / aggregate_qps).max(0.001);
    if !sleep_secs.is_finite() || sleep_secs > MAX_READ_WORKER_SLEEP.as_secs_f64() {
        MAX_READ_WORKER_SLEEP
    } else {
        Duration::from_secs_f64(sleep_secs)
    }
}

fn extract_thread_id(value: &Value) -> Option<String> {
    value
        .pointer("/thread/thread_id")
        .or_else(|| value.pointer("/thread/id"))
        .or_else(|| value.get("thread_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn submitted_or_already_submitted(value: &Value) -> bool {
    matches!(
        value.get("outcome").and_then(Value::as_str),
        Some("submitted" | "already_submitted")
    )
}

fn timeline_finalized_assistant_count(value: &Value) -> usize {
    value
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| {
            let kind = message.get("kind").and_then(Value::as_str);
            let status = message.get("status").and_then(Value::as_str);
            kind == Some("assistant") && status == Some("finalized")
        })
        .count()
}

fn stress_payload(prefix: String, minimum_bytes: usize) -> String {
    if minimum_bytes <= prefix.len() {
        return prefix;
    }
    let mut value = prefix;
    value.push(' ');
    while value.len() < minimum_bytes {
        value.push_str("stress-payload ");
    }
    value.truncate(minimum_bytes);
    value
}

fn compact_json(value: &Value) -> String {
    truncate_detail(
        serde_json::to_string(value).unwrap_or_else(|error| format!("json encode error: {error}")),
    )
}

fn truncate_detail(value: impl Into<String>) -> String {
    let value = value.into().replace(['\n', '\r'], " ");
    let mut truncated = value.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>();
    if value.chars().count() > MAX_ERROR_BODY_CHARS {
        truncated.push_str("...");
    }
    truncated
}

async fn start_mock_llm(args: &Args) -> Result<Option<MockLlmHandle>, String> {
    let Some(bind) = args.mock_llm_bind else {
        return Ok(None);
    };
    let config = MockLlmConfig {
        bind,
        model: args.mock_llm_model.clone(),
        latency_ms: args.mock_llm_latency_ms,
        background_latency_ms: args.mock_llm_background_latency_ms,
        jitter_ms: args.mock_llm_jitter_ms,
        output_bytes: args.mock_llm_output_bytes,
        failure_rate: args.mock_llm_failure_rate,
        scripted: args.api_scripted_tool,
    };
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|error| format!("bind mock llm {}: {error}", config.bind))?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("read mock llm local addr: {error}"))?;
    let state = Arc::new(MockLlmState {
        model: config.model.clone(),
        latency_ms: config.latency_ms,
        background_latency_ms: config.background_latency_ms,
        jitter_ms: config.jitter_ms,
        output_bytes: config.output_bytes,
        failure_rate: config.failure_rate,
        scripted: config.scripted,
        scripted_counters: Mutex::new(ScriptedMockCounters::default()),
        scripted_driver: Mutex::new(scripted::ScriptedDriver::new(
            scripted::DEFAULT_SCRIPTED_SESSION_CAPACITY,
        )),
        counter: AtomicU64::new(0),
        foreground_counter: AtomicU64::new(0),
        background_counter: AtomicU64::new(0),
        in_flight: AtomicU64::new(0),
        max_in_flight: AtomicU64::new(0),
        started_at: Instant::now(),
        request_latencies: Mutex::new(Vec::new()),
        foreground_request_latencies: Mutex::new(Vec::new()),
        background_request_latencies: Mutex::new(Vec::new()),
        request_start_offsets: Mutex::new(Vec::new()),
    });
    let (stop_sender, mut stop_receiver) = oneshot::channel();
    let accept_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_receiver => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            let state = Arc::clone(&accept_state);
                            tokio::spawn(async move {
                                let _ = handle_mock_llm_connection(stream, state).await;
                            });
                        }
                        Err(error) => {
                            eprintln!("mock llm accept failed: {error}");
                            break;
                        }
                    }
                }
            }
        }
    });
    let base_url = format!("http://{local_addr}/v1");
    eprintln!(
        "{} mock LLM listening at {base_url} model={}",
        crate::log_prefix(args),
        config.model
    );
    Ok(Some(MockLlmHandle {
        base_url,
        state,
        stop_sender: Some(stop_sender),
    }))
}

async fn handle_mock_llm_connection(
    mut stream: TcpStream,
    state: Arc<MockLlmState>,
) -> Result<(), String> {
    let mut buffer = vec![0_u8; 8192];
    let mut request = Vec::new();
    let header_end = loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = find_header_end(&request) {
            break index;
        }
        if request.len() > 1024 * 1024 {
            write_mock_response(&mut stream, 413, "text/plain", b"request too large").await?;
            return Ok(());
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]).into_owned();
    let content_length = parse_content_length(&headers).unwrap_or(0);
    if content_length > MAX_MOCK_LLM_REQUEST_BODY_BYTES {
        write_mock_response(&mut stream, 413, "text/plain", b"request too large").await?;
        return Ok(());
    }
    let body_start = header_end + 4;
    let total_request_len = match body_start.checked_add(content_length) {
        Some(total) => total,
        None => {
            write_mock_response(&mut stream, 413, "text/plain", b"request too large").await?;
            return Ok(());
        }
    };
    while request.len() < total_request_len {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let body = request
        .get(body_start..total_request_len)
        .unwrap_or_default();
    let first_line = headers.lines().next().unwrap_or_default();
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    match path {
        "/v1/models" | "/models" => {
            let body = json!({
                "object": "list",
                "data": [{"id": state.model, "object": "model"}],
            });
            write_json_response(&mut stream, 200, &body).await
        }
        "/v1/chat/completions" | "/chat/completions" => {
            handle_mock_completion(&mut stream, body, state).await
        }
        _ => write_mock_response(&mut stream, 404, "text/plain", b"not found").await,
    }
}

async fn handle_mock_completion(
    stream: &mut TcpStream,
    body: &[u8],
    state: Arc<MockLlmState>,
) -> Result<(), String> {
    let request_kind = classify_mock_request(body);
    let _request_guard = MockRequestGuard::new(&state, request_kind);
    let request_index = state.counter.fetch_add(1, Ordering::Relaxed) + 1;
    match request_kind {
        MockRequestKind::Foreground => {
            state.foreground_counter.fetch_add(1, Ordering::Relaxed);
        }
        MockRequestKind::Background => {
            state.background_counter.fetch_add(1, Ordering::Relaxed);
        }
    }
    if should_fail_mock_request(state.failure_rate, request_index) {
        write_mock_response(
            stream,
            500,
            "application/json",
            br#"{"error":{"message":"mock llm configured failure"}}"#,
        )
        .await?;
        return Ok(());
    }
    let jitter = if state.jitter_ms == 0 {
        0
    } else {
        deterministic_jitter_ms(request_index, state.jitter_ms)
    };
    let base_latency_ms = match request_kind {
        MockRequestKind::Foreground => state.latency_ms,
        MockRequestKind::Background => state.resolved_background_latency_ms(),
    };
    let wait = base_latency_ms.saturating_add(jitter);
    if wait > 0 {
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }
    let request: Value = serde_json::from_slice(body).unwrap_or_else(|_| json!({}));
    let stream_response = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let decision = resolve_scripted_decision_for(&state, &request, request_index).await;
    match &decision {
        ScriptedDecision::ToolCalls(calls) => {
            if stream_response {
                let (header, body, done) =
                    mock_streaming_tool_call_chunks(&state.model, request_index, calls);
                let payload =
                    format!("data: {header}\n\ndata: {body}\n\ndata: {done}\n\ndata: [DONE]\n\n");
                write_mock_response(stream, 200, "text/event-stream", payload.as_bytes()).await
            } else {
                let response = mock_tool_call_response(&state.model, request_index, calls);
                write_json_response(stream, 200, &response).await
            }
        }
        ScriptedDecision::FinalText(text) => {
            if stream_response {
                let (chunk, done) =
                    mock_streaming_completion_chunks(&state.model, request_index, text.clone());
                let payload = format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n");
                write_mock_response(stream, 200, "text/event-stream", payload.as_bytes()).await
            } else {
                let response = mock_completion_response(&state.model, request_index, text.clone());
                write_json_response(stream, 200, &response).await
            }
        }
        ScriptedDecision::RetryAfter(_) => {
            Err("delayed retry decision escaped the mock completion resolver".to_string())
        }
        ScriptedDecision::Placeholder => {
            let text = scripted::PLACEHOLDER_TEXT.to_string();
            if stream_response {
                let (chunk, done) =
                    mock_streaming_completion_chunks(&state.model, request_index, text);
                let payload = format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n");
                write_mock_response(stream, 200, "text/event-stream", payload.as_bytes()).await
            } else {
                let response = mock_completion_response(&state.model, request_index, text);
                write_json_response(stream, 200, &response).await
            }
        }
        ScriptedDecision::None => {
            let content = stress_payload("mock assistant response".to_string(), state.output_bytes);
            if stream_response {
                let (chunk, done) =
                    mock_streaming_completion_chunks(&state.model, request_index, content);
                let payload = format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n");
                write_mock_response(stream, 200, "text/event-stream", payload.as_bytes()).await
            } else {
                let response = mock_completion_response(&state.model, request_index, content);
                write_json_response(stream, 200, &response).await
            }
        }
    }
}

/// Advertised tool names in a chat completion request (`tools[].function.name`).
fn available_tool_names(request: &Value) -> HashSet<String> {
    let mut names = HashSet::new();
    let Some(tools) = request.get("tools").and_then(Value::as_array) else {
        return names;
    };
    for tool in tools {
        let function = tool
            .get("function")
            .filter(|function| function.is_object())
            .or_else(|| (tool.is_object()).then_some(tool));
        let Some(function) = function else {
            continue;
        };
        if let Some(name) = function.get("name").and_then(Value::as_str) {
            names.insert(name.to_string());
        }
    }
    names
}

/// Decide the scripted response for a request through the stateful
/// [`scripted::ScriptedDriver`] and record sidecar counters. The driver is
/// the live path: it recovers the operation from user content or the
/// canonical markers embedded in assistant tool-call arguments, tracks
/// per-operation emitted/completed steps across compacted requests, and
/// removes the session once the final verdict is emitted. `request_index`
/// is the index `handle_mock_completion` already reserved for this
/// request's response; the call id the response will carry
/// (`call-stress-<request_index>`) is recorded on the session so the
/// following request's tool result can be matched exactly. The id is taken
/// from the passed index, never reloaded from the shared counter — a
/// concurrent request could increment the counter between the reservation
/// and a reload, recording an id that differs from the one the response
/// actually carries.
fn scripted_decision_for(
    state: &MockLlmState,
    request: &Value,
    request_index: u64,
) -> ScriptedDecision {
    scripted_decision_for_inner(state, request, request_index, true)
}

fn scripted_decision_for_inner(
    state: &MockLlmState,
    request: &Value,
    request_index: u64,
    count_request: bool,
) -> ScriptedDecision {
    if state.scripted.is_none() {
        return ScriptedDecision::None;
    }
    let available = available_tool_names(request);
    let next_call_id = mock_tool_call_id(request_index);
    let (decision, op) = state
        .scripted_driver
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .decide(request, &available, &next_call_id);
    let Some(op) = op else {
        return ScriptedDecision::None;
    };
    let mut counters = state
        .scripted_counters
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if count_request {
        *counters
            .requests_seen
            .entry(op.key.as_str().to_string())
            .or_insert(0) += 1;
    }
    match &decision {
        ScriptedDecision::ToolCalls(calls) => {
            *counters
                .tool_calls_emitted
                .entry(op.key.as_str().to_string())
                .or_insert(0) += calls.len() as u64;
        }
        ScriptedDecision::RetryAfter(_) => {}
        ScriptedDecision::Placeholder => {
            counters.placeholder_responses += 1;
        }
        ScriptedDecision::FinalText(text) => {
            if let Some(verdict) = scripted::parse_result_verdict(text, &op) {
                *counters
                    .final_verdicts
                    .entry(verdict.as_str().to_string())
                    .or_insert(0) += 1;
            }
        }
        ScriptedDecision::None => {}
    }
    decision
}

/// Resolve provider-directed retry delays inside the current HTTP completion
/// request. No driver or counter mutex is held while sleeping. Re-evaluation
/// does not count as another model request, but an emitted retry tool call is
/// still recorded.
async fn resolve_scripted_decision_for(
    state: &MockLlmState,
    request: &Value,
    request_index: u64,
) -> ScriptedDecision {
    let mut decision = scripted_decision_for(state, request, request_index);
    loop {
        let ScriptedDecision::RetryAfter(delay) = decision else {
            return decision;
        };
        tokio::time::sleep(delay).await;
        decision = scripted_decision_for_inner(state, request, request_index, false);
    }
}

fn classify_mock_request(body: &[u8]) -> MockRequestKind {
    if std::str::from_utf8(body).is_ok_and(|body| body.contains(BACKGROUND_FLOW_MARKER)) {
        MockRequestKind::Background
    } else {
        MockRequestKind::Foreground
    }
}

fn mock_completion_response(model: &str, request_index: u64, content: String) -> Value {
    json!({
        "id": format!("chatcmpl-stress-{request_index}"),
        "object": "chat.completion",
        "created": MOCK_COMPLETION_CREATED_AT,
        "model": model,
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
                "tool_calls": null,
                "refusal": null
            },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 16,
            "completion_tokens": 16,
            "total_tokens": 32,
            "prompt_tokens_details": null
        }
    })
}

/// Deterministic tool-call id for the single scripted call of a response.
fn mock_tool_call_id(request_index: u64) -> String {
    mock_tool_call_id_at(request_index, 0)
}

/// Deterministic, response-local tool-call id. Preserve the established bare
/// request id for the first call; suffix later calls with their array index.
fn mock_tool_call_id_at(request_index: u64, call_index: usize) -> String {
    if call_index == 0 {
        format!("call-stress-{request_index}")
    } else {
        format!("call-stress-{request_index}-{call_index}")
    }
}

/// Non-streaming chat completion carrying the scripted tool call(s). The
/// `arguments` field is a JSON string, matching the rig OpenAI wire shape
/// the server parses (`Function::arguments` uses `stringified_json`); the
/// `tool_calls` array order is the execution order and every id is unique.
/// Scripted plans emit exactly one call per response.
fn mock_tool_call_response(
    model: &str,
    request_index: u64,
    calls: &[scripted::ToolCallSpec],
) -> Value {
    let tool_calls = calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            json!({
                "id": mock_tool_call_id_at(request_index, index),
                "type": "function",
                "function": {
                    "name": call.wire_name,
                    "arguments": call.arguments.to_string()
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "id": format!("chatcmpl-stress-{request_index}"),
        "object": "chat.completion",
        "created": MOCK_COMPLETION_CREATED_AT,
        "model": model,
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": tool_calls,
                "refusal": null
            },
            "logprobs": null,
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 16,
            "completion_tokens": 5,
            "total_tokens": 21,
            "prompt_tokens_details": null
        }
    })
}

/// Streaming chat-completion chunks carrying the scripted tool call(s): a
/// header chunk with every call's id and name, an arguments chunk with
/// every call's arguments, and a finish chunk. Delta tool calls use
/// distinct 0-based indices and unique ids so the client accumulates each
/// call independently. Scripted plans emit exactly one call per response.
fn mock_streaming_tool_call_chunks(
    model: &str,
    request_index: u64,
    calls: &[scripted::ToolCallSpec],
) -> (Value, Value, Value) {
    let base = json!({
        "id": format!("chatcmpl-stress-{request_index}"),
        "object": "chat.completion.chunk",
        "created": MOCK_COMPLETION_CREATED_AT,
        "model": model,
        "system_fingerprint": null,
    });
    let header_calls = calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            json!({
                "index": index,
                "id": mock_tool_call_id_at(request_index, index),
                "type": "function",
                "function": {"name": call.wire_name, "arguments": ""}
            })
        })
        .collect::<Vec<_>>();
    let mut header = base.clone();
    header["choices"] = json!([{
        "index": 0,
        "delta": {
            "role": "assistant",
            "content": null,
            "tool_calls": header_calls
        },
        "logprobs": null,
        "finish_reason": null
    }]);

    let body_calls = calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let arguments = call.arguments.to_string();
            json!({
                "index": index,
                "function": {"arguments": arguments}
            })
        })
        .collect::<Vec<_>>();
    let mut body = base.clone();
    body["choices"] = json!([{
        "index": 0,
        "delta": {
            "tool_calls": body_calls
        },
        "logprobs": null,
        "finish_reason": null
    }]);

    let mut done = base;
    done["choices"] = json!([{
        "index": 0,
        "delta": {},
        "logprobs": null,
        "finish_reason": "tool_calls"
    }]);
    done["usage"] = json!({
        "prompt_tokens": 16,
        "completion_tokens": 5,
        "total_tokens": 21,
        "prompt_tokens_details": null
    });
    (header, body, done)
}

fn mock_streaming_completion_chunks(
    model: &str,
    request_index: u64,
    content: String,
) -> (Value, Value) {
    let base = json!({
        "id": format!("chatcmpl-stress-{request_index}"),
        "object": "chat.completion.chunk",
        "created": MOCK_COMPLETION_CREATED_AT,
        "model": model,
        "system_fingerprint": null,
    });
    let mut chunk = base.clone();
    chunk["choices"] = json!([{
        "index": 0,
        "delta": {"content": content},
        "logprobs": null,
        "finish_reason": null
    }]);

    let mut done = base;
    done["choices"] = json!([{
        "index": 0,
        "delta": {},
        "logprobs": null,
        "finish_reason": "stop"
    }]);
    done["usage"] = json!({
        "prompt_tokens": 16,
        "completion_tokens": 16,
        "total_tokens": 32,
        "prompt_tokens_details": null
    });
    (chunk, done)
}

fn should_fail_mock_request(failure_rate: f64, request_index: u64) -> bool {
    if failure_rate <= 0.0 {
        return false;
    }
    if failure_rate >= 1.0 {
        return true;
    }
    let period = (1.0 / failure_rate).round().max(1.0) as u64;
    request_index.is_multiple_of(period)
}

fn deterministic_jitter_ms(request_index: u64, jitter_ms: u64) -> u64 {
    request_index
        .wrapping_mul(1_103_515_245)
        .wrapping_add(12_345)
        % jitter_ms.saturating_add(1)
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

async fn write_json_response(
    stream: &mut TcpStream,
    status: u16,
    body: &Value,
) -> Result<(), String> {
    let encoded = serde_json::to_vec(body).map_err(|error| error.to_string())?;
    write_mock_response(stream, status, "application/json", &encoded).await
}

async fn write_mock_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(body)
        .await
        .map_err(|error| error.to_string())?;
    stream.shutdown().await.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parsed_api_args(extra: &[&str]) -> Args {
        let mut args = vec![
            "ironclaw_stress",
            "--backend",
            "libsql",
            "--scenario",
            "api-user-capacity",
            "--api-base-url",
            "http://127.0.0.1:4216",
        ];
        args.extend_from_slice(extra);
        Args::parse_from(args)
    }

    #[test]
    fn read_mix_parses_aliases_and_weights() {
        let mix = ReadMix::parse("threads=2,timeline=3,session=1").unwrap();
        assert_eq!(mix.total_weight, 6);
        assert_eq!(mix.endpoint_for(0), ReadEndpoint::ListThreads);
        assert_eq!(mix.endpoint_for(2), ReadEndpoint::Timeline);
        assert_eq!(mix.endpoint_for(5), ReadEndpoint::Session);
    }

    #[test]
    fn extracts_thread_id_from_create_thread_response() {
        let value = json!({"thread": {"thread_id": "thread-1"}});
        assert_eq!(extract_thread_id(&value).as_deref(), Some("thread-1"));
    }

    #[test]
    fn admin_user_email_is_stable_ascii_and_bounded() {
        let email = admin_user_email("Run 01 / With Symbols", 42);
        assert_eq!(email, "stress-42-run-01-with-symbols@ironclaw-stress.local");
        let long = admin_user_email(&"x".repeat(200), 7);
        let local = long.split_once('@').unwrap().0;
        assert!(local.len() <= 64);
        assert!(long.ends_with("@ironclaw-stress.local"));
    }

    #[test]
    fn parses_admin_create_user_token_response() {
        let created: ApiAdminCreateUserResponse = serde_json::from_value(json!({
            "user": {"user_id": "stress-user"},
            "api_token": "session-token"
        }))
        .expect("admin create response parses");
        assert_eq!(created.user.user_id, "stress-user");
        assert_eq!(created.api_token, "session-token");
    }

    #[test]
    fn detects_finalized_assistant_in_timeline() {
        let value = json!({
            "messages": [
                {"kind": "user", "status": "finalized"},
                {"kind": "assistant", "status": "finalized"}
            ]
        });
        assert_eq!(timeline_finalized_assistant_count(&value), 1);
    }

    #[test]
    fn counts_only_finalized_assistant_messages_in_timeline() {
        let value = json!({
            "messages": [
                {"kind": "assistant", "status": "submitted"},
                {"kind": "assistant", "status": "finalized"},
                {"kind": "user", "status": "finalized"},
                {"kind": "assistant", "status": "finalized"}
            ]
        });
        assert_eq!(timeline_finalized_assistant_count(&value), 2);
    }

    #[test]
    fn submitted_assistant_is_not_finalized_for_full_flow() {
        let value = json!({
            "messages": [
                {"kind": "assistant", "status": "submitted"}
            ]
        });
        assert_eq!(timeline_finalized_assistant_count(&value), 0);
    }

    #[test]
    fn deterministic_jitter_handles_max_ceiling() {
        let expected = 42_u64.wrapping_mul(1_103_515_245).wrapping_add(12_345) % u64::MAX;
        assert_eq!(deterministic_jitter_ms(42, u64::MAX), expected);
    }

    #[test]
    fn mock_completion_response_matches_rig_openai_shape() {
        let response = mock_completion_response("stress-mock", 7, "ok".to_string());

        serde_json::from_value::<rig::providers::openai::completion::CompletionResponse>(response)
            .expect("stress mock should deserialize as rig-core OpenAI completion response");
    }

    #[test]
    fn mock_tool_call_response_matches_rig_openai_shape() {
        let call = scripted::ToolCallSpec {
            wire_name: "ironclaw__memory__write".to_string(),
            arguments: json!({
                "target": "stress/shared.md",
                "content": "IRONCLAW_STRESS_READBACK_u0__1 payload",
                "append": false,
            }),
        };
        let second_call = scripted::ToolCallSpec {
            wire_name: "ironclaw__memory__read".to_string(),
            arguments: json!({"path": "stress/shared.md"}),
        };
        let response = mock_tool_call_response("stress-mock", 7, &[call, second_call]);

        assert_eq!(response["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(response["choices"][0]["message"]["content"], Value::Null);
        assert_eq!(
            response["choices"][0]["message"]["tool_calls"][0]["id"],
            "call-stress-7"
        );
        assert_eq!(
            response["choices"][0]["message"]["tool_calls"][1]["id"],
            "call-stress-7-1"
        );
        let parsed: rig::providers::openai::completion::CompletionResponse =
            serde_json::from_value(response)
                .expect("stress mock tool call should deserialize as rig-core OpenAI completion");
        let message = &parsed.choices[0].message;
        let tool_calls = match message {
            rig::providers::openai::Message::Assistant { tool_calls, .. } => tool_calls,
            other => panic!("expected assistant message, got {other:?}"),
        };
        assert_eq!(tool_calls.len(), 2);
        assert_eq!(tool_calls[0].id, "call-stress-7");
        assert_eq!(tool_calls[1].id, "call-stress-7-1");
        assert_eq!(tool_calls[0].function.name, "ironclaw__memory__write");
        assert_eq!(
            tool_calls[0].function.arguments,
            json!({
                "target": "stress/shared.md",
                "content": "IRONCLAW_STRESS_READBACK_u0__1 payload",
                "append": false,
            })
        );
    }

    #[test]
    fn mock_streaming_tool_call_chunks_carry_indexed_delta_tool_calls() {
        let call = scripted::ToolCallSpec {
            wire_name: "builtin__write_file".to_string(),
            arguments: json!({"path": "stress/u0__1.txt", "content": "payload"}),
        };
        let second_call = scripted::ToolCallSpec {
            wire_name: "builtin__read_file".to_string(),
            arguments: json!({"path": "stress/u0__1.txt"}),
        };
        let (header, body, done) =
            mock_streaming_tool_call_chunks("stress-mock", 7, &[call, second_call]);

        let header_call = &header["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(header_call["index"], 0);
        assert_eq!(header_call["id"], "call-stress-7");
        assert_eq!(header_call["function"]["name"], "builtin__write_file");
        let second_header_call = &header["choices"][0]["delta"]["tool_calls"][1];
        assert_eq!(second_header_call["index"], 1);
        assert_eq!(second_header_call["id"], "call-stress-7-1");
        assert_eq!(header["choices"][0]["delta"]["role"], "assistant");

        let body_call = &body["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(body_call["index"], 0);
        let arguments: Value =
            serde_json::from_str(body_call["function"]["arguments"].as_str().expect("string"))
                .expect("arguments json");
        assert_eq!(arguments["path"], "stress/u0__1.txt");
        assert_eq!(done["choices"][0]["finish_reason"], "tool_calls");
    }

    fn mock_state(scripted: Option<ScriptKey>) -> MockLlmState {
        MockLlmState {
            model: "stress-mock".to_string(),
            latency_ms: 0,
            background_latency_ms: 0,
            jitter_ms: 0,
            output_bytes: 0,
            failure_rate: 0.0,
            scripted,
            scripted_counters: Mutex::new(ScriptedMockCounters::default()),
            scripted_driver: Mutex::new(scripted::ScriptedDriver::new(
                scripted::DEFAULT_SCRIPTED_SESSION_CAPACITY,
            )),
            counter: AtomicU64::new(0),
            foreground_counter: AtomicU64::new(0),
            background_counter: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            max_in_flight: AtomicU64::new(0),
            started_at: Instant::now(),
            request_latencies: Mutex::new(Vec::new()),
            foreground_request_latencies: Mutex::new(Vec::new()),
            background_request_latencies: Mutex::new(Vec::new()),
            request_start_offsets: Mutex::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn mock_completion_waits_then_emits_delayed_retry_tool_call() {
        let state = Arc::new(mock_state(Some(ScriptKey::MemoryRoundtrip)));
        let marker = scripted::marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let tools = json!([
            {"type": "function", "function": {"name": "ironclaw__memory__write"}},
            {"type": "function", "function": {"name": "ironclaw__memory__read"}},
        ]);
        let initial_request = json!({
            "stream": false,
            "messages": [{"role": "user", "content": marker}],
            "tools": tools,
        });
        let initial_index = state.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let ScriptedDecision::ToolCalls(calls) =
            scripted_decision_for(&state, &initial_request, initial_index)
        else {
            panic!("expected initial write call");
        };
        let initial_id = mock_tool_call_id(initial_index);
        let error_observation = r#"{"schema_version":1,"status":"error","summary":"transient","detail":{"kind":"generic_failure","failure_kind":"transient"},"artifacts":[],"recovery":{"same_call_retry":"allowed_after_delay","recovery_hint":"wait_then_retry","retry_after_ms":50},"trust":"untrusted_tool_output"}"#;
        let retry_request = json!({
            "stream": false,
            "messages": [
                {"role": "user", "content": marker},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": initial_id,
                        "type": "function",
                        "function": {
                            "name": calls[0].wire_name,
                            "arguments": serde_json::to_string(&calls[0].arguments)
                                .expect("arguments json"),
                        },
                    }],
                },
                {
                    "role": "tool",
                    "tool_call_id": initial_id,
                    "content": error_observation,
                },
            ],
            "tools": tools,
        });
        let body = serde_json::to_vec(&retry_request).expect("request json");

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("listener address");
        let (accepted, connected) = tokio::join!(listener.accept(), TcpStream::connect(address));
        let (mut server, _) = accepted.expect("accept");
        let mut client = connected.expect("connect");
        let handler_state = Arc::clone(&state);
        let handler =
            tokio::spawn(
                async move { handle_mock_completion(&mut server, &body, handler_state).await },
            );

        let mut response = Vec::new();
        let mut read_response = Box::pin(client.read_to_end(&mut response));
        tokio::select! {
            result = &mut read_response => {
                panic!("a delayed retry responded before 5ms: {result:?}");
            }
            () = tokio::time::sleep(Duration::from_millis(5)) => {}
        }
        tokio::time::timeout(Duration::from_millis(500), &mut read_response)
            .await
            .expect("delayed retry response timeout")
            .expect("read delayed retry response");
        drop(read_response);
        handler
            .await
            .expect("mock handler task")
            .expect("mock handler response");

        let body_start = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
            .expect("HTTP response body");
        let payload: Value =
            serde_json::from_slice(&response[body_start..]).expect("response json");
        assert_eq!(payload["choices"][0]["finish_reason"], "tool_calls");
        assert_ne!(
            payload["choices"][0]["message"]["tool_calls"][0]["id"],
            initial_id
        );
        let counters = state
            .scripted_counters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(counters.requests_seen["memory_roundtrip"], 2);
        assert_eq!(counters.tool_calls_emitted["memory_roundtrip"], 2);
    }

    #[test]
    fn scripted_counters_count_calls_not_responses() {
        let state = mock_state(Some(ScriptKey::MemoryRoundtrip));
        const MIB: usize = 1024 * 1024;
        let marker = scripted::marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        let request = |messages: Vec<Value>| {
            json!({
                "stream": false,
                "messages": messages,
                "tools": [
                    {"type": "function", "function": {"name": "ironclaw__memory__write"}},
                    {"type": "function", "function": {"name": "ironclaw__memory__read"}},
                    {"type": "function", "function": {"name": "ironclaw__memory__search"}},
                ]
            })
        };
        let mut messages = vec![json!({"role": "user", "content": marker})];
        // Each request emits exactly one scripted call, and the counter
        // records exactly one emitted call per request: a 1 MiB plan needs
        // one response per plan step (19 for the roundtrip), never zero
        // and never more than one. The request index mirrors the
        // production reservation in `handle_mock_completion`.
        for emitted in 1..=3 {
            let request_index = state.counter.fetch_add(1, Ordering::Relaxed) + 1;
            match scripted_decision_for(&state, &request(messages.clone()), request_index) {
                ScriptedDecision::ToolCalls(calls) => {
                    assert_eq!(calls.len(), 1, "exactly one call per response");
                }
                other => panic!("expected a tool call, got {other:?}"),
            }
            let counters = state
                .scripted_counters
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert_eq!(
                counters.requests_seen["memory_roundtrip"], emitted,
                "request {emitted}"
            );
            assert_eq!(
                counters.tool_calls_emitted["memory_roundtrip"], emitted,
                "counter must count calls, not responses"
            );
            messages.push(json!({
                "role": "tool",
                "content": "Tool ironclaw.memory.write returned: ok"
            }));
        }
    }

    #[test]
    fn scripted_one_mib_plan_emits_search_checkpoint() {
        let state = mock_state(Some(ScriptKey::MemoryRoundtrip));
        const MIB: usize = 1024 * 1024;
        let marker = scripted::marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        let request = |messages: Vec<Value>| {
            json!({
                "stream": false,
                "messages": messages,
                "tools": [
                    {"type": "function", "function": {"name": "ironclaw__memory__write"}},
                    {"type": "function", "function": {"name": "ironclaw__memory__read"}},
                    {"type": "function", "function": {"name": "ironclaw__memory__search"}},
                ]
            })
        };
        let mut messages = vec![json!({"role": "user", "content": marker})];
        let mut emitted_writes = 0usize;
        loop {
            let request_index = state.counter.fetch_add(1, Ordering::Relaxed) + 1;
            match scripted_decision_for(&state, &request(messages.clone()), request_index) {
                ScriptedDecision::ToolCalls(calls) => {
                    assert_eq!(
                        calls.len(),
                        1,
                        "one search checkpoint remains after the writes"
                    );
                    if emitted_writes < 18 {
                        assert_eq!(calls[0].wire_name, "ironclaw__memory__write");
                        emitted_writes += 1;
                        messages.push(json!({
                            "role": "tool",
                            "content": "Tool ironclaw.memory.write returned: ok"
                        }));
                    } else {
                        assert_eq!(calls[0].wire_name, "ironclaw__memory__search");
                        assert_eq!(calls[0].arguments["query"], scripted::MEMORY_SEARCH_QUERY);
                        assert_eq!(calls[0].arguments["limit"], scripted::MEMORY_SEARCH_LIMIT);
                        break;
                    }
                }
                other => panic!("expected a tool call, got {other:?}"),
            }
        }
    }

    #[test]
    fn scripted_driver_survives_compaction_to_one_mib_confirmed() {
        // The live mock path with production-style compaction: after the
        // first request the original user marker is gone and every request
        // retains only the last assistant/tool pair. The stateful driver
        // must recover the operation from the embedded marker in the write
        // call's arguments (and from the recorded call id at the search
        // checkpoint), advance exactly one step per request, and finalize
        // with confirmed after 19 singleton calls.
        let state = mock_state(Some(ScriptKey::MemoryRoundtrip));
        const MIB: usize = 1024 * 1024;
        let parsed = scripted::ScriptedOp {
            key: ScriptKey::MemoryRoundtrip,
            user: "u0".to_string(),
            op: "1".to_string(),
            size_bytes: MIB,
        };
        let marker = scripted::marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", MIB);
        let tools = json!([
            {"type": "function", "function": {"name": "ironclaw__memory__write"}},
            {"type": "function", "function": {"name": "ironclaw__memory__read"}},
            {"type": "function", "function": {"name": "ironclaw__memory__search"}},
        ]);
        let mut last_pair: Vec<Value> = vec![json!({"role": "user", "content": marker})];
        let mut calls = 0usize;
        loop {
            // Reserve the request index exactly like the production path,
            // and echo the same index's call id in the assistant message:
            // the driver must record the id the response actually carries.
            let request_index = state.counter.fetch_add(1, Ordering::Relaxed) + 1;
            let decision = scripted_decision_for(
                &state,
                &json!({ "stream": false, "messages": last_pair.clone(), "tools": tools.clone() }),
                request_index,
            );
            match decision {
                ScriptedDecision::ToolCalls(call_list) => {
                    assert_eq!(call_list.len(), 1, "never batched");
                    calls += 1;
                    let spec = call_list[0].clone();
                    let id = mock_tool_call_id(request_index);
                    let echoed = if spec.wire_name == "ironclaw__memory__search" {
                        parsed.readback_token()
                    } else {
                        "Tool ironclaw.memory.write returned: ok".to_string()
                    };
                    last_pair = vec![
                        json!({
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": spec.wire_name,
                                    "arguments": serde_json::to_string(&spec.arguments)
                                        .expect("arguments json")
                                }
                            }]
                        }),
                        json!({ "role": "tool", "tool_call_id": id, "content": echoed }),
                    ];
                }
                ScriptedDecision::FinalText(text) => {
                    assert_eq!(text, "ironclaw-stress-tool result u0__1 confirmed");
                    assert_eq!(calls, 19, "19 singleton calls before the verdict");
                    break;
                }
                other => panic!("round {calls}: unexpected decision {other:?}"),
            }
        }
    }

    #[test]
    fn scripted_records_reserved_call_id_not_counter_reload() {
        // The regression behind the `request_index` parameter: the recorded
        // session call id must be the one the response actually carries
        // (`call-stress-<request_index>`), never a reload of the shared
        // counter. Under concurrent connections another request can
        // increment the counter between the reservation in
        // `handle_mock_completion` and a reload, which would record an id
        // that differs from the wire id and break the following request's
        // exact `tool_call_id` matching. Deterministic without threads: set
        // the counter to a value that differs from the reserved index and
        // prove the recorded id follows the index, while the counter is
        // never read by the decision path.
        let state = mock_state(Some(ScriptKey::MemoryRoundtrip));
        state.counter.store(5, Ordering::Relaxed);
        let marker = scripted::marker_message(ScriptKey::MemoryRoundtrip, "u0", "1", 4096);
        let request = json!({
            "stream": false,
            "messages": [{"role": "user", "content": marker}],
            "tools": [
                {"type": "function", "function": {"name": "ironclaw__memory__write"}},
                {"type": "function", "function": {"name": "ironclaw__memory__read"}},
            ]
        });
        let decision = scripted_decision_for(&state, &request, 42);
        assert!(
            matches!(decision, ScriptedDecision::ToolCalls(_)),
            "the first write must be emitted"
        );
        let driver = state
            .scripted_driver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let parsed = scripted::ScriptedOp {
            key: ScriptKey::MemoryRoundtrip,
            user: "u0".to_string(),
            op: "1".to_string(),
            size_bytes: 4096,
        };
        assert_eq!(
            driver.last_call_id_for(&parsed),
            Some("call-stress-42"),
            "the recorded id is the reserved request_index's id, not a counter reload"
        );
        assert_eq!(
            state.counter.load(Ordering::Relaxed),
            5,
            "the decision path never reloads the shared counter"
        );
    }

    #[test]
    fn mock_serializes_memory_search_tool_call() {
        let call = scripted::ToolCallSpec {
            wire_name: "ironclaw__memory__search".to_string(),
            arguments: json!({
                "query": scripted::MEMORY_SEARCH_QUERY,
                "limit": scripted::MEMORY_SEARCH_LIMIT,
            }),
        };
        // Non-streaming shape: the arguments serialize as a JSON string the
        // rig client parses back into the query/limit pair.
        let response = mock_tool_call_response("stress-mock", 7, std::slice::from_ref(&call));
        let parsed: rig::providers::openai::completion::CompletionResponse =
            serde_json::from_value(response)
                .expect("search call should deserialize as rig-core OpenAI completion");
        let message = &parsed.choices[0].message;
        let tool_calls = match message {
            rig::providers::openai::Message::Assistant { tool_calls, .. } => tool_calls,
            other => panic!("expected assistant message, got {other:?}"),
        };
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "ironclaw__memory__search");
        assert_eq!(
            tool_calls[0].function.arguments,
            json!({
                "query": scripted::MEMORY_SEARCH_QUERY,
                "limit": scripted::MEMORY_SEARCH_LIMIT,
            })
        );
        // Streaming shape: the arguments chunk carries the same payload.
        let (header, body, done) = mock_streaming_tool_call_chunks("stress-mock", 7, &[call]);
        let header_call = &header["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(header_call["index"], 0);
        assert_eq!(header_call["function"]["name"], "ironclaw__memory__search");
        let body_call = &body["choices"][0]["delta"]["tool_calls"][0];
        let arguments: Value =
            serde_json::from_str(body_call["function"]["arguments"].as_str().expect("string"))
                .expect("arguments json");
        assert_eq!(arguments["query"], scripted::MEMORY_SEARCH_QUERY);
        assert_eq!(arguments["limit"], scripted::MEMORY_SEARCH_LIMIT);
        assert_eq!(done["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn read_worker_interval_clamps_invalid_or_huge_values() {
        assert_eq!(
            read_worker_interval_for(f64::NAN, 10, 4),
            MAX_READ_WORKER_SLEEP
        );
        assert_eq!(
            read_worker_interval_for(f64::MIN_POSITIVE, 10, 4),
            MAX_READ_WORKER_SLEEP
        );
        assert_eq!(
            read_worker_interval_for(f64::INFINITY, 10, 4),
            MAX_READ_WORKER_SLEEP
        );
    }

    #[test]
    fn fixed_api_window_processes_all_users_but_caps_active_writers() {
        let args = parsed_api_args(&["--users", "100", "--concurrency", "1", "--operations", "1"]);
        let writer_limit = writer_concurrency_limit(&args, args.users);

        assert_eq!(writer_limit, 1);
        assert_eq!(
            writer_user_count_for_window(&args, args.users, writer_limit),
            100
        );
    }

    #[test]
    fn duration_api_window_uses_active_writer_set_only() {
        let args = parsed_api_args(&[
            "--users",
            "100",
            "--concurrency",
            "10",
            "--duration-seconds",
            "60",
        ]);
        let writer_limit = writer_concurrency_limit(&args, args.users);

        assert_eq!(writer_limit, 10);
        assert_eq!(
            writer_user_count_for_window(&args, args.users, writer_limit),
            10
        );
    }

    #[test]
    fn background_concurrency_auto_caps_at_trigger_default_shape() {
        let args = parsed_api_args(&["--api-background-users", "100"]);

        assert_eq!(background_concurrency_limit(&args, 100), 8);
        assert_eq!(background_concurrency_limit(&args, 4), 4);
        assert_eq!(background_concurrency_limit(&args, 0), 0);
    }

    #[test]
    fn mock_request_classifier_detects_background_marker() {
        assert_eq!(
            classify_mock_request(br#"{"messages":[{"content":"hello"}]}"#),
            MockRequestKind::Foreground
        );
        assert_eq!(
            classify_mock_request(
                br#"{"messages":[{"content":"ironclaw-stress-background long task"}]}"#
            ),
            MockRequestKind::Background
        );
    }

    #[test]
    fn timeline_counts_tool_results_after_baseline_sequence() {
        let value = json!({
            "messages": [
                {"kind": "user", "status": "finalized", "sequence": 1},
                {"kind": "assistant", "status": "finalized", "content": "ok", "sequence": 2},
                {"kind": "tool_result_reference", "tool_result_ref": "r1", "sequence": 3},
                {"kind": "tool_result_reference", "tool_result_ref": "r2", "sequence": 4},
                {"kind": "tool_result_reference", "tool_result_ref": "r3", "sequence": 7}
            ]
        });
        assert_eq!(timeline_tool_results_after(&value, 2), 3);
        assert_eq!(timeline_tool_results_after(&value, 4), 1);
        assert_eq!(timeline_tool_results_after(&value, 7), 0);
        assert_eq!(timeline_max_sequence(&value), 7);
        assert_eq!(timeline_tool_results_after(&json!({"messages": []}), 0), 0);
    }

    #[test]
    fn timeline_finds_finalized_verdict_content() {
        let value = json!({
            "messages": [
                {"kind": "assistant", "status": "submitted", "content": "working"},
                {"kind": "assistant", "status": "finalized",
                 "content": "ironclaw-stress-tool result u0__1 confirmed"}
            ]
        });
        assert_eq!(
            timeline_finalized_verdict_content(&value, "ironclaw-stress-tool result u0__1")
                .as_deref(),
            Some("ironclaw-stress-tool result u0__1 confirmed")
        );
        assert_eq!(
            timeline_finalized_verdict_content(&value, "ironclaw-stress-tool result u0__2"),
            None
        );
        // `u0__1` must not match `u0__10`: the prefix is delimited by the
        // following space so op 1 cannot terminate on op 10's message.
        let longer_op = json!({
            "messages": [
                {"kind": "assistant", "status": "finalized",
                 "content": "ironclaw-stress-tool result u0__10 confirmed"}
            ]
        });
        assert_eq!(
            timeline_finalized_verdict_content(&longer_op, "ironclaw-stress-tool result u0__1"),
            None
        );
        assert_eq!(
            timeline_finalized_verdict_content(&longer_op, "ironclaw-stress-tool result u0__10")
                .as_deref(),
            Some("ironclaw-stress-tool result u0__10 confirmed")
        );
    }

    #[test]
    fn timeline_detects_placeholder_text() {
        let value = json!({
            "messages": [
                {"kind": "assistant", "status": "finalized",
                 "content": "ironclaw-stress-tool pending \u{2014} I'll perform the stress tool action next."}
            ]
        });
        assert!(timeline_has_placeholder(&value));
        let normal = json!({
            "messages": [
                {"kind": "assistant", "status": "finalized", "content": "hello"}
            ]
        });
        assert!(!timeline_has_placeholder(&normal));
    }

    #[test]
    fn scripted_summary_buckets_by_size_and_verdict() {
        let mut args = parsed_api_args(&[]);
        args.api_scripted_tool = Some(scripted::ScriptKey::MemoryRoundtrip);
        args.api_scripted_doc_sizes = vec![4096, 32768];
        let samples = vec![
            ScriptedOpSample {
                size_bytes: 4096,
                verdict: Some(scripted::Verdict::Confirmed),
                tools_executed: 2,
                tool_visible_latency: Some(Duration::from_millis(100)),
                finalize_latency: Some(Duration::from_millis(500)),
                failure: None,
            },
            ScriptedOpSample {
                size_bytes: 4096,
                verdict: None,
                tools_executed: 0,
                tool_visible_latency: None,
                finalize_latency: None,
                failure: Some(FailureCause::new(
                    "scripted_verdict_leak",
                    "wait_for_assistant",
                    "leak detected",
                )),
            },
            ScriptedOpSample {
                size_bytes: 32768,
                verdict: Some(scripted::Verdict::Contended),
                tools_executed: 2,
                tool_visible_latency: Some(Duration::from_millis(90)),
                finalize_latency: Some(Duration::from_millis(450)),
                failure: None,
            },
        ];
        let summary = summarize_scripted(&samples, &args).expect("scripted summary present");

        assert_eq!(summary.script, "memory_roundtrip");
        assert_eq!(summary.doc_sizes, vec![4096, 32768]);
        let four_kb = &summary.size_buckets["4096"];
        assert_eq!(four_kb.attempted, 2);
        assert_eq!(four_kb.succeeded, 1);
        assert_eq!(four_kb.failed, 1);
        assert_eq!(four_kb.verdicts["confirmed"], 1);
        assert_eq!(four_kb.failure_buckets["scripted_verdict_leak"], 1);
        assert_eq!(four_kb.tool_results, 2);
        assert_eq!(four_kb.stage_latency_us["finalize"].p50_us, 500_000);
        let thirty_two_kb = &summary.size_buckets["32768"];
        assert_eq!(thirty_two_kb.verdicts["contended"], 1);
    }

    #[test]
    fn scripted_summary_absent_without_script() {
        let args = parsed_api_args(&[]);
        assert!(summarize_scripted(&[], &args).is_none());
        let mut scripted = parsed_api_args(&[]);
        scripted.api_scripted_tool = Some(scripted::ScriptKey::MemoryRoundtrip);
        assert!(summarize_scripted(&[], &scripted).is_none());
    }
}
