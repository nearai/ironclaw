//! Composition of the generic [`ExtensionHost`] (extension-runtime P2).
//!
//! Assembly only: this module constructs the generic lifecycle host with
//! concrete loaders over the host-runtime lanes and injects its snapshot
//! resolver into the dispatch chain. The lifecycle facade
//! (`extension_lifecycle.rs`) remains the durable-lifecycle owner and the
//! production caller — it drives the host at its choke points
//! (activation commit, removal, boot restore), so the active snapshot always
//! mirrors what the facade published. Durable seven-state ownership and the
//! host-owned removal order move here when the facade collapses (P6).
//!
//! Loader dispatch, by the resolved contract's runtime kind:
//! - `first_party` with a binary-assembled [`NativeExtensionFactory`] → the
//!   factory's entrypoint, with its tool adapter wrapped in the host-side
//!   reservation-settling decorator;
//! - `first_party` without a factory → the host-runtime first-party registry
//!   lane, bridged per package (the bundled registry-handler extensions,
//!   until their crates extract);
//! - `wasm` / `mcp` / `script` → the host-runtime lane binder (the lane owns
//!   reservation settlement).
//!
//! A channel-declaring extension whose channel is still served by the host
//! graph (until the P4 ingress / P5 delivery cutovers) binds the
//! transitional [`HostServedChannelBridge`] so the binding rule holds; the
//! bridge routes nothing and is deleted when the real channel adapters land.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_host::{
    BindError, DrainController, EgressFactory, ExtensionBindings, ExtensionEntrypoint,
    ExtensionHost, ExtensionHostDeps, ExtensionLoader, HookError, InstallationRecord,
    InstallationState, LoadContext, LoadedExtension, NativeExtensionFactory,
    RehydratedInstallationRecordStore, SnapshotToolResolver,
};
use ironclaw_extensions::{
    ExtensionHealthStatus, ExtensionInstallationStore, ExtensionManifest, ExtensionPackage,
    ResolvedExtensionManifest,
};
use ironclaw_host_api::{
    CapabilityId, RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest,
    RestrictedEgressResponse, ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult, VirtualPath,
};
use ironclaw_host_runtime::{ExtensionLaneToolBinder, ExtensionToolBindError};
use ironclaw_product::{
    ChannelAdapter, ChannelContext, ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope,
    VerifiedInbound,
};
use ironclaw_resources::ResourceGovernor;

/// The composed generic host plus the resolver handle composition injects
/// into the dispatch chain.
pub(crate) struct GenericExtensionHost {
    pub(crate) host: Arc<ExtensionHost>,
    pub(crate) resolver: Arc<SnapshotToolResolver>,
}

/// Inputs for [`build_generic_extension_host`]: the runtime lanes, binding
/// tables, durable state, and policy inputs the host composes over.
pub(crate) struct GenericExtensionHostParams {
    pub(crate) binder: ExtensionLaneToolBinder,
    pub(crate) native_factories: Vec<Arc<dyn NativeExtensionFactory>>,
    pub(crate) channel_adapters: Vec<(String, Arc<dyn ChannelAdapter>)>,
    pub(crate) installation_store: Arc<dyn ExtensionInstallationStore>,
    pub(crate) admin_configuration_resolver: Option<
        Arc<
            crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver,
        >,
    >,
    pub(crate) governor: Arc<dyn ResourceGovernor>,
    pub(crate) reserved_capability_ids: BTreeSet<CapabilityId>,
    pub(crate) reserved_ingress_routes: BTreeSet<String>,
    pub(crate) channel_egress_transport:
        Option<Arc<dyn ironclaw_extension_host::egress::ChannelEgressTransport>>,
}

