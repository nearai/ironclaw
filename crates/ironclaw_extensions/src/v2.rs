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
//! - installed manifests must use `wasm` / `mcp` / `script` runtimes only;
//! - every capability declares `visibility`, relative
//!   [`CapabilityProfileSchemaRef`] input/output schema refs, optional
//!   `prompt_doc_ref` (required when visibility is `model`), and the set of
//!   host ports it needs;
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
    CapabilityId, CapabilityProfileId, CapabilityProfileSchemaRef, EffectKind, ExtensionId,
    HostApiError, HostPortCatalog, HostPortId, PermissionMode, RequestedTrustClass,
    ResourceProfile, RuntimeKind, TrustClass,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
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

/// Host API contract validator registered by composition.
///
/// `ironclaw_extensions` owns the generic envelope and section dispatch. Domain
/// crates own section patterns, cardinality, typed schema validation, and
/// projection into their read models.
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
    ) -> Result<(), String>;
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

    fn validate_manifest(
        &self,
        manifest: &ExtensionManifestV2,
        sections: &ManifestSectionsV2,
    ) -> Result<(), ManifestV2Error> {
        let mut counts: BTreeMap<&HostApiId, usize> = BTreeMap::new();
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
            contract
                .validate_section(host_api, section)
                .map_err(|reason| ManifestV2Error::HostApiSectionRejected {
                    id: host_api.id.clone(),
                    section: host_api.section.clone(),
                    reason,
                })?;
        }
        sections.reject_unreferenced_operational_sections(&manifest.host_apis)?;
        Ok(())
    }
}

impl Default for HostApiContractRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Validated v2 capability declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDeclV2 {
    pub id: CapabilityId,
    pub implements: Vec<CapabilityProfileId>,
    pub description: String,
    pub effects: Vec<EffectKind>,
    pub default_permission: PermissionMode,
    pub visibility: CapabilityVisibility,
    pub input_schema_ref: CapabilityProfileSchemaRef,
    pub output_schema_ref: CapabilityProfileSchemaRef,
    pub prompt_doc_ref: Option<CapabilityProfileSchemaRef>,
    pub required_host_ports: Vec<HostPortId>,
    pub resource_profile: Option<ResourceProfile>,
}

/// v2 runtime declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Host API contracts this extension implements. Empty only for legacy v2
    /// capability-only manifests during cutover.
    ///
    /// Contract handlers must treat manifest trust fields as untrusted
    /// declaration metadata. Runtime authority and effective trust must come
    /// from composition-owned trust policy evaluation, not from
    /// [`descriptor_trust_default`](Self::descriptor_trust_default) or the raw
    /// [`requested_trust`](Self::requested_trust) request.
    pub host_apis: Vec<HostApiRefV2>,
    pub capabilities: Vec<CapabilityDeclV2>,
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
    #[error("capability {capability} is model-visible but has no prompt_doc_ref")]
    MissingPromptDocRef { capability: CapabilityId },
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
    #[error("capability {capability} implements profile '{profile}' more than once")]
    DuplicateImplementedProfile {
        capability: CapabilityId,
        profile: CapabilityProfileId,
    },
    #[error("invalid wasm module ref '{value}': {reason}")]
    InvalidWasmModuleRef { value: String, reason: String },
    #[error("invalid mcp runtime: {reason}")]
    InvalidMcpRuntime { reason: String },
}

