use async_trait::async_trait;
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_events::{AuditSink, EventError, InMemoryAuditSink};
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
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
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
    assert_eq!(
        leases.get(&lease.scope, lease.grant.id).await.unwrap(),
        lease
    );
}

#[tokio::test]
async fn approving_pending_dispatch_request_preserves_reviewed_grant_constraints() {
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
    let secret = SecretHandle::new("api-key").unwrap();
    let resource_ceiling = ResourceCeiling {
        max_usd: None,
        max_input_tokens: Some(10),
        max_output_tokens: None,
        max_wall_clock_ms: None,
        max_output_bytes: Some(2048),
        sandbox: None,
    };

    let lease = resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![
                    EffectKind::DispatchCapability,
                    EffectKind::ReadFilesystem,
                    EffectKind::Network,
                    EffectKind::UseSecret,
                ],
                mounts: mounts.clone(),
                network: network.clone(),
                secrets: vec![secret.clone()],
                resource_ceiling: Some(resource_ceiling.clone()),
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    assert_eq!(lease.grant.constraints.mounts, mounts);
    assert_eq!(lease.grant.constraints.network, network);
    assert_eq!(lease.grant.constraints.secrets, vec![secret]);
    assert_eq!(
        lease.grant.constraints.resource_ceiling,
        Some(resource_ceiling)
    );
}

#[tokio::test]
async fn approving_pending_request_keeps_pending_when_lease_issue_fails() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = FailingIssueLeaseStore;
    let resolver = ApprovalResolver::new(&approvals, &leases);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval)
        .await
        .unwrap();

    let err = resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ApprovalResolutionError::Lease(CapabilityLeaseError::Persistence { .. })
    ));
    assert_eq!(
        approvals
            .get(&scope, request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Pending
    );
}

#[tokio::test]
async fn approving_pending_request_revokes_issued_lease_when_approval_update_fails() {
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    let approvals = FailingApproveApprovalStore {
        record: ApprovalRecord {
            scope: scope.clone(),
            request: approval,
            status: ApprovalStatus::Pending,
        },
    };
    let leases = InMemoryCapabilityLeaseStore::new();
    let resolver = ApprovalResolver::new(&approvals, &leases);

    let err = resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, ApprovalResolutionError::RunState(_)));
    let issued = leases.leases_for_scope(&scope).await;
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].status, CapabilityLeaseStatus::Revoked);
}

#[tokio::test]
async fn approving_pending_request_revokes_issued_lease_when_status_was_resolved_concurrently() {
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    let approvals = AlreadyResolvedOnApproveStore {
        record: ApprovalRecord {
            scope: scope.clone(),
            request: approval,
            status: ApprovalStatus::Pending,
        },
        resolved_status: ApprovalStatus::Denied,
    };
    let leases = InMemoryCapabilityLeaseStore::new();
    let resolver = ApprovalResolver::new(&approvals, &leases);

    let err = resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ApprovalResolutionError::NotPending {
            status: ApprovalStatus::Denied
        }
    ));
    let issued = leases.leases_for_scope(&scope).await;
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].status, CapabilityLeaseStatus::Revoked);
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
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        )
        .await
        .unwrap();

    let authorizer = LeaseBackedAuthorizer::new(&leases);
    let decision = authorizer
        .authorize_dispatch(&context, &descriptor, &ResourceEstimate::default())
        .await;

    assert!(matches!(
        decision,
        Decision::Deny {
            reason: DenyReason::MissingGrant
        }
    ));
}

#[tokio::test]
async fn approving_dispatch_without_fingerprint_fails_without_lease_or_status_change() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let resolver = ApprovalResolver::new(&approvals, &leases);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let mut approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    approval.invocation_fingerprint = None;
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval)
        .await
        .unwrap();

    let err = resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ApprovalResolutionError::MissingInvocationFingerprint
    ));
    assert_eq!(leases.leases_for_scope(&scope).await, Vec::new());
    assert_eq!(
        approvals
            .get(&scope, request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Pending
    );
}

#[tokio::test]
async fn approving_pending_dispatch_request_emits_redacted_approval_audit_event() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let audit = InMemoryAuditSink::new();
    let resolver = ApprovalResolver::new(&approvals, &leases).with_audit_sink(&audit);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();

    resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    let emitted = audit.records();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].stage, AuditStage::ApprovalResolved);
    assert_eq!(emitted[0].correlation_id, approval.correlation_id);
    assert_eq!(emitted[0].tenant_id, scope.tenant_id);
    assert_eq!(emitted[0].user_id, scope.user_id);
    assert_eq!(emitted[0].invocation_id, scope.invocation_id);
    assert_eq!(emitted[0].process_id, None);
    assert_eq!(
        emitted[0].extension_id,
        Some(ExtensionId::new("caller").unwrap())
    );
    assert_eq!(emitted[0].action.kind, "dispatch");
    assert_eq!(emitted[0].action.target.as_deref(), Some("echo.say"));
    assert_eq!(emitted[0].decision.kind, "approved");
    assert_eq!(
        emitted[0].decision.actor,
        Some(Principal::User(scope.user_id.clone()))
    );
    assert_eq!(emitted[0].result, None);
}

