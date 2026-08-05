//! Lossless startup import for channel state written by `1.0.0-rc.1`.
//!
//! Released Slack and Telegram lanes owned provider-specific roots. This
//! module is the sole compatibility reader for those roots and folds their
//! durable setup, identity, route, DM-target, and pairing-disposition state
//! into the generic extension-host stores. Source rows are never deleted.

#![allow(
    dead_code,
    reason = "frozen rc1 wire readers retain every released field for fail-closed validation"
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use ironclaw_extensions::{AdminConfigurationGroupId, ExtensionInstallationStorePort};
use ironclaw_filesystem::{
    FileType, FilesystemError, Filter, Page, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    approval::sha256_digest_token,
    ids::{AgentId, InvocationId, ProjectId, SecretHandle, TenantId, UserId},
    path::VirtualPath,
    product_adapter::AdapterInstallationId,
    resource::{ResourceScope, SYSTEM_RESERVED_ID, resource_scope_path_segment},
    user_identity::{
        RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
        RebornUserIdentityBindingStore, RebornUserIdentityLookup,
    },
};
use ironclaw_secrets::{SecretMaterial, SecretStorePort};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::channel_pairing::ChannelPairingCode;
use crate::{
    AdminConfigurationService, AdminConfigurationSubmittedValue, ChannelDmTargetRecord,
    FilesystemChannelDmTargetStore, FilesystemChannelIdentityStore, dm_target_payload,
};

const SLACK: &str = "slack";
const TELEGRAM: &str = "telegram";
const SLACK_GROUP: &str = "extension.slack";
const TELEGRAM_GROUP: &str = "extension.telegram";
const MANAGED_SLACK_SUBJECT_PREFIX: &str = "user:slack-channel:";
const MAX_RC1_CHANNEL_TENANTS: usize = 100_000;
const MAX_RC1_CHANNEL_ROWS_PER_ROOT: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rc1ChannelRootMigrationSpec {
    pub provider_key: &'static str,
}

pub fn rc1_channel_root_migration_specs() -> [Rc1ChannelRootMigrationSpec; 2] {
    [
        Rc1ChannelRootMigrationSpec {
            provider_key: SLACK,
        },
        Rc1ChannelRootMigrationSpec {
            provider_key: TELEGRAM,
        },
    ]
}

/// Dependencies for one tenant's startup import. Composition discovers the
/// tenant/operator scopes; this owner module understands the persisted wire.
pub struct Rc1ChannelStateMigrationInputs {
    pub filesystem: Arc<dyn RootFilesystem>,
    pub installation_store: Arc<dyn ExtensionInstallationStorePort>,
    pub secret_store: Arc<dyn SecretStorePort>,
    pub admin_configuration:
        Arc<AdminConfigurationService<dyn RootFilesystem, dyn SecretStorePort>>,
    pub oauth_channel_secret_scope: ResourceScope,
    pub proof_code_channel_secret_scope: ResourceScope,
    pub admin_scope: ResourceScope,
    pub identity_store: Arc<FilesystemChannelIdentityStore>,
    pub dm_targets: Arc<FilesystemChannelDmTargetStore>,
}

/// Redacted, count-only result. Pairing challenges and incomplete completion
/// notices have no safe cross-version replay contract and are explicitly
/// expired rather than copied into the new pairing snapshot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Rc1ChannelStateMigrationReport {
    pub configuration_values: usize,
    pub identities: usize,
    pub route_values: usize,
    pub dm_targets: usize,
    pub unbound_dm_targets_skipped: usize,
    pub oauth_channel_active_connections_superseded: usize,
    pub oauth_channel_stale_connections_expired: usize,
    pub oauth_channel_disconnected_connections_superseded: usize,
    pub oauth_channel_connections_unchanged: usize,
    pub proof_code_pairing_challenges_expired: usize,
    pub proof_code_pending_completions_expired: usize,
    pub proof_code_pairing_rows_unchanged: usize,
    /// One redacted count-only entry per discovered tenant scope. Tenant and
    /// actor identifiers are deliberately omitted from the persisted report.
    pub scopes: Vec<Rc1ChannelStateScopeMigrationReport>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Rc1ChannelStateScopeMigrationReport {
    pub migrated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflicting: usize,
    pub failed: usize,
}

impl Rc1ChannelStateMigrationReport {
    pub fn merge(&mut self, other: Self) {
        self.configuration_values = self
            .configuration_values
            .saturating_add(other.configuration_values);
        self.identities = self.identities.saturating_add(other.identities);
        self.route_values = self.route_values.saturating_add(other.route_values);
        self.dm_targets = self.dm_targets.saturating_add(other.dm_targets);
        self.unbound_dm_targets_skipped = self
            .unbound_dm_targets_skipped
            .saturating_add(other.unbound_dm_targets_skipped);
        self.oauth_channel_active_connections_superseded = self
            .oauth_channel_active_connections_superseded
            .saturating_add(other.oauth_channel_active_connections_superseded);
        self.oauth_channel_stale_connections_expired = self
            .oauth_channel_stale_connections_expired
            .saturating_add(other.oauth_channel_stale_connections_expired);
        self.oauth_channel_disconnected_connections_superseded = self
            .oauth_channel_disconnected_connections_superseded
            .saturating_add(other.oauth_channel_disconnected_connections_superseded);
        self.oauth_channel_connections_unchanged = self
            .oauth_channel_connections_unchanged
            .saturating_add(other.oauth_channel_connections_unchanged);
        self.proof_code_pairing_challenges_expired = self
            .proof_code_pairing_challenges_expired
            .saturating_add(other.proof_code_pairing_challenges_expired);
        self.proof_code_pending_completions_expired = self
            .proof_code_pending_completions_expired
            .saturating_add(other.proof_code_pending_completions_expired);
        self.proof_code_pairing_rows_unchanged = self
            .proof_code_pairing_rows_unchanged
            .saturating_add(other.proof_code_pairing_rows_unchanged);
        self.scopes.extend(other.scopes);
    }
}

/// One durable tenant and the released secret-owner scopes needed to import
/// its provider setup without guessing the configured default owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rc1ChannelMigrationScope {
    pub admin_scope: ResourceScope,
    pub oauth_channel_secret_scope: ResourceScope,
    pub proof_code_channel_secret_scope: ResourceScope,
}

/// Discover all released channel-state tenants and the exact owner scopes of
/// their setup secrets from durable paths. This is intentionally independent
/// of the configured default owner: hosted rc1 could persist multiple tenants
/// in one backend.
pub async fn discover_rc1_channel_migration_scopes(
    filesystem: Arc<dyn RootFilesystem>,
) -> Result<Vec<Rc1ChannelMigrationScope>, Rc1ChannelStateMigrationError> {
    let tenants_root = VirtualPath::new("/tenants").map_err(log_malformed)?;
    let tenant_entries = filesystem
        .list_dir_bounded(&tenants_root, MAX_RC1_CHANNEL_TENANTS.saturating_add(1))
        .await
        .map_err(log_unavailable)?;
    if tenant_entries.len() > MAX_RC1_CHANNEL_TENANTS {
        return Err(Rc1ChannelStateMigrationError::Unavailable);
    }
    let mut scopes = Vec::new();
    for tenant_entry in tenant_entries {
        if tenant_entry.file_type != FileType::Directory || tenant_entry.name == "__system__" {
            continue;
        }
        let tenant_segment = tenant_entry.name;
        let shared = format!("/tenants/{tenant_segment}/shared");
        let mut rows = query_all(&filesystem, &shared).await?;
        if !rows
            .iter()
            .any(|row| is_rc1_channel_state_path(row.path.as_str()))
        {
            continue;
        }
        rows.extend(query_all(&filesystem, &format!("/tenants/{tenant_segment}/users")).await?);
        let tenant = TenantId::new(&tenant_segment).map_err(log_malformed)?;
        let slack_setup = find_row(&rows, &format!("{shared}/slack-setup/installation.json"))
            .map(parse::<Rc1SlackSetup>)
            .transpose()?;
        let telegram_setup = find_row(&rows, &format!("{shared}/telegram-setup/installation.json"))
            .map(parse::<Rc1TelegramSetup>)
            .transpose()?;
        let slack_handles = slack_setup
            .as_ref()
            .map(|setup| {
                let mut handles = vec![
                    setup.bot_token_handle.clone(),
                    setup.signing_secret_handle.clone(),
                ];
                if let Some(handle) = &setup.oauth_client_secret_handle {
                    handles.push(handle.clone());
                }
                handles
            })
            .unwrap_or_default();
        let telegram_handles = match telegram_setup.as_ref() {
            Some(Rc1TelegramSetup::Active(setup)) => vec![
                setup.bot_token_handle.clone(),
                setup.webhook_secret_handle.clone(),
            ],
            Some(Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::Clearing { setup })) => {
                vec![
                    setup.bot_token_handle.clone(),
                    setup.webhook_secret_handle.clone(),
                ]
            }
            Some(Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::RollingBack {
                saved,
                ..
            })) => vec![
                saved.bot_token_handle.clone(),
                saved.webhook_secret_handle.clone(),
            ],
            Some(Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::Cleared { .. })) | None => {
                Vec::new()
            }
        };
        let admin_scope = ResourceScope {
            tenant_id: tenant.clone(),
            user_id: UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let oauth_channel_secret_scope =
            discover_secret_scope(&rows, &tenant_segment, &tenant, &slack_handles)?
                .unwrap_or_else(|| admin_scope.clone());
        let proof_code_channel_secret_scope =
            discover_secret_scope(&rows, &tenant_segment, &tenant, &telegram_handles)?
                .unwrap_or_else(|| admin_scope.clone());
        scopes.push(Rc1ChannelMigrationScope {
            admin_scope,
            oauth_channel_secret_scope,
            proof_code_channel_secret_scope,
        });
    }
    Ok(scopes)
}

