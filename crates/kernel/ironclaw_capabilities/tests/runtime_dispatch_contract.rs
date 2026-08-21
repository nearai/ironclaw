//! `RuntimeDispatcher` contract at the resolver seam (TOOL-1/TOOL-2).
//!
//! Moved here from the deleted `ironclaw_dispatcher` compatibility shim (WS8):
//! the implementation under test has always been
//! `ironclaw_capabilities::dispatch`, so this suite now sits with the code it
//! pins instead of behind a re-export crate.
//!
//! The dispatcher resolves a prebound [`BoundCapabilityAdapter`] by capability
//! id through the injected [`ToolResolver`] and never selects a package or
//! runtime kind itself. Selection semantics (unknown provider, runtime
//! mismatch, missing backend) belong to the resolver implementations and are
//! pinned where they live: `ironclaw_host_runtime` for the registry-lane
//! resolver, `ironclaw_extension_host` for the active-snapshot resolver.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_capabilities::{
    BoundCapabilityAdapter, ChainToolResolver, ResolvedCapability, RuntimeAdapterResult,
    RuntimeDispatcher, ToolResolver,
};
use ironclaw_host_api::{
    artifact::{
        ARTIFACT_INLINE_PREVIEW_MAX_BYTES, AccountedArtifactPersister, ArtifactDigest, ArtifactId,
        ArtifactNamespaceId, ArtifactRef, ArtifactWriteError, ArtifactWriteMetadata,
        CompletedArtifact,
    },
    authorized::Authorized,
    dispatch::{
        CapabilityDispatchRequest, CapabilityDispatcher, DispatchError, DispatchFailureKind,
        RuntimeDispatchErrorKind,
    },
    ids::{
        ActivityId, AgentId, CapabilityId, CorrelationId, ExtensionId, InvocationId, MissionId,
        ProductKind, ProjectId, RunId, TenantId, ThreadId, UserId,
    },
    invocation::{Actor, Invocation, InvocationOrigin},
    lane::RuntimeLane,
    mount::MountView,
    resource::{
        ReservationStatus, ResourceEstimate, ResourceReceipt, ResourceReservation, ResourceScope,
        ResourceUsage,
    },
    runtime::RuntimeKind,
};
use ironclaw_loop_contracts::ContentDigest;
use ironclaw_resources::*;
use serde_json::{Value, json};

#[tokio::test]
async fn dispatcher_routes_capability_through_resolved_binding() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let artifact_namespace = ArtifactNamespaceId::from_root_run(RunId::new());
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();
    let binding = RecordingBinding::new(json!({"message": "hello adapter"}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::Wasm, binding.clone()),
    )]);

    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());
    let result = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            artifact_namespace: Some(artifact_namespace),
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "hello dispatcher"}),
        }))
        .await
        .unwrap();

    assert_eq!(result.capability_id, CapabilityId::new("echo.say").unwrap());
    assert_eq!(result.provider, ExtensionId::new("echo").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(result.output, json!({"message": "hello adapter"}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);

    let requests = binding.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].capability_id,
        CapabilityId::new("echo.say").unwrap()
    );
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].mounts, None);
    assert_eq!(requests[0].artifact_namespace, Some(artifact_namespace));
    assert_eq!(requests[0].input, json!({"message": "hello dispatcher"}));
}

#[tokio::test]
async fn runtime_result_artifact_dispatcher_persists_once_and_bounds_transport() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
    // Sized off the ceiling itself, not a literal: the fixture only proves the
    // bound fires if it actually exceeds it.
    let full_output = json!({"content": "x".repeat(2 * ARTIFACT_INLINE_PREVIEW_MAX_BYTES)});
    let canonical = serde_json::to_vec(&full_output).unwrap();
    let canonical_len = u64::try_from(canonical.len()).unwrap();
    let expected_content_digest = ContentDigest::from_json_value(&full_output).unwrap();
    let binding = RecordingBinding::new(full_output, Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "extension.large_result",
        resolved("extension", RuntimeKind::Wasm, binding),
    )]);
    let persister = Arc::new(RecordingArtifactPersister::default());
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref())
        .with_artifact_persistence_arc(persister.clone());

    let result = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            artifact_namespace: Some(namespace),
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("extension.large_result").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default().set_output_bytes(canonical_len),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        }))
        .await
        .unwrap();

    let completed = result.completed_artifact.expect("artifact finalized");
    assert_eq!(completed.byte_len, canonical_len);
    assert_eq!(completed.digest, ArtifactDigest::from_bytes(&canonical));
    assert_eq!(
        result
            .canonical_output_digest
            .expect("canonical output digest")
            .value(),
        expected_content_digest.0
    );
    let preview = result
        .output
        .as_str()
        .expect("transport is bounded preview");
    assert!(preview.len() <= ARTIFACT_INLINE_PREVIEW_MAX_BYTES);
    assert!(u64::try_from(preview.len()).unwrap() < canonical_len);
    assert_eq!(
        result
            .receipt
            .actual
            .as_ref()
            .map(|usage| usage.output_bytes),
        Some(canonical_len)
    );
    assert_eq!(governor.usage_for(&account).output_bytes, canonical_len);

    let calls = persister.calls();
    assert_eq!(calls.len(), 1, "canonical output is persisted exactly once");
    assert_eq!(calls[0].bytes, canonical);
    assert_eq!(calls[0].receipt, result.receipt);
    assert_eq!(calls[0].metadata.expected_bytes, Some(canonical_len));
    assert_eq!(calls[0].metadata.namespace, namespace);
}

