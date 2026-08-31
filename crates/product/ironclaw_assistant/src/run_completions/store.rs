//! Durable run-completion notice store over the `/run-notices` per-user
//! mount (2026-08-13 design §5.3–§5.4).
//!
//! Additive filesystem data: versioned notice records plus ordered indexes
//! declared through `ensure_index` — no relational migration, no backend
//! branch. Every read-modify-write goes through the shared bounded
//! `cas_update` helper; the per-owner sequence counter and every
//! state-machine transition are CAS documents, so multi-replica races have
//! exactly one winner. Composition selects the backend; this store never
//! names one.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, Filter, IndexKey, IndexKind, IndexName,
    IndexSpec, IndexValue, OrderedPage, RecordKind, RootFilesystem, ScopedFilesystem,
    SortDirection, cas_update,
};
use ironclaw_host_api::ids::{InvocationId, TenantId, UserId};
use ironclaw_host_api::path::ScopedPath;
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_product_contracts::run_completions::{
    RUN_COMPLETION_MAX_INTENTS_PER_NOTICE, RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT,
};

use super::records::{
    CompletionDeliveryState, CompletionIntentRecord, CompletionReadEvidence, CompletionReadState,
    CompletionSurface, RUN_COMPLETION_NOTICE_VERSION, RunCompletionNotice,
};

/// The per-user mount alias this store lives under. Registered by
/// composition's mount catalog (`PER_USER_ALIASES`); a row in
/// `docs/internal/reborn/contracts/storage-placement.md` §4 documents it.
pub const RUN_NOTICES_MOUNT_ALIAS: &str = "/run-notices";

const NOTICE_RECORD_KIND: &str = "run_completion_notice";
const SEQUENCE_RECORD_KIND: &str = "run_completion_sequence";
const DUE_OWNERS_RECORD_KIND: &str = "run_completion_due_owners";
/// Per-tenant durable due-owner registry address (§5.4 boot reconciliation).
/// Rides the `/tenant-shared` mount so a restarted coordinator can find the
/// owners with unsettled work without a cross-user scan.
const DUE_OWNERS_PATH: &str = "/tenant-shared/run-completions/due-owners.json";
/// Registry bound: far above any active-owner population; hitting it fails
/// retryably (§5.4: the writer fails before acknowledging the observer
/// cursor rather than dropping recovery state).
const MAX_DUE_OWNERS: usize = 100_000;

const ORDER_INDEX: &str = "run_notice_order";
const UNREAD_INDEX: &str = "run_notice_unread";
const STATE_INDEX: &str = "run_notice_state";
const THREAD_INDEX: &str = "run_notice_thread";

const OWNER_KEY: &str = "owner_key";
const UNREAD_PARTITION_KEY: &str = "unread_partition";
const STATE_PARTITION_KEY: &str = "state_partition";
const THREAD_PARTITION_KEY: &str = "thread_partition";
const SEQUENCE_SORT_KEY: &str = "sequence_sort";
const NOTICE_ID_KEY: &str = "notice_id";

/// The identity every operation is scoped to: the notice owner. Both halves
/// come from the authenticated caller or the run's own scope — never from a
/// browser payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCompletionOwner {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

impl RunCompletionOwner {
    fn resource_scope(&self) -> ResourceScope {
        ResourceScope {
            tenant_id: self.tenant_id.clone(),
            user_id: self.user_id.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn owner_key(&self) -> String {
        format!("{}\u{1f}{}", self.tenant_id.as_str(), self.user_id.as_str())
    }
}

/// Sanitized store failure. The variant split is the retry contract: the
/// observer retries `Unavailable` (its durable cursor holds position) and
/// surfaces `Invalid` as a caller error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunCompletionStoreError {
    #[error("run completion store unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("run completion store rejected the operation: {reason}")]
    Invalid { reason: &'static str },
    #[error("run completion notice not found")]
    NotFound,
    /// The record was not in a state the requested transition applies to.
    /// CAS-observed; the caller re-reads and re-decides.
    #[error("run completion notice transition conflict: {reason}")]
    Conflict { reason: &'static str },
}

impl RunCompletionStoreError {
    fn backend(operation: &'static str, error: impl std::fmt::Display) -> Self {
        Self::Unavailable {
            reason: format!("{operation}: {error}"),
        }
    }
}

/// Outcome of an idempotent notice create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeCreateOutcome {
    Created(RunCompletionNotice),
    AlreadyRecorded(RunCompletionNotice),
}

/// The immutable-fact half of a new notice; the store stamps state and
/// sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRunCompletionNotice {
    pub notice_id: String,
    pub run_id: String,
    pub thread_id: String,
    /// Agent/project halves of the completed run's scope, carried so the
    /// push fallback can rebuild the exact `TurnScope` for outbound
    /// authorization (§7.9) without a transcript lookup.
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub thread_tag: String,
    pub terminal_projection_ref: String,
    pub completed_at: DateTime<Utc>,
    /// When the arbitration intent window closes (§5.4).
    pub arbitration_closes_at: DateTime<Utc>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq)]
struct SequenceDocument {
    version: u32,
    next: u64,
}

/// The per-tenant due-owner registry (§5.4): the user ids with potentially
/// unsettled coordinator work, CAS-maintained, read once at boot.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Default)]
struct DueOwnersDocument {
    version: u32,
    /// Sorted, deduplicated user ids.
    owners: Vec<String>,
}

pub struct RunCompletionNoticeStore<F>
where
    F: RootFilesystem + ?Sized,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> RunCompletionNoticeStore<F>
