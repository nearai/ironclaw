//! Selection semantics live in the registry-lane resolver now (TOOL-1):
//! prebound bindings per registry generation, with the dispatcher-era
//! selection failures (missing backend, unknown provider, runtime mismatch)
//! preserved as error bindings. These pins relocated here from the deleted
//! `ironclaw_dispatcher` per-invocation-selection tests.

use ironclaw_dispatcher::{
    BoundCapabilityAdapter, BoundCapabilityRequest, CapabilityDispatchRequest, RuntimeDispatcher,
    ToolResolver,
};
use ironclaw_events::{InMemoryEventSink, RuntimeEventKind};
use ironclaw_extensions::SharedExtensionRegistry;
use ironclaw_resources::{ResourceLimits, ResourceReservation};

use super::super::tool_resolver::RegistryLaneToolResolver;
use super::*;

fn shared_registry_with(manifest: &str, extension_id: &str) -> Arc<SharedExtensionRegistry> {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(test_package(manifest, extension_id))
        .unwrap();
    Arc::new(SharedExtensionRegistry::new(registry))
}

fn resolver_with_lanes(
    registry: Arc<SharedExtensionRegistry>,
    governor: Arc<InMemoryResourceGovernor>,
    lanes: std::collections::HashMap<
        RuntimeKind,
        Arc<dyn RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor>>,
    >,
) -> RegistryLaneToolResolver<LocalFilesystem, InMemoryResourceGovernor> {
    RegistryLaneToolResolver::new(
        registry,
        lanes,
        Arc::new(LocalFilesystem::new()),
        governor,
        policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::DirectLogged,
            SecretMode::ScrubbedEnv,
        ),
    )
}

struct EchoLane {
    governor: Arc<InMemoryResourceGovernor>,
}

#[async_trait]
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for EchoLane {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let output = request.input;
        let usage = ResourceUsage {
            output_bytes: serde_json::to_vec(&output).unwrap().len() as u64,
            ..ResourceUsage::default()
        };
        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => self
                .governor
                .reserve(request.scope, request.estimate)
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Resource,
                })?,
        };
        let receipt = self
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })?;
        Ok(RuntimeAdapterResult {
            output,
            display_preview: None,
            output_bytes: usage.output_bytes,
            usage,
            receipt,
        })
    }
}

fn wasm_capability_request(input: Value) -> CapabilityDispatchRequest {
    CapabilityDispatchRequest {
        capability_id: CapabilityId::new("test-wasm.run").unwrap(),
        scope: sample_scope(),
        estimate: ResourceEstimate {
            concurrency_slots: Some(1),
            output_bytes: Some(10_000),
            ..ResourceEstimate::default()
        },
        mounts: None,
        resource_reservation: None,
        input,
    }
}

#[tokio::test]
async fn resolver_prebinds_and_dispatches_through_the_registered_lane() {
    let registry = shared_registry_with(WASM_MANIFEST, "test-wasm");
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_concurrency_slots: Some(1),
                max_output_bytes: Some(10_000),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let mut lanes: std::collections::HashMap<
        RuntimeKind,
        Arc<dyn RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor>>,
    > = std::collections::HashMap::new();
    lanes.insert(
        RuntimeKind::Wasm,
        Arc::new(EchoLane {
            governor: Arc::clone(&governor),
        }),
    );
    let resolver: Arc<dyn ToolResolver> =
        Arc::new(resolver_with_lanes(registry, Arc::clone(&governor), lanes));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::from_arcs(resolver, Arc::clone(&governor))
        .with_event_sink_arc(Arc::new(events.clone()));

    let result = dispatcher
        .dispatch_json(wasm_capability_request(json!({"message":"prebound"})))
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message":"prebound"}));
    assert_eq!(result.provider, ExtensionId::new("test-wasm").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);
    let kinds = events
        .events()
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
}

