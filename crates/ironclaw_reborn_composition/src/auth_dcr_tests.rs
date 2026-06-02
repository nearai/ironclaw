use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthContinuationEvent, AuthProductError, CredentialAccountLabel, InMemoryAuthProductServices,
};
use ironclaw_capabilities::{CapabilityObligationHandler, CapabilityObligationRequest};
use ironclaw_host_api::{
    AgentId, ExtensionId, ProjectId, RuntimeCredentialAccountProviderId,
    RuntimeCredentialAuthRequirement, RuntimeHttpEgress, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse, TenantId, ThreadId, UserId,
};
use ironclaw_secrets::InMemorySecretStore;
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::auth::{RebornAuthContinuationDispatcher, RebornProductAuthServices};
use crate::oauth_dcr::{OAuthDcrProvider, OAuthDcrProviderConfig, OAuthDcrProviderRegistry};

#[tokio::test]
async fn dcr_challenge_errors_propagate_through_product_auth_provider() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dcr_provider = Arc::new(
        OAuthDcrProvider::new(
            OAuthDcrProviderConfig {
                spec: crate::notion_oauth::notion_provider_spec(),
                callback_origin: "http://127.0.0.1:3000".to_string(),
                client_name: "Ironclaw".to_string(),
                account_label: CredentialAccountLabel::new("notion").unwrap(),
                scopes: Vec::new(),
            },
            Arc::new(FailingDcrEgress),
            Arc::new(InMemorySecretStore::new()),
            Arc::new(NoopObligationHandler),
        )
        .unwrap(),
    );
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopAuthDispatcher))
            .with_flow_record_source(shared)
            .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![dcr_provider]))),
    );
    let provider = product_auth
        .as_auth_challenge_provider()
        .expect("challenge provider");
    let requirements = vec![RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("notion".to_string()).unwrap(),
        provider_scopes: Vec::new(),
        requester_extension: ExtensionId::new("notion-mcp".to_string()).unwrap(),
    }];

    let error = provider
        .challenge_for_gate(
            &TurnScope::new(
                TenantId::new("tenant").unwrap(),
                Some(AgentId::new("agent").unwrap()),
                Some(ProjectId::new("project").unwrap()),
                ThreadId::new("thread").unwrap(),
            ),
            &UserId::new("user").unwrap(),
            TurnRunId::new(),
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            &requirements,
        )
        .await
        .expect_err("DCR setup failure must not be swallowed");

    assert_eq!(
        error.code(),
        ironclaw_auth::AuthErrorCode::BackendUnavailable
    );
}

#[tokio::test]
async fn dcr_registry_returns_none_for_zero_and_multiple_requirements() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dcr_provider = Arc::new(
        OAuthDcrProvider::new(
            OAuthDcrProviderConfig {
                spec: crate::notion_oauth::notion_provider_spec(),
                callback_origin: "http://127.0.0.1:3000".to_string(),
                client_name: "Ironclaw".to_string(),
                account_label: CredentialAccountLabel::new("notion").unwrap(),
                scopes: Vec::new(),
            },
            Arc::new(PanickingDcrEgress),
            Arc::new(InMemorySecretStore::new()),
            Arc::new(NoopObligationHandler),
        )
        .unwrap(),
    );
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopAuthDispatcher))
            .with_flow_record_source(shared)
            .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![dcr_provider]))),
    );
    let provider = product_auth
        .as_auth_challenge_provider()
        .expect("challenge provider");
    let scope = TurnScope::new(
        TenantId::new("tenant").unwrap(),
        Some(AgentId::new("agent").unwrap()),
        Some(ProjectId::new("project").unwrap()),
        ThreadId::new("thread").unwrap(),
    );
    let owner = UserId::new("user").unwrap();
    let run_id = TurnRunId::new();
    let gate_ref = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    let notion = RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("notion".to_string()).unwrap(),
        provider_scopes: Vec::new(),
        requester_extension: ExtensionId::new("notion-mcp".to_string()).unwrap(),
    };

    assert!(
        provider
            .challenge_for_gate(&scope, &owner, run_id, gate_ref, &[])
            .await
            .expect("zero requirement lookup")
            .is_none()
    );
    assert!(
        provider
            .challenge_for_gate(&scope, &owner, run_id, gate_ref, &[notion.clone(), notion])
            .await
            .expect("multiple requirement lookup")
            .is_none()
    );
}

#[derive(Debug)]
struct NoopAuthDispatcher;

#[async_trait]
impl RebornAuthContinuationDispatcher for NoopAuthDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Ok(())
    }
}

#[derive(Debug)]
struct FailingDcrEgress;

#[async_trait]
impl RuntimeHttpEgress for FailingDcrEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
        Ok(RuntimeHttpEgressResponse {
            status: 500,
            headers: Vec::new(),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
            body: Vec::new(),
            saved_body: None,
            redaction_applied: false,
        })
    }
}

#[derive(Debug)]
struct PanickingDcrEgress;

#[async_trait]
impl RuntimeHttpEgress for PanickingDcrEgress {
    async fn execute(
        &self,
        _request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
        panic!("DCR egress should not run when registry ignores zero or multiple requirements")
    }
}

#[derive(Debug)]
struct NoopObligationHandler;

#[async_trait]
impl CapabilityObligationHandler for NoopObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
        Ok(())
    }
}
