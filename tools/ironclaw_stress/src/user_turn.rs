use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Utc;
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    CapabilityId, HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
    ResourceReservation, ResourceReservationId, ResourceScope, ThreadId, VirtualPath,
};
use ironclaw_llm::{
    ChatMessage, CompletionRequest, LlmError, LlmProvider, SessionManager,
    build_static_provider_chain, resolve_llm_config_from_env,
};
use ironclaw_resources::{ResourceError, ResourceGovernor};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AppendFinalizedAssistantMessageRequest,
    AppendToolResultReferenceRequest, CapabilityDisplayPreviewEnvelope,
    CapabilityDisplayPreviewEnvelopeInput, CapabilityDisplayPreviewStatus, EnsureThreadRequest,
    FilesystemSessionThreadService, ListThreadsForScopeRequest, LoadContextWindowRequest,
    MessageContent, SessionThreadError, SessionThreadService, ToolResultSafeSummary,
    UpdateAssistantDraftRequest,
};
use ironclaw_turns::{
    AcceptedMessageRef, BlockedReason, DefaultTurnCoordinator, FilesystemTurnStateBlockPersistence,
    FilesystemTurnStateRowStore, FilesystemTurnStateStore, GateRef, IdempotencyKey,
    InMemoryTurnStateStore, InMemoryTurnStateStoreLimits, LoopCheckpointStateRef,
    ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnCoordinator, TurnError,
    TurnErrorCategory, TurnLeaseToken, TurnRunnerId, TurnStateStore,
    runner::{
        BlockRunRequest, ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, TurnRunTransitionPort,
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::time::sleep;

use crate::{
    Args, Backend, LatencySummary, ModelLatencyProfile, ModelLatencySource, OperationTarget,
    Sample, Scenario, TurnStateBackend,
    progress::{ProgressCounters, spawn_progress_reporter, stop_progress_reporter},
    resource_ops,
    summary::{FailureCause, latency_summary},
    synthetic::SyntheticIds,
    trace::{spawn_trace_reporter, stop_trace_reporter},
};

/// Backend-agnostic turn store for the stress workload. Both the durable
/// `FilesystemTurnStateStore` and the shared `InMemoryTurnStateStore` already
/// implement the supertraits, so the impls are empty — this lets
/// `turn_store_for_context` return one `Arc<dyn StressTurnStore>` and the
/// workload submit/claim/complete against either without per-method dispatch.
pub(crate) trait StressTurnStore: TurnStateStore + TurnRunTransitionPort {}
impl<F: RootFilesystem + 'static> StressTurnStore for FilesystemTurnStateStore<F> {}
impl<F: RootFilesystem + 'static> StressTurnStore for FilesystemTurnStateRowStore<F> {}
impl StressTurnStore for InMemoryTurnStateStore {}

pub(crate) struct UserTurnServices<F>
where
    F: RootFilesystem,
{
    root: Arc<F>,
    governor: Arc<dyn ResourceGovernor>,
    thread_service: Arc<FilesystemSessionThreadService<F>>,
    model_latency: Arc<ModelLatencyDriver>,
    run_id: String,
    target: String,
    turn_state_backend: TurnStateBackend,
    turn_state_limits: InMemoryTurnStateStoreLimits,
    /// Single shared in-process turn-state authority, used when
    /// `turn_state_backend == Memory`. Shared across all workers (one process)
    /// to faithfully model the production single-process design.
    memory_turn_store: Arc<InMemoryTurnStateStore>,
    /// Shared row-store authorities keyed by tenant/user mount. This preserves
    /// durable filesystem writes while avoiding a full row-set reload for every
    /// measured operation in the same process.
    row_turn_stores: Mutex<RowTurnStoreCache>,
    /// Shared in-memory `RootFilesystem` backing the `RowMemory` variant's row
    /// stores — one in-process backend for the whole run, so the row store's
    /// journal/delta mechanism is measured without durable-backend cost.
    memory_root: Arc<InMemoryBackend>,
}

pub(crate) enum UserTurnWorkload {
    #[cfg(feature = "libsql")]
    Libsql(UserTurnServices<ironclaw_filesystem::LibSqlRootFilesystem>),
    #[cfg(feature = "postgres")]
    Postgres(UserTurnServices<ironclaw_filesystem::PostgresRootFilesystem>),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StageLatencySummary {
    pub(crate) count: u64,
    pub(crate) latency: LatencySummary,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct UserTurnStageLatencySummary {
    pub(crate) ensure_thread: StageLatencySummary,
    pub(crate) accept_inbound: StageLatencySummary,
    pub(crate) submit_turn: StageLatencySummary,
    pub(crate) mark_submitted: StageLatencySummary,
    pub(crate) mark_rejected_busy: StageLatencySummary,
    pub(crate) list_threads_cold: StageLatencySummary,
    pub(crate) list_threads_warm: StageLatencySummary,
    pub(crate) claim_run: StageLatencySummary,
    pub(crate) block_run: StageLatencySummary,
    pub(crate) resume_turn: StageLatencySummary,
    pub(crate) reclaim_run: StageLatencySummary,
    pub(crate) append_assistant: StageLatencySummary,
    pub(crate) finalize_assistant: StageLatencySummary,
    pub(crate) complete_run: StageLatencySummary,
    pub(crate) load_context: StageLatencySummary,
    pub(crate) resource_reserve: StageLatencySummary,
    pub(crate) model_wait: StageLatencySummary,
    pub(crate) tool_wait: StageLatencySummary,
    pub(crate) append_tool_result: StageLatencySummary,
    pub(crate) append_tool_preview: StageLatencySummary,
    pub(crate) update_assistant_draft: StageLatencySummary,
    pub(crate) resource_reconcile: StageLatencySummary,
    pub(crate) resource_release: StageLatencySummary,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OperationAttributionSummary {
    pub(crate) count: u64,
    pub(crate) latency: LatencySummary,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct UserTurnOperationAttributionSummary {
    pub(crate) thread_store_writes: OperationAttributionSummary,
    pub(crate) context_reads: OperationAttributionSummary,
    pub(crate) turn_store: OperationAttributionSummary,
    pub(crate) resource_governor: OperationAttributionSummary,
    pub(crate) synthetic_wait: OperationAttributionSummary,
}

pub(crate) fn operation_attribution_rows(
    attribution: &UserTurnOperationAttributionSummary,
) -> [(&'static str, &OperationAttributionSummary); 5] {
    [
        ("thread_store_writes", &attribution.thread_store_writes),
        ("context_reads", &attribution.context_reads),
        ("turn_store", &attribution.turn_store),
        ("resource_governor", &attribution.resource_governor),
        ("model_tool_wait", &attribution.synthetic_wait),
    ]
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UserTurnStageDurations {
    pub(crate) ensure_thread: Option<Duration>,
    pub(crate) accept_inbound: Option<Duration>,
    pub(crate) submit_turn: Option<Duration>,
    pub(crate) mark_submitted: Option<Duration>,
    pub(crate) mark_rejected_busy: Option<Duration>,
    pub(crate) list_threads_cold: Option<Duration>,
    pub(crate) list_threads_warm: Option<Duration>,
    pub(crate) claim_run: Option<Duration>,
    pub(crate) block_run: Option<Duration>,
    pub(crate) resume_turn: Option<Duration>,
    pub(crate) reclaim_run: Option<Duration>,
    pub(crate) append_assistant: Option<Duration>,
    pub(crate) finalize_assistant: Option<Duration>,
    pub(crate) complete_run: Option<Duration>,
    pub(crate) load_context: Option<Duration>,
    pub(crate) resource_reserve: Option<Duration>,
    pub(crate) model_wait: Option<Duration>,
    pub(crate) tool_wait: Option<Duration>,
    pub(crate) append_tool_result: Option<Duration>,
    pub(crate) append_tool_preview: Option<Duration>,
    pub(crate) update_assistant_draft: Option<Duration>,
    pub(crate) resource_reconcile: Option<Duration>,
    pub(crate) resource_release: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PrefillSummary {
    pub(crate) threads: usize,
    pub(crate) turns_per_thread: usize,
    pub(crate) concurrency: usize,
    pub(crate) attempted: u64,
    pub(crate) succeeded: u64,
    pub(crate) failed: u64,
    pub(crate) duration_ms: u128,
    pub(crate) throughput_ops_sec: f64,
    pub(crate) latency: LatencySummary,
    pub(crate) errors: BTreeMap<String, u64>,
}

pub(crate) async fn build_user_turn_workload(
    args: &Args,
    run_id: &str,
) -> Result<UserTurnWorkload, String> {
    match args.backend {
        Backend::Libsql => build_libsql_user_turn_workload(args, run_id).await,
        Backend::Postgres => build_postgres_user_turn_workload(args, run_id).await,
    }
}

#[cfg(feature = "libsql")]
async fn build_libsql_user_turn_workload(
    args: &Args,
    run_id: &str,
) -> Result<UserTurnWorkload, String> {
    let (filesystem, target) = crate::build_libsql_root(args).await?;
    let governor = crate::governor_from_root(Arc::clone(&filesystem), run_id)?;
    let model_latency = build_model_latency_driver(args).await?;
    Ok(UserTurnWorkload::Libsql(user_turn_services_from_root(
        filesystem,
        governor,
        run_id,
        target,
        model_latency,
        args.turn_state_backend,
        args.turn_state_store_limits(),
    )?))
}

#[cfg(not(feature = "libsql"))]
async fn build_libsql_user_turn_workload(
    _args: &Args,
    _run_id: &str,
) -> Result<UserTurnWorkload, String> {
    Err("binary was built without the libsql feature".to_string())
}

#[cfg(feature = "postgres")]
async fn build_postgres_user_turn_workload(
    args: &Args,
    run_id: &str,
) -> Result<UserTurnWorkload, String> {
    let (filesystem, _pool, target) = crate::build_postgres_root_and_pool(args).await?;
    let governor = crate::governor_from_root(Arc::clone(&filesystem), run_id)?;
    let model_latency = build_model_latency_driver(args).await?;
    Ok(UserTurnWorkload::Postgres(user_turn_services_from_root(
        filesystem,
        governor,
        run_id,
        target,
        model_latency,
        args.turn_state_backend,
        args.turn_state_store_limits(),
    )?))
}

#[cfg(not(feature = "postgres"))]
async fn build_postgres_user_turn_workload(
    _args: &Args,
    _run_id: &str,
) -> Result<UserTurnWorkload, String> {
    Err("binary was built without the postgres feature".to_string())
}

pub(crate) async fn run_user_turn_tasks(
    workload: Arc<UserTurnWorkload>,
    args: &Args,
    identities: Arc<SyntheticIds>,
) -> Result<Vec<Sample>, String> {
    let operation_target = args.operation_target();
    let progress = Arc::new(ProgressCounters::new(args.trace_jsonl.is_some()));
    let span_budget = Arc::new(AtomicUsize::new(span_sample_limit(args.span_sample_limit)));
    let progress_reporter = spawn_progress_reporter(
        crate::log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        args.progress_interval_seconds,
        operation_target.progress_total(),
        Arc::clone(&progress),
    );
    let trace_reporter = spawn_trace_reporter(args, workload.target(), Arc::clone(&progress));

    let mut handles = Vec::with_capacity(args.concurrency);
    for worker_index in 0..args.concurrency {
        let workload = Arc::clone(&workload);
        let identities = Arc::clone(&identities);
        let progress = Arc::clone(&progress);
        let span_budget = Arc::clone(&span_budget);
        let args = args.clone();
        handles.push((
            worker_index,
            tokio::spawn(async move {
                let mut samples = Vec::with_capacity(args.initial_worker_sample_capacity());
                let started = Instant::now();
                let mut operation_index = 0;
                while should_run_operation(args.operation_target(), started, operation_index) {
                    let sample = workload
                        .run_operation(
                            &args,
                            &identities,
                            worker_index,
                            operation_index,
                            &span_budget,
                        )
                        .await;
                    progress.record(sample.error.is_some(), sample.latency);
                    samples.push(sample);
                    operation_index += 1;
                }
                samples
            }),
        ));
    }

    let mut samples = Vec::with_capacity(operation_target.progress_total().unwrap_or_else(|| {
        args.concurrency
            .saturating_mul(args.initial_worker_sample_capacity())
    }));
    let mut first_error = None;
    for (worker_index, handle) in handles {
        match handle.await {
            Ok(worker_samples) => samples.extend(worker_samples),
            Err(error) => {
                first_error.get_or_insert_with(|| {
                    if error.is_panic() {
                        eprintln!("user-turn worker {worker_index} panicked: {error:?}");
                        format!("user-turn worker {worker_index} panicked")
                    } else {
                        eprintln!("user-turn worker {worker_index} cancelled: {error:?}");
                        format!("user-turn worker {worker_index} cancelled")
                    }
                });
            }
        }
    }
    stop_trace_reporter(trace_reporter);
    stop_progress_reporter(progress_reporter);

    if let Some(error) = first_error {
        return Err(error);
    }
    if let Some(expected) = operation_target.progress_total()
        && samples.len() != expected
    {
        return Err(format!(
            "collected {} samples but expected {expected}",
            samples.len()
        ));
    }
    Ok(samples)
}

pub(crate) async fn prefill_user_turn_history(
    workload: Arc<UserTurnWorkload>,
    args: &Args,
    identities: Arc<SyntheticIds>,
) -> Result<Option<PrefillSummary>, String> {
    if !args.prefill_enabled() {
        return Ok(None);
    }

    let total_turns = args
        .prefill_threads
        .saturating_mul(args.prefill_turns_per_thread);
    eprintln!(
        "{} prefill starting target={} threads={} turns_per_thread={} total_turns={} concurrency={}",
        crate::log_prefix(args),
        workload.target(),
        args.prefill_threads,
        args.prefill_turns_per_thread,
        total_turns,
        args.prefill_concurrency
    );

    let started = Instant::now();
    let semaphore = Arc::new(Semaphore::new(args.prefill_concurrency));
    let mut handles = Vec::with_capacity(args.prefill_threads);
    for thread_index in 0..args.prefill_threads {
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .map_err(|_| "prefill semaphore closed".to_string())?;
        let workload = Arc::clone(&workload);
        let identities = Arc::clone(&identities);
        let args = args.clone();
        handles.push((
            thread_index,
            tokio::spawn(async move {
                let _permit = permit;
                let mut samples = Vec::with_capacity(args.prefill_turns_per_thread);
                for turn_index in 0..args.prefill_turns_per_thread {
                    samples.push(
                        workload
                            .prefill_turn(&args, &identities, thread_index, turn_index)
                            .await,
                    );
                }
                samples
            }),
        ));
    }

    let mut samples = Vec::with_capacity(total_turns);
    let mut first_error = None;
    for (thread_index, handle) in handles {
        match handle.await {
            Ok(thread_samples) => samples.extend(thread_samples),
            Err(error) => {
                first_error.get_or_insert_with(|| {
                    if error.is_panic() {
                        eprintln!("prefill thread {thread_index} panicked: {error:?}");
                        format!("prefill thread {thread_index} panicked")
                    } else {
                        eprintln!("prefill thread {thread_index} cancelled: {error:?}");
                        format!("prefill thread {thread_index} cancelled")
                    }
                });
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }

    let summary = summarize_prefill(args, started.elapsed(), &samples);
    eprintln!(
        "{} prefill finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        crate::log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    if summary.failed > 0 {
        return Err(format!(
            "prefill failed attempted={} failed={} errors={}",
            summary.attempted,
            summary.failed,
            format_prefill_errors(&summary.errors)
        ));
    }

    Ok(Some(summary))
}

pub(crate) async fn prefill_thread_list(
    workload: Arc<UserTurnWorkload>,
    args: &Args,
    identities: Arc<SyntheticIds>,
) -> Result<Option<PrefillSummary>, String> {
    if !matches!(args.scenario, Scenario::ThreadList) {
        return Ok(None);
    }

    eprintln!(
        "{} thread-list prefill starting target={} threads={} concurrency={}",
        crate::log_prefix(args),
        workload.target(),
        args.thread_list_threads,
        args.prefill_concurrency
    );

    let started = Instant::now();
    let semaphore = Arc::new(Semaphore::new(args.prefill_concurrency));
    let mut handles = Vec::with_capacity(args.thread_list_threads);
    for thread_index in 0..args.thread_list_threads {
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .map_err(|_| "thread-list prefill semaphore closed".to_string())?;
        let workload = Arc::clone(&workload);
        let identities = Arc::clone(&identities);
        let args = args.clone();
        handles.push((
            thread_index,
            tokio::spawn(async move {
                let _permit = permit;
                workload
                    .prefill_thread_list_thread(&args, &identities, thread_index)
                    .await
            }),
        ));
    }

    let mut samples = Vec::with_capacity(args.thread_list_threads);
    let mut first_error = None;
    for (thread_index, handle) in handles {
        match handle.await {
            Ok(sample) => samples.push(sample),
            Err(error) => {
                first_error.get_or_insert_with(|| {
                    if error.is_panic() {
                        eprintln!("thread-list prefill thread {thread_index} panicked: {error:?}");
                        format!("thread-list prefill thread {thread_index} panicked")
                    } else {
                        eprintln!("thread-list prefill thread {thread_index} cancelled: {error:?}");
                        format!("thread-list prefill thread {thread_index} cancelled")
                    }
                });
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }

    let summary = summarize_thread_list_prefill(args, started.elapsed(), &samples);
    eprintln!(
        "{} thread-list prefill finished attempted={} succeeded={} failed={} duration_ms={} throughput_ops_sec={:.1}",
        crate::log_prefix(args),
        summary.attempted,
        summary.succeeded,
        summary.failed,
        summary.duration_ms,
        summary.throughput_ops_sec
    );
    if summary.failed > 0 {
        return Err(format!(
            "thread-list prefill failed attempted={} failed={} errors={}",
            summary.attempted,
            summary.failed,
            format_prefill_errors(&summary.errors)
        ));
    }

    Ok(Some(summary))
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

fn summarize_prefill(args: &Args, elapsed: Duration, samples: &[Sample]) -> PrefillSummary {
    let mut errors = BTreeMap::new();
    let mut latencies: Vec<u128> = samples
        .iter()
        .map(|sample| sample.latency.as_micros())
        .collect();
    latencies.sort_unstable();
    let failed = samples
        .iter()
        .filter_map(|sample| sample.error.as_ref())
        .map(|error| {
            *errors.entry(error.clone()).or_insert(0) += 1;
        })
        .count() as u64;
    let attempted = samples.len() as u64;
    let succeeded = attempted.saturating_sub(failed);
    let elapsed_secs = elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
    PrefillSummary {
        threads: args.prefill_threads,
        turns_per_thread: args.prefill_turns_per_thread,
        concurrency: args.prefill_concurrency,
        attempted,
        succeeded,
        failed,
        duration_ms: elapsed.as_millis(),
        throughput_ops_sec: attempted as f64 / elapsed_secs,
        latency: latency_summary(&latencies),
        errors,
    }
}

fn summarize_thread_list_prefill(
    args: &Args,
    elapsed: Duration,
    samples: &[Sample],
) -> PrefillSummary {
    let mut summary = summarize_prefill(args, elapsed, samples);
    summary.threads = args.thread_list_threads;
    summary.turns_per_thread = 0;
    summary
}

fn format_prefill_errors(errors: &BTreeMap<String, u64>) -> String {
    if errors.is_empty() {
        return "-".to_string();
    }
    errors
        .iter()
        .map(|(error, count)| format!("{error}={count}"))
        .collect::<Vec<_>>()
        .join(",")
}

impl UserTurnWorkload {
    pub(crate) fn target(&self) -> &str {
        match self {
            #[cfg(feature = "libsql")]
            Self::Libsql(services) => &services.target,
            #[cfg(feature = "postgres")]
            Self::Postgres(services) => &services.target,
        }
    }

    async fn run_operation(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        worker_index: usize,
        operation_index: usize,
        span_budget: &AtomicUsize,
    ) -> Sample {
        match self {
            #[cfg(feature = "libsql")]
            Self::Libsql(services) => {
                services
                    .run_operation(args, identities, worker_index, operation_index, span_budget)
                    .await
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(services) => {
                services
                    .run_operation(args, identities, worker_index, operation_index, span_budget)
                    .await
            }
        }
    }

    async fn prefill_turn(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        thread_index: usize,
        turn_index: usize,
    ) -> Sample {
        match self {
            #[cfg(feature = "libsql")]
            Self::Libsql(services) => {
                services
                    .prefill_turn(args, identities, thread_index, turn_index)
                    .await
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(services) => {
                services
                    .prefill_turn(args, identities, thread_index, turn_index)
                    .await
            }
        }
    }

    async fn prefill_thread_list_thread(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        thread_index: usize,
    ) -> Sample {
        match self {
            #[cfg(feature = "libsql")]
            Self::Libsql(services) => {
                services
                    .prefill_thread_list_thread(args, identities, thread_index)
                    .await
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(services) => {
                services
                    .prefill_thread_list_thread(args, identities, thread_index)
                    .await
            }
        }
    }
}

impl<F> UserTurnServices<F>
where
    F: RootFilesystem + 'static,
{
    async fn prefill_turn(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        thread_index: usize,
        turn_index: usize,
    ) -> Sample {
        let mut stages = UserTurnStageDurations::default();
        let started = Instant::now();
        let outcome = self
            .prefill_turn_inner(args, identities, thread_index, turn_index, &mut stages)
            .await;
        let latency = started.elapsed();
        let failure = outcome.err().map(|failure| failure.cause);
        let error = failure.as_ref().map(|cause| cause.bucket.clone());
        Sample {
            latency,
            error,
            failure,
            stages: Some(stages),
        }
    }

    async fn prefill_turn_inner(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        thread_index: usize,
        turn_index: usize,
        stages: &mut UserTurnStageDurations,
    ) -> Result<(), OperationFailure> {
        let context = identities
            .user_turn_context_for_user_index(thread_index)
            .map_err(|error| OperationFailure::invalid_request("prefill_context", error))?;
        let turn_store = self.turn_store_for_context(&context)?;
        let turn_coordinator = DefaultTurnCoordinator::new(Arc::clone(&turn_store));
        let source_binding = "ironclaw-stress-prefill";
        let reply_target = "ironclaw-stress-prefill-reply";
        let operation_ref = format!("prefill:{}:{thread_index}:{turn_index}", self.run_id);
        let user_message = stress_payload(
            format!("prefill stress message {operation_ref}"),
            args.user_message_bytes,
        );
        let assistant_message = stress_payload(
            format!("prefill stress response {operation_ref}"),
            args.assistant_message_bytes,
        );

        let thread = time_stage(
            &mut stages.ensure_thread,
            self.thread_service.ensure_thread(EnsureThreadRequest {
                scope: context.thread_scope.clone(),
                thread_id: Some(context.thread_id.clone()),
                created_by_actor_id: context.user_id.as_str().to_string(),
                title: Some(format!(
                    "Storage stress {}",
                    context.thread_owner_user_id.as_str()
                )),
                metadata_json: None,
            }),
        )
        .await
        .map_err(|error| thread_failure("prefill_ensure_thread", error))?;

        let accepted = time_stage(
            &mut stages.accept_inbound,
            self.thread_service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope: context.thread_scope.clone(),
                    thread_id: thread.thread_id.clone(),
                    actor_id: context.user_id.as_str().to_string(),
                    source_binding_id: Some(source_binding.to_string()),
                    reply_target_binding_id: Some(reply_target.to_string()),
                    external_event_id: Some(operation_ref.clone()),
                    content: MessageContent::text(user_message),
                }),
        )
        .await
        .map_err(|error| thread_failure("prefill_accept_inbound", error))?;

        let SubmitTurnResponse::Accepted {
            turn_id, run_id, ..
        } = time_stage(
            &mut stages.submit_turn,
            turn_coordinator.submit_turn(SubmitTurnRequest {
                scope: context.turn_scope.clone(),
                actor: TurnActor::new(context.user_id.clone()),
                accepted_message_ref: AcceptedMessageRef::new(accepted.message_id.to_string())
                    .map_err(|error| OperationFailure::invalid_request("prefill_submit", error))?,
                source_binding_ref: SourceBindingRef::new(source_binding)
                    .map_err(|error| OperationFailure::invalid_request("prefill_submit", error))?,
                reply_target_binding_ref: ReplyTargetBindingRef::new(reply_target)
                    .map_err(|error| OperationFailure::invalid_request("prefill_submit", error))?,
                requested_run_profile: None,
                requested_model: None,
                idempotency_key: IdempotencyKey::new(format!(
                    "ironclaw-stress-prefill:{operation_ref}"
                ))
                .map_err(|error| OperationFailure::invalid_request("prefill_submit", error))?,
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            }),
        )
        .await
        .map_err(|error| turn_failure("prefill_submit", error))?;

        time_stage(
            &mut stages.mark_submitted,
            self.thread_service.mark_message_submitted(
                &context.thread_scope,
                &thread.thread_id,
                accepted.message_id,
                turn_id.to_string(),
                run_id.to_string(),
            ),
        )
        .await
        .map_err(|error| thread_failure("prefill_mark_submitted", error))?;

        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        let claimed = time_stage(
            &mut stages.claim_run,
            turn_store.claim_next_run(ClaimRunRequest {
                runner_id,
                lease_token,
                scope_filter: Some(context.turn_scope.clone()),
            }),
        )
        .await
        .map_err(|error| turn_failure("prefill_claim_run", error))?
        .ok_or_else(|| {
            OperationFailure::new(
                "turn_claim_miss",
                "prefill_claim_run",
                "prefill run was not claimable",
            )
        })?;

        self.write_assistant_turn(
            &context,
            &thread.thread_id,
            &turn_store,
            &claimed,
            assistant_message,
            stages,
        )
        .await
    }

    async fn prefill_thread_list_thread(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        thread_index: usize,
    ) -> Sample {
        let mut stages = UserTurnStageDurations::default();
        let started = Instant::now();
        let outcome = self
            .prefill_thread_list_thread_inner(args, identities, thread_index, &mut stages)
            .await;
        let latency = started.elapsed();
        let failure = outcome.err().map(|failure| failure.cause);
        let error = failure.as_ref().map(|cause| cause.bucket.clone());
        Sample {
            latency,
            error,
            failure,
            stages: Some(stages),
        }
    }

    async fn prefill_thread_list_thread_inner(
        &self,
        _args: &Args,
        identities: &SyntheticIds,
        thread_index: usize,
        stages: &mut UserTurnStageDurations,
    ) -> Result<(), OperationFailure> {
        let context = identities
            .user_turn_context_for_user_index(0)
            .map_err(|error| OperationFailure::invalid_request("thread_list_context", error))?;
        let thread_id = thread_list_thread_id(thread_index)?;
        time_stage(
            &mut stages.ensure_thread,
            self.thread_service.ensure_thread(EnsureThreadRequest {
                scope: context.thread_scope,
                thread_id: Some(thread_id),
                created_by_actor_id: context.user_id.as_str().to_string(),
                title: Some(format!("Thread list seed {thread_index:06}")),
                metadata_json: None,
            }),
        )
        .await
        .map(|_| ())
        .map_err(|error| thread_failure("thread_list_prefill_ensure_thread", error))
    }

    async fn run_operation(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        worker_index: usize,
        operation_index: usize,
        span_budget: &AtomicUsize,
    ) -> Sample {
        let mut stages = UserTurnStageDurations::default();
        let started = Instant::now();
        let outcome = self
            .run_operation_inner(args, identities, worker_index, operation_index, &mut stages)
            .await;
        let latency = started.elapsed();
        let failure = outcome.err().map(|failure| failure.cause);
        let error = failure.as_ref().map(|cause| cause.bucket.clone());
        maybe_emit_operation_span(
            args,
            worker_index,
            operation_index,
            latency,
            &stages,
            failure.as_ref(),
            span_budget,
        );
        Sample {
            latency,
            error,
            failure,
            stages: Some(stages),
        }
    }

    async fn run_operation_inner(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        worker_index: usize,
        operation_index: usize,
        stages: &mut UserTurnStageDurations,
    ) -> Result<(), OperationFailure> {
        let context = identities
            .user_turn_context(args, worker_index, operation_index)
            .map_err(|error| OperationFailure::invalid_request("build_context", error))?;

        if matches!(args.scenario, Scenario::ThreadList) {
            return self
                .run_thread_list_operation(args, identities, stages)
                .await;
        }

        let turn_store = self.turn_store_for_context(&context)?;
        let turn_coordinator = DefaultTurnCoordinator::new(Arc::clone(&turn_store));
        let source_binding = "ironclaw-stress-webchat";
        let reply_target = "ironclaw-stress-reply";

        if matches!(args.scenario, Scenario::TurnLifecycleChurn) {
            let operation_ref = turn_operation_ref(args, worker_index, operation_index, 0, 1);
            let SubmitTurnResponse::Accepted { run_id, .. } = time_stage(
                &mut stages.submit_turn,
                turn_coordinator.submit_turn(SubmitTurnRequest {
                    scope: context.turn_scope.clone(),
                    actor: TurnActor::new(context.user_id.clone()),
                    accepted_message_ref: AcceptedMessageRef::new(format!(
                        "message:{operation_ref}"
                    ))
                    .map_err(|error| OperationFailure::invalid_request("submit_turn", error))?,
                    source_binding_ref: SourceBindingRef::new(source_binding)
                        .map_err(|error| OperationFailure::invalid_request("submit_turn", error))?,
                    reply_target_binding_ref: ReplyTargetBindingRef::new(reply_target)
                        .map_err(|error| OperationFailure::invalid_request("submit_turn", error))?,
                    requested_run_profile: None,
                    requested_model: None,
                    idempotency_key: IdempotencyKey::new(format!(
                        "ironclaw-stress:{operation_ref}"
                    ))
                    .map_err(|error| OperationFailure::invalid_request("submit_turn", error))?,
                    received_at: Utc::now(),
                    requested_run_id: None,
                    parent_run_id: None,
                    subagent_depth: 0,
                    spawn_tree_root_run_id: None,
                    product_context: None,
                }),
            )
            .await
            .map_err(|error| turn_failure("submit_turn", error))?;

            let runner_id = TurnRunnerId::new();
            let lease_token = TurnLeaseToken::new();
            time_stage(
                &mut stages.claim_run,
                turn_store.claim_next_run(ClaimRunRequest {
                    runner_id,
                    lease_token,
                    scope_filter: Some(context.turn_scope.clone()),
                }),
            )
            .await
            .map_err(|error| turn_failure("claim_run", error))?
            .ok_or_else(|| {
                OperationFailure::new(
                    "turn_claim_miss",
                    "claim_run",
                    "submitted run was not claimable",
                )
            })?;

            time_stage(
                &mut stages.complete_run,
                turn_store.complete_run(CompleteRunRequest {
                    run_id,
                    runner_id,
                    lease_token,
                }),
            )
            .await
            .map_err(|error| turn_failure("complete_run", error))?;

            return Ok(());
        }

        let thread = time_stage(
            &mut stages.ensure_thread,
            self.thread_service.ensure_thread(EnsureThreadRequest {
                scope: context.thread_scope.clone(),
                thread_id: Some(context.thread_id.clone()),
                created_by_actor_id: context.user_id.as_str().to_string(),
                title: Some(format!(
                    "Storage stress {}",
                    context.thread_owner_user_id.as_str()
                )),
                metadata_json: None,
            }),
        )
        .await
        .map_err(|error| thread_failure("ensure_thread", error))?;

        let turns_per_operation = if matches!(args.scenario, Scenario::ContextGrowth) {
            args.context_growth_turns_per_operation
        } else {
            1
        };

        for turn_index in 0..turns_per_operation {
            let operation_ref = turn_operation_ref(
                args,
                worker_index,
                operation_index,
                turn_index,
                turns_per_operation,
            );
            let user_message = stress_payload(
                format!("stress message {operation_ref}"),
                args.user_message_bytes,
            );
            let assistant_message = stress_payload(
                format!("stress response {operation_ref}"),
                args.assistant_message_bytes,
            );

            let accepted = time_stage(
                &mut stages.accept_inbound,
                self.thread_service
                    .accept_inbound_message(AcceptInboundMessageRequest {
                        scope: context.thread_scope.clone(),
                        thread_id: thread.thread_id.clone(),
                        actor_id: context.user_id.as_str().to_string(),
                        source_binding_id: Some(source_binding.to_string()),
                        reply_target_binding_id: Some(reply_target.to_string()),
                        external_event_id: Some(operation_ref.clone()),
                        content: MessageContent::text(user_message),
                    }),
            )
            .await
            .map_err(|error| thread_failure("accept_inbound", error))?;

            let submit_result = time_stage(
                &mut stages.submit_turn,
                turn_coordinator.submit_turn(SubmitTurnRequest {
                    scope: context.turn_scope.clone(),
                    actor: TurnActor::new(context.user_id.clone()),
                    accepted_message_ref: AcceptedMessageRef::new(accepted.message_id.to_string())
                        .map_err(|error| OperationFailure::invalid_request("submit_turn", error))?,
                    source_binding_ref: SourceBindingRef::new(source_binding)
                        .map_err(|error| OperationFailure::invalid_request("submit_turn", error))?,
                    reply_target_binding_ref: ReplyTargetBindingRef::new(reply_target)
                        .map_err(|error| OperationFailure::invalid_request("submit_turn", error))?,
                    requested_run_profile: None,
                    requested_model: None,
                    idempotency_key: IdempotencyKey::new(format!(
                        "ironclaw-stress:{operation_ref}"
                    ))
                    .map_err(|error| OperationFailure::invalid_request("submit_turn", error))?,
                    received_at: Utc::now(),
                    requested_run_id: None,
                    parent_run_id: None,
                    subagent_depth: 0,
                    spawn_tree_root_run_id: None,
                    product_context: None,
                }),
            )
            .await;

            let submit_response = match submit_result {
                Ok(response) => response,
                Err(error @ TurnError::ThreadBusy(_)) => {
                    time_stage(
                        &mut stages.mark_rejected_busy,
                        self.thread_service.mark_message_rejected_busy(
                            &context.thread_scope,
                            &thread.thread_id,
                            accepted.message_id,
                        ),
                    )
                    .await
                    .map_err(|error| thread_failure("mark_rejected_busy", error))?;
                    return Err(turn_failure("submit_turn", error));
                }
                Err(error) => return Err(turn_failure("submit_turn", error)),
            };

            let SubmitTurnResponse::Accepted {
                turn_id, run_id, ..
            } = submit_response;

            time_stage(
                &mut stages.mark_submitted,
                self.thread_service.mark_message_submitted(
                    &context.thread_scope,
                    &thread.thread_id,
                    accepted.message_id,
                    turn_id.to_string(),
                    run_id.to_string(),
                ),
            )
            .await
            .map_err(|error| thread_failure("mark_submitted", error))?;

            let runner_id = TurnRunnerId::new();
            let lease_token = TurnLeaseToken::new();
            let claimed = time_stage(
                &mut stages.claim_run,
                turn_store.claim_next_run(ClaimRunRequest {
                    runner_id,
                    lease_token,
                    scope_filter: Some(context.turn_scope.clone()),
                }),
            )
            .await
            .map_err(|error| turn_failure("claim_run", error))?
            .ok_or_else(|| {
                OperationFailure::new(
                    "turn_claim_miss",
                    "claim_run",
                    "submitted run was not claimable",
                )
            })?;

            // Optionally route this operation through a gate block + resume so
            // persist-on-block fires under the concurrent workload. The resumed
            // run comes back queued and is re-claimed, so the normal completion
            // path below still owns finishing it.
            let claimed = if args.gate_blocked_every > 0
                && operation_index.is_multiple_of(args.gate_blocked_every)
            {
                // Alternate approval/auth by *blocked-hit* count, not raw
                // operation-index parity: every blocked index is a multiple of
                // `gate_blocked_every`, so parity alone would only ever pick one
                // gate kind for even intervals.
                let use_auth_gate = (operation_index / args.gate_blocked_every) % 2 == 1;
                self.gate_block_and_resume(&context, &turn_store, claimed, use_auth_gate, stages)
                    .await?
            } else {
                claimed
            };

            if matches!(args.scenario, Scenario::MixedUserSession) {
                stage_trace(
                    worker_index,
                    operation_index,
                    &operation_ref,
                    "load_context:start",
                );
                time_stage(
                    &mut stages.load_context,
                    self.thread_service
                        .load_context_window(LoadContextWindowRequest {
                            scope: context.thread_scope.clone(),
                            thread_id: thread.thread_id.clone(),
                            max_messages: args.context_max_messages,
                        }),
                )
                .await
                .map_err(|error| thread_failure("load_context", error))?;
                stage_trace(
                    worker_index,
                    operation_index,
                    &operation_ref,
                    "load_context:done",
                );

                stage_trace(
                    worker_index,
                    operation_index,
                    &operation_ref,
                    "resource_reserve:start",
                );
                let reservation = time_stage(
                    &mut stages.resource_reserve,
                    reserve_resources(
                        Arc::clone(&self.governor),
                        context.turn_scope.to_resource_scope(),
                    ),
                )
                .await?;
                stage_trace(
                    worker_index,
                    operation_index,
                    &operation_ref,
                    "resource_reserve:done",
                );

                let execution = async {
                    stage_trace(
                        worker_index,
                        operation_index,
                        &operation_ref,
                        "model_wait:start",
                    );
                    time_stage(
                        &mut stages.model_wait,
                        self.model_latency.run(args, worker_index, operation_index),
                    )
                    .await?;
                    stage_trace(
                        worker_index,
                        operation_index,
                        &operation_ref,
                        "model_wait:done",
                    );

                    stage_trace(
                        worker_index,
                        operation_index,
                        &operation_ref,
                        "write_assistant:start",
                    );
                    self.write_assistant_turn(
                        &context,
                        &thread.thread_id,
                        &turn_store,
                        &claimed,
                        assistant_message.clone(),
                        stages,
                    )
                    .await?;
                    stage_trace(
                        worker_index,
                        operation_index,
                        &operation_ref,
                        "write_assistant:done",
                    );

                    Ok::<(), OperationFailure>(())
                }
                .await;

                if let Err(error) = execution {
                    let _ = time_stage(
                        &mut stages.resource_release,
                        release_resources(Arc::clone(&self.governor), reservation.id),
                    )
                    .await;
                    return Err(error);
                }

                stage_trace(
                    worker_index,
                    operation_index,
                    &operation_ref,
                    "resource_reconcile:start",
                );
                time_stage(
                    &mut stages.resource_reconcile,
                    reconcile_resources(Arc::clone(&self.governor), reservation.id),
                )
                .await?;
                stage_trace(
                    worker_index,
                    operation_index,
                    &operation_ref,
                    "resource_reconcile:done",
                );
            } else if matches!(args.scenario, Scenario::ToolSession) {
                time_stage(
                    &mut stages.load_context,
                    self.thread_service
                        .load_context_window(LoadContextWindowRequest {
                            scope: context.thread_scope.clone(),
                            thread_id: thread.thread_id.clone(),
                            max_messages: args.context_max_messages,
                        }),
                )
                .await
                .map_err(|error| thread_failure("load_context", error))?;

                let reservation = time_stage(
                    &mut stages.resource_reserve,
                    reserve_resources(
                        Arc::clone(&self.governor),
                        context.turn_scope.to_resource_scope(),
                    ),
                )
                .await?;

                let execution = self
                    .write_tool_session_turn(
                        args,
                        &context,
                        &thread.thread_id,
                        &turn_store,
                        &claimed,
                        &assistant_message,
                        &operation_ref,
                        worker_index,
                        operation_index,
                        stages,
                    )
                    .await;

                if let Err(error) = execution {
                    let _ = time_stage(
                        &mut stages.resource_release,
                        release_resources(Arc::clone(&self.governor), reservation.id),
                    )
                    .await;
                    return Err(error);
                }

                time_stage(
                    &mut stages.resource_reconcile,
                    reconcile_resources(Arc::clone(&self.governor), reservation.id),
                )
                .await?;
            } else {
                self.write_assistant_turn(
                    &context,
                    &thread.thread_id,
                    &turn_store,
                    &claimed,
                    assistant_message,
                    stages,
                )
                .await?;

                time_stage(
                    &mut stages.load_context,
                    self.thread_service
                        .load_context_window(LoadContextWindowRequest {
                            scope: context.thread_scope.clone(),
                            thread_id: thread.thread_id.clone(),
                            max_messages: args.context_max_messages,
                        }),
                )
                .await
                .map_err(|error| thread_failure("load_context", error))?;
            }
        }

        Ok(())
    }

    async fn run_thread_list_operation(
        &self,
        args: &Args,
        identities: &SyntheticIds,
        stages: &mut UserTurnStageDurations,
    ) -> Result<(), OperationFailure> {
        let context = identities
            .user_turn_context_for_user_index(0)
            .map_err(|error| OperationFailure::invalid_request("thread_list_context", error))?;
        self.thread_service
            .clear_thread_index_cache_for_scope(&context.thread_scope);

        let cold_count = time_stage(
            &mut stages.list_threads_cold,
            self.list_thread_pages(&context.thread_scope, args.thread_list_page_size),
        )
        .await?;
        if cold_count != args.thread_list_threads {
            return Err(OperationFailure::new(
                "thread_list_count_mismatch",
                "list_threads_cold",
                format!(
                    "expected {} seeded threads, listed {cold_count}",
                    args.thread_list_threads
                ),
            ));
        }

        let warm_count = time_stage(
            &mut stages.list_threads_warm,
            self.list_thread_pages(&context.thread_scope, args.thread_list_page_size),
        )
        .await?;
        if warm_count != args.thread_list_threads {
            return Err(OperationFailure::new(
                "thread_list_count_mismatch",
                "list_threads_warm",
                format!(
                    "expected {} seeded threads, listed {warm_count}",
                    args.thread_list_threads
                ),
            ));
        }

        Ok(())
    }

    async fn list_thread_pages(
        &self,
        scope: &ironclaw_threads::ThreadScope,
        page_size: usize,
    ) -> Result<usize, OperationFailure> {
        let mut cursor = None;
        let mut listed = 0_usize;
        let mut pages = 0_usize;
        loop {
            let response = self
                .thread_service
                .list_threads_for_scope(ListThreadsForScopeRequest {
                    scope: scope.clone(),
                    limit: Some(page_size as u32),
                    cursor,
                })
                .await
                .map_err(|error| thread_failure("list_threads", error))?;
            listed = listed.saturating_add(response.threads.len());
            pages = pages.saturating_add(1);
            cursor = response.next_cursor;
            if cursor.is_none() {
                return Ok(listed);
            }
            if pages > 10_000 {
                return Err(OperationFailure::new(
                    "thread_list_cursor_loop",
                    "list_threads",
                    "thread list pagination exceeded 10000 pages",
                ));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_tool_session_turn(
        &self,
        args: &Args,
        context: &crate::synthetic::UserTurnContext,
        thread_id: &ThreadId,
        turn_store: &Arc<dyn StressTurnStore>,
        claimed: &ClaimedTurnRun,
        assistant_message: &str,
        operation_ref: &str,
        worker_index: usize,
        operation_index: usize,
        stages: &mut UserTurnStageDurations,
    ) -> Result<(), OperationFailure> {
        let mut draft_text = stress_payload(
            format!(
                "stress tool session {operation_ref} starting {} tool calls",
                args.tool_calls_per_turn
            ),
            args.assistant_message_bytes,
        );
        let draft = time_stage(
            &mut stages.append_assistant,
            self.thread_service
                .append_assistant_draft(AppendAssistantDraftRequest {
                    scope: context.thread_scope.clone(),
                    thread_id: thread_id.clone(),
                    turn_run_id: claimed.state.run_id.to_string(),
                    content: MessageContent::text(draft_text.clone()),
                }),
        )
        .await
        .map_err(|error| thread_failure("append_assistant", error))?;

        for tool_index in 0..args.tool_calls_per_turn {
            time_stage(
                &mut stages.tool_wait,
                synthetic_tool_wait(args.tool_latency_ms),
            )
            .await;

            let failed = synthetic_tool_failed(args, worker_index, operation_index, tool_index);
            let result_ref = tool_result_ref(args, worker_index, operation_index, tool_index);
            let safe_summary = if failed {
                ToolResultSafeSummary::new("tool failed")
            } else {
                ToolResultSafeSummary::new("tool completed")
            }
            .map_err(|error| OperationFailure::invalid_request("append_tool_result", error))?;

            time_stage(
                &mut stages.append_tool_result,
                self.thread_service.append_tool_result_reference(
                    AppendToolResultReferenceRequest {
                        scope: context.thread_scope.clone(),
                        thread_id: thread_id.clone(),
                        turn_run_id: claimed.state.run_id.to_string(),
                        result_ref: result_ref.clone(),
                        safe_summary,
                        provider_call: None,
                        model_observation: None,
                    },
                ),
            )
            .await
            .map_err(|error| thread_failure("append_tool_result", error))?;

            time_stage(
                &mut stages.append_tool_preview,
                self.thread_service.append_capability_display_preview(
                    AppendCapabilityDisplayPreviewRequest {
                        scope: context.thread_scope.clone(),
                        thread_id: thread_id.clone(),
                        turn_run_id: claimed.state.run_id.to_string(),
                        preview: tool_preview(args, &result_ref, failed, tool_index)?,
                    },
                ),
            )
            .await
            .map_err(|error| thread_failure("append_tool_preview", error))?;

            let status = if failed { "failed" } else { "completed" };
            draft_text.push_str(&format!("\ntool {tool_index} {status}: {result_ref}"));
            draft_text = stress_payload(draft_text, args.assistant_message_bytes);
            time_stage(
                &mut stages.update_assistant_draft,
                self.thread_service
                    .update_assistant_draft(UpdateAssistantDraftRequest {
                        scope: context.thread_scope.clone(),
                        thread_id: thread_id.clone(),
                        message_id: draft.message_id,
                        content: MessageContent::text(draft_text.clone()),
                    }),
            )
            .await
            .map_err(|error| thread_failure("update_assistant_draft", error))?;
        }

        time_stage(
            &mut stages.finalize_assistant,
            self.thread_service.finalize_assistant_message(
                &context.thread_scope,
                thread_id,
                draft.message_id,
                MessageContent::text(stress_payload(
                    format!(
                        "{assistant_message}\ncompleted {} synthetic tool calls",
                        args.tool_calls_per_turn
                    ),
                    args.assistant_message_bytes,
                )),
            ),
        )
        .await
        .map_err(|error| thread_failure("finalize_assistant", error))?;

        time_stage(
            &mut stages.complete_run,
            turn_store.complete_run(CompleteRunRequest {
                run_id: claimed.state.run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
            }),
        )
        .await
        .map_err(|error| turn_failure("complete_run", error))?;

        Ok(())
    }

    async fn write_assistant_turn(
        &self,
        context: &crate::synthetic::UserTurnContext,
        thread_id: &ThreadId,
        turn_store: &Arc<dyn StressTurnStore>,
        claimed: &ClaimedTurnRun,
        assistant_message: String,
        stages: &mut UserTurnStageDurations,
    ) -> Result<(), OperationFailure> {
        time_stage(
            &mut stages.append_assistant,
            self.thread_service.append_finalized_assistant_message(
                AppendFinalizedAssistantMessageRequest {
                    scope: context.thread_scope.clone(),
                    thread_id: thread_id.clone(),
                    turn_run_id: claimed.state.run_id.to_string(),
                    content: MessageContent::text(assistant_message),
                },
            ),
        )
        .await
        .map_err(|error| thread_failure("append_assistant", error))?;

        time_stage(
            &mut stages.complete_run,
            turn_store.complete_run(CompleteRunRequest {
                run_id: claimed.state.run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
            }),
        )
        .await
        .map_err(|error| turn_failure("complete_run", error))?;

        Ok(())
    }

    /// Route a claimed run through a gate block + resume so persist-on-block
    /// fires, then re-claim it and hand the fresh claim back for the caller to
    /// complete. The caller alternates `use_auth_gate` per blocked hit so both
    /// approval and auth gate kinds exercise the durable sink.
    async fn gate_block_and_resume(
        &self,
        context: &crate::synthetic::UserTurnContext,
        turn_store: &Arc<dyn StressTurnStore>,
        claimed: ClaimedTurnRun,
        use_auth_gate: bool,
        stages: &mut UserTurnStageDurations,
    ) -> Result<ClaimedTurnRun, OperationFailure> {
        let run_id = claimed.state.run_id;
        let is_auth = use_auth_gate;
        let gate_ref = GateRef::new(format!("stress-gate:{run_id}"))
            .map_err(|error| OperationFailure::invalid_request("block_run", error))?;
        let state_ref = LoopCheckpointStateRef::new(format!("checkpoint:stress-block-{run_id}"))
            .map_err(|error| OperationFailure::invalid_request("block_run", error))?;
        let reason = if is_auth {
            BlockedReason::Auth {
                gate_ref: gate_ref.clone(),
                credential_requirements: Vec::new(),
            }
        } else {
            BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            }
        };
        time_stage(
            &mut stages.block_run,
            turn_store.block_run(BlockRunRequest {
                run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
                checkpoint_id: TurnCheckpointId::new(),
                state_ref,
                reason,
            }),
        )
        .await
        .map_err(|error| turn_failure("block_run", error))?;

        let precondition = if is_auth {
            ResumeTurnPrecondition::BlockedAuthGate
        } else {
            ResumeTurnPrecondition::BlockedApprovalGate
        };
        time_stage(
            &mut stages.resume_turn,
            turn_store.resume_turn(ResumeTurnRequest {
                scope: context.turn_scope.clone(),
                actor: TurnActor::new(context.user_id.clone()),
                run_id,
                gate_resolution_ref: gate_ref,
                source_binding_ref: SourceBindingRef::new(format!("stress-src:{run_id}"))
                    .map_err(|error| OperationFailure::invalid_request("resume_turn", error))?,
                reply_target_binding_ref: ReplyTargetBindingRef::new(format!(
                    "stress-reply:{run_id}"
                ))
                .map_err(|error| OperationFailure::invalid_request("resume_turn", error))?,
                idempotency_key: IdempotencyKey::new(format!("stress-resume:{run_id}"))
                    .map_err(|error| OperationFailure::invalid_request("resume_turn", error))?,
                precondition,
                resume_disposition: None,
            }),
        )
        .await
        .map_err(|error| turn_failure("resume_turn", error))?;

        // Resume returns the run to Queued — re-claim it so the normal
        // completion path owns finishing it.
        time_stage(
            &mut stages.reclaim_run,
            turn_store.claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: Some(context.turn_scope.clone()),
            }),
        )
        .await
        .map_err(|error| turn_failure("reclaim_run", error))?
        .ok_or_else(|| {
            OperationFailure::new(
                "turn_claim_miss",
                "reclaim_run",
                "resumed run was not claimable",
            )
        })
    }

    fn turn_store_for_context(
        &self,
        context: &crate::synthetic::UserTurnContext,
    ) -> Result<Arc<dyn StressTurnStore>, OperationFailure> {
        match self.turn_state_backend {
            // One shared authority for the whole process — concurrent same-user
            // writers coordinate in memory (fast lock), never on a per-user
            // `state.json` CAS, so they don't livelock. `MemoryPersistOnBlock`
            // shares the same authority; it differs only by the durable block
            // sink attached at construction.
            TurnStateBackend::Memory | TurnStateBackend::MemoryPersistOnBlock => {
                Ok(Arc::clone(&self.memory_turn_store) as Arc<dyn StressTurnStore>)
            }
            TurnStateBackend::FilesystemRow => {
                let resource_scope = context.turn_scope.to_resource_scope();
                self.cached_row_store(&resource_scope, |view| {
                    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
                        Arc::clone(&self.root),
                        view,
                    ));
                    Arc::new(
                        FilesystemTurnStateRowStore::new(scoped)
                            .with_limits(self.turn_state_limits),
                    )
                })
            }
            // Same row-store mechanism, but over the shared in-process
            // in-memory backend — models replacing the direct in-memory
            // authority in the `inmemory-turn-state` profile with the row
            // store (journal/delta semantics, no durable-backend cost).
            TurnStateBackend::RowMemory => {
                let resource_scope = context.turn_scope.to_resource_scope();
                self.cached_row_store(&resource_scope, |view| {
                    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
                        Arc::clone(&self.memory_root),
                        view,
                    ));
                    Arc::new(
                        FilesystemTurnStateRowStore::new(scoped)
                            .with_limits(self.turn_state_limits),
                    )
                })
            }
            // Durable path: a per-context store whose `/turns/state.json`
            // resolves per (tenant, agent, project, user), so all of a user's
            // concurrent turns contend on one document via CAS.
            TurnStateBackend::Filesystem => {
                let view =
                    user_turn_mount_view(&self.run_id, &context.turn_scope.to_resource_scope())
                        .map_err(|error| OperationFailure::invalid_request("turn_store", error))?;
                let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
                    Arc::clone(&self.root),
                    view,
                ));
                Ok(Arc::new(
                    FilesystemTurnStateStore::new(scoped).with_limits(self.turn_state_limits),
                ) as Arc<dyn StressTurnStore>)
            }
        }
    }

    /// Look up (or build and cache) the shared per-tenant/user row store for
    /// `resource_scope`. Both row-store variants share this cache — only one
    /// backend is active per process, so keys never mix roots.
    fn cached_row_store(
        &self,
        resource_scope: &ResourceScope,
        build: impl FnOnce(MountView) -> Arc<dyn StressTurnStore>,
    ) -> Result<Arc<dyn StressTurnStore>, OperationFailure> {
        let key = row_turn_store_key(resource_scope);
        // Fast path: a shared read of the cache under the lock. The lock is NOT
        // held across `build` — in a throughput benchmark, serializing every
        // thread's store construction behind one mutex would distort the very
        // latency evidence this harness measures.
        if let Some(store) = self.lock_row_stores()?.get(&key).cloned() {
            return Ok(store);
        }

        // Build outside the lock. A concurrent thread may build the same key in
        // parallel; that is a bounded, one-time waste (a cache miss races), not
        // a correctness issue — the insert below dedups to a single shared store.
        let view = user_turn_mount_view(&self.run_id, resource_scope)
            .map_err(|error| OperationFailure::invalid_request("turn_store", error))?;
        let built = build(view);

        let mut stores = self.lock_row_stores()?;
        // Re-check under the lock: if another thread inserted first, adopt theirs
        // and drop ours so every caller for this key shares one store.
        Ok(Arc::clone(stores.entry(key).or_insert(built)))
    }

    fn lock_row_stores(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, RowTurnStoreCache>, OperationFailure> {
        self.row_turn_stores.lock().map_err(|_| {
            OperationFailure::new(
                "turn_store_lock_poisoned",
                "turn_store",
                "row turn-store cache lock poisoned",
            )
        })
    }
}

fn user_turn_services_from_root<F>(
    root: Arc<F>,
    governor: Arc<dyn ResourceGovernor>,
    run_id: &str,
    target: String,
    model_latency: Arc<ModelLatencyDriver>,
    turn_state_backend: TurnStateBackend,
    turn_state_limits: InMemoryTurnStateStoreLimits,
) -> Result<UserTurnServices<F>, String>
where
    F: RootFilesystem + 'static,
{
    let run_id = run_id.to_string();
    let scoped = Arc::new(ScopedFilesystem::new(Arc::clone(&root), {
        let run_id = run_id.clone();
        move |scope| user_turn_mount_view(&run_id, scope)
    }));
    Ok(UserTurnServices {
        root,
        governor,
        thread_service: Arc::new(FilesystemSessionThreadService::new(Arc::clone(&scoped))),
        model_latency,
        run_id,
        target,
        turn_state_backend,
        turn_state_limits,
        // Constructed once and shared across every worker (the workload is held
        // behind one Arc), so the Memory backend exercises a single shared
        // authority exactly as the single-process runtime would. When the
        // backend persists on block, attach the same durable filesystem sink the
        // hosted-single-tenant-volume runtime wires so the retest measures the
        // shipped config (the sink stays idle on this never-blocking workload,
        // so what it measures is the extra probe cost per terminal transition).
        memory_turn_store: Arc::new({
            let store = InMemoryTurnStateStore::with_limits(turn_state_limits);
            if turn_state_backend.persists_on_block() {
                let sink = Arc::new(FilesystemTurnStateBlockPersistence::new(Arc::clone(
                    &scoped,
                )));
                store.with_block_persistence(sink)
            } else {
                store
            }
        }),
        row_turn_stores: Mutex::new(HashMap::new()),
        memory_root: Arc::new(InMemoryBackend::new()),
    })
}

fn row_turn_store_key(scope: &ResourceScope) -> String {
    let tenant = scope.tenant_id.as_str();
    let user = scope.user_id.as_str();
    format!("{}:{tenant}:{}:{user}", tenant.len(), user.len())
}

fn user_turn_mount_view(run_id: &str, scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = scope.tenant_id.as_str();
    let user = scope.user_id.as_str();
    let base = format!("/engine/ironclaw-stress/{run_id}/tenants/{tenant}");
    let threads_target = format!("{base}/users/{user}/threads");

    let turns_target = match (scope.agent_id.as_ref(), scope.project_id.as_ref()) {
        (Some(agent_id), Some(project_id)) => format!(
            "{base}/agents/{}/projects/{}/users/{user}/turns",
            agent_id.as_str(),
            project_id.as_str()
        ),
        (Some(agent_id), None) => {
            format!("{base}/agents/{}/users/{user}/turns", agent_id.as_str())
        }
        (None, Some(project_id)) => {
            format!("{base}/projects/{}/users/{user}/turns", project_id.as_str())
        }
        (None, None) => format!("{base}/users/{user}/turns"),
    };

    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/threads")?,
            VirtualPath::new(threads_target)?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/turns")?,
            VirtualPath::new(turns_target)?,
            MountPermissions::read_write_list_delete(),
        ),
    ])
}

async fn time_stage<T>(slot: &mut Option<Duration>, future: impl Future<Output = T>) -> T {
    let started = Instant::now();
    let output = future.await;
    let elapsed = started.elapsed();
    *slot = Some(slot.unwrap_or_default().saturating_add(elapsed));
    output
}

fn stage_trace(worker_index: usize, operation_index: usize, operation_ref: &str, stage: &str) {
    if std::env::var_os("IRONCLAW_STRESS_STAGE_TRACE").is_some() {
        eprintln!(
            "[ironclaw-stress-stage] worker={worker_index} op={operation_index} ref={operation_ref} {stage}"
        );
    }
}

async fn reserve_resources(
    governor: Arc<dyn ResourceGovernor>,
    scope: ResourceScope,
) -> Result<ResourceReservation, OperationFailure> {
    run_resource_governor_blocking("resource_reserve", move || {
        governor.reserve(scope, resource_ops::estimate())
    })
    .await
}

async fn reconcile_resources(
    governor: Arc<dyn ResourceGovernor>,
    reservation_id: ResourceReservationId,
) -> Result<(), OperationFailure> {
    run_resource_governor_blocking("resource_reconcile", move || {
        governor.reconcile(reservation_id, resource_ops::usage())
    })
    .await
    .map(|_| ())
}

async fn release_resources(
    governor: Arc<dyn ResourceGovernor>,
    reservation_id: ResourceReservationId,
) -> Result<(), OperationFailure> {
    run_resource_governor_blocking("resource_release", move || governor.release(reservation_id))
        .await
        .map(|_| ())
}

async fn run_resource_governor_blocking<T>(
    stage: &'static str,
    call: impl FnOnce() -> Result<T, ResourceError> + Send + 'static,
) -> Result<T, OperationFailure>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|error| {
            OperationFailure::new(
                "resource_governor_join",
                stage,
                format!("resource governor blocking task failed: {error}"),
            )
        })?
        .map_err(|error| resource_failure(stage, error))
}

async fn synthetic_model_wait(args: &Args, worker_index: usize, operation_index: usize) {
    let wait_ms = synthetic_model_wait_ms(args, worker_index, operation_index);
    if wait_ms > 0 {
        sleep(Duration::from_millis(wait_ms)).await;
    }
}

enum ModelLatencyDriver {
    Synthetic,
    Provider(ProviderLatencyDriver),
}

impl ModelLatencyDriver {
    async fn run(
        &self,
        args: &Args,
        worker_index: usize,
        operation_index: usize,
    ) -> Result<(), OperationFailure> {
        match self {
            Self::Synthetic => {
                synthetic_model_wait(args, worker_index, operation_index).await;
                Ok(())
            }
            Self::Provider(driver) => driver.run(args, worker_index, operation_index).await,
        }
    }
}

struct ProviderLatencyDriver {
    provider: Arc<dyn LlmProvider>,
}

impl ProviderLatencyDriver {
    async fn run(
        &self,
        args: &Args,
        worker_index: usize,
        operation_index: usize,
    ) -> Result<(), OperationFailure> {
        let prompt = provider_latency_prompt(worker_index, operation_index);
        let mut request = CompletionRequest::new(vec![ChatMessage::user(prompt)])
            .with_max_tokens(args.provider_max_tokens);
        if let Some(model) = args.provider_model.as_deref() {
            request = request.with_model(model);
        }
        self.provider
            .complete(request)
            .await
            .map(|_| ())
            .map_err(provider_latency_failure)
    }
}

async fn build_model_latency_driver(args: &Args) -> Result<Arc<ModelLatencyDriver>, String> {
    match args.model_latency_source {
        ModelLatencySource::Synthetic => Ok(Arc::new(ModelLatencyDriver::Synthetic)),
        ModelLatencySource::Provider => {
            if !matches!(args.scenario, Scenario::MixedUserSession) {
                return Err(
                    "--model-latency-source provider is only supported with --scenario mixed-user-session"
                        .to_string(),
                );
            }
            let config = resolve_llm_config_from_env(None)
                .map_err(|error| format!("resolve LLM provider config: {error}"))?
                .ok_or_else(|| {
                    "no LLM provider configuration found; set LLM_BACKEND and provider credentials"
                        .to_string()
                })?;
            let session = Arc::new(SessionManager::new_async(config.session.clone()).await);
            let provider = build_static_provider_chain(&config, session)
                .await
                .map_err(|error| format!("build LLM provider chain: {error}"))?;
            eprintln!(
                "{} provider latency source initialized backend={} model={} max_tokens={}",
                crate::log_prefix(args),
                config.backend,
                args.provider_model
                    .as_deref()
                    .unwrap_or_else(|| provider.model_name()),
                args.provider_max_tokens
            );
            Ok(Arc::new(ModelLatencyDriver::Provider(
                ProviderLatencyDriver { provider },
            )))
        }
    }
}

fn provider_latency_prompt(worker_index: usize, operation_index: usize) -> String {
    format!(
        "IronClaw stress latency probe. Reply with exactly: ok. worker={worker_index} operation={operation_index}"
    )
}

fn provider_latency_failure(error: LlmError) -> OperationFailure {
    let bucket = match &error {
        LlmError::RateLimited { .. } => "model_provider_rate_limited",
        LlmError::AuthFailed { .. }
        | LlmError::SessionExpired { .. }
        | LlmError::SessionRenewalFailed { .. } => "model_provider_auth",
        LlmError::ContextLengthExceeded { .. } => "model_provider_context_length",
        LlmError::ModelNotAvailable { .. } => "model_provider_model_unavailable",
        LlmError::BadGateway { .. }
        | LlmError::RequestFailed { .. }
        | LlmError::InvalidResponse { .. }
        | LlmError::EmptyResponse { .. }
        | LlmError::Http(_)
        | LlmError::Json(_)
        | LlmError::Io(_) => "model_provider_error",
    };
    OperationFailure::new(bucket, "model_wait", error)
}

async fn synthetic_tool_wait(wait_ms: u64) {
    if wait_ms > 0 {
        sleep(Duration::from_millis(wait_ms)).await;
    }
}

pub(crate) fn synthetic_model_wait_ms(
    args: &Args,
    worker_index: usize,
    operation_index: usize,
) -> u64 {
    match args.model_latency_profile {
        ModelLatencyProfile::Fixed => args.model_latency_ms,
        ModelLatencyProfile::Uniform => {
            args.model_latency_ms + deterministic_jitter_ms(args, worker_index, operation_index)
        }
        ModelLatencyProfile::TailSpike => {
            let sequence = if args.uses_duration_mode() {
                operation_index
                    .saturating_mul(args.concurrency)
                    .saturating_add(worker_index)
                    .saturating_add(1)
            } else {
                worker_index
                    .saturating_mul(args.operations)
                    .saturating_add(operation_index)
                    .saturating_add(1)
            };
            if args.model_latency_spike_every > 0
                && sequence.is_multiple_of(args.model_latency_spike_every)
            {
                if args.model_latency_spike_ms > 0 {
                    args.model_latency_spike_ms
                } else {
                    args.model_latency_ms
                        + deterministic_jitter_ms(args, worker_index, operation_index)
                }
            } else {
                args.model_latency_ms
            }
        }
    }
}

pub(crate) fn synthetic_tool_failed(
    args: &Args,
    worker_index: usize,
    operation_index: usize,
    tool_index: usize,
) -> bool {
    if args.tool_failure_every == 0 {
        return false;
    }
    let operation_sequence = if args.uses_duration_mode() {
        operation_index
            .saturating_mul(args.concurrency)
            .saturating_add(worker_index)
    } else {
        worker_index
            .saturating_mul(args.operations)
            .saturating_add(operation_index)
    };
    let tool_sequence = operation_sequence
        .saturating_mul(args.tool_calls_per_turn)
        .saturating_add(tool_index)
        .saturating_add(1);
    tool_sequence.is_multiple_of(args.tool_failure_every)
}

fn deterministic_jitter_ms(args: &Args, worker_index: usize, operation_index: usize) -> u64 {
    if args.model_latency_jitter_ms == 0 {
        return 0;
    }
    let mut value = args
        .run_id
        .as_deref()
        .unwrap_or("ironclaw-stress")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            hash ^ u64::from(byte).wrapping_mul(0x0000_0100_0000_01b3)
        });
    value ^= (worker_index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value ^= (operation_index as u64).wrapping_mul(0xD6E8_FD9D_5A42_9A1D);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
    value ^= value >> 33;
    value % (args.model_latency_jitter_ms + 1)
}

fn tool_result_ref(
    args: &Args,
    worker_index: usize,
    operation_index: usize,
    tool_index: usize,
) -> String {
    let phase = if args.warmup_phase { "-warmup" } else { "" };
    format!(
        "result:stress-child-{}{}-worker-{worker_index}-op-{operation_index}-tool-{tool_index}",
        args.child_index.unwrap_or(0),
        phase
    )
}

fn tool_preview(
    args: &Args,
    result_ref: &str,
    failed: bool,
    tool_index: usize,
) -> Result<CapabilityDisplayPreviewEnvelope, OperationFailure> {
    let status = if failed {
        CapabilityDisplayPreviewStatus::Failed
    } else {
        CapabilityDisplayPreviewStatus::Completed
    };
    let output_preview = stress_payload(
        format!("synthetic tool output {result_ref}"),
        args.tool_output_bytes,
    );
    CapabilityDisplayPreviewEnvelope::new(CapabilityDisplayPreviewEnvelopeInput {
        invocation_id: InvocationId::new(),
        capability_id: CapabilityId::new("ironclaw_stress.synthetic_tool")
            .map_err(|error| OperationFailure::invalid_request("append_tool_preview", error))?,
        status,
        title: format!("synthetic tool {tool_index}"),
        subtitle: Some(if failed {
            "failed synthetic tool result".to_string()
        } else {
            "completed synthetic tool result".to_string()
        }),
        input_summary: Some("synthetic bounded tool input".to_string()),
        output_summary: Some(if failed {
            "synthetic tool failed".to_string()
        } else {
            "synthetic tool completed".to_string()
        }),
        output_preview: Some(output_preview),
        output_kind: Some("text".to_string()),
        output_bytes: Some(args.tool_output_bytes as u64),
        result_ref: Some(result_ref.to_string()),
        truncated: false,
        updated_at: Utc::now(),
        activity_order: Some(tool_index as u64),
    })
    .map_err(|error| OperationFailure::invalid_request("append_tool_preview", error))
}

fn stress_payload(mut base: String, minimum_bytes: usize) -> String {
    if minimum_bytes == 0 || base.len() >= minimum_bytes {
        return base;
    }
    base.push(' ');
    let pattern = "0123456789abcdef";
    while base.len() < minimum_bytes {
        let remaining = minimum_bytes - base.len();
        let take = remaining.min(pattern.len());
        base.push_str(&pattern[..take]);
    }
    base
}

fn thread_list_thread_id(thread_index: usize) -> Result<ThreadId, OperationFailure> {
    ThreadId::new(format!("thread-list-{thread_index:06}"))
        .map_err(|error| OperationFailure::invalid_request("thread_list_thread_id", error))
}

fn operation_ref(args: &Args, worker_index: usize, operation_index: usize) -> String {
    let phase = if args.warmup_phase { ":warmup" } else { "" };
    format!(
        "{}:child-{}{}:worker-{worker_index}:op-{operation_index}",
        args.run_id.as_deref().unwrap_or("unknown-run"),
        args.child_index.unwrap_or(0),
        phase
    )
}

fn turn_operation_ref(
    args: &Args,
    worker_index: usize,
    operation_index: usize,
    turn_index: usize,
    turns_per_operation: usize,
) -> String {
    let operation_ref = operation_ref(args, worker_index, operation_index);
    if turns_per_operation == 1 {
        operation_ref
    } else {
        format!("{operation_ref}:turn-{turn_index}")
    }
}

fn maybe_emit_operation_span(
    args: &Args,
    worker_index: usize,
    operation_index: usize,
    latency: Duration,
    stages: &UserTurnStageDurations,
    failure: Option<&FailureCause>,
    span_budget: &AtomicUsize,
) {
    let slow = args.slow_span_threshold_ms > 0
        && latency >= Duration::from_millis(args.slow_span_threshold_ms);
    let failed = failure.is_some();
    if (!args.span_log_failures || !failed) && !slow {
        return;
    }
    if !try_claim_span_budget(span_budget) {
        return;
    }

    let span = serde_json::json!({
        "backend": args.backend,
        "scenario": args.scenario,
        "run_id": args.run_id.as_deref().unwrap_or("unknown-run"),
        "child_index": args.child_index.unwrap_or(0),
        "worker_index": worker_index,
        "operation_index": operation_index,
        "operation_ref": operation_ref(args, worker_index, operation_index),
        "latency_us": latency.as_micros(),
        "failed": failed,
        "failure": failure,
        "stages_us": stage_latencies_us(stages),
    });
    match serde_json::to_string(&span) {
        Ok(encoded) => eprintln!("{} span {encoded}", crate::log_prefix(args)),
        Err(error) => eprintln!("{} failed to encode span: {error}", crate::log_prefix(args)),
    }
}

fn stage_latencies_us(stages: &UserTurnStageDurations) -> serde_json::Value {
    let mut output = serde_json::Map::new();
    insert_stage_latency(&mut output, "ensure_thread", stages.ensure_thread);
    insert_stage_latency(&mut output, "accept_inbound", stages.accept_inbound);
    insert_stage_latency(&mut output, "submit_turn", stages.submit_turn);
    insert_stage_latency(&mut output, "mark_submitted", stages.mark_submitted);
    insert_stage_latency(&mut output, "mark_rejected_busy", stages.mark_rejected_busy);
    insert_stage_latency(&mut output, "list_threads_cold", stages.list_threads_cold);
    insert_stage_latency(&mut output, "list_threads_warm", stages.list_threads_warm);
    insert_stage_latency(&mut output, "claim_run", stages.claim_run);
    insert_stage_latency(&mut output, "block_run", stages.block_run);
    insert_stage_latency(&mut output, "resume_turn", stages.resume_turn);
    insert_stage_latency(&mut output, "reclaim_run", stages.reclaim_run);
    insert_stage_latency(&mut output, "append_assistant", stages.append_assistant);
    insert_stage_latency(&mut output, "finalize_assistant", stages.finalize_assistant);
    insert_stage_latency(&mut output, "complete_run", stages.complete_run);
    insert_stage_latency(&mut output, "load_context", stages.load_context);
    insert_stage_latency(&mut output, "resource_reserve", stages.resource_reserve);
    insert_stage_latency(&mut output, "model_wait", stages.model_wait);
    insert_stage_latency(&mut output, "tool_wait", stages.tool_wait);
    insert_stage_latency(&mut output, "append_tool_result", stages.append_tool_result);
    insert_stage_latency(
        &mut output,
        "append_tool_preview",
        stages.append_tool_preview,
    );
    insert_stage_latency(
        &mut output,
        "update_assistant_draft",
        stages.update_assistant_draft,
    );
    insert_stage_latency(&mut output, "resource_reconcile", stages.resource_reconcile);
    insert_stage_latency(&mut output, "resource_release", stages.resource_release);
    serde_json::Value::Object(output)
}

fn insert_stage_latency(
    output: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    duration: Option<Duration>,
) {
    if let Some(duration) = duration {
        output.insert(name.to_string(), serde_json::json!(duration.as_micros()));
    }
}

fn try_claim_span_budget(span_budget: &AtomicUsize) -> bool {
    loop {
        let remaining = span_budget.load(Ordering::Relaxed);
        if remaining == 0 {
            return false;
        }
        if span_budget
            .compare_exchange_weak(
                remaining,
                remaining.saturating_sub(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return true;
        }
    }
}

fn span_sample_limit(limit: usize) -> usize {
    if limit == 0 { usize::MAX } else { limit }
}

#[derive(Debug)]
struct OperationFailure {
    cause: FailureCause,
}

impl OperationFailure {
    fn new(
        bucket: impl Into<String>,
        stage: impl Into<String>,
        detail: impl std::fmt::Display,
    ) -> Self {
        let bucket = bucket.into();
        let stage = stage.into();
        let cause = FailureCause::new(bucket, stage, detail);
        if std::env::var_os("IRONCLAW_STRESS_DEBUG_ERRORS").is_some() {
            eprintln!(
                "[ironclaw-stress] operation error bucket={} stage={}: {}",
                cause.bucket, cause.stage, cause.detail
            );
        }
        Self { cause }
    }

    fn invalid_request(stage: impl Into<String>, detail: impl std::fmt::Display) -> Self {
        Self::new("invalid_request", stage, detail)
    }
}

fn thread_failure(stage: impl Into<String>, error: SessionThreadError) -> OperationFailure {
    let bucket = match &error {
        SessionThreadError::UnknownThread { .. } => "thread_unknown",
        SessionThreadError::UnknownMessage { .. } => "thread_message_unknown",
        SessionThreadError::ThreadScopeMismatch { .. } => "thread_scope_mismatch",
        SessionThreadError::MessageNotDraft { .. } => "thread_message_not_draft",
        SessionThreadError::InvalidMessageTransition { .. } => "thread_invalid_transition",
        SessionThreadError::IdempotentReplayThreadMismatch { .. }
        | SessionThreadError::IdempotentReplayActorMismatch { .. } => "thread_idempotency_mismatch",
        SessionThreadError::InvalidSummaryRange { .. }
        | SessionThreadError::OverlappingSummaryRange { .. }
        | SessionThreadError::InvalidAttachment(_)
        | SessionThreadError::GeneratedThreadId(_) => "thread_invalid_request",
        SessionThreadError::Serialization(_) | SessionThreadError::Deserialization(_) => {
            "thread_serialization"
        }
        SessionThreadError::InvalidMessageTimestamp { .. } => "thread_timestamp_invalid",
        SessionThreadError::Backend(_) => "thread_backend",
    };
    OperationFailure::new(bucket, stage, error)
}

fn turn_failure(stage: impl Into<String>, error: TurnError) -> OperationFailure {
    let bucket = match error.category() {
        TurnErrorCategory::ThreadBusy => "turn_thread_busy",
        TurnErrorCategory::AdmissionRejected => "turn_admission_rejected",
        TurnErrorCategory::ScopeNotFound => "turn_scope_not_found",
        TurnErrorCategory::Unauthorized => "turn_unauthorized",
        TurnErrorCategory::InvalidRequest => "turn_invalid_request",
        TurnErrorCategory::Unavailable => "turn_unavailable",
        TurnErrorCategory::Conflict => "turn_conflict",
        TurnErrorCategory::CapacityExceeded => "turn_capacity_exceeded",
    };
    OperationFailure::new(bucket, stage, error)
}

fn resource_failure(stage: &'static str, error: ResourceError) -> OperationFailure {
    OperationFailure {
        cause: resource_ops::failure_for_stage(stage, error),
    }
}
/// Per-tenant/user row turn-store cache, keyed by scope.
type RowTurnStoreCache = HashMap<String, Arc<dyn StressTurnStore>>;