#[tokio::test]
async fn unconfigured_lane_fails_missing_backend_and_releases_prepared_reservation() {
    let registry = shared_registry_with(WASM_MANIFEST, "test-wasm");
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };
    let reservation: ResourceReservation =
        governor.reserve(scope.clone(), estimate.clone()).unwrap();
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);
    let resolver: Arc<dyn ToolResolver> = Arc::new(resolver_with_lanes(
        registry,
        Arc::clone(&governor),
        std::collections::HashMap::new(),
    ));
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::from_arcs(resolver, Arc::clone(&governor))
        .with_event_sink_arc(Arc::new(events.clone()));

    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("test-wasm.run").unwrap(),
            scope,
            estimate,
            mounts: None,
            resource_reservation: Some(reservation),
            input: json!({"message":"blocked"}),
        })
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
    // The binding exists (selection succeeded when it was constructed), so
    // the failure now carries the resolved provider/runtime: requested →
    // runtime_selected → dispatch_failed.
    let recorded = events.events();
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::RuntimeSelected);
    assert_eq!(recorded[2].kind, RuntimeEventKind::DispatchFailed);
    assert_eq!(recorded[2].runtime, Some(RuntimeKind::Wasm));
    assert_eq!(
        recorded[2].error_kind.as_deref(),
        Some("missing_runtime_backend")
    );
}

#[tokio::test]
async fn resolver_tracks_registry_mutations_across_versions() {
    let registry = shared_registry_with(WASM_MANIFEST, "test-wasm");
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let mut lanes: std::collections::HashMap<
        RuntimeKind,
        Arc<dyn RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor>>,
    > = std::collections::HashMap::new();
    lanes.insert(
        RuntimeKind::Wasm,
        Arc::new(EchoLane {
            governor: Arc::clone(&governor),
        }),
    );
    let resolver = resolver_with_lanes(Arc::clone(&registry), Arc::clone(&governor), lanes);

    let echo_id = CapabilityId::new("test-wasm.run").unwrap();
    assert!(resolver.resolve(&echo_id).is_some(), "initial capability");

    // A capability published after the first resolve is served once the
    // registry version changes.
    registry
        .upsert(test_package(
            &WASM_MANIFEST.replace("test-wasm", "late-wasm"),
            "late-wasm",
        ))
        .unwrap();
    let late_id = CapabilityId::new("late-wasm.run").unwrap();
    let late = resolver.resolve(&late_id).expect("post-upsert capability");
    assert_eq!(late.provider, ExtensionId::new("late-wasm").unwrap());

    // A removed extension stops resolving.
    registry.remove(&ExtensionId::new("test-wasm").unwrap());
    assert!(
        resolver.resolve(&echo_id).is_none(),
        "removed capability must not resolve"
    );
    assert!(resolver.resolve(&late_id).is_some());
}

#[tokio::test]
async fn registry_rejects_descriptor_package_runtime_mismatch_at_insert() {
    // Relocated pin: the registry's insert validation is why a
    // descriptor/package runtime mismatch cannot reach a lane binding through
    // the public API (the resolver's mismatch error binding is defensive).
    let mut package = test_package(WASM_MANIFEST, "test-wasm");
    package.capabilities[0].runtime = RuntimeKind::Script;

    let err = ExtensionRegistry::new().insert(package).unwrap_err();

    assert!(matches!(
        err,
        ironclaw_extensions::ExtensionError::InvalidManifest { reason }
            if reason.contains("package capability descriptors do not match")
    ));
}

// `BoundCapabilityAdapter` is object-safe and the resolver returns owned
// clones — pin that a resolved binding survives a concurrent registry swap
// (in-flight work keeps the binding it resolved).
#[tokio::test]
async fn resolved_binding_survives_registry_swap_mid_flight() {
    let registry = shared_registry_with(WASM_MANIFEST, "test-wasm");
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let mut lanes: std::collections::HashMap<
        RuntimeKind,
        Arc<dyn RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor>>,
    > = std::collections::HashMap::new();
    lanes.insert(
        RuntimeKind::Wasm,
        Arc::new(EchoLane {
            governor: Arc::clone(&governor),
        }),
    );
    let resolver = resolver_with_lanes(Arc::clone(&registry), Arc::clone(&governor), lanes);

    let echo_id = CapabilityId::new("test-wasm.run").unwrap();
    let binding = resolver.resolve(&echo_id).expect("resolves before swap");
    registry.remove(&ExtensionId::new("test-wasm").unwrap());
    assert!(resolver.resolve(&echo_id).is_none());

    let adapter: Arc<dyn BoundCapabilityAdapter> = binding.adapter;
    let result = adapter
        .dispatch_json(BoundCapabilityRequest {
            capability_id: echo_id,
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            mounts: None,
            resource_reservation: None,
            input: json!({"in":"flight"}),
        })
        .await
        .unwrap();
    assert_eq!(result.output, json!({"in":"flight"}));
}