pub fn is_rc1_channel_state_path(path: &str) -> bool {
    [
        "/slack-setup/",
        "/slack-personal-binding/",
        "/slack-channel-routes/",
        "/slack-conversations/",
        "/slack-product-workflow/",
        "/telegram-setup/",
        "/telegram-binding/",
        "/telegram-dm-targets/",
        "/telegram-pairing/",
        "/telegram-conversations/",
        "/telegram-product-workflow/",
    ]
    .iter()
    .any(|segment| path.contains(segment))
}

fn find_row<'a>(rows: &'a [VersionedEntry], path: &str) -> Option<&'a VersionedEntry> {
    rows.iter().find(|row| row.path.as_str() == path)
}

fn discover_secret_scope(
    rows: &[VersionedEntry],
    tenant_segment: &str,
    tenant: &TenantId,
    handles: &[SecretHandle],
) -> Result<Option<ResourceScope>, Rc1ChannelStateMigrationError> {
    if handles.is_empty() {
        return Ok(None);
    }
    let mut candidates: BTreeMap<String, (ResourceScope, BTreeSet<String>)> = BTreeMap::new();
    for row in rows {
        for handle in handles {
            let Some(scope) =
                secret_scope_from_path(row.path.as_str(), tenant_segment, tenant, handle.as_str())?
            else {
                continue;
            };
            let key = format!(
                "{}\0{}\0{}",
                scope.user_id.as_str(),
                scope.agent_id.as_ref().map_or("", AgentId::as_str),
                scope.project_id.as_ref().map_or("", ProjectId::as_str),
            );
            candidates
                .entry(key)
                .or_insert_with(|| (scope, BTreeSet::new()))
                .1
                .insert(handle.as_str().to_string());
        }
    }
    let required = handles
        .iter()
        .map(|handle| handle.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let mut matches = candidates
        .into_values()
        .filter_map(|(scope, found)| (found == required).then_some(scope));
    let result = matches.next();
    if result.is_none() || matches.next().is_some() {
        return Err(Rc1ChannelStateMigrationError::Unavailable);
    }
    Ok(result)
}

fn secret_scope_from_path(
    path: &str,
    tenant_segment: &str,
    tenant: &TenantId,
    handle: &str,
) -> Result<Option<ResourceScope>, Rc1ChannelStateMigrationError> {
    let prefix = format!("/tenants/{tenant_segment}/users/");
    let Some(rest) = path.strip_prefix(&prefix) else {
        return Ok(None);
    };
    let parts = rest.split('/').collect::<Vec<_>>();
    if parts.len() < 4 || parts[1] != "secrets" {
        return Ok(None);
    }
    let expected_leaf = format!("{handle}.json");
    if parts.last().copied() != Some(expected_leaf.as_str())
        || parts.get(parts.len().saturating_sub(2)).copied() != Some("secrets")
    {
        return Ok(None);
    }
    let user_id = if parts[0] == "__system__" {
        UserId::from_trusted(SYSTEM_RESERVED_ID.to_string())
    } else {
        UserId::new(parts[0]).map_err(log_malformed)?
    };
    let mut cursor = 2usize;
    let mut agent_id = None;
    let mut project_id = None;
    if parts.get(cursor).copied() == Some("agents") {
        let value = parts
            .get(cursor.saturating_add(1))
            .ok_or(Rc1ChannelStateMigrationError::Malformed)?;
        agent_id = Some(AgentId::new(*value).map_err(log_malformed)?);
        cursor = cursor.saturating_add(2);
    }
    if parts.get(cursor).copied() == Some("projects") {
        let value = parts
            .get(cursor.saturating_add(1))
            .ok_or(Rc1ChannelStateMigrationError::Malformed)?;
        project_id = Some(ProjectId::new(*value).map_err(log_malformed)?);
        cursor = cursor.saturating_add(2);
    }
    if parts.get(cursor).copied() != Some("secrets") || cursor + 2 != parts.len() {
        return Ok(None);
    }
    Ok(Some(ResourceScope {
        tenant_id: tenant.clone(),
        user_id,
        agent_id,
        project_id,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }))
}

#[derive(Debug, thiserror::Error)]
pub enum Rc1ChannelStateMigrationError {
    #[error("rc1 channel state is malformed")]
    Malformed,
    #[error("rc1 channel state conflicts with current state")]
    Conflict,
    #[error("rc1 channel state migration is unavailable")]
    Unavailable,
    #[error("rc1 channel route state is too large to import ({records} source records)")]
    SourceTooLarge { records: usize },
    #[error("rc1 channel state requires an installed target extension")]
    MissingInstallation,
    #[error("rc1 channel setup was interrupted and requires operator recovery")]
    InterruptedSetup,
}

fn log_malformed(error: impl std::fmt::Display) -> Rc1ChannelStateMigrationError {
    tracing::error!(%error, "rc1 channel migration record validation failed");
    Rc1ChannelStateMigrationError::Malformed
}

fn log_unavailable(error: impl std::fmt::Display) -> Rc1ChannelStateMigrationError {
    tracing::error!(%error, "rc1 channel migration dependency failed");
    Rc1ChannelStateMigrationError::Unavailable
}

fn log_conflict(error: impl std::fmt::Display) -> Rc1ChannelStateMigrationError {
    tracing::error!(%error, "rc1 channel migration authority write conflicted");
    Rc1ChannelStateMigrationError::Conflict
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1SlackSetup {
    installation_id: String,
    team_id: String,
    api_app_id: String,
    user_id: String,
    #[serde(default)]
    shared_subject_user_id: Option<String>,
    bot_token_handle: SecretHandle,
    signing_secret_handle: SecretHandle,
    #[serde(default)]
    oauth_client_id: Option<String>,
    #[serde(default)]
    oauth_client_secret_handle: Option<SecretHandle>,
    revision: u64,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Rc1SlackIdentityState {
    Active,
    Disconnected,
}

fn active_slack_identity() -> Rc1SlackIdentityState {
    Rc1SlackIdentityState::Active
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1SlackIdentity {
    provider: String,
    provider_user_id: String,
    user_id: String,
    #[serde(default)]
    epoch: Option<String>,
    #[serde(default = "active_slack_identity")]
    state: Rc1SlackIdentityState,
    #[serde(default)]
    disconnected_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1SlackRoute {
    tenant_id: String,
    installation_id: String,
    team_id: String,
    channel_id: String,
    subject_user_id: String,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1SlackDmTarget {
    tenant_id: String,
    installation_id: String,
    team_id: String,
    user_id: String,
    slack_user_id: String,
    dm_channel_id: String,
    #[serde(default)]
    epoch: Option<String>,
    #[serde(default)]
    deleted_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Rc1SlackConnectionState {
    Connecting,
    Active,
    Disconnecting,
    Disconnected,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "epoch", rename_all = "snake_case")]
enum Rc1SlackDisconnectCleanup {
    AllOwned,
    Epoch(ironclaw_auth::AuthFlowId),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1SlackConnection {
    tenant_id: String,
    user_id: String,
    installation_id: String,
    epoch: ironclaw_auth::AuthFlowId,
    state: Rc1SlackConnectionState,
    #[serde(default)]
    disconnect_cleanup: Option<Rc1SlackDisconnectCleanup>,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Rc1SlackConnectionDispositionMarker {
    schema: String,
    source_digest: String,
    active_superseded: usize,
    stale_expired: usize,
    disconnected_superseded: usize,
    source_rows: usize,
}

#[derive(Debug)]
struct Rc1SlackConnectionDisposition {
    marker: Rc1SlackConnectionDispositionMarker,
    already_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Rc1TelegramPairingDispositionMarker {
    schema: String,
    source_digest: String,
    challenges_expired: usize,
    pending_completions_expired: usize,
    source_rows: usize,
}

#[derive(Debug)]
struct Rc1TelegramPairingDisposition {
    marker: Rc1TelegramPairingDispositionMarker,
    already_complete: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1TelegramSetupActive {
    bot_id: i64,
    bot_username: String,
    webhook_url: String,
    bot_token_handle: SecretHandle,
    webhook_secret_handle: SecretHandle,
    revision: u64,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "lifecycle", rename_all = "snake_case", deny_unknown_fields)]
enum Rc1TelegramSetupLifecycle {
    Clearing {
        setup: Rc1TelegramSetupActive,
    },
    RollingBack {
        saved: Rc1TelegramSetupActive,
        previous: Option<Rc1TelegramSetupActive>,
        #[serde(default)]
        provider_compensated: bool,
    },
    Cleared {
        cleared_revision: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Rc1TelegramSetup {
    Active(Rc1TelegramSetupActive),
    Lifecycle(Rc1TelegramSetupLifecycle),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1TelegramBinding {
    provider_user_id: String,
    user_id: String,
    epoch: String,
    #[serde(default = "default_true")]
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1TelegramDmTarget {
    user_id: UserId,
    chat_id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1TelegramPairingRecord {
    code: ChannelPairingCode,
    tenant_id: TenantId,
    user_id: UserId,
    installation_id: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1TelegramPairingPointer {
    code: ChannelPairingCode,
    #[serde(default = "default_true")]
    active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1TelegramPairingCompletion {
    installation_id: String,
    user_id: UserId,
    chat_id: i64,
    completed: bool,
}

fn default_true() -> bool {
    true
}

/// Migrate one tenant before channel listeners or extension writers start.
pub async fn migrate_rc1_channel_state(
    inputs: &Rc1ChannelStateMigrationInputs,
) -> Result<Rc1ChannelStateMigrationReport, Rc1ChannelStateMigrationError> {
    if inputs.oauth_channel_secret_scope.tenant_id != inputs.admin_scope.tenant_id
        || inputs.proof_code_channel_secret_scope.tenant_id != inputs.admin_scope.tenant_id
    {
        return Err(Rc1ChannelStateMigrationError::Malformed);
    }
    let tenant = resource_scope_path_segment(inputs.admin_scope.tenant_id.as_str());
    let shared = format!("/tenants/{tenant}/shared");
    let installation_ids = target_installation_ids(inputs).await?;
    let mut report = Rc1ChannelStateMigrationReport::default();

    let slack_setup =
        read_optional::<Rc1SlackSetup>(inputs, &format!("{shared}/slack-setup/installation.json"))
            .await?;
    let telegram_setup = read_optional::<Rc1TelegramSetup>(
        inputs,
        &format!("{shared}/telegram-setup/installation.json"),
    )
    .await?;
    let telegram_active = match telegram_setup.as_ref() {
        Some(Rc1TelegramSetup::Active(setup)) => Some(setup),
        Some(Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::Cleared { .. })) | None => None,
        Some(Rc1TelegramSetup::Lifecycle(
            Rc1TelegramSetupLifecycle::Clearing { .. }
            | Rc1TelegramSetupLifecycle::RollingBack { .. },
        )) => return Err(Rc1ChannelStateMigrationError::InterruptedSetup),
    };

    // Parse every released record and determine the non-replayable pairing
    // disposition before the first normalized write. A malformed old row must
    // never leave a partially imported tenant behind.
    validate_source_rows(inputs, &shared).await?;
    let slack_connections =
        inspect_slack_connection_disposition(&inputs.filesystem, &inputs.admin_scope, &shared)
            .await?;
    let telegram_pairing =
        inspect_telegram_pairing_disposition(&inputs.filesystem, &inputs.admin_scope, &shared)
            .await?;

    if let Some(setup) = slack_setup.as_ref() {
        report.configuration_values += migrate_slack_setup(inputs, setup).await?;
    }
    report.identities += migrate_slack_identities(
        inputs,
        &shared,
        slack_setup.as_ref(),
        installation_ids.get(SLACK),
    )
    .await?;
    report.route_values += migrate_slack_routes(inputs, &shared, slack_setup.as_ref()).await?;
    report.dm_targets += migrate_slack_dm_targets(inputs, &shared, slack_setup.as_ref()).await?;

    if let Some(setup) = telegram_active {
        report.configuration_values += migrate_telegram_setup(inputs, setup).await?;
    }
    let telegram_bindings = migrate_telegram_identities(
        inputs,
        &shared,
        telegram_active,
        installation_ids.get(TELEGRAM),
    )
    .await?;
    report.identities += telegram_bindings.changed;
    let (telegram_dm_targets, unbound_dm_targets_skipped) =
        migrate_telegram_dm_targets(inputs, &shared, telegram_active, &telegram_bindings.active)
            .await?;
    report.dm_targets += telegram_dm_targets;
    report.unbound_dm_targets_skipped += unbound_dm_targets_skipped;
    if slack_connections.already_complete {
        report.oauth_channel_connections_unchanged = slack_connections.marker.source_rows;
    } else {
        report.oauth_channel_active_connections_superseded =
            slack_connections.marker.active_superseded;
        report.oauth_channel_stale_connections_expired = slack_connections.marker.stale_expired;
        report.oauth_channel_disconnected_connections_superseded =
            slack_connections.marker.disconnected_superseded;
    }
    if telegram_pairing.already_complete {
        report.proof_code_pairing_rows_unchanged = telegram_pairing.marker.source_rows;
    } else {
        report.proof_code_pairing_challenges_expired = telegram_pairing.marker.challenges_expired;
        report.proof_code_pending_completions_expired =
            telegram_pairing.marker.pending_completions_expired;
    }
    commit_disposition_marker(
        &inputs.filesystem,
        &format!("{shared}/channel-extensions/slack/migrations/rc1-connections-v1.complete.json"),
        &slack_connections.marker,
        slack_connections.already_complete,
    )
    .await?;
    commit_disposition_marker(
        &inputs.filesystem,
        &format!("{shared}/channel-extensions/telegram/migrations/rc1-pairing-v1.complete.json"),
        &telegram_pairing.marker,
        telegram_pairing.already_complete,
    )
    .await?;
    report.scopes.push(Rc1ChannelStateScopeMigrationReport {
        migrated: report
            .configuration_values
            .saturating_add(report.identities)
            .saturating_add(report.route_values)
            .saturating_add(report.dm_targets),
        unchanged: report
            .oauth_channel_connections_unchanged
            .saturating_add(report.proof_code_pairing_rows_unchanged),
        skipped: report.unbound_dm_targets_skipped.saturating_add(
            report
                .oauth_channel_active_connections_superseded
                .saturating_add(report.oauth_channel_stale_connections_expired)
                .saturating_add(report.oauth_channel_disconnected_connections_superseded)
                .saturating_add(report.proof_code_pairing_challenges_expired)
                .saturating_add(report.proof_code_pending_completions_expired),
        ),
        conflicting: 0,
        failed: 0,
    });
    Ok(report)
}

async fn target_installation_ids(
    inputs: &Rc1ChannelStateMigrationInputs,
) -> Result<BTreeMap<String, String>, Rc1ChannelStateMigrationError> {
    let installations = inputs
        .installation_store
        .list_installations()
        .await
        .map_err(log_unavailable)?;
    let mut ids = BTreeMap::new();
    for installation in installations {
        let extension = installation.extension_id().as_str();
        if extension == SLACK || extension == TELEGRAM {
            AdapterInstallationId::new(installation.installation_id().as_str())
                .map_err(log_malformed)?;
            match ids.get(extension) {
                Some(existing) if existing != installation.installation_id().as_str() => {
                    return Err(Rc1ChannelStateMigrationError::Conflict);
                }
                Some(_) => {}
                None => {
                    ids.insert(
                        extension.to_string(),
                        installation.installation_id().as_str().to_string(),
                    );
                }
            }
        }
    }
    Ok(ids)
}

async fn migrate_slack_setup(
    inputs: &Rc1ChannelStateMigrationInputs,
    setup: &Rc1SlackSetup,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let mut values = vec![
        submitted("slack_installation_id", setup.installation_id.clone())?,
        submitted("slack_team_id", setup.team_id.clone())?,
        submitted("slack_api_app_id", setup.api_app_id.clone())?,
        submitted("slack_bot_user_id", setup.user_id.clone())?,
        submitted_secret(
            inputs,
            &inputs.oauth_channel_secret_scope,
            "slack_bot_token",
            &setup.bot_token_handle,
        )
        .await?,
        submitted_secret(
            inputs,
            &inputs.oauth_channel_secret_scope,
            "slack_signing_secret",
            &setup.signing_secret_handle,
        )
        .await?,
    ];
    if let Some(subject) = &setup.shared_subject_user_id {
        values.push(submitted("slack_shared_subject_user_id", subject.clone())?);
    }
    if let Some(client_id) = &setup.oauth_client_id {
        values.push(submitted("slack_oauth_client_id", client_id.clone())?);
    }
    if let Some(client_secret) = &setup.oauth_client_secret_handle {
        values.push(
            submitted_secret(
                inputs,
                &inputs.oauth_channel_secret_scope,
                "slack_oauth_client_secret",
                client_secret,
            )
            .await?,
        );
    }
    import_admin(inputs, SLACK_GROUP, "rc1-slack-setup-v1", values).await
}

async fn migrate_telegram_setup(
    inputs: &Rc1ChannelStateMigrationInputs,
    setup: &Rc1TelegramSetupActive,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    import_admin(
        inputs,
        TELEGRAM_GROUP,
        "rc1-telegram-setup-v1",
        vec![
            submitted("bot_username", setup.bot_username.clone())?,
            submitted("telegram_webhook_url", setup.webhook_url.clone())?,
            submitted_secret(
                inputs,
                &inputs.proof_code_channel_secret_scope,
                "telegram_bot_token",
                &setup.bot_token_handle,
            )
            .await?,
            submitted_secret(
                inputs,
                &inputs.proof_code_channel_secret_scope,
                "telegram_webhook_secret",
                &setup.webhook_secret_handle,
            )
            .await?,
        ],
    )
    .await
}

async fn import_admin(
    inputs: &Rc1ChannelStateMigrationInputs,
    group: &str,
    migration_id: &str,
    values: Vec<AdminConfigurationSubmittedValue>,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let group = AdminConfigurationGroupId::new(group).map_err(log_malformed)?;
    inputs
        .admin_configuration
        .import_legacy_values(&inputs.admin_scope, &group, migration_id, values)
        .await
        .map_err(|error| match error {
            crate::AdminConfigurationServiceError::IdempotencyConflict => {
                Rc1ChannelStateMigrationError::Conflict
            }
            error => log_unavailable(error),
        })
}

fn submitted(
    handle: &str,
    value: String,
) -> Result<AdminConfigurationSubmittedValue, Rc1ChannelStateMigrationError> {
    Ok(AdminConfigurationSubmittedValue {
        handle: SecretHandle::new(handle).map_err(log_malformed)?,
        value: SecretMaterial::from(value),
    })
}

async fn submitted_secret(
    inputs: &Rc1ChannelStateMigrationInputs,
    source_scope: &ResourceScope,
    target: &str,
    source: &SecretHandle,
) -> Result<AdminConfigurationSubmittedValue, Rc1ChannelStateMigrationError> {
    let lease = inputs
        .secret_store
        .lease_once(source_scope, source)
        .await
        .map_err(log_unavailable)?;
    let material = inputs
        .secret_store
        .consume(source_scope, lease.id)
        .await
        .map_err(log_unavailable)?;
    submitted(target, material.expose_secret().to_string())
}

async fn migrate_slack_identities(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1SlackSetup>,
    target_installation: Option<&String>,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-personal-binding/identities"),
    )
    .await?;
    let mut changed = 0;
    for row in rows {
        let record: Rc1SlackIdentity = parse(&row)?;
        if matches!(record.state, Rc1SlackIdentityState::Disconnected) {
            continue;
        }
        if let Some(epoch) = &record.epoch {
            ironclaw_conversations::ExternalActorBindingEpoch::new(epoch.clone())
                .map_err(log_malformed)?;
        }
        if record.disconnected_at.is_some() {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        let provider_user_id = rewrite_installation_prefix(
            &record.provider_user_id,
            setup.map(|setup| setup.installation_id.as_str()),
            target_installation.map(String::as_str),
        )?;
        changed +=
            bind_identity(inputs, &record.provider, provider_user_id, &record.user_id).await?;
    }
    Ok(changed)
}

async fn migrate_slack_routes(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1SlackSetup>,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-channel-routes"),
    )
    .await?;
    let route_count = rows.len();
    let mut allowed = Vec::new();
    let mut explicit = BTreeMap::new();
    for row in rows {
        let route: Rc1SlackRoute = parse(&row)?;
        if route.tenant_id != inputs.admin_scope.tenant_id.as_str() {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        if route.deleted_at.is_some() {
            continue;
        }
        let Some(setup) = setup else {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        };
        if route.installation_id != setup.installation_id || route.team_id != setup.team_id {
            return Err(Rc1ChannelStateMigrationError::Conflict);
        }
        if route
            .subject_user_id
            .starts_with(MANAGED_SLACK_SUBJECT_PREFIX)
        {
            allowed.push(route.channel_id);
        } else if let Some(previous) =
            explicit.insert(route.channel_id, route.subject_user_id.clone())
            && previous != route.subject_user_id
        {
            return Err(Rc1ChannelStateMigrationError::Conflict);
        }
    }
    allowed.sort();
    allowed.dedup();
    let mut values = Vec::new();
    if !allowed.is_empty() {
        let allowed = serde_json::to_string(&allowed).map_err(log_malformed)?;
        if allowed.len() > crate::admin_configuration_service::MAX_VALUE_BYTES {
            return Err(Rc1ChannelStateMigrationError::SourceTooLarge {
                records: route_count,
            });
        }
        values.push(submitted("slack_allowed_channels", allowed)?);
    }
    if !explicit.is_empty() {
        let explicit = serde_json::to_string(&explicit).map_err(log_malformed)?;
        if explicit.len() > crate::admin_configuration_service::MAX_VALUE_BYTES {
            return Err(Rc1ChannelStateMigrationError::SourceTooLarge {
                records: route_count,
            });
        }
        values.push(submitted("slack_subject_routes", explicit)?);
    }
    if values.is_empty() {
        return Ok(0);
    }
    import_admin(inputs, SLACK_GROUP, "rc1-slack-routes-v1", values).await
}

async fn migrate_slack_dm_targets(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1SlackSetup>,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-personal-binding/dm-targets"),
    )
    .await?;
    let mut changed = 0;
    for row in rows {
        let target: Rc1SlackDmTarget = parse(&row)?;
        if target.tenant_id != inputs.admin_scope.tenant_id.as_str() {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        if target.deleted_at.is_some() {
            continue;
        }
        let Some(setup) = setup else {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        };
        if target.installation_id != setup.installation_id || target.team_id != setup.team_id {
            return Err(Rc1ChannelStateMigrationError::Conflict);
        }
        let user = UserId::new(target.user_id).map_err(log_malformed)?;
        let payload = dm_target_payload(Some(&target.team_id), &target.dm_channel_id);
        changed += upsert_dm_target(inputs, SLACK, &user, target.slack_user_id, payload).await?;
    }
    Ok(changed)
}

struct TelegramBindingImport {
    changed: usize,
    active: Vec<Rc1TelegramBinding>,
}

async fn migrate_telegram_identities(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1TelegramSetupActive>,
    target_installation: Option<&String>,
) -> Result<TelegramBindingImport, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        &inputs.filesystem,
        &format!("{shared}/telegram-binding/identities"),
    )
    .await?;
    let old_installation = setup.map(|setup| format!("tg-bot-{}", setup.bot_id));
    let mut changed = 0;
    let mut active = Vec::new();
    for row in rows {
        let record: Rc1TelegramBinding = parse(&row)?;
        if !record.active {
            continue;
        }
        ironclaw_conversations::ExternalActorBindingEpoch::new(record.epoch.clone())
            .map_err(log_malformed)?;
        let provider_user_id = rewrite_installation_prefix(
            &record.provider_user_id,
            old_installation.as_deref(),
            target_installation.map(String::as_str),
        )?;
        changed += bind_identity(inputs, TELEGRAM, provider_user_id, &record.user_id).await?;
        active.push(record);
    }
    Ok(TelegramBindingImport { changed, active })
}

async fn migrate_telegram_dm_targets(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1TelegramSetupActive>,
    bindings: &[Rc1TelegramBinding],
) -> Result<(usize, usize), Rc1ChannelStateMigrationError> {
    let rows = query_all(&inputs.filesystem, &format!("{shared}/telegram-dm-targets")).await?;
    let old_installation = setup.map(|setup| format!("tg-bot-{}", setup.bot_id));
    let mut changed = 0;
    let mut skipped = 0usize;
    for row in rows {
        let target: Rc1TelegramDmTarget = parse(&row)?;
        let candidates = bindings
            .iter()
            .filter(|binding| binding.user_id == target.user_id.as_str())
            .filter_map(|binding| {
                let (installation, actor) = binding.provider_user_id.split_once(':')?;
                old_installation
                    .as_deref()
                    .is_none_or(|expected| expected == installation)
                    .then_some(actor)
            })
            .collect::<Vec<_>>();
        let actor = match candidates.as_slice() {
            [actor] => *actor,
            [] => {
                skipped = skipped.saturating_add(1);
                continue;
            }
            _ => return Err(Rc1ChannelStateMigrationError::Conflict),
        };
        changed += upsert_dm_target(
            inputs,
            TELEGRAM,
            &target.user_id,
            actor.to_string(),
            dm_target_payload(None, &target.chat_id.to_string()),
        )
        .await?;
    }
    Ok((changed, skipped))
}

async fn bind_identity(
    inputs: &Rc1ChannelStateMigrationInputs,
    provider: &str,
    provider_user_id: String,
    user: &str,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let user_id = UserId::new(user).map_err(log_malformed)?;
    match inputs
        .identity_store
        .resolve_user_identity(provider, &provider_user_id)
        .await
        .map_err(log_unavailable)?
    {
        Some(current) if current == user_id => return Ok(0),
        Some(_) => return Err(Rc1ChannelStateMigrationError::Conflict),
        None => {}
    }
    let binding = RebornUserIdentityBinding {
        provider: RebornIdentityProviderId::new(provider).map_err(log_malformed)?,
        provider_user_id: RebornIdentityProviderUserId::new(provider_user_id)
            .map_err(log_malformed)?,
        user_id,
    };
    inputs
        .identity_store
        .bind_user_identity(binding)
        .await
        .map_err(log_conflict)?;
    Ok(1)
}

async fn upsert_dm_target(
    inputs: &Rc1ChannelStateMigrationInputs,
    extension: &str,
    user: &UserId,
    actor: String,
    target: serde_json::Value,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    match inputs
        .dm_targets
        .load(extension, user)
        .await
        .map_err(log_unavailable)?
    {
        Some(existing) if dm_target_matches(&existing, extension, user, &actor, &target) => {
            return Ok(0);
        }
        Some(_) => return Err(Rc1ChannelStateMigrationError::Conflict),
        None => {}
    }
    inputs
        .dm_targets
        .upsert(extension, user, actor, target)
        .await
        .map_err(log_unavailable)?;
    Ok(1)
}

fn dm_target_matches(
    current: &ChannelDmTargetRecord,
    extension: &str,
    user: &UserId,
    actor: &str,
    target: &serde_json::Value,
) -> bool {
    current.extension_id == extension
        && current.user_id == user.as_str()
        && current.external_actor_id == actor
        && &current.target == target
}

async fn validate_source_rows(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
) -> Result<(), Rc1ChannelStateMigrationError> {
    for row in query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-personal-binding/identities"),
    )
    .await?
    {
        let _: Rc1SlackIdentity = parse(&row)?;
    }
    for row in query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-channel-routes"),
    )
    .await?
    {
        let _: Rc1SlackRoute = parse(&row)?;
    }
    for row in query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-personal-binding/dm-targets"),
    )
    .await?
    {
        let _: Rc1SlackDmTarget = parse(&row)?;
    }
    for row in query_all(
        &inputs.filesystem,
        &format!("{shared}/telegram-binding/identities"),
    )
    .await?
    {
        let _: Rc1TelegramBinding = parse(&row)?;
    }
    for row in query_all(&inputs.filesystem, &format!("{shared}/telegram-dm-targets")).await? {
        let _: Rc1TelegramDmTarget = parse(&row)?;
    }
    Ok(())
}

async fn inspect_slack_connection_disposition(
    filesystem: &Arc<dyn RootFilesystem>,
    admin_scope: &ResourceScope,
    shared: &str,
) -> Result<Rc1SlackConnectionDisposition, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        filesystem,
        &format!("{shared}/slack-personal-binding/connections"),
    )
    .await?;
    let mut active_superseded = 0usize;
    let mut stale_expired = 0usize;
    let mut disconnected_superseded = 0usize;
    for row in &rows {
        let connection: Rc1SlackConnection = parse(row)?;
        if connection.tenant_id != admin_scope.tenant_id.as_str() {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        let user = UserId::new(&connection.user_id).map_err(log_malformed)?;
        AdapterInstallationId::new(&connection.installation_id).map_err(log_malformed)?;
        let expected_suffix = format!("/{}/{}.json", connection.installation_id, user.as_str());
        if !row.path.as_str().ends_with(&expected_suffix) {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        match connection.state {
            Rc1SlackConnectionState::Connecting => {
                if connection.disconnect_cleanup.is_some() {
                    return Err(Rc1ChannelStateMigrationError::Malformed);
                }
                stale_expired = stale_expired.saturating_add(1);
            }
            Rc1SlackConnectionState::Active => {
                if connection.disconnect_cleanup.is_some() {
                    return Err(Rc1ChannelStateMigrationError::Malformed);
                }
                active_superseded = active_superseded.saturating_add(1);
            }
            Rc1SlackConnectionState::Disconnecting => {
                // Generic channel state has no compatible halfway-cleanup
                // journal. Starting ingress could re-authorize rows the rc1
                // disconnect intended to remove, so require operator recovery.
                return Err(Rc1ChannelStateMigrationError::InterruptedSetup);
            }
            Rc1SlackConnectionState::Disconnected => {
                if connection.disconnect_cleanup.is_some() {
                    return Err(Rc1ChannelStateMigrationError::Malformed);
                }
                disconnected_superseded = disconnected_superseded.saturating_add(1);
            }
        }
    }
    let marker = Rc1SlackConnectionDispositionMarker {
        schema: "rc1-slack-connections-v1".to_string(),
        source_digest: source_rows_digest(&rows),
        active_superseded,
        stale_expired,
        disconnected_superseded,
        source_rows: rows.len(),
    };
    let marker_path =
        format!("{shared}/channel-extensions/slack/migrations/rc1-connections-v1.complete.json");
    let already_complete = disposition_marker_matches(filesystem, &marker_path, &marker).await?;
    Ok(Rc1SlackConnectionDisposition {
        marker,
        already_complete,
    })
}

async fn inspect_telegram_pairing_disposition(
    filesystem: &Arc<dyn RootFilesystem>,
    admin_scope: &ResourceScope,
    shared: &str,
) -> Result<Rc1TelegramPairingDisposition, Rc1ChannelStateMigrationError> {
    let codes = query_all(filesystem, &format!("{shared}/telegram-pairing/codes")).await?;
    let mut challenges = 0;
    for row in &codes {
        let record: Rc1TelegramPairingRecord = parse(row)?;
        if record.tenant_id != admin_scope.tenant_id {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        AdapterInstallationId::new(&record.installation_id).map_err(log_malformed)?;
        if record.consumed_at.is_none() {
            challenges += 1;
        }
    }
    let users = query_all(filesystem, &format!("{shared}/telegram-pairing/users")).await?;
    for row in &users {
        let _: Rc1TelegramPairingPointer = parse(row)?;
    }
    let mut completions = 0;
    let pending_completions = query_all(
        filesystem,
        &format!("{shared}/telegram-pairing/pending-completions"),
    )
    .await?;
    for row in &pending_completions {
        let completion: Rc1TelegramPairingCompletion = parse(row)?;
        AdapterInstallationId::new(&completion.installation_id).map_err(log_malformed)?;
        if !completion.completed {
            completions += 1;
        }
    }
    let mut all_rows = codes;
    all_rows.extend(users);
    all_rows.extend(pending_completions);
    let marker = Rc1TelegramPairingDispositionMarker {
        schema: "rc1-telegram-pairing-v1".to_string(),
        source_digest: source_rows_digest(&all_rows),
        challenges_expired: challenges,
        pending_completions_expired: completions,
        source_rows: all_rows.len(),
    };
    let marker_path =
        format!("{shared}/channel-extensions/telegram/migrations/rc1-pairing-v1.complete.json");
    let already_complete = disposition_marker_matches(filesystem, &marker_path, &marker).await?;
    Ok(Rc1TelegramPairingDisposition {
        marker,
        already_complete,
    })
}

fn source_rows_digest(rows: &[VersionedEntry]) -> String {
    let mut ordered = rows.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.path.as_str().cmp(right.path.as_str()));
    let mut bytes = Vec::new();
    for row in ordered {
        let path = row.path.as_str().as_bytes();
        bytes.extend_from_slice(&(path.len() as u64).to_be_bytes());
        bytes.extend_from_slice(path);
        bytes.extend_from_slice(&(row.entry.body.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&row.entry.body);
    }
    sha256_digest_token(&bytes)
}

async fn disposition_marker_matches<T>(
    filesystem: &Arc<dyn RootFilesystem>,
    path: &str,
    expected: &T,
) -> Result<bool, Rc1ChannelStateMigrationError>
where
    T: for<'de> Deserialize<'de> + PartialEq,
{
    let path = VirtualPath::new(path).map_err(log_malformed)?;
    let Some(entry) = filesystem.get(&path).await.map_err(log_unavailable)? else {
        return Ok(false);
    };
    let actual = serde_json::from_slice::<T>(&entry.entry.body).map_err(log_malformed)?;
    if &actual != expected {
        return Err(Rc1ChannelStateMigrationError::Conflict);
    }
    Ok(true)
}

async fn commit_disposition_marker<T>(
    filesystem: &Arc<dyn RootFilesystem>,
    path: &str,
    marker: &T,
    already_complete: bool,
) -> Result<(), Rc1ChannelStateMigrationError>
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq,
{
    if already_complete {
        return Ok(());
    }
    let path = VirtualPath::new(path).map_err(log_malformed)?;
    let kind = ironclaw_filesystem::RecordKind::new("rc1_channel_state_disposition")
        .map_err(log_malformed)?;
    let value = serde_json::to_value(marker).map_err(log_unavailable)?;
    let entry = ironclaw_filesystem::Entry::record(kind, &value).map_err(log_unavailable)?;
    match filesystem
        .put(&path, entry, ironclaw_filesystem::CasExpectation::Absent)
        .await
    {
        Ok(_) => Ok(()),
        Err(FilesystemError::VersionMismatch { .. }) => {
            if disposition_marker_matches(filesystem, path.as_str(), marker).await? {
                Ok(())
            } else {
                Err(Rc1ChannelStateMigrationError::Conflict)
            }
        }
        Err(error) => Err(log_unavailable(error)),
    }
}

fn rewrite_installation_prefix(
    provider_user_id: &str,
    old_installation: Option<&str>,
    target_installation: Option<&str>,
) -> Result<String, Rc1ChannelStateMigrationError> {
    let Some(old) = old_installation else {
        return Err(Rc1ChannelStateMigrationError::Malformed);
    };
    let Some(target) = target_installation else {
        return Err(Rc1ChannelStateMigrationError::MissingInstallation);
    };
    let Some(actor) = provider_user_id.strip_prefix(&format!("{old}:")) else {
        return Err(Rc1ChannelStateMigrationError::Malformed);
    };
    if actor.is_empty() {
        return Err(Rc1ChannelStateMigrationError::Malformed);
    }
    Ok(format!("{target}:{actor}"))
}

async fn read_optional<T: for<'de> Deserialize<'de>>(
    inputs: &Rc1ChannelStateMigrationInputs,
    path: &str,
) -> Result<Option<T>, Rc1ChannelStateMigrationError> {
    let path = VirtualPath::new(path).map_err(log_malformed)?;
    match inputs.filesystem.get(&path).await {
        Ok(Some(entry)) => serde_json::from_slice(&entry.entry.body)
            .map(Some)
            .map_err(log_malformed),
        Ok(None) | Err(FilesystemError::NotFound { .. }) => Ok(None),
        Err(error) => Err(log_unavailable(error)),
    }
}

async fn query_all(
    filesystem: &Arc<dyn RootFilesystem>,
    prefix: &str,
) -> Result<Vec<VersionedEntry>, Rc1ChannelStateMigrationError> {
    let prefix = VirtualPath::new(prefix).map_err(log_malformed)?;
    let mut rows = Vec::new();
    let mut offset = 0;
    loop {
        let page = match filesystem
            .query(&prefix, &Filter::All, Page::new(offset, Page::MAX_LIMIT))
            .await
        {
            Ok(page) => page,
            Err(FilesystemError::NotFound { .. }) => break,
            Err(error) => return Err(log_unavailable(error)),
        };
        let count = page.len();
        if rows.len().saturating_add(count) > MAX_RC1_CHANNEL_ROWS_PER_ROOT {
            return Err(Rc1ChannelStateMigrationError::Unavailable);
        }
        rows.extend(page);
        if count < Page::MAX_LIMIT as usize {
            break;
        }
        offset = offset.saturating_add(count as u64);
    }
    Ok(rows)
}

fn parse<T: for<'de> Deserialize<'de>>(
    row: &VersionedEntry,
) -> Result<T, Rc1ChannelStateMigrationError> {
    serde_json::from_slice(&row.entry.body).map_err(log_malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extensions::{
        AdminConfigurationField, ExtensionAdminConfigurationDescriptor, ExtensionInstallation,
        ExtensionInstallationId, ExtensionInstallationStore, ExtensionManifestRecord,
        ExtensionManifestRef, InstallationOwner, ManifestHash, ManifestSource, PackageRootBinding,
    };
    use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        mount::{MountGrant, MountPermissions, MountView},
        path::MountAlias,
    };
    use ironclaw_secrets::{SecretStore, SecretStorePort};

    fn slack_manifest_record() -> ExtensionManifestRecord {
        let raw = r#"
schema_version = "reborn.extension_manifest.v3"
id = "slack"
name = "Slack fixture"
version = "0.1.0"
description = "rc1 channel-state migration fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "slack.fixture/v1"
"#;
        let hash = ManifestHash::new(sha256_digest_token(raw.as_bytes())).expect("manifest hash");
        ExtensionManifestRecord::from_toml_with_root_binding(
            raw,
            ManifestSource::HostBundled,
            &ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
            Some(hash),
            &crate::product_extension_host_api_contract_registry().expect("contracts"),
            PackageRootBinding::Virtual,
        )
        .expect("Slack fixture manifest")
    }

    fn telegram_manifest_record() -> ExtensionManifestRecord {
        let raw = r#"
schema_version = "reborn.extension_manifest.v3"
id = "telegram"
name = "Telegram fixture"
version = "0.1.0"
description = "rc1 channel-state migration fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "telegram.fixture/v1"
"#;
        let hash = ManifestHash::new(sha256_digest_token(raw.as_bytes())).expect("manifest hash");
        ExtensionManifestRecord::from_toml_with_root_binding(
            raw,
            ManifestSource::HostBundled,
            &ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
            Some(hash),
            &crate::product_extension_host_api_contract_registry().expect("contracts"),
            PackageRootBinding::Virtual,
        )
        .expect("Telegram fixture manifest")
    }

    fn slack_admin_descriptor() -> ExtensionAdminConfigurationDescriptor {
        let fields = [
            ("slack_bot_token", true, true),
            ("slack_signing_secret", true, true),
            ("slack_team_id", false, true),
            ("slack_api_app_id", false, true),
            ("slack_installation_id", false, true),
            ("slack_bot_user_id", false, true),
            ("slack_shared_subject_user_id", false, false),
            ("slack_oauth_client_id", false, true),
            ("slack_oauth_client_secret", true, true),
            ("slack_allowed_channels", false, false),
            ("slack_subject_routes", false, false),
        ]
        .into_iter()
        .map(|(handle, secret, required)| AdminConfigurationField {
            handle: SecretHandle::new(handle).expect("fixture handle"),
            label: handle.to_string(),
            secret,
            required,
        })
        .collect();
        ExtensionAdminConfigurationDescriptor {
            group_id: AdminConfigurationGroupId::new(SLACK_GROUP).expect("group"),
            display_name: "Slack fixture".to_string(),
            description: "rc1 migration fixture".to_string(),
            fields,
        }
    }

    fn telegram_admin_descriptor() -> ExtensionAdminConfigurationDescriptor {
        let fields = [
            ("telegram_bot_token", true, true),
            ("telegram_webhook_secret", true, true),
            ("telegram_webhook_url", false, true),
            ("bot_username", false, true),
        ]
        .into_iter()
        .map(|(handle, secret, required)| AdminConfigurationField {
            handle: SecretHandle::new(handle).expect("fixture handle"),
            label: handle.to_string(),
            secret,
            required,
        })
        .collect();
        ExtensionAdminConfigurationDescriptor {
            group_id: AdminConfigurationGroupId::new(TELEGRAM_GROUP).expect("group"),
            display_name: "Telegram fixture".to_string(),
            description: "rc1 migration fixture".to_string(),
            fields,
        }
    }

    fn fixed_admin_mount() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/extension-admin-configuration").expect("alias"),
            VirtualPath::new("/tenants/tenant-a/shared/admin-configuration").expect("target root"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("admin mount")
    }

    #[test]
    fn frozen_rc1_channel_wires_remain_readable_and_strict() {
        let slack_setup = r#"{
            "installation_id":"slack-old-install",
            "team_id":"T1",
            "api_app_id":"A1",
            "user_id":"U-BOT",
            "shared_subject_user_id":"operator",
            "bot_token_handle":"slack-bot-r1",
            "signing_secret_handle":"slack-signing-r1",
            "oauth_client_id":"oauth-client-1",
            "oauth_client_secret_handle":"slack-oauth-r1",
            "revision":1,
            "updated_at":"2026-07-01T00:00:00Z"
        }"#;
        let setup: Rc1SlackSetup = serde_json::from_str(slack_setup).expect("exact rc1 Slack wire");
        assert_eq!(setup.installation_id, "slack-old-install");
        assert_eq!(setup.oauth_client_id.as_deref(), Some("oauth-client-1"));
        assert_eq!(
            setup
                .oauth_client_secret_handle
                .as_ref()
                .map(SecretHandle::as_str),
            Some("slack-oauth-r1")
        );

        let telegram_setup = r#"{
            "bot_id":4242,
            "bot_username":"ironclaw_fixture_bot",
            "webhook_url":"https://example.invalid/telegram",
            "bot_token_handle":"telegram-bot-r2",
            "webhook_secret_handle":"telegram-webhook-r2",
            "revision":2,
            "updated_at":"2026-07-01T00:00:00Z"
        }"#;
        assert!(matches!(
            serde_json::from_str::<Rc1TelegramSetup>(telegram_setup)
                .expect("exact rc1 Telegram wire"),
            Rc1TelegramSetup::Active(_)
        ));

        let malformed =
            slack_setup.replace("\"revision\":1", "\"revision\":1,\"unknown_p0_field\":true");
        assert!(
            serde_json::from_str::<Rc1SlackSetup>(&malformed).is_err(),
            "unknown released-state fields must fail closed"
        );
    }

    #[test]
    fn identity_prefix_rewrite_is_exact_and_requires_the_target_installation() {
        assert_eq!(
            rewrite_installation_prefix(
                "slack-old-install:U123",
                Some("slack-old-install"),
                Some("slack-new-install"),
            )
            .unwrap(),
            "slack-new-install:U123"
        );
        assert!(matches!(
            rewrite_installation_prefix(
                "slack-old-install-extra:U123",
                Some("slack-old-install"),
                Some("slack-new-install"),
            ),
            Err(Rc1ChannelStateMigrationError::Malformed)
        ));
        assert!(matches!(
            rewrite_installation_prefix("slack-old-install:U123", Some("slack-old-install"), None,),
            Err(Rc1ChannelStateMigrationError::MissingInstallation)
        ));
    }

    #[tokio::test]
    async fn caller_imports_rc1_slack_setup_and_secrets_idempotently() {
        let backend = Arc::new(InMemoryBackend::new());
        let filesystem: Arc<dyn RootFilesystem> = backend.clone();
        let secret_store = Arc::new(SecretStore::ephemeral_over(Arc::clone(&backend)));
        let secret_store_port: Arc<dyn SecretStorePort> = secret_store.clone();
        let admin_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>> = Arc::new(
            ScopedFilesystem::with_fixed_view(Arc::clone(&filesystem), fixed_admin_mount()),
        );
        let admin_configuration = Arc::new(
            AdminConfigurationService::<dyn RootFilesystem, dyn SecretStorePort>::new(
                crate::FilesystemAdminConfigurationStore::new(admin_filesystem),
                Arc::clone(&secret_store_port),
                [slack_admin_descriptor()],
            )
            .expect("admin configuration"),
        );
        let installation_store = ExtensionInstallationStore::load_at(
            Arc::clone(&filesystem),
            ExtensionInstallationStore::default_state_path().expect("state path"),
            ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
            crate::product_extension_host_api_contract_registry().expect("contracts"),
        )
        .await
        .expect("installation store");
        let manifest = slack_manifest_record();
        let extension_id = ironclaw_host_api::ids::ExtensionId::new("slack").expect("extension");
        let installation = ExtensionInstallation::new(
            ExtensionInstallationId::new("slack-target").expect("installation"),
            extension_id.clone(),
            ExtensionManifestRef::new(extension_id, manifest.manifest_hash().cloned()),
            Vec::new(),
            Utc::now(),
            InstallationOwner::Tenant,
        )
        .expect("installation");
        installation_store
            .upsert_manifest_and_installation(manifest, installation)
            .await
            .expect("seed target installation");
        let installation_store: Arc<dyn ExtensionInstallationStorePort> =
            Arc::new(installation_store);
        let slack_scope = ResourceScope {
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            user_id: UserId::new("operator-a").expect("operator"),
            agent_id: Some(AgentId::new("agent-a").expect("agent")),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let admin_scope = ResourceScope {
            tenant_id: slack_scope.tenant_id.clone(),
            user_id: UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        for (handle, material) in [
            ("rc1-bot-token", "xoxb-rc1"),
            ("rc1-signing", "signing-rc1"),
            ("rc1-oauth", "oauth-rc1"),
        ] {
            secret_store
                .put(
                    slack_scope.clone(),
                    SecretHandle::new(handle).expect("handle"),
                    SecretMaterial::from(material.to_string()),
                    None,
                )
                .await
                .expect("seed source secret");
        }
        let setup = serde_json::json!({
            "installation_id": "slack-rc1-install",
            "team_id": "T-RC1",
            "api_app_id": "A-RC1",
            "user_id": "U-BOT-RC1",
            "shared_subject_user_id": "operator-a",
            "bot_token_handle": "rc1-bot-token",
            "signing_secret_handle": "rc1-signing",
            "oauth_client_id": "client-rc1",
            "oauth_client_secret_handle": "rc1-oauth",
            "revision": 7,
            "updated_at": "2026-07-01T00:00:00Z"
        });
        filesystem
            .put(
                &VirtualPath::new("/tenants/tenant-a/shared/slack-setup/installation.json")
                    .expect("setup path"),
                Entry::bytes(serde_json::to_vec(&setup).expect("setup wire")),
                CasExpectation::Absent,
            )
            .await
            .expect("seed setup");
        for (path, value) in [
            (
                "/tenants/tenant-a/shared/slack-personal-binding/identities/U-PERSON.json",
                serde_json::json!({
                    "provider": "slack",
                    "provider_user_id": "slack-rc1-install:U-PERSON",
                    "user_id": "operator-a",
                    "state": "active",
                    "created_at": "2026-07-01T00:00:00Z",
                    "updated_at": "2026-07-01T00:00:00Z"
                }),
            ),
            (
                "/tenants/tenant-a/shared/slack-channel-routes/C-ALLOWED.json",
                serde_json::json!({
                    "tenant_id": "tenant-a",
                    "installation_id": "slack-rc1-install",
                    "team_id": "T-RC1",
                    "channel_id": "C-ALLOWED",
                    "subject_user_id": "user:slack-channel:C-ALLOWED",
                    "updated_at": "2026-07-01T00:00:00Z"
                }),
            ),
            (
                "/tenants/tenant-a/shared/slack-channel-routes/C-EXPLICIT.json",
                serde_json::json!({
                    "tenant_id": "tenant-a",
                    "installation_id": "slack-rc1-install",
                    "team_id": "T-RC1",
                    "channel_id": "C-EXPLICIT",
                    "subject_user_id": "operator-a",
                    "updated_at": "2026-07-01T00:00:00Z"
                }),
            ),
            (
                "/tenants/tenant-a/shared/slack-personal-binding/dm-targets/operator-a.json",
                serde_json::json!({
                    "tenant_id": "tenant-a",
                    "installation_id": "slack-rc1-install",
                    "team_id": "T-RC1",
                    "user_id": "operator-a",
                    "slack_user_id": "U-PERSON",
                    "dm_channel_id": "D-RC1",
                    "created_at": "2026-07-01T00:00:00Z",
                    "updated_at": "2026-07-01T00:00:00Z"
                }),
            ),
        ] {
            filesystem
                .put(
                    &VirtualPath::new(path).expect("legacy state path"),
                    Entry::bytes(serde_json::to_vec(&value).expect("legacy state wire")),
                    CasExpectation::Absent,
                )
                .await
                .expect("seed legacy state");
        }

        let inputs = Rc1ChannelStateMigrationInputs {
            filesystem: Arc::clone(&filesystem),
            installation_store,
            secret_store: secret_store_port,
            admin_configuration: Arc::clone(&admin_configuration),
            oauth_channel_secret_scope: slack_scope,
            proof_code_channel_secret_scope: admin_scope.clone(),
            admin_scope: admin_scope.clone(),
            identity_store: Arc::new(FilesystemChannelIdentityStore::new(
                Arc::clone(&filesystem),
                admin_scope.tenant_id.clone(),
                admin_scope.user_id.clone(),
            )),
            dm_targets: Arc::new(FilesystemChannelDmTargetStore::new(
                Arc::clone(&filesystem),
                admin_scope.tenant_id.clone(),
                admin_scope.user_id.clone(),
            )),
        };
        let first = migrate_rc1_channel_state(&inputs)
            .await
            .expect("migrate exact setup");
        assert_eq!(first.configuration_values, 9);
        assert_eq!(first.identities, 1);
        assert_eq!(first.route_values, 2);
        assert_eq!(first.dm_targets, 1);
        let state = admin_configuration
            .get(
                &admin_scope,
                &AdminConfigurationGroupId::new(SLACK_GROUP).expect("group"),
            )
            .await
            .expect("read imported setup");
        assert!(state.complete);
        assert_eq!(
            state
                .fields
                .iter()
                .find(|field| field.handle.as_str() == "slack_team_id")
                .and_then(|field| field.value.as_deref()),
            Some("T-RC1")
        );
        let target_secrets = secret_store
            .metadata_for_scope(&admin_scope.tenant_shared_managed_scope())
            .await
            .expect("list migrated secrets");
        assert_eq!(target_secrets.len(), 3);
        let binding = inputs
            .identity_store
            .resolve_user_identity("slack", "slack-target:U-PERSON")
            .await
            .expect("identity lookup")
            .expect("identity migrated");
        assert_eq!(binding.as_str(), "operator-a");
        let dm = inputs
            .dm_targets
            .load(SLACK, &UserId::new("operator-a").expect("operator"))
            .await
            .expect("DM lookup")
            .expect("DM target migrated");
        assert_eq!(dm.external_actor_id, "U-PERSON");
        assert_eq!(dm.target["conversation_id"], "D-RC1");

        // Reconstruct every 1.1 reader over the durable backend. This models
        // the next process start rather than proving only that the migration
        // service's in-memory objects can still see their own writes.
        let restarted_admin_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>> = Arc::new(
            ScopedFilesystem::with_fixed_view(Arc::clone(&filesystem), fixed_admin_mount()),
        );
        let restarted_admin =
            AdminConfigurationService::<dyn RootFilesystem, dyn SecretStorePort>::new(
                crate::FilesystemAdminConfigurationStore::new(restarted_admin_filesystem),
                Arc::clone(&inputs.secret_store),
                [slack_admin_descriptor()],
            )
            .expect("reopen admin configuration");
        let restarted_state = restarted_admin
            .get(
                &admin_scope,
                &AdminConfigurationGroupId::new(SLACK_GROUP).expect("group"),
            )
            .await
            .expect("read setup after restart");
        assert!(restarted_state.complete);
        for (handle, expected) in [
            ("slack_bot_token", "xoxb-rc1"),
            ("slack_signing_secret", "signing-rc1"),
            ("slack_oauth_client_secret", "oauth-rc1"),
        ] {
            let material = restarted_admin
                .secret_material(
                    &admin_scope,
                    &AdminConfigurationGroupId::new(SLACK_GROUP).expect("group"),
                    &SecretHandle::new(handle).expect("handle"),
                )
                .await
                .expect("consume migrated secret after restart")
                .expect("migrated secret exists");
            assert_eq!(material.expose_secret(), expected);
        }
        let restarted_identities = FilesystemChannelIdentityStore::new(
            Arc::clone(&filesystem),
            admin_scope.tenant_id.clone(),
            admin_scope.user_id.clone(),
        );
        assert_eq!(
            restarted_identities
                .resolve_user_identity("slack", "slack-target:U-PERSON")
                .await
                .expect("identity lookup after restart")
                .expect("identity retained after restart")
                .as_str(),
            "operator-a"
        );
        let restarted_dm_targets = FilesystemChannelDmTargetStore::new(
            Arc::clone(&filesystem),
            admin_scope.tenant_id.clone(),
            admin_scope.user_id.clone(),
        );
        let restarted_dm = restarted_dm_targets
            .load(SLACK, &UserId::new("operator-a").expect("operator"))
            .await
            .expect("DM lookup after restart")
            .expect("DM target retained after restart");
        assert_eq!(restarted_dm.external_actor_id, "U-PERSON");
        assert_eq!(restarted_dm.target["conversation_id"], "D-RC1");

        let second = migrate_rc1_channel_state(&inputs)
            .await
            .expect("second pass revalidates");
        assert_eq!(second.configuration_values, 0);
        assert_eq!(second.identities, 0);
        assert_eq!(second.route_values, 0);
        assert_eq!(second.dm_targets, 0);
    }

    #[tokio::test]
    async fn caller_imports_rc1_telegram_setup_and_reopens_every_usable_state() {
        let backend = Arc::new(InMemoryBackend::new());
        let filesystem: Arc<dyn RootFilesystem> = backend.clone();
        let secret_store = Arc::new(SecretStore::ephemeral_over(Arc::clone(&backend)));
        let secret_store_port: Arc<dyn SecretStorePort> = secret_store.clone();
        let admin_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>> = Arc::new(
            ScopedFilesystem::with_fixed_view(Arc::clone(&filesystem), fixed_admin_mount()),
        );
        let admin_configuration = Arc::new(
            AdminConfigurationService::<dyn RootFilesystem, dyn SecretStorePort>::new(
                crate::FilesystemAdminConfigurationStore::new(admin_filesystem),
                Arc::clone(&secret_store_port),
                [telegram_admin_descriptor()],
            )
            .expect("admin configuration"),
        );
        let installation_store = ExtensionInstallationStore::load_at(
            Arc::clone(&filesystem),
            ExtensionInstallationStore::default_state_path().expect("state path"),
            ironclaw_host_api::host_port::default_host_port_catalog().expect("host ports"),
            crate::product_extension_host_api_contract_registry().expect("contracts"),
        )
        .await
        .expect("installation store");
        let manifest = telegram_manifest_record();
        let extension_id = ironclaw_host_api::ids::ExtensionId::new(TELEGRAM).expect("extension");
        let installation = ExtensionInstallation::new(
            ExtensionInstallationId::new("telegram-target").expect("installation"),
            extension_id.clone(),
            ExtensionManifestRef::new(extension_id, manifest.manifest_hash().cloned()),
            Vec::new(),
            Utc::now(),
            InstallationOwner::Tenant,
        )
        .expect("installation");
        installation_store
            .upsert_manifest_and_installation(manifest, installation)
            .await
            .expect("seed target installation");
        let installation_store: Arc<dyn ExtensionInstallationStorePort> =
            Arc::new(installation_store);
        let telegram_scope = ResourceScope {
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            user_id: UserId::new("operator-a").expect("operator"),
            agent_id: Some(AgentId::new("agent-a").expect("agent")),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let admin_scope = ResourceScope {
            tenant_id: telegram_scope.tenant_id.clone(),
            user_id: UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        for (handle, material) in [
            ("rc1-telegram-bot", "telegram-token-rc1"),
            ("rc1-telegram-webhook", "telegram-webhook-rc1"),
        ] {
            secret_store
                .put(
                    telegram_scope.clone(),
                    SecretHandle::new(handle).expect("handle"),
                    SecretMaterial::from(material.to_string()),
                    None,
                )
                .await
                .expect("seed source secret");
        }
        let legacy_rows = [
            (
                "/tenants/tenant-a/shared/telegram-setup/installation.json",
                serde_json::json!({
                    "bot_id": 4242,
                    "bot_username": "ironclaw_fixture_bot",
                    "webhook_url": "https://example.invalid/telegram",
                    "bot_token_handle": "rc1-telegram-bot",
                    "webhook_secret_handle": "rc1-telegram-webhook",
                    "revision": 2,
                    "updated_at": "2026-07-01T00:00:00Z"
                }),
            ),
            (
                "/tenants/tenant-a/shared/telegram-binding/identities/9001.json",
                serde_json::json!({
                    "provider_user_id": "tg-bot-4242:9001",
                    "user_id": "operator-a",
                    "epoch": "epoch-rc1",
                    "active": true
                }),
            ),
            (
                "/tenants/tenant-a/shared/telegram-dm-targets/operator-a.json",
                serde_json::json!({
                    "user_id": "operator-a",
                    "chat_id": 12345
                }),
            ),
            (
                "/tenants/tenant-a/shared/telegram-dm-targets/stale-operator.json",
                serde_json::json!({
                    "user_id": "stale-operator",
                    "chat_id": 54321
                }),
            ),
            (
                "/tenants/tenant-a/shared/telegram-pairing/codes/ABCDEFGH.json",
                serde_json::json!({
                    "code": "ABCDEFGH",
                    "tenant_id": "tenant-a",
                    "user_id": "operator-a",
                    "installation_id": "tg-bot-4242",
                    "created_at": "2026-07-01T00:00:00Z",
                    "expires_at": "2026-07-01T00:15:00Z",
                    "consumed_at": null
                }),
            ),
            (
                "/tenants/tenant-a/shared/telegram-pairing/users/operator-a.json",
                serde_json::json!({
                    "code": "ABCDEFGH",
                    "active": true
                }),
            ),
            (
                "/tenants/tenant-a/shared/telegram-pairing/pending-completions/operator-a.json",
                serde_json::json!({
                    "installation_id": "tg-bot-4242",
                    "user_id": "operator-a",
                    "chat_id": 12345,
                    "completed": false
                }),
            ),
        ];
        for (path, value) in legacy_rows {
            filesystem
                .put(
                    &VirtualPath::new(path).expect("legacy state path"),
                    Entry::bytes(serde_json::to_vec(&value).expect("legacy state wire")),
                    CasExpectation::Absent,
                )
                .await
                .expect("seed legacy state");
        }

        let inputs = Rc1ChannelStateMigrationInputs {
            filesystem: Arc::clone(&filesystem),
            installation_store,
            secret_store: secret_store_port,
            admin_configuration,
            oauth_channel_secret_scope: admin_scope.clone(),
            proof_code_channel_secret_scope: telegram_scope,
            admin_scope: admin_scope.clone(),
            identity_store: Arc::new(FilesystemChannelIdentityStore::new(
                Arc::clone(&filesystem),
                admin_scope.tenant_id.clone(),
                admin_scope.user_id.clone(),
            )),
            dm_targets: Arc::new(FilesystemChannelDmTargetStore::new(
                Arc::clone(&filesystem),
                admin_scope.tenant_id.clone(),
                admin_scope.user_id.clone(),
            )),
        };
        let first = migrate_rc1_channel_state(&inputs)
            .await
            .expect("migrate exact Telegram setup");
        assert_eq!(first.configuration_values, 4);
        assert_eq!(first.identities, 1);
        assert_eq!(first.dm_targets, 1);
        assert_eq!(first.unbound_dm_targets_skipped, 1);
        assert_eq!(first.proof_code_pairing_challenges_expired, 1);
        assert_eq!(first.proof_code_pending_completions_expired, 1);

        let restarted_admin_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>> = Arc::new(
            ScopedFilesystem::with_fixed_view(Arc::clone(&filesystem), fixed_admin_mount()),
        );
        let restarted_admin =
            AdminConfigurationService::<dyn RootFilesystem, dyn SecretStorePort>::new(
                crate::FilesystemAdminConfigurationStore::new(restarted_admin_filesystem),
                Arc::clone(&inputs.secret_store),
                [telegram_admin_descriptor()],
            )
            .expect("reopen admin configuration");
        let group = AdminConfigurationGroupId::new(TELEGRAM_GROUP).expect("group");
        let state = restarted_admin
            .get(&admin_scope, &group)
            .await
            .expect("read Telegram setup after restart");
        assert!(state.complete);
        for (handle, expected) in [
            ("telegram_bot_token", "telegram-token-rc1"),
            ("telegram_webhook_secret", "telegram-webhook-rc1"),
        ] {
            let material = restarted_admin
                .secret_material(
                    &admin_scope,
                    &group,
                    &SecretHandle::new(handle).expect("handle"),
                )
                .await
                .expect("consume migrated secret after restart")
                .expect("migrated secret exists");
            assert_eq!(material.expose_secret(), expected);
        }
        let restarted_identities = FilesystemChannelIdentityStore::new(
            Arc::clone(&filesystem),
            admin_scope.tenant_id.clone(),
            admin_scope.user_id.clone(),
        );
        assert_eq!(
            restarted_identities
                .resolve_user_identity(TELEGRAM, "telegram-target:9001")
                .await
                .expect("identity lookup after restart")
                .expect("identity retained after restart")
                .as_str(),
            "operator-a"
        );
        let restarted_dm_targets = FilesystemChannelDmTargetStore::new(
            Arc::clone(&filesystem),
            admin_scope.tenant_id.clone(),
            admin_scope.user_id.clone(),
        );
        let dm = restarted_dm_targets
            .load(TELEGRAM, &UserId::new("operator-a").expect("operator"))
            .await
            .expect("DM lookup after restart")
            .expect("DM target retained after restart");
        assert_eq!(dm.external_actor_id, "9001");
        assert_eq!(dm.target["conversation_id"], "12345");

        let second = migrate_rc1_channel_state(&inputs)
            .await
            .expect("second pass revalidates");
        assert_eq!(second.configuration_values, 0);
        assert_eq!(second.identities, 0);
        assert_eq!(second.dm_targets, 0);
        assert_eq!(second.proof_code_pairing_rows_unchanged, 3);
    }

    #[tokio::test]
    async fn scope_discovery_finds_every_tenant_and_exact_secret_owner() {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        for (tenant, user, agent) in [
            ("tenant-a", "operator-a", "agent-a"),
            ("tenant-b", "operator-b", "agent-b"),
        ] {
            let setup_path = VirtualPath::new(format!(
                "/tenants/{tenant}/shared/slack-setup/installation.json"
            ))
            .expect("setup path");
            let setup = serde_json::json!({
                "installation_id": format!("slack-{tenant}"),
                "team_id": format!("team-{tenant}"),
                "api_app_id": format!("app-{tenant}"),
                "user_id": format!("bot-{tenant}"),
                "shared_subject_user_id": user,
                "bot_token_handle": format!("bot-token-{tenant}"),
                "signing_secret_handle": format!("signing-{tenant}"),
                "revision": 1,
                "updated_at": "2026-07-01T00:00:00Z"
            });
            filesystem
                .put(
                    &setup_path,
                    Entry::bytes(serde_json::to_vec(&setup).expect("setup wire")),
                    CasExpectation::Absent,
                )
                .await
                .expect("seed setup");
            for handle in [format!("bot-token-{tenant}"), format!("signing-{tenant}")] {
                let secret_path = VirtualPath::new(format!(
                    "/tenants/{tenant}/users/{user}/secrets/agents/{agent}/secrets/{handle}.json"
                ))
                .expect("secret path");
                filesystem
                    .put(
                        &secret_path,
                        Entry::bytes(vec![1, 2, 3]),
                        CasExpectation::Absent,
                    )
                    .await
                    .expect("seed secret authority");
            }
        }

        let scopes = discover_rc1_channel_migration_scopes(filesystem)
            .await
            .expect("discover all rc1 tenants");
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].admin_scope.tenant_id.as_str(), "tenant-a");
        assert_eq!(
            scopes[0].oauth_channel_secret_scope.user_id.as_str(),
            "operator-a"
        );
        assert_eq!(
            scopes[0]
                .oauth_channel_secret_scope
                .agent_id
                .as_ref()
                .map(AgentId::as_str),
            Some("agent-a")
        );
        assert_eq!(scopes[1].admin_scope.tenant_id.as_str(), "tenant-b");
        assert_eq!(
            scopes[1].oauth_channel_secret_scope.user_id.as_str(),
            "operator-b"
        );
    }

    #[tokio::test]
    async fn slack_connection_disposition_pages_and_second_run_is_unchanged() {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let admin_scope = ResourceScope {
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            user_id: UserId::new("operator-a").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let shared = "/tenants/tenant-a/shared";
        let total = Page::MAX_LIMIT as usize + 1;
        for index in 0..total {
            let user = format!("user-{index:04}");
            let path = VirtualPath::new(format!(
                "{shared}/slack-personal-binding/connections/slack-install/{user}.json"
            ))
            .expect("connection path");
            let state = if index + 1 == total {
                "connecting"
            } else {
                "active"
            };
            let connection = serde_json::json!({
                "tenant_id": "tenant-a",
                "user_id": user,
                "installation_id": "slack-install",
                "epoch": ironclaw_auth::AuthFlowId::new(),
                "state": state,
                "expires_at": "2026-07-02T00:00:00Z",
                "created_at": "2026-07-01T00:00:00Z",
                "updated_at": "2026-07-01T00:00:00Z"
            });
            filesystem
                .put(
                    &path,
                    Entry::bytes(serde_json::to_vec(&connection).expect("connection wire")),
                    CasExpectation::Absent,
                )
                .await
                .expect("seed connection");
        }

        let first = inspect_slack_connection_disposition(&filesystem, &admin_scope, shared)
            .await
            .expect("inspect every page");
        assert!(!first.already_complete);
        assert_eq!(first.marker.source_rows, total);
        assert_eq!(first.marker.active_superseded, total - 1);
        assert_eq!(first.marker.stale_expired, 1);
        let marker_path = format!(
            "{shared}/channel-extensions/slack/migrations/rc1-connections-v1.complete.json"
        );
        commit_disposition_marker(&filesystem, &marker_path, &first.marker, false)
            .await
            .expect("commit versioned disposition");

        let second = inspect_slack_connection_disposition(&filesystem, &admin_scope, shared)
            .await
            .expect("reverify retained source");
        assert!(second.already_complete);
        assert_eq!(second.marker.source_rows, total);
    }

    #[tokio::test]
    async fn interrupted_rc1_slack_disconnect_fails_closed() {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let admin_scope = ResourceScope {
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            user_id: UserId::new("operator-a").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let shared = "/tenants/tenant-a/shared";
        let path = VirtualPath::new(format!(
            "{shared}/slack-personal-binding/connections/slack-install/user-a.json"
        ))
        .expect("connection path");
        let connection = serde_json::json!({
            "tenant_id": "tenant-a",
            "user_id": "user-a",
            "installation_id": "slack-install",
            "epoch": ironclaw_auth::AuthFlowId::new(),
            "state": "disconnecting",
            "disconnect_cleanup": {"kind": "all_owned"},
            "expires_at": "2026-07-02T00:00:00Z",
            "created_at": "2026-07-01T00:00:00Z",
            "updated_at": "2026-07-01T00:00:00Z"
        });
        filesystem
            .put(
                &path,
                Entry::bytes(serde_json::to_vec(&connection).expect("connection wire")),
                CasExpectation::Absent,
            )
            .await
            .expect("seed connection");

        assert!(matches!(
            inspect_slack_connection_disposition(&filesystem, &admin_scope, shared).await,
            Err(Rc1ChannelStateMigrationError::InterruptedSetup)
        ));
    }
}