/// Construct the generic extension host over the host-runtime lanes and
/// hydrate it from the facade's durable installation memberships.
pub(crate) async fn build_generic_extension_host(
    params: GenericExtensionHostParams,
) -> Result<GenericExtensionHost, crate::RebornBuildError> {
    let GenericExtensionHostParams {
        binder,
        native_factories,
        channel_adapters,
        installation_store,
        admin_configuration_resolver,
        governor,
        reserved_capability_ids,
        reserved_ingress_routes,
        channel_egress_transport,
    } = params;
    let factories: HashMap<String, Arc<dyn NativeExtensionFactory>> = native_factories
        .into_iter()
        .map(|factory| (factory.service().to_string(), factory))
        .collect();
    let loader = Arc::new(CompositionExtensionLoader {
        binder,
        factories,
        channel_adapters: channel_adapters.into_iter().collect(),
        governor,
        installation_store: Arc::clone(&installation_store),
    });
    // Channel hooks (and, at P5, deliver()) egress through the declared
    // [[channel.egress]] policy over the injected transport; compositions
    // built without a transport stay fail-closed.
    let egress: Arc<dyn EgressFactory> = match channel_egress_transport {
        Some(transport) => {
            Arc::new(ironclaw_extension_host::egress::TransportBackedEgressFactory::new(transport))
        }
        None => Arc::new(DenyAllEgressFactory),
    };
    let host = Arc::new(
        ExtensionHost::new(ExtensionHostDeps {
            // The facade owns durable lifecycle state in P2b; this store is
            // the host's working set, rehydrated below from the facade's
            // durable records at every boot.
            store: Arc::new(RehydratedInstallationRecordStore::default()),
            loader,
            drain: Arc::new(GenerationDrain),
            egress,
            reserved_capability_ids,
            reserved_ingress_routes,
            hook_deadline: Duration::from_secs(30),
        })
        .await,
    );

    // Hydrate every installation membership restored by the facade into the
    // snapshot. Membership is the durable runtime-presence signal; caller
    // readiness is derived separately. A failure records the host record's
    // terminal Failed state (with a redacted last_error) and must not block
    // boot.
    for installation in installation_store
        .list_installations()
        .await
        .map_err(|error| crate::RebornBuildError::InvalidConfig {
            reason: format!("extension installations could not be listed: {error}"),
        })?
    {
        let extension_id = installation.extension_id().clone();
        let Some(manifest_record) = installation_store
            .get_manifest(&extension_id)
            .await
            .map_err(|error| crate::RebornBuildError::InvalidConfig {
                reason: format!("extension manifest could not be loaded: {error}"),
            })?
        else {
            continue;
        };
        let package = ExtensionPackage::from_manifest(
            ExtensionManifest::try_from(manifest_record.manifest().clone()).map_err(|error| {
                crate::RebornBuildError::InvalidConfig {
                    reason: format!("extension manifest could not be rebuilt: {error}"),
                }
            })?,
            VirtualPath::new(format!("/system/extensions/{extension_id}")).map_err(|error| {
                crate::RebornBuildError::InvalidConfig {
                    reason: format!("extension root could not be rebuilt: {error}"),
                }
            })?,
        )
        .map_err(|error| crate::RebornBuildError::InvalidConfig {
            reason: format!("extension package could not be rebuilt: {error}"),
        })?;
        // Hosted HTTP MCP tool catalogs are live-discovered for the caller
        // and are intentionally not durable. Do not stage or publish the
        // bundled connection template at boot: the caller's idempotent
        // install/setup reconciliation must rediscover tools first.
        if crate::extension_host::mcp_discovery::is_hosted_http_mcp_package(&package) {
            continue;
        }
        // A durable installation recorded as terminally unhealthy is the
        // persisted `InstallationState::Failed` projection (overview.md §6.1):
        // it "does not auto-retry". Boot restore must honor that contract —
        // re-publishing here would silently re-run activation on every boot
        // and, on success, mask the recorded failure as active. Skip it,
        // leaving the membership installed-but-not-served and remediable via an
        // explicit caller re-activation, without ever re-attempting activation.
        // (Hosted-MCP rehydration is decided above and is unaffected.)
        if installation.health().status() == ExtensionHealthStatus::Unhealthy {
            tracing::debug!(
                extension_id = extension_id.as_str(),
                "generic extension host skips boot re-publication of a terminally failed installation"
            );
            continue;
        }
        // Deployment-owned non-secret values come only from the manifest's
        // administrator configuration projection. Compositions without that
        // projection pass no deployment configuration; installation state is
        // never a fallback configuration store.
        let config = match &admin_configuration_resolver {
            Some(admin_configuration_resolver) => admin_configuration_resolver
                .effective_non_secret_config(&extension_id)
                .await
                .map_err(|error| crate::RebornBuildError::InvalidConfig {
                    reason: format!(
                        "effective extension configuration could not be loaded: {error}"
                    ),
                })?,
            None => Vec::new(),
        };
        let record = InstallationRecord {
            extension_id: extension_id.as_str().to_string(),
            installation_id: installation.installation_id().as_str().to_string(),
            state: InstallationState::Installed,
            resolved: Arc::new(manifest_record.resolved().clone()),
            config,
            last_error: None,
        };
        if let Err(error) = host.publish_candidate(record).await {
            tracing::warn!(
                extension_id = extension_id.as_str(),
                error = %error,
                "generic extension host could not publish installation at boot"
            );
        }
    }

    let resolver = Arc::new(SnapshotToolResolver::new(host.snapshot_watch()));
    Ok(GenericExtensionHost { host, resolver })
}

