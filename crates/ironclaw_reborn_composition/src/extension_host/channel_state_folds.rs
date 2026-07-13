//! One-time load-time folds of the retired Slack channel lane's durable
//! state onto the generic channel storage (extension-runtime H.3/H.4).
//!
//! Sanctioned specificity: like
//! `extension_host/extension_installation_store.rs`, this migration module
//! names the concrete state it folds forward — nothing outside this module
//! reads a `slack`-named state root once the lane is deleted.
//!
//! - **H.3** — the retired setup surface's installation record
//!   (`/tenant-shared/slack-setup/installation.json`: plaintext ids plus
//!   revision-suffixed secret handles at the operator scope) folds onto the
//!   `[channel.config]` storage: non-secret values into the durable
//!   installation channel config, secret material re-put under the
//!   manifest-declared handles at the channel-egress scope.
//! - **H.4** — the lane's state roots fold onto the generic scheme:
//!   identity bindings (`slack-personal-binding/identities`) into the
//!   generic channel-identity store with the installation prefix rewritten
//!   onto the durable extension installation id; channel routes
//!   (`slack-channel-routes`) into the `slack_allowed_channels` /
//!   `slack_subject_routes` config values; DM targets
//!   (`slack-personal-binding/dm-targets`) into the generic channel
//!   DM-target store.
//!
//! Every step is idempotent (skip when the target value already exists),
//! skips malformed records with a log line, and never fails boot: the fold
//! reads the retired roots and only writes generic homes.

use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{ExtensionId, ResourceScope, SecretHandle, UserId, VirtualPath};
use ironclaw_secrets::{SecretMaterial, SecretStore};
use serde::Deserialize;

use crate::extension_host::channel_dm_targets::FilesystemChannelDmTargetStore;
use crate::extension_host::channel_identity_store::FilesystemChannelIdentityStore;
use crate::provider_identity::{
    RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
    RebornUserIdentityBindingError, RebornUserIdentityBindingStore, RebornUserIdentityLookup,
};

const FOLDED_EXTENSION_ID: &str = "slack";
/// Managed allowed-channel routes carry a host-derived subject with this
/// prefix; explicit subject routes carry an operator-chosen subject.
const MANAGED_CHANNEL_SUBJECT_PREFIX: &str = "user:slack-channel:";

const CONFIG_INSTALLATION_ID: &str = "slack_installation_id";
const CONFIG_TEAM_ID: &str = "slack_team_id";
const CONFIG_API_APP_ID: &str = "slack_api_app_id";
const CONFIG_BOT_USER_ID: &str = "slack_bot_user_id";
const CONFIG_SHARED_SUBJECT_USER_ID: &str = "slack_shared_subject_user_id";
const CONFIG_OAUTH_CLIENT_ID: &str = "slack_oauth_client_id";
const CONFIG_ALLOWED_CHANNELS: &str = "slack_allowed_channels";
const CONFIG_SUBJECT_ROUTES: &str = "slack_subject_routes";
const TARGET_BOT_TOKEN_HANDLE: &str = "slack_bot_token";
const TARGET_SIGNING_SECRET_HANDLE: &str = "slack_signing_secret";
const TARGET_OAUTH_CLIENT_SECRET_HANDLE: &str = "slack_oauth_client_secret";

/// Sanctioned legacy storage roots (extension-runtime H.4b): extensions
/// whose durable conversation-binding and idempotency trees predate the
/// generic extension-keyed root scheme keep reading and writing their
/// original roots — a **permanent data-compat surface**, not a pending
/// migration. LLM data is never deleted (repo law), and resetting or
/// re-rooting the idempotency tree would risk duplicate replays of
/// already-settled inbound events, so these trees are never folded.
///
/// One entry today: the retired Slack channel lane.
pub(crate) fn sanctioned_legacy_channel_workflow_storage_roots(
    tenant_id: &ironclaw_host_api::TenantId,
    extension_id: &str,
) -> Option<crate::extension_host::channel_host::ChannelWorkflowStorageRoots> {
    if extension_id != FOLDED_EXTENSION_ID {
        return None;
    }
    let tenant = crate::resource_scope_path_segment(tenant_id.as_str());
    let root = |path: String| match VirtualPath::new(path) {
        Ok(path) => Some(path),
        Err(error) => {
            tracing::warn!(%error, "invalid sanctioned legacy storage root; using generic root");
            None
        }
    };
    Some(
        crate::extension_host::channel_host::ChannelWorkflowStorageRoots {
            idempotency: root(format!(
                "/tenants/{tenant}/shared/slack-product-workflow/idempotency"
            ))?,
            conversations: root(format!("/tenants/{tenant}/shared/slack-conversations"))?,
        },
    )
}

/// Everything the fold reads and writes. The retired lane's secrets live at
/// `legacy_secret_scope` (the operator scope its setup service used); the
/// `[channel.config]` secret home is `channel_config_secret_scope` (the
/// channel-egress credential scope).
pub(crate) struct RetiredChannelStateFoldInputs {
    pub(crate) filesystem: Arc<dyn RootFilesystem>,
    pub(crate) installation_store: Arc<dyn ExtensionInstallationStore>,
    pub(crate) secret_store: Arc<dyn SecretStore>,
    pub(crate) legacy_secret_scope: ResourceScope,
    pub(crate) channel_config_secret_scope: ResourceScope,
    pub(crate) identity_store: Arc<FilesystemChannelIdentityStore>,
    pub(crate) dm_targets: Arc<FilesystemChannelDmTargetStore>,
}

/// What one fold pass changed — all zeros on an idempotent second run.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RetiredChannelStateFoldReport {
    pub(crate) config_values: usize,
    pub(crate) secrets: usize,
    pub(crate) identities: usize,
    pub(crate) route_values: usize,
    pub(crate) dm_targets: usize,
}

