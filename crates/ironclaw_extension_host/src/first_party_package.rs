use ironclaw_host_api::EffectKind;

/// Byte content of one asset shipped inside a first-party package.
#[derive(Debug, Clone)]
pub struct FirstPartyPackageAsset {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// A package's user-facing onboarding copy, carried as plain data.
#[derive(Debug, Clone)]
pub struct FirstPartyPackageOnboarding {
    pub instructions: String,
    pub credential_instructions: Option<String>,
    pub setup_url: Option<String>,
    pub credential_next_step: String,
}

/// A bespoke OAuth setup credential requirement replacing the manifest-derived
/// one.
#[derive(Debug, Clone)]
pub struct FirstPartyPackageOAuthSetup {
    pub requirement_name: String,
    pub provider: String,
    pub scopes: Vec<String>,
}

/// An opaque, data-only first-party package the binary hands the extension
/// host. Concrete package identity lives only in the injecting binary.
#[derive(Debug, Clone)]
pub struct FirstPartyPackageBundle {
    pub id: String,
    pub display_name: String,
    pub manifest_toml: String,
    pub assets: Vec<FirstPartyPackageAsset>,
    pub onboarding: Option<FirstPartyPackageOnboarding>,
    pub oauth_setup: Option<FirstPartyPackageOAuthSetup>,
    pub trust_effects: Option<Vec<EffectKind>>,
    pub search_aliases: Vec<String>,
}

/// The reserved host-bundled extension ids contributed by the injected first
/// party bundle set.
pub fn first_party_reserved_extension_ids(bundles: &[FirstPartyPackageBundle]) -> Vec<String> {
    bundles.iter().map(|bundle| bundle.id.clone()).collect()
}
