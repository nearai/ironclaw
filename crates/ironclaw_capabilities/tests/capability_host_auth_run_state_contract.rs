use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_run_state::*;
use ironclaw_trust::TrustDecision;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_blocks_auth_when_obligation_requires_secret_recovery() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let handler = AuthRequiredObligationHandler;
    let host = CapabilityHost::new(&registry, &dispatcher, &ObligatingAuthorizer)
        .with_run_state(&run_state)
        .with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs auth"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresAuth { .. }
    ));
    assert!(!dispatcher.has_request());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::BlockedAuth);
    assert_eq!(run.error_kind.as_deref(), Some("AuthRequired"));
}

#[tokio::test]
async fn capability_host_blocks_auth_when_dispatch_returns_auth_required() {
    // P1 regression: dispatch-path DispatchError::AuthRequired must transition
    // the run to BlockedAuth, not Failed, so auth-resume can pick it up.
    let registry = registry_with_echo_capability();
    let dispatcher = AuthRequiredDispatcher;
    let run_state = InMemoryRunStateStore::new();
    let authorizer = PlainAllowAuthorizer;
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "dispatch auth required"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            CapabilityInvocationError::AuthorizationRequiresAuth { .. }
        ),
        "expected AuthorizationRequiresAuth, got {err:?}"
    );
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(
        run.status,
        RunStatus::BlockedAuth,
        "dispatch AuthRequired must set BlockedAuth, not Failed"
    );
    assert_eq!(run.error_kind.as_deref(), Some("AuthRequired"));
}

#[tokio::test]
async fn capability_host_fails_post_dispatch_auth_required_without_retryable_gate() {
    let registry = registry_with_echo_capability();
    let dispatcher = RecordingDispatcher::default();
    let run_state = InMemoryRunStateStore::new();
    let handler = PostDispatchAuthRequiredObligationHandler;
    let host = CapabilityHost::new(&registry, &dispatcher, &ObligatingAuthorizer)
        .with_run_state(&run_state)
        .with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "post dispatch auth"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ObligationFailed {
            kind: CapabilityObligationFailureKind::Secret,
            ..
        }
    ));
    assert!(dispatcher.has_request());
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("ObligationFailed"));
}

struct AuthRequiredObligationHandler;

#[async_trait]
impl CapabilityObligationHandler for AuthRequiredObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Err(CapabilityObligationError::AuthRequired)
    }

    async fn prepare(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<CapabilityObligationOutcome, CapabilityObligationError> {
        Err(CapabilityObligationError::AuthRequired)
    }
}

struct PostDispatchAuthRequiredObligationHandler;

#[async_trait]
impl CapabilityObligationHandler for PostDispatchAuthRequiredObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Ok(())
    }

    async fn prepare(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<CapabilityObligationOutcome, CapabilityObligationError> {
        Ok(CapabilityObligationOutcome::default())
    }

    async fn complete_dispatch(
        &self,
        _request: CapabilityObligationCompletionRequest<'_>,
    ) -> Result<CapabilityDispatchResult, CapabilityObligationError> {
        Err(CapabilityObligationError::AuthRequired)
    }
}

/// An authorizer that allows dispatch with no obligations, used to let dispatch
/// reach the dispatcher so `DispatchError::AuthRequired` can be tested.
struct PlainAllowAuthorizer;

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for PlainAllowAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::empty(),
        }
    }
}

struct AuthRequiredDispatcher;

#[async_trait]
impl CapabilityDispatcher for AuthRequiredDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        Err(DispatchError::AuthRequired {
            capability: request.capability_id,
            required_secrets: vec![],
        })
    }
}
