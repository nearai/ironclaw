use std::{
    future::Future,
    sync::Arc,
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
        let args = args.clone();
        handles.push((
            worker_index,
            tokio::spawn(async move {
                let mut samples = Vec::with_capacity(args.operations);
                for operation_index in 0..args.operations {
                    let sample = workload
                        .run_operation(&args, &identities, worker_index, operation_index)
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
    ) -> Sample {
        match self {
            #[cfg(feature = "libsql")]
            Self::Libsql(services) => {
                services
                    .run_operation(args, identities, worker_index, operation_index)
                    .await
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(services) => {
                services
                    .run_operation(args, identities, worker_index, operation_index)
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
    ) -> Sample {
        let mut stages = UserTurnStageDurations::default();
        let started = Instant::now();
        let outcome = self
            .run_operation_inner(args, identities, worker_index, operation_index, &mut stages)
            .await;
        Sample {
            latency: started.elapsed(),
            error: outcome.err().map(|failure| failure.bucket),
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
            .map_err(OperationFailure::invalid_request)?;
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
        .map_err(thread_failure)?;

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
        .map_err(thread_failure)?;

        let submit_result = time_stage(
            &mut stages.submit_turn,
            turn_coordinator.submit_turn(SubmitTurnRequest {
                scope: context.turn_scope.clone(),
                actor: TurnActor::new(context.user_id.clone()),
                accepted_message_ref: AcceptedMessageRef::new(accepted.message_id.to_string())
                    .map_err(OperationFailure::invalid_request)?,
                source_binding_ref: SourceBindingRef::new(source_binding)
                    .map_err(OperationFailure::invalid_request)?,
                reply_target_binding_ref: ReplyTargetBindingRef::new(reply_target)
                    .map_err(OperationFailure::invalid_request)?,
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new(format!("storage-stress:{operation_ref}"))
                    .map_err(OperationFailure::invalid_request)?,
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
                .map_err(thread_failure)?;
                return Err(turn_failure(error));
            }
            Err(error) => return Err(turn_failure(error)),
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
        .map_err(thread_failure)?;

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
        .map_err(turn_failure)?
        .ok_or_else(|| {
            OperationFailure::new("turn_claim_miss", "submitted run was not claimable")
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
        .map_err(thread_failure)?;

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
        .map_err(thread_failure)?;

        time_stage(
            &mut stages.complete_run,
            turn_store.complete_run(CompleteRunRequest {
                run_id: claimed.state.run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
            }),
        )
        .await
        .map_err(turn_failure)?;

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
        .map_err(thread_failure)?;

        Ok(())
    }

    fn turn_store_for_context(
        &self,
        context: &crate::synthetic::UserTurnContext,
    ) -> Result<Arc<FilesystemTurnStateStore<F>>, OperationFailure> {
        let view = user_turn_mount_view(&self.run_id, &context.turn_scope.to_resource_scope())
            .map_err(OperationFailure::invalid_request)?;
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

#[derive(Debug)]
struct OperationFailure {
    bucket: String,
}

impl OperationFailure {
    fn new(bucket: impl Into<String>, detail: impl std::fmt::Display) -> Self {
        let bucket = bucket.into();
        if std::env::var_os("IRONCLAW_STORAGE_STRESS_DEBUG_ERRORS").is_some() {
            eprintln!("[storage-stress] operation error bucket={bucket}: {detail}");
        }
        Self { bucket }
    }

    fn invalid_request(detail: impl std::fmt::Display) -> Self {
        Self::new("invalid_request", detail)
    }
}

fn thread_failure(error: SessionThreadError) -> OperationFailure {
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
    OperationFailure::new(bucket, error)
}

fn turn_failure(error: TurnError) -> OperationFailure {
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
    OperationFailure::new(bucket, error)
}
