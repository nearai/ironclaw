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
        AgentLoopHostError, AgentLoopHostErrorKind, HookDecisionSummary, LoopHostMilestone,
        LoopHostMilestoneKind, LoopHostMilestoneSink,
    },
};

const MODEL_CAPABILITY_ID: &str = "loop.model";
const ASSISTANT_REPLY_CAPABILITY_ID: &str = "loop.assistant_reply";
const LOOP_RUN_CAPABILITY_ID: &str = "loop.run";
const HOOK_CAPABILITY_ID: &str = "loop.hook";

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
/// The adapter writes only metadata-only model/reply milestones into the
/// runtime event log. Raw prompts, assistant content, provider errors, message
/// refs, host paths, and secrets stay in their owning stores and never enter
/// runtime events.
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
            // Hook telemetry is projected into the durable event log so audit
            // consumers can replay the same hook dispatched/decision/failed
            // trail that SSE observers see live. Only closed-vocabulary labels
            // and the blake3-hex hook identity cross into the event;
            // sanitized reasons stay in the hook milestone stream and do not
            // enter durable storage through this seam.
            LoopHostMilestoneKind::HookDispatched {
                hook_id,
                point,
                trust_class,
            } => RuntimeEvent::hook_dispatched(
                scope,
                capability_id(HOOK_CAPABILITY_ID)?,
                hook_id.clone(),
                point.clone(),
                trust_class.clone(),
            ),
            LoopHostMilestoneKind::HookDecisionEmitted { hook_id, decision } => {
                RuntimeEvent::hook_decision_emitted(
                    scope,
                    capability_id(HOOK_CAPABILITY_ID)?,
                    hook_id.clone(),
                    hook_decision_label(decision),
                )
            }
            LoopHostMilestoneKind::HookFailed {
                hook_id,
                category,
                disposition,
            } => RuntimeEvent::hook_failed(
                scope,
                capability_id(HOOK_CAPABILITY_ID)?,
                hook_id.clone(),
                category.clone(),
                disposition.clone(),
            ),
            LoopHostMilestoneKind::PromptBundleBuilt { .. }
            | LoopHostMilestoneKind::CapabilityInvoked { .. }
            | LoopHostMilestoneKind::CheckpointCreated { .. }
            | LoopHostMilestoneKind::Blocked { .. }
            | LoopHostMilestoneKind::DriverNote { .. } => return Ok(None),
        };
        Ok(Some(event))
    }
}

/// Render a [`HookDecisionSummary`] as the closed-vocabulary kind label
/// expected by [`RuntimeEvent::hook_decision_emitted`]. Sanitized reasons live
/// in the in-memory hook milestone stream only — durable runtime events carry
/// the kind label alone so that audit replay never depends on free-form reason
/// text.
fn hook_decision_label(decision: &HookDecisionSummary) -> &'static str {
    decision.kind_name()
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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_events::{InMemoryDurableEventLog, RuntimeEventKind};
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use ironclaw_threads::ThreadScope;
    use ironclaw_turns::{
        TurnId, TurnScope,
        run_profile::{HookDecisionSummary, LoopDriverId, LoopHostMilestone},
    };

    const HOOK_HEX_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn fixture_thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-hook-projection").unwrap(),
            agent_id: AgentId::new("agent-hook-projection").unwrap(),
            project_id: Some(ProjectId::new("project-hook-projection").unwrap()),
            owner_user_id: Some(UserId::new("user-hook-projection").unwrap()),
            mission_id: None,
        }
    }

    fn fixture_milestone(kind: LoopHostMilestoneKind) -> (LoopHostMilestone, ThreadId, TurnRunId) {
        let thread_id = ThreadId::new("thread-hook-projection").unwrap();
        let run_id = TurnRunId::new();
        let scope = TurnScope::new(
            TenantId::new("tenant-hook-projection").unwrap(),
            Some(AgentId::new("agent-hook-projection").unwrap()),
            Some(ProjectId::new("project-hook-projection").unwrap()),
            thread_id.clone(),
        );
        let milestone = LoopHostMilestone {
            scope,
            turn_id: TurnId::new(),
            run_id,
            loop_driver_id: LoopDriverId::new("hook-projection-driver").unwrap(),
            kind,
        };
        (milestone, thread_id, run_id)
    }

    fn projector_for(thread_id: ThreadId, run_id: TurnRunId) -> DurableLoopHostMilestoneSink {
        let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
        let milestone_scope = DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &fixture_thread_scope(),
            thread_id,
            run_id,
        )
        .expect("durable milestone scope requires owner user — fixture supplies one");
        DurableLoopHostMilestoneSink::new(event_log, milestone_scope)
    }

    #[test]
    fn hook_dispatched_milestone_projects_to_runtime_event() {
        let (milestone, thread_id, run_id) =
            fixture_milestone(LoopHostMilestoneKind::HookDispatched {
                hook_id: HOOK_HEX_ID.to_string(),
                point: "before_capability".to_string(),
                trust_class: "builtin".to_string(),
            });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("hook dispatched milestone now projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::HookDispatched);
        assert_eq!(
            event.capability_id,
            CapabilityId::new(HOOK_CAPABILITY_ID).unwrap()
        );
        assert_eq!(event.hook_id.as_deref(), Some(HOOK_HEX_ID));
        assert_eq!(event.hook_point.as_deref(), Some("before_capability"));
        assert_eq!(event.hook_trust_class.as_deref(), Some("builtin"));
        assert!(event.hook_decision.is_none());
        assert!(event.hook_failure_category.is_none());
    }

    #[test]
    fn hook_decision_emitted_milestone_projects_to_runtime_event() {
        let (milestone, thread_id, run_id) =
            fixture_milestone(LoopHostMilestoneKind::HookDecisionEmitted {
                hook_id: HOOK_HEX_ID.to_string(),
                // Reason text must NOT leak into the durable event — only the
                // closed-vocabulary `kind_name()` should be projected.
                decision: HookDecisionSummary::Deny {
                    reason: "policy-denied raw text".to_string(),
                },
            });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("hook decision milestone now projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::HookDecisionEmitted);
        assert_eq!(event.hook_decision.as_deref(), Some("deny"));
        assert_eq!(event.hook_id.as_deref(), Some(HOOK_HEX_ID));
        let wire = serde_json::to_string(&event).expect("serialize hook decision event");
        assert!(
            !wire.contains("policy-denied"),
            "raw decision reason leaked into durable event payload: {wire}"
        );
    }

    #[test]
    fn hook_failed_milestone_projects_to_runtime_event() {
        let (milestone, thread_id, run_id) = fixture_milestone(LoopHostMilestoneKind::HookFailed {
            hook_id: HOOK_HEX_ID.to_string(),
            category: "timeout".to_string(),
            disposition: "fail_closed".to_string(),
        });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("hook failed milestone now projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::HookFailed);
        assert_eq!(event.hook_failure_category.as_deref(), Some("timeout"));
        assert_eq!(
            event.hook_failure_disposition.as_deref(),
            Some("fail_closed")
        );
        assert_eq!(event.hook_id.as_deref(), Some(HOOK_HEX_ID));
    }
}
