//! Host-mediated capability invocation — the kernel's authorization membrane.
//!
//! Every privileged effect in the Reborn stack crosses [`CapabilityHost`], so
//! this module is split one file per **workflow** rather than by mechanism
//! (PROPOSAL §6.5.6, CHECKLIST WS3 — "split along its six workflows, module
//! charter only"). The submodules are private and every workflow stays an
//! inherent method on [`CapabilityHost`], so `host::CapabilityHost` remains the
//! single path for callers and nothing outside this module sees the split.
//!
//! | Module | Owns | Never contains |
//! |---|---|---|
//! | `invoke` | Workflow 1 — `invoke_json`, the fresh inline invocation | The authorization decision itself |
//! | `authorize` | The shared fold: trust, runtime policy, persistent approval, and the [`Authorized`] seal | Anything workflow-shaped |
//! | `approval_resume` | Workflow 2 — `resume_json` | The dispatch tail (that is `resume_support`) |
//! | `auth_resume` | Workflows 3 and 4 — `auth_resume_json` and `decline_auth_json` | The dispatch tail |
//! | `spawn_resume` | Workflow 5 — `resume_spawn_json` | Inline dispatch |
//! | `spawn` | Workflow 6 — `spawn_json` and its private `authorize_spawn` fold | Inline dispatch |
//! | `resume_support` | The preflight / authorize / dispatch tail all three resume workflows converge on | A workflow preamble |
//! | `obligation_seams` | The prepare / complete / abort calls made around dispatch | Obligation *implementation* |
//! | `error_mapping` | Foreign errors and verdicts renamed into this crate's vocabulary | Any policy decision |
//!
//! Three rules keep the charter honest:
//!
//! - **A workflow module owns its preamble, never the tail.** The moment two
//!   resume workflows agree on a step, that step belongs to `resume_support`.
//! - **`authorize` decides; a workflow only maps the verdict.** A workflow that
//!   grows a policy branch of its own has taken authority the membrane is
//!   meant to hold in one place.
//! - **This file holds state and vocabulary, not behavior.** The struct, the
//!   [`CapabilityAuthorizer`] seal, the cross-workflow request/fold types and
//!   the constructors live here because every submodule needs them; a type used
//!   by exactly one workflow belongs in that workflow's module.

use chrono::Utc;
use ironclaw_approvals::ApprovalRequestStorePort;
use ironclaw_authorization::{
    CapabilityLease, CapabilityLeaseStorePort, TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_extension_registry::ExtensionRegistry;
use ironclaw_host_api::{
    Timestamp,
    approval::InvocationFingerprint,
    authorized::{
        AuthorizeResult, Authorized, CapabilityAuthorizer, ProcessAuthorizedContinuation,
    },
    capability::CapabilityDescriptor,
    decision::{DenyReason, Obligation},
    dispatch::{CapabilityDispatcher, DispatchError},
    ids::{ApprovalRequestId, CapabilityGrantId, CapabilityId, InvocationId, ProcessId},
    resource::{ResourceEstimate, ResourceScope},
    runtime::RuntimeKind,
    runtime_policy::EffectiveRuntimePolicy,
    scope::ExecutionContext,
};
use ironclaw_processes::{ProcessInvocationStatePort, ProcessManager};
use ironclaw_trust::TrustPolicy;

use crate::ports::HostPolicyFacts;
use crate::{CapabilityInvocationError, CapabilityObligationHandler, CapabilityObligationOutcome};

mod approval_resume;
mod auth_resume;
mod authorize;
mod error_mapping;
mod invoke;
mod obligation_seams;
mod resume_support;
mod spawn;
mod spawn_resume;

#[cfg(test)]
mod tests;

pub struct CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    registry: &'a ExtensionRegistry,
    dispatcher: &'a D,
    authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
    /// Provider-trust classifier the kernel evaluates in-fold (§5.3.2/§9), so
    /// trust is computed here rather than received as a caller-stamped field.
    trust_policy: &'a dyn TrustPolicy,
    /// Resolved runtime policy the in-fold planner (`plan_capability`) enforces
    /// before dispatch — the relocation of host_runtime's `enforce_runtime_policy`.
    runtime_policy: &'a EffectiveRuntimePolicy,
    /// Host-mediated policy *facts* the `authorize()` fold reads (§5.3.2/§9).
    /// Supplies credential-presence facts so a missing credential surfaces as
    /// `AuthorizationRequiresAuth` *before* the approval decision — the
    /// relocation of host_runtime's `credential_preflight_check`. Facts only:
    /// the kernel maps them to the verdict; the port never decides.
    policy_facts: &'a dyn HostPolicyFacts,
    invocation_state: Option<&'a dyn ProcessInvocationStatePort>,
    approval_requests: Option<&'a dyn ApprovalRequestStorePort>,
    capability_leases: Option<&'a dyn CapabilityLeaseStorePort>,
    process_manager: Option<&'a dyn ProcessManager>,
    obligation_handler: Option<&'a dyn CapabilityObligationHandler>,
}

