//! Extension Manifest v2.
//!
//! v2 is the manifest shape consumed by Reborn. It is intentionally additive in
//! this slice: this module ships alongside the v1 parser so downstream crates
//! can migrate one at a time. The v1 parser is deleted in the follow-up slice
//! once every caller is on v2.
//!
//! Key contract changes from v1:
//! - manifests carry a loader-supplied [`ManifestSource`]; first-party / system
//!   trust and runtime are only ever effective for [`ManifestSource::HostBundled`];
//! - extension IDs starting with `ironclaw.` are reserved for HostBundled;
//! - every manifest declares at least one `[[host_api]]` contract; top-level
//!   `[[capabilities]]` is rejected — capabilities are declared under
//!   `ironclaw.capability_provider/v1` sections;
//! - installed manifests must use `wasm` / `mcp` / `script` runtimes only;
//! - every capability declares `visibility`, relative
//!   [`CapabilityProfileSchemaRef`] input/output schema refs, optional lazy
//!   `prompt_doc_ref`, and the set of host ports it needs;
//! - host port names validate against a host-defined [`HostPortCatalog`].
//!
//! This module does **not** dispatch capabilities, load WASM modules, evaluate
//! trust policy, or grant authority. It is contract vocabulary only.
//!
//! ## Whitespace and field shape
//!
//! `name`, `version`, and `description` are rejected when empty or
//! whitespace-only, but the *exact* bytes from the TOML are preserved on the
//! validated manifest (no `trim`). `version` is treated as opaque,
//! registry-defined vocabulary — v2 does **not** require semver; downstream
//! consumers that need ordered comparison must parse it themselves.
//!
//! ## Serialization
//!
//! v2 deliberately ships `Deserialize`-only types. `ExtensionManifestV2` has
//! no `Serialize` impl: this module is a parser/validator contract, not a
//! registry write path. If a future diagnostic / registry tool needs round
//! tripping it should add a deliberate serialization layer with its own
//! schema.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, CapabilitySurfaceKind, EffectKind, ExtensionId,
    HostApiError, HostPortCatalog, HostPortId, NetworkScheme, NetworkTargetPattern,
    OriginGateMatrix, PermissionMode, RequestedTrustClass, ResourceProfile,
    RuntimeCredentialAccountSetup, RuntimeCredentialRequirement,
    RuntimeCredentialRequirementSource, RuntimeCredentialTarget, RuntimeKind, SecretHandle,
    TrustClass, VendorId,
};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

/// Required value of the `schema_version` field for v2 manifests.
pub const MANIFEST_SCHEMA_VERSION: &str = "reborn.extension_manifest.v2";

/// Reserved extension-ID prefix for host-bundled extensions.
pub const RESERVED_HOST_BUNDLED_ID_PREFIX: &str = "ironclaw.";

/// Upper bound on raw manifest TOML input size.
///
/// Loaders feed installed manifests of ≤ a few KB; this cap exists to fail
/// closed before `toml::from_str` parses and allocates a pathological input.
/// Tune cautiously — raising this also raises peak loader memory.
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// Maximum number of `[[hooks]]` entries a single manifest may declare.
///
/// This is a *structural* bound enforced by `ironclaw_extensions` at parse
/// time so a hostile or buggy manifest cannot make the parser allocate an
/// unbounded vector of hook entries. It intentionally matches the
/// per-extension registration ceiling enforced downstream by the hook
/// registrar (`ironclaw_hooks::registrar::MAX_HOOKS_PER_EXTENSION`); the
/// registrar re-checks the cap (cumulatively across install batches) when
/// the composition loader installs these entries, so this crate does not
/// depend on the hook crate to know the value — it just refuses to parse a
/// manifest that could never install cleanly anyway.
pub const MAX_MANIFEST_HOOKS: usize = 32;

/// Maximum serialized size, in bytes, of a single `[[hooks]]` entry's body.
///
/// The hook-declaration section is carried as an opaque, structurally-typed
/// TOML payload (see [`HookSectionEntryV2`]) so `ironclaw_extensions` never
/// imports the hook predicate vocabulary. This cap bounds the per-entry blob
/// the parser retains before the composition loader projects it into a typed
/// `ironclaw_hooks::HookManifestEntry` (which applies its own field-level
/// bounds during validation).
pub const MAX_HOOK_ENTRY_BYTES: usize = 8 * 1024;

/// A single hook declaration carried on an `ExtensionManifestV2`.
///
/// **Clean-boundary contract:** `ironclaw_extensions` is substrate and must
/// not depend on `ironclaw_hooks`. So this DTO does *not* embed the typed
/// hook entry (`ironclaw_hooks::manifest::HookManifestEntry`). Instead it
/// captures the raw, deserialized TOML for the entry as a structurally-typed
/// [`toml::Value`] table plus the local id string. The composition-layer
/// loader (`ironclaw_reborn_composition`, which depends on *both* crates)
/// is the single seam that projects this raw payload into a typed
/// `HookManifestEntry`, validates it, and installs it through
/// `HookRegistrar::install` at the `Installed` trust tier.
///
/// What this crate validates: the entry is a table, carries a non-empty
/// `id`, and its serialized size is within [`MAX_HOOK_ENTRY_BYTES`]. It does
/// **not** interpret `kind`, `body`, `phase`, predicate trees, or windows —
/// that is the hook crate's job, applied at the composition seam.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct HookSectionEntryV2 {
    /// The manifest-local hook id (the `id` field of the TOML entry).
    /// Surfaced separately so the loader and diagnostics can identify the
    /// entry without re-parsing the raw payload.
    pub local_id: String,
    /// The complete raw TOML body for this hook entry, including `id`,
    /// re-serialized to a canonical TOML string. Projected into a typed
    /// `ironclaw_hooks::HookManifestEntry` by the composition loader (via
    /// `toml::from_str`). Kept as an opaque string — rather than a
    /// `toml::Value` — so this crate stays free of the hook predicate
    /// vocabulary *and* so the enclosing manifest can keep deriving `Eq`
    /// (`toml::Value` is not `Eq` because it can hold floats).
    pub raw_toml: String,
}

/// Loader-supplied source for a manifest. Never read from TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManifestSource {
    /// Compiled or bundled with the host binary. Only source eligible for
    /// effective FirstParty/System trust and runtime.
    HostBundled,
    /// Locally installed extension under `/system/extensions/`. Never eligible
    /// for effective FirstParty/System.
    InstalledLocal,
    /// Installed from registry/catalog with digest/signature metadata. Never
    /// eligible for effective FirstParty/System in v2.
    RegistryInstalled,
}

impl ManifestSource {
    /// True if the source is allowed to assert FirstParty/System trust/runtime.
    pub fn allows_first_party(self) -> bool {
        matches!(self, Self::HostBundled)
    }
}

/// Per-capability surface visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityVisibility {
    /// Visible to the model through the Hot Capability Surface.
    Model,
    /// Used by host-internal flows (memory injection, audit) only.
    HostInternal,
    /// Reachable through the gateway/API surface only.
    Api,
}

/// Host API contract identifier declared by an extension manifest.
///
/// This is the manifest-level contract discriminator, for example
/// `ironclaw.product_adapter/v1` or `ironclaw.capability_provider/v1`. It is
/// not a runtime kind, trust level, permission grant, or adapter factory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostApiId(String);