/// The effective contract an activation publishes: the persisted declaration
/// with the tool set replaced by the package actually being published
/// (identical for static manifests; the ceiling-validated discovered set for
/// hosted MCP).
pub(crate) fn effective_resolved_for_package(
    base: &ResolvedExtensionManifest,
    package: &ExtensionPackage,
) -> ResolvedExtensionManifest {
    ResolvedExtensionManifest {
        tools: package.manifest.capabilities.clone(),
        ..base.clone()
    }
}

/// Loader over the host-runtime lanes and the binary-assembled native
/// factory set.
struct CompositionExtensionLoader {
    binder: ExtensionLaneToolBinder,
    factories: HashMap<String, Arc<dyn NativeExtensionFactory>>,
    /// Real channel adapters keyed by extension id, for channel-declaring
    /// extensions whose TOOLS load via the runtime lanes (P4 ingress cutover).
    /// An extension without an entry binds the transitional bridge until its
    /// adapter lands.
    channel_adapters: HashMap<String, Arc<dyn ChannelAdapter>>,
    governor: Arc<dyn ResourceGovernor>,
    installation_store: Arc<dyn ExtensionInstallationStore>,
}

#[async_trait]
impl ExtensionLoader for CompositionExtensionLoader {
    async fn load(&self, ctx: &LoadContext) -> Result<LoadedExtension, BindError> {
        // Rebuild the validated package from the resolved contract — no TOML
        // reparse; the manifest source re-checks come from the persisted
        // record.
        let extension_id = ironclaw_host_api::ExtensionId::new(&ctx.extension_id)
            .map_err(|error| load_error(format!("invalid extension id: {error}")))?;
        let source = match self
            .installation_store
            .get_manifest(&extension_id)
            .await
            .map_err(|error| load_error(format!("manifest record unavailable: {error}")))?
        {
            Some(record) => record.manifest().source,
            // No durable record (host-published test fixtures): derive the
            // least source that admits the contract's requested trust —
            // `to_internal` re-checks source-vs-trust either way.
            None => match ctx.resolved.requested_trust {
                ironclaw_host_api::RequestedTrustClass::FirstPartyRequested
                | ironclaw_host_api::RequestedTrustClass::SystemRequested => {
                    ironclaw_extensions::ManifestSource::HostBundled
                }
                _ => ironclaw_extensions::ManifestSource::InstalledLocal,
            },
        };
        let manifest_v2 = ctx
            .resolved
            .to_internal(source)
            .map_err(|error| load_error(format!("resolved contract rebuild failed: {error}")))?;
        let declares_channel = ctx.resolved.channel.is_some();

        if let ironclaw_extensions::ExtensionRuntimeV2::FirstParty { service } =
            &ctx.resolved.runtime
            && let Some(factory) = self.factories.get(service)
        {
            let entrypoint = factory.load(ctx)?;
            return Ok(LoadedExtension::new(Box::new(SettlingEntrypoint {
                inner: entrypoint,
                governor: Arc::clone(&self.governor),
            })));
        }

        let manifest = ExtensionManifest::try_from(manifest_v2)
            .map_err(|error| load_error(format!("manifest rebuild failed: {error}")))?;
        let root = VirtualPath::new(format!("/system/extensions/{}", ctx.extension_id))
            .map_err(|error| load_error(format!("extension root invalid: {error}")))?;
        let package = ExtensionPackage::from_manifest(manifest, root)
            .map_err(|error| load_error(format!("package rebuild failed: {error}")))?;
        let adapter = self
            .binder
            .bind_package(Arc::new(package))
            .map_err(|error| match error {
                ExtensionToolBindError::MissingRuntimeBackend { runtime } => load_error(format!(
                    "no runtime backend is configured for {runtime:?} extensions"
                )),
            })?;
        Ok(LoadedExtension::new(Box::new(LaneEntrypoint {
            adapter,
            // A channel-declaring extension binds its REAL channel adapter
            // when the binary/composition assembled one (the P4 inbound
            // cutover); otherwise the transitional bridge keeps the binding
            // rule satisfied until the adapter lands.
            channel: declares_channel.then(|| {
                self.channel_adapters
                    .get(&ctx.extension_id)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(HostServedChannelBridge) as Arc<dyn ChannelAdapter>)
            }),
        })))
    }
}

