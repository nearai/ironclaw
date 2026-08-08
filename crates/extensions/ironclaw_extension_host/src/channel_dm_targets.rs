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

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    error::HostApiError,
    ids::{InvocationId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
    resource::{ResourceScope, resource_scope_path_segment},
};
use serde::{Deserialize, Serialize};

const CHANNEL_DM_TARGET_ALIAS: &str = "/tenant-shared/channel-dm-targets";

/// Canonical DM-target payload keys: the direct conversation's external
/// ref. One shape for folded and freshly-provisioned records — vendor
/// knowledge stays in the adapters that produce the ref and the codecs
/// that encode reply-target binding refs from it.
pub const DM_TARGET_SPACE_ID_KEY: &str = "space_id";
pub const DM_TARGET_CONVERSATION_ID_KEY: &str = "conversation_id";

/// Build the canonical DM-target payload for one direct conversation.
pub fn dm_target_payload(space_id: Option<&str>, conversation_id: &str) -> serde_json::Value {
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
pub struct ChannelDmTargetRecord {
    pub extension_id: String,
    pub user_id: String,
    /// The proven external actor id the target was provisioned for.
    pub external_actor_id: String,
    /// The direct conversation's external ref in the canonical
    /// [`dm_target_payload`] shape.
    pub target: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Typed store failures. Never carries payload material.
#[derive(Debug, thiserror::Error)]
pub enum ChannelDmTargetError {
    #[error("channel DM-target store unavailable")]
    StoreUnavailable,
}

/// The per-scope mount view: one alias onto the tenant's shared
/// `channel-dm-targets` root.
pub fn channel_dm_target_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(CHANNEL_DM_TARGET_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/channel-dm-targets"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

fn path_segment(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

/// The generic filesystem-backed channel DM-target store.
pub struct FilesystemChannelDmTargetStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
}

impl std::fmt::Debug for FilesystemChannelDmTargetStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemChannelDmTargetStore")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl FilesystemChannelDmTargetStore {
    pub fn new(filesystem: Arc<dyn RootFilesystem>, tenant_id: TenantId, user_id: UserId) -> Self {
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

    pub async fn load(
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
    ///
    /// An unchanged record short-circuits without writing. The post-admission
    /// backfill calls this for EVERY inbound direct message, and after the
    /// first one the stored row is already correct — so without this the
    /// steady state is one durable write per message, forever, whose only
    /// effect is a new `updated_at`. The existing record is loaded here
    /// anyway (to preserve `created_at`), so the comparison is free.
    pub async fn upsert(
        &self,
        extension_id: &str,
        user_id: &UserId,
        external_actor_id: String,
        target: serde_json::Value,
    ) -> Result<ChannelDmTargetRecord, ChannelDmTargetError> {
        let existing = self.load(extension_id, user_id).await?;
        if let Some(existing) = &existing
            && existing.external_actor_id == external_actor_id
            && existing.target == target
        {
            return Ok(existing.clone());
        }
        let created_at = existing
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
    pub async fn delete(
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

    fn store() -> FilesystemChannelDmTargetStore {
        FilesystemChannelDmTargetStore::new(
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

    /// The post-admission backfill calls `upsert` for EVERY inbound direct
    /// message, so an unchanged record must not rewrite the row — otherwise
    /// the steady state is one durable write per message whose only effect is
    /// a new `updated_at`.
    #[tokio::test]
    async fn unchanged_upsert_does_not_rewrite_the_record() {
        let store = store();
        let user = UserId::new("user-steady").expect("user");
        let target = serde_json::json!({"dm_channel_id": "D42"});

        let first = store
            .upsert("vendorx", &user, "U123".to_string(), target.clone())
            .await
            .expect("first upsert");

        let again = store
            .upsert("vendorx", &user, "U123".to_string(), target.clone())
            .await
            .expect("identical upsert");

        assert_eq!(
            again, first,
            "an identical upsert must return the stored record untouched"
        );
        assert_eq!(
            again.updated_at, first.updated_at,
            "an identical upsert must not bump updated_at — that is the write it avoided"
        );

        // A genuine change still writes.
        let changed = store
            .upsert(
                "vendorx",
                &user,
                "U123".to_string(),
                serde_json::json!({"dm_channel_id": "D99"}),
            )
            .await
            .expect("changed upsert");
        assert_eq!(changed.target["dm_channel_id"], "D99");
        assert_eq!(
            changed.created_at, first.created_at,
            "created_at still survives a real update"
        );

        // So does a change of external actor alone.
        let reactored = store
            .upsert(
                "vendorx",
                &user,
                "U999".to_string(),
                serde_json::json!({"dm_channel_id": "D99"}),
            )
            .await
            .expect("actor change upsert");
        assert_eq!(reactored.external_actor_id, "U999");
    }
}
