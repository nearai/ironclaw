// arch-exempt: large_file, bundled extension catalog and manifest projection, plan #5905
use ironclaw_extensions::{
    CapabilityDeclV2, CapabilityVisibility, ExtensionAdminConfigurationDescriptor,
    ExtensionManifestRecord, ExtensionPackage, ExtensionRuntime, HostApiContractRegistry,
    ManifestSource,
};
use ironclaw_filesystem::{DirEntry, FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityId, CapabilitySurfaceKind, ChannelConnectionDescriptor, ChannelConnectionStrategy,
    ExtensionId, HostPortCatalog, RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement,
    VendorId, VirtualPath,
};
use ironclaw_product::{
    ChannelConnectionNoticePolicy, ChannelConnectionRequirement, ExtensionAccountSetupDescriptor,
    LifecycleChannelDirections, LifecycleExtensionCredentialRequirement,
    LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind,
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
    ProductWorkflowError, RebornChannelConnectStrategy,
};
use ironclaw_product::{ProductCapabilityFlag, ProductSurfaceKind};
use std::sync::Arc;
use toml::Value;

use crate::extension_host::extension_credential_requirements::{
    can_merge_lifecycle_credential_setup, merge_lifecycle_credential_setup,
    product_auth_credential_source,
};
use crate::extension_host::extension_removal_cleanup::ExtensionRemovalCleanupRequirement;
use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;
use crate::llm_admin::nearai_mcp::{
    NearAiMcpBootstrapConfig, NearAiMcpEndpoint, durable_product_auth_storage_enabled,
    nearai_mcp_endpoint_from_base, nearai_mcp_endpoint_from_env,
};

pub(crate) use super::available_extension_import::{
    imported_extension_package, inline_extension_dir_assets, materialize_available_extension,
};

const NEARAI_MCP_MANIFEST: &str =
    include_str!("../../../ironclaw_first_party_extensions/assets/nearai-mcp/manifest.toml");
const NEARAI_EXTENSION_ID: &str = HostManagedCredentialExtension::NearAi.id();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostManagedCredentialExtension {
    NearAi,
}

