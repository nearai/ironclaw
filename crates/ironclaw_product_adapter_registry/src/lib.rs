//! ProductAdapter host-api section contract and projection types.
//!
//! Validates and projects `ironclaw.product_adapter/v1` manifest sections.
//! The old registry runtime projection (`ProductAdapterRuntimeEntry` and its
//! store scan) was never the production path and was deleted by the
//! extension-runtime P2 dispatch cutover; the active snapshot is the
//! dispatch-time source of truth.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionInstallationError, ExtensionManifestRecord, ExtensionManifestV2,
    HostApiContractRegistry, HostApiId, HostApiManifestContext, HostApiManifestContract,
    HostApiManifestProjection, HostApiMultiplicity, HostApiRefV2, HostApiSectionError,
    ManifestSectionPath, ManifestSource, ManifestV2Error,
};
use ironclaw_host_api::{
    CapabilitySurfaceKind, ExtensionId, HostPortCatalog, IngressAuthPolicy, IngressRouteDescriptor,
    IngressRouteId,
};
use ironclaw_product_adapters::{
    AuthRequirement, DeclaredEgressTarget, EgressCredentialHandle, ProductAdapterCapabilities,
    ProductAdapterId, ProductCapabilityFlag, ProductSurfaceKind,
};
use serde::Deserialize;
use thiserror::Error;

pub use ironclaw_extensions::ManifestHash;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const PRODUCT_ADAPTER_HOST_API_ID: &str = "ironclaw.product_adapter/v1";
pub const PRODUCT_ADAPTER_SECTION_PREFIX: &str = "product_adapter";

pub fn parse_product_adapter_manifest_record(
    raw_toml: impl Into<String>,
    source: ManifestSource,
    host_port_catalog: &HostPortCatalog,
    manifest_hash: Option<ManifestHash>,
) -> Result<ExtensionManifestRecord, RegistryError> {
    let mut contracts = HostApiContractRegistry::new();
    register_product_adapter_host_api_contract(&mut contracts)?;
    let record = ExtensionManifestRecord::from_toml(
        raw_toml,
        source,
        host_port_catalog,
        manifest_hash,
        &contracts,
    )
    .map_err(|error| match error {
        ExtensionInstallationError::Manifest(error) => RegistryError::Manifest(error),
        other => RegistryError::Installation(other),
    })?;
    product_adapter_sections(&record)?;
    Ok(record)
}

pub fn product_adapter_sections(
    record: &ExtensionManifestRecord,
) -> Result<Vec<ProductAdapterHostApiSection>, RegistryError> {
    project_product_adapter_sections(record.raw_toml(), record.manifest())
}

/// A host-ingress route declared by a ProductAdapter manifest section, paired
/// with the credential handles that verify it.
///
/// The route itself is the host-owned [`IngressRouteDescriptor`] vocabulary
/// (`ironclaw_host_api` owns route/policy validation, including the fail-closed
/// floor that a `PublicWebhook` listener must require `WebhookSignature`). That
/// descriptor deliberately carries **no** credential binding — host_api is
/// route/policy vocabulary only. The manifest layer is therefore where "which
/// credential handle verifies this route" is declared, and this crate makes it
/// credential-coherent against the section's `required_credentials`
/// (see [`ProductAdapterHostApiSection::validate`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIngressRoute {
    descriptor: IngressRouteDescriptor,
    credential_handles: Vec<EgressCredentialHandle>,
}

impl HostIngressRoute {
    /// The host-owned, already-validated ingress route/policy descriptor.
    pub fn descriptor(&self) -> &IngressRouteDescriptor {
        &self.descriptor
    }

