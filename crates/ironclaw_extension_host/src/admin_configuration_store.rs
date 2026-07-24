//! Tenant-scoped durable state for manifest-declared admin configuration.
//!
//! The manifest descriptor remains in `ironclaw_extensions`; this module owns
//! only the host-side configured values and their visibility transition. A
//! save is deliberately two phase:
//!
//! 1. reserve a monotonically increasing revision using bounded CAS;
//! 2. stage any secret material under handles derived from that revision;
//! 3. atomically swap the active record to those handles with another CAS.
//!
//! A crash before step 3 leaves the previous active record intact. Retrying
//! with the same idempotency key returns the same reservation, so the caller
//! can safely restage and finish. Completed receipts are retained in the
//! record and replay unchanged even after later revisions commit.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ironclaw_extensions::AdminConfigurationGroupId;
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FilesystemError, RecordKind,
    RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{ResourceScope, ScopedPath, SecretHandle, TenantId};
use serde::{Deserialize, Serialize};

const ADMIN_CONFIGURATION_RECORD_KIND: &str = "extension_admin_configuration";
const ADMIN_CONFIGURATION_PREFIX: &str = "/extension-admin-configuration/groups";
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;
const MAX_IDEMPOTENCY_RECORDS: usize = 1_024;
const MAX_RECORD_BYTES: usize = 1024 * 1024;
const PENDING_RETENTION_HOURS: i64 = 24;
const REPLAY_RETENTION_DAYS: i64 = 7;

/// Stable client-supplied identity for one configuration mutation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdminConfigurationIdempotencyKey(String);

impl AdminConfigurationIdempotencyKey {
    pub fn new(value: impl Into<String>) -> Result<Self, AdminConfigurationStoreError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_IDEMPOTENCY_KEY_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(AdminConfigurationStoreError::InvalidIdempotencyKey);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical digest of the validated mutation input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AdminConfigurationRequestDigest([u8; 32]);

impl AdminConfigurationRequestDigest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// One effective value in an active admin-configuration record.
///
/// Secret material is never stored here. `Secret` carries only a staged,
/// revision-specific host secret handle. Non-secret configuration stays
/// inline so the operator read model can return it without leasing secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "storage", content = "value", rename_all = "snake_case")]
pub enum AdminConfigurationValueRef {
    Inline(String),
    Secret(SecretHandle),
}

/// Redacted active state for one manifest-declared configuration group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminConfigurationRecord {
    pub tenant_id: TenantId,
    pub group_id: AdminConfigurationGroupId,
    pub revision: u64,
    pub values: BTreeMap<SecretHandle, AdminConfigurationValueRef>,
}

/// Durable result of one completed mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminConfigurationCommit {
    pub revision: u64,
    pub values: BTreeMap<SecretHandle, AdminConfigurationValueRef>,
}

/// Durable reservation allocated before secret staging begins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminConfigurationReservation {
    pub revision: u64,
    pub expected_revision: u64,
    tenant_id: TenantId,
    group_id: AdminConfigurationGroupId,
    idempotency_key: AdminConfigurationIdempotencyKey,
    request_digest: AdminConfigurationRequestDigest,
}

impl AdminConfigurationReservation {
    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn group_id(&self) -> &AdminConfigurationGroupId {
        &self.group_id
    }

    pub fn idempotency_key(&self) -> &AdminConfigurationIdempotencyKey {
        &self.idempotency_key
    }

    pub fn request_digest(&self) -> AdminConfigurationRequestDigest {
        self.request_digest
    }
}

/// Result of reserving a mutation identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminConfigurationReserveOutcome {
    Reserved(AdminConfigurationReservation),
    Replay(AdminConfigurationCommit),
}

enum StoredReserveOutcome {
    Reserved(AdminConfigurationReservation),
    ReplayRevision(u64),
}

