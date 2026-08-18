//! Synthetic terminal result used by suppression-enabled scheduled runs.
//!
//! Native structured output is finalized outside the core loop and does not
//! use this capability. This module remains solely for the pre-existing typed
//! `nothing_to_report` automation outcome.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    ids::InvocationId,
    prepared_context::{STRUCTURED_RESULT_CAPABILITY_ID, STRUCTURED_RESULT_PROVIDER_TOOL_NAME},
    resolution::Resolution,
    result_meta::FailureKind,
    turn::LoopResultRef,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailureDetail, CapabilityProgress,
    MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleArtifact,
    ModelVisibleToolObservation, ObservationTrust, ToolObservationDetail, ToolObservationStatus,
    resolution, sanitize_model_visible_text,
};

use crate::{
    CapabilityResultWrite, DurablePersistence, SyntheticCapability, SyntheticCapabilityDescriptor,
    SyntheticCapabilityHandler, SyntheticCapabilityInvocation,
};

const NOTHING_TO_REPORT_TOOL_DESCRIPTION: &str =
    include_str!("../prompts/nothing_to_report_tool.md");

/// Bounded number of schema violations surfaced per rejected call.
const MAX_REPORTED_SCHEMA_VIOLATIONS: usize = 3;

/// Build the narrow terminal capability used by suppression-enabled scheduled runs.
/// Ordinary scheduled results remain assistant replies; this terminal output
/// exists only for the typed no-result outcome.
pub fn nothing_to_report_result_capability() -> Result<SyntheticCapability, AgentLoopHostError> {
    structured_result_capability_with_description(
        serde_json::json!({
            "type": "object",
            "properties": {
                "outcome": { "const": "nothing_to_report" }
            },
            "required": ["outcome"],
            "additionalProperties": false
        }),
        NOTHING_TO_REPORT_TOOL_DESCRIPTION,
    )
}

fn structured_result_capability_with_description(
    schema: serde_json::Value,
    description: &'static str,
) -> Result<SyntheticCapability, AgentLoopHostError> {
    let validator = jsonschema::validator_for(&schema).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "declared output schema does not compile",
        )
        .with_detail(format!("schema compilation failed: {error}"))
    })?;
    Ok(SyntheticCapability::new(
        SyntheticCapabilityDescriptor::new(
            STRUCTURED_RESULT_CAPABILITY_ID,
            STRUCTURED_RESULT_PROVIDER_TOOL_NAME,
            description,
            schema,
        )?,
        Arc::new(StructuredResultHandler {
            validator: Arc::new(validator),
        }),
    ))
}

struct StructuredResultHandler {
    validator: Arc<jsonschema::Validator>,
}

#[async_trait]
impl SyntheticCapabilityHandler for StructuredResultHandler {
    fn validate_provider_arguments(
        &self,
        _arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        // Registration must not terminalize a turn for a model-correctable
        // schema violation; `invoke` returns that shape as a model-visible
        // failure with a repair hint instead.
        Ok(())
    }

    async fn invoke(
        &self,
        invocation: SyntheticCapabilityInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
        // Strict-only validation (design §4.5): the terminal output must
        // validate against the declared schema or the attempt is rejected
        // into the repair path.
        let violations: Vec<String> = self
            .validator
            .iter_errors(&invocation.input)
            .take(MAX_REPORTED_SCHEMA_VIOLATIONS)
            .map(|error| {
                sanitize_model_visible_text(format!(
                    "{path}: {error}",
                    path = error.instance_path()
                ))
            })
            .collect();
        if !violations.is_empty() {
            let text = sanitize_model_visible_text(format!(
                "structured result rejected; fix these schema violations and call \
                 {STRUCTURED_RESULT_PROVIDER_TOOL_NAME} again: {}",
                violations.join("; ")
            ));
            return Ok(resolution::failed(
                FailureKind::InputEncode,
                "structured result failed schema validation".to_string(),
                CapabilityFailureDetail::Diagnostic { text },
            ));
        }

        let mut write = invocation
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &invocation.run_context,
                input_ref: &invocation.request.input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &invocation.request.capability_id,
                output: invocation.input.clone(),
                display_preview: None,
                // The validated value IS the run's terminal output: durable,
                // read back by the product tier after the run completes.
                durable_persistence: DurablePersistence::Persist,
            })
            .await?;
        write.model_observation = Some(ModelVisibleToolObservation {
            schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
            status: ToolObservationStatus::Success,
            summary: "Structured result recorded; the run is complete.".to_string(),
            detail: ToolObservationDetail::ResultReference {
                result_ref: write.result_ref.as_str().to_string(),
                byte_len: write.byte_len,
                preview: None,
                total_bytes: Some(write.byte_len),
                next_offset: None,
                item_count: None,
            },
            artifacts: vec![ModelVisibleArtifact {
                artifact_ref: write.result_ref.as_str().to_string(),
                summary: "Validated structured run result".to_string(),
            }],
            recovery: None,
            trust: ObservationTrust::UntrustedToolOutput,
        });
        let result_ref =
            LoopResultRef::new(write.result_ref.as_str().to_string()).map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "written structured result reference could not be represented",
                )
                .with_detail(format!("loop result reference validation failed: {error}"))
            })?;
        Ok(resolution::completed(
            result_ref,
            "structured result recorded".to_string(),
            CapabilityProgress::MadeProgress,
            // Terminate hint: the run's work is complete once the result is
            // recorded — the unbound stop conditions also key on this call's
            // completed signature.
            true,
            write.byte_len,
            write.output_digest,
            write.model_observation,
        ))
    }
}
