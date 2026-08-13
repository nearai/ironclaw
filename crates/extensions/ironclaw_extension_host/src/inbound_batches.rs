//! Durable staging for provider-level inbound message batches.
//!
//! Some providers deliver one logical message as multiple serialized webhook
//! requests. Each verified fragment is therefore staged before its webhook is
//! acknowledged, then a leased background worker admits the merged message
//! after the provider-selected quiet window. The sharded rows are host-private:
//! completed attachment bytes never enter events, projections, transcripts,
//! or model-visible state before admission.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelConversationContext, InboundBatchFragment, NormalizedInboundMessage,
    ProductTriggerReason,
};
use ironclaw_extension_contracts::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
};
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FilesystemError, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    attachment::InboundAttachment,
    error::HostApiError,
    ids::{InvocationId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
    resource::{ResourceScope, resource_scope_path_segment},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const INBOUND_BATCH_ALIAS: &str = "/tenant-shared/inbound-batches";
const INBOUND_BATCH_SNAPSHOT_PATH: &str = "/tenant-shared/inbound-batches/pending.json";
const INBOUND_BATCH_CATALOG_PATH: &str = "/tenant-shared/inbound-batches/catalog.json";
const INBOUND_BATCH_ROWS_PATH: &str = "/tenant-shared/inbound-batches/batches";
const INBOUND_BATCH_FRAGMENTS_PATH: &str = "/tenant-shared/inbound-batches/fragments";
const MAX_BATCHES: usize = 1_024;
const MAX_FRAGMENTS_PER_BATCH: usize = 32;
// Headers contain only fragment identities, aggregate budgets, scheduling,
// and lease state. Complete canonical fragment bytes live in immutable rows.
const MAX_BATCH_HEADER_BYTES: usize = 256 * 1024;
// One fragment may carry the complete 10 MiB batch attachment budget. Base64
// persistence adds ~4/3 overhead.
const MAX_FRAGMENT_ROW_BYTES: usize = 16 * 1024 * 1024;
const MAX_CATALOG_BYTES: usize = 128 * 1024;
const PENDING_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const TERMINAL_TTL: Duration = Duration::from_secs(60 * 60);
pub(crate) const CLAIM_LEASE: Duration = Duration::from_secs(2 * 60);

fn inbound_batch_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(INBOUND_BATCH_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/inbound-batches"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

/// Stable identity for one provider-level batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundBatchKey {
    pub extension_id: String,
    pub installation_id: String,
    pub batch_key: String,
}

/// A fragment plus the exact resolved adapter contract that parsed it.
#[derive(Clone)]
pub struct InboundBatchStageRequest {
    pub key: InboundBatchKey,
    pub binding_fingerprint: String,
    pub fragment: InboundBatchFragment,
    pub staged_at: DateTime<Utc>,
}

/// A durable open batch revision that a worker may claim after `due_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundBatchSchedule {
    pub key: InboundBatchKey,
    pub binding_fingerprint: String,
    pub revision: u64,
    pub due_at: DateTime<Utc>,
}

/// Result of staging an authenticated provider fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundBatchStageOutcome {
    Pending(InboundBatchSchedule),
    AlreadyCompleted,
    Rejected,
}

/// One leased batch. `Debug` is deliberately omitted because fragments carry
/// host-private canonical attachment bytes.
#[derive(Clone)]
pub struct ClaimedInboundBatch {
    pub schedule: InboundBatchSchedule,
    pub claim_id: String,
    pub fragments: Vec<InboundBatchFragment>,
}

/// A bounded persistence failure. Permanent failures reject the current
/// provider payload; retryable failures must produce a non-2xx response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("inbound batch store unavailable: {reason}")]
pub struct InboundBatchStoreError {
    pub retryable: bool,
    pub reason: String,
}

/// Durable provider-batch staging and lease contract.
#[async_trait]
pub trait InboundBatchStore: Send + Sync {
    async fn stage(
        &self,
        request: InboundBatchStageRequest,
    ) -> Result<InboundBatchStageOutcome, InboundBatchStoreError>;

    async fn claim_due(
        &self,
        schedule: &InboundBatchSchedule,
        now: DateTime<Utc>,
    ) -> Result<Option<ClaimedInboundBatch>, InboundBatchStoreError>;

    async fn complete(
        &self,
        claim: &ClaimedInboundBatch,
        now: DateTime<Utc>,
    ) -> Result<bool, InboundBatchStoreError>;

    async fn reject(
        &self,
        claim: &ClaimedInboundBatch,
        now: DateTime<Utc>,
    ) -> Result<bool, InboundBatchStoreError>;

    async fn release(
        &self,
        claim: &ClaimedInboundBatch,
        now: DateTime<Utc>,
        retry_after: Duration,
    ) -> Result<Option<InboundBatchSchedule>, InboundBatchStoreError>;

    async fn pending(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<InboundBatchSchedule>, InboundBatchStoreError>;
}

/// Filesystem-backed [`InboundBatchStore`] using one CAS row per provider
/// batch. The bounded catalog contains only opaque row keys, so one batch's
/// attachment bytes never participate in another batch's transition.
pub struct FilesystemInboundBatchStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
    /// Immediately preceding single-snapshot format. Once migrated, this path
    /// contains a marker that makes rollback to a pre-shard binary fail closed
    /// instead of replaying a stale snapshot.
    path: ScopedPath,
    catalog_path: ScopedPath,
}