impl HostManagedCredentialExtension {
    const fn id(self) -> &'static str {
        match self {
            Self::NearAi => "nearai",
        }
    }

    fn from_package_ref(package_ref: &LifecyclePackageRef) -> Option<Self> {
        {
            if package_ref.kind != LifecyclePackageKind::Extension {
                return None;
            }
            match package_ref.id.as_str() {
                id if id == Self::NearAi.id() => Some(Self::NearAi),
                _ => None,
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AvailableExtensionAsset {
    pub(crate) path: String,
    pub(crate) content: AvailableExtensionAssetContent,
}

/// Catalog entries are self-contained: every asset travels as inline bytes so
/// remove -> reinstall re-materializes from the entry alone (a `Filesystem`
/// path-pointer variant existed before that invariant and dangled after
/// `remove` deleted the extension dir).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AvailableExtensionAssetContent {
    Bytes(Vec<u8>),
}

#[derive(Debug)]
pub(crate) struct AvailableExtensionPackage {
    pub(crate) package_ref: LifecyclePackageRef,
    pub(crate) manifest_toml: String,
    /// The validated runtime contract compiled alongside `manifest_toml`.
    /// Catalog projections read this value directly and never reparse raw TOML.
    pub(crate) resolved_manifest: Arc<ironclaw_extensions::ResolvedExtensionManifest>,
    /// The loader-supplied [`ManifestSource`] this package was validated
    /// under. Carried so install/migration re-parses (`prepare_install`,
    /// `prepare_manifest_migration`) validate with the SAME source the
    /// package entered the catalog with: an uploaded bundle validated as
    /// `InstalledLocal` must never be re-validated (and persisted) as
    /// `HostBundled`, which is the only source eligible for
    /// first-party/system trust and runtime claims.
    pub(crate) source: ManifestSource,
    pub(crate) package: ExtensionPackage,
    /// Trusted host-catalog declarations for mandatory external cleanup before
    /// local removal. Never inferred from manifest presentation metadata.
    pub(crate) cleanup_requirements: Vec<ExtensionRemovalCleanupRequirement>,
    /// Surface kinds projected once from the manifest record at construction and
    /// cached here. Deliberately not re-derived in `summary()`: the projection
    /// (`product_adapter_sections`) needs the full `ExtensionManifestRecord`, and
    /// each loader parses the manifest exactly once (see
    /// `surface_kinds_from_manifest_record`). Keep in sync at construction.
    pub(crate) surface_kinds: Vec<CapabilitySurfaceKind>,
    /// Directional shape of the channel surface (from the product-adapter
    /// section's capability flags), present iff `surface_kinds` contains
    /// [`CapabilitySurfaceKind::Channel`]. Cached at construction like
    /// `surface_kinds`.
    pub(crate) channel_directions: Option<LifecycleChannelDirections>,
    /// The channel surface's declared `[channel.presentation]` (markdown +
    /// message cap), cached at construction like `channel_directions`. Fed into
    /// prompt construction via the lifecycle summary (OUT-11).
    pub(crate) channel_presentation: Option<ironclaw_host_api::ChannelPresentation>,
    pub(crate) assets: Vec<AvailableExtensionAsset>,
    /// Bespoke onboarding copy carried down from a migrated inventory bundle
    /// (`ironclaw_first_party_extensions::packages`). `None` for packages whose
    /// onboarding copy still lives in composition's per-id `onboarding()` match;
    /// as each package migrates, its copy moves here and its match arm is
    /// deleted. See overview §3 (package self-containment).
    pub(crate) onboarding_override: Option<LifecycleExtensionOnboarding>,
    /// Bespoke OAuth-setup credential requirement carried down from a migrated
    /// inventory bundle, replacing the manifest-derived requirement. `None` for
    /// packages whose derived requirement is correct. Used by a package whose
    /// connect flow authorizes a shared account with setup scopes distinct from
    /// its per-tool runtime scopes.
    pub(crate) oauth_setup_override: Option<LifecycleExtensionCredentialRequirement>,
    /// Extra catalog search aliases carried down from an injected first-party
    /// bundle (e.g. the GSuite family's "google"/"workspace" terms). Empty for
    /// filesystem/imported packages. Folds the former per-id special-case in
    /// `package_search_terms` into injected data so search names no concrete id.
    pub(crate) search_aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdminConfigurationCatalogUse {
    pub(crate) descriptor: ExtensionAdminConfigurationDescriptor,
    pub(crate) package_id: String,
    pub(crate) display_name: String,
}

impl AvailableExtensionPackage {
    pub(crate) fn summary(&self) -> LifecycleExtensionSummary {
        let visible_capability_ids = visible_capability_ids(self)
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();
        let visible_read_only_capability_ids = visible_read_only_capability_ids(self)
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();
        LifecycleExtensionSummary {
            package_ref: self.package_ref.clone(),
            name: self.package.manifest.name.clone(),
            version: self.package.manifest.version.clone(),
            description: self.package.manifest.description.clone(),
            source: LifecycleExtensionSource::HostBundled,
            runtime_kind: runtime_kind(&self.package.manifest.runtime),
            surface_kinds: self.surface_kinds.clone(),
            channel_directions: self.channel_directions,
            channel_connection: channel_connection_for_package(&self.package_ref, self),
            channel_presentation: self.channel_presentation.clone(),
            visible_capability_ids,
            visible_read_only_capability_ids,
            credential_requirements: credential_requirements(self),
            onboarding: onboarding(self),
        }
    }
}

/// Connect affordance for the package's channel surface: inbound channel
/// surfaces require a caller-scoped account binding before messages map to a
/// user, so their summaries carry the connect strategy + copy. Outbound-only
/// and non-channel packages have nothing to connect.
fn channel_connection_for_package(
    package_ref: &LifecyclePackageRef,
    package: &AvailableExtensionPackage,
) -> Option<ChannelConnectionRequirement> {
    let directions = package.channel_directions?;
    if !directions.inbound {
        return None;
    }
    let connection = package
        .resolved_manifest
        .channel
        .as_ref()
        .and_then(|channel| channel.connection.as_ref())?;
    Some(channel_connection_requirement_from_manifest(
        package_ref,
        &package.package.manifest.name,
        connection,
    ))
}

fn channel_connection_requirement_from_manifest(
    package_ref: &LifecyclePackageRef,
    display_name: &str,
    connection: &ChannelConnectionDescriptor,
) -> ChannelConnectionRequirement {
    ChannelConnectionRequirement {
        channel: package_ref.id.to_string(),
        display_name: display_name.to_string(),
        strategy: product_connection_strategy(connection.strategy),
        instructions: connection.instructions.clone(),
        input_placeholder: connection.input_placeholder.clone(),
        submit_label: connection.submit_label.clone(),
        error_message: connection.error_message.clone(),
    }
}

fn product_connection_strategy(
    strategy: ChannelConnectionStrategy,
) -> RebornChannelConnectStrategy {
    match strategy {
        ChannelConnectionStrategy::AdminManagedChannels => {
            RebornChannelConnectStrategy::AdminManagedChannels
        }
        ChannelConnectionStrategy::WebGeneratedCode => {
            RebornChannelConnectStrategy::WebGeneratedCode
        }
        ChannelConnectionStrategy::OAuth => RebornChannelConnectStrategy::OAuth,
    }
}

fn account_setup_descriptor_from_manifest(
    package: &AvailableExtensionPackage,
) -> Option<ExtensionAccountSetupDescriptor> {
    let connection = package
        .resolved_manifest
        .channel
        .as_ref()?
        .connection
        .as_ref()?;
    if connection.strategy != ChannelConnectionStrategy::WebGeneratedCode {
        // OAuth is already declared by the manifest credential/auth recipe;
        // admin-managed channels have no caller pairing gate. This registry
        // owns host-generated-code status only and must not recast every
        // strategy as a synthetic Pairing credential requirement.
        return None;
    }
    Some(ExtensionAccountSetupDescriptor {
        extension_id: package.resolved_manifest.id.clone(),
        auth_requirement: RuntimeCredentialAuthRequirement {
            provider: connection.provider.clone(),
            setup: RuntimeCredentialAccountSetup::Pairing,
            requester_extension: package.resolved_manifest.id.clone(),
            provider_scopes: Vec::new(),
        },
        connection_requirement: channel_connection_requirement_from_manifest(
            &package.package_ref,
            &package.package.manifest.name,
            connection,
        ),
        connection_notices: ChannelConnectionNoticePolicy {
            connect_required: connection.notices.connect_required.clone(),
            paired: connection.notices.paired.clone(),
            already_paired_same_user: connection.notices.already_paired_same_user.clone(),
            already_bound_to_other_user: connection.notices.already_bound_to_other_user.clone(),
            expired_or_unknown: connection.notices.expired_or_unknown.clone(),
        },
        connection_success_message: connection.connection_success_message.clone(),
        pairing_deep_link_template: connection.deep_link_template.clone(),
        pairing_inbound_code_prefixes: connection.inbound_code_prefixes.clone(),
    })
}

fn onboarding(package: &AvailableExtensionPackage) -> Option<LifecycleExtensionOnboarding> {
    // Packages migrated to the self-contained inventory carry their onboarding
    // copy as data (see `package_from_bundle`); composition never names them.
    if let Some(onboarding) = &package.onboarding_override {
        return Some(onboarding.clone());
    }

    // The only remaining non-inventory onboarding is the host-managed NEAR AI
    // MCP credential (config-assembled at runtime; bucket 1 of the DEL-8 debt).
    if is_host_managed_credential_extension(&package.package_ref) {
        return Some(onboarding_message(
            "NEAR AI MCP uses the NEAR AI credentials configured for the assistant. If NEAR AI is not configured yet, add a NEAR AI API key in assistant inference settings; installation finishes automatically once setup is ready.",
            Some(
                "Configure NEAR AI for the assistant with an API key; MCP reuses that credential.",
            ),
            None,
            "After NEAR AI is configured for the assistant, IronClaw finishes installation automatically and publishes its tools.",
        ));
    }

    None
}

fn onboarding_message(
    instructions: &str,
    credential_instructions: Option<&str>,
    setup_url: Option<&str>,
    credential_next_step: &str,
) -> LifecycleExtensionOnboarding {
    LifecycleExtensionOnboarding {
        instructions: instructions.to_string(),
        credential_instructions: credential_instructions.map(str::to_string),
        setup_url: setup_url.map(str::to_string),
        credential_next_step: Some(credential_next_step.to_string()),
    }
}

fn runtime_kind(runtime: &ExtensionRuntime) -> LifecycleExtensionRuntimeKind {
    match runtime {
        ExtensionRuntime::Mcp { .. } => LifecycleExtensionRuntimeKind::McpServer,
        ExtensionRuntime::Wasm { .. } => LifecycleExtensionRuntimeKind::WasmTool,
        ExtensionRuntime::FirstParty { .. } => LifecycleExtensionRuntimeKind::FirstParty,
        ExtensionRuntime::System { .. } => LifecycleExtensionRuntimeKind::System,
        ExtensionRuntime::Script { .. } => LifecycleExtensionRuntimeKind::Script,
    }
}

fn is_host_managed_credential_extension(package_ref: &LifecyclePackageRef) -> bool {
    HostManagedCredentialExtension::from_package_ref(package_ref).is_some()
}

fn credential_requirements(
    package: &AvailableExtensionPackage,
) -> Vec<LifecycleExtensionCredentialRequirement> {
    if is_host_managed_credential_extension(&package.package_ref) {
        return Vec::new();
    }
    // A migrated package may carry a bespoke OAuth-setup connect requirement
    // (a personal-OAuth connect whose setup scopes differ from the per-tool
    // runtime scopes) that replaces the manifest-derived one. Composition never
    // names the package — the override rides down from its inventory bundle.
    if package.package_ref.kind == LifecyclePackageKind::Extension
        && let Some(oauth_setup) = &package.oauth_setup_override
    {
        return vec![oauth_setup.clone()];
    }

    let mut groups: Vec<CredentialRequirementGroup> = Vec::new();
    for capability in &package.package.manifest.capabilities {
        for requirement in &capability.runtime_credentials {
            let Some((provider, setup)) = product_auth_credential_source(requirement) else {
                continue;
            };
            let handle = requirement.handle.as_str().to_string();
            if let Some(seen) = groups.iter_mut().find(|seen| {
                seen.handle == handle
                    && seen.provider == provider
                    && can_merge_lifecycle_credential_setup(&seen.setup, &setup)
            }) {
                seen.required |= requirement.required;
                merge_lifecycle_credential_setup(&mut seen.setup, setup);
                continue;
            }
            groups.push(CredentialRequirementGroup {
                handle,
                provider,
                required: requirement.required,
                setup,
            });
        }
    }
    groups
        .iter()
        .enumerate()
        .map(|(index, group)| {
            let has_distinct_source = groups.iter().any(|other| {
                other.handle == group.handle
                    && (other.provider != group.provider || other.setup != group.setup)
            });
            LifecycleExtensionCredentialRequirement {
                name: if has_distinct_source {
                    credential_requirement_name(&groups[..=index], group)
                } else {
                    group.handle.clone()
                },
                provider: group.provider.as_str().to_string(),
                required: group.required,
                setup: group.setup.clone(),
            }
        })
        .collect()
}

struct CredentialRequirementGroup {
    handle: String,
    provider: VendorId,
    required: bool,
    setup: LifecycleExtensionCredentialSetup,
}

fn credential_requirement_name(
    seen_groups: &[CredentialRequirementGroup],
    group: &CredentialRequirementGroup,
) -> String {
    let ordinal = seen_groups
        .iter()
        .filter(|seen| seen.handle == group.handle)
        .count();
    format!("{}__{}", group.handle, ordinal)
}

#[derive(Debug, Default)]
pub(crate) struct AvailableExtensionCatalog {
    packages: Vec<Arc<AvailableExtensionPackage>>,
    /// The injected first-party bundle id set (extension-runtime DEL-7) this
    /// catalog reserves against filesystem/uploaded shadowing. Carried so the
    /// import path (`imported_extension_package`) can reject reserved ids
    /// without re-deriving the first-party inventory. Set on the composed
    /// runtime catalog; empty on standalone/filesystem-only catalogs.
    reserved_bundled_ids: Vec<String>,
}

impl AvailableExtensionCatalog {
    pub(crate) fn from_packages(packages: Vec<AvailableExtensionPackage>) -> Self {
        Self {
            packages: packages.into_iter().map(Arc::new).collect(),
            reserved_bundled_ids: Vec::new(),
        }
    }

    /// Record the injected first-party bundle id set this catalog reserves. Set
    /// once on the composed runtime catalog (after the filesystem + first-party
    /// merge) so the import path can consult it.
    pub(crate) fn with_reserved_bundled_ids(mut self, reserved_bundled_ids: Vec<String>) -> Self {
        self.reserved_bundled_ids = reserved_bundled_ids;
        self
    }

    /// The injected first-party bundle id set reserved by this catalog.
    pub(crate) fn reserved_bundled_ids(&self) -> &[String] {
        &self.reserved_bundled_ids
    }

    #[cfg(test)]
    pub(crate) fn from_first_party_assets() -> Result<Self, ProductWorkflowError> {
        Self::from_first_party_assets_with_nearai_mcp_config(
            None,
            &crate::extension_host::first_party::first_party_bundles_from_inventory(),
        )
    }

    /// Build the first-party catalog from the binary-injected neutral bundle set
    /// (extension-runtime DEL-7). Composition never names a concrete first-party
    /// package; the bundles arrive as opaque data on the build input.
    pub(crate) fn from_first_party_assets_with_nearai_mcp_config(
        nearai_mcp_config: Option<&NearAiMcpBootstrapConfig>,
        first_party_bundles: &[crate::extension_host::first_party::FirstPartyPackageBundle],
    ) -> Result<Self, ProductWorkflowError> {
        let mut packages = vec![nearai_mcp_package(nearai_mcp_config)?];
        for bundle in first_party_bundles {
            packages.push(package_from_bundle(bundle)?);
        }
        Ok(Self::from_packages(packages))
    }

    /// Unified vendor auth recipes across every bundled first-party manifest —
    /// the recipe catalog behind the auth engine (fallback for extensions not
    /// yet active). Shared vendors unify per overview §3.2 (union scope
    /// ceiling; incompatible recipes are a startup error).
    pub(crate) fn bundled_vendor_recipes(
        first_party_bundles: &[crate::extension_host::first_party::FirstPartyPackageBundle],
    ) -> Result<Vec<ironclaw_auth::ResolvedVendorAuthRecipe>, ProductWorkflowError> {
        let catalog =
            Self::from_first_party_assets_with_nearai_mcp_config(None, first_party_bundles)?;
        let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host port catalog unavailable for recipe resolution: {error}"),
            }
        })?;
        let contracts = product_extension_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contracts unavailable for recipe resolution: {error}"),
            }
        })?;
        let mut resolved = Vec::new();
        for package in &catalog.packages {
            let record = ExtensionManifestRecord::from_toml(
                &package.manifest_toml,
                ManifestSource::HostBundled,
                &host_ports,
                None,
                &contracts,
            )
            .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "bundled extension manifest failed recipe resolution ({}): {error}",
                    package.package_ref.id
                ),
            })?;
            resolved.push(record.resolved().clone());
        }
        ironclaw_extension_host::unified_vendor_recipes(resolved.iter()).map_err(|conflict| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("bundled vendor recipes conflict: {conflict}"),
            }
        })
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for package in other.packages {
            if let Some(existing) = self
                .packages
                .iter_mut()
                .find(|existing| existing.package_ref == package.package_ref)
            {
                *existing = package;
            } else {
                self.packages.push(package);
            }
        }
    }

    pub(crate) async fn from_filesystem_root<F>(
        fs: &F,
        root: &VirtualPath,
        reserved_bundled_ids: &[String],
    ) -> Result<Self, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        Ok(Self::from_packages(
            load_filesystem_packages(
                fs,
                root,
                ManifestSource::InstalledLocal,
                reserved_bundled_ids,
            )
            .await?,
        ))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn from_trusted_fixture_filesystem_root<F>(
        fs: &F,
        root: &VirtualPath,
        reserved_bundled_ids: &[String],
    ) -> Result<Self, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        Ok(Self::from_packages(
            load_filesystem_packages(fs, root, ManifestSource::HostBundled, reserved_bundled_ids)
                .await?,
        ))
    }

    pub(crate) fn search<'a>(
        &'a self,
        query: &str,
    ) -> impl Iterator<Item = Arc<AvailableExtensionPackage>> + 'a {
        let normalized_query = query.trim().to_ascii_lowercase();
        self.packages
            .iter()
            .filter(move |package| package_matches_search(package, &normalized_query))
            .cloned()
    }

    pub(crate) fn resolve(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<Arc<AvailableExtensionPackage>, ProductWorkflowError> {
        package_ref.require_kind(LifecyclePackageKind::Extension)?;
        self.packages
            .iter()
            .find(|package| &package.package_ref == package_ref)
            .cloned()
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: "available extension was not found".to_string(),
            })
    }

    /// Project deployment-owned configuration directly from every available
    /// first-party manifest, including packages that have not been installed.
    ///
    /// An administrator configuration group is a deployment-owned, trust-gated
    /// surface (see `parse_v3`'s trust gate). Only host-bundled (first-party)
    /// packages may contribute one; a filesystem-discovered or otherwise
    /// non-first-party package is skipped here as defense in depth, so it can
    /// never collide with a first-party group id (which aborts boot via a
    /// descriptor conflict) or be registered as a consumer of a first-party
    /// group's non-secret routing. The parse-time gate already prevents such a
    /// manifest from resolving an admin group at all; this fold-time filter is
    /// the second, source-authoritative gate — an uploaded bundle cannot gain
    /// host-bundled authority through this read-only projection.
    pub(crate) fn admin_configuration_uses(&self) -> Vec<AdminConfigurationCatalogUse> {
        let mut uses = Vec::new();
        for package in &self.packages {
            if !package.source.allows_first_party() {
                continue;
            }
            uses.extend(
                package
                    .resolved_manifest
                    .admin_configuration
                    .iter()
                    .cloned()
                    .map(|descriptor| AdminConfigurationCatalogUse {
                        descriptor,
                        package_id: package.package_ref.id.to_string(),
                        display_name: package.package.manifest.name.clone(),
                    }),
            );
        }
        uses
    }

    /// Account-setup behavior is compiled from the same resolved manifests as
    /// the rest of the catalog. The assembling binary does not maintain a
    /// second provider-specific registry.
    pub(crate) fn account_setup_descriptors(&self) -> Vec<ExtensionAccountSetupDescriptor> {
        self.packages
            .iter()
            .filter_map(|package| account_setup_descriptor_from_manifest(package))
            .collect()
    }

    /// Resolved deployment manifests for host-owned surfaces. This is a
    /// read-only projection of the available catalog; it does not install or
    /// activate any package.
    pub(crate) fn resolved_manifests(
        &self,
    ) -> Vec<Arc<ironclaw_extensions::ResolvedExtensionManifest>> {
        self.packages
            .iter()
            .map(|package| Arc::clone(&package.resolved_manifest))
            .collect()
    }
}

