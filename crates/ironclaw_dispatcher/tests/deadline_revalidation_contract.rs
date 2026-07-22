//! IronLoop finding #6436 (§5.3.2): the authorization deadline must be
//! revalidated at the EXECUTION boundary, not only when the caller consumes the
//! `Authorized` witness.
//!
//! `dispatch_inputs_from_witness` checks the witness deadline once (via
//! `Authorized::into_parts`) and hands the dispatcher `(mounts, reservation,
//! lane)` plus the deadline. `RuntimeDispatcher::dispatch_json` then `await`s the
//! `DispatchRequested` and `RuntimeSelected` event emissions BEFORE calling the
//! resolved binding. A claimed approval lease can expire during either await, so
//! without a re-check the binding side effect could run AFTER its authorization
//! deadline. This contract forces that race with a DELAYED event sink whose
//! `emit` sleeps past a near-future deadline, and asserts the binding is NEVER
//! invoked and dispatch fails closed with `AuthorizationExpired`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_dispatcher::*;
use ironclaw_events::{EventError, EventSink, InMemoryEventSink, RuntimeEvent, RuntimeEventKind};
use ironclaw_host_api::*;
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits,
};
use serde_json::{Value, json};

/// An event sink whose `emit` sleeps `delay` before recording — modelling the
/// real dispatcher's pre-execution event-emission awaits taking long enough for a
/// claimed approval lease to expire mid-dispatch. Wraps an `InMemoryEventSink` so
/// the emitted kinds are still inspectable.
#[derive(Clone)]
struct DelayedEventSink {
    inner: InMemoryEventSink,
    delay: Duration,
}

impl DelayedEventSink {
    fn new(delay: Duration) -> Self {
        Self {
            inner: InMemoryEventSink::new(),
            delay,
        }
    }

    fn events(&self) -> Vec<RuntimeEvent> {
        self.inner.events()
    }
}

#[async_trait]
impl EventSink for DelayedEventSink {
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError> {
        tokio::time::sleep(self.delay).await;
        self.inner.emit(event).await
    }
}

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
    deadline: Option<Timestamp>,
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
        pinned_lane: None,
        deadline,
        input: json!({"message": "hello"}),
    }
}

fn set_limit(governor: &InMemoryResourceGovernor, scope: &ResourceScope) {
    governor
        .set_limit(
            ResourceAccount::tenant(scope.tenant_id.clone()),
            ResourceLimits::default()
                .set_max_concurrency_slots(1)
                .set_max_output_bytes(10_000),
        )
        .unwrap();
}

fn echo_resolver(governor: &Arc<InMemoryResourceGovernor>) -> (ScriptedResolver, RecordingBinding) {
    let binding = RecordingBinding::new(json!({"message": "echo"}), Arc::clone(governor));
    let resolver = ScriptedResolver::from_entries([(
        "echo.say",
        resolved("echo", RuntimeKind::Wasm, binding.clone()),
    )]);
    (resolver, binding)
}

// A witness deadline crossed by the dispatcher's pre-execution event-emission
// awaits must fail closed with `AuthorizationExpired` and execute NOTHING — the
// side effect must never run after its authorization deadline.
#[tokio::test]
async fn deadline_crossed_during_pre_execution_awaits_fails_closed_and_never_executes() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    set_limit(&governor, &scope);

    // The binding is fully executable, so if the deadline re-check were absent the
    // dispatch would happily execute — the test proves the re-check fails closed
    // BEFORE the binding, not that the lane is merely unconfigured.
    let (resolver, binding) = echo_resolver(&governor);
    // Each emit sleeps 60ms; the deadline is only 5ms out, so it is already past
    // by the time the dispatcher reaches the pre-execution re-check.
    let events = DelayedEventSink::new(Duration::from_millis(60));
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref())
        .with_event_sink_arc(Arc::new(events.clone()));

    let deadline = chrono::Utc::now() + chrono::Duration::milliseconds(5);
    let error = dispatcher
        .dispatch_json(dispatch_request(&scope, Some(deadline)))
        .await
        .unwrap_err();

    assert!(
        matches!(error, DispatchError::AuthorizationExpired { .. }),
        "an expired witness deadline must fail closed with AuthorizationExpired, got {error:?}"
    );
    assert_eq!(
        error.failure_kind(),
        DispatchFailureKind::AuthorizationExpired
    );

    // The binding must NEVER have run — no side effect after the deadline.
    assert!(
        binding.requests().is_empty(),
        "the binding must not run after the authorization deadline"
    );

    // The redacted failure event is emitted; a DispatchSucceeded never is.
    let kinds: Vec<_> = events.events().iter().map(|event| event.kind).collect();
    assert!(
        kinds.contains(&RuntimeEventKind::DispatchFailed),
        "an expired deadline must emit DispatchFailed, got {kinds:?}"
    );
    assert!(
        !kinds.contains(&RuntimeEventKind::DispatchSucceeded),
        "an expired deadline must never emit DispatchSucceeded, got {kinds:?}"
    );
    let failed = events
        .events()
        .into_iter()
        .find(|event| event.kind == RuntimeEventKind::DispatchFailed)
        .unwrap();
    assert_eq!(failed.error_kind.as_deref(), Some("authorization_expired"));
}

// Behavior-neutrality control: a deadline comfortably in the future still
// dispatches normally even through the delayed sink — the re-check is a no-op on
// the common path.
#[tokio::test]
async fn future_deadline_dispatches_normally_through_delayed_sink() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    set_limit(&governor, &scope);

    let (resolver, binding) = echo_resolver(&governor);
    let events = DelayedEventSink::new(Duration::from_millis(5));
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref())
        .with_event_sink_arc(Arc::new(events.clone()));

    let deadline = chrono::Utc::now() + chrono::Duration::seconds(60);
    let result = dispatcher
        .dispatch_json(dispatch_request(&scope, Some(deadline)))
        .await
        .unwrap();

    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(binding.requests().len(), 1);
}

// A `None` deadline (witness-free/legacy dispatch) skips the re-check entirely —
// even through the delayed sink, the binding still runs. Byte-identical to the
// pre-#6436 path.
#[tokio::test]
async fn none_deadline_skips_recheck_and_executes() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    set_limit(&governor, &scope);

    let (resolver, binding) = echo_resolver(&governor);
    let events = DelayedEventSink::new(Duration::from_millis(60));
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref())
        .with_event_sink_arc(Arc::new(events.clone()));

    let result = dispatcher
        .dispatch_json(dispatch_request(&scope, None))
        .await
        .unwrap();

    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(binding.requests().len(), 1);
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
/// regressions in `deadline`/`pinned_lane`/mounts/reservation stay observable),
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
