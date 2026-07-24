//! Generic durable per-user channel DM-target store (extension-runtime
//! §5.4, migration H.4).
//!
//! One record per `(extension, user)` under
//! `/tenant-shared/channel-dm-targets/{extension}/{user}.json`: the proven
//! external actor id plus the direct conversation's external ref in the
//! canonical payload shape ([`dm_target_payload`]). The extension's
//! outbound-target surface encodes reply-target binding refs from it —
//! vendor knowledge stays in the adapters and codecs, never here.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
    ScopedPath, TenantId, UserId, VirtualPath,
};
use serde::{Deserialize, Serialize};

use crate::extension_host::channel_identity_store::path_segment;

const CHANNEL_DM_TARGET_ALIAS: &str = "/tenant-shared/channel-dm-targets";

/// Canonical DM-target payload keys: the direct conversation's external
/// ref. One shape for folded and freshly-provisioned records — vendor
/// knowledge stays in the adapters that produce the ref and the codecs
/// that encode reply-target binding refs from it.
pub(crate) const DM_TARGET_SPACE_ID_KEY: &str = "space_id";
pub(crate) const DM_TARGET_CONVERSATION_ID_KEY: &str = "conversation_id";

/// Build the canonical DM-target payload for one direct conversation.
pub(crate) fn dm_target_payload(
    space_id: Option<&str>,
    conversation_id: &str,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    if let Some(space_id) = space_id {
        payload.insert(
            DM_TARGET_SPACE_ID_KEY.to_string(),
            serde_json::Value::String(space_id.to_string()),
        );
    }
    payload.insert(
        DM_TARGET_CONVERSATION_ID_KEY.to_string(),
        serde_json::Value::String(conversation_id.to_string()),
    );
    serde_json::Value::Object(payload)
}

/// One user's DM target for one channel extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChannelDmTargetRecord {
    pub(crate) extension_id: String,
    pub(crate) user_id: String,
    /// The proven external actor id the target was provisioned for.
    pub(crate) external_actor_id: String,
    /// The direct conversation's external ref in the canonical
    /// [`dm_target_payload`] shape.
    pub(crate) target: serde_json::Value,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

/// Typed store failures. Never carries payload material.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ChannelDmTargetError {
    #[error("channel DM-target store unavailable")]
    StoreUnavailable,
}

