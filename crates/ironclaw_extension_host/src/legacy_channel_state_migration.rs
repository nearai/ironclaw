//! Lossless startup import for channel state written by `1.0.0-rc.1`.
//!
//! Released channel lanes owned provider-specific roots. This module owns the
//! generic migration transaction while the two frozen wire readers stay in
//! narrowly allowlisted compatibility modules. Source rows are never deleted.

#![allow(
    dead_code,
    reason = "frozen rc1 wire readers retain every released field for fail-closed validation"
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use ironclaw_extension_contracts::external::ExternalActorBindingEpoch;
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
    FilesystemChannelDmTargetStore, FilesystemChannelIdentityStore,
};

const MAX_RC1_CHANNEL_TENANTS: usize = 100_000;
const MAX_RC1_CHANNEL_USERS_PER_TENANT: usize = 100_000;
const MAX_RC1_CHANNEL_AGENTS_PER_USER: usize = 100_000;
const MAX_RC1_CHANNEL_PROJECTS_PER_OWNER: usize = 100_000;
const MAX_RC1_CHANNEL_SECRET_SCOPE_CANDIDATES: usize = 1_000_000;
const MAX_RC1_CHANNEL_ROWS_PER_ROOT: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rc1ChannelRootMigrationSpec {
    pub provider_key: &'static str,
}

pub fn rc1_channel_root_migration_specs() -> [Rc1ChannelRootMigrationSpec; 2] {
    [
        oauth_channel::root_migration_spec(),
        proof_code_channel::root_migration_spec(),
    ]
}

/// Dependencies for one tenant's startup import.
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

/// Redacted, count-only result. Non-replayable state is explicitly expired
/// rather than copied into the new channel snapshot.
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
    /// One redacted count-only entry per discovered tenant scope.
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

mod discovery;
mod disposition;
mod oauth_channel;
mod proof_code_channel;

pub use discovery::{
    Rc1ChannelMigrationScope, discover_rc1_channel_migration_scopes, is_rc1_channel_state_path,
};
use disposition::*;

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

/// Discover and migrate every rc1 channel-state tenant before listeners or
/// extension writers start.
pub async fn migrate_all_rc1_channel_state(
    filesystem: Arc<dyn RootFilesystem>,
    installation_store: Arc<dyn ExtensionInstallationStorePort>,
    secret_store: Arc<dyn SecretStorePort>,
    admin_configuration: Arc<AdminConfigurationService<dyn RootFilesystem, dyn SecretStorePort>>,
) -> Result<Rc1ChannelStateMigrationReport, Rc1ChannelStateMigrationError> {
    let scopes = discover_rc1_channel_migration_scopes(Arc::clone(&filesystem)).await?;
    let mut aggregate = Rc1ChannelStateMigrationReport::default();
    for scope in scopes {
        let identity_store = Arc::new(FilesystemChannelIdentityStore::new(
            Arc::clone(&filesystem),
            scope.admin_scope.tenant_id.clone(),
            scope.admin_scope.user_id.clone(),
        ));
        let dm_targets = Arc::new(FilesystemChannelDmTargetStore::new(
            Arc::clone(&filesystem),
            scope.admin_scope.tenant_id.clone(),
            scope.admin_scope.user_id.clone(),
        ));
        aggregate.merge(
            migrate_rc1_channel_state(&Rc1ChannelStateMigrationInputs {
                filesystem: Arc::clone(&filesystem),
                installation_store: Arc::clone(&installation_store),
                secret_store: Arc::clone(&secret_store),
                admin_configuration: Arc::clone(&admin_configuration),
                oauth_channel_secret_scope: scope.oauth_channel_secret_scope,
                proof_code_channel_secret_scope: scope.proof_code_channel_secret_scope,
                admin_scope: scope.admin_scope,
                identity_store,
                dm_targets,
            })
            .await?,
        );
    }
    Ok(aggregate)
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
    let provider_keys = [
        oauth_channel::provider_key(),
        proof_code_channel::provider_key(),
    ];
    let installation_ids = target_installation_ids(inputs, provider_keys).await?;

    // Finish both fail-closed source reads before the first normalized write.
    let oauth_prepared = oauth_channel::prepare(inputs, &shared).await?;
    let proof_code_prepared = proof_code_channel::prepare(inputs, &shared).await?;

    let mut report = oauth_channel::migrate(
        inputs,
        &shared,
        installation_ids.get(provider_keys[0]),
        oauth_prepared,
    )
    .await?;
    report.merge(
        proof_code_channel::migrate(
            inputs,
            &shared,
            installation_ids.get(provider_keys[1]),
            proof_code_prepared,
        )
        .await?,
    );
    report.scopes.push(scope_report(&report));
    Ok(report)
}

fn scope_report(report: &Rc1ChannelStateMigrationReport) -> Rc1ChannelStateScopeMigrationReport {
    Rc1ChannelStateScopeMigrationReport {
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
    }
}

async fn target_installation_ids(
    inputs: &Rc1ChannelStateMigrationInputs,
    provider_keys: [&str; 2],
) -> Result<BTreeMap<String, String>, Rc1ChannelStateMigrationError> {
    let installations = inputs
        .installation_store
        .list_installations()
        .await
        .map_err(log_unavailable)?;
    let mut ids = BTreeMap::new();
    for installation in installations {
        let extension = installation.extension_id().as_str();
        if provider_keys.contains(&extension) {
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
            tracing::debug!(
                root = %prefix,
                limit = MAX_RC1_CHANNEL_ROWS_PER_ROOT,
                observed = rows.len().saturating_add(count),
                "rc1 channel migration row discovery bound exceeded"
            );
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
mod tests;
