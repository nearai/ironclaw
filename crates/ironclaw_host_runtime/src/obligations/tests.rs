//! Behavioural coverage for the three obligation owners.
//!
//! The suite is deliberately kept whole rather than partitioned by module: every
//! case here drives obligation handling *through* the staged-handoff stores and
//! the process-obligation store, which is the interaction the split must
//! preserve.

use std::{sync::Arc, time::Duration};

use ironclaw_capabilities::{
    CapabilityObligationCompletionRequest, CapabilityObligationError,
    CapabilityObligationFailureKind, CapabilityObligationHandler, CapabilityObligationPhase,
    CapabilityObligationRequest,
};
use ironclaw_events::InMemoryAuditSink;
use ironclaw_host_api::{
    action::{NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    capability::CapabilitySet,
    decision::Obligation,
    dispatch::{CapabilityDispatchResult, CapabilityDisplayOutputPreview},
    ids::{
        AgentId, CapabilityId, CorrelationId, ExtensionId, InvocationId, ProjectId,
        ResourceReservationId, SecretHandle, TenantId, UserId,
    },
    mount::MountView,
    resource::{ResourceEstimate, ResourceScope},
    runtime::{RuntimeKind, TrustClass},
    scope::ExecutionContext,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount};
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStorePort};

use super::*;

