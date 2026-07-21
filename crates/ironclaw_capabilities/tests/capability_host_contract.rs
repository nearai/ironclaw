use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_denies_missing_grant_before_dispatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let authorizer = GrantAuthorizer::new();
    let host = capability_host(&registry, &dispatcher, &authorizer);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "blocked"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::MissingGrant,
            ..
        }
    ));
    assert!(dispatcher.call_count() == 0);
}

#[tokio::test]
async fn capability_host_denies_dispatch_when_trust_ceiling_omits_capability_effect() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let authorizer = GrantAuthorizer::new();
    // The kernel now computes trust in-fold (§5.3.2/§9); inject a trust policy
    // whose authority ceiling omits the capability's effect so the trust-aware
    // authorizer denies on the trust ceiling (previously a caller-stamped
    // empty-effects `trust_decision` drove this).
    let trust_policy = FixedTrustPolicy::with_effects(Vec::new());
    let host =
        capability_host_with_trust_policy(&registry, &dispatcher, &authorizer, &trust_policy);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "blocked by trust"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::PolicyDenied,
            ..
        }
    ));
    assert!(dispatcher.call_count() == 0);
}

#[tokio::test]
async fn capability_host_authorized_dispatch_uses_neutral_dispatch_port() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let authorizer = GrantAuthorizer::new();
    let host = capability_host(&registry, &dispatcher, &authorizer);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default().set_output_bytes(4096),
            input: json!({"message": "authorized"}),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    let recorded = dispatcher.last_request().unwrap();
    assert_eq!(recorded.invocation.capability, capability_id());
    assert_eq!(recorded.invocation.scope, scope);
    assert_eq!(recorded.invocation.input, json!({"message": "authorized"}));
    assert_eq!(recorded.mounts, None);
    assert_eq!(recorded.resource_reservation, None);
}

#[tokio::test]
async fn capability_host_returns_approval_store_missing_when_approval_cannot_be_persisted() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let host = capability_host(&registry, &dispatcher, &ApprovalAuthorizer);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs approval"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalStoreMissing { .. }
    ));
    assert!(dispatcher.call_count() == 0);
}

/// Credential pre-flight (§5.3.2/§9) must run inside `authorize()` BEFORE the
/// approval decision: a missing credential surfaces as
/// `AuthorizationRequiresAuth`, never the approval outcome.
///
/// The authorizer here (`ApprovalAuthorizer`) would `RequireApproval` — and with
/// no approval stores wired the approval path returns `ApprovalStoreMissing`
/// (see `capability_host_returns_approval_store_missing_...` above). Proving we
/// instead get `AuthorizationRequiresAuth` proves the credential check ordered
/// ahead of the approval decision, so a human approval is never consumed for an
/// action blocked on a missing credential. Regression for the credential
/// pre-flight relocation from host_runtime into the kernel.
#[tokio::test]
async fn capability_host_missing_credential_blocks_before_approval_decision() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let host = capability_host_with_policy_facts(
        &registry,
        &dispatcher,
        &ApprovalAuthorizer,
        &MissingCredentialPolicyFacts,
    );
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs credential"}),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::AuthorizationRequiresAuth { .. }
        ),
        "credential pre-flight must fire before the approval decision; got {err:?}"
    );
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_fails_closed_on_unsupported_obligations_before_dispatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let authorizer = ObligatingAuthorizer;
    let host = capability_host(&registry, &dispatcher, &authorizer);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "must not dispatch"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(dispatcher.call_count() == 0);
}
