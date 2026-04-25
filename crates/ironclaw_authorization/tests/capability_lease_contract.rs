use ironclaw_authorization::*;
use ironclaw_host_api::*;

#[test]
fn lease_authorizer_allows_matching_active_lease_without_context_grant() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());

    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    leases.issue(lease.clone());

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision =
        authorizer.authorize_dispatch(&context, &descriptor, &ResourceEstimate::default());

    assert!(matches!(decision, Decision::Allow { .. }));
    assert_eq!(
        leases.get(&context.resource_scope, lease.grant.id).unwrap(),
        lease
    );
}

#[test]
fn lease_authorizer_hides_leases_across_tenant_scope() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let other_scope = ResourceScope {
        tenant_id: TenantId::new("tenant2").unwrap(),
        user_id: context.resource_scope.user_id.clone(),
        project_id: context.resource_scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: context.invocation_id,
    };
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());

    leases.issue(CapabilityLease::new(
        other_scope,
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    ));

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision =
        authorizer.authorize_dispatch(&context, &descriptor, &ResourceEstimate::default());

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[test]
fn revocation_is_scoped_to_tenant_and_user() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let other_scope = ResourceScope {
        tenant_id: TenantId::new("tenant2").unwrap(),
        user_id: context.resource_scope.user_id.clone(),
        project_id: context.resource_scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: context.invocation_id,
    };
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    let lease_id = lease.grant.id;
    leases.issue(lease.clone());

    let err = leases.revoke(&other_scope, lease_id).unwrap_err();

    assert!(matches!(
        err,
        CapabilityLeaseError::UnknownLease { lease_id: id } if id == lease_id
    ));
    assert_eq!(
        leases
            .get(&context.resource_scope, lease_id)
            .unwrap()
            .status,
        CapabilityLeaseStatus::Active
    );
}

#[test]
fn lease_authorizer_denies_invalid_context_before_grant_match() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let mut context = execution_context(CapabilitySet::default());
    context.tenant_id = TenantId::new("tenant2").unwrap();
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    leases.issue(CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    ));

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision =
        authorizer.authorize_dispatch(&context, &descriptor, &ResourceEstimate::default());

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::InternalInvariantViolation
        }
    ));
}

#[test]
fn one_off_lease_does_not_authorize_different_invocation_in_same_tenant() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let mut next_invocation_context = execution_context(CapabilitySet::default());
    next_invocation_context.tenant_id = context.tenant_id.clone();
    next_invocation_context.user_id = context.user_id.clone();
    next_invocation_context.project_id = context.project_id.clone();
    next_invocation_context.resource_scope.tenant_id = context.tenant_id.clone();
    next_invocation_context.resource_scope.user_id = context.user_id.clone();
    next_invocation_context.resource_scope.project_id = context.project_id.clone();
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    leases.issue(lease);

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer.authorize_dispatch(
        &next_invocation_context,
        &descriptor,
        &ResourceEstimate::default(),
    );

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[test]
fn revoked_lease_no_longer_authorizes_dispatch() {
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let lease = CapabilityLease::new(
        context.resource_scope.clone(),
        grant_for(
            descriptor.id.clone(),
            Principal::Extension(context.extension_id.clone()),
            vec![EffectKind::DispatchCapability],
        ),
    );
    let lease_id = lease.grant.id;
    leases.issue(lease);
    leases.revoke(&context.resource_scope, lease_id).unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision =
        authorizer.authorize_dispatch(&context, &descriptor, &ResourceEstimate::default());

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

fn descriptor(id: CapabilityId) -> CapabilityDescriptor {
    CapabilityDescriptor {
        provider: ExtensionId::new(id.as_str().split('.').next().unwrap()).unwrap(),
        id,
        runtime: RuntimeKind::Wasm,
        trust_ceiling: TrustClass::Sandbox,
        description: "test".to_string(),
        parameters_schema: serde_json::json!({"type": "object"}),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Deny,
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
        issued_by: Principal::System,
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
