//! Durable reply-context store for the generic channel ingress (ING-11).
//!
//! One CAS-updated snapshot per `(extension, installation)` under
//! `/tenant-shared/reply-contexts/{extension}/{installation}.json`, holding
//! the latest `reply_context` per conversation with bounded FIFO eviction —
//! the same semantics the previous process-local store had, made durable so
//! a restart between admission and delivery no longer loses source-route
//! replies. Tests ride the same store over `InMemoryBackend`
//! (arch-simplification §4.3: in-memory is a backend, not a store).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_extension_host::ingress::{IngressPortError, ReplyContextKey, ReplyContextStorePort};
use ironclaw_filesystem::{
    CasApply, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
    ScopedPath, TenantId, UserId, VirtualPath,
};
use serde::{Deserialize, Serialize};

use crate::extension_host::channel_identity_store::path_segment;

const REPLY_CONTEXT_ALIAS: &str = "/tenant-shared/reply-contexts";

/// Latest-per-conversation entries retained per `(extension, installation)`
/// snapshot. The previous process-local store bounded the same way (oldest
/// conversation evicted first), but process-globally.
const REPLY_CONTEXT_CAP: usize = 1024;

/// The per-scope mount view: one alias onto the tenant's shared
/// `reply-contexts` root.
fn reply_context_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = crate::resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(REPLY_CONTEXT_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/reply-contexts"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReplyContextEntry {
    conversation: String,
    context: Vec<u8>,
    stored_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ReplyContextSnapshot {
    entries: Vec<ReplyContextEntry>,
}

/// Filesystem-backed [`ReplyContextStorePort`] shared by the ingress router
/// (write half) and the delivery coordinator (read half).
pub(crate) struct ReplyContextStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
}

impl std::fmt::Debug for ReplyContextStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplyContextStore")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl ReplyContextStore {
    pub(crate) fn new(
        filesystem: Arc<dyn RootFilesystem>,
        tenant_id: TenantId,
        user_id: UserId,
    ) -> Self {
        let scoped = Arc::new(ScopedFilesystem::new(filesystem, reply_context_mount_view));
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

    fn snapshot_path(key: &ReplyContextKey) -> Result<ScopedPath, IngressPortError> {
        ScopedPath::new(format!(
            "{REPLY_CONTEXT_ALIAS}/{}/{}.json",
            path_segment(&key.extension_id),
            path_segment(&key.installation_id)
        ))
        .map_err(|error| {
            tracing::debug!(%error, "invalid reply-context path");
            store_unavailable()
        })
    }
}

fn store_unavailable() -> IngressPortError {
    IngressPortError {
        reason: "reply-context store unavailable".to_string(),
    }
}

