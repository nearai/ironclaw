//! Host-side sink for `report_stage_result` capability invocations.
//!
//! [`GithubWorkflowStageResultSink`] derives the authoritative stage identity
//! from the trusted executing-thread metadata (written by
//! [`super::stage_turn_submitter`]), cross-checks the model-supplied wire fields,
//! validates the stage result, persists it, records the `StageCompleted` workflow
//! event, and wakes the poller so it re-ticks the run at the stage boundary.
//! [`WorkflowStageResultSinkSlot`] is the deferred-init slot the first-party
//! capability registry resolves through, and
//! [`insert_workflow_stage_result_handler`] wires the delegating handler into a
//! [`FirstPartyCapabilityRegistry`]. The shared stage-thread `kind` discriminator
//! and the capability-id const live in the parent module (`super::`).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_github_issue_workflow::{
    AcceptStageResultInput, AcceptStageResultOutcome, GithubIssueStage, GithubIssueStageRunId,
    GithubIssueWorkflowError, GithubIssueWorkflowPollerWakeSender, GithubIssueWorkflowRepository,
    GithubIssueWorkflowRunId, RecordWorkflowEventInput, StageCompletedPayload,
    WorkflowEventEnvelope, WorkflowEventSourceKind, issue_binding_ref, stage_result_reported_key,
    validate_stage_result,
};
use ironclaw_host_api::{AgentId, CapabilityId};
use ironclaw_host_runtime::{
    ExecutingStageThread, FirstPartyCapabilityError, FirstPartyCapabilityHandler,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
    ReportWorkflowStageResultInput, WorkflowStageResultAck, WorkflowStageResultSink,
    WorkflowStageResultSinkError, builtin_first_party_handlers_with_workflow_stage_result_sink,
};
use ironclaw_threads::{
    SessionThreadError, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::TurnRunId;
use serde_json::Value as JsonValue;
use tracing::debug;

use super::{GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND, RESULT_SINK_CAPABILITY_ID};

pub(crate) fn insert_workflow_stage_result_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    workflow_stage_result_sink_slot: Arc<WorkflowStageResultSinkSlot>,
) -> Result<(), ironclaw_host_api::HostApiError> {
    let capability_id = CapabilityId::new(RESULT_SINK_CAPABILITY_ID)?;
    let workflow_stage_result_sink: Arc<dyn WorkflowStageResultSink> =
        workflow_stage_result_sink_slot;
    let workflow_registry = builtin_first_party_handlers_with_workflow_stage_result_sink(
        trigger_repository,
        workflow_stage_result_sink,
    )?;
    let handler = workflow_registry.get(&capability_id).ok_or_else(|| {
        ironclaw_host_api::HostApiError::InvariantViolation {
            reason: format!(
                "workflow stage result helper did not register {RESULT_SINK_CAPABILITY_ID}"
            ),
        }
    })?;
    registry.insert_handler(
        capability_id,
        Arc::new(DelegatingWorkflowStageResultHandler { inner: handler }),
    );
    Ok(())
}

pub(crate) const GITHUB_ISSUE_WORKFLOW_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct WorkflowStageResultSinkSlot {
    inner: OnceLock<Arc<dyn WorkflowStageResultSink>>,
}

impl WorkflowStageResultSinkSlot {
    pub(crate) fn new() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }

    pub(crate) fn set(
        &self,
        sink: Arc<dyn WorkflowStageResultSink>,
    ) -> Result<(), Arc<dyn WorkflowStageResultSink>> {
        self.inner.set(sink)
    }
}

impl Default for WorkflowStageResultSinkSlot {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WorkflowStageResultSink for WorkflowStageResultSinkSlot {
    async fn report_stage_result(
        &self,
        executing_thread: ExecutingStageThread,
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        let Some(sink) = self.inner.get().cloned() else {
            return Err(WorkflowStageResultSinkError::Unavailable);
        };
        sink.report_stage_result(executing_thread, input).await
    }
}

struct DelegatingWorkflowStageResultHandler {
    inner: Arc<dyn FirstPartyCapabilityHandler>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for DelegatingWorkflowStageResultHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        self.inner.dispatch(request).await
    }
}

/// The authoritative stage identity the host derives from the trusted executing
/// thread's metadata. The model never supplies these — they are read back from
/// the thread the stage turn was dispatched into.
#[derive(serde::Deserialize)]
struct StageThreadBinding {
    kind: String,
    workflow_run_id: String,
    stage_run_id: String,
    stage: String,
}

pub(crate) struct GithubWorkflowStageResultSink {
    repository: Arc<dyn GithubIssueWorkflowRepository>,
    thread_service: Arc<dyn SessionThreadService>,
    default_agent_id: AgentId,
    // Required (not Option) per architecture.md rule #2: production always wires
    // a real sender, and tests construct a throwaway one via
    // `GithubIssueWorkflowPollerWakeReceiver::channel().0`. Fired right after a
    // StageCompleted event is recorded so the poller re-ticks the affected run
    // immediately rather than after a full poll interval.
    poller_wake: GithubIssueWorkflowPollerWakeSender,
}

impl GithubWorkflowStageResultSink {
    pub(crate) fn new(
        repository: Arc<dyn GithubIssueWorkflowRepository>,
        thread_service: Arc<dyn SessionThreadService>,
        default_agent_id: AgentId,
        poller_wake: GithubIssueWorkflowPollerWakeSender,
    ) -> Self {
        Self {
            repository,
            thread_service,
            default_agent_id,
            poller_wake,
        }
    }
}

