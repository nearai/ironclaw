//! Host-owned authority fence for hosted-MCP catalog discovery.
//!
//! Discovery runs outside the lifecycle operation lock. This value captures
//! the exact typed inputs that authorized one discovery generation and owns
//! the equality decision used before that generation may be published.

use ironclaw_auth::CredentialAccount;
use ironclaw_extensions::{
    ExtensionInstallationError, ExtensionManifestRecord, ExtensionPackage, ManifestHash,
};
use ironclaw_host_api::sha256_digest_token;

/// The sealed, non-secret authority inputs for one hosted-MCP discovery pass.
///
/// This is not a transport DTO: it is an in-process lifecycle state that
/// prevents a catalog discovered from stale package, manifest, ceiling, or
/// credential authority from entering the active snapshot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct McpDiscoveryFence {
    package: ExtensionPackage,
    manifest_hash: ManifestHash,
    max_tools: u32,
    credential_accounts: Vec<CredentialAccount>,
}

impl McpDiscoveryFence {
    /// Capture the current discovery inputs from their existing canonical
    /// records. The raw manifest digest fences changes that may not alter the
    /// resolved package projection.
    pub(crate) fn capture(
        package: ExtensionPackage,
        manifest: &ExtensionManifestRecord,
        credential_accounts: Vec<CredentialAccount>,
    ) -> Result<Self, ExtensionInstallationError> {
        let max_tools = manifest
            .resolved()
            .mcp
            .as_ref()
            .ok_or_else(|| ExtensionInstallationError::InvalidInstallation {
                reason: format!(
                    "hosted MCP extension {} has no resolved MCP declaration",
                    package.id.as_str()
                ),
            })?
            .max_tools;
        let manifest_hash = ManifestHash::new(sha256_digest_token(manifest.raw_toml().as_bytes()))?;
        Ok(Self {
            package,
            manifest_hash,
            max_tools,
            credential_accounts,
        })
    }

    pub(crate) fn package(&self) -> &ExtensionPackage {
        &self.package
    }

    pub(crate) fn max_tools(&self) -> u32 {
        self.max_tools
    }

    /// Returns true only when a fresh capture still grants the exact
    /// discovery generation represented by `self`.
    pub(crate) fn still_authorizes(&self, current: &Self) -> bool {
        self == current
    }
}