#[tokio::test]
async fn denying_pending_dispatch_request_emits_redacted_approval_audit_event() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let audit = InMemoryAuditSink::new();
    let resolver = ApprovalResolver::new(&approvals, &leases).with_audit_sink(&audit);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();

    resolver
        .deny(
            &scope,
            request_id,
            DenyApproval {
                denied_by: Principal::User(scope.user_id.clone()),
            },
        )
        .await
        .unwrap();

    let emitted = audit.records();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].stage, AuditStage::ApprovalResolved);
    assert_eq!(emitted[0].correlation_id, approval.correlation_id);
    assert_eq!(emitted[0].tenant_id, scope.tenant_id);
    assert_eq!(emitted[0].user_id, scope.user_id);
    assert_eq!(emitted[0].invocation_id, scope.invocation_id);
    assert_eq!(
        emitted[0].extension_id,
        Some(ExtensionId::new("caller").unwrap())
    );
    assert_eq!(emitted[0].action.kind, "dispatch");
    assert_eq!(emitted[0].action.target.as_deref(), Some("echo.say"));
    assert_eq!(emitted[0].decision.kind, "denied");
    assert_eq!(
        emitted[0].decision.actor,
        Some(Principal::User(scope.user_id.clone()))
    );
    assert_eq!(emitted[0].result, None);
}

#[tokio::test]
async fn approval_audit_event_sink_failure_does_not_change_resolution_outcome() {
    let approvals = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let audit = FailingAuditSink;
    let resolver = ApprovalResolver::new(&approvals, &leases).with_audit_sink(&audit);
    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant1", "user1");
    let approval = approval_request(invocation_id, CapabilityId::new("echo.say").unwrap());
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval)
        .await
        .unwrap();

    let lease = resolver
        .approve_dispatch(
            &scope,
            request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();

    assert_eq!(
        approvals
            .get(&scope, request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Approved
    );
    assert_eq!(leases.get(&scope, lease.grant.id).await, Some(lease));
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

    let denied = resolver
        .deny(
            &scope,
            request_id,
            DenyApproval {
                denied_by: Principal::User(scope.user_id.clone()),
            },
        )
        .await
        .unwrap();

    assert_eq!(denied.status, ApprovalStatus::Denied);
    assert_eq!(leases.leases_for_scope(&scope).await, Vec::new());
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

    let err = resolver
        .deny(
            &scope,
            request_id,
            DenyApproval {
                denied_by: Principal::User(scope.user_id.clone()),
            },
        )
        .await
        .unwrap_err();

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
                mounts: Default::default(),
                network: Default::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, ApprovalResolutionError::RunState(_)));
    assert_eq!(leases.leases_for_scope(&tenant_a).await, Vec::new());
    assert_eq!(leases.leases_for_scope(&tenant_b).await, Vec::new());
}

struct FailingAuditSink;

#[async_trait]
impl AuditSink for FailingAuditSink {
    async fn emit_audit(&self, _record: AuditEnvelope) -> Result<(), EventError> {
        Err(EventError::Sink {
            reason: "audit sink unavailable".to_string(),
        })
    }
}

struct FailingApproveApprovalStore {
    record: ApprovalRecord,
}

#[async_trait]
impl ApprovalRequestStore for FailingApproveApprovalStore {
    async fn save_pending(
        &self,
        _scope: ResourceScope,
        _request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        Ok(self.record.clone())
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        if self.record.scope == *scope && self.record.request.id == request_id {
            Ok(Some(self.record.clone()))
        } else {
            Ok(None)
        }
    }

    async fn approve(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        Err(RunStateError::Filesystem(
            "injected approval write failure".to_string(),
        ))
    }

    async fn deny(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        Ok(self.record.clone())
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        if same_tenant_user_for_test(&self.record.scope, scope) {
            Ok(vec![self.record.clone()])
        } else {
            Ok(Vec::new())
        }
    }
}

struct AlreadyResolvedOnApproveStore {
    record: ApprovalRecord,
    resolved_status: ApprovalStatus,
}

#[async_trait]
impl ApprovalRequestStore for AlreadyResolvedOnApproveStore {
    async fn save_pending(
        &self,
        _scope: ResourceScope,
        _request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        Ok(self.record.clone())
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        if self.record.scope == *scope && self.record.request.id == request_id {
            Ok(Some(self.record.clone()))
        } else {
            Ok(None)
        }
    }

    async fn approve(
        &self,
        _scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        Err(RunStateError::ApprovalNotPending {
            request_id,
            status: self.resolved_status,
        })
    }

    async fn deny(
        &self,
        _scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        Err(RunStateError::ApprovalNotPending {
            request_id,
            status: self.resolved_status,
        })
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        if same_tenant_user_for_test(&self.record.scope, scope) {
            Ok(vec![self.record.clone()])
        } else {
            Ok(Vec::new())
        }
    }
}

struct FailingIssueLeaseStore;

#[async_trait]
impl CapabilityLeaseStore for FailingIssueLeaseStore {
    async fn issue(
        &self,
        _lease: CapabilityLease,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        Err(CapabilityLeaseError::Persistence {
            reason: "injected lease write failure".to_string(),
        })
    }

    async fn revoke(
        &self,
        _scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        Err(CapabilityLeaseError::UnknownLease { lease_id })
    }

    async fn get(
        &self,
        _scope: &ResourceScope,
        _lease_id: CapabilityGrantId,
    ) -> Option<CapabilityLease> {
        None
    }

    async fn claim(
        &self,
        _scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        _invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        Err(CapabilityLeaseError::UnknownLease { lease_id })
    }

    async fn consume(
        &self,
        _scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        Err(CapabilityLeaseError::UnknownLease { lease_id })
    }

    async fn leases_for_scope(&self, _scope: &ResourceScope) -> Vec<CapabilityLease> {
        Vec::new()
    }

    async fn active_leases_for_context(&self, _context: &ExecutionContext) -> Vec<CapabilityLease> {
        Vec::new()
    }
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
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    }
}

fn same_tenant_user_for_test(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id && left.user_id == right.user_id
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
        agent_id: None,
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