impl HostApiId {
    pub fn new(value: impl Into<String>) -> Result<Self, ManifestV2Error> {
        let value = value.into();
        validate_host_api_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HostApiId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Dotted path to a manifest section owned by a host API contract handler.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ManifestSectionPath(String);

impl ManifestSectionPath {
    pub fn new(value: impl Into<String>) -> Result<Self, ManifestV2Error> {
        let value = value.into();
        validate_section_path(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('.')
    }

    fn is_prefix_of(&self, other: &ManifestSectionPath) -> bool {
        other
            .as_str()
            .strip_prefix(self.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    }
}

impl std::fmt::Display for ManifestSectionPath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One host API contract instance declared by a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostApiRefV2 {
    pub id: HostApiId,
    pub section: ManifestSectionPath,
}

/// Contract-defined cardinality for repeated host API instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostApiMultiplicity {
    Single,
    Multiple,
}

/// Manifest-level context available to host API contract validators.
///
/// This is validation-only metadata. It must not be treated as runtime
/// authority, and it must not trigger side effects.
pub struct HostApiManifestContext<'a> {
    pub extension_id: &'a ExtensionId,
    pub host_port_catalog: &'a HostPortCatalog,
}

/// Neutral manifest projection produced by host API contract validators.
///
/// Projections keep host API section parsing separate from runtime publication:
/// contracts can publish already-validated manifest declarations without wiring
/// hot descriptors, dispatch, or domain-specific read models.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostApiManifestProjection {
    pub capabilities: Vec<CapabilityDeclV2>,
    /// Product-facing surface kinds the validated section declares (e.g. a
    /// `channel` surface for an external-channel product-adapter section).
    /// Tool and auth kinds are rejected here — they have dedicated
    /// declaration paths (capability declarations and product-auth
    /// credential requirements); see [`CapabilitySurfaceDeclV2`].
    pub surfaces: Vec<CapabilitySurfaceKind>,
}

/// Error a host API contract raises for one manifest section.
///
/// Contracts that validate with this crate's own vocabulary (the
/// capability-provider contract parses [`CapabilityDeclV2`] declarations)
/// preserve the typed [`ManifestV2Error`] so callers keep precise variants
/// (`DuplicateEffect`, `UnknownHostPort`, ...). Domain crates outside this
/// crate report a redacted reason string, which the parse path wraps as
/// [`ManifestV2Error::HostApiSectionRejected`].
#[derive(Debug)]
pub enum HostApiSectionError {
    Manifest(Box<ManifestV2Error>),
    Contract(String),
}

impl From<ManifestV2Error> for HostApiSectionError {
    fn from(error: ManifestV2Error) -> Self {
        Self::Manifest(Box::new(error))
    }
}

impl From<String> for HostApiSectionError {
    fn from(reason: String) -> Self {
        Self::Contract(reason)
    }
}

impl From<&str> for HostApiSectionError {
    fn from(reason: &str) -> Self {
        Self::Contract(reason.to_string())
    }
}

/// Host API contract validator registered by composition.
///
/// `ironclaw_extensions` owns the generic envelope and section dispatch. Domain
/// crates own section patterns, cardinality, typed section schema validation,
/// and projection into their read models.
pub trait HostApiManifestContract: Send + Sync {
    fn id(&self) -> &HostApiId;

    fn multiplicity(&self) -> HostApiMultiplicity {
        HostApiMultiplicity::Single
    }

    fn accepts_section_path(&self, section: &ManifestSectionPath) -> bool;

    fn validate_section(
        &self,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), HostApiSectionError>;

    fn validate_section_with_context(
        &self,
        context: &HostApiManifestContext<'_>,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), HostApiSectionError> {
        let _ = context;
        self.validate_section(host_api, section)
    }

    fn project_section_with_context(
        &self,
        context: &HostApiManifestContext<'_>,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<HostApiManifestProjection, HostApiSectionError> {
        self.validate_section_with_context(context, host_api, section)?;
        Ok(HostApiManifestProjection::default())
    }
}

/// Composition-wired registry of host API manifest contracts.
pub struct HostApiContractRegistry {
    contracts: BTreeMap<HostApiId, Arc<dyn HostApiManifestContract>>,
}

impl HostApiContractRegistry {
    pub fn new() -> Self {
        Self {
            contracts: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        contract: Arc<dyn HostApiManifestContract>,
    ) -> Result<(), ManifestV2Error> {
        let id = contract.id().clone();
        if self.contracts.contains_key(&id) {
            return Err(ManifestV2Error::DuplicateHostApiContractRegistration { id });
        }
        self.contracts.insert(id, contract);
        Ok(())
    }

    fn project_manifest(
        &self,
        manifest: &ExtensionManifestV2,
        sections: &ManifestSectionsV2,
        host_port_catalog: &HostPortCatalog,
    ) -> Result<ProjectedManifestV2, ManifestV2Error> {
        let mut counts: BTreeMap<&HostApiId, usize> = BTreeMap::new();
        let mut projected = ProjectedManifestV2::default();
        let mut seen_capabilities = BTreeSet::new();
        for host_api in &manifest.host_apis {
            let contract = self.contracts.get(&host_api.id).ok_or_else(|| {
                ManifestV2Error::UnknownHostApi {
                    id: host_api.id.clone(),
                }
            })?;
            let count = counts.entry(&host_api.id).or_insert(0);
            *count += 1;
            if *count > 1 && contract.multiplicity() != HostApiMultiplicity::Multiple {
                return Err(ManifestV2Error::DuplicateHostApi {
                    id: host_api.id.clone(),
                });
            }
            if !contract.accepts_section_path(&host_api.section) {
                return Err(ManifestV2Error::HostApiSectionRejected {
                    id: host_api.id.clone(),
                    section: host_api.section.clone(),
                    reason: "section path is not accepted by host API contract".to_string(),
                });
            }
            let section = sections.get(&host_api.section)?;
            let context = HostApiManifestContext {
                extension_id: &manifest.id,
                host_port_catalog,
            };
            let section_projection = contract
                .project_section_with_context(&context, host_api, section)
                .map_err(|error| match error {
                    HostApiSectionError::Manifest(error) => *error,
                    HostApiSectionError::Contract(reason) => {
                        ManifestV2Error::HostApiSectionRejected {
                            id: host_api.id.clone(),
                            section: host_api.section.clone(),
                            reason,
                        }
                    }
                })?;
            for capability in section_projection.capabilities {
                if !seen_capabilities.insert(capability.id.clone()) {
                    return Err(ManifestV2Error::DuplicateCapability { id: capability.id });
                }
                projected.capabilities.push(capability);
            }
            for kind in section_projection.surfaces {
                if matches!(
                    kind,
                    CapabilitySurfaceKind::Tool | CapabilitySurfaceKind::Auth
                ) {
                    // Fail closed: tool and auth surfaces derive from their
                    // dedicated declaration paths. A contract projecting them
                    // as opaque section surfaces is an implementation bug.
                    return Err(ManifestV2Error::HostApiSectionRejected {
                        id: host_api.id.clone(),
                        section: host_api.section.clone(),
                        reason: format!(
                            "host API contracts must not project '{kind}' section surfaces; \
                             tool surfaces derive from capability declarations and auth \
                             surfaces from product-auth credential requirements"
                        ),
                    });
                }
                projected
                    .surfaces
                    .push(CapabilitySurfaceDeclV2::HostApiSection {
                        kind,
                        host_api: host_api.id.clone(),
                        section: host_api.section.clone(),
                    });
            }
        }
        sections.reject_unreferenced_operational_sections(&manifest.host_apis)?;
        Ok(projected)
    }
}

impl Default for HostApiContractRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Aggregate of every host API contract projection for one manifest:
/// projected capability declarations plus origin-stamped section surfaces.
/// Internal to the parse path — contracts see [`HostApiManifestProjection`].
#[derive(Debug, Default)]
struct ProjectedManifestV2 {
    capabilities: Vec<CapabilityDeclV2>,
    surfaces: Vec<CapabilitySurfaceDeclV2>,
}

/// Validated v2 capability declaration.
///
/// Carries `Serialize`/`Deserialize` as part of the resolved-record
/// persistence layer (`crate::resolved`): records are produced by this
/// crate's validators and rehydrated from the trusted installation store;
/// component newtypes re-validate on deserialize.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct CapabilityDeclV2 {
    pub id: CapabilityId,
    pub description: String,
    pub effects: Vec<EffectKind>,
    pub default_permission: PermissionMode,
    pub visibility: CapabilityVisibility,
    pub input_schema_ref: CapabilityProfileSchemaRef,
    /// Optional since manifest v3 dropped output schema declarations;
    /// v2 manifests may still carry one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema_ref: Option<CapabilityProfileSchemaRef>,
    pub prompt_doc_ref: Option<CapabilityProfileSchemaRef>,
    pub required_host_ports: Vec<HostPortId>,
    pub runtime_credentials: Vec<RuntimeCredentialRequirement>,
    /// Declared network egress targets, independent of runtime credentials.
    /// A capability that declares the `network` effect but no credential uses
    /// this to populate its egress allowlist directly from the manifest.
    pub network_targets: Vec<NetworkTargetPattern>,
    /// Optional per-capability egress cap (bytes), independent of credentials.
    /// A networked capability uses this to bound its egress from the manifest
    /// rather than a composition special-case. `#[serde(default)]` keeps
    /// persisted records without the field parsing to `None`.
    #[serde(default)]
    pub max_egress_bytes: Option<u64>,
    pub resource_profile: Option<ResourceProfile>,
    /// Declared per-origin gate matrix (§5.2.1). `None` = undeclared; a later
    /// slice populates real matrices and threads this into authorization.
    pub origin_gate_matrix: Option<OriginGateMatrix>,
}

/// One product-facing surface a validated manifest declares, with the
/// manifest declaration it derives from.
///
/// The surface set answers "which faces of this extension can be configured
/// and enabled?" — tools, an external channel, provider accounts. It is
/// derived vocabulary: the owning declarations (capability entries, host API
/// sections, credential requirements) stay the single source of truth, and
/// [`ExtensionManifestV2::capability_surfaces`] projects them on demand.
/// Runtime kind deliberately plays no part in this taxonomy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySurfaceDeclV2 {
    /// A model/host-callable capability. One per capability declaration
    /// (top-level or host-API projected).
    Tool { capability: CapabilityId },
    /// A provider-account requirement, derived from `product_auth_account`
    /// runtime-credential sources across all capabilities: one surface per
    /// distinct provider. When several requirements name the same provider,
    /// OAuth setups fold to the union of their scopes (sorted, deduplicated)
    /// and mask weaker manual-token setups — the account is shared, so its
    /// grant is the union of what the declaring capabilities need.
    Auth {
        provider: VendorId,
        setup: RuntimeCredentialAccountSetup,
    },
    /// The extension's declared channel surface (manifest v3 `[channel]`).
    /// The full descriptor lives on the resolved contract; this carries the
    /// channel surface id for surface enumeration.
    Channel { channel: String },
    /// A surface projected by a host API contract section, stamped with the
    /// owning contract id and section path (e.g. a `channel` surface from an
    /// `ironclaw.product_adapter/v1` external-channel section).
    HostApiSection {
        kind: CapabilitySurfaceKind,
        host_api: HostApiId,
        section: ManifestSectionPath,
    },
}

