//! Lifecycle contract tests (extension-runtime P2, workstream B).
//!
//! Drives `ExtensionHost` through the standard installation pipeline and
//! pins: the binding rule at activation (LIFE-1), activation failure publishes
//! nothing and records the terminal `Failed` state with a `last_error`
//! (LIFE-8), `channel.activate()` runs and its failure aborts (LIFE-9),
//! duplicate capability/route conflicts (LIFE-14), and in-flight snapshot
//! generation isolation (LIFE-15). The dormant multi-step removal machine and
//! crash-resume restore were deleted with the honest-state-machine refactor;
//! production removal is the service path (`remove_record` + auth cleanup) and
//! is covered through the composition services.

use ironclaw_extension_contracts::channel_adapter::ChannelSurfaces;
use ironclaw_extension_contracts::state::InstallationState;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ironclaw_extension_contracts::tool_adapter::ToolAdapter;
use ironclaw_extension_host::test_support::{
    FakeChannelAdapter, FakeEgressFactory, FakeLoader, RecordingDrain, RecordingEgressFactory,
    mcp_manifest, registering_channel_manifest, tool_and_channel_manifest,
};
use ironclaw_extension_host::{
    ExtensionBindings, ExtensionHost, ExtensionHostDeps, InstallationRecord,
    InstallationRecordStore, LifecycleError, RehydratedInstallationRecordStore,
};
use ironclaw_host_api::{ids::ProductKind, invocation::InvocationOrigin};

struct Harness {
    host: ExtensionHost,
    store: Arc<RehydratedInstallationRecordStore>,
    load_calls: Arc<AtomicUsize>,
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
        linked_sessions: ironclaw_extension_host::LinkedSessionStore::unavailable(),
        linked_accounts: std::sync::Arc::new(
            ironclaw_extension_host::UnavailableLinkedAccountResolution,
        ),
        admin_secrets: None,
    };
    let host = ExtensionHost::new(deps).await;
    Harness {
        host,
        store,
        load_calls,
    }
}

fn record(
    extension_id: &str,
    resolved: ironclaw_extension_registry::ResolvedExtensionManifest,
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
        // The fixture manifest declares a webhook ingress, a message reply,
        // and a delivery section, so every half must be bound or the per-axis
        // binding rule fails activation.
        channel: ChannelSurfaces::default()
            .with_ingress(channel.clone())
            .with_reply(channel.clone())
            .with_delivery(channel),
        device_link: None,
    }
}

/// Bindings for the registration fixture: same three halves, no tools.
fn channel_only_bindings(channel: Arc<FakeChannelAdapter>) -> ExtensionBindings {
    ExtensionBindings {
        tools: None,
        channel: ChannelSurfaces::default()
            .with_ingress(channel.clone())
            .with_reply(channel.clone())
            .with_delivery(channel),
        device_link: None,
    }
}

async fn harness_with_egress(
    bindings: ExtensionBindings,
    egress: Arc<RecordingEgressFactory>,
) -> Harness {
    let store = Arc::new(RehydratedInstallationRecordStore::default());
    let load_calls = Arc::new(AtomicUsize::new(0));
    let deps = ExtensionHostDeps {
        store: Arc::clone(&store) as Arc<dyn InstallationRecordStore>,
        loader: Arc::new(FakeLoader {
            bindings,
            load_calls: Arc::clone(&load_calls),
            fail_load: false,
        }),
        drain: Arc::new(RecordingDrain::default()) as Arc<_>,
        egress,
        reserved_capability_ids: Default::default(),
        reserved_ingress_routes: Default::default(),
        hook_deadline: Duration::from_secs(5),
        linked_sessions: ironclaw_extension_host::LinkedSessionStore::unavailable(),
        linked_accounts: std::sync::Arc::new(
            ironclaw_extension_host::UnavailableLinkedAccountResolution,
        ),
        admin_secrets: None,
    };
    Harness {
        host: ExtensionHost::new(deps).await,
        store,
        load_calls,
    }
}

fn registering_record(config: Vec<(String, String)>) -> InstallationRecord {
    let mut record = record("acme-hook", registering_channel_manifest());
    record.config = config;
    record
}