#[tokio::test]
async fn agent_scoped_dispatch_without_artifact_persistence_never_invokes_adapter() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let binding = RecordingBinding::new(json!({"must_not_run": true}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "extension.requires_artifact",
        resolved("extension", RuntimeKind::Wasm, binding.clone()),
    )]);
    let mut scope = sample_scope();
    scope.agent_id = Some(AgentId::new("agent-a").unwrap());
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());

    let result = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            artifact_namespace: Some(ArtifactNamespaceId::from_root_run(RunId::new())),
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("extension.requires_artifact").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            input: json!({}),
        }))
        .await;

    assert!(result.is_err(), "missing persistence must fail closed");
    assert!(
        binding.requests().is_empty(),
        "persistence must be preflighted before adapter side effects"
    );
}

#[tokio::test]
async fn dispatcher_redacts_binding_failure_details() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let binding = RecordingBinding::failing(
        || DispatchError::Rejected {
            runtime: Some(RuntimeKind::Script),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::ExitFailure),
            diagnostic: None,
            detail: None,
        },
        Arc::clone(&governor),
    );
    let resolver = ScriptedResolver::from_entries([(
        "script.echo",
        resolved("script", RuntimeKind::Script, binding),
    )]);

    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());
    let err = dispatcher
        .dispatch_json(sample_request("script.echo", json!({"message": "boom"})))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Rejected {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::ExitFailure),
            ..
        }
    ));
    let message = err.to_string();
    assert!(!message.contains("secret token"));
    assert!(!message.contains("/tmp/private"));
}

#[tokio::test]
async fn dispatcher_fails_unknown_capability_before_any_binding_work() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let binding = RecordingBinding::new(json!({}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "known.say",
        resolved("known", RuntimeKind::Wasm, binding.clone()),
    )]);

    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());
    let err = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            artifact_namespace: None,
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("missing.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default().set_concurrency_slots(1),
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "nope"}),
        }))
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert!(binding.requests().is_empty());
}

#[tokio::test]
async fn dispatcher_releases_prepared_reservation_when_resolution_fails() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate::default().set_concurrency_slots(1);
    let reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);
    let resolver = ScriptedResolver::empty();

    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());
    let err = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            artifact_namespace: None,
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("missing.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({"message": "release on resolution failure"}),
        }))
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_hands_prepared_reservation_to_the_binding() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    let reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    let reservation_id = reservation.id;
    let binding = RecordingBinding::new(json!({"ok": true}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::Wasm, binding.clone()),
    )]);

    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());
    let result = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            artifact_namespace: None,
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({}),
        }))
        .await
        .unwrap();

    let requests = binding.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id),
        Some(reservation_id),
        "the prebound binding owns the reconcile-or-release leg for a prepared reservation"
    );
    assert_eq!(result.receipt.id, reservation_id);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_rejects_stale_authorized_lane_before_binding_dispatch() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let binding = RecordingBinding::new(json!({}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::Wasm, binding.clone()),
    )]);

    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());
    let err = dispatcher
        .dispatch_json(authorized_with_lane(
            CapabilityDispatchRequest {
                artifact_namespace: None,
                run_id: None,
                origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
                capability_id: CapabilityId::new("echo.say").unwrap(),
                scope,
                authenticated_actor_user_id: None,
                estimate: ResourceEstimate::default().set_concurrency_slots(1),
                mounts: None,
                resource_reservation: None,
                input: json!({"message": "stale lane"}),
            },
            RuntimeLane::Process,
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Wasm
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert!(binding.requests().is_empty());
}

#[tokio::test]
async fn dispatcher_fails_closed_when_prepared_reservation_was_revoked_before_binding_dispatch() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate::default().set_concurrency_slots(1);
    let reservation = governor.reserve(scope.clone(), estimate.clone()).unwrap();
    governor.release(reservation.id).unwrap();
    let binding = RecordingBinding::new(json!({}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::Wasm, binding.clone()),
    )]);

    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());
    let err = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            artifact_namespace: None,
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({"message": "revoked reservation"}),
        }))
        .await
        .unwrap_err();

    let DispatchError::Rejected {
        runtime: Some(RuntimeKind::Wasm),
        kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Resource),
        diagnostic: Some(diagnostic),
        ..
    } = &err
    else {
        panic!("expected resource failure with preserved cause, got {err:?}");
    };
    let cause = diagnostic
        .message
        .as_ref()
        .expect("resource failure must preserve a cause")
        .as_str();
    assert!(cause.contains("resource reservation"));
    assert!(
        err.to_string()
            .contains("provider dispatch rejected: Resource"),
        "dispatch error remains redacted at the public surface"
    );
    assert!(binding.requests().is_empty());
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_dispatches_through_the_capability_dispatcher_trait_object() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let binding = RecordingBinding::new(json!({"message": "trait object"}), Arc::clone(&governor));
    let resolver: Arc<dyn ToolResolver> = Arc::new(ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::Wasm, binding),
    )]));

    let dispatcher: Arc<dyn CapabilityDispatcher> =
        Arc::new(RuntimeDispatcher::from_arcs(resolver, governor));
    let result = dispatcher
        .dispatch_json(sample_request("echo.say", json!({"message": "hi"})))
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message": "trait object"}));
    assert_eq!(result.provider, ExtensionId::new("echo").unwrap());
}

