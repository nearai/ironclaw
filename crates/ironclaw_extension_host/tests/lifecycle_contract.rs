//! Lifecycle contract tests (extension-runtime P2, workstream B).
//!
//! Drives `ExtensionHost` through the standard installation pipeline and
//! pins: the binding rule at activation (LIFE-1), activation failure publishes
//! nothing and records the terminal `Failed` state with a `last_error`
//! (LIFE-8), `channel.activate()` runs and its failure aborts (LIFE-9),
//! duplicate capability/route conflicts (LIFE-14), and in-flight snapshot
//! generation isolation (LIFE-15). The dormant multi-step removal machine and
//! crash-resume restore were deleted with the honest-state-machine refactor;
//! production removal is the facade path (`remove_record` + auth cleanup) and
//! is covered through the composition facades.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ironclaw_extension_host::test_support::{
    FakeChannelAdapter, FakeEgressFactory, FakeLoader, RecordingDrain, mcp_manifest,
    tool_and_channel_manifest,
};
use ironclaw_extension_host::{
    BindContext, BindError, ExtensionBindings, ExtensionEntrypoint, ExtensionHost,
    ExtensionHostDeps, ExtensionLoader, InstallationRecord, InstallationRecordStore,
    InstallationState, LifecycleError, LoadContext, LoadedExtension,
    RehydratedInstallationRecordStore,
};
use ironclaw_host_api::ToolAdapter;
use ironclaw_product::ChannelAdapter;

struct Harness {
    host: ExtensionHost,
    store: Arc<RehydratedInstallationRecordStore>,
    load_calls: Arc<AtomicUsize>,
}

struct ConfigRejectingLoader {
    bindings: ExtensionBindings,
}

#[async_trait::async_trait]
impl ExtensionLoader for ConfigRejectingLoader {
    async fn load(&self, _ctx: &LoadContext) -> Result<LoadedExtension, BindError> {
        Ok(LoadedExtension::new(Box::new(ConfigRejectingEntrypoint {
            bindings: self.bindings.clone(),
        })))
    }
}

struct ConfigRejectingEntrypoint {
    bindings: ExtensionBindings,
}

impl ExtensionEntrypoint for ConfigRejectingEntrypoint {
    fn bind(&self, ctx: BindContext) -> Result<ExtensionBindings, BindError> {
        if ctx
            .config
            .iter()
            .any(|(key, value)| key == "reject" && value == "true")
        {
            return Err(BindError::Load {
                reason: "scripted candidate rejection".to_string(),
            });
        }
        Ok(self.bindings.clone())
    }
}

async fn harness_with(bindings: ExtensionBindings, _channel: Arc<FakeChannelAdapter>) -> Harness {
    harness_full(bindings, false).await
}

async fn harness_full(bindings: ExtensionBindings, fail_load: bool) -> Harness {
    let store = Arc::new(RehydratedInstallationRecordStore::default());
    let load_calls = Arc::new(AtomicUsize::new(0));
    let deps = ExtensionHostDeps {
        store: Arc::clone(&store) as Arc<dyn InstallationRecordStore>,
        loader: Arc::new(FakeLoader {
            bindings,
            load_calls: Arc::clone(&load_calls),
            fail_load,
        }),
        drain: Arc::new(RecordingDrain::default()) as Arc<_>,
        egress: Arc::new(FakeEgressFactory),
        reserved_capability_ids: Default::default(),
        reserved_ingress_routes: Default::default(),
        hook_deadline: Duration::from_secs(5),
    };
    let host = ExtensionHost::new(deps).await;
    Harness {
        host,
        store,
        load_calls,
    }
}

async fn config_rejecting_harness(bindings: ExtensionBindings) -> Harness {
    let store = Arc::new(RehydratedInstallationRecordStore::default());
    let load_calls = Arc::new(AtomicUsize::new(0));
    let host = ExtensionHost::new(ExtensionHostDeps {
        store: Arc::clone(&store) as Arc<dyn InstallationRecordStore>,
        loader: Arc::new(ConfigRejectingLoader { bindings }),
        drain: Arc::new(RecordingDrain::default()) as Arc<_>,
        egress: Arc::new(FakeEgressFactory),
        reserved_capability_ids: Default::default(),
        reserved_ingress_routes: Default::default(),
        hook_deadline: Duration::from_secs(5),
    })
    .await;
    Harness {
        host,
        store,
        load_calls,
    }
}

