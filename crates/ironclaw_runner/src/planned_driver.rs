// arch-exempt: large_file, caller-level failure mapping regression stays with the driver, plan #4088
//! Planned Reborn loop driver.
//!
//! This module is the bridge from the runner-facing `AgentLoopDriver` trait to
//! the sealed `ironclaw_agent_loop` framework. It intentionally holds an opaque
//! `LoopFamily` and the canonical executor; it does not expose planner slots to
//! `ironclaw_runner`.

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

use crate::model_failure_mapping::model_stage_failure_category;

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

        let mut initial = match LoopExecutionState::from_checkpoint_payload(
            payload.payload.as_bytes(),
            checkpoint_kind,
        ) {
            Ok(initial) => initial.rebase_for_run(run_context),
            Err(error) => {
                log_resume_payload_error(error);
                return checkpoint_unavailable_exit(run_context.run_id);
            }
        };

        // A run resumed from a user-denied gate carries the denial disposition
        // in the resume request. Stamp it onto whichever pending resume slot's
        // `gate_ref` matches `last_gate` — the gate the run is blocked on.
        //
        // Why match by gate_ref: GateStage (executor/gates.rs) intentionally
        // PRESERVES `pending_auth_resume` when a non-auth (approval/resource)
        // gate fires mid-re-dispatch, so a checkpoint can carry BOTH slots
        // simultaneously. Stamping both blindly would corrupt the unrelated
        // auth resume when the user denies only the approval gate (and vice
        // versa). `state.last_gate` records the exact gate_ref set at the
        // blocking `GateOutcome::Block` branch, so comparing against it
        // identifies the one slot whose denial should be surfaced.
        //
        // If neither slot's gate_ref matches (e.g. state was written before
        // last_gate was introduced), we stamp neither rather than risking a
        // misattribution. The executor will re-dispatch normally in that case.
        if let Some(disposition) = request.resume_disposition.clone() {
            stamp_resume_disposition(&mut initial, disposition);
        }

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