struct ReserveCasOutcome {
    outcome: StoredReserveOutcome,
    retired_revisions: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdminConfigurationStoreError {
    #[error("invalid admin-configuration idempotency key")]
    InvalidIdempotencyKey,
    #[error("admin-configuration idempotency key was reused with different input")]
    IdempotencyConflict,
    #[error("admin-configuration idempotency capacity is exhausted")]
    IdempotencyCapacityExhausted,
    #[error("admin-configuration reservation is unknown")]
    UnknownReservation,
    #[error("admin-configuration reservation was superseded")]
    StaleReservation,
    #[error("admin-configuration revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("admin-configuration record is invalid")]
    InvalidRecord,
    #[error("admin-configuration store requires compare-and-swap")]
    CasUnsupported,
    #[error("admin-configuration store is contended; retry the request")]
    Contended,
    #[error("admin-configuration store is unavailable")]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredReservation {
    revision: u64,
    expected_revision: u64,
    request_digest: AdminConfigurationRequestDigest,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredReplay {
    request_digest: AdminConfigurationRequestDigest,
    revision: u64,
    completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredRevisionSnapshot {
    tenant_id: TenantId,
    group_id: AdminConfigurationGroupId,
    commit: AdminConfigurationCommit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredAdminConfigurationRecord {
    tenant_id: TenantId,
    group_id: AdminConfigurationGroupId,
    active_revision: u64,
    next_revision: u64,
    values: BTreeMap<SecretHandle, AdminConfigurationValueRef>,
    pending: BTreeMap<String, StoredReservation>,
    replays: BTreeMap<String, StoredReplay>,
    /// Revision snapshots whose replay receipts have expired. Entries stay in
    /// this durable cleanup queue until deletion succeeds and is acknowledged.
    #[serde(default)]
    retired_revisions: BTreeSet<u64>,
}

impl StoredAdminConfigurationRecord {
    fn new(tenant_id: TenantId, group_id: AdminConfigurationGroupId) -> Self {
        Self {
            tenant_id,
            group_id,
            active_revision: 0,
            next_revision: 1,
            values: BTreeMap::new(),
            pending: BTreeMap::new(),
            replays: BTreeMap::new(),
            retired_revisions: BTreeSet::new(),
        }
    }

    fn public_record(&self) -> AdminConfigurationRecord {
        AdminConfigurationRecord {
            tenant_id: self.tenant_id.clone(),
            group_id: self.group_id.clone(),
            revision: self.active_revision,
            values: self.values.clone(),
        }
    }

    fn idempotency_count(&self) -> usize {
        self.pending.len().saturating_add(self.replays.len())
    }
}

/// Filesystem-backed tenant group store over an already-scoped filesystem.
///
/// The caller owns mount selection. Production supplies a tenant-rewriting
/// `/extension-admin-configuration` alias; tests may use a fixed view. The
/// record still carries `tenant_id` as defense in depth, and a mismatch reads
/// as absent rather than disclosing another tenant's state.
pub struct FilesystemAdminConfigurationStore<F>
where
    F: RootFilesystem + ?Sized,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemAdminConfigurationStore<F>
where
    F: RootFilesystem + ?Sized,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    pub async fn get(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
    ) -> Result<Option<AdminConfigurationRecord>, AdminConfigurationStoreError> {
        let path = group_path(group_id)?;
        let Some(versioned) = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(|error| map_backend_error("get", &error))?
        else {
            return Ok(None);
        };
        let record = decode_record(&versioned.entry.body)?;
        if record.tenant_id != scope.tenant_id || record.group_id != *group_id {
            return Ok(None);
        }
        Ok(Some(record.public_record()))
    }

    /// Reserve a revision before staging secrets.
    ///
    /// An exact completed replay returns its original receipt. An interrupted
    /// request returns its existing non-stale reservation. If a later revision
    /// already committed, retrying the interrupted request allocates a new
    /// revision rather than allowing it to roll back active state.
    pub async fn reserve(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        idempotency_key: &AdminConfigurationIdempotencyKey,
        request_digest: AdminConfigurationRequestDigest,
        expected_revision: u64,
    ) -> Result<AdminConfigurationReserveOutcome, AdminConfigurationStoreError> {
        self.reserve_at(
            scope,
            group_id,
            idempotency_key,
            request_digest,
            expected_revision,
            Utc::now(),
        )
        .await
    }

    async fn reserve_at(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        idempotency_key: &AdminConfigurationIdempotencyKey,
        request_digest: AdminConfigurationRequestDigest,
        expected_revision: u64,
        now: DateTime<Utc>,
    ) -> Result<AdminConfigurationReserveOutcome, AdminConfigurationStoreError> {
        let path = group_path(group_id)?;
        let tenant_id = scope.tenant_id.clone();
        let group_id = group_id.clone();
        let key = idempotency_key.as_str().to_string();
        let cas_outcome = cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            decode_record,
            encode_record,
            |current: Option<StoredAdminConfigurationRecord>| {
                let mut record = current.unwrap_or_else(|| {
                    StoredAdminConfigurationRecord::new(tenant_id.clone(), group_id.clone())
                });
                let outcome = (|| {
                    ensure_record_owner(&record, &tenant_id, &group_id)?;
                    prune_expired_idempotency(&mut record, now)?;
                    if let Some(replay) = record.replays.get(&key) {
                        if replay.request_digest != request_digest {
                            return Err(AdminConfigurationStoreError::IdempotencyConflict);
                        }
                        return Ok(CasApply::new(
                            record.clone(),
                            ReserveCasOutcome {
                                outcome: StoredReserveOutcome::ReplayRevision(replay.revision),
                                retired_revisions: record
                                    .retired_revisions
                                    .iter()
                                    .copied()
                                    .collect(),
                            },
                        ));
                    }
                    if let Some(pending) = record.pending.get(&key) {
                        if pending.request_digest != request_digest {
                            return Err(AdminConfigurationStoreError::IdempotencyConflict);
                        }
                        if pending.expected_revision == record.active_revision
                            && pending.expected_revision == expected_revision
                        {
                            let reservation = reservation_from(
                                &record,
                                idempotency_key.clone(),
                                request_digest,
                                pending.revision,
                                pending.expected_revision,
                            );
                            return Ok(CasApply::new(
                                record.clone(),
                                ReserveCasOutcome {
                                    outcome: StoredReserveOutcome::Reserved(reservation),
                                    retired_revisions: record
                                        .retired_revisions
                                        .iter()
                                        .copied()
                                        .collect(),
                                },
                            ));
                        }
                    }
                    if record.active_revision != expected_revision {
                        return Err(AdminConfigurationStoreError::RevisionConflict {
                            expected: expected_revision,
                            actual: record.active_revision,
                        });
                    }
                    if record.idempotency_count() >= MAX_IDEMPOTENCY_RECORDS
                        && !record.pending.contains_key(&key)
                    {
                        return Err(AdminConfigurationStoreError::IdempotencyCapacityExhausted);
                    }
                    let revision = record.next_revision;
                    record.next_revision = record
                        .next_revision
                        .checked_add(1)
                        .ok_or(AdminConfigurationStoreError::InvalidRecord)?;
                    record.pending.insert(
                        key.clone(),
                        StoredReservation {
                            revision,
                            expected_revision,
                            request_digest,
                            created_at: now,
                        },
                    );
                    let reservation = reservation_from(
                        &record,
                        idempotency_key.clone(),
                        request_digest,
                        revision,
                        expected_revision,
                    );
                    let retired_revisions: Vec<u64> =
                        record.retired_revisions.iter().copied().collect();
                    Ok(CasApply::new(
                        record,
                        ReserveCasOutcome {
                            outcome: StoredReserveOutcome::Reserved(reservation),
                            retired_revisions,
                        },
                    ))
                })();
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)?;
        self.cleanup_retired_revisions(scope, &group_id, &cas_outcome.retired_revisions)
            .await;
        match cas_outcome.outcome {
            StoredReserveOutcome::Reserved(reservation) => {
                Ok(AdminConfigurationReserveOutcome::Reserved(reservation))
            }
            StoredReserveOutcome::ReplayRevision(revision) => self
                .load_revision_snapshot(scope, &group_id, revision)
                .await
                .map(AdminConfigurationReserveOutcome::Replay),
        }
    }

    /// Atomically make previously staged value references active.
    pub async fn commit(
        &self,
        scope: &ResourceScope,
        reservation: &AdminConfigurationReservation,
        values: BTreeMap<SecretHandle, AdminConfigurationValueRef>,
    ) -> Result<AdminConfigurationCommit, AdminConfigurationStoreError> {
        self.commit_at(scope, reservation, values, Utc::now()).await
    }

    async fn commit_at(
        &self,
        scope: &ResourceScope,
        reservation: &AdminConfigurationReservation,
        values: BTreeMap<SecretHandle, AdminConfigurationValueRef>,
        now: DateTime<Utc>,
    ) -> Result<AdminConfigurationCommit, AdminConfigurationStoreError> {
        if reservation.tenant_id != scope.tenant_id {
            return Err(AdminConfigurationStoreError::UnknownReservation);
        }
        let staged_commit = AdminConfigurationCommit {
            revision: reservation.revision,
            values: values.clone(),
        };
        self.stage_revision_snapshot(
            scope,
            &reservation.group_id,
            &reservation.tenant_id,
            &staged_commit,
        )
        .await?;
        let path = group_path(&reservation.group_id)?;
        let tenant_id = scope.tenant_id.clone();
        let group_id = reservation.group_id.clone();
        let key = reservation.idempotency_key.as_str().to_string();
        let requested_revision = reservation.revision;
        let expected_revision = reservation.expected_revision;
        let request_digest = reservation.request_digest;
        let result = cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            decode_record,
            encode_record,
            |current: Option<StoredAdminConfigurationRecord>| {
                let outcome = (|| {
                    let mut record =
                        current.ok_or(AdminConfigurationStoreError::UnknownReservation)?;
                    ensure_record_owner(&record, &tenant_id, &group_id)?;
                    if let Some(replay) = record.replays.get(&key) {
                        if replay.request_digest != request_digest {
                            return Err(AdminConfigurationStoreError::IdempotencyConflict);
                        }
                        return Ok(CasApply::no_op(record.clone(), replay.revision));
                    }
                    let pending = record
                        .pending
                        .get(&key)
                        .ok_or(AdminConfigurationStoreError::UnknownReservation)?;
                    if pending.request_digest != request_digest
                        || pending.revision != requested_revision
                        || pending.expected_revision != expected_revision
                    {
                        return Err(AdminConfigurationStoreError::UnknownReservation);
                    }
                    if record.active_revision != expected_revision {
                        return Err(AdminConfigurationStoreError::RevisionConflict {
                            expected: expected_revision,
                            actual: record.active_revision,
                        });
                    }
                    record.active_revision = requested_revision;
                    record.values = values.clone();
                    record.pending.remove(&key);
                    record.replays.insert(
                        key.clone(),
                        StoredReplay {
                            request_digest,
                            revision: requested_revision,
                            completed_at: now,
                        },
                    );
                    Ok(CasApply::new(record, requested_revision))
                })();
                async move { outcome }
            },
        )
        .await;
        let published_revision = match result {
            Ok(revision) => revision,
            Err(error) => {
                let mapped = map_cas_error(error);
                self.cleanup_unpublished_snapshot(scope, &group_id, requested_revision)
                    .await;
                return Err(mapped);
            }
        };
        self.load_revision_snapshot(scope, &group_id, published_revision)
            .await
    }

    /// Roll back exactly the revision published by `reservation` to the
    /// record that was active when that reservation was created.
    ///
    /// The active-revision comparison and replay-receipt removal happen in
    /// the same CAS update. A later writer therefore fences this rollback:
    /// once another revision is active, this method returns
    /// [`AdminConfigurationStoreError::StaleReservation`] without changing
    /// its values. Removing the failed request's replay receipt lets the
    /// caller retry the same idempotency key against the restored revision.
    pub async fn rollback_commit(
        &self,
        scope: &ResourceScope,
        reservation: &AdminConfigurationReservation,
        previous: Option<&AdminConfigurationRecord>,
    ) -> Result<(), AdminConfigurationStoreError> {
        if reservation.tenant_id != scope.tenant_id {
            return Err(AdminConfigurationStoreError::UnknownReservation);
        }
        let (previous_revision, previous_values) = match previous {
            Some(previous)
                if previous.tenant_id == scope.tenant_id
                    && previous.group_id == reservation.group_id
                    && previous.revision == reservation.expected_revision =>
            {
                (previous.revision, previous.values.clone())
            }
            None if reservation.expected_revision == 0 => (0, BTreeMap::new()),
            _ => return Err(AdminConfigurationStoreError::UnknownReservation),
        };
        let path = group_path(&reservation.group_id)?;
        let tenant_id = scope.tenant_id.clone();
        let group_id = reservation.group_id.clone();
        let key = reservation.idempotency_key.as_str().to_string();
        let request_digest = reservation.request_digest;
        let failed_revision = reservation.revision;
        let retired_revisions = cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            decode_record,
            encode_record,
            |current: Option<StoredAdminConfigurationRecord>| {
                let outcome = (|| {
                    let mut record =
                        current.ok_or(AdminConfigurationStoreError::UnknownReservation)?;
                    ensure_record_owner(&record, &tenant_id, &group_id)?;
                    let Some(replay) = record.replays.get(&key) else {
                        return Err(AdminConfigurationStoreError::StaleReservation);
                    };
                    if replay.request_digest != request_digest
                        || replay.revision != failed_revision
                        || record.active_revision != failed_revision
                    {
                        return Err(AdminConfigurationStoreError::StaleReservation);
                    }
                    record.active_revision = previous_revision;
                    record.values = previous_values.clone();
                    record.replays.remove(&key);
                    record.retired_revisions.insert(failed_revision);
                    let retired_revisions: Vec<u64> =
                        record.retired_revisions.iter().copied().collect();
                    Ok(CasApply::new(record, retired_revisions))
                })();
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)?;
        self.cleanup_retired_revisions(scope, &group_id, &retired_revisions)
            .await;
        Ok(())
    }

    async fn stage_revision_snapshot(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        tenant_id: &TenantId,
        commit: &AdminConfigurationCommit,
    ) -> Result<(), AdminConfigurationStoreError> {
        let path = revision_path(group_id, commit.revision)?;
        let snapshot = StoredRevisionSnapshot {
            tenant_id: tenant_id.clone(),
            group_id: group_id.clone(),
            commit: commit.clone(),
        };
        let entry = encode_revision_snapshot(&snapshot)?;
        match self
            .filesystem
            .put(scope, &path, entry, CasExpectation::Absent)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                let existing = self
                    .load_stored_revision_snapshot(scope, group_id, commit.revision)
                    .await?;
                if existing == snapshot {
                    Ok(())
                } else {
                    Err(AdminConfigurationStoreError::InvalidRecord)
                }
            }
            Err(error) => Err(map_backend_error("stage_revision_snapshot", &error)),
        }
    }