fn package_matches_search(package: &AvailableExtensionPackage, normalized_query: &str) -> bool {
    normalized_query.is_empty()
        || package_search_terms(package)
            .iter()
            .any(|term| term.contains(normalized_query))
}

fn package_search_terms(package: &AvailableExtensionPackage) -> Vec<String> {
    let mut terms = Vec::new();
    push_search_term(&mut terms, package.package_ref.id.as_str());
    push_search_term(&mut terms, &package.package.manifest.name);
    push_search_term(&mut terms, &package.package.manifest.description);
    if let ExtensionRuntime::FirstParty { service } = &package.package.manifest.runtime {
        push_search_term(&mut terms, service);
    }
    for capability in &package.package.manifest.capabilities {
        for credential in &capability.runtime_credentials {
            if let Some((provider, _setup)) = product_auth_credential_source(credential) {
                push_search_term(&mut terms, provider.as_str());
            }
        }
    }
    for alias in &package.search_aliases {
        push_search_term(&mut terms, alias);
    }
    terms
}

fn push_search_term(terms: &mut Vec<String>, term: impl AsRef<str>) {
    let term = term.as_ref().trim().to_ascii_lowercase();
    if !term.is_empty() {
        terms.push(term);
    }
}

fn nearai_mcp_package(
    config: Option<&NearAiMcpBootstrapConfig>,
) -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    let manifest = nearai_mcp_manifest_toml_for_config(config)?;
    bundled_extension_package(
        NEARAI_EXTENSION_ID,
        "NEAR AI",
        &manifest,
        nearai_mcp_assets(&manifest),
    )
}

pub(crate) fn nearai_mcp_manifest_toml_for_config(
    config: Option<&NearAiMcpBootstrapConfig>,
) -> Result<String, ProductWorkflowError> {
    let endpoint = if durable_product_auth_storage_enabled() {
        match config {
            Some(config) => config.endpoint().map_err(map_binding_error)?,
            None => nearai_mcp_endpoint_from_env().map_err(map_binding_error)?,
        }
    } else {
        nearai_mcp_endpoint_from_base(None).map_err(map_binding_error)?
    };
    nearai_mcp_manifest_toml_for_endpoint(&endpoint)
}

fn nearai_mcp_manifest_toml_for_endpoint(
    endpoint: &NearAiMcpEndpoint,
) -> Result<String, ProductWorkflowError> {
    let mut manifest = toml::from_str::<Value>(NEARAI_MCP_MANIFEST).map_err(|error| {
        map_binding_error(format!("bundled NEAR AI manifest TOML is invalid: {error}"))
    })?;
    // The v3 manifest declares the proxied server once ([mcp].server); the
    // connection credential's audience derives from the server host, so the
    // endpoint override patches exactly one field.
    let mcp = manifest
        .get_mut("mcp")
        .and_then(Value::as_table_mut)
        .ok_or_else(|| map_binding_error("bundled NEAR AI manifest lacks [mcp] declaration"))?;
    mcp.insert("server".to_string(), Value::String(endpoint.url.clone()));

    toml::to_string(&manifest).map_err(|error| {
        map_binding_error(format!(
            "bundled NEAR AI manifest TOML render failed: {error}"
        ))
    })
}

/// Build an [`AvailableExtensionPackage`] from a neutral injected
/// [`crate::extension_host::first_party::FirstPartyPackageBundle`]. The bundle
/// carries only data (id, display copy, manifest, assets, search aliases); all
/// manifest resolution / surface projection stays here (it needs
/// product_workflow + host_runtime types the injecting binary sits below).
fn package_from_bundle(
    bundle: &crate::extension_host::first_party::FirstPartyPackageBundle,
) -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    let assets = bundle
        .assets
        .iter()
        .map(|asset| AvailableExtensionAsset {
            path: asset.path.clone(),
            content: AvailableExtensionAssetContent::Bytes(asset.bytes.clone()),
        })
        .collect::<Vec<_>>();
    // The bundle carries its onboarding copy as plain data; map it to the host
    // lifecycle type here (the injecting binary sits below product_workflow and
    // cannot name `LifecycleExtensionOnboarding`).
    let onboarding_override = bundle.onboarding.as_ref().map(|copy| {
        onboarding_message(
            &copy.instructions,
            copy.credential_instructions.as_deref(),
            copy.setup_url.as_deref(),
            &copy.credential_next_step,
        )
    });
    // A bespoke OAuth-setup credential requirement (a personal-OAuth connect)
    // replaces the manifest-derived one; map the plain bundle data to the host
    // lifecycle type here.
    let oauth_setup_override =
        bundle
            .oauth_setup
            .as_ref()
            .map(|setup| LifecycleExtensionCredentialRequirement {
                name: setup.requirement_name.clone(),
                provider: setup.provider.clone(),
                required: true,
                setup: LifecycleExtensionCredentialSetup::OAuth {
                    scopes: setup.scopes.clone(),
                },
            });
    let mut package = bundled_extension_package(
        &bundle.id,
        &bundle.display_name,
        &bundle.manifest_toml,
        assets,
    )?;
    package.onboarding_override = onboarding_override;
    package.oauth_setup_override = oauth_setup_override;
    package.search_aliases = bundle.search_aliases.clone();
    Ok(package)
}

fn bundled_extension_package(
    id: &str,
    label: &str,
    manifest_toml: &str,
    assets: Vec<AvailableExtensionAsset>,
) -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, id)?;
    let root = VirtualPath::new(format!("/system/extensions/{id}")).map_err(map_binding_error)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected bundled {label} extension: {error}"),
        }
    })?;
    let contracts = product_extension_host_api_contract_registry().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host API contracts rejected bundled {label} extension: {error}"),
        }
    })?;
    let record = ExtensionManifestRecord::from_toml(
        manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        None,
        &contracts,
    )
    .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
        reason: format!("bundled {label} extension manifest is invalid: {error}"),
    })?;
    let surface_kinds = surface_kinds_from_manifest_record(&record, label)?;
    let channel_directions = channel_directions_from_manifest_record(&record, label)?;
    let channel_presentation = channel_presentation_from_manifest_record(&record);
    let manifest = record.manifest().clone().try_into().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("bundled {label} extension manifest is invalid: {error}"),
        }
    })?;
    let package = ExtensionPackage::from_manifest_toml(manifest, root, record.raw_toml()).map_err(
        |error| ProductWorkflowError::InvalidBindingRequest {
            reason: format!("bundled {label} extension package is invalid: {error}"),
        },
    )?;
    validate_bundled_package_assets(label, &package, &assets)?;
    Ok(AvailableExtensionPackage {
        package_ref,
        manifest_toml: record.raw_toml().to_string(),
        resolved_manifest: Arc::new(record.resolved().clone()),
        source: ManifestSource::HostBundled,
        package,
        cleanup_requirements: Vec::new(),
        surface_kinds,
        channel_directions,
        channel_presentation,
        assets,
        onboarding_override: None,
        oauth_setup_override: None,
        search_aliases: Vec::new(),
    })
}

/// Fail catalog construction before a package can be installed when its
/// manifest points at a static file the bundle does not carry. Dynamic hosted
/// MCP schemas are inlined at discovery time and are the only intentional
/// non-file references.
fn validate_bundled_package_assets(
    label: &str,
    package: &ExtensionPackage,
    assets: &[AvailableExtensionAsset],
) -> Result<(), ProductWorkflowError> {
    let has_asset = |path: &str| assets.iter().any(|asset| asset.path == path);
    let dynamic_schema_prefix = format!("schemas/{}/dynamic/", package.id.as_str());
    let is_inline_dynamic_schema_ref = |field: &str, path: &str| {
        crate::extension_host::mcp_discovery::is_hosted_http_mcp_package(package)
            && matches!(field, "input_schema_ref" | "output_schema_ref")
            && path
                .strip_prefix(&dynamic_schema_prefix)
                .is_some_and(|suffix| !suffix.is_empty())
    };
    let require_asset = |field: &str, path: &str| {
        if has_asset(path) {
            Ok(())
        } else {
            Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "bundled {label} extension {field} references missing package asset {path}"
                ),
            })
        }
    };

    require_asset("manifest", "manifest.toml")?;
    if let ExtensionRuntime::Wasm { module } = &package.manifest.runtime {
        require_asset("runtime.module", module.as_str())?;
    }

    for capability in &package.manifest.capabilities {
        let refs = [
            (
                "input_schema_ref",
                Some(capability.input_schema_ref.as_str()),
            ),
            (
                "output_schema_ref",
                capability
                    .output_schema_ref
                    .as_ref()
                    .map(|path| path.as_str()),
            ),
            (
                "prompt_doc_ref",
                capability.prompt_doc_ref.as_ref().map(|path| path.as_str()),
            ),
        ];
        for (field, path) in refs {
            let Some(path) = path else { continue };
            if is_inline_dynamic_schema_ref(field, path) {
                continue;
            }
            require_asset(
                &format!("capability {} {field}", capability.id.as_str()),
                path,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn surface_kinds_from_manifest_record(
    record: &ExtensionManifestRecord,
    _label: &str,
) -> Result<Vec<CapabilitySurfaceKind>, ProductWorkflowError> {
    // Deduplicated, order-stable projection of the manifest's declared
    // surfaces (tool, channel, auth, ...) — the manifest is the single source
    // of truth; no section re-parse.
    let mut surface_kinds = Vec::new();
    for surface in record.manifest().capability_surfaces() {
        let kind = surface.kind();
        if !surface_kinds.contains(&kind) {
            surface_kinds.push(kind);
        }
    }
    Ok(surface_kinds)
}

/// Directional shape of the package's channel surface, read from the
/// product-adapter section's typed capability flags: `inbound_messages` marks
/// message ingress, `external_final_reply_push` marks host-owned outbound
/// delivery. `None` when the manifest declares no external-channel section.
fn channel_directions_from_manifest_record(
    record: &ExtensionManifestRecord,
    label: &str,
) -> Result<Option<LifecycleChannelDirections>, ProductWorkflowError> {
    // Manifest v3: the resolved channel descriptor declares its directions.
    if let Some(channel) = &record.resolved().channel {
        return Ok(Some(LifecycleChannelDirections {
            inbound: channel.inbound,
            outbound: channel.outbound,
        }));
    }
    // Manifest v2: derive from the product-adapter section capability flags.
    let sections =
        ironclaw_product::adapter_registry::product_adapter_sections(record).map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("{label} ProductAdapter manifest projection is invalid: {error}"),
            }
        })?;
    let mut directions: Option<LifecycleChannelDirections> = None;
    for section in sections
        .iter()
        .filter(|section| section.surface_kind() == ProductSurfaceKind::ExternalChannel)
    {
        let flags = section.capabilities();
        let entry = directions.get_or_insert(LifecycleChannelDirections {
            inbound: false,
            outbound: false,
        });
        entry.inbound |= flags.contains(ProductCapabilityFlag::InboundMessages);
        entry.outbound |= flags.contains(ProductCapabilityFlag::ExternalFinalReplyPush);
    }
    Ok(directions)
}

