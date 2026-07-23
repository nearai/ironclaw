//! Generic durable channel-identity binding store (extension-runtime §5.5,
//! migration H.4).
//!
//! One tenant-scoped filesystem store backs the post-OAuth channel identity
//! bindings for every channel extension: primary records keyed by
//! `(provider, provider_user_id)` under
//! `/tenant-shared/channel-identities/identities`, plus a best-effort
//! per-`(provider, user)` inverse index under
//! `/tenant-shared/channel-identities/identities-by-user` so connection
//! checks can resolve a bound caller by listing only that caller's own
//! bindings. The index is advisory: a missing marker only falls back to the
//! full scan, and readers verify the primary record before trusting a
//! marker, so a stale marker can never be a false positive.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RecordVersion,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
    ScopedPath, TenantId, UserId, VirtualPath,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::provider_identity::{
    RebornUserIdentityBinding, RebornUserIdentityBindingDeleteStore,
    RebornUserIdentityBindingError, RebornUserIdentityBindingStore, RebornUserIdentityLookup,
    RebornUserIdentityLookupError,
};

const CHANNEL_IDENTITY_ALIAS: &str = "/tenant-shared/channel-identities";
const IDENTITY_ROOT: &str = "/tenant-shared/channel-identities/identities";
const IDENTITY_BY_USER_ROOT: &str = "/tenant-shared/channel-identities/identities-by-user";

/// The per-scope mount view for the channel-identity subtree: one alias onto
/// the tenant's shared `channel-identities` root.
pub(crate) fn channel_identity_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, HostApiError> {
    let tenant = crate::resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(CHANNEL_IDENTITY_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/channel-identities"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

/// The generic filesystem-backed channel-identity binding store.
pub(crate) struct ChannelIdentityStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
    locks: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl std::fmt::Debug for ChannelIdentityStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelIdentityStore")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl ChannelIdentityStore {
    pub(crate) fn new(
        filesystem: Arc<dyn RootFilesystem>,
        tenant_id: TenantId,
        user_id: UserId,
    ) -> Self {
        let scoped = Arc::new(ScopedFilesystem::new(
            filesystem,
            channel_identity_mount_view,
        ));
        Self {
            filesystem: scoped,
            scope: ResourceScope {
                tenant_id,
                user_id,
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The (tenant, user) identity scope this store reads and writes under —
    /// captured by the channel-connection test bundle so its restart-survival
    /// reopen probe reconstructs the store with the same scoping production
    /// composed (`build_runtime`' channel egress scope). Tests only.
    #[cfg(feature = "test-support")]
    pub(crate) fn identity_scope_tenant_and_user(&self) -> (&TenantId, &UserId) {
        (&self.scope.tenant_id, &self.scope.user_id)
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
                reason: "channel-identity record is invalid JSON".into(),
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
                reason: "channel-identity record could not be serialized".into(),
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

    fn identity_path(
        provider: &str,
        provider_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{IDENTITY_ROOT}/{}/{}.json",
            path_segment(provider),
            path_segment(provider_user_id)
        ))
    }

    fn identity_user_index_dir(
        provider: &str,
        user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{IDENTITY_BY_USER_ROOT}/{}/{}",
            path_segment(provider),
            path_segment(user_id)
        ))
    }

    fn identity_user_index_path(
        provider: &str,
        user_id: &str,
        provider_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        // The marker file name reuses `path_segment(provider_user_id)`,
        // exactly like the primary record, so the primary path can be
        // rebuilt from a marker entry name without decoding.
        scoped_path(&format!(
            "{IDENTITY_BY_USER_ROOT}/{}/{}/{}.json",
            path_segment(provider),
            path_segment(user_id),
            path_segment(provider_user_id)
        ))
    }

    /// Best-effort write of the per-user index marker for a binding.
    async fn write_user_binding_index_marker(&self, binding: &RebornUserIdentityBinding) {
        let path = match Self::identity_user_index_path(
            binding.provider.as_str(),
            binding.user_id.as_str(),
            binding.provider_user_id.as_str(),
        ) {
            Ok(path) => path,
            Err(error) => {
                tracing::debug!(%error, "could not build channel user-binding index path");
                return;
            }
        };
        let marker = StoredUserBindingIndexMarker {
            provider_user_id: binding.provider_user_id.as_str().to_string(),
        };
        if let Err(error) = self.write_record(&path, &marker, CasExpectation::Any).await {
            tracing::debug!(
                %error,
                "failed to write channel user-binding index marker; connection check will fall back to a scan"
            );
        }
    }

    /// Best-effort delete of a per-user index marker.
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
        match self.filesystem.delete(&self.scope, &path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => {
                tracing::debug!(%error, "failed to delete channel user-binding index marker");
            }
        }
    }

    /// Fast-path connection check via the per-user index; `true` only after
    /// verifying the primary record still matches.
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
            let primary = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_lookup_fs_error)?;
            let Some((record, _)) = self
                .read_record::<StoredChannelUserIdentity>(&primary)
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

    async fn reconcile_identity_version_mismatch(
        &self,
        path: &ScopedPath,
        binding: &RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        let Some((existing, _)) = self
            .read_record::<StoredChannelUserIdentity>(path)
            .await
            .map_err(map_binding_fs_error)?
        else {
            return Err(RebornUserIdentityBindingError::Backend(
                "channel actor binding changed concurrently".into(),
            ));
        };
        if existing.user_id == binding.user_id.as_str() {
            return Ok(());
        }
        Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound)
    }
}

