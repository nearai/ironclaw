use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Utc;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    HostApiError, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, VirtualPath,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendAssistantDraftRequest, EnsureThreadRequest,
    FilesystemSessionThreadService, LoadContextWindowRequest, MessageContent, SessionThreadError,
    SessionThreadService,
};
use ironclaw_turns::{
    AcceptedMessageRef, DefaultTurnCoordinator, FilesystemTurnStateStore, IdempotencyKey,
    ReplyTargetBindingRef, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCoordinator, TurnError, TurnErrorCategory, TurnLeaseToken, TurnRunnerId,
    runner::{ClaimRunRequest, CompleteRunRequest, TurnRunTransitionPort},
};
use serde::{Deserialize, Serialize};

use crate::{
    Args, Backend, LatencySummary, Sample,
    progress::{ProgressCounters, spawn_progress_reporter, stop_progress_reporter},
    summary::FailureCause,
    synthetic::SyntheticIds,
};

pub(crate) struct UserTurnServices<F>
where
    F: RootFilesystem,
{
    root: Arc<F>,
    thread_service: Arc<FilesystemSessionThreadService<F>>,
    run_id: String,
    target: String,
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
    pub(crate) claim_run: StageLatencySummary,
    pub(crate) append_assistant: StageLatencySummary,
    pub(crate) finalize_assistant: StageLatencySummary,
    pub(crate) complete_run: StageLatencySummary,
    pub(crate) load_context: StageLatencySummary,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UserTurnStageDurations {
    pub(crate) ensure_thread: Option<Duration>,
    pub(crate) accept_inbound: Option<Duration>,
    pub(crate) submit_turn: Option<Duration>,
    pub(crate) mark_submitted: Option<Duration>,
    pub(crate) mark_rejected_busy: Option<Duration>,
    pub(crate) claim_run: Option<Duration>,
    pub(crate) append_assistant: Option<Duration>,
    pub(crate) finalize_assistant: Option<Duration>,
    pub(crate) complete_run: Option<Duration>,
    pub(crate) load_context: Option<Duration>,
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
    Ok(UserTurnWorkload::Libsql(user_turn_services_from_root(
        filesystem, run_id, target,
    )))
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
    let (filesystem, target) = crate::build_postgres_root(args).await?;
    Ok(UserTurnWorkload::Postgres(user_turn_services_from_root(
        filesystem, run_id, target,
    )))
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
    let total_operations = args.concurrency.saturating_mul(args.operations);
    let progress = Arc::new(ProgressCounters::default());
    let span_budget = Arc::new(AtomicUsize::new(span_sample_limit(args.span_sample_limit)));
    let progress_reporter = spawn_progress_reporter(
        crate::log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        args.progress_interval_seconds,
        total_operations,
        Arc::clone(&progress),
    );

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
                let mut samples = Vec::with_capacity(args.operations);
                for operation_index in 0..args.operations {
                    let sample = workload
                        .run_operation(
                            &args,
                            &identities,
                            worker_index,
                            operation_index,
                            &span_budget,
                        )
                        .await;
                    progress.record(sample.error.is_some());
                    samples.push(sample);
                }
                samples
            }),
        ));
    }

    let mut samples = Vec::with_capacity(total_operations);
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
    stop_progress_reporter(progress_reporter);

    if let Some(error) = first_error {
        return Err(error);
    }
    if samples.len() != total_operations {
        return Err(format!(
            "collected {} samples but expected {total_operations}",
            samples.len()
        ));
    }
    Ok(samples)
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
}

