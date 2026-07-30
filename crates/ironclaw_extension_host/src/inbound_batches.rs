//! Durable staging for provider-level inbound message batches.
//!
//! Some providers deliver one logical message as multiple serialized webhook
//! requests. Each verified fragment is therefore staged before its webhook is
//! acknowledged, then a leased background worker admits the merged message
//! after the provider-selected quiet window. The snapshot is host-private:
//! opaque vendor attachment references never enter events, projections,
//! transcripts, or model-visible state.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, FilesystemError, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    HostApiError, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
    ScopedPath, TenantId, UserId, VirtualPath, resource_scope_path_segment,
};
use ironclaw_product::{
    ChannelAttachmentRef, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    InboundBatchFragment, NormalizedInboundMessage, ProductAttachmentDescriptor,
    ProductTriggerReason,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const INBOUND_BATCH_ALIAS: &str = "/tenant-shared/inbound-batches";
const INBOUND_BATCH_SNAPSHOT_PATH: &str = "/tenant-shared/inbound-batches/pending.json";
const MAX_BATCHES: usize = 1_024;
const MAX_FRAGMENTS_PER_BATCH: usize = 32;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
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
/// host-private provider attachment handles.
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

/// Filesystem-backed [`InboundBatchStore`] using one bounded CAS snapshot per
/// tenant. A single snapshot makes startup recovery independent of directory
/// scans and keeps every transition atomic.
pub struct FilesystemInboundBatchStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
    path: ScopedPath,
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
        })
    }

    async fn update<T, F>(&self, apply: F) -> Result<T, InboundBatchStoreError>
    where
        T: Send,
        F: FnMut(
                Option<StoredInboundBatchSnapshot>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = Result<
                                CasApply<StoredInboundBatchSnapshot, T>,
                                InboundBatchStoreError,
                            >,
                        > + Send,
                >,
            > + Send,
    {
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &self.path,
            decode_snapshot,
            encode_snapshot,
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
}

#[async_trait]
impl InboundBatchStore for FilesystemInboundBatchStore {
    async fn stage(
        &self,
        request: InboundBatchStageRequest,
    ) -> Result<InboundBatchStageOutcome, InboundBatchStoreError> {
        let storage_key = storage_key(&request.key);
        self.update(move |current| {
            let request = request.clone();
            let storage_key = storage_key.clone();
            Box::pin(async move {
                let mut snapshot = current.unwrap_or_default();
                prune_expired(&mut snapshot, request.staged_at);
                let outcome = if let Some(batch) = snapshot.batches.get_mut(&storage_key) {
                    if batch.key != StoredInboundBatchKey::from(&request.key)
                        || batch.binding_fingerprint != request.binding_fingerprint
                    {
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
                        batch.state = StoredInboundBatchState::Rejected {
                            terminal_at: request.staged_at,
                        };
                        batch.fragments.clear();
                        InboundBatchStageOutcome::Rejected
                    } else {
                        match &batch.state {
                            StoredInboundBatchState::Claimed { .. } => {
                                if batch.fragments.iter().any(|stored| {
                                    stored == &StoredInboundBatchFragment::from(&request.fragment)
                                }) {
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
                                    if existing
                                        != &StoredInboundBatchFragment::from(&request.fragment)
                                    {
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
                                        batch.state = StoredInboundBatchState::Rejected {
                                            terminal_at: request.staged_at,
                                        };
                                        batch.fragments.clear();
                                        return Ok(CasApply::new(
                                            snapshot,
                                            InboundBatchStageOutcome::Rejected,
                                        ));
                                    }
                                    batch
                                        .fragments
                                        .push(StoredInboundBatchFragment::from(&request.fragment));
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
                    if snapshot.batches.len() >= MAX_BATCHES {
                        return Err(InboundBatchStoreError {
                            retryable: true,
                            reason: "provider batch staging capacity exhausted".to_string(),
                        });
                    }
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
                        fragments: vec![StoredInboundBatchFragment::from(&request.fragment)],
                        state: StoredInboundBatchState::Open,
                    };
                    let schedule = batch.schedule();
                    snapshot.batches.insert(storage_key, batch);
                    InboundBatchStageOutcome::Pending(schedule)
                };
                Ok(CasApply::new(snapshot, outcome))
            })
        })
        .await
    }

    async fn claim_due(
        &self,
        schedule: &InboundBatchSchedule,
        now: DateTime<Utc>,
    ) -> Result<Option<ClaimedInboundBatch>, InboundBatchStoreError> {
        let storage_key = storage_key(&schedule.key);
        let schedule = schedule.clone();
        let claim_id = InvocationId::new().to_string();
        self.update(move |current| {
            let schedule = schedule.clone();
            let storage_key = storage_key.clone();
            let claim_id = claim_id.clone();
            Box::pin(async move {
                let Some(mut snapshot) = current else {
                    return Ok(CasApply::no_op(StoredInboundBatchSnapshot::default(), None));
                };
                let Some(batch) = snapshot.batches.get_mut(&storage_key) else {
                    return Ok(CasApply::no_op(snapshot, None));
                };
                if batch.key != StoredInboundBatchKey::from(&schedule.key)
                    || batch.binding_fingerprint != schedule.binding_fingerprint
                    || batch.revision != schedule.revision
                    || batch.due_at > now
                {
                    return Ok(CasApply::no_op(snapshot, None));
                }
                let claimable = match &batch.state {
                    StoredInboundBatchState::Open => true,
                    StoredInboundBatchState::Claimed { lease_until, .. } => *lease_until <= now,
                    StoredInboundBatchState::Completed { .. }
                    | StoredInboundBatchState::Rejected { .. } => false,
                };
                if !claimable {
                    return Ok(CasApply::no_op(snapshot, None));
                }
                let lease_until = add_duration(now, CLAIM_LEASE)?;
                batch.state = StoredInboundBatchState::Claimed {
                    claim_id: claim_id.clone(),
                    lease_until,
                };
                let claim = ClaimedInboundBatch {
                    schedule: batch.schedule(),
                    claim_id,
                    fragments: batch
                        .fragments
                        .iter()
                        .cloned()
                        .map(StoredInboundBatchFragment::into_fragment)
                        .collect(),
                };
                Ok(CasApply::new(snapshot, Some(claim)))
            })
        })
        .await
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
        let storage_key = storage_key(&claim.schedule.key);
        let claim = claim.clone();
        self.update(move |current| {
            let storage_key = storage_key.clone();
            let claim = claim.clone();
            Box::pin(async move {
                let Some(mut snapshot) = current else {
                    return Ok(CasApply::no_op(StoredInboundBatchSnapshot::default(), None));
                };
                let Some(batch) = snapshot.batches.get_mut(&storage_key) else {
                    return Ok(CasApply::no_op(snapshot, None));
                };
                if !claim_matches(batch, &claim) {
                    return Ok(CasApply::no_op(snapshot, None));
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
                Ok(CasApply::new(snapshot, Some(schedule)))
            })
        })
        .await
    }

    async fn pending(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<InboundBatchSchedule>, InboundBatchStoreError> {
        let versioned = match self.filesystem.get(&self.scope, &self.path).await {
            Ok(versioned) => versioned,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => {
                tracing::debug!(%error, "inbound batch snapshot read failed");
                return Err(store_unavailable());
            }
        };
        let Some(versioned) = versioned else {
            return Ok(Vec::new());
        };
        let snapshot = decode_snapshot(&versioned.entry.body)?;
        Ok(snapshot
            .batches
            .values()
            .filter(|batch| {
                age_within(now, batch.last_staged_at, PENDING_TTL)
                    && match &batch.state {
                        StoredInboundBatchState::Open => true,
                        StoredInboundBatchState::Claimed { lease_until, .. } => *lease_until <= now,
                        StoredInboundBatchState::Completed { .. }
                        | StoredInboundBatchState::Rejected { .. } => false,
                    }
            })
            .map(StoredInboundBatch::schedule)
            .collect())
    }
}

impl FilesystemInboundBatchStore {
    async fn finish(
        &self,
        claim: &ClaimedInboundBatch,
        terminal: StoredTerminal,
        now: DateTime<Utc>,
    ) -> Result<bool, InboundBatchStoreError> {
        let storage_key = storage_key(&claim.schedule.key);
        let claim = claim.clone();
        self.update(move |current| {
            let storage_key = storage_key.clone();
            let claim = claim.clone();
            Box::pin(async move {
                let Some(mut snapshot) = current else {
                    return Ok(CasApply::no_op(
                        StoredInboundBatchSnapshot::default(),
                        false,
                    ));
                };
                let Some(batch) = snapshot.batches.get_mut(&storage_key) else {
                    return Ok(CasApply::no_op(snapshot, false));
                };
                if !claim_matches(batch, &claim) {
                    return Ok(CasApply::no_op(snapshot, false));
                }
                batch.fragments.clear();
                batch.state = match terminal {
                    StoredTerminal::Completed => {
                        StoredInboundBatchState::Completed { terminal_at: now }
                    }
                    StoredTerminal::Rejected => {
                        StoredInboundBatchState::Rejected { terminal_at: now }
                    }
                };
                Ok(CasApply::new(snapshot, true))
            })
        })
        .await
    }
}

#[derive(Clone, Copy)]
enum StoredTerminal {
    Completed,
    Rejected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StoredInboundBatchSnapshot {
    batches: BTreeMap<String, StoredInboundBatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredInboundBatch {
    key: StoredInboundBatchKey,
    binding_fingerprint: String,
    revision: u64,
    settle_millis: u64,
    last_staged_at: DateTime<Utc>,
    due_at: DateTime<Utc>,
    fragments: Vec<StoredInboundBatchFragment>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredNormalizedInboundMessage {
    actor: ExternalActorRef,
    conversation: ExternalConversationRef,
    event_id: ExternalEventId,
    text: String,
    trigger: ProductTriggerReason,
    attachments: Vec<StoredChannelAttachmentRef>,
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
                .map(StoredChannelAttachmentRef::from)
                .collect(),
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
                .map(StoredChannelAttachmentRef::into_attachment)
                .collect(),
            reply_context: self.reply_context,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredChannelAttachmentRef {
    descriptor: ProductAttachmentDescriptor,
    vendor_ref: String,
}

impl std::fmt::Debug for StoredChannelAttachmentRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredChannelAttachmentRef")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

impl From<&ChannelAttachmentRef> for StoredChannelAttachmentRef {
    fn from(attachment: &ChannelAttachmentRef) -> Self {
        Self {
            descriptor: attachment.descriptor.clone(),
            vendor_ref: attachment.vendor_ref.clone(),
        }
    }
}

impl StoredChannelAttachmentRef {
    fn into_attachment(self) -> ChannelAttachmentRef {
        ChannelAttachmentRef {
            descriptor: self.descriptor,
            vendor_ref: self.vendor_ref,
        }
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

fn fragments_are_compatible(batch: &StoredInboundBatch, incoming: &InboundBatchFragment) -> bool {
    let Some(first) = batch.fragments.first() else {
        return false;
    };
    first.batch_key == incoming.batch_key
        && first.message.event_id == incoming.message.event_id
        && first.message.actor == incoming.message.actor
        && first.message.conversation == incoming.message.conversation
        && batch.fragments.iter().all(|fragment| {
            !fragment.triggered
                || !incoming.triggered
                || fragment.message.trigger == incoming.message.trigger
        })
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

fn prune_expired(snapshot: &mut StoredInboundBatchSnapshot, now: DateTime<Utc>) {
    snapshot.batches.retain(|_, batch| match &batch.state {
        StoredInboundBatchState::Completed { terminal_at }
        | StoredInboundBatchState::Rejected { terminal_at } => {
            age_within(now, *terminal_at, TERMINAL_TTL)
        }
        StoredInboundBatchState::Open | StoredInboundBatchState::Claimed { .. } => {
            age_within(now, batch.last_staged_at, PENDING_TTL)
        }
    });
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

fn decode_snapshot(bytes: &[u8]) -> Result<StoredInboundBatchSnapshot, InboundBatchStoreError> {
    serde_json::from_slice(bytes).map_err(|error| {
        tracing::warn!(%error, "malformed inbound batch snapshot");
        store_unavailable()
    })
}

fn encode_snapshot(snapshot: &StoredInboundBatchSnapshot) -> Result<Entry, InboundBatchStoreError> {
    let body = serde_json::to_vec(snapshot).map_err(|error| {
        tracing::debug!(%error, "inbound batch snapshot serialization failed");
        store_unavailable()
    })?;
    if body.len() > MAX_SNAPSHOT_BYTES {
        return Err(InboundBatchStoreError {
            retryable: true,
            reason: "provider batch snapshot exceeds its storage bound".to_string(),
        });
    }
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
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
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_product::ProductAttachmentKind;

    fn store() -> FilesystemInboundBatchStore {
        FilesystemInboundBatchStore::new(
            Arc::new(InMemoryBackend::new()),
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
                attachments: vec![ChannelAttachmentRef {
                    descriptor: ProductAttachmentDescriptor::new(
                        fragment_id,
                        "text/plain",
                        Some(format!("{fragment_id}.txt")),
                        Some(1),
                        ProductAttachmentKind::Document,
                    )
                    .expect("descriptor"),
                    vendor_ref: format!("opaque-{fragment_id}"),
                }],
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
            stage_pending(
                &store,
                request("album", fragment("album", &id, index as u64, "event"), now),
            )
            .await;
        }
        assert_eq!(
            store
                .stage(request(
                    "album",
                    fragment("album", "overflow", 33, "event"),
                    now
                ))
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
    async fn binding_drift_rejects_staged_provider_handles() {
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
