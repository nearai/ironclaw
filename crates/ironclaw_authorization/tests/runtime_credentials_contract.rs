use ironclaw_authorization::*;
use ironclaw_host_api::*;
use serde_json::json;

#[tokio::test]
async fn capability_access_uses_declared_runtime_credential_handles() {
    let declared = SecretHandle::new("github_token").unwrap();
    let other = SecretHandle::new("other_token").unwrap();
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        runtime_credentials: vec![runtime_credential(
            declared.clone(),
            github_audience(),
            true,
        )],
        ..wasm_descriptor()
    };
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    );
    grant.constraints.secrets = vec![other, declared.clone()];

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    let Decision::Allow { obligations } = decision else {
        panic!("expected allow decision, got {decision:?}");
    };
    assert_eq!(
        obligations.as_slice(),
        &[Obligation::InjectSecretOnce { handle: declared }]
    );
}

#[tokio::test]
async fn capability_access_denies_when_declared_runtime_credential_is_not_granted() {
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        runtime_credentials: vec![runtime_credential(
            SecretHandle::new("github_token").unwrap(),
            github_audience(),
            true,
        )],
        ..wasm_descriptor()
    };
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    );
    grant.constraints.secrets = vec![SecretHandle::new("other_token").unwrap()];

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::PolicyDenied
        }
    );
}

#[tokio::test]
async fn capability_access_injects_all_declared_runtime_credentials() {
    let first = SecretHandle::new("github_token").unwrap();
    let second = SecretHandle::new("uploads_token").unwrap();
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        runtime_credentials: vec![
            runtime_credential(first.clone(), github_audience(), true),
            runtime_credential(
                second.clone(),
                NetworkTargetPattern {
                    scheme: Some(NetworkScheme::Https),
                    host_pattern: "uploads.github.com".to_string(),
                    port: None,
                },
                true,
            ),
        ],
        ..wasm_descriptor()
    };
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    );
    grant.constraints.secrets = vec![first.clone(), second.clone()];

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    let Decision::Allow { obligations } = decision else {
        panic!("expected allow decision, got {decision:?}");
    };
    assert_eq!(
        obligations.as_slice(),
        &[
            Obligation::InjectSecretOnce { handle: first },
            Obligation::InjectSecretOnce { handle: second },
        ]
    );
}

#[tokio::test]
async fn capability_access_skips_missing_optional_runtime_credentials() {
    let required = SecretHandle::new("github_token").unwrap();
    let optional = SecretHandle::new("uploads_token").unwrap();
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        runtime_credentials: vec![
            runtime_credential(required.clone(), github_audience(), true),
            runtime_credential(
                optional,
                NetworkTargetPattern {
                    scheme: Some(NetworkScheme::Https),
                    host_pattern: "uploads.github.com".to_string(),
                    port: None,
                },
                false,
            ),
        ],
        ..wasm_descriptor()
    };
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    );
    grant.constraints.secrets = vec![required.clone()];

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    let Decision::Allow { obligations } = decision else {
        panic!("expected allow decision, got {decision:?}");
    };
    assert_eq!(
        obligations.as_slice(),
        &[Obligation::InjectSecretOnce { handle: required }]
    );
}

#[tokio::test]
async fn capability_access_resolves_product_auth_account_runtime_credentials() {
    let slot = SecretHandle::new("github_runtime_token").unwrap();
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
        runtime_credentials: vec![RuntimeCredentialRequirement {
            source: RuntimeCredentialRequirementSource::ProductAuthAccount {
                provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
            },
            ..runtime_credential(slot.clone(), github_audience(), true)
        }],
        ..wasm_descriptor()
    };
    let grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::UseSecret],
    );

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    let Decision::Allow { obligations } = decision else {
        panic!("expected allow decision, got {decision:?}");
    };
    assert_eq!(
        obligations.as_slice(),
        &[Obligation::InjectCredentialAccountOnce {
            handle: slot,
            provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
            requester_extension: ExtensionId::new("echo").unwrap(),
        }]
    );
}

#[tokio::test]
async fn capability_access_denies_runtime_credentials_without_use_secret_effect() {
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability],
        runtime_credentials: vec![runtime_credential(
            SecretHandle::new("github_token").unwrap(),
            github_audience(),
            true,
        )],
        ..wasm_descriptor()
    };
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.secrets = vec![SecretHandle::new("github_token").unwrap()];

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::PolicyDenied
        }
    );
}

fn runtime_credential(
    handle: SecretHandle,
    audience: NetworkTargetPattern,
    required: bool,
) -> RuntimeCredentialRequirement {
    RuntimeCredentialRequirement {
        handle,
        source: Default::default(),
        audience,
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required,
    }
}

fn wasm_descriptor() -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new("echo.say").unwrap(),
        provider: ExtensionId::new("echo").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust_ceiling: TrustClass::Sandbox,
        description: "Echo text".to_string(),
        parameters_schema: json!({"type": "object"}),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Allow,
        runtime_credentials: Vec::new(),
        resource_profile: None,
    }
}

fn grant_for(
    capability: CapabilityId,
    grantee: Principal,
    allowed_effects: Vec<EffectKind>,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability,
        grantee,
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects,
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn github_audience() -> NetworkTargetPattern {
    NetworkTargetPattern {
        scheme: Some(NetworkScheme::Https),
        host_pattern: "api.github.com".to_string(),
        port: None,
    }
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants,
        mounts: MountView::default(),
        resource_scope,
    }
}