fn load_error(reason: String) -> BindError {
    BindError::Load { reason }
}

/// Entrypoint over a lane-bound tool adapter (wasm / mcp / script /
/// first-party-registry packages).
struct LaneEntrypoint {
    adapter: Arc<dyn ToolAdapter>,
    channel: Option<Arc<dyn ChannelAdapter>>,
}

impl ExtensionEntrypoint for LaneEntrypoint {
    fn bind(
        &self,
        _ctx: ironclaw_extension_host::BindContext,
    ) -> Result<ExtensionBindings, BindError> {
        Ok(ExtensionBindings {
            tools: Some(Arc::clone(&self.adapter)),
            channel: self.channel.clone(),
        })
    }
}

/// Wraps a native factory's entrypoint so its tool adapter settles forwarded
/// reservations (native adapters are behavior-only; the settle legs are
/// host-side).
struct SettlingEntrypoint {
    inner: Box<dyn ExtensionEntrypoint>,
    governor: Arc<dyn ResourceGovernor>,
}

impl ExtensionEntrypoint for SettlingEntrypoint {
    fn bind(
        &self,
        ctx: ironclaw_extension_host::BindContext,
    ) -> Result<ExtensionBindings, BindError> {
        let bindings = self.inner.bind(ctx)?;
        Ok(ExtensionBindings {
            tools: bindings.tools.map(|inner| {
                Arc::new(SettlingToolAdapter {
                    inner,
                    governor: Arc::clone(&self.governor),
                }) as Arc<dyn ToolAdapter>
            }),
            channel: bindings.channel,
        })
    }
}

/// Reservation settlement for native adapters: reconcile-or-release the
/// prepared reservation (or reserve fresh) around the behavior-only invoke —
/// the same legs the runtime lanes own internally.
struct SettlingToolAdapter {
    inner: Arc<dyn ToolAdapter>,
    governor: Arc<dyn ResourceGovernor>,
}