// `CapabilityHost` IS the kernel authorizer (Slice-C wiring, arch-simplification
// §3/§5.3.2). Implementing `CapabilityAuthorizer` here — and NOWHERE else, per
// the `reborn_authorized_seal_ratchet` — is the "test-seal" half of the
// `Authorized` witness: only this crate can mint an `AuthorizationGrant`, so only
// the code that runs the authorize fold can seal an `Authorized`. The
// `authorize()` method that consumes the grant lands in a following wiring slice;
// this activates the seal so that ratchet becomes load-bearing.
impl<'a, D> CapabilityAuthorizer for CapabilityHost<'a, D> where D: CapabilityDispatcher + ?Sized {}

/// Specification for a lease that must be claimed AFTER authorization succeeds.
///
/// Used by `resume_json` where the approval lease is claimed only after
/// `authorize_dispatch_with_trust` returns `Allow` — keeping the lease `Active`
/// if authorization is denied.
struct PendingClaimAfterAuth<'r> {
    leases: &'r dyn CapabilityLeaseStorePort,
    grant_id: CapabilityGrantId,
    fingerprint: InvocationFingerprint,
    /// The approval lease's frozen expiry, carried from the full grant so the
    /// sealed witness never outlives the approval that authorized it. `None`
    /// when the grant declares no `expires_at`. Threaded through even though the
    /// claim is deferred past authorization: the seal is minted before the
    /// claim, so the expiry must travel on the pending-claim spec rather than
    /// being read back from a not-yet-claimed lease.
    grant_expiry: Option<Timestamp>,
}

/// Which blocked run a resume-path preflight failure may fail (§5.3.2/§9, R-A).
/// Mirrors host_runtime's two deleted matchers: the approval-resume /
/// spawn-resume paths key on a `BlockedApproval` record and compare the
/// `approval_request_id`; the auth-resume path keys on a `BlockedAuth` record and
/// does NOT compare `approval_request_id` (its `block_auth` transition clears the
/// persisted id to `None`).
#[derive(Debug, Clone, Copy)]
enum BlockedResumeKind {
    Approval {
        approval_request_id: ApprovalRequestId,
    },
    Auth,
}

/// Encodes the three mutually-exclusive approval-lease states that
/// `dispatch_resumed_capability` must handle.
enum ResumedLeaseState<'r> {
    /// A one-shot `Active` lease to claim *after* `authorize_dispatch_with_trust`
    /// returns `Allow`.  Used by `resume_json` so that a `Deny` leaves the
    /// lease `Active` (the claim is deferred past the authorize call).
    PendingClaim(PendingClaimAfterAuth<'r>),
    /// A lease already transitioned to `Claimed` by a prior `resume_json` auth
    /// bounce.  Used by `auth_resume_json` when the invocation previously passed
    /// an approval gate; reuses the existing `Claimed` lease without a second
    /// approval prompt.
    AlreadyClaimed(&'r dyn CapabilityLeaseStorePort, Box<CapabilityLease>),
    /// No prior approval lease is in play.  Used by `auth_resume_json` when
    /// `approval_request_id` is `None` (the invocation never passed an approval
    /// gate before hitting the auth gate).
    NoPriorLease,
}

/// Parameters for the converging dispatch tail shared between `resume_json`
/// and `auth_resume_json`.  All fields are resolved by the respective
/// method preamble before the shared tail begins.
struct ResumedDispatchParams<'r> {
    invocation_state: &'r dyn ProcessInvocationStatePort,
    scope: ResourceScope,
    invocation_id: InvocationId,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
    authorized_context: ExecutionContext,
    descriptor: &'r CapabilityDescriptor,
    /// Approval-lease state for this resume.  See [`ResumedLeaseState`].
    lease_state: ResumedLeaseState<'r>,
}

