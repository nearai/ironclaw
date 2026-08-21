//! Generic [`ExtensionHost`] assembly (extension-runtime P2).
//!
//! Assembly only: this module constructs the generic lifecycle host with
//! concrete loaders over the host-runtime lanes and injects its snapshot
//! resolver into the dispatch chain. The durable lifecycle manager in
//! [`crate::product_lifecycle`] drives this host at install, activation,
//! removal, and restore choke points, so the active snapshot mirrors durable
//! lifecycle state.
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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelDelivery, ChannelError, ChannelIngress, ChannelReply, ChannelSurfaces, DeliveryReport,
    InboundOutcome, OutboundEnvelope, VerifiedInbound,
};
use ironclaw_extension_contracts::extension::ExtensionHostAssemblyConfig;
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
    ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult,
};
use ironclaw_extension_registry::{
    ExtensionInstallationError, ExtensionInstallationStorePort, ExtensionManifest,
    ExtensionPackage, ResolvedExtensionManifest,
};
use ironclaw_host_api::path::VirtualPath;
use ironclaw_host_api::{dispatch::RuntimeDispatchErrorKind, ids::ExtensionId};
use ironclaw_host_runtime::{ExtensionLaneToolBinder, ExtensionToolBindError};
use ironclaw_resources::ResourceGovernor;

use crate::{
    BindError, ChannelConfigError, ChannelConfigService, DrainController, EgressFactory,
    ExtensionBindings, ExtensionEntrypoint, ExtensionHost, ExtensionHostDeps, ExtensionLoader,
    HookError, InstallationRecord, LinkedSessionStore, LoadContext, LoadedExtension,
    NativeExtensionFactory, RehydratedInstallationRecordStore, SnapshotToolResolver,
};

/// The composed generic host plus the resolver handle composition injects
/// into the dispatch chain.
pub struct GenericExtensionHost {
    pub host: Arc<ExtensionHost>,
    pub resolver: Arc<SnapshotToolResolver>,
}

/// Inputs for [`build_generic_extension_host`]: the runtime lanes, binding
/// tables, durable state, and policy inputs the host composes over.
pub struct GenericExtensionHostParams {
    pub binder: ExtensionLaneToolBinder,
    pub native_factories: Vec<Arc<dyn NativeExtensionFactory>>,
    pub channel_adapters: Vec<(ExtensionId, ChannelSurfaces)>,
    pub installation_store: Arc<dyn ExtensionInstallationStorePort>,
    pub boot_installations: Vec<InstallationRecord>,
    pub governor: Arc<dyn ResourceGovernor>,
    pub assembly: ExtensionHostAssemblyConfig,
    pub channel_egress_transport: Option<Arc<dyn crate::egress::ChannelEgressTransport>>,
    /// Linked-account custody. A deployment that wires none passes
    /// [`LinkedSessionStore::unavailable`] — the fail-closed store, chosen at
    /// the composition boundary rather than defaulted here, so this type
    /// cannot claim custody is optional when production always supplies it.
    pub linked_sessions: Arc<LinkedSessionStore>,
    /// The per-extension linked-account resolver factory; the unwired shape
    /// is the explicit `UnavailableLinkedAccountResolution`.
    pub linked_accounts: Arc<dyn crate::linked_account_resolution::LinkedAccountResolution>,
    /// Admin-configuration reads for load-time factory construction.
    ///
    /// Required, like its two custody siblings: production always supplies it,
    /// so an `Option` here described a deployment that does not exist and hid
    /// the fail-closed choice inside this crate. A deployment without admin
    /// configuration passes `None` at the composition boundary instead, where
    /// the choice is visible.
    pub admin_secrets: Option<Arc<crate::ChannelConfigService>>,
}

#[derive(Debug, thiserror::Error)]
pub enum BootInstallationRecordsError {
    #[error("extension installations could not be listed: {source}")]
    ListInstallations {
        #[source]
        source: ExtensionInstallationError,
    },
    #[error("extension manifest could not be loaded: {source}")]
    LoadManifest {
        #[source]
        source: ExtensionInstallationError,
    },
    #[error("effective extension configuration could not be loaded: {source}")]
    EffectiveChannelConfig {
        #[source]
        source: ChannelConfigError,
    },
    #[error("extension channel config could not be loaded: {source}")]
    ChannelConfig {
        #[source]
        source: ExtensionInstallationError,
    },
}

/// Build the host-owned installation records for every durable enabled
/// installation. The caller owns boot fail policy; missing manifests are
/// skipped to preserve the previous composition behavior.
pub async fn boot_installation_records(
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    channel_config: Option<&Arc<ChannelConfigService>>,
) -> Result<Vec<InstallationRecord>, BootInstallationRecordsError> {
    let mut records = Vec::new();
    for installation in installation_store
        .list_installations()
        .await
        .map_err(|source| BootInstallationRecordsError::ListInstallations { source })?
    {
        let extension_id = installation.extension_id().clone();
        let Some(manifest_record) = installation_store
            .get_manifest(&extension_id)
            .await
            .map_err(|source| BootInstallationRecordsError::LoadManifest { source })?
        else {
            continue;
        };
        // Every durable enabled installation stages into the host; whether a
        // package has anything to publish is a property of its own declared
        // capabilities, derived where the tool adapter is actually bound
        // (`CompositionExtensionLoader::load`'s `declares_tools`), not a
        // separate flag read here. A package that has not yet resolved a real
        // capability set stages and attempts activation like any other
        // record; the binding-rule check downstream fails that specific
        // activation closed (no publish) without this boot list needing to
        // know why.
        records.push(boot_installation_record(
            installation.installation_id().as_str(),
            &extension_id,
            manifest_record.resolved(),
            effective_channel_config(installation_store, channel_config, &extension_id).await?,
        ));
    }
    Ok(records)
}

