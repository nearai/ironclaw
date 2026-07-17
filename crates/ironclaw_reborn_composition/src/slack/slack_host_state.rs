//! Durable host state for Slack host-beta personal binding.
//!
//! The Slack ingress path starts before a Slack actor is bound to a Reborn
//! user, so this state is tenant-scoped and lives under `/tenant-shared`.
//! The underlying `ScopedFilesystem` still routes through host APIs and is
//! backed by the selected durable root filesystem in libSQL/Postgres builds.

// arch-exempt: large_file, CAS-backed Slack binding lifecycle state and tests, plan #5905

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::Arc,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
#[cfg(test)]
use ironclaw_filesystem::Entry;
use ironclaw_filesystem::{
    CasExpectation, FileType, FilesystemError, FilesystemOperation, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use ironclaw_product_adapters::AdapterInstallationId;
use rand::RngExt as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::slack::slack_actor_identity::parse_slack_user_identity_provider_user_id;
use crate::slack::slack_channel_routes::{
    SlackChannelRoute, SlackChannelRouteAssignment, SlackChannelRouteError, SlackChannelRouteKey,
    SlackChannelRouteListPage, SlackChannelRouteStore,
};
use crate::slack::slack_outbound_targets::{
    SlackPersonalDmTarget, SlackPersonalDmTargetError, SlackPersonalDmTargetKey,
    SlackPersonalDmTargetStore,
};
use crate::slack::slack_personal_binding::{
    RebornUserIdentityBinding, RebornUserIdentityBindingDeleteStore,
    RebornUserIdentityBindingError, RebornUserIdentityBindingStore, SlackConnectionCleanupSelector,
    SlackConnectionEpoch, SlackConnectionOwner, SlackConnectionState, SlackDisconnectFence,
    SlackUserBindingLifecycleError, SlackUserBindingLifecycleStore,
    SlackUserIdentityBindingRollback, SlackUserIdentityCleanupBinding,
};
use crate::slack::slack_serve::{SlackTeamId, SlackUserId};
use crate::slack::slack_setup::{
    SlackInstallationSetup, SlackInstallationSetupStore, SlackSetupError,
};
use ironclaw_channel_host::identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};

const SLACK_HOST_STATE_ROOT: &str = "/tenant-shared/slack-personal-binding";
const SLACK_INSTALLATION_SETUP_PATH: &str = "/tenant-shared/slack-setup/installation.json";
const IDENTITY_ROOT: &str = "/tenant-shared/slack-personal-binding/identities";
const CONNECTION_ROOT: &str = "/tenant-shared/slack-personal-binding/connections";
const LIFECYCLE_CAS_RETRIES: usize = 16;
// Per-(provider, user) inverse index of identity bindings. Primary identity
// records are keyed by `provider_user_id`, so answering "is THIS user bound?"
// otherwise scans every identity. This index lets the connection check
// (WebUI extension listing + Slack activation gate) resolve a bound caller by
// listing only that caller's own bindings. It is maintained best-effort on
// bind/delete: a missing marker only makes the reader fall back to the scan
// (correct, just slower), and the reader verifies the primary record before
// trusting a marker, so a stale marker can never be a false positive.
const IDENTITY_BY_USER_ROOT: &str = "/tenant-shared/slack-personal-binding/identities-by-user";
const CHANNEL_ROUTE_ROOT: &str = "/tenant-shared/slack-channel-routes";
const PERSONAL_DM_TARGET_ROOT: &str = "/tenant-shared/slack-personal-binding/dm-targets";
const CHANNEL_ROUTE_REPLACE_LIST_LIMIT: usize = 500;
const CHANNEL_ROUTE_REPLACE_LOCK_RETRIES: usize = 16;
const CHANNEL_ROUTE_REPLACE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(25);
#[cfg(not(test))]
const CHANNEL_ROUTE_REPLACE_LOCK_TTL_SECONDS: i64 = 10;
#[cfg(test)]
const CHANNEL_ROUTE_REPLACE_LOCK_TTL_SECONDS: i64 = 1;
#[cfg(not(test))]
const CHANNEL_ROUTE_REPLACE_LOCK_RENEW_INTERVAL: Duration = Duration::from_secs(3);
#[cfg(test)]
const CHANNEL_ROUTE_REPLACE_LOCK_RENEW_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
    locks: Arc<ironclaw_channel_host::host_state_records::KeyedAsyncLocks>,
}

impl<F> Clone for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    fn clone(&self) -> Self {
        Self {
            filesystem: Arc::clone(&self.filesystem),
            scope: self.scope.clone(),
            locks: Arc::clone(&self.locks),
        }
    }
}

impl<F> std::fmt::Debug for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemSlackHostState")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            filesystem,
            scope: ResourceScope {
                tenant_id,
                user_id,
                agent_id: Some(agent_id),
                project_id,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            locks: Arc::new(ironclaw_channel_host::host_state_records::KeyedAsyncLocks::default()),
        }
    }

    fn lock_for(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock_for(key)
    }

    async fn read_record<T>(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<(T, RecordVersion)>, FilesystemError>
    where
        T: DeserializeOwned,
    {
        ironclaw_channel_host::host_state_records::read_json_record(
            &self.filesystem,
            &self.scope,
            path,
            "Slack host-state",
        )
        .await
    }

    async fn write_record<T>(
        &self,
        path: &ScopedPath,
        value: &T,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError>
    where
        T: Serialize,
    {
        ironclaw_channel_host::host_state_records::write_json_record(
            &self.filesystem,
            &self.scope,
            path,
            value,
            cas,
            "Slack host-state",
        )
        .await
    }

    async fn delete_record(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        self.filesystem.delete(&self.scope, path).await
    }

    /// Best-effort write of the per-user index marker for a binding. A missing
    /// marker only makes the connection check fall back to a scan (correct, just
    /// slower), so a fault here is logged, not propagated. See
    /// [`IDENTITY_BY_USER_ROOT`].
    async fn write_user_binding_index_marker(&self, binding: &RebornUserIdentityBinding) {
        let path = match Self::identity_user_index_path(
            binding.provider.as_str(),
            binding.user_id.as_str(),
            binding.provider_user_id.as_str(),
        ) {
            Ok(path) => path,
            Err(error) => {
                tracing::debug!(%error, "could not build Slack user-binding index path");
                return;
            }
        };
        let marker = StoredUserBindingIndexMarker {
            provider_user_id: binding.provider_user_id.as_str().to_string(),
        };
        if let Err(error) = self.write_record(&path, &marker, CasExpectation::Any).await {
            tracing::debug!(
                %error,
                "failed to write Slack user-binding index marker; connection check will fall back to a scan"
            );
        }
    }

    /// Best-effort delete of a per-user index marker. A stale marker cannot
    /// cause a false positive (the reader verifies the primary record), so a
    /// delete fault is logged, not propagated.
    async fn delete_user_binding_index_marker(
        &self,
        provider: &str,
        user_id: &str,
        provider_user_id: &str,
    ) {
        let path = match Self::identity_user_index_path(provider, user_id, provider_user_id) {
            Ok(path) => path,
            Err(_) => return,
        };
        match self.delete_record(&path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => {
                tracing::debug!(%error, "failed to delete Slack user-binding index marker");
            }
        }
    }

    /// Fast-path connection check via the per-user index. Returns `true` only
    /// after verifying the primary record still exists and matches (so a stale
    /// marker is never a false positive). Returns `false` when the index has no
    /// verified match; the caller falls back to the full scan because bindings
    /// written before this index existed have no marker.
    async fn user_binding_via_index_marker(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        let dir = Self::identity_user_index_dir(provider, user_id.as_str())
            .map_err(map_lookup_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(false),
            Err(error) => return Err(map_lookup_fs_error(error)),
        };
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            // The marker file name is `path_segment(provider_user_id).json`,
            // identical to the primary record's file name, so the primary path
            // is the identity dir plus this entry name; no decoding needed.
            let primary = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_lookup_fs_error)?;
            let Some((record, _)) = self
                .read_record::<StoredSlackUserIdentity>(&primary)
                .await
                .map_err(map_lookup_fs_error)?
            else {
                // Stale marker (primary gone); skip. Verifying the primary is
                // what keeps a failed delete-marker from becoming a false
                // positive.
                continue;
            };
            if identity_record_matches_user_binding(
                &record,
                provider,
                user_id,
                provider_user_id_prefix,
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn acquire_channel_route_replace_lease(
        &self,
        installation_id: &AdapterInstallationId,
        team_id: &str,
    ) -> Result<SlackChannelRouteReplaceLease, SlackChannelRouteError> {
        let path = Self::channel_route_team_replace_lock_path(installation_id, team_id)
            .map_err(map_route_fs_error)?;
        for _ in 0..CHANNEL_ROUTE_REPLACE_LOCK_RETRIES {
            let nonce = random_lock_nonce();
            let record = StoredSlackChannelRouteReplaceLock::new(nonce.clone());
            match self
                .write_record(&path, &record, CasExpectation::Absent)
                .await
            {
                Ok(_) => {
                    return Ok(SlackChannelRouteReplaceLease {
                        path: path.clone(),
                        nonce,
                    });
                }
                Err(FilesystemError::VersionMismatch { .. }) => {
                    if self
                        .try_steal_expired_channel_route_replace_lease(&path, &nonce)
                        .await?
                    {
                        return Ok(SlackChannelRouteReplaceLease {
                            path: path.clone(),
                            nonce,
                        });
                    }
                    tokio::time::sleep(CHANNEL_ROUTE_REPLACE_LOCK_RETRY_DELAY).await;
                }
                Err(error) => return Err(map_route_fs_error(error)),
            }
        }
        Err(SlackChannelRouteError::StoreUnavailable)
    }

    async fn try_steal_expired_channel_route_replace_lease(
        &self,
        path: &ScopedPath,
        nonce: &str,
    ) -> Result<bool, SlackChannelRouteError> {
        let Some((record, version)) = self
            .read_record::<StoredSlackChannelRouteReplaceLock>(path)
            .await
            .map_err(map_route_fs_error)?
        else {
            return Ok(false);
        };
        if record.expires_at > Utc::now() {
            return Ok(false);
        }
        let replacement = StoredSlackChannelRouteReplaceLock::new(nonce.to_string());
        match self
            .write_record(path, &replacement, CasExpectation::Version(version))
            .await
        {
            Ok(_) => Ok(true),
            Err(FilesystemError::VersionMismatch { .. }) => Ok(false),
            Err(error) => Err(map_route_fs_error(error)),
        }
    }

    async fn release_channel_route_replace_lease(&self, lease: SlackChannelRouteReplaceLease) {
        let current = self
            .read_record::<StoredSlackChannelRouteReplaceLock>(&lease.path)
            .await;
        let Ok(Some((record, version))) = current else {
            return;
        };
        if record.nonce != lease.nonce {
            return;
        }
        let expired = StoredSlackChannelRouteReplaceLock::expired(lease.nonce);
        match self
            .write_record(&lease.path, &expired, CasExpectation::Version(version))
            .await
        {
            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "failed to expire Slack channel route replacement lease"
                );
            }
        }
    }

    async fn renew_channel_route_replace_lease(
        &self,
        lease: &SlackChannelRouteReplaceLease,
    ) -> Result<(), SlackChannelRouteError> {
        let Some((record, version)) = self
            .read_record::<StoredSlackChannelRouteReplaceLock>(&lease.path)
            .await
            .map_err(map_route_fs_error)?
        else {
            return Err(SlackChannelRouteError::StoreUnavailable);
        };
        if record.nonce != lease.nonce {
            return Err(SlackChannelRouteError::StoreUnavailable);
        }
        let renewed = StoredSlackChannelRouteReplaceLock::new(lease.nonce.clone());
        match self
            .write_record(&lease.path, &renewed, CasExpectation::Version(version))
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => Err(map_route_fs_error(error)),
        }
    }

    async fn with_channel_route_replace_lease<T, Fut>(
        &self,
        installation_id: &AdapterInstallationId,
        team_id: &str,
        operation: Fut,
    ) -> Result<T, SlackChannelRouteError>
    where
        Fut: Future<Output = Result<T, SlackChannelRouteError>>,
    {
        let lease = self
            .acquire_channel_route_replace_lease(installation_id, team_id)
            .await?;
        let mut renewer = ChannelRouteReplaceLeaseRenewer::start(self.clone(), lease.clone());
        let mut result = tokio::select! {
            result = operation => result,
            error = renewer.failed() => Err(error),
        };
        if let Err(error) = renewer.stop().await
            && result.is_ok()
        {
            result = Err(error);
        }
        self.release_channel_route_replace_lease(lease).await;
        result
    }

    async fn restore_channel_route_snapshot(
        &self,
        tenant_id: &TenantId,
        installation_id: &AdapterInstallationId,
        team_id: &str,
        snapshot: &HashMap<String, UserId>,
        touched_channels: &[String],
    ) {
        for channel_id in touched_channels {
            let key = match SlackChannelRouteKey::new(
                tenant_id.clone(),
                installation_id.clone(),
                team_id.to_string(),
                channel_id.clone(),
            ) {
                Ok(key) => key,
                Err(error) => {
                    tracing::warn!(?error, %channel_id, "failed to rebuild Slack channel route rollback key");
                    continue;
                }
            };
            let result = if let Some(subject_user_id) = snapshot.get(channel_id) {
                self.upsert_route_record(key, subject_user_id.clone())
                    .await
                    .map(|_| ())
            } else {
                self.delete_route_record(&key).await.map(|_| ())
            };
            if let Err(error) = result {
                tracing::warn!(?error, %channel_id, "failed to roll back Slack channel route replacement");
            }
        }
    }

    async fn replace_managed_routes_while_lease_active(
        &self,
        tenant_id: &TenantId,
        installation_id: &AdapterInstallationId,
        team_id: &str,
        assignments: Vec<SlackChannelRouteAssignment>,
        renewer: &mut ChannelRouteReplaceLeaseRenewer<F>,
    ) -> Result<Vec<SlackChannelRoute>, SlackChannelRouteError> {
        let requested = assignments
            .iter()
            .map(|assignment| assignment.channel_id.clone())
            .collect::<std::collections::HashSet<_>>();
        let mut existing_routes = Vec::new();
        let mut cursor = 0;
        loop {
            renewer.ensure_active()?;
            let page = self
                .list_routes(
                    tenant_id,
                    installation_id,
                    team_id,
                    cursor,
                    CHANNEL_ROUTE_REPLACE_LIST_LIMIT,
                )
                .await?;
            renewer.ensure_active()?;
            existing_routes.extend(page.routes);
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            if next_cursor <= cursor {
                return Err(SlackChannelRouteError::StoreUnavailable);
            }
            cursor = next_cursor;
        }
        let mut snapshot = HashMap::new();
        for route in &existing_routes {
            snapshot.insert(
                route.channel_id.clone(),
                UserId::new(route.subject_user_id.clone())
                    .map_err(|_| SlackChannelRouteError::StoreUnavailable)?,
            );
        }
        let mut replaced = Vec::with_capacity(assignments.len());
        let mut touched_channels = Vec::new();
        for assignment in assignments {
            let channel_id = assignment.channel_id.clone();
            let key = SlackChannelRouteKey::new(
                tenant_id.clone(),
                installation_id.clone(),
                team_id.to_string(),
                assignment.channel_id,
            )?;
            renewer.ensure_active()?;
            match self
                .upsert_route_record(key, assignment.subject_user_id)
                .await
            {
                Ok(route) => {
                    touched_channels.push(channel_id);
                    if let Err(error) = renewer.ensure_active() {
                        self.restore_channel_route_snapshot(
                            tenant_id,
                            installation_id,
                            team_id,
                            &snapshot,
                            &touched_channels,
                        )
                        .await;
                        return Err(error);
                    }
                    replaced.push(route);
                }
                Err(error) => {
                    self.restore_channel_route_snapshot(
                        tenant_id,
                        installation_id,
                        team_id,
                        &snapshot,
                        &touched_channels,
                    )
                    .await;
                    return Err(error);
                }
            }
        }
        for route in existing_routes {
            if !requested.contains(&route.channel_id) {
                let channel_id = route.channel_id.clone();
                let key = SlackChannelRouteKey::new(
                    tenant_id.clone(),
                    installation_id.clone(),
                    team_id.to_string(),
                    route.channel_id,
                )?;
                renewer.ensure_active()?;
                if let Err(error) = self.delete_route_record(&key).await {
                    self.restore_channel_route_snapshot(
                        tenant_id,
                        installation_id,
                        team_id,
                        &snapshot,
                        &touched_channels,
                    )
                    .await;
                    return Err(error);
                }
                touched_channels.push(channel_id);
                if let Err(error) = renewer.ensure_active() {
                    self.restore_channel_route_snapshot(
                        tenant_id,
                        installation_id,
                        team_id,
                        &snapshot,
                        &touched_channels,
                    )
                    .await;
                    return Err(error);
                }
            }
        }
        replaced.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
        Ok(replaced)
    }

    fn identity_path(
        provider: &str,
        provider_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}.json",
            IDENTITY_ROOT,
            path_segment(provider),
            path_segment(provider_user_id)
        ))
    }

    fn connection_path(owner: &SlackConnectionOwner) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}.json",
            CONNECTION_ROOT,
            path_segment(owner.installation_id().as_str()),
            path_segment(owner.user_id().as_str())
        ))
    }

    fn identity_user_index_dir(
        provider: &str,
        user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}",
            IDENTITY_BY_USER_ROOT,
            path_segment(provider),
            path_segment(user_id)
        ))
    }

    fn identity_user_index_path(
        provider: &str,
        user_id: &str,
        provider_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        // The marker file name reuses `path_segment(provider_user_id)`, exactly
        // like the primary record, so the primary path can be rebuilt from a
        // marker entry name without decoding.
        scoped_path(&format!(
            "{}/{}/{}/{}.json",
            IDENTITY_BY_USER_ROOT,
            path_segment(provider),
            path_segment(user_id),
            path_segment(provider_user_id)
        ))
    }

    fn channel_route_team_dir_path(
        installation_id: &AdapterInstallationId,
        team_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}",
            CHANNEL_ROUTE_ROOT,
            path_segment(installation_id.as_str()),
            path_segment(team_id)
        ))
    }

    fn channel_route_team_replace_lock_path(
        installation_id: &AdapterInstallationId,
        team_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}/replace-lock",
            CHANNEL_ROUTE_ROOT,
            path_segment(installation_id.as_str()),
            path_segment(team_id)
        ))
    }

    fn channel_route_path(key: &SlackChannelRouteKey) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}/{}.json",
            CHANNEL_ROUTE_ROOT,
            path_segment(key.installation_id.as_str()),
            path_segment(&key.team_id),
            path_segment(&key.channel_id)
        ))
    }

    fn personal_dm_target_path(
        key: &SlackPersonalDmTargetKey,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}/{}.json",
            PERSONAL_DM_TARGET_ROOT,
            path_segment(key.installation_id.as_str()),
            path_segment(key.team_id.as_str()),
            path_segment(key.user_id.as_str())
        ))
    }

    fn listed_channel_route_path(
        installation_id: &AdapterInstallationId,
        team_id: &str,
        entry_name: &str,
    ) -> Result<Option<ScopedPath>, FilesystemError> {
        let Some(stem) = entry_name.strip_suffix(".json") else {
            return Ok(None);
        };
        let decoded = match URL_SAFE_NO_PAD.decode(stem.as_bytes()) {
            Ok(decoded) => decoded,
            Err(_) => return Ok(None),
        };
        let Ok(channel_id) = String::from_utf8(decoded) else {
            return Ok(None);
        };
        let canonical_name = format!("{}.json", path_segment(&channel_id));
        if canonical_name != entry_name {
            return Ok(None);
        }
        scoped_path(&format!(
            "{}/{}/{}/{}",
            CHANNEL_ROUTE_ROOT,
            path_segment(installation_id.as_str()),
            path_segment(team_id),
            canonical_name
        ))
        .map(Some)
    }

    fn channel_route_team_lock_key(
        installation_id: &AdapterInstallationId,
        team_id: &str,
    ) -> String {
        format!(
            "channel-route-team:{}:{}",
            installation_id.as_str(),
            team_id
        )
    }

    fn channel_route_lock_key(key: &SlackChannelRouteKey) -> String {
        format!(
            "channel-route:{}:{}:{}",
            key.installation_id.as_str(),
            key.team_id,
            key.channel_id
        )
    }

    async fn upsert_route_record(
        &self,
        key: SlackChannelRouteKey,
        subject_user_id: UserId,
    ) -> Result<SlackChannelRoute, SlackChannelRouteError> {
        let path = Self::channel_route_path(&key).map_err(map_route_fs_error)?;
        let lock = self.lock_for(Self::channel_route_lock_key(&key));
        let _guard = lock.lock().await;
        let record = StoredSlackChannelRoute::new(&key, &subject_user_id);
        self.write_record(&path, &record, CasExpectation::Any)
            .await
            .map_err(map_route_fs_error)?;
        Ok(SlackChannelRoute::new(key, subject_user_id))
    }

    async fn delete_route_record(
        &self,
        key: &SlackChannelRouteKey,
    ) -> Result<bool, SlackChannelRouteError> {
        let path = Self::channel_route_path(key).map_err(map_route_fs_error)?;
        let lock = self.lock_for(Self::channel_route_lock_key(key));
        let _guard = lock.lock().await;
        match self.delete_record(&path).await {
            Ok(()) => Ok(true),
            Err(FilesystemError::NotFound { .. }) => Ok(false),
            Err(error) if is_unsupported_delete_error(&error) => {
                let Some((mut record, _)) = self
                    .read_record::<StoredSlackChannelRoute>(&path)
                    .await
                    .map_err(map_route_fs_error)?
                else {
                    return Ok(false);
                };
                record.deleted_at = Some(Utc::now());
                record.updated_at = Utc::now();
                self.write_record(&path, &record, CasExpectation::Any)
                    .await
                    .map_err(map_route_fs_error)?;
                Ok(true)
            }
            Err(error) => Err(map_route_fs_error(error)),
        }
    }
}