#[async_trait]
impl ReplyContextStorePort for ReplyContextStore {
    async fn put(&self, key: ReplyContextKey, context: Vec<u8>) -> Result<(), IngressPortError> {
        let path = Self::snapshot_path(&key)?;
        let conversation = key.conversation.clone();
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            |bytes| {
                serde_json::from_slice::<ReplyContextSnapshot>(bytes)
                    .map_err(|error| error.to_string())
            },
            |snapshot| {
                serde_json::to_vec(snapshot)
                    .map(|body| Entry::bytes(body).with_content_type(ContentType::json()))
                    .map_err(|error| error.to_string())
            },
            |current: Option<ReplyContextSnapshot>| {
                let conversation = conversation.clone();
                let context = context.clone();
                async move {
                    let mut snapshot = current.unwrap_or_default();
                    snapshot
                        .entries
                        .retain(|entry| entry.conversation != conversation);
                    snapshot.entries.push(ReplyContextEntry {
                        conversation,
                        context,
                        stored_at: Utc::now(),
                    });
                    if snapshot.entries.len() > REPLY_CONTEXT_CAP {
                        let excess = snapshot.entries.len() - REPLY_CONTEXT_CAP;
                        snapshot.entries.drain(0..excess);
                    }
                    Ok::<_, String>(CasApply::new(snapshot, ()))
                }
            },
        )
        .await
        .map_err(|error| {
            tracing::debug!(?error, "reply-context put failed");
            store_unavailable()
        })
    }

    async fn get(&self, key: &ReplyContextKey) -> Result<Option<Vec<u8>>, IngressPortError> {
        let path = Self::snapshot_path(key)?;
        let versioned = match self.filesystem.get(&self.scope, &path).await {
            Ok(versioned) => versioned,
            Err(FilesystemError::NotFound { .. }) => return Ok(None),
            Err(error) => {
                tracing::debug!(%error, "reply-context get failed");
                return Err(store_unavailable());
            }
        };
        let Some(versioned) = versioned else {
            return Ok(None);
        };
        let snapshot = match serde_json::from_slice::<ReplyContextSnapshot>(&versioned.entry.body) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(%error, "malformed reply-context snapshot");
                return Ok(None);
            }
        };
        Ok(snapshot
            .entries
            .iter()
            .find(|entry| entry.conversation == key.conversation)
            .map(|entry| entry.context.clone()))
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::InMemoryBackend;

    use super::*;

    fn key(conversation: &str) -> ReplyContextKey {
        ReplyContextKey {
            extension_id: "vendorx".to_string(),
            installation_id: "install-1".to_string(),
            conversation: conversation.to_string(),
        }
    }

    fn store_over(backend: Arc<InMemoryBackend>) -> ReplyContextStore {
        ReplyContextStore::new(
            backend,
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("operator").expect("user"),
        )
    }

    #[tokio::test]
    async fn put_get_round_trip_keeps_latest_context_per_conversation() {
        let store = store_over(Arc::new(InMemoryBackend::new()));

        assert!(store.get(&key("c-1")).await.expect("get").is_none());

        store.put(key("c-1"), b"first".to_vec()).await.expect("put");
        store
            .put(key("c-1"), b"second".to_vec())
            .await
            .expect("re-put");
        store.put(key("c-2"), b"other".to_vec()).await.expect("put");

        assert_eq!(
            store.get(&key("c-1")).await.expect("get"),
            Some(b"second".to_vec())
        );
        assert_eq!(
            store.get(&key("c-2")).await.expect("get"),
            Some(b"other".to_vec())
        );

        // Foreign installation resolves nothing.
        let foreign = ReplyContextKey {
            installation_id: "install-2".to_string(),
            ..key("c-1")
        };
        assert!(store.get(&foreign).await.expect("get").is_none());
    }

    /// The regression this store exists for: the previous process-local
    /// store lost every pre-admission reply context on restart, so a
    /// source-route reply after a restart had no context to bind to. A new
    /// store instance over the same filesystem must read back what the old
    /// instance wrote.
    #[tokio::test]
    async fn contexts_survive_store_recreation_over_the_same_filesystem() {
        let backend = Arc::new(InMemoryBackend::new());
        let before_restart = store_over(Arc::clone(&backend));
        before_restart
            .put(key("c-1"), b"survives".to_vec())
            .await
            .expect("put");
        drop(before_restart);

        let after_restart = store_over(backend);
        assert_eq!(
            after_restart.get(&key("c-1")).await.expect("get"),
            Some(b"survives".to_vec())
        );
    }

    #[tokio::test]
    async fn oldest_conversation_is_evicted_beyond_the_cap() {
        let store = store_over(Arc::new(InMemoryBackend::new()));

        for index in 0..=REPLY_CONTEXT_CAP {
            store
                .put(key(&format!("c-{index}")), vec![1])
                .await
                .expect("put");
        }

        // The first conversation fell off; the newest is present.
        assert!(store.get(&key("c-0")).await.expect("get").is_none());
        assert_eq!(
            store
                .get(&key(&format!("c-{REPLY_CONTEXT_CAP}")))
                .await
                .expect("get"),
            Some(vec![1])
        );
    }
}