async fn effective_channel_config(
    _installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    channel_config: Option<&Arc<ChannelConfigService>>,
    extension_id: &ironclaw_host_api::ids::ExtensionId,
) -> Result<Vec<(String, String)>, BootInstallationRecordsError> {
    match channel_config {
        Some(channel_config) => channel_config
            .effective_non_secret_config(extension_id)
            .await
            .map_err(|source| BootInstallationRecordsError::EffectiveChannelConfig { source }),
        None => Ok(Vec::new()),
    }
}

fn boot_installation_record(
    installation_id: &str,
    extension_id: &ironclaw_host_api::ids::ExtensionId,
    resolved: &ResolvedExtensionManifest,
    config: Vec<(String, String)>,
) -> InstallationRecord {
    InstallationRecord {
        extension_id: extension_id.as_str().to_string(),
        installation_id: installation_id.to_string(),
        state: ironclaw_extension_contracts::state::InstallationState::Installed,
        resolved: Arc::new(resolved.clone()),
        config,
        last_error: None,
    }
}

/// Construct the generic extension host over the host-runtime lanes and
/// hydrate it from the service's durable installation state (every `Enabled`
/// installation activates into the first published generation).
pub async fn build_generic_extension_host(
    params: GenericExtensionHostParams,
) -> GenericExtensionHost {
    let GenericExtensionHostParams {
        binder,
        native_factories,
        channel_adapters,
        installation_store,
        boot_installations,
        governor,
        assembly,
        channel_egress_transport,
        linked_sessions,
        linked_accounts,
        admin_secrets,
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
        Some(transport) => Arc::new(crate::egress::TransportBackedEgressFactory::new(transport)),
        None => Arc::new(DenyAllEgressFactory),
    };
    let host = Arc::new(
        ExtensionHost::new(ExtensionHostDeps {
            // The service owns durable lifecycle state in P2b; this store is
            // the host's working set, rehydrated below from the service's
            // durable records at every boot.
            store: Arc::new(RehydratedInstallationRecordStore::default()),
            loader,
            drain: Arc::new(GenerationDrain),
            egress,
            reserved_capability_ids: assembly.reserved_capability_ids,
            reserved_ingress_routes: assembly.reserved_ingress_routes,
            hook_deadline: assembly.hook_deadline,
            linked_sessions,
            linked_accounts,
            admin_secrets,
        })
        .await,
    );

    // Hydrate: composition supplies every boot-active installation record; the
    // generic host owns publication. A failure records the host record's
    // terminal Failed state (with a redacted last_error) and must not block
    // boot; the durable installation stays Enabled, so the extension projects
    // `Failed` until a successful (re)activation clears it.
    for record in boot_installations {
        let extension_id = record.extension_id.clone();
        if let Err(error) = host.install(record).await {
            tracing::warn!(
                extension_id = extension_id.as_str(),
                error = %error,
                "generic extension host could not stage installation at boot"
            );
            continue;
        }
        if let Err(error) = host.activate(extension_id.as_str()).await {
            tracing::warn!(
                extension_id = extension_id.as_str(),
                error = %error,
                "generic extension host could not activate installation at boot"
            );
        }
    }

    let resolver = Arc::new(SnapshotToolResolver::new(host.snapshot_watch()));
    GenericExtensionHost { host, resolver }
}

/// The effective contract an activation publishes: the persisted declaration
/// with the tool set replaced by the package actually being published
/// (identical for static manifests; the ceiling-validated discovered set for
/// hosted MCP).
pub fn effective_resolved_for_package(
    base: &ResolvedExtensionManifest,
    package: &ExtensionPackage,
) -> ResolvedExtensionManifest {
    let mut resolved = ResolvedExtensionManifest {
        tools: package.manifest.capabilities.clone(),
        ..base.clone()
    };
    // Discovered per-tool schemas must be persisted for both the virtual
    // (user-registered, remote-only) package shape and the materialized
    // host-bundled shape whose descriptors are inline-dynamic (hosted MCP
    // providers built by `package_with_discovered_hosted_mcp_tools`). The
    // schema mode itself is not persisted on `ResolvedExtensionManifest` —
    // `rebuild_package_from_resolved` re-derives which constructor to use
    // from whether this map ends up non-empty.
    let captures_dynamic_schemas = package.root_binding
        == ironclaw_extension_registry::PackageRootBinding::Virtual
        || (matches!(
            package.root_binding,
            ironclaw_extension_registry::PackageRootBinding::Materialized(_)
        ) && package.descriptor_schema_mode
            == ironclaw_extension_registry::CapabilityDescriptorSchemaMode::InlineDynamic);
    if captures_dynamic_schemas && let Some(mcp) = resolved.mcp.as_mut() {
        mcp.dynamic_input_schemas = package
            .capabilities
            .iter()
            .map(|descriptor| {
                (
                    descriptor.id.as_str().to_string(),
                    descriptor.parameters_schema.clone(),
                )
            })
            .collect();
    }
    resolved
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
    channel_adapters: HashMap<ExtensionId, ChannelSurfaces>,
    governor: Arc<dyn ResourceGovernor>,
    installation_store: Arc<dyn ExtensionInstallationStorePort>,
}

