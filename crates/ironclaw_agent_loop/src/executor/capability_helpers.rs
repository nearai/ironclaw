use std::collections::{HashMap, HashSet};

use ironclaw_host_api::{CapabilityId, DispatchInputIssueCode};
use ironclaw_turns::{
    LoopResultRef,
    run_profile::{
        AgentLoopDriverHost, AppendCapabilityResultRef, CapabilityApprovalResume,
        CapabilityAuthResume, CapabilityCallCandidate, CapabilityDescriptorView, CapabilityFailure,
        CapabilityFailureDetail, CapabilityFailureKind, CapabilityInputIssue,
        CapabilityInputRepair, CapabilityInvocation, CapabilityRecoveryHint,
        CapabilityResultMessage, CapabilitySurfaceVersion, ModelVisibleToolObservation,
        ObservationTrust, ProviderToolCall, ProviderToolCallReference,
        RegisterProviderToolCallRequest, SameCallRetryConstraint, ToolObservationDetail,
        ToolObservationStatus, ToolRecoveryObservation, VisibleCapabilitySurface,
    },
};

use crate::{
    state::{
        CapabilityCallSignature, LoopExecutionState, PendingApprovalResume, PendingAuthResume,
        PendingExternalToolResume,
    },
    strategies::{CapabilityCallSummary, CapabilityErrorSummary, CapabilityFilter, GateKind},
};

use super::{AgentLoopExecutorError, capability_host_error};

const MAX_MODEL_OBSERVATION_INPUT_ISSUES: usize = 3;
const MAX_MODEL_OBSERVATION_TEXT_BYTES: usize = 256;

pub(super) fn capability_invocation_from_candidate(
    call: CapabilityCallCandidate,
    approval_resume: Option<CapabilityApprovalResume>,
) -> CapabilityInvocation {
    CapabilityInvocation {
        activity_id: call.activity_id,
        surface_version: call.surface_version,
        capability_id: call.capability_id,
        input_ref: call.input_ref,
        approval_resume,
        auth_resume: None,
    }
}

/// Builds a `CapabilityInvocation` from an auth-resumed candidate.
///
/// When `pending_auth.resume_token` is set (i.e., the invocation previously
/// passed an approval gate), the returned invocation carries a
/// `CapabilityAuthResume` so the host can reuse the original invocation
/// identifier and claim any matching approval lease.
pub(super) fn capability_invocation_from_auth_resume_candidate(
    call: CapabilityCallCandidate,
    pending_auth: &PendingAuthResume,
) -> CapabilityInvocation {
    let auth_resume = pending_auth
        .resume_token
        .as_ref()
        .map(|token| CapabilityAuthResume {
            resume_token: token.clone(),
            prior_approval: pending_auth.prior_approval.as_ref().map(|pa| {
                ironclaw_turns::run_profile::AuthResumeApprovalIdentity {
                    approval_request_id: pa.approval_request_id,
                    correlation_id: pa.correlation_id,
                }
            }),
            replay: pending_auth.replay.clone(),
        });
    CapabilityInvocation {
        activity_id: call.activity_id,
        surface_version: call.surface_version,
        capability_id: call.capability_id,
        input_ref: call.input_ref,
        approval_resume: None,
        auth_resume,
    }
}

pub(super) fn pending_approval_resume_candidate(
    resume: &PendingApprovalResume,
    surface_version: CapabilitySurfaceVersion,
) -> CapabilityCallCandidate {
    CapabilityCallCandidate {
        activity_id: resume.activity_id_for_resume(),
        surface_version,
        capability_id: resume.capability_id.clone(),
        input_ref: resume.input_ref.clone(),
        effective_capability_ids: resume.effective_capability_ids.clone(),
        provider_replay: resume.provider_replay.clone(),
    }
}

