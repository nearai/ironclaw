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
    state::{CHECKPOINT_SCHEMA_ID, CHECKPOINT_SCHEMA_VERSION, CheckpointKind, LoopExecutionState},
};
use ironclaw_turns::{
    LoopExit, RunProfileVersion,
    run_profile::{
        AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverHost,
        AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, LoopDriverId,
    },
};

pub const PLANNED_DRIVER_DEFAULT_ID: &str = "reborn:planned-default";
const PLANNED_DRIVER_VERSION: u64 = 1;

/// Non-generic adapter from one resolved loop family to `AgentLoopDriver`.
pub struct PlannedDriver {
    descriptor: AgentLoopDriverDescriptor,
    family: Arc<LoopFamily>,
    executor: Arc<CanonicalAgentLoopExecutor>,
}

impl PlannedDriver {
    pub fn from_family(
        driver_id: LoopDriverId,
        family: Arc<LoopFamily>,
        executor: Arc<CanonicalAgentLoopExecutor>,
        version: RunProfileVersion,
    ) -> Result<Self, AgentLoopDriverError> {
        let descriptor = descriptor_for_driver_id(driver_id, version)?;
        Ok(Self {
            descriptor,
            family,
            executor,
        })
    }

    pub fn from_registry(
        driver_id: LoopDriverId,
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
        Self::from_family(driver_id, family, executor, version)
    }

    pub fn default_from_registry(
        registry: &LoopFamilyRegistry,
    ) -> Result<Self, AgentLoopDriverError> {
        Self::from_registry(
            planned_default_driver_id()?,
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
            .and_then(reject_blocked_exit_until_resume_supported)
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        validate_resume_request(&request, &self.descriptor)?;
        Err(pending_resume_error())
    }
}

fn planned_default_driver_id() -> Result<LoopDriverId, AgentLoopDriverError> {
    LoopDriverId::new(PLANNED_DRIVER_DEFAULT_ID)
        .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })
}

fn descriptor_for_driver_id(
    driver_id: LoopDriverId,
    version: RunProfileVersion,
) -> Result<AgentLoopDriverDescriptor, AgentLoopDriverError> {
    AgentLoopDriverDescriptor::new(driver_id.as_str(), version)
        .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?
        .with_checkpoint_schema(
            CHECKPOINT_SCHEMA_ID,
            RunProfileVersion::new(CHECKPOINT_SCHEMA_VERSION),
        )
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

fn pending_resume_error() -> AgentLoopDriverError {
    AgentLoopDriverError::Unavailable {
        reason: "planned driver resume requires WS-10 checkpoint payload loading".to_string(),
    }
}

fn reject_blocked_exit_until_resume_supported(
    exit: LoopExit,
) -> Result<LoopExit, AgentLoopDriverError> {
    if matches!(exit, LoopExit::Blocked(_)) {
        return Err(AgentLoopDriverError::Unavailable {
            reason: "planned driver blocked exits require WS-10 checkpoint payload loading"
                .to_string(),
        });
    }
    Ok(exit)
}

pub(crate) fn map_executor_error(error: AgentLoopExecutorError) -> AgentLoopDriverError {
    if matches!(error, AgentLoopExecutorError::Cancelled) {
        tracing::debug!(?error, "planned driver executor cancelled");
    } else {
        tracing::warn!(?error, "planned driver executor returned sanitized error");
    }
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
    use ironclaw_turns::{
        LoopBlocked, LoopBlockedKind, LoopExitId, LoopGateRef, TurnCheckpointId,
        run_profile::{CheckpointSchemaId, LoopCheckpointStateRef, LoopDriverId},
    };

    #[test]
    fn default_planned_driver_descriptor_uses_default_family_identity() {
        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let descriptor = driver.descriptor();

        assert_eq!(
            descriptor.id,
            LoopDriverId::new(PLANNED_DRIVER_DEFAULT_ID).expect("valid")
        );
        assert_eq!(
            descriptor.checkpoint_schema_id,
            Some(CheckpointSchemaId::new(CHECKPOINT_SCHEMA_ID).expect("valid"))
        );
        assert_eq!(
            descriptor.checkpoint_schema_version,
            Some(RunProfileVersion::new(CHECKPOINT_SCHEMA_VERSION))
        );
    }

    #[test]
    fn descriptor_for_family_uses_independent_checkpoint_schema_version() {
        let descriptor = descriptor_for_driver_id(
            LoopDriverId::new("reborn:custom-planned").expect("valid"),
            RunProfileVersion::new(PLANNED_DRIVER_VERSION + 1),
        )
        .expect("descriptor");

        assert_eq!(
            descriptor.version,
            RunProfileVersion::new(PLANNED_DRIVER_VERSION + 1)
        );
        assert_eq!(
            descriptor.checkpoint_schema_version,
            Some(RunProfileVersion::new(CHECKPOINT_SCHEMA_VERSION))
        );
    }

    #[test]
    fn validate_descriptor_assignment_rejects_wrong_driver() {
        let descriptor = descriptor_for_driver_id(
            LoopDriverId::new(PLANNED_DRIVER_DEFAULT_ID).expect("valid"),
            RunProfileVersion::new(1),
        )
        .expect("descriptor");
        let wrong_descriptor =
            AgentLoopDriverDescriptor::new("reborn:other-loop", RunProfileVersion::new(1))
                .expect("wrong descriptor")
                .with_checkpoint_schema(
                    CHECKPOINT_SCHEMA_ID,
                    RunProfileVersion::new(CHECKPOINT_SCHEMA_VERSION),
                )
                .expect("wrong checkpoint schema");

        let err = validate_descriptor_assignment(&wrong_descriptor, &descriptor)
            .expect_err("descriptor mismatch should be rejected");

        assert_eq!(
            err,
            AgentLoopDriverError::InvalidRequest {
                reason: "driver request profile is not assigned to this planned driver".to_string()
            }
        );
    }

    #[test]
    fn planned_resume_pending_error_is_unavailable() {
        let descriptor = descriptor_for_driver_id(
            LoopDriverId::new(PLANNED_DRIVER_DEFAULT_ID).expect("valid"),
            RunProfileVersion::new(1),
        )
        .expect("descriptor");
        let request_descriptor = descriptor.clone();

        validate_descriptor_assignment(&request_descriptor, &descriptor)
            .expect("matching descriptor");
        assert_eq!(
            pending_resume_error(),
            AgentLoopDriverError::Unavailable {
                reason: "planned driver resume requires WS-10 checkpoint payload loading"
                    .to_string()
            }
        );
    }

    #[test]
    fn blocked_exits_are_unavailable_until_resume_is_supported() {
        let blocked = LoopExit::Blocked(LoopBlocked {
            kind: LoopBlockedKind::Approval,
            gate_ref: LoopGateRef::new("gate:approval").expect("valid"),
            checkpoint_id: TurnCheckpointId::new(),
            state_ref: LoopCheckpointStateRef::new("checkpoint:state").expect("valid"),
            exit_id: LoopExitId::new("exit:blocked").expect("valid"),
        });

        assert_eq!(
            reject_blocked_exit_until_resume_supported(blocked).expect_err("blocked pending WS10"),
            AgentLoopDriverError::Unavailable {
                reason: "planned driver blocked exits require WS-10 checkpoint payload loading"
                    .to_string()
            }
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