impl CapabilitySurfaceDeclV2 {
    pub fn kind(&self) -> CapabilitySurfaceKind {
        match self {
            Self::Tool { .. } => CapabilitySurfaceKind::Tool,
            Self::Auth { .. } => CapabilitySurfaceKind::Auth,
            Self::Channel { .. } => CapabilitySurfaceKind::Channel,
            Self::HostApiSection { kind, .. } => *kind,
        }
    }
}

/// v2 runtime declaration.
///
/// Serde derives exist for the resolved-record persistence layer
/// (`crate::resolved`); loader-facing parsing still goes through the raw
/// document shapes, and manifest-source rules are re-checked when a stored
/// record rehydrates (`ResolvedExtensionManifest::to_internal`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtensionRuntimeV2 {
    Wasm {
        module: String,
    },
    Script {
        runner: String,
        image: Option<String>,
        command: String,
        args: Vec<String>,
    },
    Mcp {
        transport: String,
        command: Option<String>,
        args: Vec<String>,
        url: Option<String>,
    },
    FirstParty {
        service: String,
    },
    System {
        service: String,
    },
}

impl ExtensionRuntimeV2 {
    pub fn kind(&self) -> RuntimeKind {
        match self {
            Self::Wasm { .. } => RuntimeKind::Wasm,
            Self::Script { .. } => RuntimeKind::Script,
            Self::Mcp { .. } => RuntimeKind::Mcp,
            Self::FirstParty { .. } => RuntimeKind::FirstParty,
            Self::System { .. } => RuntimeKind::System,
        }
    }

    /// Runtimes that an installed (non-bundled) manifest may declare.
    ///
    /// Exhaustive match — adding a new `ExtensionRuntimeV2` variant must force
    /// an explicit decision here rather than silently defaulting to `false`.
    fn installed_allows(&self) -> bool {
        match self {
            Self::Wasm { .. } | Self::Mcp { .. } | Self::Script { .. } => true,
            Self::FirstParty { .. } | Self::System { .. } => false,
        }
    }
}

/// Validated v2 manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifestV2 {
    pub schema_version: String,
    pub id: ExtensionId,
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: ManifestSource,
    /// Raw, loader-supplied trust *request*. Untrusted vocabulary.
    pub requested_trust: RequestedTrustClass,
    /// Default `TrustClass` that downstream code may use **only when no host
    /// trust policy has run yet**.
    ///
    /// Mapping:
    /// - `ThirdParty` → `UserTrusted`
    /// - `Untrusted` / `FirstPartyRequested` / `SystemRequested` → `Sandbox`
    ///
    /// FirstParty/System requests intentionally map to `Sandbox` here even
    /// for `ManifestSource::HostBundled`: this field is a safe pre-policy
    /// default, **not** the effective trust class. Effective privileged trust
    /// only ever comes from `ironclaw_trust::TrustPolicy::evaluate` on a
    /// `TrustPolicyInput`. Consumers that need effective trust **must** run
    /// the policy; they must not read this field as authoritative.
    pub descriptor_trust_default: TrustClass,
    pub runtime: ExtensionRuntimeV2,
    /// Host API contracts this extension implements. Never empty: every
    /// manifest declares at least one contract, and every capability and
    /// surface reaches the manifest through one.
    ///
    /// Contract handlers must treat manifest trust fields as untrusted
    /// declaration metadata. Runtime authority and effective trust must come
    /// from composition-owned trust policy evaluation, not from
    /// [`descriptor_trust_default`](Self::descriptor_trust_default) or the raw
    /// [`requested_trust`](Self::requested_trust) request.
    pub host_apis: Vec<HostApiRefV2>,
    pub capabilities: Vec<CapabilityDeclV2>,
    /// Surfaces projected by host API contract sections during
    /// [`Self::parse`] (channel and future section-declared
    /// kinds). Tool and auth surfaces are *not* stored here — they derive on
    /// demand from capability declarations; [`Self::capability_surfaces`]
    /// returns the complete set. Empty when the manifest was parsed without
    /// contracts or declares no surface-projecting sections.
    pub host_api_surfaces: Vec<CapabilitySurfaceDeclV2>,
    /// Declarative hook entries the extension wants installed. Carried as
    /// structurally-typed [`HookSectionEntryV2`] payloads; the composition
    /// loader projects them into typed `ironclaw_hooks::HookManifestEntry`
    /// values at the `Installed` trust tier. Empty for manifests that
    /// declare no hooks (the common case). See [`HookSectionEntryV2`] for
    /// the clean-boundary rationale.
    pub hooks: Vec<HookSectionEntryV2>,
}