#[async_trait]
impl ExtensionLoader for CompositionExtensionLoader {
    async fn load(&self, ctx: &LoadContext) -> Result<LoadedExtension, BindError> {
        // Rebuild the validated package from the resolved contract — no TOML
        // reparse; the manifest source re-checks come from the persisted
        // record.
        let extension_id = ironclaw_host_api::ids::ExtensionId::new(&ctx.extension_id)
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
                ironclaw_host_api::trust::RequestedTrustClass::FirstPartyRequested
                | ironclaw_host_api::trust::RequestedTrustClass::SystemRequested => {
                    ironclaw_extension_registry::ManifestSource::HostBundled
                }
                _ => ironclaw_extension_registry::ManifestSource::InstalledLocal,
            },
        };
        let manifest_v2 = ctx
            .resolved
            .to_internal(source)
            .map_err(|error| load_error(format!("resolved contract rebuild failed: {error}")))?;
        // Pure capability count, mirroring `check_binding`'s own
        // declared-tools test (entrypoint.rs) — the two must stay in
        // lockstep or activation fails the binding-rule check before
        // publish. A resolved contract with a genuinely empty declared-tool
        // list has no tool surface at all (e.g. a channel-only first-party
        // extension); the lane binder must not be asked to produce an
        // adapter for it, since the lane binder always succeeds with a
        // (possibly empty-routed) tool adapter, which would then fail the
        // binding-rule check with `UndeclaredToolAdapter` even though
        // nothing is actually bound. A package whose only declared tool is
        // an internal, never-model-visible connection-management entry
        // still counts as declaring tools by this same count — that
        // package's model-visible catalog readiness is validated
        // separately, downstream, at the binding-rule check.
        let declares_tools = !ctx.resolved.tools.is_empty();

        if let ironclaw_extension_registry::ExtensionRuntimeV2::FirstParty { service } =
            &ctx.resolved.runtime
            && let Some(factory) = self.factories.get(service)
        {
            let entrypoint = factory.load(ctx).await?;
            return Ok(LoadedExtension::new(Box::new(SettlingEntrypoint {
                inner: entrypoint,
                governor: Arc::clone(&self.governor),
            })));
        }

        let manifest = ExtensionManifest::try_from(manifest_v2)
            .map_err(|error| load_error(format!("manifest rebuild failed: {error}")))?;
        let package = rebuild_package_from_resolved(manifest, &ctx.resolved, &ctx.extension_id)
            .map_err(load_error)?;
        let adapter = if declares_tools {
            Some(
                self.binder
                    .bind_package(Arc::new(package))
                    .map_err(|error| match error {
                        ExtensionToolBindError::MissingRuntimeBackend { runtime } => load_error(
                            format!("no runtime backend is configured for {runtime:?} extensions"),
                        ),
                    })?,
            )
        } else {
            None
        };
        Ok(LoadedExtension::new(Box::new(LaneEntrypoint {
            adapter,
            // A channel-declaring extension binds its REAL channel adapter
            // when the binary/composition assembled one (the P4 inbound
            // cutover); otherwise the transitional bridge keeps the binding
            // rule satisfied until the adapter lands.
            channel: match (
                &ctx.resolved.channel,
                self.channel_adapters.get(&extension_id),
            ) {
                (Some(_), Some(surfaces)) => surfaces.clone(),
                // Until a real adapter is assembled, the bridge stands in —
                // but only for the halves this manifest declares, because the
                // per-axis binding rule now checks declaration against code.
                // A bridge with a half nobody declared would fail activation,
                // which is the rule doing its job.
                (Some(channel), None) => host_served_bridge(channel),
                (None, _) => ChannelSurfaces::default(),
            },
        })))
    }
}

fn load_error(reason: String) -> BindError {
    BindError::Load { reason }
}

/// Use the package root persisted with the resolved contract when one is
/// present. Fall back to fabricating `/system/extensions/{id}` for rows
/// persisted before `ResolvedExtensionManifest::root` existed (back-compat
/// with pre-existing installations) — this is the ONLY reason the fallback
/// exists; do not remove it.
pub(crate) fn rebuild_package_from_resolved(
    manifest: ExtensionManifest,
    resolved: &ResolvedExtensionManifest,
    extension_id: &str,
) -> Result<ExtensionPackage, String> {
    use ironclaw_extension_registry::PackageRootBinding;
    match &resolved.root_binding {
        PackageRootBinding::Materialized(root) => {
            // A materialized host-bundled package that persisted discovered
            // dynamic schemas (hosted MCP providers such as `nearai`) must be
            // rebuilt through the inline-dynamic constructor: `from_manifest`
            // hardcodes `ManifestRefs`, which routes descriptor schemas
            // through filesystem `$ref` reads that were never written for
            // discovered tools. Whether the persisted map is non-empty is the
            // only signal available — `descriptor_schema_mode` itself is not
            // persisted on `ResolvedExtensionManifest`.
            let dynamic_schemas = resolved
                .mcp
                .as_ref()
                .map(|mcp| &mcp.dynamic_input_schemas)
                .filter(|schemas| !schemas.is_empty());
            match dynamic_schemas {
                Some(schemas) => {
                    let capabilities = descriptors_from_dynamic_schemas(&manifest, schemas)?;
                    ExtensionPackage::from_host_bundled_manifest_with_inline_dynamic_schemas(
                        manifest,
                        root.clone(),
                        None,
                        capabilities,
                    )
                }
                None => ExtensionPackage::from_manifest(manifest, root.clone()),
            }
        }
        PackageRootBinding::FabricateOnLoad => {
            let root = VirtualPath::new(format!("/system/extensions/{extension_id}"))
                .map_err(|error| format!("extension root invalid: {error}"))?;
            ExtensionPackage::from_manifest(manifest, root)
        }
        PackageRootBinding::Virtual => {
            let schemas = resolved
                .mcp
                .as_ref()
                .map(|mcp| &mcp.dynamic_input_schemas)
                .ok_or_else(|| "virtual package lacks MCP catalog metadata".to_string())?;
            // A virtual (user-registered hosted MCP) package whose discovery
            // has not run yet — including the pending/auth-checkpoint states
            // `HostedMcpPreparationService::sync_lifecycle_package` rebuilds
            // through while OAuth/bearer setup is still outstanding —
            // persists an EMPTY dynamic-schema map while its manifest still
            // carries the discovery-placeholder capability. That is a
            // legitimate pre-discovery state, not a persistence defect: the
            // rebuilt in-memory package declares no model-visible capability
            // until that later step records one, so it is never published for
            // callable dispatch and a placeholder null-schema descriptor is
            // safe here — mirrors the
            // Materialized branch's `!schemas.is_empty()` gate above and
            // `hosted_mcp_manifest::available_package`'s tolerant
            // construction for the same not-yet-discovered state. Fail
            // closed only once discovery has recorded at least one schema
            // and a declared capability is STILL missing one — that
            // remains the real persistence defect the fail-closed check
            // exists to catch.
            let capabilities = if schemas.is_empty() {
                placeholder_descriptors_from_manifest(&manifest)
            } else {
                descriptors_from_dynamic_schemas(&manifest, schemas)?
            };
            ExtensionPackage::from_virtual_manifest(manifest, None, capabilities)
        }
    }
    .map_err(|error| format!("package rebuild failed: {error}"))
}

