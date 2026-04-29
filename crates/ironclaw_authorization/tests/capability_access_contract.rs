use chrono::{Duration, Utc};
use ironclaw_authorization::*;
use ironclaw_host_api::*;
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use serde_json::json;

#[tokio::test]
async fn capability_access_denies_without_matching_grant() {
    let context = execution_context(CapabilitySet::default());
    let descriptor = wasm_descriptor();
    let decision = GrantAuthorizer::new()
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    );
}

#[tokio::test]
async fn capability_access_allows_matching_extension_grant() {
    let descriptor = wasm_descriptor();
    let grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    let context = execution_context(CapabilitySet {
        grants: vec![grant],
    });

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &context,
            &descriptor,
            &ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .await;

    assert_eq!(
        decision,
        Decision::Allow {
            obligations: Default::default()
        }
    );
}

#[tokio::test]
async fn capability_access_returns_grant_constraints_as_runtime_obligations() {
    let descriptor = CapabilityDescriptor {
        effects: vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ReadFilesystem,
        ],
        ..wasm_descriptor()
    };
    let secret = SecretHandle::new("api-key").unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/project1").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();
    let network = NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.com".to_string(),
            port: Some(443),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    };
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ReadFilesystem,
        ],
    );
    grant.constraints.mounts = mounts.clone();
    grant.constraints.network = network.clone();
    grant.constraints.secrets = vec![secret.clone()];
    grant.constraints.resource_ceiling = Some(ResourceCeiling {
        max_usd: None,
        max_input_tokens: None,
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: Some(2048),
        sandbox: None,
    });

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate {
                output_bytes: Some(512),
                ..ResourceEstimate::default()
            },
        )
        .await;

    let Decision::Allow { obligations } = decision else {
        panic!("expected allow decision with obligations, got {decision:?}");
    };
    assert!(obligations.as_slice().iter().any(
        |obligation| matches!(obligation, Obligation::UseScopedMounts { mounts: value } if value == &mounts)
    ));
    assert!(obligations.as_slice().iter().any(
        |obligation| matches!(obligation, Obligation::ApplyNetworkPolicy { policy } if policy == &network)
    ));
    assert!(obligations.as_slice().iter().any(
        |obligation| matches!(obligation, Obligation::InjectSecretOnce { handle } if handle == &secret)
    ));
    assert!(
        obligations
            .as_slice()
            .iter()
            .any(|obligation| matches!(obligation, Obligation::EnforceOutputLimit { bytes: 2048 }))
    );
}

#[tokio::test]
async fn capability_access_denies_when_grant_is_for_different_principal_or_capability() {
    let descriptor = wasm_descriptor();
    let wrong_principal = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("other-extension").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    let wrong_capability = grant_for(
        CapabilityId::new("echo.other").unwrap(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    let authorizer = GrantAuthorizer::new();

    assert_eq!(
        authorizer
            .authorize_dispatch(
                &execution_context(CapabilitySet {
                    grants: vec![wrong_principal]
                }),
                &descriptor,
                &ResourceEstimate::default(),
            )
            .await,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    );
    assert_eq!(
        authorizer
            .authorize_dispatch(
                &execution_context(CapabilitySet {
                    grants: vec![wrong_capability]
                }),
                &descriptor,
                &ResourceEstimate::default(),
            )
            .await,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    );
}

#[tokio::test]
async fn capability_access_denies_when_grant_does_not_cover_declared_effects() {
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        ..wasm_descriptor()
    };
    let grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    let context = execution_context(CapabilitySet {
        grants: vec![grant],
    });

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::PolicyDenied
        }
    );
}

#[tokio::test]
async fn capability_access_with_trust_denies_when_authority_ceiling_excludes_effect() {
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        ..wasm_descriptor()
    };
    let grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    );
    let context = execution_context(CapabilitySet {
        grants: vec![grant],
    });
    let trust = trust_decision(vec![EffectKind::DispatchCapability], None);

    let decision = GrantAuthorizer::new()
        .authorize_dispatch_with_trust(&context, &descriptor, &ResourceEstimate::default(), &trust)
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::PolicyDenied
        }
    );
}

