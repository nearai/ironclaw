//! Foreign errors and verdicts mapped into this crate's vocabulary.
//!
//! Free functions only, and deliberately so: every one is pure over its input
//! (or, for the two lease/approval cleanups, does one compensating store call)
//! and holds no reference to [`CapabilityHost`]. Nothing here may make a policy
//! decision — it only renames one that was already made.

use ironclaw_authorization::CapabilityLeaseStorePort;
use ironclaw_host_api::{
    decision::{DenyReason, Obligation},
    dispatch::DispatchError,
    ids::{CapabilityGrantId, CapabilityId, InvocationId},
    resource::ResourceScope,
};
use ironclaw_runtime_policy::PlannerError;
use ironclaw_safety::shell_command_display_text;
use tracing::{debug, warn};

use crate::helpers::{CapabilityInvocationStateTransition, capability_lease_error_kind};
use crate::trust::TrustEvaluationError;
use crate::{
    CapabilityInvocationError, CapabilityObligationError, CapabilityObligationFailureKind,
};

/// Map a kernel trust-classification failure to the model-visible invocation
/// error, preserving today's outcome kinds: the "unknown capability" case →
/// `UnknownCapability` (host `MissingRuntime`); every other variant →
/// `AuthorizationDenied` (host `Authorization`).
pub(super) fn trust_error_to_invocation_error(
    capability_id: &CapabilityId,
    error: TrustEvaluationError,
) -> CapabilityInvocationError {
    debug!(
        capability_id = %capability_id,
        trust_error = error.message(),
        "kernel trust classification refused to produce a decision"
    );
    if error.is_unknown_capability() {
        CapabilityInvocationError::UnknownCapability {
            capability: capability_id.clone(),
        }
    } else {
        CapabilityInvocationError::AuthorizationDenied {
            capability: capability_id.clone(),
            reason: DenyReason::InternalInvariantViolation,
            detail: None,
        }
    }
}

/// Map an in-fold runtime-policy planner refusal to the model-visible
/// `AuthorizationDenied` (host `Authorization`), matching today's
/// `runtime_policy_failure`.
pub(super) fn runtime_policy_error_to_invocation_error(
    capability_id: &CapabilityId,
    error: PlannerError,
) -> CapabilityInvocationError {
    // The verdict collapses to `PolicyDenied`, but a bare `PolicyDenied` tells the
    // model nothing about *why*. So the model-visible `detail` carries a
    // plain-language explanation of the refusal — deliberately NOT the raw
    // `PlannerError` Display, which leaks internal `ProcessBackendKind::`/
    // `NetworkMode::`/`SecretMode::` enum tokens the model must never see (see
    // `planner_error_kind`). The full enum-token message stays server-side via
    // `debug!` (never `info!`/`warn!`) for operator diagnosis.
    debug!(
        capability_id = %capability_id,
        %error,
        "runtime-policy planner refused capability dispatch (fail-closed)"
    );
    CapabilityInvocationError::AuthorizationDenied {
        capability: capability_id.clone(),
        reason: DenyReason::PolicyDenied,
        detail: Some(planner_error_model_reason(&error).to_string()),
    }
}

/// Sanitized, model-visible explanation of a runtime-policy planner refusal:
/// a plain-language reason the model can surface or explain, deliberately free
/// of the internal `ProcessBackendKind::`/`NetworkMode::`/`SecretMode::` planner
/// enum tokens (see [`planner_error_kind`] and #6386). Rides the
/// `AuthorizationDenied { detail }` field.
fn planner_error_model_reason(error: &PlannerError) -> &'static str {
    match error {
        PlannerError::ProcessEffectsRequiredButProcessBackendIsNone { .. } => {
            "this capability needs to run a process, but process execution is disabled by policy for this runtime"
        }
        PlannerError::NetworkRequiredButNetworkModeIsDeny { .. } => {
            "this capability needs network access, but network egress is disabled by policy for this runtime"
        }
        PlannerError::SecretAccessRequiredButSecretModeIsDeny { .. } => {
            "this capability needs secret access, but secret access is disabled by policy for this runtime"
        }
    }
}