/// v2 manifest parser/validator errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestV2Error {
    #[error(transparent)]
    Contract(#[from] HostApiError),
    #[error("failed to parse extension manifest: {reason}")]
    Parse { reason: String },
    #[error("invalid extension manifest: {reason}")]
    Invalid { reason: String },
    #[error("invalid host API id '{value}': {reason}")]
    InvalidHostApiId { value: String, reason: String },
    #[error("invalid manifest section path '{value}': {reason}")]
    InvalidSectionPath { value: String, reason: String },
    #[error("unknown host API id {id}")]
    UnknownHostApi { id: HostApiId },
    #[error("duplicate host API contract registration {id}")]
    DuplicateHostApiContractRegistration { id: HostApiId },
    #[error("host API {id} does not allow multiple instances")]
    DuplicateHostApi { id: HostApiId },
    #[error("manifest section {section} is referenced more than once")]
    DuplicateHostApiSection { section: ManifestSectionPath },
    #[error("host API {id} rejected section {section}: {reason}")]
    HostApiSectionRejected {
        id: HostApiId,
        section: ManifestSectionPath,
        reason: String,
    },
    #[error("manifest section {section} was referenced by host_api but does not exist")]
    MissingHostApiSection { section: ManifestSectionPath },
    #[error("manifest section {section} is operational but not referenced by host_api")]
    UnreferencedOperationalSection { section: ManifestSectionPath },
    #[error("schema_version must be '{expected}', got '{actual}'")]
    SchemaVersion {
        expected: &'static str,
        actual: String,
    },
    #[error("manifest source {manifest_source:?} is not allowed to assert trust '{requested:?}'")]
    TrustForbiddenForSource {
        manifest_source: ManifestSource,
        requested: RequestedTrustClass,
    },
    #[error(
        "manifest source {manifest_source:?} is not allowed to declare runtime kind '{kind:?}'"
    )]
    RuntimeForbiddenForSource {
        manifest_source: ManifestSource,
        kind: RuntimeKind,
    },
    #[error("extension id '{id}' uses the reserved '{prefix}' prefix, which is host-bundled only")]
    ReservedIdForInstalledSource {
        id: ExtensionId,
        prefix: &'static str,
    },
    #[error(
        "capability {capability} declares unknown host port '{port}' (not in host-defined catalog)"
    )]
    UnknownHostPort {
        capability: CapabilityId,
        port: HostPortId,
    },
    #[error("duplicate capability id {id}")]
    DuplicateCapability { id: CapabilityId },
    #[error("capability id {id} must be provider-prefixed with '{expected}.' (extension id)")]
    CapabilityIdNotPrefixed {
        id: CapabilityId,
        expected: ExtensionId,
    },
    #[error("manifest exceeds maximum size: {bytes} > {max} bytes")]
    ManifestTooLarge { bytes: usize, max: usize },
    #[error("capability {capability} declares duplicate effect {effect:?}")]
    DuplicateEffect {
        capability: CapabilityId,
        effect: EffectKind,
    },
    #[error("capability {capability} field '{field}' is invalid: {reason}")]
    InvalidSchemaRef {
        capability: CapabilityId,
        field: &'static str,
        reason: String,
    },
    #[error("capability {capability} declares duplicate required host port '{port}'")]
    DuplicateRequiredHostPort {
        capability: CapabilityId,
        port: HostPortId,
    },
    #[error("invalid wasm module ref '{value}': {reason}")]
    InvalidWasmModuleRef { value: String, reason: String },
    #[error("invalid mcp runtime: {reason}")]
    InvalidMcpRuntime { reason: String },
    #[error("manifest declares {count} hooks, exceeding the maximum of {max}")]
    TooManyHooks { count: usize, max: usize },
    #[error("hook entry {index} is invalid: {reason}")]
    InvalidHookEntry { index: usize, reason: String },
    #[error(
        "hook entry '{id}' body is {bytes} bytes, exceeding the per-entry maximum of {max} bytes"
    )]
    HookEntryTooLarge {
        id: String,
        bytes: usize,
        max: usize,
    },
    #[error("duplicate hook id '{id}' declared in manifest")]
    DuplicateHookId { id: String },
}

impl ExtensionManifestV2 {
    pub fn runtime_kind(&self) -> RuntimeKind {
        self.runtime.kind()
    }

    /// Parse a v2 manifest TOML body: validate the envelope against
    /// `host_port_catalog`, require at least one `[[host_api]]` contract,
    /// and validate/project every declared contract section through the
    /// composition-supplied registry.
    ///
    /// This is the only parse entry point. Every capability, surface, and
    /// operational section reaches the manifest through a registered host
    /// API contract — there is no contract-free manifest form.
    ///
    /// `source` is supplied by the loader/install path, never read from TOML.
    pub fn parse(
        input: &str,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        registry: &HostApiContractRegistry,
    ) -> Result<Self, ManifestV2Error> {
        // Fail closed on pathological inputs *before* invoking the TOML parser.
        // `toml::from_str` will otherwise read and allocate the full input.
        if input.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestV2Error::ManifestTooLarge {
                bytes: input.len(),
                max: MAX_MANIFEST_BYTES,
            });
        }
        let document = RawManifestDocumentV2::parse(input)?;
        let mut manifest = Self::from_raw(document.raw, source, &document.sections)?;
        manifest.project_and_extend_capabilities(
            &document.sections,
            host_port_catalog,
            registry,
        )?;
        Ok(manifest)
    }

    fn project_and_extend_capabilities(
        &mut self,
        sections: &ManifestSectionsV2,
        host_port_catalog: &HostPortCatalog,
        registry: &HostApiContractRegistry,
    ) -> Result<(), ManifestV2Error> {
        let projection = registry.project_manifest(self, sections, host_port_catalog)?;
        self.capabilities.extend(projection.capabilities);
        self.host_api_surfaces.extend(projection.surfaces);
        Ok(())
    }

    /// Derived, order-stable projection of every product-facing surface this
    /// manifest declares: one tool surface per capability (declaration
    /// order), then host-API projected surfaces (declaration order), then
    /// one auth surface per distinct product-auth provider (sorted by
    /// provider id).
    ///
    /// This is the product taxonomy of the extension. Runtime kind (`wasm`,
    /// `mcp`, `first_party`, ...) deliberately plays no part in it: how an
    /// adapter loads never decides whether the extension is a tool provider,
    /// a channel, or both.
    pub fn capability_surfaces(&self) -> Vec<CapabilitySurfaceDeclV2> {
        capability_surfaces_from_parts(&self.capabilities, &self.host_api_surfaces)
    }
}

/// Shared surface derivation for the v2 manifest and the package-level
/// [`crate::ExtensionManifest`] mirror: one tool surface per capability
/// (declaration order), then host-API projected surfaces (declaration
/// order), then one auth surface per distinct product-auth provider
/// (sorted by provider id).
pub(crate) fn capability_surfaces_from_parts(
    capabilities: &[CapabilityDeclV2],
    host_api_surfaces: &[CapabilitySurfaceDeclV2],
) -> Vec<CapabilitySurfaceDeclV2> {
    {
        let mut surfaces: Vec<CapabilitySurfaceDeclV2> = capabilities
            .iter()
            .map(|capability| CapabilitySurfaceDeclV2::Tool {
                capability: capability.id.clone(),
            })
            .collect();
        surfaces.extend(host_api_surfaces.iter().cloned());

        // One auth surface per provider. OAuth setups fold to the union of
        // their scopes and mask manual-token setups; a provider referenced
        // only through retired setups still surfaces (as Retired) so
        // discovery can see the unserviceable requirement instead of
        // silently dropping it.
        #[derive(Default)]
        struct AuthAccumulator {
            oauth_scopes: Option<BTreeSet<String>>,
            saw_manual_token: bool,
        }
        let mut providers: BTreeMap<VendorId, AuthAccumulator> = BTreeMap::new();
        for capability in capabilities {
            for credential in &capability.runtime_credentials {
                let RuntimeCredentialRequirementSource::ProductAuthAccount { provider, setup } =
                    &credential.source
                else {
                    continue;
                };
                let accumulator = providers.entry(provider.clone()).or_default();
                match setup {
                    RuntimeCredentialAccountSetup::OAuth { scopes } => {
                        accumulator
                            .oauth_scopes
                            .get_or_insert_with(BTreeSet::new)
                            .extend(scopes.iter().cloned());
                    }
                    RuntimeCredentialAccountSetup::ManualToken => {
                        accumulator.saw_manual_token = true;
                    }
                    RuntimeCredentialAccountSetup::Retired => {}
                    // Pairing-setup credentials (WebGeneratedCode channel
                    // pairing) surface through the channel connection
                    // strategy, not as an auth-provider surface.
                    RuntimeCredentialAccountSetup::Pairing => {}
                }
            }
        }
        surfaces.extend(providers.into_iter().map(|(provider, accumulator)| {
            CapabilitySurfaceDeclV2::Auth {
                provider,
                setup: match accumulator {
                    AuthAccumulator {
                        oauth_scopes: Some(scopes),
                        ..
                    } => RuntimeCredentialAccountSetup::OAuth {
                        scopes: scopes.into_iter().collect(),
                    },
                    AuthAccumulator {
                        saw_manual_token: true,
                        ..
                    } => RuntimeCredentialAccountSetup::ManualToken,
                    AuthAccumulator { .. } => RuntimeCredentialAccountSetup::Retired,
                },
            }
        }));
        surfaces
    }
}

