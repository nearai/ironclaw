//! Thin composition shim for generic extension-host assembly.
//!
//! The loader, lane/native/channel binding, active publication, and resolver
//! construction live in `ironclaw_extension_host`. Composition only translates
//! durable product-owned installation/config state into boot records.

use std::sync::Arc;

use ironclaw_extensions::{ExtensionActivationState, ExtensionInstallationStore};
use ironclaw_host_api::ExtensionId;

pub(crate) use ironclaw_extension_host::{
    GenericExtensionHostParams, build_generic_extension_host, effective_resolved_for_package,
};

use crate::RebornBuildError;
use crate::extension_host::channel_config::ChannelConfigService;

pub(crate) async fn boot_installation_records(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    channel_config: Option<&Arc<ChannelConfigService>>,
) -> Result<Vec<ironclaw_extension_host::InstallationRecord>, RebornBuildError> {
    let mut records = Vec::new();
    for installation in installation_store
        .list_installations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("extension installations could not be listed: {error}"),
        })?
    {
        if installation.activation_state() != ExtensionActivationState::Enabled {
            continue;
        }
        let extension_id = installation.extension_id().clone();
        let Some(manifest_record) = installation_store
            .get_manifest(&extension_id)
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("extension manifest could not be loaded: {error}"),
            })?
        else {
            continue;
        };
        records.push(boot_installation_record(
            installation.installation_id().as_str(),
            &extension_id,
            manifest_record.resolved(),
            effective_channel_config(installation_store, channel_config, &extension_id).await?,
        ));
    }
    Ok(records)
}

async fn effective_channel_config(
    installation_store: &Arc<dyn ExtensionInstallationStore>,
    channel_config: Option<&Arc<ChannelConfigService>>,
    extension_id: &ExtensionId,
) -> Result<Vec<(String, String)>, RebornBuildError> {
    match channel_config {
        Some(channel_config) => channel_config
            .effective_non_secret_config(extension_id)
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("effective extension configuration could not be loaded: {error}"),
            }),
        None => installation_store
            .channel_config(extension_id)
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("extension channel config could not be loaded: {error}"),
            }),
    }
}

fn boot_installation_record(
    installation_id: &str,
    extension_id: &ExtensionId,
    resolved: &ironclaw_extensions::ResolvedExtensionManifest,
    config: Vec<(String, String)>,
) -> ironclaw_extension_host::InstallationRecord {
    ironclaw_extension_host::InstallationRecord {
        extension_id: extension_id.as_str().to_string(),
        installation_id: installation_id.to_string(),
        state: ironclaw_extension_host::InstallationState::Installed,
        resolved: Arc::new(resolved.clone()),
        config,
        last_error: None,
    }
}