impl std::fmt::Debug for FilesystemInboundBatchStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemInboundBatchStore")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl FilesystemInboundBatchStore {
    pub fn new(
        filesystem: Arc<dyn RootFilesystem>,
        tenant_id: TenantId,
        user_id: UserId,
    ) -> Result<Self, HostApiError> {
        let scoped = Arc::new(ScopedFilesystem::new(filesystem, inbound_batch_mount_view));
        let path = ScopedPath::new(INBOUND_BATCH_SNAPSHOT_PATH)?;
        let catalog_path = ScopedPath::new(INBOUND_BATCH_CATALOG_PATH)?;
        Ok(Self {
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
            path,
            catalog_path,
        })
    }

    async fn write_immutable_fragment(
        &self,
        storage_key: &str,
        reference: &StoredInboundBatchFragmentRef,
        entry: Entry,
    ) -> Result<(), InboundBatchStoreError> {
        let path = fragment_path(
            storage_key,
            &reference.fragment_id_hash,
            &reference.content_hash,
        )?;
        let expected_body = entry.body.clone();
        match self
            .filesystem
            .put(&self.scope, &path, entry, CasExpectation::Absent)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                let existing = self
                    .filesystem
                    .get(&self.scope, &path)
                    .await
                    .map_err(|error| {
                        tracing::debug!(%error, "inbound fragment readback failed");
                        store_unavailable()
                    })?;
                if existing.is_some_and(|versioned| versioned.entry.body == expected_body) {
                    Ok(())
                } else {
                    tracing::warn!("inbound fragment content-address collision");
                    Err(store_unavailable())
                }
            }
            Err(error) => {
                tracing::debug!(%error, "inbound fragment write failed");
                Err(store_unavailable())
            }
        }
    }

    async fn header_from_complete_batch(
        &self,
        storage_key: &str,
        batch: CompleteStoredInboundBatch,
    ) -> Result<StoredInboundBatch, InboundBatchStoreError> {
        let mut references = Vec::with_capacity(batch.fragments.len());
        for fragment in batch.fragments {
            let canonical = fragment.clone().into_fragment();
            let (reference, _stored, entry) = prepare_fragment(&canonical)?;
            self.write_immutable_fragment(storage_key, &reference, entry)
                .await?;
            references.push(reference);
        }
        Ok(StoredInboundBatch {
            key: batch.key,
            binding_fingerprint: batch.binding_fingerprint,
            revision: batch.revision,
            settle_millis: batch.settle_millis,
            last_staged_at: batch.last_staged_at,
            due_at: batch.due_at,
            fragments: references,
            state: batch.state,
        })
    }

    /// Upgrade the immediately preceding one-row-per-batch shape lazily. The
    /// row CAS makes concurrent replicas converge on the same metadata header;
    /// content-addressed fragment writes are idempotent.
    async fn ensure_batch_header(&self, storage_key: &str) -> Result<(), InboundBatchStoreError> {
        let path = batch_path(storage_key)?;
        loop {
            let Some(versioned) =
                self.filesystem
                    .get(&self.scope, &path)
                    .await
                    .map_err(|error| {
                        tracing::debug!(%error, "inbound batch row read failed");
                        store_unavailable()
                    })?
            else {
                return Ok(());
            };
            if decode_batch(&versioned.entry.body).is_ok() {
                return Ok(());
            }
            let complete = decode_complete_batch(&versioned.entry.body)?;
            let header = self
                .header_from_complete_batch(storage_key, complete)
                .await?;
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    encode_batch(&header)?,
                    CasExpectation::Version(versioned.version),
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => {
                    tracing::debug!(%error, "inbound batch header migration failed");
                    return Err(store_unavailable());
                }
            }
        }
    }

    async fn load_batch_header(
        &self,
        storage_key: &str,
    ) -> Result<Option<StoredInboundBatch>, InboundBatchStoreError> {
        self.ensure_batch_header(storage_key).await?;
        let path = batch_path(storage_key)?;
        let Some(versioned) = self
            .filesystem
            .get(&self.scope, &path)
            .await
            .map_err(|error| {
                tracing::debug!(%error, "inbound batch header read failed");
                store_unavailable()
            })?
        else {
            return Ok(None);
        };
        decode_batch(&versioned.entry.body).map(Some)
    }

    async fn load_fragment(
        &self,
        storage_key: &str,
        reference: &StoredInboundBatchFragmentRef,
    ) -> Result<InboundBatchFragment, InboundBatchStoreError> {
        let path = fragment_path(
            storage_key,
            &reference.fragment_id_hash,
            &reference.content_hash,
        )?;
        let Some(versioned) = self
            .filesystem
            .get(&self.scope, &path)
            .await
            .map_err(|error| {
                tracing::debug!(%error, "inbound fragment read failed");
                store_unavailable()
            })?
        else {
            return Err(store_unavailable());
        };
        if sha256_hex(&versioned.entry.body) != reference.content_hash {
            tracing::warn!("inbound fragment content hash mismatch");
            return Err(store_unavailable());
        }
        let fragment = decode_fragment(&versioned.entry.body)?.into_fragment();
        if fragment.fragment_id != reference.fragment_id
            || fragment.batch_key != reference.batch_key
        {
            tracing::warn!("inbound fragment identity mismatch");
            return Err(store_unavailable());
        }
        Ok(fragment)
    }

    async fn cleanup_fragments(
        &self,
        storage_key: &str,
        references: &[StoredInboundBatchFragmentRef],
    ) {
        for reference in references {
            let Ok(path) = fragment_path(
                storage_key,
                &reference.fragment_id_hash,
                &reference.content_hash,
            ) else {
                continue;
            };
            if let Err(error) = self.filesystem.delete(&self.scope, &path).await
                && !matches!(error, FilesystemError::NotFound { .. })
            {
                tracing::debug!(%error, "inbound fragment cleanup failed");
            }
        }
    }

    async fn cleanup_fragment_if_unreferenced(
        &self,
        storage_key: &str,
        reference: &StoredInboundBatchFragmentRef,
    ) {
        match self.load_batch_header(storage_key).await {
            Ok(Some(batch)) if batch.fragments.iter().any(|stored| stored == reference) => return,
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(%error, "skipping unreferenced fragment cleanup after header read failure");
                return;
            }
        }
        self.cleanup_fragments(storage_key, std::slice::from_ref(reference))
            .await;
    }

    async fn update_batch<T, F>(
        &self,
        storage_key: &str,
        apply: F,
    ) -> Result<T, InboundBatchStoreError>
    where
        T: Send,
        F: FnMut(
                Option<StoredInboundBatch>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = Result<
                                CasApply<StoredInboundBatch, T>,
                                InboundBatchStoreError,
                            >,
                        > + Send,
                >,
            > + Send,
    {
        self.ensure_batch_header(storage_key).await?;
        let path = batch_path(storage_key)?;
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            decode_batch,
            encode_batch,
            apply,
        )
        .await
        .map_err(|error| match error {
            CasUpdateError::Apply(error) => error,
            error => {
                tracing::debug!(?error, "inbound batch CAS update failed");
                store_unavailable()
            }
        })
    }

    async fn update_catalog<T, F>(&self, apply: F) -> Result<T, InboundBatchStoreError>
    where
        T: Send,
        F: FnMut(
                Option<StoredInboundBatchCatalog>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = Result<
                                CasApply<StoredInboundBatchCatalog, T>,
                                InboundBatchStoreError,
                            >,
                        > + Send,
                >,
            > + Send,
    {
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &self.catalog_path,
            decode_catalog,
            encode_catalog,
            apply,
        )
        .await
        .map_err(map_cas_error)
    }

    async fn ensure_sharded(&self) -> Result<(), InboundBatchStoreError> {
        loop {
            let current = self
                .filesystem
                .get(&self.scope, &self.path)
                .await
                .map_err(|error| {
                    tracing::debug!(%error, "inbound batch compatibility state read failed");
                    store_unavailable()
                })?;
            let (state, expectation) = match current {
                Some(versioned) => (
                    decode_main_format(&versioned.entry.body)?,
                    CasExpectation::Version(versioned.version),
                ),
                None => (StoredMainFormat::Absent, CasExpectation::Absent),
            };
            match state {
                StoredMainFormat::Sharded => return Ok(()),
                StoredMainFormat::Absent => {}
                StoredMainFormat::Complete(snapshot) => {
                    self.migrate_batches(snapshot.batches).await?;
                }
                StoredMainFormat::Legacy(snapshot) => {
                    let mut batches = BTreeMap::new();
                    for (storage_key, legacy) in snapshot.batches {
                        if let Some(batch) = legacy.into_complete() {
                            batches.insert(storage_key, batch);
                        } else {
                            tracing::warn!(
                                storage_key = %storage_key,
                                "discarding an unclaimable pre-fetch inbound batch; provider redelivery can restage it"
                            );
                        }
                    }
                    self.migrate_batches(batches).await?;
                }
            }
            match self
                .filesystem
                .put(
                    &self.scope,
                    &self.path,
                    encode_sharded_marker(),
                    expectation,
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => {
                    tracing::debug!(%error, "inbound batch compatibility marker write failed");
                    return Err(store_unavailable());
                }
            }
        }
    }

    async fn migrate_batches(
        &self,
        batches: BTreeMap<String, CompleteStoredInboundBatch>,
    ) -> Result<(), InboundBatchStoreError> {
        if batches.len() > MAX_BATCHES {
            return Err(store_unavailable());
        }
        if batches.iter().any(|(persisted_key, batch)| {
            storage_key(&batch.key.clone().into_key()) != *persisted_key
        }) {
            return Err(store_unavailable());
        }
        let keys = batches
            .iter()
            .map(|(storage_key, batch)| (storage_key.clone(), batch.last_staged_at))
            .collect::<BTreeMap<_, _>>();
        self.update_catalog(move |current| {
            let keys = keys.clone();
            Box::pin(async move {
                let mut catalog = current.unwrap_or_default();
                catalog.batch_keys.extend(keys);
                if catalog.batch_keys.len() > MAX_BATCHES {
                    return Err(store_unavailable());
                }
                Ok(CasApply::new(catalog, ()))
            })
        })
        .await?;

        for (storage_key, batch) in batches {
            let header = self.header_from_complete_batch(&storage_key, batch).await?;
            let path = batch_path(&storage_key)?;
            match self
                .filesystem
                .put(
                    &self.scope,
                    &path,
                    encode_batch(&header)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => {}
                Err(FilesystemError::VersionMismatch { .. }) => {
                    self.ensure_batch_header(&storage_key).await?;
                }
                Err(error) => {
                    tracing::debug!(%error, "inbound batch row migration failed");
                    return Err(store_unavailable());
                }
            }
        }
        Ok(())
    }

    async fn reserve_storage_key(
        &self,
        storage_key: &str,
        now: DateTime<Utc>,
    ) -> Result<(), InboundBatchStoreError> {
        let result = self.try_reserve_storage_key(storage_key, now).await?;
        if result != CatalogReservation::Full {
            return Ok(());
        }
        self.scan_live_batches(now).await?;
        if self.try_reserve_storage_key(storage_key, now).await? == CatalogReservation::Full {
            return Err(InboundBatchStoreError {
                retryable: true,
                reason: "provider batch staging capacity exhausted".to_string(),
            });
        }
        Ok(())
    }

    async fn try_reserve_storage_key(
        &self,
        storage_key: &str,
        now: DateTime<Utc>,
    ) -> Result<CatalogReservation, InboundBatchStoreError> {
        let storage_key = storage_key.to_string();
        self.update_catalog(move |current| {
            let storage_key = storage_key.clone();
            Box::pin(async move {
                let mut catalog = current.unwrap_or_default();
                if catalog.batch_keys.contains_key(&storage_key) {
                    return Ok(CasApply::no_op(catalog, CatalogReservation::Present));
                }
                if catalog.batch_keys.len() >= MAX_BATCHES {
                    return Ok(CasApply::no_op(catalog, CatalogReservation::Full));
                }
                catalog.batch_keys.insert(storage_key, now);
                Ok(CasApply::new(catalog, CatalogReservation::Reserved))
            })
        })
        .await
    }

    async fn scan_live_batches(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<InboundBatchSchedule>, InboundBatchStoreError> {
        let catalog = self.load_catalog().await?;
        let mut schedules = Vec::new();
        let mut removable = BTreeSet::new();
        for (storage_key, reserved_at) in catalog.batch_keys {
            self.ensure_batch_header(&storage_key).await?;
            let path = batch_path(&storage_key)?;
            let Some(versioned) =
                self.filesystem
                    .get(&self.scope, &path)
                    .await
                    .map_err(|error| {
                        tracing::debug!(%error, "inbound batch row read failed");
                        store_unavailable()
                    })?
            else {
                // Reservation precedes the first row CAS. Keep a fresh absent
                // entry so a concurrent stage cannot be made undiscoverable;
                // abandoned reservations age out with the same pending TTL.
                if !age_within(now, reserved_at, PENDING_TTL) {
                    removable.insert(storage_key);
                }
                continue;
            };
            let batch = decode_batch(&versioned.entry.body)?;
            if batch_is_expired(&batch, now) {
                match self
                    .filesystem
                    .delete_if_version(&self.scope, &path, versioned.version)
                    .await
                {
                    Ok(()) | Err(FilesystemError::NotFound { .. }) => {
                        self.cleanup_fragments(&storage_key, &batch.fragments).await;
                        removable.insert(storage_key);
                    }
                    Err(FilesystemError::VersionMismatch { .. }) => {}
                    Err(error) => {
                        tracing::debug!(%error, "expired inbound batch row deletion failed");
                        return Err(store_unavailable());
                    }
                }
                continue;
            }
            if match &batch.state {
                StoredInboundBatchState::Open => true,
                StoredInboundBatchState::Claimed { lease_until, .. } => *lease_until <= now,
                StoredInboundBatchState::Completed { .. }
                | StoredInboundBatchState::Rejected { .. } => false,
            } {
                schedules.push(batch.schedule());
            }
        }
        if !removable.is_empty() {
            self.remove_catalog_keys(removable).await?;
        }
        Ok(schedules)
    }

    async fn load_catalog(&self) -> Result<StoredInboundBatchCatalog, InboundBatchStoreError> {
        let Some(versioned) = self
            .filesystem
            .get(&self.scope, &self.catalog_path)
            .await
            .map_err(|error| {
                tracing::debug!(%error, "inbound batch catalog read failed");
                store_unavailable()
            })?
        else {
            return Ok(StoredInboundBatchCatalog::default());
        };
        decode_catalog(&versioned.entry.body)
    }

    async fn remove_catalog_keys(
        &self,
        removable: BTreeSet<String>,
    ) -> Result<(), InboundBatchStoreError> {
        self.update_catalog(move |current| {
            let removable = removable.clone();
            Box::pin(async move {
                let Some(mut catalog) = current else {
                    return Ok(CasApply::no_op(StoredInboundBatchCatalog::default(), ()));
                };
                catalog.batch_keys.retain(|key, _| !removable.contains(key));
                Ok(CasApply::new(catalog, ()))
            })
        })
        .await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogReservation {
    Reserved,
    Present,
    Full,
}

#[async_trait]
impl InboundBatchStore for FilesystemInboundBatchStore {
    async fn stage(
        &self,
        request: InboundBatchStageRequest,
    ) -> Result<InboundBatchStageOutcome, InboundBatchStoreError> {
        if !attachment_sizes_fit_budget(
            request
                .fragment
                .message
                .attachments
                .iter()
                .map(|attachment| attachment.bytes.len()),
        ) {
            return Ok(InboundBatchStageOutcome::Rejected);
        }
        self.ensure_sharded().await?;
        let storage_key = storage_key(&request.key);
        self.reserve_storage_key(&storage_key, request.staged_at)
            .await?;
        let (incoming_ref, _stored_fragment, fragment_entry) = prepare_fragment(&request.fragment)?;

        // Avoid creating unreferenced payload rows for settled batches or a
        // batch currently owned by an admission worker.
        if let Some(batch) = self.load_batch_header(&storage_key).await?
            && !batch_is_expired(&batch, request.staged_at)
        {
            match &batch.state {
                StoredInboundBatchState::Completed { .. } => {
                    return Ok(InboundBatchStageOutcome::AlreadyCompleted);
                }
                StoredInboundBatchState::Rejected { .. } => {
                    return Ok(InboundBatchStageOutcome::Rejected);
                }
                StoredInboundBatchState::Claimed { .. } => {
                    if batch.fragments.iter().any(|stored| stored == &incoming_ref) {
                        return Ok(InboundBatchStageOutcome::Pending(batch.schedule()));
                    }
                    return Err(InboundBatchStoreError {
                        retryable: true,
                        reason: "provider batch is already being admitted".to_string(),
                    });
                }
                StoredInboundBatchState::Open => {}
            }
        }
        self.write_immutable_fragment(&storage_key, &incoming_ref, fragment_entry)
            .await?;

        let incoming_ref_for_cleanup = incoming_ref.clone();
        let (outcome, mut stale) = self
            .update_batch(&storage_key, move |current| {
                let request = request.clone();
                let incoming_ref = incoming_ref.clone();
                Box::pin(async move {
                    let mut stale = current
                        .as_ref()
                        .filter(|batch| batch_is_expired(batch, request.staged_at))
                        .map(|batch| batch.fragments.clone())
                        .unwrap_or_default();
                    let mut current =
                        current.filter(|batch| !batch_is_expired(batch, request.staged_at));
                    let outcome = if let Some(batch) = current.as_mut() {
                        if batch.key != StoredInboundBatchKey::from(&request.key)
                            || batch.binding_fingerprint != request.binding_fingerprint
                        {
                            stale.extend(batch.fragments.clone());
                            stale.push(incoming_ref.clone());
                            batch.state = StoredInboundBatchState::Rejected {
                                terminal_at: request.staged_at,
                            };
                            batch.fragments.clear();
                            InboundBatchStageOutcome::Rejected
                        } else if matches!(batch.state, StoredInboundBatchState::Completed { .. }) {
                            InboundBatchStageOutcome::AlreadyCompleted
                        } else if matches!(batch.state, StoredInboundBatchState::Rejected { .. }) {
                            InboundBatchStageOutcome::Rejected
                        } else if batch.settle_millis != request.fragment.settle_millis
                            || !fragments_are_compatible(batch, &request.fragment)
                        {
                            stale.extend(batch.fragments.clone());
                            stale.push(incoming_ref.clone());
                            batch.state = StoredInboundBatchState::Rejected {
                                terminal_at: request.staged_at,
                            };
                            batch.fragments.clear();
                            InboundBatchStageOutcome::Rejected
                        } else {
                            match &batch.state {
                                StoredInboundBatchState::Claimed { .. } => {
                                    if batch.fragments.iter().any(|stored| stored == &incoming_ref)
                                    {
                                        InboundBatchStageOutcome::Pending(batch.schedule())
                                    } else {
                                        return Err(InboundBatchStoreError {
                                            retryable: true,
                                            reason: "provider batch is already being admitted"
                                                .to_string(),
                                        });
                                    }
                                }
                                StoredInboundBatchState::Open => {
                                    if let Some(existing) = batch.fragments.iter().find(|stored| {
                                        stored.fragment_id == request.fragment.fragment_id
                                    }) {
                                        if existing != &incoming_ref {
                                            stale.extend(batch.fragments.clone());
                                            stale.push(incoming_ref.clone());
                                            batch.state = StoredInboundBatchState::Rejected {
                                                terminal_at: request.staged_at,
                                            };
                                            batch.fragments.clear();
                                            InboundBatchStageOutcome::Rejected
                                        } else {
                                            InboundBatchStageOutcome::Pending(batch.schedule())
                                        }
                                    } else {
                                        if batch.fragments.len() >= MAX_FRAGMENTS_PER_BATCH {
                                            stale.extend(batch.fragments.clone());
                                            stale.push(incoming_ref.clone());
                                            batch.state = StoredInboundBatchState::Rejected {
                                                terminal_at: request.staged_at,
                                            };
                                            batch.fragments.clear();
                                            return Ok(CasApply::new(
                                                batch.clone(),
                                                (InboundBatchStageOutcome::Rejected, stale),
                                            ));
                                        }
                                        if !batch_with_fragment_fits_attachment_budget(
                                            batch,
                                            &request.fragment,
                                        ) {
                                            stale.extend(batch.fragments.clone());
                                            stale.push(incoming_ref.clone());
                                            batch.state = StoredInboundBatchState::Rejected {
                                                terminal_at: request.staged_at,
                                            };
                                            batch.fragments.clear();
                                            return Ok(CasApply::new(
                                                batch.clone(),
                                                (InboundBatchStageOutcome::Rejected, stale),
                                            ));
                                        }
                                        batch.fragments.push(incoming_ref.clone());
                                        batch.revision =
                                            batch.revision.checked_add(1).ok_or_else(|| {
                                                InboundBatchStoreError {
                                                    retryable: false,
                                                    reason: "provider batch revision overflow"
                                                        .to_string(),
                                                }
                                            })?;
                                        batch.last_staged_at = request.staged_at;
                                        batch.due_at = add_duration(
                                            request.staged_at,
                                            Duration::from_millis(request.fragment.settle_millis),
                                        )?;
                                        InboundBatchStageOutcome::Pending(batch.schedule())
                                    }
                                }
                                StoredInboundBatchState::Completed { .. }
                                | StoredInboundBatchState::Rejected { .. } => {
                                    InboundBatchStageOutcome::Rejected
                                }
                            }
                        }
                    } else {
                        let due_at = add_duration(
                            request.staged_at,
                            Duration::from_millis(request.fragment.settle_millis),
                        )?;
                        let batch = StoredInboundBatch {
                            key: StoredInboundBatchKey::from(&request.key),
                            binding_fingerprint: request.binding_fingerprint,
                            revision: 1,
                            settle_millis: request.fragment.settle_millis,
                            last_staged_at: request.staged_at,
                            due_at,
                            fragments: vec![incoming_ref.clone()],
                            state: StoredInboundBatchState::Open,
                        };
                        let schedule = batch.schedule();
                        current = Some(batch);
                        InboundBatchStageOutcome::Pending(schedule)
                    };
                    let stored = current.ok_or_else(store_unavailable)?;
                    Ok(CasApply::new(stored, (outcome, stale)))
                })
            })
            .await?;
        stale.retain(|reference| reference != &incoming_ref_for_cleanup);
        self.cleanup_fragments(&storage_key, &stale).await;
        self.cleanup_fragment_if_unreferenced(&storage_key, &incoming_ref_for_cleanup)
            .await;
        Ok(outcome)
    }

    async fn claim_due(
        &self,
        schedule: &InboundBatchSchedule,
        now: DateTime<Utc>,
    ) -> Result<Option<ClaimedInboundBatch>, InboundBatchStoreError> {
        self.ensure_sharded().await?;
        let storage_key = storage_key(&schedule.key);
        let schedule = schedule.clone();
        let claim_id = InvocationId::new().to_string();
        let claimed = self
            .update_batch(&storage_key, move |current| {
                let schedule = schedule.clone();
                let claim_id = claim_id.clone();
                Box::pin(async move {
                    let Some(mut batch) = current else {
                        return Ok(CasApply::no_op(missing_batch(&schedule), None));
                    };
                    if batch.key != StoredInboundBatchKey::from(&schedule.key)
                        || batch.binding_fingerprint != schedule.binding_fingerprint
                        || batch.revision != schedule.revision
                        || batch.due_at > now
                    {
                        return Ok(CasApply::no_op(batch, None));
                    }
                    let claimable = match &batch.state {
                        StoredInboundBatchState::Open => true,
                        StoredInboundBatchState::Claimed { lease_until, .. } => *lease_until <= now,
                        StoredInboundBatchState::Completed { .. }
                        | StoredInboundBatchState::Rejected { .. } => false,
                    };
                    if !claimable {
                        return Ok(CasApply::no_op(batch, None));
                    }
                    let lease_until = add_duration(now, CLAIM_LEASE)?;
                    batch.state = StoredInboundBatchState::Claimed {
                        claim_id: claim_id.clone(),
                        lease_until,
                    };
                    let claimed = Some((batch.schedule(), claim_id, batch.fragments.clone()));
                    Ok(CasApply::new(batch, claimed))
                })
            })
            .await?;
        let Some((schedule, claim_id, references)) = claimed else {
            return Ok(None);
        };
        let mut fragments = Vec::with_capacity(references.len());
        for reference in &references {
            fragments.push(self.load_fragment(&storage_key, reference).await?);
        }
        Ok(Some(ClaimedInboundBatch {
            schedule,
            claim_id,
            fragments,
        }))
    }

    async fn complete(
        &self,
        claim: &ClaimedInboundBatch,
        now: DateTime<Utc>,
    ) -> Result<bool, InboundBatchStoreError> {
        self.finish(claim, StoredTerminal::Completed, now).await
    }

    async fn reject(
        &self,
        claim: &ClaimedInboundBatch,
        now: DateTime<Utc>,
    ) -> Result<bool, InboundBatchStoreError> {
        self.finish(claim, StoredTerminal::Rejected, now).await
    }

    async fn release(
        &self,
        claim: &ClaimedInboundBatch,
        now: DateTime<Utc>,
        retry_after: Duration,
    ) -> Result<Option<InboundBatchSchedule>, InboundBatchStoreError> {
        self.ensure_sharded().await?;
        let storage_key = storage_key(&claim.schedule.key);
        let claim = claim.clone();
        self.update_batch(&storage_key, move |current| {
            let claim = claim.clone();
            Box::pin(async move {
                let Some(mut batch) = current else {
                    return Ok(CasApply::no_op(missing_batch(&claim.schedule), None));
                };
                if !claim_matches(&batch, &claim) {
                    return Ok(CasApply::no_op(batch, None));
                }
                batch.revision =
                    batch
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| InboundBatchStoreError {
                            retryable: false,
                            reason: "provider batch revision overflow".to_string(),
                        })?;
                batch.last_staged_at = now;
                batch.due_at = add_duration(now, retry_after)?;
                batch.state = StoredInboundBatchState::Open;
                let schedule = batch.schedule();
                Ok(CasApply::new(batch, Some(schedule)))
            })
        })
        .await
    }

    async fn pending(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<InboundBatchSchedule>, InboundBatchStoreError> {
        self.ensure_sharded().await?;
        self.scan_live_batches(now).await
    }
}

impl FilesystemInboundBatchStore {
    async fn finish(
        &self,
        claim: &ClaimedInboundBatch,
        terminal: StoredTerminal,
        now: DateTime<Utc>,
    ) -> Result<bool, InboundBatchStoreError> {
        self.ensure_sharded().await?;
        let storage_key = storage_key(&claim.schedule.key);
        let claim = claim.clone();
        let (finished, fragments) = self
            .update_batch(&storage_key, move |current| {
                let claim = claim.clone();
                Box::pin(async move {
                    let Some(mut batch) = current else {
                        return Ok(CasApply::no_op(
                            missing_batch(&claim.schedule),
                            (false, Vec::new()),
                        ));
                    };
                    if !claim_matches(&batch, &claim) {
                        return Ok(CasApply::no_op(batch, (false, Vec::new())));
                    }
                    let fragments = std::mem::take(&mut batch.fragments);
                    batch.state = match terminal {
                        StoredTerminal::Completed => {
                            StoredInboundBatchState::Completed { terminal_at: now }
                        }
                        StoredTerminal::Rejected => {
                            StoredInboundBatchState::Rejected { terminal_at: now }
                        }
                    };
                    Ok(CasApply::new(batch, (true, fragments)))
                })
            })
            .await?;
        if finished {
            self.cleanup_fragments(&storage_key, &fragments).await;
        }
        Ok(finished)
    }
}

#[derive(Clone, Copy)]
enum StoredTerminal {
    Completed,
    Rejected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StoredInboundBatchCatalog {
    batch_keys: BTreeMap<String, DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct CompleteStoredInboundBatchSnapshot {
    batches: BTreeMap<String, CompleteStoredInboundBatch>,
}

enum StoredMainFormat {
    Absent,
    Sharded,
    Complete(CompleteStoredInboundBatchSnapshot),
    Legacy(LegacyStoredInboundBatchSnapshot),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct LegacyStoredInboundBatchSnapshot {
    batches: BTreeMap<String, LegacyStoredInboundBatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LegacyStoredInboundBatch {
    key: StoredInboundBatchKey,
    binding_fingerprint: String,
    revision: u64,
    settle_millis: u64,
    last_staged_at: DateTime<Utc>,
    due_at: DateTime<Utc>,
    fragments: Vec<LegacyStoredInboundBatchFragment>,
    state: StoredInboundBatchState,
}

impl LegacyStoredInboundBatch {
    fn into_complete(self) -> Option<CompleteStoredInboundBatch> {
        let terminal = matches!(
            &self.state,
            StoredInboundBatchState::Completed { .. } | StoredInboundBatchState::Rejected { .. }
        );
        if !terminal
            && self
                .fragments
                .iter()
                .any(|fragment| !fragment.message.attachments.is_empty())
        {
            // The complete-message contract cannot reconstruct bytes from an
            // old opaque provider handle. Dropping this open row is fail-safe:
            // it is never admitted partially, and a provider redelivery may
            // stage a new complete row under the same key.
            return None;
        }
        let fragments = if terminal {
            Vec::new()
        } else {
            self.fragments
                .into_iter()
                .map(LegacyStoredInboundBatchFragment::into_complete)
                .collect()
        };
        Some(CompleteStoredInboundBatch {
            key: self.key,
            binding_fingerprint: self.binding_fingerprint,
            revision: self.revision,
            settle_millis: self.settle_millis,
            last_staged_at: self.last_staged_at,
            due_at: self.due_at,
            fragments,
            state: self.state,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LegacyStoredInboundBatchFragment {
    batch_key: String,
    fragment_id: String,
    order: u64,
    settle_millis: u64,
    triggered: bool,
    message: LegacyStoredNormalizedInboundMessage,
}

impl LegacyStoredInboundBatchFragment {
    fn into_complete(self) -> StoredInboundBatchFragment {
        StoredInboundBatchFragment {
            batch_key: self.batch_key,
            fragment_id: self.fragment_id,
            order: self.order,
            settle_millis: self.settle_millis,
            triggered: self.triggered,
            message: self.message.into_current(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LegacyStoredNormalizedInboundMessage {
    actor: ExternalActorRef,
    conversation: ExternalConversationRef,
    event_id: ExternalEventId,
    text: String,
    trigger: ProductTriggerReason,
    attachments: Vec<LegacyStoredChannelAttachmentRef>,
    reply_context: Option<Vec<u8>>,
}

impl LegacyStoredNormalizedInboundMessage {
    fn into_current(self) -> StoredNormalizedInboundMessage {
        StoredNormalizedInboundMessage {
            actor: self.actor,
            conversation: self.conversation,
            event_id: self.event_id,
            text: self.text,
            trigger: self.trigger,
            attachments: Vec::new(),
            conversation_context: None,
            reply_context: self.reply_context,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LegacyStoredChannelAttachmentRef {
    #[serde(rename = "descriptor")]
    _descriptor: ProductAttachmentDescriptor,
    #[serde(rename = "vendor_ref")]
    _vendor_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CompleteStoredInboundBatch {
    key: StoredInboundBatchKey,
    binding_fingerprint: String,
    revision: u64,
    settle_millis: u64,
    last_staged_at: DateTime<Utc>,
    due_at: DateTime<Utc>,
    fragments: Vec<StoredInboundBatchFragment>,
    state: StoredInboundBatchState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredInboundBatch {
    key: StoredInboundBatchKey,
    binding_fingerprint: String,
    revision: u64,
    settle_millis: u64,
    last_staged_at: DateTime<Utc>,
    due_at: DateTime<Utc>,
    fragments: Vec<StoredInboundBatchFragmentRef>,
    state: StoredInboundBatchState,
}

impl StoredInboundBatch {
    fn schedule(&self) -> InboundBatchSchedule {
        InboundBatchSchedule {
            key: self.key.clone().into_key(),
            binding_fingerprint: self.binding_fingerprint.clone(),
            revision: self.revision,
            due_at: self.due_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredInboundBatchKey {
    extension_id: String,
    installation_id: String,
    batch_key: String,
}

impl From<&InboundBatchKey> for StoredInboundBatchKey {
    fn from(key: &InboundBatchKey) -> Self {
        Self {
            extension_id: key.extension_id.clone(),
            installation_id: key.installation_id.clone(),
            batch_key: key.batch_key.clone(),
        }
    }
}

impl StoredInboundBatchKey {
    fn into_key(self) -> InboundBatchKey {
        InboundBatchKey {
            extension_id: self.extension_id,
            installation_id: self.installation_id,
            batch_key: self.batch_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum StoredInboundBatchState {
    Open,
    Claimed {
        claim_id: String,
        lease_until: DateTime<Utc>,
    },
    Completed {
        terminal_at: DateTime<Utc>,
    },
    Rejected {
        terminal_at: DateTime<Utc>,
    },
}

/// Small header projection for an immutable canonical fragment row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredInboundBatchFragmentRef {
    batch_key: String,
    fragment_id: String,
    fragment_id_hash: String,
    content_hash: String,
    triggered: bool,
    event_id: ExternalEventId,
    actor: ExternalActorRef,
    conversation: ExternalConversationRef,
    trigger: ProductTriggerReason,
    attachment_count: usize,
    attachment_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredInboundBatchFragment {
    batch_key: String,
    fragment_id: String,
    order: u64,
    settle_millis: u64,
    triggered: bool,
    message: StoredNormalizedInboundMessage,
}

impl From<&InboundBatchFragment> for StoredInboundBatchFragment {
    fn from(fragment: &InboundBatchFragment) -> Self {
        Self {
            batch_key: fragment.batch_key.clone(),
            fragment_id: fragment.fragment_id.clone(),
            order: fragment.order,
            settle_millis: fragment.settle_millis,
            triggered: fragment.triggered,
            message: StoredNormalizedInboundMessage::from(&fragment.message),
        }
    }
}

impl StoredInboundBatchFragment {
    fn into_fragment(self) -> InboundBatchFragment {
        InboundBatchFragment {
            batch_key: self.batch_key,
            fragment_id: self.fragment_id,
            order: self.order,
            settle_millis: self.settle_millis,
            triggered: self.triggered,
            message: self.message.into_message(),
        }
    }
}

fn prepare_fragment(
    fragment: &InboundBatchFragment,
) -> Result<
    (
        StoredInboundBatchFragmentRef,
        StoredInboundBatchFragment,
        Entry,
    ),
    InboundBatchStoreError,
> {
    let stored = StoredInboundBatchFragment::from(fragment);
    let entry = encode_fragment(&stored)?;
    let reference = StoredInboundBatchFragmentRef {
        batch_key: fragment.batch_key.clone(),
        fragment_id: fragment.fragment_id.clone(),
        fragment_id_hash: sha256_hex(fragment.fragment_id.as_bytes()),
        content_hash: sha256_hex(&entry.body),
        triggered: fragment.triggered,
        event_id: fragment.message.event_id.clone(),
        actor: fragment.message.actor.clone(),
        conversation: fragment.message.conversation.clone(),
        trigger: fragment.message.trigger,
        attachment_count: fragment.message.attachments.len(),
        attachment_bytes: fragment
            .message
            .attachments
            .iter()
            .map(|attachment| attachment.bytes.len())
            .sum(),
    };
    Ok((reference, stored, entry))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredNormalizedInboundMessage {
    actor: ExternalActorRef,
    conversation: ExternalConversationRef,
    event_id: ExternalEventId,
    text: String,
    trigger: ProductTriggerReason,
    attachments: Vec<StoredInboundAttachment>,
    #[serde(default)]
    conversation_context: Option<String>,
    reply_context: Option<Vec<u8>>,
}

impl From<&NormalizedInboundMessage> for StoredNormalizedInboundMessage {
    fn from(message: &NormalizedInboundMessage) -> Self {
        Self {
            actor: message.actor.clone(),
            conversation: message.conversation.clone(),
            event_id: message.event_id.clone(),
            text: message.text.clone(),
            trigger: message.trigger,
            attachments: message
                .attachments
                .iter()
                .map(StoredInboundAttachment::from)
                .collect(),
            conversation_context: message
                .conversation_context
                .as_ref()
                .map(|context| context.text.clone()),
            reply_context: message.reply_context.clone(),
        }
    }
}

impl StoredNormalizedInboundMessage {
    fn into_message(self) -> NormalizedInboundMessage {
        NormalizedInboundMessage {
            actor: self.actor,
            conversation: self.conversation,
            event_id: self.event_id,
            text: self.text,
            trigger: self.trigger,
            attachments: self
                .attachments
                .into_iter()
                .map(StoredInboundAttachment::into_attachment)
                .collect(),
            conversation_context: self
                .conversation_context
                .map(|text| ChannelConversationContext { text }),
            reply_context: self.reply_context,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
struct StoredInboundAttachment {
    id: String,
    mime_type: String,
    filename: Option<String>,
    #[serde(with = "base64_bytes")]
    bytes: Vec<u8>,
}

impl<'de> Deserialize<'de> for StoredInboundAttachment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Canonical {
                id: String,
                mime_type: String,
                filename: Option<String>,
                #[serde(with = "base64_bytes")]
                bytes: Vec<u8>,
            },
            PreviousComplete {
                #[serde(rename = "descriptor")]
                _descriptor: ProductAttachmentDescriptor,
                fetched_id: String,
                fetched_mime_type: String,
                fetched_filename: Option<String>,
                #[serde(with = "base64_bytes")]
                fetched_bytes: Vec<u8>,
            },
        }

        match Wire::deserialize(deserializer)? {
            Wire::Canonical {
                id,
                mime_type,
                filename,
                bytes,
            } => Ok(Self {
                id,
                mime_type,
                filename,
                bytes,
            }),
            Wire::PreviousComplete {
                _descriptor: _,
                fetched_id,
                fetched_mime_type,
                fetched_filename,
                fetched_bytes,
            } => Ok(Self {
                id: fetched_id,
                mime_type: fetched_mime_type,
                filename: fetched_filename,
                bytes: fetched_bytes,
            }),
        }
    }
}

impl std::fmt::Debug for StoredInboundAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredInboundAttachment")
            .field("id", &self.id)
            .field("mime_type", &self.mime_type)
            .field("filename", &self.filename)
            .field("size_bytes", &self.bytes.len())
            .finish()
    }
}

impl From<&InboundAttachment> for StoredInboundAttachment {
    fn from(attachment: &InboundAttachment) -> Self {
        Self {
            id: attachment.id.clone(),
            mime_type: attachment.mime_type.clone(),
            filename: attachment.filename.clone(),
            bytes: attachment.bytes.clone(),
        }
    }
}

impl StoredInboundAttachment {
    fn into_attachment(self) -> InboundAttachment {
        InboundAttachment {
            id: self.id,
            mime_type: self.mime_type,
            filename: self.filename,
            bytes: self.bytes,
        }
    }
}

mod base64_bytes {
    use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD_NO_PAD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        STANDARD_NO_PAD.decode(encoded).map_err(D::Error::custom)
    }
}

fn storage_key(key: &InboundBatchKey) -> String {
    fn segment(hasher: &mut Sha256, value: &str) {
        hasher.update(value.len().to_be_bytes());
        hasher.update(value.as_bytes());
    }
    let mut hasher = Sha256::new();
    segment(&mut hasher, &key.extension_id);
    segment(&mut hasher, &key.installation_id);
    segment(&mut hasher, &key.batch_key);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn batch_path(storage_key: &str) -> Result<ScopedPath, InboundBatchStoreError> {
    if !is_sha256_hex(storage_key) {
        return Err(store_unavailable());
    }
    ScopedPath::new(format!("{INBOUND_BATCH_ROWS_PATH}/{storage_key}.json"))
        .map_err(|_| store_unavailable())
}

fn fragment_path(
    storage_key: &str,
    fragment_id_hash: &str,
    content_hash: &str,
) -> Result<ScopedPath, InboundBatchStoreError> {
    if !is_sha256_hex(storage_key)
        || !is_sha256_hex(fragment_id_hash)
        || !is_sha256_hex(content_hash)
    {
        return Err(store_unavailable());
    }
    ScopedPath::new(format!(
        "{INBOUND_BATCH_FRAGMENTS_PATH}/{storage_key}/{fragment_id_hash}/{content_hash}.json"
    ))
    .map_err(|_| store_unavailable())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn fragments_are_compatible(batch: &StoredInboundBatch, incoming: &InboundBatchFragment) -> bool {
    let Some(first) = batch.fragments.first() else {
        return false;
    };
    first.batch_key == incoming.batch_key
        && first.event_id == incoming.message.event_id
        && first.actor == incoming.message.actor
        && first.conversation == incoming.message.conversation
        && batch.fragments.iter().all(|fragment| {
            !fragment.triggered
                || !incoming.triggered
                || fragment.trigger == incoming.message.trigger
        })
}

fn batch_with_fragment_fits_attachment_budget(
    batch: &StoredInboundBatch,
    incoming: &InboundBatchFragment,
) -> bool {
    let existing_count = batch
        .fragments
        .iter()
        .map(|fragment| fragment.attachment_count)
        .sum::<usize>();
    let existing_bytes = batch
        .fragments
        .iter()
        .map(|fragment| fragment.attachment_bytes)
        .sum::<usize>();
    let incoming_count = incoming.message.attachments.len();
    let incoming_bytes = incoming
        .message
        .attachments
        .iter()
        .map(|attachment| attachment.bytes.len())
        .sum::<usize>();
    attachment_sizes_fit_budget(
        incoming
            .message
            .attachments
            .iter()
            .map(|attachment| attachment.bytes.len()),
    ) && existing_count.saturating_add(incoming_count) <= DEFAULT_ATTACHMENT_BUDGETS.max_count
        && existing_bytes.saturating_add(incoming_bytes)
            <= DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes
}

fn attachment_sizes_fit_budget(sizes: impl IntoIterator<Item = usize>) -> bool {
    let mut count = 0usize;
    let mut total_bytes = 0usize;
    for size in sizes {
        count = count.saturating_add(1);
        total_bytes = total_bytes.saturating_add(size);
        if count > DEFAULT_ATTACHMENT_BUDGETS.max_count
            || size > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes
            || total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes
        {
            return false;
        }
    }
    true
}

fn claim_matches(batch: &StoredInboundBatch, claim: &ClaimedInboundBatch) -> bool {
    batch.key == StoredInboundBatchKey::from(&claim.schedule.key)
        && batch.binding_fingerprint == claim.schedule.binding_fingerprint
        && batch.revision == claim.schedule.revision
        && matches!(
            &batch.state,
            StoredInboundBatchState::Claimed { claim_id, .. } if claim_id == &claim.claim_id
        )
}

fn missing_batch(schedule: &InboundBatchSchedule) -> StoredInboundBatch {
    StoredInboundBatch {
        key: StoredInboundBatchKey::from(&schedule.key),
        binding_fingerprint: schedule.binding_fingerprint.clone(),
        revision: schedule.revision,
        settle_millis: 0,
        last_staged_at: schedule.due_at,
        due_at: schedule.due_at,
        fragments: Vec::new(),
        state: StoredInboundBatchState::Rejected {
            terminal_at: schedule.due_at,
        },
    }
}

fn batch_is_expired(batch: &StoredInboundBatch, now: DateTime<Utc>) -> bool {
    !match &batch.state {
        StoredInboundBatchState::Completed { terminal_at }
        | StoredInboundBatchState::Rejected { terminal_at } => {
            age_within(now, *terminal_at, TERMINAL_TTL)
        }
        StoredInboundBatchState::Open | StoredInboundBatchState::Claimed { .. } => {
            age_within(now, batch.last_staged_at, PENDING_TTL)
        }
    }
}

fn age_within(now: DateTime<Utc>, since: DateTime<Utc>, limit: Duration) -> bool {
    let Ok(limit) = TimeDelta::from_std(limit) else {
        return true;
    };
    now.signed_duration_since(since) <= limit
}

fn add_duration(
    at: DateTime<Utc>,
    duration: Duration,
) -> Result<DateTime<Utc>, InboundBatchStoreError> {
    let delta = TimeDelta::from_std(duration).map_err(|_| InboundBatchStoreError {
        retryable: false,
        reason: "provider batch duration is out of range".to_string(),
    })?;
    at.checked_add_signed(delta)
        .ok_or_else(|| InboundBatchStoreError {
            retryable: false,
            reason: "provider batch timestamp overflow".to_string(),
        })
}

fn decode_main_format(bytes: &[u8]) -> Result<StoredMainFormat, InboundBatchStoreError> {
    #[derive(Deserialize)]
    struct Marker {
        format: String,
    }

    if let Ok(marker) = serde_json::from_slice::<Marker>(bytes)
        && marker.format == "sharded_v2"
    {
        return Ok(StoredMainFormat::Sharded);
    }
    if bytes.len() > MAX_FRAGMENT_ROW_BYTES {
        return Err(store_unavailable());
    }
    if let Ok(snapshot) = serde_json::from_slice::<CompleteStoredInboundBatchSnapshot>(bytes) {
        return Ok(StoredMainFormat::Complete(snapshot));
    }
    serde_json::from_slice::<LegacyStoredInboundBatchSnapshot>(bytes)
        .map(StoredMainFormat::Legacy)
        .map_err(|error| {
            tracing::debug!(%error, "malformed inbound batch compatibility snapshot");
            store_unavailable()
        })
}

fn encode_sharded_marker() -> Entry {
    Entry::bytes(br#"{"format":"sharded_v2"}"#.to_vec()).with_content_type(ContentType::json())
}

fn decode_batch(bytes: &[u8]) -> Result<StoredInboundBatch, InboundBatchStoreError> {
    if bytes.len() > MAX_BATCH_HEADER_BYTES {
        return Err(store_unavailable());
    }
    serde_json::from_slice(bytes).map_err(|error| {
        tracing::debug!(%error, "malformed inbound batch row");
        store_unavailable()
    })
}

fn encode_batch(batch: &StoredInboundBatch) -> Result<Entry, InboundBatchStoreError> {
    let body = serde_json::to_vec(batch).map_err(|error| {
        tracing::debug!(%error, "inbound batch row serialization failed");
        store_unavailable()
    })?;
    if body.len() > MAX_BATCH_HEADER_BYTES {
        return Err(InboundBatchStoreError {
            retryable: true,
            reason: "provider batch header exceeds its storage bound".to_string(),
        });
    }
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn decode_complete_batch(
    bytes: &[u8],
) -> Result<CompleteStoredInboundBatch, InboundBatchStoreError> {
    if bytes.len() > MAX_FRAGMENT_ROW_BYTES {
        return Err(store_unavailable());
    }
    serde_json::from_slice(bytes).map_err(|error| {
        tracing::debug!(%error, "malformed complete inbound batch row");
        store_unavailable()
    })
}

fn encode_fragment(fragment: &StoredInboundBatchFragment) -> Result<Entry, InboundBatchStoreError> {
    let body = serde_json::to_vec(fragment).map_err(|error| {
        tracing::debug!(%error, "inbound batch fragment serialization failed");
        store_unavailable()
    })?;
    if body.len() > MAX_FRAGMENT_ROW_BYTES {
        return Err(InboundBatchStoreError {
            retryable: true,
            reason: "provider batch fragment exceeds its storage bound".to_string(),
        });
    }
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn decode_fragment(bytes: &[u8]) -> Result<StoredInboundBatchFragment, InboundBatchStoreError> {
    if bytes.len() > MAX_FRAGMENT_ROW_BYTES {
        return Err(store_unavailable());
    }
    serde_json::from_slice(bytes).map_err(|error| {
        tracing::debug!(%error, "malformed inbound batch fragment row");
        store_unavailable()
    })
}

fn decode_catalog(bytes: &[u8]) -> Result<StoredInboundBatchCatalog, InboundBatchStoreError> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(store_unavailable());
    }
    let catalog = serde_json::from_slice::<StoredInboundBatchCatalog>(bytes).map_err(|error| {
        tracing::debug!(%error, "malformed inbound batch catalog");
        store_unavailable()
    })?;
    if catalog.batch_keys.len() > MAX_BATCHES
        || catalog
            .batch_keys
            .keys()
            .any(|storage_key| batch_path(storage_key).is_err())
    {
        return Err(store_unavailable());
    }
    Ok(catalog)
}

fn encode_catalog(catalog: &StoredInboundBatchCatalog) -> Result<Entry, InboundBatchStoreError> {
    if catalog.batch_keys.len() > MAX_BATCHES {
        return Err(store_unavailable());
    }
    let body = serde_json::to_vec(catalog).map_err(|error| {
        tracing::debug!(%error, "inbound batch catalog serialization failed");
        store_unavailable()
    })?;
    if body.len() > MAX_CATALOG_BYTES {
        return Err(store_unavailable());
    }
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn map_cas_error(error: CasUpdateError<InboundBatchStoreError>) -> InboundBatchStoreError {
    match error {
        CasUpdateError::Apply(error) => error,
        error => {
            tracing::debug!(?error, "inbound batch CAS update failed");
            store_unavailable()
        }
    }
}

fn store_unavailable() -> InboundBatchStoreError {
    InboundBatchStoreError {
        retryable: true,
        reason: "durable provider batch state is unavailable".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::{CasExpectation, InMemoryBackend};
    use serde_json::json;

    fn store() -> FilesystemInboundBatchStore {
        store_on(Arc::new(InMemoryBackend::new()))
    }

    fn store_on(filesystem: Arc<InMemoryBackend>) -> FilesystemInboundBatchStore {
        FilesystemInboundBatchStore::new(
            filesystem,
            TenantId::new("tenant-batch-test").expect("tenant"),
            UserId::new("user-batch-test").expect("user"),
        )
        .expect("batch store")
    }

    fn key(batch_key: &str) -> InboundBatchKey {
        InboundBatchKey {
            extension_id: "acme-chat".to_string(),
            installation_id: "acme-chat-install".to_string(),
            batch_key: batch_key.to_string(),
        }
    }

    fn fragment(
        batch_key: &str,
        fragment_id: &str,
        order: u64,
        event_id: &str,
    ) -> InboundBatchFragment {
        InboundBatchFragment {
            batch_key: batch_key.to_string(),
            fragment_id: fragment_id.to_string(),
            order,
            settle_millis: 100,
            triggered: true,
            message: NormalizedInboundMessage {
                actor: ExternalActorRef::new("acme_user", "42", None::<&str>).expect("actor"),
                conversation: ExternalConversationRef::new(None, "chat-1", None, None)
                    .expect("conversation"),
                event_id: ExternalEventId::new(event_id).expect("event"),
                text: format!("fragment {fragment_id}"),
                trigger: ProductTriggerReason::DirectChat,
                attachments: vec![InboundAttachment {
                    id: fragment_id.to_string(),
                    mime_type: "text/plain".to_string(),
                    filename: Some(format!("{fragment_id}.txt")),
                    bytes: vec![order as u8],
                }],
                conversation_context: None,
                reply_context: None,
            },
        }
    }

    fn request(
        batch_key: &str,
        fragment: InboundBatchFragment,
        staged_at: DateTime<Utc>,
    ) -> InboundBatchStageRequest {
        InboundBatchStageRequest {
            key: key(batch_key),
            binding_fingerprint: "binding-v1".to_string(),
            fragment,
            staged_at,
        }
    }

    async fn stage_pending(
        store: &FilesystemInboundBatchStore,
        request: InboundBatchStageRequest,
    ) -> InboundBatchSchedule {
        match store.stage(request).await.expect("stage") {
            InboundBatchStageOutcome::Pending(schedule) => schedule,
            outcome => panic!("expected pending batch, got {outcome:?}"),
        }
    }

    async fn write_main_snapshot(store: &FilesystemInboundBatchStore, snapshot: serde_json::Value) {
        store
            .filesystem
            .put(
                &store.scope,
                &store.path,
                Entry::bytes(serde_json::to_vec(&snapshot).expect("snapshot json"))
                    .with_content_type(ContentType::json()),
                CasExpectation::Any,
            )
            .await
            .expect("write main snapshot");
    }

    fn main_snapshot_value(
        batch_key: &str,
        fragment: &InboundBatchFragment,
        staged_at: DateTime<Utc>,
    ) -> serde_json::Value {
        let key = key(batch_key);
        let due_at =
            add_duration(staged_at, Duration::from_millis(fragment.settle_millis)).expect("due at");
        let batch = CompleteStoredInboundBatch {
            key: StoredInboundBatchKey::from(&key),
            binding_fingerprint: "binding-v1".to_string(),
            revision: 1,
            settle_millis: fragment.settle_millis,
            last_staged_at: staged_at,
            due_at,
            fragments: vec![StoredInboundBatchFragment::from(fragment)],
            state: StoredInboundBatchState::Open,
        };
        json!({ "batches": { storage_key(&key): batch } })
    }

    #[tokio::test]
    async fn two_maximum_sized_batches_stage_without_a_shared_payload_ceiling() {
        let store = Arc::new(store());
        let now = Utc::now();
        let mut first = fragment("album-one", "one", 1, "event-one");
        first.message.attachments[0].bytes = vec![1; DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes];
        let mut second = fragment("album-two", "two", 1, "event-two");
        second.message.attachments[0].bytes = vec![2; DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes];

        let (first, second) = tokio::join!(
            store.stage(request("album-one", first, now)),
            store.stage(request("album-two", second, now)),
        );

        assert!(matches!(
            first.expect("first maximum batch"),
            InboundBatchStageOutcome::Pending(_)
        ));
        assert!(matches!(
            second.expect("second maximum batch"),
            InboundBatchStageOutcome::Pending(_)
        ));
        assert_eq!(store.pending(now).await.expect("pending").len(), 2);
    }

    #[tokio::test]
    async fn batch_header_is_metadata_only_and_claim_loads_immutable_fragment_bytes() {
        let store = store();
        let now = Utc::now();
        let mut original = fragment("album", "one", 1, "event");
        original.message.attachments[0].bytes = vec![7; 64 * 1024];
        let schedule = stage_pending(&store, request("album", original.clone(), now)).await;
        let storage_key = storage_key(&key("album"));
        let header = store
            .filesystem
            .get(
                &store.scope,
                &batch_path(&storage_key).expect("header path"),
            )
            .await
            .expect("header read")
            .expect("header row");
        assert!(header.entry.body.len() < 8 * 1024);
        let decoded = decode_batch(&header.entry.body).expect("metadata header");
        assert_eq!(decoded.fragments.len(), 1);

        let fragment_row = store
            .filesystem
            .get(
                &store.scope,
                &fragment_path(
                    &storage_key,
                    &decoded.fragments[0].fragment_id_hash,
                    &decoded.fragments[0].content_hash,
                )
                .expect("fragment path"),
            )
            .await
            .expect("fragment read")
            .expect("immutable fragment row");
        assert!(fragment_row.entry.body.len() > original.message.attachments[0].bytes.len());

        let due = add_duration(schedule.due_at, Duration::from_millis(1)).expect("due");
        let claim = store
            .claim_due(&schedule, due)
            .await
            .expect("claim")
            .expect("claimed");
        assert_eq!(claim.fragments, vec![original]);
    }

    #[tokio::test]
    async fn preceding_per_batch_payload_row_migrates_to_fragment_rows() {
        let store = store();
        let now = Utc::now();
        let original = fragment("album", "one", 1, "event");
        let storage_key = storage_key(&key("album"));
        let complete = CompleteStoredInboundBatch {
            key: StoredInboundBatchKey::from(&key("album")),
            binding_fingerprint: "binding-v1".to_string(),
            revision: 1,
            settle_millis: original.settle_millis,
            last_staged_at: now,
            due_at: add_duration(now, Duration::from_millis(original.settle_millis)).expect("due"),
            fragments: vec![StoredInboundBatchFragment::from(&original)],
            state: StoredInboundBatchState::Open,
        };
        store
            .filesystem
            .put(
                &store.scope,
                &store.path,
                encode_sharded_marker(),
                CasExpectation::Any,
            )
            .await
            .expect("marker");
        store
            .filesystem
            .put(
                &store.scope,
                &store.catalog_path,
                encode_catalog(&StoredInboundBatchCatalog {
                    batch_keys: BTreeMap::from([(storage_key.clone(), now)]),
                })
                .expect("catalog"),
                CasExpectation::Any,
            )
            .await
            .expect("catalog write");
        store
            .filesystem
            .put(
                &store.scope,
                &batch_path(&storage_key).expect("batch path"),
                Entry::bytes(serde_json::to_vec(&complete).expect("complete row"))
                    .with_content_type(ContentType::json()),
                CasExpectation::Any,
            )
            .await
            .expect("complete row write");

        let schedule = store
            .pending(now)
            .await
            .expect("migrated pending")
            .into_iter()
            .next()
            .expect("schedule");
        let header = store
            .load_batch_header(&storage_key)
            .await
            .expect("header")
            .expect("migrated header");
        assert_eq!(header.fragments.len(), 1);
        let due = add_duration(schedule.due_at, Duration::from_millis(1)).expect("due");
        let claim = store
            .claim_due(&schedule, due)
            .await
            .expect("claim")
            .expect("claimed");
        assert_eq!(claim.fragments, vec![original]);
    }

    #[tokio::test]
    async fn current_complete_attachment_snapshot_migrates_before_restart() {
        let filesystem = Arc::new(InMemoryBackend::new());
        let store = store_on(Arc::clone(&filesystem));
        let now = Utc::now();
        let original = fragment("album", "one", 1, "event");
        let mut snapshot = main_snapshot_value("album", &original, now);
        let attachment = &mut snapshot["batches"][storage_key(&key("album"))]["fragments"][0]["message"]
            ["attachments"][0];
        *attachment = json!({
            "descriptor": ProductAttachmentDescriptor::new(
                "one",
                "text/plain",
                Some("one.txt".to_string()),
                Some(1),
                ironclaw_extension_contracts::external::ProductAttachmentKind::Document,
            ).expect("previous descriptor"),
            "fetched_id": "one",
            "fetched_mime_type": "text/plain",
            "fetched_filename": "one.txt",
            "fetched_bytes": "AQ"
        });
        write_main_snapshot(&store, snapshot).await;

        let schedule = store
            .pending(now)
            .await
            .expect("migrate current snapshot")
            .into_iter()
            .next()
            .expect("migrated schedule");
        store
            .filesystem
            .delete(&store.scope, &store.path)
            .await
            .expect("remove compatibility marker to prove shard recovery");

        let restarted = store_on(filesystem);
        let due = add_duration(schedule.due_at, Duration::from_millis(1)).expect("due");
        let claim = restarted
            .claim_due(&schedule, due)
            .await
            .expect("claim migrated batch")
            .expect("migrated claim");
        assert_eq!(claim.fragments, vec![original]);
    }

    #[tokio::test]
    async fn legacy_vendor_reference_snapshot_is_quarantined_without_blocking_redelivery() {
        let filesystem = Arc::new(InMemoryBackend::new());
        let store = store_on(Arc::clone(&filesystem));
        let now = Utc::now();
        let original = fragment("album", "one", 1, "event");
        let mut snapshot = main_snapshot_value("album", &original, now);
        let message =
            &mut snapshot["batches"][storage_key(&key("album"))]["fragments"][0]["message"];
        message
            .as_object_mut()
            .expect("message object")
            .remove("conversation_context");
        message["attachments"] = json!([{
            "descriptor": ProductAttachmentDescriptor::new(
                "one",
                "text/plain",
                Some("one.txt".to_string()),
                Some(1),
                ironclaw_extension_contracts::external::ProductAttachmentKind::Document,
            ).expect("legacy descriptor"),
            "vendor_ref": "opaque-provider-handle"
        }]);
        write_main_snapshot(&store, snapshot).await;

        assert!(
            store
                .pending(now)
                .await
                .expect("load legacy main snapshot")
                .is_empty()
        );

        let restarted = store_on(filesystem);
        assert!(matches!(
            restarted
                .stage(request("album", original, now))
                .await
                .expect("complete provider redelivery"),
            InboundBatchStageOutcome::Pending(_)
        ));
    }

    #[tokio::test]
    async fn claim_release_complete_survive_each_restart() {
        let filesystem = Arc::new(InMemoryBackend::new());
        let now = Utc::now();
        let original = fragment("album", "one", 1, "event");
        let schedule = stage_pending(
            &store_on(Arc::clone(&filesystem)),
            request("album", original.clone(), now),
        )
        .await;

        let store = store_on(Arc::clone(&filesystem));
        assert_eq!(
            store.pending(now).await.expect("restart pending"),
            vec![schedule.clone()]
        );
        let due = add_duration(schedule.due_at, Duration::from_millis(1)).expect("due");
        let claim = store
            .claim_due(&schedule, due)
            .await
            .expect("restart claim")
            .expect("claim");

        let store = store_on(Arc::clone(&filesystem));
        let released = store
            .release(&claim, due, Duration::from_millis(5))
            .await
            .expect("restart release")
            .expect("released");
        let retry_due = add_duration(released.due_at, Duration::from_millis(1)).expect("retry due");

        let store = store_on(Arc::clone(&filesystem));
        let claim = store
            .claim_due(&released, retry_due)
            .await
            .expect("restart reclaim")
            .expect("reclaimed");
        assert!(store.complete(&claim, retry_due).await.expect("complete"));

        let store = store_on(filesystem);
        assert_eq!(
            store
                .stage(request("album", original, retry_due))
                .await
                .expect("restart settled redelivery"),
            InboundBatchStageOutcome::AlreadyCompleted
        );
    }

    #[tokio::test]
    async fn duplicate_fragment_is_idempotent_but_conflicting_duplicate_tombstones_batch() {
        let store = store();
        let now = Utc::now();
        let original = fragment("album", "one", 1, "event");
        let first = stage_pending(&store, request("album", original.clone(), now)).await;
        let duplicate = stage_pending(&store, request("album", original.clone(), now)).await;
        assert_eq!(duplicate.revision, first.revision);

        let mut conflicting = original;
        conflicting.message.text = "different payload".to_string();
        assert_eq!(
            store
                .stage(request("album", conflicting, now))
                .await
                .expect("conflicting stage"),
            InboundBatchStageOutcome::Rejected
        );
        assert_eq!(
            store
                .stage(request("album", fragment("album", "two", 2, "event"), now))
                .await
                .expect("rejected tombstone"),
            InboundBatchStageOutcome::Rejected
        );
    }

    #[tokio::test]
    async fn fragment_limit_rejects_the_whole_batch_atomically() {
        let store = store();
        let now = Utc::now();
        for index in 0..MAX_FRAGMENTS_PER_BATCH {
            let id = format!("fragment-{index}");
            let mut next = fragment("album", &id, index as u64, "event");
            next.message.attachments.clear();
            stage_pending(&store, request("album", next, now)).await;
        }
        let mut overflow = fragment("album", "overflow", 33, "event");
        overflow.message.attachments.clear();
        assert_eq!(
            store
                .stage(request("album", overflow, now))
                .await
                .expect("overflow stage"),
            InboundBatchStageOutcome::Rejected
        );
        assert!(store.pending(now).await.expect("pending").is_empty());
    }

    #[tokio::test]
    async fn lease_expiry_reclaims_once_and_stale_claim_cannot_finish() {
        let store = store();
        let now = Utc::now();
        let schedule = stage_pending(
            &store,
            request("album", fragment("album", "one", 1, "event"), now),
        )
        .await;
        let due = add_duration(schedule.due_at, Duration::from_millis(1)).expect("due");
        let first = store
            .claim_due(&schedule, due)
            .await
            .expect("first claim")
            .expect("claim");
        assert!(
            store
                .claim_due(&schedule, due)
                .await
                .expect("concurrent claim")
                .is_none()
        );

        let after_lease =
            add_duration(due, CLAIM_LEASE + Duration::from_millis(1)).expect("after lease");
        let second = store
            .claim_due(&schedule, after_lease)
            .await
            .expect("reclaim")
            .expect("reclaimed claim");
        assert_ne!(first.claim_id, second.claim_id);
        assert!(
            !store
                .complete(&first, after_lease)
                .await
                .expect("stale finish")
        );
        assert!(
            store
                .complete(&second, after_lease)
                .await
                .expect("current finish")
        );
    }

    #[tokio::test]
    async fn retry_release_advances_revision_and_defers_reclaim() {
        let store = store();
        let now = Utc::now();
        let schedule = stage_pending(
            &store,
            request("album", fragment("album", "one", 1, "event"), now),
        )
        .await;
        let due = add_duration(schedule.due_at, Duration::from_millis(1)).expect("due");
        let claim = store
            .claim_due(&schedule, due)
            .await
            .expect("claim")
            .expect("claimed");
        let retry_after = Duration::from_secs(10);
        let released = store
            .release(&claim, due, retry_after)
            .await
            .expect("release")
            .expect("released schedule");
        assert_eq!(released.revision, schedule.revision + 1);
        assert!(
            store
                .claim_due(&released, due)
                .await
                .expect("early claim")
                .is_none()
        );
        let retry_due = add_duration(released.due_at, Duration::from_millis(1)).expect("retry due");
        assert!(
            store
                .claim_due(&released, retry_due)
                .await
                .expect("retry claim")
                .is_some()
        );
    }

    #[tokio::test]
    async fn completion_tombstone_absorbs_redelivery_then_expires() {
        let store = store();
        let now = Utc::now();
        let original = fragment("album", "one", 1, "event");
        let schedule = stage_pending(&store, request("album", original.clone(), now)).await;
        let due = add_duration(schedule.due_at, Duration::from_millis(1)).expect("due");
        let claim = store
            .claim_due(&schedule, due)
            .await
            .expect("claim")
            .expect("claimed");
        assert!(store.complete(&claim, due).await.expect("complete"));
        assert_eq!(
            store
                .stage(request("album", original.clone(), due))
                .await
                .expect("redelivery"),
            InboundBatchStageOutcome::AlreadyCompleted
        );

        let after_tombstone =
            add_duration(due, TERMINAL_TTL + Duration::from_millis(1)).expect("expiry");
        let schedule = stage_pending(&store, request("album", original, after_tombstone)).await;
        assert_eq!(schedule.revision, 1);
    }

    #[tokio::test]
    async fn binding_drift_rejects_staged_canonical_messages() {
        let store = store();
        let now = Utc::now();
        stage_pending(
            &store,
            request("album", fragment("album", "one", 1, "event"), now),
        )
        .await;
        let mut drifted = request("album", fragment("album", "two", 2, "event"), now);
        drifted.binding_fingerprint = "binding-v2".to_string();
        assert_eq!(
            store.stage(drifted).await.expect("drifted stage"),
            InboundBatchStageOutcome::Rejected
        );
        assert!(store.pending(now).await.expect("pending").is_empty());
    }
}