struct InvocationInput {
    context: ExecutionContext,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
}

struct ApprovalResumeInput {
    context: ExecutionContext,
    approval_request_id: ApprovalRequestId,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
}

struct AuthResumeInput {
    context: ExecutionContext,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
    approval_request_id: Option<ApprovalRequestId>,
}

/// Outcome of the extracted `authorize()` fold (arch-simplification §5.3.2,
/// §9 step 2): the sealed [`AuthorizeResult`] trichotomy (§3) *plus* the
/// behavior-preserving side-band `invoke_json` still needs to reproduce today's
/// exact dispatch and error mapping while the capability path is mid-migration.
///
/// Why this wraps `AuthorizeResult` rather than being one:
/// - `Denied` — `AuthorizeResult::Denied(DenyRef)` collapses the policy
///   [`DenyReason`] to an opaque correlation UUID; today's caller returns
///   `AuthorizationDenied { reason }`, so the reason rides here until denial
///   folds into `Resolution` (a later slice).
/// - `Authorized` — today's `invoke_json` still owns `dispatch_json` and the
///   post-dispatch obligation lifecycle, so it needs the raw `obligations` and
///   the prepared `obligation_outcome`. Those `Option`-shaped mounts/reservation
///   are the *exact* values dispatch receives; the sealed witness's provisional,
///   forward-looking `mounts`/`reservation` deliberately do NOT drive today's
///   dispatch (§5.3.2/§5.3.3 — the dispatcher still reserves against the
///   governor when `resource_reservation` is `None`).
enum AuthorizeFold {
    /// Authorization allowed dispatch. Boxed because its payload (obligations +
    /// prepared outcome + the boxed witness) dwarfs the ref-sized deny/block
    /// variants (`clippy::large_enum_variant`).
    Authorized(Box<AuthorizedFold>),
    /// Terminal policy denial (`AuthorizeResult::Denied`). `reason` is the
    /// model-visible policy verdict the caller resurfaces as
    /// `AuthorizationDenied { reason }`.
    Denied {
        result: AuthorizeResult,
        reason: DenyReason,
    },
    /// A re-entrant approval gate (`AuthorizeResult::Blocked(Blocked::Approval)`).
    /// The pending approval was persisted and the run transitioned to
    /// `BlockedApproval` inside `authorize`; the caller returns
    /// `AuthorizationRequiresApproval`.
    Blocked { result: AuthorizeResult },
}

/// Payload of [`AuthorizeFold::Authorized`] — the allowed-dispatch side-band.
///
/// `result` is `Some(AuthorizeResult::Authorized(..))` for every allowed,
/// dispatchable invocation: actor-less contexts seal as [`Actor::System`] and
/// origin is the real ingress fact. `result` is `None` only when the descriptor
/// resolves to no untrusted [`RuntimeLane`] (a host-internal `System` runtime) or
/// when a context carries no resolvable ingress origin. Inline dispatch requires
/// a witness; process spawn allows `System` runtime continuations to remain
/// witness-less because those execute through the process host path, not an
/// untrusted runtime lane.
struct AuthorizedFold {
    result: Option<AuthorizeResult>,
    frozen_deadline: Option<Timestamp>,
    obligations: Vec<Obligation>,
    obligation_outcome: CapabilityObligationOutcome,
}

