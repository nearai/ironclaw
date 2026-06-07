use std::collections::{HashMap, HashSet};

use ironclaw_host_api::CapabilityId;
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopDriverHost, AppendCapabilityResultRef, CapabilityCallCandidate,
        CapabilityDescriptorView, CapabilityFailure, CapabilityFailureDetail, CapabilityInputIssue,
        CapabilityInputIssueCode, CapabilityInputRepair, CapabilityInvocation,
        CapabilityRecoveryHint, CapabilityResultMessage, CapabilitySurfaceVersion,
        ModelVisibleToolObservation, ObservationTrust, ProviderToolCallReference,
        SameCallRetryConstraint, ToolObservationDetail, ToolObservationStatus,
        ToolRecoveryObservation, VisibleCapabilitySurface,
    },
};

use crate::{
    state::{CapabilityCallSignature, LoopExecutionState},
    strategies::{CapabilityCallSummary, CapabilityErrorSummary, CapabilityFilter, GateKind},
};

use super::{AgentLoopExecutorError, capability_host_error};

pub(super) fn capability_invocation_from_candidate(
    call: CapabilityCallCandidate,
) -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: call.surface_version,
        capability_id: call.capability_id,
        input_ref: call.input_ref,
    }
}

pub(super) struct CapabilitySurfaceIndex<'a> {
    version: &'a CapabilitySurfaceVersion,
    descriptors: HashMap<&'a CapabilityId, &'a CapabilityDescriptorView>,
}

impl<'a> CapabilitySurfaceIndex<'a> {
    pub(super) fn new(surface: &'a VisibleCapabilitySurface) -> Self {
        let descriptors = surface
            .descriptors
            .iter()
            .map(|descriptor| (&descriptor.capability_id, descriptor))
            .collect();
        Self {
            version: &surface.version,
            descriptors,
        }
    }
}

pub(super) fn capability_summary(
    surface: &CapabilitySurfaceIndex<'_>,
    call: &CapabilityCallCandidate,
) -> CapabilityCallSummary {
    let concurrency_hint = surface
        .descriptors
        .get(&call.capability_id)
        .map(|descriptor| descriptor.concurrency_hint)
        .unwrap_or(ironclaw_turns::run_profile::ConcurrencyHint::Exclusive);
    CapabilityCallSummary {
        name: call.capability_id.clone(),
        concurrency_hint,
    }
}

pub(super) fn capability_is_visible(
    surface: &CapabilitySurfaceIndex<'_>,
    call: &CapabilityCallCandidate,
) -> bool {
    if &call.surface_version != surface.version {
        return false;
    }
    surface.descriptors.contains_key(&call.capability_id)
}

pub(super) fn apply_capability_filter(
    surface: &mut VisibleCapabilitySurface,
    filter: &CapabilityFilter,
) {
    surface
        .descriptors
        .retain(|descriptor| filter.permits(&descriptor.capability_id));
}

pub(super) fn push_call_signature_once(
    state: &mut LoopExecutionState,
    signatures: &mut HashSet<CapabilityCallSignature>,
    call: &CapabilityCallCandidate,
) -> Result<CapabilityCallSignature, AgentLoopExecutorError> {
    let signature = capability_call_signature(call)?;
    if signatures.insert(signature.clone()) {
        state.recent_call_signatures.push(signature.clone());
    }
    Ok(signature)
}

pub(super) fn capability_call_signature(
    call: &CapabilityCallCandidate,
) -> Result<CapabilityCallSignature, AgentLoopExecutorError> {
    let args = call
        .provider_replay
        .as_ref()
        .map(|replay| replay.arguments.clone())
        .unwrap_or_else(|| serde_json::json!({ "input_ref": call.input_ref.as_str() }));
    CapabilityCallSignature::from_call(call.capability_id.clone(), &args).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "capability call signature could not be built",
        }
    })
}

pub(super) async fn append_capability_result_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    call: &CapabilityCallCandidate,
    result: &CapabilityResultMessage,
) -> Result<(), AgentLoopExecutorError> {
    host.append_capability_result_ref(AppendCapabilityResultRef {
        result_ref: result.result_ref.clone(),
        safe_summary: result.safe_summary.clone(),
        provider_call: provider_tool_call_reference(call),
        model_observation: None,
    })
    .await
    .map_err(capability_host_error)?;
    Ok(())
}

pub(super) fn provider_tool_call_reference(
    call: &CapabilityCallCandidate,
) -> Option<ProviderToolCallReference> {
    let provider_replay = call.provider_replay.as_ref()?;
    Some(ProviderToolCallReference {
        provider_id: provider_replay.provider_id.clone(),
        provider_model_id: provider_replay.provider_model_id.clone(),
        provider_turn_id: provider_replay.provider_turn_id.clone(),
        provider_call_id: provider_replay.provider_call_id.clone(),
        provider_tool_name: provider_replay.provider_tool_name.clone(),
        capability_id: call.capability_id.clone(),
        arguments: provider_replay.arguments.clone(),
        response_reasoning: provider_replay.response_reasoning.clone(),
        reasoning: provider_replay.reasoning.clone(),
        signature: provider_replay.signature.clone(),
    })
}

