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
    state::{
        CHECKPOINT_SCHEMA_ID, CHECKPOINT_SCHEMA_VERSION, CheckpointKind, CheckpointPayloadError,
        LoopExecutionState,
    },
};
use ironclaw_turns::{
    LoopExit, LoopExitId, LoopFailureKind, RunProfileVersion,
    run_profile::{
        AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverHost,
        AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, AgentLoopHostError,
        LoadCheckpointPayloadRequest, LoopCheckpointKind, LoopDriverId, LoopRunContext,
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
    pub fn from_family_with_descriptor(
        family: Arc<LoopFamily>,
        executor: Arc<CanonicalAgentLoopExecutor>,
        descriptor: AgentLoopDriverDescriptor,
    ) -> Result<Self, AgentLoopDriverError> {
        if descriptor.checkpoint_schema_id.is_none()
            || descriptor.checkpoint_schema_version.is_none()
        {
            return Err(AgentLoopDriverError::InvalidRequest {
                reason: "planned driver descriptor must carry a checkpoint schema".to_string(),
            });
        }
        Ok(Self {
            descriptor,
            family,
            executor,
        })
    }

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
    }

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        let run_context = host.run_context();
        validate_resume_request(&request, run_context, &self.descriptor)?;
        let payload = match host
            .load_checkpoint_payload(LoadCheckpointPayloadRequest {
                checkpoint_id: request.checkpoint_id,
                expected_schema_id: run_context.checkpoint_schema_id.clone(),
                expected_schema_version: run_context.checkpoint_schema_version,
            })
            .await
        {
            Ok(payload) => payload,
            Err(error) => {
                log_resume_load_error(error);
                return checkpoint_unavailable_exit(run_context.run_id);
            }
        };

        let checkpoint_kind = match resumable_checkpoint_kind_from_host(payload.kind) {
            Ok(kind) => kind,
            Err(()) => return checkpoint_unavailable_exit(run_context.run_id),
        };

        let initial = match LoopExecutionState::from_checkpoint_payload(
            payload.payload.as_bytes(),
            checkpoint_kind,
        ) {
            Ok(initial) => initial,
            Err(error) => {
                log_resume_payload_error(error);
                return checkpoint_unavailable_exit(run_context.run_id);
            }
        };

        self.executor
            .execute_family(self.family.as_ref(), host, initial)
            .await
            .map_err(map_executor_error)
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
    run_context: &LoopRunContext,
    descriptor: &AgentLoopDriverDescriptor,
) -> Result<(), AgentLoopDriverError> {
    if request.turn_id != run_context.turn_id || request.run_id != run_context.run_id {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "driver request does not match loop host run context".to_string(),
        });
    }
    if request.resolved_run_profile != run_context.resolved_run_profile {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "driver request profile does not match loop host run context".to_string(),
        });
    }
    validate_descriptor_assignment(&request.resolved_run_profile.loop_driver, descriptor)?;
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

fn log_resume_load_error(error: AgentLoopHostError) {
    tracing::warn!(?error, "planned driver could not load checkpoint payload");
}

fn log_resume_payload_error(error: CheckpointPayloadError) {
    tracing::warn!(?error, "planned driver could not decode checkpoint payload");
}

fn checkpoint_unavailable_exit(
    run_id: ironclaw_turns::TurnRunId,
) -> Result<LoopExit, AgentLoopDriverError> {
    let exit_id =
        LoopExitId::new(format!("exit:{run_id}-checkpoint-unavailable")).map_err(|_| {
            AgentLoopDriverError::Failed {
                reason_kind: "driver_bug".to_string(),
            }
        })?;
    Ok(LoopExit::failed(
        LoopFailureKind::CheckpointUnavailable,
        exit_id,
    ))
}

