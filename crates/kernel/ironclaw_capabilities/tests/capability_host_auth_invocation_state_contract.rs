use async_trait::async_trait;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_host_api::{
    capability::{CapabilityDescriptor, CapabilitySet},
    decision::{Decision, Obligations},
    dispatch::{CapabilityDispatchResult, DispatchError},
    ids::SecretHandle,
    resource::ResourceEstimate,
    scope::ExecutionContext,
};
use ironclaw_processes::*;
use ironclaw_trust::TrustDecision;
use serde_json::json;

mod support;
use support::*;

#[tokio::test]
async fn capability_host_blocks_auth_when_obligation_requires_secret_recovery() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_processes::in_memory_backed_process_invocation_state_store();
    let handler = AuthRequiredObligationHandler;
    let host = capability_host(&registry, &dispatcher, &ObligatingAuthorizer)
        .with_invocation_state(&run_state)
        .with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "needs auth"}),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresAuth { .. }
    ));
    assert!(dispatcher.call_count() == 0);
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, ProcessInvocationStatus::BlockedAuth);
    assert_eq!(run.error_kind.as_deref(), Some("AuthRequired"));
}

#[tokio::test]
async fn capability_host_blocks_auth_when_dispatch_returns_auth_required() {
    // P1 regression: dispatch-path DispatchError::AuthRequired must transition
    // the run to BlockedAuth, not Failed, so auth-resume can pick it up.
    let registry = registry_with_echo_capability();
    let dispatcher = TestDispatcher::scripted(vec![Err(DispatchError::AuthRequired {
        capability: capability_id(),
        required_secrets: vec![SecretHandle::new("echo_token").unwrap()],
        credential_requirements: Vec::new(),
        model_visible_cause: None,
    })]);
    let run_state = ironclaw_processes::in_memory_backed_process_invocation_state_store();
    let authorizer = PlainAllowAuthorizer;
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_invocation_state(&run_state);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "dispatch auth required"}),
        )
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
        ProcessInvocationStatus::BlockedAuth,
        "dispatch AuthRequired must set BlockedAuth, not Failed"
    );
    assert_eq!(run.error_kind.as_deref(), Some("AuthRequired"));
}

#[tokio::test]
async fn capability_host_fails_post_dispatch_auth_required_without_retryable_gate() {
    let registry = registry_with_echo_capability();
    let dispatcher = recording_dispatcher();
    let run_state = ironclaw_processes::in_memory_backed_process_invocation_state_store();
    let handler = PostDispatchAuthRequiredObligationHandler;
    let host = capability_host(&registry, &dispatcher, &ObligatingAuthorizer)
        .with_invocation_state(&run_state)
        .with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(
            context,
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "post dispatch auth"}),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ObligationFailed {
            kind: CapabilityObligationFailureKind::Secret,
            ..
        }
    ));
    assert!(dispatcher.call_count() > 0);
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, ProcessInvocationStatus::Failed);
    assert_eq!(run.error_kind.as_deref(), Some("ObligationFailed"));
}

struct AuthRequiredObligationHandler;

#[async_trait]
impl CapabilityObligationHandler for AuthRequiredObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Err(CapabilityObligationError::AuthRequired {
            credential_requirements: Vec::new(),
        })
    }

    async fn prepare(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<CapabilityObligationOutcome, CapabilityObligationError> {
        Err(CapabilityObligationError::AuthRequired {
            credential_requirements: Vec::new(),
        })
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
        Err(CapabilityObligationError::AuthRequired {
            credential_requirements: Vec::new(),
        })
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