/// Build capability descriptors for a manifest whose per-tool `input_schema`
/// values come from a discovered-dynamic-schema map rather than the
/// manifest's own `$ref` projections. Fails closed: a capability declared in
/// the manifest with no admitted schema is a persistence defect, not a
/// reason to publish a `null`-schema descriptor.
fn descriptors_from_dynamic_schemas(
    manifest: &ExtensionManifest,
    schemas: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<Vec<ironclaw_host_api::capability::CapabilityDescriptor>, String> {
    manifest
        .capabilities
        .iter()
        .map(|capability| {
            let parameters_schema =
                schemas
                    .get(capability.id.as_str())
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "no discovered input schema recorded for capability {}",
                            capability.id
                        )
                    })?;
            Ok(ironclaw_host_api::capability::CapabilityDescriptor {
                id: capability.id.clone(),
                provider: manifest.id.clone(),
                runtime: manifest.runtime.kind(),
                trust_ceiling: manifest.descriptor_trust_default,
                description: capability.description.clone(),
                parameters_schema,
                effects: capability.effects.clone(),
                default_permission: capability.default_permission,
                runtime_credentials: capability.runtime_credentials.clone(),
                network_targets: capability.network_targets.clone(),
                max_egress_bytes: capability.max_egress_bytes,
                resource_profile: capability.resource_profile.clone(),
                origin_gate_matrix: capability.origin_gate_matrix.clone(),
                standard_op: capability.standard_op,
            })
        })
        .collect()
}

/// Build capability descriptors for a virtual hosted-MCP manifest whose
/// dynamic-schema map is still empty (no discovery has run yet). Same shape
/// as `descriptors_from_dynamic_schemas` but never fails closed — every
/// capability gets a `null` parameters schema, matching
/// `hosted_mcp_manifest::available_package`'s tolerant construction for this
/// exact pre-discovery state.
fn placeholder_descriptors_from_manifest(
    manifest: &ExtensionManifest,
) -> Vec<ironclaw_host_api::capability::CapabilityDescriptor> {
    manifest
        .capabilities
        .iter()
        .map(
            |capability| ironclaw_host_api::capability::CapabilityDescriptor {
                id: capability.id.clone(),
                provider: manifest.id.clone(),
                runtime: manifest.runtime.kind(),
                trust_ceiling: manifest.descriptor_trust_default,
                description: capability.description.clone(),
                parameters_schema: serde_json::Value::Null,
                effects: capability.effects.clone(),
                default_permission: capability.default_permission,
                runtime_credentials: capability.runtime_credentials.clone(),
                network_targets: capability.network_targets.clone(),
                max_egress_bytes: capability.max_egress_bytes,
                resource_profile: capability.resource_profile.clone(),
                origin_gate_matrix: capability.origin_gate_matrix.clone(),
                standard_op: capability.standard_op,
            },
        )
        .collect()
}

#[cfg(test)]
fn resolve_package_root(
    persisted: Option<&VirtualPath>,
    extension_id: &str,
) -> Result<VirtualPath, BindError> {
    match persisted {
        Some(root) => Ok(root.clone()),
        None => VirtualPath::new(format!("/system/extensions/{extension_id}"))
            .map_err(|error| load_error(format!("extension root invalid: {error}"))),
    }
}

/// Entrypoint over a lane-bound tool adapter (wasm / mcp / script /
/// first-party-registry packages). `adapter` is `None` for a resolved
/// contract that declares no tool surface at all (e.g. a channel-only
/// first-party extension) — the lane binder is never invoked for those, so
/// there is nothing to report as bound.
struct LaneEntrypoint {
    adapter: Option<Arc<dyn ToolAdapter>>,
    channel: ChannelSurfaces,
}