#[async_trait::async_trait]
impl RebornUserIdentityLookup for ChannelIdentityStore {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        let path = Self::identity_path(provider, provider_user_id).map_err(map_lookup_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredChannelUserIdentity>(&path)
            .await
            .map_err(map_lookup_fs_error)?
        else {
            return Ok(None);
        };
        let user_id = UserId::new(record.user_id)
            .map_err(|error| RebornUserIdentityLookupError::InvalidUserId(error.to_string()))?;
        Ok(Some(user_id))
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
        if self
            .user_binding_via_index_marker(provider, user_id, provider_user_id_prefix)
            .await?
        {
            return Ok(true);
        }
        // Bindings written before the index existed have no marker: fall
        // back to the full provider scan.
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
                .read_record::<StoredChannelUserIdentity>(&path)
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
impl RebornUserIdentityBindingStore for ChannelIdentityStore {
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
            .read_record::<StoredChannelUserIdentity>(&path)
            .await
            .map_err(map_binding_fs_error)?
        {
            if existing.user_id != binding.user_id.as_str() {
                return Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound);
            }
            let updated = StoredChannelUserIdentity::from_binding(&binding, existing.created_at);
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
            self.write_user_binding_index_marker(&binding).await;
            return Ok(());
        }

        let record = StoredChannelUserIdentity::from_binding(&binding, Utc::now());
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
        self.write_user_binding_index_marker(&binding).await;
        Ok(())
    }
}