#[tokio::test]
async fn runtime_secret_injection_store_prunes_expired_handoffs() {
    let store = RuntimeSecretInjectionStore::with_ttl(Duration::from_millis(5));
    let scope = resource_scope_with_agent("agent-a");
    let capability_id = capability_id();
    let handle = SecretHandle::new("api_token").unwrap();

    store
        .insert(
            &scope,
            &capability_id,
            &handle,
            SecretMaterial::from("runtime-secret"),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(store.prune_expired().unwrap(), 1);
    assert!(
        store
            .take(&scope, &capability_id, &handle)
            .unwrap()
            .is_none()
    );
}

#[test]
fn network_obligation_policy_store_isolates_agent_scope() {
    let store = NetworkObligationPolicyStore::new();
    let (agent_a, agent_b) = same_invocation_agent_scopes();
    let capability_id = capability_id();

    store.insert(&agent_a, &capability_id, allowed_network_policy());

    assert!(store.take(&agent_b, &capability_id).is_none());
    assert!(store.take(&agent_a, &capability_id).is_some());
}

#[test]
fn runtime_secret_injection_store_isolates_agent_scope() {
    let store = RuntimeSecretInjectionStore::new();
    let (agent_a, agent_b) = same_invocation_agent_scopes();
    let capability_id = capability_id();
    let handle = SecretHandle::new("api_token").unwrap();

    store
        .insert(
            &agent_a,
            &capability_id,
            &handle,
            SecretMaterial::from("runtime-secret"),
        )
        .unwrap();

    assert!(
        store
            .take(&agent_b, &capability_id, &handle)
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .take(&agent_a, &capability_id, &handle)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn builtin_obligation_handler_satisfy_release_preserves_staged_handoffs() {
    let network_policies = Arc::new(NetworkObligationPolicyStore::new());
    let secret_injections = Arc::new(RuntimeSecretInjectionStore::new());
    let secret_store = Arc::new(SecretStore::ephemeral());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let services = BuiltinObligationServices::with_handoff_stores(
        Arc::new(InMemoryAuditSink::new()),
        network_policies.clone(),
        secret_store.clone(),
        secret_injections.clone(),
        governor.clone(),
    );
    let handler = services.obligation_handler();
    let context = execution_context();
    let account = ResourceAccount::tenant(context.resource_scope.tenant_id.clone());
    let capability_id = capability_id();
    let handle = SecretHandle::new("api_token").unwrap();
    let estimate = ResourceEstimate::default().set_concurrency_slots(1);
    secret_store
        .put(
            context.resource_scope.clone(),
            handle.clone(),
            SecretMaterial::from("runtime-secret"),
            None,
        )
        .await
        .unwrap();
    let obligations = vec![
        Obligation::ApplyNetworkPolicy {
            policy: allowed_network_policy(),
        },
        Obligation::InjectSecretOnce {
            handle: handle.clone(),
        },
        Obligation::ReserveResources {
            reservation_id: ResourceReservationId::new(),
        },
    ];

    handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap();

    assert_eq!(governor.reserved_for(&account).concurrency_slots, 0);
    assert!(
        network_policies
            .take(&context.resource_scope, &capability_id)
            .is_some()
    );
    assert!(
        secret_injections
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_some()
    );
}

// #5459 tenant-shared credential resolution: a caller's own secret wins;
// otherwise the tenant-shared admin-managed scope; otherwise absent.
#[tokio::test]
async fn secret_owner_scope_prefers_caller_then_tenant_shared_then_none() {
    let handle = SecretHandle::new("market_data_api_key").unwrap();
    let caller = execution_context().resource_scope;
    let shared = caller.tenant_shared_managed_scope();

    // Absent in both scopes -> None (dispatch then gates with AuthRequired).
    let store = SecretStore::ephemeral();
    assert_eq!(
        secret_owner_scope(&store, &caller, &handle).await.unwrap(),
        None,
    );

    // Present ONLY at the tenant-shared admin-managed scope -> resolves there,
    // so one admin-set key satisfies a caller who never provisioned it.
    let store = SecretStore::ephemeral();
    store
        .put(
            shared.clone(),
            handle.clone(),
            SecretMaterial::from("shared-admin-key"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        secret_owner_scope(&store, &caller, &handle)
            .await
            .unwrap()
            .as_ref(),
        Some(&shared),
    );

    // Present at BOTH scopes -> the caller's OWN secret wins over the shared one.
    let store = SecretStore::ephemeral();
    store
        .put(
            caller.clone(),
            handle.clone(),
            SecretMaterial::from("caller-own-key"),
            None,
        )
        .await
        .unwrap();
    store
        .put(
            shared.clone(),
            handle.clone(),
            SecretMaterial::from("shared-admin-key"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        secret_owner_scope(&store, &caller, &handle)
            .await
            .unwrap()
            .as_ref(),
        Some(&caller),
    );
}

// Through the caller: InjectSecretOnce is satisfied by an admin-set
// tenant-shared key even when the caller has no personal secret, and the
// material is staged at the caller's own invocation slot (#5459).
#[tokio::test]
async fn inject_secret_once_falls_back_to_tenant_shared_admin_key() {
    let secret_store = Arc::new(SecretStore::ephemeral());
    let secret_injections = Arc::new(RuntimeSecretInjectionStore::new());
    let services = BuiltinObligationServices::with_handoff_stores(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(NetworkObligationPolicyStore::new()),
        secret_store.clone(),
        secret_injections.clone(),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let handler = services.obligation_handler();
    let context = execution_context();
    let capability_id = capability_id();
    let handle = SecretHandle::new("market_data_api_key").unwrap();
    let estimate = ResourceEstimate::default();

    // Admin set the key ONLY at the tenant-shared scope; the caller has none.
    secret_store
        .put(
            context.resource_scope.tenant_shared_managed_scope(),
            handle.clone(),
            SecretMaterial::from("shared-admin-key"),
            None,
        )
        .await
        .unwrap();

    let obligations = vec![Obligation::InjectSecretOnce {
        handle: handle.clone(),
    }];
    handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .expect(
            "tenant-shared key satisfies InjectSecretOnce for a caller with no personal secret",
        );

    assert!(
        secret_injections
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_some(),
        "shared-sourced secret must be staged at the caller's own invocation slot",
    );
}

#[tokio::test]
async fn redact_output_clears_display_preview_side_channel() {
    use ironclaw_host_api::{
        resource::{ReservationStatus, ResourceReceipt, ResourceUsage},
        runtime::RuntimeKind,
    };

    let services = BuiltinObligationServices::with_handoff_stores(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(NetworkObligationPolicyStore::new()),
        Arc::new(SecretStore::ephemeral()),
        Arc::new(RuntimeSecretInjectionStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let handler = services.obligation_handler();
    let context = execution_context();
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];
    let dispatch = CapabilityDispatchResult {
        capability_id: capability_id.clone(),
        provider: context.extension_id.clone(),
        runtime: RuntimeKind::Wasm,
        output: serde_json::json!({"secret": "sk-secret", "safe": "ok"}),
        display_preview: Some(CapabilityDisplayOutputPreview {
            output_summary: Some("contains secret".to_string()),
            output_preview: "sk-secret".to_string(),
            output_kind: "text".to_string(),
            subtitle: None,
            truncated: false,
        }),
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: context.resource_scope.clone(),
            status: ReservationStatus::Released,
            estimate: ResourceEstimate::default(),
            actual: None,
        },
    };

    let completed = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        })
        .await
        .expect("redacted dispatch completes");

    assert!(completed.display_preview.is_none());
    assert_eq!(completed.output["safe"], serde_json::json!("ok"));
}

#[tokio::test]
async fn complete_dispatch_extracts_base64_document_into_text() {
    use base64::Engine as _;
    use ironclaw_host_api::{
        resource::{ReservationStatus, ResourceReceipt, ResourceUsage},
        runtime::RuntimeKind,
    };

    // Drive the *caller* (`complete_dispatch`), not the helper: a dispatch
    // result carrying `content_base64` + `mime_type` must come back with the
    // extracted text in `content` and no base64 left for the model to see.
    let services = BuiltinObligationServices::with_handoff_stores(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(NetworkObligationPolicyStore::new()),
        Arc::new(SecretStore::ephemeral()),
        Arc::new(RuntimeSecretInjectionStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let handler = services.obligation_handler();
    let context = execution_context();
    // The document-extraction transform is capability-gated, so this test
    // must dispatch a capability that opts in (`google-drive.download_file`)
    // — the shared `echo.say` helper id would pass through untouched.
    let capability_id = CapabilityId::new("google-drive.download_file").unwrap();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];
    let encoded = base64::engine::general_purpose::STANDARD.encode(b"name,age\nAlice,30");
    let dispatch = CapabilityDispatchResult {
        capability_id: capability_id.clone(),
        provider: context.extension_id.clone(),
        runtime: RuntimeKind::Wasm,
        output: serde_json::json!({
            "file_id": "f1",
            "name": "data.csv",
            "mime_type": "text/csv",
            "content_base64": encoded,
        }),
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: context.resource_scope.clone(),
            status: ReservationStatus::Released,
            estimate: ResourceEstimate::default(),
            actual: None,
        },
    };

    let completed = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        })
        .await
        .expect("base64 document dispatch completes");

    assert_eq!(
        completed.output["content"],
        serde_json::json!("name,age\nAlice,30")
    );
    assert!(
        completed.output.get("content_base64").is_none(),
        "base64 must be stripped before the result reaches the model"
    );
}

#[tokio::test]
async fn leak_detector_block_records_security_audit_event_through_complete_dispatch() {
    use ironclaw_events::{
        InMemorySecurityAuditSink, SecurityAuditSink, SecurityBoundary, SecurityDecision,
    };
    use ironclaw_host_api::{
        resource::{ReservationStatus, ResourceReceipt, ResourceUsage},
        runtime::RuntimeKind,
    };

    // Build a handler with both an audit sink (unused here — we hit the
    // redact branch, not the AuditAfter branch) and a recording
    // security-audit sink. Other backing stores are not exercised by
    // the redact-only path, but the handler requires them to be set
    // for safety; we install minimal in-memory ones.
    let security_sink: Arc<InMemorySecurityAuditSink> = Arc::new(InMemorySecurityAuditSink::new());
    let security_sink_dyn: Arc<dyn SecurityAuditSink> = security_sink.clone();

    let services = BuiltinObligationServices::with_handoff_stores(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(NetworkObligationPolicyStore::new()),
        Arc::new(SecretStore::ephemeral()),
        Arc::new(RuntimeSecretInjectionStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let handler = services
        .obligation_handler()
        .with_security_audit_sink(security_sink_dyn);

    let context = execution_context();
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];

    // An AWS access-key shaped string is a built-in BLOCK pattern in
    // `ironclaw_safety::LeakDetector` (`AKIA[0-9A-Z]{16}`). Per the
    // module invariant we drive the *caller* (`complete_dispatch`),
    // not the helper, and assert the recorded event:
    //   - boundary  == LeakDetector
    //   - decision  == Blocked
    //   - code      == LEAK_REDACT_FAILED_CODE
    //   - capability_id + scope are populated
    //   - no payload (the offending string never appears in the event)
    let leaky_payload = serde_json::Value::String("hello AKIAIOSFODNN7EXAMPLE goodbye".to_string());
    let dispatch = CapabilityDispatchResult {
        capability_id: capability_id.clone(),
        provider: context.extension_id.clone(),
        runtime: RuntimeKind::Wasm,
        output: leaky_payload,
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: context.resource_scope.clone(),
            status: ReservationStatus::Released,
            estimate: ResourceEstimate::default(),
            actual: None,
        },
    };

    let request = CapabilityObligationCompletionRequest {
        phase: CapabilityObligationPhase::Invoke,
        context: &context,
        capability_id: &capability_id,
        estimate: &estimate,
        obligations: &obligations,
        dispatch: &dispatch,
    };

    let result = handler.complete_dispatch(request).await;
    assert!(
        matches!(
            result,
            Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Output
            })
        ),
        "expected output-obligation failure, got {result:?}"
    );

    let events = security_sink.snapshot();
    assert_eq!(
        events.len(),
        1,
        "exactly one boundary decision should have been recorded, got {events:?}"
    );
    let event = &events[0];
    assert_eq!(event.boundary, SecurityBoundary::LeakDetector);
    assert_eq!(event.decision, SecurityDecision::Blocked);
    assert_eq!(event.code, LEAK_REDACT_FAILED_CODE);
    assert_eq!(event.code, "leak_redact_failed"); // stability lock
    assert_eq!(event.capability_id.as_ref(), Some(&capability_id));
    assert_eq!(event.scope.as_ref(), Some(&context.resource_scope));

    // The `SecurityAuditEvent` shape has no free-form payload field.
    // That invariant is enforced at the type level by the absence of
    // a `String` member on the struct. The check below is therefore a
    // documentation-only assertion: it locks the field set at the
    // value level for future readers, but the real guard is the type
    // shape in `ironclaw_events::security_audit`.
    //
    //   pub struct SecurityAuditEvent {
    //       pub boundary: SecurityBoundary,
    //       pub decision: SecurityDecision,
    //       pub capability_id: Option<CapabilityId>,
    //       pub scope: Option<ResourceScope>,
    //       pub timestamp: SystemTime,
    //       pub code: &'static str,
    //   }
}