#[async_trait::async_trait]
impl<F> SlackInstallationSetupStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn get_slack_installation_setup(
        &self,
    ) -> Result<Option<SlackInstallationSetup>, SlackSetupError> {
        let path = ScopedPath::new(SLACK_INSTALLATION_SETUP_PATH)
            .map_err(|_| SlackSetupError::StoreUnavailable)?;
        self.read_record(&path)
            .await
            .map(|record| record.map(|(setup, _)| setup))
            .map_err(map_setup_fs_error)
    }

    async fn put_slack_installation_setup(
        &self,
        setup: &SlackInstallationSetup,
    ) -> Result<(), SlackSetupError> {
        let path = ScopedPath::new(SLACK_INSTALLATION_SETUP_PATH)
            .map_err(|_| SlackSetupError::StoreUnavailable)?;
        let lock = self.lock_for("slack-installation-setup".to_string());
        let _guard = lock.lock().await;
        let cas = self
            .read_record::<SlackInstallationSetup>(&path)
            .await
            .map_err(map_setup_fs_error)?
            .map(|(_, version)| CasExpectation::Version(version))
            .unwrap_or(CasExpectation::Absent);
        self.write_record(&path, setup, cas)
            .await
            .map(|_| ())
            .map_err(map_setup_fs_error)
    }

    async fn delete_slack_installation_setup(&self) -> Result<(), SlackSetupError> {
        let path = ScopedPath::new(SLACK_INSTALLATION_SETUP_PATH)
            .map_err(|_| SlackSetupError::StoreUnavailable)?;
        let lock = self.lock_for("slack-installation-setup".to_string());
        let _guard = lock.lock().await;
        match self.delete_record(&path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_setup_fs_error(error)),
        }
    }
}