fn webhook_config() -> Vec<(String, String)> {
    vec![(
        "acme_webhook_url".to_string(),
        "https://host.example/webhooks/extensions/acme-hook/events".to_string(),
    )]
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
            channel: Default::default(),
            device_link: None,
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
// LIFE-9: the ingress-wiring recipes run at activation/deactivation, and a
// registration failure aborts activation.
//
// This is where `ChannelAdapter::activate`/`cleanup` went. The assertions
// below are the ones the deleted Telegram adapter tests made, re-aimed at the
// generic executor that now owns the behavior — the credential travels as a
// HANDLE and never as bytes, the shared secret rides `body_credentials` so the
// host inserts its VALUE at the manifest's declared pointer, and the rendered
// body carries neither the secret nor the handle name.
// -------------------------------------------------------------------------

#[tokio::test]
async fn ingress_registration_runs_at_activation_with_host_side_credentials() {
    let egress = Arc::new(RecordingEgressFactory::ok());
    let h = harness_with_egress(
        channel_only_bindings(Arc::new(FakeChannelAdapter::default())),
        Arc::clone(&egress),
    )
    .await;
    h.host
        .install(registering_record(webhook_config()))
        .await
        .unwrap();
    h.host.activate("acme-hook").await.unwrap();

    let requests = egress.requests();
    assert_eq!(
        requests.len(),
        1,
        "activation runs the registration recipe once"
    );
    let request = &requests[0];
    assert_eq!(
        request.url, "https://api.acme.example/bot{acme_hook_token}/setWebhook",
        "the credential placeholder reaches egress UNRESOLVED — the host \
         substitutes it, so token bytes never pass through the executor"
    );
    assert_eq!(
        request
            .body_credentials
            .iter()
            .map(ironclaw_host_api::ids::SecretHandle::as_str)
            .collect::<Vec<_>>(),
        vec!["acme_hook_secret"],
        "the shared secret rides as a declared body-credential handle"
    );
    let body: serde_json::Value =
        serde_json::from_slice(request.body.as_deref().expect("body")).expect("json");
    assert_eq!(
        body["url"], "https://host.example/webhooks/extensions/acme-hook/events",
        "non-secret config substitutes normally"
    );
    assert!(
        body.get("secret_token").is_none(),
        "insertion at the declared pointer is host-side; the executor must not fabricate it"
    );
    assert!(
        !String::from_utf8_lossy(request.body.as_deref().unwrap()).contains("acme_hook_secret"),
        "a handle name must never be sent to the vendor"
    );

    // Deactivation runs the deregistration half.
    h.host.deactivate("acme-hook").await.unwrap();
    let requests = egress.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].url.ends_with("/deleteWebhook"));
    assert!(
        requests[1].body.is_none(),
        "a bodyless recipe sends no body"
    );
}

#[tokio::test]
async fn ingress_registration_selects_its_declared_egress_independent_of_order() {
    let egress = Arc::new(RecordingEgressFactory::ok());
    let h = harness_with_egress(
        channel_only_bindings(Arc::new(FakeChannelAdapter::default())),
        Arc::clone(&egress),
    )
    .await;
    let mut manifest = registering_channel_manifest();
    let channel = manifest.channel.as_mut().expect("channel");
    let mut decoy = channel.egress[0].clone();
    decoy.host = "decoy.example".to_string();
    decoy.paths = vec!["/unrelated".to_string()];
    channel.egress.insert(0, decoy);
    let mut record = record("acme-hook", manifest);
    record.config = webhook_config();
    h.host.install(record).await.expect("install");

    h.host.activate("acme-hook").await.expect("activate");

    let requests = egress.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].url.starts_with("https://api.acme.example/"),
        "reordering unrelated egress entries must not retarget registration: {}",
        requests[0].url
    );
}

#[tokio::test]
async fn a_failing_ingress_registration_aborts_activation() {
    let egress = Arc::new(RecordingEgressFactory::failing());
    let h = harness_with_egress(
        channel_only_bindings(Arc::new(FakeChannelAdapter::default())),
        Arc::clone(&egress),
    )
    .await;
    h.host
        .install(registering_record(webhook_config()))
        .await
        .unwrap();

    let error = h.host.activate("acme-hook").await.unwrap_err();
    assert!(
        matches!(error, LifecycleError::ActivationHook { .. }),
        "{error:?}"
    );
    assert_eq!(egress.requests().len(), 1, "the recipe was attempted");
    assert!(
        h.host.snapshot().await.extension("acme-hook").is_none(),
        "a failed registration publishes nothing"
    );
    let stored = h.store.get("acme-hook").await.unwrap().unwrap();
    assert_eq!(stored.state, InstallationState::Failed);
    assert!(stored.last_error.is_some());
}

/// Deregistration is best-effort by contract: the extension is already
/// unpublished, so an unreachable vendor must not strand the deactivation.
#[tokio::test]
async fn a_failing_deregistration_does_not_strand_deactivation() {
    let egress = Arc::new(RecordingEgressFactory::ok());
    let h = harness_with_egress(
        channel_only_bindings(Arc::new(FakeChannelAdapter::default())),
        Arc::clone(&egress),
    )
    .await;
    h.host
        .install(registering_record(webhook_config()))
        .await
        .unwrap();
    h.host.activate("acme-hook").await.unwrap();
    egress.set_status(500);
    h.host
        .deactivate("acme-hook")
        .await
        .expect("a vendor failure must not block deactivation");
    let requests = egress.requests();
    assert_eq!(requests.len(), 2, "deregistration must be attempted");
    assert!(requests[1].url.ends_with("/deleteWebhook"));
    assert_eq!(
        h.store.get("acme-hook").await.unwrap().unwrap().state,
        InstallationState::Installed
    );
}