#[tokio::test]
async fn capability_access_with_trust_denies_when_estimate_exceeds_authority_ceiling() {
    let descriptor = wasm_descriptor();
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.resource_ceiling = Some(ResourceCeiling {
        max_usd: None,
        max_input_tokens: None,
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: Some(2048),
        sandbox: None,
    });
    let context = execution_context(CapabilitySet {
        grants: vec![grant],
    });
    let trust = trust_decision(
        vec![EffectKind::DispatchCapability],
        Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: None,
            max_output_bytes: Some(1024),
            sandbox: None,
        }),
    );

    let decision = GrantAuthorizer::new()
        .authorize_dispatch_with_trust(
            &context,
            &descriptor,
            &ResourceEstimate {
                output_bytes: Some(1500),
                ..ResourceEstimate::default()
            },
            &trust,
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
async fn capability_access_with_trust_clamps_runtime_resource_obligation_to_authority_ceiling() {
    let descriptor = wasm_descriptor();
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.resource_ceiling = Some(ResourceCeiling {
        max_usd: None,
        max_input_tokens: None,
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: Some(2048),
        sandbox: None,
    });
    let context = execution_context(CapabilitySet {
        grants: vec![grant],
    });
    let trust = trust_decision(
        vec![EffectKind::DispatchCapability],
        Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: None,
            max_output_bytes: Some(1024),
            sandbox: None,
        }),
    );

    let decision = GrantAuthorizer::new()
        .authorize_dispatch_with_trust(
            &context,
            &descriptor,
            &ResourceEstimate {
                output_bytes: Some(512),
                ..ResourceEstimate::default()
            },
            &trust,
        )
        .await;

    let Decision::Allow { obligations } = decision else {
        panic!("expected allow with clamped obligation, got {decision:?}");
    };
    assert!(
        obligations
            .as_slice()
            .iter()
            .any(|obligation| matches!(obligation, Obligation::EnforceOutputLimit { bytes: 1024 }))
    );
    assert!(
        !obligations
            .as_slice()
            .iter()
            .any(|obligation| matches!(obligation, Obligation::EnforceOutputLimit { bytes: 2048 }))
    );
}

#[tokio::test]
async fn capability_access_with_trust_denies_when_context_trust_differs_from_decision() {
    let descriptor = wasm_descriptor();
    let grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    let mut context = execution_context(CapabilitySet {
        grants: vec![grant],
    });
    context.trust = TrustClass::UserTrusted;
    let trust = trust_decision(vec![EffectKind::DispatchCapability], None);

    let decision = GrantAuthorizer::new()
        .authorize_dispatch_with_trust(&context, &descriptor, &ResourceEstimate::default(), &trust)
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::PolicyDenied
        }
    );
}

#[tokio::test]
async fn capability_spawn_with_trust_requires_spawn_process_in_authority_ceiling() {
    let descriptor = wasm_descriptor();
    let grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
    );
    let context = execution_context(CapabilitySet {
        grants: vec![grant],
    });
    let trust = trust_decision(vec![EffectKind::DispatchCapability], None);

    let decision = GrantAuthorizer::new()
        .authorize_spawn_with_trust(&context, &descriptor, &ResourceEstimate::default(), &trust)
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::PolicyDenied
        }
    );
}

#[test]
fn grant_exceeds_authority_ceiling_detects_effect_reductions() {
    let grant = grant_for(
        CapabilityId::new("echo.say").unwrap(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    );
    let ceiling = AuthorityCeiling {
        allowed_effects: vec![EffectKind::DispatchCapability],
        max_resource_ceiling: None,
    };

    assert!(grant_exceeds_authority_ceiling(&grant, &ceiling));
}

#[test]
fn grant_exceeds_authority_ceiling_detects_resource_reductions() {
    let mut grant = grant_for(
        CapabilityId::new("echo.say").unwrap(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.resource_ceiling = Some(ResourceCeiling {
        max_usd: None,
        max_input_tokens: None,
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: Some(2048),
        sandbox: None,
    });
    let ceiling = AuthorityCeiling {
        allowed_effects: vec![EffectKind::DispatchCapability],
        max_resource_ceiling: Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: None,
            max_output_bytes: Some(1024),
            sandbox: None,
        }),
    };

    assert!(grant_exceeds_authority_ceiling(&grant, &ceiling));
}

#[test]
fn grant_exceeds_authority_ceiling_keeps_grants_within_ceiling() {
    let mut grant = grant_for(
        CapabilityId::new("echo.say").unwrap(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.resource_ceiling = Some(ResourceCeiling {
        max_usd: None,
        max_input_tokens: None,
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: Some(512),
        sandbox: None,
    });
    let ceiling = AuthorityCeiling {
        allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        max_resource_ceiling: Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: None,
            max_output_bytes: Some(1024),
            sandbox: None,
        }),
    };

    assert!(!grant_exceeds_authority_ceiling(&grant, &ceiling));
}

#[tokio::test]
async fn capability_access_skips_expired_and_exhausted_grants() {
    let descriptor = wasm_descriptor();
    let mut expired = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    expired.constraints.expires_at = Some(Utc::now() - Duration::minutes(1));
    let mut exhausted = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    exhausted.constraints.max_invocations = Some(0);

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![expired, exhausted],
            }),
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    );
}

