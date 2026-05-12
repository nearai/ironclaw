use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use serde::{Deserialize, Serialize};

use crate::{
    LoopExitId, LoopGateRef, LoopMessageRef, TurnCheckpointId, TurnId, TurnRunId, TurnScope,
};

use super::host::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilitySurfaceVersion, LoopCheckpointKind,
    LoopDriverNoteKind, LoopPromptBundleRef, LoopRunContext, LoopSafeSummary, PromptMode,
};
use super::refs::{LoopDriverId, ModelProfileId};
use crate::{LoopCompletionKind, LoopFailureKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopHostMilestone {
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub loop_driver_id: LoopDriverId,
    pub kind: LoopHostMilestoneKind,
}

impl LoopHostMilestone {
    fn from_context(context: &LoopRunContext, kind: LoopHostMilestoneKind) -> Self {
        Self {
            scope: context.scope.clone(),
            turn_id: context.turn_id,
            run_id: context.run_id,
            loop_driver_id: context.loop_driver_id.clone(),
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSkillContextMetadata {
    pub ordinal: usize,
    pub source_name: String,
    pub trust_level: String,
}

/// Public wire shape for host-loop milestones.
///
/// Milestones may be serialized into traces or delivered across process
/// boundaries. Consumers must treat this enum as extensible and prefer
/// [`LoopHostMilestoneKind::kind_name`] plus a catch-all branch rather than
/// assuming the historical closed set. `PromptBundleBuilt` was added as an
/// additive wire-format variant for prompt-bundle construction; it carries only
/// refs, mode, optional surface version, counts, and active-skill metadata,
/// never raw prompt/model content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopHostMilestoneKind {
    PromptBundleBuilt {
        bundle_ref: LoopPromptBundleRef,
        mode: PromptMode,
        surface_version: Option<CapabilitySurfaceVersion>,
        message_count: usize,
        #[serde(default)]
        skill_context: Vec<PromptSkillContextMetadata>,
    },
    ModelStarted {
        requested_model_profile_id: Option<ModelProfileId>,
    },
    ModelCompleted {
        effective_model_profile_id: ModelProfileId,
    },
    ModelFailed {
        reason_kind: AgentLoopHostErrorKind,
    },
    CapabilityInvoked {
        capability_id: CapabilityId,
    },
    CheckpointCreated {
        checkpoint_id: TurnCheckpointId,
        checkpoint_kind: LoopCheckpointKind,
    },
    AssistantReplyFinalized {
        message_ref: LoopMessageRef,
    },
    Blocked {
        gate_ref: LoopGateRef,
        checkpoint_id: TurnCheckpointId,
    },
    Completed {
        completion_kind: LoopCompletionKind,
        exit_id: LoopExitId,
    },
    Failed {
        reason_kind: LoopFailureKind,
        exit_id: LoopExitId,
    },
    DriverNote {
        kind: LoopDriverNoteKind,
        safe_summary: LoopSafeSummary,
    },
}

impl LoopHostMilestoneKind {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::PromptBundleBuilt { .. } => "prompt_bundle_built",
            Self::ModelStarted { .. } => "model_started",
            Self::ModelCompleted { .. } => "model_completed",
            Self::ModelFailed { .. } => "model_failed",
            Self::CapabilityInvoked { .. } => "capability_invoked",
            Self::CheckpointCreated { .. } => "checkpoint_created",
            Self::AssistantReplyFinalized { .. } => "assistant_reply_finalized",
            Self::Blocked { .. } => "blocked",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::DriverNote { .. } => "driver_note",
        }
    }
}

#[async_trait]
pub trait LoopHostMilestoneSink: Send + Sync {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError>;
}

#[derive(Default)]
pub struct InMemoryLoopHostMilestoneSink {
    milestones: Mutex<Vec<LoopHostMilestone>>,
}

impl InMemoryLoopHostMilestoneSink {
    pub fn milestones(&self) -> Vec<LoopHostMilestone> {
        match self.milestones.lock() {
            Ok(milestones) => milestones.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait]
impl LoopHostMilestoneSink for InMemoryLoopHostMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        let mut milestones = self.milestones.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "loop milestone sink mutex poisoned",
            )
        })?;
        milestones.push(milestone);
        Ok(())
    }
}

#[derive(Clone)]
pub struct LoopHostMilestoneEmitter<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    context: LoopRunContext,
    sink: Arc<S>,
}

impl<S> LoopHostMilestoneEmitter<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    pub fn new(context: LoopRunContext, sink: Arc<S>) -> Self {
        Self { context, sink }
    }

    pub async fn prompt_bundle_built(
        &self,
        bundle_ref: LoopPromptBundleRef,
        mode: PromptMode,
        surface_version: Option<CapabilitySurfaceVersion>,
        message_count: usize,
        skill_context: Vec<PromptSkillContextMetadata>,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::PromptBundleBuilt {
            bundle_ref,
            mode,
            surface_version,
            message_count,
            skill_context,
        })
        .await
    }

    pub async fn model_started(
        &self,
        requested_model_profile_id: Option<ModelProfileId>,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id,
        })
        .await
    }

    pub async fn model_completed(
        &self,
        effective_model_profile_id: ModelProfileId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::ModelCompleted {
            effective_model_profile_id,
        })
        .await
    }

    pub async fn model_failed(
        &self,
        reason_kind: AgentLoopHostErrorKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::ModelFailed { reason_kind })
            .await
    }

    pub async fn capability_invoked(
        &self,
        capability_id: CapabilityId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CapabilityInvoked { capability_id })
            .await
    }

    pub async fn checkpoint_created(
        &self,
        checkpoint_id: TurnCheckpointId,
        checkpoint_kind: LoopCheckpointKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CheckpointCreated {
            checkpoint_id,
            checkpoint_kind,
        })
        .await
    }

    pub async fn assistant_reply_finalized(
        &self,
        message_ref: LoopMessageRef,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::AssistantReplyFinalized { message_ref })
            .await
    }

    pub async fn blocked(
        &self,
        gate_ref: LoopGateRef,
        checkpoint_id: TurnCheckpointId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::Blocked {
            gate_ref,
            checkpoint_id,
        })
        .await
    }

    pub async fn completed(
        &self,
        completion_kind: LoopCompletionKind,
        exit_id: LoopExitId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::Completed {
            completion_kind,
            exit_id,
        })
        .await
    }

    pub async fn failed(
        &self,
        reason_kind: LoopFailureKind,
        exit_id: LoopExitId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::Failed {
            reason_kind,
            exit_id,
        })
        .await
    }

    pub async fn driver_note(
        &self,
        kind: LoopDriverNoteKind,
        safe_summary: LoopSafeSummary,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::DriverNote { kind, safe_summary })
            .await
    }

    async fn publish(&self, kind: LoopHostMilestoneKind) -> Result<(), AgentLoopHostError> {
        self.sink
            .publish_loop_milestone(LoopHostMilestone::from_context(&self.context, kind))
            .await
    }
}
