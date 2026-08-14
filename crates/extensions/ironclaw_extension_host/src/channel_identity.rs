//! Generic channel identity scoping over durable extension installation state.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::{
    channel::ChannelConnectionStrategy,
    channel_identity::{ChannelConnectionScope, ChannelConnectionScopeSource},
};
use ironclaw_extension_registry::ExtensionInstallationStorePort;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::product_adapter::AdapterInstallationId;

use crate::ChannelConfigService;

/// Durable identity-key namespace selected by the connection ceremony.
///
/// The proof-code pairing and OAuth paths write `{installation}:{actor}`.
/// Device-linked channels write a versioned segment instead, and that prefix
/// is a **fence, not a fallback**: a row written by the retired proof-code
/// ceremony can never satisfy a device-link lookup, so switching a channel's
/// strategy to `device_link` fails closed — previously paired users are back
/// at setup and re-link, exactly the ceremony a fresh installation gets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChannelIdentityKeyspace {
    #[default]
    Unversioned,
    DeviceLinkV1,
}

impl ChannelIdentityKeyspace {
    /// The one keyspace a connection strategy reads and writes.
    ///
    /// Each strategy owns exactly one namespace — deliberately not a list, so
    /// a cross-namespace fallback chain is not expressible. Rows the retired
    /// proof-code ceremony left under `Unversioned` are inert data to a
    /// device-link channel: never resolved for admission, connection status,
    /// command roles, or outbound-target validation, and removed only by
    /// their owner's explicit disconnect (the unversioned delete prefix
    /// lexically contains the versioned one).
    pub fn for_strategy(strategy: Option<ChannelConnectionStrategy>) -> Self {
        match strategy {
            Some(ChannelConnectionStrategy::DeviceLink) => Self::DeviceLinkV1,
            _ => Self::Unversioned,
        }
    }

    pub fn provider_user_id(
        self,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> String {
        format!(
            "{}{external_actor_id}",
            self.provider_user_id_prefix(installation_id)
        )
    }

    pub fn provider_user_id_prefix(self, installation_id: &AdapterInstallationId) -> String {
        match self {
            Self::Unversioned => format!("{}:", installation_id.as_str()),
            Self::DeviceLinkV1 => format!("{}:device-link-v1:", installation_id.as_str()),
        }
    }
}

/// The generic `[channel.config]`-backed scope source: the installation record
/// supplies the adapter installation id; non-secret config values whose
/// handles carry a claim suffix supply the expected claim values.
struct ChannelConfigConnectionScopeSource {
    installation_store: Arc<dyn ExtensionInstallationStorePort>,
    extension_id: ExtensionId,
    channel_config: Option<Arc<ChannelConfigService>>,
}

#[async_trait]
impl ChannelConnectionScopeSource for ChannelConfigConnectionScopeSource {
    async fn resolve_connection_scope(&self) -> Result<Option<ChannelConnectionScope>, String> {
        let Some(record) = self
            .installation_store
            .get_manifest(&self.extension_id)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        if record.resolved().channel.is_none() {
            return Ok(None);
        };
        let installation = self
            .installation_store
            .list_installations()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|installation| installation.extension_id() == &self.extension_id);
        let Some(installation) = installation else {
            return Ok(None);
        };
        let installation_id = AdapterInstallationId::new(installation.installation_id().as_str())
            .map_err(|error| error.to_string())?;
        let values = if let Some(channel_config) = &self.channel_config {
            channel_config
                .effective_non_secret_config(&self.extension_id)
                .await
                .map_err(|error| error.to_string())?
        } else {
            Vec::new()
        };
        let expected = |claim: &str| -> Option<String> {
            values
                .iter()
                .find(|(handle, _)| handle_declares_claim(handle, claim))
                .map(|(_, value)| value.clone())
                .filter(|value| !value.trim().is_empty())
        };
        Ok(Some(ChannelConnectionScope {
            installation_id,
            expected_team_id: expected("team_id"),
            expected_enterprise_id: expected("enterprise_id"),
            expected_app_id: expected("app_id"),
        }))
    }
}

/// Handle-suffix convention: `{claim}` or `*_{claim}` declares the expected
/// value for that identity claim.
pub fn handle_declares_claim(handle: &str, claim: &str) -> bool {
    handle == claim
        || handle
            .strip_suffix(claim)
            .is_some_and(|prefix| prefix.ends_with('_'))
}

/// The generic scope source for one extension over the durable installation
/// store.
pub fn channel_config_connection_scope_source(
    installation_store: Arc<dyn ExtensionInstallationStorePort>,
    extension_id: ExtensionId,
    channel_config: Option<Arc<ChannelConfigService>>,
) -> Arc<dyn ChannelConnectionScopeSource> {
    Arc::new(ChannelConfigConnectionScopeSource {
        installation_store,
        extension_id,
        channel_config,
    })
}

/// A generically-discovered channel extension: its id and the auth vendors
/// its manifest declares.
pub struct DiscoveredChannelExtension {
    pub extension_id: String,
    pub providers: Vec<String>,
    pub identity_keyspace: ChannelIdentityKeyspace,
}

/// Installed extensions whose manifest declares a channel surface, excluding
/// `overridden` extension ids whose lane owns identity binding.
pub async fn discover_channel_extensions(
    installation_store: &Arc<dyn ExtensionInstallationStorePort>,
    overridden: &BTreeSet<String>,
) -> Result<Vec<DiscoveredChannelExtension>, String> {
    let manifests = installation_store
        .list_manifests()
        .await
        .map_err(|error| error.to_string())?;
    let mut discovered = Vec::new();
    for record in manifests {
        let resolved = record.resolved();
        if resolved.channel.is_none() {
            continue;
        }
        let extension_id = resolved.id.as_str().to_string();
        if overridden.contains(&extension_id) {
            continue;
        }
        discovered.push(DiscoveredChannelExtension {
            extension_id,
            identity_keyspace: ChannelIdentityKeyspace::for_strategy(
                resolved
                    .channel
                    .as_ref()
                    .and_then(|channel| channel.connection.as_ref())
                    .map(|connection| connection.strategy),
            ),
            providers: resolved
                .auth
                .iter()
                .map(|surface| surface.vendor.as_str().to_string())
                .collect(),
        });
    }
    Ok(discovered)
}