#[async_trait]
impl ToolAdapter for SettlingToolAdapter {
    async fn invoke(
        &self,
        mut call: ToolCall,
        ports: &ToolPorts<'_>,
    ) -> Result<ToolResult, ToolError> {
        let scope = call.scope.clone();
        let estimate = call.resources.estimate.clone();
        let reservation = call.resources.reservation.take();
        let reservation = match reservation {
            Some(reservation) => reservation,
            None => self
                .governor
                .reserve(scope, estimate)
                .map_err(|_| ToolError::Failed {
                    kind: ironclaw_host_api::RuntimeDispatchErrorKind::Resource,
                    safe_summary: None,
                    model_visible_cause: None,
                })?,
        };
        match self.inner.invoke(call, ports).await {
            Ok(result) => {
                let usage = ironclaw_host_api::ResourceUsage {
                    output_bytes: result.output_bytes,
                    ..ironclaw_host_api::ResourceUsage::default()
                };
                if self.governor.reconcile(reservation.id, usage).is_err() {
                    release_reservation(self.governor.as_ref(), reservation.id);
                }
                Ok(result)
            }
            Err(error) => {
                release_reservation(self.governor.as_ref(), reservation.id);
                Err(error)
            }
        }
    }
}

fn release_reservation(
    governor: &dyn ResourceGovernor,
    reservation_id: ironclaw_host_api::ResourceReservationId,
) {
    if let Err(error) = governor.release(reservation_id) {
        tracing::warn!(
            reservation_id = %reservation_id,
            error = %error,
            "failed to release native extension tool reservation"
        );
    }
}

/// Transitional channel binding for extensions whose channel surface is
/// still served by the host graph (until the P4 ingress / P5 delivery
/// cutovers). Routes nothing; deleted when the real channel adapters bind.
struct HostServedChannelBridge;

