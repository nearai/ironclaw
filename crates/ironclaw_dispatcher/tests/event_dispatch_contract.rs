//! Dispatch event semantics at the resolver seam (part of TOOL-3).
//!
//! The event sequence, best-effort sink behavior, and redacted failure kinds
//! are unchanged by the prebound-resolver cutover: `dispatch_requested` →
//! `runtime_selected` (from the resolved binding's provider/runtime) →
//! `dispatch_succeeded`/`dispatch_failed`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_dispatcher::*;
use ironclaw_events::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use serde_json::{Value, json};
use tracing::Instrument;
use tracing_test::traced_test;

#[tokio::test]
async fn dispatcher_emits_events_for_resolved_dispatch_success() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = InMemoryEventSink::new();
    let resolver = ScriptedResolver::from_entries([
        (
            "echo-wasm.say",
            resolved_echo("echo-wasm", RuntimeKind::Wasm, &governor),
        ),
        (
            "echo-script.say",
            resolved_echo("echo-script", RuntimeKind::Script, &governor),
        ),
    ]);
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref()).with_event_sink(&events);

    dispatcher
        .dispatch_json(sample_request(
            "echo-wasm.say",
            json!({"message": "hello wasm"}),
        ))
        .await
        .unwrap();
    dispatcher
        .dispatch_json(sample_request(
            "echo-script.say",
            json!({"message": "hello script"}),
        ))
        .await
        .unwrap();

    let recorded = events.events();
    let kinds = recorded.iter().map(|event| event.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
    assert_eq!(
        recorded[0].capability_id,
        CapabilityId::new("echo-wasm.say").unwrap()
    );
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Wasm));
    assert_eq!(recorded[2].output_bytes, Some(24));
    assert_eq!(
        recorded[3].capability_id,
        CapabilityId::new("echo-script.say").unwrap()
    );
    assert_eq!(recorded[4].runtime, Some(RuntimeKind::Script));
    assert_eq!(
        recorded[5].provider,
        Some(ExtensionId::new("echo-script").unwrap())
    );
}

#[tokio::test]
async fn dispatcher_ignores_event_sink_failures_on_success() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = FailingEventSink;
    let resolver = ScriptedResolver::from_entries([(
        "echo-wasm.say",
        resolved_echo("echo-wasm", RuntimeKind::Wasm, &governor),
    )]);
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref()).with_event_sink(&events);

    let result = dispatcher
        .dispatch_json(sample_request(
            "echo-wasm.say",
            json!({"message": "event sink fails"}),
        ))
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message": "event sink fails"}));
}

#[tokio::test]
async fn dispatcher_preserves_original_error_when_failure_event_sink_fails() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = FailingEventSink;
    let resolver = ScriptedResolver::empty();
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref()).with_event_sink(&events);

    let err = dispatcher
        .dispatch_json(sample_request(
            "echo-script.say",
            json!({"message": "unknown capability"}),
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
}

#[tokio::test]
#[traced_test]
async fn dispatcher_logs_release_failure_without_masking_dispatch_error() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    // A reservation the governor never issued: releasing it fails, and that
    // failure must be logged without masking the dispatch error.
    let reservation = ResourceReservation {
        id: ResourceReservationId::new(),
        scope: scope.clone(),
        estimate: ResourceEstimate {
            concurrency_slots: Some(1),
            ..ResourceEstimate::default()
        },
    };
    let resolver = ScriptedResolver::empty();
    let dispatcher = RuntimeDispatcher::new(&resolver, &governor);

    let err = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: Some(reservation.clone()),
            input: json!({"message": "unknown capability"}),
        }))
        .instrument(tracing::info_span!(
            "dispatcher_logs_release_failure_without_masking_dispatch_error"
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
    assert!(logs_contain(
        "failed to release prepared resource reservation after dispatcher validation failure"
    ));
    assert!(logs_contain(&reservation.id.to_string()));
}

#[tokio::test]
async fn dispatcher_emits_redacted_runtime_error_kind_for_binding_failure() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let events = InMemoryEventSink::new();
    let resolver = ScriptedResolver::from_entries([(
        "echo-script.say",
        ResolvedCapability {
            provider: ExtensionId::new("echo-script").unwrap(),
            runtime: RuntimeKind::Script,
            adapter: Arc::new(FailingBinding {
                error: || DispatchError::Script {
                    kind: RuntimeDispatchErrorKind::ExitFailure,
                    model_visible_cause: None,
                },
            }),
        },
    )]);
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref()).with_event_sink(&events);

    let err = dispatcher
        .dispatch_json(sample_request(
            "echo-script.say",
            json!({"message": "binding fails"}),
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::Script {
            kind: RuntimeDispatchErrorKind::ExitFailure,
            ..
        }
    ));

    let recorded = events.events();
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[1].kind, RuntimeEventKind::RuntimeSelected);
    assert_eq!(recorded[2].kind, RuntimeEventKind::DispatchFailed);
    assert_eq!(recorded[2].error_kind.as_deref(), Some("exit_failure"));
    assert_eq!(recorded[2].runtime, Some(RuntimeKind::Script));
    assert_eq!(
        recorded[2].provider,
        Some(ExtensionId::new("echo-script").unwrap())
    );
}