/// Internal (audit-only) `error_kind` for a runtime-policy planner refusal, kept
/// distinct from the sanitized model-visible `DenyReason::PolicyDenied` that
/// `runtime_policy_error_to_invocation_error` produces. Mirrors the strings
/// host_runtime's deleted `RuntimePolicyEvaluationError::kind` recorded on the
/// blocked-run failure so the process-invocation audit record is unchanged (e.g.
/// `"process_backend_none"`); the planner enum name never reaches the model.
pub(super) fn planner_error_kind(error: &PlannerError) -> &'static str {
    match error {
        PlannerError::ProcessEffectsRequiredButProcessBackendIsNone { .. } => {
            "process_backend_none"
        }
        PlannerError::NetworkRequiredButNetworkModeIsDeny { .. } => "network_denied",
        PlannerError::SecretAccessRequiredButSecretModeIsDeny { .. } => "secret_denied",
    }
}

pub(super) fn add_capability_input_display_hint(
    reason: &mut String,
    capability_id: &CapabilityId,
    input: &serde_json::Value,
) {
    let capability_id = capability_id.as_str();
    if capability_id != "shell"
        && capability_id != "builtin.shell"
        && !capability_id.ends_with(".shell")
    {
        return;
    }
    let Some(command) = input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(shell_command_display_text)
    else {
        return;
    };
    if command.text.is_empty() {
        return;
    }
    reason.push_str("\n\nCommand:\n");
    reason.push_str(&command.text);
    if command.truncated {
        reason.push_str("\n[truncated]");
    }
}

/// Cleans up a claimed lease after a resume-path error using best-effort
/// abort-or-revoke semantics.
///
/// - If `error` is a `BlockAuth` (non-terminal auth gate), aborts the
///   `Dispatching` lease back to `Claimed` so the next `auth_resume_json`
///   call can reuse it without a new human approval.
/// - Otherwise revokes the lease terminally.
///
/// Both operations are best-effort: failures are logged as warnings and do
/// not propagate — the caller should already be returning an error.
///
/// `revoke_context` names the failure site ("obligation failure" or
/// "dispatch failure") and is included in the revoke warn message.
pub(super) async fn cleanup_claimed_lease_after_resume_error(
    capability_leases: &dyn CapabilityLeaseStorePort,
    scope: &ResourceScope,
    claimed_grant_id: CapabilityGrantId,
    invocation_id: InvocationId,
    capability_id: &CapabilityId,
    error: &CapabilityInvocationError,
    revoke_context: &str,
) {
    if is_block_auth_transition(error) {
        if let Err(abort_error) = capability_leases
            .abort_dispatch_claimed(scope, claimed_grant_id)
            .await
        {
            warn!(
                lease_id = %claimed_grant_id,
                invocation_id = %invocation_id,
                capability_id = %capability_id,
                abort_error_kind = capability_lease_error_kind(&abort_error),
                "capability lease abort-dispatch failed after non-terminal auth bounce; lease may remain Dispatching",
            );
        }
    } else if let Err(revoke_error) = capability_leases.revoke(scope, claimed_grant_id).await {
        warn!(
            lease_id = %claimed_grant_id,
            invocation_id = %invocation_id,
            capability_id = %capability_id,
            revoke_error_kind = capability_lease_error_kind(&revoke_error),
            "capability lease revoke failed after {revoke_context}; lease may remain claimed",
        );
    }
}

/// Returns `true` when the error will transition the run to `BlockedAuth`
/// (a non-terminal, retriable auth gate).  Used to decide whether to skip
/// the post-claim lease revoke so `auth_resume_json` can reuse the same
/// Claimed lease without requiring a new human approval.
fn is_block_auth_transition(error: &CapabilityInvocationError) -> bool {
    matches!(
        error.invocation_state_transition(),
        Some(CapabilityInvocationStateTransition::BlockAuth { .. })
    )
}