impl ExtensionManifestV2 {
    /// Parse a v2 manifest TOML body and validate it against `host_port_catalog`.
    ///
    /// `source` is supplied by the loader/install path, never read from TOML.
    pub fn parse(
        input: &str,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
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
        if !document.raw.host_api.is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: "host_api manifests require parse_with_host_api_contracts".to_string(),
            });
        }
        if let Some(key) = document.sections.first_non_envelope_top_level_key() {
            return Err(ManifestV2Error::Parse {
                reason: format!("unknown top-level field {key:?}"),
            });
        }
        Self::from_raw(document.raw, source, host_port_catalog, &document.sections)
    }

    /// Parse a v2 manifest and validate every `[[host_api]]` contract through
    /// the composition-supplied registry.
    pub fn parse_with_host_api_contracts(
        input: &str,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        registry: &HostApiContractRegistry,
    ) -> Result<Self, ManifestV2Error> {
        if input.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestV2Error::ManifestTooLarge {
                bytes: input.len(),
                max: MAX_MANIFEST_BYTES,
            });
        }
        let document = RawManifestDocumentV2::parse(input)?;
        let manifest = Self::from_raw(document.raw, source, host_port_catalog, &document.sections)?;
        registry.validate_manifest(&manifest, &document.sections)?;
        Ok(manifest)
    }

    /// Construct a manifest from an already-deserialized raw representation.
    fn from_raw(
        raw: RawManifestV2,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
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
        if raw.capabilities.is_empty() && raw.host_api.is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason: "at least one host_api contract or legacy capability is required"
                    .to_string(),
            });
        }
        if !raw.host_api.is_empty() && !raw.capabilities.is_empty() {
            return Err(ManifestV2Error::Invalid {
                reason:
                    "top-level capabilities are not allowed when host_api contracts are declared"
                        .to_string(),
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

        let mut seen_capabilities = BTreeSet::new();
        let mut capabilities = Vec::with_capacity(raw.capabilities.len());
        for raw_cap in raw.capabilities {
            let cap = CapabilityDeclV2::from_raw(raw_cap, &id, host_port_catalog)?;
            if !seen_capabilities.insert(cap.id.clone()) {
                return Err(ManifestV2Error::DuplicateCapability { id: cap.id });
            }
            capabilities.push(cap);
        }

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
            capabilities,
        })
    }
}

impl CapabilityDeclV2 {
    fn from_raw(
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

        let mut implements_seen = BTreeSet::new();
        let mut implements = Vec::with_capacity(raw.implements.len());
        for profile in raw.implements {
            let profile = CapabilityProfileId::new(profile)?;
            if !implements_seen.insert(profile.clone()) {
                return Err(ManifestV2Error::DuplicateImplementedProfile {
                    capability: id,
                    profile,
                });
            }
            implements.push(profile);
        }

        let input_schema_ref =
            CapabilityProfileSchemaRef::new(raw.input_schema_ref).map_err(|err| {
                ManifestV2Error::InvalidSchemaRef {
                    capability: id.clone(),
                    field: "input_schema_ref",
                    reason: err.to_string(),
                }
            })?;
        let output_schema_ref =
            CapabilityProfileSchemaRef::new(raw.output_schema_ref).map_err(|err| {
                ManifestV2Error::InvalidSchemaRef {
                    capability: id.clone(),
                    field: "output_schema_ref",
                    reason: err.to_string(),
                }
            })?;
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

        if matches!(raw.visibility, CapabilityVisibility::Model) && prompt_doc_ref.is_none() {
            return Err(ManifestV2Error::MissingPromptDocRef { capability: id });
        }

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

        Ok(Self {
            id,
            implements,
            description: raw.description,
            effects: raw.effects,
            default_permission: raw.default_permission,
            visibility: raw.visibility,
            input_schema_ref,
            output_schema_ref,
            prompt_doc_ref,
            required_host_ports,
            resource_profile: raw.resource_profile,
        })
    }
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

fn requested_trust_to_descriptor_trust(requested: RequestedTrustClass) -> TrustClass {
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
    fn first_non_envelope_top_level_key(&self) -> Option<&str> {
        self.table
            .keys()
            .find(|key| !is_envelope_key(key))
            .map(String::as_str)
    }

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
    #[serde(default)]
    capabilities: Vec<RawCapabilityV2>,
}

fn default_requested_trust() -> RequestedTrustClass {
    RequestedTrustClass::Untrusted
}

fn deserialize_requested_trust<'de, D>(deserializer: D) -> Result<RequestedTrustClass, D::Error>
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
struct RawCapabilityV2 {
    id: String,
    #[serde(default)]
    implements: Vec<String>,
    description: String,
    #[serde(default)]
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
    visibility: CapabilityVisibility,
    input_schema_ref: String,
    output_schema_ref: String,
    #[serde(default)]
    prompt_doc_ref: Option<String>,
    #[serde(default)]
    required_host_ports: Vec<String>,
    #[serde(default)]
    resource_profile: Option<ResourceProfile>,
}