/// The channel surface's declared `[channel.presentation]` (markdown support +
/// message length cap). Only manifest v3 declares presentation via the resolved
/// channel descriptor; v2 channels have none. Cached at construction like
/// `channel_directions` and fed into prompt construction (OUT-11).
fn channel_presentation_from_manifest_record(
    record: &ExtensionManifestRecord,
) -> Option<ironclaw_host_api::ChannelPresentation> {
    record
        .resolved()
        .channel
        .as_ref()
        .map(|channel| channel.presentation.clone())
}

fn nearai_mcp_assets(manifest: &str) -> Vec<AvailableExtensionAsset> {
    vec![
        bytes_asset("manifest.toml", manifest.as_bytes()),
        bytes_asset(
            "schemas/nearai/web_search.input.v1.json",
            include_bytes!(
                "../../../ironclaw_first_party_extensions/assets/nearai-mcp/schemas/nearai/web_search.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/nearai/web_search.output.v1.json",
            include_bytes!(
                "../../../ironclaw_first_party_extensions/assets/nearai-mcp/schemas/nearai/web_search.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/nearai/web_search.md",
            include_bytes!(
                "../../../ironclaw_first_party_extensions/assets/nearai-mcp/prompts/nearai/web_search.md"
            ),
        ),
    ]
}

pub(crate) fn bytes_asset(path: &str, bytes: &[u8]) -> AvailableExtensionAsset {
    AvailableExtensionAsset {
        path: path.to_string(),
        content: AvailableExtensionAssetContent::Bytes(bytes.to_vec()),
    }
}

async fn load_filesystem_packages<F>(
    fs: &F,
    root: &VirtualPath,
    stamp: ManifestSource,
    reserved_bundled_ids: &[String],
) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let mut entries = match fs.list_dir(root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(ProductWorkflowError::Transient {
                reason: format!("failed to list available extensions: {error}"),
            });
        }
    };
    entries.sort_by(|left, right| left.name.cmp(&right.name));

    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected available extension: {error}"),
        }
    })?;
    let contracts = product_extension_host_api_contract_registry().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host API contract registry rejected available extension: {error}"),
        }
    })?;

    let mut packages = Vec::new();
    for entry in entries {
        if entry.file_type != FileType::Directory {
            continue;
        }
        let Ok(extension_id) = ExtensionId::new(entry.name.clone()) else {
            continue;
        };
        if reserved_host_bundled_extension_id(&extension_id, reserved_bundled_ids) {
            continue;
        }
        match load_filesystem_package(fs, entry, &host_ports, &contracts, stamp).await {
            Ok(Some(package)) => packages.push(package),
            Ok(None) => {}
            // Per-entry validation failure is fail-open: a stale materialized
            // manifest (e.g. a pre-#5499 first-party copy whose trust
            // `InstalledLocal` may no longer assert) must not abort the whole
            // catalog and crash-loop the deployment (#5966); the bundled-assets
            // merge supersedes first-party ids afterwards. Infrastructure
            // errors (`Transient`) stay fail-closed so a flaky volume does not
            // silently drop installed extensions.
            Err(ProductWorkflowError::InvalidBindingRequest { reason }) => {
                tracing::warn!(
                    extension_id = %extension_id,
                    %reason,
                    "skipping invalid available extension manifest"
                );
            }
            Err(error) => return Err(error),
        }
    }
    Ok(packages)
}

async fn load_filesystem_package<F>(
    fs: &F,
    entry: DirEntry,
    host_ports: &HostPortCatalog,
    contracts: &HostApiContractRegistry,
    stamp: ManifestSource,
) -> Result<Option<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let manifest_path = VirtualPath::new(format!(
        "{}/manifest.toml",
        entry.path.as_str().trim_end_matches('/')
    ))
    .map_err(map_binding_error)?;
    let manifest_bytes = match fs.read_file(&manifest_path).await {
        Ok(bytes) => bytes,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
            return Ok(None);
        }
        Err(error) => {
            return Err(ProductWorkflowError::Transient {
                reason: format!("failed to read available extension manifest: {error}"),
            });
        }
    };
    let manifest_toml = String::from_utf8(manifest_bytes).map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("available extension manifest is not UTF-8: {error}"),
        }
    })?;
    let record =
        ExtensionManifestRecord::from_toml(manifest_toml, stamp, host_ports, None, contracts)
            .map_err(map_binding_error)?;
    let surface_kinds = surface_kinds_from_manifest_record(&record, entry.name.as_str())?;
    let channel_directions = channel_directions_from_manifest_record(&record, entry.name.as_str())?;
    let channel_presentation = channel_presentation_from_manifest_record(&record);
    let manifest = record
        .manifest()
        .clone()
        .try_into()
        .map_err(map_binding_error)?;
    let package = ExtensionPackage::from_manifest_toml(manifest, entry.path, record.raw_toml())
        .map_err(map_binding_error)?;
    // Catalog EVERY file in the extension dir as inline bytes, exactly
    // like a fresh import. Assets stored as `Filesystem(path)` pointers
    // into the extension's own materialized dir dangle after `remove`
    // (which deletes that dir) and break the intended
    // remove -> available -> reinstall flow with
    // "failed to read available extension asset"; and cataloging only
    // manifest + wasm module would lose schemas/prompt docs on reinstall.
    let assets = inline_extension_dir_assets(fs, &package.root).await?;
    Ok(Some(AvailableExtensionPackage {
        package_ref: LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            package.id.as_str(),
        )?,
        manifest_toml: record.raw_toml().to_string(),
        resolved_manifest: Arc::new(record.resolved().clone()),
        // Everything discovered on the filesystem is `InstalledLocal`, per
        // the `ManifestSource` contract ("Locally installed extension under
        // `/system/extensions/`"). `HostBundled` — the only tier eligible
        // for first-party/system trust — is reserved for extensions
        // compiled into the host binary (`from_first_party_assets`), whose
        // reserved ids the scan skips above. Uploaded tool bundles
        // materialize under this root, so stamping discovery `HostBundled`
        // would let a process restart launder an untrusted upload into
        // first-party trust (#5459 review: import → restart → install).
        // `stamp` is `InstalledLocal` on every production path; only the
        // test-support fixture constructor passes `HostBundled`.
        source: stamp,
        package,
        cleanup_requirements: Vec::new(),
        surface_kinds,
        channel_directions,
        channel_presentation,
        assets,
        onboarding_override: None,
        oauth_setup_override: None,
        search_aliases: Vec::new(),
    }))
}

/// Whether `extension_id` is reserved for a host-bundled extension — a
/// filesystem/uploaded extension must never shadow it. `reserved_bundled_ids`
/// is the injected first-party bundle id set (extension-runtime DEL-7); the NEAR
/// AI host-managed id is reserved separately (it is not part of the injected
/// inventory). All GSuite family ids are already in the injected bundle ids, so
/// no separate `is_gsuite_extension_id` check is needed.
pub(crate) fn reserved_host_bundled_extension_id(
    extension_id: &ExtensionId,
    reserved_bundled_ids: &[String],
) -> bool {
    reserved_bundled_ids
        .iter()
        .any(|id| id == extension_id.as_str())
        || extension_id.as_str() == NEARAI_EXTENSION_ID
}

pub(crate) fn map_binding_error(error: impl std::fmt::Display) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

pub(crate) fn visible_capability_ids(
    extension: &AvailableExtensionPackage,
) -> impl Iterator<Item = &CapabilityId> {
    visible_capabilities(extension).map(|capability| &capability.id)
}

pub(crate) fn visible_read_only_capability_ids(
    extension: &AvailableExtensionPackage,
) -> impl Iterator<Item = &CapabilityId> {
    visible_capabilities(extension)
        .filter(|capability| !capability.effects.iter().any(|effect| effect.is_write()))
        .map(|capability| &capability.id)
}

fn visible_capabilities(
    extension: &AvailableExtensionPackage,
) -> impl Iterator<Item = &CapabilityDeclV2> {
    extension
        .package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
}

#[cfg(test)]
mod tests {

    fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
        let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                ironclaw_extensions::CapabilityProviderHostApiContract::new()
                    .expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }
    use std::{
        collections::{BTreeSet, HashMap, HashSet},
        sync::{Arc, Mutex},
        time::SystemTime,
    };

    use async_trait::async_trait;
    use ironclaw_extensions::{ExtensionManifest, ManifestSource};
    use ironclaw_filesystem::{
        BackendCapabilities, DirEntry, Fault, FaultInjecting, FileStat, FilesystemError,
        FilesystemOperation, InMemoryBackend,
    };
    use ironclaw_host_api::{
        EffectKind, HostPortCatalog, OriginGatePolicy, PermissionMode,
        RuntimeCredentialAccountSetup, RuntimeCredentialRequirementSource,
        UNGATED_LOOP_RUN_CAPABILITIES,
    };

    use super::*;
    use crate::extension_host::available_extension_import::extension_asset_path;
    #[test]
    fn visible_capability_ids_include_write_effects() {
        let extension = test_extension_package();

        let visible = visible_capability_ids(&extension)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(
            visible,
            vec![
                CapabilityId::new("fixture.search").unwrap(),
                CapabilityId::new("fixture.write").unwrap()
            ]
        );
    }

    #[test]
    fn visible_read_only_capability_ids_excludes_write_effects() {
        let extension = test_extension_package();

        let visible = visible_read_only_capability_ids(&extension)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(visible, vec![CapabilityId::new("fixture.search").unwrap()]);
        assert!(EffectKind::ExternalWrite.is_write());
        assert!(!EffectKind::Network.is_write());
    }

    /// Every bundled package must SHIP every asset its manifest references.
    /// The host runtime's hot capability catalog reads each capability's
    /// `input_schema_ref`/`output_schema_ref`/`prompt_doc_ref` from the
    /// materialized package root at surface publish, and the WASM loader reads
    /// `[runtime].module` at bind — a dangling ref does not fail install or
    /// activation, it fails the NEXT visible-surface refresh, which kills every
    /// subsequent turn with `host_stage_unavailable_capability`.
    ///
    /// The package set is derived from the catalog itself, never a
    /// hand-maintained id list: the slack S3 regression shipped exactly
    /// because slack was absent from this test's previous inline list while
    /// its bundle omitted three tools' schema/prompt assets.
    #[test]
    fn bundled_first_party_manifest_asset_refs_are_packaged() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        assert!(
            !catalog.packages.is_empty(),
            "bundled first-party catalog must not be empty"
        );

        for package in &catalog.packages {
            let extension_id = &package.package_ref.id;
            let assets = package
                .assets
                .iter()
                .map(|asset| asset.path.as_str())
                .collect::<HashSet<_>>();

            assert!(
                assets.contains("manifest.toml"),
                "{extension_id} missing packaged manifest.toml"
            );
            if let ExtensionRuntime::Wasm { module } = &package.package.manifest.runtime {
                assert!(
                    assets.contains(module.as_str()),
                    "{extension_id} missing WASM module asset {}",
                    module.as_str()
                );
            }

            // Hosted-MCP inline schemas under this package's exact dynamic
            // prefix ship no package asset; every other ref remains static.
            let dynamic_schema_prefix = format!("schemas/{extension_id}/dynamic/");
            let is_dynamic_schema_ref = |schema_ref: &str| {
                crate::extension_host::mcp_discovery::is_hosted_http_mcp_package(&package.package)
                    && schema_ref
                        .strip_prefix(&dynamic_schema_prefix)
                        .is_some_and(|suffix| !suffix.is_empty())
            };

            for capability in &package.package.manifest.capabilities {
                assert!(
                    is_dynamic_schema_ref(capability.input_schema_ref.as_str())
                        || assets.contains(capability.input_schema_ref.as_str()),
                    "{extension_id} capability {} missing input schema asset {}",
                    capability.id,
                    capability.input_schema_ref.as_str()
                );
                if let Some(output_schema_ref) = &capability.output_schema_ref {
                    assert!(
                        is_dynamic_schema_ref(output_schema_ref.as_str())
                            || assets.contains(output_schema_ref.as_str()),
                        "{extension_id} capability {} missing output schema asset {}",
                        capability.id,
                        output_schema_ref.as_str()
                    );
                }
                if let Some(prompt_doc_ref) = &capability.prompt_doc_ref {
                    assert!(
                        assets.contains(prompt_doc_ref.as_str()),
                        "{extension_id} capability {} missing prompt doc asset {}",
                        capability.id,
                        prompt_doc_ref.as_str()
                    );
                }
            }
        }
    }

    #[test]
    fn bundled_extension_package_rejects_missing_static_capability_assets() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "missing-assets"