#[tokio::test]
async fn chain_resolver_returns_first_binding_and_falls_through_misses() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let first = RecordingBinding::new(json!({"from": "first"}), Arc::clone(&governor));
    let second = RecordingBinding::new(json!({"from": "second"}), Arc::clone(&governor));
    let chain = ChainToolResolver::new(vec![
        Arc::new(ScriptedResolver::from_entries([(
            "shared.tool",
            resolved("first", RuntimeKind::Wasm, first),
        )])) as Arc<dyn ToolResolver>,
        Arc::new(ScriptedResolver::from_entries([
            (
                "shared.tool",
                resolved("second", RuntimeKind::Mcp, second.clone()),
            ),
            (
                "only-second.tool",
                resolved("second", RuntimeKind::Mcp, second),
            ),
        ])) as Arc<dyn ToolResolver>,
    ]);

    let shared = chain
        .resolve(&CapabilityId::new("shared.tool").unwrap())
        .expect("shared id resolves");
    assert_eq!(shared.provider, ExtensionId::new("first").unwrap());

    let fallthrough = chain
        .resolve(&CapabilityId::new("only-second.tool").unwrap())
        .expect("second resolver serves the miss");
    assert_eq!(fallthrough.provider, ExtensionId::new("second").unwrap());

    assert!(
        chain
            .resolve(&CapabilityId::new("missing.tool").unwrap())
            .is_none()
    );
}

fn resolved(provider: &str, runtime: RuntimeKind, binding: RecordingBinding) -> ResolvedCapability {
    ResolvedCapability {
        provider: ExtensionId::new(provider).unwrap(),
        runtime,
        adapter: Arc::new(binding),
    }
}

struct ScriptedResolver {
    bindings: HashMap<CapabilityId, ResolvedCapability>,
}

impl ScriptedResolver {
    fn empty() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    fn from_entries<const N: usize>(entries: [(&str, ResolvedCapability); N]) -> Self {
        Self {
            bindings: entries
                .into_iter()
                .map(|(id, resolved)| (CapabilityId::new(id).unwrap(), resolved))
                .collect(),
        }
    }
}

impl ToolResolver for ScriptedResolver {
    fn resolve(&self, capability_id: &CapabilityId) -> Option<ResolvedCapability> {
        self.bindings.get(capability_id).cloned()
    }
}

#[derive(Clone)]
struct ArtifactPersistCall {
    metadata: ArtifactWriteMetadata,
    bytes: Vec<u8>,
    receipt: ResourceReceipt,
}

#[derive(Default)]
struct RecordingArtifactPersister {
    calls: Mutex<Vec<ArtifactPersistCall>>,
}