impl RetiredChannelStateFoldReport {
    pub(crate) fn changed(&self) -> bool {
        *self != Self::default()
    }
}

/// The retired setup record's wire shape (minimal reader — the setup
/// surface itself is deleted).
#[derive(Debug, Deserialize)]
struct RetiredInstallationSetup {
    installation_id: String,
    team_id: String,
    api_app_id: String,
    user_id: String,
    #[serde(default)]
    shared_subject_user_id: Option<String>,
    bot_token_handle: String,
    signing_secret_handle: String,
    #[serde(default)]
    oauth_client_id: Option<String>,
    #[serde(default)]
    oauth_client_secret_handle: Option<String>,
}

/// A retired identity-binding record.
#[derive(Debug, Deserialize)]
struct RetiredUserIdentity {
    provider: String,
    provider_user_id: String,
    user_id: String,
}

/// A retired channel-route record.
#[derive(Debug, Deserialize)]
struct RetiredChannelRoute {
    channel_id: String,
    subject_user_id: String,
    #[serde(default)]
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A retired personal DM-target record.
#[derive(Debug, Deserialize)]
struct RetiredPersonalDmTarget {
    team_id: String,
    user_id: String,
    slack_user_id: String,
    dm_channel_id: String,
}

/// Run the H.3 + H.4 folds. Never fails boot: faults are logged and the
/// affected records are skipped (they fold on a later boot).
pub(crate) async fn fold_retired_slack_channel_state(
    inputs: &RetiredChannelStateFoldInputs,
) -> RetiredChannelStateFoldReport {
    let mut report = RetiredChannelStateFoldReport::default();
    let tenant = crate::resource_scope_path_segment(inputs.legacy_secret_scope.tenant_id.as_str());
    let shared_root = format!("/tenants/{tenant}/shared");

    let extension_id = match ExtensionId::new(FOLDED_EXTENSION_ID) {
        Ok(extension_id) => extension_id,
        Err(error) => {
            tracing::warn!(%error, "retired channel-state fold skipped: invalid extension id");
            return report;
        }
    };

    let setup = read_retired_setup(inputs, &shared_root).await;
    if let Some(setup) = &setup {
        fold_setup_config(inputs, &extension_id, setup, &mut report).await;
        fold_setup_secrets(inputs, setup, &mut report).await;
    }
    fold_identities(inputs, &shared_root, setup.as_ref(), &mut report).await;
    fold_channel_routes(inputs, &extension_id, &shared_root, &mut report).await;
    fold_dm_targets(inputs, &shared_root, &mut report).await;

    if report.changed() {
        tracing::debug!(
            config_values = report.config_values,
            secrets = report.secrets,
            identities = report.identities,
            route_values = report.route_values,
            dm_targets = report.dm_targets,
            "folded retired channel-lane state onto the generic channel storage"
        );
    }
    report
}

async fn read_retired_setup(
    inputs: &RetiredChannelStateFoldInputs,
    shared_root: &str,
) -> Option<RetiredInstallationSetup> {
    let path = virtual_path(&format!("{shared_root}/slack-setup/installation.json"))?;
    let bytes = match inputs.filesystem.read_file(&path).await {
        Ok(bytes) => bytes,
        Err(FilesystemError::NotFound { .. }) => return None,
        Err(error) => {
            tracing::warn!(%error, "retired setup record unreadable; fold skipped");
            return None;
        }
    };
    match serde_json::from_slice::<RetiredInstallationSetup>(&bytes) {
        Ok(setup) => Some(setup),
        Err(error) => {
            tracing::warn!(%error, "retired setup record malformed; fold skipped");
            None
        }
    }
}

/// H.3 non-secret values → durable installation channel config (per-key
/// idempotence: a key the operator already saved is never overwritten).
async fn fold_setup_config(
    inputs: &RetiredChannelStateFoldInputs,
    extension_id: &ExtensionId,
    setup: &RetiredInstallationSetup,
    report: &mut RetiredChannelStateFoldReport,
) {
    let mut values = match inputs.installation_store.channel_config(extension_id).await {
        Ok(values) => values,
        Err(error) => {
            tracing::warn!(%error, "channel config unreadable; setup config fold skipped");
            return;
        }
    };
    let mut changed = 0_usize;
    let mut fold_value = |key: &str, value: &str| {
        if value.trim().is_empty() {
            return;
        }
        if values.iter().any(|(stored, _)| stored == key) {
            return;
        }
        values.push((key.to_string(), value.to_string()));
        changed += 1;
    };
    fold_value(CONFIG_INSTALLATION_ID, &setup.installation_id);
    fold_value(CONFIG_TEAM_ID, &setup.team_id);
    fold_value(CONFIG_API_APP_ID, &setup.api_app_id);
    fold_value(CONFIG_BOT_USER_ID, &setup.user_id);
    if let Some(shared_subject) = &setup.shared_subject_user_id {
        fold_value(CONFIG_SHARED_SUBJECT_USER_ID, shared_subject);
    }
    if let Some(oauth_client_id) = &setup.oauth_client_id {
        fold_value(CONFIG_OAUTH_CLIENT_ID, oauth_client_id);
    }
    if changed == 0 {
        return;
    }
    match inputs
        .installation_store
        .set_channel_config(extension_id, values)
        .await
    {
        Ok(()) => report.config_values += changed,
        Err(error) => {
            tracing::warn!(%error, "channel config write failed; setup config fold skipped");
        }
    }
}

/// H.3 secret material → the `[channel.config]` secret home (skip when the
/// manifest-named handle already has material at the channel-egress scope).
async fn fold_setup_secrets(
    inputs: &RetiredChannelStateFoldInputs,
    setup: &RetiredInstallationSetup,
    report: &mut RetiredChannelStateFoldReport,
) {
    let mut pairs: Vec<(&str, &str)> = vec![
        (setup.bot_token_handle.as_str(), TARGET_BOT_TOKEN_HANDLE),
        (
            setup.signing_secret_handle.as_str(),
            TARGET_SIGNING_SECRET_HANDLE,
        ),
    ];
    if let Some(oauth_secret_handle) = &setup.oauth_client_secret_handle {
        pairs.push((
            oauth_secret_handle.as_str(),
            TARGET_OAUTH_CLIENT_SECRET_HANDLE,
        ));
    }
    for (legacy, target) in pairs {
        if fold_one_secret(inputs, legacy, target).await {
            report.secrets += 1;
        }
    }
}

async fn fold_one_secret(
    inputs: &RetiredChannelStateFoldInputs,
    legacy_handle: &str,
    target_handle: &str,
) -> bool {
    let (Ok(legacy), Ok(target)) = (
        SecretHandle::new(legacy_handle),
        SecretHandle::new(target_handle),
    ) else {
        tracing::warn!(target_handle, "invalid secret handle; secret fold skipped");
        return false;
    };
    match inputs
        .secret_store
        .metadata(&inputs.channel_config_secret_scope, &target)
        .await
    {
        Ok(Some(_)) => return false,
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, target_handle, "secret metadata unreadable; fold skipped");
            return false;
        }
    }
    let lease = match inputs
        .secret_store
        .lease_once(&inputs.legacy_secret_scope, &legacy)
        .await
    {
        Ok(lease) => lease,
        Err(error) if error.is_unknown_secret() => return false,
        Err(error) => {
            tracing::warn!(%error, target_handle, "retired secret unreadable; fold skipped");
            return false;
        }
    };
    let material = match inputs
        .secret_store
        .consume(&inputs.legacy_secret_scope, lease.id)
        .await
    {
        Ok(material) => material,
        Err(error) => {
            tracing::warn!(%error, target_handle, "retired secret unreadable; fold skipped");
            return false;
        }
    };
    let material =
        SecretMaterial::from(secrecy::ExposeSecret::expose_secret(&material).to_string());
    match inputs
        .secret_store
        .put(
            inputs.channel_config_secret_scope.clone(),
            target,
            material,
            None,
        )
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, target_handle, "secret fold write failed; skipped");
            false
        }
    }
}