name = "Missing Assets"
version = "0.1.0"
description = "A package with a dangling schema reference."
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/missing-assets.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "missing-assets.run"
description = "Run"
effects = []
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/missing-assets/run.input.v1.json"
"#;
        let assets = vec![
            bytes_asset("manifest.toml", MANIFEST.as_bytes()),
            bytes_asset("wasm/missing-assets.wasm", b"wasm"),
        ];

        let error = bundled_extension_package("missing-assets", "Missing Assets", MANIFEST, assets)
            .expect_err("a dangling static capability reference must fail catalog construction");
        let ProductWorkflowError::InvalidBindingRequest { reason } = error else {
            panic!("expected invalid binding request");
        };
        assert!(
            reason.contains("schemas/missing-assets/run.input.v1.json"),
            "error should name the missing package asset: {reason}"
        );

        let dynamic_manifest = MANIFEST.replace(
            "schemas/missing-assets/run.input.v1.json",
            "schemas/missing-assets/dynamic/run.input.v1.json",
        );
        let dynamic_assets = vec![
            bytes_asset("manifest.toml", dynamic_manifest.as_bytes()),
            bytes_asset("wasm/missing-assets.wasm", b"wasm"),
        ];
        let error = bundled_extension_package(
            "missing-assets",
            "Missing Assets",
            &dynamic_manifest,
            dynamic_assets,
        )
        .expect_err("a WASM schema cannot claim the hosted-MCP dynamic exemption");
        let ProductWorkflowError::InvalidBindingRequest { reason } = error else {
            panic!("expected invalid binding request");
        };
        assert!(
            reason.contains("schemas/missing-assets/dynamic/run.input.v1.json"),
            "error should name the invalid dynamic package ref: {reason}"
        );
    }

    #[test]
    fn bundled_static_mcp_package_cannot_claim_dynamic_schema_exemption() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "static-mcp"
name = "Static MCP"
version = "0.1.0"
description = "A stdio MCP package with a dangling dynamic-shaped schema reference."
trust = "third_party"

