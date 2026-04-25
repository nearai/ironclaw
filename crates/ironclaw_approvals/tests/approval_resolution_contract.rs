use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_host_api::*;
use ironclaw_run_state::*;

#[tokio::test]
async fn approving_pending_dispatch_request_issues_scoped_capability_lease() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let resolver = ApprovalResolver::new(&approvals, &leases);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();

    let lease = resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    assert_eq!(lease.scope, scope);
    assert_eq!(
        lease.grant.capability,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(lease.grant.grantee, approval.requested_by);
    assert_eq!(
        lease.invocation_fingerprint,
        approval.invocation_fingerprint
    );
    assert_eq!(lease.grant.constraints.max_invocations, Some(1));
    assert_eq!(
        approvals
            .get(&lease.scope, request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Approved
    );
    assert_eq!(leases.get(&lease.scope, lease.grant.id).unwrap(), lease);
}

#[tokio::test]
async fn lease_from_approved_request_is_resume_only_and_not_plain_authority() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let resolver = ApprovalResolver::new(&approvals, &leases);
    let context = execution_context(CapabilitySet::default());
    let descriptor = descriptor(CapabilityId::new("echo.say").unwrap());
    let approval = approval_request(context.invocation_id, descriptor.id.clone());
    approvals
        .save_pending(context.resource_scope.clone(), approval.clone())
        .await
        .unwrap();

    resolver
        .approve_dispatch(
            &context.resource_scope,
            approval.id,
            LeaseApproval {
                issued_by: Principal::User(context.user_id.clone()),
                allowed_effects: descriptor.effects.clone(),
                expires_at: None,
                max_invocations: None,
            },
        )
        .await
        .unwrap();

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

#[tokio::test]
async fn denying_pending_request_does_not_issue_lease() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let resolver = ApprovalResolver::new(&approvals, &leases);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval)
        .await
        .unwrap();

    let denied = resolver.deny(&scope, request_id).await.unwrap();

    assert_eq!(denied.status, ApprovalStatus::Denied);
    assert_eq!(leases.leases_for_scope(&scope), Vec::new());
}

#[tokio::test]
async fn denying_non_pending_request_fails_without_changing_status() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let resolver = ApprovalResolver::new(&approvals, &leases);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval)
        .await
        .unwrap();
    approvals.approve(&scope, request_id).await.unwrap();

    let err = resolver.deny(&scope, request_id).await.unwrap_err();

    assert!(matches!(
        err,
        ApprovalResolutionError::NotPending {
            status: ApprovalStatus::Approved
        }
    ));
    assert_eq!(
        approvals
            .get(&scope, request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Approved
    );
}

#[tokio::test]
async fn approving_request_from_other_tenant_fails_closed() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let resolver = ApprovalResolver::new(&approvals, &leases);
    let invocation_id = InvocationId::new();
    let tenant_a = sample_scope(invocation_id, "tenant1", "user1");
    let tenant_b = sample_scope(invocation_id, "tenant2", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    approvals
        .save_pending(tenant_a.clone(), approval)
        .await
        .unwrap();

    let err = resolver
        .approve_dispatch(
            &tenant_b,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(tenant_b.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: None,
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, ApprovalResolutionError::RunState(_)));
    assert_eq!(leases.leases_for_scope(&tenant_a), Vec::new());
    assert_eq!(leases.leases_for_scope(&tenant_b), Vec::new());
}

fn approval_request(invocation_id: InvocationId, capability: CapabilityId) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
        action: Box::new(Action::Dispatch {
            capability: capability.clone(),
            estimated_resources: ResourceEstimate::default(),
        }),
        reason: format!("approval for {invocation_id}"),
        reusable_scope: None,
        invocation_fingerprint: Some(
            InvocationFingerprint::for_dispatch(
                &sample_scope(invocation_id, "tenant1", "user1"),
                &capability,
                &ResourceEstimate::default(),
                &serde_json::json!({"message": "approved"}),
            )
            .unwrap(),
        ),
    }
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

fn sample_scope(invocation_id: InvocationId, tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    }
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = sample_scope(invocation_id, "tenant1", "user1");
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
