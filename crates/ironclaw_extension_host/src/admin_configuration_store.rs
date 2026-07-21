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

use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_extensions::AdminConfigurationGroupId;
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, RecordKind, RootFilesystem, ScopedFilesystem,
    cas_update,
};
use ironclaw_host_api::{ResourceScope, ScopedPath, SecretHandle, TenantId};
use serde::{Deserialize, Serialize};

const ADMIN_CONFIGURATION_RECORD_KIND: &str = "extension_admin_configuration";
const ADMIN_CONFIGURATION_PREFIX: &str = "/extension-admin-configuration/groups";
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;
const MAX_IDEMPOTENCY_RECORDS: usize = 1_024;
const MAX_RECORD_BYTES: usize = 1024 * 1024;

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
    request_digest: AdminConfigurationRequestDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredReplay {
    request_digest: AdminConfigurationRequestDigest,
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
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemAdminConfigurationStore<F>
where
    F: RootFilesystem,
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
    ) -> Result<AdminConfigurationReserveOutcome, AdminConfigurationStoreError> {
        let path = group_path(group_id)?;
        let tenant_id = scope.tenant_id.clone();
        let group_id = group_id.clone();
        let key = idempotency_key.as_str().to_string();
        cas_update(
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
                    if let Some(replay) = record.replays.get(&key) {
                        if replay.request_digest != request_digest {
                            return Err(AdminConfigurationStoreError::IdempotencyConflict);
                        }
                        return Ok(CasApply::no_op(
                            record.clone(),
                            AdminConfigurationReserveOutcome::Replay(replay.commit.clone()),
                        ));
                    }
                    if let Some(pending) = record.pending.get(&key) {
                        if pending.request_digest != request_digest {
                            return Err(AdminConfigurationStoreError::IdempotencyConflict);
                        }
                        if pending.revision > record.active_revision {
                            let reservation = reservation_from(
                                &record,
                                idempotency_key.clone(),
                                request_digest,
                                pending.revision,
                            );
                            return Ok(CasApply::no_op(
                                record.clone(),
                                AdminConfigurationReserveOutcome::Reserved(reservation),
                            ));
                        }
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
                            request_digest,
                        },
                    );
                    let reservation = reservation_from(
                        &record,
                        idempotency_key.clone(),
                        request_digest,
                        revision,
                    );
                    Ok(CasApply::new(
                        record,
                        AdminConfigurationReserveOutcome::Reserved(reservation),
                    ))
                })();
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    /// Atomically make previously staged value references active.
    pub async fn commit(
        &self,
        scope: &ResourceScope,
        reservation: &AdminConfigurationReservation,
        values: BTreeMap<SecretHandle, AdminConfigurationValueRef>,
    ) -> Result<AdminConfigurationCommit, AdminConfigurationStoreError> {
        if reservation.tenant_id != scope.tenant_id {
            return Err(AdminConfigurationStoreError::UnknownReservation);
        }
        let path = group_path(&reservation.group_id)?;
        let tenant_id = scope.tenant_id.clone();
        let group_id = reservation.group_id.clone();
        let key = reservation.idempotency_key.as_str().to_string();
        let requested_revision = reservation.revision;
        let request_digest = reservation.request_digest;
        cas_update(
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
                        return Ok(CasApply::no_op(record.clone(), replay.commit.clone()));
                    }
                    let pending = record
                        .pending
                        .get(&key)
                        .ok_or(AdminConfigurationStoreError::UnknownReservation)?;
                    if pending.request_digest != request_digest
                        || pending.revision != requested_revision
                    {
                        return Err(AdminConfigurationStoreError::UnknownReservation);
                    }
                    if requested_revision <= record.active_revision {
                        return Err(AdminConfigurationStoreError::StaleReservation);
                    }
                    let commit = AdminConfigurationCommit {
                        revision: requested_revision,
                        values: values.clone(),
                    };
                    record.active_revision = requested_revision;
                    record.values = values.clone();
                    record.pending.remove(&key);
                    record.replays.insert(
                        key.clone(),
                        StoredReplay {
                            request_digest,
                            commit: commit.clone(),
                        },
                    );
                    Ok(CasApply::new(record, commit))
                })();
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }
}

fn reservation_from(
    record: &StoredAdminConfigurationRecord,
    idempotency_key: AdminConfigurationIdempotencyKey,
    request_digest: AdminConfigurationRequestDigest,
    revision: u64,
) -> AdminConfigurationReservation {
    AdminConfigurationReservation {
        revision,
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
