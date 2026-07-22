//! Finding C (§5.3.2, mutable-registry TOCTOU): dispatch must bind to the lane
//! the authorization pinned into the witness, not to whatever the resolver
//! returns at dispatch time.
//!
//! An extension update between authorize and dispatch can replace a capability's
//! runtime — moving it onto a DIFFERENT execution lane — while the mounts,
//! reservation, and trust prepared for the OLD binding still ride the request.
//! The dispatcher resolves the capability through the injected [`ToolResolver`]
//! (a fresh snapshot-shaped binding), so it must compare that freshly-resolved
//! lane against the request's `pinned_lane` and fail CLOSED on a mismatch —
//! never executing a replacement runtime under authorization it did not receive.
//! Modeling the swap at the resolver seam is exactly the post-swap observation:
//! the resolver now hands back a binding on a lane the authorization never named.
//!
//! Full capability-content pinning (a same-lane binding swap) is tracked
//! separately in #6434; this contract locks the lane (runtime/executor) binding.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_dispatcher::*;
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_host_api::*;
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits,
};
use serde_json::{Value, json};

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn dispatch_request(
    scope: &ResourceScope,
    pinned_lane: Option<RuntimeLane>,
) -> CapabilityDispatchRequest {
    CapabilityDispatchRequest {
        run_id: None,
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
        pinned_lane,
        deadline: None,
        input: json!({"message": "hello"}),
    }
}

fn set_tenant_limit(governor: &InMemoryResourceGovernor, scope: &ResourceScope) {
    governor
        .set_limit(
            ResourceAccount::tenant(scope.tenant_id.clone()),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();
}

// A capability update that resolves the descriptor onto a DIFFERENT lane than
// the authorization pinned must fail closed with `LaneMismatch` and execute
// nothing. Here the authorization pinned the WASM lane, but the resolver now
// returns a Script (Process-lane) binding — exactly the post-swap observation.
#[tokio::test]
async fn resolver_lane_differs_from_pinned_lane_fails_closed() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    set_tenant_limit(&governor, &scope);

    // The binding is fully executable, so if the pinned-lane check were absent
    // the dispatch would happily run on the swapped-in Process lane — the test
    // proves the check fails closed BEFORE that execution, not that the lane
    // merely happens to be unconfigured.
    let binding = RecordingBinding::new(json!({"message": "echo"}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::Script, binding.clone()),
    )]);
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref()).with_event_sink(&events);

    let error = dispatcher
        .dispatch_json(dispatch_request(&scope, Some(RuntimeLane::Wasm)))
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            DispatchError::LaneMismatch {
                authorized: RuntimeLane::Wasm,
                resolved: RuntimeLane::Process,
                ..
            }
        ),
        "swapped runtime lane must fail closed with LaneMismatch(Wasm→Process), got {error:?}"
    );
    assert_eq!(error.failure_kind(), DispatchFailureKind::LaneMismatch);

    // The binding must NOT have run the replacement runtime.
    assert!(
        binding.requests().is_empty(),
        "no execution may occur on a lane the authorization did not name"
    );

    // The dispatcher must have emitted the redacted failure event and NEVER a
    // RuntimeSelected/DispatchSucceeded for the swapped lane.
    let recorded = events.events();
    let kinds: Vec<_> = recorded.iter().map(|event| event.kind).collect();
    assert!(
        kinds.contains(&RuntimeEventKind::DispatchFailed),
        "a lane mismatch must emit DispatchFailed, got {kinds:?}"
    );
    assert!(
        !kinds.contains(&RuntimeEventKind::RuntimeSelected),
        "a lane mismatch must fail before RuntimeSelected, got {kinds:?}"
    );
    let failed = recorded
        .iter()
        .find(|event| event.kind == RuntimeEventKind::DispatchFailed)
        .unwrap();
    assert_eq!(failed.error_kind.as_deref(), Some("lane_mismatch"));
}

// Control: a request whose pinned lane matches the freshly-resolved lane
// dispatches normally — the pinning check does not break the common path.
#[tokio::test]
async fn matching_pinned_lane_dispatches_normally() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    set_tenant_limit(&governor, &scope);

    let binding = RecordingBinding::new(json!({"message": "echo"}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::Wasm, binding.clone()),
    )]);
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref());

    let result = dispatcher
        .dispatch_json(dispatch_request(&scope, Some(RuntimeLane::Wasm)))
        .await
        .unwrap();

    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(
        binding.requests().len(),
        1,
        "a matching pinned lane must dispatch through the resolved binding"
    );
}

// A resolved runtime that maps to NO untrusted lane (host-internal `System`)
// must fail closed with `MissingRuntimeBackend` and execute nothing — even when
// no lane was pinned (`pinned_lane: None`). `System` is host-internal and is
// never dispatched through a runtime adapter (§4.2).
#[tokio::test]
async fn system_runtime_fails_closed_even_without_a_pinned_lane() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    set_tenant_limit(&governor, &scope);

    let binding = RecordingBinding::new(json!({"message": "echo"}), Arc::clone(&governor));
    let resolver = ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::System, binding.clone()),
    )]);
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref()).with_event_sink(&events);

    // No lane pinned — the fail-close must NOT depend on a pin being present.
    let error = dispatcher
        .dispatch_json(dispatch_request(&scope, None))
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            DispatchError::MissingRuntimeBackend {
                runtime: RuntimeKind::System
            }
        ),
        "a host-internal System runtime must fail closed with MissingRuntimeBackend, got {error:?}"
    );
    assert_eq!(
        error.failure_kind(),
        DispatchFailureKind::MissingRuntimeBackend
    );
    assert!(
        binding.requests().is_empty(),
        "System must never reach the runtime adapter"
    );
    let kinds: Vec<_> = events.events().iter().map(|event| event.kind).collect();
    assert!(
        !kinds.contains(&RuntimeEventKind::RuntimeSelected),
        "an unmappable runtime must fail before RuntimeSelected, got {kinds:?}"
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

/// Records every dispatch it receives (the COMPLETE request, so forwarding
/// regressions in `pinned_lane`/`deadline`/mounts/reservation stay observable),
/// then reserves-and-reconciles through the governor so a routed request produces
/// a real receipt.
#[derive(Clone)]
struct RecordingBinding {
    output: Value,
    governor: Arc<InMemoryResourceGovernor>,
    requests: Arc<Mutex<Vec<CapabilityDispatchRequest>>>,
}

impl RecordingBinding {
    fn new(output: Value, governor: Arc<InMemoryResourceGovernor>) -> Self {
        Self {
            output,
            governor,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<CapabilityDispatchRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl BoundCapabilityAdapter for RecordingBinding {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        self.requests.lock().unwrap().push(request.clone());
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
