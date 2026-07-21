//! Persistent-approval fold regression (§5.2.7/§5.3.2).
//!
//! The persistent-approval decision moved out of host_runtime's former
//! `apply_persistent_approval_policy` into the capability kernel's `authorize()`
//! fold: the kernel reads candidate grants from
//! [`HostPolicyFacts::persistent_grants`], re-authorizes with each injected into
//! a candidate context, and adopts the first grant that flips the decision to
//! `Allow` — so a prior scoped ("always allow") approval authorizes the
//! invocation WITHOUT raising a fresh approval gate. These tests drive
//! `invoke_json` through the production caller and assert the observable
//! dispatch-vs-gate outcome.

use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_trust::TrustDecision;
use serde_json::json;

mod support;
use support::*;

/// Authorizer that mirrors a manifest-`Ask` capability with an "always allow"
/// persistent policy: it `Allow`s only when the context already carries a grant
/// bearing `DispatchCapability`, and otherwise `RequireApproval`. This is exactly
/// the shape the persistent-approval fold must resolve — the bare context gates,
/// the grant-injected candidate allows.
struct GrantAwareApprovalAuthorizer;

fn context_has_dispatch_grant(context: &ExecutionContext) -> bool {
    context.grants.grants.iter().any(|grant| {
        grant
            .constraints
            .allowed_effects
            .contains(&EffectKind::DispatchCapability)
    })
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for GrantAwareApprovalAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        if context_has_dispatch_grant(context) {
            return Decision::Allow {
                obligations: Obligations::empty(),
            };
        }
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: descriptor.id.clone(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        self.authorize_dispatch_with_trust(context, descriptor, estimate, _trust_decision)
            .await
    }
}

/// Positive: a matching persistent grant flips the authorizer to `Allow`, so the
/// kernel adopts it and DISPATCHES — no approval gate is raised. Proves the fold
/// relocated into `authorize()` is load-bearing.
#[tokio::test]
async fn persistent_grant_flips_to_allow_and_dispatches_without_gate() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = GrantAwareApprovalAuthorizer;
    // The candidate grant the port surfaces carries `DispatchCapability`, so the
    // re-authorize probe flips to `Allow`.
    let policy_facts = PersistentGrantPolicyFacts::new(dispatch_grant());
    let host =
        capability_host_with_policy_facts(&registry, &dispatcher, &authorizer, &policy_facts);
    // Bare context: no grant, so the authorizer would gate without the fold.
    let context = execution_context(CapabilitySet::default());

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "persistent allow"}),
        })
        .await
        .expect("persistent grant must authorize dispatch without an approval gate");

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    assert!(
        dispatcher.has_request(),
        "kernel must dispatch once the persistent grant is adopted"
    );
}

/// Negative: the surfaced persistent grant does NOT flip the decision (it lacks
/// `DispatchCapability`), so no grant is adopted and the approval gate IS raised.
/// Proves the fold adopts only a grant that genuinely authorizes.
#[tokio::test]
async fn non_flipping_persistent_grant_still_raises_approval_gate() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = GrantAwareApprovalAuthorizer;
    // Grant carries no `DispatchCapability`, so the re-authorize probe stays
    // `RequireApproval` and the candidate is never adopted.
    let policy_facts = PersistentGrantPolicyFacts::new(capability_grant_with_effects(Vec::new()));
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();
    let approval_requests = ironclaw_run_state::in_memory_backed_approval_request_store();
    let host =
        capability_host_with_policy_facts(&registry, &dispatcher, &authorizer, &policy_facts)
            .with_run_state(&run_state)
            .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "still needs approval"}),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::AuthorizationRequiresApproval { .. }
        ),
        "a non-flipping persistent grant must not be adopted; the approval gate stays, got {err:?}"
    );
    assert!(!dispatcher.has_request());
}