#[async_trait]
impl WorkflowStageResultSink for GithubWorkflowStageResultSink {
    async fn report_stage_result(
        &self,
        executing_thread: ExecutingStageThread,
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError> {
        // The host stamps the executing thread scope. An absent thread id means
        // the result tool was invoked outside any stage turn — unauthenticated.
        let Some(thread_id) = executing_thread.scope.thread_id.clone() else {
            debug!("workflow stage result rejected: executing thread id is absent");
            return Err(WorkflowStageResultSinkError::MismatchedBinding);
        };

        // Reconstruct the thread scope EXACTLY as IronClawStageTurnSubmitter::
        // thread_scope wrote it, sourced from the trusted executing scope, so
        // read_thread's exact-scope ownership check matches the write side.
        let executing_scope = &executing_thread.scope;
        let thread_scope = ThreadScope {
            tenant_id: executing_scope.tenant_id.clone(),
            agent_id: executing_scope
                .agent_id
                .clone()
                .unwrap_or_else(|| self.default_agent_id.clone()),
            project_id: executing_scope.project_id.clone(),
            owner_user_id: Some(executing_scope.user_id.clone()),
            mission_id: executing_scope.mission_id.clone(),
        };

        let record = self
            .thread_service
            .read_thread(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(stage_result_thread_error)?;

        // Derive the AUTHORITATIVE stage identity from the trusted, host-written
        // thread metadata — never from the model-supplied input fields.
        let Some(metadata_json) = record.metadata_json else {
            debug!("workflow stage result rejected: executing thread carries no binding metadata");
            return Err(WorkflowStageResultSinkError::MismatchedBinding);
        };
        let binding: StageThreadBinding = serde_json::from_str(&metadata_json).map_err(|_| {
            debug!(
                "workflow stage result rejected: executing thread metadata is not a stage binding"
            );
            WorkflowStageResultSinkError::MismatchedBinding
        })?;
        if binding.kind != GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND {
            debug!(
                "workflow stage result rejected: executing thread is not a github issue workflow stage"
            );
            return Err(WorkflowStageResultSinkError::MismatchedBinding);
        }
        let workflow_run_id = GithubIssueWorkflowRunId::from_trusted(binding.workflow_run_id)
            .map_err(stage_result_invalid_input)?;
        let stage_run_id = GithubIssueStageRunId::from_trusted(binding.stage_run_id)
            .map_err(stage_result_invalid_input)?;
        let stage = serde_json::from_value::<GithubIssueStage>(JsonValue::String(binding.stage))
            .map_err(|error| WorkflowStageResultSinkError::InvalidInput {
                reason: format!("invalid stage in executing thread binding: {error}"),
            })?;

        // Validate the model-supplied wire fields and cross-check them against
        // the authoritative identity (defense in depth + clearer errors). The
        // completion_nonce is deliberately NOT checked: it is never injected
        // into a stage prompt, so it carries no authority — the thread binding
        // is the authority.
        // turn_run_id is non-authoritative (the host binds via the executing
        // thread); validate its FORMAT only when the model bothered to send it.
        if let Some(turn_run_id) = input.turn_run_id.as_deref() {
            TurnRunId::parse(turn_run_id).map_err(|error| {
                WorkflowStageResultSinkError::InvalidInput {
                    reason: format!("invalid turn_run_id: {error}"),
                }
            })?;
        }
        let input_stage =
            serde_json::from_value::<GithubIssueStage>(JsonValue::String(input.stage.clone()))
                .map_err(|error| WorkflowStageResultSinkError::InvalidInput {
                    reason: format!("invalid stage: {error}"),
                })?;
        // The input schema no longer requires the model to supply
        // workflow_run_id/stage_run_id — it has no authoritative source for them
        // (they are not injected into any stage prompt). Cross-check them against
        // the thread-derived authoritative ids ONLY when present; a
        // present-but-wrong id is still a hard MismatchedBinding (defense in
        // depth). `stage` is always cross-checked.
        let workflow_run_mismatch = input
            .workflow_run_id
            .as_deref()
            .is_some_and(|value| value != workflow_run_id.as_str());
        let stage_run_mismatch = input
            .stage_run_id
            .as_deref()
            .is_some_and(|value| value != stage_run_id.as_str());
        if workflow_run_mismatch || stage_run_mismatch || input_stage != stage {
            debug!(
                "workflow stage result rejected: model-supplied identity does not match the executing thread binding"
            );
            return Err(WorkflowStageResultSinkError::MismatchedBinding);
        }

        let validated =
            validate_stage_result(stage, &input.schema_version, input.result).map_err(|error| {
                WorkflowStageResultSinkError::ValidationFailed {
                    reason: error.to_string(),
                }
            })?;
        let result = serde_json::to_value(&validated.envelope).map_err(|error| {
            WorkflowStageResultSinkError::InvalidInput {
                reason: format!("validated stage result could not be serialized: {error}"),
            }
        })?;
        let now = Utc::now();
        // `input.stage_run_id` is now optional; the ack reports the authoritative
        // (thread-derived) stage run id, not the model-supplied value.
        let ack_stage_run_id = stage_run_id.as_str().to_string();

        debug!(
            workflow_run_id = workflow_run_id.as_str(),
            stage_run_id = stage_run_id.as_str(),
            "workflow stage result bound to executing thread; accepting"
        );

        match self
            .repository
            .accept_stage_result(AcceptStageResultInput {
                workflow_run_id: workflow_run_id.clone(),
                stage_run_id: stage_run_id.clone(),
                result: result.clone(),
                now,
            })
            .await
            .map_err(stage_result_repository_error)?
        {
            AcceptStageResultOutcome::Accepted { run } => {
                self.repository
                    .record_workflow_event(RecordWorkflowEventInput {
                        workflow_run_id,
                        workflow_event_type:
                            ironclaw_github_issue_workflow::GithubIssueWorkflowEventType::StageCompleted,
                        envelope: WorkflowEventEnvelope {
                            source_kind: WorkflowEventSourceKind::WorkflowInternal,
                            source_delivery_id: None,
                            provider: issue_binding_ref(&run.issue_ref).provider_ref,
                            observed_at: now,
                            provider_updated_at: None,
                            idempotency_key: stage_result_reported_key(
                                &stage_run_id,
                                &validated.schema_version,
                            ),
                            payload_schema: "stage.completed.v1".to_string(),
                            payload: serde_json::to_value(StageCompletedPayload {
                                stage_run_id,
                                stage: validated.stage,
                                schema_version: validated.schema_version,
                                result,
                            })
                            .map_err(|error| WorkflowStageResultSinkError::InvalidInput {
                                reason: format!(
                                    "stage completed workflow event could not be serialized: {error}"
                                ),
                            })?,
                        },
                    })
                    .await
                    .map_err(stage_result_repository_error)?;
                // Wake the poller so it re-ticks this run at the stage boundary
                // immediately instead of waiting up to a full poll interval.
                // Best-effort/edge-triggered: the interval fallback still covers
                // a dropped wake.
                self.poller_wake.wake();
                Ok(WorkflowStageResultAck {
                    accepted: true,
                    duplicate: false,
                    stage_run_id: ack_stage_run_id,
                })
            }
            AcceptStageResultOutcome::NotActiveStage { .. } => {
                Err(WorkflowStageResultSinkError::StageNotActive)
            }
            AcceptStageResultOutcome::Terminal => Err(WorkflowStageResultSinkError::StageNotActive),
        }
    }
}

fn stage_result_thread_error(error: SessionThreadError) -> WorkflowStageResultSinkError {
    match error {
        // The executing thread does not exist under the reconstructed scope, or
        // exists under a different scope: the result tool is not bound to the
        // stage it claims to complete.
        SessionThreadError::UnknownThread { .. }
        | SessionThreadError::ThreadScopeMismatch { .. } => {
            WorkflowStageResultSinkError::MismatchedBinding
        }
        // Backend/serialization faults are transient infrastructure errors, not
        // a binding decision.
        _ => WorkflowStageResultSinkError::Unavailable,
    }
}

fn stage_result_invalid_input(error: GithubIssueWorkflowError) -> WorkflowStageResultSinkError {
    WorkflowStageResultSinkError::InvalidInput {
        reason: error.to_string(),
    }
}

fn stage_result_repository_error(error: GithubIssueWorkflowError) -> WorkflowStageResultSinkError {
    match error {
        GithubIssueWorkflowError::InvalidId { .. }
        | GithubIssueWorkflowError::InvalidConfig { .. } => {
            WorkflowStageResultSinkError::InvalidInput {
                reason: error.to_string(),
            }
        }
        GithubIssueWorkflowError::PolicyDenied { .. } | GithubIssueWorkflowError::Policy { .. } => {
            WorkflowStageResultSinkError::MismatchedBinding
        }
        GithubIssueWorkflowError::ProviderRead { .. }
        | GithubIssueWorkflowError::ProviderRateLimited { .. }
        | GithubIssueWorkflowError::Repository { .. } => WorkflowStageResultSinkError::Unavailable,
    }
}

#[cfg(test)]
mod github_issue_workflow_stage_result_sink_tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};