    /// Credential handles that verify this route. Every handle is guaranteed to
    /// be declared in the owning section's `required_credentials`; an
    /// auth-required route names at least one, and a public (no-auth) route
    /// names none.
    ///
    /// The handle type is [`EgressCredentialHandle`] — the single credential-
    /// handle newtype `ironclaw_product_adapters` owns. It is reused here rather
    /// than mirrored into an ingress-specific type (per the type-placement
    /// rule); its `Display` renders only the handle string, so no "egress"
    /// wording leaks into ingress error messages.
    pub fn credential_handles(&self) -> &[EgressCredentialHandle] {
        &self.credential_handles
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAdapterHostApiSection {
    adapter_id: ProductAdapterId,
    section: ManifestSectionPath,
    surface_kind: ProductSurfaceKind,
    capabilities: ProductAdapterCapabilities,
    auth_requirement: AuthRequirement,
    declared_egress: Vec<DeclaredEgressTarget>,
    required_credentials: Vec<EgressCredentialHandle>,
    host_ingress: Vec<HostIngressRoute>,
}

impl ProductAdapterHostApiSection {
    fn from_value(
        extension_id: &ExtensionId,
        section: ManifestSectionPath,
        value: toml::Value,
    ) -> Result<Self, RegistryError> {
        reject_inline_secret_material_value(section.as_str(), &value)?;
        let raw: RawProductAdapterSection =
            value.try_into().map_err(|error: toml::de::Error| {
                RegistryError::ManifestSectionParse {
                    section: section.clone(),
                    reason: error.to_string(),
                }
            })?;
        // Derive adapter_id from the extension id and section subsection name
        // so that multiple product-adapter sections within the same extension
        // are distinguishable downstream.
        let subsection = section
            .as_str()
            .strip_prefix(PRODUCT_ADAPTER_SECTION_PREFIX)
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or("default");
        let adapter_id_str = format!("{}/{}", extension_id.as_str(), subsection);
        let adapter_id = ProductAdapterId::new(&adapter_id_str).map_err(|error| {
            RegistryError::InvalidValue {
                field: "adapter_id",
                reason: error.to_string(),
            }
        })?;
        let auth_requirement = raw.auth.into_auth_requirement()?;
        let required_credentials = raw
            .required_credentials
            .into_iter()
            .map(|c| c.handle)
            .collect();
        let host_ingress = raw
            .host_ingress
            .into_iter()
            .map(|route| HostIngressRoute {
                descriptor: route.descriptor,
                credential_handles: route.credential_handles,
            })
            .collect();
        let projected = Self {
            adapter_id,
            section,
            surface_kind: raw.surface_kind,
            capabilities: ProductAdapterCapabilities::new(raw.capabilities.flags),
            auth_requirement,
            declared_egress: raw.egress,
            required_credentials,
            host_ingress,
        };
        projected.validate()?;
        Ok(projected)
    }

    pub fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }
    pub fn section(&self) -> &ManifestSectionPath {
        &self.section
    }
    pub fn surface_kind(&self) -> ProductSurfaceKind {
        self.surface_kind
    }
    pub fn capabilities(&self) -> &ProductAdapterCapabilities {
        &self.capabilities
    }
    pub fn auth_requirement(&self) -> &AuthRequirement {
        &self.auth_requirement
    }
    pub fn declared_egress(&self) -> &[DeclaredEgressTarget] {
        &self.declared_egress
    }
    pub fn required_credentials(&self) -> &[EgressCredentialHandle] {
        &self.required_credentials
    }

    /// Host-ingress routes this ProductAdapter section declares. Each carries a
    /// host-owned [`IngressRouteDescriptor`] and its verifying credential
    /// handles; the serve layer projects these into mounted routes. Empty for
    /// sections that declare no ingress (the common case today).
    pub fn host_ingress(&self) -> &[HostIngressRoute] {
        &self.host_ingress
    }

    fn validate(&self) -> Result<(), RegistryError> {
        validate_auth_requirement(&self.auth_requirement)?;
        let mut required = BTreeSet::new();
        for handle in &self.required_credentials {
            if !required.insert(handle.clone()) {
                return Err(RegistryError::DuplicateCredentialHandle {
                    handle: handle.clone(),
                });
            }
        }
        let mut pairs = BTreeSet::new();
        for target in &self.declared_egress {
            if let Some(handle) = target.credential_handle.as_ref()
                && !required.contains(handle)
            {
                return Err(RegistryError::UndeclaredEgressCredentialHandle {
                    handle: handle.clone(),
                });
            }
            if !pairs.insert((target.host.clone(), target.credential_handle.clone())) {
                return Err(RegistryError::DuplicateEgressTarget);
            }
        }
        // Host-ingress credential coherence, fail closed. A route's declared
        // verifying credentials must line up with whether it is actually
        // authenticated, and every named handle must be declared in
        // `required_credentials` (mirroring the egress rule above, so ingress
        // handles flow into the same declared set installation bindings are
        // validated against). Route ids stay distinct within a section so a
        // mounted route can be addressed unambiguously.
        let mut route_ids: BTreeSet<&IngressRouteId> = BTreeSet::new();
        for route in &self.host_ingress {
            let route_id = route.descriptor.route_id();
            if !route_ids.insert(route_id) {
                return Err(RegistryError::DuplicateIngressRoute {
                    route_id: route_id.clone(),
                });
            }
            match route.descriptor.policy().auth() {
                // An auth-required route with no verifying credential is a route
                // nothing could authenticate — reject it.
                IngressAuthPolicy::Required { .. } => {
                    if route.credential_handles.is_empty() {
                        return Err(RegistryError::IngressRouteMissingCredential {
                            route_id: route_id.clone(),
                        });
                    }
                }
                // A public (no-auth) route is verified by nothing, so declaring a
                // credential handle on it is incoherent and misleading — a reader
                // would assume the route is authenticated by that credential.
                IngressAuthPolicy::Public { .. } => {
                    if !route.credential_handles.is_empty() {
                        return Err(RegistryError::PublicIngressRouteHasCredential {
                            route_id: route_id.clone(),
                        });
                    }
                }
            }
            for handle in &route.credential_handles {
                if !required.contains(handle) {
                    return Err(RegistryError::UndeclaredIngressCredentialHandle {
                        handle: handle.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ProductAdapter host-api contract validator
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ProductAdapterHostApiContract {
    id: HostApiId,
}

impl ProductAdapterHostApiContract {
    pub fn new() -> Result<Self, RegistryError> {
        Ok(Self {
            id: HostApiId::new(PRODUCT_ADAPTER_HOST_API_ID)?,
        })
    }
}

pub fn register_product_adapter_host_api_contract(
    registry: &mut HostApiContractRegistry,
) -> Result<(), RegistryError> {
    registry.register(Arc::new(ProductAdapterHostApiContract::new()?))?;
    Ok(())
}

impl HostApiManifestContract for ProductAdapterHostApiContract {
    fn id(&self) -> &HostApiId {
        &self.id
    }

    fn multiplicity(&self) -> HostApiMultiplicity {
        HostApiMultiplicity::Multiple
    }

    fn accepts_section_path(&self, section: &ManifestSectionPath) -> bool {
        section.as_str() == PRODUCT_ADAPTER_SECTION_PREFIX
            || section
                .as_str()
                .strip_prefix(PRODUCT_ADAPTER_SECTION_PREFIX)
                .is_some_and(|rest| rest.starts_with('.'))
    }

    fn validate_section(
        &self,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), HostApiSectionError> {
        // The contract hook runs while the generic manifest parser is still
        // validating the host-api section envelope, before it exposes the real
        // extension id to contract implementations. `from_value` needs an id
        // only to derive the adapter_id that this shape-only path discards;
        // cross-field checks involving the real extension id belong in
        // `project_product_adapter_sections` below.
        let placeholder =
            ExtensionId::new("x").map_err(|e| HostApiSectionError::from(e.to_string()))?;
        ProductAdapterHostApiSection::from_value(
            &placeholder,
            host_api.section.clone(),
            section.clone(),
        )
        .map(|_| ())
        .map_err(|e| HostApiSectionError::from(e.to_string()))
    }

    fn validate_section_with_context(
        &self,
        context: &HostApiManifestContext<'_>,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), HostApiSectionError> {
        ProductAdapterHostApiSection::from_value(
            context.extension_id,
            host_api.section.clone(),
            section.clone(),
        )
        .map(|_| ())
        .map_err(|e| HostApiSectionError::from(e.to_string()))
    }

    fn project_section_with_context(
        &self,
        context: &HostApiManifestContext<'_>,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<HostApiManifestProjection, HostApiSectionError> {
        let parsed = ProductAdapterHostApiSection::from_value(
            context.extension_id,
            host_api.section.clone(),
            section.clone(),
        )
        .map_err(|e| HostApiSectionError::from(e.to_string()))?;
        // External-channel adapter sections are the extension's channel
        // surface. The other product surface kinds (`web`, `cli`,
        // `synchronous_api`) describe host-native surfaces and project no
        // extension surface.
        let surfaces = match parsed.surface_kind() {
            ProductSurfaceKind::ExternalChannel => vec![CapabilitySurfaceKind::Channel],
            ProductSurfaceKind::Web
            | ProductSurfaceKind::Cli
            | ProductSurfaceKind::SynchronousApi => Vec::new(),
        };
        Ok(HostApiManifestProjection {
            capabilities: Vec::new(),
            surfaces,
        })
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error(transparent)]
    Installation(#[from] ExtensionInstallationError),
    #[error(transparent)]
    Manifest(#[from] ManifestV2Error),
    #[error("invalid {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    #[error("product adapter manifest section {section} parse failed: {reason}")]
    ManifestSectionParse {
        section: ManifestSectionPath,
        reason: String,
    },
    #[error("inline secret material is not allowed in manifest field {field}")]
    InlineSecretMaterial { field: String },
    #[error("duplicate credential handle {handle}")]
    DuplicateCredentialHandle { handle: EgressCredentialHandle },
    #[error("duplicate egress target")]
    DuplicateEgressTarget,
    #[error("egress references undeclared credential handle {handle}")]
    UndeclaredEgressCredentialHandle { handle: EgressCredentialHandle },
    #[error("host-ingress route references undeclared credential handle {handle}")]
    UndeclaredIngressCredentialHandle { handle: EgressCredentialHandle },
    #[error("auth-required host-ingress route {route_id} declares no verifying credential handle")]
    IngressRouteMissingCredential { route_id: IngressRouteId },
    #[error(
        "public host-ingress route {route_id} declares a verifying credential handle but is not authenticated"
    )]
    PublicIngressRouteHasCredential { route_id: IngressRouteId },
    #[error("duplicate host-ingress route {route_id}")]
    DuplicateIngressRoute { route_id: IngressRouteId },
    #[error("installation references unknown extension manifest {extension_id}")]
    UnknownManifest { extension_id: ExtensionId },
    #[error("installation binds undeclared credential handle {handle}")]
    UndeclaredCredentialHandle { handle: EgressCredentialHandle },
    #[error(
        "installation extension {extension_id} does not match manifest extension {manifest_extension_id}"
    )]
    ManifestExtensionMismatch {
        extension_id: ExtensionId,
        manifest_extension_id: ExtensionId,
    },
    #[error(
        "installation manifest hash does not match registered manifest hash for {extension_id}"
    )]
    ManifestHashMismatch { extension_id: ExtensionId },
}

// ---------------------------------------------------------------------------
// Internal validation helpers
// ---------------------------------------------------------------------------

fn validate_auth_requirement(requirement: &AuthRequirement) -> Result<(), RegistryError> {
    match requirement {
        AuthRequirement::RequestSignature {
            header_name,
            timestamp_header_name,
        } => {
            validate_http_token("auth.header_name", header_name)?;
            if let Some(t) = timestamp_header_name.as_deref() {
                validate_http_token("auth.timestamp_header_name", t)?;
            }
        }
        AuthRequirement::SharedSecretHeader { header_name } => {
            validate_http_token("auth.header_name", header_name)?;
        }
        AuthRequirement::SessionCookie { name } => {
            validate_http_token("auth.name", name)?;
        }
        AuthRequirement::BearerToken => {}
    }
    Ok(())
}

fn validate_http_token(field: &'static str, value: &str) -> Result<(), RegistryError> {
    if value.is_empty() {
        return Err(RegistryError::InvalidValue {
            field,
            reason: "must not be empty".to_string(),
        });
    }
    for c in value.chars() {
        if !is_http_tchar(c) {
            return Err(RegistryError::InvalidValue {
                field,
                reason: format!(
                    "must be an RFC 7230 token (no CTL, whitespace, or separators); got {value:?}"
                ),
            });
        }
    }
    Ok(())
}

fn is_http_tchar(c: char) -> bool {
    matches!(
        c,
        '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '.' | '^' | '_' | '`' | '|' | '~'
    ) || c.is_ascii_alphanumeric()
}

fn reject_inline_secret_material_value(
    path: &str,
    value: &toml::Value,
) -> Result<(), RegistryError> {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let child_path = format!("{path}.{key}");
                if is_secret_key_name(key) {
                    return Err(RegistryError::InlineSecretMaterial { field: child_path });
                }
                reject_inline_secret_material_value(&child_path, value)?;
            }
        }
        toml::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                reject_inline_secret_material_value(&format!("{path}[{index}]"), value)?;
            }
        }
        toml::Value::String(value) if looks_like_inline_secret(value) => {
            return Err(RegistryError::InlineSecretMaterial {
                field: path.to_string(),
            });
        }
        _ => {}
    }
    Ok(())
}

fn is_secret_key_name(key: &str) -> bool {
    let normalised: String = key
        .chars()
        .map(|c| {
            if c == '-' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    matches!(
        normalised.as_str(),
        "secret"
            | "secrets"
            | "secret_value"
            | "client_secret"
            | "webhook_secret"
            | "token"
            | "raw_token"
            | "access_token"
            | "refresh_token"
            | "bearer_token"
            | "oauth_token"
            | "auth_token"
            | "id_token"
            | "api_key"
            | "apikey"
            | "api_secret"
            | "private_key"
            | "password"
            | "passphrase"
    )
}

fn looks_like_inline_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("sha256:") {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "sk-",   // OpenAI / Anthropic style API keys.
        "xoxb-", // Slack bot token.
        "xoxa-", // Slack app token.
        "xoxp-", // Slack user token.
        "xoxs-", // Slack service token.
        "xoxe-", // Slack configuration token.
        "ghp_",  // GitHub personal access token.
        "gho_",  // GitHub OAuth token.
        "ghu_",  // GitHub user-to-server token.
        "ghs_",  // GitHub server-to-server token.
        "ghr_",  // GitHub refresh token.
    ];
    PREFIXES.iter().any(|p| lower.starts_with(p))
        || looks_like_aws_access_key(value)
        || lower.contains("begin private key")
        || lower.contains("begin rsa private key")
        || (value.len() >= 30 && value.starts_with("eyJ") && value.contains('.'))
        || has_uri_userinfo(value)
        || looks_like_telegram_token(value)
}

fn looks_like_aws_access_key(value: &str) -> bool {
    if value.len() != 20 {
        return false;
    }
    let Some(prefix) = value.get(..4) else {
        return false;
    };
    (prefix.eq_ignore_ascii_case("AKIA") || prefix.eq_ignore_ascii_case("ASIA"))
        && value[4..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn has_uri_userinfo(value: &str) -> bool {
    let Some((_, rest)) = value.split_once("://") else {
        return false;
    };
    rest.split('/').next().unwrap_or_default().contains('@')
}

fn looks_like_telegram_token(value: &str) -> bool {
    let Some((prefix, suffix)) = value.split_once(':') else {
        return false;
    };
    prefix.len() >= 6
        && prefix.chars().all(|c| c.is_ascii_digit())
        && suffix.len() >= 10
        && suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn project_product_adapter_sections(
    raw_toml: &str,
    manifest: &ExtensionManifestV2,
) -> Result<Vec<ProductAdapterHostApiSection>, RegistryError> {
    // Safety: PRODUCT_ADAPTER_SECTION_PREFIX is a non-empty, control-char-free
    // ASCII identifier defined as a module constant.
    let root_section = ManifestSectionPath::new(PRODUCT_ADAPTER_SECTION_PREFIX)
        .map_err(RegistryError::Manifest)?;
    // `ironclaw_extensions` validates host-api sections from its internal
    // TOML section table but does not expose that table as a public projection
    // API. Re-parse here so this crate can build typed ProductAdapter entries
    // without reaching through the manifest parser's private representation.
    // If profiling shows this is material, add a targeted section projection
    // API in `ironclaw_extensions` instead of caching private parser state here.
    let value: toml::Value =
        toml::from_str(raw_toml).map_err(|error| RegistryError::ManifestSectionParse {
            section: root_section.clone(),
            reason: error.to_string(),
        })?;
    let mut sections = Vec::new();
    for host_api in &manifest.host_apis {
        if host_api.id.as_str() != PRODUCT_ADAPTER_HOST_API_ID {
            continue;
        }
        let section_value = section_value(&value, &host_api.section)?;
        sections.push(ProductAdapterHostApiSection::from_value(
            &manifest.id,
            host_api.section.clone(),
            section_value.clone(),
        )?);
    }
    Ok(sections)
}

fn section_value<'a>(
    root: &'a toml::Value,
    path: &ManifestSectionPath,
) -> Result<&'a toml::Value, RegistryError> {
    let mut current = root;
    for segment in path.as_str().split('.') {
        current = current
            .as_table()
            .and_then(|table| table.get(segment))
            .ok_or_else(|| RegistryError::ManifestSectionParse {
                section: path.clone(),
                reason: "section path does not exist".to_string(),
            })?;
    }
    Ok(current)
}

// ---------------------------------------------------------------------------
// Raw deserialization shapes for ProductAdapter section
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProductAdapterSection {
    surface_kind: ProductSurfaceKind,
    auth: RawProductAdapterAuth,
    capabilities: RawProductAdapterCapabilities,
    #[serde(default)]
    required_credentials: Vec<RawProductAdapterCredential>,
    #[serde(default)]
    egress: Vec<DeclaredEgressTarget>,
    #[serde(default)]
    host_ingress: Vec<RawHostIngressRoute>,
}

/// Manifest shape for a declared host-ingress route: the full host-owned
/// [`IngressRouteDescriptor`] (validated by `ironclaw_host_api`'s own
/// `Deserialize` — `deny_unknown_fields`, dotted route id, absolute path, and
/// all policy cross-field invariants) plus the credential handles that verify
/// it. Credential coherence against `required_credentials` is enforced in
/// [`ProductAdapterHostApiSection::validate`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostIngressRoute {
    descriptor: IngressRouteDescriptor,
    #[serde(default)]
    credential_handles: Vec<EgressCredentialHandle>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProductAdapterCapabilities {
    flags: Vec<ProductCapabilityFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProductAdapterCredential {
    handle: EgressCredentialHandle,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawProductAdapterAuth {
    RequestSignature {
        header_name: String,
        #[serde(default)]
        timestamp_header_name: Option<String>,
    },
    SharedSecretHeader {
        header_name: String,
    },
    SessionCookie {
        name: String,
    },
    BearerToken,
}

impl RawProductAdapterAuth {
    fn into_auth_requirement(self) -> Result<AuthRequirement, RegistryError> {
        let requirement = match self {
            Self::RequestSignature {
                header_name,
                timestamp_header_name,
            } => AuthRequirement::RequestSignature {
                header_name,
                timestamp_header_name,
            },
            Self::SharedSecretHeader { header_name } => {
                AuthRequirement::SharedSecretHeader { header_name }
            }
            Self::SessionCookie { name } => AuthRequirement::SessionCookie { name },
            Self::BearerToken => AuthRequirement::BearerToken,
        };
        validate_auth_requirement(&requirement)?;
        Ok(requirement)
    }
}

#[cfg(test)]
mod tests {
    //! Unit coverage for host-ingress credential coherence — the novel logic
    //! this crate adds on top of host_api's already-validated ingress
    //! descriptor. Descriptors are built in Rust (not TOML text) so these
    //! cases are robust to serde renames; the wire path is covered end-to-end
    //! in `tests/manifest_ingestion.rs`.
    use super::*;
    use ironclaw_host_api::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthScheme,
        IngressJustification, IngressPolicy, IngressPolicyParts, IngressScopeSource, ListenerClass,
        NetworkMethod, RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
    };
    use serde::Serialize;
    use std::num::{NonZeroU32, NonZeroU64};

    /// A fail-closed public-webhook descriptor mirroring the values Slack's
    /// `slack_events_policy()` uses, parameterized by route id.
    fn webhook_descriptor(route_id: &str) -> IngressRouteDescriptor {
        let policy = IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::PublicWebhook,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::WebhookSignature],
            },
            scope_source: IngressScopeSource::HostResolved,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(262_144).expect("nonzero"),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::Global,
                max_requests: NonZeroU32::new(600).expect("nonzero"),
                window_seconds: NonZeroU32::new(60).expect("nonzero"),
            },
            cors: CorsPolicy::NotApplicable,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::ProductWorkflow,
        })
        .expect("policy validates");
        IngressRouteDescriptor::new(
            route_id,
            NetworkMethod::Post,
            "/webhooks/telegram/updates",
            policy,
        )
        .expect("descriptor validates")
    }

    /// A valid public (no-auth) route, mirroring the SSO login mount's policy
    /// combination (LocalGateway + Public + PublicRoute + NoEffect).
    fn public_descriptor(route_id: &str) -> IngressRouteDescriptor {
        let policy = IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::LocalGateway,
            auth: IngressAuthPolicy::Public {
                justification: IngressJustification::new("ingress", "public test route")
                    .expect("justification"),
            },
            scope_source: IngressScopeSource::PublicRoute,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(4096).expect("nonzero"),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::PerIp,
                max_requests: NonZeroU32::new(60).expect("nonzero"),
                window_seconds: NonZeroU32::new(60).expect("nonzero"),
            },
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::NoEffect,
        })
        .expect("public policy validates");
        IngressRouteDescriptor::new(route_id, NetworkMethod::Post, "/public/callback", policy)
            .expect("descriptor validates")
    }

    #[derive(Serialize)]
    struct RouteFixture {
        descriptor: IngressRouteDescriptor,
        credential_handles: Vec<String>,
    }

    /// Build a ProductAdapter section `toml::Value` with a valid base and the
    /// given host-ingress routes, then run it through the real projection.
    fn project(routes: Vec<RouteFixture>) -> Result<ProductAdapterHostApiSection, RegistryError> {
        let mut value: toml::Value = toml::from_str(
            r#"
surface_kind = "external_channel"
[auth]
kind = "shared_secret_header"
header_name = "X-Telegram-Bot-Api-Secret-Token"
[capabilities]
flags = ["inbound_messages"]
[[required_credentials]]
handle = "telegram_bot_token"
"#,
        )
        .expect("base section parses");
        let host_ingress = toml::Value::try_from(routes).expect("routes serialize");
        value
            .as_table_mut()
            .expect("section is a table")
            .insert("host_ingress".to_string(), host_ingress);

        let extension_id = ExtensionId::new("telegram-v2").expect("extension id");
        let section = ManifestSectionPath::new("product_adapter.inbound").expect("section path");
        ProductAdapterHostApiSection::from_value(&extension_id, section, value)
    }

    fn route(route_id: &str, credential_handles: &[&str]) -> RouteFixture {
        RouteFixture {
            descriptor: webhook_descriptor(route_id),
            credential_handles: credential_handles.iter().map(|h| h.to_string()).collect(),
        }
    }

    #[test]
    fn host_ingress_route_projects_descriptor_and_handles() {
        let section = project(vec![route("telegram.updates", &["telegram_bot_token"])])
            .expect("valid section projects");
        assert_eq!(section.host_ingress().len(), 1);
        let projected = &section.host_ingress()[0];
        assert_eq!(
            projected.descriptor().route_id().as_str(),
            "telegram.updates"
        );
        assert_eq!(
            projected.descriptor().route_pattern().as_str(),
            "/webhooks/telegram/updates"
        );
        assert_eq!(projected.credential_handles().len(), 1);
        assert_eq!(
            projected.credential_handles()[0].as_str(),
            "telegram_bot_token"
        );
    }

    #[test]
    fn host_ingress_undeclared_credential_handle_rejected() {
        let err = project(vec![route("telegram.updates", &["not_declared_token"])])
            .expect_err("undeclared handle must reject");
        assert!(
            matches!(err, RegistryError::UndeclaredIngressCredentialHandle { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn host_ingress_auth_required_route_needs_credential() {
        // Fail closed: an auth-required route with no verifying credential
        // handle must reject, not mount a route nothing can authenticate.
        let err = project(vec![route("telegram.updates", &[])])
            .expect_err("auth-required route without a credential must reject");
        assert!(
            matches!(err, RegistryError::IngressRouteMissingCredential { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn host_ingress_duplicate_route_id_rejected() {
        let err = project(vec![
            route("telegram.updates", &["telegram_bot_token"]),
            route("telegram.updates", &["telegram_bot_token"]),
        ])
        .expect_err("duplicate route id must reject");
        assert!(
            matches!(err, RegistryError::DuplicateIngressRoute { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn host_ingress_public_route_must_not_declare_credentials() {
        // Fail closed on the dual of the auth-required rule: a public (no-auth)
        // route is verified by nothing, so declaring a credential handle on it
        // is incoherent and would mislead a reader into assuming it is
        // authenticated.
        let err = project(vec![RouteFixture {
            descriptor: public_descriptor("public.callback"),
            credential_handles: vec!["telegram_bot_token".to_string()],
        }])
        .expect_err("public route with a credential handle must reject");
        assert!(
            matches!(err, RegistryError::PublicIngressRouteHasCredential { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn host_ingress_public_route_without_credentials_projects() {
        // The complement: a public route that declares no credentials is valid.
        let section = project(vec![RouteFixture {
            descriptor: public_descriptor("public.callback"),
            credential_handles: vec![],
        }])
        .expect("public route with no credentials projects");
        assert_eq!(section.host_ingress().len(), 1);
    }
}
