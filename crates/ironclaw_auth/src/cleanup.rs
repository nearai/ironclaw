use async_trait::async_trait;
use ironclaw_host_api::ExtensionId;
use serde::{Deserialize, Serialize};

use crate::{
    AuthFlowId, AuthProductError, AuthProviderId, AuthResolved, CredentialAccountId,
    LifecyclePackageRef, scope::AuthProductScope,
};

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
    /// Cancel every non-terminal flow whose `LifecycleActivation`
    /// continuation names this package, regardless of provider. Uninstall
    /// callers pass the removed extension's package ref so its own connect
    /// flows die with it even when the provider is shared with another
    /// installed extension (a provider-keyed cancel is deliberately skipped
    /// for shared providers, but the removed extension's flows must not
    /// survive to complete a late callback and then compensate away the
    /// shared credential).
    pub lifecycle_package: Option<LifecyclePackageRef>,
    pub action: SecretCleanupAction,
}

/// A flow that lifecycle cleanup drove to (or found already in) a terminal
/// state, so callers can drop per-flow material stored outside the flow
/// record — today the setup-path PKCE verifier secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanceledCleanupFlow {
    pub scope: AuthProductScope,
    pub flow_id: AuthFlowId,
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
    /// Canceled turn-gate continuations that the composition layer must deny
    /// through the canonical turn coordinator before lifecycle cleanup is
    /// complete. This internal handoff is deliberately omitted from product
    /// responses; it carries no secret material.
    #[serde(skip)]
    pub auth_resolutions: Vec<AuthResolved>,
    /// Flows this cleanup walked to a terminal state, so the composition
    /// layer can eagerly drop their durable setup PKCE verifier secrets
    /// instead of leaving them to TTL expiry. Internal handoff only — never
    /// serialized into product responses.
    #[serde(skip)]
    pub canceled_flows: Vec<CanceledCleanupFlow>,
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