where
    F: RootFilesystem + ?Sized,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn alias_root() -> Result<ScopedPath, RunCompletionStoreError> {
        ScopedPath::new(RUN_NOTICES_MOUNT_ALIAS).map_err(|error| {
            RunCompletionStoreError::backend("run-notices alias parse", error)
        })
    }

    fn notice_path(notice_id: &str) -> Result<ScopedPath, RunCompletionStoreError> {
        if notice_id.is_empty()
            || notice_id.len() > 128
            || !notice_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(RunCompletionStoreError::Invalid {
                reason: "notice id is not a bounded opaque identifier",
            });
        }
        ScopedPath::new(format!("{RUN_NOTICES_MOUNT_ALIAS}/notices/{notice_id}.json"))
            .map_err(|error| RunCompletionStoreError::backend("notice path parse", error))
    }

    fn due_owners_path() -> Result<ScopedPath, RunCompletionStoreError> {
        ScopedPath::new(DUE_OWNERS_PATH)
            .map_err(|error| RunCompletionStoreError::backend("due-owners path parse", error))
    }

    fn sequence_path() -> Result<ScopedPath, RunCompletionStoreError> {
        ScopedPath::new(format!("{RUN_NOTICES_MOUNT_ALIAS}/sequence.json"))
            .map_err(|error| RunCompletionStoreError::backend("sequence path parse", error))
    }

    fn index_key(name: &str) -> Result<IndexKey, RunCompletionStoreError> {
        IndexKey::new(name)
            .map_err(|error| RunCompletionStoreError::backend("index key parse", error))
    }

    fn index_name(name: &str) -> Result<IndexName, RunCompletionStoreError> {
        IndexName::new(name)
            .map_err(|error| RunCompletionStoreError::backend("index name parse", error))
    }

    fn sequence_sort(sequence: u64) -> String {
        format!("{sequence:020}")
    }

    fn unread_partition(owner: &RunCompletionOwner, unread: bool) -> String {
        format!("{}\u{1f}{}", owner.owner_key(), u8::from(unread))
    }

    fn state_partition(owner: &RunCompletionOwner, state: &CompletionDeliveryState) -> String {
        let state_kind = match state {
            CompletionDeliveryState::PendingArbitration { .. } => "pending",
            CompletionDeliveryState::Granted { .. } => "granted",
            CompletionDeliveryState::Presented { .. } => "presented",
            CompletionDeliveryState::PushOwned { .. } => "push_owned",
            CompletionDeliveryState::NoExternalTarget { .. } => "no_external_target",
        };
        format!("{}\u{1f}{state_kind}", owner.owner_key())
    }

    fn thread_partition(owner: &RunCompletionOwner, thread_id: &str, unread: bool) -> String {
        format!(
            "{}\u{1f}{thread_id}\u{1f}{}",
            owner.owner_key(),
            u8::from(unread)
        )
    }

    fn notice_entry(
        owner: &RunCompletionOwner,
        notice: &RunCompletionNotice,
    ) -> Result<Entry, RunCompletionStoreError> {
        let body = serde_json::to_vec(notice)
            .map_err(|error| RunCompletionStoreError::backend("notice serialize", error))?;
        let kind = RecordKind::new(NOTICE_RECORD_KIND)
            .map_err(|error| RunCompletionStoreError::backend("notice record kind", error))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        let unread = !notice.is_read();
        Ok(entry
            .with_indexed(
                Self::index_key(OWNER_KEY)?,
                IndexValue::Text(owner.owner_key()),
            )
            .with_indexed(
                Self::index_key(UNREAD_PARTITION_KEY)?,
                IndexValue::Text(Self::unread_partition(owner, unread)),
            )
            .with_indexed(
                Self::index_key(STATE_PARTITION_KEY)?,
                IndexValue::Text(Self::state_partition(owner, &notice.delivery)),
            )
            .with_indexed(
                Self::index_key(THREAD_PARTITION_KEY)?,
                IndexValue::Text(Self::thread_partition(owner, &notice.thread_id, unread)),
            )
            .with_indexed(
                Self::index_key(SEQUENCE_SORT_KEY)?,
                IndexValue::Text(Self::sequence_sort(notice.sequence)),
            )
            .with_indexed(
                Self::index_key(NOTICE_ID_KEY)?,
                IndexValue::Text(notice.notice_id.clone()),
            ))
    }

    fn decode(bytes: &[u8]) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        serde_json::from_slice(bytes)
            .map_err(|error| RunCompletionStoreError::backend("notice decode", error))
    }

    /// Declare the ordered listing projections. Idempotent; byte-only test
    /// backends without index support are tolerated (queries then fail with
    /// `Unsupported`, which production record-store mounts never hit).
    pub async fn ensure_indexes(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<(), RunCompletionStoreError> {
        let scope = owner.resource_scope();
        let root = Self::alias_root()?;
        for (name, partition_key) in [
            (ORDER_INDEX, OWNER_KEY),
            (UNREAD_INDEX, UNREAD_PARTITION_KEY),
            (STATE_INDEX, STATE_PARTITION_KEY),
            (THREAD_INDEX, THREAD_PARTITION_KEY),
        ] {
            let spec = IndexSpec::new(
                Self::index_name(name)?,
                vec![
                    Self::index_key(partition_key)?,
                    Self::index_key(SEQUENCE_SORT_KEY)?,
                    Self::index_key(NOTICE_ID_KEY)?,
                ],
                IndexKind::Exact,
            );
            match self.filesystem.ensure_index(&scope, &root, &spec).await {
                Ok(()) => {}
                Err(ironclaw_filesystem::FilesystemError::Unsupported { .. }) => return Ok(()),
                Err(error) => {
                    return Err(RunCompletionStoreError::backend("ensure_index", error));
                }
            }
        }
        Ok(())
    }

    /// Allocate the next member of the owner's monotonic completion
    /// sequence. Racing replicas may burn numbers; the sequence is
    /// monotonic, not dense.
    async fn allocate_sequence(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<u64, RunCompletionStoreError> {
        let scope = owner.resource_scope();
        let path = Self::sequence_path()?;
        let allocated = cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            |bytes: &[u8]| {
                serde_json::from_slice::<SequenceDocument>(bytes).map_err(|error| {
                    RunCompletionStoreError::backend("sequence decode", error)
                })
            },
            |document: &SequenceDocument| {
                let body = serde_json::to_vec(document).map_err(|error| {
                    RunCompletionStoreError::backend("sequence serialize", error)
                })?;
                let kind = RecordKind::new(SEQUENCE_RECORD_KIND).map_err(|error| {
                    RunCompletionStoreError::backend("sequence record kind", error)
                })?;
                let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
                entry.kind = Some(kind);
                Ok(entry)
            },
            |current: Option<SequenceDocument>| async move {
                let next = current.map(|document| document.next).unwrap_or(1);
                Ok(CasApply::new(
                    SequenceDocument {
                        version: 1,
                        next: next.saturating_add(1),
                    },
                    next,
                ))
            },
        )
        .await
        .map_err(flatten_cas_error("sequence allocate"))?;
        Ok(allocated)
    }

    async fn update_due_owners<M>(
        &self,
        scope_owner: &RunCompletionOwner,
        operation: &'static str,
        mutate: M,
    ) -> Result<(), RunCompletionStoreError>
    where
        M: Fn(Vec<String>) -> Result<Vec<String>, RunCompletionStoreError>
            + Send
            + Sync
            + 'static,
    {
        let scope = scope_owner.resource_scope();
        let path = Self::due_owners_path()?;
        let mutate = std::sync::Arc::new(mutate);
        cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            |bytes: &[u8]| {
                serde_json::from_slice::<DueOwnersDocument>(bytes).map_err(|error| {
                    RunCompletionStoreError::backend("due-owners decode", error)
                })
            },
            |document: &DueOwnersDocument| {
                let body = serde_json::to_vec(document).map_err(|error| {
                    RunCompletionStoreError::backend("due-owners serialize", error)
                })?;
                let kind = RecordKind::new(DUE_OWNERS_RECORD_KIND).map_err(|error| {
                    RunCompletionStoreError::backend("due-owners record kind", error)
                })?;
                let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
                entry.kind = Some(kind);
                Ok(entry)
            },
            {
                let mutate = std::sync::Arc::clone(&mutate);
                move |current: Option<DueOwnersDocument>| {
                    let mutate = std::sync::Arc::clone(&mutate);
                    async move {
                        let owners = mutate(current.map(|doc| doc.owners).unwrap_or_default())?;
                        Ok(CasApply::new(DueOwnersDocument { version: 1, owners }, ()))
                    }
                }
            },
        )
        .await
        .map_err(flatten_cas_error(operation))?;
        Ok(())
    }

    /// Record `owner` as having potentially due coordinator work (§5.4).
    /// Idempotent; bounded by [`MAX_DUE_OWNERS`] with a retryable failure on
    /// overflow so the observer cursor holds rather than losing recovery
    /// state.
    pub async fn mark_owner_due(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<(), RunCompletionStoreError> {
        let user = owner.user_id.as_str().to_string();
        self.update_due_owners(owner, "due-owners mark", move |mut owners| {
            match owners.binary_search(&user) {
                Ok(_) => Ok(owners),
                Err(position) => {
                    if owners.len() >= MAX_DUE_OWNERS {
                        return Err(RunCompletionStoreError::Unavailable {
                            reason: "due-owner registry is full; retry after settlement"
                                .to_string(),
                        });
                    }
                    owners.insert(position, user.clone());
                    Ok(owners)
                }
            }
        })
        .await
    }

    /// Remove `owner` from the due registry after a scan found no work.
    /// A lost removal is harmless (one extra empty scan later).
    pub async fn clear_owner_due(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<(), RunCompletionStoreError> {
        let user = owner.user_id.as_str().to_string();
        self.update_due_owners(owner, "due-owners clear", move |mut owners| {
            if let Ok(position) = owners.binary_search(&user) {
                owners.remove(position);
            }
            Ok(owners)
        })
        .await
    }

    /// The owners with potentially due work in `scope_owner`'s tenant (§5.4
    /// boot reconciliation). `scope_owner` supplies only the tenant half of
    /// the registry address.
    pub async fn due_owners(
        &self,
        scope_owner: &RunCompletionOwner,
    ) -> Result<Vec<RunCompletionOwner>, RunCompletionStoreError> {
        let scope = scope_owner.resource_scope();
        let path = Self::due_owners_path()?;
        let Some(entry) = self
            .filesystem
            .get(&scope, &path)
            .await
            .map_err(|error| RunCompletionStoreError::backend("due-owners read", error))?
        else {
            return Ok(Vec::new());
        };
        let document: DueOwnersDocument = serde_json::from_slice(&entry.entry.body)
            .map_err(|error| RunCompletionStoreError::backend("due-owners decode", error))?;
        let mut owners = Vec::with_capacity(document.owners.len());
        for user in document.owners {
            let Ok(user_id) = UserId::new(user) else {
                // A malformed persisted id cannot be scanned; skip rather
                // than wedge boot recovery on one bad row.
                continue;
            };
            owners.push(RunCompletionOwner {
                tenant_id: scope_owner.tenant_id.clone(),
                user_id,
            });
        }
        Ok(owners)
    }

    /// Idempotently create one notice. Duplicate journal delivery (or a
    /// racing replica) observes the existing record and rewrites nothing.
    pub async fn create_notice(
        &self,
        owner: &RunCompletionOwner,
        new_notice: NewRunCompletionNotice,
    ) -> Result<NoticeCreateOutcome, RunCompletionStoreError> {
        self.ensure_indexes(owner).await?;
        let scope = owner.resource_scope();
        let path = Self::notice_path(&new_notice.notice_id)?;
        if let Some(existing) = self
            .filesystem
            .get(&scope, &path)
            .await
            .map_err(|error| RunCompletionStoreError::backend("notice read", error))?
        {
            return Ok(NoticeCreateOutcome::AlreadyRecorded(Self::decode(
                &existing.entry.body,
            )?));
        }
        let sequence = self.allocate_sequence(owner).await?;
        let now = Utc::now();
        let notice = RunCompletionNotice {
            version: RUN_COMPLETION_NOTICE_VERSION,
            notice_id: new_notice.notice_id,
            sequence,
            tenant_id: owner.tenant_id.as_str().to_string(),
            owner_user_id: owner.user_id.as_str().to_string(),
            run_id: new_notice.run_id,
            thread_id: new_notice.thread_id,
            agent_id: new_notice.agent_id,
            project_id: new_notice.project_id,
            thread_tag: new_notice.thread_tag,
            terminal_projection_ref: new_notice.terminal_projection_ref,
            completed_at: new_notice.completed_at,
            delivery: CompletionDeliveryState::PendingArbitration {
                closes_at: new_notice.arbitration_closes_at,
                grants_issued: 0,
            },
            read: CompletionReadState::Unread,
            intents: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        let entry = Self::notice_entry(owner, &notice)?;
        match self
            .filesystem
            .put(
                &scope,
                &path,
                entry,
                ironclaw_filesystem::CasExpectation::Absent,
            )
            .await
        {
            Ok(_) => Ok(NoticeCreateOutcome::Created(notice)),
            Err(ironclaw_filesystem::FilesystemError::VersionMismatch { .. }) => {
                // A racing writer created it first; observe their record.
                let existing = self
                    .filesystem
                    .get(&scope, &path)
                    .await
                    .map_err(|error| RunCompletionStoreError::backend("notice reread", error))?
                    .ok_or(RunCompletionStoreError::Unavailable {
                        reason: "notice vanished between conflicting create and reread"
                            .to_string(),
                    })?;
                Ok(NoticeCreateOutcome::AlreadyRecorded(Self::decode(
                    &existing.entry.body,
                )?))
            }
            Err(error) => Err(RunCompletionStoreError::backend("notice create", error)),
        }
    }

    pub async fn get(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
    ) -> Result<Option<RunCompletionNotice>, RunCompletionStoreError> {
        let scope = owner.resource_scope();
        let path = Self::notice_path(notice_id)?;
        let Some(versioned) = self
            .filesystem
            .get(&scope, &path)
            .await
            .map_err(|error| RunCompletionStoreError::backend("notice read", error))?
        else {
            return Ok(None);
        };
        let notice = Self::decode(&versioned.entry.body)?;
        // The path already partitions by owner; the field check is a
        // defense-in-depth guard against cross-scope row confusion.
        if notice.tenant_id != owner.tenant_id.as_str()
            || notice.owner_user_id != owner.user_id.as_str()
        {
            return Ok(None);
        }
        Ok(Some(notice))
    }

    /// One bounded CAS state transition. `apply` sees the current record and
    /// either produces the updated record, `Ok(None)` for an idempotent
    /// no-op (the caller re-reads the result), or a typed error.
    pub async fn transition<T, A>(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        apply: A,
    ) -> Result<(RunCompletionNotice, T), RunCompletionStoreError>
    where
        A: Fn(RunCompletionNotice) -> Result<(RunCompletionNotice, T), RunCompletionStoreError>
            + Send
            + Sync,
        T: Send + Clone,
    {
        let scope = owner.resource_scope();
        let path = Self::notice_path(notice_id)?;
        let owner_for_entry = owner.clone();
        let apply = Arc::new(apply);
        let result = cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            Self::decode,
            move |notice: &RunCompletionNotice| Self::notice_entry(&owner_for_entry, notice),
            move |current: Option<RunCompletionNotice>| {
                let apply = Arc::clone(&apply);
                async move {
                    let Some(notice) = current else {
                        return Err(RunCompletionStoreError::NotFound);
                    };
                    let (mut updated, outcome) = apply(notice)?;
                    updated.updated_at = Utc::now();
                    Ok(CasApply::new(updated.clone(), (updated, outcome)))
                }
            },
        )
        .await
        .map_err(flatten_cas_error("notice transition"))?;
        Ok(result)
    }

    /// Record (or replace) one browser profile's intent, bounded per §5.4.
    pub async fn record_intent(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        intent: CompletionIntentRecord,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        let (notice, ()) = self
            .transition(owner, notice_id, move |mut notice| {
                if notice.is_read() {
                    // Read settles arbitration; late intents are ignored
                    // idempotently rather than rejected.
                    return Ok((notice, ()));
                }
                notice
                    .intents
                    .retain(|existing| existing.browser_instance_id != intent.browser_instance_id);
                if notice.intents.len() >= RUN_COMPLETION_MAX_INTENTS_PER_NOTICE {
                    return Err(RunCompletionStoreError::Invalid {
                        reason: "intent budget for this notice is exhausted",
                    });
                }
                notice.intents.push(intent.clone());
                Ok((notice, ()))
            })
            .await?;
        Ok(notice)
    }

    /// Mark the notice read with evidence. Settles pending/granted delivery
    /// (§5.3: a read transition prevents future presentation but never
    /// deletes the completion fact).
    pub async fn mark_read(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        evidence: CompletionReadEvidence,
        read_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        let (notice, ()) = self
            .transition(owner, notice_id, move |mut notice| {
                if notice.is_read() {
                    return Ok((notice, ()));
                }
                notice.read = CompletionReadState::Read {
                    read_at,
                    evidence: evidence.clone(),
                };
                if matches!(
                    notice.delivery,
                    CompletionDeliveryState::PendingArbitration { .. }
                        | CompletionDeliveryState::Granted { .. }
                ) {
                    notice.delivery = CompletionDeliveryState::NoExternalTarget {
                        settled_at: read_at,
                    };
                }
                Ok((notice, ()))
            })
            .await?;
        Ok(notice)
    }

    /// Pending → Granted (§5.3). Fails `Conflict` from any other state so
    /// racing coordinators observe exactly one issued grant.
    #[allow(clippy::too_many_arguments)]
    pub async fn issue_grant(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        browser_instance_id: &str,
        surface: CompletionSurface,
        state_revision: u64,
        expires_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        let (notice, ()) = self
            .transition(owner, notice_id, move |mut notice| {
                let grants_issued = match &notice.delivery {
                    CompletionDeliveryState::PendingArbitration { grants_issued, .. } => {
                        *grants_issued
                    }
                    CompletionDeliveryState::Granted { grants_issued, .. } => *grants_issued,
                    _ => {
                        return Err(RunCompletionStoreError::Conflict {
                            reason: "grant requires a pending notice",
                        });
                    }
                };
                if notice.is_read() {
                    return Err(RunCompletionStoreError::Conflict {
                        reason: "notice already read",
                    });
                }
                if matches!(notice.delivery, CompletionDeliveryState::Granted { .. }) {
                    return Err(RunCompletionStoreError::Conflict {
                        reason: "a grant is already outstanding",
                    });
                }
                notice.delivery = CompletionDeliveryState::Granted {
                    grant_id: grant_id.to_string(),
                    browser_instance_id: browser_instance_id.to_string(),
                    surface,
                    state_revision,
                    expires_at,
                    grants_issued: grants_issued.saturating_add(1),
                };
                Ok((notice, ()))
            })
            .await?;
        Ok(notice)
    }

    /// Granted → Presented on a matching acknowledgement (§5.3).
    pub async fn acknowledge_presented(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        presented_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        let (notice, ()) = self
            .transition(owner, notice_id, move |mut notice| {
                match &notice.delivery {
                    CompletionDeliveryState::Granted {
                        grant_id: outstanding,
                        surface,
                        ..
                    } if outstanding == grant_id => {
                        notice.delivery = CompletionDeliveryState::Presented {
                            surface: *surface,
                            presented_at,
                        };
                        Ok((notice, ()))
                    }
                    CompletionDeliveryState::Presented { .. } => Ok((notice, ())),
                    _ => Err(RunCompletionStoreError::Conflict {
                        reason: "acknowledgement does not match the outstanding grant",
                    }),
                }
            })
            .await?;
        Ok(notice)
    }

    /// Granted → PendingArbitration after grant expiry or a stale-state
    /// rejection; carries the re-arbitration deadline (§5.4).
    pub async fn regress_expired_grant(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        closes_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        let (notice, ()) = self
            .transition(owner, notice_id, move |mut notice| {
                match &notice.delivery {
                    CompletionDeliveryState::Granted {
                        grant_id: outstanding,
                        grants_issued,
                        ..
                    } if outstanding == grant_id => {
                        let grants_issued = *grants_issued;
                        notice.delivery = CompletionDeliveryState::PendingArbitration {
                            closes_at,
                            grants_issued,
                        };
                        Ok((notice, ()))
                    }
                    CompletionDeliveryState::PendingArbitration { .. } => Ok((notice, ())),
                    _ => Err(RunCompletionStoreError::Conflict {
                        reason: "grant regression does not match the outstanding grant",
                    }),
                }
            })
            .await?;
        Ok(notice)
    }

    /// Pending → PushOwned. Only one replica can win this CAS (§5.3).
    pub async fn claim_push(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        delivery_id: &str,
        claimed_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        let (notice, ()) = self
            .transition(owner, notice_id, move |mut notice| {
                if notice.is_read() {
                    return Err(RunCompletionStoreError::Conflict {
                        reason: "notice already read",
                    });
                }
                match &notice.delivery {
                    CompletionDeliveryState::PendingArbitration { .. } => {
                        notice.delivery = CompletionDeliveryState::PushOwned {
                            delivery_id: delivery_id.to_string(),
                            claimed_at,
                        };
                        Ok((notice, ()))
                    }
                    CompletionDeliveryState::PushOwned {
                        delivery_id: existing,
                        ..
                    } if existing == delivery_id => Ok((notice, ())),
                    _ => Err(RunCompletionStoreError::Conflict {
                        reason: "push ownership requires a pending notice",
                    }),
                }
            })
            .await?;
        Ok(notice)
    }

    /// Pending → NoExternalTarget when no browser responded and no push
    /// target exists (§5.3).
    pub async fn settle_no_target(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        settled_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        let (notice, ()) = self
            .transition(owner, notice_id, move |mut notice| {
                match &notice.delivery {
                    CompletionDeliveryState::PendingArbitration { .. } => {
                        notice.delivery =
                            CompletionDeliveryState::NoExternalTarget { settled_at };
                        Ok((notice, ()))
                    }
                    CompletionDeliveryState::NoExternalTarget { .. } => Ok((notice, ())),
                    _ => Err(RunCompletionStoreError::Conflict {
                        reason: "no-target settlement requires a pending notice",
                    }),
                }
            })
            .await?;
        Ok(notice)
    }

    async fn query_partition(
        &self,
        owner: &RunCompletionOwner,
        index: &str,
        partition_key: &str,
        partition_value: String,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        let scope = owner.resource_scope();
        let root = Self::alias_root()?;
        let limit = limit.clamp(1, RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT.max(1));
        let mut page = OrderedPage::new(
            Self::index_name(index)?,
            Self::index_key(SEQUENCE_SORT_KEY)?,
            Self::index_key(NOTICE_ID_KEY)?,
            SortDirection::Ascending,
            u32::try_from(limit).unwrap_or(u32::MAX),
        );
        if let Some(after) = after_sequence {
            // Sequences are unique per owner (CAS-allocated), so resuming
            // strictly after one means excluding its own row: the tie-break
            // sentinel `~` (0x7E) sorts above every notice id byte.
            page = page.after(ironclaw_filesystem::OrderedQueryCursor {
                value: IndexValue::Text(Self::sequence_sort(after)),
                tie_breaker: IndexValue::Text("~".to_string()),
            });
        }
        let rows = self
            .filesystem
            .query_ordered(
                &scope,
                &root,
                &Filter::Eq {
                    key: Self::index_key(partition_key)?,
                    value: IndexValue::Text(partition_value),
                },
                &page,
            )
            .await
            .map_err(|error| RunCompletionStoreError::backend("notice query", error))?;
        rows.into_iter()
            .map(|row| Self::decode(&row.entry.body))
            .collect()
    }

    /// Replay: every notice after `sequence`, oldest first, bounded.
    pub async fn list_after(
        &self,
        owner: &RunCompletionOwner,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        self.query_partition(
            owner,
            ORDER_INDEX,
            OWNER_KEY,
            owner.owner_key(),
            after_sequence,
            limit,
        )
        .await
    }

    /// The bounded unread snapshot (§5.4): oldest first, at most 250.
    pub async fn unread_snapshot(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        self.query_partition(
            owner,
            UNREAD_INDEX,
            UNREAD_PARTITION_KEY,
            Self::unread_partition(owner, true),
            None,
            RUN_COMPLETION_UNREAD_SNAPSHOT_LIMIT,
        )
        .await
    }

    /// Unread notices for one thread, oldest first, bounded.
    pub async fn unread_for_thread(
        &self,
        owner: &RunCompletionOwner,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        self.query_partition(
            owner,
            THREAD_INDEX,
            THREAD_PARTITION_KEY,
            Self::thread_partition(owner, thread_id, true),
            None,
            limit,
        )
        .await
    }

    /// Boot reconciliation scan: notices in one delivery state, oldest
    /// first, bounded by the active workload (non-terminal states only).
    pub async fn in_delivery_state(
        &self,
        owner: &RunCompletionOwner,
        state: &CompletionDeliveryState,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        self.query_partition(
            owner,
            STATE_INDEX,
            STATE_PARTITION_KEY,
            Self::state_partition(owner, state),
            None,
            limit,
        )
        .await
    }
}

/// Object-safe notice operations the product surface, coordinator, ingest,
/// and stream hub consume. Implemented by [`RunCompletionNoticeStore`];
/// composition erases the concrete filesystem type at construction.
#[async_trait::async_trait]
pub trait RunCompletionNotices: Send + Sync {
    async fn create_notice(
        &self,
        owner: &RunCompletionOwner,
        new_notice: NewRunCompletionNotice,
    ) -> Result<NoticeCreateOutcome, RunCompletionStoreError>;

    async fn get(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
    ) -> Result<Option<RunCompletionNotice>, RunCompletionStoreError>;

    async fn record_intent(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        intent: CompletionIntentRecord,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError>;

    async fn mark_read(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        evidence: CompletionReadEvidence,
        read_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError>;

    #[allow(clippy::too_many_arguments)]
    async fn issue_grant(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        browser_instance_id: &str,
        surface: CompletionSurface,
        state_revision: u64,
        expires_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError>;

    async fn acknowledge_presented(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        presented_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError>;

    async fn regress_expired_grant(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        closes_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError>;

    async fn claim_push(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        delivery_id: &str,
        claimed_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError>;

    async fn settle_no_target(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        settled_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError>;

    async fn list_after(
        &self,
        owner: &RunCompletionOwner,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError>;

    async fn unread_snapshot(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError>;

    async fn unread_for_thread(
        &self,
        owner: &RunCompletionOwner,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError>;

    async fn in_delivery_state(
        &self,
        owner: &RunCompletionOwner,
        state: &CompletionDeliveryState,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError>;

    /// Record `owner` in the per-tenant durable due registry (§5.4).
    async fn mark_owner_due(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<(), RunCompletionStoreError>;

    /// Remove `owner` from the due registry after an empty scan.
    async fn clear_owner_due(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<(), RunCompletionStoreError>;

    /// Owners with potentially due work in `scope_owner`'s tenant, for the
    /// coordinator's one bounded boot reconciliation (§5.4).
    async fn due_owners(
        &self,
        scope_owner: &RunCompletionOwner,
    ) -> Result<Vec<RunCompletionOwner>, RunCompletionStoreError>;
}

#[async_trait::async_trait]
impl<F> RunCompletionNotices for RunCompletionNoticeStore<F>
where
    F: RootFilesystem + ?Sized,
{
    async fn create_notice(
        &self,
        owner: &RunCompletionOwner,
        new_notice: NewRunCompletionNotice,
    ) -> Result<NoticeCreateOutcome, RunCompletionStoreError> {
        RunCompletionNoticeStore::create_notice(self, owner, new_notice).await
    }

    async fn get(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
    ) -> Result<Option<RunCompletionNotice>, RunCompletionStoreError> {
        RunCompletionNoticeStore::get(self, owner, notice_id).await
    }

    async fn record_intent(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        intent: CompletionIntentRecord,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        RunCompletionNoticeStore::record_intent(self, owner, notice_id, intent).await
    }

    async fn mark_read(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        evidence: CompletionReadEvidence,
        read_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        RunCompletionNoticeStore::mark_read(self, owner, notice_id, evidence, read_at).await
    }

    async fn issue_grant(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        browser_instance_id: &str,
        surface: CompletionSurface,
        state_revision: u64,
        expires_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        RunCompletionNoticeStore::issue_grant(
            self,
            owner,
            notice_id,
            grant_id,
            browser_instance_id,
            surface,
            state_revision,
            expires_at,
        )
        .await
    }

    async fn acknowledge_presented(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        presented_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        RunCompletionNoticeStore::acknowledge_presented(
            self,
            owner,
            notice_id,
            grant_id,
            presented_at,
        )
        .await
    }

    async fn regress_expired_grant(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        grant_id: &str,
        closes_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        RunCompletionNoticeStore::regress_expired_grant(self, owner, notice_id, grant_id, closes_at)
            .await
    }

    async fn claim_push(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        delivery_id: &str,
        claimed_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        RunCompletionNoticeStore::claim_push(self, owner, notice_id, delivery_id, claimed_at).await
    }

    async fn settle_no_target(
        &self,
        owner: &RunCompletionOwner,
        notice_id: &str,
        settled_at: DateTime<Utc>,
    ) -> Result<RunCompletionNotice, RunCompletionStoreError> {
        RunCompletionNoticeStore::settle_no_target(self, owner, notice_id, settled_at).await
    }

    async fn list_after(
        &self,
        owner: &RunCompletionOwner,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        RunCompletionNoticeStore::list_after(self, owner, after_sequence, limit).await
    }

    async fn unread_snapshot(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        RunCompletionNoticeStore::unread_snapshot(self, owner).await
    }

    async fn unread_for_thread(
        &self,
        owner: &RunCompletionOwner,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        RunCompletionNoticeStore::unread_for_thread(self, owner, thread_id, limit).await
    }

    async fn in_delivery_state(
        &self,
        owner: &RunCompletionOwner,
        state: &CompletionDeliveryState,
        limit: usize,
    ) -> Result<Vec<RunCompletionNotice>, RunCompletionStoreError> {
        RunCompletionNoticeStore::in_delivery_state(self, owner, state, limit).await
    }

    async fn mark_owner_due(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<(), RunCompletionStoreError> {
        RunCompletionNoticeStore::mark_owner_due(self, owner).await
    }

    async fn clear_owner_due(
        &self,
        owner: &RunCompletionOwner,
    ) -> Result<(), RunCompletionStoreError> {
        RunCompletionNoticeStore::clear_owner_due(self, owner).await
    }

    async fn due_owners(
        &self,
        scope_owner: &RunCompletionOwner,
    ) -> Result<Vec<RunCompletionOwner>, RunCompletionStoreError> {
        RunCompletionNoticeStore::due_owners(self, scope_owner).await
    }
}

fn flatten_cas_error(
    operation: &'static str,
) -> impl Fn(CasUpdateError<RunCompletionStoreError>) -> RunCompletionStoreError {
    move |error| match error {
        CasUpdateError::Apply(inner) => inner,
        other => RunCompletionStoreError::backend(operation, other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
    use ironclaw_host_api::path::{MountAlias, VirtualPath};

    fn store() -> RunCompletionNoticeStore<InMemoryBackend> {
        RunCompletionNoticeStore::new(Arc::new(ScopedFilesystem::new(
            Arc::new(InMemoryBackend::new()),
            |scope: &ResourceScope| {
                MountView::new(vec![MountGrant::new(
                    MountAlias::new(RUN_NOTICES_MOUNT_ALIAS)?,
                    VirtualPath::new(format!(
                        "/tenants/{}/users/{}/run-notices",
                        scope.tenant_id, scope.user_id
                    ))?,
                    MountPermissions::read_write_list_delete(),
                )])
            },
        )))
    }

    fn owner(user: &str) -> RunCompletionOwner {
        RunCompletionOwner {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new(user).expect("user"),
        }
    }

    fn new_notice(suffix: &str) -> NewRunCompletionNotice {
        NewRunCompletionNotice {
            notice_id: format!("rcn-{suffix}"),
            run_id: format!("run-{suffix}"),
            thread_id: format!("thread-{suffix}"),
            agent_id: Some("agent-alpha".to_string()),
            project_id: None,
            thread_tag: format!("rct-{suffix}"),
            terminal_projection_ref: format!("run-completion/rcn-{suffix}"),
            completed_at: Utc::now(),
            arbitration_closes_at: Utc::now() + ChronoDuration::seconds(1),
        }
    }

    #[tokio::test]
    async fn create_is_idempotent_and_sequences_are_monotonic() {
        let store = store();
        let owner = owner("user-a");

        let first = store
            .create_notice(&owner, new_notice("a"))
            .await
            .expect("first create");
        let NoticeCreateOutcome::Created(first_notice) = first else {
            panic!("first write must create");
        };
        let second = store
            .create_notice(&owner, new_notice("b"))
            .await
            .expect("second create");
        let NoticeCreateOutcome::Created(second_notice) = second else {
            panic!("second write must create");
        };
        assert!(
            second_notice.sequence > first_notice.sequence,
            "the owner sequence is monotonic",
        );

        let replay = store
            .create_notice(&owner, new_notice("a"))
            .await
            .expect("duplicate create");
        let NoticeCreateOutcome::AlreadyRecorded(replayed) = replay else {
            panic!("duplicate journal delivery must rewrite nothing");
        };
        assert_eq!(replayed, first_notice, "the immutable fact is unchanged");
    }

    #[tokio::test]
    async fn read_settles_pending_delivery_but_keeps_the_fact() {
        let store = store();
        let owner = owner("user-a");
        let NoticeCreateOutcome::Created(notice) = store
            .create_notice(&owner, new_notice("a"))
            .await
            .expect("create")
        else {
            panic!("create");
        };

        let read = store
            .mark_read(
                &owner,
                &notice.notice_id,
                CompletionReadEvidence::ReplyRendered {
                    browser_instance_id: "browser-1".to_string(),
                },
                Utc::now(),
            )
            .await
            .expect("read transition");
        assert!(read.is_read());
        assert!(matches!(
            read.delivery,
            CompletionDeliveryState::NoExternalTarget { .. }
        ));
        // Read is not deletion: the record remains durable and queryable.
        let reread = store
            .get(&owner, &notice.notice_id)
            .await
            .expect("get")
            .expect("record retained");
        assert!(reread.is_read());
        assert!(
            store
                .unread_snapshot(&owner)
                .await
                .expect("snapshot")
                .is_empty(),
            "read notices leave the unread snapshot",
        );
    }

    #[tokio::test]
    async fn grant_lifecycle_transitions_are_state_checked() {
        let store = store();
        let owner = owner("user-a");
        let NoticeCreateOutcome::Created(notice) = store
            .create_notice(&owner, new_notice("a"))
            .await
            .expect("create")
        else {
            panic!("create");
        };

        let expires = Utc::now() + ChronoDuration::seconds(2);
        store
            .issue_grant(
                &owner,
                &notice.notice_id,
                "grant-1",
                "browser-1",
                CompletionSurface::InApp,
                41,
                expires,
            )
            .await
            .expect("grant issues from pending");
        let double_grant = store
            .issue_grant(
                &owner,
                &notice.notice_id,
                "grant-2",
                "browser-2",
                CompletionSurface::InApp,
                42,
                expires,
            )
            .await;
        assert!(
            matches!(double_grant, Err(RunCompletionStoreError::Conflict { .. })),
            "a second concurrent grant must lose the CAS: {double_grant:?}",
        );

        let mismatched_ack = store
            .acknowledge_presented(&owner, &notice.notice_id, "grant-2", Utc::now())
            .await;
        assert!(matches!(
            mismatched_ack,
            Err(RunCompletionStoreError::Conflict { .. })
        ));
        let presented = store
            .acknowledge_presented(&owner, &notice.notice_id, "grant-1", Utc::now())
            .await
            .expect("matching acknowledgement");
        assert!(matches!(
            presented.delivery,
            CompletionDeliveryState::Presented {
                surface: CompletionSurface::InApp,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn push_ownership_has_exactly_one_winner() {
        let store = Arc::new(store());
        let owner = owner("user-a");
        let NoticeCreateOutcome::Created(notice) = store
            .create_notice(&owner, new_notice("a"))
            .await
            .expect("create")
        else {
            panic!("create");
        };

        let mut winners = 0;
        let mut tasks = tokio::task::JoinSet::new();
        for attempt in 0..8 {
            let store = Arc::clone(&store);
            let owner = owner.clone();
            let notice_id = notice.notice_id.clone();
            tasks.spawn(async move {
                store
                    .claim_push(
                        &owner,
                        &notice_id,
                        &format!("delivery-{attempt}"),
                        Utc::now(),
                    )
                    .await
                    .is_ok()
            });
        }
        while let Some(result) = tasks.join_next().await {
            if result.expect("task joins") {
                winners += 1;
            }
        }
        assert_eq!(winners, 1, "only one replica may own the push CAS");
    }

    #[tokio::test]
    async fn listing_queries_are_owner_partitioned_and_ordered() {
        let store = store();
        let owner_a = owner("user-a");
        let owner_b = owner("user-b");
        for suffix in ["a1", "a2", "a3"] {
            store
                .create_notice(&owner_a, new_notice(suffix))
                .await
                .expect("create");
        }
        store
            .create_notice(&owner_b, new_notice("b1"))
            .await
            .expect("create");

        let all_a = store.list_after(&owner_a, None, 250).await.expect("list");
        assert_eq!(all_a.len(), 3, "owner A sees exactly their notices");
        assert!(
            all_a.windows(2).all(|w| w[0].sequence < w[1].sequence),
            "replay is oldest-first",
        );
        let after = store
            .list_after(&owner_a, Some(all_a[0].sequence), 250)
            .await
            .expect("list after");
        assert_eq!(after.len(), 2, "resume excludes the cursor position");

        let unread_b = store.unread_snapshot(&owner_b).await.expect("snapshot");
        assert_eq!(unread_b.len(), 1);
        assert_eq!(unread_b[0].run_id, "run-b1");

        let thread_unread = store
            .unread_for_thread(&owner_a, "thread-a2", 99)
            .await
            .expect("thread unread");
        assert_eq!(thread_unread.len(), 1);

        let pending = store
            .in_delivery_state(
                &owner_a,
                &CompletionDeliveryState::PendingArbitration {
                    closes_at: Utc::now(),
                    grants_issued: 0,
                },
                250,
            )
            .await
            .expect("state scan");
        assert_eq!(pending.len(), 3, "boot reconciliation sees pending work");
    }

    #[tokio::test]
    async fn intents_replace_per_browser_and_stay_bounded() {
        let store = store();
        let owner = owner("user-a");
        let NoticeCreateOutcome::Created(notice) = store
            .create_notice(&owner, new_notice("a"))
            .await
            .expect("create")
        else {
            panic!("create");
        };
        let intent = |browser: &str, revision: u64| CompletionIntentRecord {
            browser_instance_id: browser.to_string(),
            tab_id: "tab-1".to_string(),
            state_revision: revision,
            focus_epoch: 1,
            intent:
                ironclaw_product_contracts::run_completions::RunCompletionIntentKind::InApp,
            offered_at: Utc::now(),
        };

        store
            .record_intent(&owner, &notice.notice_id, intent("browser-1", 1))
            .await
            .expect("first intent");
        let updated = store
            .record_intent(&owner, &notice.notice_id, intent("browser-1", 2))
            .await
            .expect("replacement intent");
        assert_eq!(
            updated.intents.len(),
            1,
            "a newer revision replaces the same profile's intent",
        );
        assert_eq!(updated.intents[0].state_revision, 2);

        for extra in 0..(RUN_COMPLETION_MAX_INTENTS_PER_NOTICE - 1) {
            store
                .record_intent(
                    &owner,
                    &notice.notice_id,
                    intent(&format!("browser-extra-{extra}"), 1),
                )
                .await
                .expect("intent within budget");
        }
        let overflow = store
            .record_intent(&owner, &notice.notice_id, intent("browser-overflow", 1))
            .await;
        assert!(
            matches!(overflow, Err(RunCompletionStoreError::Invalid { .. })),
            "the per-notice intent budget is enforced: {overflow:?}",
        );
    }
}