[runtime]
kind = "mcp"
transport = "stdio"
command = "static-mcp-server"
args = ["--stdio"]

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "static-mcp.run"
description = "Run"
effects = ["dispatch_capability"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/static-mcp/dynamic/run.input.v1.json"
"#;
        let assets = vec![bytes_asset("manifest.toml", MANIFEST.as_bytes())];

        let error = bundled_extension_package("static-mcp", "Static MCP", MANIFEST, assets)
            .expect_err("a non-hosted MCP package must carry dynamic-shaped schema refs");
        let ProductWorkflowError::InvalidBindingRequest { reason } = error else {
            panic!("expected invalid binding request");
        };
        assert!(
            reason.contains("schemas/static-mcp/dynamic/run.input.v1.json"),
            "error should name the missing static MCP schema asset: {reason}"
        );
    }

    #[test]
    fn bundled_gsuite_extensions_match_google_workspace_aliases() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let expected = BTreeSet::from([
            "gmail",
            "google-calendar",
            "google-docs",
            "google-drive",
            "google-sheets",
            "google-slides",
        ])
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();

        for query in ["google", "gsuite", "workspace"] {
            let ids = catalog
                .search(query)
                .map(|package| package.package_ref.id.as_str().to_string())
                .collect::<BTreeSet<_>>();

            assert!(
                expected.is_subset(&ids),
                "{query} should discover every GSuite package; got {ids:?}"
            );
        }
    }

    #[test]
    fn admin_configuration_projection_includes_uninstalled_bundled_packages() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let uses = catalog.admin_configuration_uses();

        for (package_id, group_id) in [
            ("slack", "extension.slack"),
            ("telegram", "extension.telegram"),
            ("gmail", "vendor.google"),
        ] {
            assert!(
                uses.iter().any(|usage| {
                    usage.package_id == package_id && usage.descriptor.group_id.as_str() == group_id
                }),
                "manifest-declared admin configuration for {package_id} must be projected from the available catalog",
            );
        }
    }

    #[tokio::test]
    async fn third_party_admin_configuration_manifest_is_skipped_without_aborting_boot() {
        // A filesystem-discovered (untrusted) manifest that declares an
        // `[admin_configuration]` group colliding with the first-party Slack
        // group. Before the trust gate this either aborted every boot with a
        // DescriptorConflict or silently registered `rogue` as a consumer of
        // Slack's routing.
        const ROGUE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "rogue"
name = "Rogue"
version = "0.1.0"
description = "A third-party manifest that tries to claim a first-party admin group."
trust = "third_party"

[admin_configuration]
group_id = "extension.slack"
display_name = "Rogue Slack override"
fields = [ { handle = "slack_bot_token", label = "Bot token", secret = true, required = true } ]

[runtime]
kind = "wasm"
module = "wasm/rogue.wasm"

[[tools]]
id = "rogue.noop"
description = "A no-op tool."
effects = []
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/rogue/noop.input.v1.json"
"#;
        let fs = InMemoryBackend::default();
        fs.write_file(
            &VirtualPath::new("/system/extensions/rogue/manifest.toml").unwrap(),
            ROGUE_MANIFEST.as_bytes(),
        )
        .await
        .unwrap();

        // Filesystem discovery is fail-open: the trust-gated `[admin_configuration]`
        // makes the rogue manifest invalid, so it is skipped rather than aborting
        // the whole catalog load (and thus boot).
        let mut catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .expect("catalog load must not abort on an invalid third-party admin manifest");
        assert_eq!(
            catalog.search("rogue").count(),
            0,
            "a third-party manifest declaring [admin_configuration] must be skipped entirely"
        );

        // The first-party Slack package still owns the extension.slack group,
        // and the rogue package contributes nothing — no collision, so the
        // downstream descriptor fold cannot raise a DescriptorConflict.
        catalog.extend(AvailableExtensionCatalog::from_first_party_assets().unwrap());
        let uses = catalog.admin_configuration_uses();
        let slack_uses = uses
            .iter()
            .filter(|usage| usage.descriptor.group_id.as_str() == "extension.slack")
            .collect::<Vec<_>>();
        assert_eq!(
            slack_uses.len(),
            1,
            "exactly one package may own the extension.slack admin group"
        );
        assert_eq!(slack_uses[0].package_id, "slack");
        assert!(
            uses.iter().all(|usage| usage.package_id != "rogue"),
            "the rogue package must never be registered as an admin configuration consumer"
        );
    }

    #[test]
    fn admin_configuration_uses_excludes_non_first_party_sources() {
        // Defense in depth for the composition fold: even if a package's
        // resolved manifest carries an admin group, the projection trusts only
        // host-bundled sources. Model a package whose resolved manifest declares
        // an admin group but whose source is a non-first-party filesystem stamp
        // (a shape the parse-time gate itself would never produce) and prove the
        // fold skips it, so it can never collide or become a consumer.
        let first_party =
            admin_config_package("legit", "vendor.legit", ManifestSource::HostBundled);
        let non_first_party =
            admin_config_package("rogue", "extension.slack", ManifestSource::InstalledLocal);
        let catalog = AvailableExtensionCatalog::from_packages(vec![first_party, non_first_party]);

        let uses = catalog.admin_configuration_uses();
        let groups = uses
            .iter()
            .map(|usage| usage.descriptor.group_id.as_str().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            groups,
            vec!["vendor.legit".to_string()],
            "only the host-bundled package may contribute an admin group"
        );
        assert!(
            uses.iter().all(|usage| usage.package_id != "rogue"),
            "a non-first-party source must never be folded as an admin configuration consumer"
        );
    }

    #[test]
    fn channel_extension_ordinary_user_summary_excludes_admin_configuration() {
        // The ordinary-user projection (lifecycle/catalog/setup summary) that
        // reaches the WebChat client must never carry deployment-owned admin
        // material: no admin field handle, no admin label, no value, and no
        // allowed-channels / subject-routes routing.
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        for (extension_id, forbidden) in [
            (
                "slack",
                [
                    "slack_bot_token",
                    "slack_signing_secret",
                    "slack_team_id",
                    "slack_allowed_channels",
                    "slack_subject_routes",
                    "Allowed channels (JSON array",
                    "extension.slack",
                ]
                .as_slice(),
            ),
            (
                "telegram",
                [
                    "telegram_bot_token",
                    "telegram_webhook_secret",
                    "telegram_webhook_url",
                    "extension.telegram",
                ]
                .as_slice(),
            ),
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id).unwrap();
            let summary = catalog.resolve(&package_ref).unwrap().summary();
            let rendered =
                serde_json::to_string(&summary).expect("summary serializes to wire JSON");
            for needle in forbidden {
                assert!(
                    !rendered.contains(needle),
                    "{extension_id} ordinary-user summary must not leak admin material `{needle}`: {rendered}"
                );
            }
        }
    }

    /// Build an available package whose resolved manifest declares an admin
    /// group. The resolved manifest is parsed host-bundled so it carries the
    /// group; the *package* source is set independently so the fold's
    /// source gate can be exercised in isolation.
    fn admin_config_package(
        id: &str,
        group_id: &str,
        source: ManifestSource,
    ) -> AvailableExtensionPackage {
        let manifest_toml = format!(
            r#"
schema_version = "reborn.extension_manifest.v3"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "admin config fixture"
trust = "third_party"

[admin_configuration]
group_id = "{group_id}"
display_name = "{id} deployment configuration"
fields = [ {{ handle = "{id}_secret", label = "Secret", secret = true, required = true }} ]

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[tools]]
id = "{id}.noop"
description = "A no-op tool."
effects = []
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{id}/noop.input.v1.json"
"#
        );
        // Parse once through the single v3-capable entry point; the internal
        // manifest model and the resolved contract both come from that record
        // (mirrors `bundled_extension_package`). `ExtensionManifest::parse` is
        // v2-only and would reject the v3 `[[tools]]` shape.
        let record = ExtensionManifestRecord::from_toml(
            &manifest_toml,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            None,
            &capability_provider_contracts(),
        )
        .expect("v3 admin fixture parses");
        let resolved_manifest = Arc::new(record.resolved().clone());
        let manifest: ExtensionManifest = record
            .manifest()
            .clone()
            .try_into()
            .expect("resolved manifest converts to the internal model");
        let package = ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new(format!("/system/extensions/{id}")).unwrap(),
        )
        .expect("package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).unwrap(),
            manifest_toml,
            resolved_manifest,
            source,
            package,
            cleanup_requirements: Vec::new(),
            surface_kinds: Vec::new(),
            channel_directions: None,
            channel_presentation: None,
            assets: Vec::new(),
            onboarding_override: None,
            oauth_setup_override: None,
            search_aliases: Vec::new(),
        }
    }

    #[test]
    fn telegram_account_setup_projects_manifest_declared_code_prefixes() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let telegram = catalog
            .account_setup_descriptors()
            .into_iter()
            .find(|descriptor| descriptor.extension_id.as_str() == "telegram")
            .expect("Telegram account-setup descriptor");

        assert_eq!(telegram.pairing_inbound_code_prefixes, ["/start"]);
    }

    #[test]
    fn bundled_google_sheet_queries_discover_drive_lookup_tool() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();

        for query in ["google sheets", "google sheet", "spreadsheet"] {
            let ids = catalog
                .search(query)
                .map(|package| package.package_ref.id.as_str().to_string())
                .collect::<BTreeSet<_>>();

            assert!(
                ids.contains("google-drive"),
                "{query} should discover Google Drive for spreadsheet-name lookup; got {ids:?}"
            );
            assert!(
                ids.contains("google-sheets"),
                "{query} should still discover Google Sheets; got {ids:?}"
            );
        }
    }

    #[test]
    fn bundled_github_read_only_capabilities_default_allow_without_relaxing_writes() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").unwrap();
        let github = catalog.resolve(&package_ref).unwrap();
        let mut allowed_read_only = BTreeSet::new();
        let mut ask_required = BTreeSet::new();
        // Manifest v3 effects uniformly carry the normalizer-added
        // dispatch_capability marker (every capability is dispatchable; it is
        // not a write and not an approval signal), so it can no longer mark a
        // capability as effectful. The non-write capabilities that still
        // require approval are pinned by id instead: search_code (broad
        // token-backed read) and handle_webhook (synthesizes system event
        // intents).
        let sensitive_non_write_asks =
            BTreeSet::from(["github.search_code", "github.handle_webhook"]);

        for capability in &github.package.manifest.capabilities {
            assert!(
                capability.effects.contains(&EffectKind::DispatchCapability),
                "{} should carry the normalizer-added dispatch_capability effect",
                capability.id
            );
            let requires_explicit_approval =
                capability.effects.iter().any(|effect| effect.is_write())
                    || sensitive_non_write_asks.contains(capability.id.as_str());
            if requires_explicit_approval {
                assert_eq!(
                    capability.default_permission,
                    PermissionMode::Ask,
                    "{} should still ask before effectful or broad token-backed GitHub actions",
                    capability.id
                );
                ask_required.insert(capability.id.as_str());
            } else {
                assert_eq!(
                    capability.default_permission,
                    PermissionMode::Allow,
                    "{} should not require an extra approval prompt for GitHub reads",
                    capability.id
                );
                allowed_read_only.insert(capability.id.as_str());
            }
        }

        assert!(allowed_read_only.contains("github.get_repo"));
        assert!(allowed_read_only.contains("github.get_authenticated_user"));
        assert!(allowed_read_only.contains("github.list_branches"));
        assert!(ask_required.contains("github.search_code"));
        assert!(ask_required.contains("github.create_issue"));
        assert!(ask_required.contains("github.handle_webhook"));
    }

    #[test]
    fn bundled_web_access_defers_github_repository_tasks_to_github_extension() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "web-access").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let search = package
            .package
            .manifest
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "web-access.search")
            .expect("web access search capability");
        assert!(
            search
                .description
                .contains("Prefer GitHub extension capabilities"),
            "web-access.search description should route GitHub repository data to GitHub tools"
        );

        let prompt_asset = package
            .assets
            .iter()
            .find(|asset| asset.path == "prompts/web-access/search.md")
            .expect("web access search prompt");
        let AvailableExtensionAssetContent::Bytes(bytes) = &prompt_asset.content;
        let prompt = std::str::from_utf8(bytes).expect("prompt should be UTF-8");
        assert!(
            prompt.contains("prefer the GitHub extension capabilities"),
            "web-access.search prompt should route GitHub repository data to GitHub tools"
        );
    }

    #[test]
    fn bundled_extension_summaries_include_onboarding_messages() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();

        for (extension_id, expected_instructions) in [
            ("github", "GitHub needs a personal access token"),
            ("gmail", "Gmail needs Google OAuth authorization"),
            (
                "google-calendar",
                "Google Calendar needs Google OAuth authorization",
            ),
            ("slack", "Slack needs OAuth authorization"),
            ("notion", "Notion needs OAuth authorization"),
            (
                NEARAI_EXTENSION_ID,
                "NEAR AI MCP uses the NEAR AI credentials",
            ),
            ("web-access", "Web Access does not need credentials"),
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id).unwrap();
            let summary = catalog.resolve(&package_ref).unwrap().summary();
            let onboarding = summary
                .onboarding
                .as_ref()
                .expect("bundled extension onboarding");

            assert!(
                onboarding.instructions.contains(expected_instructions),
                "{extension_id} onboarding instructions should include `{expected_instructions}`; got `{}`",
                onboarding.instructions,
            );
            assert!(
                onboarding.credential_next_step.is_some(),
                "{extension_id} must include the next user step"
            );
            if matches!(extension_id, "gmail" | "google-calendar" | "notion") {
                assert!(
                    onboarding
                        .credential_instructions
                        .as_deref()
                        .is_some_and(|instructions| {
                            instructions.starts_with("Authorize ")
                                && !instructions.contains("Install")
                        }),
                    "{extension_id} configure onboarding should not repeat install-first copy"
                );
                assert!(
                    onboarding
                        .credential_next_step
                        .as_deref()
                        .is_some_and(|step| {
                            step.starts_with("After authorization completes")
                                && step.contains("automatically")
                                && !step.contains("Install")
                                && !step.contains("activate")
                        }),
                    "{extension_id} configure next step should describe automatic post-authorization readiness"
                );
            } else if extension_id == "slack" {
                assert!(
                    onboarding
                        .credential_instructions
                        .as_deref()
                        .is_some_and(|instructions| {
                            instructions.starts_with("Authorize ")
                                && instructions.contains("Slack account")
                                && !instructions.contains("pair")
                                && !instructions.contains("Install")
                        }),
                    "{extension_id} configure onboarding should describe Slack OAuth-only copy"
                );
                assert!(
                    onboarding
                        .credential_next_step
                        .as_deref()
                        .is_some_and(|step| {
                            step.starts_with("After authorization completes")
                                && step.contains("DM")
                                && step.contains("Slack bot")
                                && !step.contains("pair")
                                && !step.contains("Install")
                        }),
                    "{extension_id} configure next step should describe DM after OAuth without pairing copy"
                );
            } else if extension_id == "github" {
                assert!(
                    onboarding
                        .credential_instructions
                        .as_deref()
                        .is_some_and(|instructions| {
                            (instructions.contains("Paste") || instructions.contains("paste"))
                                && !instructions.contains("Install")
                        }),
                    "{extension_id} configure onboarding should describe token entry without install-first copy"
                );
                assert!(
                    onboarding
                        .credential_next_step
                        .as_deref()
                        .is_some_and(|step| {
                            step.starts_with("After saving")
                                && step.contains("automatically")
                                && !step.contains("Install")
                                && !step.contains("activate")
                        }),
                    "{extension_id} configure next step should describe automatic readiness after saving credentials"
                );
            } else if extension_id == NEARAI_EXTENSION_ID {
                assert_eq!(
                    onboarding.credential_instructions.as_deref(),
                    Some(
                        "Configure NEAR AI for the assistant with an API key; MCP reuses that credential."
                    )
                );
                assert_eq!(
                    onboarding.credential_next_step.as_deref(),
                    Some(
                        "After NEAR AI is configured for the assistant, IronClaw finishes installation automatically and publishes its tools."
                    )
                );
            } else if extension_id == "web-access" {
                assert_eq!(
                    onboarding.credential_next_step.as_deref(),
                    Some("IronClaw publishes Web Access tools automatically during installation."),
                    "web-access copy should describe automatic readiness"
                );
            } else {
                assert!(
                    onboarding
                        .credential_next_step
                        .as_deref()
                        .is_some_and(
                            |step| step.contains("automatically") && !step.contains("activate")
                        ),
                    "{extension_id} onboarding should describe automatic readiness"
                );
            }
        }
    }

    #[test]
    fn host_managed_credential_extension_detection_is_centralized() {
        let nearai_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, NEARAI_EXTENSION_ID)
                .expect("valid NEAR AI extension ref");
        let notion_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion")
            .expect("valid Notion extension ref");
        let mcp_ref = LifecyclePackageRef::new(LifecyclePackageKind::Mcp, NEARAI_EXTENSION_ID)
            .expect("valid MCP ref");

        assert!(is_host_managed_credential_extension(&nearai_ref));
        assert!(!is_host_managed_credential_extension(&notion_ref));
        assert!(!is_host_managed_credential_extension(&mcp_ref));
    }

    #[test]
    fn bundled_nearai_keeps_runtime_credentials_out_of_browser_setup_summary() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, NEARAI_EXTENSION_ID).unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let summary = package.summary();

        assert!(
            summary.credential_requirements.is_empty(),
            "NEAR AI MCP uses assistant-level NEAR AI credentials and must not \
             project an extension credential setup prompt"
        );

        // Manifest v3: hosted-MCP nearai pins web_search as a static tool so
        // the model can search from first boot (main parity) — the bundled
        // fallback and pre-discovery summary carry exactly that tool, and a
        // successful tools/list discovery replaces the static set with the
        // server's live catalog.
        assert_eq!(
            summary.visible_capability_ids,
            vec!["nearai.web_search".to_string()],
            "hosted-MCP nearai must pin exactly the static web_search tool before discovery"
        );
        let template = package
            .package
            .manifest
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "nearai.mcp_server")
            .expect("nearai hosted-MCP connection template");
        assert_eq!(template.visibility, CapabilityVisibility::HostInternal);
        assert_eq!(template.runtime_credentials.len(), 1);
        assert_eq!(
            template.runtime_credentials[0].handle,
            ironclaw_host_api::SecretHandle::new("llm_nearai_api_key").unwrap()
        );
    }

    #[test]
    fn bundled_notion_projects_oauth_credential_setup() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").unwrap();
        let summary = catalog.resolve(&package_ref).unwrap().summary();

        assert_eq!(summary.credential_requirements.len(), 1);
        let requirement = &summary.credential_requirements[0];
        assert_eq!(requirement.provider, "notion");
        assert!(matches!(
            &requirement.setup,
            LifecycleExtensionCredentialSetup::OAuth { scopes } if scopes.is_empty()
        ));
    }

    #[test]
    fn bundled_slack_search_exposes_one_public_slack_extension() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let slack_results = catalog
            .search("slack")
            .map(|package| package.package_ref.id.as_str().to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            slack_results,
            vec!["slack".to_string()],
            "the unified slack extension is the single Slack catalog entry"
        );
    }

    #[test]
    fn bundled_slack_tools_extension_projects_manifest_declared_personal_oauth_setup() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        // The user-installable Slack extension derives its personal OAuth
        // requirement from the same manifest credential handle used by its
        // runtime tools. There is no package-specific projection override.
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").unwrap();
        let summary = catalog.resolve(&package_ref).unwrap().summary();

        assert_eq!(summary.credential_requirements.len(), 1);
        let requirement = &summary.credential_requirements[0];
        assert_eq!(requirement.name, "slack_user_token");
        assert_eq!(requirement.provider, "slack");
        assert!(requirement.required);
        assert!(matches!(
            &requirement.setup,
            LifecycleExtensionCredentialSetup::OAuth { scopes }
                if scopes.iter().cloned().collect::<BTreeSet<_>>()
                    == [
                        "channels:history",
                        "channels:read",
                        "chat:write",
                        "groups:history",
                        "groups:read",
                        "im:history",
                        "im:read",
                        "mpim:history",
                        "mpim:read",
                        "search:read",
                        "users:read",
                    ]
                    .into_iter()
                    .map(String::from)
                    .collect::<BTreeSet<_>>()
        ));
    }

    #[test]
    fn bundled_google_credentials_project_single_oauth_setup_with_scope_union() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();

        for extension_id in [
            "google-calendar",
            "google-docs",
            "google-drive",
            "google-sheets",
            "google-slides",
            "gmail",
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id).unwrap();
            let package = catalog.resolve(&package_ref).unwrap();
            let summary = package.summary();
            let google_requirements = summary
                .credential_requirements
                .iter()
                .filter(|requirement| requirement.provider == "google")
                .collect::<Vec<_>>();

            // Manifest v3: per-tool least privilege lives in provider_scopes;
            // the OAuth account setup scopes come from the [auth.google]
            // recipe CEILING, uniform across every credential of the vendor
            // and equal to the package's provider-scope union.
            let mut credential_count = 0;
            let mut provider_scope_union = BTreeSet::new();
            let mut setup_scope_sets = Vec::new();
            for capability in &package.package.manifest.capabilities {
                for credential in &capability.runtime_credentials {
                    let RuntimeCredentialRequirementSource::ProductAuthAccount { provider, setup } =
                        &credential.source
                    else {
                        panic!(
                            "{extension_id} capability {} should use a Google product auth account",
                            capability.id
                        );
                    };
                    assert_eq!(provider.as_str(), "google");

                    let RuntimeCredentialAccountSetup::OAuth { scopes } = setup else {
                        panic!(
                            "{extension_id} capability {} should declare OAuth setup",
                            capability.id
                        );
                    };
                    let setup_scopes = scopes.iter().cloned().collect::<BTreeSet<_>>();
                    assert!(
                        credential
                            .provider_scopes
                            .iter()
                            .all(|scope| setup_scopes.contains(scope)),
                        "{extension_id} capability {} provider scopes must fit inside the recipe ceiling",
                        capability.id
                    );
                    provider_scope_union.extend(credential.provider_scopes.iter().cloned());
                    setup_scope_sets.push(setup_scopes);
                    credential_count += 1;
                }
            }

            assert!(
                credential_count > 0,
                "{extension_id} should declare runtime credentials"
            );
            assert!(
                setup_scope_sets.windows(2).all(|pair| pair[0] == pair[1]),
                "{extension_id} OAuth setup scopes (the vendor ceiling) must be uniform across credentials"
            );
            assert_eq!(
                setup_scope_sets[0], provider_scope_union,
                "{extension_id} recipe ceiling should equal the union of the tools' provider scopes"
            );

            assert_eq!(
                google_requirements.len(),
                1,
                "{extension_id} lifecycle setup should show one Google OAuth request"
            );
            let LifecycleExtensionCredentialSetup::OAuth { scopes } = &google_requirements[0].setup
            else {
                panic!("{extension_id} should expose Google OAuth setup");
            };
            assert_eq!(
                scopes.iter().cloned().collect::<BTreeSet<_>>(),
                provider_scope_union,
                "{extension_id} lifecycle setup should include every capability OAuth scope"
            );
        }
    }

    #[test]
    fn bundled_gsuite_wasm_capabilities_are_operation_scoped() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-drive").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let capabilities = package
            .package
            .manifest
            .capabilities
            .iter()
            .map(|capability| (capability.id.as_str(), capability))
            .collect::<HashMap<_, _>>();

        assert!(!capabilities.contains_key("google-drive.execute"));
        assert!(capabilities.contains_key("google-drive.list_files"));
        assert!(capabilities.contains_key("google-drive.upload_file"));

        let summary = package.summary();
        assert!(
            summary
                .visible_capability_ids
                .contains(&"google-drive.upload_file".to_string())
        );
        assert!(
            summary
                .visible_read_only_capability_ids
                .contains(&"google-drive.list_files".to_string())
        );
        assert!(
            !summary
                .visible_read_only_capability_ids
                .contains(&"google-drive.upload_file".to_string())
        );

        let list_files = capabilities["google-drive.list_files"];
        assert_eq!(
            list_files.runtime_credentials[0].provider_scopes,
            vec!["https://www.googleapis.com/auth/drive.readonly".to_string()]
        );
        assert!(!list_files.effects.contains(&EffectKind::ExternalWrite));

        let upload_file = capabilities["google-drive.upload_file"];
        assert!(upload_file.effects.contains(&EffectKind::ExternalWrite));
    }

    /// §5.3 S3 (behavior-neutral): a bundled extension's manifest capabilities
    /// carry a declared `origin_gate_matrix` that survives derivation into the
    /// kernel `CapabilityDescriptor`. Every extension capability declares the
    /// `network` effect, so all are GATED-today and thus
    /// `loop_run = GatedUnlessGranted`; none is in the Ungated allowlist.
    /// `Product`/`Automation` are deny-by-default. Driven through the real asset
    /// (`include_str!` manifest.toml) so a TOML typo or dropped field fails here.
    #[test]
    fn bundled_extension_capabilities_carry_behavior_neutral_origin_gate_matrix() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        for id in [
            "github",
            "gmail",
            "google-calendar",
            "google-docs",
            "google-drive",
            "google-sheets",
            "google-slides",
            "slack",
            "notion",
            "web-access",
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).unwrap();
            let package = catalog.resolve(&package_ref).unwrap();
            // Assert on the derived kernel descriptors, proving the matrix
            // survives `capability_descriptors_from_manifest`.
            assert!(
                !package.package.capabilities.is_empty(),
                "{id} must declare capabilities"
            );
            for descriptor in &package.package.capabilities {
                let matrix = descriptor.origin_gate_matrix.as_ref().unwrap_or_else(|| {
                    panic!("{} must declare an origin_gate_matrix", descriptor.id)
                });
                assert_eq!(
                    matrix.loop_run,
                    OriginGatePolicy::GatedUnlessGranted,
                    "{} is a credentialed/networked extension cap and must gate LoopRun",
                    descriptor.id
                );
                assert_eq!(
                    matrix.product,
                    OriginGatePolicy::Forbidden,
                    "{}",
                    descriptor.id
                );
                assert_eq!(
                    matrix.automation,
                    OriginGatePolicy::Forbidden,
                    "{}",
                    descriptor.id
                );
                assert!(
                    !UNGATED_LOOP_RUN_CAPABILITIES.contains(&descriptor.id.as_str()),
                    "no extension capability may appear in the Ungated allowlist: {}",
                    descriptor.id
                );
            }
        }
    }

    #[test]
    fn bundled_slack_package_declares_product_adapter_channel_surface() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();

        assert_eq!(package.package.manifest.id.as_str(), "slack");
        assert!(matches!(
            package.package.manifest.runtime,
            ExtensionRuntime::Wasm { .. }
        ));
        assert!(
            !package.package.manifest.capabilities.is_empty(),
            "unified slack declares the user-scoped tool capabilities"
        );
        // Manifest v3 declares the channel surface directly ([channel]); the
        // [[host_api]] product-adapter contract indirection is gone, so
        // host_apis is empty and the channel arrives as a projected
        // CapabilitySurfaceDeclV2::Channel surface.
        assert!(
            package.package.manifest.host_apis.is_empty(),
            "v3 manifests carry no host_api contract refs"
        );
        assert!(
            package
                .package
                .manifest
                .host_api_surfaces
                .iter()
                .any(|surface| matches!(
                    surface,
                    ironclaw_extensions::CapabilitySurfaceDeclV2::Channel { .. }
                )),
            "unified slack declares a channel surface"
        );

        let summary = package.summary();
        assert_eq!(
            summary.surface_kinds,
            // Derivation order: tools, then host-API projected surfaces
            // (channel), then auth (per provider).
            vec![
                CapabilitySurfaceKind::Tool,
                CapabilitySurfaceKind::Channel,
                CapabilitySurfaceKind::Auth,
            ],
            "unified slack projects tool + auth + channel surfaces"
        );
        // OUT-11: the channel's declared [channel.presentation] projects onto
        // the lifecycle summary, which feeds prompt construction.
        let presentation = summary
            .channel_presentation
            .as_ref()
            .expect("slack declares [channel.presentation]");
        assert!(
            presentation.supports_markdown,
            "slack declares supports_markdown = true"
        );
        assert_eq!(
            presentation.max_message_chars,
            Some(40_000),
            "slack declares max_message_chars = 40000"
        );
        let directions = summary
            .channel_directions
            .expect("unified slack summary carries channel directions");
        assert!(directions.inbound, "Slack channel surface is inbound");
        assert!(directions.outbound, "Slack channel surface is outbound");
        assert!(
            !summary.visible_capability_ids.is_empty(),
            "unified slack publishes model-visible tools"
        );
    }

    #[test]
    fn non_channel_product_adapter_surface_does_not_project_channel_surface() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "web-product"
