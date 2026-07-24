//! Generic channel identity scoping over durable extension installation state.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_host_api::{
    AdapterInstallationId, ChannelConnectionScope, ChannelConnectionScopeSource, ExtensionId,
};

use crate::ChannelConfigService;

/// The generic `[channel.config]`-backed scope source: the installation record
/// supplies the adapter installation id; non-secret config values whose
/// handles carry a claim suffix supply the expected claim values.
struct ChannelConfigConnectionScopeSource {
    installation_store: Arc<dyn ExtensionInstallationStore>,
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
        let Some(channel) = record.resolved().channel.as_ref() else {
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
            self.installation_store
                .channel_config(&self.extension_id)
                .await
                .map_err(|error| error.to_string())?
        };
        let expected = |claim: &str| -> Option<String> {
            channel
                .config
                .fields
                .iter()
                .filter(|field| !field.secret)
                .find(|field| handle_declares_claim(field.handle.as_str(), claim))
                .and_then(|field| {
                    values
                        .iter()
                        .find(|(handle, _)| handle == field.handle.as_str())
                        .map(|(_, value)| value.clone())
                })
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
    installation_store: Arc<dyn ExtensionInstallationStore>,
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
}

/// Installed extensions whose manifest declares a channel surface, excluding
/// `overridden` extension ids whose lane owns identity binding.
pub async fn discover_channel_extensions(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
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
            providers: resolved
                .auth
                .iter()
                .map(|surface| surface.vendor.as_str().to_string())
                .collect(),
        });
    }
    Ok(discovered)
}
