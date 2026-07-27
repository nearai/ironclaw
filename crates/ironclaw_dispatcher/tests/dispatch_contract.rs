//! Dispatcher contract at the resolver seam (TOOL-1/TOOL-2).
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
use ironclaw_dispatcher::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use serde_json::{Value, json};

#[tokio::test]
async fn dispatcher_routes_capability_through_resolved_binding() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
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
    assert_eq!(requests[0].input, json!({"message": "hello dispatcher"}));
}

#[tokio::test]
async fn dispatcher_redacts_binding_failure_details() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let binding = RecordingBinding::failing(
        || DispatchError::Script {
            kind: RuntimeDispatchErrorKind::ExitFailure,
            model_visible_cause: None,
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
        DispatchError::Script {
            kind: RuntimeDispatchErrorKind::ExitFailure,
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

    let DispatchError::Wasm {
        kind: RuntimeDispatchErrorKind::Resource,
        model_visible_cause: Some(cause),
    } = &err
    else {
        panic!("expected resource failure with preserved cause, got {err:?}");
    };
    assert!(cause.contains("resource reservation"));
    assert!(
        err.to_string().contains("WASM dispatch failed: Resource"),
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
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Resource,
                    model_visible_cause: None,
                })?,
        };
        let receipt = self
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
                model_visible_cause: None,
            })?;
        Ok(RuntimeAdapterResult {
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
    let invocation = Invocation {
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