#[tokio::test]
async fn capability_access_allows_later_grant_that_covers_effects() {
    let descriptor = CapabilityDescriptor {
        effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        ..wasm_descriptor()
    };
    let weak = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    let strong = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    );

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![weak, strong],
            }),
            &descriptor,
            &ResourceEstimate::default(),
        )
        .await;

    assert!(matches!(decision, Decision::Allow { .. }));
}

#[tokio::test]
async fn capability_access_denies_when_resource_estimate_exceeds_grant_ceiling() {
    let descriptor = wasm_descriptor();
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.resource_ceiling = Some(ResourceCeiling {
        max_usd: None,
        max_input_tokens: Some(10),
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: None,
        sandbox: None,
    });

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate {
                input_tokens: Some(11),
                ..ResourceEstimate::default()
            },
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
async fn capability_access_denies_when_grant_ceiling_dimension_has_no_estimate() {
    let descriptor = wasm_descriptor();
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.resource_ceiling = Some(ResourceCeiling {
        max_usd: None,
        max_input_tokens: Some(10),
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: None,
        sandbox: None,
    });

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
async fn capability_access_returns_full_resource_ceiling_as_runtime_obligation() {
    let descriptor = wasm_descriptor();
    let ceiling = ResourceCeiling {
        max_usd: None,
        max_input_tokens: Some(10),
        max_output_tokens: Some(20),
        max_wall_clock_ms: Some(1_000),
        max_output_bytes: Some(2_048),
        sandbox: Some(SandboxQuota {
            cpu_time_ms: None,
            memory_bytes: None,
            disk_bytes: None,
            network_egress_bytes: Some(4_096),
            process_count: Some(1),
        }),
    };
    let mut grant = grant_for(
        descriptor.id.clone(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    grant.constraints.resource_ceiling = Some(ceiling.clone());

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(
            &execution_context(CapabilitySet {
                grants: vec![grant],
            }),
            &descriptor,
            &ResourceEstimate {
                input_tokens: Some(5),
                output_tokens: Some(10),
                wall_clock_ms: Some(500),
                output_bytes: Some(1_024),
                network_egress_bytes: Some(2_048),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .await;

    let Decision::Allow { obligations } = decision else {
        panic!("expected allow decision with resource ceiling obligation, got {decision:?}");
    };
    assert!(obligations.as_slice().iter().any(
        |obligation| matches!(obligation, Obligation::EnforceResourceCeiling { ceiling: value } if value == &ceiling)
    ));
}

#[tokio::test]
async fn spawn_access_requires_spawn_process_effect_in_addition_to_capability_effects() {
    let descriptor = wasm_descriptor();
    let dispatch_only = execution_context(CapabilitySet {
        grants: vec![grant_for(
            descriptor.id.clone(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability],
        )],
    });
    let spawn_grant = execution_context(CapabilitySet {
        grants: vec![grant_for(
            descriptor.id.clone(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
        )],
    });
    let authorizer = GrantAuthorizer::new();

    assert_eq!(
        authorizer
            .authorize_spawn(&dispatch_only, &descriptor, &ResourceEstimate::default())
            .await,
        Decision::Deny {
            reason: DenyReason::PolicyDenied
        }
    );
    assert_eq!(
        authorizer
            .authorize_spawn(&spawn_grant, &descriptor, &ResourceEstimate::default())
            .await,
        Decision::Allow {
            obligations: Default::default()
        }
    );
}

#[tokio::test]
async fn capability_access_denies_invalid_execution_context() {
    let grant = grant_for(
        CapabilityId::new("echo.say").unwrap(),
        Principal::Extension(ExtensionId::new("caller").unwrap()),
        vec![EffectKind::DispatchCapability],
    );
    let mut context = execution_context(CapabilitySet {
        grants: vec![grant],
    });
    context.resource_scope.tenant_id = TenantId::new("wrong-tenant").unwrap();

    let decision = GrantAuthorizer::new()
        .authorize_dispatch(&context, &wasm_descriptor(), &ResourceEstimate::default())
        .await;

    assert_eq!(
        decision,
        Decision::Deny {
            reason: DenyReason::InternalInvariantViolation
        }
    );
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
        resource_profile: None,
    }
}

fn trust_decision(
    allowed_effects: Vec<EffectKind>,
    max_resource_ceiling: Option<ResourceCeiling>,
) -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::sandbox(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects,
            max_resource_ceiling,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
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