#[async_trait::async_trait]
impl RebornUserIdentityBindingDeleteStore for ChannelIdentityStore {
    async fn delete_user_identity_bindings_for_user(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<usize, RebornUserIdentityBindingError> {
        let provider_dir = scoped_path(&format!("{IDENTITY_ROOT}/{}", path_segment(provider)))
            .map_err(map_binding_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &provider_dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(0),
            Err(error) => return Err(map_binding_fs_error(error)),
        };
        let mut deleted = 0;
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
                .read_record::<StoredChannelUserIdentity>(&path)
                .await
                .map_err(map_binding_fs_error)?
            else {
                continue;
            };
            if !identity_record_matches_user_binding(
                &candidate,
                provider,
                user_id,
                provider_user_id_prefix,
            ) {
                continue;
            }
            let lock = self.lock_for(format!(
                "identity:{}:{}",
                candidate.provider, candidate.provider_user_id
            ));
            let _guard = lock.lock().await;
            let Some((current, _)) = self
                .read_record::<StoredChannelUserIdentity>(&path)
                .await
                .map_err(map_binding_fs_error)?
            else {
                continue;
            };
            if !identity_record_matches_user_binding(
                &current,
                provider,
                user_id,
                provider_user_id_prefix,
            ) {
                continue;
            }
            match self.filesystem.delete(&self.scope, &path).await {
                Ok(()) => {
                    deleted += 1;
                    self.delete_user_binding_index_marker(
                        provider,
                        user_id.as_str(),
                        &current.provider_user_id,
                    )
                    .await;
                }
                Err(FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(map_binding_fs_error(error)),
            }
        }
        Ok(deleted)
    }
}

/// The durable identity record. Field-compatible with the pre-generic
/// channel-lane records so migration H.4 copies them forward verbatim
/// (modulo the installation-prefix rewrite).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StoredChannelUserIdentity {
    pub(crate) provider: String,
    pub(crate) provider_user_id: String,
    pub(crate) user_id: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl StoredChannelUserIdentity {
    fn from_binding(binding: &RebornUserIdentityBinding, created_at: DateTime<Utc>) -> Self {
        Self {
            provider: binding.provider.as_str().to_string(),
            provider_user_id: binding.provider_user_id.as_str().to_string(),
            user_id: binding.user_id.as_str().to_string(),
            created_at,
            updated_at: Utc::now(),
        }
    }
}

/// Per-user index marker; the file name encodes the `provider_user_id`.
#[derive(Debug, Serialize, Deserialize)]
struct StoredUserBindingIndexMarker {
    provider_user_id: String,
}

fn identity_record_matches_user_binding(
    record: &StoredChannelUserIdentity,
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

pub(crate) fn path_segment(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn scoped_path(raw: &str) -> Result<ScopedPath, FilesystemError> {
    ScopedPath::new(raw).map_err(|error| FilesystemError::BackendInfrastructure {
        operation: FilesystemOperation::WriteFile,
        reason: format!("invalid channel-identity path under {CHANNEL_IDENTITY_ALIAS}: {error}"),
    })
}

fn map_lookup_fs_error(error: FilesystemError) -> RebornUserIdentityLookupError {
    RebornUserIdentityLookupError::Backend(error.to_string())
}

fn map_binding_fs_error(error: FilesystemError) -> RebornUserIdentityBindingError {
    RebornUserIdentityBindingError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::InMemoryBackend;

    use crate::provider_identity::{RebornIdentityProviderId, RebornIdentityProviderUserId};

    use super::*;

    fn store() -> ChannelIdentityStore {
        ChannelIdentityStore::new(
            Arc::new(InMemoryBackend::new()),
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("operator").expect("user"),
        )
    }

    fn binding(provider_user_id: &str, user: &str) -> RebornUserIdentityBinding {
        RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("vendorx").expect("provider"),
            provider_user_id: RebornIdentityProviderUserId::new(provider_user_id)
                .expect("provider user id"),
            user_id: UserId::new(user).expect("user"),
        }
    }

    #[tokio::test]
    async fn bind_resolve_prefix_check_and_delete_round_trip() {
        let store = store();
        store
            .bind_user_identity(binding("install-1:U123", "user-alice"))
            .await
            .expect("bind");

        assert_eq!(
            store
                .resolve_user_identity("vendorx", "install-1:U123")
                .await
                .expect("resolve"),
            Some(UserId::new("user-alice").expect("user"))
        );
        assert!(
            store
                .user_has_provider_binding("vendorx", &UserId::new("user-alice").expect("user"))
                .await
                .expect("check")
        );
        assert!(
            store
                .user_has_provider_binding_with_provider_user_id_prefix(
                    "vendorx",
                    &UserId::new("user-alice").expect("user"),
                    Some("install-1:"),
                )
                .await
                .expect("prefix check")
        );
        assert!(
            !store
                .user_has_provider_binding_with_provider_user_id_prefix(
                    "vendorx",
                    &UserId::new("user-alice").expect("user"),
                    Some("install-2:"),
                )
                .await
                .expect("foreign prefix check")
        );

        let deleted = store
            .delete_user_identity_bindings_for_user(
                "vendorx",
                &UserId::new("user-alice").expect("user"),
                Some("install-1:U123"),
            )
            .await
            .expect("delete");
        assert_eq!(deleted, 1);
        assert_eq!(
            store
                .resolve_user_identity("vendorx", "install-1:U123")
                .await
                .expect("resolve after delete"),
            None
        );
        assert!(
            !store
                .user_has_provider_binding("vendorx", &UserId::new("user-alice").expect("user"))
                .await
                .expect("check after delete")
        );
    }

    #[tokio::test]
    async fn rebinding_to_a_different_user_is_rejected() {
        let store = store();
        store
            .bind_user_identity(binding("install-1:U123", "user-alice"))
            .await
            .expect("bind");

        let error = store
            .bind_user_identity(binding("install-1:U123", "user-bob"))
            .await
            .expect_err("identity already bound to another user");
        assert!(matches!(
            error,
            RebornUserIdentityBindingError::ProviderIdentityAlreadyBound
        ));

        // Same user re-binding is an idempotent refresh.
        store
            .bind_user_identity(binding("install-1:U123", "user-alice"))
            .await
            .expect("same-user rebind");
    }
}
