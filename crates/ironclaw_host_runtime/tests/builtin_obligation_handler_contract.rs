use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_capabilities::{
    CapabilityObligationCompletionRequest, CapabilityObligationError,
    CapabilityObligationFailureKind, CapabilityObligationHandler, CapabilityObligationPhase,
    CapabilityObligationRequest,
};
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_host_api::*;
use ironclaw_host_runtime::{
    BuiltinObligationHandler, CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntime,
    NetworkObligationPolicyStore, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeFailureKind, RuntimeSecretInjectionStore,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use secrecy::ExposeSecret;
use serde_json::json;

#[tokio::test]
async fn builtin_obligation_handler_emits_metadata_only_audit_before() {
    let audit = Arc::new(InMemoryAuditSink::new());
    let handler = BuiltinObligationHandler::new().with_audit_sink(audit.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::AuditBefore];

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

    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].stage, AuditStage::Before);
    assert_eq!(records[0].tenant_id, context.tenant_id);
    assert_eq!(records[0].user_id, context.user_id);
    assert_eq!(records[0].invocation_id, context.invocation_id);
    assert_eq!(records[0].action.kind, "capability_invoke");
    assert_eq!(records[0].action.target.as_deref(), Some("echo.say"));
    assert_eq!(records[0].decision.kind, "obligation_satisfied");
    assert_eq!(
        records[0]
            .result
            .as_ref()
            .and_then(|result| result.status.as_deref()),
        Some("audit_before")
    );
    let serialized = serde_json::to_string(&records[0]).unwrap();
    assert!(!serialized.contains("raw input"));
    assert!(!serialized.contains("secret"));
}

#[tokio::test]
async fn builtin_obligation_handler_satisfy_fails_closed_on_post_dispatch_obligations() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];

    let err = handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityObligationError::Unsupported { obligations } if obligations == vec![Obligation::RedactOutput]
    ));
}

#[tokio::test]
async fn builtin_obligation_handler_enforces_output_limit_after_dispatch() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::EnforceOutputLimit { bytes: 8 }];
    let dispatch = sample_dispatch(
        &context.resource_scope,
        &capability_id,
        json!({"message": "this output is too large"}),
    );

    let err = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Output
        }
    ));
}

#[tokio::test]
async fn builtin_obligation_handler_redacts_output_after_dispatch() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];
    let leaked = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let dispatch = sample_dispatch(
        &context.resource_scope,
        &capability_id,
        json!({"authorization": leaked}),
    );

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
        .unwrap();

    let serialized = serde_json::to_string(&completed.output).unwrap();
    assert!(serialized.contains("[REDACTED]"));
    assert!(!serialized.contains(leaked));
}

#[tokio::test]
async fn builtin_obligation_handler_redacts_secret_like_object_keys() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];
    let leaked = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let dispatch = sample_dispatch(
        &context.resource_scope,
        &capability_id,
        json!({ leaked: "value" }),
    );

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
        .unwrap();

    let serialized = serde_json::to_string(&completed.output).unwrap();
    assert!(serialized.contains("[REDACTED]"));
    assert!(!serialized.contains(leaked));
}

#[tokio::test]
async fn builtin_obligation_handler_fails_closed_when_redacted_object_keys_collide() {
    let handler = BuiltinObligationHandler::new();
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::RedactOutput];
    let leaked = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let dispatch = sample_dispatch(
        &context.resource_scope,
        &capability_id,
        json!({
            leaked: "secret-key",
            "[REDACTED].aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb": "existing",
        }),
    );

    let err = handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        CapabilityObligationError::Failed {
            kind: CapabilityObligationFailureKind::Output
        }
    ));
}

