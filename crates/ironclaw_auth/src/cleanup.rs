use async_trait::async_trait;
use ironclaw_host_api::ExtensionId;
use serde::{Deserialize, Serialize};

use crate::{AuthProductError, CredentialAccountId, scope::AuthProductScope};

/// Lifecycle event that drives credential/session cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretCleanupAction {
    Deactivate,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretCleanupRequest {
    pub scope: AuthProductScope,
    pub extension_id: ExtensionId,
    pub action: SecretCleanupAction,
}

/// Redacted cleanup report. It carries account ids only, never secret handles or
/// backend diagnostic details.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretCleanupReport {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked_accounts: Vec<CredentialAccountId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retained_accounts: Vec<CredentialAccountId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_grants: Vec<CredentialAccountId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quarantined_accounts: Vec<SecretCleanupQuarantine>,
}

/// Stable redacted cleanup quarantine category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretCleanupQuarantineReason {
    RevokeFailed,
    GrantRevokeFailed,
    TombstoneFailed,
    BackendUnavailable,
}

/// Redacted cleanup diagnostic. It names only the affected account and stable
/// failure category, never backend strings, secret handles, or host paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretCleanupQuarantine {
    pub account_id: CredentialAccountId,
    pub reason: SecretCleanupQuarantineReason,
}

#[async_trait]
pub trait SecretCleanupService: Send + Sync {
    async fn cleanup_for_lifecycle(
        &self,
        request: SecretCleanupRequest,
    ) -> Result<SecretCleanupReport, AuthProductError>;
}