fn record(
    extension_id: &str,
    resolved: ironclaw_extensions::ResolvedExtensionManifest,
) -> InstallationRecord {
    InstallationRecord {
        extension_id: extension_id.to_string(),
        installation_id: format!("{extension_id}-install"),
        state: InstallationState::Installed,
        resolved: Arc::new(resolved),
        config: Vec::new(),
        last_error: None,
    }
}

fn tool_and_channel_bindings(channel: Arc<FakeChannelAdapter>) -> ExtensionBindings {
    ExtensionBindings {
        tools: Some(
            Arc::new(ironclaw_extension_host::test_support::FakeToolAdapter)
                as Arc<dyn ToolAdapter>,
        ),
        channel: Some(channel as Arc<dyn ChannelAdapter>),
    }
}

// -------------------------------------------------------------------------
// LIFE-1: binding rule enforced at activation
// -------------------------------------------------------------------------

#[tokio::test]
async fn declared_tool_without_bound_adapter_fails_activation() {
    // mcp manifest declares tools; bind nothing.
    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(ExtensionBindings::default(), channel).await;
    h.host
        .install(record("acme-tools", mcp_manifest()))
        .await
        .unwrap();
    let error = h.host.activate("acme-tools").await.unwrap_err();
    assert!(matches!(error, LifecycleError::Bind(_)), "{error:?}");
    // LIFE-8: activation failure publishes nothing and records terminal Failed.
    assert!(h.host.snapshot().await.extension("acme-tools").is_none());
    let stored = h.store.get("acme-tools").await.unwrap().unwrap();
    assert_eq!(stored.state, InstallationState::Failed);
    assert!(stored.last_error.is_some());
}

#[tokio::test]
async fn hosted_mcp_connection_template_alone_fails_activation() {
    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        ExtensionBindings {
            tools: Some(Arc::new(
                ironclaw_extension_host::test_support::FakeToolAdapter,
            )),
            channel: None,
        },
        channel,
    )
    .await;
    h.host
        .install(record("acme-tools", mcp_manifest()))
        .await
        .unwrap();

    let error = h.host.activate("acme-tools").await.unwrap_err();

    assert!(
        matches!(
            error,
            LifecycleError::Bind(ironclaw_extension_host::BindError::EmptyHostedMcpToolCatalog)
        ),
        "{error:?}"
    );
    assert!(h.host.snapshot().await.extension("acme-tools").is_none());
    let stored = h.store.get("acme-tools").await.unwrap().unwrap();
    assert_eq!(stored.state, InstallationState::Failed);
    assert!(stored.last_error.is_some());
}

// -------------------------------------------------------------------------
// LIFE-9: channel.activate() runs; failure aborts activation
// -------------------------------------------------------------------------

#[tokio::test]
async fn channel_activate_runs_and_its_failure_aborts() {
    let channel = Arc::new(FakeChannelAdapter {
        fail_activate: true,
        ..FakeChannelAdapter::default()
    });
    let activate_calls = Arc::clone(&channel.activate_calls);
    let h = harness_with(
        tool_and_channel_bindings(Arc::clone(&channel)),
        Arc::clone(&channel),
    )
    .await;
    h.host
        .install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();
    let error = h.host.activate("acme").await.unwrap_err();
    assert!(
        matches!(error, LifecycleError::ActivationHook { .. }),
        "{error:?}"
    );
    assert_eq!(
        activate_calls.load(Ordering::SeqCst),
        1,
        "activate hook ran"
    );
    assert!(h.host.snapshot().await.extension("acme").is_none());
    let stored = h.store.get("acme").await.unwrap().unwrap();
    assert_eq!(stored.state, InstallationState::Failed);
    assert!(stored.last_error.is_some());
}

