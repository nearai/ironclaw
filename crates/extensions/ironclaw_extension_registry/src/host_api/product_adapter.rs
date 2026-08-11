//! The `ironclaw.product_adapter/v1` host-API manifest contract and its
//! resolved projection.
//!
//! The declared section *schema* — what an extension writes under
//! `[product_adapter.*]`, and the cross-field invariants it must satisfy — is
//! `ironclaw_extension_contracts::product_adapter_section` (§6.1.2: this is the
//! neutral host↔extension membrane vocabulary, and that crate parses no
//! manifests). What lives here is the registry's half (§6.8.1): hooking that
//! schema into v2 manifest ingestion, the raw-TOML guards that run before
//! deserialization, and pairing each resolved section with the
//! [`ManifestSectionPath`] it was declared at.
//!
//! The old registry runtime projection (`ProductAdapterRuntimeEntry` and its
//! store scan) was never the production path and was deleted by the
//! extension-runtime P2 dispatch cutover; the active snapshot is the
//! dispatch-time source of truth.

use std::sync::Arc;

use ironclaw_extension_contracts::product_adapter_section::{
    PRODUCT_ADAPTER_HOST_API_ID, PRODUCT_ADAPTER_SECTION_PREFIX, ProductAdapterSection,
    ProductAdapterSectionDeclaration, ProductAdapterSectionError,
};
use ironclaw_extension_contracts::surface::CapabilitySurfaceKind;
use ironclaw_host_api::product_adapter::ProductSurfaceKind;
use ironclaw_host_api::{host_port::HostPortCatalog, ids::ExtensionId};
use thiserror::Error;

use crate::installations::{ExtensionInstallationError, ExtensionManifestRecord, ManifestHash};
use crate::resolved::PackageRootBinding;
use crate::v2::{
    ExtensionManifestV2, HostApiContractRegistry, HostApiId, HostApiManifestContext,
    HostApiManifestContract, HostApiManifestProjection, HostApiMultiplicity, HostApiRefV2,
    HostApiSectionError, ManifestSectionPath, ManifestSource, ManifestV2Error,
};

/// Parse an extension manifest with the ProductAdapter host-API contract
/// registered, then project its product-adapter sections to prove they resolve.
pub fn parse_product_adapter_manifest_record(
    raw_toml: impl Into<String>,
    source: ManifestSource,
    host_port_catalog: &HostPortCatalog,
    manifest_hash: Option<ManifestHash>,
) -> Result<ExtensionManifestRecord, RegistryError> {
    let mut contracts = HostApiContractRegistry::new();
    register_product_adapter_host_api_contract(&mut contracts)?;
    let record = ExtensionManifestRecord::from_toml_with_root_binding(
        raw_toml,
        source,
        host_port_catalog,
        manifest_hash,
        &contracts,
        // Contract-projection helper: no package root is materialized here.
        PackageRootBinding::FabricateOnLoad,
    )
    .map_err(|error| match error {
        ExtensionInstallationError::Manifest(error) => RegistryError::Manifest(error),
        other => RegistryError::Installation(other),
    })?;
    product_adapter_sections(&record)?;
    Ok(record)
}

/// Every `[product_adapter.*]` section this manifest declares, resolved.
pub fn product_adapter_sections(
    record: &ExtensionManifestRecord,
) -> Result<Vec<ProductAdapterHostApiSection>, RegistryError> {
    project_product_adapter_sections(record.raw_toml(), record.manifest())
}

/// A resolved product-adapter section paired with the manifest section path it
/// was declared at.
///
/// The section path is the registry's vocabulary (v2 manifest grammar), which
/// is why this type lives here and the resolved section it wraps lives in
/// `ironclaw_extension_contracts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAdapterHostApiSection {
    section: ManifestSectionPath,
    resolved: ProductAdapterSection,
}

impl ProductAdapterHostApiSection {
    fn from_value(
        extension_id: &ExtensionId,
        section: ManifestSectionPath,
        value: toml::Value,
    ) -> Result<Self, RegistryError> {
        reject_inline_secret_material_value(section.as_str(), &value)?;
        let declaration: ProductAdapterSectionDeclaration =
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
        let resolved = declaration.resolve(extension_id, subsection)?;
        Ok(Self { section, resolved })
    }

    /// The manifest section path this section was declared at.
    pub fn section(&self) -> &ManifestSectionPath {
        &self.section
    }

    /// The resolved section itself. Reached through here rather than through
    /// per-field delegates, so this type adds the section path and mirrors
    /// nothing (`.claude/rules/type-placement.md`).
    pub fn resolved(&self) -> &ProductAdapterSection {
        &self.resolved
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
        let surfaces = match parsed.resolved().surface_kind() {
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
    /// The declared section is not a valid product-adapter section. Rendered
    /// transparently so the schema's own wording is what a manifest author
    /// reads, wherever the section was declared.
    #[error(transparent)]
    Section(#[from] ProductAdapterSectionError),
    #[error("product adapter manifest section {section} parse failed: {reason}")]
    ManifestSectionParse {
        section: ManifestSectionPath,
        reason: String,
    },
    #[error("inline secret material is not allowed in manifest field {field}")]
    InlineSecretMaterial { field: String },
    // Four installation-record variants (`UnknownManifest`,
    // `UndeclaredCredentialHandle`, `ManifestExtensionMismatch`,
    // `ManifestHashMismatch`) were dropped with the move: measured at zero
    // constructors and zero match sites workspace-wide, and each duplicated a
    // live `ExtensionInstallationError` variant this enum already wraps
    // transparently — a mirror inside one crate once the module landed here.
}

// ---------------------------------------------------------------------------
// Raw-TOML guards
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Section lookup
// ---------------------------------------------------------------------------

fn project_product_adapter_sections(
    raw_toml: &str,
    manifest: &ExtensionManifestV2,
) -> Result<Vec<ProductAdapterHostApiSection>, RegistryError> {
    // Safety: PRODUCT_ADAPTER_SECTION_PREFIX is a non-empty, control-char-free
    // ASCII identifier defined as a module constant.
    let root_section = ManifestSectionPath::new(PRODUCT_ADAPTER_SECTION_PREFIX)
        .map_err(RegistryError::Manifest)?;
    // The manifest parser validates host-api sections from its internal TOML
    // section table but does not expose that table as a public projection API.
    // Re-parse here rather than reaching through the parser's private
    // representation. If profiling shows this is material, add a targeted
    // section projection API to the parser instead of caching private state.
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