/// H.4 identity bindings → the generic channel-identity store, with the
/// retired operator installation prefix rewritten onto the durable
/// extension installation id (the key scheme generic inbound actor
/// resolution uses).
async fn fold_identities(
    inputs: &RetiredChannelStateFoldInputs,
    shared_root: &str,
    setup: Option<&RetiredInstallationSetup>,
    report: &mut RetiredChannelStateFoldReport,
) {
    let identities_root = format!("{shared_root}/slack-personal-binding/identities");
    let Some(root) = virtual_path(&identities_root) else {
        return;
    };
    let providers = match inputs.filesystem.list_dir(&root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return,
        Err(error) => {
            tracing::warn!(%error, "retired identity root unreadable; fold skipped");
            return;
        }
    };
    let generic_installation_id = durable_installation_id(inputs).await;
    let legacy_prefix = setup.map(|setup| format!("{}:", setup.installation_id));

    for provider_dir in providers {
        if provider_dir.file_type != FileType::Directory {
            continue;
        }
        let entries = match inputs.filesystem.list_dir(&provider_dir.path).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(%error, "retired identity provider dir unreadable; skipped");
                continue;
            }
        };
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let bytes = match inputs.filesystem.read_file(&entry.path).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(%error, "retired identity record unreadable; skipped");
                    continue;
                }
            };
            let record = match serde_json::from_slice::<RetiredUserIdentity>(&bytes) {
                Ok(record) => record,
                Err(error) => {
                    tracing::warn!(%error, "retired identity record malformed; skipped");
                    continue;
                }
            };
            let provider_user_id = rewrite_installation_prefix(
                &record.provider_user_id,
                legacy_prefix.as_deref(),
                generic_installation_id.as_deref(),
            );
            match inputs
                .identity_store
                .resolve_user_identity(&record.provider, &provider_user_id)
                .await
            {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "generic identity store unreadable; record skipped");
                    continue;
                }
            }
            let binding = match retired_identity_binding(&record, provider_user_id) {
                Some(binding) => binding,
                None => continue,
            };
            match inputs.identity_store.bind_user_identity(binding).await {
                Ok(()) => report.identities += 1,
                Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound) => {
                    tracing::warn!(
                        provider = %record.provider,
                        "retired identity binding conflicts with a generic binding; skipped"
                    );
                }
                Err(error) => {
                    tracing::warn!(%error, "retired identity binding fold failed; skipped");
                }
            }
        }
    }
}

fn retired_identity_binding(
    record: &RetiredUserIdentity,
    provider_user_id: String,
) -> Option<RebornUserIdentityBinding> {
    let provider = RebornIdentityProviderId::new(record.provider.clone()).ok()?;
    let provider_user_id = RebornIdentityProviderUserId::new(provider_user_id).ok()?;
    let user_id = UserId::new(record.user_id.clone()).ok()?;
    Some(RebornUserIdentityBinding {
        provider,
        provider_user_id,
        user_id,
    })
}

