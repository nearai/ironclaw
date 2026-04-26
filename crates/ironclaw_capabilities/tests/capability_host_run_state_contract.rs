use async_trait::async_trait;
use ironclaw_approvals::*;
use ironclaw_authorization::*;
use ironclaw_capabilities::*;
use ironclaw_dispatcher::*;
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_run_state::*;
use ironclaw_wasm::*;
use serde_json::json;

#[tokio::test]
async fn capability_host_rejects_invalid_context_before_persisting_run_state() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let authorizer = GrantAuthorizer::new();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let mut context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability],
        )],
    });
    let original_scope = context.resource_scope.clone();
    context.tenant_id = TenantId::new("tenant2").unwrap();

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "invalid context"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::InternalInvariantViolation,
            ..
        }
    ));
    assert_eq!(
        run_state.records_for_scope(&original_scope).await.unwrap(),
        Vec::new()
    );
    assert_eq!(
        approval_requests
            .records_for_scope(&original_scope)
            .await
            .unwrap(),
        Vec::new()
    );
}

#[tokio::test]
async fn capability_host_blocks_for_approval_without_dispatch_or_reservation() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let authorizer = ApprovalAuthorizer;
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "needs approval"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationRequiresApproval { .. }
    ));
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::BlockedApproval);
    let approval_request_id = record.approval_request_id.unwrap();
    let approval = approval_requests
        .get(&scope, approval_request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.scope, scope);
    assert_eq!(approval.status, ApprovalStatus::Pending);
    assert_eq!(approval.request.id, approval_request_id);
    assert_eq!(
        approval.request.invocation_fingerprint,
        Some(
            InvocationFingerprint::for_dispatch(
                &scope,
                &CapabilityId::new("echo.say").unwrap(),
                &ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                &json!({"message": "needs approval"}),
            )
            .unwrap()
        )
    );
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn capability_host_rejects_mismatched_approval_fingerprint_without_persisting_approval() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let authorizer = MismatchedFingerprintAuthorizer;
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "real input"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::InternalInvariantViolation,
            ..
        }
    ));
    assert_eq!(
        approval_requests.records_for_scope(&scope).await.unwrap(),
        Vec::new()
    );
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(
        record.error_kind.as_deref(),
        Some("InvocationFingerprintMismatch")
    );
}

#[tokio::test]
async fn capability_host_resumes_approved_invocation_and_consumes_lease() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&wasm_runtime);
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        output_bytes: Some(10_000),
        ..ResourceEstimate::default()
    };
    let input = json!({"message": "approved"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();
    let blocked = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    let approval_request_id = blocked.approval_request_id.unwrap();
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();
    let lease_authorizer = LeaseBackedAuthorizer::new(&leases);
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &lease_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let result = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate,
            input,
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"message": "approved"}));
    assert_eq!(
        run_state
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Completed
    );
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Consumed
    );
}

#[tokio::test]
async fn capability_host_rejects_resume_with_unsupported_obligations_before_claim_or_dispatch() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&wasm_runtime);
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        output_bytes: Some(10_000),
        ..ResourceEstimate::default()
    };
    let input = json!({"message": "approved but unsupported obligations"});

    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: estimate.clone(),
            input: input.clone(),
        })
        .await
        .unwrap_err();
    let blocked = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    let approval_request_id = blocked.approval_request_id.unwrap();
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &ObligatingResumeAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate,
            input,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::UnsupportedObligations { .. }
    ));
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Active
    );
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.error_kind.as_deref(), Some("UnsupportedObligations"));
}

#[tokio::test]
async fn capability_host_rejects_resume_when_input_fingerprint_differs() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&wasm_runtime);
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let estimate = ResourceEstimate::default();
    let approved_input = json!({"message": "approved"});
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: estimate.clone(),
            input: approved_input,
        })
        .await
        .unwrap_err();
    let approval_request_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();
    let lease_authorizer = LeaseBackedAuthorizer::new(&leases);
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &lease_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate,
            input: json!({"message": "tampered"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalFingerprintMismatch { .. }
    ));
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Active
    );
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(
        record.error_kind.as_deref(),
        Some("InvocationFingerprintMismatch")
    );
}

#[tokio::test]
async fn capability_host_rejects_resume_when_approval_was_denied() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&wasm_runtime);
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "denied"}),
        })
        .await
        .unwrap_err();
    let approval_request_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    approval_requests
        .deny(&scope, approval_request_id)
        .await
        .unwrap();
    let lease_authorizer = LeaseBackedAuthorizer::new(&leases);
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &lease_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "denied"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalNotApproved {
            status: ApprovalStatus::Denied,
            ..
        }
    ));
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.error_kind.as_deref(), Some("ApprovalDenied"));
}

