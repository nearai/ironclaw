use ironclaw_auth::{
    AuthContinuationRef, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
    CredentialAccountLookupRequest, CredentialAccountSelectionRequest,
};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, InvocationId, MountView, NetworkPolicy, NetworkTargetPattern,
    Principal, ResourceScope, RunId, RuntimeCredentialAccountSetup, RuntimeKind, TenantId,
    ThreadId, TrustClass, UserId,
};
use ironclaw_host_runtime::{RuntimeCapabilityOutcome, RuntimeFailureKind};

use crate::extension_host::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::extension_host::extension_lifecycle_capabilities::{
    EXTENSION_ACTIVATE_CAPABILITY_ID, EXTENSION_INSTALL_CAPABILITY_ID,
};
use crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionRequest;
use crate::{
    RebornBuildInput, RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest,
    RebornServices, build_reborn_services,
};

#[tokio::test]
async fn local_dev_extension_activate_accepts_manual_token_from_webui_gate_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let services = build_reborn_services(RebornBuildInput::local_dev(
        "3eee560a-7fe5-474c-965a-67cb69df3d04",
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let extension_management = services
        .local_runtime
        .as_ref()
        .expect("local runtime substrate")
        .extension_management
        .as_ref()
        .expect("extension management")
        .clone();
    let install_scope = webui_gate_resource_scope();
    let auth_scope_resource = webui_gate_resource_scope();
    let activate_scope = webui_gate_resource_scope();

    invoke_json_with_context(
        &services,
        EXTENSION_INSTALL_CAPABILITY_ID,
        execution_context_for_scope(install_scope, [EXTENSION_INSTALL_CAPABILITY_ID]),
        serde_json::json!({"extension_id": "github"}),
    )
    .await
    .expect("install succeeds");

    let auth_scope = AuthProductScope::new(auth_scope_resource.clone(), AuthSurface::Callback);
    let product_auth = services.product_auth.as_ref().expect("product auth");
    let challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest {
            scope: auth_scope.clone(),
            provider: AuthProviderId::new("github").expect("provider"),
            label: CredentialAccountLabel::new("work github").expect("label"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        })
        .await
        .expect("manual token setup");
    let submitted = product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            auth_scope,
            challenge.interaction_id,
            secrecy::SecretString::from("github-token".to_string()),
        ))
        .await
        .expect("manual token submit");
    let account = product_auth
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            AuthProductScope::new(auth_scope_resource.clone(), AuthSurface::Callback),
            submitted.account_id,
        ))
        .await
        .expect("manual token account lookup")
        .expect("manual token account");
    assert!(
        account.access_secret.is_some(),
        "manual-token submit must configure an access secret"
    );
    let selected = product_auth
        .runtime_credential_account_selection_service()
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(
                AuthProductScope::credential_owner(&activate_scope, AuthSurface::Api),
                AuthProviderId::new("github").expect("provider"),
            )
            .for_extension(ExtensionId::new("github").expect("extension")),
            AuthProductScope::new(activate_scope.clone(), AuthSurface::Api),
            RuntimeCredentialAccountSetup::ManualToken,
            Vec::new(),
        ))
        .await
        .expect("runtime selector should find submitted manual token");
    assert!(
        selected.access_secret.is_some(),
        "runtime selector must return the configured manual-token access secret"
    );

    let activate_outcome = invoke_outcome_with_context(
        &services,
        EXTENSION_ACTIVATE_CAPABILITY_ID,
        execution_context_for_scope(activate_scope, [EXTENSION_ACTIVATE_CAPABILITY_ID]),
        serde_json::json!({"extension_id": "github"}),
    )
    .await;
    let RuntimeCapabilityOutcome::Completed(activate_completed) = activate_outcome else {
        panic!("expected activation to use submitted manual token, got {activate_outcome:?}");
    };
    assert_eq!(activate_completed.output["payload"]["activated"], true);

    let active = active_extension_capability_ids(&extension_management).await;
    assert!(active.iter().any(|id| id == "github.search_issues"));
}