fn host_stage_name(stage: HostStage) -> &'static str {
    match stage {
        HostStage::Prompt => "Prompt",
        HostStage::Model => "Model",
        HostStage::Capability => "Capability",
        HostStage::Transcript => "Transcript",
        HostStage::Checkpoint => "Checkpoint",
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

fn resumable_checkpoint_kind_from_host(kind: LoopCheckpointKind) -> Result<CheckpointKind, ()> {
    match kind {
        LoopCheckpointKind::BeforeModel => Ok(CheckpointKind::BeforeModel),
        LoopCheckpointKind::BeforeBlock => Ok(CheckpointKind::BeforeBlock),
        LoopCheckpointKind::BeforeSideEffect | LoopCheckpointKind::Final => {
            tracing::warn!(
                ?kind,
                "planned driver cannot resume checkpoint kind without exact continuation semantics"
            );
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_loop_family::build_loop_family_registry;
    use ironclaw_agent_loop::test_support::{
        MockAgentLoopDriverHost, MockHostCall, test_run_context,
    };
    use ironclaw_turns::{
        LoopMessageRef, RedactedCheckpointPayload, TurnCheckpointId,
        run_profile::{
            AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef,
            BeginAssistantDraft, CapabilityBatchInvocation, CapabilityBatchOutcome,
            CapabilityInvocation, CapabilityOutcome, CheckpointSchemaId, FinalizeAssistantMessage,
            LoadCheckpointPayloadRequest, LoadedCheckpointPayload, LoopCancellationPort,
            LoopCancellationSignal, LoopCapabilityPort, LoopCheckpointPort, LoopCheckpointRequest,
            LoopCheckpointStateRef, LoopContextBundle, LoopContextPort, LoopContextRequest,
            LoopDriverId, LoopInputAckToken, LoopInputBatch, LoopInputCursor, LoopInputPort,
            LoopModelPort, LoopModelRequest, LoopModelResponse, LoopProgressEvent,
            LoopProgressPort, LoopPromptBundle, LoopPromptBundleRequest, LoopPromptPort,
            LoopRunContext, LoopRunInfoPort, LoopTranscriptPort, StageCheckpointPayloadRequest,
            UpdateAssistantDraft, VisibleCapabilityRequest, VisibleCapabilitySurface,
        },
    };
    use std::sync::Mutex;

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
            // Keep the unprefixed `CHECKPOINT_SCHEMA_ID` (already in scope via
            // `use super::*` -> `use ironclaw_agent_loop::state::*`) — the
            // `crate::PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID` alias trips clippy's
            // `unused-imports` lint on newer toolchains because it resolves to
            // the same const value, and that gate is enforced on this PR.
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
    fn executor_cancelled_error_maps_to_failed_not_unavailable() {
        let mapped = map_executor_error(AgentLoopExecutorError::Cancelled);

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: "interrupted_unexpectedly".to_string()
            }
        );
    }

    #[tokio::test]
    async fn resume_missing_checkpoint_payload_returns_checkpoint_unavailable_exit() {
        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);
        let (host, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .build();

        let result = driver
            .resume(
                AgentLoopDriverResumeRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    checkpoint_id: TurnCheckpointId::new(),
                    resolved_run_profile: context.resolved_run_profile.clone(),
                },
                &host,
            )
            .await;

        assert_checkpoint_unavailable_exit(result);
    }

    #[tokio::test]
    async fn resume_loads_checkpoint_payload_and_continues_from_loaded_state() {
        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);
        let mut restored_state = LoopExecutionState::initial_for_run(&context);
        restored_state.iteration = 7;
        let checkpoint_id = TurnCheckpointId::new();
        let loaded = LoadedCheckpointPayload {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: context.checkpoint_schema_id.clone(),
            schema_version: context.checkpoint_schema_version,
            payload: RedactedCheckpointPayload::new(
                serde_json::to_vec(&restored_state).expect("serialize checkpoint state"),
            )
            .expect("valid checkpoint payload"),
        };
        let (inner, checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .build();
        let host = ResumePayloadHost::new(inner, checkpoint_id, loaded);

        let result = driver
            .resume(
                AgentLoopDriverResumeRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    checkpoint_id,
                    resolved_run_profile: context.resolved_run_profile.clone(),
                },
                &host,
            )
            .await;

        result.expect("resume should continue the loop");
        assert_eq!(host.load_call_count(), 1);
        assert!(host.call_log().contains(&MockHostCall::StreamModel));
        assert_eq!(
            checkpoints.sequence().first(),
            Some(&(CheckpointKind::BeforeModel, 7)),
            "first executor checkpoint must start from the loaded state"
        );
    }

    #[tokio::test]
    async fn resume_rejects_wrong_loop_family_before_loading_checkpoint() {
        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);
        let (host, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .build();
        let mut resolved_run_profile = context.resolved_run_profile.clone();
        resolved_run_profile.loop_driver.id =
            LoopDriverId::new("reborn:other-loop").expect("valid");

        let result = driver
            .resume(
                AgentLoopDriverResumeRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    checkpoint_id: TurnCheckpointId::new(),
                    resolved_run_profile,
                },
                &host,
            )
            .await;

        assert!(matches!(
            result,
            Err(AgentLoopDriverError::InvalidRequest { reason })
                if reason == "driver request profile does not match loop host run context"
        ));
        assert!(
            host.call_log().is_empty(),
            "invalid family must fail before any host port is invoked"
        );
    }

    #[tokio::test]
    async fn resume_schema_mismatch_load_error_returns_checkpoint_unavailable_exit() {
        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);
        let checkpoint_id = TurnCheckpointId::new();
        let loaded = LoadedCheckpointPayload {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: CheckpointSchemaId::new("different_checkpoint_schema").expect("valid"),
            schema_version: context.checkpoint_schema_version,
            payload: RedactedCheckpointPayload::new(b"{}".to_vec())
                .expect("valid checkpoint payload"),
        };
        let (inner, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .build();
        let host = ResumePayloadHost::new(inner, checkpoint_id, loaded);

        let result = driver
            .resume(
                AgentLoopDriverResumeRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    checkpoint_id,
                    resolved_run_profile: context.resolved_run_profile.clone(),
                },
                &host,
            )
            .await;

        assert_checkpoint_unavailable_exit(result);
        assert_eq!(host.load_call_count(), 1);
        assert!(
            host.call_log().is_empty(),
            "invalid checkpoint load must fail before executor host ports"
        );
    }

    #[tokio::test]
    async fn planned_driver_resume_schema_version_drift_fails_cleanly() {
        // Stage a valid checkpoint payload under schema_version = 1 (current).
        // Resume with a run context bumped to schema_version = 2.
        // The host sees expected_schema_version = 2 but stored = 1 -> Invalid
        // -> mapped to a terminal checkpoint_unavailable loop exit.
        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let mut context = run_context_for_driver(&driver);
        let checkpoint_id = TurnCheckpointId::new();

        // The stored payload carries the old (correct) schema version.
        let stored_schema_version = context.checkpoint_schema_version;
        let loaded = LoadedCheckpointPayload {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: context.checkpoint_schema_id.clone(),
            schema_version: stored_schema_version,
            payload: RedactedCheckpointPayload::new(b"{}".to_vec())
                .expect("valid checkpoint payload"),
        };

        // Bump the run context's schema version to simulate a driver upgrade.
        let bumped_version = RunProfileVersion::new(stored_schema_version.as_u64() + 1);
        context.checkpoint_schema_version = bumped_version;
        context.resolved_run_profile.checkpoint_schema_version = bumped_version;

        let (inner, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .build();
        let host = ResumePayloadHost::new(inner, checkpoint_id, loaded);

        let result = driver
            .resume(
                AgentLoopDriverResumeRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    checkpoint_id,
                    resolved_run_profile: context.resolved_run_profile.clone(),
                },
                &host,
            )
            .await;

        assert_checkpoint_unavailable_exit(result);
        assert_eq!(host.load_call_count(), 1);
        assert!(
            host.call_log().is_empty(),
            "schema version drift must fail before any executor host ports are invoked"
        );
    }

    #[tokio::test]
    async fn resume_unsupported_checkpoint_kind_returns_checkpoint_unavailable_exit() {
        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);
        let checkpoint_id = TurnCheckpointId::new();
        let loaded = LoadedCheckpointPayload {
            kind: LoopCheckpointKind::BeforeSideEffect,
            schema_id: context.checkpoint_schema_id.clone(),
            schema_version: context.checkpoint_schema_version,
            payload: RedactedCheckpointPayload::new(b"{}".to_vec())
                .expect("valid checkpoint payload"),
        };
        let (inner, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .build();
        let host = ResumePayloadHost::new(inner, checkpoint_id, loaded);

        let result = driver
            .resume(
                AgentLoopDriverResumeRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    checkpoint_id,
                    resolved_run_profile: context.resolved_run_profile.clone(),
                },
                &host,
            )
            .await;

        assert_checkpoint_unavailable_exit(result);
        assert_eq!(host.load_call_count(), 1);
        assert!(
            host.call_log().is_empty(),
            "unsupported checkpoint kinds must fail before executor host ports"
        );
    }

    fn run_context_for_driver(
        driver: &PlannedDriver,
    ) -> ironclaw_turns::run_profile::LoopRunContext {
        let descriptor = driver.descriptor();
        let mut context = test_run_context("planned-driver-resume");
        context.resolved_run_profile.loop_driver = descriptor.clone();
        context.resolved_run_profile.checkpoint_schema_id = descriptor
            .checkpoint_schema_id
            .clone()
            .expect("planned driver checkpoint schema");
        context.resolved_run_profile.checkpoint_schema_version = descriptor
            .checkpoint_schema_version
            .expect("planned driver checkpoint schema version");
        context.loop_driver_id = descriptor.id;
        context.loop_driver_version = descriptor.version;
        context.checkpoint_schema_id = context.resolved_run_profile.checkpoint_schema_id.clone();
        context.checkpoint_schema_version = context.resolved_run_profile.checkpoint_schema_version;
        context
    }

    fn assert_checkpoint_unavailable_exit(result: Result<LoopExit, AgentLoopDriverError>) {
        match result.expect("resume should return a terminal failed loop exit") {
            LoopExit::Failed(failed) => {
                assert_eq!(failed.reason_kind, LoopFailureKind::CheckpointUnavailable);
                assert!(
                    failed.exit_id.as_str().ends_with("-checkpoint-unavailable"),
                    "checkpoint resume failures should use a checkpoint-specific exit id"
                );
            }
            other => panic!("expected checkpoint_unavailable failed exit, got {other:?}"),
        }
    }

    struct ResumePayloadHost {
        inner: MockAgentLoopDriverHost,
        checkpoint_id: TurnCheckpointId,
        loaded: LoadedCheckpointPayload,
        load_calls: Mutex<usize>,
    }

    impl ResumePayloadHost {
        fn new(
            inner: MockAgentLoopDriverHost,
            checkpoint_id: TurnCheckpointId,
            loaded: LoadedCheckpointPayload,
        ) -> Self {
            Self {
                inner,
                checkpoint_id,
                loaded,
                load_calls: Mutex::new(0),
            }
        }

        fn load_call_count(&self) -> usize {
            *self.load_calls.lock().expect("load call lock")
        }

        fn call_log(&self) -> Vec<MockHostCall> {
            self.inner.call_log()
        }
    }

    impl LoopRunInfoPort for ResumePayloadHost {
        fn run_context(&self) -> &LoopRunContext {
            self.inner.run_context()
        }
    }

    impl LoopCancellationPort for ResumePayloadHost {
        fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
            self.inner.observe_cancellation()
        }
    }

    #[async_trait::async_trait]
    impl LoopContextPort for ResumePayloadHost {
        async fn load_loop_context(
            &self,
            request: LoopContextRequest,
        ) -> Result<LoopContextBundle, AgentLoopHostError> {
            self.inner.load_loop_context(request).await
        }
    }

    #[async_trait::async_trait]
    impl LoopPromptPort for ResumePayloadHost {
        async fn build_prompt_bundle(
            &self,
            request: LoopPromptBundleRequest,
        ) -> Result<LoopPromptBundle, AgentLoopHostError> {
            self.inner.build_prompt_bundle(request).await
        }
    }

    #[async_trait::async_trait]
    impl LoopInputPort for ResumePayloadHost {
        async fn poll_inputs(
            &self,
            after: LoopInputCursor,
            limit: usize,
        ) -> Result<LoopInputBatch, AgentLoopHostError> {
            self.inner.poll_inputs(after, limit).await
        }

        async fn ack_inputs(
            &self,
            tokens: Vec<LoopInputAckToken>,
        ) -> Result<(), AgentLoopHostError> {
            self.inner.ack_inputs(tokens).await
        }
    }

    #[async_trait::async_trait]
    impl LoopModelPort for ResumePayloadHost {
        async fn stream_model(
            &self,
            request: LoopModelRequest,
        ) -> Result<LoopModelResponse, AgentLoopHostError> {
            self.inner.stream_model(request).await
        }
    }

    #[async_trait::async_trait]
    impl LoopCapabilityPort for ResumePayloadHost {
        async fn visible_capabilities(
            &self,
            request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            self.inner.visible_capabilities(request).await
        }

        async fn invoke_capability(
            &self,
            request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            self.inner.invoke_capability(request).await
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            self.inner.invoke_capability_batch(request).await
        }
    }

    #[async_trait::async_trait]
    impl LoopTranscriptPort for ResumePayloadHost {
        async fn begin_assistant_draft(
            &self,
            request: BeginAssistantDraft,
        ) -> Result<LoopMessageRef, AgentLoopHostError> {
            self.inner.begin_assistant_draft(request).await
        }

        async fn update_assistant_draft(
            &self,
            request: UpdateAssistantDraft,
        ) -> Result<(), AgentLoopHostError> {
            self.inner.update_assistant_draft(request).await
        }

        async fn finalize_assistant_message(
            &self,
            request: FinalizeAssistantMessage,
        ) -> Result<LoopMessageRef, AgentLoopHostError> {
            self.inner.finalize_assistant_message(request).await
        }

        async fn append_capability_result_ref(
            &self,
            request: AppendCapabilityResultRef,
        ) -> Result<LoopMessageRef, AgentLoopHostError> {
            self.inner.append_capability_result_ref(request).await
        }
    }

    #[async_trait::async_trait]
    impl LoopCheckpointPort for ResumePayloadHost {
        async fn checkpoint(
            &self,
            request: LoopCheckpointRequest,
        ) -> Result<TurnCheckpointId, AgentLoopHostError> {
            self.inner.checkpoint(request).await
        }

        async fn stage_checkpoint_payload(
            &self,
            request: StageCheckpointPayloadRequest,
        ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
            self.inner.stage_checkpoint_payload(request).await
        }

        async fn load_checkpoint_payload(
            &self,
            request: LoadCheckpointPayloadRequest,
        ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
            *self.load_calls.lock().expect("load call lock") += 1;
            if request.checkpoint_id != self.checkpoint_id {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "test checkpoint not found",
                ));
            }
            if request.expected_schema_id != self.loaded.schema_id
                || request.expected_schema_version != self.loaded.schema_version
            {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Invalid,
                    "test checkpoint schema mismatch",
                ));
            }
            Ok(self.loaded.clone())
        }
    }

    #[async_trait::async_trait]
    impl LoopProgressPort for ResumePayloadHost {
        async fn emit_loop_progress(
            &self,
            event: LoopProgressEvent,
        ) -> Result<(), AgentLoopHostError> {
            self.inner.emit_loop_progress(event).await
        }
    }
    // Note: a duplicate `impl LoopCancellationPort for ResumePayloadHost`
    // existed here on baseline and broke `cargo test --no-run` for this crate.
    // The earlier delegating impl (a few hundred lines above) is the
    // intended one; the trailing one returned `None` unconditionally and
    // was unreachable behind the conflict. Removed here while updating
    // tests for the narrowed public surface.
}