    async fn load_revision_snapshot(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        revision: u64,
    ) -> Result<AdminConfigurationCommit, AdminConfigurationStoreError> {
        let snapshot = self
            .load_stored_revision_snapshot(scope, group_id, revision)
            .await?;
        if snapshot.tenant_id != scope.tenant_id
            || snapshot.group_id != *group_id
            || snapshot.commit.revision != revision
        {
            return Err(AdminConfigurationStoreError::InvalidRecord);
        }
        Ok(snapshot.commit)
    }

    async fn load_stored_revision_snapshot(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        revision: u64,
    ) -> Result<StoredRevisionSnapshot, AdminConfigurationStoreError> {
        let path = revision_path(group_id, revision)?;
        let versioned = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(|error| map_backend_error("load_revision_snapshot", &error))?
            .ok_or(AdminConfigurationStoreError::InvalidRecord)?;
        decode_revision_snapshot(&versioned.entry.body)
    }

    async fn cleanup_retired_revisions(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        revisions: &[u64],
    ) {
        if revisions.is_empty() {
            return;
        }
        let Ok(Some(record)) = self.load_stored_record(scope, group_id).await else {
            return;
        };
        let mut deleted = Vec::new();
        for revision in revisions {
            if !record.retired_revisions.contains(revision)
                || record.active_revision == *revision
                || record
                    .replays
                    .values()
                    .any(|replay| replay.revision == *revision)
            {
                continue;
            }
            let Ok(path) = revision_path(group_id, *revision) else {
                continue;
            };
            match self.filesystem.delete(scope, &path).await {
                Ok(()) | Err(FilesystemError::NotFound { .. }) => deleted.push(*revision),
                Err(error) => {
                    tracing::warn!(
                        revision,
                        error = ?error,
                        "admin-configuration revision cleanup will retry"
                    );
                }
            }
        }
        if let Err(error) = self
            .acknowledge_retired_revisions(scope, group_id, &deleted)
            .await
        {
            tracing::warn!(error = ?error, "admin-configuration cleanup acknowledgement failed");
        }
    }

