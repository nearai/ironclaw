//! Planned Reborn loop driver.
//!
//! This module is the bridge from the runner-facing `AgentLoopDriver` trait to
//! the sealed `ironclaw_agent_loop` framework. It intentionally holds an opaque
//! `LoopFamily` and the canonical executor; it does not expose planner slots to
//! `ironclaw_reborn`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, AgentLoopExecutorError, CanonicalAgentLoopExecutor, HostStage},
    family::{LoopFamily, LoopFamilyId, LoopFamilyRegistry},
    state::{CHECKPOINT_SCHEMA_ID, CheckpointKind, LoopExecutionState},
};
use ironclaw_turns::{
    LoopExit, RunProfileVersion,
    run_profile::{
        AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverHost,
        AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest,
    },
};

const PLANNED_DRIVER_VERSION: u64 = 1;

/// Non-generic adapter from one resolved loop family to `AgentLoopDriver`.
pub struct PlannedDriver {
    descriptor: AgentLoopDriverDescriptor,
    family: Arc<LoopFamily>,
    executor: Arc<CanonicalAgentLoopExecutor>,
}

impl PlannedDriver {
    pub fn from_family(
        family: Arc<LoopFamily>,
        executor: Arc<CanonicalAgentLoopExecutor>,
        version: RunProfileVersion,
    ) -> Result<Self, AgentLoopDriverError> {
        let descriptor = descriptor_for_family(family.id(), version)?;
        Ok(Self {
            descriptor,
            family,
            executor,
        })
    }

    pub fn from_registry(
        registry: &LoopFamilyRegistry,
        id: &LoopFamilyId,
        executor: Arc<CanonicalAgentLoopExecutor>,
        version: RunProfileVersion,
    ) -> Result<Self, AgentLoopDriverError> {
        let family = registry
            .get(id)
            .ok_or_else(|| AgentLoopDriverError::InvalidRequest {
                reason: format!("unknown loop family: {id}"),
            })?;
        Self::from_family(family, executor, version)
    }

    pub fn default_from_registry(
        registry: &LoopFamilyRegistry,
    ) -> Result<Self, AgentLoopDriverError> {
        Self::from_registry(
            registry,
            &LoopFamilyId::DEFAULT,
            Arc::new(CanonicalAgentLoopExecutor),
            RunProfileVersion::new(PLANNED_DRIVER_VERSION),
        )
    }
}

#[async_trait]
impl AgentLoopDriver for PlannedDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        self.descriptor.clone()
    }

    async fn run(
        &self,
        request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        validate_run_request(&request, &self.descriptor)?;
        let initial = LoopExecutionState::initial_for_run(host.run_context());
        self.executor
            .execute_family(self.family.as_ref(), host, initial)
            .await
            .map_err(map_executor_error)
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        validate_resume_request(&request, &self.descriptor)?;
        Err(AgentLoopDriverError::InvalidRequest {
            reason: "planned driver resume requires WS-10 checkpoint payload loading".to_string(),
        })
    }
}

fn descriptor_for_family(
    family_id: &LoopFamilyId,
    version: RunProfileVersion,
) -> Result<AgentLoopDriverDescriptor, AgentLoopDriverError> {
    let driver_id = format!("reborn:{family_id}-loop");
    AgentLoopDriverDescriptor::new(driver_id, version)
        .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?
        .with_checkpoint_schema(CHECKPOINT_SCHEMA_ID, version)
        .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })
}

fn validate_run_request(
    request: &AgentLoopDriverRunRequest,
    descriptor: &AgentLoopDriverDescriptor,
) -> Result<(), AgentLoopDriverError> {
    validate_descriptor_assignment(&request.resolved_run_profile.loop_driver, descriptor)
}

fn validate_resume_request(
    request: &AgentLoopDriverResumeRequest,
    descriptor: &AgentLoopDriverDescriptor,
) -> Result<(), AgentLoopDriverError> {
    validate_descriptor_assignment(&request.resolved_run_profile.loop_driver, descriptor)?;
    let want = descriptor.checkpoint_schema_id.as_ref();
    let have = request
        .resolved_run_profile
        .loop_driver
        .checkpoint_schema_id
        .as_ref();
    if want != have {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "checkpoint schema id does not match driver descriptor".to_string(),
        });
    }
    Ok(())
}

fn validate_descriptor_assignment(
    request_descriptor: &AgentLoopDriverDescriptor,
    descriptor: &AgentLoopDriverDescriptor,
) -> Result<(), AgentLoopDriverError> {
    if request_descriptor != descriptor {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "driver request profile is not assigned to this planned driver".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn map_executor_error(error: AgentLoopExecutorError) -> AgentLoopDriverError {
    tracing::warn!(?error, "planned driver executor returned sanitized error");
    match error {
        AgentLoopExecutorError::HostUnavailable { stage } => AgentLoopDriverError::Unavailable {
            reason: format!("{}: unavailable", host_stage_name(stage)),
        },
        AgentLoopExecutorError::PlannerContract { detail } => AgentLoopDriverError::Failed {
            reason_kind: format!("driver_bug:{detail}"),
        },
        AgentLoopExecutorError::CheckpointFailed { stage } => AgentLoopDriverError::Failed {
            reason_kind: format!("checkpoint_rejected:{}", checkpoint_kind_name(stage)),
        },
        AgentLoopExecutorError::Cancelled => AgentLoopDriverError::Failed {
            reason_kind: "interrupted_unexpectedly".to_string(),
        },
    }
}

fn host_stage_name(stage: HostStage) -> &'static str {
    match stage {
        HostStage::Prompt => "Prompt",
        HostStage::Model => "Model",
        HostStage::Capability => "Capability",
        HostStage::Transcript => "Transcript",
        HostStage::Checkpoint => "Checkpoint",
        HostStage::Progress => "Progress",
        HostStage::Input => "Input",
    }
}

fn checkpoint_kind_name(kind: CheckpointKind) -> &'static str {
    match kind {
        CheckpointKind::BeforeModel => "before_model",
        CheckpointKind::BeforeSideEffect => "before_side_effect",
        CheckpointKind::BeforeBlock => "before_block",
        CheckpointKind::Final => "final",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_loop_family_registry;
    use ironclaw_turns::run_profile::{CheckpointSchemaId, LoopDriverId};

    #[test]
    fn default_planned_driver_descriptor_uses_default_family_identity() {
        let registry = build_loop_family_registry();
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let descriptor = driver.descriptor();

        assert_eq!(
            descriptor.id,
            LoopDriverId::new("reborn:default-loop").expect("valid")
        );
        assert_eq!(
            descriptor.checkpoint_schema_id,
            Some(CheckpointSchemaId::new(CHECKPOINT_SCHEMA_ID).expect("valid"))
        );
    }

    #[test]
    fn executor_cancelled_error_maps_to_failed_not_unavailable() {
        let mapped = map_executor_error(AgentLoopExecutorError::Cancelled);

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: "interrupted_unexpectedly".to_string()
            }
        );
    }
}