pub(super) async fn pending_auth_resume_candidate(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    resume: &PendingAuthResume,
    surface_version: CapabilitySurfaceVersion,
) -> Result<CapabilityCallCandidate, AgentLoopExecutorError> {
    if let Some(replay) = resume.provider_replay.as_ref() {
        let candidate = host
            .register_provider_tool_call(RegisterProviderToolCallRequest::for_activity(
                ProviderToolCall {
                    provider_id: replay.provider_id.clone(),
                    provider_model_id: replay.provider_model_id.clone(),
                    turn_id: Some(replay.provider_turn_id.clone()),
                    id: replay.provider_call_id.clone(),
                    name: replay.provider_tool_name.clone(),
                    arguments: replay.arguments.clone(),
                    response_reasoning: replay.response_reasoning.clone(),
                    reasoning: replay.reasoning.clone(),
                    signature: replay.signature.clone(),
                },
                resume.activity_id_for_resume(),
            ))
            .await
            .map_err(capability_host_error)?;
        if candidate.activity_id != resume.activity_id_for_resume()
            || candidate.capability_id != resume.capability_id
            || candidate.effective_capability_ids != resume.effective_capability_ids
        {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "auth resume provider replay no longer matches blocked capability activity",
            });
        }
        return Ok(candidate);
    }
    Ok(pending_auth_resume_staged_input_candidate(
        resume,
        surface_version,
    ))
}

fn pending_auth_resume_staged_input_candidate(
    resume: &PendingAuthResume,
    surface_version: CapabilitySurfaceVersion,
) -> CapabilityCallCandidate {
    CapabilityCallCandidate {
        activity_id: resume.activity_id_for_resume(),
        surface_version,
        capability_id: resume.capability_id.clone(),
        input_ref: resume.input_ref.clone(),
        effective_capability_ids: resume.effective_capability_ids.clone(),
        provider_replay: resume.provider_replay.clone(),
    }
}

pub(super) struct CapabilitySurfaceIndex<'a> {
    version: &'a CapabilitySurfaceVersion,
    descriptors: HashMap<&'a CapabilityId, &'a CapabilityDescriptorView>,
    /// The wider set of capabilities the model may *invoke* this turn, distinct
    /// from the advertised `descriptors`. Under progressive tool disclosure the
    /// advertised set is narrowed for token economy, but bridge / forgiving-direct
    /// calls can still reach disclosed-but-unadvertised catalog tools. `None`
    /// means no narrowing is in effect (callable == advertised).
    callable: Option<HashSet<&'a CapabilityId>>,
}

impl<'a> CapabilitySurfaceIndex<'a> {
    pub(super) fn new(surface: &'a VisibleCapabilitySurface) -> Self {
        let descriptors = surface
            .descriptors
            .iter()
            .map(|descriptor| (&descriptor.capability_id, descriptor))
            .collect();
        let callable = surface
            .callable_capability_ids
            .as_ref()
            .map(|ids| ids.iter().collect());
        Self {
            version: &surface.version,
            descriptors,
            callable,
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
    if surface.descriptors.contains_key(&call.capability_id) {
        return true;
    }
    // Under progressive tool disclosure the advertised `descriptors` are a
    // narrowed view; a bridge / forgiving-direct call resolves to a
    // disclosed-but-unadvertised catalog tool that is authorized to run this turn.
    // Authorize it against the wider callable set (the same set the port-level
    // model-visible filter uses), or this executor gate would reject the very
    // calls disclosure is meant to allow. `None` = no narrowing, so only the
    // advertised descriptors are visible (pre-disclosure behavior, unchanged).
    surface
        .callable
        .as_ref()
        .is_some_and(|callable| callable.contains(&call.capability_id))
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
        model_observation: model_visible_capability_success_observation(call, result),
    })
    .await
    .map_err(capability_host_error)?;
    Ok(())
}