#[tokio::test]
async fn capability_host_rejects_resume_when_no_matching_lease_exists() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&wasm_runtime);
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "approved-no-lease"}),
        })
        .await
        .unwrap_err();
    let approval_request_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    approval_requests
        .approve(&scope, approval_request_id)
        .await
        .unwrap();
    let lease_authorizer = LeaseBackedAuthorizer::new(&leases);
    let resume_host = CapabilityHost::new(&registry, &dispatcher, &lease_authorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests)
        .with_capability_leases(&leases);

    let err = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "approved-no-lease"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalLeaseMissing { .. }
    ));
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.error_kind.as_deref(), Some("ApprovalLeaseMissing"));
}

#[tokio::test]
async fn capability_host_does_not_allow_approval_lease_through_plain_invoke_json() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&wasm_runtime);
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let input = json!({"message": "approved"});
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let block_host = CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer)
        .with_run_state(&run_state)
        .with_approval_requests(&approval_requests);
    block_host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: capability_id.clone(),
            estimate: ResourceEstimate::default(),
            input: input.clone(),
        })
        .await
        .unwrap_err();
    let approval_request_id = run_state
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap()
        .approval_request_id
        .unwrap();
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval_request_id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();
    let lease_authorizer = LeaseBackedAuthorizer::new(&leases);
    let plain_host = CapabilityHost::new(&registry, &dispatcher, &lease_authorizer);

    let err = plain_host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id,
            estimate: ResourceEstimate::default(),
            input,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::AuthorizationDenied {
            reason: DenyReason::MissingGrant,
            ..
        }
    ));
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Active
    );
}

#[tokio::test]
async fn capability_host_fails_approval_required_invocation_when_approval_store_is_missing() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let run_state = InMemoryRunStateStore::new();
    let host =
        CapabilityHost::new(&registry, &dispatcher, &ApprovalAuthorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({"message": "needs approval"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityInvocationError::ApprovalStoreMissing { .. }
    ));
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.error_kind.as_deref(), Some("ApprovalStoreMissing"));
    assert_eq!(record.approval_request_id, None);
}

#[tokio::test]
async fn capability_host_claims_approval_lease_before_resume_dispatch() {
    let (_fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let run_state = InMemoryRunStateStore::new();
    let approval_requests = InMemoryApprovalRequestStore::new();
    let leases = InMemoryCapabilityLeaseStore::new();
    let context = execution_context(CapabilitySet::default());
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let estimate = ResourceEstimate::default();
    let input = json!({"message": "approved"});
    run_state
        .start(RunStart {
            invocation_id,
            capability_id: capability_id.clone(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    let approval = ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: context.correlation_id,
        requested_by: Principal::Extension(context.extension_id.clone()),
        action: Box::new(Action::Dispatch {
            capability: capability_id.clone(),
            estimated_resources: estimate.clone(),
        }),
        invocation_fingerprint: Some(
            InvocationFingerprint::for_dispatch(&scope, &capability_id, &estimate, &input).unwrap(),
        ),
        reason: "test approval".to_string(),
        reusable_scope: None,
    };
    approval_requests
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();
    run_state
        .block_approval(&scope, invocation_id, approval.clone())
        .await
        .unwrap();
    let lease = ApprovalResolver::new(&approval_requests, &leases)
        .approve_dispatch(
            &scope,
            approval.id,
            LeaseApproval {
                issued_by: Principal::User(scope.user_id.clone()),
                allowed_effects: vec![EffectKind::DispatchCapability],
                expires_at: None,
                max_invocations: Some(1),
            },
        )
        .await
        .unwrap();
    let claim_asserting_dispatcher = ClaimStatusAssertingDispatcher {
        leases: &leases,
        scope: scope.clone(),
        lease_id: lease.grant.id,
    };
    let lease_authorizer = LeaseBackedAuthorizer::new(&leases);
    let resume_host =
        CapabilityHost::new(&registry, &claim_asserting_dispatcher, &lease_authorizer)
            .with_run_state(&run_state)
            .with_approval_requests(&approval_requests)
            .with_capability_leases(&leases);

    let result = resume_host
        .resume_json(CapabilityResumeRequest {
            context,
            approval_request_id: approval.id,
            capability_id,
            estimate,
            input,
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"message": "approved"}));
    assert_eq!(
        leases.get(&scope, lease.grant.id).await.unwrap().status,
        CapabilityLeaseStatus::Consumed
    );
}

#[tokio::test]
async fn capability_host_records_completed_run_after_authorized_dispatch() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&wasm_runtime);
    let authorizer = GrantAuthorizer::new();
    let run_state = InMemoryRunStateStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability],
        )],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let result = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "ok"}),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"message": "ok"}));
    assert_eq!(
        run_state
            .get(&scope, invocation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn capability_host_records_failed_run_after_dispatch_error() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let governor = InMemoryResourceGovernor::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let authorizer = GrantAuthorizer::new();
    let run_state = InMemoryRunStateStore::new();
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer).with_run_state(&run_state);
    let context = execution_context(CapabilitySet {
        grants: vec![grant_for(
            CapabilityId::new("echo.say").unwrap(),
            Principal::Extension(ExtensionId::new("caller").unwrap()),
            vec![EffectKind::DispatchCapability],
        )],
    });
    let scope = context.resource_scope.clone();
    let invocation_id = context.invocation_id;

    let err = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "missing runtime"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, CapabilityInvocationError::Dispatch { .. }));
    let record = run_state.get(&scope, invocation_id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.error_kind.as_deref(), Some("Dispatch"));
}