fn authorized_dispatch_witness(
    result: Option<AuthorizeResult>,
    capability_id: &CapabilityId,
) -> Result<Box<Authorized>, CapabilityInvocationError> {
    match result {
        Some(AuthorizeResult::Authorized(authorized)) => Ok(authorized),
        _ => Err(CapabilityInvocationError::from(
            DispatchError::MissingAuthorization {
                capability: capability_id.clone(),
            },
        )),
    }
}

fn process_authorized_continuation(
    result: Option<AuthorizeResult>,
    capability_id: &CapabilityId,
    runtime: RuntimeKind,
    process_id: ProcessId,
) -> Result<Option<ProcessAuthorizedContinuation>, CapabilityInvocationError> {
    match result {
        Some(AuthorizeResult::Authorized(authorized)) => {
            ProcessAuthorizedContinuation::from_authorized(*authorized, Utc::now(), process_id)
                .map(Some)
                .map_err(|authorized| {
                    let reservation = authorized.abort();
                    if reservation.is_some() {
                        tracing::warn!(
                            process_id = %process_id,
                            capability_id = %capability_id,
                            "spawn authorization witness expired before process start; reservation returned to obligation abort path"
                        );
                    }
                    CapabilityInvocationError::from(DispatchError::AuthorizationExpired {
                        capability: capability_id.clone(),
                    })
                })
        }
        None if runtime == RuntimeKind::System => Ok(None),
        _ => Err(CapabilityInvocationError::from(
            DispatchError::MissingAuthorization {
                capability: capability_id.clone(),
            },
        )),
    }
}

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    pub fn new(
        registry: &'a ExtensionRegistry,
        dispatcher: &'a D,
        authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
        trust_policy: &'a dyn TrustPolicy,
        runtime_policy: &'a EffectiveRuntimePolicy,
        policy_facts: &'a dyn HostPolicyFacts,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
            trust_policy,
            runtime_policy,
            policy_facts,
            invocation_state: None,
            approval_requests: None,
            capability_leases: None,
            process_manager: None,
            obligation_handler: None,
        }
    }

    /// Attaches the process-invocation store used to record invocation lifecycle.
    ///
    /// Required for `resume_json`. Strongly recommended for `invoke_json` and
    /// `spawn_json` so denials, obligation rejections, and dispatch failures
    /// transition the invocation record to `Failed` instead of being silently
    /// dropped. Without it, error paths still return the right user-facing
    /// error but no invocation record is persisted.
    pub fn with_invocation_state(
        mut self,
        invocation_state: &'a dyn ProcessInvocationStatePort,
    ) -> Self {
        self.invocation_state = Some(invocation_state);
        self
    }

    /// Attaches the approval-request store used to persist approval prompts.
    ///
    /// Required for `invoke_json` paths whose authorizer returns
    /// `Decision::RequireApproval` and for `resume_json`. Without it, an
    /// approval-required dispatch fails with `ApprovalStoreMissing` rather
    /// than blocking for human review.
    pub fn with_approval_requests(
        mut self,
        approval_requests: &'a dyn ApprovalRequestStorePort,
    ) -> Self {
        self.approval_requests = Some(approval_requests);
        self
    }

    /// Attaches the capability-lease store used to consume approved leases.
    ///
    /// Required for `resume_json`; not consulted by `invoke_json` or
    /// `spawn_json`.
    pub fn with_capability_leases(
        mut self,
        capability_leases: &'a dyn CapabilityLeaseStorePort,
    ) -> Self {
        self.capability_leases = Some(capability_leases);
        self
    }

    /// Attaches the process manager used to spawn long-running invocations.
    ///
    /// Required for `spawn_json`; not consulted by `invoke_json` or
    /// `resume_json`. Without it, `spawn_json` fails with
    /// `ProcessManagerMissing`.
    pub fn with_process_manager(mut self, process_manager: &'a dyn ProcessManager) -> Self {
        self.process_manager = Some(process_manager);
        self
    }

    /// Attaches the obligation handler that satisfies allow-decision
    /// obligations before/after side effects. Without a handler, non-empty
    /// obligations fail closed.
    pub fn with_obligation_handler(mut self, handler: &'a dyn CapabilityObligationHandler) -> Self {
        self.obligation_handler = Some(handler);
        self
    }
}