// -------------------------------------------------------------------------
// Happy path activation publishes exactly one generation and resolves tools
// -------------------------------------------------------------------------

#[tokio::test]
async fn activation_publishes_and_resolves() {
    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        tool_and_channel_bindings(Arc::clone(&channel)),
        Arc::clone(&channel),
    )
    .await;
    h.host
        .install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();
    h.host.activate("acme").await.unwrap();

    let snapshot = h.host.snapshot().await;
    assert!(snapshot.extension("acme").is_some());
    assert_eq!(channel.activate_calls.load(Ordering::SeqCst), 1);
    // Tool resolves (TOOL-1 groundwork: prebound adapter by capability id).
    let capability = ironclaw_host_api::CapabilityId::new("acme.ping").unwrap();
    let binding = snapshot.resolve_tool(&capability).expect("tool resolves");
    assert_eq!(binding.declaration.id.as_str(), "acme");
    assert_eq!(
        h.store.get("acme").await.unwrap().unwrap().state,
        InstallationState::Active
    );
    assert_eq!(h.load_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_candidate_refresh_keeps_prior_record_and_snapshot_generation() {
    let channel = Arc::new(FakeChannelAdapter::default());
    let h = config_rejecting_harness(tool_and_channel_bindings(channel)).await;
    let mut initial = record("acme", tool_and_channel_manifest());
    initial.config = vec![("reject".to_string(), "false".to_string())];
    h.host
        .publish_candidate(initial)
        .await
        .expect("initial candidate publishes");
    let before = h.host.snapshot().await;

    let mut rejected = record("acme", tool_and_channel_manifest());
    rejected.config = vec![("reject".to_string(), "true".to_string())];
    let error = h
        .host
        .publish_candidate(rejected)
        .await
        .expect_err("invalid refresh candidate must not replace the live generation");

    assert!(matches!(error, LifecycleError::Bind(_)), "{error:?}");
    let after = h.host.snapshot().await;
    assert_eq!(after.generation(), before.generation());
    assert!(after.extension("acme").is_some());
    let stored = h.store.get("acme").await.unwrap().unwrap();
    assert_eq!(stored.state, InstallationState::Active);
    assert_eq!(
        stored.config,
        vec![("reject".to_string(), "false".to_string())]
    );
}

// -------------------------------------------------------------------------
// LIFE-14: duplicate capability id across active extensions fails activation
// -------------------------------------------------------------------------

#[tokio::test]
async fn duplicate_capability_across_extensions_fails_activation() {
    let channel_a = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        tool_and_channel_bindings(Arc::clone(&channel_a)),
        Arc::clone(&channel_a),
    )
    .await;
    // Two installations resolving to the same manifest declare the same
    // capability id `acme.ping` and the same route `hooks`.
    h.host
        .install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();
    h.host.activate("acme").await.unwrap();
    h.host
        .install(record("acme-dup", tool_and_channel_manifest()))
        .await
        .unwrap();
    let error = h.host.activate("acme-dup").await.unwrap_err();
    assert!(matches!(error, LifecycleError::Conflict(_)), "{error:?}");
    // The first extension is still active; the conflicting one published nothing.
    assert!(h.host.snapshot().await.extension("acme").is_some());
    assert!(h.host.snapshot().await.extension("acme-dup").is_none());
    // The conflicting installation recorded the terminal Failed state.
    let dup = h.store.get("acme-dup").await.unwrap().unwrap();
    assert_eq!(dup.state, InstallationState::Failed);
    assert!(dup.last_error.is_some());
}

// -------------------------------------------------------------------------
// LIFE-15: in-flight resolution keeps its generation across an upgrade swap
// -------------------------------------------------------------------------