    use async_trait::async_trait;
    use ironclaw_github_issue_workflow::{
        AdvanceWorkflowRunInput, ClaimRunnableWorkflowRunsInput, CreateDraftPullRequestInput,
        CreateIssueCommentInput, CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome,
        CreateStageRunInput, GetAuthenticatedWorkflowActorInput, GithubActorSnapshot,
        GithubCommentRef, GithubIssueCommentSnapshot, GithubIssueRef, GithubIssueStage,
        GithubIssueWorkflowError, GithubIssueWorkflowEventType, GithubIssueWorkflowMode,
        GithubIssueWorkflowPolicy, GithubIssueWorkflowPolicyPorts,
        GithubIssueWorkflowPollerWakeReceiver, GithubIssueWorkflowPollerWakeSender,
        GithubIssueWorkflowRepository, GithubIssueWorkflowRun, GithubIssueWorkflowRunKey,
        GithubIssueWorkspaceSession, GithubIssueWorkspaceSessionId, GithubProviderAccountRef,
        GithubPullRequestRef, GithubPullRequestSnapshot, GithubRepositorySelector,
        InMemoryGithubIssueWorkflowRepository, ListIssueCommentsInput, ListPullRequestsInput,
        ListWorkflowEventsAfterInput, PrepareWorkflowWorkspaceOutcome,
        PrepareWorkflowWorkspaceRequest, PublishWorkflowWorkspaceOutcome,
        PublishWorkflowWorkspaceRequest, StageTurnSubmitter, SubmitStageTurnOutcome,
        SubmitStageTurnRequest, TransitionOutcome, WorkflowClock, WorkflowEventSourceKind,
        WorkflowProjectAccess, WorkflowProjectAccessRequest, WorkflowRunTransition,
        WorkflowWorkerId, WorkflowWorkspaceManager, WorkflowWorkspaceMountRef,
        WorkflowWorkspaceRef,
    };
    use ironclaw_host_api::{
        AgentId, InvocationId, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
    };
    use ironclaw_host_runtime::{
        ExecutingStageThread, ReportWorkflowStageResultInput, WorkflowStageResultSink,
    };
    use ironclaw_threads::{
        EnsureThreadRequest, InMemorySessionThreadService, SessionThreadService, ThreadScope,
    };
    use ironclaw_turns::{TurnRunId, TurnScope};
    use serde_json::json;
    use tokio::sync::Mutex;

    use super::GithubWorkflowStageResultSink;
    use crate::github_issue_workflow::GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND;

    fn sink_tenant() -> TenantId {
        TenantId::new("tenant-stage-result-sink").unwrap()
    }

    fn sink_user() -> UserId {
        UserId::new("user-stage-result-sink").unwrap()
    }

    fn sink_agent() -> AgentId {
        AgentId::new("agent-stage-result-sink").unwrap()
    }

    fn sink_project() -> ProjectId {
        ProjectId::new("project-stage-result-sink").unwrap()
    }

    /// A throwaway wake sender for sink tests that do not assert on the wake.
    /// The receiver is dropped immediately; `wake()` is a no-op `notify_one`,
    /// which is safe on a disconnected `Notify`.
    fn test_poller_wake() -> GithubIssueWorkflowPollerWakeSender {
        GithubIssueWorkflowPollerWakeReceiver::channel().0
    }

    fn stage_thread_id(
        workflow_run_id: &ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        stage_run_id: &ironclaw_github_issue_workflow::GithubIssueStageRunId,
    ) -> ThreadId {
        ThreadId::new(format!(
            "github-issue-workflow:{}:stage:{}",
            workflow_run_id.as_str(),
            stage_run_id.as_str()
        ))
        .unwrap()
    }