#[tokio::test]
async fn builtin_obligation_handler_stores_network_policy_for_runtime_handoff() {
    let policy_store = Arc::new(NetworkObligationPolicyStore::new());
    let handler = BuiltinObligationHandler::new().with_network_policy_store(policy_store.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::ApplyNetworkPolicy {
        policy: allowed_network_policy(),
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
        .unwrap();

    assert!(
        policy_store
            .take(&context.resource_scope, &capability_id)
            .is_some(),
        "accepted network obligations must be handed to runtime adapters"
    );
}

#[tokio::test]
async fn builtin_obligation_handler_removes_network_policy_on_abort() {
    let policy_store = Arc::new(NetworkObligationPolicyStore::new());
    let handler = BuiltinObligationHandler::new().with_network_policy_store(policy_store.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::ApplyNetworkPolicy {
        policy: allowed_network_policy(),
    }];

    let outcome = handler
        .prepare(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap();
    handler
        .abort(ironclaw_capabilities::CapabilityObligationAbortRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            outcome: &outcome,
        })
        .await
        .unwrap();

    assert!(
        policy_store
            .take(&context.resource_scope, &capability_id)
            .is_none()
    );
}

#[test]
fn network_obligation_policy_store_isolates_agent_scope() {
    let store = NetworkObligationPolicyStore::new();
    let capability_id = capability_id();
    let mut agent_a = execution_context(CapabilitySet::default()).resource_scope;
    agent_a.agent_id = Some(AgentId::new("agent-a").unwrap());
    let mut agent_b = agent_a.clone();
    agent_b.agent_id = Some(AgentId::new("agent-b").unwrap());

    store.insert(&agent_a, &capability_id, allowed_network_policy());

    assert!(store.take(&agent_b, &capability_id).is_none());
    assert!(store.take(&agent_a, &capability_id).is_some());
}

#[tokio::test]
async fn builtin_obligation_handler_leases_consumes_and_stages_secret_once() {
    let secret_store = Arc::new(InMemorySecretStore::new());
    let injection_store = Arc::new(RuntimeSecretInjectionStore::new());
    let handler = BuiltinObligationHandler::new()
        .with_secret_store(secret_store.clone())
        .with_secret_injection_store(injection_store.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let handle = SecretHandle::new("api_token").unwrap();
    secret_store
        .put(
            context.resource_scope.clone(),
            handle.clone(),
            SecretMaterial::from("runtime-secret"),
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
        .unwrap();

    let material = injection_store
        .take(&context.resource_scope, &capability_id, &handle)
        .unwrap()
        .expect("secret material should be staged exactly once");
    assert_eq!(material.expose_secret(), "runtime-secret");
    assert!(
        injection_store
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_none(),
        "runtime secret injection store must be one-shot"
    );
}

#[tokio::test]
async fn builtin_obligation_handler_removes_staged_secret_on_abort() {
    let secret_store = Arc::new(InMemorySecretStore::new());
    let injection_store = Arc::new(RuntimeSecretInjectionStore::new());
    let handler = BuiltinObligationHandler::new()
        .with_secret_store(secret_store.clone())
        .with_secret_injection_store(injection_store.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let handle = SecretHandle::new("api_token").unwrap();
    secret_store
        .put(
            context.resource_scope.clone(),
            handle.clone(),
            SecretMaterial::from("runtime-secret"),
        )
        .await
        .unwrap();
    let obligations = vec![Obligation::InjectSecretOnce {
        handle: handle.clone(),
    }];

    let outcome = handler
        .prepare(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap();
    assert!(
        injection_store
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_some()
    );
    injection_store
        .insert(
            &context.resource_scope,
            &capability_id,
            &handle,
            SecretMaterial::from("runtime-secret"),
        )
        .unwrap();

    handler
        .abort(ironclaw_capabilities::CapabilityObligationAbortRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            outcome: &outcome,
        })
        .await
        .unwrap();

    assert!(
        injection_store
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn builtin_obligation_handler_satisfy_preserves_staged_handoffs_when_releasing_reservation() {
    let policy_store = Arc::new(NetworkObligationPolicyStore::new());
    let secret_store = Arc::new(InMemorySecretStore::new());
    let injection_store = Arc::new(RuntimeSecretInjectionStore::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let handler = BuiltinObligationHandler::new()
        .with_network_policy_store(policy_store.clone())
        .with_secret_store(secret_store.clone())
        .with_secret_injection_store(injection_store.clone())
        .with_resource_governor(governor.clone());
    let context = execution_context(CapabilitySet::default());
    let account = ResourceAccount::tenant(context.resource_scope.tenant_id.clone());
    let capability_id = capability_id();
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    let handle = SecretHandle::new("api_token").unwrap();
    secret_store
        .put(
            context.resource_scope.clone(),
            handle.clone(),
            SecretMaterial::from("runtime-secret"),
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
        policy_store
            .take(&context.resource_scope, &capability_id)
            .is_some(),
        "direct satisfy should preserve staged network handoff after releasing reservation"
    );
    assert!(
        injection_store
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_some(),
        "direct satisfy should preserve staged secret handoff after releasing reservation"
    );
}

#[tokio::test]
async fn builtin_obligation_handler_cleans_unused_staged_handoffs_after_dispatch_completion() {
    let policy_store = Arc::new(NetworkObligationPolicyStore::new());
    let secret_store = Arc::new(InMemorySecretStore::new());
    let injection_store = Arc::new(RuntimeSecretInjectionStore::new());
    let handler = BuiltinObligationHandler::new()
        .with_network_policy_store(policy_store.clone())
        .with_secret_store(secret_store.clone())
        .with_secret_injection_store(injection_store.clone());
    let context = execution_context(CapabilitySet::default());
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let handle = SecretHandle::new("api_token").unwrap();
    secret_store
        .put(
            context.resource_scope.clone(),
            handle.clone(),
            SecretMaterial::from("runtime-secret"),
        )
        .await
        .unwrap();
    let policy = allowed_network_policy();
    let obligations = vec![
        Obligation::ApplyNetworkPolicy {
            policy: policy.clone(),
        },
        Obligation::InjectSecretOnce {
            handle: handle.clone(),
        },
    ];

    handler
        .prepare(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap();
    assert!(
        policy_store
            .take(&context.resource_scope, &capability_id)
            .is_some()
    );
    assert!(
        injection_store
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_some()
    );
    policy_store.insert(&context.resource_scope, &capability_id, policy);
    injection_store
        .insert(
            &context.resource_scope,
            &capability_id,
            &handle,
            SecretMaterial::from("runtime-secret"),
        )
        .unwrap();

    handler
        .complete_dispatch(CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &sample_dispatch(
                &context.resource_scope,
                &capability_id,
                json!({"ok": true}),
            ),
        })
        .await
        .unwrap();

    assert!(
        policy_store
            .take(&context.resource_scope, &capability_id)
            .is_none()
    );
    assert!(
        injection_store
            .take(&context.resource_scope, &capability_id, &handle)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn builtin_obligation_handler_returns_scoped_mount_outcome_when_subset() {
    let handler = BuiltinObligationHandler::new();
    let mut context = execution_context(CapabilitySet::default());
    context.mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_write(),
    );
    let scoped_mounts = mount_view(
        "/workspace",
        "/projects/demo",
        MountPermissions::read_only(),
    );
    let capability_id = capability_id();
    let estimate = ResourceEstimate::default();
    let obligations = vec![Obligation::UseScopedMounts {
        mounts: scoped_mounts.clone(),
    }];

    let outcome = handler
        .prepare(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap();

    assert_eq!(outcome.mounts, Some(scoped_mounts));
}

#[tokio::test]
async fn builtin_obligation_handler_reserves_requested_resources_and_releases_on_abort() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let handler = BuiltinObligationHandler::new().with_resource_governor(governor.clone());
    let context = execution_context(CapabilitySet::default());
    let account = ResourceAccount::tenant(context.resource_scope.tenant_id.clone());
    let capability_id = capability_id();
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    let reservation_id = ResourceReservationId::new();
    let obligations = vec![Obligation::ReserveResources { reservation_id }];

    let outcome = handler
        .prepare(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
        })
        .await
        .unwrap();

    assert_eq!(
        outcome
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id),
        Some(reservation_id)
    );
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);

    handler
        .abort(ironclaw_capabilities::CapabilityObligationAbortRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            outcome: &outcome,
        })
        .await
        .unwrap();

    assert_eq!(governor.reserved_for(&account).concurrency_slots, 0);
}

#[tokio::test]
async fn default_host_runtime_fails_closed_on_resource_ceiling_until_handoff_exists() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(PanicDispatcher);
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::EnforceResourceCeiling {
                ceiling: resource_ceiling(),
            },
        ]));
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_builtin_obligation_handler();

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context(CapabilitySet::default()),
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "must not dispatch"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.kind, RuntimeFailureKind::Authorization);
        }
        other => panic!("expected failed outcome, got {other:?}"),
    }
}