#[tokio::test]
async fn in_flight_snapshot_survives_a_later_swap() {
    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        tool_and_channel_bindings(Arc::clone(&channel)),
        Arc::clone(&channel),
    )
    .await;
    h.host
        .install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();
    h.host.activate("acme").await.unwrap();

    // Take a snapshot as an "in-flight" reader would.
    let in_flight = h.host.snapshot().await;
    let generation_before = in_flight.generation();
    assert!(in_flight.extension("acme").is_some());

    // Deactivate → the host swaps to a new generation with acme gone.
    h.host.deactivate("acme").await.unwrap();
    let after = h.host.snapshot().await;
    assert!(after.generation() > generation_before);
    assert!(after.extension("acme").is_none());
    // Deactivation returns the record to Installed (no longer serving).
    assert_eq!(
        h.store.get("acme").await.unwrap().unwrap().state,
        InstallationState::Installed
    );

    // The in-flight Arc still sees acme at its own generation.
    assert!(in_flight.extension("acme").is_some());
    assert_eq!(in_flight.generation(), generation_before);
}

#[tokio::test]
async fn snapshot_watch_subscription_observes_every_publish() {
    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        tool_and_channel_bindings(Arc::clone(&channel)),
        Arc::clone(&channel),
    )
    .await;
    let watch = h.host.snapshot_watch();
    let mut subscription = watch.subscribe();

    h.host
        .install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();
    h.host.activate("acme").await.unwrap();

    // The activation published a new generation: the subscription wakes and
    // the watch's current snapshot already carries the extension.
    subscription.changed().await.unwrap();
    let activated_generation = watch.current().generation();
    assert!(watch.current().extension("acme").is_some());

    h.host.deactivate("acme").await.unwrap();
    subscription.changed().await.unwrap();
    assert!(watch.current().generation() > activated_generation);
    assert!(watch.current().extension("acme").is_none());
}

// ── Snapshot resolution at the dispatch seam (TOOL-1 snapshot side, TOOL-10) ──

#[tokio::test]
async fn snapshot_resolver_serves_activated_tools_and_stops_after_deactivate() {
    use ironclaw_dispatcher::ToolResolver;
    use ironclaw_host_api::CapabilityId;

    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        tool_and_channel_bindings(Arc::clone(&channel)),
        Arc::clone(&channel),
    )
    .await;
    let resolver = ironclaw_extension_host::SnapshotToolResolver::new(h.host.snapshot_watch());
    let ping = CapabilityId::new("acme.ping").unwrap();

    assert!(
        resolver.resolve(&ping).is_none(),
        "nothing resolves before activation"
    );

    h.host
        .install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();
    h.host.activate("acme").await.unwrap();

    let resolved = resolver.resolve(&ping).expect("activated tool resolves");
    assert_eq!(resolved.provider.as_str(), "acme");
    assert_eq!(resolved.runtime, ironclaw_host_api::RuntimeKind::Wasm);

    // An in-flight binding keeps working across the deactivation swap; new
    // resolution stops.
    let in_flight = resolver.resolve(&ping).expect("binding before swap");
    h.host.deactivate("acme").await.unwrap();
    assert!(
        resolver.resolve(&ping).is_none(),
        "deactivated tool must not resolve"
    );
    let outcome = in_flight
        .adapter
        .dispatch_json(ironclaw_dispatcher::CapabilityDispatchRequest {
            run_id: None,
            origin: ironclaw_host_api::InvocationOrigin::Product(
                ironclaw_host_api::ProductKind::new("test").unwrap(),
            ),
            capability_id: ping.clone(),
            scope: sample_scope(),
            estimate: ironclaw_host_api::ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            authenticated_actor_user_id: None,
            input: serde_json::json!({"message": "in flight"}),
        })
        .await
        .expect("in-flight binding dispatches");
    assert_eq!(outcome.output, serde_json::json!({"ok": true}));
    assert!(outcome.output_bytes > 0);
}

