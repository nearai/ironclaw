//! Durable host state for Slack host-beta personal binding.
//!
//! The Slack ingress path starts before a Slack actor is bound to a Reborn
//! user, so this state is tenant-scoped and lives under `/tenant-shared`.
//! The underlying `ScopedFilesystem` still routes through host APIs and is
//! backed by the selected durable root filesystem in libSQL/Postgres builds.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FileType, FilesystemError, FilesystemOperation,
    RecordVersion, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use ironclaw_product_adapters::AdapterInstallationId;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::slack_actor_identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};
use crate::slack_channel_routes::{
    SlackChannelRoute, SlackChannelRouteAssignment, SlackChannelRouteError, SlackChannelRouteKey,
    SlackChannelRouteListPage, SlackChannelRouteStore,
};
use crate::slack_outbound_targets::{
    SlackPersonalDmTarget, SlackPersonalDmTargetError, SlackPersonalDmTargetKey,
    SlackPersonalDmTargetStore,
};
use crate::slack_personal_binding::{
    RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
};
use crate::slack_personal_binding_pairing::{
    IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingChallenge,
    SlackPersonalBindingPairingChallengeStore, SlackPersonalBindingPairingCode,
    SlackPersonalBindingPairingError,
};
use crate::slack_serve::SlackUserId;
use crate::slack_setup::{SlackInstallationSetup, SlackInstallationSetupStore, SlackSetupError};

const SLACK_HOST_STATE_ROOT: &str = "/tenant-shared/slack-personal-binding";
const SLACK_INSTALLATION_SETUP_PATH: &str = "/tenant-shared/slack-setup/installation.json";
const IDENTITY_ROOT: &str = "/tenant-shared/slack-personal-binding/identities";
const PAIRING_CODE_ROOT: &str = "/tenant-shared/slack-personal-binding/pairing/codes";
const PAIRING_ACTOR_ROOT: &str = "/tenant-shared/slack-personal-binding/pairing/actors";
const CHANNEL_ROUTE_ROOT: &str = "/tenant-shared/slack-channel-routes";
const PERSONAL_DM_TARGET_ROOT: &str = "/tenant-shared/slack-personal-binding/dm-targets";
const PAIRING_CODE_LEN: usize = 8;
const PAIRING_CODE_RETRIES: usize = 16;
const DEFAULT_PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
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
    pairing_ttl: Duration,
    locks: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl<F> Clone for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    fn clone(&self) -> Self {
        Self {
            filesystem: Arc::clone(&self.filesystem),
            scope: self.scope.clone(),
            pairing_ttl: self.pairing_ttl,
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
            .field("pairing_ttl", &self.pairing_ttl)
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
            pairing_ttl: DEFAULT_PAIRING_TTL,
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    fn with_pairing_ttl(mut self, pairing_ttl: Duration) -> Self {
        self.pairing_ttl = pairing_ttl;
        self
    }

    fn lock_for(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    async fn read_record<T>(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<(T, RecordVersion)>, FilesystemError>
    where
        T: DeserializeOwned,
    {
        let Some(versioned) = self.filesystem.get(&self.scope, path).await? else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&versioned.entry.body).map_err(|_| {
            FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::ReadFile,
                reason: "Slack host-state record is invalid JSON".into(),
            }
        })?;
        Ok(Some((value, versioned.version)))
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
        let body =
            serde_json::to_vec(value).map_err(|_| FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::WriteFile,
                reason: "Slack host-state record could not be serialized".into(),
            })?;
        self.filesystem
            .put(
                &self.scope,
                path,
                Entry::bytes(body).with_content_type(ContentType::json()),
                cas,
            )
            .await
    }

    async fn delete_record(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        self.filesystem.delete(&self.scope, path).await
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

    fn pairing_code_path(
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!("{}/{}.json", PAIRING_CODE_ROOT, code.as_str()))
    }

    fn pairing_actor_path(
        challenge: &SlackPersonalBindingPairingChallenge,
    ) -> Result<ScopedPath, FilesystemError> {
        Self::pairing_actor_path_for(&challenge.installation_id, challenge.slack_user_id.as_str())
    }

    fn pairing_actor_path_for(
        installation_id: &AdapterInstallationId,
        slack_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}.json",
            PAIRING_ACTOR_ROOT,
            path_segment(installation_id.as_str()),
            path_segment(slack_user_id)
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
            path_segment(&key.team_id),
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
        let path = Self::identity_path(provider, provider_user_id).map_err(map_lookup_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredSlackUserIdentity>(&path)
            .await
            .map_err(map_lookup_fs_error)?
        else {
            return Ok(None);
        };
        let user_id = UserId::new(record.user_id)
            .map_err(|error| RebornUserIdentityLookupError::InvalidUserId(error.to_string()))?;
        Ok(Some(user_id))
    }
}

