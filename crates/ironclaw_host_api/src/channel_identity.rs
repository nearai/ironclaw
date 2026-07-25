//! Neutral channel identity and connection-scoping vocabulary.
//!
//! Channel OAuth and proof-code pairing flows need a shared description of the
//! installation-scoped identity binding key and optional post-bind hooks. This
//! module intentionally contains no product-auth, persistence, or runtime
//! implementation.

use std::sync::Arc;

use crate::{AdapterInstallationId, UserId};

/// One extension's connection scope: the adapter installation the bindings
/// key under plus the identity claim values a proven vendor identity must
/// match before it may bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConnectionScope {
    pub installation_id: AdapterInstallationId,
    pub expected_team_id: Option<String>,
    pub expected_enterprise_id: Option<String>,
    pub expected_app_id: Option<String>,
}

impl ChannelConnectionScope {
    /// Whether any scoping claim value is configured. A scope with no
    /// expected claims is "not configured yet"; binding paths fail closed.
    pub fn has_expected_claims(&self) -> bool {
        self.expected_team_id.is_some()
            || self.expected_enterprise_id.is_some()
            || self.expected_app_id.is_some()
    }

    /// The installation-scoped provider-user-id prefix every binding under
    /// this scope shares.
    pub fn provider_user_id_prefix(&self) -> String {
        format!("{}:", self.installation_id.as_str())
    }
}

/// Resolves one extension's current [`ChannelConnectionScope`].
///
/// `Ok(None)` means the extension's connection scoping is not configured yet;
/// identity-binding and connection paths fail closed on it.
#[async_trait::async_trait]
pub trait ChannelConnectionScopeSource: Send + Sync {
    async fn resolve_connection_scope(&self) -> Result<Option<ChannelConnectionScope>, String>;
}

/// Vendor residue hook after a successful identity bind. Implementations must
/// not fail the already-completed bind; surface failures via their own logs or
/// telemetry.
pub trait ChannelIdentityPostBind: Send + Sync {
    fn provision_after_bind(&self, user_id: UserId, external_actor_id: &str);
}

/// Builds per-extension post-bind provisioning for discovered channel
/// extensions.
pub trait ChannelIdentityPostBindFactory: Send + Sync {
    fn post_bind_for_extension(
        &self,
        extension_id: &str,
    ) -> Option<Arc<dyn ChannelIdentityPostBind>>;
}

/// Per-extension override for a channel lane whose configure surface predates
/// `[channel.config]`.
#[derive(Clone)]
pub struct ChannelIdentityOverride {
    pub extension_id: String,
    pub provider: String,
    pub scope_source: Arc<dyn ChannelConnectionScopeSource>,
    pub post_bind: Option<Arc<dyn ChannelIdentityPostBind>>,
}