struct ClaimStatusAssertingDispatcher<'a> {
    leases: &'a InMemoryCapabilityLeaseStore,
    scope: ResourceScope,
    lease_id: CapabilityGrantId,
}

#[async_trait]
impl CapabilityDispatcher for ClaimStatusAssertingDispatcher<'_> {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        assert_eq!(
            self.leases
                .get(&self.scope, self.lease_id)
                .await
                .unwrap()
                .status,
            CapabilityLeaseStatus::Claimed
        );
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: ExtensionId::new("echo").unwrap(),
            runtime: RuntimeKind::Wasm,
            output: request.input,
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

struct ApprovalAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for ApprovalAuthorizer {
    async fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: descriptor.id.clone(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: None,
                reason: "test approval".to_string(),
                reusable_scope: None,
            },
        }
    }
}

struct ObligatingResumeAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for ObligatingResumeAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::AuditBefore],
        }
    }
}

struct MismatchedFingerprintAuthorizer;

#[async_trait]
impl CapabilityDispatchAuthorizer for MismatchedFingerprintAuthorizer {
    async fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
    ) -> Decision {
        let mut other_scope = context.resource_scope.clone();
        other_scope.invocation_id = InvocationId::new();
        Decision::RequireApproval {
            request: ApprovalRequest {
                id: ApprovalRequestId::new(),
                correlation_id: context.correlation_id,
                requested_by: Principal::Extension(context.extension_id.clone()),
                action: Box::new(Action::Dispatch {
                    capability: descriptor.id.clone(),
                    estimated_resources: estimate.clone(),
                }),
                invocation_fingerprint: Some(
                    InvocationFingerprint::for_dispatch(
                        &other_scope,
                        &descriptor.id,
                        estimate,
                        &json!({"message": "different input"}),
                    )
                    .unwrap(),
                ),
                reason: "mismatched fingerprint".to_string(),
                reusable_scope: None,
            },
        }
    }
}

fn wasm_package_with_module(bytes: Vec<u8>) -> (LocalFilesystem, ExtensionPackage) {
    let storage = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(storage.join("echo/wasm")).unwrap();
    std::fs::write(storage.join("echo/wasm/echo.wasm"), bytes).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    (fs, package_from_manifest(WASM_MANIFEST))
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn json_echo_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (global $heap (mut i32) (i32.const 1024))
            (global $out_ptr (mut i32) (i32.const 0))
            (global $out_len (mut i32) (i32.const 0))
            (func (export "alloc") (param $len i32) (result i32)
              (local $ptr i32)
              global.get $heap
              local.set $ptr
              global.get $heap
              local.get $len
              i32.add
              global.set $heap
              local.get $ptr)
            (func (export "say") (param $ptr i32) (param $len i32) (result i32)
              local.get $ptr
              global.set $out_ptr
              local.get $len
              global.set $out_len
              i32.const 0)
            (func (export "output_ptr") (result i32)
              global.get $out_ptr)
            (func (export "output_len") (result i32)
              global.get $out_len))"#,
    )
    .unwrap()
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

const WASM_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