impl ExtensionManifestV2 {
    /// Construct a manifest from an already-deserialized raw representation.
    fn from_raw(
        raw: RawManifestV2,
        source: ManifestSource,
        sections: &ManifestSectionsV2,
    ) -> Result<Self, ManifestV2Error> {
        if raw.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(ManifestV2Error::SchemaVersion {
                expected: MANIFEST_SCHEMA_VERSION,
                actual: raw.schema_version,
            });
        }

        // Cheap empty-string checks first — surface them before the more
        // structured id / runtime / capability errors so hand-edited manifests
        // get the most actionable message.
        if raw.name.trim().is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: "name must not be empty".to_string(),
            });
        }
        if raw.version.trim().is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: "version must not be empty".to_string(),
            });
        }
        if raw.description.trim().is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: "description must not be empty".to_string(),
            });
        }
        if !raw.capabilities.is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: "top-level [[capabilities]] is not supported; declare capabilities \
                         under an ironclaw.capability_provider/v1 host_api section"
                    .to_string(),
            });
        }
        if raw.host_api.is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: "at least one host_api contract is required".to_string(),
            });
        }

        let id = ExtensionId::new(raw.id)?;
        if !source.allows_first_party() && id.as_str().starts_with(RESERVED_HOST_BUNDLED_ID_PREFIX)
        {
            return Err(ManifestV2Error::ReservedIdForInstalledSource {
                id,
                prefix: RESERVED_HOST_BUNDLED_ID_PREFIX,
            });
        }

        let requested_trust = raw.trust;
        if !source.allows_first_party()
            && matches!(
                requested_trust,
                RequestedTrustClass::FirstPartyRequested | RequestedTrustClass::SystemRequested
            )
        {
            return Err(ManifestV2Error::TrustForbiddenForSource {
                manifest_source: source,
                requested: requested_trust,
            });
        }
        let descriptor_trust_default = requested_trust_to_descriptor_trust(requested_trust);

        let runtime = raw.runtime.into_runtime()?;
        if !source.allows_first_party() && !runtime.installed_allows() {
            return Err(ManifestV2Error::RuntimeForbiddenForSource {
                manifest_source: source,
                kind: runtime.kind(),
            });
        }

        let host_apis = validate_host_api_refs(raw.host_api, sections)?;

        let hooks = validate_hook_entries(raw.hooks)?;

        // `capabilities` and `host_api_surfaces` start empty and are filled
        // exclusively by host API contract projection in
        // [`Self::project_and_extend_capabilities`].
        Ok(Self {
            schema_version: raw.schema_version,
            id,
            name: raw.name,
            version: raw.version,
            description: raw.description,
            source,
            requested_trust,
            descriptor_trust_default,
            runtime,
            host_apis,
            capabilities: Vec::new(),
            host_api_surfaces: Vec::new(),
            hooks,
        })
    }
}

/// Validate the raw `[[hooks]]` entries: enforce the count cap, that each
/// entry is a table carrying a non-empty `id`, that ids are unique within the
/// manifest, and that each entry's serialized body is within the per-entry
/// size bound. Returns the structurally-typed [`HookSectionEntryV2`] payloads
/// for the composition loader to project into typed hook entries.
fn validate_hook_entries(
    raw_hooks: Vec<toml::Value>,
) -> Result<Vec<HookSectionEntryV2>, ManifestV2Error> {
    if raw_hooks.len() > MAX_MANIFEST_HOOKS {
        return Err(ManifestV2Error::TooManyHooks {
            count: raw_hooks.len(),
            max: MAX_MANIFEST_HOOKS,
        });
    }
    let mut seen_ids = HashSet::new();
    let mut entries = Vec::with_capacity(raw_hooks.len());
    for (index, raw) in raw_hooks.into_iter().enumerate() {
        let table = raw
            .as_table()
            .ok_or_else(|| ManifestV2Error::InvalidHookEntry {
                index,
                reason: "hook entry must be a TOML table".to_string(),
            })?;
        let local_id = table
            .get("id")
            .and_then(toml::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ManifestV2Error::InvalidHookEntry {
                index,
                reason: "hook entry must declare a string `id`".to_string(),
            })?;
        if local_id.trim().is_empty() {
            return Err(ManifestV2Error::InvalidHookEntry {
                index,
                reason: "hook entry `id` must not be empty".to_string(),
            });
        }
        // Size-bound the per-entry payload before retaining it. Serialize
        // back to canonical TOML — this is both the size measurement and the
        // exact payload the loader re-parses. Structural guard, not semantic.
        let raw_toml =
            toml::to_string(&raw).map_err(|error| ManifestV2Error::InvalidHookEntry {
                index,
                reason: format!("hook entry is not serializable TOML: {error}"),
            })?;
        if raw_toml.len() > MAX_HOOK_ENTRY_BYTES {
            return Err(ManifestV2Error::HookEntryTooLarge {
                id: local_id,
                bytes: raw_toml.len(),
                max: MAX_HOOK_ENTRY_BYTES,
            });
        }
        if !seen_ids.insert(local_id.clone()) {
            return Err(ManifestV2Error::DuplicateHookId { id: local_id });
        }
        entries.push(HookSectionEntryV2 { local_id, raw_toml });
    }
    Ok(entries)
}