#[async_trait]
impl ChannelAdapter for HostServedChannelBridge {
    async fn activate(
        &self,
        _ctx: &ChannelContext<'_>,
        _egress: &dyn RestrictedEgress,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn cleanup(
        &self,
        _ctx: &ChannelContext<'_>,
        _egress: &dyn RestrictedEgress,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    fn inbound(&self, _request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        Err(ChannelError::Unsupported)
    }

    async fn deliver(
        &self,
        _envelope: OutboundEnvelope,
        _egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        Err(ChannelError::Unsupported)
    }
}

/// In-flight work completes on the generation `Arc` it resolved; there is no
/// additional drain source until the delivery coordinator (P5).
struct GenerationDrain;

#[async_trait]
impl DrainController for GenerationDrain {
    async fn drain(&self, _extension_id: &str, _deadline: Duration) -> Result<(), HookError> {
        Ok(())
    }
}

/// Fail-closed factory for paths built without a channel egress transport
/// (override/test compositions). Production serve paths wire the real
/// `TransportBackedEgressFactory` over the host runtime egress.
struct DenyAllEgressFactory;

impl EgressFactory for DenyAllEgressFactory {
    fn egress_for_channel(
        &self,
        _extension_id: &str,
        _installation_id: &str,
        _declared: &[ironclaw_host_api::ChannelEgressDescriptor],
    ) -> Arc<dyn RestrictedEgress> {
        Arc::new(DenyAllRestrictedEgress)
    }
}

struct DenyAllRestrictedEgress;

#[async_trait]
impl RestrictedEgress for DenyAllRestrictedEgress {
    async fn send(
        &self,
        _request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Err(RestrictedEgressError::PolicyDenied)
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_extension_host::test_support::{FakeEntrypoint, FakeToolAdapter};
    use ironclaw_extensions::{
        ExtensionHealthMessage, ExtensionHealthSnapshot, ExtensionHealthStatus,
        ExtensionInstallation, ExtensionInstallationId, ExtensionManifestRecord,
        ExtensionManifestRef, ExtensionRegistry, FilesystemExtensionInstallationStore,
        MANIFEST_SCHEMA_VERSION, ManifestSource,
    };
    use ironclaw_filesystem::DiskFilesystem;
    use ironclaw_host_api::ids::ExtensionId;
    use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
    use ironclaw_processes::ProcessServices;
    use ironclaw_resources::InMemoryResourceGovernor;

    use super::*;
    use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

    const FIXTURE_SERVICE: &str = "h5_fixture_host";

    fn fixture_manifest_toml(id: &str) -> String {
        format!(
            r#"
schema_version = "{schema}"
id = "{id}"
name = "H5 hydration fixture"
version = "0.1.0"
description = "boot hydration fixture extension"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "{service}"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.echo"
description = "Echoes input"
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/echo.input.json"
"#,
            schema = MANIFEST_SCHEMA_VERSION,
            id = id,
            service = FIXTURE_SERVICE,
        )
    }

    fn hosted_mcp_fixture_manifest_toml(id: &str) -> String {
        format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "Hosted MCP hydration fixture"
version = "0.1.0"
description = "boot hydration must not publish a stale MCP template"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.com/mcp"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.template"
description = "Connection template, not a discovered tool"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/template.input.json"
"#,
        )
    }

    /// A native factory whose entrypoint binds a no-op tool adapter — the
    /// first_party loader branch, no runtime lane required.
    struct FixtureNativeFactory;

    impl NativeExtensionFactory for FixtureNativeFactory {
        fn service(&self) -> &str {
            FIXTURE_SERVICE
        }

        fn load(
            &self,
            _ctx: &LoadContext,
        ) -> Result<Box<dyn ironclaw_extension_host::ExtensionEntrypoint>, BindError> {
            Ok(Box::new(FakeEntrypoint {
                bindings: ExtensionBindings {
                    tools: Some(Arc::new(FakeToolAdapter)),
                    channel: None,
                },
            }))
        }
    }

    async fn seed_installation(store: &FilesystemExtensionInstallationStore, id: &str) {
        seed_installation_with_manifest(store, id, fixture_manifest_toml(id)).await;
    }

    async fn seed_installation_with_manifest(
        store: &FilesystemExtensionInstallationStore,
        id: &str,
        manifest_toml: String,
    ) {
        let record = ExtensionManifestRecord::from_toml(
            manifest_toml,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("fixture manifest resolves");
        let extension_id = ExtensionId::new(id).expect("extension id");
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(id.to_string()).expect("installation id"),
                    extension_id.clone(),
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    chrono::Utc::now(),
                    ironclaw_extensions::InstallationOwner::user(
                        ironclaw_host_api::UserId::new(format!("user:{id}"))
                            .expect("fixture user id"),
                    ),
                )
                .expect("installation record"),
            )
            .await
            .expect("persist installation");
    }

    fn test_binder() -> ExtensionLaneToolBinder {
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(DiskFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
        )
        .extension_lane_tool_binder()
    }

    /// Durable installation memberships hydrate into the generic host's
    /// runtime records at boot. Readiness remains derived by the lifecycle
    /// caller rather than persisted as a parallel activation state.
    #[tokio::test]
    async fn boot_hydration_loads_each_durable_installation_membership() {
        let store = Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
        seed_installation(&store, "h5-first").await;
        seed_installation(&store, "h5-second").await;

        let generic = build_generic_extension_host(GenericExtensionHostParams {
            binder: test_binder(),
            native_factories: vec![Arc::new(FixtureNativeFactory)],
            channel_adapters: Vec::new(),
            installation_store: Arc::clone(&store) as Arc<dyn ExtensionInstallationStore>,
            admin_configuration_resolver: None,
            governor: Arc::new(InMemoryResourceGovernor::new()),
            reserved_capability_ids: BTreeSet::new(),
            reserved_ingress_routes: BTreeSet::new(),
            channel_egress_transport: None,
        })
        .await
        .expect("generic host builds");

        let snapshot = generic.host.snapshot().await;
        assert!(
            snapshot.extension("h5-first").is_some(),
            "the first durable membership must hydrate into the first published generation"
        );
        assert!(
            snapshot
                .resolve_tool(&CapabilityId::new("h5-first.echo").expect("capability id"))
                .is_some(),
            "the first hydrated extension capability must resolve from the snapshot"
        );
        assert!(
            snapshot.extension("h5-second").is_some(),
            "the second durable membership must hydrate independently"
        );
    }

    #[tokio::test]
    async fn boot_hydration_does_not_publish_hosted_mcp_without_live_discovery() {
        let store = Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
        seed_installation_with_manifest(
            &store,
            "hosted-mcp",
            hosted_mcp_fixture_manifest_toml("hosted-mcp"),
        )
        .await;

        let generic = build_generic_extension_host(GenericExtensionHostParams {
            binder: test_binder(),
            native_factories: Vec::new(),
            channel_adapters: Vec::new(),
            installation_store: Arc::clone(&store) as Arc<dyn ExtensionInstallationStore>,
            admin_configuration_resolver: None,
            governor: Arc::new(InMemoryResourceGovernor::new()),
            reserved_capability_ids: BTreeSet::new(),
            reserved_ingress_routes: BTreeSet::new(),
            channel_egress_transport: None,
        })
        .await
        .expect("generic host builds");

        let snapshot = generic.host.snapshot().await;
        assert!(
            snapshot.extension("hosted-mcp").is_none(),
            "boot must not claim a hosted MCP is active before live tool discovery"
        );
        assert!(
            snapshot
                .resolve_tool(&CapabilityId::new("hosted-mcp.template").expect("capability id"))
                .is_none(),
            "the bundled connection template must never be exposed as a discovered MCP tool"
        );
    }

    /// A durable installation persisted as terminally unhealthy is the §6.1
    /// `InstallationState::Failed` projection ("does not auto-retry"). Boot
    /// restore must not re-publish or re-activate it — that would re-run
    /// activation every boot and, since the fixture factory activates cleanly,
    /// mask the recorded failure as active. A healthy sibling still hydrates.
    #[tokio::test]
    async fn boot_hydration_skips_terminally_failed_installation() {
        let store = Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
        seed_installation(&store, "h5-healthy").await;
        seed_installation(&store, "h5-failed").await;
        // Persist the durable terminal-failure health for one installation. In
        // production the lifecycle facade records this on an activation failure;
        // here it stands in for a boot that follows a prior failed activation.
        store
            .update_health(
                &ExtensionInstallationId::new("h5-failed".to_string()).expect("installation id"),
                ExtensionHealthSnapshot::new(
                    ExtensionHealthStatus::Unhealthy,
                    Some(ExtensionHealthMessage::new("activation failed")),
                    chrono::Utc::now(),
                ),
            )
            .await
            .expect("persist terminal-failure health");

        let generic = build_generic_extension_host(GenericExtensionHostParams {
            binder: test_binder(),
            native_factories: vec![Arc::new(FixtureNativeFactory)],
            channel_adapters: Vec::new(),
            installation_store: Arc::clone(&store) as Arc<dyn ExtensionInstallationStore>,
            admin_configuration_resolver: None,
            governor: Arc::new(InMemoryResourceGovernor::new()),
            reserved_capability_ids: BTreeSet::new(),
            reserved_ingress_routes: BTreeSet::new(),
            channel_egress_transport: None,
        })
        .await
        .expect("generic host builds");

        let snapshot = generic.host.snapshot().await;
        assert!(
            snapshot.extension("h5-healthy").is_some(),
            "a healthy durable membership still restores at boot"
        );
        assert!(
            snapshot.extension("h5-failed").is_none(),
            "a terminally failed installation must not be re-published/re-activated at boot"
        );
        // Skipped, not re-attempted: the host holds no working activation record
        // for the failed installation (a re-attempt would have either activated
        // it into the snapshot above or recorded a fresh Failed error here).
        assert!(
            !generic
                .host
                .installation_errors()
                .await
                .expect("installation errors")
                .contains_key("h5-failed"),
            "boot restore must not re-run activation for a terminally failed installation"
        );
    }
}
