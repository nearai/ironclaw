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
use ironclaw_secrets::FilesystemSecretStore;
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::product_auth::api::auth::RebornProductAuthServices;
use crate::product_auth::oauth::oauth_dcr::{
    OAuthDcrProvider, OAuthDcrProviderConfig, OAuthDcrProviderRegistry,
};
use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;

#[tokio::test]
async fn dcr_challenge_errors_propagate_through_product_auth_provider() {
    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dcr_provider = Arc::new(
        OAuthDcrProvider::new(
            OAuthDcrProviderConfig {
                spec: crate::product_auth::oauth::notion_oauth::notion_provider_spec(),
                callback_origin: "http://127.0.0.1:3000".to_string(),
                client_name: "Ironclaw".to_string(),
                account_label: CredentialAccountLabel::new("notion").unwrap(),
                scopes: Vec::new(),
            },
            Arc::new(FailingDcrEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
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
        setup: Default::default(),
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
                spec: crate::product_auth::oauth::notion_oauth::notion_provider_spec(),
                callback_origin: "http://127.0.0.1:3000".to_string(),
                client_name: "Ironclaw".to_string(),
                account_label: CredentialAccountLabel::new("notion").unwrap(),
                scopes: Vec::new(),
            },
            Arc::new(PanickingDcrEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
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
        setup: Default::default(),
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

/// Supersede-on-start (the `AuthFlowManager::create_flow` contract) must also
/// cover the DCR start route:
/// non-Google extension providers (e.g. Notion) reach flow creation through
/// the DCR registry rather than `start_setup_oauth_flow`, and a re-opened
/// connect popup must not leave two live authorization requests racing to
/// write the same credential.
#[tokio::test]
async fn dcr_setup_restart_supersedes_prior_flow() {
    use ironclaw_auth::AuthFlowManager as _;

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dcr_provider = Arc::new(
        OAuthDcrProvider::new(
            OAuthDcrProviderConfig {
                spec: crate::product_auth::oauth::notion_oauth::notion_provider_spec(),
                callback_origin: "http://127.0.0.1:3000".to_string(),
                client_name: "Ironclaw".to_string(),
                account_label: CredentialAccountLabel::new("notion").unwrap(),
                scopes: Vec::new(),
            },
            Arc::new(ScriptedDcrSetupEgress),
            Arc::new(FilesystemSecretStore::ephemeral()),
            Arc::new(NoopObligationHandler),
        )
        .unwrap(),
    );
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopAuthDispatcher))
            .with_flow_record_source(shared.clone())
            .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![dcr_provider]))),
    );
    let owner = UserId::new("dcr-owner").unwrap();
    let scope_for_request = || {
        ironclaw_auth::AuthProductScope::new(
            ironclaw_host_api::ResourceScope::local_default(
                owner.clone(),
                ironclaw_host_api::InvocationId::new(),
            )
            .unwrap(),
            ironclaw_auth::AuthSurface::Web,
        )
    };
    let start_request = |scope: ironclaw_auth::AuthProductScope| {
        crate::product_auth::api::auth::RebornDcrOAuthStartFlowRequest {
            scope,
            provider: ironclaw_auth::AuthProviderId::new("notion").unwrap(),
            account_label: CredentialAccountLabel::new("personal notion").unwrap(),
            provider_scopes: Vec::new(),
            continuation: ironclaw_auth::AuthContinuationRef::LifecycleActivation {
                package_ref: ironclaw_auth::LifecyclePackageRef::new("notion-mcp").unwrap(),
            },
            update_binding: None,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        }
    };

    // Click Connect: flow one is live.
    let first_scope = scope_for_request();
    let first = product_auth
        .start_dcr_setup_oauth_flow(start_request(first_scope.clone()))
        .await
        .expect("first DCR setup start")
        .expect("DCR registry is configured");

    // Close the popup, click Connect again: a fresh invocation, same owner.
    let second = product_auth
        .start_dcr_setup_oauth_flow(start_request(scope_for_request()))
        .await
        .expect("second DCR setup start")
        .expect("DCR registry is configured");

    assert_ne!(second.id, first.id);
    assert_eq!(second.status, ironclaw_auth::AuthFlowStatus::AwaitingUser);
    assert_eq!(
        shared
            .get_flow(&first_scope, first.id)
            .await
            .expect("first flow lookup")
            .expect("superseded flow is retained")
            .status,
        ironclaw_auth::AuthFlowStatus::Canceled,
        "re-opening the Notion connect popup must supersede the prior DCR setup flow"
    );
}

#[derive(Debug)]
struct ScriptedDcrSetupEgress;

#[async_trait]
impl RuntimeHttpEgress for ScriptedDcrSetupEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
        let body = match request.url.as_str() {
            "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource" => {
                br#"{"authorization_servers":["https://oauth.notion.com"]}"#.to_vec()
            }
            "https://oauth.notion.com/.well-known/oauth-authorization-server" => {
                br#"{"authorization_endpoint":"https://oauth.notion.com/authorize","token_endpoint":"https://oauth.notion.com/token","registration_endpoint":"https://oauth.notion.com/register"}"#.to_vec()
            }
            "https://oauth.notion.com/register" => {
                br#"{"client_id":"dcr-client","registration_client_uri":"https://oauth.notion.com/register/dcr-client","registration_access_token":"registration-token"}"#.to_vec()
            }
            other => panic!("unexpected DCR egress URL: {other}"),
        };
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: Vec::new(),
            request_bytes: request.body.len() as u64,
            response_bytes: body.len() as u64,
            body,
            saved_body: None,
            redaction_applied: false,
        })
    }
}
