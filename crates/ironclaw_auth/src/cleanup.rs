use async_trait::async_trait;
use ironclaw_host_api::ExtensionId;
use serde::{Deserialize, Serialize};

use crate::{AuthProductError, AuthProviderId, CredentialAccountId, scope::AuthProductScope};

/// Lifecycle event that drives credential/session cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretCleanupAction {
    Deactivate,
    Uninstall,
}

/// Accounts are matched at credential-owner granularity (the scope's
/// tenant/user/agent/project owner — see
/// [`AuthProductScope::to_credential_owner`]), never by full scope equality:
/// every lifecycle/disconnect caller re-derives its scope with a fresh
/// `invocation_id`, so exact-scope matching could never find the account the
/// OAuth flow stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretCleanupRequest {
    pub scope: AuthProductScope,
    pub extension_id: ExtensionId,
    /// Explicit opt-in that ALSO selects the owner's accounts issued by this
    /// provider. OAuth-minted personal credentials are stored `UserReusable`
    /// with no extension ownership or grants, so an extension-keyed cleanup
    /// can never reach them; per the crate guardrail, reusable credentials are
    /// untouched *by default* — this selector is the deliberate exception a
    /// channel disconnect uses to revoke (not delete) the caller's own
    /// personal token.
    pub provider: Option<AuthProviderId>,
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