    fn stage_thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: sink_tenant(),
            agent_id: sink_agent(),
            project_id: Some(sink_project()),
            owner_user_id: Some(sink_user()),
            mission_id: None,
        }
    }

    /// Builds the trusted executing-thread scope the host would stamp for a turn
    /// running inside the stage thread of `(workflow_run_id, stage_run_id)`. It
    /// reconstructs to exactly `stage_thread_scope()`.
    fn executing_thread_for(
        workflow_run_id: &ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        stage_run_id: &ironclaw_github_issue_workflow::GithubIssueStageRunId,
    ) -> ExecutingStageThread {
        ExecutingStageThread {
            scope: ResourceScope {
                tenant_id: sink_tenant(),
                user_id: sink_user(),
                agent_id: Some(sink_agent()),
                project_id: Some(sink_project()),
                mission_id: None,
                thread_id: Some(stage_thread_id(workflow_run_id, stage_run_id)),
                invocation_id: InvocationId::new(),
            },
        }
    }

    /// Creates the stage thread (with the `kind = github_issue_workflow_stage`
    /// binding metadata) that the sink reads to derive authoritative identity.
    async fn seed_stage_thread(
        thread_service: &InMemorySessionThreadService,
        workflow_run_id: &ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        stage_run_id: &ironclaw_github_issue_workflow::GithubIssueStageRunId,
        stage: GithubIssueStage,
    ) {
        let metadata = serde_json::to_string(&json!({
            "kind": GITHUB_ISSUE_WORKFLOW_STAGE_THREAD_KIND,
            "workflow_run_id": workflow_run_id.as_str(),
            "stage_run_id": stage_run_id.as_str(),
            "stage": stage_name(&stage),
        }))
        .unwrap();
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: stage_thread_scope(),
                thread_id: Some(stage_thread_id(workflow_run_id, stage_run_id)),
                created_by_actor_id: sink_user().as_str().to_string(),
                title: Some("github issue workflow stage".to_string()),
                metadata_json: Some(metadata),
            })
            .await
            .expect("seed stage thread");
    }

    #[tokio::test]
    async fn stage_result_sink_accepts_and_records_event_for_matching_executing_thread() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        let ack = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: Some("nonce-triage".to_string()),
                    result: json!({
                        "outcome": "completed",
                        "summary": "triage completed",
                        "evidence": [],
                        "next_actions": [],
                        "payload": {
                            "is_reproducible": true,
                            "suspected_area": "composition sink",
                            "risk": "medium",
                            "recommended_next_stage": "planning"
                        }
                    }),
                },
            )
            .await
            .expect("stage result should be accepted");

        assert!(ack.accepted);
        assert!(!ack.duplicate);
        assert_eq!(ack.stage_run_id, stage_run_id.as_str());

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id,
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list workflow events");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].workflow_event_type,
            GithubIssueWorkflowEventType::StageCompleted
        );
        assert_eq!(
            events[0].source_kind,
            WorkflowEventSourceKind::WorkflowInternal
        );
        assert_eq!(events[0].payload_schema, "stage.completed.v1");
        assert_eq!(events[0].payload["schema_version"], "triage.v1");
        assert_eq!(
            events[0].payload["result"]["payload"]["is_reproducible"],
            true
        );
    }

    #[tokio::test]
    async fn stage_result_sink_wakes_poller_after_recording_stage_completed() {
        // A1: the sink must fire the poller wake right after recording the
        // StageCompleted event so the poller re-ticks the run at the stage
        // boundary instead of after a full poll interval.
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let (wake_sender, wake_receiver) = GithubIssueWorkflowPollerWakeReceiver::channel();
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            wake_sender,
        );

        let ack = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: Some("nonce-triage".to_string()),
                    result: triage_result(),
                },
            )
            .await
            .expect("stage result should be accepted");
        assert!(ack.accepted);

        // The wake was fired during `report_stage_result`. Because `Notify`
        // retains a single pending permit, a `notified()` issued after the fact
        // resolves immediately; a missing wake would make this future hang, so
        // a short timeout asserts the wake arrived.
        tokio::time::timeout(std::time::Duration::from_secs(1), wake_receiver.notified())
            .await
            .expect("poller wake should have been fired after recording StageCompleted");
    }

    #[tokio::test]
    async fn stage_result_sink_rejects_when_executing_thread_targets_other_active_stage() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        // Stage A: the stage whose thread the turn is actually executing in.
        let (workflow_run_id_a, stage_run_id_a) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        // Stage B: a different run's active stage the model tries to complete.
        let other_issue = GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 9999,
            node_id: Some("issue-node-stage-result-sink-other".to_string()),
            url: "https://github.com/nearai/ironclaw/issues/9999".to_string(),
            default_branch: "main".to_string(),
        };
        let (workflow_run_id_b, stage_run_id_b) =
            create_active_stage_with_issue(&repository, GithubIssueStage::Triage, other_issue)
                .await;

        let thread_service = Arc::new(InMemorySessionThreadService::default());
        // Only stage A's thread exists and is the executing thread.
        seed_stage_thread(
            &thread_service,
            &workflow_run_id_a,
            &stage_run_id_a,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        // Turn executes in stage A's thread but reports stage B's identity.
        let error = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id_a, &stage_run_id_a),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id_b.as_str().to_string()),
                    stage_run_id: Some(stage_run_id_b.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: Some("nonce-triage".to_string()),
                    result: triage_result(),
                },
            )
            .await
            .expect_err("reporting another stage's result from this thread must be rejected");
        assert!(matches!(
            error,
            ironclaw_host_runtime::WorkflowStageResultSinkError::MismatchedBinding
        ));

        // Neither run advanced: no stage was accepted.
        for run_id in [workflow_run_id_a, workflow_run_id_b] {
            let events = repository
                .list_workflow_events_after(ListWorkflowEventsAfterInput {
                    workflow_run_id: run_id,
                    after_sequence: 0,
                    limit: 10,
                })
                .await
                .expect("list workflow events");
            assert!(events.is_empty());
        }
    }

    #[tokio::test]
    async fn stage_result_sink_rejects_when_executing_thread_id_absent() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        // An executing thread with no thread id is unauthenticated.
        let mut executing = executing_thread_for(&workflow_run_id, &stage_run_id);
        executing.scope.thread_id = None;

        let error = sink
            .report_stage_result(
                executing,
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: Some("nonce-triage".to_string()),
                    result: triage_result(),
                },
            )
            .await
            .expect_err("an absent executing thread id must be rejected");
        assert!(matches!(
            error,
            ironclaw_host_runtime::WorkflowStageResultSinkError::MismatchedBinding
        ));

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id,
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list workflow events");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn stage_result_sink_ignores_model_supplied_nonce() {
        // Both a garbage nonce and an empty nonce are accepted when the thread
        // binding matches: the nonce carries no authority — the host-derived
        // thread binding is the authority.
        for nonce in ["totally-bogus-nonce", ""] {
            let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
            let (workflow_run_id, stage_run_id) =
                create_active_stage(&repository, GithubIssueStage::Triage).await;
            let thread_service = Arc::new(InMemorySessionThreadService::default());
            seed_stage_thread(
                &thread_service,
                &workflow_run_id,
                &stage_run_id,
                GithubIssueStage::Triage,
            )
            .await;
            let sink = GithubWorkflowStageResultSink::new(
                repository.clone(),
                thread_service.clone(),
                sink_agent(),
                test_poller_wake(),
            );

            let ack = sink
                .report_stage_result(
                    executing_thread_for(&workflow_run_id, &stage_run_id),
                    ReportWorkflowStageResultInput {
                        workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                        stage_run_id: Some(stage_run_id.as_str().to_string()),
                        turn_run_id: Some(TurnRunId::new().to_string()),
                        stage: "triage".to_string(),
                        schema_version: "triage.v1".to_string(),
                        completion_nonce: Some(nonce.to_string()),
                        result: triage_result(),
                    },
                )
                .await
                .expect("matching thread binding must accept regardless of the nonce");
            assert!(ack.accepted);
        }
    }

    #[tokio::test]
    async fn stage_result_sink_accepts_when_model_omits_optional_identity_fields() {
        // The input schema no longer requires workflow_run_id/stage_run_id/
        // turn_run_id/completion_nonce — the model is never told them. Supplying
        // only {stage, schema_version, result} must succeed (the host derives the
        // authoritative identity from the executing thread), and the ack must
        // carry the authoritative stage run id.
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        let ack = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: None,
                    stage_run_id: None,
                    turn_run_id: None,
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: None,
                    result: triage_result(),
                },
            )
            .await
            .expect("omitting the optional identity fields must be accepted");
        assert!(ack.accepted);
        assert_eq!(ack.stage_run_id, stage_run_id.as_str());

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id,
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list workflow events");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].workflow_event_type,
            GithubIssueWorkflowEventType::StageCompleted
        );
    }

    #[tokio::test]
    async fn stage_result_sink_accepts_executing_scope_from_real_turn_scope_conversion() {
        // Regression guard for the #4 scope match. Instead of a hand-authored
        // executing scope (which would mask divergence), derive it the way the
        // runtime does: build the SAME `TurnScope` the submitter writes, then run
        // the REAL `TurnScope::to_resource_scope()` conversion. If that conversion
        // ever stops reconstructing to the persisted thread scope (e.g. starts
        // setting mission_id, or maps the owner differently), read_thread's
        // exact-scope check fails and this test catches it before a live run.
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Triage).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Triage,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        // Mirror IronClawStageTurnSubmitter::submit_accepted_message: it builds the
        // turn scope from the thread scope it wrote, and the runtime stamps the
        // executing ResourceScope via TurnScope::to_resource_scope().
        let write_scope = stage_thread_scope();
        let turn_scope = TurnScope::new_with_owner(
            write_scope.tenant_id.clone(),
            Some(write_scope.agent_id.clone()),
            write_scope.project_id.clone(),
            stage_thread_id(&workflow_run_id, &stage_run_id),
            write_scope.owner_user_id.clone(),
        );
        let executing = ExecutingStageThread {
            scope: turn_scope.to_resource_scope(),
        };

        let ack = sink
            .report_stage_result(
                executing,
                ReportWorkflowStageResultInput {
                    workflow_run_id: None,
                    stage_run_id: None,
                    turn_run_id: None,
                    stage: "triage".to_string(),
                    schema_version: "triage.v1".to_string(),
                    completion_nonce: None,
                    result: triage_result(),
                },
            )
            .await
            .expect("real TurnScope::to_resource_scope() must reconstruct to the write-side scope");
        assert!(ack.accepted);
    }

    #[tokio::test]
    async fn stage_result_sink_rejects_invalid_implementation_without_recording_event() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let (workflow_run_id, stage_run_id) =
            create_active_stage(&repository, GithubIssueStage::Implementation).await;
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        seed_stage_thread(
            &thread_service,
            &workflow_run_id,
            &stage_run_id,
            GithubIssueStage::Implementation,
        )
        .await;
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );

        let error = sink
            .report_stage_result(
                executing_thread_for(&workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: "implementation".to_string(),
                    schema_version: "implementation.v1".to_string(),
                    completion_nonce: Some("nonce-implementation".to_string()),
                    result: json!({
                        "outcome": "completed",
                        "summary": "implementation claims PR readiness without commands",
                        "evidence": [],
                        "next_actions": [],
                        "payload": {
                            "changed_files": ["src/lib.rs"],
                            "test_evidence": ["not enough"],
                            "pr_ready": true
                        }
                    }),
                },
            )
            .await
            .expect_err("missing commands_run must fail validation");

        assert!(matches!(
            error,
            ironclaw_host_runtime::WorkflowStageResultSinkError::ValidationFailed { .. }
        ));

        let events = repository
            .list_workflow_events_after(ListWorkflowEventsAfterInput {
                workflow_run_id,
                after_sequence: 0,
                limit: 10,
            })
            .await
            .expect("list workflow events");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn stage_result_sink_events_drive_policy_to_draft_pr_once() {
        let repository = Arc::new(InMemoryGithubIssueWorkflowRepository::default());
        let github = Arc::new(FakeGithubPort::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let sink = GithubWorkflowStageResultSink::new(
            repository.clone(),
            thread_service.clone(),
            sink_agent(),
            test_poller_wake(),
        );
        let policy = GithubIssueWorkflowPolicy::new(
            FakePolicyPorts {
                repository: repository.clone(),
                github: github.clone(),
                stage_turns: Arc::new(FakeStageTurnSubmitter::default()),
                project_access: Arc::new(FakeProjectAccess),
                workspace: Arc::new(FakeWorkspaceManager),
                clock: Arc::new(FakeClock::new()),
                worker_id: worker(),
            },
            "stage-result-smoke-v1",
        );

        let run = create_claimed_run(&repository).await;
        let run = set_mode(
            &repository,
            run,
            GithubIssueWorkflowMode::Implementation,
            None,
        )
        .await;
        let run = attach_workspace_session(&repository, run).await;
        let run = create_stage_run(&repository, run, GithubIssueStage::Implementation).await;
        report_stage_result(
            &sink,
            &thread_service,
            &run,
            GithubIssueStage::Implementation,
            "implementation.v1",
            implementation_result(),
        )
        .await;

        let implementation_run = current_run(&repository).await;
        let pr_synthesis = policy.tick(implementation_run).await.expect("policy tick");
        assert_eq!(
            pr_synthesis.run.workflow_state.mode,
            GithubIssueWorkflowMode::PrSynthesis
        );
        assert_eq!(policy.ports().stage_turns.requests().await.len(), 1);

        let run = current_run(&repository).await;
        report_stage_result(
            &sink,
            &thread_service,
            &run,
            GithubIssueStage::PrSynthesis,
            "pr_synthesis.v1",
            pr_synthesis_result(),
        )
        .await;

        let pr_run = current_run(&repository).await;
        let first = policy.tick(pr_run).await.expect("draft PR policy tick");
        let second = policy.tick(first.run.clone()).await.expect("replay tick");

        assert_eq!(
            first.run.workflow_state.mode,
            GithubIssueWorkflowMode::PrOpen
        );
        assert_eq!(second.processed_event_count, 0);
        assert_eq!(github.created_prs().await.len(), 1);
        assert_eq!(
            first
                .run
                .workflow_state
                .primary_pr
                .as_ref()
                .map(|pull_request| pull_request.number),
            Some(123)
        );
    }

    async fn create_active_stage(
        repository: &InMemoryGithubIssueWorkflowRepository,
        stage: GithubIssueStage,
    ) -> (
        ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        ironclaw_github_issue_workflow::GithubIssueStageRunId,
    ) {
        create_active_stage_with_issue(repository, stage, issue()).await
    }

    async fn create_active_stage_with_issue(
        repository: &InMemoryGithubIssueWorkflowRepository,
        stage: GithubIssueStage,
        issue_ref: GithubIssueRef,
    ) -> (
        ironclaw_github_issue_workflow::GithubIssueWorkflowRunId,
        ironclaw_github_issue_workflow::GithubIssueStageRunId,
    ) {
        let run = match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: TenantId::new("tenant-stage-result-sink").unwrap(),
                creator_user_id: UserId::new("user-stage-result-sink").unwrap(),
                agent_id: Some(AgentId::new("agent-stage-result-sink").unwrap()),
                project_id: Some(ProjectId::new("project-stage-result-sink").unwrap()),
                provider_account_ref: None,
                issue_ref,
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "stage-result-sink-test".to_string(),
                now: chrono::Utc::now(),
            })
            .await
            .expect("create workflow run")
        {
            ironclaw_github_issue_workflow::CreateOrGetWorkflowRunOutcome::Created { run }
            | ironclaw_github_issue_workflow::CreateOrGetWorkflowRunOutcome::Existing { run } => {
                run
            }
        };
        assert_eq!(
            run.workflow_run_key,
            GithubIssueWorkflowRunKey::for_issue(&run.issue_ref).expect("workflow run key")
        );

        match repository
            .create_stage_run(CreateStageRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                stage,
                now: chrono::Utc::now(),
            })
            .await
            .expect("create stage run")
        {
            ironclaw_github_issue_workflow::CreateStageRunOutcome::Created {
                stage_run_id, ..
            }
            | ironclaw_github_issue_workflow::CreateStageRunOutcome::ActiveStageExists {
                existing_stage_run_id: stage_run_id,
                ..
            } => (run.workflow_run_id, stage_run_id),
            ironclaw_github_issue_workflow::CreateStageRunOutcome::Terminal => {
                panic!("new run should not be terminal")
            }
        }
    }

    fn issue() -> GithubIssueRef {
        GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 4242,
            node_id: Some("issue-node-stage-result-sink".to_string()),
            url: "https://github.com/nearai/ironclaw/issues/4242".to_string(),
            default_branch: "main".to_string(),
        }
    }

    async fn create_claimed_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
    ) -> GithubIssueWorkflowRun {
        let run = match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: TenantId::new("tenant-stage-result-sink").unwrap(),
                creator_user_id: UserId::new("user-stage-result-sink").unwrap(),
                agent_id: Some(AgentId::new("agent-stage-result-sink").unwrap()),
                project_id: Some(ProjectId::new("project-stage-result-sink").unwrap()),
                provider_account_ref: Some(provider_account_ref()),
                issue_ref: issue(),
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "stage-result-sink-test".to_string(),
                now: chrono::Utc::now(),
            })
            .await
            .expect("create workflow run")
        {
            CreateOrGetWorkflowRunOutcome::Created { run }
            | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
        };

        repository
            .claim_runnable_workflow_runs(ClaimRunnableWorkflowRunsInput {
                tenant_id: run.tenant_id.clone(),
                worker_id: worker(),
                now: chrono::Utc::now(),
                lease_expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
                limit: 1,
            })
            .await
            .expect("claim workflow run")
            .pop()
            .unwrap_or(run)
    }

    async fn current_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
    ) -> GithubIssueWorkflowRun {
        match repository
            .create_or_get_workflow_run(CreateOrGetWorkflowRunInput {
                tenant_id: TenantId::new("tenant-stage-result-sink").unwrap(),
                creator_user_id: UserId::new("user-stage-result-sink").unwrap(),
                agent_id: Some(AgentId::new("agent-stage-result-sink").unwrap()),
                project_id: Some(ProjectId::new("project-stage-result-sink").unwrap()),
                provider_account_ref: Some(provider_account_ref()),
                issue_ref: issue(),
                workflow_policy_key: "github-bug-workflow".to_string(),
                workflow_policy_version: "stage-result-sink-test".to_string(),
                now: chrono::Utc::now(),
            })
            .await
            .expect("get current workflow run")
        {
            CreateOrGetWorkflowRunOutcome::Created { run }
            | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
        }
    }

    async fn set_mode(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
        mode: GithubIssueWorkflowMode,
        primary_pr: Option<GithubPullRequestRef>,
    ) -> GithubIssueWorkflowRun {
        match repository
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                worker_id: worker(),
                expected_workflow_run_version: run.workflow_run_version,
                expected_event_cursor: run.event_cursor,
                next_event_cursor: run.event_cursor,
                transition: WorkflowRunTransition {
                    mode: Some(mode),
                    primary_pr,
                    clear_active_block: true,
                    ..WorkflowRunTransition::default()
                },
                now: chrono::Utc::now(),
            })
            .await
            .expect("set workflow mode")
        {
            TransitionOutcome::Applied { run } => run,
            other => panic!("mode transition should apply: {other:?}"),
        }
    }

    async fn attach_workspace_session(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
    ) -> GithubIssueWorkflowRun {
        let workspace_session_id =
            GithubIssueWorkspaceSessionId::from_trusted("workspace-session-smoke".to_string())
                .unwrap();
        let session = GithubIssueWorkspaceSession {
            workspace_session_id: workspace_session_id.clone(),
            workflow_run_id: run.workflow_run_id.clone(),
            repository: GithubRepositorySelector {
                owner: run.issue_ref.owner.clone(),
                repo: run.issue_ref.repo.clone(),
            },
            base_branch: run.issue_ref.default_branch.clone(),
            base_sha: None,
            working_branch: "ironclaw/fix-4242".to_string(),
            current_head_sha: Some("head-sha-4242".to_string()),
            workspace_ref: WorkflowWorkspaceRef {
                thread_id: None,
                workspace_session_id: Some(workspace_session_id),
                turn_run_id: None,
            },
            mount_ref: WorkflowWorkspaceMountRef {
                mount_id: "workspace-mount-smoke".to_string(),
                alias: "/workspace".to_string(),
            },
            created_at: chrono::Utc::now(),
        };
        match repository
            .advance_event_cursor_and_transition(AdvanceWorkflowRunInput {
                workflow_run_id: run.workflow_run_id.clone(),
                worker_id: worker(),
                expected_workflow_run_version: run.workflow_run_version,
                expected_event_cursor: run.event_cursor,
                next_event_cursor: run.event_cursor,
                transition: WorkflowRunTransition {
                    workspace_session: Some(session),
                    ..WorkflowRunTransition::default()
                },
                now: chrono::Utc::now(),
            })
            .await
            .expect("attach workspace session")
        {
            TransitionOutcome::Applied { run } => run,
            other => panic!("workspace session transition should apply: {other:?}"),
        }
    }

    async fn create_stage_run(
        repository: &InMemoryGithubIssueWorkflowRepository,
        run: GithubIssueWorkflowRun,
        stage: GithubIssueStage,
    ) -> GithubIssueWorkflowRun {
        match repository
            .create_stage_run(CreateStageRunInput {
                workflow_run_id: run.workflow_run_id,
                stage,
                now: chrono::Utc::now(),
            })
            .await
            .expect("create stage run")
        {
            ironclaw_github_issue_workflow::CreateStageRunOutcome::Created { run, .. }
            | ironclaw_github_issue_workflow::CreateStageRunOutcome::ActiveStageExists {
                run,
                ..
            } => run,
            ironclaw_github_issue_workflow::CreateStageRunOutcome::Terminal => {
                panic!("workflow run should not be terminal")
            }
        }
    }

    async fn report_stage_result(
        sink: &GithubWorkflowStageResultSink,
        thread_service: &InMemorySessionThreadService,
        run: &GithubIssueWorkflowRun,
        stage: GithubIssueStage,
        schema_version: &str,
        result: serde_json::Value,
    ) {
        let stage_run_id = run
            .active_stage_run_id
            .as_ref()
            .expect("active stage")
            .clone();
        seed_stage_thread(
            thread_service,
            &run.workflow_run_id,
            &stage_run_id,
            stage.clone(),
        )
        .await;
        let ack = sink
            .report_stage_result(
                executing_thread_for(&run.workflow_run_id, &stage_run_id),
                ReportWorkflowStageResultInput {
                    workflow_run_id: Some(run.workflow_run_id.as_str().to_string()),
                    stage_run_id: Some(stage_run_id.as_str().to_string()),
                    turn_run_id: Some(TurnRunId::new().to_string()),
                    stage: stage_name(&stage).to_string(),
                    schema_version: schema_version.to_string(),
                    completion_nonce: Some(format!("nonce-{schema_version}")),
                    result,
                },
            )
            .await
            .expect("stage result should be accepted");
        assert!(ack.accepted);
    }

    fn stage_name(stage: &GithubIssueStage) -> &'static str {
        match stage {
            GithubIssueStage::Triage => "triage",
            GithubIssueStage::Planning => "planning",
            GithubIssueStage::Implementation => "implementation",
            GithubIssueStage::PrSynthesis => "pr_synthesis",
            GithubIssueStage::CiRepair => "ci_repair",
            GithubIssueStage::ReviewResponse => "review_response",
        }
    }

    fn triage_result() -> serde_json::Value {
        json!({
            "outcome": "completed",
            "summary": "triage completed",
            "evidence": [],
            "next_actions": [],
            "payload": {
                "is_reproducible": true,
                "suspected_area": "composition sink",
                "risk": "medium",
                "recommended_next_stage": "planning"
            }
        })
    }

    fn implementation_result() -> serde_json::Value {
        json!({
            "outcome": "completed",
            "summary": "implementation completed",
            "evidence": [],
            "next_actions": [],
            "payload": {
                "changed_files": ["src/lib.rs"],
                "commands_run": ["cargo test"],
                "test_evidence": ["tests passed"],
                "pr_ready": true
            }
        })
    }

    fn pr_synthesis_result() -> serde_json::Value {
        json!({
            "outcome": "completed",
            "summary": "draft PR ready",
            "evidence": [],
            "next_actions": [],
            "payload": {
                "title": "Fix issue 4242",
                "body": "This fixes issue 4242.",
                "branch_name": "ironclaw/fix-4242",
                "base_branch": "main",
                "head_sha": "head-sha-4242"
            }
        })
    }

    fn provider_account_ref() -> GithubProviderAccountRef {
        GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: "github-stage-result-sink".to_string(),
        }
    }

    fn worker() -> WorkflowWorkerId {
        WorkflowWorkerId::from_trusted("worker-stage-result-sink".to_string()).unwrap()
    }

    struct FakeClock {
        now: StdMutex<chrono::DateTime<chrono::Utc>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                now: StdMutex::new(chrono::Utc::now()),
            }
        }
    }

    impl WorkflowClock for FakeClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            *self.now.lock().expect("clock lock")
        }
    }

    #[derive(Debug)]
    struct FakeGithubPort {
        created_prs: Mutex<Vec<CreateDraftPullRequestInput>>,
        create_pr_results: Mutex<VecDeque<Result<GithubPullRequestRef, GithubIssueWorkflowError>>>,
    }

    impl FakeGithubPort {
        fn new() -> Self {
            Self {
                created_prs: Mutex::new(Vec::new()),
                create_pr_results: Mutex::new(VecDeque::from([Ok(GithubPullRequestRef {
                    owner: "nearai".to_string(),
                    repo: "ironclaw".to_string(),
                    number: 123,
                    node_id: Some("pr-node-123".to_string()),
                    url: "https://github.com/nearai/ironclaw/pull/123".to_string(),
                    head_branch: "ironclaw/fix-4242".to_string(),
                    head_sha: Some("head-sha-4242".to_string()),
                })])),
            }
        }

        async fn created_prs(&self) -> Vec<CreateDraftPullRequestInput> {
            self.created_prs.lock().await.clone()
        }
    }

    #[async_trait]
    impl ironclaw_github_issue_workflow::GithubIssueWorkflowPort for FakeGithubPort {
        async fn get_authenticated_workflow_actor(
            &self,
            _input: GetAuthenticatedWorkflowActorInput,
        ) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
            Ok(GithubActorSnapshot {
                login: "ironclaw-bot".to_string(),
                node_id: Some("actor-node-stage-result-sink".to_string()),
            })
        }

        async fn list_issue_comments(
            &self,
            _input: ListIssueCommentsInput,
        ) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
            Ok(Vec::new())
        }

        async fn create_issue_comment(
            &self,
            _input: CreateIssueCommentInput,
        ) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
            Ok(GithubCommentRef {
                node_id: Some("comment-node-stage-result-sink".to_string()),
                url: "https://github.com/nearai/ironclaw/issues/4242#issuecomment-1".to_string(),
            })
        }

        async fn list_pull_requests(
            &self,
            _input: ListPullRequestsInput,
        ) -> Result<Vec<GithubPullRequestSnapshot>, GithubIssueWorkflowError> {
            Ok(Vec::new())
        }

        async fn create_draft_pull_request(
            &self,
            input: CreateDraftPullRequestInput,
        ) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
            self.created_prs.lock().await.push(input);
            self.create_pr_results
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| {
                    Err(GithubIssueWorkflowError::ProviderRead {
                        reason: "unexpected draft PR retry".to_string(),
                    })
                })
        }
    }

    #[derive(Debug, Default)]
    struct FakeProjectAccess;

    #[async_trait]
    impl WorkflowProjectAccess for FakeProjectAccess {
        async fn assert_workflow_project_access(
            &self,
            _request: WorkflowProjectAccessRequest,
        ) -> Result<(), GithubIssueWorkflowError> {
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakeWorkspaceManager;

    #[async_trait]
    impl WorkflowWorkspaceManager for FakeWorkspaceManager {
        async fn prepare_workspace(
            &self,
            request: PrepareWorkflowWorkspaceRequest,
        ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
            let workspace_session_id =
                GithubIssueWorkspaceSessionId::from_trusted("workspace-session-smoke".to_string())
                    .unwrap();
            Ok(PrepareWorkflowWorkspaceOutcome {
                session: GithubIssueWorkspaceSession {
                    workspace_session_id: workspace_session_id.clone(),
                    workflow_run_id: request.workflow_run_id,
                    repository: GithubRepositorySelector {
                        owner: request.issue.owner,
                        repo: request.issue.repo,
                    },
                    base_branch: request.base_branch,
                    base_sha: None,
                    working_branch: "ironclaw/fix-4242".to_string(),
                    current_head_sha: Some("head-sha-4242".to_string()),
                    workspace_ref: WorkflowWorkspaceRef {
                        thread_id: Some(ThreadId::new("workspace-thread-smoke").unwrap()),
                        workspace_session_id: Some(workspace_session_id),
                        turn_run_id: Some(TurnRunId::new()),
                    },
                    mount_ref: WorkflowWorkspaceMountRef {
                        mount_id: "workspace-mount-smoke".to_string(),
                        alias: "/workspace".to_string(),
                    },
                    created_at: request.requested_at,
                },
            })
        }

        async fn publish_workspace(
            &self,
            request: PublishWorkflowWorkspaceRequest,
        ) -> Result<PublishWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
            Ok(PublishWorkflowWorkspaceOutcome {
                working_branch: "ironclaw/fix-4242".to_string(),
                base_branch: request.base_branch,
                head_sha: "head-sha-4242".to_string(),
                has_changes: true,
            })
        }
    }

    #[derive(Debug, Default)]
    struct FakeStageTurnSubmitter {
        requests: Mutex<Vec<SubmitStageTurnRequest>>,
    }

    impl FakeStageTurnSubmitter {
        async fn requests(&self) -> Vec<SubmitStageTurnRequest> {
            self.requests.lock().await.clone()
        }
    }

    #[async_trait]
    impl StageTurnSubmitter for FakeStageTurnSubmitter {
        async fn submit_stage_turn(
            &self,
            request: SubmitStageTurnRequest,
        ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError> {
            let request_count = {
                let mut requests = self.requests.lock().await;
                requests.push(request);
                requests.len()
            };
            Ok(SubmitStageTurnOutcome::Submitted {
                thread_id: ThreadId::new(format!("thread-stage-result-sink-{request_count}"))
                    .unwrap(),
                turn_run_id: TurnRunId::new(),
            })
        }
    }

    struct FakePolicyPorts {
        repository: Arc<InMemoryGithubIssueWorkflowRepository>,
        github: Arc<FakeGithubPort>,
        stage_turns: Arc<FakeStageTurnSubmitter>,
        project_access: Arc<FakeProjectAccess>,
        workspace: Arc<FakeWorkspaceManager>,
        clock: Arc<FakeClock>,
        worker_id: WorkflowWorkerId,
    }

    impl GithubIssueWorkflowPolicyPorts for FakePolicyPorts {
        type Clock = FakeClock;
        type GithubPort = FakeGithubPort;
        type ProjectAccess = FakeProjectAccess;
        type Repository = InMemoryGithubIssueWorkflowRepository;
        type StageTurnSubmitter = FakeStageTurnSubmitter;
        type WorkspaceManager = FakeWorkspaceManager;

        fn clock(&self) -> Arc<Self::Clock> {
            self.clock.clone()
        }

        fn github_port(&self) -> Arc<Self::GithubPort> {
            self.github.clone()
        }

        fn project_access(&self) -> Arc<Self::ProjectAccess> {
            self.project_access.clone()
        }

        fn repository(&self) -> Arc<Self::Repository> {
            self.repository.clone()
        }

        fn stage_turn_submitter(&self) -> Arc<Self::StageTurnSubmitter> {
            self.stage_turns.clone()
        }

        fn workspace_manager(&self) -> Arc<Self::WorkspaceManager> {
            self.workspace.clone()
        }

        fn worker_id(&self) -> WorkflowWorkerId {
            self.worker_id.clone()
        }
    }
}