fn model_visible_capability_success_observation(
    call: &CapabilityCallCandidate,
    result: &CapabilityResultMessage,
) -> Option<ModelVisibleToolObservation> {
    call.provider_replay.as_ref()?;
    // "none" is the static sentinel for successful capability observations and
    // satisfies CapabilityFailureKind's validation invariants.
    let failure_kind = CapabilityFailureKind::unknown("none")
        .expect("static success observation failure kind must be valid"); // safety: static valid sentinel
    Some(ModelVisibleToolObservation {
        schema_version: ironclaw_turns::run_profile::MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        summary: result.safe_summary.clone(),
        detail: ToolObservationDetail::GenericFailure {
            failure_kind,
            detail: None,
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    })
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
            invalid_input_observation(bounded_input_issues(issues))
        }
        detail => {
            let diagnostic = match detail {
                Some(CapabilityFailureDetail::Diagnostic { text }) => {
                    Some(bounded_diagnostic_detail(text))
                }
                _ => None,
            };
            ModelVisibleToolObservation {
                schema_version:
                    ironclaw_turns::run_profile::MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
                status: ToolObservationStatus::Error,
                summary: model_visible_failure_summary(&failure.error_kind),
                detail: ToolObservationDetail::GenericFailure {
                    failure_kind: failure.error_kind.clone(),
                    detail: diagnostic,
                },
                artifacts: Vec::new(),
                recovery: Some(generic_failure_recovery(&failure.error_kind)),
                trust: ObservationTrust::UntrustedToolOutput,
            }
        }
    }
}

/// Bound a free-text diagnostic to the model-visible detail cap, truncating on
/// a UTF-8 boundary. The diagnostic is already secret-scrubbed by the producer.
fn bounded_diagnostic_detail(value: &str) -> String {
    const MAX: usize = ironclaw_turns::run_profile::MODEL_OBSERVATION_DETAIL_MAX_BYTES;
    if value.len() <= MAX {
        return value.to_string();
    }
    let mut end = MAX;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string() // safety: `end` reduced to a valid UTF-8 boundary.
}

fn model_visible_failure_summary(error_kind: &CapabilityFailureKind) -> String {
    match error_kind {
        CapabilityFailureKind::GateDeclined => "Capability declined by user.".to_string(),
        _ => format!("Capability failed with {}.", error_kind.as_str()),
    }
}

fn bounded_input_issues(issues: &[CapabilityInputIssue]) -> Vec<CapabilityInputIssue> {
    issues
        .iter()
        .take(MAX_MODEL_OBSERVATION_INPUT_ISSUES)
        .map(|issue| CapabilityInputIssue {
            path: truncate_model_observation_text(&issue.path),
            code: issue.code,
            expected: issue
                .expected
                .as_deref()
                .map(truncate_model_observation_text),
            received: issue
                .received
                .as_deref()
                .map(truncate_model_observation_text),
            schema_path: issue
                .schema_path
                .as_deref()
                .map(truncate_model_observation_text),
        })
        .collect()
}

fn truncate_model_observation_text(value: &str) -> String {
    if value.len() <= MAX_MODEL_OBSERVATION_TEXT_BYTES {
        return value.to_string();
    }
    let mut end = MAX_MODEL_OBSERVATION_TEXT_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string() // safety: `end` is reduced until it lands on a valid UTF-8 boundary.
}