    async fn acknowledge_retired_revisions(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        revisions: &[u64],
    ) -> Result<(), AdminConfigurationStoreError> {
        if revisions.is_empty() {
            return Ok(());
        }
        let path = group_path(group_id)?;
        let tenant_id = scope.tenant_id.clone();
        let group_id = group_id.clone();
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            decode_record,
            encode_record,
            |current: Option<StoredAdminConfigurationRecord>| {
                let outcome = (|| {
                    let mut record = current.ok_or(AdminConfigurationStoreError::InvalidRecord)?;
                    ensure_record_owner(&record, &tenant_id, &group_id)?;
                    for revision in revisions {
                        record.retired_revisions.remove(revision);
                    }
                    Ok(CasApply::new(record, ()))
                })();
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn cleanup_unpublished_snapshot(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        revision: u64,
    ) {
        let Ok(Some(record)) = self.load_stored_record(scope, group_id).await else {
            return;
        };
        let published = record.active_revision == revision
            || record
                .replays
                .values()
                .any(|replay| replay.revision == revision);
        if published {
            return;
        }
        let Ok(path) = revision_path(group_id, revision) else {
            return;
        };
        if let Err(error) = self.filesystem.delete(scope, &path).await
            && !matches!(error, FilesystemError::NotFound { .. })
        {
            tracing::warn!(revision, error = ?error, "unpublished admin-configuration snapshot cleanup failed");
        }
    }

    async fn load_stored_record(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
    ) -> Result<Option<StoredAdminConfigurationRecord>, AdminConfigurationStoreError> {
        let path = group_path(group_id)?;
        let Some(versioned) = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(|error| map_backend_error("load_stored_record", &error))?
        else {
            return Ok(None);
        };
        decode_record(&versioned.entry.body).map(Some)
    }
}

fn reservation_from(
    record: &StoredAdminConfigurationRecord,
    idempotency_key: AdminConfigurationIdempotencyKey,
    request_digest: AdminConfigurationRequestDigest,
    revision: u64,
    expected_revision: u64,
) -> AdminConfigurationReservation {
    AdminConfigurationReservation {
        revision,
        expected_revision,
        tenant_id: record.tenant_id.clone(),
        group_id: record.group_id.clone(),
        idempotency_key,
        request_digest,
    }
}

fn ensure_record_owner(
    record: &StoredAdminConfigurationRecord,
    tenant_id: &TenantId,
    group_id: &AdminConfigurationGroupId,
) -> Result<(), AdminConfigurationStoreError> {
    if record.tenant_id == *tenant_id && record.group_id == *group_id {
        Ok(())
    } else {
        Err(AdminConfigurationStoreError::InvalidRecord)
    }
}

fn group_path(
    group_id: &AdminConfigurationGroupId,
) -> Result<ScopedPath, AdminConfigurationStoreError> {
    ScopedPath::new(format!("{ADMIN_CONFIGURATION_PREFIX}/{group_id}.json"))
        .map_err(|_| AdminConfigurationStoreError::InvalidRecord)
}

fn revision_path(
    group_id: &AdminConfigurationGroupId,
    revision: u64,
) -> Result<ScopedPath, AdminConfigurationStoreError> {
    ScopedPath::new(format!(
        "{ADMIN_CONFIGURATION_PREFIX}/{group_id}/revisions/{revision}.json"
    ))
    .map_err(|_| AdminConfigurationStoreError::InvalidRecord)
}

fn prune_expired_idempotency(
    record: &mut StoredAdminConfigurationRecord,
    now: DateTime<Utc>,
) -> Result<(), AdminConfigurationStoreError> {
    let pending_cutoff = now
        .checked_sub_signed(Duration::hours(PENDING_RETENTION_HOURS))
        .ok_or(AdminConfigurationStoreError::InvalidRecord)?;
    record.pending.retain(|_, pending| {
        pending.created_at > pending_cutoff && pending.expected_revision == record.active_revision
    });

    let replay_cutoff = now
        .checked_sub_signed(Duration::days(REPLAY_RETENTION_DAYS))
        .ok_or(AdminConfigurationStoreError::InvalidRecord)?;
    let mut expired_revisions = BTreeSet::new();
    record.replays.retain(|_, replay| {
        let keep = replay.completed_at > replay_cutoff;
        if !keep {
            expired_revisions.insert(replay.revision);
        }
        keep
    });
    for revision in expired_revisions {
        let still_replayed = record
            .replays
            .values()
            .any(|replay| replay.revision == revision);
        if revision != record.active_revision && !still_replayed {
            record.retired_revisions.insert(revision);
        }
    }
    Ok(())
}

fn decode_record(
    bytes: &[u8],
) -> Result<StoredAdminConfigurationRecord, AdminConfigurationStoreError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(AdminConfigurationStoreError::InvalidRecord);
    }
    serde_json::from_slice(bytes).map_err(|_| AdminConfigurationStoreError::InvalidRecord)
}