pub(super) async fn append_capability_error_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    summary: &CapabilityErrorSummary,
    model_observation: Option<ModelVisibleToolObservation>,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_safe_summary_ref_with_observation(
        host,
        state,
        call,
        summary.safe_summary.as_str().to_string(),
        model_observation,
    )
    .await
}

pub(super) async fn append_capability_safe_summary_ref(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    safe_summary: String,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_safe_summary_ref_with_observation(host, state, call, safe_summary, None).await
}

async fn append_capability_safe_summary_ref_with_observation(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    safe_summary: String,
    model_observation: Option<ModelVisibleToolObservation>,
) -> Result<(), AgentLoopExecutorError> {
    if call.provider_replay.is_none() {
        return Ok(());
    }
    let result_ref = synthetic_provider_error_result_ref(call)?;
    host.append_capability_result_ref(AppendCapabilityResultRef {
        result_ref: result_ref.clone(),
        safe_summary,
        provider_call: provider_tool_call_reference(call),
        model_observation,
    })
    .await
    .map_err(capability_host_error)?;
    state.result_refs.push(result_ref);
    Ok(())
}

pub(super) fn model_visible_capability_failure_observation(
    failure: &CapabilityFailure,
) -> ModelVisibleToolObservation {
    match &failure.detail {
        Some(CapabilityFailureDetail::InvalidInput { issues }) => {
            invalid_input_observation(issues.clone())
        }
        _ => ModelVisibleToolObservation {
            status: ToolObservationStatus::Error,
            summary: format!("Capability failed with {}.", failure.error_kind.as_str()),
            detail: ToolObservationDetail::GenericFailure {
                failure_kind: failure.error_kind.clone(),
            },
            artifacts: Vec::new(),
            recovery: None,
            trust: ObservationTrust::UntrustedToolOutput,
        },
    }
}

fn invalid_input_observation(issues: Vec<CapabilityInputIssue>) -> ModelVisibleToolObservation {
    let repairs = issues.iter().map(input_issue_repair).collect();
    ModelVisibleToolObservation {
        status: ToolObservationStatus::Error,
        summary: "Tool input failed schema validation.".to_string(),
        detail: ToolObservationDetail::InvalidInput { issues },
        artifacts: Vec::new(),
        recovery: Some(ToolRecoveryObservation {
            same_call_retry: SameCallRetryConstraint::RequiresChangedInput,
            repairs,
            recovery_hint: CapabilityRecoveryHint::CorrectArgumentsBeforeRetry,
        }),
        trust: ObservationTrust::UntrustedToolOutput,
    }
}

fn input_issue_repair(issue: &CapabilityInputIssue) -> CapabilityInputRepair {
    match issue.code {
        CapabilityInputIssueCode::MissingRequired => CapabilityInputRepair::ProvideRequiredField {
            path: issue.path.clone(),
        },
        CapabilityInputIssueCode::UnexpectedField => CapabilityInputRepair::RemoveUnexpectedField {
            path: issue.path.clone(),
        },
        CapabilityInputIssueCode::TypeMismatch => CapabilityInputRepair::ChangeType {
            path: issue.path.clone(),
            expected: issue.expected.clone(),
        },
        CapabilityInputIssueCode::InvalidValue => CapabilityInputRepair::UseAllowedValue {
            path: issue.path.clone(),
        },
    }
}

pub(super) fn synthetic_provider_error_result_ref(
    call: &CapabilityCallCandidate,
) -> Result<LoopResultRef, AgentLoopExecutorError> {
    let provider_replay =
        call.provider_replay
            .as_ref()
            .ok_or(AgentLoopExecutorError::PlannerContract {
                detail: "provider replay metadata is required for provider error result ref",
            })?;
    let mut suffix = format!(
        "provider-error-{}-{}",
        sanitize_result_ref_suffix(&provider_replay.provider_turn_id),
        sanitize_result_ref_suffix(&provider_replay.provider_call_id)
    );
    suffix.truncate(240);
    LoopResultRef::new(format!("result:{suffix}")).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "provider error result ref was invalid",
        }
    })
}

pub(super) fn sanitize_result_ref_suffix(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        sanitized.push_str("unknown");
    }
    sanitized
}

pub(super) fn gate_tool_result_summary(kind: GateKind, outcome: &'static str) -> String {
    let gate = match kind {
        GateKind::Approval => "approval",
        GateKind::Auth => "auth",
        GateKind::Resource => "resource",
        GateKind::AwaitDependentRun => "await_dependent_run",
    };
    format!("{gate} gate {outcome}")
}

pub(super) fn push_completed_result(
    state: &mut LoopExecutionState,
    result: CapabilityResultMessage,
) {
    state.recovery_state = state.recovery_state.cleared_attempts();
    state.result_refs.push(result.result_ref);
}