fn invalid_input_observation(issues: Vec<CapabilityInputIssue>) -> ModelVisibleToolObservation {
    let repairs = issues.iter().map(input_issue_repair).collect();
    ModelVisibleToolObservation {
        schema_version: ironclaw_turns::run_profile::MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
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

fn generic_failure_recovery(error_kind: &CapabilityFailureKind) -> ToolRecoveryObservation {
    let same_call_retry = match error_kind {
        CapabilityFailureKind::Authorization
        | CapabilityFailureKind::GateDeclined
        | CapabilityFailureKind::PolicyDenied => SameCallRetryConstraint::Forbidden,
        CapabilityFailureKind::InvalidInput
        | CapabilityFailureKind::InvalidOutput
        | CapabilityFailureKind::OutputTooLarge => SameCallRetryConstraint::RequiresChangedInput,
        CapabilityFailureKind::Backend
        | CapabilityFailureKind::Network
        | CapabilityFailureKind::Transient
        | CapabilityFailureKind::Unavailable => SameCallRetryConstraint::AllowedAfterDelay,
        CapabilityFailureKind::Cancelled
        | CapabilityFailureKind::MissingRuntime
        | CapabilityFailureKind::Permanent => SameCallRetryConstraint::NotUseful,
        CapabilityFailureKind::Dispatcher
        | CapabilityFailureKind::OperationFailed
        | CapabilityFailureKind::Process
        | CapabilityFailureKind::Resource
        | CapabilityFailureKind::Internal
        | CapabilityFailureKind::Unknown(_) => SameCallRetryConstraint::Allowed,
    };
    ToolRecoveryObservation {
        same_call_retry,
        repairs: Vec::new(),
        recovery_hint: CapabilityRecoveryHint::RespectFailureConstraint,
    }
}

fn input_issue_repair(issue: &CapabilityInputIssue) -> CapabilityInputRepair {
    match issue.code {
        DispatchInputIssueCode::MissingRequired => CapabilityInputRepair::ProvideRequiredField {
            path: issue.path.clone(),
        },
        DispatchInputIssueCode::UnexpectedField => CapabilityInputRepair::RemoveUnexpectedField {
            path: issue.path.clone(),
        },
        DispatchInputIssueCode::TypeMismatch => CapabilityInputRepair::ChangeType {
            path: issue.path.clone(),
            expected: issue.expected.clone(),
        },
        DispatchInputIssueCode::InvalidValue => CapabilityInputRepair::UseAllowedValue {
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
        GateKind::ExternalTool => "external_tool",
    };
    format!("{gate} gate {outcome}")
}

pub(super) fn clear_matching_pending_auth_resume(
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
) {
    if state
        .pending_auth_resume
        .as_ref()
        .is_some_and(|resume| resume.capability_id == call.capability_id)
    {
        state.pending_auth_resume = None;
    }
}

pub(super) fn clear_matching_pending_external_tool_resume(
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
) {
    if state
        .pending_external_tool_resume
        .as_ref()
        .is_some_and(|resume| resume.capability_id == call.capability_id)
    {
        state.pending_external_tool_resume = None;
    }
}

/// Reconstruct the parked external-tool call on resume.
///
/// Re-registers the provider tool call (via the stored replay) so the host's
/// external-tool decorator re-binds `input_ref -> call_id` and re-stages the
/// model's arguments, then the re-dispatched invocation completes from the
/// run-scoped catalog's client-submitted output. Falls back to a staged-input
/// candidate when no provider replay was captured.
pub(super) async fn pending_external_tool_resume_candidate(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    resume: &PendingExternalToolResume,
    surface_version: CapabilitySurfaceVersion,
) -> Result<CapabilityCallCandidate, AgentLoopExecutorError> {
    if let Some(replay) = resume.provider_replay.as_ref() {
        let candidate = host
            .register_provider_tool_call(RegisterProviderToolCallRequest::for_activity(
                ProviderToolCall {
                    provider_id: replay.provider_id.clone(),
                    provider_model_id: replay.provider_model_id.clone(),
                    turn_id: Some(replay.provider_turn_id.clone()),
                    id: replay.provider_call_id.clone(),
                    name: replay.provider_tool_name.clone(),
                    arguments: replay.arguments.clone(),
                    response_reasoning: replay.response_reasoning.clone(),
                    reasoning: replay.reasoning.clone(),
                    signature: replay.signature.clone(),
                },
                resume.activity_id_for_resume(),
            ))
            .await
            .map_err(capability_host_error)?;
        if candidate.activity_id != resume.activity_id_for_resume()
            || candidate.capability_id != resume.capability_id
            || candidate.effective_capability_ids != resume.effective_capability_ids
        {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "external tool resume provider replay no longer matches blocked capability activity",
            });
        }
        return Ok(candidate);
    }
    Ok(CapabilityCallCandidate {
        activity_id: resume.activity_id_for_resume(),
        surface_version,
        capability_id: resume.capability_id.clone(),
        input_ref: resume.input_ref.clone(),
        effective_capability_ids: resume.effective_capability_ids.clone(),
        provider_replay: resume.provider_replay.clone(),
    })
}

pub(super) fn push_completed_result(
    state: &mut LoopExecutionState,
    capability_id: &CapabilityId,
    result: CapabilityResultMessage,
) {
    state.recovery_state = state.recovery_state.cleared_attempts();
    if let Some(n) = state
        .post_capability_state
        .pending_capability_bytes
        .get_mut(capability_id)
    {
        *n = n.saturating_add(result.byte_len);
    } else {
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(capability_id.clone(), result.byte_len);
    }
    state.result_refs.push(result.result_ref);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_run_context;
    use ironclaw_turns::run_profile::{CapabilityProgress, CapabilitySurfaceVersion};

    #[test]
    fn push_completed_result_accumulates_bytes_per_capability() {
        let ctx = test_run_context("push-completed-result-bytes");
        let mut state = LoopExecutionState::initial_for_run(&ctx);
        let cap_a = CapabilityId::new("test.cap_a").unwrap();
        let cap_b = CapabilityId::new("test.cap_b").unwrap();
        let result_a1 = CapabilityResultMessage {
            result_ref: ironclaw_turns::LoopResultRef::new("result:a1".to_string()).unwrap(),
            safe_summary: "a1".into(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: 1000,
            output_digest: None,
        };
        let result_a2 = CapabilityResultMessage {
            result_ref: ironclaw_turns::LoopResultRef::new("result:a2".to_string()).unwrap(),
            safe_summary: "a2".into(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: 500,
            output_digest: None,
        };
        let result_b = CapabilityResultMessage {
            result_ref: ironclaw_turns::LoopResultRef::new("result:b".to_string()).unwrap(),
            safe_summary: "b".into(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: 2000,
            output_digest: None,
        };
        push_completed_result(&mut state, &cap_a, result_a1);
        push_completed_result(&mut state, &cap_a, result_a2);
        push_completed_result(&mut state, &cap_b, result_b);
        assert_eq!(
            state
                .post_capability_state
                .pending_capability_bytes
                .get(&cap_a),
            Some(&1500)
        );
        assert_eq!(
            state
                .post_capability_state
                .pending_capability_bytes
                .get(&cap_b),
            Some(&2000)
        );
        assert_eq!(state.result_refs.len(), 3);
    }

    #[test]
    fn capability_is_visible_authorizes_disclosed_but_unadvertised_callable_tool() {
        use ironclaw_host_api::RuntimeKind;
        use ironclaw_turns::run_profile::{
            CapabilityDescriptorView, CapabilityInputRef, ConcurrencyHint, VisibleCapabilitySurface,
        };

        let version = CapabilitySurfaceVersion::new("surface:v1").unwrap();
        let advertised = CapabilityId::new("test.tool_search").unwrap();
        let deferred = CapabilityId::new("google-docs.get_document").unwrap();

        let descriptor = CapabilityDescriptorView {
            capability_id: advertised.clone(),
            provider: None,
            runtime: RuntimeKind::FirstParty,
            safe_name: "tool_search".to_string(),
            safe_description: "search".to_string(),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
            parameters_schema: serde_json::json!({"type": "object"}),
        };
        let candidate = |cap: &CapabilityId| CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: version.clone(),
            capability_id: cap.clone(),
            input_ref: CapabilityInputRef::new("input:x").unwrap(),
            effective_capability_ids: vec![cap.clone()],
            provider_replay: None,
        };

        // Disclosure narrows `descriptors` to the advertised bridge, but the
        // deferred catalog tool is in the wider callable set → it MUST be visible
        // (this is the bug that made bridge calls loop on "not visible in the
        // filtered surface").
        let narrowed = VisibleCapabilitySurface {
            version: version.clone(),
            descriptors: vec![descriptor.clone()],
            callable_capability_ids: Some(vec![advertised.clone(), deferred.clone()]),
        };
        let index = CapabilitySurfaceIndex::new(&narrowed);
        assert!(
            capability_is_visible(&index, &candidate(&deferred)),
            "a disclosed-but-unadvertised tool in the callable set must be visible"
        );
        assert!(
            capability_is_visible(&index, &candidate(&advertised)),
            "the advertised tool stays visible"
        );

        // No narrowing (callable None): only advertised descriptors are visible —
        // pre-disclosure behavior, unchanged.
        let unnarrowed = VisibleCapabilitySurface {
            version: version.clone(),
            descriptors: vec![descriptor],
            callable_capability_ids: None,
        };
        let index = CapabilitySurfaceIndex::new(&unnarrowed);
        assert!(
            !capability_is_visible(&index, &candidate(&deferred)),
            "without a callable set, an unadvertised tool is not visible"
        );
        assert!(capability_is_visible(&index, &candidate(&advertised)));
    }

    #[test]
    fn generic_failure_observation_carries_diagnostic_detail_to_model() {
        let path = "missing input_schema_ref at /system/extensions/google-calendar/schemas/google-calendar/list_calendars.input.v1.json";
        let failure = CapabilityFailure {
            error_kind: CapabilityFailureKind::MissingRuntime,
            safe_summary: "capability invocation failed".to_string(),
            detail: Some(CapabilityFailureDetail::Diagnostic {
                text: path.to_string(),
            }),
        };

        let observation = model_visible_capability_failure_observation(&failure);
        observation
            .validate()
            .expect("observation with path-bearing diagnostic must validate");

        let ToolObservationDetail::GenericFailure { detail, .. } = &observation.detail else {
            panic!("expected a generic failure detail");
        };
        assert_eq!(
            detail.as_deref(),
            Some(path),
            "the raw path-bearing cause must reach the model-visible observation"
        );
    }

    #[test]
    fn generic_failure_observation_without_diagnostic_has_no_detail() {
        let failure = CapabilityFailure {
            error_kind: CapabilityFailureKind::Backend,
            safe_summary: "capability invocation failed".to_string(),
            detail: None,
        };
        let observation = model_visible_capability_failure_observation(&failure);
        let ToolObservationDetail::GenericFailure { detail, .. } = &observation.detail else {
            panic!("expected a generic failure detail");
        };
        assert_eq!(detail.as_deref(), None);
    }

    #[test]
    fn pending_auth_resume_candidate_carries_non_empty_effective_capability_ids() {
        use crate::state::PendingAuthResume;
        use ironclaw_turns::run_profile::{CapabilityInputRef, CapabilitySurfaceVersion};

        let cap_a = CapabilityId::new("test.cap_a").unwrap();
        let cap_b = CapabilityId::new("test.cap_b").unwrap();
        let resume = PendingAuthResume {
            gate_ref: ironclaw_turns::LoopGateRef::new("gate:auth-test").unwrap(),
            capability_id: cap_a.clone(),
            surface_version: CapabilitySurfaceVersion::new("surface:v1").unwrap(),
            input_ref: CapabilityInputRef::new("input:test").unwrap(),
            effective_capability_ids: vec![cap_a.clone(), cap_b.clone()],
            provider_replay: None,
            resume_token: None,
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            prior_approval: None,
            replay: None,
            disposition: None,
        };
        let surface_version = CapabilitySurfaceVersion::new("surface:v1").unwrap();

        let candidate = pending_auth_resume_staged_input_candidate(&resume, surface_version);

        assert_eq!(
            candidate.effective_capability_ids,
            vec![cap_a, cap_b],
            "pending_auth_resume_candidate must propagate all effective_capability_ids"
        );
    }

    /// When both `resume_token` and `prior_approval` are set (the invocation
    /// previously passed both an approval gate and is now being resumed after an
    /// auth gate), the resulting `CapabilityInvocation` must:
    /// - carry `auth_resume: Some(...)` with the resume token,
    /// - carry `auth_resume.prior_approval: Some(...)` with both the
    ///   `approval_request_id` and `correlation_id` from `prior_approval`,
    /// - carry `approval_resume: None` (the approval-resume path is separate).
    #[test]
    fn capability_invocation_from_auth_resume_candidate_with_both_token_and_prior_approval() {
        use ironclaw_host_api::{ApprovalRequestId, CorrelationId};
        use ironclaw_turns::run_profile::{
            AuthResumeApprovalIdentity, CapabilityInputRef, CapabilityResumeToken,
        };

        let cap = CapabilityId::new("test.cap").unwrap();
        let resume_token =
            CapabilityResumeToken::new("00000000-0000-0000-0000-000000000002").unwrap();
        let approval_request_id = ApprovalRequestId::new();
        let correlation_id = CorrelationId::new();

        let resume = PendingAuthResume {
            gate_ref: ironclaw_turns::LoopGateRef::new("gate:auth-both-fields").unwrap(),
            capability_id: cap.clone(),
            surface_version: CapabilitySurfaceVersion::new("surface:v1").unwrap(),
            input_ref: CapabilityInputRef::new("input:both-fields").unwrap(),
            effective_capability_ids: vec![cap.clone()],
            provider_replay: None,
            resume_token: Some(resume_token.clone()),
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            prior_approval: Some(AuthResumeApprovalIdentity {
                approval_request_id,
                correlation_id,
            }),
            replay: None,
            disposition: None,
        };
        let surface_version = CapabilitySurfaceVersion::new("surface:v1").unwrap();
        let call = CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version,
            capability_id: cap.clone(),
            input_ref: CapabilityInputRef::new("input:both-fields").unwrap(),
            effective_capability_ids: vec![cap],
            provider_replay: None,
        };

        let invocation = capability_invocation_from_auth_resume_candidate(call, &resume);

        // The auth_resume field must be present with the correct resume token.
        let auth_resume = invocation
            .auth_resume
            .as_ref()
            .expect("auth_resume must be Some when resume_token is set");
        assert_eq!(
            auth_resume.resume_token, resume_token,
            "auth_resume.resume_token must match the pending resume token"
        );

        // The prior_approval correlation must be carried through.
        let prior_approval = auth_resume
            .prior_approval
            .as_ref()
            .expect("auth_resume.prior_approval must be Some when prior_approval is set");
        assert_eq!(
            prior_approval.approval_request_id, approval_request_id,
            "prior_approval.approval_request_id must be propagated from PendingAuthResume"
        );
        assert_eq!(
            prior_approval.correlation_id, correlation_id,
            "prior_approval.correlation_id must be propagated from PendingAuthResume"
        );

        // The approval_resume field must be None on the auth-resume path.
        assert!(
            invocation.approval_resume.is_none(),
            "approval_resume must be None for auth-resume path invocations; got {:?}",
            invocation.approval_resume
        );
    }

    #[test]
    fn capability_invocation_from_auth_resume_candidate_with_none_resume_token_sets_auth_resume_none()
     {
        // When `PendingAuthResume.resume_token` is None (the invocation never
        // passed an approval gate), the returned `CapabilityInvocation` must
        // carry `auth_resume: None` so the host routes through `invoke_json`,
        // while preserving the call activity id as the invocation identity.
        use ironclaw_turns::run_profile::CapabilityInputRef;

        let cap = CapabilityId::new("test.cap").unwrap();
        let resume = PendingAuthResume {
            gate_ref: ironclaw_turns::LoopGateRef::new("gate:auth-none-token").unwrap(),
            capability_id: cap.clone(),
            surface_version: CapabilitySurfaceVersion::new("surface:v1").unwrap(),
            input_ref: CapabilityInputRef::new("input:none-token").unwrap(),
            effective_capability_ids: vec![cap.clone()],
            provider_replay: None,
            resume_token: None, // no prior approval — the key precondition
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            prior_approval: None,
            replay: None,
            disposition: None,
        };
        let surface_version = CapabilitySurfaceVersion::new("surface:v1").unwrap();
        let activity_id = ironclaw_turns::CapabilityActivityId::new();
        let call = CapabilityCallCandidate {
            activity_id,
            surface_version,
            capability_id: cap.clone(),
            input_ref: CapabilityInputRef::new("input:none-token").unwrap(),
            effective_capability_ids: vec![cap],
            provider_replay: None,
        };

        let invocation = capability_invocation_from_auth_resume_candidate(call, &resume);

        assert_eq!(
            invocation.activity_id, activity_id,
            "tokenless auth-resume candidates must preserve the parked activity id"
        );
        assert!(
            invocation.auth_resume.is_none(),
            "auth_resume must be None when resume_token is None; got {:?}",
            invocation.auth_resume
        );
        assert!(
            invocation.approval_resume.is_none(),
            "approval_resume must be None for auth-resume path invocations"
        );
    }
}