#[tokio::test]
async fn snapshot_resolver_maps_tool_auth_required_to_the_generic_gate() {
    use ironclaw_dispatcher::ToolResolver;
    use ironclaw_host_api::{
        CapabilityId, DispatchError, SecretHandle, ToolAdapter, ToolCall, ToolError, ToolPorts,
        ToolResult,
    };

    struct AuthGatingAdapter;

    #[async_trait::async_trait]
    impl ToolAdapter for AuthGatingAdapter {
        async fn invoke(
            &self,
            _call: ToolCall,
            _ports: &ToolPorts<'_>,
        ) -> Result<ToolResult, ToolError> {
            Err(ToolError::AuthRequired {
                required_secrets: vec![SecretHandle::new("acme_token").unwrap()],
                credential_requirements: Vec::new(),
            })
        }
    }

    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        ExtensionBindings {
            tools: Some(Arc::new(AuthGatingAdapter)),
            channel: Some(Arc::clone(&channel) as Arc<dyn ChannelAdapter>),
        },
        channel,
    )
    .await;
    h.host
        .install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();
    h.host.activate("acme").await.unwrap();

    let resolver = ironclaw_extension_host::SnapshotToolResolver::new(h.host.snapshot_watch());
    let resolved = resolver
        .resolve(&CapabilityId::new("acme.ping").unwrap())
        .expect("resolves");
    let err = resolved
        .adapter
        .dispatch_json(ironclaw_dispatcher::CapabilityDispatchRequest {
            run_id: None,
            origin: ironclaw_host_api::InvocationOrigin::Product(
                ironclaw_host_api::ProductKind::new("test").unwrap(),
            ),
            capability_id: CapabilityId::new("acme.ping").unwrap(),
            scope: sample_scope(),
            estimate: ironclaw_host_api::ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            authenticated_actor_user_id: None,
            input: serde_json::json!({}),
        })
        .await
        .unwrap_err();

    // The gate payload survives the ABI so the standard blocked-turn re-auth
    // flow drives it (TOOL-5's dispatch leg).
    match err {
        DispatchError::AuthRequired {
            capability,
            required_secrets,
            ..
        } => {
            assert_eq!(capability.as_str(), "acme.ping");
            assert_eq!(required_secrets.len(), 1);
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn extension_capability_colliding_with_a_host_builtin_fails_activation() {
    use ironclaw_host_api::CapabilityId;

    let channel = Arc::new(FakeChannelAdapter::default());
    let store = Arc::new(RehydratedInstallationRecordStore::default());
    let deps = ExtensionHostDeps {
        store: Arc::clone(&store) as Arc<dyn InstallationRecordStore>,
        loader: Arc::new(FakeLoader {
            bindings: tool_and_channel_bindings(channel),
            load_calls: Arc::new(AtomicUsize::new(0)),
            fail_load: false,
        }),
        drain: Arc::new(RecordingDrain::default()),
        egress: Arc::new(FakeEgressFactory),
        reserved_capability_ids: [CapabilityId::new("acme.ping").unwrap()]
            .into_iter()
            .collect(),
        reserved_ingress_routes: Default::default(),
        hook_deadline: Duration::from_secs(5),
    };
    let host = ExtensionHost::new(deps).await;
    host.install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();

    let err = host.activate("acme").await.unwrap_err();
    assert!(
        matches!(
            &err,
            LifecycleError::Conflict(
                ironclaw_extension_host::SnapshotConflict::ReservedCapability { capability_id, .. }
            ) if capability_id == "acme.ping"
        ),
        "expected reserved-capability conflict, got {err:?}"
    );
    // Nothing published; the record recorded the terminal Failed state.
    assert!(host.snapshot().await.extension("acme").is_none());
    let stored = store.get("acme").await.unwrap().unwrap();
    assert_eq!(stored.state, InstallationState::Failed);
    assert!(stored.last_error.is_some());

    // The redacted reason is exposed to the product projection via
    // `installation_errors()` — the single source both the `Failed` projection
    // and the wire's `activation_error` are driven from.
    let errors = host.installation_errors().await.unwrap();
    assert_eq!(
        errors.len(),
        1,
        "one failed extension has a recorded reason"
    );
    assert!(
        errors.get("acme").is_some_and(|reason| !reason.is_empty()),
        "the failed activation reason is keyed by extension id"
    );
}

fn sample_scope() -> ironclaw_host_api::ResourceScope {
    ironclaw_host_api::ResourceScope {
        tenant_id: ironclaw_host_api::TenantId::new("tenant-a").unwrap(),
        user_id: ironclaw_host_api::UserId::new("user-a").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    }
}