fn rewrite_installation_prefix(
    provider_user_id: &str,
    legacy_prefix: Option<&str>,
    generic_installation_id: Option<&str>,
) -> String {
    if let (Some(legacy_prefix), Some(generic)) = (legacy_prefix, generic_installation_id)
        && let Some(rest) = provider_user_id.strip_prefix(legacy_prefix)
    {
        return format!("{generic}:{rest}");
    }
    provider_user_id.to_string()
}

/// The durable extension installation id the generic channel workflows key
/// under.
async fn durable_installation_id(inputs: &RetiredChannelStateFoldInputs) -> Option<String> {
    let installations = match inputs.installation_store.list_installations().await {
        Ok(installations) => installations,
        Err(error) => {
            tracing::warn!(%error, "installation store unreadable; keys not rewritten");
            return None;
        }
    };
    installations
        .into_iter()
        .find(|installation| installation.extension_id().as_str() == FOLDED_EXTENSION_ID)
        .map(|installation| installation.installation_id().as_str().to_string())
}

/// H.4 channel routes → the `slack_allowed_channels` / `slack_subject_routes`
/// config values. Managed routes (host-derived subject) become allowed
/// channels; explicit subject routes become subject-route entries. Skipped
/// entirely once either config key exists (the operator owns them from then
/// on).
async fn fold_channel_routes(
    inputs: &RetiredChannelStateFoldInputs,
    extension_id: &ExtensionId,
    shared_root: &str,
    report: &mut RetiredChannelStateFoldReport,
) {
    let mut values = match inputs.installation_store.channel_config(extension_id).await {
        Ok(values) => values,
        Err(error) => {
            tracing::warn!(%error, "channel config unreadable; route fold skipped");
            return;
        }
    };
    if values
        .iter()
        .any(|(key, _)| key == CONFIG_ALLOWED_CHANNELS || key == CONFIG_SUBJECT_ROUTES)
    {
        return;
    }
    let Some(root) = virtual_path(&format!("{shared_root}/slack-channel-routes")) else {
        return;
    };
    let installations = match inputs.filesystem.list_dir(&root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return,
        Err(error) => {
            tracing::warn!(%error, "retired route root unreadable; fold skipped");
            return;
        }
    };
    let mut allowed: Vec<String> = Vec::new();
    let mut subject_routes: BTreeMap<String, String> = BTreeMap::new();
    for installation_dir in installations {
        if installation_dir.file_type != FileType::Directory {
            continue;
        }
        let teams = match inputs.filesystem.list_dir(&installation_dir.path).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(%error, "retired route installation dir unreadable; skipped");
                continue;
            }
        };
        for team_dir in teams {
            if team_dir.file_type != FileType::Directory {
                continue;
            }
            let entries = match inputs.filesystem.list_dir(&team_dir.path).await {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::warn!(%error, "retired route team dir unreadable; skipped");
                    continue;
                }
            };
            for entry in entries {
                if !entry.name.ends_with(".json") {
                    continue;
                }
                let bytes = match inputs.filesystem.read_file(&entry.path).await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        tracing::warn!(%error, "retired route record unreadable; skipped");
                        continue;
                    }
                };
                let route = match serde_json::from_slice::<RetiredChannelRoute>(&bytes) {
                    Ok(route) => route,
                    Err(error) => {
                        tracing::warn!(%error, "retired route record malformed; skipped");
                        continue;
                    }
                };
                if route.deleted_at.is_some() {
                    continue;
                }
                if route
                    .subject_user_id
                    .starts_with(MANAGED_CHANNEL_SUBJECT_PREFIX)
                {
                    allowed.push(route.channel_id);
                } else {
                    subject_routes.insert(route.channel_id, route.subject_user_id);
                }
            }
        }
    }
    allowed.sort();
    allowed.dedup();
    let mut changed = 0_usize;
    if !allowed.is_empty() {
        match serde_json::to_string(&allowed) {
            Ok(rendered) => {
                values.push((CONFIG_ALLOWED_CHANNELS.to_string(), rendered));
                changed += 1;
            }
            Err(error) => {
                tracing::warn!(%error, "allowed-channel list unserializable; skipped");
            }
        }
    }
    if !subject_routes.is_empty() {
        match serde_json::to_string(&subject_routes) {
            Ok(rendered) => {
                values.push((CONFIG_SUBJECT_ROUTES.to_string(), rendered));
                changed += 1;
            }
            Err(error) => {
                tracing::warn!(%error, "subject-route map unserializable; skipped");
            }
        }
    }
    if changed == 0 {
        return;
    }
    match inputs
        .installation_store
        .set_channel_config(extension_id, values)
        .await
    {
        Ok(()) => report.route_values += changed,
        Err(error) => {
            tracing::warn!(%error, "channel config write failed; route fold skipped");
        }
    }
}