fn encode_record(
    record: &StoredAdminConfigurationRecord,
) -> Result<Entry, AdminConfigurationStoreError> {
    let body =
        serde_json::to_vec(record).map_err(|_| AdminConfigurationStoreError::InvalidRecord)?;
    if body.len() > MAX_RECORD_BYTES {
        return Err(AdminConfigurationStoreError::InvalidRecord);
    }
    let kind = RecordKind::new(ADMIN_CONFIGURATION_RECORD_KIND)
        .map_err(|_| AdminConfigurationStoreError::InvalidRecord)?;
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    entry.kind = Some(kind);
    Ok(entry)
}

fn decode_revision_snapshot(
    bytes: &[u8],
) -> Result<StoredRevisionSnapshot, AdminConfigurationStoreError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(AdminConfigurationStoreError::InvalidRecord);
    }
    serde_json::from_slice(bytes).map_err(|_| AdminConfigurationStoreError::InvalidRecord)
}

fn encode_revision_snapshot(
    snapshot: &StoredRevisionSnapshot,
) -> Result<Entry, AdminConfigurationStoreError> {
    let body =
        serde_json::to_vec(snapshot).map_err(|_| AdminConfigurationStoreError::InvalidRecord)?;
    if body.len() > MAX_RECORD_BYTES {
        return Err(AdminConfigurationStoreError::InvalidRecord);
    }
    let kind = RecordKind::new(ADMIN_CONFIGURATION_RECORD_KIND)
        .map_err(|_| AdminConfigurationStoreError::InvalidRecord)?;
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    entry.kind = Some(kind);
    Ok(entry)
}