#[tokio::test]
async fn leak_detector_block_without_security_sink_does_not_panic() {
    use ironclaw_host_api::{
        resource::{ReservationStatus, ResourceReceipt, ResourceUsage},
        runtime::RuntimeKind,
    };

    let services = BuiltinObligationServices::with_handoff_stores(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(NetworkObligationPolicyStore::new()),
        Arc::new(SecretStore::ephemeral()),
        Arc::new(RuntimeSecretInjectionStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    // No `.with_security_audit_sink(...)` — confirms the sink is
    // optional and the original failure semantics are preserved.
    let handler = services.obligation_handler();

    let context = execution_context();
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];
    let dispatch = CapabilityDispatchResult {
        capability_id: capability_id.clone(),
        provider: context.extension_id.clone(),
        runtime: RuntimeKind::Wasm,
        output: serde_json::Value::String("leak AKIAIOSFODNN7EXAMPLE".to_string()),
        display_preview: None,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: context.resource_scope.clone(),
            status: ReservationStatus::Released,
            estimate: ResourceEstimate::default(),
            actual: None,
        },
    };

    let result = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        })
        .await;
    assert!(matches!(
        result,
        Err(CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Output
        })
    ));
}

fn same_invocation_agent_scopes() -> (ResourceScope, ResourceScope) {
    let mut agent_a = resource_scope_with_agent("agent-a");
    agent_a.invocation_id = InvocationId::new();
    let mut agent_b = agent_a.clone();
    agent_b.agent_id = Some(AgentId::new("agent-b").unwrap());
    (agent_a, agent_b)
}

fn resource_scope_with_agent(agent_id: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: Some(AgentId::new(agent_id).unwrap()),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn execution_context() -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    ExecutionContext {
        run_id: None,
        origin: None,
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        authenticated_actor_user_id: None,
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope,
    }
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}

fn allowed_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    }
}