/// H.4 DM targets → the generic channel DM-target store (skip per user when
/// a generic record already exists).
async fn fold_dm_targets(
    inputs: &RetiredChannelStateFoldInputs,
    shared_root: &str,
    report: &mut RetiredChannelStateFoldReport,
) {
    let Some(root) = virtual_path(&format!("{shared_root}/slack-personal-binding/dm-targets"))
    else {
        return;
    };
    let installations = match inputs.filesystem.list_dir(&root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return,
        Err(error) => {
            tracing::warn!(%error, "retired DM-target root unreadable; fold skipped");
            return;
        }
    };
    for installation_dir in installations {
        if installation_dir.file_type != FileType::Directory {
            continue;
        }
        let teams = match inputs.filesystem.list_dir(&installation_dir.path).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(%error, "retired DM-target installation dir unreadable; skipped");
                continue;
            }
        };
        for team_dir in teams {
            if team_dir.file_type != FileType::Directory {
                continue;
            }
            let entries = match inputs.filesystem.list_dir(&team_dir.path).await {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::warn!(%error, "retired DM-target team dir unreadable; skipped");
                    continue;
                }
            };
            for entry in entries {
                if !entry.name.ends_with(".json") {
                    continue;
                }
                let bytes = match inputs.filesystem.read_file(&entry.path).await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        tracing::warn!(%error, "retired DM-target record unreadable; skipped");
                        continue;
                    }
                };
                let target = match serde_json::from_slice::<RetiredPersonalDmTarget>(&bytes) {
                    Ok(target) => target,
                    Err(error) => {
                        tracing::warn!(%error, "retired DM-target record malformed; skipped");
                        continue;
                    }
                };
                let Ok(user_id) = UserId::new(target.user_id.clone()) else {
                    tracing::warn!("retired DM-target user id invalid; skipped");
                    continue;
                };
                match inputs.dm_targets.load(FOLDED_EXTENSION_ID, &user_id).await {
                    Ok(Some(_)) => continue,
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "generic DM-target store unreadable; skipped");
                        continue;
                    }
                }
                // Canonical generic payload: the DM's external ref
                // (space = the retired record's team, conversation = its DM
                // channel) — the same shape fresh generic provisioning
                // writes.
                let payload = crate::extension_host::channel_dm_targets::dm_target_payload(
                    Some(&target.team_id),
                    &target.dm_channel_id,
                );
                match inputs
                    .dm_targets
                    .upsert(FOLDED_EXTENSION_ID, &user_id, target.slack_user_id, payload)
                    .await
                {
                    Ok(_) => report.dm_targets += 1,
                    Err(error) => {
                        tracing::warn!(%error, "retired DM-target fold write failed; skipped");
                    }
                }
            }
        }
    }
}

