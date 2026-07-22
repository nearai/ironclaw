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
// `enrich_dispatch_error_credential_requirements` alone, so they cover the layer where
// the enrichment input (obligations) is silently dropped if the fix is absent.
// (See `.claude/rules/testing.md` "Test Through the Caller, Not Just the Helper".)
use async_trait::async_trait;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_capabilities::*;
use ironclaw_host_api::dispatch_test_support::TestDispatcher;
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
    provider: VendorId,
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

// The dispatcher double that always returns AuthRequired with an empty
// credential_requirements list (simulating a WASM adapter that only knows the
// error string) is `TestDispatcher::auth_required()`.

// ---------------------------------------------------------------------------
// Test: invoke_json enriches empty credential_requirements from obligations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invoke_json_enriches_auth_required_credential_requirements_from_obligations() {
    let registry = registry_with_echo_capability();
    let provider = VendorId::new("github").unwrap();
    let requester = ExtensionId::new("github").unwrap();
    let authorizer = CredentialObligationAuthorizer {
        provider: provider.clone(),
        setup: RuntimeCredentialAccountSetup::ManualToken,
        requester_extension: requester,
    };
    let dispatcher = TestDispatcher::auth_required();
    let handler = PassthroughObligationHandler;
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({"owner": "acme", "repo": "api", "issue_number": 1, "body": "hi"}),
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
    let registry = registry_with_echo_capability();
    let obligation_provider = VendorId::new("github").unwrap();
    let requester = ExtensionId::new("github").unwrap();
    let authorizer = CredentialObligationAuthorizer {
        provider: obligation_provider,
        setup: RuntimeCredentialAccountSetup::ManualToken,
        requester_extension: requester,
    };
    let dispatcher = TestDispatcher::responding(|request, _| {
        Err(DispatchError::AuthRequired {
            capability: request.invocation.capability.clone(),
            required_secrets: Vec::new(),
            credential_requirements: vec![RuntimeCredentialAuthRequirement {
                provider: VendorId::new("mcp_provider").unwrap(),
                setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
                requester_extension: ExtensionId::new("mcp_ext").unwrap(),
                provider_scopes: Vec::new(),
            }],
        })
    });
    let handler = PassthroughObligationHandler;
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({}),
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
        VendorId::new("mcp_provider").unwrap(),
    );
}

// ---------------------------------------------------------------------------
// Test: dispatch_resumed_capability (second call site) also enriches
//
// Drives `invoke_json` → BlockedAuth → `auth_resume_json` where the
// dispatcher returns AuthRequired with empty credential_requirements on the
// resumed dispatch.  Asserts that the enriched credential_requirements carry
// the provider declared by the authorizer's InjectCredentialAccountOnce
// obligation — proving the second call site in `dispatch_resumed_capability`
// is covered.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_resume_json_enriches_auth_required_credential_requirements_from_obligations() {
    use ironclaw_run_state::{RunStateStore, RunStatus};

    // A dispatcher that returns AuthRequired with an empty credential_requirements
    // list on every call (simulating a WASM adapter at both invoke and resume time).
    let registry = registry_with_echo_capability();
    let provider = VendorId::new("github").unwrap();
    let requester = ExtensionId::new("github").unwrap();
    let authorizer = CredentialObligationAuthorizer {
        provider: provider.clone(),
        setup: RuntimeCredentialAccountSetup::ManualToken,
        requester_extension: requester,
    };
    let dispatcher = TestDispatcher::auth_required();
    let handler = PassthroughObligationHandler;
    let run_state = ironclaw_run_state::in_memory_backed_run_state_store();

    let host = capability_host(&registry, &dispatcher, &authorizer)
        .with_obligation_handler(&handler)
        .with_run_state(&run_state);

    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    // Phase 1: invoke_json → blocked at auth.
    let invoke_err = host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"owner": "acme", "repo": "api", "issue_number": 1, "body": "hi"}),
        })
        .await
        .unwrap_err();

    assert!(
        matches!(
            invoke_err,
            CapabilityInvocationError::AuthorizationRequiresAuth { .. }
        ),
        "expected AuthorizationRequiresAuth from invoke_json, got {invoke_err:?}"
    );

    // Manually block the run so auth_resume_json can act on it.
    let run = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::BlockedAuth);

    // Phase 2: auth_resume_json → dispatcher returns AuthRequired again →
    // dispatch_resumed_capability enriches from obligations.
    let resume_err = host
        .auth_resume_json(CapabilityAuthResumeRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({"owner": "acme", "repo": "api", "issue_number": 1, "body": "hi"}),
            approval_request_id: None,
        })
        .await
        .unwrap_err();

    let CapabilityInvocationError::AuthorizationRequiresAuth {
        credential_requirements,
        ..
    } = resume_err
    else {
        panic!("expected AuthorizationRequiresAuth from auth_resume_json, got {resume_err:?}");
    };

    assert_eq!(
        credential_requirements.len(),
        1,
        "resume path must enrich empty credential_requirements from InjectCredentialAccountOnce obligation"
    );
    assert_eq!(
        credential_requirements[0].provider, provider,
        "enriched requirement on resume path must carry the declared provider id"
    );
}