/// A channel declaring no recipes is a no-op — what "default no-op" used to
/// mean when these were trait methods, minus the trait surface.
#[tokio::test]
async fn a_channel_without_recipes_makes_no_vendor_call() {
    let egress = Arc::new(RecordingEgressFactory::ok());
    let h = harness_with_egress(
        tool_and_channel_bindings(Arc::new(FakeChannelAdapter::default())),
        Arc::clone(&egress),
    )
    .await;
    h.host
        .install(record("acme", tool_and_channel_manifest()))
        .await
        .unwrap();
    h.host.activate("acme").await.unwrap();
    assert!(egress.requests().is_empty());
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
    // Tool resolves (TOOL-1 groundwork: prebound adapter by capability id).
    let capability = ironclaw_host_api::ids::CapabilityId::new("acme.ping").unwrap();
    let binding = snapshot.resolve_tool(&capability).expect("tool resolves");
    assert_eq!(binding.declaration.id.as_str(), "acme");
    assert_eq!(
        h.store.get("acme").await.unwrap().unwrap().state,
        InstallationState::Active
    );
    assert_eq!(h.load_calls.load(Ordering::SeqCst), 1);
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
    use ironclaw_capabilities::ToolResolver;
    use ironclaw_host_api::ids::CapabilityId;

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
    assert_eq!(
        resolved.runtime,
        ironclaw_host_api::runtime::RuntimeKind::Wasm
    );

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
        .dispatch_json(ironclaw_capabilities::CapabilityDispatchRequest {
            authorized_descriptor: None,
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: ping.clone(),
            scope: sample_scope(),
            estimate: ironclaw_host_api::resource::ResourceEstimate::default(),
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
    use ironclaw_capabilities::ToolResolver;
    use ironclaw_extension_contracts::tool_adapter::{
        ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult,
    };
    use ironclaw_host_api::{
        dispatch::{
            DispatchAuthRequirement, DispatchError, ProviderDiagnostic, UntrustedProviderMessage,
        },
        ids::{CapabilityId, SecretHandle},
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
                requirement: Box::new(DispatchAuthRequirement {
                    required_secrets: vec![SecretHandle::new("acme_token").unwrap()],
                    credential_requirements: Vec::new(),
                    model_visible_cause: Some(ProviderDiagnostic {
                        code: None,
                        message: Some(UntrustedProviderMessage::new(
                            "provider error code: github_api_error_status_401; provider message: Bad credentials",
                        )),
                        retry_after: None,
                    }),
                }),
            })
        }
    }

    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        ExtensionBindings {
            tools: Some(Arc::new(AuthGatingAdapter)),
            channel: channel_only_bindings(Arc::clone(&channel)).channel,
            device_link: None,
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
        .dispatch_json(ironclaw_capabilities::CapabilityDispatchRequest {
            authorized_descriptor: None,
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("acme.ping").unwrap(),
            scope: sample_scope(),
            estimate: ironclaw_host_api::resource::ResourceEstimate::default(),
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
            requirement,
        } => {
            assert_eq!(capability.as_str(), "acme.ping");
            assert_eq!(requirement.required_secrets.len(), 1);
            assert_eq!(
                requirement
                    .model_visible_cause
                    .as_ref()
                    .and_then(|diagnostic| diagnostic.message.as_ref())
                    .map(|message| message.as_str()),
                Some(
                    "provider error code: github_api_error_status_401; provider message: Bad credentials"
                )
            );
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn snapshot_resolver_preserves_typed_provider_rejection() {
    use ironclaw_capabilities::ToolResolver;
    use ironclaw_extension_contracts::tool_adapter::{
        ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult,
    };
    use ironclaw_host_api::{
        dispatch::{
            DispatchError, ProviderDiagnostic, ProviderErrorCode, RuntimeDispatchErrorKind,
            UntrustedProviderMessage,
        },
        ids::CapabilityId,
    };

    struct RejectingAdapter;

    #[async_trait::async_trait]
    impl ToolAdapter for RejectingAdapter {
        async fn invoke(
            &self,
            _call: ToolCall,
            _ports: &ToolPorts<'_>,
        ) -> Result<ToolResult, ToolError> {
            Err(ToolError::Rejected {
                kind: RuntimeDispatchErrorKind::Client,
                diagnostic: Some(Box::new(ProviderDiagnostic {
                    code: Some(ProviderErrorCode::new("mcp_tool_rejected")),
                    message: Some(UntrustedProviderMessage::new("Bad credentials")),
                    retry_after: None,
                })),
                detail: None,
            })
        }
    }

    let channel = Arc::new(FakeChannelAdapter::default());
    let h = harness_with(
        ExtensionBindings {
            tools: Some(Arc::new(RejectingAdapter)),
            channel: channel_only_bindings(Arc::clone(&channel)).channel,
            device_link: None,
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
    let error = resolved
        .adapter
        .dispatch_json(ironclaw_capabilities::CapabilityDispatchRequest {
            authorized_descriptor: None,
            run_id: None,
            origin: InvocationOrigin::Product(ProductKind::new("test").unwrap()),
            capability_id: CapabilityId::new("acme.ping").unwrap(),
            scope: sample_scope(),
            estimate: ironclaw_host_api::resource::ResourceEstimate::default(),
            mounts: None,
            resource_reservation: None,
            authenticated_actor_user_id: None,
            input: serde_json::json!({}),
        })
        .await
        .unwrap_err();

    let DispatchError::Rejected {
        diagnostic: Some(diagnostic),
        ..
    } = error
    else {
        panic!("provider rejection must survive snapshot resolver");
    };
    assert_eq!(
        diagnostic.code.as_ref().map(|code| code.as_str()),
        Some("mcp_tool_rejected")
    );
    assert_eq!(
        diagnostic.message.as_ref().map(|message| message.as_str()),
        Some("Bad credentials")
    );
}

#[tokio::test]
async fn extension_capabilities_colliding_with_host_bridges_fail_activation() {
    use ironclaw_host_api::ids::CapabilityId;

    let channel = Arc::new(FakeChannelAdapter::default());
    let store = Arc::new(RehydratedInstallationRecordStore::default());
    let reserved_capability_ids: std::collections::BTreeSet<_> = [
        "ironclaw.tool_search",
        "ironclaw.tool_describe",
        "ironclaw.tool_call",
    ]
    .into_iter()
    .map(|id| CapabilityId::new(id).unwrap())
    .collect();
    let deps = ExtensionHostDeps {
        store: Arc::clone(&store) as Arc<dyn InstallationRecordStore>,
        loader: Arc::new(FakeLoader {
            bindings: tool_and_channel_bindings(channel),
            load_calls: Arc::new(AtomicUsize::new(0)),
            fail_load: false,
        }),
        drain: Arc::new(RecordingDrain::default()),
        egress: Arc::new(FakeEgressFactory),
        reserved_capability_ids: reserved_capability_ids.clone(),
        reserved_ingress_routes: Default::default(),
        hook_deadline: Duration::from_secs(5),
        linked_sessions: ironclaw_extension_host::LinkedSessionStore::unavailable(),
        linked_accounts: std::sync::Arc::new(
            ironclaw_extension_host::UnavailableLinkedAccountResolution,
        ),
        admin_secrets: None,
    };
    let host = ExtensionHost::new(deps).await;

    for capability_id in reserved_capability_ids {
        let extension_id = capability_id.as_str().replace('.', "-");
        let mut manifest = tool_and_channel_manifest();
        manifest.tools[0].id = capability_id.clone();
        host.install(record(&extension_id, manifest)).await.unwrap();

        let err = host.activate(&extension_id).await.unwrap_err();
        assert!(
            matches!(
                &err,
                LifecycleError::Conflict(
                    ironclaw_extension_host::SnapshotConflict::ReservedCapability {
                        capability_id: conflicting_id,
                        ..
                    }
                ) if conflicting_id == capability_id.as_str()
            ),
            "expected reserved-capability conflict for {capability_id}, got {err:?}"
        );
        // Nothing published; the record recorded the terminal Failed state.
        assert!(host.snapshot().await.extension(&extension_id).is_none());
        let stored = store.get(&extension_id).await.unwrap().unwrap();
        assert_eq!(stored.state, InstallationState::Failed);
        assert!(stored.last_error.is_some());
    }

    // The redacted reason is exposed to the product projection via
    // `installation_errors()` — the single source both the `Failed` projection
    // and the wire's `activation_error` are driven from.
    let errors = host.installation_errors().await.unwrap();
    assert_eq!(
        errors.len(),
        3,
        "every failed bridge collision has a recorded reason"
    );
    for extension_id in [
        "ironclaw-tool_search",
        "ironclaw-tool_describe",
        "ironclaw-tool_call",
    ] {
        assert!(
            errors
                .get(extension_id)
                .is_some_and(|reason| !reason.is_empty()),
            "the failed activation reason is keyed by extension id"
        );
    }
}

fn sample_scope() -> ironclaw_host_api::resource::ResourceScope {
    ironclaw_host_api::resource::ResourceScope {
        tenant_id: ironclaw_host_api::ids::TenantId::new("tenant-a").unwrap(),
        user_id: ironclaw_host_api::ids::UserId::new("user-a").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::ids::InvocationId::new(),
    }
}
