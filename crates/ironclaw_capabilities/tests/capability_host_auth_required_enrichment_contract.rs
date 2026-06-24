// Regression tests for credential-requirement enrichment on dispatch-time AuthRequired.
//
// Bug: WASM extensions that signal `auth_required` after a 401 on an injected
// credential produced a `DispatchError::AuthRequired` with empty
// `credential_requirements` because the WASM adapter only receives the error
// string, not the obligation list.  The WebUI's manual-token card inspects
// `provider` (derived from `credential_requirements`) and refused to send the
// network request when it was absent.
//
// Fix: `CapabilityHost::invoke_json` (and the shared dispatch-resumed tail)
// enriches empty `credential_requirements` from the capability's declared
// `InjectCredentialAccountOnce` obligations before converting the dispatch
// error into a `CapabilityInvocationError`.
//
// These tests drive `CapabilityHost::invoke_json` — the caller — rather than
// `enrich_auth_required_from_obligations` alone, so they cover the layer where
// the enrichment input (obligations) is silently dropped if the fix is absent.
// (See `.claude/rules/testing.md` "Test Through the Caller, Not Just the Helper".)
use async_trait::async_trait;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_capabilities::*;
use ironclaw_host_api::*;
use ironclaw_trust::TrustDecision;
use serde_json::json;

mod support;
use support::*;

// ---------------------------------------------------------------------------
// Stub: authorizer that returns Allow with an InjectCredentialAccountOnce
// obligation carrying a specific provider identity.
// ---------------------------------------------------------------------------

struct CredentialObligationAuthorizer {
    provider: RuntimeCredentialAccountProviderId,
    setup: RuntimeCredentialAccountSetup,
    requester_extension: ExtensionId,
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for CredentialObligationAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(vec![Obligation::InjectCredentialAccountOnce {
                handle: SecretHandle::new("github_pat").unwrap(),
                provider: self.provider.clone(),
                setup: self.setup.clone(),
                provider_scopes: Vec::new(),
                requester_extension: self.requester_extension.clone(),
            }])
            .unwrap(),
        }
    }
}

// ---------------------------------------------------------------------------
// Stub: obligation handler that accepts all obligations unconditionally (the
// failure under test happens at dispatch time, not obligation time).
// ---------------------------------------------------------------------------

struct PassthroughObligationHandler;

#[async_trait]
impl CapabilityObligationHandler for PassthroughObligationHandler {
    async fn satisfy(
        &self,
        _request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Stub: dispatcher that always returns AuthRequired with an empty
// credential_requirements list, simulating a WASM adapter that only knows the
// error string and has no access to the obligation list.
// ---------------------------------------------------------------------------

struct AuthRequiredDispatcher;

#[async_trait]
impl CapabilityDispatcher for AuthRequiredDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        Err(DispatchError::AuthRequired {
            capability: request.capability_id,
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Test: invoke_json enriches empty credential_requirements from obligations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invoke_json_enriches_auth_required_credential_requirements_from_obligations() {
    let registry = registry_with_echo_capability();
    let provider = RuntimeCredentialAccountProviderId::new("github").unwrap();
    let requester = ExtensionId::new("github").unwrap();
    let authorizer = CredentialObligationAuthorizer {
        provider: provider.clone(),
        setup: RuntimeCredentialAccountSetup::ManualToken,
        requester_extension: requester,
    };
    let dispatcher = AuthRequiredDispatcher;
    let handler = PassthroughObligationHandler;
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"owner": "acme", "repo": "api", "issue_number": 1, "body": "hi"}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    let CapabilityInvocationError::AuthorizationRequiresAuth {
        required_secrets,
        credential_requirements,
        ..
    } = err
    else {
        panic!("expected AuthorizationRequiresAuth, got {err:?}");
    };
    assert!(required_secrets.is_empty());
    assert_eq!(
        credential_requirements.len(),
        1,
        "expected one credential requirement enriched from InjectCredentialAccountOnce obligation"
    );
    assert_eq!(
        credential_requirements[0].provider, provider,
        "enriched requirement must carry the declared provider id"
    );
    assert_eq!(
        credential_requirements[0].setup,
        RuntimeCredentialAccountSetup::ManualToken,
    );
}

// ---------------------------------------------------------------------------
// Test: invoke_json does NOT overwrite non-empty credential_requirements
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invoke_json_preserves_non_empty_credential_requirements_from_dispatcher() {
    // When the runtime already supplies requirements (e.g. MCP), the enrichment
    // must not replace them.
    struct AuthRequiredWithRequirementsDispatcher;

    #[async_trait]
    impl CapabilityDispatcher for AuthRequiredWithRequirementsDispatcher {
        async fn dispatch_json(
            &self,
            request: CapabilityDispatchRequest,
        ) -> Result<CapabilityDispatchResult, DispatchError> {
            let mcp_provider = RuntimeCredentialAccountProviderId::new("mcp_provider").unwrap();
            let mcp_ext = ExtensionId::new("mcp_ext").unwrap();
            Err(DispatchError::AuthRequired {
                capability: request.capability_id,
                required_secrets: Vec::new(),
                credential_requirements: vec![RuntimeCredentialAuthRequirement {
                    provider: mcp_provider,
                    setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
                    requester_extension: mcp_ext,
                    provider_scopes: Vec::new(),
                }],
            })
        }
    }

    let registry = registry_with_echo_capability();
    let obligation_provider = RuntimeCredentialAccountProviderId::new("github").unwrap();
    let requester = ExtensionId::new("github").unwrap();
    let authorizer = CredentialObligationAuthorizer {
        provider: obligation_provider,
        setup: RuntimeCredentialAccountSetup::ManualToken,
        requester_extension: requester,
    };
    let dispatcher = AuthRequiredWithRequirementsDispatcher;
    let handler = PassthroughObligationHandler;
    let host =
        CapabilityHost::new(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({}),
            trust_decision: trust_decision(),
        })
        .await
        .unwrap_err();

    let CapabilityInvocationError::AuthorizationRequiresAuth {
        credential_requirements,
        ..
    } = err
    else {
        panic!("expected AuthorizationRequiresAuth, got {err:?}");
    };
    assert_eq!(
        credential_requirements.len(),
        1,
        "non-empty runtime requirements must not be replaced by obligation enrichment"
    );
    // The retained requirement must be the one from the dispatcher (mcp_provider),
    // not the one from the obligation (github).
    assert_eq!(
        credential_requirements[0].provider,
        RuntimeCredentialAccountProviderId::new("mcp_provider").unwrap(),
    );
}
