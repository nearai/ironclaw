//! Composition-neutral first-party package data + handler-registrar seam
//! (extension-runtime DEL-7).
//!
//! Composition must not name `ironclaw_first_party_extensions` in production
//! code, so the binary (`ironclaw_reborn_cli`) converts the concrete
//! `ironclaw_first_party_extensions::packages::PackageBundle` inventory into
//! these neutral, data-only [`FirstPartyPackageBundle`]s and injects them on the
//! [`crate::RebornHostBindings`]. Likewise it supplies concrete first-party
//! capability executors (GSuite, web tooling) as [`FirstPartyHandlerRegistrar`]s;
//! composition owns the generic registration loop and the
//! [`FirstPartyRegistrarContext`] each registrar consumes.

use std::sync::Arc;

use ironclaw_auth::{CredentialAccountRecordSource, CredentialAccountService};
use ironclaw_host_api::{EffectKind, HostApiError};
use ironclaw_host_runtime::{FirstPartyCapabilityRegistry, ProductAuthProviderRuntimePorts};

/// Byte content of one asset shipped inside a first-party package.
pub struct FirstPartyPackageAsset {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// A package's user-facing onboarding copy, carried as plain data (mirrors
/// `ironclaw_first_party_extensions::packages::PackageOnboarding`).
pub struct FirstPartyPackageOnboarding {
    pub instructions: String,
    pub credential_instructions: Option<String>,
    pub setup_url: Option<String>,
    pub credential_next_step: String,
}

/// A bespoke OAuth-*setup* credential requirement replacing the manifest-derived
/// one (mirrors `ironclaw_first_party_extensions::packages::PackageOAuthSetup`).
pub struct FirstPartyPackageOAuthSetup {
    pub requirement_name: String,
    pub provider: String,
    pub scopes: Vec<String>,
}

/// An opaque, data-only first-party package the binary hands composition. Host
/// code consumes this without naming the concrete package; the concrete
/// identity lives only in the injecting binary.
pub struct FirstPartyPackageBundle {
    pub id: String,
    pub display_name: String,
    pub manifest_toml: String,
    pub assets: Vec<FirstPartyPackageAsset>,
    /// Bespoke onboarding copy, `None` for packages that need no setup guidance.
    pub onboarding: Option<FirstPartyPackageOnboarding>,
    /// A bespoke OAuth-setup credential requirement replacing the
    /// manifest-derived one, `None` when the derived requirement is correct.
    pub oauth_setup: Option<FirstPartyPackageOAuthSetup>,
    /// Host authority effects this package is granted in the built-in trust
    /// policy (defense in depth; not derived from the manifest). `None` for
    /// packages whose trust comes from the WASM extension registry.
    pub trust_effects: Option<Vec<EffectKind>>,
    /// Extra catalog search aliases folded in by the injecting binary (e.g. the
    /// GSuite family's "google"/"workspace" terms), so composition search does
    /// not special-case any concrete id.
    pub search_aliases: Vec<String>,
}

/// The context composition supplies to each [`FirstPartyHandlerRegistrar`] so
/// the binary-owned registrar can build its concrete executor wrappers with the
/// host-mediated ports.
pub struct FirstPartyRegistrarContext {
    pub credential_account_service: Arc<dyn CredentialAccountService>,
    pub credential_account_record_source: Arc<dyn CredentialAccountRecordSource>,
    pub product_auth_runtime_ports: ProductAuthProviderRuntimePorts,
    /// Whether a Google OAuth backend was registered at build time. Gates a
    /// pre-dispatch "not configured" tool result (see the GSuite handler).
    pub google_oauth_configured: bool,
}

/// A binary-assembled first-party capability handler installer. Composition
/// runs every registrar once against the shared registry before installing it
/// via `with_first_party_capabilities`; the concrete executors and capability
/// ids live in the binary, never composition.
pub trait FirstPartyHandlerRegistrar: Send + Sync {
    fn register(
        &self,
        registry: &mut FirstPartyCapabilityRegistry,
        context: &FirstPartyRegistrarContext,
    ) -> Result<(), HostApiError>;
}

/// The reserved host-bundled extension ids contributed by the injected first
/// party bundle set: a filesystem/uploaded extension must never shadow one of
/// these ids. The NEAR AI host-managed extension id is reserved separately by
/// the catalog (it is not part of the injected inventory).
pub(crate) fn first_party_reserved_extension_ids(
    bundles: &[FirstPartyPackageBundle],
) -> Vec<String> {
    bundles.iter().map(|bundle| bundle.id.clone()).collect()
}

/// Convert the concrete `ironclaw_first_party_extensions` package inventory into
/// neutral [`FirstPartyPackageBundle`]s. Test-support only: this is the one
/// composition-side spot allowed to name the concrete inventory, mirroring the
/// conversion the binary performs in production, so unit tests can build the
/// catalog and trust policy without the binary. Production sources the bundles
/// from the injected build input.
///
/// Gated `#[cfg(test)]` (not `test-support`): it names
/// `ironclaw_first_party_extensions`, a dev-dependency unavailable when a
/// downstream consumer builds the crate with the `test-support` feature.
/// Integration tests build their bundles directly from the dev-dependency (see
/// `tests/support/first_party.rs`).
#[cfg(test)]
pub(crate) fn first_party_bundles_from_inventory() -> Vec<FirstPartyPackageBundle> {
    use ironclaw_first_party_extensions::is_gsuite_extension_id;
    use ironclaw_first_party_extensions::packages::{PackageAssetContent, bundled_packages};
    use ironclaw_host_api::ExtensionId;

    bundled_packages()
        .into_iter()
        .map(|bundle| {
            let assets = bundle
                .assets
                .into_iter()
                .map(|asset| {
                    let PackageAssetContent::Bytes(bytes) = asset.content;
                    FirstPartyPackageAsset {
                        path: asset.path,
                        bytes,
                    }
                })
                .collect();
            // Fold the GSuite family's catalog search aliases into the bundle so
            // composition search never special-cases a concrete id (mirrors the
            // binary-side conversion).
            let search_aliases = if ExtensionId::new(bundle.id)
                .map(|id| is_gsuite_extension_id(&id))
                .unwrap_or(false)
            {
                [
                    "google",
                    "gsuite",
                    "g suite",
                    "workspace",
                    "google workspace",
                ]
                .into_iter()
                .map(str::to_string)
                .collect()
            } else {
                Vec::new()
            };
            FirstPartyPackageBundle {
                id: bundle.id.to_string(),
                display_name: bundle.display_name.to_string(),
                manifest_toml: bundle.manifest_toml.into_owned(),
                assets,
                onboarding: bundle.onboarding.map(|copy| FirstPartyPackageOnboarding {
                    instructions: copy.instructions,
                    credential_instructions: copy.credential_instructions,
                    setup_url: copy.setup_url,
                    credential_next_step: copy.credential_next_step,
                }),
                oauth_setup: bundle.oauth_setup.map(|setup| FirstPartyPackageOAuthSetup {
                    requirement_name: setup.requirement_name,
                    provider: setup.provider,
                    scopes: setup.scopes,
                }),
                trust_effects: bundle.trust_effects,
                search_aliases,
            }
        })
        .collect()
}