impl CapabilityDeclV2 {
    pub(crate) fn from_raw(
        raw: RawCapabilityV2,
        extension_id: &ExtensionId,
        host_port_catalog: &HostPortCatalog,
    ) -> Result<Self, ManifestV2Error> {
        let id = CapabilityId::new(raw.id)?;
        // Provider-prefix check without an intermediate `format!` allocation:
        // capability id must be `<extension_id>.<...>` (the dot is required so
        // `foo.bar` cannot squat `foo`'s namespace via `foobar.baz`).
        let prefixed = id
            .as_str()
            .strip_prefix(extension_id.as_str())
            .is_some_and(|rest| rest.starts_with('.'));
        if !prefixed {
            return Err(ManifestV2Error::CapabilityIdNotPrefixed {
                id,
                expected: extension_id.clone(),
            });
        }

        if raw.description.trim().is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: format!("capability {id} description must not be empty"),
            });
        }

        // Reject duplicate effects — declaring the same `EffectKind` twice in
        // one capability is always a manifest bug, never load-bearing, and
        // letting it through would defeat consistency with the dedup applied
        // to `implements` and `required_host_ports`.
        let mut effects_seen: HashSet<EffectKind> = HashSet::new();
        for effect in &raw.effects {
            if !effects_seen.insert(*effect) {
                return Err(ManifestV2Error::DuplicateEffect {
                    capability: id,
                    effect: *effect,
                });
            }
        }

        let input_schema_ref =
            CapabilityProfileSchemaRef::new(raw.input_schema_ref).map_err(|err| {
                ManifestV2Error::InvalidSchemaRef {
                    capability: id.clone(),
                    field: "input_schema_ref",
                    reason: err.to_string(),
                }
            })?;
        let output_schema_ref = raw
            .output_schema_ref
            .map(|value| {
                CapabilityProfileSchemaRef::new(value).map_err(|err| {
                    ManifestV2Error::InvalidSchemaRef {
                        capability: id.clone(),
                        field: "output_schema_ref",
                        reason: err.to_string(),
                    }
                })
            })
            .transpose()?;
        let prompt_doc_ref = raw
            .prompt_doc_ref
            .map(|value| {
                CapabilityProfileSchemaRef::new(value).map_err(|err| {
                    ManifestV2Error::InvalidSchemaRef {
                        capability: id.clone(),
                        field: "prompt_doc_ref",
                        reason: err.to_string(),
                    }
                })
            })
            .transpose()?;

        let mut required_host_ports_seen = BTreeSet::new();
        let mut required_host_ports = Vec::with_capacity(raw.required_host_ports.len());
        for port in raw.required_host_ports {
            let port = HostPortId::new(port)?;
            if !required_host_ports_seen.insert(port.clone()) {
                return Err(ManifestV2Error::DuplicateRequiredHostPort {
                    capability: id.clone(),
                    port,
                });
            }
            if !host_port_catalog.contains(&port) {
                return Err(ManifestV2Error::UnknownHostPort {
                    capability: id.clone(),
                    port,
                });
            }
            required_host_ports.push(port);
        }

        if !raw.runtime_credentials.is_empty() && !raw.effects.contains(&EffectKind::UseSecret) {
            return Err(ManifestV2Error::Invalid {
                reason: format!(
                    "capability {id} declares runtime_credentials without use_secret effect"
                ),
            });
        }
        let mut credential_handles_seen = BTreeSet::new();
        let mut runtime_credentials = Vec::with_capacity(raw.runtime_credentials.len());
        for raw_credential in raw.runtime_credentials {
            let handle = SecretHandle::new(raw_credential.handle)?;
            if !credential_handles_seen.insert(handle.clone()) {
                return Err(ManifestV2Error::Invalid {
                    reason: format!(
                        "capability {id} declares duplicate runtime credential handle {handle}"
                    ),
                });
            }
            raw_credential
                .target
                .validate_declaration()
                .map_err(|error| ManifestV2Error::Invalid {
                    reason: format!(
                        "capability {id} declares invalid runtime credential target: {error}"
                    ),
                })?;
            raw_credential
                .audience
                .validate_declaration()
                .map_err(|error| ManifestV2Error::Invalid {
                    reason: format!(
                        "capability {id} declares invalid runtime credential audience: {error}"
                    ),
                })?;
            validate_runtime_credential_audience(&id, &raw_credential.audience)?;
            let provider_scopes = validate_runtime_credential_provider_scopes(
                &id,
                &raw_credential.source,
                raw_credential.provider_scopes,
            )?;
            runtime_credentials.push(RuntimeCredentialRequirement {
                handle,
                source: raw_credential.source,
                provider_scopes,
                audience: raw_credential.audience,
                target: raw_credential.target,
                required: raw_credential.required,
            });
        }

        // Declared network egress requires the `network` effect — same
        // effect/declaration symmetry the `runtime_credentials`/`use_secret`
        // check above enforces. This lets a keyless-but-networked tool declare
        // its egress allowlist without inventing a fake credential.
        if !raw.network_targets.is_empty() && !raw.effects.contains(&EffectKind::Network) {
            return Err(ManifestV2Error::Invalid {
                reason: format!("capability {id} declares network_targets without network effect"),
            });
        }
        let mut network_targets_seen = HashSet::new();
        let mut network_targets = Vec::with_capacity(raw.network_targets.len());
        for target in raw.network_targets {
            target
                .validate_declaration()
                .map_err(|error| ManifestV2Error::Invalid {
                    reason: format!("capability {id} declares invalid network target: {error}"),
                })?;
            if !network_targets_seen.insert(target.clone()) {
                return Err(ManifestV2Error::Invalid {
                    reason: format!(
                        "capability {id} declares duplicate network target {}",
                        target.host_pattern
                    ),
                });
            }
            network_targets.push(target);
        }

        Ok(Self {
            id,
            description: raw.description,
            effects: raw.effects,
            default_permission: raw.default_permission,
            visibility: raw.visibility,
            input_schema_ref,
            output_schema_ref,
            prompt_doc_ref,
            required_host_ports,
            runtime_credentials,
            network_targets,
            max_egress_bytes: raw.max_egress_bytes,
            resource_profile: raw.resource_profile,
            origin_gate_matrix: raw.origin_gate_matrix,
        })
    }
}

fn validate_runtime_credential_audience(
    id: &CapabilityId,
    audience: &NetworkTargetPattern,
) -> Result<(), ManifestV2Error> {
    if audience.scheme != Some(NetworkScheme::Https) {
        return Err(ManifestV2Error::Invalid {
            reason: format!(
                "capability {id} declares runtime credential audience without https scheme"
            ),
        });
    }
    Ok(())
}

fn validate_host_api_refs(
    raw_refs: Vec<RawHostApiRefV2>,
    sections: &ManifestSectionsV2,
) -> Result<Vec<HostApiRefV2>, ManifestV2Error> {
    let mut seen_sections: BTreeSet<ManifestSectionPath> = BTreeSet::new();
    let mut refs = Vec::with_capacity(raw_refs.len());
    for raw_ref in raw_refs {
        let host_api = HostApiRefV2 {
            id: HostApiId::new(raw_ref.id)?,
            section: ManifestSectionPath::new(raw_ref.section)?,
        };
        if seen_sections.iter().any(|seen| {
            seen == &host_api.section
                || seen.is_prefix_of(&host_api.section)
                || host_api.section.is_prefix_of(seen)
        }) {
            return Err(ManifestV2Error::DuplicateHostApiSection {
                section: host_api.section,
            });
        }
        seen_sections.insert(host_api.section.clone());
        sections.get(&host_api.section)?;
        refs.push(host_api);
    }
    Ok(refs)
}

fn validate_host_api_id(value: &str) -> Result<(), ManifestV2Error> {
    let raise = |reason: &str| ManifestV2Error::InvalidHostApiId {
        value: value.to_string(),
        reason: reason.to_string(),
    };
    if value.is_empty() {
        return Err(raise("must not be empty"));
    }
    if !value.contains("/v") {
        return Err(raise("must include an explicit /vN contract version"));
    }
    if value.starts_with('.') || value.ends_with('.') || value.contains("..") {
        return Err(raise("dotted name segments must not be empty"));
    }
    for ch in value.chars() {
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-' | '/'))
        {
            return Err(raise(
                "only lowercase ASCII letters, digits, '.', '_', '-', and '/' are allowed",
            ));
        }
    }
    Ok(())
}

fn validate_section_path(value: &str) -> Result<(), ManifestV2Error> {
    let raise = |reason: &str| ManifestV2Error::InvalidSectionPath {
        value: value.to_string(),
        reason: reason.to_string(),
    };
    if value.is_empty() {
        return Err(raise("must not be empty"));
    }
    if value.starts_with('.') || value.ends_with('.') || value.contains("..") {
        return Err(raise("section path segments must not be empty"));
    }
    for segment in value.split('.') {
        if segment.is_empty() {
            return Err(raise("section path segments must not be empty"));
        }
        for ch in segment.chars() {
            if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
                return Err(raise(
                    "only ASCII letters, digits, '_', '-', and '.' separators are allowed",
                ));
            }
        }
    }
    Ok(())
}