#[tokio::test]
async fn default_host_runtime_installs_configured_obligation_handler() {
    let registry = Arc::new(registry_with_echo_capability());
    let dispatcher = Arc::new(RecordingDispatcher);
    let authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer> =
        Arc::new(ObligatingAuthorizer::new(vec![Obligation::AuditBefore]));
    let audit = Arc::new(InMemoryAuditSink::new());
    let handler = Arc::new(BuiltinObligationHandler::new().with_audit_sink(audit.clone()));
    let runtime = DefaultHostRuntime::new(
        registry,
        dispatcher,
        authorizer,
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_obligation_handler(handler);

    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            execution_context(CapabilitySet::default()),
            capability_id(),
            ResourceEstimate::default(),
            json!({"message": "obligated"}),
            trust_decision(),
        ))
        .await
        .unwrap();

    assert!(matches!(outcome, RuntimeCapabilityOutcome::Completed(_)));
    let records = audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].action.kind, "capability_invoke");
}

struct ObligatingAuthorizer {
    obligations: Vec<Obligation>,
}

impl ObligatingAuthorizer {
    fn new(obligations: Vec<Obligation>) -> Self {
        Self { obligations }
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }
}

#[derive(Default)]
struct RecordingDispatcher;

struct PanicDispatcher;