#[tokio::test]
async fn dispatcher_emits_failed_event_for_unknown_capability_without_reserving() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let events = InMemoryEventSink::new();
    let resolver = ScriptedResolver::empty();
    let dispatcher = RuntimeDispatcher::new(&resolver, governor.as_ref()).with_event_sink(&events);

    let err = dispatcher
        .dispatch_json(authorized(CapabilityDispatchRequest {
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"message": "blocked"}),
        }))
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());

    let recorded = events.events();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::DispatchFailed);
    assert_eq!(recorded[1].provider, None);
    assert_eq!(recorded[1].runtime, None);
    assert_eq!(
        recorded[1].error_kind.as_deref(),
        Some("unknown_capability")
    );
}

struct FailingEventSink;

#[async_trait]
impl EventSink for FailingEventSink {
    async fn emit(&self, _event: RuntimeEvent) -> Result<(), EventError> {
        Err(EventError::Sink {
            reason: "event sink unavailable".to_string(),
        })
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

fn resolved_echo(
    provider: &str,
    runtime: RuntimeKind,
    governor: &Arc<InMemoryResourceGovernor>,
) -> ResolvedCapability {
    ResolvedCapability {
        provider: ExtensionId::new(provider).unwrap(),
        runtime,
        adapter: Arc::new(EchoBinding {
            governor: Arc::clone(governor),
        }),
    }
}

/// Echoes the input back and reconciles usage against the governor, mirroring
/// the real lane legs.
struct EchoBinding {
    governor: Arc<InMemoryResourceGovernor>,
}

#[async_trait]
impl BoundCapabilityAdapter for EchoBinding {
    async fn dispatch_json(
        &self,
        request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let output: Value = request.input;
        let output_bytes = serde_json::to_vec(&output).unwrap().len() as u64;
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
            output,
            display_preview: None,
            output_bytes,
            usage,
            receipt,
        })
    }
}

struct FailingBinding<F>
where
    F: Fn() -> DispatchError + Send + Sync,
{
    error: F,
}

#[async_trait]
impl<F> BoundCapabilityAdapter for FailingBinding<F>
where
    F: Fn() -> DispatchError + Send + Sync,
{
    async fn dispatch_json(
        &self,
        _request: CapabilityDispatchRequest,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        Err((self.error)())
    }
}

fn authorized(request: CapabilityDispatchRequest) -> Authorized {
    let lane = match request.capability_id.as_str() {
        id if id.contains("mcp") => RuntimeLane::Mcp,
        id if id.contains("script") => RuntimeLane::Process,
        id if id.contains("first_party") => RuntimeLane::FirstParty,
        _ => RuntimeLane::Wasm,
    };
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
            output_bytes: Some(10_000),
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
