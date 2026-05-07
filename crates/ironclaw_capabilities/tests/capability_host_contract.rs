use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_denies_missing_grant_before_dispatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = GrantAuthorizer::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "blocked"}),
            trust_decision: trust_decision(),
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
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_denies_dispatch_when_trust_ceiling_omits_capability_effect() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = GrantAuthorizer::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "blocked by trust"}),
            trust_decision: trust_decision_with_effects(Vec::new()),
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
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_authorized_dispatch_uses_neutral_dispatch_port() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = GrantAuthorizer::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate {
                output_bytes: Some(4096),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "authorized"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    let recorded = dispatcher.take_request();
    assert_eq!(recorded.capability_id, capability_id());
    assert_eq!(recorded.scope, scope);
    assert_eq!(recorded.input, json!({"message": "authorized"}));
    assert_eq!(recorded.mounts, None);
    assert_eq!(recorded.resource_reservation, None);
}

#[tokio::test]
async fn capability_host_returns_approval_store_missing_when_approval_cannot_be_persisted() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs approval"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalStoreMissing { .. }
    ));
    assert!(!dispatcher.has_request());
}

#[tokio::test]
async fn capability_host_fails_closed_on_unsupported_obligations_before_dispatch() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let authorizer = ObligatingAuthorizer;
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer);
    let context = execution_context(CapabilitySet::default());

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "must not dispatch"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert!(!dispatcher.has_request());
}
