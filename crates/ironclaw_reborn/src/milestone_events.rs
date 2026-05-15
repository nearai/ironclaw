use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{DurableEventLog, EventError, RuntimeEvent};
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, MissionId, ProjectId, ResourceScope, TenantId, ThreadId,
    UserId,
};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::{
    LoopFailureKind, TurnRunId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, LoopHostMilestone, LoopHostMilestoneKind,
        LoopHostMilestoneSink,
    },
};

const MODEL_CAPABILITY_ID: &str = "loop.model";
const ASSISTANT_REPLY_CAPABILITY_ID: &str = "loop.assistant_reply";
const LOOP_RUN_CAPABILITY_ID: &str = "loop.run";

/// Scope authority bound into the sink at construction time.
///
/// Building this from a canonical thread scope prevents callers from stitching
/// runtime events together from an unrelated user or mission scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableLoopHostMilestoneScope {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
    run_id: Option<TurnRunId>,
}

impl DurableLoopHostMilestoneScope {
    pub fn from_thread_scope(thread_scope: &ThreadScope) -> Result<Self, AgentLoopHostError> {
        let Some(user_id) = thread_scope.owner_user_id.clone() else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "loop milestone event scope requires a thread owner user",
            ));
        };
        Ok(Self {
            tenant_id: thread_scope.tenant_id.clone(),
            user_id,
            agent_id: Some(thread_scope.agent_id.clone()),
            project_id: thread_scope.project_id.clone(),
            mission_id: thread_scope.mission_id.clone(),
            thread_id: None,
            run_id: None,
        })
    }

    pub fn from_thread_scope_for_run(
        thread_scope: &ThreadScope,
        thread_id: ThreadId,
        run_id: TurnRunId,
    ) -> Result<Self, AgentLoopHostError> {
        let mut scope = Self::from_thread_scope(thread_scope)?;
        scope.thread_id = Some(thread_id);
        scope.run_id = Some(run_id);
        Ok(scope)
    }

    fn resource_scope(
        &self,
        milestone: &LoopHostMilestone,
    ) -> Result<ResourceScope, AgentLoopHostError> {
        if milestone.scope.tenant_id != self.tenant_id
            || milestone.scope.agent_id != self.agent_id
            || milestone.scope.project_id != self.project_id
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "loop milestone scope does not match durable event scope",
            ));
        }
        match &self.thread_id {
            Some(thread_id) if milestone.scope.thread_id != *thread_id => {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::ScopeMismatch,
                    "loop milestone thread does not match durable event scope",
                ));
            }
            _ => {}
        }
        match &self.run_id {
            Some(run_id) if milestone.run_id != *run_id => {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::ScopeMismatch,
                    "loop milestone run does not match durable event scope",
                ));
            }
            _ => {}
        }
        Ok(ResourceScope {
            tenant_id: self.tenant_id.clone(),
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            project_id: self.project_id.clone(),
            mission_id: self.mission_id.clone(),
            thread_id: Some(milestone.scope.thread_id.clone()),
            invocation_id: InvocationId::from_uuid(milestone.run_id.as_uuid()),
        })
    }
}

/// Durable projection adapter for public AgentLoopHost milestones.
///
/// The adapter writes only metadata-only loop lifecycle milestones into the
/// runtime event log. Progress milestones that carry useful counters or typed
/// checkpoint/prompt metadata stay in the milestone-sink substrate rather than
/// being collapsed into lossy `RuntimeEvent` rows. Raw prompts, assistant
/// content, provider errors, message refs, host paths, and secrets stay in
/// their owning stores and never enter runtime events.
#[derive(Clone)]
pub struct DurableLoopHostMilestoneSink {
    event_log: Arc<dyn DurableEventLog>,
    scope: DurableLoopHostMilestoneScope,
}

impl DurableLoopHostMilestoneSink {
    pub fn new(event_log: Arc<dyn DurableEventLog>, scope: DurableLoopHostMilestoneScope) -> Self {
        Self { event_log, scope }
    }

    pub fn event_log(&self) -> Arc<dyn DurableEventLog> {
        Arc::clone(&self.event_log)
    }

    fn resource_scope(
        &self,
        milestone: &LoopHostMilestone,
    ) -> Result<ResourceScope, AgentLoopHostError> {
        self.scope.resource_scope(milestone)
    }
}

impl std::fmt::Debug for DurableLoopHostMilestoneSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableLoopHostMilestoneSink")
            .field("event_log", &"<durable_event_log>")
            .field("scope", &self.scope)
            .finish()
    }
}