impl<F> UserTurnServices<F>
where
    F: RootFilesystem + 'static,
{
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
        let turn_store = self.turn_store_for_context(&context)?;
        let turn_coordinator = DefaultTurnCoordinator::new(Arc::clone(&turn_store));
        let operation_ref = operation_ref(args, worker_index, operation_index);
        let source_binding = "storage-stress-webchat";
        let reply_target = "storage-stress-reply";

        let thread = time_stage(
            &mut stages.ensure_thread,
            self.thread_service.ensure_thread(EnsureThreadRequest {
                scope: context.thread_scope.clone(),
                thread_id: Some(context.thread_id.clone()),
                created_by_actor_id: context.user_id.as_str().to_string(),
                title: Some(format!("Storage stress {}", context.user_id.as_str())),
                metadata_json: None,
            }),
        )
        .await
        .map_err(|error| thread_failure("ensure_thread", error))?;

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
                    content: MessageContent::text(format!("stress message {operation_ref}")),
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
                idempotency_key: IdempotencyKey::new(format!("storage-stress:{operation_ref}"))
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

        let draft = time_stage(
            &mut stages.append_assistant,
            self.thread_service
                .append_assistant_draft(AppendAssistantDraftRequest {
                    scope: context.thread_scope.clone(),
                    thread_id: thread.thread_id.clone(),
                    turn_run_id: claimed.state.run_id.to_string(),
                    content: MessageContent::text(format!("stress response {operation_ref}")),
                }),
        )
        .await
        .map_err(|error| thread_failure("append_assistant", error))?;

        time_stage(
            &mut stages.finalize_assistant,
            self.thread_service.finalize_assistant_message(
                &context.thread_scope,
                &thread.thread_id,
                draft.message_id,
                MessageContent::text(format!("stress response {operation_ref}")),
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

        time_stage(
            &mut stages.load_context,
            self.thread_service
                .load_context_window(LoadContextWindowRequest {
                    scope: context.thread_scope,
                    thread_id: thread.thread_id,
                    max_messages: 20,
                }),
        )
        .await
        .map_err(|error| thread_failure("load_context", error))?;

        Ok(())
    }

    fn turn_store_for_context(
        &self,
        context: &crate::synthetic::UserTurnContext,
    ) -> Result<Arc<FilesystemTurnStateStore<F>>, OperationFailure> {
        let view = user_turn_mount_view(&self.run_id, &context.turn_scope.to_resource_scope())
            .map_err(|error| OperationFailure::invalid_request("turn_store", error))?;
        let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::clone(&self.root),
            view,
        ));
        Ok(Arc::new(FilesystemTurnStateStore::new(scoped)))
    }
}

fn user_turn_services_from_root<F>(
    root: Arc<F>,
    run_id: &str,
    target: String,
) -> UserTurnServices<F>
where
    F: RootFilesystem + 'static,
{
    let run_id = run_id.to_string();
    let scoped = Arc::new(ScopedFilesystem::new(Arc::clone(&root), {
        let run_id = run_id.clone();
        move |scope| user_turn_mount_view(&run_id, scope)
    }));
    UserTurnServices {
        root,
        thread_service: Arc::new(FilesystemSessionThreadService::new(scoped)),
        run_id,
        target,
    }
}

fn user_turn_mount_view(run_id: &str, scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = scope.tenant_id.as_str();
    let user = scope.user_id.as_str();
    let base = format!("/engine/storage-stress/{run_id}/tenants/{tenant}");
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
    *slot = Some(started.elapsed());
    output
}

fn operation_ref(args: &Args, worker_index: usize, operation_index: usize) -> String {
    format!(
        "{}:child-{}:worker-{worker_index}:op-{operation_index}",
        args.run_id.as_deref().unwrap_or("unknown-run"),
        args.child_index.unwrap_or(0)
    )
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
    insert_stage_latency(&mut output, "claim_run", stages.claim_run);
    insert_stage_latency(&mut output, "append_assistant", stages.append_assistant);
    insert_stage_latency(&mut output, "finalize_assistant", stages.finalize_assistant);
    insert_stage_latency(&mut output, "complete_run", stages.complete_run);
    insert_stage_latency(&mut output, "load_context", stages.load_context);
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
        if std::env::var_os("IRONCLAW_STORAGE_STRESS_DEBUG_ERRORS").is_some() {
            eprintln!(
                "[storage-stress] operation error bucket={} stage={}: {}",
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