/// Stamps `disposition` onto the single pending resume slot whose `gate_ref`
/// matches `state.last_gate` (the gate the run is blocked on).
///
/// Both `pending_auth_resume` and `pending_approval_resume` can be populated
/// simultaneously (GateStage preserves `pending_auth_resume` when a non-auth
/// gate fires mid-re-dispatch). Matching on `last_gate` ensures the denial is
/// attributed only to the slot that corresponds to the current blocking gate,
/// leaving the other slot untouched.
///
/// If BOTH slots carry the same `gate_ref` as `last_gate` (should never happen
/// in practice), the function stamps NEITHER and emits a `warn!` — failing
/// closed rather than misattributing the denial.
fn stamp_resume_disposition(
    state: &mut ironclaw_agent_loop::state::LoopExecutionState,
    disposition: ironclaw_turns::GateResumeDisposition,
) {
    let Some(last_gate) = state.last_gate.clone() else {
        return;
    };
    // Compute matches before taking mutable borrows.
    let auth_matches = state
        .pending_auth_resume
        .as_ref()
        .is_some_and(|p| p.gate_ref == last_gate);
    let approval_matches = state
        .pending_approval_resume
        .as_ref()
        .is_some_and(|p| p.gate_ref == last_gate);
    let external_tool_matches = state
        .pending_external_tool_resume
        .as_ref()
        .is_some_and(|p| p.gate_ref == last_gate);
    // Exactly one pending slot may own a given gate_ref. Fail closed (stamp
    // nothing) when zero or more than one slot claims it, rather than
    // misattribute the disposition.
    let match_count = usize::from(auth_matches)
        + usize::from(approval_matches)
        + usize::from(external_tool_matches);
    if match_count != 1 {
        if match_count > 1 {
            tracing::debug!(
                ?last_gate,
                "ambiguous gate resume disposition; refusing to stamp"
            );
        }
        return;
    }
    if auth_matches {
        if let Some(pending) = state.pending_auth_resume.as_mut() {
            pending.disposition = Some(disposition);
        }
    } else if approval_matches {
        if let Some(pending) = state.pending_approval_resume.as_mut() {
            pending.disposition = Some(disposition);
        }
    } else if let Some(pending) = state.pending_external_tool_resume.as_mut() {
        pending.disposition = Some(disposition);
    }
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
        AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage,
            kind,
            safe_summary,
            reason_kind,
            diagnostic_ref,
            detail,
        } => {
            tracing::warn!(
                stage = ?stage,
                kind = ?kind,
                reason_kind = ?reason_kind,
                diagnostic_ref = ?diagnostic_ref,
                safe_summary = %safe_summary,
                "planned driver host stage unavailable"
            );
            if let Some(category) =
                model_stage_failure_category(stage == HostStage::Model, kind, reason_kind)
            {
                // Prefer the secret-scrubbed model-visible detail; fall back to
                // the bounded safe summary so the explainer still gets the real
                // cause rather than only the category. Fail-closed backstop:
                // executor-side producers can only run the token-prefix scrub
                // (ironclaw_agent_loop cannot depend on the hardened scrubber),
                // so re-scrub through the full LeakDetector registry +
                // injection fencing here, where the detail becomes visible.
                let detail = detail
                    .or_else(|| Some(safe_summary.as_str().to_string()))
                    .map(ironclaw_loop_host::scrub_model_visible_detail);
                return AgentLoopDriverError::Failed {
                    reason_kind: category.to_string(),
                    detail,
                };
            }
            AgentLoopDriverError::Unavailable {
                reason: format!("{}: {safe_summary}", host_stage_name(stage)),
            }
        }
        AgentLoopExecutorError::PlannerContract { detail } => {
            tracing::warn!(detail = %detail, "planned driver planner contract failed");
            AgentLoopDriverError::Failed {
                reason_kind: "driver_bug".to_string(),
                detail: None,
            }
        }
        AgentLoopExecutorError::CheckpointFailed { stage } => {
            tracing::warn!(stage = ?stage, "planned driver checkpoint failed");
            AgentLoopDriverError::Failed {
                reason_kind: "checkpoint_rejected".to_string(),
                detail: None,
            }
        }
        AgentLoopExecutorError::Cancelled => AgentLoopDriverError::Failed {
            reason_kind: "interrupted_unexpectedly".to_string(),
            detail: None,
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
                detail: None,
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
    use crate::failure_categories::{
        MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY, MODEL_CREDITS_EXHAUSTED_CATEGORY,
        MODEL_CREDITS_EXHAUSTED_REASON_KIND,
    };
    use ironclaw_agent_loop::test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityOutcome,
        test_run_context,
    };
    use ironclaw_turns::{
        LoopMessageRef, RedactedCheckpointPayload, TurnCheckpointId,
        run_profile::{
            AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef,
            BeginAssistantDraft, CheckpointSchemaId, FinalizeAssistantMessage,
            LoadCheckpointPayloadRequest, LoadedCheckpointPayload, LoopCancellationPort,
            LoopCancellationSignal, LoopCapabilityPort, LoopCheckpointPort, LoopCheckpointRequest,
            LoopCheckpointStateRef, LoopCompactionError, LoopCompactionOutcome, LoopCompactionPort,
            LoopCompactionRequest, LoopContextBundle, LoopContextPort, LoopContextRequest,
            LoopDriverId, LoopInputAckToken, LoopInputBatch, LoopInputCursor, LoopInputPort,
            LoopModelPort, LoopModelRequest, LoopModelResponse, LoopProgressEvent,
            LoopProgressPort, LoopPromptBundle, LoopPromptBundleRequest, LoopPromptPort,
            LoopRequest, LoopRequestBatch, LoopRunContext, LoopRunInfoPort, LoopSafeSummary,
            LoopTranscriptPort, StageCheckpointPayloadRequest, UpdateAssistantDraft,
            VisibleCapabilityRequest, VisibleCapabilitySurface,
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
                reason_kind: "interrupted_unexpectedly".to_string(),
                detail: None,
            }
        );
    }

    #[tokio::test]
    async fn run_inflight_model_cancelled_maps_to_interrupted_unexpectedly_without_cancel_signal() {
        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);
        let (host, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .fail_model_with(AgentLoopHostErrorKind::Cancelled)
            .build();

        let result = driver
            .run(
                AgentLoopDriverRunRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    resolved_run_profile: context.resolved_run_profile.clone(),
                },
                &host,
            )
            .await;

        assert_eq!(
            result,
            Err(AgentLoopDriverError::Failed {
                reason_kind: "interrupted_unexpectedly".to_string(),
                detail: None,
            })
        );
        assert_eq!(
            host.observe_cancellation(),
            None,
            "scripted in-flight model cancellation must not depend on cooperative cancellation"
        );
        assert!(
            host.call_log().contains(&MockHostCall::StreamModel),
            "the driver must reach the in-flight model call before mapping cancellation"
        );
    }

    #[test]
    fn executor_model_credential_diagnostics_map_to_credentials_category() {
        let mapped = map_executor_error(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::CredentialUnavailable,
            safe_summary: LoopSafeSummary::new("model credentials are unavailable").expect("safe"),
            reason_kind: None,
            diagnostic_ref: None,
            detail: None,
        });

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY.to_string(),
                // No upstream detail: the bounded safe summary is the fallback.
                detail: Some("model credentials are unavailable".to_string()),
            }
        );
    }

    #[test]
    fn executor_budget_accounting_diagnostics_preserve_distinct_failure_category() {
        let mapped = map_executor_error(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::BudgetAccountingFailed,
            safe_summary: LoopSafeSummary::new("resource accounting storage is unavailable")
                .expect("safe"),
            reason_kind: None,
            diagnostic_ref: None,
            detail: None,
        });

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: "budget_accounting_failed".to_string(),
                detail: Some("resource accounting storage is unavailable".to_string()),
            }
        );
    }

    #[test]
    fn executor_host_diagnostics_preserve_model_credit_exhaustion_category() {
        let mapped = map_executor_error(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::CredentialUnavailable,
            safe_summary: LoopSafeSummary::new("safe summary wording is display-only")
                .expect("safe"),
            reason_kind: Some(MODEL_CREDITS_EXHAUSTED_REASON_KIND),
            diagnostic_ref: None,
            detail: None,
        });

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: MODEL_CREDITS_EXHAUSTED_CATEGORY.to_string(),
                detail: Some("safe summary wording is display-only".to_string()),
            }
        );
    }

    #[test]
    fn executor_model_diagnostics_carry_detail_into_failed() {
        let mapped = map_executor_error(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::CredentialUnavailable,
            safe_summary: LoopSafeSummary::new("model credentials are unavailable").expect("safe"),
            reason_kind: None,
            diagnostic_ref: None,
            detail: Some("HTTP 404 model not found".to_string()),
        });

        match mapped {
            AgentLoopDriverError::Failed { detail, .. } => {
                assert_eq!(detail.as_deref(), Some("HTTP 404 model not found"));
            }
            other => panic!("expected Failed with detail, got {other:?}"),
        }
    }

    #[test]
    fn executor_diagnostics_detail_is_rescrubbed_at_the_driver_seam() {
        // Fail-closed backstop: executor-side producers (checkpoint/mapping/
        // model stages) scrub detail with the token-prefix pass only —
        // ironclaw_agent_loop cannot depend on the hardened scrubber. This
        // seam is where the detail becomes driver/model-visible, so registry
        // secrets must be redacted here even if an upstream producer missed
        // them. The descriptive cause survives.
        let secret = concat!("ghp_", "012345678901234567890123456789012345");
        let mapped = map_executor_error(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Model,
            kind: AgentLoopHostErrorKind::CredentialUnavailable,
            safe_summary: LoopSafeSummary::new("model credentials are unavailable").expect("safe"),
            reason_kind: None,
            diagnostic_ref: None,
            detail: Some(format!("provider rejected token {secret} at /host/route")),
        });

        match mapped {
            AgentLoopDriverError::Failed { detail, .. } => {
                let detail = detail.expect("detail must survive the seam");
                assert!(
                    !detail.contains(secret),
                    "registry secret must be redacted at the driver seam: {detail}"
                );
                assert!(
                    detail.contains("provider rejected token"),
                    "descriptive cause must survive: {detail}"
                );
                assert!(
                    detail.contains("/host/route"),
                    "paths must survive on the model-visible detail: {detail}"
                );
            }
            other => panic!("expected Failed with detail, got {other:?}"),
        }
    }

    #[test]
    fn non_model_stage_with_credit_reason_maps_to_unavailable() {
        const CREDIT_SUMMARY: &str = "model provider account is out of credits";
        let mapped = map_executor_error(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Prompt,
            kind: AgentLoopHostErrorKind::CredentialUnavailable,
            safe_summary: LoopSafeSummary::new(CREDIT_SUMMARY).expect("safe"),
            reason_kind: Some(MODEL_CREDITS_EXHAUSTED_REASON_KIND),
            diagnostic_ref: None,
            detail: None,
        });

        assert_eq!(
            mapped,
            AgentLoopDriverError::Unavailable {
                reason: format!("Prompt: {CREDIT_SUMMARY}")
            }
        );
    }

    #[test]
    fn non_model_stage_with_credential_unavailable_maps_to_unavailable() {
        const CREDENTIAL_SUMMARY: &str = "model credentials are unavailable";
        let mapped = map_executor_error(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Prompt,
            kind: AgentLoopHostErrorKind::CredentialUnavailable,
            safe_summary: LoopSafeSummary::new(CREDENTIAL_SUMMARY).expect("safe"),
            reason_kind: None,
            diagnostic_ref: None,
            detail: None,
        });

        assert_eq!(
            mapped,
            AgentLoopDriverError::Unavailable {
                reason: format!("Prompt: {CREDENTIAL_SUMMARY}")
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
                    resume_disposition: None,
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
        let source_context = test_run_context("planned-driver-source-run");
        let mut restored_state = LoopExecutionState::initial_for_run(&source_context);
        restored_state.iteration = 7;
        restored_state.input_cursor = LoopInputCursor::from_host_token(
            &source_context,
            ironclaw_turns::run_profile::LoopInputCursorToken::new("input-cursor:source-seen")
                .expect("valid cursor"),
        );
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
                    resume_disposition: None,
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
                    resume_disposition: None,
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
                    resume_disposition: None,
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
                    resume_disposition: None,
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
                    resume_disposition: None,
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
                assert_eq!(failed.reason_kind.as_str(), "checkpoint_unavailable");
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

    #[async_trait::async_trait]
    impl LoopCancellationPort for ResumePayloadHost {
        fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
            self.inner.observe_cancellation()
        }

        async fn cancellation_requested(&self) -> LoopCancellationSignal {
            self.inner.cancellation_requested().await
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
    impl LoopCompactionPort for ResumePayloadHost {
        async fn compact_loop_context(
            &self,
            request: LoopCompactionRequest,
        ) -> Result<LoopCompactionOutcome, LoopCompactionError> {
            self.inner.compact_loop_context(request).await
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
            request: LoopRequest,
        ) -> Result<ironclaw_host_api::Resolution, AgentLoopHostError> {
            self.inner.invoke_capability(request).await
        }

        async fn invoke_capability_batch(
            &self,
            request: LoopRequestBatch,
        ) -> Result<ironclaw_host_api::ResolutionBatch, AgentLoopHostError> {
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

    // ---- auth-deny disposition injection tests ---------------------------------

    /// Helper: build a `LoopExecutionState` with a parked `pending_auth_resume`
    /// so we can assert the injection path stamps the disposition onto it.
    ///
    /// `last_gate` is set to the same gate_ref as `pending_auth_resume` so the
    /// narrowed stamping logic (which matches on `last_gate`) targets this slot.
    fn state_with_pending_auth_resume(
        context: &LoopRunContext,
    ) -> ironclaw_agent_loop::state::LoopExecutionState {
        use ironclaw_agent_loop::state::PendingAuthResume;
        use ironclaw_host_api::CapabilityId;
        use ironclaw_turns::LoopGateRef;
        use ironclaw_turns::run_profile::{CapabilityInputRef, CapabilitySurfaceVersion};

        let gate_ref = LoopGateRef::new("gate:test-auth-deny").expect("valid gate ref");
        let activity_id = ironclaw_turns::CapabilityActivityId::new();
        let mut state = ironclaw_agent_loop::state::LoopExecutionState::initial_for_run(context);
        state.last_gate = Some(gate_ref.clone());
        state.pending_auth_resume = Some(PendingAuthResume {
            gate_ref,
            capability_id: CapabilityId::new("test.capability").expect("valid capability id"),
            surface_version: CapabilitySurfaceVersion::new("surface:v1")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:test-auth-deny").expect("valid input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
            resume_token: None,
            activity_id,
            prior_approval: None,
            disposition: None,
        });
        state
    }

    /// Verifies that the `resume` method stamps `GateResumeDisposition::Denied`
    /// onto `pending_auth_resume` when the run context carries a denial, then
    /// passes the modified state to the executor.
    ///
    /// The injection is exercised by confirming the executor does NOT re-block
    /// on auth: the run produces a terminal exit (completed or failed-auth-denied)
    /// rather than another `Blocked` exit, which is what would happen if the
    /// `disposition` were left as `None` and the executor re-dispatched the
    /// parked capability.
    ///
    /// "Test through the caller": the assertion drives `PlannedDriver::resume`,
    /// not the injection snippet in isolation.
    #[tokio::test]
    async fn resume_with_auth_deny_disposition_stamps_denial_onto_pending_auth_resume() {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");

        // Build a run context (no disposition on context — disposition travels
        // via the resume request, not the host context).
        let context = run_context_for_driver(&driver);
        let disposition = Some(GateResumeDisposition::Denied);

        // Stage a checkpoint payload whose execution state has a parked auth
        // resume (disposition: None — as it would be when written at block time).
        let staged_state = state_with_pending_auth_resume(&context);
        // Encode and re-decode to exercise the serde boundary (mirrors what the
        // production checkpoint path does via RedactedCheckpointPayload).
        let payload_bytes = serde_json::to_vec(&staged_state).expect("serialize checkpoint state");
        let loaded = LoadedCheckpointPayload {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: context.checkpoint_schema_id.clone(),
            schema_version: context.checkpoint_schema_version,
            payload: RedactedCheckpointPayload::new(payload_bytes)
                .expect("valid checkpoint payload"),
        };
        let checkpoint_id = TurnCheckpointId::new();

        // The denied resume crosses the capability port as a typed terminal
        // auth resume (the host terminalizes it as gate-declined); the loop
        // then continues to the model, which closes the turn.
        let (inner, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .script(
                ScenarioScript::reply_only("denial surfaced").with_capability_outcomes(vec![vec![
                    ScriptedCapabilityOutcome::failed("gate_declined"),
                ]]),
            )
            .build();
        let host = ResumePayloadHost::new(inner, checkpoint_id, loaded);

        let result = driver
            .resume(
                AgentLoopDriverResumeRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    checkpoint_id,
                    resolved_run_profile: context.resolved_run_profile.clone(),
                    resume_disposition: disposition,
                },
                &host,
            )
            .await;

        // The executor must not re-block on auth. A denial-stamped pending auth
        // resume is surfaced as a model-visible failure and the loop continues
        // to completion or another terminal state — never another Blocked exit.
        if let ironclaw_turns::LoopExit::Blocked(_) =
            result.expect("resume should produce a terminal loop exit")
        {
            panic!(
                "resume with GateResumeDisposition::Denied must not re-block on auth; \
                 the executor must surface a model-visible denial and continue"
            );
        }
    }

    /// Unit-level smoke test: confirm that `stamp_resume_disposition` stamps
    /// `Some(Denied)` onto `pending_auth_resume.disposition` when `last_gate`
    /// matches the auth slot's gate_ref — exercises the real helper, not a copy.
    #[test]
    fn auth_deny_disposition_is_stamped_onto_pending_auth_resume_before_execution() {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);

        let mut state = state_with_pending_auth_resume(&context);
        // Confirm the state starts with no disposition (precondition).
        assert!(
            state
                .pending_auth_resume
                .as_ref()
                .expect("pending_auth_resume must be set")
                .disposition
                .is_none(),
            "staged pending_auth_resume must start with disposition: None"
        );

        // Call the real helper — not an inline copy of the logic.
        stamp_resume_disposition(&mut state, GateResumeDisposition::Denied);

        // Assert the disposition was stamped onto the auth slot.
        let disposition = state
            .pending_auth_resume
            .expect("pending_auth_resume must survive round-trip")
            .disposition
            .expect("disposition must be Some after stamp_resume_disposition");
        assert!(
            matches!(disposition, GateResumeDisposition::Denied),
            "disposition must be Denied after stamp_resume_disposition, got {disposition:?}"
        );
    }

    /// Confirm the injection is a no-op when `last_gate` is `None` — do not
    /// regress the normal (non-deny) resume path where no gate was recorded.
    #[test]
    fn auth_deny_disposition_injection_is_noop_when_last_gate_is_none() {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);

        let mut state = state_with_pending_auth_resume(&context);
        // Clear last_gate to simulate a checkpoint without a recorded gate.
        state.last_gate = None;

        // stamp_resume_disposition should be a no-op when last_gate is None.
        stamp_resume_disposition(&mut state, GateResumeDisposition::Denied);

        // Disposition must remain None — no-op.
        assert!(
            state
                .pending_auth_resume
                .expect("pending_auth_resume must survive")
                .disposition
                .is_none(),
            "disposition must remain None when last_gate is None"
        );
    }

    // ---- approval-deny disposition injection tests -----------------------------

    /// Helper: build a `LoopExecutionState` with a parked `pending_approval_resume`
    /// so we can assert the injection path stamps the disposition onto it.
    ///
    /// `last_gate` is set to the same gate_ref as `pending_approval_resume` so
    /// the narrowed stamping logic (which matches on `last_gate`) targets this slot.
    fn state_with_pending_approval_resume(
        context: &LoopRunContext,
    ) -> ironclaw_agent_loop::state::LoopExecutionState {
        use ironclaw_agent_loop::state::PendingApprovalResume;
        use ironclaw_host_api::{ApprovalRequestId, CapabilityId, CorrelationId};
        use ironclaw_turns::LoopGateRef;
        use ironclaw_turns::run_profile::{
            CapabilityInputRef, CapabilityResumeToken, CapabilitySurfaceVersion,
        };

        let gate_ref = LoopGateRef::new("gate:test-approval-deny").expect("valid gate ref");
        let activity_id = ironclaw_turns::CapabilityActivityId::new();
        let mut state = ironclaw_agent_loop::state::LoopExecutionState::initial_for_run(context);
        state.last_gate = Some(gate_ref.clone());
        state.pending_approval_resume = Some(PendingApprovalResume {
            gate_ref,
            capability_id: CapabilityId::new("test.capability").expect("valid capability id"),
            approval_request_id: ApprovalRequestId::new(),
            resume_token: CapabilityResumeToken::new(activity_id.to_string())
                .expect("valid resume token"),
            activity_id,
            correlation_id: CorrelationId::new(),
            surface_version: CapabilitySurfaceVersion::new("surface:v1")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:test-approval-deny")
                .expect("valid input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
            disposition: None,
        });
        state
    }

    /// Unit-level smoke test: confirm that `stamp_resume_disposition` stamps
    /// `Some(Denied)` onto `pending_approval_resume.disposition` when `last_gate`
    /// matches the approval slot's gate_ref, and does NOT stamp the absent auth
    /// slot — exercises the real helper, not a copy.
    #[test]
    fn approval_deny_disposition_is_stamped_onto_pending_approval_resume_before_execution() {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);

        let mut state = state_with_pending_approval_resume(&context);
        // Confirm preconditions: approval resume has no disposition; auth resume absent.
        assert!(
            state
                .pending_approval_resume
                .as_ref()
                .expect("pending_approval_resume must be set")
                .disposition
                .is_none(),
            "staged pending_approval_resume must start with disposition: None"
        );
        assert!(
            state.pending_auth_resume.is_none(),
            "pending_auth_resume must be absent in this fixture"
        );

        // Call the real helper — not an inline copy.
        stamp_resume_disposition(&mut state, GateResumeDisposition::Denied);

        // Assert the disposition was stamped onto the approval slot.
        let disposition = state
            .pending_approval_resume
            .expect("pending_approval_resume must survive round-trip")
            .disposition
            .expect("disposition must be Some after stamp_resume_disposition");
        assert!(
            matches!(disposition, GateResumeDisposition::Denied),
            "disposition must be Denied after stamp_resume_disposition, got {disposition:?}"
        );
        // Auth resume must remain absent — stamping must not create it.
        assert!(
            state.pending_auth_resume.is_none(),
            "pending_auth_resume must remain absent when only approval resume is present"
        );
    }

    /// Confirm that `stamp_resume_disposition` is a no-op when `last_gate` is
    /// `None` — do not regress the normal (non-deny) approval resume path.
    #[test]
    fn approval_deny_disposition_injection_is_noop_when_last_gate_is_none() {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);

        let mut state = state_with_pending_approval_resume(&context);
        // Clear last_gate to simulate a checkpoint without a recorded gate.
        state.last_gate = None;

        stamp_resume_disposition(&mut state, GateResumeDisposition::Denied);

        // Disposition must remain None — no-op.
        assert!(
            state
                .pending_approval_resume
                .expect("pending_approval_resume must survive round-trip")
                .disposition
                .is_none(),
            "disposition must remain None when last_gate is None"
        );
    }

    // ---- external-tool disposition injection tests --------------------------------

    fn state_with_pending_external_tool_resume(
        context: &LoopRunContext,
    ) -> ironclaw_agent_loop::state::LoopExecutionState {
        use ironclaw_agent_loop::state::PendingExternalToolResume;
        use ironclaw_host_api::CapabilityId;
        use ironclaw_turns::CapabilityActivityId;
        use ironclaw_turns::LoopGateRef;
        use ironclaw_turns::run_profile::{CapabilityInputRef, CapabilitySurfaceVersion};

        let gate_ref = LoopGateRef::new("gate:test-external-tool-deny").expect("valid gate ref");
        let mut state = ironclaw_agent_loop::state::LoopExecutionState::initial_for_run(context);
        state.last_gate = Some(gate_ref.clone());
        state.pending_external_tool_resume = Some(PendingExternalToolResume {
            gate_ref,
            capability_id: CapabilityId::new("test.external_tool").expect("valid capability id"),
            activity_id: CapabilityActivityId::new(),
            surface_version: CapabilitySurfaceVersion::new("surface:v1")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:test-external-tool-deny")
                .expect("valid input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
            disposition: None,
        });
        state
    }

    #[test]
    fn external_tool_deny_disposition_is_stamped_onto_pending_external_tool_resume_before_execution()
     {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);

        let mut state = state_with_pending_external_tool_resume(&context);
        assert!(
            state
                .pending_external_tool_resume
                .as_ref()
                .expect("pending_external_tool_resume must be set")
                .disposition
                .is_none(),
            "staged pending_external_tool_resume must start with disposition: None"
        );

        stamp_resume_disposition(&mut state, GateResumeDisposition::Denied);

        let disposition = state
            .pending_external_tool_resume
            .expect("pending_external_tool_resume must survive round-trip")
            .disposition
            .expect("disposition must be Some after stamp_resume_disposition");
        assert!(
            matches!(disposition, GateResumeDisposition::Denied),
            "disposition must be Denied after stamp_resume_disposition, got {disposition:?}"
        );
        assert!(
            state.pending_auth_resume.is_none(),
            "pending_auth_resume must remain absent when only external tool resume is present"
        );
        assert!(
            state.pending_approval_resume.is_none(),
            "pending_approval_resume must remain absent when only external tool resume is present"
        );
    }

    // ---- dual-slot regression test -----------------------------------------------

    /// Helper: build a `LoopExecutionState` carrying BOTH `pending_auth_resume`
    /// (gate_ref = `gate:dual-auth`) AND `pending_approval_resume`
    /// (gate_ref = `gate:dual-approval`), with `last_gate` pointing to the
    /// approval gate.
    ///
    /// This mirrors the real scenario GateStage produces: an auth gate blocked
    /// during the first dispatch, then a non-auth (approval) gate fired mid-
    /// re-dispatch — leaving both slots populated simultaneously.
    fn state_with_both_resumes_last_gate_approval(
        context: &LoopRunContext,
    ) -> ironclaw_agent_loop::state::LoopExecutionState {
        use ironclaw_agent_loop::state::{PendingApprovalResume, PendingAuthResume};
        use ironclaw_host_api::{ApprovalRequestId, CapabilityId, CorrelationId};
        use ironclaw_turns::LoopGateRef;
        use ironclaw_turns::run_profile::{
            CapabilityInputRef, CapabilityResumeToken, CapabilitySurfaceVersion,
        };

        let auth_gate_ref = LoopGateRef::new("gate:dual-auth").expect("valid gate ref");
        let approval_gate_ref = LoopGateRef::new("gate:dual-approval").expect("valid gate ref");
        let auth_activity_id = ironclaw_turns::CapabilityActivityId::new();
        let approval_activity_id = ironclaw_turns::CapabilityActivityId::new();

        let mut state = ironclaw_agent_loop::state::LoopExecutionState::initial_for_run(context);
        // last_gate = approval — the run is currently blocked on this gate.
        state.last_gate = Some(approval_gate_ref.clone());
        state.pending_auth_resume = Some(PendingAuthResume {
            gate_ref: auth_gate_ref,
            capability_id: CapabilityId::new("test.capability.auth").expect("valid capability id"),
            surface_version: CapabilitySurfaceVersion::new("surface:v1")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:dual-auth").expect("valid input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
            resume_token: None,
            activity_id: auth_activity_id,
            prior_approval: None,
            disposition: None,
        });
        state.pending_approval_resume = Some(PendingApprovalResume {
            gate_ref: approval_gate_ref,
            capability_id: CapabilityId::new("test.capability.approval")
                .expect("valid capability id"),
            approval_request_id: ApprovalRequestId::new(),
            resume_token: CapabilityResumeToken::new(approval_activity_id.to_string())
                .expect("valid resume token"),
            activity_id: approval_activity_id,
            correlation_id: CorrelationId::new(),
            surface_version: CapabilitySurfaceVersion::new("surface:v1")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:dual-approval").expect("valid input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
            disposition: None,
        });
        state
    }

    /// Bug-regression: when BOTH `pending_auth_resume` and
    /// `pending_approval_resume` are set and `last_gate` points to the approval
    /// gate, denying the approval must stamp ONLY the approval slot. The auth
    /// slot must remain `disposition: None`.
    ///
    /// This is the scenario the old "stamp both" code got wrong: the auth resume
    /// would have been corrupted with `Some(Denied)`, causing CapabilityStage to
    /// misattribute the denial and lose the auth context on the next tick.
    ///
    /// Drives `PlannedDriver::resume()` end-to-end to exercise the real
    /// stamping path, not an inline copy.
    #[tokio::test]
    async fn resume_with_approval_deny_does_not_corrupt_unrelated_auth_resume() {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);

        // Build a checkpoint with BOTH resumes populated; last_gate = approval.
        let staged_state = state_with_both_resumes_last_gate_approval(&context);
        // Precondition: both slots start without a disposition.
        assert!(
            staged_state
                .pending_auth_resume
                .as_ref()
                .expect("auth resume must be set")
                .disposition
                .is_none(),
            "pending_auth_resume must start with disposition: None"
        );
        assert!(
            staged_state
                .pending_approval_resume
                .as_ref()
                .expect("approval resume must be set")
                .disposition
                .is_none(),
            "pending_approval_resume must start with disposition: None"
        );

        let payload_bytes =
            serde_json::to_vec(&staged_state).expect("serialize dual-slot checkpoint state");
        let loaded = LoadedCheckpointPayload {
            kind: LoopCheckpointKind::BeforeModel,
            schema_id: context.checkpoint_schema_id.clone(),
            schema_version: context.checkpoint_schema_version,
            payload: RedactedCheckpointPayload::new(payload_bytes)
                .expect("valid checkpoint payload"),
        };
        let checkpoint_id = TurnCheckpointId::new();

        let (inner, _checkpoints) = MockAgentLoopDriverHost::builder()
            .run_context(context.clone())
            .build();
        let host = ResumePayloadHost::new(inner, checkpoint_id, loaded);

        // Resume with approval denied — the key assertion is that the executor
        // does NOT re-block (which would happen if both dispositions were stamped,
        // confusing the capability stage) and that the auth slot was NOT corrupted.
        // We capture the result to check it isn't a blocked exit; the primary
        // regression proof is in the unit-level assertion below that drives the
        // helper directly.
        let result = driver
            .resume(
                AgentLoopDriverResumeRequest {
                    turn_id: context.turn_id,
                    run_id: context.run_id,
                    checkpoint_id,
                    resolved_run_profile: context.resolved_run_profile.clone(),
                    resume_disposition: Some(GateResumeDisposition::Denied),
                },
                &host,
            )
            .await;

        // The loop must not re-block on the approval gate — the denial is stamped
        // and the executor surfaces it as a model-visible failure.
        if let ironclaw_turns::LoopExit::Blocked(_) =
            result.expect("resume should produce a terminal loop exit")
        {
            panic!(
                "resume with approval GateResumeDisposition::Denied must not re-block; \
                 the executor must surface a model-visible denial and continue"
            );
        }
    }

    /// Unit-level regression assertion for the dual-slot scenario: drives
    /// `stamp_resume_disposition` directly and asserts that the auth slot's
    /// `disposition` remains `None` while the approval slot receives `Denied`.
    ///
    /// This complements the async `resume` test above with a precise assertion
    /// on the intermediate state — the auth slot disposition — that the
    /// executor-level test cannot cheaply inspect after the fact.
    #[test]
    fn stamp_resume_disposition_does_not_corrupt_auth_slot_when_approval_gate_is_last() {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);

        let mut state = state_with_both_resumes_last_gate_approval(&context);

        stamp_resume_disposition(&mut state, GateResumeDisposition::Denied);

        // CRITICAL: auth slot must NOT be corrupted — this is the bug assertion.
        assert!(
            state
                .pending_auth_resume
                .as_ref()
                .expect("pending_auth_resume must still be present after stamp")
                .disposition
                .is_none(),
            "BUG REGRESSION: pending_auth_resume.disposition must remain None \
             when the denial is for the approval gate, not the auth gate"
        );

        // Approval slot must be stamped.
        let approval_disposition = state
            .pending_approval_resume
            .expect("pending_approval_resume must still be present after stamp")
            .disposition
            .expect("pending_approval_resume.disposition must be Some(Denied) after stamp");
        assert!(
            matches!(approval_disposition, GateResumeDisposition::Denied),
            "pending_approval_resume.disposition must be Denied, got {approval_disposition:?}"
        );
    }

    // ---- ambiguous-gate fail-closed test -----------------------------------------

    /// Helper: build a `LoopExecutionState` where BOTH `pending_auth_resume` AND
    /// `pending_approval_resume` carry the SAME `gate_ref`, and `last_gate` is
    /// set to that same ref.  This is the impossible-in-practice state that
    /// `stamp_resume_disposition` must handle by stamping NEITHER slot.
    fn state_with_both_slots_same_gate_ref(
        context: &LoopRunContext,
    ) -> ironclaw_agent_loop::state::LoopExecutionState {
        use ironclaw_agent_loop::state::{PendingApprovalResume, PendingAuthResume};
        use ironclaw_host_api::{ApprovalRequestId, CapabilityId, CorrelationId};
        use ironclaw_turns::LoopGateRef;
        use ironclaw_turns::run_profile::{
            CapabilityInputRef, CapabilityResumeToken, CapabilitySurfaceVersion,
        };

        let gate_ref = LoopGateRef::new("gate:ambiguous-shared").expect("valid gate ref");
        let auth_activity_id = ironclaw_turns::CapabilityActivityId::new();
        let approval_activity_id = ironclaw_turns::CapabilityActivityId::new();
        let mut state = ironclaw_agent_loop::state::LoopExecutionState::initial_for_run(context);
        state.last_gate = Some(gate_ref.clone());
        state.pending_auth_resume = Some(PendingAuthResume {
            gate_ref: gate_ref.clone(),
            capability_id: CapabilityId::new("test.capability.auth").expect("valid capability id"),
            surface_version: CapabilitySurfaceVersion::new("surface:v1")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:ambiguous-auth").expect("valid input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
            resume_token: None,
            activity_id: auth_activity_id,
            prior_approval: None,
            disposition: None,
        });
        state.pending_approval_resume = Some(PendingApprovalResume {
            gate_ref,
            capability_id: CapabilityId::new("test.capability.approval")
                .expect("valid capability id"),
            approval_request_id: ApprovalRequestId::new(),
            resume_token: CapabilityResumeToken::new(approval_activity_id.to_string())
                .expect("valid resume token"),
            activity_id: approval_activity_id,
            correlation_id: CorrelationId::new(),
            surface_version: CapabilitySurfaceVersion::new("surface:v1")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:ambiguous-approval")
                .expect("valid input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
            disposition: None,
        });
        state
    }

    /// Fail-closed safety test: when BOTH `pending_auth_resume` and
    /// `pending_approval_resume` carry the same `gate_ref` as `last_gate`,
    /// `stamp_resume_disposition` must stamp NEITHER slot rather than
    /// misattribute the denial to whichever slot the old `else if` happened
    /// to evaluate first.
    #[test]
    fn stamp_resume_disposition_fails_closed_when_both_slots_match_last_gate() {
        use ironclaw_turns::GateResumeDisposition;

        let registry = build_loop_family_registry().expect("registry");
        let driver = PlannedDriver::default_from_registry(&registry).expect("driver");
        let context = run_context_for_driver(&driver);

        let mut state = state_with_both_slots_same_gate_ref(&context);

        // Precondition: both slots start without a disposition.
        assert!(
            state
                .pending_auth_resume
                .as_ref()
                .expect("pending_auth_resume must be set")
                .disposition
                .is_none(),
            "pending_auth_resume must start with disposition: None"
        );
        assert!(
            state
                .pending_approval_resume
                .as_ref()
                .expect("pending_approval_resume must be set")
                .disposition
                .is_none(),
            "pending_approval_resume must start with disposition: None"
        );

        // Call with the ambiguous state — should warn and stamp neither.
        stamp_resume_disposition(&mut state, GateResumeDisposition::Denied);

        // BOTH dispositions must remain None — fail closed.
        assert!(
            state
                .pending_auth_resume
                .as_ref()
                .expect("pending_auth_resume must survive")
                .disposition
                .is_none(),
            "pending_auth_resume.disposition must remain None when both slots are ambiguous"
        );
        assert!(
            state
                .pending_approval_resume
                .as_ref()
                .expect("pending_approval_resume must survive")
                .disposition
                .is_none(),
            "pending_approval_resume.disposition must remain None when both slots are ambiguous"
        );
    }
}
