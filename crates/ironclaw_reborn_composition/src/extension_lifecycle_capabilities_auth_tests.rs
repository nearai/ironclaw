use ironclaw_auth::{
    AuthContinuationRef, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
    CredentialAccountLookupRequest, CredentialAccountSelectionRequest,
};
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, InvocationId, MountView, NetworkPolicy, NetworkTargetPattern,
    Principal, ResourceScope, RuntimeCredentialAccountSetup, RuntimeKind, TenantId, ThreadId,
    TrustClass, UserId,
};
use ironclaw_host_runtime::{RuntimeCapabilityOutcome, RuntimeFailureKind};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::extension_lifecycle_capabilities::{
    EXTENSION_ACTIVATE_CAPABILITY_ID, EXTENSION_INSTALL_CAPABILITY_ID,
};
use crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest;
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
        trust_decision(),
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
        trust_decision(),
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
        invocation_id: resource_scope.invocation_id,
        correlation_id: ironclaw_host_api::CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
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

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: allowed_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}