#[async_trait::async_trait]
impl<F> RebornUserIdentityBindingStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn bind_user_identity(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        let path =
            Self::identity_path(binding.provider.as_str(), binding.provider_user_id.as_str())
                .map_err(map_binding_fs_error)?;
        let lock = self.lock_for(format!(
            "identity:{}:{}",
            binding.provider.as_str(),
            binding.provider_user_id.as_str()
        ));
        let _guard = lock.lock().await;
        if let Some((existing, version)) = self
            .read_record::<StoredSlackUserIdentity>(&path)
            .await
            .map_err(map_binding_fs_error)?
        {
            if existing.user_id != binding.user_id.as_str() {
                return Err(RebornUserIdentityBindingError::Backend(
                    "Slack actor is already bound to a different user".into(),
                ));
            }
            let updated = StoredSlackUserIdentity::from_binding(&binding, existing.created_at);
            match self
                .write_record(&path, &updated, CasExpectation::Version(version))
                .await
            {
                Ok(_) => {}
                Err(FilesystemError::VersionMismatch { .. }) => {
                    self.reconcile_identity_version_mismatch(&path, &binding)
                        .await?;
                }
                Err(error) => return Err(map_binding_fs_error(error)),
            }
            return Ok(());
        }

        let record = StoredSlackUserIdentity::from_binding(&binding, Utc::now());
        match self
            .write_record(&path, &record, CasExpectation::Absent)
            .await
        {
            Ok(_) => {}
            Err(FilesystemError::VersionMismatch { .. }) => {
                self.reconcile_identity_version_mismatch(&path, &binding)
                    .await?;
            }
            Err(error) => return Err(map_binding_fs_error(error)),
        }
        Ok(())
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn reconcile_identity_version_mismatch(
        &self,
        path: &ScopedPath,
        binding: &RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        let Some((existing, _)) = self
            .read_record::<StoredSlackUserIdentity>(path)
            .await
            .map_err(map_binding_fs_error)?
        else {
            return Err(RebornUserIdentityBindingError::Backend(
                "Slack actor binding changed concurrently".into(),
            ));
        };
        if existing.user_id == binding.user_id.as_str() {
            return Ok(());
        }
        Err(RebornUserIdentityBindingError::Backend(
            "Slack actor is already bound to a different user".into(),
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
        stored_personal_dm_target(record).map(Some)
    }

    async fn upsert_personal_dm_target(
        &self,
        target: SlackPersonalDmTarget,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        if target.key.tenant_id != self.scope.tenant_id {
            return Err(SlackPersonalDmTargetError::InvalidTarget);
        }
        let path =
            Self::personal_dm_target_path(&target.key).map_err(map_personal_dm_target_fs_error)?;
        // Lock key omits tenant_id: this store instance is pinned to one
        // tenant (cross-tenant writes are rejected above before locking),
        // so installation/team/user uniquely identify the record within
        // the instance. Revisit if a multi-tenant store instance ever
        // shares this lock map.
        let lock = self.lock_for(format!(
            "personal-dm:{}:{}:{}",
            target.key.installation_id.as_str(),
            target.key.team_id,
            target.key.user_id.as_str()
        ));
        let _guard = lock.lock().await;
        let existing = self.read_personal_dm_target_record(&path).await?;
        let created_at = existing
            .as_ref()
            .map(|(record, _)| record.created_at)
            .unwrap_or_else(Utc::now);
        let record = StoredSlackPersonalDmTarget::from_target(&target, created_at);
        let cas = existing
            .map(|(_, version)| CasExpectation::Version(version))
            .unwrap_or(CasExpectation::Absent);
        match self.write_record(&path, &record, cas).await {
            Ok(_) => Ok(target),
            Err(FilesystemError::VersionMismatch { .. }) => {
                // CAS lost to a concurrent writer. Read back and return the winning record.
                // This is last-write-wins semantics: whichever writer committed first wins.
                // This is safe today because provisioning the same (tenant, user) DM target is
                // idempotent — both concurrent writers store the same DM channel ID for the same
                // Slack user. If a future change makes the payload non-idempotent (e.g. storing a
                // caller-chosen dm_channel_id that could differ between writers), this branch must
                // be replaced with explicit conflict semantics (e.g. reject the loser with a
                // retriable error rather than silently discarding the losing value).
                let Some((record, _)) = self.read_personal_dm_target_record(&path).await? else {
                    return Err(SlackPersonalDmTargetError::StoreUnavailable);
                };
                stored_personal_dm_target(record)
            }
            Err(error) => Err(map_personal_dm_target_fs_error(error)),
        }
    }
}

#[async_trait::async_trait]
impl<F> SlackPersonalBindingPairingChallengeStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn issue_challenge(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let actor_path = Self::pairing_actor_path(&challenge).map_err(map_pairing_fs_error)?;
        let actor_lock = self.lock_for(format!(
            "pairing-actor:{}:{}",
            challenge.installation_id.as_str(),
            challenge.slack_user_id.as_str()
        ));
        let _actor_guard = actor_lock.lock().await;
        let existing_actor = self
            .read_record::<StoredSlackPairingActorChallenge>(&actor_path)
            .await
            .map_err(map_pairing_fs_error)?;
        if let Some((actor_record, _)) = existing_actor.as_ref()
            && let Some(issued) = self
                .active_actor_pairing_challenge(actor_record, &challenge)
                .await?
        {
            return Ok(issued);
        }
        if let Some((actor_record, _)) = existing_actor.as_ref()
            && actor_record.expires_at <= Utc::now()
        {
            self.cleanup_actor_pairing_code_record(actor_record).await;
        }

        let expires_at = Utc::now()
            + chrono::Duration::from_std(self.pairing_ttl).map_err(|_| {
                SlackPersonalBindingPairingError::Backend(
                    "Slack pairing TTL could not be represented".into(),
                )
            })?;
        for _ in 0..PAIRING_CODE_RETRIES {
            let code = SlackPersonalBindingPairingCode::new(random_pairing_code())?;
            let path = Self::pairing_code_path(&code).map_err(map_pairing_fs_error)?;
            let record = StoredSlackPairingChallenge::pending(&code, &challenge, expires_at);
            match self
                .write_record(&path, &record, CasExpectation::Absent)
                .await
            {
                Ok(_) => {
                    let actor_record =
                        StoredSlackPairingActorChallenge::pending(&code, &challenge, expires_at);
                    let actor_cas = existing_actor
                        .as_ref()
                        .map(|(_, version)| CasExpectation::Version(*version))
                        .unwrap_or(CasExpectation::Absent);
                    match self
                        .write_record(&actor_path, &actor_record, actor_cas)
                        .await
                    {
                        Ok(_) => {}
                        Err(FilesystemError::VersionMismatch { .. }) => {
                            self.cleanup_pairing_code_record(&path).await;
                            let Some((winner, _)) = self
                                .read_record::<StoredSlackPairingActorChallenge>(&actor_path)
                                .await
                                .map_err(map_pairing_fs_error)?
                            else {
                                continue;
                            };
                            if let Some(issued) = self
                                .active_actor_pairing_challenge(&winner, &challenge)
                                .await?
                            {
                                return Ok(issued);
                            }
                            continue;
                        }
                        Err(error) => {
                            self.cleanup_pairing_code_record(&path).await;
                            return Err(map_pairing_fs_error(error));
                        }
                    }
                    return Ok(IssuedSlackPersonalBindingPairingChallenge { code, challenge });
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_pairing_fs_error(error)),
            }
        }
        Err(SlackPersonalBindingPairingError::Backend(
            "could not allocate a unique Slack pairing code".into(),
        ))
    }

    async fn get_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let path = Self::pairing_code_path(code).map_err(map_pairing_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredSlackPairingChallenge>(&path)
            .await
            .map_err(map_pairing_fs_error)?
        else {
            return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
        };

        active_pairing_challenge(&record)
    }

    async fn consume_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let path = Self::pairing_code_path(code).map_err(map_pairing_fs_error)?;
        let lock = self.lock_for(format!("pairing:{}", code.as_str()));
        let _guard = lock.lock().await;
        let Some((mut record, version)) = self
            .read_record::<StoredSlackPairingChallenge>(&path)
            .await
            .map_err(map_pairing_fs_error)?
        else {
            return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
        };
        let challenge = active_pairing_challenge(&record)?;
        let actor_path = Self::pairing_actor_path_for(
            &challenge.installation_id,
            challenge.slack_user_id.as_str(),
        )
        .map_err(map_pairing_fs_error)?;
        let actor_lock = self.lock_for(format!(
            "pairing-actor:{}:{}",
            challenge.installation_id.as_str(),
            challenge.slack_user_id.as_str()
        ));
        let _actor_guard = actor_lock.lock().await;
        record.status = StoredSlackPairingStatus::Consumed;
        record.consumed_at = Some(Utc::now());
        match self
            .write_record(&path, &record, CasExpectation::Version(version))
            .await
        {
            Ok(_) => {}
            Err(FilesystemError::VersionMismatch { .. }) => {
                return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
            }
            Err(error) => return Err(map_pairing_fs_error(error)),
        }
        self.cleanup_pairing_code_record(&path).await;
        self.cleanup_pairing_actor_record(&actor_path, code).await;
        Ok(challenge)
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

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn active_actor_pairing_challenge(
        &self,
        actor_record: &StoredSlackPairingActorChallenge,
        requested: &SlackPersonalBindingPairingChallenge,
    ) -> Result<Option<IssuedSlackPersonalBindingPairingChallenge>, SlackPersonalBindingPairingError>
    {
        if actor_record.installation_id != requested.installation_id.as_str()
            || actor_record.slack_user_id != requested.slack_user_id.as_str()
            || actor_record.expires_at <= Utc::now()
        {
            return Ok(None);
        }
        let code = SlackPersonalBindingPairingCode::new(actor_record.code.clone())?;
        let path = Self::pairing_code_path(&code).map_err(map_pairing_fs_error)?;
        let Some((code_record, _)) = self
            .read_record::<StoredSlackPairingChallenge>(&path)
            .await
            .map_err(map_pairing_fs_error)?
        else {
            return Ok(None);
        };
        let challenge = match active_pairing_challenge(&code_record) {
            Ok(challenge) => challenge,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        if challenge == *requested {
            return Ok(Some(IssuedSlackPersonalBindingPairingChallenge {
                code,
                challenge,
            }));
        }
        Ok(None)
    }

    async fn cleanup_pairing_code_record(&self, path: &ScopedPath) {
        if self.delete_record(path).await.is_err() {
            tracing::warn!("failed to delete Slack pairing code record");
        }
    }

    async fn cleanup_actor_pairing_code_record(
        &self,
        actor_record: &StoredSlackPairingActorChallenge,
    ) {
        let Ok(code) = SlackPersonalBindingPairingCode::new(actor_record.code.clone()) else {
            return;
        };
        let Ok(path) = Self::pairing_code_path(&code) else {
            return;
        };
        self.cleanup_pairing_code_record(&path).await;
    }

    async fn cleanup_pairing_actor_record(
        &self,
        actor_path: &ScopedPath,
        code: &SlackPersonalBindingPairingCode,
    ) {
        let Some((mut record, version)) = (match self
            .read_record::<StoredSlackPairingActorChallenge>(actor_path)
            .await
        {
            Ok(Some((record, version))) if record.code == code.as_str() => Some((record, version)),
            Ok(Some(_)) | Ok(None) => None,
            Err(_) => {
                tracing::warn!("failed to read Slack pairing actor record for cleanup");
                None
            }
        }) else {
            return;
        };
        let now = Utc::now();
        record.expires_at = now;
        record.updated_at = now;
        match self
            .write_record(actor_path, &record, CasExpectation::Version(version))
            .await
        {
            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(_) => {
                tracing::warn!("failed to expire Slack pairing actor record for cleanup");
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackUserIdentity {
    provider: String,
    provider_user_id: String,
    user_id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredSlackUserIdentity {
    fn from_binding(binding: &RebornUserIdentityBinding, created_at: DateTime<Utc>) -> Self {
        Self {
            provider: binding.provider.as_str().to_string(),
            provider_user_id: binding.provider_user_id.as_str().to_string(),
            user_id: binding.user_id.as_str().to_string(),
            created_at,
            updated_at: Utc::now(),
        }
    }

    #[cfg(test)]
    fn binding(&self) -> Option<RebornUserIdentityBinding> {
        Some(RebornUserIdentityBinding {
            provider: crate::slack_personal_binding::RebornIdentityProviderId::new(
                self.provider.clone(),
            )
            .ok()?,
            provider_user_id: crate::slack_personal_binding::RebornIdentityProviderUserId::new(
                self.provider_user_id.clone(),
            )
            .ok()?,
            user_id: UserId::new(self.user_id.clone()).ok()?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackPersonalDmTarget {
    tenant_id: String,
    installation_id: String,
    team_id: String,
    user_id: String,
    slack_user_id: String,
    dm_channel_id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredSlackPersonalDmTarget {
    #[allow(
        dead_code,
        reason = "paired with the optional explicit Slack DM target upsert path"
    )]
    fn from_target(target: &SlackPersonalDmTarget, created_at: DateTime<Utc>) -> Self {
        Self {
            tenant_id: target.key.tenant_id.as_str().to_string(),
            installation_id: target.key.installation_id.as_str().to_string(),
            team_id: target.key.team_id.clone(),
            user_id: target.key.user_id.as_str().to_string(),
            slack_user_id: target.slack_user_id.as_str().to_string(),
            dm_channel_id: target.dm_channel_id.clone(),
            created_at,
            updated_at: Utc::now(),
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
        record.team_id,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredSlackPairingStatus {
    Pending,
    Consumed,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackPairingChallenge {
    code: String,
    installation_id: String,
    slack_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    setup_revision: Option<u64>,
    status: StoredSlackPairingStatus,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

impl StoredSlackPairingChallenge {
    fn pending(
        code: &SlackPersonalBindingPairingCode,
        challenge: &SlackPersonalBindingPairingChallenge,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            code: code.as_str().to_string(),
            installation_id: challenge.installation_id.as_str().to_string(),
            slack_user_id: challenge.slack_user_id.as_str().to_string(),
            setup_revision: challenge.setup_revision,
            status: StoredSlackPairingStatus::Pending,
            created_at: Utc::now(),
            expires_at,
            consumed_at: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackPairingActorChallenge {
    installation_id: String,
    slack_user_id: String,
    code: String,
    expires_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredSlackPairingActorChallenge {
    fn pending(
        code: &SlackPersonalBindingPairingCode,
        challenge: &SlackPersonalBindingPairingChallenge,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            installation_id: challenge.installation_id.as_str().to_string(),
            slack_user_id: challenge.slack_user_id.as_str().to_string(),
            code: code.as_str().to_string(),
            expires_at,
            updated_at: Utc::now(),
        }
    }
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
            let _ = self.handle.await;
            return Err(error);
        }
        let _ = self.stop.send(());
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

fn active_pairing_challenge(
    record: &StoredSlackPairingChallenge,
) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
    if record.status != StoredSlackPairingStatus::Pending || record.expires_at <= Utc::now() {
        return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
    }
    Ok(SlackPersonalBindingPairingChallenge {
        installation_id: AdapterInstallationId::new(record.installation_id.clone())
            .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?,
        slack_user_id: SlackUserId::new(record.slack_user_id.clone()),
        setup_revision: record.setup_revision,
    })
}

fn random_pairing_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0_u8; PAIRING_CODE_LEN];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|byte| ALPHABET[usize::from(*byte) % ALPHABET.len()] as char)
        .collect()
}

fn random_lock_nonce() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
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

fn map_pairing_fs_error(error: FilesystemError) -> SlackPersonalBindingPairingError {
    SlackPersonalBindingPairingError::Backend(error.to_string())
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

    use crate::slack_personal_binding::{RebornIdentityProviderId, RebornIdentityProviderUserId};

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
        assert_eq!(stored.binding(), Some(binding));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_persists_personal_dm_targets_across_state_recreation() {
        let root = Arc::new(InMemoryBackend::default());
        let writer = state_with_root(root.clone());
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            "T123".to_string(),
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
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_rejects_cross_tenant_personal_dm_target_operations() {
        let state = state();
        let foreign_key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-foreign").unwrap(),
            installation(),
            "T123".to_string(),
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
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reports_corrupt_personal_dm_target_as_unavailable() {
        let state = state();
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new("tenant-alpha").unwrap(),
            installation(),
            "T123".to_string(),
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
            "T123".to_string(),
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
            "T123".to_string(),
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

        assert!(matches!(error, RebornUserIdentityBindingError::Backend(_)));
        assert_eq!(
            state
                .resolve_user_identity("slack", "install-alpha:U123")
                .await
                .expect("resolve succeeds"),
            Some(user("user:alice"))
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_consumes_pairing_code_once() {
        let state = state();
        let issued = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");

        let consumed = state
            .consume_challenge(&issued.code)
            .await
            .expect("consume succeeds");

        assert_eq!(consumed, challenge());
        assert!(matches!(
            state.consume_challenge(&issued.code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_previews_pairing_code_without_consuming_it() {
        let state = state();
        let issued = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");

        let preview = state
            .get_challenge(&issued.code)
            .await
            .expect("preview succeeds");
        let consumed = state
            .consume_challenge(&issued.code)
            .await
            .expect("consume succeeds");

        assert_eq!(preview, challenge());
        assert_eq!(consumed, challenge());
        assert!(matches!(
            state.get_challenge(&issued.code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reuses_active_pairing_code_for_actor() {
        let state = state();

        let first = state
            .issue_challenge(challenge())
            .await
            .expect("first issue succeeds");
        let second = state
            .issue_challenge(challenge())
            .await
            .expect("second issue succeeds");

        assert_eq!(second.code, first.code);
        assert_eq!(second.challenge, challenge());
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_concurrent_consume_allows_exactly_one_success() {
        let state = Arc::new(state());
        let issued = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");
        let first_state = Arc::clone(&state);
        let second_state = Arc::clone(&state);
        let first_code = issued.code.clone();
        let second_code = issued.code.clone();

        let (first, second) = tokio::join!(
            first_state.consume_challenge(&first_code),
            second_state.consume_challenge(&second_code)
        );
        let successes = [&first, &second]
            .into_iter()
            .filter(|result| result.is_ok())
            .count();
        let not_found = [&first, &second]
            .into_iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(SlackPersonalBindingPairingError::ChallengeNotFound)
                )
            })
            .count();

        assert_eq!(successes, 1);
        assert_eq!(not_found, 1);
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reissues_after_consumed_actor_challenge() {
        let state = state();
        let consumed = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");

        state
            .consume_challenge(&consumed.code)
            .await
            .expect("consume succeeds");
        let reissued = state
            .issue_challenge(challenge())
            .await
            .expect("reissue succeeds");

        assert_ne!(reissued.code, consumed.code);
        assert_eq!(reissued.challenge, challenge());
        assert!(matches!(
            state.get_challenge(&consumed.code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
        assert_eq!(
            state
                .get_challenge(&reissued.code)
                .await
                .expect("reissued code remains active"),
            challenge()
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_rejects_expired_pairing_code() {
        let state = state().with_pairing_ttl(Duration::from_millis(1));
        let issued = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");
        tokio::time::sleep(Duration::from_millis(5)).await;

        assert!(matches!(
            state.consume_challenge(&issued.code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
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
        let assigner = crate::slack_channel_routes::SlackChannelSubjectAssigner::new(
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
        let assigner = crate::slack_channel_routes::SlackChannelSubjectAssigner::new(
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
        let assigner = crate::slack_channel_routes::SlackChannelSubjectAssigner::new(
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
        let assigner = crate::slack_channel_routes::SlackChannelSubjectAssigner::new(
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
        let assigner = crate::slack_channel_routes::SlackChannelSubjectAssigner::new(
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
        let assigner = crate::slack_channel_routes::SlackChannelSubjectAssigner::new(
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
        let assigner = crate::slack_channel_routes::SlackChannelSubjectAssigner::new(
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
        let assigner = crate::slack_channel_routes::SlackChannelSubjectAssigner::new(
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
        route_write_failures: AtomicUsize,
        lock_puts: AtomicUsize,
    }

    impl RouteLockTestBackend {
        fn normal() -> Self {
            Self {
                inner: InMemoryBackend::default(),
                reject_lock_renewal: false,
                route_write_delay: None,
                personal_dm_write_barrier: None,
                route_write_failures: AtomicUsize::new(0),
                lock_puts: AtomicUsize::new(0),
            }
        }

        fn delay_route_writes(delay: Duration) -> Self {
            Self {
                inner: InMemoryBackend::default(),
                reject_lock_renewal: false,
                route_write_delay: Some(delay),
                personal_dm_write_barrier: None,
                route_write_failures: AtomicUsize::new(0),
                lock_puts: AtomicUsize::new(0),
            }
        }

        fn delay_route_writes_and_reject_lock_renewal(delay: Duration) -> Self {
            Self {
                inner: InMemoryBackend::default(),
                reject_lock_renewal: true,
                route_write_delay: Some(delay),
                personal_dm_write_barrier: None,
                route_write_failures: AtomicUsize::new(0),
                lock_puts: AtomicUsize::new(0),
            }
        }

        fn barrier_personal_dm_writes() -> Self {
            Self {
                inner: InMemoryBackend::default(),
                reject_lock_renewal: false,
                route_write_delay: None,
                personal_dm_write_barrier: Some(Arc::new(tokio::sync::Barrier::new(2))),
                route_write_failures: AtomicUsize::new(0),
                lock_puts: AtomicUsize::new(0),
            }
        }

        fn fail_next_route_writes(&self, count: usize) {
            self.route_write_failures.store(count, Ordering::SeqCst);
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

    fn challenge() -> SlackPersonalBindingPairingChallenge {
        SlackPersonalBindingPairingChallenge {
            installation_id: installation(),
            slack_user_id: SlackUserId::new("U123"),
            setup_revision: None,
        }
    }

    fn installation() -> AdapterInstallationId {
        AdapterInstallationId::new("install-alpha").unwrap()
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).unwrap()
    }
}