impl ExtensionEntrypoint for LaneEntrypoint {
    fn bind(&self, _ctx: crate::BindContext) -> Result<ExtensionBindings, BindError> {
        Ok(ExtensionBindings {
            tools: self.adapter.clone(),
            channel: self.channel.clone(),
            // A runtime lane cannot carry a device-link handshake: the login is
            // bound to a live connection the package owns across calls, which
            // is exactly what a per-invocation lane does not have. A
            // device-link extension binds through a native entrypoint.
            device_link: None,
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
    fn bind(&self, ctx: crate::BindContext) -> Result<ExtensionBindings, BindError> {
        let bindings = self.inner.bind(ctx)?;
        Ok(ExtensionBindings {
            tools: bindings.tools.map(|inner| {
                Arc::new(SettlingToolAdapter {
                    inner,
                    governor: Arc::clone(&self.governor),
                }) as Arc<dyn ToolAdapter>
            }),
            channel: bindings.channel,
            // Passed through undecorated: the settle legs this wrapper adds are
            // resource reservations on a capability dispatch, and a device-link
            // call is neither.
            device_link: bindings.device_link,
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
            None => self.governor.reserve(scope, estimate).map_err(|error| {
                // The tool boundary deliberately exposes only the stable
                // resource failure class. Keep the governor's cause in the
                // host log before it is redacted from the adapter result.
                tracing::warn!(%error, "native extension tool resource reservation failed");
                ToolError::Rejected {
                    kind: RuntimeDispatchErrorKind::Resource,
                    diagnostic: None,
                    detail: None,
                }
            })?,
        };
        match self.inner.invoke(call, ports).await {
            Ok(result) => {
                let usage = ironclaw_host_api::resource::ResourceUsage {
                    output_bytes: result.output_bytes,
                    ..ironclaw_host_api::resource::ResourceUsage::default()
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
    reservation_id: ironclaw_host_api::ids::ResourceReservationId,
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

/// Bind the bridge to exactly the halves this manifest declares.
///
/// The per-axis binding rule checks declaration against code, so a blanket
/// bridge implementing everything would fail activation for any channel that
/// declares only some halves — and a bridge implementing nothing would fail
/// for any channel that declares any. Deriving the set from the descriptor is
/// the only shape that stays correct as manifests differ.
fn host_served_bridge(
    channel: &ironclaw_extension_contracts::channel::ChannelDescriptor,
) -> ChannelSurfaces {
    let bridge = Arc::new(HostServedChannelBridge);
    let mut surfaces = ChannelSurfaces::default();
    let expected = crate::entrypoint::channel_half_expectations(channel);
    if expected.ingress {
        surfaces = surfaces.with_ingress(bridge.clone());
    }
    if expected.reply {
        surfaces = surfaces.with_reply(bridge.clone());
    }
    if expected.delivery {
        surfaces = surfaces.with_delivery(bridge);
    }
    surfaces
}

#[async_trait]
impl ChannelIngress for HostServedChannelBridge {
    async fn receive(
        &self,
        _request: VerifiedInbound<'_>,
        _egress: &dyn RestrictedEgress,
    ) -> Result<InboundOutcome, ChannelError> {
        Err(ChannelError::Unsupported)
    }
}

#[async_trait]
impl ChannelReply for HostServedChannelBridge {
    async fn send_reply(
        &self,
        _envelope: OutboundEnvelope,
        _egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        Err(ChannelError::Unsupported)
    }
}

#[async_trait]
impl ChannelDelivery for HostServedChannelBridge {
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
        _declared: &[ironclaw_extension_contracts::channel::ChannelEgressDescriptor],
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
    use std::collections::BTreeSet;

    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_extension_contracts::extension::ExtensionHostAssemblyConfig;
    use ironclaw_extension_registry::{
        ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStore,
        ExtensionManifestRecord, ExtensionManifestRef, ExtensionRegistry, MANIFEST_SCHEMA_VERSION,
        ManifestSource,
    };
    use ironclaw_filesystem::DiskFilesystem;
    use ironclaw_host_api::ids::{CapabilityId, ExtensionId};
    use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
    use ironclaw_processes::ProcessServices;
    use ironclaw_resources::InMemoryResourceGovernor;

    use super::*;
    use crate::test_support::{FakeEntrypoint, FakeToolAdapter};

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

    /// A native factory whose entrypoint binds a no-op tool adapter — the
    /// first_party loader branch, no runtime lane required.
    struct FixtureNativeFactory;

    #[async_trait]
    impl NativeExtensionFactory for FixtureNativeFactory {
        fn service(&self) -> &str {
            FIXTURE_SERVICE
        }

        async fn load(
            &self,
            _ctx: &LoadContext,
        ) -> Result<Box<dyn crate::ExtensionEntrypoint>, BindError> {
            Ok(Box::new(FakeEntrypoint {
                bindings: ExtensionBindings {
                    tools: Some(Arc::new(FakeToolAdapter)),
                    channel: ChannelSurfaces::default(),
                    device_link: None,
                },
            }))
        }
    }

    async fn seed_installation(store: &ExtensionInstallationStore, id: &str) {
        let record = ExtensionManifestRecord::from_toml(
            fixture_manifest_toml(id),
            ManifestSource::HostBundled,
            &ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog"),
            None,
            &ironclaw_extension_registry::default_host_api_contract_registry().expect("contracts"),
            None,
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
                    ironclaw_extension_registry::InstallationOwner::Tenant,
                )
                .expect("installation record"),
            )
            .await
            .expect("persist installation");
    }

    /// A `wasm` / `third_party` fixture — deliberately never eligible for
    /// first-party trust, used to pin the *other* arm of the no-durable-
    /// record classification fallback below.
    fn third_party_wasm_fixture_manifest_toml(id: &str) -> String {
        format!(
            r#"
schema_version = "{schema}"
id = "{id}"
name = "H5 third-party fallback fixture"
version = "0.1.0"
description = "classification fallback fixture extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

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
        )
    }

    /// Builds a `LoadContext` for `toml`, without persisting anything in an
    /// installation store — the resulting `resolved` is exactly what
    /// `CompositionExtensionLoader::load` sees when there is no durable
    /// record for the extension id.
    async fn load_context_for(id: &str, toml: &str) -> LoadContext {
        let record = ExtensionManifestRecord::from_toml(
            toml.to_string(),
            ManifestSource::HostBundled,
            &ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog"),
            None,
            &ironclaw_extension_registry::default_host_api_contract_registry().expect("contracts"),
            None,
        )
        .expect("fixture manifest resolves");
        LoadContext {
            extension_id: id.to_string(),
            installation_id: id.to_string(),
            resolved: Arc::new(record.resolved().clone()),
            admin_secrets: Arc::new(crate::loaders::UnavailableLoadTimeAdminSecrets),
        }
    }

    fn test_loader(
        installation_store: Arc<dyn ExtensionInstallationStorePort>,
    ) -> CompositionExtensionLoader {
        CompositionExtensionLoader {
            binder: test_binder(),
            factories: HashMap::new(),
            channel_adapters: HashMap::new(),
            governor: Arc::new(InMemoryResourceGovernor::new()),
            installation_store,
        }
    }

    /// H.5 classification-fallback pin (extension-runtime P2 trap):
    /// `CompositionExtensionLoader::load`'s no-durable-record branch derives
    /// `ManifestSource` from `RequestedTrustClass`, not from any persisted
    /// source — adding a `ManifestSource` variant produces no compiler error
    /// here, so this test pins both arms directly. If the match ever
    /// collapses or reorders (e.g. the wildcard arm starts returning
    /// `HostBundled`), a `ThirdParty`-trust manifest would silently pass the
    /// `to_internal` trust re-check it must fail, and this test's second
    /// assertion changes from a runtime-backend error to a success/entirely
    /// different error shape.
    #[tokio::test]
    async fn no_durable_record_fallback_derives_source_from_requested_trust_only() {
        let store = Arc::new(filesystem_installation_store_for_test().await);
        // No installation seeded for either id: `get_manifest` returns
        // `None`, forcing the fallback branch under test.
        let loader = test_loader(Arc::clone(&store) as Arc<dyn ExtensionInstallationStorePort>);

        // FirstPartyRequested trust -> fallback must derive `HostBundled`
        // (`to_internal` allows it) and fail downstream on the *missing
        // runtime backend*, not on a trust rejection.
        let ctx = load_context_for(
            "h5-first-party-fallback",
            &fixture_manifest_toml("h5-first-party-fallback"),
        )
        .await;
        let error = match loader.load(&ctx).await {
            Ok(_) => panic!("no first-party factory is registered for this fixture's service"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("no runtime backend is configured"),
            "FirstPartyRequested trust with no durable record must still resolve to \
             ManifestSource::HostBundled, failing downstream on the missing runtime backend \
             rather than on trust; got: {error}"
        );

        // ThirdParty trust -> fallback must derive `InstalledLocal`
        // (never eligible for first-party) and still pass `to_internal`,
        // failing downstream on the missing wasm lane, not on trust.
        let ctx = load_context_for(
            "h5-third-party-fallback",
            &third_party_wasm_fixture_manifest_toml("h5-third-party-fallback"),
        )
        .await;
        let error = match loader.load(&ctx).await {
            Ok(_) => panic!("no wasm lane is registered in the test binder"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("no runtime backend is configured"),
            "ThirdParty trust with no durable record must resolve to a source that still \
             passes to_internal, failing downstream on the missing wasm lane rather than on \
             trust; got: {error}"
        );
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

    #[test]
    fn resolve_package_root_uses_the_persisted_root_when_present() {
        let persisted =
            VirtualPath::new("/system/extensions/actually-persisted-root").expect("persisted root");
        let resolved =
            resolve_package_root(Some(&persisted), "foo").expect("persisted root resolves");
        assert_eq!(
            resolved, persisted,
            "a persisted root must win over the fabricated fallback, even when \
             it does not match the fabricated `/system/extensions/{{id}}` shape"
        );
    }

    #[test]
    fn resolve_package_root_fabricates_the_legacy_path_when_none() {
        let resolved = resolve_package_root(None, "legacy-ext").expect("legacy fallback resolves");
        assert_eq!(
            resolved,
            VirtualPath::new("/system/extensions/legacy-ext").expect("fabricated root")
        );
    }

    /// H.5 / MIG-4: durable installation records hydrate into the generic
    /// host's standard active records at boot.
    #[tokio::test]
    async fn boot_hydration_activates_persisted_installations() {
        let store = Arc::new(filesystem_installation_store_for_test().await);
        seed_installation(&store, "h5-enabled").await;
        let boot_installations = boot_installation_records_for_test(&store).await;

        let generic = build_generic_extension_host(GenericExtensionHostParams {
            binder: test_binder(),
            native_factories: vec![Arc::new(FixtureNativeFactory)],
            channel_adapters: Vec::new(),
            installation_store: Arc::clone(&store) as Arc<dyn ExtensionInstallationStorePort>,
            boot_installations,
            governor: Arc::new(InMemoryResourceGovernor::new()),
            assembly: ExtensionHostAssemblyConfig::new(
                BTreeSet::new(),
                BTreeSet::new(),
                Duration::from_secs(30),
            ),
            channel_egress_transport: None,
            linked_sessions: LinkedSessionStore::unavailable(),
            linked_accounts: Arc::new(
                crate::linked_account_resolution::UnavailableLinkedAccountResolution,
            ),
            admin_secrets: None,
        })
        .await;

        let snapshot = generic.host.snapshot().await;
        assert!(
            snapshot.extension("h5-enabled").is_some(),
            "durable installation must hydrate to an Active record \
             in the first published generation"
        );
        assert!(
            snapshot
                .resolve_tool(&CapabilityId::new("h5-enabled.echo").expect("capability id"))
                .is_some(),
            "the hydrated Active extension's capability must resolve from the snapshot"
        );
    }

    async fn filesystem_installation_store_for_test() -> ExtensionInstallationStore {
        use ironclaw_filesystem::InMemoryBackend;
        use ironclaw_host_api::{host_port::HostPortCatalog, path::VirtualPath};

        ExtensionInstallationStore::load_at(
            Arc::new(InMemoryBackend::new()),
            VirtualPath::new("/system/extensions/.installations/test").expect("valid test path"),
            HostPortCatalog::empty(),
            ironclaw_extension_registry::default_host_api_contract_registry().expect("contracts"),
        )
        .await
        .expect("filesystem extension installation store")
    }

    async fn boot_installation_records_for_test(
        store: &ExtensionInstallationStore,
    ) -> Vec<InstallationRecord> {
        let mut records = Vec::new();
        for installation in store
            .list_installations()
            .await
            .expect("list installations")
        {
            let extension_id = installation.extension_id().clone();
            let Some(manifest_record) = store.get_manifest(&extension_id).await.expect("manifest")
            else {
                continue;
            };
            records.push(InstallationRecord {
                extension_id: extension_id.as_str().to_string(),
                installation_id: installation.installation_id().as_str().to_string(),
                state: ironclaw_extension_contracts::state::InstallationState::Installed,
                resolved: Arc::new(manifest_record.resolved().clone()),
                config: Vec::new(),
                last_error: None,
            });
        }
        records
    }

    /// Stub MCP transport that answers the standard `initialize` /
    /// `notifications/initialized` / `tools/list` handshake with one
    /// discovered tool carrying a non-trivial input schema. Mirrors
    /// `mcp_discovery::tests::TwoToolEgress`, scoped to this test module so it
    /// can name its own tool/schema.
    struct OneToolEgress {
        tool_name: &'static str,
        input_schema: serde_json::Value,
    }

    #[async_trait]
    impl ironclaw_host_api::http::RuntimeHttpEgress for OneToolEgress {
        async fn execute(
            &self,
            request: ironclaw_host_api::http::RuntimeHttpEgressRequest,
        ) -> Result<
            ironclaw_host_api::http::RuntimeHttpEgressResponse,
            ironclaw_host_api::http::RuntimeHttpEgressError,
        > {
            use ironclaw_host_api::http::RuntimeHttpEgressError;
            let body: serde_json::Value = serde_json::from_slice(&request.body).map_err(|_| {
                RuntimeHttpEgressError::Request {
                    reason: "invalid_json_rpc_body".to_string(),
                    request_bytes: request.body.len() as u64,
                    response_bytes: 0,
                }
            })?;
            let method =
                body["method"]
                    .as_str()
                    .ok_or_else(|| RuntimeHttpEgressError::Request {
                        reason: "missing_json_rpc_method".to_string(),
                        request_bytes: request.body.len() as u64,
                        response_bytes: 0,
                    })?;
            let result = match method {
                "initialize" => serde_json::json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "test", "version": "1"}
                }),
                "notifications/initialized" => serde_json::json!({}),
                "tools/list" => serde_json::json!({"tools": [
                    {
                        "name": self.tool_name,
                        "description": "discovered tool fixture",
                        "inputSchema": self.input_schema.clone(),
                    }
                ]}),
                _ => {
                    return Err(RuntimeHttpEgressError::Request {
                        reason: "unexpected_json_rpc_method".to_string(),
                        request_bytes: request.body.len() as u64,
                        response_bytes: 0,
                    });
                }
            };
            Ok(ironclaw_host_api::http::RuntimeHttpEgressResponse {
                status: 200,
                headers: vec![
                    ("content-type".to_string(), "application/json".to_string()),
                    ("Mcp-Session-Id".to_string(), "session-1".to_string()),
                ],
                body: serde_json::to_vec(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": body["id"],
                    "result": result,
                }))
                .map_err(|_| RuntimeHttpEgressError::Request {
                    reason: "serialize_json_rpc_response".to_string(),
                    request_bytes: request.body.len() as u64,
                    response_bytes: 0,
                })?,
                saved_body: None,
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
                redaction_applied: false,
            })
        }
    }

    /// Nearai-shaped `HostBundled` fixture manifest: `[mcp]` connection plus a
    /// static `[[tools]]` template capability, matching
    /// `crates/extensions/packages/nearai-mcp/manifest.toml`'s
    /// shape (source, root binding, and discovered-schema path convention),
    /// without depending on the real nearai asset files.
    fn hosted_mcp_first_party_manifest_toml(id: &str) -> String {
        format!(
            r#"
schema_version = "reborn.extension_manifest.v3"
id = "{id}"
name = "Hosted MCP fixture"
version = "0.1.0"
description = "hosted MCP discovery fixture mirroring the nearai bundled provider"
trust = "first_party_requested"

[mcp]
server = "https://mcp.example.test/mcp"
namespace = "{id}"
max_tools = 8
default_permission = "ask"
effects = ["network"]

[[tools]]
id = "{id}.web_search"
description = "Static template tool"
default_permission = "ask"
input_schema_ref = "schemas/{id}/web_search.input.v1.json"
"#,
            id = id,
        )
    }

    /// Regression test for the production incident: "The run failed while
    /// preparing the runtime host" /
    /// `missing input_schema_ref at /system/extensions/nearai/schemas/nearai/dynamic/web_search.input.v1.json`.
    ///
    /// Drives the real production caller chain for a `HostBundled` hosted-MCP
    /// provider shaped exactly like `nearai` (source `HostBundled`,
    /// `root_binding: Materialized`, `descriptor_schema_mode: InlineDynamic`
    /// after discovery):
    ///
    /// 1. `discover_hosted_mcp_package` (the real discovery path activation
    ///    uses) against a stubbed MCP server returning one tool with a
    ///    non-trivial input schema.
    /// 2. `effective_resolved_for_package` (the real activation-publish
    ///    helper) — pins defect 1: the persisted `ResolvedExtensionManifest`
    ///    must carry the discovered schema in `mcp.dynamic_input_schemas`.
    /// 3. `rebuild_package_from_resolved` from ONLY that durable record (no
    ///    live discovery) — pins defect 2: rebuild must choose the
    ///    inline-dynamic constructor, not the `ManifestRefs` one.
    /// 4. `publish_hot_capability_catalog` (the real capability-catalog path)
    ///    against a filesystem that does NOT contain the schema file —
    ///    reproduces the exact production absence and must still succeed,
    ///    with `parameters_schema` equal to the originally discovered schema.
    #[tokio::test]
    async fn hosted_mcp_discovered_schema_survives_persist_and_rebuild_without_filesystem_schema() {
        let id = "hosted-mcp-fixture";
        let toml = hosted_mcp_first_party_manifest_toml(id);
        let root = VirtualPath::new(format!("/system/extensions/{id}")).expect("test root");
        let record = ExtensionManifestRecord::from_toml_with_root_binding(
            toml,
            ManifestSource::HostBundled,
            &ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog"),
            None,
            &crate::product_extension_host_api_contract_registry().expect("test contracts"),
            ironclaw_extension_registry::PackageRootBinding::Materialized(root.clone()),
        )
        .expect("hosted MCP fixture manifest resolves");
        let base_resolved = record.resolved().clone();
        assert!(
            base_resolved
                .mcp
                .as_ref()
                .is_some_and(|mcp| mcp.dynamic_input_schemas.is_empty()),
            "the pre-discovery durable record must not yet carry a discovered schema"
        );

        // Build the pre-discovery package exactly as the production loader
        // would (`to_internal` + `try_from`, no TOML reparse), in
        // `ManifestRefs` mode (the ordinary Materialized shape before
        // discovery ever runs).
        let manifest_v2 = base_resolved
            .to_internal(ManifestSource::HostBundled)
            .expect("resolved contract rebuilds to v2");
        let manifest = ironclaw_extension_registry::ExtensionManifest::try_from(manifest_v2)
            .expect("v2 manifest rebuilds to v1");
        let initial_package = ExtensionPackage::from_manifest(manifest, root.clone())
            .expect("pre-discovery package constructs");

        let discovered_schema = serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"]
        });
        let scope = ironclaw_host_api::resource::ResourceScope::local_default(
            ironclaw_host_api::ids::UserId::new("hosted-mcp-fixture-user").expect("test user"),
            ironclaw_host_api::ids::InvocationId::new(),
        )
        .expect("test scope");
        let discovered = crate::discover_hosted_mcp_package(
            &initial_package,
            8,
            scope,
            Arc::new(OneToolEgress {
                tool_name: "web_search",
                input_schema: discovered_schema.clone(),
            }),
        )
        .await
        .expect("stubbed discovery succeeds");
        assert_eq!(
            discovered.descriptor_schema_mode,
            ironclaw_extension_registry::CapabilityDescriptorSchemaMode::InlineDynamic,
            "a HostBundled hosted-MCP package built from discovery is InlineDynamic, \
             exactly like nearai's `package_with_discovered_hosted_mcp_tools`"
        );

        // Defect 1: the persisted record must capture the discovered schema
        // for THIS package shape, not only for `Virtual` packages.
        let effective = effective_resolved_for_package(&base_resolved, &discovered);
        let capability_id = format!("{id}.web_search");
        assert_eq!(
            effective
                .mcp
                .as_ref()
                .expect("hosted MCP declaration persists")
                .dynamic_input_schemas
                .get(capability_id.as_str()),
            Some(&discovered_schema),
            "effective_resolved_for_package must persist the discovered schema for a \
             Materialized + InlineDynamic package, not only for Virtual packages"
        );

        // Defect 2: rebuilding from ONLY the durable record (no live
        // discovery) must reconstruct an InlineDynamic package whose
        // descriptor already carries the discovered schema.
        let rebuild_manifest_v2 = effective
            .to_internal(ManifestSource::HostBundled)
            .expect("persisted resolved contract rebuilds to v2");
        let rebuild_manifest =
            ironclaw_extension_registry::ExtensionManifest::try_from(rebuild_manifest_v2)
                .expect("v2 manifest rebuilds to v1");
        let rebuilt = rebuild_package_from_resolved(rebuild_manifest, &effective, id)
            .expect("rebuild from the durable record alone succeeds");
        assert_eq!(
            rebuilt.descriptor_schema_mode,
            ironclaw_extension_registry::CapabilityDescriptorSchemaMode::InlineDynamic,
            "rebuilding a Materialized package with a persisted discovered-schema map must \
             choose the inline-dynamic constructor, not `from_manifest`'s ManifestRefs"
        );

        // Reproduce the exact production absence: publish through the real
        // capability-catalog path against a filesystem with no schema file at
        // all (not even the directory), and confirm success with the
        // originally discovered schema.
        let fs = ironclaw_filesystem::InMemoryBackend::new();
        let mut registry = ExtensionRegistry::new();
        registry
            .insert(rebuilt)
            .expect("rebuilt package inserts into the registry");
        let catalog = ironclaw_host_runtime::publish_hot_capability_catalog(&fs, &registry)
            .await
            .expect(
                "publishing the rebuilt package must succeed even though \
                 /system/extensions/hosted-mcp-fixture/schemas/hosted-mcp-fixture/dynamic/\
                 web_search.input.v1.json was never written to the filesystem",
            );
        let record = catalog
            .get(&CapabilityId::new(capability_id.as_str()).expect("capability id"))
            .expect("discovered capability publishes");
        assert_eq!(
            record.descriptor.parameters_schema, discovered_schema,
            "the published descriptor must carry the originally discovered schema"
        );
    }
}