#[async_trait::async_trait]
impl<F> SlackUserBindingLifecycleStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn begin_connection(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
        expires_at: ironclaw_auth::Timestamp,
    ) -> Result<(), SlackUserBindingLifecycleError> {
        self.validate_connection_owner(owner)?;
        let path = Self::connection_path(owner).map_err(map_lifecycle_fs_error)?;
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let current = self
                .read_record::<StoredSlackConnection>(&path)
                .await
                .map_err(map_lifecycle_fs_error)?;
            let (record, cas) = match current {
                None => (
                    StoredSlackConnection::new(
                        owner,
                        epoch,
                        SlackConnectionState::Connecting,
                        Utc::now(),
                        expires_at,
                    ),
                    CasExpectation::Absent,
                ),
                Some((record, version)) => {
                    record.validate(owner)?;
                    match record.state {
                        SlackConnectionState::Connecting if record.epoch == epoch => return Ok(()),
                        SlackConnectionState::Connecting if record.expires_at <= Utc::now() => (
                            StoredSlackConnection::new(
                                owner,
                                epoch,
                                SlackConnectionState::Connecting,
                                Utc::now(),
                                expires_at,
                            ),
                            CasExpectation::Version(version),
                        ),
                        SlackConnectionState::Connecting => {
                            return Err(SlackUserBindingLifecycleError::ConnectionInProgress);
                        }
                        SlackConnectionState::Active => match record.pending_connection {
                            Some(pending) if pending.epoch == epoch => return Ok(()),
                            Some(pending) if pending.expires_at > Utc::now() => {
                                return Err(SlackUserBindingLifecycleError::ConnectionInProgress);
                            }
                            Some(_) | None => (
                                record.with_pending_connection(epoch, expires_at),
                                CasExpectation::Version(version),
                            ),
                        },
                        SlackConnectionState::Disconnecting => {
                            return Err(SlackUserBindingLifecycleError::DisconnectInProgress);
                        }
                        SlackConnectionState::Disconnected => (
                            StoredSlackConnection::new(
                                owner,
                                epoch,
                                SlackConnectionState::Connecting,
                                Utc::now(),
                                expires_at,
                            ),
                            CasExpectation::Version(version),
                        ),
                    }
                }
            };
            match self.write_record(&path, &record, cas).await {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_lifecycle_fs_error(error)),
            }
        }
        Err(SlackUserBindingLifecycleError::Backend(
            "Slack connection state changed concurrently".to_string(),
        ))
    }

    async fn connection_state(
        &self,
        owner: &SlackConnectionOwner,
    ) -> Result<Option<(SlackConnectionEpoch, SlackConnectionState)>, SlackUserBindingLifecycleError>
    {
        self.validate_connection_owner(owner)?;
        let path = Self::connection_path(owner).map_err(map_lifecycle_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredSlackConnection>(&path)
            .await
            .map_err(map_lifecycle_fs_error)?
        else {
            return Ok(None);
        };
        record.validate(owner)?;
        Ok(Some(record.visible_connection_state()))
    }

    async fn connection_owner_for_epoch(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        epoch: SlackConnectionEpoch,
    ) -> Result<Option<SlackConnectionOwner>, SlackUserBindingLifecycleError> {
        if tenant_id != &self.scope.tenant_id {
            return Err(SlackUserBindingLifecycleError::Backend(
                "Slack connection owner is outside the tenant scope".to_string(),
            ));
        }
        let root = scoped_path(CONNECTION_ROOT).map_err(map_lifecycle_fs_error)?;
        let installation_entries = match self.filesystem.list_dir(&self.scope, &root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(map_lifecycle_fs_error(error)),
        };
        let mut matched_owner = None;
        for installation_entry in installation_entries {
            if installation_entry.file_type != FileType::Directory {
                continue;
            }
            let path = scoped_path(&format!(
                "{}/{}/{}.json",
                CONNECTION_ROOT,
                installation_entry.name,
                path_segment(user_id.as_str())
            ))
            .map_err(map_lifecycle_fs_error)?;
            let Some((record, _)) = self
                .read_record::<StoredSlackConnection>(&path)
                .await
                .map_err(map_lifecycle_fs_error)?
            else {
                continue;
            };
            if record.tenant_id != tenant_id.as_str()
                || record.user_id != user_id.as_str()
                || !record.owns_connection_epoch(epoch)
            {
                continue;
            }
            if path_segment(&record.installation_id) != installation_entry.name {
                return Err(SlackUserBindingLifecycleError::Backend(
                    "stored Slack connection path does not match its owner".to_string(),
                ));
            }
            let installation_id = AdapterInstallationId::new(record.installation_id.clone())
                .map_err(|error| SlackUserBindingLifecycleError::Backend(error.to_string()))?;
            let owner =
                SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id);
            record.validate(&owner)?;
            if matched_owner.replace(owner).is_some() {
                return Err(SlackUserBindingLifecycleError::Backend(
                    "Slack connection epoch is assigned to multiple owners".to_string(),
                ));
            }
        }
        Ok(matched_owner)
    }

    async fn connection_owners_for_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<Vec<SlackConnectionOwner>, SlackUserBindingLifecycleError> {
        if tenant_id != &self.scope.tenant_id {
            return Err(SlackUserBindingLifecycleError::Backend(
                "Slack connection owner is outside the tenant scope".to_string(),
            ));
        }
        let root = scoped_path(CONNECTION_ROOT).map_err(map_lifecycle_fs_error)?;
        let installation_entries = match self.filesystem.list_dir(&self.scope, &root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(map_lifecycle_fs_error(error)),
        };
        let mut owners = Vec::new();
        for installation_entry in installation_entries {
            if installation_entry.file_type != FileType::Directory {
                continue;
            }
            let path = scoped_path(&format!(
                "{}/{}/{}.json",
                CONNECTION_ROOT,
                installation_entry.name,
                path_segment(user_id.as_str())
            ))
            .map_err(map_lifecycle_fs_error)?;
            let Some((record, _)) = self
                .read_record::<StoredSlackConnection>(&path)
                .await
                .map_err(map_lifecycle_fs_error)?
            else {
                continue;
            };
            if record.tenant_id != tenant_id.as_str()
                || record.user_id != user_id.as_str()
                || path_segment(&record.installation_id) != installation_entry.name
            {
                return Err(SlackUserBindingLifecycleError::Backend(
                    "stored Slack connection path does not match its owner".to_string(),
                ));
            }
            let installation_id = AdapterInstallationId::new(record.installation_id.clone())
                .map_err(|error| SlackUserBindingLifecycleError::Backend(error.to_string()))?;
            let owner =
                SlackConnectionOwner::new(tenant_id.clone(), user_id.clone(), installation_id);
            record.validate(&owner)?;
            owners.push(owner);
        }
        Ok(owners)
    }

    async fn begin_disconnect(
        &self,
        owner: &SlackConnectionOwner,
    ) -> Result<SlackDisconnectFence, SlackUserBindingLifecycleError> {
        self.validate_connection_owner(owner)?;
        let path = Self::connection_path(owner).map_err(map_lifecycle_fs_error)?;
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let current = self
                .read_record::<StoredSlackConnection>(&path)
                .await
                .map_err(map_lifecycle_fs_error)?;
            let (updated, cas, fence) = match current {
                None => {
                    let now = Utc::now();
                    let fence_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
                    let fence = SlackDisconnectFence::new(
                        fence_epoch,
                        SlackConnectionCleanupSelector::AllOwned,
                    );
                    (
                        StoredSlackConnection::new(
                            owner,
                            fence_epoch,
                            SlackConnectionState::Disconnecting,
                            now,
                            now,
                        )
                        .with_disconnect_cleanup(StoredSlackDisconnectCleanup::AllOwned),
                        CasExpectation::Absent,
                        fence,
                    )
                }
                Some((record, version)) => {
                    record.validate(owner)?;
                    match record.state {
                        SlackConnectionState::Disconnecting => {
                            return Ok(record.disconnect_fence());
                        }
                        SlackConnectionState::Connecting | SlackConnectionState::Active => {
                            let fence = SlackDisconnectFence::new(
                                record.epoch,
                                SlackConnectionCleanupSelector::Epoch(record.epoch),
                            );
                            (
                                record
                                    .with_state(SlackConnectionState::Disconnecting)
                                    .without_pending_connection()
                                    .with_disconnect_cleanup(StoredSlackDisconnectCleanup::Epoch(
                                        record.epoch,
                                    )),
                                CasExpectation::Version(version),
                                fence,
                            )
                        }
                        SlackConnectionState::Disconnected => {
                            // A legacy binding, or a failed old rollback, may
                            // survive a disconnected record. Fence with a new
                            // generation and clean all owner state.
                            let now = Utc::now();
                            let fence_epoch =
                                SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
                            let fence = SlackDisconnectFence::new(
                                fence_epoch,
                                SlackConnectionCleanupSelector::AllOwned,
                            );
                            (
                                StoredSlackConnection::new(
                                    owner,
                                    fence_epoch,
                                    SlackConnectionState::Disconnecting,
                                    now,
                                    now,
                                )
                                .with_disconnect_cleanup(StoredSlackDisconnectCleanup::AllOwned),
                                CasExpectation::Version(version),
                                fence,
                            )
                        }
                    }
                }
            };
            match self.write_record(&path, &updated, cas).await {
                Ok(_) => return Ok(fence),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_lifecycle_fs_error(error)),
            }
        }
        Err(SlackUserBindingLifecycleError::Backend(
            "Slack connection state changed concurrently".to_string(),
        ))
    }

    async fn complete_disconnect(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<(), SlackUserBindingLifecycleError> {
        self.transition_connection_to_disconnected(owner, epoch, false)
            .await
    }

    async fn begin_failed_connection_cleanup(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<(), SlackUserBindingLifecycleError> {
        self.validate_connection_owner(owner)?;
        let path = Self::connection_path(owner).map_err(map_lifecycle_fs_error)?;
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let Some((record, version)) = self
                .read_record::<StoredSlackConnection>(&path)
                .await
                .map_err(map_lifecycle_fs_error)?
            else {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            };
            record.validate(owner)?;
            if record.state == SlackConnectionState::Active
                && record
                    .pending_connection
                    .is_some_and(|pending| pending.epoch == epoch)
            {
                // A pending replacement is already excluded by the active
                // epoch check at ingress. Keep it attached so retry can still
                // recover the owner after an identity-store failure.
                return Ok(());
            }
            if record.epoch != epoch {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            }
            if matches!(
                record.state,
                SlackConnectionState::Disconnecting | SlackConnectionState::Disconnected
            ) {
                return Ok(());
            }
            let updated = record
                .with_state(SlackConnectionState::Disconnecting)
                .with_disconnect_cleanup(StoredSlackDisconnectCleanup::Epoch(epoch));
            match self
                .write_record(&path, &updated, CasExpectation::Version(version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_lifecycle_fs_error(error)),
            }
        }
        Err(SlackUserBindingLifecycleError::Backend(
            "Slack connection state changed concurrently".to_string(),
        ))
    }

    async fn complete_failed_connection_cleanup(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<(), SlackUserBindingLifecycleError> {
        self.validate_connection_owner(owner)?;
        let path = Self::connection_path(owner).map_err(map_lifecycle_fs_error)?;
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let Some((record, version)) = self
                .read_record::<StoredSlackConnection>(&path)
                .await
                .map_err(map_lifecycle_fs_error)?
            else {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            };
            record.validate(owner)?;
            let updated = if record.state == SlackConnectionState::Active
                && record
                    .pending_connection
                    .is_some_and(|pending| pending.epoch == epoch)
            {
                // A failed reconfigure must leave the previous active epoch
                // usable after the replacement identity has been removed.
                record.without_pending_connection()
            } else if record.epoch == epoch && record.state == SlackConnectionState::Disconnecting {
                record
                    .with_state(SlackConnectionState::Disconnected)
                    .without_pending_connection()
            } else if record.epoch == epoch && record.state == SlackConnectionState::Disconnected {
                return Ok(());
            } else {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            };
            match self
                .write_record(&path, &updated, CasExpectation::Version(version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_lifecycle_fs_error(error)),
            }
        }
        Err(SlackUserBindingLifecycleError::Backend(
            "Slack connection state changed concurrently".to_string(),
        ))
    }

    async fn abandon_connection(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<(), SlackUserBindingLifecycleError> {
        self.transition_connection_to_disconnected(owner, epoch, true)
            .await
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    fn validate_connection_owner(
        &self,
        owner: &SlackConnectionOwner,
    ) -> Result<(), SlackUserBindingLifecycleError> {
        if owner.tenant_id() != &self.scope.tenant_id {
            return Err(SlackUserBindingLifecycleError::Backend(
                "Slack connection owner is outside the tenant scope".to_string(),
            ));
        }
        Ok(())
    }

    async fn connection_is_active_at_epoch_for_owner(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<bool, SlackUserBindingLifecycleError> {
        self.validate_connection_owner(owner)?;
        let path = Self::connection_path(owner).map_err(map_lifecycle_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredSlackConnection>(&path)
            .await
            .map_err(map_lifecycle_fs_error)?
        else {
            return Ok(false);
        };
        record.validate(owner)?;
        Ok(record.state == SlackConnectionState::Active && record.epoch == epoch)
    }

    async fn activate_connection(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackConnectionActivation, SlackUserBindingLifecycleError> {
        self.validate_connection_owner(owner)?;
        let path = Self::connection_path(owner).map_err(map_lifecycle_fs_error)?;
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let Some((record, version)) = self
                .read_record::<StoredSlackConnection>(&path)
                .await
                .map_err(map_lifecycle_fs_error)?
            else {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            };
            record.validate(owner)?;
            let updated = match record.state {
                SlackConnectionState::Active if record.epoch == epoch => {
                    return Ok(SlackConnectionActivation {
                        previous: None,
                        written_version: version,
                    });
                }
                SlackConnectionState::Active
                    if record
                        .pending_connection
                        .is_some_and(|pending| pending.epoch == epoch) =>
                {
                    record
                        .promote_pending_connection()
                        .ok_or(SlackUserBindingLifecycleError::StaleEpoch)?
                }
                SlackConnectionState::Connecting if record.epoch == epoch => {
                    record.with_state(SlackConnectionState::Active)
                }
                SlackConnectionState::Active
                | SlackConnectionState::Connecting
                | SlackConnectionState::Disconnecting
                | SlackConnectionState::Disconnected => {
                    return Err(SlackUserBindingLifecycleError::StaleEpoch);
                }
            };
            match self
                .write_record(&path, &updated, CasExpectation::Version(version))
                .await
            {
                Ok(written_version) => {
                    return Ok(SlackConnectionActivation {
                        previous: Some(record),
                        written_version,
                    });
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_lifecycle_fs_error(error)),
            }
        }
        Err(SlackUserBindingLifecycleError::Backend(
            "Slack connection state changed concurrently".to_string(),
        ))
    }

    async fn transition_connection_to_disconnected(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
        allow_pre_disconnect_states: bool,
    ) -> Result<(), SlackUserBindingLifecycleError> {
        self.validate_connection_owner(owner)?;
        let path = Self::connection_path(owner).map_err(map_lifecycle_fs_error)?;
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let Some((record, version)) = self
                .read_record::<StoredSlackConnection>(&path)
                .await
                .map_err(map_lifecycle_fs_error)?
            else {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            };
            record.validate(owner)?;
            if allow_pre_disconnect_states
                && record.state == SlackConnectionState::Active
                && record
                    .pending_connection
                    .is_some_and(|pending| pending.epoch == epoch)
            {
                match self
                    .write_record(
                        &path,
                        &record.without_pending_connection(),
                        CasExpectation::Version(version),
                    )
                    .await
                {
                    Ok(_) => return Ok(()),
                    Err(FilesystemError::VersionMismatch { .. }) => continue,
                    Err(error) => return Err(map_lifecycle_fs_error(error)),
                }
            }
            if record.epoch != epoch {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            }
            if record.state == SlackConnectionState::Disconnected {
                return Ok(());
            }
            if allow_pre_disconnect_states && record.state == SlackConnectionState::Disconnecting {
                // Disconnect owns this transition and must keep ingress fenced
                // until all credential, DM, pairing, and identity cleanup has
                // completed.
                return Ok(());
            }
            if !allow_pre_disconnect_states && record.state != SlackConnectionState::Disconnecting {
                return Err(SlackUserBindingLifecycleError::StaleEpoch);
            }
            let updated = record
                .with_state(SlackConnectionState::Disconnected)
                .without_pending_connection();
            match self
                .write_record(&path, &updated, CasExpectation::Version(version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_lifecycle_fs_error(error)),
            }
        }
        Err(SlackUserBindingLifecycleError::Backend(
            "Slack connection state changed concurrently".to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl<F> RebornUserIdentityLookup for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        self.resolve_user_identity_with_binding_epoch(provider, provider_user_id)
            .await
            .map(|resolved| resolved.map(|(user_id, _)| user_id))
    }

    async fn resolve_user_identity_with_binding_epoch(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<
        Option<(
            UserId,
            Option<ironclaw_conversations::ExternalActorBindingEpoch>,
        )>,
        RebornUserIdentityLookupError,
    > {
        let path = Self::identity_path(provider, provider_user_id).map_err(map_lookup_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredSlackUserIdentity>(&path)
            .await
            .map_err(map_lookup_fs_error)?
        else {
            return Ok(None);
        };
        record
            .validate_for_key(provider, provider_user_id)
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
        if record.state != StoredSlackIdentityState::Active {
            return Ok(None);
        }
        let user_id = UserId::new(record.user_id.clone())
            .map_err(|error| RebornUserIdentityLookupError::InvalidUserId(error.to_string()))?;
        let Some(epoch) = record.epoch else {
            return Ok(Some((user_id, None)));
        };
        if provider != crate::slack::slack_actor_identity::SLACK_IDENTITY_PROVIDER {
            return Err(RebornUserIdentityLookupError::Backend(
                "only Slack identities may carry Slack connection epochs".to_string(),
            ));
        }
        let Some((installation_id, _)) =
            parse_slack_user_identity_provider_user_id(provider_user_id)
        else {
            return Err(RebornUserIdentityLookupError::Backend(
                "stored Slack provider user identity is malformed".to_string(),
            ));
        };
        let owner = SlackConnectionOwner::new(
            self.scope.tenant_id.clone(),
            user_id.clone(),
            installation_id,
        );
        if !self
            .connection_is_active_at_epoch_for_owner(&owner, epoch)
            .await
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?
        {
            return Ok(None);
        }
        let binding_epoch =
            ironclaw_conversations::ExternalActorBindingEpoch::new(epoch.to_string())
                .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
        Ok(Some((user_id, Some(binding_epoch))))
    }

    async fn user_identity_binding_epoch_is_current(
        &self,
        provider: &str,
        provider_user_id: &str,
        expected_user_id: &UserId,
        expected_epoch: &ironclaw_conversations::ExternalActorBindingEpoch,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        self.resolve_user_identity_with_binding_epoch(provider, provider_user_id)
            .await
            .map(|resolved| {
                resolved.is_some_and(|(user_id, epoch)| {
                    user_id == *expected_user_id && epoch.as_ref() == Some(expected_epoch)
                })
            })
    }

    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        self.user_has_provider_binding_with_provider_user_id_prefix(provider, user_id, None)
            .await
    }

    async fn user_has_provider_binding_with_provider_user_id_prefix(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        // Fast path: the per-user index resolves a bound caller by listing only
        // that caller's own bindings. A verified `true` short-circuits the scan;
        // a miss is inconclusive (bindings predating the index have no marker),
        // so fall through to the full scan below.
        if self
            .user_binding_via_index_marker(provider, user_id, provider_user_id_prefix)
            .await?
        {
            return Ok(true);
        }
        let provider_dir = scoped_path(&format!("{IDENTITY_ROOT}/{}", path_segment(provider)))
            .map_err(map_lookup_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &provider_dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(false),
            Err(error) => return Err(map_lookup_fs_error(error)),
        };
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let path = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_lookup_fs_error)?;
            let Some((record, _)) = self
                .read_record::<StoredSlackUserIdentity>(&path)
                .await
                .map_err(map_lookup_fs_error)?
            else {
                continue;
            };
            if identity_record_matches_user_binding(
                &record,
                provider,
                user_id,
                provider_user_id_prefix,
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[async_trait::async_trait]
impl<F> RebornUserIdentityBindingStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    #[cfg(test)]
    async fn bind_user_identity(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        self.bind_user_identity_inner(binding, None)
            .await
            .map(|_| ())
    }

    async fn bind_user_identity_for_epoch(
        &self,
        binding: RebornUserIdentityBinding,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackUserIdentityBindingRollback, RebornUserIdentityBindingError> {
        self.bind_user_identity_inner(binding, Some(epoch))
            .await?
            .ok_or_else(|| {
                RebornUserIdentityBindingError::Backend(
                    "Slack epoch binding did not create a rollback guard".to_string(),
                )
            })
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn rollback_epoch_identity_binding(
        &self,
        rollback: SlackIdentityBindingRollbackContext,
    ) -> Result<(), RebornUserIdentityBindingError> {
        let SlackIdentityBindingRollbackContext {
            owner,
            failed_epoch,
            identity_path,
            binding,
            identity_written_version,
            previous_identity,
            activation,
        } = rollback;
        let connection_path = Self::connection_path(&owner)
            .map_err(|error| RebornUserIdentityBindingError::Backend(error.to_string()))?;
        let restorable_connection = activation.previous.as_ref().and_then(|previous| {
            (previous.state == SlackConnectionState::Active
                && previous
                    .pending_connection
                    .is_some_and(|pending| pending.epoch == failed_epoch))
            .then(|| previous.without_pending_connection())
        });

        let mut restored_epoch = None;
        let Some((current_connection, current_connection_version)) = self
            .read_record::<StoredSlackConnection>(&connection_path)
            .await
            .map_err(map_binding_fs_error)?
        else {
            return Ok(());
        };
        current_connection
            .validate(&owner)
            .map_err(|error| RebornUserIdentityBindingError::Backend(error.to_string()))?;
        if current_connection.state == SlackConnectionState::Active
            && current_connection.epoch == failed_epoch
        {
            let exact_target = restorable_connection.clone().unwrap_or_else(|| {
                current_connection
                    .with_state(SlackConnectionState::Disconnected)
                    .without_pending_connection()
            });
            match self
                .write_record(
                    &connection_path,
                    &exact_target,
                    CasExpectation::Version(activation.written_version),
                )
                .await
            {
                Ok(_) => restored_epoch = restorable_connection.map(|record| record.epoch),
                Err(FilesystemError::VersionMismatch { .. }) => {
                    // A newer reconfigure can only have been staged after the
                    // failed epoch became active. Do not resurrect the older
                    // epoch across that write; fence the failed active epoch
                    // instead. Disconnecting or a promoted newer epoch wins.
                    if current_connection.state == SlackConnectionState::Active
                        && current_connection.epoch == failed_epoch
                    {
                        let disconnected = current_connection
                            .with_state(SlackConnectionState::Disconnected)
                            .without_pending_connection();
                        match self
                            .write_record(
                                &connection_path,
                                &disconnected,
                                CasExpectation::Version(current_connection_version),
                            )
                            .await
                        {
                            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => {}
                            Err(error) => return Err(map_binding_fs_error(error)),
                        }
                    }
                }
                Err(error) => return Err(map_binding_fs_error(error)),
            }
        }

        let Some((current_identity, current_identity_version)) = self
            .read_record::<StoredSlackUserIdentity>(&identity_path)
            .await
            .map_err(map_binding_fs_error)?
        else {
            return Ok(());
        };
        current_identity
            .validate_for_key(binding.provider.as_str(), binding.provider_user_id.as_str())?;
        if current_identity.state != StoredSlackIdentityState::Active
            || current_identity.user_id != binding.user_id.as_str()
            || current_identity.epoch != Some(failed_epoch)
        {
            return Ok(());
        }
        let restored_identity = restored_epoch.and_then(|epoch| {
            previous_identity.filter(|identity| {
                identity.state == StoredSlackIdentityState::Active
                    && identity.user_id == binding.user_id.as_str()
                    && identity.epoch == Some(epoch)
            })
        });
        let identity_target = restored_identity
            .clone()
            .unwrap_or_else(|| current_identity.tombstone());
        let identity_restore_version = match self
            .write_record(
                &identity_path,
                &identity_target,
                CasExpectation::Version(identity_written_version),
            )
            .await
        {
            Ok(version) => Some(version),
            Err(FilesystemError::VersionMismatch { .. }) => self
                .write_record(
                    &identity_path,
                    &identity_target,
                    CasExpectation::Version(current_identity_version),
                )
                .await
                .ok(),
            Err(error) => return Err(map_binding_fs_error(error)),
        };

        if let (Some(epoch), Some(identity_version)) = (restored_epoch, identity_restore_version) {
            if !self
                .connection_is_active_at_epoch_for_owner(&owner, epoch)
                .await
                .map_err(|error| RebornUserIdentityBindingError::Backend(error.to_string()))?
            {
                let _ = self
                    .write_record(
                        &identity_path,
                        &identity_target.tombstone(),
                        CasExpectation::Version(identity_version),
                    )
                    .await;
            } else if let Some(restored_binding) =
                restored_identity.and_then(|identity| identity.binding_including_tombstone())
            {
                self.write_user_binding_index_marker(&restored_binding)
                    .await;
                return Ok(());
            }
        }
        self.delete_user_binding_index_marker(
            binding.provider.as_str(),
            binding.user_id.as_str(),
            binding.provider_user_id.as_str(),
        )
        .await;
        Ok(())
    }

    async fn bind_user_identity_inner(
        &self,
        binding: RebornUserIdentityBinding,
        epoch: Option<SlackConnectionEpoch>,
    ) -> Result<Option<SlackUserIdentityBindingRollback>, RebornUserIdentityBindingError> {
        let owner =
            match epoch {
                Some(epoch) => {
                    let installation_id = self.binding_installation(&binding)?;
                    let owner = SlackConnectionOwner::new(
                        self.scope.tenant_id.clone(),
                        binding.user_id.clone(),
                        installation_id,
                    );
                    match self.connection_state(&owner).await.map_err(|error| {
                        RebornUserIdentityBindingError::Backend(error.to_string())
                    })? {
                        Some((current_epoch, SlackConnectionState::Connecting))
                            if current_epoch == epoch => {}
                        _ => {
                            return Err(RebornUserIdentityBindingError::Backend(
                                SlackUserBindingLifecycleError::StaleEpoch.to_string(),
                            ));
                        }
                    }
                    Some(owner)
                }
                None => None,
            };
        let path =
            Self::identity_path(binding.provider.as_str(), binding.provider_user_id.as_str())
                .map_err(map_binding_fs_error)?;

        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let current = self
                .read_record::<StoredSlackUserIdentity>(&path)
                .await
                .map_err(map_binding_fs_error)?;
            let (record, cas, previous_identity) = match current {
                Some((existing, version)) => {
                    existing.validate_for_key(
                        binding.provider.as_str(),
                        binding.provider_user_id.as_str(),
                    )?;
                    if existing.state == StoredSlackIdentityState::Active
                        && existing.user_id != binding.user_id.as_str()
                    {
                        log_duplicate_identity_binding(&existing, &binding);
                        return Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound);
                    }
                    if existing.state == StoredSlackIdentityState::Active {
                        // When an epoch is requested, `owner` was checked above
                        // and that epoch currently owns the Connecting lifecycle
                        // record. A same-user row from another epoch can therefore
                        // only be stale (for example, a crash after the identity
                        // write but before lifecycle activation). Replace it with
                        // this current generation; the cross-user branch above
                        // remains a hard conflict.
                    }
                    (
                        StoredSlackUserIdentity::from_binding(&binding, epoch, existing.created_at),
                        CasExpectation::Version(version),
                        (existing.state == StoredSlackIdentityState::Active
                            && existing.epoch != epoch)
                            .then_some(existing),
                    )
                }
                None => (
                    StoredSlackUserIdentity::from_binding(&binding, epoch, Utc::now()),
                    CasExpectation::Absent,
                    None,
                ),
            };
            match self.write_record(&path, &record, cas).await {
                Ok(identity_written_version) => {
                    let rollback = if let (Some(owner), Some(epoch)) = (owner.as_ref(), epoch) {
                        let activation = match self.activate_connection(owner, epoch).await {
                            Ok(activation) => activation,
                            Err(error) => {
                                let restored_identity = if let Some(previous) =
                                    previous_identity.as_ref()
                                    && let Some(previous_epoch) = previous.epoch
                                    && previous_epoch != epoch
                                    && self
                                        .connection_is_active_at_epoch_for_owner(
                                            owner,
                                            previous_epoch,
                                        )
                                        .await
                                        .map_err(|lifecycle_error| {
                                            RebornUserIdentityBindingError::Backend(
                                                lifecycle_error.to_string(),
                                            )
                                        })? {
                                    Some(previous.clone())
                                } else {
                                    None
                                };
                                let replacement = restored_identity
                                    .clone()
                                    .unwrap_or_else(|| record.tombstone());
                                self.write_record(
                                    &path,
                                    &replacement,
                                    CasExpectation::Version(identity_written_version),
                                )
                                .await
                                .map_err(map_binding_fs_error)?;
                                if let Some(restored_binding) = restored_identity
                                    .and_then(|identity| identity.binding_including_tombstone())
                                {
                                    self.write_user_binding_index_marker(&restored_binding)
                                        .await;
                                } else {
                                    self.delete_user_binding_index_marker(
                                        binding.provider.as_str(),
                                        binding.user_id.as_str(),
                                        binding.provider_user_id.as_str(),
                                    )
                                    .await;
                                }
                                return Err(RebornUserIdentityBindingError::Backend(
                                    error.to_string(),
                                ));
                            }
                        };
                        let store = self.clone();
                        let owner = owner.clone();
                        let path = path.clone();
                        let binding_for_rollback = binding.clone();
                        Some(SlackUserIdentityBindingRollback::new(async move {
                            if let Err(error) = store
                                .rollback_epoch_identity_binding(
                                    SlackIdentityBindingRollbackContext {
                                        owner,
                                        failed_epoch: epoch,
                                        identity_path: path,
                                        binding: binding_for_rollback,
                                        identity_written_version,
                                        previous_identity,
                                        activation,
                                    },
                                )
                                .await
                            {
                                tracing::warn!(
                                    %error,
                                    %epoch,
                                    "failed to roll back Slack identity binding transaction"
                                );
                            }
                        }))
                    } else {
                        None
                    };
                    self.write_user_binding_index_marker(&binding).await;
                    return Ok(rollback);
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_binding_fs_error(error)),
            }
        }
        Err(RebornUserIdentityBindingError::Backend(
            "Slack actor binding changed concurrently".into(),
        ))
    }

    fn binding_installation(
        &self,
        binding: &RebornUserIdentityBinding,
    ) -> Result<AdapterInstallationId, RebornUserIdentityBindingError> {
        if binding.provider.as_str() != crate::slack::slack_actor_identity::SLACK_IDENTITY_PROVIDER
        {
            return Err(RebornUserIdentityBindingError::Backend(
                "connection epochs are only supported for Slack identities".to_string(),
            ));
        }
        let Some((installation_id, _)) =
            parse_slack_user_identity_provider_user_id(binding.provider_user_id.as_str())
        else {
            return Err(RebornUserIdentityBindingError::Backend(
                "Slack provider user identity is malformed".to_string(),
            ));
        };
        Ok(installation_id)
    }
}

#[async_trait::async_trait]
impl<F> RebornUserIdentityBindingDeleteStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn user_identity_bindings_for_user(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
        self.user_identity_bindings_for_user_inner(
            provider,
            user_id,
            provider_user_id_prefix,
            IdentityBindingScan::AllOwned,
        )
        .await
    }

    async fn user_identity_bindings_for_user_at_epoch(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
        expected_epoch: Option<SlackConnectionEpoch>,
    ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
        self.user_identity_bindings_for_user_inner(
            provider,
            user_id,
            provider_user_id_prefix,
            IdentityBindingScan::CleanupEpoch(expected_epoch),
        )
        .await
    }

    async fn delete_user_identity_bindings_for_user_at_epoch(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
        expected_epoch: Option<SlackConnectionEpoch>,
    ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
        self.delete_user_identity_bindings_for_user_inner(
            provider,
            user_id,
            provider_user_id_prefix,
            expected_epoch,
        )
        .await
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn delete_user_identity_bindings_for_user_inner(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
        expected_epoch: Option<SlackConnectionEpoch>,
    ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
        let provider_dir = scoped_path(&format!("{IDENTITY_ROOT}/{}", path_segment(provider)))
            .map_err(map_binding_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &provider_dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(map_binding_fs_error(error)),
        };
        let mut deleted = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let path = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_binding_fs_error)?;
            let Some((candidate, _)) = self
                .read_record::<StoredSlackUserIdentity>(&path)
                .await
                .map_err(map_binding_fs_error)?
            else {
                continue;
            };
            candidate.validate_for_provider(provider)?;
            if !identity_record_matches_user_binding(
                &candidate,
                provider,
                user_id,
                provider_user_id_prefix,
            ) || expected_epoch.is_some_and(|epoch| candidate.epoch != Some(epoch))
            {
                continue;
            }
            if let Some(deleted_binding) = self
                .tombstone_identity_path_if_owned(&path, provider, user_id, expected_epoch)
                .await?
            {
                self.delete_user_binding_index_marker(
                    provider,
                    user_id.as_str(),
                    deleted_binding.binding().provider_user_id.as_str(),
                )
                .await;
                deleted.push(deleted_binding);
            }
        }
        Ok(deleted)
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn user_identity_bindings_for_user_inner(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
        scan: IdentityBindingScan,
    ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
        let provider_dir = scoped_path(&format!("{IDENTITY_ROOT}/{}", path_segment(provider)))
            .map_err(map_binding_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &provider_dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(map_binding_fs_error(error)),
        };
        let mut bindings = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let path = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_binding_fs_error)?;
            let Some((candidate, _)) = self
                .read_record::<StoredSlackUserIdentity>(&path)
                .await
                .map_err(map_binding_fs_error)?
            else {
                continue;
            };
            candidate.validate_for_provider(provider)?;
            let matches = match scan {
                IdentityBindingScan::AllOwned => {
                    identity_record_is_owned(&candidate, provider, user_id, provider_user_id_prefix)
                }
                IdentityBindingScan::CleanupEpoch(expected_epoch) => {
                    identity_record_is_owned_for_cleanup(
                        &candidate,
                        provider,
                        user_id,
                        provider_user_id_prefix,
                        expected_epoch,
                    )
                }
            };
            if !matches {
                continue;
            }
            bindings.push(candidate.cleanup_binding().ok_or_else(|| {
                RebornUserIdentityBindingError::Backend(
                    "stored Slack user identity is invalid".to_string(),
                )
            })?);
        }
        Ok(bindings)
    }

    async fn tombstone_identity_path_if_owned(
        &self,
        path: &ScopedPath,
        provider: &str,
        user_id: &UserId,
        expected_epoch: Option<SlackConnectionEpoch>,
    ) -> Result<Option<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError> {
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let Some((current, version)) = self
                .read_record::<StoredSlackUserIdentity>(path)
                .await
                .map_err(map_binding_fs_error)?
            else {
                return Ok(None);
            };
            current.validate_for_provider(provider)?;
            if current.state != StoredSlackIdentityState::Active
                || current.user_id != user_id.as_str()
                || expected_epoch.is_some_and(|epoch| current.epoch != Some(epoch))
            {
                return Ok(None);
            }
            let binding = current.cleanup_binding().ok_or_else(|| {
                RebornUserIdentityBindingError::Backend(
                    "stored Slack user identity is invalid".to_string(),
                )
            })?;
            let tombstone = current.tombstone();
            match self
                .write_record(path, &tombstone, CasExpectation::Version(version))
                .await
            {
                Ok(_) => return Ok(Some(binding)),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_binding_fs_error(error)),
            }
        }
        Err(RebornUserIdentityBindingError::Backend(
            "Slack actor binding changed concurrently".to_string(),
        ))
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn read_personal_dm_target_record(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<(StoredSlackPersonalDmTarget, RecordVersion)>, SlackPersonalDmTargetError>
    {
        match self.read_record::<StoredSlackPersonalDmTarget>(path).await {
            Ok(record) => Ok(record),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(map_personal_dm_target_fs_error(error)),
        }
    }
}

#[async_trait::async_trait]
impl<F> SlackPersonalDmTargetStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn load_personal_dm_target(
        &self,
        key: &SlackPersonalDmTargetKey,
    ) -> Result<Option<SlackPersonalDmTarget>, SlackPersonalDmTargetError> {
        // Cross-tenant reads return Ok(None) (not an error) so a caller
        // cannot distinguish "other tenant has this key" from "no target
        // exists" — reads stay free of a tenant-existence oracle. Writes
        // below differ deliberately: a cross-tenant upsert is a caller bug
        // and fails loudly with InvalidTarget.
        if key.tenant_id != self.scope.tenant_id {
            return Ok(None);
        }
        let path = Self::personal_dm_target_path(key).map_err(map_personal_dm_target_fs_error)?;
        let Some((record, _)) = self.read_personal_dm_target_record(&path).await? else {
            return Ok(None);
        };
        if record.deleted_at.is_some() {
            return Ok(None);
        }
        let epoch = record.epoch;
        let target = stored_personal_dm_target(record)?;
        if let Some(epoch) = epoch
            && !self
                .connection_is_active_at_epoch(&target.key, epoch)
                .await?
        {
            return Ok(None);
        }
        Ok(Some(target))
    }

    #[cfg(test)]
    async fn upsert_personal_dm_target(
        &self,
        target: SlackPersonalDmTarget,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        self.upsert_personal_dm_target_inner(target, None).await
    }

    async fn upsert_personal_dm_target_for_epoch(
        &self,
        target: SlackPersonalDmTarget,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        if !self
            .connection_is_active_at_epoch(&target.key, epoch)
            .await?
        {
            return Err(SlackPersonalDmTargetError::StoreUnavailable);
        }
        let key = target.key.clone();
        let stored = self
            .upsert_personal_dm_target_inner(target, Some(epoch))
            .await?;
        if self.connection_is_active_at_epoch(&key, epoch).await? {
            return Ok(stored);
        }
        let path = Self::personal_dm_target_path(&key).map_err(map_personal_dm_target_fs_error)?;
        self.tombstone_personal_dm_target_at_epoch(&path, Some(epoch))
            .await?;
        Err(SlackPersonalDmTargetError::StoreUnavailable)
    }

    async fn personal_dm_target_installations_for_owner(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<Vec<AdapterInstallationId>, SlackPersonalDmTargetError> {
        if tenant_id != &self.scope.tenant_id {
            return Ok(Vec::new());
        }
        let root = scoped_path(PERSONAL_DM_TARGET_ROOT).map_err(map_personal_dm_target_fs_error)?;
        let installation_entries = match self.filesystem.list_dir(&self.scope, &root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(map_personal_dm_target_fs_error(error)),
        };
        let mut installations = HashSet::new();
        for installation_entry in installation_entries {
            if installation_entry.file_type != FileType::Directory {
                continue;
            }
            let installation_dir = scoped_path(&format!(
                "{}/{}",
                PERSONAL_DM_TARGET_ROOT, installation_entry.name
            ))
            .map_err(map_personal_dm_target_fs_error)?;
            let team_entries = match self
                .filesystem
                .list_dir(&self.scope, &installation_dir)
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => continue,
                Err(error) => return Err(map_personal_dm_target_fs_error(error)),
            };
            for team_entry in team_entries {
                if team_entry.file_type != FileType::Directory {
                    continue;
                }
                let path = scoped_path(&format!(
                    "{}/{}/{}/{}.json",
                    PERSONAL_DM_TARGET_ROOT,
                    installation_entry.name,
                    team_entry.name,
                    path_segment(user_id.as_str())
                ))
                .map_err(map_personal_dm_target_fs_error)?;
                let Some((record, _)) = self.read_personal_dm_target_record(&path).await? else {
                    continue;
                };
                if record.tenant_id != tenant_id.as_str()
                    || record.user_id != user_id.as_str()
                    || path_segment(&record.installation_id) != installation_entry.name
                    || path_segment(&record.team_id) != team_entry.name
                {
                    return Err(SlackPersonalDmTargetError::StoreUnavailable);
                }
                if record.deleted_at.is_none() {
                    installations.insert(
                        AdapterInstallationId::new(record.installation_id)
                            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?,
                    );
                }
            }
        }
        let mut installations = installations.into_iter().collect::<Vec<_>>();
        installations.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        Ok(installations)
    }

    async fn delete_personal_dm_targets_for_owner(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        installation_id: &AdapterInstallationId,
        expected_epoch: Option<SlackConnectionEpoch>,
    ) -> Result<usize, SlackPersonalDmTargetError> {
        if tenant_id != &self.scope.tenant_id {
            return Ok(0);
        }
        let installation_dir = scoped_path(&format!(
            "{}/{}",
            PERSONAL_DM_TARGET_ROOT,
            path_segment(installation_id.as_str())
        ))
        .map_err(map_personal_dm_target_fs_error)?;
        let team_entries = match self
            .filesystem
            .list_dir(&self.scope, &installation_dir)
            .await
        {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(0),
            Err(error) => return Err(map_personal_dm_target_fs_error(error)),
        };
        let mut deleted = 0;
        for team_entry in team_entries {
            if team_entry.file_type != FileType::Directory {
                continue;
            }
            let path = scoped_path(&format!(
                "{}/{}/{}/{}.json",
                PERSONAL_DM_TARGET_ROOT,
                path_segment(installation_id.as_str()),
                team_entry.name,
                path_segment(user_id.as_str())
            ))
            .map_err(map_personal_dm_target_fs_error)?;
            let Some((record, _)) = self.read_personal_dm_target_record(&path).await? else {
                continue;
            };
            if record.tenant_id != tenant_id.as_str()
                || record.user_id != user_id.as_str()
                || record.installation_id != installation_id.as_str()
            {
                return Err(SlackPersonalDmTargetError::StoreUnavailable);
            }
            deleted += usize::from(
                self.tombstone_personal_dm_target_at_epoch(&path, expected_epoch)
                    .await?,
            );
        }
        Ok(deleted)
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn upsert_personal_dm_target_inner(
        &self,
        target: SlackPersonalDmTarget,
        epoch: Option<SlackConnectionEpoch>,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        if target.key.tenant_id != self.scope.tenant_id {
            return Err(SlackPersonalDmTargetError::InvalidTarget);
        }
        let path =
            Self::personal_dm_target_path(&target.key).map_err(map_personal_dm_target_fs_error)?;
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let existing = self.read_personal_dm_target_record(&path).await?;
            let created_at = existing
                .as_ref()
                .map(|(record, _)| record.created_at)
                .unwrap_or_else(Utc::now);
            let record = StoredSlackPersonalDmTarget::from_target(&target, epoch, created_at);
            let cas = existing
                .map(|(_, version)| CasExpectation::Version(version))
                .unwrap_or(CasExpectation::Absent);
            match self.write_record(&path, &record, cas).await {
                Ok(_) => return Ok(target),
                Err(FilesystemError::VersionMismatch { .. }) => {
                    if let Some((winner, _)) = self.read_personal_dm_target_record(&path).await?
                        && winner.epoch == epoch
                        && winner.deleted_at.is_none()
                    {
                        return stored_personal_dm_target(winner);
                    }
                    continue;
                }
                Err(error) => return Err(map_personal_dm_target_fs_error(error)),
            }
        }
        Err(SlackPersonalDmTargetError::StoreUnavailable)
    }

    async fn connection_is_active_at_epoch(
        &self,
        key: &SlackPersonalDmTargetKey,
        epoch: SlackConnectionEpoch,
    ) -> Result<bool, SlackPersonalDmTargetError> {
        let owner = SlackConnectionOwner::new(
            key.tenant_id.clone(),
            key.user_id.clone(),
            key.installation_id.clone(),
        );
        self.connection_is_active_at_epoch_for_owner(&owner, epoch)
            .await
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)
    }

    async fn tombstone_personal_dm_target_at_epoch(
        &self,
        path: &ScopedPath,
        expected_epoch: Option<SlackConnectionEpoch>,
    ) -> Result<bool, SlackPersonalDmTargetError> {
        for _ in 0..LIFECYCLE_CAS_RETRIES {
            let Some((record, version)) = self.read_personal_dm_target_record(path).await? else {
                return Ok(false);
            };
            if record.deleted_at.is_some()
                || expected_epoch.is_some_and(|epoch| record.epoch != Some(epoch))
            {
                return Ok(false);
            }
            let tombstone = record.tombstone();
            match self
                .write_record(path, &tombstone, CasExpectation::Version(version))
                .await
            {
                Ok(_) => return Ok(true),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_personal_dm_target_fs_error(error)),
            }
        }
        Err(SlackPersonalDmTargetError::StoreUnavailable)
    }
}

#[async_trait::async_trait]
impl<F> SlackChannelRouteStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn list_routes(
        &self,
        tenant_id: &TenantId,
        installation_id: &AdapterInstallationId,
        team_id: &str,
        cursor: usize,
        limit: usize,
    ) -> Result<SlackChannelRouteListPage, SlackChannelRouteError> {
        if tenant_id != &self.scope.tenant_id {
            return Ok(SlackChannelRouteListPage {
                routes: Vec::new(),
                next_cursor: None,
            });
        }
        let dir = Self::channel_route_team_dir_path(installation_id, team_id)
            .map_err(map_route_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => {
                return Ok(SlackChannelRouteListPage {
                    routes: Vec::new(),
                    next_cursor: None,
                });
            }
            Err(error) => return Err(map_route_fs_error(error)),
        };
        let mut paths = entries
            .into_iter()
            .filter_map(|entry| {
                if entry.file_type != FileType::File {
                    return None;
                }
                Some(Self::listed_channel_route_path(
                    installation_id,
                    team_id,
                    &entry.name,
                ))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_route_fs_error)?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        let start = cursor.min(paths.len());
        let end = cursor.saturating_add(limit).min(paths.len());
        let reads = paths[start..end]
            .iter()
            .map(|path| async move { self.read_record::<StoredSlackChannelRoute>(path).await });
        let records = futures::future::try_join_all(reads)
            .await
            .map_err(map_route_fs_error)?;
        let mut routes = Vec::new();
        for record in records.into_iter().flatten() {
            if let Some(route) = stored_channel_route(record.0)? {
                routes.push(route);
            }
        }
        routes.sort_by(|left, right| {
            left.team_id
                .cmp(&right.team_id)
                .then(left.channel_id.cmp(&right.channel_id))
        });
        Ok(SlackChannelRouteListPage {
            routes,
            next_cursor: if end < paths.len() { Some(end) } else { None },
        })
    }

    async fn upsert_route(
        &self,
        key: SlackChannelRouteKey,
        subject_user_id: UserId,
    ) -> Result<SlackChannelRoute, SlackChannelRouteError> {
        if key.tenant_id != self.scope.tenant_id {
            return Err(SlackChannelRouteError::InvalidRoute);
        }
        let lock = self.lock_for(Self::channel_route_team_lock_key(
            &key.installation_id,
            &key.team_id,
        ));
        let _guard = lock.lock().await;
        let installation_id = key.installation_id.clone();
        let team_id = key.team_id.clone();
        self.with_channel_route_replace_lease(
            &installation_id,
            &team_id,
            self.upsert_route_record(key, subject_user_id),
        )
        .await
    }

    async fn delete_route(
        &self,
        key: &SlackChannelRouteKey,
    ) -> Result<bool, SlackChannelRouteError> {
        if key.tenant_id != self.scope.tenant_id {
            return Ok(false);
        }
        let lock = self.lock_for(Self::channel_route_team_lock_key(
            &key.installation_id,
            &key.team_id,
        ));
        let _guard = lock.lock().await;
        self.with_channel_route_replace_lease(
            &key.installation_id,
            &key.team_id,
            self.delete_route_record(key),
        )
        .await
    }

    async fn replace_managed_routes(
        &self,
        tenant_id: &TenantId,
        installation_id: &AdapterInstallationId,
        team_id: &str,
        assignments: Vec<SlackChannelRouteAssignment>,
    ) -> Result<Vec<SlackChannelRoute>, SlackChannelRouteError> {
        if tenant_id != &self.scope.tenant_id {
            return Err(SlackChannelRouteError::InvalidRoute);
        }
        let lock = self.lock_for(Self::channel_route_team_lock_key(installation_id, team_id));
        let _guard = lock.lock().await;
        let lease = self
            .acquire_channel_route_replace_lease(installation_id, team_id)
            .await?;
        let mut renewer = ChannelRouteReplaceLeaseRenewer::start(self.clone(), lease.clone());
        let mut result = self
            .replace_managed_routes_while_lease_active(
                tenant_id,
                installation_id,
                team_id,
                assignments,
                &mut renewer,
            )
            .await;
        if let Err(error) = renewer.stop().await
            && result.is_ok()
        {
            result = Err(error);
        }
        self.release_channel_route_replace_lease(lease).await;
        result
    }

    async fn resolve_subject_user_id(
        &self,
        key: &SlackChannelRouteKey,
    ) -> Result<Option<UserId>, SlackChannelRouteError> {
        if key.tenant_id != self.scope.tenant_id {
            return Ok(None);
        }
        let path = Self::channel_route_path(key).map_err(map_route_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredSlackChannelRoute>(&path)
            .await
            .map_err(map_route_fs_error)?
        else {
            return Ok(None);
        };
        if record.deleted_at.is_some() {
            return Ok(None);
        }
        let subject_user_id = UserId::new(record.subject_user_id)
            .map_err(|_| SlackChannelRouteError::StoreUnavailable)?;
        Ok(Some(subject_user_id))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", content = "epoch", rename_all = "snake_case")]
enum StoredSlackDisconnectCleanup {
    AllOwned,
    Epoch(SlackConnectionEpoch),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct StoredSlackPendingConnection {
    epoch: SlackConnectionEpoch,
    expires_at: DateTime<Utc>,
}

struct SlackConnectionActivation {
    previous: Option<StoredSlackConnection>,
    written_version: RecordVersion,
}

struct SlackIdentityBindingRollbackContext {
    owner: SlackConnectionOwner,
    failed_epoch: SlackConnectionEpoch,
    identity_path: ScopedPath,
    binding: RebornUserIdentityBinding,
    identity_written_version: RecordVersion,
    previous_identity: Option<StoredSlackUserIdentity>,
    activation: SlackConnectionActivation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSlackConnection {
    tenant_id: String,
    user_id: String,
    installation_id: String,
    epoch: SlackConnectionEpoch,
    state: SlackConnectionState,
    /// A replacement OAuth generation staged while the previous generation
    /// remains active. Keeping it beside (rather than overwriting) `epoch`
    /// preserves the working binding until the replacement callback commits.
    #[serde(default)]
    pending_connection: Option<StoredSlackPendingConnection>,
    #[serde(default)]
    disconnect_cleanup: Option<StoredSlackDisconnectCleanup>,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredSlackConnection {
    fn new(
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
        state: SlackConnectionState,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id: owner.tenant_id().as_str().to_string(),
            user_id: owner.user_id().as_str().to_string(),
            installation_id: owner.installation_id().as_str().to_string(),
            epoch,
            state,
            pending_connection: None,
            disconnect_cleanup: None,
            expires_at,
            created_at,
            updated_at: Utc::now(),
        }
    }

    fn validate(&self, owner: &SlackConnectionOwner) -> Result<(), SlackUserBindingLifecycleError> {
        if self.tenant_id != owner.tenant_id().as_str()
            || self.user_id != owner.user_id().as_str()
            || self.installation_id != owner.installation_id().as_str()
        {
            return Err(SlackUserBindingLifecycleError::Backend(
                "stored Slack connection owner is malformed".to_string(),
            ));
        }
        Ok(())
    }

    fn with_state(&self, state: SlackConnectionState) -> Self {
        Self {
            state,
            updated_at: Utc::now(),
            ..self.clone()
        }
    }

    fn with_pending_connection(
        &self,
        epoch: SlackConnectionEpoch,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            pending_connection: Some(StoredSlackPendingConnection { epoch, expires_at }),
            updated_at: Utc::now(),
            ..self.clone()
        }
    }

    fn without_pending_connection(&self) -> Self {
        Self {
            pending_connection: None,
            updated_at: Utc::now(),
            ..self.clone()
        }
    }

    fn promote_pending_connection(&self) -> Option<Self> {
        let pending = self.pending_connection?;
        Some(Self {
            epoch: pending.epoch,
            state: SlackConnectionState::Active,
            pending_connection: None,
            disconnect_cleanup: None,
            expires_at: pending.expires_at,
            updated_at: Utc::now(),
            ..self.clone()
        })
    }

    fn visible_connection_state(&self) -> (SlackConnectionEpoch, SlackConnectionState) {
        match (self.state, self.pending_connection) {
            (SlackConnectionState::Active, Some(pending)) => {
                (pending.epoch, SlackConnectionState::Connecting)
            }
            _ => (self.epoch, self.state),
        }
    }

    fn owns_connection_epoch(&self, epoch: SlackConnectionEpoch) -> bool {
        if self.state == SlackConnectionState::Disconnected {
            return false;
        }
        self.epoch == epoch
            || (self.state == SlackConnectionState::Active
                && self
                    .pending_connection
                    .is_some_and(|pending| pending.epoch == epoch))
    }

    fn with_disconnect_cleanup(mut self, cleanup: StoredSlackDisconnectCleanup) -> Self {
        self.disconnect_cleanup = Some(cleanup);
        self
    }

    fn disconnect_fence(&self) -> SlackDisconnectFence {
        let cleanup_selector = match self.disconnect_cleanup {
            Some(StoredSlackDisconnectCleanup::AllOwned) => {
                SlackConnectionCleanupSelector::AllOwned
            }
            Some(StoredSlackDisconnectCleanup::Epoch(epoch)) => {
                SlackConnectionCleanupSelector::Epoch(epoch)
            }
            // Disconnecting records written before the selector field existed
            // always used their connection epoch for cleanup.
            None => SlackConnectionCleanupSelector::Epoch(self.epoch),
        };
        SlackDisconnectFence::new(self.epoch, cleanup_selector)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredSlackIdentityState {
    Active,
    Disconnected,
}

#[derive(Clone, Copy)]
enum IdentityBindingScan {
    AllOwned,
    CleanupEpoch(Option<SlackConnectionEpoch>),
}

fn active_identity_state() -> StoredSlackIdentityState {
    StoredSlackIdentityState::Active
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSlackUserIdentity {
    provider: String,
    provider_user_id: String,
    user_id: String,
    #[serde(default)]
    epoch: Option<SlackConnectionEpoch>,
    #[serde(default = "active_identity_state")]
    state: StoredSlackIdentityState,
    #[serde(default)]
    disconnected_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredSlackUserIdentity {
    fn from_binding(
        binding: &RebornUserIdentityBinding,
        epoch: Option<SlackConnectionEpoch>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            provider: binding.provider.as_str().to_string(),
            provider_user_id: binding.provider_user_id.as_str().to_string(),
            user_id: binding.user_id.as_str().to_string(),
            epoch,
            state: StoredSlackIdentityState::Active,
            disconnected_at: None,
            created_at,
            updated_at: Utc::now(),
        }
    }

    fn binding_including_tombstone(&self) -> Option<RebornUserIdentityBinding> {
        Some(RebornUserIdentityBinding {
            provider: crate::slack::slack_personal_binding::RebornIdentityProviderId::new(
                self.provider.clone(),
            )
            .ok()?,
            provider_user_id:
                crate::slack::slack_personal_binding::RebornIdentityProviderUserId::new(
                    self.provider_user_id.clone(),
                )
                .ok()?,
            user_id: UserId::new(self.user_id.clone()).ok()?,
        })
    }

    fn cleanup_binding(&self) -> Option<SlackUserIdentityCleanupBinding> {
        Some(SlackUserIdentityCleanupBinding::new(
            self.binding_including_tombstone()?,
            self.epoch,
        ))
    }

    fn tombstone(&self) -> Self {
        Self {
            state: StoredSlackIdentityState::Disconnected,
            disconnected_at: Some(Utc::now()),
            updated_at: Utc::now(),
            ..self.clone()
        }
    }

    fn validate_for_provider(&self, provider: &str) -> Result<(), RebornUserIdentityBindingError> {
        if self.provider != provider
            || crate::slack::slack_personal_binding::RebornIdentityProviderId::new(
                self.provider.clone(),
            )
            .is_err()
            || crate::slack::slack_personal_binding::RebornIdentityProviderUserId::new(
                self.provider_user_id.clone(),
            )
            .is_err()
            || UserId::new(self.user_id.clone()).is_err()
            || (self.state == StoredSlackIdentityState::Active && self.disconnected_at.is_some())
            || (self.state == StoredSlackIdentityState::Disconnected
                && self.disconnected_at.is_none())
        {
            return Err(RebornUserIdentityBindingError::Backend(
                "stored Slack user identity is invalid".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_for_key(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<(), RebornUserIdentityBindingError> {
        self.validate_for_provider(provider)?;
        if self.provider_user_id != provider_user_id {
            return Err(RebornUserIdentityBindingError::Backend(
                "stored Slack user identity key does not match its record".to_string(),
            ));
        }
        Ok(())
    }
}

/// Per-user index marker (see [`IDENTITY_BY_USER_ROOT`]). The file name encodes
/// the `provider_user_id`; the body carries the raw id for debuggability but is
/// never read on the hot path (the reader verifies the primary record instead).
#[derive(Debug, Serialize, Deserialize)]
struct StoredUserBindingIndexMarker {
    provider_user_id: String,
}

fn identity_record_matches_user_binding(
    record: &StoredSlackUserIdentity,
    provider: &str,
    user_id: &UserId,
    provider_user_id_prefix: Option<&str>,
) -> bool {
    record.state == StoredSlackIdentityState::Active
        && record.provider == provider
        && record.user_id == user_id.as_str()
        && provider_user_id_prefix
            .map(|prefix| record.provider_user_id.starts_with(prefix))
            .unwrap_or(true)
}

fn identity_record_is_owned_for_cleanup(
    record: &StoredSlackUserIdentity,
    provider: &str,
    user_id: &UserId,
    provider_user_id_prefix: Option<&str>,
    expected_epoch: Option<SlackConnectionEpoch>,
) -> bool {
    record.provider == provider
        && record.user_id == user_id.as_str()
        && record.epoch == expected_epoch
        && provider_user_id_prefix
            .map(|prefix| record.provider_user_id.starts_with(prefix))
            .unwrap_or(true)
}

fn identity_record_is_owned(
    record: &StoredSlackUserIdentity,
    provider: &str,
    user_id: &UserId,
    provider_user_id_prefix: Option<&str>,
) -> bool {
    record.provider == provider
        && record.user_id == user_id.as_str()
        && provider_user_id_prefix
            .map(|prefix| record.provider_user_id.starts_with(prefix))
            .unwrap_or(true)
}

fn log_duplicate_identity_binding(
    existing: &StoredSlackUserIdentity,
    binding: &RebornUserIdentityBinding,
) {
    tracing::warn!(
        provider = %binding.provider.as_str(),
        provider_user_id = %binding.provider_user_id.as_str(),
        existing_user_id = %existing.user_id,
        connecting_user_id = %binding.user_id.as_str(),
        "rejecting Slack identity bind: provider identity already bound to a different \
         reborn user (connecting user is not the current owner)"
    );
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSlackPersonalDmTarget {
    tenant_id: String,
    installation_id: String,
    team_id: String,
    user_id: String,
    slack_user_id: String,
    dm_channel_id: String,
    #[serde(default)]
    epoch: Option<SlackConnectionEpoch>,
    #[serde(default)]
    deleted_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredSlackPersonalDmTarget {
    #[allow(
        dead_code,
        reason = "used with the optional explicit Slack DM target upsert path"
    )]
    fn from_target(
        target: &SlackPersonalDmTarget,
        epoch: Option<SlackConnectionEpoch>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id: target.key.tenant_id.as_str().to_string(),
            installation_id: target.key.installation_id.as_str().to_string(),
            team_id: target.key.team_id.as_str().to_string(),
            user_id: target.key.user_id.as_str().to_string(),
            slack_user_id: target.slack_user_id.as_str().to_string(),
            dm_channel_id: target.dm_channel_id.clone(),
            epoch,
            deleted_at: None,
            created_at,
            updated_at: Utc::now(),
        }
    }

    fn tombstone(&self) -> Self {
        Self {
            deleted_at: Some(Utc::now()),
            updated_at: Utc::now(),
            ..self.clone()
        }
    }
}

fn stored_personal_dm_target(
    record: StoredSlackPersonalDmTarget,
) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
    let key = SlackPersonalDmTargetKey::new(
        TenantId::new(record.tenant_id)
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?,
        AdapterInstallationId::new(record.installation_id)
            .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?,
        SlackTeamId::new(record.team_id),
        UserId::new(record.user_id).map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?,
    )
    .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)?;
    SlackPersonalDmTarget::new(
        key,
        SlackUserId::new(record.slack_user_id),
        record.dm_channel_id,
    )
    .map_err(|_| SlackPersonalDmTargetError::StoreUnavailable)
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackChannelRoute {
    tenant_id: String,
    installation_id: String,
    team_id: String,
    channel_id: String,
    subject_user_id: String,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    deleted_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
struct SlackChannelRouteReplaceLease {
    path: ScopedPath,
    nonce: String,
}

struct ChannelRouteReplaceLeaseRenewer<F>
where
    F: RootFilesystem + 'static,
{
    stop: tokio::sync::oneshot::Sender<()>,
    failure: tokio::sync::oneshot::Receiver<SlackChannelRouteError>,
    handle: tokio::task::JoinHandle<()>,
    _marker: std::marker::PhantomData<F>,
}

impl<F> ChannelRouteReplaceLeaseRenewer<F>
where
    F: RootFilesystem + 'static,
{
    fn start(state: FilesystemSlackHostState<F>, lease: SlackChannelRouteReplaceLease) -> Self {
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let (failure, failure_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut failure = Some(failure);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(CHANNEL_ROUTE_REPLACE_LOCK_RENEW_INTERVAL) => {
                        if let Err(error) = state.renew_channel_route_replace_lease(&lease).await {
                            tracing::warn!(?error, "failed to renew Slack channel route replacement lease");
                            if let Some(failure) = failure.take() {
                                #[allow(clippy::let_underscore_must_use)] // oneshot send; dropped receiver is expected
                                let _ = failure.send(error);
                            }
                            return;
                        }
                    }
                    _ = &mut stopped => return,
                }
            }
        });
        Self {
            stop,
            failure: failure_rx,
            handle,
            _marker: std::marker::PhantomData,
        }
    }

    async fn failed(&mut self) -> SlackChannelRouteError {
        (&mut self.failure)
            .await
            .unwrap_or(SlackChannelRouteError::StoreUnavailable)
    }

    fn ensure_active(&mut self) -> Result<(), SlackChannelRouteError> {
        match self.failure.try_recv() {
            Ok(error) => Err(error),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => Ok(()),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Ok(()),
        }
    }

    async fn stop(mut self) -> Result<(), SlackChannelRouteError> {
        if let Ok(error) = self.failure.try_recv() {
            #[allow(clippy::let_underscore_must_use)]
            // join result unused during shutdown; error already captured
            let _ = self.handle.await;
            return Err(error);
        }
        #[allow(clippy::let_underscore_must_use)]
        // oneshot stop signal; dropped receiver means the task already exited
        let _ = self.stop.send(());
        #[allow(clippy::let_underscore_must_use)] // join result unused during shutdown
        let _ = self.handle.await;
        match self.failure.try_recv() {
            Ok(error) => Err(error),
            Err(_) => Ok(()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackChannelRouteReplaceLock {
    nonce: String,
    expires_at: DateTime<Utc>,
}

impl StoredSlackChannelRouteReplaceLock {
    fn new(nonce: String) -> Self {
        Self {
            nonce,
            expires_at: Utc::now()
                + chrono::Duration::seconds(CHANNEL_ROUTE_REPLACE_LOCK_TTL_SECONDS),
        }
    }

    fn expired(nonce: String) -> Self {
        Self {
            nonce,
            expires_at: Utc::now() - chrono::Duration::seconds(1),
        }
    }
}

impl StoredSlackChannelRoute {
    fn new(key: &SlackChannelRouteKey, subject_user_id: &UserId) -> Self {
        Self {
            tenant_id: key.tenant_id.as_str().to_string(),
            installation_id: key.installation_id.as_str().to_string(),
            team_id: key.team_id.clone(),
            channel_id: key.channel_id.clone(),
            subject_user_id: subject_user_id.as_str().to_string(),
            updated_at: Utc::now(),
            deleted_at: None,
        }
    }
}

fn stored_channel_route(
    record: StoredSlackChannelRoute,
) -> Result<Option<SlackChannelRoute>, SlackChannelRouteError> {
    if record.deleted_at.is_some() {
        return Ok(None);
    }
    let key = SlackChannelRouteKey::new(
        TenantId::new(record.tenant_id).map_err(|_| SlackChannelRouteError::StoreUnavailable)?,
        AdapterInstallationId::new(record.installation_id)
            .map_err(|_| SlackChannelRouteError::StoreUnavailable)?,
        record.team_id,
        record.channel_id,
    )?;
    let subject_user_id = UserId::new(record.subject_user_id)
        .map_err(|_| SlackChannelRouteError::StoreUnavailable)?;
    Ok(Some(SlackChannelRoute::new(key, subject_user_id)))
}

fn random_lock_nonce() -> String {
    let mut bytes = [0_u8; 16];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn path_segment(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn scoped_path(raw: &str) -> Result<ScopedPath, FilesystemError> {
    ScopedPath::new(raw).map_err(|error| FilesystemError::BackendInfrastructure {
        operation: FilesystemOperation::WriteFile,
        reason: format!("invalid Slack host-state path under {SLACK_HOST_STATE_ROOT}: {error}"),
    })
}

fn map_lookup_fs_error(error: FilesystemError) -> RebornUserIdentityLookupError {
    RebornUserIdentityLookupError::Backend(error.to_string())
}

fn map_binding_fs_error(error: FilesystemError) -> RebornUserIdentityBindingError {
    RebornUserIdentityBindingError::Backend(error.to_string())
}

fn map_lifecycle_fs_error(error: FilesystemError) -> SlackUserBindingLifecycleError {
    SlackUserBindingLifecycleError::Backend(error.to_string())
}

fn map_route_fs_error(error: FilesystemError) -> SlackChannelRouteError {
    tracing::error!(%error, "Slack channel route filesystem operation failed");
    SlackChannelRouteError::StoreUnavailable
}

fn map_personal_dm_target_fs_error(error: FilesystemError) -> SlackPersonalDmTargetError {
    tracing::debug!(%error, "Slack personal DM target filesystem operation failed");
    SlackPersonalDmTargetError::StoreUnavailable
}

fn map_setup_fs_error(error: FilesystemError) -> SlackSetupError {
    tracing::debug!(%error, "Slack setup filesystem operation failed");
    SlackSetupError::StoreUnavailable
}

fn is_unsupported_delete_error(error: &FilesystemError) -> bool {
    match error {
        FilesystemError::Unsupported {
            operation: FilesystemOperation::Delete,
            ..
        } => true,
        FilesystemError::Backend {
            operation: FilesystemOperation::Delete,
            reason,
            ..
        } => reason.contains("delete is not supported"),
        FilesystemError::PermissionDenied {
            operation: FilesystemOperation::Delete,
            ..
        } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_filesystem::{
        BackendCapabilities, DirEntry, FileStat, Filter, InMemoryBackend, Page, VersionedEntry,
    };
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    use crate::slack::slack_personal_binding::{
        RebornIdentityProviderId, RebornIdentityProviderUserId,
        RebornUserIdentityBindingDeleteStore, SlackConnectionEpoch, SlackConnectionOwner,
        SlackConnectionState, SlackUserBindingLifecycleStore,
    };

    #[tokio::test]
    async fn filesystem_slack_host_state_reclaims_expired_connection_epoch_and_finds_its_owner() {
        let root = Arc::new(InMemoryBackend::default());
        let first = state_with_root(Arc::clone(&root));
        let second = state_with_root(root);
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let expired_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let current_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());

        first
            .begin_connection(
                &owner,
                expired_epoch,
                Utc::now() - chrono::Duration::seconds(1),
            )
            .await
            .expect("expired attempt is recorded");
        second
            .begin_connection(&owner, current_epoch, connection_expiry())
            .await
            .expect("a new attempt reclaims the expired gate");

        assert_eq!(
            first
                .connection_owners_for_user(owner.tenant_id(), owner.user_id())
                .await
                .expect("owner enumeration"),
            vec![owner.clone()],
            "owner enumeration must include a Connecting attempt with no identity row"
        );

        assert_eq!(
            first
                .connection_owner_for_epoch(owner.tenant_id(), owner.user_id(), expired_epoch,)
                .await
                .expect("expired owner lookup"),
            None,
            "the replaced epoch can no longer target the owner"
        );
        assert_eq!(
            first
                .connection_owner_for_epoch(owner.tenant_id(), owner.user_id(), current_epoch)
                .await
                .expect("current owner lookup"),
            Some(owner.clone())
        );
        let competing_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        assert_eq!(
            first
                .begin_connection(&owner, competing_epoch, connection_expiry())
                .await
                .expect_err("a live connection attempt remains exclusive"),
            SlackUserBindingLifecycleError::ConnectionInProgress
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reclaimed_epoch_replaces_same_user_crash_window_identity()
    {
        let root = Arc::new(InMemoryBackend::default());
        let first = state_with_root(Arc::clone(&root));
        let second = state_with_root(root);
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let expired_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let current_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        first
            .begin_connection(
                &owner,
                expired_epoch,
                Utc::now() - chrono::Duration::seconds(1),
            )
            .await
            .expect("expired attempt is recorded");

        // Model a process crash after the callback wrote the identity row but
        // before it activated the matching connection record.
        let identity_path = FilesystemSlackHostState::<InMemoryBackend>::identity_path(
            binding.provider.as_str(),
            binding.provider_user_id.as_str(),
        )
        .expect("identity path");
        first
            .write_record(
                &identity_path,
                &StoredSlackUserIdentity::from_binding(&binding, Some(expired_epoch), Utc::now()),
                CasExpectation::Absent,
            )
            .await
            .expect("crash-window identity is durable");

        second
            .begin_connection(&owner, current_epoch, connection_expiry())
            .await
            .expect("new attempt reclaims the expired lifecycle epoch");
        second
            .bind_user_identity_for_epoch(binding, current_epoch)
            .await
            .expect("the current attempt replaces its own stale crash-window identity");

        assert_eq!(
            second
                .connection_state(&owner)
                .await
                .expect("connection state"),
            Some((current_epoch, SlackConnectionState::Active))
        );
        let resolved = second
            .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
            .await
            .expect("identity lookup")
            .expect("current identity resolves");
        assert_eq!(resolved.0, user("user:alice"));
        assert_eq!(
            resolved.1.as_ref().map(ToString::to_string),
            Some(current_epoch.to_string())
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reconfigure_keeps_active_identity_until_commit() {
        let state = state();
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let active_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let replacement_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let competing_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        state
            .begin_connection(&owner, active_epoch, connection_expiry())
            .await
            .expect("initial connection begins");
        state
            .bind_user_identity_for_epoch(binding.clone(), active_epoch)
            .await
            .expect("initial identity activates");

        state
            .begin_connection(&owner, replacement_epoch, connection_expiry())
            .await
            .expect("an active owner may stage one replacement connection");

        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("pending replacement state"),
            Some((replacement_epoch, SlackConnectionState::Connecting)),
            "OAuth must observe the replacement epoch as the current connection attempt"
        );
        let resolved = state
            .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
            .await
            .expect("identity lookup")
            .expect("the existing active identity stays usable during reauthorization");
        assert_eq!(resolved.0, user("user:alice"));
        assert_eq!(
            resolved.1.as_ref().map(ToString::to_string),
            Some(active_epoch.to_string())
        );
        assert_eq!(
            state
                .begin_connection(&owner, competing_epoch, connection_expiry())
                .await
                .expect_err("only one replacement attempt may be pending"),
            SlackUserBindingLifecycleError::ConnectionInProgress
        );

        state
            .bind_user_identity_for_epoch(binding, replacement_epoch)
            .await
            .expect("replacement callback promotes the new generation");
        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("promoted connection state"),
            Some((replacement_epoch, SlackConnectionState::Active))
        );
        let resolved = state
            .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
            .await
            .expect("identity lookup")
            .expect("replacement identity resolves");
        assert_eq!(
            resolved.1.as_ref().map(ToString::to_string),
            Some(replacement_epoch.to_string())
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_failed_reconfigure_restores_active_binding() {
        let state = state();
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let active_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let replacement_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        state
            .begin_connection(&owner, active_epoch, connection_expiry())
            .await
            .expect("initial connection begins");
        state
            .bind_user_identity_for_epoch(binding.clone(), active_epoch)
            .await
            .expect("initial identity activates");
        state
            .begin_connection(&owner, replacement_epoch, connection_expiry())
            .await
            .expect("replacement begins");
        let rollback = state
            .bind_user_identity_for_epoch(binding, replacement_epoch)
            .await
            .expect("replacement identity stages");

        rollback.into_future().await;

        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("restored connection state"),
            Some((active_epoch, SlackConnectionState::Active))
        );
        let resolved = state
            .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
            .await
            .expect("identity lookup")
            .expect("previous identity is restored");
        assert_eq!(
            resolved.1.as_ref().map(ToString::to_string),
            Some(active_epoch.to_string())
        );
    }

    #[tokio::test]
    async fn failed_connection_cleanup_fences_active_epoch_before_identity_deletion() {
        let state = state();
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };
        state
            .begin_connection(&owner, epoch, connection_expiry())
            .await
            .expect("connection begins");
        state
            .bind_user_identity_for_epoch(binding, epoch)
            .await
            .expect("identity activates");

        state
            .begin_failed_connection_cleanup(&owner, epoch)
            .await
            .expect("failed callback fences ingress");
        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("fenced connection state"),
            Some((epoch, SlackConnectionState::Disconnecting))
        );

        state
            .complete_failed_connection_cleanup(&owner, epoch)
            .await
            .expect("failed callback cleanup completes");
        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("disconnected state"),
            Some((epoch, SlackConnectionState::Disconnected))
        );
    }

    #[tokio::test]
    async fn failed_reconfigure_cleanup_preserves_previous_active_epoch() {
        let state = state();
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let active_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let replacement_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };
        state
            .begin_connection(&owner, active_epoch, connection_expiry())
            .await
            .expect("initial connection begins");
        state
            .bind_user_identity_for_epoch(binding, active_epoch)
            .await
            .expect("initial identity activates");
        state
            .begin_connection(&owner, replacement_epoch, connection_expiry())
            .await
            .expect("replacement begins");

        state
            .begin_failed_connection_cleanup(&owner, replacement_epoch)
            .await
            .expect("pending replacement is already ingress-fenced");
        state
            .complete_failed_connection_cleanup(&owner, replacement_epoch)
            .await
            .expect("pending replacement cleanup completes");

        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("restored active state"),
            Some((active_epoch, SlackConnectionState::Active))
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_activation_failure_restores_previous_identity() {
        let root = Arc::new(RouteLockTestBackend::normal());
        let state = state_with_backend(Arc::clone(&root));
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let active_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let replacement_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        state
            .begin_connection(&owner, active_epoch, connection_expiry())
            .await
            .expect("initial connection begins");
        state
            .bind_user_identity_for_epoch(binding.clone(), active_epoch)
            .await
            .expect("initial identity activates");
        state
            .begin_connection(&owner, replacement_epoch, connection_expiry())
            .await
            .expect("replacement begins");
        root.fail_next_connection_write();

        state
            .bind_user_identity_for_epoch(binding, replacement_epoch)
            .await
            .err()
            .expect("activation write failure surfaces");
        state
            .abandon_connection(&owner, replacement_epoch)
            .await
            .expect("callback failure abandons pending replacement");

        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("restored lifecycle"),
            Some((active_epoch, SlackConnectionState::Active))
        );
        let resolved = state
            .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
            .await
            .expect("identity lookup")
            .expect("previous identity survives activation failure");
        assert_eq!(
            resolved.1.as_ref().map(ToString::to_string),
            Some(active_epoch.to_string())
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_disconnect_wins_reconfigure_rollback() {
        let state = state();
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let active_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let replacement_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        state
            .begin_connection(&owner, active_epoch, connection_expiry())
            .await
            .expect("initial connection begins");
        state
            .bind_user_identity_for_epoch(binding.clone(), active_epoch)
            .await
            .expect("initial identity activates");
        state
            .begin_connection(&owner, replacement_epoch, connection_expiry())
            .await
            .expect("replacement begins");
        let rollback = state
            .bind_user_identity_for_epoch(binding, replacement_epoch)
            .await
            .expect("replacement identity stages");
        let fence = state
            .begin_disconnect(&owner)
            .await
            .expect("disconnect fences the promoted replacement");

        rollback.into_future().await;

        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("disconnecting state"),
            Some((replacement_epoch, SlackConnectionState::Disconnecting))
        );
        assert!(
            state
                .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
                .await
                .expect("identity lookup")
                .is_none(),
            "rollback must not resurrect the old binding after disconnect owns the lifecycle"
        );
        state
            .complete_disconnect(&owner, fence.fence_epoch())
            .await
            .expect("disconnect completes");
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_abandoned_reconfigure_restores_active_epoch() {
        let state = state();
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let active_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let replacement_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        state
            .begin_connection(&owner, active_epoch, connection_expiry())
            .await
            .expect("initial connection begins");
        state
            .bind_user_identity_for_epoch(binding, active_epoch)
            .await
            .expect("initial identity activates");
        state
            .begin_connection(&owner, replacement_epoch, connection_expiry())
            .await
            .expect("replacement begins");

        state
            .abandon_connection(&owner, replacement_epoch)
            .await
            .expect("failed replacement rolls back");

        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("restored connection state"),
            Some((active_epoch, SlackConnectionState::Active))
        );
        let resolved = state
            .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
            .await
            .expect("identity lookup")
            .expect("the original identity remains active after replacement failure");
        assert_eq!(
            resolved.1.as_ref().map(ToString::to_string),
            Some(active_epoch.to_string())
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_disconnect_fences_pending_reconfigure() {
        let state = state();
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let active_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let replacement_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        state
            .begin_connection(&owner, active_epoch, connection_expiry())
            .await
            .expect("initial connection begins");
        state
            .bind_user_identity_for_epoch(binding, active_epoch)
            .await
            .expect("initial identity activates");
        state
            .begin_connection(&owner, replacement_epoch, connection_expiry())
            .await
            .expect("replacement begins");

        let fence = state
            .begin_disconnect(&owner)
            .await
            .expect("disconnect fences both active and pending generations");

        assert_eq!(fence.fence_epoch(), active_epoch);
        assert_eq!(
            fence.cleanup_selector(),
            SlackConnectionCleanupSelector::Epoch(active_epoch)
        );
        assert_eq!(
            state
                .connection_owner_for_epoch(owner.tenant_id(), owner.user_id(), replacement_epoch,)
                .await
                .expect("pending owner lookup after disconnect"),
            None,
            "a fenced replacement callback must no longer find its lifecycle owner"
        );
        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("disconnecting state"),
            Some((active_epoch, SlackConnectionState::Disconnecting))
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_absent_owner_disconnect_fences_concurrent_oauth_start() {
        let root = Arc::new(InMemoryBackend::default());
        let disconnect = state_with_root(Arc::clone(&root));
        let oauth_start = state_with_root(root);
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );

        let fence = disconnect
            .begin_disconnect(&owner)
            .await
            .expect("legacy disconnect establishes a fence");
        assert_eq!(
            fence.cleanup_selector(),
            SlackConnectionCleanupSelector::AllOwned,
            "legacy cleanup must not mistake the new fence epoch for a binding epoch"
        );
        assert_eq!(
            disconnect
                .connection_state(&owner)
                .await
                .expect("fence state"),
            Some((fence.fence_epoch(), SlackConnectionState::Disconnecting)),
            "an owner without a lifecycle record still needs a durable disconnect fence"
        );

        let competing_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        assert_eq!(
            oauth_start
                .begin_connection(&owner, competing_epoch, connection_expiry())
                .await
                .expect_err("OAuth start must not pass an in-progress legacy cleanup"),
            SlackUserBindingLifecycleError::DisconnectInProgress
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_epoch_recheck_requires_an_active_identity() {
        let state = state();
        let owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation(),
        );
        let epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let provider_user_id = "install-alpha:U123";

        state
            .begin_connection(&owner, epoch, connection_expiry())
            .await
            .expect("connection begins");
        state
            .bind_user_identity_for_epoch(
                RebornUserIdentityBinding {
                    provider: RebornIdentityProviderId::new("slack").unwrap(),
                    provider_user_id: RebornIdentityProviderUserId::new(provider_user_id).unwrap(),
                    user_id: user("user:alice"),
                },
                epoch,
            )
            .await
            .expect("identity activates");
        let binding_epoch =
            ironclaw_conversations::ExternalActorBindingEpoch::new(epoch.to_string())
                .expect("Slack connection epoch is a valid actor binding epoch");

        state
            .delete_user_identity_bindings_for_user_at_epoch(
                "slack",
                &user("user:alice"),
                Some("install-alpha:"),
                Some(epoch),
            )
            .await
            .expect("identity tombstones");
        assert_eq!(
            state
                .connection_state(&owner)
                .await
                .expect("connection state remains readable"),
            Some((epoch, SlackConnectionState::Active)),
            "this models identity rollback succeeding before lifecycle rollback fails"
        );

        assert!(
            !state
                .user_identity_binding_epoch_is_current(
                    "slack",
                    provider_user_id,
                    &user("user:alice"),
                    &binding_epoch,
                )
                .await
                .expect("epoch freshness check"),
            "an active lifecycle record cannot revive a tombstoned canonical identity"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_connection_epoch_preserves_a_newer_owner() {
        let root = Arc::new(InMemoryBackend::default());
        let first = state_with_root(Arc::clone(&root));
        let second = state_with_root(root);
        let installation_id = installation();
        let alice_owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:alice"),
            installation_id.clone(),
        );
        let bob_owner = SlackConnectionOwner::new(
            TenantId::new("tenant-alpha").unwrap(),
            user("user:bob"),
            installation_id,
        );
        let alice_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let bob_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        let provider_user_id = RebornIdentityProviderUserId::new("install-alpha:U123").unwrap();
        let alice_binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: provider_user_id.clone(),
            user_id: user("user:alice"),
        };

        first
            .begin_connection(&alice_owner, alice_epoch, connection_expiry())
            .await
            .expect("Alice connection begins");
        first
            .bind_user_identity_for_epoch(alice_binding, alice_epoch)
            .await
            .expect("Alice identity activates");
        assert_eq!(
            first
                .connection_state(&alice_owner)
                .await
                .expect("Alice state"),
            Some((alice_epoch, SlackConnectionState::Active))
        );
        let resolved = first
            .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
            .await
            .expect("active identity resolves")
            .expect("active identity exists");
        assert_eq!(resolved.0, user("user:alice"));
        assert_eq!(
            resolved.1.as_ref().map(ToString::to_string),
            Some(alice_epoch.to_string())
        );
        let dm_keys = ["T123", "T999"].map(|team_id| {
            SlackPersonalDmTargetKey::new(
                TenantId::new("tenant-alpha").unwrap(),
                installation(),
                SlackTeamId::new(team_id),
                user("user:alice"),
            )
            .unwrap()
        });
        for (key, channel_id) in dm_keys.iter().cloned().zip(["D123", "D999"]) {
            first
                .upsert_personal_dm_target_for_epoch(
                    SlackPersonalDmTarget::new(
                        key,
                        SlackUserId::new("U123"),
                        channel_id.to_string(),
                    )
                    .unwrap(),
                    alice_epoch,
                )
                .await
                .expect("epoch-bound DM target activates");
        }
        second
            .begin_connection(&bob_owner, bob_epoch, connection_expiry())
            .await
            .expect("Bob may start a competing connection attempt");
        let duplicate = second
            .bind_user_identity_for_epoch(
                RebornUserIdentityBinding {
                    provider: RebornIdentityProviderId::new("slack").unwrap(),
                    provider_user_id: provider_user_id.clone(),
                    user_id: user("user:bob"),
                },
                bob_epoch,
            )
            .await
            .err()
            .expect("Alice's active Slack identity rejects Bob");
        assert_eq!(
            duplicate,
            RebornUserIdentityBindingError::ProviderIdentityAlreadyBound,
            "the canonical cross-user error must remain unchanged"
        );

        assert_eq!(
            second
                .begin_disconnect(&alice_owner)
                .await
                .expect("Alice disconnect fences"),
            SlackDisconnectFence::new(
                alice_epoch,
                SlackConnectionCleanupSelector::Epoch(alice_epoch),
            )
        );
        second
            .abandon_connection(&alice_owner, alice_epoch)
            .await
            .expect("a racing callback rollback defers to disconnect");
        assert_eq!(
            second
                .connection_state(&alice_owner)
                .await
                .expect("disconnect still owns the fence"),
            Some((alice_epoch, SlackConnectionState::Disconnecting))
        );
        assert_eq!(
            second
                .resolve_user_identity_with_binding_epoch("slack", "install-alpha:U123")
                .await
                .expect("disconnecting identity fails closed"),
            None
        );
        let stale_dm = SlackPersonalDmTarget::new(
            dm_keys[0].clone(),
            SlackUserId::new("U123"),
            "DSTALE".to_string(),
        )
        .unwrap();
        assert!(
            second
                .upsert_personal_dm_target_for_epoch(stale_dm, alice_epoch)
                .await
                .is_err(),
            "detached provisioning from the disconnected epoch must fail closed"
        );
        assert_eq!(
            second
                .delete_personal_dm_targets_for_owner(
                    &TenantId::new("tenant-alpha").unwrap(),
                    &user("user:alice"),
                    &installation(),
                    Some(alice_epoch),
                )
                .await
                .expect("owner-wide DM cleanup works without setup"),
            2
        );
        for key in &dm_keys {
            assert_eq!(
                second
                    .load_personal_dm_target(key)
                    .await
                    .expect("DM tombstone reads as absent"),
                None
            );
        }
        second
            .delete_user_identity_bindings_for_user_at_epoch(
                "slack",
                &user("user:alice"),
                Some("install-alpha:"),
                Some(alice_epoch),
            )
            .await
            .expect("Alice identity tombstones");
        second
            .complete_disconnect(&alice_owner, alice_epoch)
            .await
            .expect("Alice disconnect completes");

        second
            .begin_connection(&bob_owner, bob_epoch, connection_expiry())
            .await
            .expect("Bob connection begins");
        second
            .bind_user_identity_for_epoch(
                RebornUserIdentityBinding {
                    provider: RebornIdentityProviderId::new("slack").unwrap(),
                    provider_user_id,
                    user_id: user("user:bob"),
                },
                bob_epoch,
            )
            .await
            .expect("Bob identity activates");

        first
            .delete_user_identity_bindings_for_user_at_epoch(
                "slack",
                &user("user:alice"),
                Some("install-alpha:"),
                Some(alice_epoch),
            )
            .await
            .expect("stale Alice cleanup is harmless");
        assert_eq!(
            first
                .resolve_user_identity("slack", "install-alpha:U123")
                .await
                .expect("resolve Bob"),
            Some(user("user:bob"))
        );

        let alice_next_epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        first
            .begin_connection(&alice_owner, alice_next_epoch, connection_expiry())
            .await
            .expect("Alice N+1 connection begins");
        first
            .bind_user_identity_for_epoch(
                RebornUserIdentityBinding {
                    provider: RebornIdentityProviderId::new("slack").unwrap(),
                    provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U456")
                        .unwrap(),
                    user_id: user("user:alice"),
                },
                alice_next_epoch,
            )
            .await
            .expect("Alice N+1 identity activates");
        second
            .delete_user_identity_bindings_for_user_at_epoch(
                "slack",
                &user("user:alice"),
                Some("install-alpha:"),
                Some(alice_epoch),
            )
            .await
            .expect("Alice N stale cleanup is harmless to N+1");
        assert_eq!(
            second
                .resolve_user_identity("slack", "install-alpha:U456")
                .await
                .expect("resolve Alice N+1"),
            Some(user("user:alice"))
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_binds_and_resolves_identity() {
        let state = state();
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        state
            .bind_user_identity(binding.clone())
            .await
            .expect("bind succeeds");
        let resolved = state
            .resolve_user_identity("slack", "install-alpha:U123")
            .await
            .expect("resolve succeeds");

        assert_eq!(resolved, Some(user("user:alice")));
        let stored = read_identity(&state, "slack", "install-alpha:U123").await;
        assert_eq!(stored.binding_including_tombstone(), Some(binding));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_fails_closed_for_malformed_identity_record() {
        let state = state();
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };
        let mut malformed = StoredSlackUserIdentity::from_binding(&binding, None, Utc::now());
        malformed.provider = "github".to_string();
        let path = FilesystemSlackHostState::<InMemoryBackend>::identity_path(
            "slack",
            "install-alpha:U123",
        )
        .unwrap();
        state
            .write_record(&path, &malformed, CasExpectation::Any)
            .await
            .expect("seed malformed identity");

        assert!(
            state
                .resolve_user_identity("slack", "install-alpha:U123")
                .await
                .is_err(),
            "malformed canonical identity must never authorize ingress"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_user_has_provider_binding() {
        let state = state();
        assert!(
            !state
                .user_has_provider_binding("slack", &user("user:alice"))
                .await
                .expect("lookup succeeds"),
            "no binding yet -> not connected"
        );

        state
            .bind_user_identity(RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("slack").unwrap(),
                provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
                user_id: user("user:alice"),
            })
            .await
            .expect("bind succeeds");

        assert!(
            state
                .user_has_provider_binding("slack", &user("user:alice"))
                .await
                .expect("lookup succeeds"),
            "bound user reports connected"
        );
        assert!(
            state
                .user_has_provider_binding_with_provider_user_id_prefix(
                    "slack",
                    &user("user:alice"),
                    Some("install-alpha:"),
                )
                .await
                .expect("scoped lookup succeeds"),
            "bound user reports connected inside the matching installation scope"
        );
        assert!(
            !state
                .user_has_provider_binding_with_provider_user_id_prefix(
                    "slack",
                    &user("user:alice"),
                    Some("install-beta:"),
                )
                .await
                .expect("scoped lookup succeeds"),
            "binding in another installation does not satisfy this scope"
        );
        assert!(
            !state
                .user_has_provider_binding("slack", &user("user:bob"))
                .await
                .expect("lookup succeeds"),
            "different user reports not connected"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_connection_check_survives_index_faults() {
        // The per-user index is a best-effort accelerator: a missing marker must
        // still resolve via the scan, and a stale marker (primary gone) must
        // never be reported as a live binding.
        let state = state();
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };
        state
            .bind_user_identity(binding.clone())
            .await
            .expect("bind");

        let marker = scoped_path(&format!(
            "{IDENTITY_BY_USER_ROOT}/{}/{}/{}.json",
            path_segment("slack"),
            path_segment("user:alice"),
            path_segment("install-alpha:U123"),
        ))
        .unwrap();
        let primary = scoped_path(&format!(
            "{IDENTITY_ROOT}/{}/{}.json",
            path_segment("slack"),
            path_segment("install-alpha:U123"),
        ))
        .unwrap();

        // Legacy shape: primary present, index marker absent -> the scan
        // fallback still resolves the binding.
        state.delete_record(&marker).await.expect("drop marker");
        assert!(
            state
                .user_has_provider_binding("slack", &user("user:alice"))
                .await
                .expect("lookup"),
            "a binding with no index marker is still found via the scan fallback"
        );

        // Stale marker: re-bind to restore the marker, then drop the primary,
        // leaving the marker dangling. The verify-read must reject it.
        state.bind_user_identity(binding).await.expect("rebind");
        state.delete_record(&primary).await.expect("drop primary");
        assert!(
            !state
                .user_has_provider_binding("slack", &user("user:alice"))
                .await
                .expect("lookup"),
            "a stale index marker whose primary record is gone is not a false positive"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_deletes_installation_scoped_identity_bindings_for_user() {
        let state = state();
        state
            .bind_user_identity(RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("slack").unwrap(),
                provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
                user_id: user("user:alice"),
            })
            .await
            .expect("bind alpha succeeds");
        state
            .bind_user_identity(RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("slack").unwrap(),
                provider_user_id: RebornIdentityProviderUserId::new("install-beta:U123").unwrap(),
                user_id: user("user:alice"),
            })
            .await
            .expect("bind beta succeeds");
        state
            .bind_user_identity(RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("slack").unwrap(),
                provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U999").unwrap(),
                user_id: user("user:bob"),
            })
            .await
            .expect("bind bob succeeds");

        let deleted = state
            .delete_user_identity_bindings_for_user_at_epoch(
                "slack",
                &user("user:alice"),
                Some("install-alpha:"),
                None,
            )
            .await
            .expect("delete succeeds");

        assert_eq!(deleted.len(), 1);
        assert_eq!(
            deleted[0].binding().provider_user_id.as_str(),
            "install-alpha:U123"
        );
        assert_eq!(
            state
                .resolve_user_identity("slack", "install-alpha:U123")
                .await
                .expect("alpha alice lookup succeeds"),
            None
        );
        assert_eq!(
            state
                .resolve_user_identity("slack", "install-beta:U123")
                .await
                .expect("beta alice lookup succeeds"),
            Some(user("user:alice"))
        );
        assert_eq!(
            state
                .resolve_user_identity("slack", "install-alpha:U999")
                .await
                .expect("alpha bob lookup succeeds"),
            Some(user("user:bob"))
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_persists_personal_dm_targets_across_state_recreation() {
        let root = Arc::new(InMemoryBackend::default());
        let writer = state_with_root(root.clone());
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            SlackTeamId::new("T123"),
            user("user:alice"),
        )
        .unwrap();
        let target =
            SlackPersonalDmTarget::new(key.clone(), SlackUserId::new("U123"), "D123".to_string())
                .unwrap();

        writer
            .upsert_personal_dm_target(target.clone())
            .await
            .expect("upsert personal DM target succeeds");
        assert_eq!(
            writer
                .load_personal_dm_target(&key)
                .await
                .expect("load personal DM target succeeds"),
            Some(target.clone())
        );

        let reader = state_with_root(root);
        assert_eq!(
            reader
                .load_personal_dm_target(&key)
                .await
                .expect("load persisted personal DM target succeeds"),
            Some(target)
        );
        assert_eq!(
            reader
                .personal_dm_target_installations_for_owner(
                    &TenantId::new("tenant-alpha").unwrap(),
                    &user("user:alice"),
                )
                .await
                .expect("discover personal DM target installation succeeds"),
            vec![installation()],
            "legacy DM state remains discoverable without identity or lifecycle records"
        );

        assert_eq!(
            reader
                .delete_personal_dm_targets_for_owner(
                    &TenantId::new("tenant-alpha").unwrap(),
                    &user("user:alice"),
                    &installation(),
                    None,
                )
                .await
                .expect("delete personal DM target succeeds"),
            1
        );
        assert_eq!(
            reader
                .load_personal_dm_target(&key)
                .await
                .expect("load after delete succeeds"),
            None
        );
        assert_eq!(
            reader
                .delete_personal_dm_targets_for_owner(
                    &TenantId::new("tenant-alpha").unwrap(),
                    &user("user:alice"),
                    &installation(),
                    None,
                )
                .await
                .expect("second delete is idempotent"),
            0
        );
        assert_eq!(
            reader
                .personal_dm_target_installations_for_owner(
                    &TenantId::new("tenant-alpha").unwrap(),
                    &user("user:alice"),
                )
                .await
                .expect("discover after delete succeeds"),
            Vec::<AdapterInstallationId>::new(),
            "tombstoned DM targets are no longer cleanup owners"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_rejects_cross_tenant_personal_dm_target_operations() {
        let state = state();
        let foreign_key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-foreign").unwrap(),
            installation(),
            SlackTeamId::new("T123"),
            user("user:alice"),
        )
        .unwrap();
        let foreign_target = SlackPersonalDmTarget::new(
            foreign_key.clone(),
            SlackUserId::new("U123"),
            "D123".to_string(),
        )
        .unwrap();

        assert!(matches!(
            state.upsert_personal_dm_target(foreign_target).await,
            Err(SlackPersonalDmTargetError::InvalidTarget)
        ));
        assert_eq!(
            state
                .load_personal_dm_target(&foreign_key)
                .await
                .expect("foreign tenant load fails closed"),
            None
        );
        assert_eq!(
            state
                .delete_personal_dm_targets_for_owner(
                    &foreign_key.tenant_id,
                    &foreign_key.user_id,
                    &foreign_key.installation_id,
                    None,
                )
                .await
                .expect("foreign tenant delete fails closed"),
            0
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reports_corrupt_personal_dm_target_as_unavailable() {
        let state = state();
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            SlackTeamId::new("T123"),
            user("user:alice"),
        )
        .unwrap();
        let path =
            FilesystemSlackHostState::<InMemoryBackend>::personal_dm_target_path(&key).unwrap();
        let record = StoredSlackPersonalDmTarget {
            tenant_id: String::new(),
            installation_id: installation().as_str().to_string(),
            team_id: "T123".to_string(),
            user_id: "user:alice".to_string(),
            slack_user_id: "U123".to_string(),
            dm_channel_id: "D123".to_string(),
            epoch: None,
            deleted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        state
            .write_record(&path, &record, CasExpectation::Any)
            .await
            .expect("write corrupt personal DM record");

        assert!(matches!(
            state.load_personal_dm_target(&key).await,
            Err(SlackPersonalDmTargetError::StoreUnavailable)
        ));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reports_corrupt_personal_dm_target_key_fields_as_unavailable()
     {
        // Regression guard: corrupt team_id (fails SlackPersonalDmTargetKey::new validation)
        // and corrupt dm_channel_id (fails SlackPersonalDmTarget::new validation) must both map
        // to StoreUnavailable (503), not InvalidTarget (404). A stored record that exists on disk
        // with an invalid field is a data-integrity problem, not an absence.
        let state = state();
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            SlackTeamId::new("T123"),
            user("user:alice"),
        )
        .unwrap();
        let path =
            FilesystemSlackHostState::<InMemoryBackend>::personal_dm_target_path(&key).unwrap();

        // Corrupt team_id — fails SlackPersonalDmTargetKey::new (validate_slack_id)
        let record_bad_team = StoredSlackPersonalDmTarget {
            tenant_id: "tenant-alpha".to_string(),
            installation_id: installation().as_str().to_string(),
            team_id: String::new(), // empty string fails Slack-ID validation
            user_id: "user:alice".to_string(),
            slack_user_id: "U123".to_string(),
            dm_channel_id: "D123".to_string(),
            epoch: None,
            deleted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        state
            .write_record(&path, &record_bad_team, CasExpectation::Any)
            .await
            .expect("write corrupt personal DM record with bad team_id");
        assert!(
            matches!(
                state.load_personal_dm_target(&key).await,
                Err(SlackPersonalDmTargetError::StoreUnavailable)
            ),
            "corrupt team_id must surface as StoreUnavailable, not InvalidTarget"
        );

        // Corrupt dm_channel_id — fails SlackPersonalDmTarget::new (validate_slack_dm_channel_id)
        let record_bad_dm = StoredSlackPersonalDmTarget {
            tenant_id: "tenant-alpha".to_string(),
            installation_id: installation().as_str().to_string(),
            team_id: "T123".to_string(),
            user_id: "user:alice".to_string(),
            slack_user_id: "U123".to_string(),
            dm_channel_id: "NOTADM".to_string(), // must start with "D"
            epoch: None,
            deleted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        state
            .write_record(&path, &record_bad_dm, CasExpectation::Any)
            .await
            .expect("write corrupt personal DM record with bad dm_channel_id");
        assert!(
            matches!(
                state.load_personal_dm_target(&key).await,
                Err(SlackPersonalDmTargetError::StoreUnavailable)
            ),
            "corrupt dm_channel_id must surface as StoreUnavailable, not InvalidTarget"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_upsert_personal_dm_target_concurrent_write_returns_winner()
    {
        let root = Arc::new(RouteLockTestBackend::barrier_personal_dm_writes());
        let writer_one = state_with_backend(root.clone());
        let writer_two = state_with_backend(root.clone());
        let reader = state_with_backend(root);
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            SlackTeamId::new("T123"),
            user("user:alice"),
        )
        .unwrap();
        let target_one =
            SlackPersonalDmTarget::new(key.clone(), SlackUserId::new("U123"), "D123".to_string())
                .unwrap();
        let target_two =
            SlackPersonalDmTarget::new(key.clone(), SlackUserId::new("U123"), "D456".to_string())
                .unwrap();

        let (stored_one, stored_two) = tokio::join!(
            writer_one.upsert_personal_dm_target(target_one),
            writer_two.upsert_personal_dm_target(target_two)
        );
        let stored_one = stored_one.expect("first upsert succeeds");
        let stored_two = stored_two.expect("second upsert succeeds");
        let persisted = reader
            .load_personal_dm_target(&key)
            .await
            .expect("load personal DM target succeeds")
            .expect("personal DM target persists");

        assert_eq!(stored_one, stored_two);
        assert_eq!(persisted, stored_one);
        assert!(matches!(persisted.dm_channel_id.as_str(), "D123" | "D456"));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_rejects_rebinding_actor_to_different_user() {
        let state = state();
        state
            .bind_user_identity(binding("user:alice"))
            .await
            .expect("first bind succeeds");
        let error = state
            .bind_user_identity(binding("user:bob"))
            .await
            .expect_err("rebind should fail");

        assert!(matches!(
            error,
            RebornUserIdentityBindingError::ProviderIdentityAlreadyBound
        ));
        assert_eq!(
            state
                .resolve_user_identity("slack", "install-alpha:U123")
                .await
                .expect("resolve succeeds"),
            Some(user("user:alice"))
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_persists_channel_routes_across_state_recreation() {
        let root = Arc::new(InMemoryBackend::default());
        let first = state_with_root(root.clone());
        let key = SlackChannelRouteKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            "T123".to_string(),
            "CENG".to_string(),
        )
        .unwrap();

        first
            .upsert_route(key.clone(), user("user:eng-team-agent"))
            .await
            .expect("upsert route");
        let second = state_with_root(root);

        assert_eq!(
            second
                .resolve_subject_user_id(&key)
                .await
                .expect("resolve route"),
            Some(user("user:eng-team-agent"))
        );
        let routes = second
            .list_routes(
                &TenantId::new("tenant-alpha").unwrap(),
                &installation(),
                "T123",
                0,
                100,
            )
            .await
            .expect("list routes");
        assert_eq!(routes.routes.len(), 1);
        assert_eq!(routes.routes[0].team_id, "T123");
        assert_eq!(routes.routes[0].channel_id, "CENG");
        assert_eq!(routes.routes[0].subject_user_id, "user:eng-team-agent");
        assert!(second.delete_route(&key).await.expect("delete route"));
        assert_eq!(
            second
                .resolve_subject_user_id(&key)
                .await
                .expect("resolve deleted route"),
            None
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_replaces_allowed_channel_routes() {
        let state = state();
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let assigner = crate::slack::slack_channel_routes::SlackChannelSubjectAssigner::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
        );
        let ceng = assigner
            .assignment_for("CENG".to_string())
            .expect("CENG assignment");
        let cops = assigner
            .assignment_for("COPS".to_string())
            .expect("COPS assignment");
        state
            .replace_managed_routes(
                &tenant_id,
                &installation_id,
                "T123",
                vec![cops.clone(), ceng.clone()],
            )
            .await
            .expect("initial replace succeeds");

        let manual_ops_subject = user("user:ops-agent");
        state
            .upsert_route(
                SlackChannelRouteKey::new(
                    tenant_id.clone(),
                    installation_id.clone(),
                    "T123".to_string(),
                    "COPS".to_string(),
                )
                .unwrap(),
                manual_ops_subject.clone(),
            )
            .await
            .expect("manual route succeeds");

        let replaced = state
            .replace_managed_routes(&tenant_id, &installation_id, "T123", vec![ceng.clone()])
            .await
            .expect("second replace succeeds");

        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].channel_id, "CENG");
        assert_eq!(replaced[0].subject_user_id, ceng.subject_user_id.as_str());
        assert_eq!(
            state
                .resolve_subject_user_id(
                    &SlackChannelRouteKey::new(
                        tenant_id.clone(),
                        installation_id.clone(),
                        "T123".to_string(),
                        "CENG".to_string(),
                    )
                    .unwrap(),
                )
                .await
                .expect("resolve retained other-subject route"),
            Some(ceng.subject_user_id)
        );
        assert_eq!(
            state
                .resolve_subject_user_id(
                    &SlackChannelRouteKey::new(
                        tenant_id,
                        installation_id,
                        "T123".to_string(),
                        "COPS".to_string(),
                    )
                    .unwrap(),
                )
                .await
                .expect("resolve removed route"),
            None
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_concurrent_managed_replace_serializes_team_updates() {
        let state = state();
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let assigner = crate::slack::slack_channel_routes::SlackChannelSubjectAssigner::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
        );

        let first = state.replace_managed_routes(
            &tenant_id,
            &installation_id,
            "T123",
            vec![
                assigner.assignment_for("CONE".to_string()).unwrap(),
                assigner.assignment_for("CTWO".to_string()).unwrap(),
            ],
        );
        let second = state.replace_managed_routes(
            &tenant_id,
            &installation_id,
            "T123",
            vec![assigner.assignment_for("CTHREE".to_string()).unwrap()],
        );
        let (first, second) = tokio::join!(first, second);
        first.expect("first replace succeeds");
        second.expect("second replace succeeds");

        let routes = state
            .list_routes(&tenant_id, &installation_id, "T123", 0, 100)
            .await
            .expect("list routes")
            .routes;
        let route_ids = routes
            .iter()
            .map(|route| route.channel_id.as_str())
            .collect::<Vec<_>>();
        assert!(
            route_ids == ["CONE", "CTWO"] || route_ids == ["CTHREE"],
            "final replacement must be one complete update, got {route_ids:?}"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_resolve_observes_cross_process_route_revocation() {
        let root = Arc::new(InMemoryBackend::default());
        let writer = state_with_root(root.clone());
        let reader = state_with_root(root);
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let assigner = crate::slack::slack_channel_routes::SlackChannelSubjectAssigner::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
        );
        let key = SlackChannelRouteKey::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
            "CENG".to_string(),
        )
        .unwrap();
        let assignment = assigner.assignment_for("CENG".to_string()).unwrap();

        writer
            .replace_managed_routes(
                &tenant_id,
                &installation_id,
                "T123",
                vec![assignment.clone()],
            )
            .await
            .expect("seed route");
        assert_eq!(
            reader
                .resolve_subject_user_id(&key)
                .await
                .expect("reader resolves seeded route"),
            Some(assignment.subject_user_id)
        );

        writer
            .replace_managed_routes(&tenant_id, &installation_id, "T123", Vec::new())
            .await
            .expect("revoke route");

        assert_eq!(
            reader
                .resolve_subject_user_id(&key)
                .await
                .expect("reader observes revoked route"),
            None
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_replace_steals_expired_route_lease() {
        let state = state();
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let assigner = crate::slack::slack_channel_routes::SlackChannelSubjectAssigner::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
        );
        let lock_path =
            FilesystemSlackHostState::<InMemoryBackend>::channel_route_team_replace_lock_path(
                &installation_id,
                "T123",
            )
            .expect("lock path");
        state
            .write_record(
                &lock_path,
                &StoredSlackChannelRouteReplaceLock {
                    nonce: "expired".to_string(),
                    expires_at: Utc::now() - chrono::Duration::seconds(1),
                },
                CasExpectation::Absent,
            )
            .await
            .expect("seed expired lock");

        let replaced = state
            .replace_managed_routes(
                &tenant_id,
                &installation_id,
                "T123",
                vec![assigner.assignment_for("CENG".to_string()).unwrap()],
            )
            .await
            .expect("replace succeeds after stealing expired lock");

        assert_eq!(replaced.len(), 1);
        let (lock, _) = state
            .read_record::<StoredSlackChannelRouteReplaceLock>(&lock_path)
            .await
            .expect("read lock")
            .expect("released lock is retained as an expired record");
        assert!(
            lock.expires_at <= Utc::now(),
            "successful replacement expires the stolen lock"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_replace_expires_lock_on_release() {
        let state = state();
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let assigner = crate::slack::slack_channel_routes::SlackChannelSubjectAssigner::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
        );

        state
            .replace_managed_routes(
                &tenant_id,
                &installation_id,
                "T123",
                vec![assigner.assignment_for("CENG".to_string()).unwrap()],
            )
            .await
            .expect("first replace succeeds");
        let second = state
            .replace_managed_routes(
                &tenant_id,
                &installation_id,
                "T123",
                vec![assigner.assignment_for("COPS".to_string()).unwrap()],
            )
            .await
            .expect("second replace should not wait for stale lock ttl");

        assert_eq!(second.len(), 1);
        assert_eq!(second[0].channel_id, "COPS");
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_replace_renews_lock_during_slow_route_writes() {
        let root = Arc::new(RouteLockTestBackend::delay_route_writes(
            CHANNEL_ROUTE_REPLACE_LOCK_RENEW_INTERVAL * 2,
        ));
        let state = state_with_backend(root.clone());
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let assigner = crate::slack::slack_channel_routes::SlackChannelSubjectAssigner::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
        );

        state
            .replace_managed_routes(
                &tenant_id,
                &installation_id,
                "T123",
                vec![assigner.assignment_for("CENG".to_string()).unwrap()],
            )
            .await
            .expect("replace succeeds while renewal task runs");

        assert!(
            root.lock_puts() >= 2,
            "lock must be written for acquisition and at least one renewal"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_replace_aborts_when_route_lease_renewal_fails() {
        let root = Arc::new(
            RouteLockTestBackend::delay_route_writes_and_reject_lock_renewal(
                CHANNEL_ROUTE_REPLACE_LOCK_RENEW_INTERVAL * 2,
            ),
        );
        let state = state_with_backend(root.clone());
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let assigner = crate::slack::slack_channel_routes::SlackChannelSubjectAssigner::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
        );

        let result = state
            .replace_managed_routes(
                &tenant_id,
                &installation_id,
                "T123",
                vec![assigner.assignment_for("CENG".to_string()).unwrap()],
            )
            .await;

        assert!(matches!(
            result,
            Err(SlackChannelRouteError::StoreUnavailable)
        ));
        assert!(
            root.lock_puts() >= 2,
            "test backend must exercise acquisition and failed renewal"
        );
        assert!(
            state
                .list_routes(&tenant_id, &installation_id, "T123", 0, 100)
                .await
                .expect("list routes after failed replacement")
                .routes
                .is_empty(),
            "replacement must not continue writing routes after lease renewal fails"
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_single_route_mutations_respect_active_replace_lease() {
        let state = state();
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let key = SlackChannelRouteKey::new(
            tenant_id,
            installation_id.clone(),
            "T123".to_string(),
            "CENG".to_string(),
        )
        .unwrap();
        let lock_path =
            FilesystemSlackHostState::<InMemoryBackend>::channel_route_team_replace_lock_path(
                &installation_id,
                "T123",
            )
            .expect("lock path");
        state
            .write_record(
                &lock_path,
                &StoredSlackChannelRouteReplaceLock {
                    nonce: "other-process".to_string(),
                    expires_at: Utc::now() + chrono::Duration::seconds(60),
                },
                CasExpectation::Absent,
            )
            .await
            .expect("seed active lock");

        assert!(matches!(
            state.upsert_route(key.clone(), user("user:first")).await,
            Err(SlackChannelRouteError::StoreUnavailable)
        ));
        assert_eq!(
            state
                .resolve_subject_user_id(&key)
                .await
                .expect("blocked upsert leaves no route"),
            None
        );

        state
            .upsert_route_record(key.clone(), user("user:first"))
            .await
            .expect("seed route without public mutation path");
        assert!(matches!(
            state.delete_route(&key).await,
            Err(SlackChannelRouteError::StoreUnavailable)
        ));
        assert_eq!(
            state
                .resolve_subject_user_id(&key)
                .await
                .expect("blocked delete leaves route"),
            Some(user("user:first"))
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_replace_rolls_back_when_route_write_fails() {
        let root = Arc::new(RouteLockTestBackend::normal());
        let state = state_with_backend(root.clone());
        let tenant_id = TenantId::new("tenant-alpha").unwrap();
        let installation_id = installation();
        let assigner = crate::slack::slack_channel_routes::SlackChannelSubjectAssigner::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
        );
        let old_key = SlackChannelRouteKey::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
            "COLD".to_string(),
        )
        .unwrap();
        let new_key = SlackChannelRouteKey::new(
            tenant_id.clone(),
            installation_id.clone(),
            "T123".to_string(),
            "CNEW".to_string(),
        )
        .unwrap();
        state
            .upsert_route(old_key.clone(), user("user:old"))
            .await
            .expect("seed old route");

        root.fail_next_route_writes(1);
        let result = state
            .replace_managed_routes(
                &tenant_id,
                &installation_id,
                "T123",
                vec![assigner.assignment_for("CNEW".to_string()).unwrap()],
            )
            .await;

        assert!(matches!(
            result,
            Err(SlackChannelRouteError::StoreUnavailable)
        ));
        assert_eq!(
            state
                .resolve_subject_user_id(&old_key)
                .await
                .expect("old route survives failed replacement"),
            Some(user("user:old"))
        );
        assert_eq!(
            state
                .resolve_subject_user_id(&new_key)
                .await
                .expect("failed replacement does not add new route"),
            None
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_rejects_cross_tenant_route_operations() {
        let state = state();
        let key = SlackChannelRouteKey::new(
            TenantId::new("tenant-other").unwrap(),
            installation(),
            "T123".to_string(),
            "CENG".to_string(),
        )
        .unwrap();

        assert!(matches!(
            state
                .upsert_route(key.clone(), user("user:eng-team-agent"))
                .await,
            Err(SlackChannelRouteError::InvalidRoute)
        ));
        assert!(
            !state
                .delete_route(&key)
                .await
                .expect("delete returns false")
        );
        assert_eq!(
            state
                .resolve_subject_user_id(&key)
                .await
                .expect("resolve returns none"),
            None
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_invalidates_route_cache_on_update_and_delete() {
        let state = state();
        let key = SlackChannelRouteKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            "T123".to_string(),
            "CENG".to_string(),
        )
        .unwrap();

        state
            .upsert_route(key.clone(), user("user:first"))
            .await
            .expect("first upsert");
        assert_eq!(
            state
                .resolve_subject_user_id(&key)
                .await
                .expect("first resolve"),
            Some(user("user:first"))
        );

        state
            .upsert_route(key.clone(), user("user:second"))
            .await
            .expect("second upsert");
        assert_eq!(
            state
                .resolve_subject_user_id(&key)
                .await
                .expect("second resolve"),
            Some(user("user:second"))
        );

        assert!(state.delete_route(&key).await.expect("delete"));
        assert_eq!(
            state
                .resolve_subject_user_id(&key)
                .await
                .expect("deleted resolve"),
            None
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_list_routes_skips_non_json_entries() {
        let state = state();
        let key = SlackChannelRouteKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            "T123".to_string(),
            "CENG".to_string(),
        )
        .unwrap();
        state
            .upsert_route(key, user("user:eng-team-agent"))
            .await
            .expect("upsert route");
        let junk_path = scoped_path(&format!(
            "{}/{}/{}/{}",
            CHANNEL_ROUTE_ROOT,
            path_segment(installation().as_str()),
            path_segment("T123"),
            "swap.tmp"
        ))
        .expect("junk path");
        state
            .write_record(
                &junk_path,
                &serde_json::json!({"not":"a route"}),
                CasExpectation::Any,
            )
            .await
            .expect("write junk record");

        let routes = state
            .list_routes(
                &TenantId::new("tenant-alpha").unwrap(),
                &installation(),
                "T123",
                0,
                100,
            )
            .await
            .expect("list routes");

        assert_eq!(routes.routes.len(), 1);
        assert_eq!(routes.routes[0].channel_id, "CENG");
    }

    fn state() -> FilesystemSlackHostState<InMemoryBackend> {
        state_with_root(Arc::new(InMemoryBackend::default()))
    }

    fn state_with_root(root: Arc<InMemoryBackend>) -> FilesystemSlackHostState<InMemoryBackend> {
        state_with_backend(root)
    }

    fn state_with_backend<F>(root: Arc<F>) -> FilesystemSlackHostState<F>
    where
        F: RootFilesystem + 'static,
    {
        let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
            root,
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/tenant-shared").unwrap(),
                VirtualPath::new("/tenants/tenant-alpha/shared").unwrap(),
                MountPermissions::read_write_list_delete(),
            )])
            .unwrap(),
        ));
        FilesystemSlackHostState::new(
            scoped,
            TenantId::new("tenant-alpha").unwrap(),
            user("user:host"),
            AgentId::new("agent:host").unwrap(),
            Some(ProjectId::new("project:host").unwrap()),
        )
    }

    struct RouteLockTestBackend {
        inner: InMemoryBackend,
        reject_lock_renewal: bool,
        route_write_delay: Option<Duration>,
        personal_dm_write_barrier: Option<Arc<tokio::sync::Barrier>>,
        personal_dm_puts: AtomicUsize,
        route_write_failures: AtomicUsize,
        connection_write_failures: AtomicUsize,
        lock_puts: AtomicUsize,
    }

    impl RouteLockTestBackend {
        fn normal() -> Self {
            Self {
                inner: InMemoryBackend::default(),
                reject_lock_renewal: false,
                route_write_delay: None,
                personal_dm_write_barrier: None,
                personal_dm_puts: AtomicUsize::new(0),
                route_write_failures: AtomicUsize::new(0),
                connection_write_failures: AtomicUsize::new(0),
                lock_puts: AtomicUsize::new(0),
            }
        }

        fn delay_route_writes(delay: Duration) -> Self {
            Self {
                inner: InMemoryBackend::default(),
                reject_lock_renewal: false,
                route_write_delay: Some(delay),
                personal_dm_write_barrier: None,
                personal_dm_puts: AtomicUsize::new(0),
                route_write_failures: AtomicUsize::new(0),
                connection_write_failures: AtomicUsize::new(0),
                lock_puts: AtomicUsize::new(0),
            }
        }

        fn delay_route_writes_and_reject_lock_renewal(delay: Duration) -> Self {
            Self {
                inner: InMemoryBackend::default(),
                reject_lock_renewal: true,
                route_write_delay: Some(delay),
                personal_dm_write_barrier: None,
                personal_dm_puts: AtomicUsize::new(0),
                route_write_failures: AtomicUsize::new(0),
                connection_write_failures: AtomicUsize::new(0),
                lock_puts: AtomicUsize::new(0),
            }
        }

        fn barrier_personal_dm_writes() -> Self {
            Self {
                inner: InMemoryBackend::default(),
                reject_lock_renewal: false,
                route_write_delay: None,
                personal_dm_write_barrier: Some(Arc::new(tokio::sync::Barrier::new(2))),
                personal_dm_puts: AtomicUsize::new(0),
                route_write_failures: AtomicUsize::new(0),
                connection_write_failures: AtomicUsize::new(0),
                lock_puts: AtomicUsize::new(0),
            }
        }

        fn fail_next_route_writes(&self, count: usize) {
            self.route_write_failures.store(count, Ordering::SeqCst);
        }

        fn fail_next_connection_write(&self) {
            self.connection_write_failures.store(1, Ordering::SeqCst);
        }

        fn lock_puts(&self) -> usize {
            self.lock_puts.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl RootFilesystem for RouteLockTestBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            if is_replace_lock_path(path) {
                let previous_puts = self.lock_puts.fetch_add(1, Ordering::SeqCst);
                if self.reject_lock_renewal && previous_puts > 0 {
                    return Err(FilesystemError::VersionMismatch {
                        path: path.clone(),
                        expected: Some(RecordVersion::from_backend(0)),
                        found: Some(RecordVersion::from_backend(1)),
                    });
                }
            } else if is_channel_route_record_path(path)
                && let Some(delay) = self.route_write_delay
            {
                tokio::time::sleep(delay).await;
            } else if is_personal_dm_target_record_path(path)
                && let Some(barrier) = &self.personal_dm_write_barrier
                && self.personal_dm_puts.fetch_add(1, Ordering::SeqCst) < 2
            {
                barrier.wait().await;
            }
            if is_channel_route_record_path(path)
                && self.route_write_failures.load(Ordering::SeqCst) > 0
            {
                self.route_write_failures.fetch_sub(1, Ordering::SeqCst);
                return Err(FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::WriteFile,
                    reason: "injected route write failure".to_string(),
                });
            }
            if is_connection_record_path(path)
                && self.connection_write_failures.load(Ordering::SeqCst) > 0
            {
                self.connection_write_failures
                    .fetch_sub(1, Ordering::SeqCst);
                return Err(FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::WriteFile,
                    reason: "injected connection write failure".to_string(),
                });
            }
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.inner.get(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn query(
            &self,
            path: &VirtualPath,
            filter: &Filter,
            page: Page,
        ) -> Result<Vec<VersionedEntry>, FilesystemError> {
            self.inner.query(path, filter, page).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }
    }

    fn is_replace_lock_path(path: &VirtualPath) -> bool {
        path.as_str().ends_with("/replace-lock")
    }

    fn is_channel_route_record_path(path: &VirtualPath) -> bool {
        path.as_str().contains("/slack-channel-routes/")
            && path.as_str().ends_with(".json")
            && !is_replace_lock_path(path)
    }

    fn is_personal_dm_target_record_path(path: &VirtualPath) -> bool {
        path.as_str()
            .contains("/slack-personal-binding/dm-targets/")
            && path.as_str().ends_with(".json")
    }

    fn is_connection_record_path(path: &VirtualPath) -> bool {
        path.as_str()
            .contains("/slack-personal-binding/connections/")
            && path.as_str().ends_with(".json")
    }

    fn binding(user_id: &str) -> RebornUserIdentityBinding {
        RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user(user_id),
        }
    }

    async fn read_identity(
        state: &FilesystemSlackHostState<InMemoryBackend>,
        provider: &str,
        provider_user_id: &str,
    ) -> StoredSlackUserIdentity {
        let path =
            FilesystemSlackHostState::<InMemoryBackend>::identity_path(provider, provider_user_id)
                .unwrap();
        state
            .read_record(&path)
            .await
            .unwrap()
            .expect("identity exists")
            .0
    }

    fn installation() -> AdapterInstallationId {
        AdapterInstallationId::new("install-alpha").unwrap()
    }

    fn connection_expiry() -> ironclaw_auth::Timestamp {
        Utc::now() + chrono::Duration::minutes(5)
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).unwrap()
    }
}
