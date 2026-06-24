use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, HostApiError, PermissionMode,
    ResourceScope, ResourceUsage, RuntimeDispatchErrorKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

use super::{
    FIRST_PARTY_MAX_OUTPUT_BYTES, bounded_input_size, bounded_output_bytes,
    first_party_capability_manifest, input_error, resource_profile,
};

pub const WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID: &str = "builtin.workflow_report_stage_result";

#[derive(Debug, Clone, Deserialize)]
pub struct ReportWorkflowStageResultInput {
    /// Optional and **never trusted**: the host derives the authoritative
    /// workflow run id from the executing thread. The model has no authoritative
    /// source for it, so the input schema does not require it; when present it is
    /// cross-checked against the thread-derived id as defense in depth.
    pub workflow_run_id: Option<String>,
    /// Optional and **never trusted** (see [`Self::workflow_run_id`]);
    /// cross-checked against the thread-derived id when present.
    pub stage_run_id: Option<String>,
    /// Optional and **never trusted**; its FORMAT is validated when present, but
    /// it is not a binding axis (the host binds via the executing thread).
    pub turn_run_id: Option<String>,
    pub stage: String,
    pub schema_version: String,
    /// Accepted for wire back-compat only, optional, and **never trusted**. The
    /// model has no authoritative source for this value (it is not injected into
    /// any stage prompt); the host derives the authoritative stage identity from
    /// the trusted executing thread (see [`ExecutingStageThread`]). Do not
    /// re-introduce a check against it.
    pub completion_nonce: Option<String>,
    pub result: Value,
}

/// The trusted, host-stamped scope of the thread that is executing the turn
/// reporting this stage result.
///
/// The composition sink uses [`ResourceScope::thread_id`] plus the thread's
/// host-written metadata to derive the authoritative stage identity, so a
/// turn can only complete the stage it is actually executing — model-supplied
/// identity fields are cross-checked against this, never trusted on their own.
#[derive(Debug, Clone)]
pub struct ExecutingStageThread {
    pub scope: ResourceScope,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowStageResultAck {
    pub accepted: bool,
    pub duplicate: bool,
    pub stage_run_id: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum WorkflowStageResultSinkError {
    #[error("invalid workflow stage result input")]
    InvalidInput { reason: String },
    #[error("workflow stage result binding mismatch")]
    MismatchedBinding,
    #[error("workflow stage result stale attempt")]
    StaleAttempt,
    #[error("workflow stage is not active")]
    StageNotActive,
    #[error("workflow stage result validation failed")]
    ValidationFailed { reason: String },
    #[error("workflow stage result sink unavailable")]
    Unavailable,
}

#[async_trait]
pub trait WorkflowStageResultSink: Send + Sync {
    async fn report_stage_result(
        &self,
        executing_thread: ExecutingStageThread,
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError>;
}

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    let mut manifest = first_party_capability_manifest(
        WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID,
        "Report an opaque structured workflow stage result to the host-composed workflow sink",
        vec![EffectKind::DispatchCapability],
        PermissionMode::Allow,
        resource_profile(),
    )?;
    manifest.input_schema_ref = CapabilityProfileSchemaRef::new(
        "schemas/builtin/workflow-report-stage-result.input.v1.json",
    )?;
    manifest.output_schema_ref = CapabilityProfileSchemaRef::new(
        "schemas/builtin/workflow-report-stage-result.output.v1.json",
    )?;
    Ok(manifest)
}

pub(super) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    sink: Arc<dyn WorkflowStageResultSink>,
) -> Result<(), HostApiError> {
    registry.insert_handler(
        CapabilityId::new(WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID)?,
        Arc::new(WorkflowResultToolHandler { sink }),
    );
    Ok(())
}

struct WorkflowResultToolHandler {
    sink: Arc<dyn WorkflowStageResultSink>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for WorkflowResultToolHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        if request.capability_id.as_str() != WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::UndeclaredCapability,
            ));
        }
        bounded_input_size(request.capability_id.as_str(), &request.input)?;
        let started = Instant::now();
        // Capture the trusted host-stamped scope BEFORE moving `request.input`
        // out of the request; the sink derives authoritative stage identity
        // from this, never from the model-supplied input fields.
        let executing_thread = ExecutingStageThread {
            scope: request.scope.clone(),
        };
        let input: ReportWorkflowStageResultInput =
            serde_json::from_value(request.input).map_err(|_| input_error())?;
        let ack = self
            .sink
            .report_stage_result(executing_thread, input)
            .await
            .map_err(workflow_sink_error)?;
        let output = serde_json::to_value(ack)
            .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InvalidResult))?;
        let output_bytes = bounded_output_bytes(&output, FIRST_PARTY_MAX_OUTPUT_BYTES)?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            elapsed_usage_with_bytes(started, output_bytes),
        ))
    }
}

fn workflow_sink_error(error: WorkflowStageResultSinkError) -> FirstPartyCapabilityError {
    match error {
        WorkflowStageResultSinkError::InvalidInput { .. } => workflow_safe_error(
            RuntimeDispatchErrorKind::InputEncode,
            "invalid workflow stage result input",
        ),
        WorkflowStageResultSinkError::MismatchedBinding => workflow_safe_error(
            RuntimeDispatchErrorKind::PolicyDenied,
            "workflow stage result binding mismatch",
        ),
        WorkflowStageResultSinkError::StaleAttempt => workflow_safe_error(
            RuntimeDispatchErrorKind::OperationFailed,
            "workflow stage result stale attempt",
        ),
        WorkflowStageResultSinkError::StageNotActive => workflow_safe_error(
            RuntimeDispatchErrorKind::OperationFailed,
            "workflow stage is not active",
        ),
        WorkflowStageResultSinkError::ValidationFailed { .. } => workflow_safe_error(
            RuntimeDispatchErrorKind::InputEncode,
            "workflow stage result validation failed",
        ),
        WorkflowStageResultSinkError::Unavailable => workflow_safe_error(
            RuntimeDispatchErrorKind::Backend,
            "workflow stage result sink unavailable",
        ),
    }
}

fn workflow_safe_error(
    kind: RuntimeDispatchErrorKind,
    reason: impl Into<String>,
) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::with_safe_summary(kind, reason)
}

fn elapsed_usage_with_bytes(started: Instant, output_bytes: u64) -> ResourceUsage {
    ResourceUsage {
        wall_clock_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        output_bytes,
        ..ResourceUsage::default()
    }
}