#[async_trait]
impl CapabilityDispatcher for PanicDispatcher {
    async fn dispatch_json(
        &self,
        _request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        panic!("dispatcher must not be called for unsupported resource-ceiling obligations")
    }
}

#[async_trait]
impl CapabilityDispatcher for RecordingDispatcher {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<CapabilityDispatchResult, DispatchError> {
        Ok(CapabilityDispatchResult {
            capability_id: request.capability_id,
            provider: ExtensionId::new("echo").unwrap(),
            runtime: RuntimeKind::Wasm,
            output: json!({"ok": true}),
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: request
                    .resource_reservation
                    .as_ref()
                    .map(|reservation| reservation.id)
                    .unwrap_or_default(),
                scope: request.scope,
                status: ReservationStatus::Reconciled,
                estimate: request.estimate,
                actual: Some(ResourceUsage::default()),
            },
        })
    }
}

fn sample_dispatch(
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    output: serde_json::Value,
) -> CapabilityDispatchResult {
    CapabilityDispatchResult {
        capability_id: capability_id.clone(),
        provider: ExtensionId::new("echo").unwrap(),
        runtime: RuntimeKind::Wasm,
        output,
        usage: ResourceUsage::default(),
        receipt: ResourceReceipt {
            id: ResourceReservationId::new(),
            scope: scope.clone(),
            status: ReservationStatus::Reconciled,
            estimate: ResourceEstimate::default(),
            actual: Some(ResourceUsage::default()),
        },
    }
}

fn mount_view(alias: &str, target: &str, permissions: MountPermissions) -> MountView {
    MountView::new(vec![MountGrant::new(
        MountAlias::new(alias).unwrap(),
        VirtualPath::new(target).unwrap(),
        permissions,
    )])
    .unwrap()
}

fn resource_ceiling() -> ResourceCeiling {
    ResourceCeiling {
        max_usd: None,
        max_input_tokens: None,
        max_output_tokens: None,
        max_wall_clock_ms: Some(1),
        max_output_bytes: None,
        sandbox: None,
    }
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

fn registry_with_echo_capability() -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse(ECHO_MANIFEST).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
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

fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::sandbox(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}

const ECHO_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echoes input"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = {}
"#;