/// The per-scope mount view: one alias onto the tenant's shared
/// `channel-dm-targets` root.
pub(crate) fn channel_dm_target_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, HostApiError> {
    let tenant = crate::resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(CHANNEL_DM_TARGET_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/channel-dm-targets"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

/// The generic filesystem-backed channel DM-target store.
pub(crate) struct ChannelDmTargetStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
}

impl std::fmt::Debug for ChannelDmTargetStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelDmTargetStore")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl ChannelDmTargetStore {
    pub(crate) fn new(
        filesystem: Arc<dyn RootFilesystem>,
        tenant_id: TenantId,
        user_id: UserId,
    ) -> Self {
        let scoped = Arc::new(ScopedFilesystem::new(
            filesystem,
            channel_dm_target_mount_view,
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
        }
    }

    fn target_path(extension_id: &str, user_id: &str) -> Result<ScopedPath, FilesystemError> {
        ScopedPath::new(format!(
            "{CHANNEL_DM_TARGET_ALIAS}/{}/{}.json",
            path_segment(extension_id),
            path_segment(user_id)
        ))
        .map_err(|error| FilesystemError::BackendInfrastructure {
            operation: FilesystemOperation::WriteFile,
            reason: format!("invalid channel DM-target path: {error}"),
        })
    }

    pub(crate) async fn load(
        &self,
        extension_id: &str,
        user_id: &UserId,
    ) -> Result<Option<ChannelDmTargetRecord>, ChannelDmTargetError> {
        let path = Self::target_path(extension_id, user_id.as_str()).map_err(map_fs_error)?;
        let versioned = match self.filesystem.get(&self.scope, &path).await {
            Ok(versioned) => versioned,
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(map_fs_error(error)),
        };
        let Some(versioned) = versioned else {
            return Ok(None);
        };
        match serde_json::from_slice::<ChannelDmTargetRecord>(&versioned.entry.body) {
            Ok(record) => Ok(Some(record)),
            Err(error) => {
                tracing::warn!(%error, extension_id, "malformed channel DM-target record");
                Ok(None)
            }
        }
    }

    /// Upsert one user's DM target for an extension. `created_at` is
    /// preserved across updates.
    pub(crate) async fn upsert(
        &self,
        extension_id: &str,
        user_id: &UserId,
        external_actor_id: String,
        target: serde_json::Value,
    ) -> Result<ChannelDmTargetRecord, ChannelDmTargetError> {
        let created_at = self
            .load(extension_id, user_id)
            .await?
            .map(|existing| existing.created_at)
            .unwrap_or_else(Utc::now);
        let record = ChannelDmTargetRecord {
            extension_id: extension_id.to_string(),
            user_id: user_id.as_str().to_string(),
            external_actor_id,
            target,
            created_at,
            updated_at: Utc::now(),
        };
        let path = Self::target_path(extension_id, user_id.as_str()).map_err(map_fs_error)?;
        let body =
            serde_json::to_vec(&record).map_err(|_| ChannelDmTargetError::StoreUnavailable)?;
        self.filesystem
            .put(
                &self.scope,
                &path,
                Entry::bytes(body).with_content_type(ContentType::json()),
                CasExpectation::Any,
            )
            .await
            .map_err(map_fs_error)?;
        Ok(record)
    }

    /// Delete one user's DM target for an extension (idempotent) — the
    /// generic disconnect cleanup drops the caller's provisioned target so
    /// outbound targets never offer a stale direct conversation.
    pub(crate) async fn delete(
        &self,
        extension_id: &str,
        user_id: &UserId,
    ) -> Result<(), ChannelDmTargetError> {
        let path = Self::target_path(extension_id, user_id.as_str()).map_err(map_fs_error)?;
        match self.filesystem.delete(&self.scope, &path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_fs_error(error)),
        }
    }
}

fn map_fs_error(error: FilesystemError) -> ChannelDmTargetError {
    tracing::debug!(%error, "channel DM-target filesystem operation failed");
    ChannelDmTargetError::StoreUnavailable
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::InMemoryBackend;

    use super::*;

    fn store() -> ChannelDmTargetStore {
        ChannelDmTargetStore::new(
            Arc::new(InMemoryBackend::new()),
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("operator").expect("user"),
        )
    }

    #[tokio::test]
    async fn upsert_load_delete_round_trip_preserves_created_at() {
        let store = store();
        let user = UserId::new("user-alice").expect("user");

        assert!(store.load("vendorx", &user).await.expect("load").is_none());

        let first = store
            .upsert(
                "vendorx",
                &user,
                "U123".to_string(),
                serde_json::json!({"dm_channel_id": "D42"}),
            )
            .await
            .expect("upsert");
        let updated = store
            .upsert(
                "vendorx",
                &user,
                "U123".to_string(),
                serde_json::json!({"dm_channel_id": "D43"}),
            )
            .await
            .expect("re-upsert");
        assert_eq!(updated.created_at, first.created_at);
        assert_eq!(updated.target["dm_channel_id"], "D43");

        let loaded = store
            .load("vendorx", &user)
            .await
            .expect("load")
            .expect("record present");
        assert_eq!(loaded, updated);

        // Foreign extension key resolves nothing.
        assert!(store.load("other", &user).await.expect("load").is_none());

        store.delete("vendorx", &user).await.expect("delete");
        assert!(store.load("vendorx", &user).await.expect("load").is_none());
        store
            .delete("vendorx", &user)
            .await
            .expect("second delete is idempotent");
    }
}
