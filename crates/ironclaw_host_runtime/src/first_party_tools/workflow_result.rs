use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, HostApiError, PermissionMode,
    ResourceUsage, RuntimeDispatchErrorKind,
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
    pub workflow_run_id: String,
    pub stage_run_id: String,
    pub turn_run_id: String,
    pub stage: String,
    pub schema_version: String,
    pub completion_nonce: String,
    pub result: Value,
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
        let input: ReportWorkflowStageResultInput =
            serde_json::from_value(request.input).map_err(|_| input_error())?;
        let ack = self
            .sink
            .report_stage_result(input)
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