/// W4-MCP-SSO-WIRING (#5439 class): the NEAR AI MCP host-managed credential
/// fallback (`HostManagedCredentialFallbackRule`) must actually reach the
/// runtime-credential-selection path `build_reborn_services` wires up — not
/// just the private rule/selector types the crate's other unit tests exercise
/// directly. Before #5439, the bootstrapped NEAR AI MCP API key was resolvable
/// only under the boot-owner's own scope; a Google-SSO user in the SAME
/// tenant/agent/project with no NEAR AI token of their own was prompted for
/// one instead of transparently sharing the host-managed key.
///
/// Drives ONLY the composition's public surface: `build_reborn_services` (the
/// local-dev path always derives `nearai_mcp_host_managed_scope` from the
/// boot owner — see `local_dev_nearai_mcp_owner_scope` in `factory.rs` — so no
/// live NEAR AI config injection is needed to prove the wiring) and
/// `RebornProductAuthServices`'s existing `pub(crate)`
/// `runtime_credential_account_selection_service()` accessor (this test file
/// is already inside the crate, same seam the sibling github test above
/// drives). Two arms on the SAME composed `services`, discriminating the
/// fallback's scope match rather than asserting vacuous success:
/// - an SSO user in the SAME tenant/agent as the boot owner (a different
///   project — local-dev's host scope is project-*unscoped*, so it is
///   reusable across projects under that agent by design, see
///   `HostManagedCredentialFallbackRule::scope_matches`'s doc) resolves the
///   owner's NEAR AI account via fallback (the #5439 fix);
/// - an SSO user under a DIFFERENT tenant/agent does NOT
///   (`HostManagedCredentialFallbackRule` requires an exact tenant+agent
///   match; it is not a global bypass) -- proves the positive arm is a real
///   scope match, not the selector silently always succeeding.
#[tokio::test]
async fn local_dev_nearai_runtime_selection_falls_back_to_host_managed_account_for_sso_user() {
    let dir = tempfile::tempdir().expect("tempdir");
    let owner_id = "3eee560a-7fe5-474c-965a-67cb69df3d04";
    let services = build_reborn_services(RebornBuildInput::local_dev(
        owner_id,
        dir.path().join("local-dev"),
    ))
    .await
    .expect("local-dev services build");
    let product_auth = services.product_auth.as_ref().expect("product auth");
    let nearai_provider = AuthProviderId::new("nearai").expect("provider"); // safety: static test provider id is valid.
    let nearai_extension = ExtensionId::new("nearai").expect("extension"); // safety: static test extension id is valid.

    // The boot owner submits their own NEAR AI manual token (mirrors the
    // production bootstrap's ProductAuthExtensionCredentialSetup submission,
    // through the same public manual-token flow the github test above uses --
    // the selection service does not care which entry point created the
    // account).
    let owner_scope = webui_gate_resource_scope();
    let owner_auth_scope = AuthProductScope::new(owner_scope.clone(), AuthSurface::Callback);
    let challenge = product_auth
        .request_manual_token_setup(RebornManualTokenSetupRequest {
            scope: owner_auth_scope.clone(),
            provider: nearai_provider.clone(),
            label: CredentialAccountLabel::new("host nearai key").expect("label"),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        })
        .await
        .expect("owner nearai manual token setup");
    product_auth
        .submit_manual_token(RebornManualTokenSubmitRequest::new(
            owner_auth_scope,
            challenge.interaction_id,
            secrecy::SecretString::from("nearai-host-key".to_string()),
        ))
        .await
        .expect("owner nearai manual token submit");

    // Arm 1: an SSO user in the SAME tenant/agent as the boot owner but a
    // DIFFERENT project, with NO NEAR AI account of their own, resolves via
    // the host-managed fallback -- the local-dev host scope is
    // project-unscoped, so it is reusable across projects under the same
    // agent by design (the #5439 fix).
    let sso_other_project_scope = ResourceScope {
        user_id: UserId::new("sso-user-other-project").expect("user"), // safety: static test user id is valid.
        project_id: Some(ironclaw_host_api::ProjectId::new("other-project").expect("project")), // safety: static test project id is valid.
        thread_id: None,
        ..owner_scope.clone()
    };
    let sso_selected = product_auth
        .runtime_credential_account_selection_service()
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(
                AuthProductScope::credential_owner(&sso_other_project_scope, AuthSurface::Api),
                nearai_provider.clone(),
            )
            .for_extension(nearai_extension.clone()),
            AuthProductScope::new(sso_other_project_scope, AuthSurface::Api),
            RuntimeCredentialAccountSetup::ManualToken,
            Vec::new(),
        ))
        .await
        .expect(
            "SSO user in the boot owner's tenant/agent (different project) must resolve the \
             host-managed NEAR AI account via the fallback rule (#5439)",
        );
    assert!(
        sso_selected.access_secret.is_some(),
        "fallback-resolved account must carry the owner's configured access secret"
    );

    // Arm 2 (negative control): an SSO user under a DIFFERENT tenant must NOT
    // fall back -- `HostManagedCredentialFallbackRule::scope_matches` requires
    // an exact tenant+agent match. Proves arm 1 is a real scope match, not the
    // selector silently always succeeding.
    let sso_other_tenant_scope = ResourceScope {
        tenant_id: TenantId::new("other-tenant").expect("tenant"), // safety: static test tenant id is valid.
        user_id: UserId::new("sso-user-other-tenant").expect("user"), // safety: static test user id is valid.
        project_id: None,
        thread_id: None,
        ..owner_scope
    };
    let sso_other_tenant_error = product_auth
        .runtime_credential_account_selection_service()
        .select_unique_configured_runtime_account(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(
                AuthProductScope::credential_owner(&sso_other_tenant_scope, AuthSurface::Api),
                nearai_provider,
            )
            .for_extension(nearai_extension),
            AuthProductScope::new(sso_other_tenant_scope, AuthSurface::Api),
            RuntimeCredentialAccountSetup::ManualToken,
            Vec::new(),
        ))
        .await
        .expect_err(
            "an SSO user under a different tenant must NOT resolve the host-managed account -- \
             the fallback rule is scoped, not a global bypass",
        );
    assert!(
        matches!(
            sso_other_tenant_error,
            ironclaw_auth::AuthProductError::CredentialMissing
        ),
        "expected CredentialMissing for the out-of-scope SSO user, got {sso_other_tenant_error:?}"
    );
}