#[async_trait]
impl LoopHostMilestoneSink for DurableLoopHostMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        let Some(event) = self.runtime_event_for_milestone(&milestone)? else {
            return Ok(());
        };
        self.event_log
            .append(event)
            .await
            .map(|_| ())
            .map_err(durable_event_error)
    }
}

impl DurableLoopHostMilestoneSink {
    fn runtime_event_for_milestone(
        &self,
        milestone: &LoopHostMilestone,
    ) -> Result<Option<RuntimeEvent>, AgentLoopHostError> {
        let scope = self.resource_scope(milestone)?;
        let event = match &milestone.kind {
            LoopHostMilestoneKind::ModelStarted { .. } => {
                RuntimeEvent::model_started(scope, capability_id(MODEL_CAPABILITY_ID)?)
            }
            LoopHostMilestoneKind::ModelCompleted { .. } => {
                RuntimeEvent::model_completed(scope, capability_id(MODEL_CAPABILITY_ID)?)
            }
            LoopHostMilestoneKind::ModelFailed { reason_kind } => RuntimeEvent::model_failed(
                scope,
                capability_id(MODEL_CAPABILITY_ID)?,
                reason_kind.as_str(),
            ),
            LoopHostMilestoneKind::AssistantReplyFinalized { .. } => {
                RuntimeEvent::assistant_reply_finalized(
                    scope,
                    capability_id(ASSISTANT_REPLY_CAPABILITY_ID)?,
                )
            }
            LoopHostMilestoneKind::Completed { .. } => {
                RuntimeEvent::loop_completed(scope, capability_id(LOOP_RUN_CAPABILITY_ID)?)
            }
            LoopHostMilestoneKind::Failed { reason_kind, .. } => RuntimeEvent::loop_failed(
                scope,
                capability_id(LOOP_RUN_CAPABILITY_ID)?,
                loop_failure_kind(reason_kind),
            ),
            // PromptBundleBuilt and CheckpointCreated are suppressed here intentionally.
            // Checkpoint durability is owned by LoopCheckpointPort::write_checkpoint; the
            // CheckpointCreated runtime-event milestone is emitted there with the authoritative
            // durable payload. The CheckpointWritten progress event is an advisory echo only —
            // emitting it here would create a duplicate weaker record. WS-10 resume must rely
            // on the checkpoint-port milestone, NOT this progress echo.
            // Similarly, PromptBundleBuilt is emitted by LoopPromptPort with richer context.
            LoopHostMilestoneKind::IterationStarted { .. }
            | LoopHostMilestoneKind::PromptBundleBuilt { .. }
            | LoopHostMilestoneKind::CapabilityInvoked { .. }
            | LoopHostMilestoneKind::CapabilityBatchStarted { .. }
            | LoopHostMilestoneKind::CapabilityBatchCompleted { .. }
            | LoopHostMilestoneKind::GateBlocked { .. }
            | LoopHostMilestoneKind::CheckpointCreated { .. }
            | LoopHostMilestoneKind::Blocked { .. }
            | LoopHostMilestoneKind::DriverNote { .. } => return Ok(None),
        };
        Ok(Some(event))
    }
}

fn loop_failure_kind(reason_kind: &LoopFailureKind) -> &'static str {
    match reason_kind {
        LoopFailureKind::ModelError => "model_error",
        LoopFailureKind::ContextBuildFailed => "context_build_failed",
        LoopFailureKind::CapabilityProtocolError => "capability_protocol_error",
        LoopFailureKind::IterationLimit => "iteration_limit",
        LoopFailureKind::InvalidModelOutput => "invalid_model_output",
        LoopFailureKind::CheckpointRejected => "checkpoint_rejected",
        LoopFailureKind::TranscriptWriteFailed => "transcript_write_failed",
        LoopFailureKind::DriverBug => "driver_bug",
        LoopFailureKind::InterruptedUnexpectedly => "interrupted_unexpectedly",
        LoopFailureKind::NoProgressDetected => "no_progress_detected",
        LoopFailureKind::PolicyDenied => "policy_denied",
        // LoopFailureKind is `#[non_exhaustive]`; fail closed if a new variant
        // lands in `ironclaw_turns` ahead of this matcher being updated.
        _ => "driver_bug",
    }
}

fn capability_id(value: &'static str) -> Result<CapabilityId, AgentLoopHostError> {
    CapabilityId::new(value).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "loop milestone event capability id is invalid",
        )
    })
}

fn durable_event_error(_error: EventError) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "loop milestone event log is unavailable",
    )
}