fn validate_wasm_module_ref(value: &str) -> Result<(), ManifestV2Error> {
    let raise = |reason: &str| ManifestV2Error::InvalidWasmModuleRef {
        value: value.to_string(),
        reason: reason.to_string(),
    };
    if value.is_empty() {
        return Err(raise("must not be empty"));
    }
    if value.chars().any(|ch| ch == ' ' || ch.is_control()) {
        return Err(raise("NUL/control characters and spaces are not allowed"));
    }
    if value.contains("://") {
        return Err(raise("URLs are not extension asset paths"));
    }
    if value.starts_with('/') {
        return Err(raise("must be relative"));
    }
    if value.contains('\\') {
        return Err(raise("host path separators are not allowed"));
    }
    let bytes = value.as_bytes();
    let looks_windows = (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || (bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'));
    if looks_windows {
        return Err(raise("host path separators are not allowed"));
    }
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(raise("empty or dot path segments are not allowed"));
        }
    }
    Ok(())
}

fn validate_mcp_runtime_shape(
    transport: &str,
    command: Option<&str>,
    url: Option<&str>,
) -> Result<(), ManifestV2Error> {
    if transport.trim().is_empty() {
        return Err(ManifestV2Error::InvalidMcpRuntime {
            reason: "transport must not be empty".to_string(),
        });
    }
    if let Some(command) = command
        && command.trim().is_empty()
    {
        return Err(ManifestV2Error::InvalidMcpRuntime {
            reason: "command must not be empty".to_string(),
        });
    }
    if let Some(url) = url
        && url.trim().is_empty()
    {
        return Err(ManifestV2Error::InvalidMcpRuntime {
            reason: "url must not be empty".to_string(),
        });
    }
    match transport {
        "stdio" => {
            if url.is_some() {
                return Err(ManifestV2Error::InvalidMcpRuntime {
                    reason: "stdio transport must not specify url".to_string(),
                });
            }
            if command.is_none() {
                return Err(ManifestV2Error::InvalidMcpRuntime {
                    reason: "stdio transport requires command".to_string(),
                });
            }
        }
        "http" | "sse" => {
            if command.is_some() {
                return Err(ManifestV2Error::InvalidMcpRuntime {
                    reason: format!("{transport} transport must not specify command"),
                });
            }
            let Some(url) = url else {
                return Err(ManifestV2Error::InvalidMcpRuntime {
                    reason: format!("{transport} transport requires url"),
                });
            };
            validate_mcp_http_url(transport, url)?;
        }
        other => {
            return Err(ManifestV2Error::InvalidMcpRuntime {
                reason: format!(
                    "transport '{other}' is not supported; expected stdio, http, or sse"
                ),
            });
        }
    }
    Ok(())
}

fn validate_mcp_http_url(transport: &str, value: &str) -> Result<(), ManifestV2Error> {
    let parsed = url::Url::parse(value).map_err(|_| ManifestV2Error::InvalidMcpRuntime {
        reason: format!("{transport} transport url must be an absolute http(s) URL"),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ManifestV2Error::InvalidMcpRuntime {
            reason: format!("{transport} transport url must use http or https"),
        });
    }
    Ok(())
}

pub(crate) fn requested_trust_to_descriptor_trust(requested: RequestedTrustClass) -> TrustClass {
    match requested {
        RequestedTrustClass::ThirdParty => TrustClass::UserTrusted,
        RequestedTrustClass::Untrusted
        | RequestedTrustClass::FirstPartyRequested
        | RequestedTrustClass::SystemRequested => TrustClass::Sandbox,
    }
}

// ---- Raw deserialization shapes --------------------------------------------

struct RawManifestDocumentV2 {
    raw: RawManifestV2,
    sections: ManifestSectionsV2,
}

impl RawManifestDocumentV2 {
    fn parse(input: &str) -> Result<Self, ManifestV2Error> {
        let value: toml::Value = toml::from_str(input).map_err(|error| ManifestV2Error::Parse {
            reason: error.to_string(),
        })?;
        let table = value.as_table().ok_or_else(|| ManifestV2Error::Parse {
            reason: "manifest root must be a TOML table".to_string(),
        })?;
        let mut envelope = toml::value::Table::new();
        for (key, value) in table {
            if is_envelope_key(key) {
                envelope.insert(key.clone(), value.clone());
            } else if !value.is_table() {
                return Err(ManifestV2Error::Parse {
                    reason: format!("unknown top-level field {key:?}"),
                });
            }
        }
        let raw: RawManifestV2 =
            toml::Value::Table(envelope)
                .try_into()
                .map_err(|error: toml::de::Error| ManifestV2Error::Parse {
                    reason: error.to_string(),
                })?;
        Ok(Self {
            raw,
            sections: ManifestSectionsV2 {
                table: table.clone(),
            },
        })
    }
}

#[derive(Debug, Clone)]
struct ManifestSectionsV2 {
    table: toml::value::Table,
}

impl ManifestSectionsV2 {
    fn get(&self, path: &ManifestSectionPath) -> Result<&toml::Value, ManifestV2Error> {
        let mut current = &self.table;
        let mut segments = path.segments().peekable();
        while let Some(segment) = segments.next() {
            let value =
                current
                    .get(segment)
                    .ok_or_else(|| ManifestV2Error::MissingHostApiSection {
                        section: path.clone(),
                    })?;
            if segments.peek().is_none() {
                if value.is_table() {
                    return Ok(value);
                }
                return Err(ManifestV2Error::MissingHostApiSection {
                    section: path.clone(),
                });
            }
            current = value
                .as_table()
                .ok_or_else(|| ManifestV2Error::MissingHostApiSection {
                    section: path.clone(),
                })?;
        }
        Err(ManifestV2Error::MissingHostApiSection {
            section: path.clone(),
        })
    }

    fn reject_unreferenced_operational_sections(
        &self,
        host_apis: &[HostApiRefV2],
    ) -> Result<(), ManifestV2Error> {
        if host_apis.is_empty() {
            return Ok(());
        }
        let referenced: BTreeSet<_> = host_apis
            .iter()
            .map(|entry| entry.section.clone())
            .collect();
        for path in self.operational_table_paths()? {
            let used = referenced.iter().any(|section| {
                section == &path || path.is_prefix_of(section) || section.is_prefix_of(&path)
            });
            if !used {
                return Err(ManifestV2Error::UnreferencedOperationalSection { section: path });
            }
        }
        Ok(())
    }

    fn operational_table_paths(&self) -> Result<Vec<ManifestSectionPath>, ManifestV2Error> {
        let mut paths = Vec::new();
        collect_operational_table_paths(&self.table, None, &mut paths)?;
        Ok(paths)
    }
}

fn collect_operational_table_paths(
    table: &toml::value::Table,
    prefix: Option<&str>,
    paths: &mut Vec<ManifestSectionPath>,
) -> Result<(), ManifestV2Error> {
    for (key, value) in table {
        let path = match prefix {
            Some(prefix) => format!("{prefix}.{key}"),
            None => key.clone(),
        };
        let root = path.split('.').next().unwrap_or_default();
        if is_envelope_key(root) || matches!(root, "metadata" | "x") {
            continue;
        }
        if let Some(child) = value.as_table() {
            let section_path = ManifestSectionPath::new(path.clone())?;
            paths.push(section_path);
            collect_operational_table_paths(child, Some(&path), paths)?;
        }
    }
    Ok(())
}

fn is_envelope_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "id"
            | "name"
            | "version"
            | "description"
            | "trust"
            | "runtime"
            | "capabilities"
            | "host_api"
            | "hooks"
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifestV2 {
    schema_version: String,
    id: String,
    name: String,
    version: String,
    description: String,
    #[serde(
        default = "default_requested_trust",
        deserialize_with = "deserialize_requested_trust"
    )]
    trust: RequestedTrustClass,
    runtime: RawRuntimeV2,
    #[serde(default)]
    host_api: Vec<RawHostApiRefV2>,
    /// Legacy top-level `[[capabilities]]` payload. Kept only so its
    /// presence can be rejected with an actionable error; entries are never
    /// parsed. Capabilities are declared under
    /// `ironclaw.capability_provider/v1` host_api sections.
    #[serde(default)]
    capabilities: Vec<toml::Value>,
    /// Raw `[[hooks]]` entries. Each is an arbitrary TOML table validated
    /// structurally here (table shape, non-empty `id`, size bound) and
    /// projected into a typed hook entry by the composition loader. Kept as
    /// raw `toml::Value` so this crate never imports the hook vocabulary.
    #[serde(default)]
    hooks: Vec<toml::Value>,
}