pub(super) fn prepare_obligation_error_to_invocation(
    capability_id: &ironclaw_host_api::ids::CapabilityId,
    error: CapabilityObligationError,
) -> CapabilityInvocationError {
    match error {
        CapabilityObligationError::Unsupported { obligations } => {
            CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations,
            }
        }
        CapabilityObligationError::AuthRequired {
            credential_requirements,
        } => CapabilityInvocationError::AuthorizationRequiresAuth {
            capability: capability_id.clone(),
            required_secrets: Vec::new(),
            credential_requirements,
        },
        CapabilityObligationError::Failed { kind } => CapabilityInvocationError::ObligationFailed {
            capability: capability_id.clone(),
            kind,
        },
    }
}

pub(super) fn completion_obligation_error_to_invocation(
    capability_id: &ironclaw_host_api::ids::CapabilityId,
    error: CapabilityObligationError,
) -> CapabilityInvocationError {
    match error {
        CapabilityObligationError::AuthRequired { .. } => {
            CapabilityInvocationError::ObligationFailed {
                capability: capability_id.clone(),
                kind: CapabilityObligationFailureKind::Secret,
            }
        }
        other => prepare_obligation_error_to_invocation(capability_id, other),
    }
}

pub(super) fn obligation_invocation_error_kind(error: &CapabilityInvocationError) -> &'static str {
    // `invocation_state_transition` returns `None` for `CapabilityInvocationError::Dispatch`
    // because PR #4236 handles those failures via the disposition policy on the
    // outcome path. The obligation call sites only see this function for
    // diagnostic logging; fall back to a stable "Dispatch" label in that case.
    error
        .invocation_state_transition()
        .map(CapabilityInvocationStateTransition::error_kind)
        .unwrap_or("Dispatch")
}

/// Synthesize the auth-gate credential requirement for a runtime `AuthRequired`
/// that carries no auth detail of its own (the WASM-style 401 case), from the
/// capability's declared credential obligation.
///
/// Fires ONLY when the runtime gave no auth signal at all — both `required_secrets`
/// and `credential_requirements` empty — AND the capability declares EXACTLY ONE
/// credential obligation. A raw-secret-handle gate (`required_secrets` populated)
/// must not be turned into a product-auth provider prompt; and with multiple
/// credential obligations the failed credential cannot be attributed, so we leave
/// the gate unmodified rather than guess the wrong provider. The downstream WebUI
/// auth surface consumes exactly one provider (manual-token card for
/// `ManualToken` setup, OAuth launch for `OAuth` setup).
///
/// FOLLOW-UP (reactive OAuth refresh on runtime 401): for an `OAuth` credential
/// this gate is the *fallback* after refresh is exhausted — proactive refresh
/// may already have been attempted inline at injection (within the 5-min expiry
/// margin) or by the background keepalive worker. A runtime 401 still slips through when the token
/// looked fresh by `expires_at` but was revoked mid-life, where one reactive
/// "refresh + retry" before surfacing the gate would recover silently. That
/// retry does not exist today (pre-existing gap, not introduced here); the gate
/// remains correct for the genuinely-revoked case. Track as a resolver/egress
/// enhancement, not a change to this enrichment.
pub(super) fn enrich_dispatch_error_credential_requirements(
    error: DispatchError,
    obligations: &[Obligation],
) -> DispatchError {
    // Matched by value in one pass: the guard borrows the two vectors, so a
    // non-enriching outcome falls through to `other` with `error` un-moved.
    // Enriching rebuilds the variant from the parts it already owns, which is
    // what lets this be total — there is no "matched above" branch to assert.
    match error {
        DispatchError::AuthRequired {
            capability,
            required_secrets,
            credential_requirements,
        } if required_secrets.is_empty() && credential_requirements.is_empty() => {
            let derived: Vec<_> = obligations
                .iter()
                .filter_map(Obligation::credential_auth_requirement)
                .collect();
            match derived.as_slice() {
                [requirement] => DispatchError::AuthRequired {
                    capability,
                    required_secrets,
                    credential_requirements: vec![requirement.clone()],
                },
                // zero or >1 credential obligations: do not guess
                _ => DispatchError::AuthRequired {
                    capability,
                    required_secrets,
                    credential_requirements,
                },
            }
        }
        other => other,
    }
}
