//! Host-owned authority fence for hosted-MCP catalog discovery.
//!
//! Discovery runs outside the lifecycle operation lock. This value captures
//! the exact typed inputs that authorized one discovery generation and owns
//! the equality decision used before that generation may be published.

use ironclaw_auth::{
    CredentialAccount, CredentialAccountId, CredentialAccountStatus, ProviderScope,
};
use ironclaw_extensions::{
    ExtensionInstallationError, ExtensionManifestRecord, ExtensionPackage, ManifestHash,
};
use ironclaw_host_api::{SecretHandle, sha256_digest_token};

/// The sealed, non-secret authority inputs for one hosted-MCP discovery pass.
///
/// This is not a transport DTO: it is an in-process lifecycle state that
/// prevents a catalog discovered from stale package, manifest, ceiling, or
/// credential authority from entering the active snapshot.
///
/// Authority equality is computed by [`Self::still_authorizes`] over a stable
/// projection (see [`credential_authority`]); this type deliberately does not
/// derive `PartialEq` so no caller can accidentally compare fences by their
/// full field set, which would include the credential accounts' volatile
/// `created_at`/`updated_at` timestamps and force spurious rechecks.
#[derive(Debug, Clone)]
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
    ///
    /// Credential accounts are compared through [`credential_authority`], a
    /// stable projection that excludes the volatile `created_at`/`updated_at`
    /// timestamps so a benign write to the credential row does not force a
    /// spurious recheck. Every other authority-relevant input (package,
    /// manifest digest, tool ceiling, and each account's identity, status,
    /// secret handles, and granted scopes) is still compared exactly, so the
    /// fence stays fail-closed on any real authority change.
    pub(crate) fn still_authorizes(&self, current: &Self) -> bool {
        self.package == current.package
            && self.manifest_hash == current.manifest_hash
            && self.max_tools == current.max_tools
            && credential_authority(&self.credential_accounts)
                == credential_authority(&current.credential_accounts)
    }
}

/// Stable, timestamp-free projection of a credential account's authority.
///
/// Borrowed from the account so the projection allocates nothing beyond the
/// outer `Vec`. Ordering is preserved (position-sensitive equality), matching
/// the previous whole-`Vec` comparison; only the volatile timestamps are
/// dropped.
#[derive(PartialEq, Eq)]
struct CredentialAuthorityProjection<'a> {
    id: CredentialAccountId,
    status: CredentialAccountStatus,
    access_secret: Option<&'a SecretHandle>,
    refresh_secret: Option<&'a SecretHandle>,
    scopes: &'a [ProviderScope],
}

fn credential_authority(accounts: &[CredentialAccount]) -> Vec<CredentialAuthorityProjection<'_>> {
    accounts
        .iter()
        .map(|account| CredentialAuthorityProjection {
            id: account.id,
            status: account.status,
            access_secret: account.access_secret.as_ref(),
            refresh_secret: account.refresh_secret.as_ref(),
            scopes: &account.scopes,
        })
        .collect()
}