pub(crate) fn default_requested_trust() -> RequestedTrustClass {
    RequestedTrustClass::Untrusted
}

pub(crate) fn deserialize_requested_trust<'de, D>(
    deserializer: D,
) -> Result<RequestedTrustClass, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    match value.as_str() {
        "untrusted" => Ok(RequestedTrustClass::Untrusted),
        "third_party" => Ok(RequestedTrustClass::ThirdParty),
        "first_party_requested" => Ok(RequestedTrustClass::FirstPartyRequested),
        "system_requested" => Ok(RequestedTrustClass::SystemRequested),
        other => Err(serde::de::Error::custom(format!(
            "unsupported trust value {other:?}; expected one of untrusted, third_party, first_party_requested, system_requested"
        ))),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostApiRefV2 {
    id: String,
    section: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawRuntimeV2 {
    Wasm {
        module: String,
    },
    Script {
        runner: String,
        #[serde(default)]
        image: Option<String>,
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Mcp {
        transport: String,
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        url: Option<String>,
    },
    FirstParty {
        service: String,
    },
    System {
        service: String,
    },
}

impl RawRuntimeV2 {
    fn into_runtime(self) -> Result<ExtensionRuntimeV2, ManifestV2Error> {
        match self {
            Self::Wasm { module } => {
                validate_wasm_module_ref(&module)?;
                Ok(ExtensionRuntimeV2::Wasm { module })
            }
            Self::Script {
                runner,
                image,
                command,
                args,
            } => {
                if runner.trim().is_empty() {
                    return Err(ManifestV2Error::Invalid {
                        reason: "script runner must not be empty".to_string(),
                    });
                }
                if command.trim().is_empty() {
                    return Err(ManifestV2Error::Invalid {
                        reason: "script command must not be empty".to_string(),
                    });
                }
                if runner == "docker" {
                    let image_str = image.as_deref().unwrap_or_default();
                    if image_str.trim().is_empty() {
                        return Err(ManifestV2Error::Invalid {
                            reason: "script image is required for docker runner".to_string(),
                        });
                    }
                }
                Ok(ExtensionRuntimeV2::Script {
                    runner,
                    image,
                    command,
                    args,
                })
            }
            Self::Mcp {
                transport,
                command,
                args,
                url,
            } => {
                validate_mcp_runtime_shape(&transport, command.as_deref(), url.as_deref())?;
                Ok(ExtensionRuntimeV2::Mcp {
                    transport,
                    command,
                    args,
                    url,
                })
            }
            Self::FirstParty { service } => {
                if service.trim().is_empty() {
                    return Err(ManifestV2Error::Invalid {
                        reason: "first-party service must not be empty".to_string(),
                    });
                }
                Ok(ExtensionRuntimeV2::FirstParty { service })
            }
            Self::System { service } => {
                if service.trim().is_empty() {
                    return Err(ManifestV2Error::Invalid {
                        reason: "system service must not be empty".to_string(),
                    });
                }
                Ok(ExtensionRuntimeV2::System { service })
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCapabilityV2 {
    pub(crate) id: String,
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) effects: Vec<EffectKind>,
    pub(crate) default_permission: PermissionMode,
    pub(crate) visibility: CapabilityVisibility,
    pub(crate) input_schema_ref: String,
    #[serde(default)]
    pub(crate) output_schema_ref: Option<String>,
    #[serde(default)]
    pub(crate) prompt_doc_ref: Option<String>,
    #[serde(default)]
    pub(crate) required_host_ports: Vec<String>,
    #[serde(default)]
    pub(crate) runtime_credentials: Vec<RawRuntimeCredentialV2>,
    #[serde(default)]
    pub(crate) network_targets: Vec<NetworkTargetPattern>,
    /// Optional per-capability egress cap (bytes). `#[serde(default)]` so
    /// existing manifests without the key parse to `None`.
    #[serde(default)]
    pub(crate) max_egress_bytes: Option<u64>,
    #[serde(default)]
    pub(crate) resource_profile: Option<ResourceProfile>,
    /// Per-origin gate matrix (§5.2.1). `#[serde(default)]` so existing
    /// manifests without the key parse to `None`. `OriginGateMatrix`
    /// deserializes directly from TOML (snake_case fields, each defaulting to
    /// `Forbidden` when omitted), so no raw mirror is needed.
    #[serde(default)]
    pub(crate) origin_gate_matrix: Option<OriginGateMatrix>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRuntimeCredentialV2 {
    pub(crate) handle: String,
    #[serde(default)]
    pub(crate) source: RuntimeCredentialRequirementSource,
    #[serde(default)]
    pub(crate) provider_scopes: Vec<String>,
    pub(crate) audience: NetworkTargetPattern,
    pub(crate) target: RuntimeCredentialTarget,
    #[serde(default = "default_runtime_credential_required")]
    pub(crate) required: bool,
}

fn default_runtime_credential_required() -> bool {
    true
}

fn validate_runtime_credential_provider_scopes(
    capability_id: &CapabilityId,
    source: &RuntimeCredentialRequirementSource,
    raw_scopes: Vec<String>,
) -> Result<Vec<String>, ManifestV2Error> {
    if !raw_scopes.is_empty()
        && !matches!(
            source,
            RuntimeCredentialRequirementSource::ProductAuthAccount { .. }
        )
    {
        return Err(ManifestV2Error::Invalid {
            reason: format!(
                "capability {capability_id} declares runtime credential provider scopes for a non product-auth credential source"
            ),
        });
    }
    let mut seen = BTreeSet::new();
    let mut scopes = Vec::with_capacity(raw_scopes.len());
    for raw_scope in raw_scopes {
        if raw_scope.trim() != raw_scope || raw_scope.is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: format!(
                    "capability {capability_id} declares invalid runtime credential provider scope"
                ),
            });
        }
        if !seen.insert(raw_scope.clone()) {
            return Err(ManifestV2Error::Invalid {
                reason: format!(
                    "capability {capability_id} declares duplicate runtime credential provider scope {raw_scope}"
                ),
            });
        }
        scopes.push(raw_scope);
    }
    Ok(scopes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability_id() -> CapabilityId {
        CapabilityId::new("acme.echo").unwrap()
    }

    fn product_auth_source() -> RuntimeCredentialRequirementSource {
        RuntimeCredentialRequirementSource::ProductAuthAccount {
            provider: ironclaw_host_api::VendorId::new("google").unwrap(),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
        }
    }

    #[test]
    fn validate_runtime_credential_provider_scopes_rejects_empty_scope() {
        let err = validate_runtime_credential_provider_scopes(
            &capability_id(),
            &product_auth_source(),
            vec!["".to_string()],
        )
        .unwrap_err();

        assert!(matches!(err, ManifestV2Error::Invalid { .. }), "{err:?}");
    }

    #[test]
    fn validate_runtime_credential_provider_scopes_rejects_whitespace_padded_scope() {
        let err = validate_runtime_credential_provider_scopes(
            &capability_id(),
            &product_auth_source(),
            vec![" https://www.googleapis.com/auth/drive".to_string()],
        )
        .unwrap_err();

        assert!(matches!(err, ManifestV2Error::Invalid { .. }), "{err:?}");
    }

    #[test]
    fn validate_runtime_credential_provider_scopes_rejects_duplicate_scope() {
        let err = validate_runtime_credential_provider_scopes(
            &capability_id(),
            &product_auth_source(),
            vec![
                "https://www.googleapis.com/auth/drive".to_string(),
                "https://www.googleapis.com/auth/drive".to_string(),
            ],
        )
        .unwrap_err();

        assert!(matches!(err, ManifestV2Error::Invalid { .. }), "{err:?}");
    }
}
