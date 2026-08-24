//! Durable reply-context store for the generic channel ingress (ING-11).
//!
//! Context bytes live in immutable content-addressed rows. One small
//! CAS-updated catalog per `(extension, installation)` points at the current
//! row for each conversation and provides bounded FIFO eviction. Updating one
//! conversation therefore never reads, clones, or rewrites another
//! conversation's opaque bytes. The immediately preceding aggregate snapshot
//! remains a private one-version migration reader.

use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasApply, CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    error::HostApiError,
    ids::{InvocationId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
    resource::{ResourceScope, resource_scope_path_segment},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ingress::{IngressPortError, ReplyContextKey, ReplyContextStore};

const REPLY_CONTEXT_ALIAS: &str = "/tenant-shared/reply-contexts";

/// Latest-per-conversation entries retained per `(extension, installation)`.
const REPLY_CONTEXT_CAP: usize = 1024;

/// The per-scope mount view: one alias onto the tenant's shared
/// `reply-contexts` root.
fn reply_context_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(REPLY_CONTEXT_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/reply-contexts"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

fn path_segment(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReplyContextEntry {
    conversation: String,
    context: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReplyContextCatalogEntry {
    conversation_hash: String,
    content_hash: String,
    stored_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ReplyContextCatalog {
    entries: Vec<ReplyContextCatalogEntry>,
}

/// Immediately preceding aggregate wire shape. This type is migration-only;
/// new writes never serialize opaque context bytes into a shared snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct LegacyReplyContextSnapshot {
    entries: Vec<LegacyReplyContextEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LegacyReplyContextEntry {
    conversation: String,
    context: Vec<u8>,
    stored_at: DateTime<Utc>,
}

/// Filesystem-backed [`ReplyContextStore`] shared by the ingress router
/// (write half) and the delivery coordinator (read half).
pub struct FilesystemReplyContextStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
}

impl std::fmt::Debug for FilesystemReplyContextStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemReplyContextStore")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl FilesystemReplyContextStore {
    pub fn new(filesystem: Arc<dyn RootFilesystem>, tenant_id: TenantId, user_id: UserId) -> Self {
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

    fn installation_prefix(key: &ReplyContextKey) -> String {
        format!(
            "{REPLY_CONTEXT_ALIAS}/{}/{}",
            path_segment(&key.extension_id),
            path_segment(&key.installation_id)
        )
    }

    fn legacy_snapshot_path(key: &ReplyContextKey) -> Result<ScopedPath, IngressPortError> {
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

    fn catalog_path(key: &ReplyContextKey) -> Result<ScopedPath, IngressPortError> {
        ScopedPath::new(format!("{}/catalog.json", Self::installation_prefix(key))).map_err(
            |error| {
                tracing::debug!(%error, "invalid reply-context catalog path");
                store_unavailable()
            },
        )
    }

    fn entry_path(
        key: &ReplyContextKey,
        conversation_hash: &str,
        content_hash: &str,
    ) -> Result<ScopedPath, IngressPortError> {
        ScopedPath::new(format!(
            "{}/entries/{conversation_hash}/{content_hash}.json",
            Self::installation_prefix(key)
        ))
        .map_err(|error| {
            tracing::debug!(%error, "invalid reply-context entry path");
            store_unavailable()
        })
    }

    async fn load_catalog(
        &self,
        key: &ReplyContextKey,
    ) -> Result<Option<ReplyContextCatalog>, IngressPortError> {
        let path = Self::catalog_path(key)?;
        let Some(versioned) = self
            .filesystem
            .get(&self.scope, &path)
            .await
            .map_err(|error| {
                tracing::debug!(%error, "reply-context catalog read failed");
                store_unavailable()
            })?
        else {
            return Ok(None);
        };
        serde_json::from_slice(&versioned.entry.body)
            .map(Some)
            .map_err(|error| {
                tracing::warn!(%error, "malformed reply-context catalog");
                store_unavailable()
            })
    }

    async fn write_immutable_entry(
        &self,
        path: &ScopedPath,
        entry: &ReplyContextEntry,
    ) -> Result<(), IngressPortError> {
        let body = serde_json::to_vec(entry).map_err(|error| {
            tracing::debug!(%error, "reply-context entry serialization failed");
            store_unavailable()
        })?;
        let stored = Entry::bytes(body.clone()).with_content_type(ContentType::json());
        match self
            .filesystem
            .put(&self.scope, path, stored, CasExpectation::Absent)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                let existing = self
                    .filesystem
                    .get(&self.scope, path)
                    .await
                    .map_err(|error| {
                        tracing::debug!(%error, "reply-context entry readback failed");
                        store_unavailable()
                    })?;
                if existing.is_some_and(|versioned| versioned.entry.body == body) {
                    Ok(())
                } else {
                    tracing::warn!("reply-context content-address collision");
                    Err(store_unavailable())
                }
            }
            Err(error) => {
                tracing::debug!(%error, "reply-context entry write failed");
                Err(store_unavailable())
            }
        }
    }

    async fn migrate_legacy_snapshot(&self, key: &ReplyContextKey) -> Result<(), IngressPortError> {
        if self.load_catalog(key).await?.is_some() {
            return Ok(());
        }
        let legacy_path = Self::legacy_snapshot_path(key)?;
        let Some(versioned) = self
            .filesystem
            .get(&self.scope, &legacy_path)
            .await
            .map_err(|error| {
                tracing::debug!(%error, "legacy reply-context snapshot read failed");
                store_unavailable()
            })?
        else {
            return Ok(());
        };
        let legacy: LegacyReplyContextSnapshot = serde_json::from_slice(&versioned.entry.body)
            .map_err(|error| {
                tracing::warn!(%error, "malformed legacy reply-context snapshot");
                store_unavailable()
            })?;
        let mut catalog = ReplyContextCatalog::default();
        for legacy_entry in legacy.entries {
            let conversation_hash = hash_bytes(legacy_entry.conversation.as_bytes());
            let content_hash =
                context_content_hash(&legacy_entry.conversation, legacy_entry.context.as_slice());
            let entry = ReplyContextEntry {
                conversation: legacy_entry.conversation,
                context: legacy_entry.context,
            };
            let path = Self::entry_path(key, &conversation_hash, &content_hash)?;
            self.write_immutable_entry(&path, &entry).await?;
            catalog.entries.push(ReplyContextCatalogEntry {
                conversation_hash,
                content_hash,
                stored_at: legacy_entry.stored_at,
            });
        }
        if catalog.entries.len() > REPLY_CONTEXT_CAP {
            catalog.entries.sort_by_key(|entry| entry.stored_at);
            let excess = catalog.entries.len() - REPLY_CONTEXT_CAP;
            catalog.entries.drain(0..excess);
        }
        let catalog_path = Self::catalog_path(key)?;
        let body = serde_json::to_vec(&catalog).map_err(|error| {
            tracing::debug!(%error, "reply-context catalog serialization failed");
            store_unavailable()
        })?;
        match self
            .filesystem
            .put(
                &self.scope,
                &catalog_path,
                Entry::bytes(body).with_content_type(ContentType::json()),
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => Ok(()),
            Err(error) => {
                tracing::debug!(%error, "reply-context legacy migration failed");
                Err(store_unavailable())
            }
        }
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}

fn context_content_hash(conversation: &str, context: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(conversation.as_bytes());
    hasher.update([0]);
    hasher.update(context);
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn store_unavailable() -> IngressPortError {
    IngressPortError {
        reason: "reply-context store unavailable".to_string(),
    }
}

#[async_trait]
impl ReplyContextStore for FilesystemReplyContextStore {
    async fn put(&self, key: ReplyContextKey, context: Vec<u8>) -> Result<(), IngressPortError> {
        self.migrate_legacy_snapshot(&key).await?;
        let conversation_hash = hash_bytes(key.conversation.as_bytes());
        let content_hash = context_content_hash(&key.conversation, &context);
        let entry_path = Self::entry_path(&key, &conversation_hash, &content_hash)?;
        self.write_immutable_entry(
            &entry_path,
            &ReplyContextEntry {
                conversation: key.conversation.clone(),
                context,
            },
        )
        .await?;

        let catalog_path = Self::catalog_path(&key)?;
        let stored_at = Utc::now();
        let stale_entries = cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &catalog_path,
            |bytes| {
                serde_json::from_slice::<ReplyContextCatalog>(bytes)
                    .map_err(|error| error.to_string())
            },
            |catalog| {
                serde_json::to_vec(catalog)
                    .map(|body| Entry::bytes(body).with_content_type(ContentType::json()))
                    .map_err(|error| error.to_string())
            },
            |current: Option<ReplyContextCatalog>| {
                let conversation_hash = conversation_hash.clone();
                let content_hash = content_hash.clone();
                async move {
                    let mut catalog = current.unwrap_or_default();
                    let mut stale = catalog
                        .entries
                        .iter()
                        .filter(|entry| entry.conversation_hash == conversation_hash)
                        .cloned()
                        .collect::<Vec<_>>();
                    catalog
                        .entries
                        .retain(|entry| entry.conversation_hash != conversation_hash);
                    catalog.entries.push(ReplyContextCatalogEntry {
                        conversation_hash,
                        content_hash,
                        stored_at,
                    });
                    if catalog.entries.len() > REPLY_CONTEXT_CAP {
                        let excess = catalog.entries.len() - REPLY_CONTEXT_CAP;
                        stale.extend(catalog.entries.drain(0..excess));
                    }
                    Ok::<_, String>(CasApply::new(catalog, stale))
                }
            },
        )
        .await
        .map_err(|error| {
            tracing::debug!(?error, "reply-context put failed");
            store_unavailable()
        })?;

        for stale in stale_entries {
            if stale.conversation_hash == conversation_hash && stale.content_hash == content_hash {
                continue;
            }
            let stale_path = Self::entry_path(&key, &stale.conversation_hash, &stale.content_hash)?;
            if let Err(error) = self.filesystem.delete(&self.scope, &stale_path).await
                && !matches!(error, FilesystemError::NotFound { .. })
            {
                tracing::debug!(%error, "stale reply-context cleanup failed");
            }
        }
        Ok(())
    }

    async fn get(&self, key: &ReplyContextKey) -> Result<Option<Vec<u8>>, IngressPortError> {
        self.migrate_legacy_snapshot(key).await?;
        let conversation_hash = hash_bytes(key.conversation.as_bytes());
        // A concurrent replacement may retire the row named by our first
        // catalog read. Re-read once instead of turning that benign race into
        // a missing reply anchor.
        for _ in 0..2 {
            let Some(catalog) = self.load_catalog(key).await? else {
                return Ok(None);
            };
            let Some(current) = catalog
                .entries
                .iter()
                .find(|entry| entry.conversation_hash == conversation_hash)
            else {
                return Ok(None);
            };
            let path = Self::entry_path(key, &conversation_hash, &current.content_hash)?;
            let versioned = self
                .filesystem
                .get(&self.scope, &path)
                .await
                .map_err(|error| {
                    tracing::debug!(%error, "reply-context entry read failed");
                    store_unavailable()
                })?;
            let Some(versioned) = versioned else {
                continue;
            };
            let entry: ReplyContextEntry =
                serde_json::from_slice(&versioned.entry.body).map_err(|error| {
                    tracing::warn!(%error, "malformed reply-context entry");
                    store_unavailable()
                })?;
            if entry.conversation != key.conversation {
                tracing::warn!("reply-context conversation hash collision");
                return Err(store_unavailable());
            }
            return Ok(Some(entry.context));
        }
        Err(store_unavailable())
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

    fn store_over(backend: Arc<InMemoryBackend>) -> FilesystemReplyContextStore {
        FilesystemReplyContextStore::new(
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
    async fn immediately_preceding_aggregate_snapshot_migrates_on_first_read() {
        let store = store_over(Arc::new(InMemoryBackend::new()));
        let legacy_path = FilesystemReplyContextStore::legacy_snapshot_path(&key("c-legacy"))
            .expect("legacy path");
        let legacy = serde_json::json!({
            "entries": [{
                "conversation": "c-legacy",
                "context": [108, 101, 103, 97, 99, 121],
                "stored_at": "2026-08-11T00:00:00Z"
            }]
        });
        store
            .filesystem
            .put(
                &store.scope,
                &legacy_path,
                Entry::bytes(serde_json::to_vec(&legacy).expect("legacy wire"))
                    .with_content_type(ContentType::json()),
                CasExpectation::Absent,
            )
            .await
            .expect("seed legacy snapshot");

        assert_eq!(
            store.get(&key("c-legacy")).await.expect("migrated read"),
            Some(b"legacy".to_vec())
        );
        assert!(
            store
                .load_catalog(&key("c-legacy"))
                .await
                .expect("catalog read")
                .is_some(),
            "the aggregate snapshot must be normalized into the metadata-only catalog"
        );
    }

    #[tokio::test]
    async fn catalog_is_metadata_only_instead_of_rewriting_context_bytes() {
        let backend = Arc::new(InMemoryBackend::new());
        let store = store_over(backend);
        let large_context = vec![b'x'; 4096];
        store
            .put(key("c-1"), large_context.clone())
            .await
            .expect("put");

        let catalog = ScopedPath::new(format!(
            "{REPLY_CONTEXT_ALIAS}/{}/{}/catalog.json",
            path_segment("vendorx"),
            path_segment("install-1")
        ))
        .expect("catalog path");
        let stored = store
            .filesystem
            .get(&store.scope, &catalog)
            .await
            .expect("catalog read")
            .expect("sharded catalog exists");

        assert!(stored.entry.body.len() < large_context.len());
        assert!(
            !stored
                .entry
                .body
                .windows(64)
                .any(|window| window == &large_context[..64])
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