// ---------------------------------------------------------------------------
// Test: multiple InjectCredentialAccountOnce obligations → NOT enriched
//
// When the authorizer declares two InjectCredentialAccountOnce obligations
// (different providers), the gate is left unmodified — credential_requirements
// stays EMPTY — because the failed credential cannot be attributed to one
// provider without guessing.  Emitting the wrong provider would point the
// WebUI manual-token card at the wrong credential and make the gate
// unresolvable.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invoke_json_does_not_enrich_when_multiple_credential_obligations_declared() {
    struct MultiObligationAuthorizer;

    #[async_trait]
    impl TrustAwareCapabilityDispatchAuthorizer for MultiObligationAuthorizer {
        async fn authorize_dispatch_with_trust(
            &self,
            _context: &ExecutionContext,
            _descriptor: &CapabilityDescriptor,
            _estimate: &ResourceEstimate,
            _trust_decision: &ironclaw_trust::TrustDecision,
        ) -> Decision {
            Decision::Allow {
                obligations: Obligations::new(vec![
                    Obligation::InjectCredentialAccountOnce {
                        handle: SecretHandle::new("github_pat").unwrap(),
                        provider: VendorId::new("github").unwrap(),
                        setup: RuntimeCredentialAccountSetup::ManualToken,
                        provider_scopes: Vec::new(),
                        requester_extension: ExtensionId::new("github").unwrap(),
                    },
                    Obligation::InjectCredentialAccountOnce {
                        handle: SecretHandle::new("gitlab_pat").unwrap(),
                        provider: VendorId::new("gitlab").unwrap(),
                        setup: RuntimeCredentialAccountSetup::ManualToken,
                        provider_scopes: Vec::new(),
                        requester_extension: ExtensionId::new("gitlab").unwrap(),
                    },
                ])
                .unwrap(),
            }
        }
    }

    let registry = registry_with_echo_capability();
    let authorizer = MultiObligationAuthorizer;
    let dispatcher = TestDispatcher::auth_required();
    let handler = PassthroughObligationHandler;
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: serde_json::json!({}),
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

    assert!(
        credential_requirements.is_empty(),
        "must NOT enrich when two InjectCredentialAccountOnce obligations are declared — \
         failed credential cannot be attributed to one provider; gate is left unmodified \
         rather than mis-pointed at the wrong provider"
    );
}

// ---------------------------------------------------------------------------
// Test: invoke_json does NOT enrich when required_secrets is already populated
//
// Regression guard: the enrichment helper bails out when `required_secrets` is
// populated, but earlier caller-level tests only exercise the empty-required_secrets
// path.  A future wiring regression in `CapabilityHost::invoke_json` that strips
// the raw-secret gate and re-derives it as a provider prompt would not be caught
// without a test that drives the caller with a pre-populated `required_secrets`.
//
// The authorizer DOES declare an InjectCredentialAccountOnce obligation so the
// test proves the preservation is due to the populated `required_secrets` check,
// not merely an absence of obligations.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invoke_json_preserves_required_secrets_from_dispatcher() {
    // A dispatcher that returns AuthRequired with required_secrets POPULATED
    // and credential_requirements EMPTY — the raw-secret-handle gate case.
    let registry = registry_with_echo_capability();
    let obligation_provider = VendorId::new("github").unwrap();
    let requester = ExtensionId::new("github").unwrap();
    // Authorizer declares an InjectCredentialAccountOnce obligation — enrichment
    // WOULD fire on an empty gate, but must be suppressed here because
    // required_secrets is already populated.
    let authorizer = CredentialObligationAuthorizer {
        provider: obligation_provider,
        setup: RuntimeCredentialAccountSetup::ManualToken,
        requester_extension: requester,
    };
    let dispatcher = TestDispatcher::responding(|request, _| {
        Err(DispatchError::AuthRequired {
            capability: request.invocation.capability.clone(),
            required_secrets: vec![SecretHandle::new("raw_secret_handle").unwrap()],
            credential_requirements: Vec::new(),
        })
    });
    let handler = PassthroughObligationHandler;
    let host =
        capability_host(&registry, &dispatcher, &authorizer).with_obligation_handler(&handler);
    let context = execution_context(CapabilitySet {
        grants: vec![dispatch_grant()],
    });

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: capability_id(),
            estimate: ResourceEstimate::default(),
            input: json!({}),
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
    assert_eq!(
        required_secrets.len(),
        1,
        "required_secrets from dispatcher must be preserved when non-empty"
    );
    assert!(
        credential_requirements.is_empty(),
        "credential_requirements must remain empty when required_secrets are present \
         — enrichment from obligations must be suppressed"
    );
}