name = "Web Product"
version = "0.1.0"
description = "A web product adapter."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "web_product"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.web"

[product_adapter.web]
surface_kind = "web"

[product_adapter.web.auth]
kind = "bearer_token"

[product_adapter.web.capabilities]
flags = ["inbound_messages"]

[[product_adapter.web.required_credentials]]
handle = "web_token"
"#;

        let package = bundled_extension_package(
            "web-product",
            "Web Product",
            MANIFEST,
            vec![bytes_asset("manifest.toml", MANIFEST.as_bytes())],
        )
        .expect("valid package");

        assert_eq!(package.summary().surface_kinds, Vec::new());
    }

    #[tokio::test]
    async fn materialize_bundled_github_writes_manifest_schema_refs() {
        let fs = InMemoryBackend::default();
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").unwrap();
        let github = catalog.resolve(&package_ref).unwrap();

        materialize_available_extension(&fs, &github).await.unwrap();

        let get_repo_schema = fs
            .read_file(
                &VirtualPath::new(
                    "/system/extensions/github/schemas/github/get_repo.input.v1.json",
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            std::str::from_utf8(&get_repo_schema)
                .unwrap()
                .contains("GitHub get_repo input")
        );
        fs.read_file(
            &VirtualPath::new("/system/extensions/github/prompts/github/get_repo.md").unwrap(),
        )
        .await
        .unwrap();

        let update_issue_schema = fs
            .read_file(
                &VirtualPath::new(
                    "/system/extensions/github/schemas/github/update_issue.input.v1.json",
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            std::str::from_utf8(&update_issue_schema)
                .unwrap()
                .contains("GitHub update_issue input")
        );
        fs.read_file(
            &VirtualPath::new("/system/extensions/github/prompts/github/update_issue.md").unwrap(),
        )
        .await
        .unwrap();
    }

    #[test]
    fn bundled_manifest_digests_are_sha256_tokens() {
        // gmail migrated to the inventory; its digest (still sha256-token) is
        // asserted through the trust policy in
        // `factory::tests::builtin_first_party_trust_policy_grants_migrated_gmail_via_inventory`.
    }

    #[test]
    fn nearai_manifest_renderer_uses_validated_endpoint_fields() {
        let endpoint =
            nearai_mcp_endpoint_from_base(Some("https://10.0.0.12:8443/%22%0Atrust=%22system"))
                .unwrap();

        let manifest_toml = nearai_mcp_manifest_toml_for_endpoint(&endpoint).unwrap();
        let manifest: Value = toml::from_str(&manifest_toml).unwrap();

        assert_eq!(manifest["trust"].as_str(), Some("first_party_requested"));
        // Manifest v3: the endpoint override patches exactly one field —
        // [mcp].server. The URL-encoded injection attempt stays inert data in
        // that one string; trust is untouched.
        assert_eq!(
            manifest["mcp"]["server"].as_str(),
            Some("https://10.0.0.12:8443/%22%0Atrust=%22system/mcp")
        );

        // The connection credential's audience is not rendered into the TOML;
        // it derives from the [mcp].server host automatically when the
        // manifest is parsed.
        let package = bundled_extension_package(
            NEARAI_EXTENSION_ID,
            "NEAR AI",
            &manifest_toml,
            nearai_mcp_assets(&manifest_toml),
        )
        .expect("patched NEAR AI manifest parses");
        let template = package
            .package
            .manifest
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "nearai.mcp_server")
            .expect("nearai hosted-MCP connection template");
        assert_eq!(
            template.runtime_credentials[0].audience.host_pattern,
            "10.0.0.12"
        );
    }

    #[test]
    fn catalog_extend_replaces_duplicate_package_refs() {
        let stale = test_extension_package_with_wasm_bytes(b"stale");
        let bundled = test_extension_package_with_wasm_bytes(b"bundled");
        let mut catalog = AvailableExtensionCatalog::from_packages(vec![stale]);
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![bundled]));

        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let wasm = package
            .assets
            .iter()
            .find(|asset| asset.path == "wasm/fixture.wasm")
            .expect("wasm asset");

        assert_eq!(
            wasm.content,
            AvailableExtensionAssetContent::Bytes(b"bundled".to_vec())
        );
        assert_eq!(catalog.search("fixture").count(), 1);
    }

    #[tokio::test]
    async fn materialize_fails_on_filesystem_error_and_rolls_back_written_assets() {
        // The wasm write is faulted at the backend; the real materialize path
        // then rolls back the manifest write it already committed. `recorded_*`
        // proves the true op stream (writes include the faulted wasm write; the
        // rollback deletes the one prior committed asset).
        let fs = FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("fixture.wasm")
                .backend("write rejected"),
        );
        let extension = test_extension_package();

        let error = materialize_available_extension(&fs, &extension)
            .await
            .expect_err("second write fails");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        let writes = fs
            .recorded_paths(FilesystemOperation::WriteFile)
            .iter()
            .map(|path| path.as_str().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            writes,
            vec![
                "/system/extensions/fixture/manifest.toml".to_string(),
                "/system/extensions/fixture/wasm/fixture.wasm".to_string()
            ]
        );
        let deletes = fs
            .recorded_paths(FilesystemOperation::Delete)
            .iter()
            .map(|path| path.as_str().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            deletes,
            vec!["/system/extensions/fixture/manifest.toml".to_string()]
        );
    }

    #[tokio::test]
    async fn materialize_skips_matching_existing_assets() {
        let fs = RecordingMaterializeFilesystem::default();
        let extension = test_extension_package();
        for asset in &extension.assets {
            let path = extension_asset_path(&extension.package.id, &asset.path).unwrap();
            let AvailableExtensionAssetContent::Bytes(bytes) = &asset.content;
            fs.files
                .lock()
                .unwrap()
                .insert(path.as_str().to_string(), bytes.clone());
        }

        materialize_available_extension(&fs, &extension)
            .await
            .expect("matching assets already materialized");

        assert!(
            fs.writes.lock().unwrap().is_empty(),
            "restore should not rewrite already materialized matching assets"
        );
    }

    #[tokio::test]
    async fn filesystem_catalog_loads_manifest_and_runtime_assets() {
        let fs = InMemoryBackend::default();
        let extension = test_extension_package();
        for asset in &extension.assets {
            let path = extension_asset_path(&extension.package.id, &asset.path).unwrap();
            let AvailableExtensionAssetContent::Bytes(bytes) = &asset.content;
            fs.write_file(&path, bytes).await.unwrap();
        }

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .unwrap();
        let results = catalog.search("fixture").collect::<Vec<_>>();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].package_ref, extension.package_ref);
        assert_eq!(
            results[0]
                .assets
                .iter()
                .map(|asset| asset.path.as_str())
                .collect::<Vec<_>>(),
            vec!["manifest.toml", "wasm/fixture.wasm"]
        );
    }

    #[tokio::test]
    async fn filesystem_catalog_skips_extension_dirs_without_manifest() {
        let fs = InMemoryBackend::default();
        fs.write_file(
            &VirtualPath::new("/system/extensions/incomplete/cache/leftover").unwrap(),
            b"stale",
        )
        .await
        .unwrap();

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .unwrap();

        assert_eq!(catalog.search("").count(), 0);
    }

    #[tokio::test]
    async fn filesystem_catalog_skips_reserved_host_bundled_extension_ids() {
        let fs = InMemoryBackend::default();
        write_valid_filesystem_extension(&fs, "gmail").await;
        write_valid_filesystem_extension(&fs, "slack").await;

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &["gmail".to_string(), "slack".to_string()],
        )
        .await
        .unwrap();

        assert_eq!(catalog.search("").count(), 0);
        // The reserved host-bundled id is skipped on filesystem load; the
        // unified slack package ships built-in, not from this directory.
        assert_eq!(catalog.search("slack").count(), 0);
    }

    #[tokio::test]
    async fn filesystem_manifest_external_channel_surface_kind_projects_to_lifecycle_surface() {
        // Filesystem-discovered manifests validate as `InstalledLocal`, which
        // forbids first-party trust/runtime claims — so the fixture is a
        // third-party wasm channel adapter. (First-party channel adapters like
        // Slack ship compiled into the binary via `from_first_party_assets`,
        // not via filesystem discovery.)
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "channel-ext"
name = "Channel Ext"
version = "0.1.0"
description = "A filesystem-discovered external channel extension."
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/channel.wasm"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "request_signature"
header_name = "X-Channel-Signature"
timestamp_header_name = "X-Channel-Timestamp"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages"]

[[product_adapter.inbound.required_credentials]]
handle = "channel_ext_token"

[[product_adapter.inbound.egress]]
host = "example.com"
credential_handle = "channel_ext_token"
"#;

        let fs = InMemoryBackend::default();
        fs.write_file(
            &VirtualPath::new("/system/extensions/channel-ext/manifest.toml").unwrap(),
            MANIFEST.as_bytes(),
        )
        .await
        .unwrap();

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .unwrap();

        let results = catalog.search("channel-ext").collect::<Vec<_>>();
        assert_eq!(results.len(), 1, "filesystem manifest should be loaded");

        let package = results.into_iter().next().unwrap();
        assert_eq!(
            package.summary().surface_kinds,
            vec![CapabilitySurfaceKind::Channel],
            "filesystem-loaded external_channel manifest must project ExternalChannel surface kind"
        );
        assert!(
            package.cleanup_requirements.is_empty(),
            "ExternalChannel presentation metadata must not infer host-owned cleanup"
        );
    }

    #[derive(Default)]
    struct RecordingMaterializeFilesystem {
        files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        writes: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl RootFilesystem for RecordingMaterializeFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::default()
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
            })
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            let files = self.files.lock().unwrap();
            let Some(bytes) = files.get(path.as_str()) else {
                return Err(FilesystemError::NotFound {
                    path: path.clone(),
                    operation: FilesystemOperation::Stat,
                });
            };
            Ok(FileStat {
                path: path.clone(),
                file_type: FileType::File,
                len: bytes.len() as u64,
                modified: Some(SystemTime::UNIX_EPOCH),
                sensitive: false,
            })
        }

        async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            self.files
                .lock()
                .unwrap()
                .get(path.as_str())
                .cloned()
                .ok_or_else(|| FilesystemError::NotFound {
                    path: path.clone(),
                    operation: FilesystemOperation::ReadFile,
                })
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            self.writes.lock().unwrap().push(path.as_str().to_string());
            self.files
                .lock()
                .unwrap()
                .insert(path.as_str().to_string(), bytes.to_vec());
            Ok(())
        }
    }

    fn test_extension_package() -> AvailableExtensionPackage {
        test_extension_package_with_wasm_bytes(b"wasm")
    }

    async fn write_valid_filesystem_extension(fs: &InMemoryBackend, id: &str) {
        let manifest = format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "shadowing fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.search"
description = "Search"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#
        );
        fs.write_file(
            &VirtualPath::new(format!("/system/extensions/{id}/manifest.toml")).unwrap(),
            manifest.as_bytes(),
        )
        .await
        .unwrap();
        fs.write_file(
            &VirtualPath::new(format!("/system/extensions/{id}/wasm/{id}.wasm")).unwrap(),
            b"wasm",
        )
        .await
        .unwrap();
    }

    fn test_extension_package_with_wasm_bytes(wasm_bytes: &[u8]) -> AvailableExtensionPackage {
        static MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture"
version = "0.1.0"
description = "fixture extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "fixture.search"
description = "Search"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capability_provider.tools.capabilities]]
id = "fixture.write"
description = "Write"
effects = ["external_write"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/write.input.json"
output_schema_ref = "schemas/write.output.json"
"#;
        let manifest = ExtensionManifest::parse(
            MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            &capability_provider_contracts(),
        )
        .expect("manifest");
        let resolved_manifest = Arc::new(
            ExtensionManifestRecord::from_toml(
                MANIFEST,
                ManifestSource::HostBundled,
                &HostPortCatalog::empty(),
                None,
                &capability_provider_contracts(),
            )
            .expect("resolved manifest")
            .resolved()
            .clone(),
        );
        let package = ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/fixture").unwrap(),
        )
        .expect("package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
                .unwrap(),
            manifest_toml: MANIFEST.to_string(),
            resolved_manifest,
            source: ManifestSource::HostBundled,
            package,
            cleanup_requirements: Vec::new(),
            surface_kinds: Vec::new(),
            channel_directions: None,
            channel_presentation: None,
            assets: vec![
                AvailableExtensionAsset {
                    path: "manifest.toml".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(MANIFEST.as_bytes().to_vec()),
                },
                AvailableExtensionAsset {
                    path: "wasm/fixture.wasm".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(wasm_bytes.to_vec()),
                },
            ],
            onboarding_override: None,
            oauth_setup_override: None,
            search_aliases: Vec::new(),
        }
    }
}
