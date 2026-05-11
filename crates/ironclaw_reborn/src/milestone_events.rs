use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::{DurableEventLog, EventError, RuntimeEvent};
use ironclaw_host_api::{CapabilityId, InvocationId, ResourceScope, UserId};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopHostMilestone, LoopHostMilestoneKind,
    LoopHostMilestoneSink,
};

const MODEL_CAPABILITY_ID: &str = "loop.model";
const ASSISTANT_REPLY_CAPABILITY_ID: &str = "loop.assistant_reply";

/// Durable projection adapter for public AgentLoopHost milestones.
///
/// The adapter writes only metadata-only model/reply milestones into the
/// runtime event log. Raw prompts, assistant content, provider errors, message
/// refs, host paths, and secrets stay in their owning stores and never enter
/// runtime events.
#[derive(Clone)]
pub struct DurableLoopHostMilestoneSink {
    event_log: Arc<dyn DurableEventLog>,
    user_id: UserId,
}

impl DurableLoopHostMilestoneSink {
    pub fn new(event_log: Arc<dyn DurableEventLog>, user_id: UserId) -> Self {
        Self { event_log, user_id }
    }

    pub fn event_log(&self) -> Arc<dyn DurableEventLog> {
        Arc::clone(&self.event_log)
    }

    fn resource_scope(&self, milestone: &LoopHostMilestone) -> ResourceScope {
        ResourceScope {
            tenant_id: milestone.scope.tenant_id.clone(),
            user_id: self.user_id.clone(),
            agent_id: milestone.scope.agent_id.clone(),
            project_id: milestone.scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(milestone.scope.thread_id.clone()),
            invocation_id: InvocationId::from_uuid(milestone.run_id.as_uuid()),
        }
    }
}

impl std::fmt::Debug for DurableLoopHostMilestoneSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableLoopHostMilestoneSink")
            .field("event_log", &"<durable_event_log>")
            .field("user_id", &self.user_id)
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
        let scope = self.resource_scope(milestone);
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
            LoopHostMilestoneKind::PromptBundleBuilt { .. }
            | LoopHostMilestoneKind::CapabilityInvoked { .. }
            | LoopHostMilestoneKind::CheckpointCreated { .. }
            | LoopHostMilestoneKind::Blocked { .. }
            | LoopHostMilestoneKind::Completed { .. }
            | LoopHostMilestoneKind::Failed { .. }
            | LoopHostMilestoneKind::DriverNote { .. } => return Ok(None),
        };
        Ok(Some(event))
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