async fn invoke_json_with_context(
    services: &RebornServices,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> Result<serde_json::Value, RuntimeFailureKind> {
    crate::approval_test_support::invoke_json_with_local_dev_approval(
        services,
        capability_id,
        context,
        input,
    )
    .await
}

async fn invoke_outcome_with_context(
    services: &RebornServices,
    capability_id: &str,
    context: ExecutionContext,
    input: serde_json::Value,
) -> RuntimeCapabilityOutcome {
    crate::approval_test_support::invoke_with_local_dev_approval(
        services,
        capability_id,
        context,
        input,
    )
    .await
}

async fn active_extension_capability_ids(
    extension_management: &RebornLocalExtensionManagementPort,
) -> Vec<String> {
    extension_management
        .active_model_visible_capabilities()
        .await
        .expect("active extension capabilities") // safety: test-only helper asserts fixture setup.
        .into_iter()
        .map(|capability| capability.id.as_str().to_string())
        .collect()
}

fn execution_context_for_scope<'a>(
    resource_scope: ResourceScope,
    capability_ids: impl IntoIterator<Item = &'a str>,
) -> ExecutionContext {
    let caller = ExtensionId::new("extension-tool-test-caller").expect("valid extension id"); // safety: static test extension id is valid.
    let context = ExecutionContext {
        run_id: Some(RunId::new()),
        origin: None,
        invocation_id: resource_scope.invocation_id,
        correlation_id: ironclaw_host_api::CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        authenticated_actor_user_id: None,
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: caller.clone(),
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::FirstParty,
        grants: CapabilitySet {
            grants: capability_ids
                .into_iter()
                .map(|capability_id| capability_grant(capability_id, caller.clone()))
                .collect(),
        },
        mounts: MountView::default(),
        resource_scope,
    };
    context.validate().expect("valid execution context"); // safety: test fixture builds a matching execution/resource scope.
    context
}

fn webui_gate_resource_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("reborn-cli").expect("tenant"), // safety: static test tenant id is valid.
        user_id: UserId::new("3eee560a-7fe5-474c-965a-67cb69df3d04").expect("user"), // safety: static test user id is valid.
        agent_id: Some(ironclaw_host_api::AgentId::new("reborn-cli-agent").expect("agent")), // safety: static test agent id is valid.
        project_id: None,
        mission_id: None,
        thread_id: Some(ThreadId::new("80aa051d-7670-5534-a2c5-2c14339e8af7").expect("thread")), // safety: static test thread id is valid.
        invocation_id: InvocationId::new(),
    }
}

fn capability_grant(capability_id: &str, grantee: ExtensionId) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: CapabilityId::new(capability_id).expect("valid capability id"), // safety: test passes known lifecycle capability ids.
        grantee: Principal::Extension(grantee),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: allowed_effects(),
            mounts: MountView::default(),
            network: NetworkPolicy {
                allowed_targets: vec![NetworkTargetPattern {
                    scheme: None,
                    host_pattern: "*".to_string(),
                    port: None,
                }],
                deny_private_ip_ranges: true,
                max_egress_bytes: None,
            },
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::Network,
    ]
}
