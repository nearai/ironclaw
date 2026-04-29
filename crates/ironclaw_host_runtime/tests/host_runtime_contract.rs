use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    CancelReason, CancelRuntimeWorkRequest, CapabilitySurfaceKind, CapabilitySurfaceVersion,
    HostRuntime, HostRuntimeHealth, HostRuntimeStatus, IdempotencyKey, RuntimeAuthGate,
    RuntimeBlockedReason, RuntimeCaller, RuntimeCapabilityCompleted, RuntimeCapabilityFailure,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind, RuntimeGateId,
    RuntimeStatusRequest, RuntimeWorkId, RuntimeWorkSummary, VisibleCapabilityRequest,
    VisibleCapabilitySurface, testkit::FakeHostRuntime,
};
use serde_json::json;

#[tokio::test]
async fn fake_runtime_records_invocation_and_preserves_structured_outcomes() {
    let context = execution_context();
    let capability_id = CapabilityId::new("demo.echo").unwrap();
    let completed = RuntimeCapabilityOutcome::Completed(Box::new(RuntimeCapabilityCompleted {
        capability_id: capability_id.clone(),
        output: json!({"ok": true}),
        usage: ResourceUsage::default(),
    }));
    let runtime = FakeHostRuntime::new().with_outcome(completed.clone());

    let request = RuntimeCapabilityRequest {
        context: context.clone(),
        capability_id: capability_id.clone(),
        estimate: ResourceEstimate::default(),
        input: json!({"message": "hello"}),
        caller: RuntimeCaller::AgentLoopHost,
        idempotency_key: Some(IdempotencyKey::new("turn-1/tool-1").unwrap()),
    };

    let actual = runtime.invoke_capability(request.clone()).await.unwrap();

    assert_eq!(actual, completed);
    assert_eq!(runtime.recorded_invocations(), vec![request]);
}

#[tokio::test]
async fn fake_runtime_surfaces_approval_auth_and_resource_waits_as_values_not_errors() {
    let context = execution_context();
    let capability_id = CapabilityId::new("demo.email.send").unwrap();
    let auth_gate = RuntimeCapabilityOutcome::AuthRequired(RuntimeAuthGate {
        gate_id: RuntimeGateId::new(),
        capability_id: capability_id.clone(),
        reason: RuntimeBlockedReason::AuthRequired,
        required_secrets: vec![SecretHandle::new("google.oauth").unwrap()],
    });
    let runtime = FakeHostRuntime::new()
        .with_outcome(RuntimeCapabilityOutcome::ApprovalRequired(
            ironclaw_host_runtime::RuntimeApprovalGate {
                approval_request_id: ApprovalRequestId::new(),
                capability_id: capability_id.clone(),
                reason: RuntimeBlockedReason::ApprovalRequired,
            },
        ))
        .with_outcome(auth_gate.clone())
        .with_outcome(RuntimeCapabilityOutcome::ResourceBlocked(
            ironclaw_host_runtime::RuntimeResourceGate {
                gate_id: RuntimeGateId::new(),
                capability_id: capability_id.clone(),
                reason: RuntimeBlockedReason::ResourceLimit,
                estimate: ResourceEstimate {
                    output_tokens: Some(5000),
                    ..ResourceEstimate::default()
                },
            },
        ));

    for expected in ["approval_required", "auth_required", "resource_blocked"] {
        let outcome = runtime
            .invoke_capability(RuntimeCapabilityRequest {
                context: context.clone(),
                capability_id: capability_id.clone(),
                estimate: ResourceEstimate::default(),
                input: json!({}),
                caller: RuntimeCaller::TurnCoordinator,
                idempotency_key: None,
            })
            .await
            .unwrap();
        assert_eq!(outcome.kind(), expected);
    }

    assert_eq!(runtime.recorded_invocations().len(), 3);
}

#[tokio::test]
async fn fake_runtime_returns_versioned_visible_surface_and_records_requests() {
    let context = execution_context();
    let descriptor = CapabilityDescriptor {
        id: CapabilityId::new("demo.echo").unwrap(),
        provider: ExtensionId::new("demo").unwrap(),
        runtime: RuntimeKind::Script,
        trust_ceiling: TrustClass::FirstParty,
        description: "Echo input".to_string(),
        parameters_schema: json!({"type": "object"}),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Allow,
        resource_profile: None,
    };
    let surface = VisibleCapabilitySurface {
        version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        descriptors: vec![descriptor.clone()],
    };
    let runtime = FakeHostRuntime::new().with_visible_surface(surface.clone());
    let request = VisibleCapabilityRequest {
        scope: context.resource_scope.clone(),
        correlation_id: context.correlation_id,
        caller: RuntimeCaller::AgentLoopHost,
        surface_kind: CapabilitySurfaceKind::AgentLoop,
    };

    let actual = runtime.visible_capabilities(request.clone()).await.unwrap();

    assert_eq!(actual, surface);
    assert_eq!(
        runtime.recorded_visible_capability_requests(),
        vec![request]
    );
}

#[tokio::test]
async fn fake_runtime_records_cancellation_status_and_health_calls() {
    let context = execution_context();
    let work_id = RuntimeWorkId::Invocation(context.invocation_id);
    let status = HostRuntimeStatus {
        active_work: vec![RuntimeWorkSummary {
            work_id: work_id.clone(),
            capability_id: Some(CapabilityId::new("demo.echo").unwrap()),
            runtime: Some(RuntimeKind::Script),
        }],
    };
    let health = HostRuntimeHealth {
        ready: true,
        missing_runtime_backends: vec![RuntimeKind::Wasm],
    };
    let runtime = FakeHostRuntime::new()
        .with_cancelled_work(vec![work_id.clone()])
        .with_status(status.clone())
        .with_health(health.clone());
    let cancel = CancelRuntimeWorkRequest {
        scope: context.resource_scope.clone(),
        correlation_id: context.correlation_id,
        reason: CancelReason::UserRequested,
    };

    let cancelled = runtime.cancel_work(cancel.clone()).await.unwrap();
    let actual_status = runtime
        .runtime_status(RuntimeStatusRequest {
            scope: context.resource_scope.clone(),
            correlation_id: context.correlation_id,
        })
        .await
        .unwrap();
    let actual_health = runtime.health().await.unwrap();

    assert_eq!(cancelled.cancelled, vec![work_id]);
    assert_eq!(actual_status, status);
    assert_eq!(actual_health, health);
    assert_eq!(runtime.recorded_cancellations(), vec![cancel]);
}

#[tokio::test]
async fn fake_runtime_reports_sanitized_failures_as_outcomes() {
    let context = execution_context();
    let capability_id = CapabilityId::new("demo.fail").unwrap();
    let failure = RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
        capability_id: capability_id.clone(),
        kind: RuntimeFailureKind::Backend,
        message: Some("backend".to_string()),
    });
    let runtime = FakeHostRuntime::new().with_outcome(failure.clone());

    let actual = runtime
        .invoke_capability(RuntimeCapabilityRequest {
            context,
            capability_id,
            estimate: ResourceEstimate::default(),
            input: json!({}),
            caller: RuntimeCaller::SystemService,
            idempotency_key: None,
        })
        .await
        .unwrap();

    assert_eq!(actual, failure);
}

fn execution_context() -> ExecutionContext {
    let user_id = UserId::new("user1").unwrap();
    ExecutionContext::local_default(
        user_id,
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::Script,
        TrustClass::FirstParty,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap()
}