fn map_backend_error(
    operation: &'static str,
    error: &ironclaw_filesystem::FilesystemError,
) -> AdminConfigurationStoreError {
    tracing::warn!(operation, error = ?error, "admin-configuration store operation failed");
    AdminConfigurationStoreError::Unavailable
}

fn map_cas_error(
    error: CasUpdateError<AdminConfigurationStoreError>,
) -> AdminConfigurationStoreError {
    match error {
        CasUpdateError::Apply(error) => error,
        CasUpdateError::CasUnsupported => AdminConfigurationStoreError::CasUnsupported,
        CasUpdateError::RetriesExhausted => AdminConfigurationStoreError::Contended,
        CasUpdateError::Timeout => AdminConfigurationStoreError::Unavailable,
        CasUpdateError::Backend(error) => map_backend_error("cas_update", &error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{Duration, TimeZone, Utc};
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope,
        SecretHandle, TenantId, UserId, VirtualPath,
    };

    use super::*;

    #[tokio::test]
    async fn replay_is_exact_inside_retention_and_old_snapshot_is_cleaned_after_expiry() {
        let store = test_store();
        let scope = test_scope();
        let group = AdminConfigurationGroupId::new("vendor.example").unwrap();
        let started = Utc.with_ymd_and_hms(2026, 7, 21, 12, 0, 0).unwrap();
        let first_key = AdminConfigurationIdempotencyKey::new("first").unwrap();
        let first_digest = AdminConfigurationRequestDigest::from_bytes([1; 32]);
        let first = reserved(
            store
                .reserve_at(&scope, &group, &first_key, first_digest, 0, started)
                .await
                .unwrap(),
        );
        let first_commit = store
            .commit_at(
                &scope,
                &first,
                test_values(first.revision, "first"),
                started,
            )
            .await
            .unwrap();

        let replay = store
            .reserve_at(
                &scope,
                &group,
                &first_key,
                first_digest,
                0,
                started + Duration::days(REPLAY_RETENTION_DAYS - 1),
            )
            .await
            .unwrap();
        assert_eq!(
            replay,
            AdminConfigurationReserveOutcome::Replay(first_commit)
        );

        let second = reserved(
            store
                .reserve_at(
                    &scope,
                    &group,
                    &AdminConfigurationIdempotencyKey::new("second").unwrap(),
                    AdminConfigurationRequestDigest::from_bytes([2; 32]),
                    1,
                    started + Duration::hours(1),
                )
                .await
                .unwrap(),
        );
        store
            .commit_at(
                &scope,
                &second,
                test_values(second.revision, "second"),
                started + Duration::hours(1),
            )
            .await
            .unwrap();

        let _third = store
            .reserve_at(
                &scope,
                &group,
                &AdminConfigurationIdempotencyKey::new("third").unwrap(),
                AdminConfigurationRequestDigest::from_bytes([3; 32]),
                2,
                started + Duration::days(REPLAY_RETENTION_DAYS + 1),
            )
            .await
            .unwrap();

        assert!(
            store
                .filesystem
                .get(&scope, &revision_path(&group, 1).unwrap())
                .await
                .unwrap()
                .is_none(),
            "expired non-active replay snapshot must be deleted",
        );
        assert!(
            store
                .filesystem
                .get(&scope, &revision_path(&group, 2).unwrap())
                .await
                .unwrap()
                .is_some(),
            "the active snapshot must never be deleted",
        );
    }

    #[tokio::test]
    async fn abandoned_pending_capacity_is_recovered_after_the_retention_horizon() {
        let store = test_store();
        let scope = test_scope();
        let group = AdminConfigurationGroupId::new("vendor.example").unwrap();
        let started = Utc.with_ymd_and_hms(2026, 7, 21, 12, 0, 0).unwrap();
        for index in 0..MAX_IDEMPOTENCY_RECORDS {
            store
                .reserve_at(
                    &scope,
                    &group,
                    &AdminConfigurationIdempotencyKey::new(format!("pending-{index}")).unwrap(),
                    AdminConfigurationRequestDigest::from_bytes([(index % 255) as u8; 32]),
                    0,
                    started,
                )
                .await
                .unwrap();
        }
        let full = store
            .reserve_at(
                &scope,
                &group,
                &AdminConfigurationIdempotencyKey::new("over-capacity").unwrap(),
                AdminConfigurationRequestDigest::from_bytes([9; 32]),
                0,
                started,
            )
            .await
            .unwrap_err();
        assert_eq!(
            full,
            AdminConfigurationStoreError::IdempotencyCapacityExhausted
        );

        let recovered = store
            .reserve_at(
                &scope,
                &group,
                &AdminConfigurationIdempotencyKey::new("after-expiry").unwrap(),
                AdminConfigurationRequestDigest::from_bytes([10; 32]),
                0,
                started + Duration::hours(PENDING_RETENTION_HOURS + 1),
            )
            .await
            .unwrap();
        assert!(matches!(
            recovered,
            AdminConfigurationReserveOutcome::Reserved(_)
        ));
    }

    fn reserved(outcome: AdminConfigurationReserveOutcome) -> AdminConfigurationReservation {
        match outcome {
            AdminConfigurationReserveOutcome::Reserved(reservation) => reservation,
            AdminConfigurationReserveOutcome::Replay(_) => panic!("new key cannot replay"),
        }
    }

    fn test_values(
        revision: u64,
        value: &str,
    ) -> BTreeMap<SecretHandle, AdminConfigurationValueRef> {
        BTreeMap::from([(
            SecretHandle::new("client_id").unwrap(),
            AdminConfigurationValueRef::Inline(format!("{value}-{revision}")),
        )])
    }

    fn test_store() -> FilesystemAdminConfigurationStore<InMemoryBackend> {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/extension-admin-configuration").unwrap(),
            VirtualPath::new("/tenants/test/shared/admin-configuration").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            view,
        )))
    }

    fn test_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("operator-a").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}