impl RecordingArtifactPersister {
    fn calls(&self) -> Vec<ArtifactPersistCall> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AccountedArtifactPersister for RecordingArtifactPersister {
    async fn persist(
        &self,
        metadata: ArtifactWriteMetadata,
        bytes: &[u8],
        receipt: &ResourceReceipt,
    ) -> Result<CompletedArtifact, ArtifactWriteError> {
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let artifact_id = ArtifactId::new(u64::try_from(calls.len()).unwrap());
        calls.push(ArtifactPersistCall {
            metadata: metadata.clone(),
            bytes: bytes.to_vec(),
            receipt: receipt.clone(),
        });
        Ok(CompletedArtifact {
            artifact_ref: ArtifactRef::new(artifact_id),
            byte_len: u64::try_from(bytes.len()).unwrap(),
            total_lines: None,
            content_type: metadata.content_type,
            digest: ArtifactDigest::from_bytes(bytes),
        })
    }
}

/// A scripted binding that mirrors the real lane legs: reconcile the prepared
/// reservation when one was handed over, else reserve fresh and reconcile.
#[derive(Clone)]
struct RecordingBinding {
    output: Value,
    failure: Option<Arc<dyn Fn() -> DispatchError + Send + Sync>>,
    governor: Arc<InMemoryResourceGovernor>,
    requests: Arc<Mutex<Vec<RecordedBindingRequest>>>,
}

struct RecordedBindingRequest {
    capability_id: CapabilityId,
    scope: ResourceScope,
    artifact_namespace: Option<ArtifactNamespaceId>,
    mounts: Option<MountView>,
    resource_reservation: Option<ResourceReservation>,
    input: Value,
}

impl RecordingBinding {
    fn new(output: Value, governor: Arc<InMemoryResourceGovernor>) -> Self {
        Self {
            output,
            failure: None,
            governor,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing(
        error: impl Fn() -> DispatchError + Send + Sync + 'static,
        governor: Arc<InMemoryResourceGovernor>,
    ) -> Self {
        Self {
            output: json!(null),
            failure: Some(Arc::new(error)),
            governor,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RecordedBindingRequest> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

#[async_trait]
impl BoundCapabilityAdapter for RecordingBinding {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        self.requests.lock().unwrap().push(RecordedBindingRequest {
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            artifact_namespace: request.artifact_namespace,
            mounts: request.mounts.clone(),
            resource_reservation: request.resource_reservation.clone(),
            input: request.input.clone(),
        });
        if let Some(failure) = &self.failure {
            return Err(failure());
        }
        let output_bytes = serde_json::to_vec(&self.output).unwrap().len() as u64;
        let usage = ResourceUsage {
            output_bytes,
            ..ResourceUsage::default()
        };
        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => self
                .governor
                .reserve(request.scope.clone(), request.estimate.clone())
                .map_err(|_| DispatchError::Rejected {
                    runtime: Some(RuntimeKind::Wasm),
                    kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Resource),
                    diagnostic: None,
                    detail: None,
                })?,
        };
        let receipt = self
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| DispatchError::Rejected {
                runtime: Some(RuntimeKind::Wasm),
                kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Resource),
                diagnostic: None,
                detail: None,
            })?;
        Ok(RuntimeAdapterResult {
            canonical_output_digest: None,
            completed_artifact: None,
            output: self.output.clone(),
            display_preview: None,
            output_bytes,
            usage,
            receipt,
        })
    }
}

fn authorized(request: CapabilityDispatchRequest) -> Authorized {
    let lane = match request.capability_id.as_str() {
        id if id.contains("mcp") => RuntimeLane::Mcp,
        id if id.contains("script") => RuntimeLane::Process,
        id if id.contains("first_party") => RuntimeLane::FirstParty,
        _ => RuntimeLane::Wasm,
    };
    authorized_with_lane(request, lane)
}

fn authorized_with_lane(request: CapabilityDispatchRequest, lane: RuntimeLane) -> Authorized {
    let artifact_namespace = request.artifact_namespace;
    let invocation = Invocation {
        artifact_namespace,
        activity_id: ActivityId::new(),
        capability: request.capability_id,
        input: request.input,
        scope: request.scope,
        actor: request
            .authenticated_actor_user_id
            .map(Actor::Sealed)
            .unwrap_or(Actor::System),
        origin: request
            .run_id
            .map(InvocationOrigin::LoopRun)
            .unwrap_or_else(|| InvocationOrigin::Product(ProductKind::new("test").unwrap())),
        estimate: request.estimate,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
    };
    Authorized::seal_for_test_with_mounts(
        invocation,
        lane,
        request.mounts,
        request.resource_reservation,
        chrono::DateTime::<chrono::Utc>::MAX_UTC,
    )
}

fn sample_request(capability_id: &str, input: Value) -> Authorized {
    authorized(CapabilityDispatchRequest {
        artifact_namespace: None,
        run_id: None,
        origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
        capability_id: CapabilityId::new(capability_id).unwrap(),
        scope: sample_scope(),
        authenticated_actor_user_id: None,
        estimate: ResourceEstimate {
            concurrency_slots: Some(1),
            ..ResourceEstimate::default()
        },
        mounts: None,
        resource_reservation: None,
        input,
    })
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}