fn virtual_path(raw: &str) -> Option<VirtualPath> {
    match VirtualPath::new(raw) {
        Ok(path) => Some(path),
        Err(error) => {
            tracing::warn!(%error, "invalid fold path; skipped");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use chrono::Utc;
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
        ExtensionManifestRecord, ExtensionManifestRef, InMemoryExtensionInstallationStore,
        ManifestSource,
    };
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{InvocationId, TenantId};
    use ironclaw_secrets::InMemorySecretStore;

    use super::*;

    const TENANT: &str = "tenant-alpha";
    const OPERATOR: &str = "user-operator";
    const DURABLE_INSTALLATION_ID: &str = "slack-durable-install";
    const RETIRED_INSTALLATION_ID: &str = "install-a";

    /// Minimal channel manifest carrying the folded extension id; the fold
    /// writes through the installation store directly, so only identity and
    /// parseability matter here.
    const FOLD_FIXTURE_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "slack"
name = "Slack"
version = "0.1.0"
description = "channel-state fold fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "slack.extension/v1"

[channel]
id = "messages"
display_name = "Slack messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "slack_signing_secret"
header = "X-Fixture-Secret"

[channel.config]
fields = [
  { handle = "slack_bot_token", label = "Bot token", secret = true },
  { handle = "slack_signing_secret", label = "Signing secret", secret = true },
  { handle = "slack_team_id", label = "Workspace (team) ID", secret = false },
]

[channel.presentation]
supports_markdown = true
supports_threads = true
"#;

    fn seg(value: &str) -> String {
        URL_SAFE_NO_PAD.encode(value.as_bytes())
    }

    fn scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            user_id: UserId::new(OPERATOR).expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    async fn write_json(
        filesystem: &Arc<dyn RootFilesystem>,
        path: &str,
        value: &serde_json::Value,
    ) {
        filesystem
            .write_file(
                &VirtualPath::new(path).expect("path"),
                &serde_json::to_vec(value).expect("serialize"),
            )
            .await
            .expect("seed retired record");
    }

    async fn installed_store() -> Arc<InMemoryExtensionInstallationStore> {
        let store = Arc::new(InMemoryExtensionInstallationStore::default());
        let record = ExtensionManifestRecord::from_toml(
            FOLD_FIXTURE_MANIFEST,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &ironclaw_host_runtime::default_host_api_contract_registry().expect("contracts"),
        )
        .expect("fixture manifest parses");
        let extension_id = ExtensionId::new("slack").expect("extension id");
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(DURABLE_INSTALLATION_ID.to_string())
                        .expect("installation id"),
                    extension_id.clone(),
                    ExtensionActivationState::Enabled,
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    Utc::now(),
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");
        store
    }

    struct Fixture {
        inputs: RetiredChannelStateFoldInputs,
        filesystem: Arc<dyn RootFilesystem>,
        installation_store: Arc<InMemoryExtensionInstallationStore>,
        secret_store: Arc<InMemorySecretStore>,
        scope: ResourceScope,
    }

    async fn fixture() -> Fixture {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let installation_store = installed_store().await;
        let secret_store = Arc::new(InMemorySecretStore::new());
        let scope = scope();
        let identity_store = Arc::new(FilesystemChannelIdentityStore::new(
            Arc::clone(&filesystem),
            scope.tenant_id.clone(),
            scope.user_id.clone(),
        ));
        let dm_targets = Arc::new(FilesystemChannelDmTargetStore::new(
            Arc::clone(&filesystem),
            scope.tenant_id.clone(),
            scope.user_id.clone(),
        ));
        let inputs = RetiredChannelStateFoldInputs {
            filesystem: Arc::clone(&filesystem),
            installation_store: Arc::clone(&installation_store)
                as Arc<dyn ExtensionInstallationStore>,
            secret_store: Arc::clone(&secret_store) as Arc<dyn SecretStore>,
            legacy_secret_scope: scope.clone(),
            channel_config_secret_scope: scope.clone(),
            identity_store,
            dm_targets,
        };
        Fixture {
            inputs,
            filesystem,
            installation_store,
            secret_store,
            scope,
        }
    }

    async fn seed_retired_state(fixture: &Fixture) {
        let shared = format!("/tenants/{TENANT}/shared");
        write_json(
            &fixture.filesystem,
            &format!("{shared}/slack-setup/installation.json"),
            &serde_json::json!({
                "installation_id": RETIRED_INSTALLATION_ID,
                "team_id": "T123",
                "api_app_id": "A9",
                "user_id": "user-bot-subject",
                "shared_subject_user_id": "user-shared",
                "bot_token_handle": "slack_bot_token_abc_v3",
                "signing_secret_handle": "slack_signing_secret_abc_v3",
                "oauth_client_id": "111.222",
                "oauth_client_secret_handle": "slack_oauth_client_secret_abc_v3",
                "revision": 3,
                "updated_at": Utc::now(),
            }),
        )
        .await;
        for (handle, material) in [
            ("slack_bot_token_abc_v3", "xoxb-material"),
            ("slack_signing_secret_abc_v3", "signing-material"),
            ("slack_oauth_client_secret_abc_v3", "oauth-secret-material"),
        ] {
            fixture
                .secret_store
                .put(
                    fixture.scope.clone(),
                    SecretHandle::new(handle).expect("handle"),
                    SecretMaterial::from(material.to_string()),
                    None,
                )
                .await
                .expect("seed retired secret");
        }
        write_json(
            &fixture.filesystem,
            &format!(
                "{shared}/slack-personal-binding/identities/{}/{}.json",
                seg("slack"),
                seg(&format!("{RETIRED_INSTALLATION_ID}:U777"))
            ),
            &serde_json::json!({
                "provider": "slack",
                "provider_user_id": format!("{RETIRED_INSTALLATION_ID}:U777"),
                "user_id": "user-alice",
                "created_at": Utc::now(),
                "updated_at": Utc::now(),
            }),
        )
        .await;
        let route_dir = format!(
            "{shared}/slack-channel-routes/{}/{}",
            seg(RETIRED_INSTALLATION_ID),
            seg("T123")
        );
        write_json(
            &fixture.filesystem,
            &format!("{route_dir}/{}.json", seg("C1")),
            &serde_json::json!({
                "tenant_id": TENANT,
                "installation_id": RETIRED_INSTALLATION_ID,
                "team_id": "T123",
                "channel_id": "C1",
                "subject_user_id": "user:slack-channel:deadbeef",
                "updated_at": Utc::now(),
            }),
        )
        .await;
        write_json(
            &fixture.filesystem,
            &format!("{route_dir}/{}.json", seg("C2")),
            &serde_json::json!({
                "tenant_id": TENANT,
                "installation_id": RETIRED_INSTALLATION_ID,
                "team_id": "T123",
                "channel_id": "C2",
                "subject_user_id": "user-ops",
                "updated_at": Utc::now(),
            }),
        )
        .await;
        // A tombstoned route must not fold.
        write_json(
            &fixture.filesystem,
            &format!("{route_dir}/{}.json", seg("C3")),
            &serde_json::json!({
                "tenant_id": TENANT,
                "installation_id": RETIRED_INSTALLATION_ID,
                "team_id": "T123",
                "channel_id": "C3",
                "subject_user_id": "user-gone",
                "updated_at": Utc::now(),
                "deleted_at": Utc::now(),
            }),
        )
        .await;
        write_json(
            &fixture.filesystem,
            &format!(
                "{shared}/slack-personal-binding/dm-targets/{}/{}/{}.json",
                seg(RETIRED_INSTALLATION_ID),
                seg("T123"),
                seg("user-alice")
            ),
            &serde_json::json!({
                "tenant_id": TENANT,
                "installation_id": RETIRED_INSTALLATION_ID,
                "team_id": "T123",
                "user_id": "user-alice",
                "slack_user_id": "U777",
                "dm_channel_id": "D42",
                "created_at": Utc::now(),
                "updated_at": Utc::now(),
            }),
        )
        .await;
    }

    fn config_value(values: &[(String, String)], key: &str) -> Option<String> {
        values
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, value)| value.clone())
    }

    #[tokio::test]
    async fn fold_moves_setup_state_roots_onto_generic_homes_and_second_run_is_a_noop() {
        let fixture = fixture().await;
        seed_retired_state(&fixture).await;
        let extension_id = ExtensionId::new("slack").expect("extension id");

        let report = fold_retired_slack_channel_state(&fixture.inputs).await;
        assert_eq!(
            report,
            RetiredChannelStateFoldReport {
                config_values: 6,
                secrets: 3,
                identities: 1,
                route_values: 2,
                dm_targets: 1,
            }
        );

        // H.3 non-secret values landed in the durable channel config.
        let values = fixture
            .installation_store
            .channel_config(&extension_id)
            .await
            .expect("channel config");
        assert_eq!(
            config_value(&values, "slack_installation_id").as_deref(),
            Some(RETIRED_INSTALLATION_ID)
        );
        assert_eq!(
            config_value(&values, "slack_team_id").as_deref(),
            Some("T123")
        );
        assert_eq!(
            config_value(&values, "slack_api_app_id").as_deref(),
            Some("A9")
        );
        assert_eq!(
            config_value(&values, "slack_bot_user_id").as_deref(),
            Some("user-bot-subject")
        );
        assert_eq!(
            config_value(&values, "slack_shared_subject_user_id").as_deref(),
            Some("user-shared")
        );
        assert_eq!(
            config_value(&values, "slack_oauth_client_id").as_deref(),
            Some("111.222")
        );

        // H.3 secret material re-put under the manifest handles.
        for (handle, expected) in [
            ("slack_bot_token", "xoxb-material"),
            ("slack_signing_secret", "signing-material"),
            ("slack_oauth_client_secret", "oauth-secret-material"),
        ] {
            let handle = SecretHandle::new(handle).expect("handle");
            let lease = fixture
                .secret_store
                .lease_once(&fixture.scope, &handle)
                .await
                .expect("folded secret leases");
            let material = fixture
                .secret_store
                .consume(&fixture.scope, lease.id)
                .await
                .expect("folded secret resolves");
            assert_eq!(secrecy::ExposeSecret::expose_secret(&material), expected);
        }

        // H.4 identity binding: prefix rewritten onto the durable
        // installation id and resolvable through the generic store.
        assert_eq!(
            fixture
                .inputs
                .identity_store
                .resolve_user_identity("slack", &format!("{DURABLE_INSTALLATION_ID}:U777"))
                .await
                .expect("identity resolves"),
            Some(UserId::new("user-alice").expect("user"))
        );

        // H.4 routes: managed -> allowed channels; explicit -> subject
        // routes; tombstoned routes dropped.
        assert_eq!(
            config_value(&values, "slack_allowed_channels").as_deref(),
            Some(r#"["C1"]"#)
        );
        assert_eq!(
            config_value(&values, "slack_subject_routes").as_deref(),
            Some(r#"{"C2":"user-ops"}"#)
        );

        // H.4 DM target: generic per-(extension, user) record.
        let dm = fixture
            .inputs
            .dm_targets
            .load("slack", &UserId::new("user-alice").expect("user"))
            .await
            .expect("dm target load")
            .expect("dm target folded");
        assert_eq!(dm.external_actor_id, "U777");
        assert_eq!(dm.target["space_id"], "T123");
        assert_eq!(dm.target["conversation_id"], "D42");

        // Idempotence: a second run changes nothing.
        let second = fold_retired_slack_channel_state(&fixture.inputs).await;
        assert!(!second.changed(), "second run must be a no-op: {second:?}");
        let values_after = fixture
            .installation_store
            .channel_config(&extension_id)
            .await
            .expect("channel config after second run");
        assert_eq!(values, values_after);
    }

    #[tokio::test]
    async fn fold_skips_malformed_records_and_operator_owned_values() {
        let fixture = fixture().await;
        seed_retired_state(&fixture).await;
        let extension_id = ExtensionId::new("slack").expect("extension id");

        // The operator already saved a team id through the configure
        // surface: the fold must not overwrite it.
        fixture
            .installation_store
            .set_channel_config(
                &extension_id,
                vec![("slack_team_id".to_string(), "T-OPERATOR".to_string())],
            )
            .await
            .expect("seed operator value");

        // A malformed identity record sits beside the valid one.
        fixture
            .filesystem
            .write_file(
                &VirtualPath::new(format!(
                    "/tenants/{TENANT}/shared/slack-personal-binding/identities/{}/{}.json",
                    seg("slack"),
                    seg("garbled")
                ))
                .expect("path"),
                b"{not-json",
            )
            .await
            .expect("seed malformed record");

        let report = fold_retired_slack_channel_state(&fixture.inputs).await;
        assert_eq!(report.identities, 1, "the valid record still folds");
        assert_eq!(
            report.config_values, 5,
            "the operator-owned key is not re-folded"
        );

        let values = fixture
            .installation_store
            .channel_config(&extension_id)
            .await
            .expect("channel config");
        assert_eq!(
            config_value(&values, "slack_team_id").as_deref(),
            Some("T-OPERATOR"),
            "operator-saved values win over retired setup values"
        );
    }

    /// H.4b: the sanctioned table maps exactly the retired lane's roots and
    /// nothing else — every other extension gets the generic root scheme.
    #[test]
    fn sanctioned_legacy_roots_cover_only_the_retired_lane() {
        let tenant = ironclaw_host_api::TenantId::new(TENANT).expect("tenant");
        let roots = sanctioned_legacy_channel_workflow_storage_roots(&tenant, "slack")
            .expect("slack keeps its legacy trees");
        assert_eq!(
            roots.idempotency.as_str(),
            format!("/tenants/{TENANT}/shared/slack-product-workflow/idempotency")
        );
        assert_eq!(
            roots.conversations.as_str(),
            format!("/tenants/{TENANT}/shared/slack-conversations")
        );
        assert!(
            sanctioned_legacy_channel_workflow_storage_roots(&tenant, "telegram").is_none(),
            "no other extension carries a legacy-root sanction"
        );
        assert!(sanctioned_legacy_channel_workflow_storage_roots(&tenant, "acmechat").is_none());
    }

    /// Value-shape parity: the generic managed-subject derivation reproduces
    /// the retired lane's `user:slack-channel:{sha16}` scheme for the slack
    /// extension id — the SAME prefix this fold classifies managed routes by.
    #[test]
    fn generic_managed_subject_derivation_matches_the_fold_classification_prefix() {
        let tenant = ironclaw_host_api::TenantId::new(TENANT).expect("tenant");
        let installation =
            ironclaw_product_adapters::AdapterInstallationId::new(RETIRED_INSTALLATION_ID)
                .expect("installation");
        let derived =
            crate::extension_host::channel_subject_routes::managed_channel_subject_user_id(
                FOLDED_EXTENSION_ID,
                &tenant,
                &installation,
                Some("T123"),
                "C1",
            )
            .expect("derivation");
        assert!(
            derived.as_str().starts_with(MANAGED_CHANNEL_SUBJECT_PREFIX),
            "generic derivation must keep the retired managed-subject shape: {derived}"
        );
    }

    /// Both-DB shape: the fold reads raw filesystem roots, so the libSQL
    /// root filesystem exercises the same paths the durable backends serve.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn fold_runs_against_the_libsql_root_filesystem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            libsql::Builder::new_local(dir.path().join("fold.db"))
                .build()
                .await
                .expect("open libsql database"),
        );
        let filesystem = ironclaw_filesystem::LibSqlRootFilesystem::new(db);
        filesystem.run_migrations().await.expect("migrations");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(filesystem);

        let installation_store = installed_store().await;
        let secret_store = Arc::new(InMemorySecretStore::new());
        let scope = scope();
        let inputs = RetiredChannelStateFoldInputs {
            filesystem: Arc::clone(&filesystem),
            installation_store: Arc::clone(&installation_store)
                as Arc<dyn ExtensionInstallationStore>,
            secret_store: secret_store.clone() as Arc<dyn SecretStore>,
            legacy_secret_scope: scope.clone(),
            channel_config_secret_scope: scope.clone(),
            identity_store: Arc::new(FilesystemChannelIdentityStore::new(
                Arc::clone(&filesystem),
                scope.tenant_id.clone(),
                scope.user_id.clone(),
            )),
            dm_targets: Arc::new(FilesystemChannelDmTargetStore::new(
                Arc::clone(&filesystem),
                scope.tenant_id.clone(),
                scope.user_id.clone(),
            )),
        };
        let fixture = Fixture {
            inputs,
            filesystem,
            installation_store,
            secret_store,
            scope,
        };
        seed_retired_state(&fixture).await;

        let report = fold_retired_slack_channel_state(&fixture.inputs).await;
        assert_eq!(report.identities, 1);
        assert_eq!(report.dm_targets, 1);
        assert_eq!(report.route_values, 2);
        assert_eq!(
            fixture
                .inputs
                .identity_store
                .resolve_user_identity("slack", &format!("{DURABLE_INSTALLATION_ID}:U777"))
                .await
                .expect("identity resolves"),
            Some(UserId::new("user-alice").expect("user"))
        );
        let second = fold_retired_slack_channel_state(&fixture.inputs).await;
        assert!(!second.changed(), "second run must be a no-op: {second:?}");
    }

    /// No-skip Postgres testcontainer provisioner (REL-3: a Postgres skip is a
    /// failure, not a pass). The fold is a `pub(crate)` `src/` function the
    /// integration harness cannot reach, so — per correction A's escape hatch —
    /// Postgres is provisioned inside the composition crate's own test module.
    #[cfg(feature = "postgres")]
    async fn start_postgres_pool_or_fail() -> (
        testcontainers_modules::testcontainers::ContainerAsync<
            testcontainers_modules::postgres::Postgres,
        >,
        deadpool_postgres::Pool,
    ) {
        use deadpool_postgres::tokio_postgres;
        use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};

        let container = testcontainers_modules::postgres::Postgres::default()
            .with_db_name("ironclaw_test")
            .with_user("postgres")
            .with_password("postgres")
            .with_tag("16-alpine")
            .start()
            .await
            .expect(
                "Postgres testcontainer must start (REL-3: a skip is a failure; \
                 locally run `colima start` or start Docker Desktop)",
            );
        let host = container.get_host().await.expect("resolve container host");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/ironclaw_test");
        let config: tokio_postgres::Config = database_url.parse().expect("database url must parse");
        let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        let pool = deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .expect("postgres pool must build");
        (container, pool)
    }

    /// MIG-7: the same fold on a real PostgreSQL root filesystem — the both-DB
    /// leg (REL-3). The fold reads only `Arc<dyn RootFilesystem>` roots, so it
    /// is backend-agnostic; this proves it on real Postgres, not just libSQL.
    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn fold_runs_against_the_postgres_root_filesystem() {
        let (_container, pool) = start_postgres_pool_or_fail().await;
        let filesystem = ironclaw_filesystem::PostgresRootFilesystem::new(pool);
        filesystem.run_migrations().await.expect("migrations");
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(filesystem);

        let installation_store = installed_store().await;
        let secret_store = Arc::new(InMemorySecretStore::new());
        let scope = scope();
        let inputs = RetiredChannelStateFoldInputs {
            filesystem: Arc::clone(&filesystem),
            installation_store: Arc::clone(&installation_store)
                as Arc<dyn ExtensionInstallationStore>,
            secret_store: secret_store.clone() as Arc<dyn SecretStore>,
            legacy_secret_scope: scope.clone(),
            channel_config_secret_scope: scope.clone(),
            identity_store: Arc::new(FilesystemChannelIdentityStore::new(
                Arc::clone(&filesystem),
                scope.tenant_id.clone(),
                scope.user_id.clone(),
            )),
            dm_targets: Arc::new(FilesystemChannelDmTargetStore::new(
                Arc::clone(&filesystem),
                scope.tenant_id.clone(),
                scope.user_id.clone(),
            )),
        };
        let fixture = Fixture {
            inputs,
            filesystem,
            installation_store,
            secret_store,
            scope,
        };
        seed_retired_state(&fixture).await;

        let report = fold_retired_slack_channel_state(&fixture.inputs).await;
        assert_eq!(report.identities, 1);
        assert_eq!(report.dm_targets, 1);
        assert_eq!(report.route_values, 2);
        assert_eq!(
            fixture
                .inputs
                .identity_store
                .resolve_user_identity("slack", &format!("{DURABLE_INSTALLATION_ID}:U777"))
                .await
                .expect("identity resolves"),
            Some(UserId::new("user-alice").expect("user"))
        );
        let second = fold_retired_slack_channel_state(&fixture.inputs).await;
        assert!(!second.changed(), "second run must be a no-op: {second:?}");
    }
}
