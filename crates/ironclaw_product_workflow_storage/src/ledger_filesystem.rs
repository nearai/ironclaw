//! Filesystem-backed [`IdempotencyLedger`] implementation.
//!
//! Replaces the previous per-backend SQL implementations
//! (`ledger_libsql.rs` / `ledger_postgres.rs`) with a single store
//! written against the universal `RootFilesystem` surface. The version
//! token returned by [`RootFilesystem::put`] / [`RootFilesystem::get`]
//! is the natural **ownership token** for the saga state machine: every
//! `begin_or_replay` reclaim mints a fresh row, every `settle` / `release`
//! is a `CasExpectation::Version` transition. A stale worker that resumes
//! after another caller reclaimed a row simply observes a version
//! mismatch and surfaces `Transient { reason: superseded }` — the
//! action-id check from the old SQL impls becomes a property of the
//! type system instead of a per-statement `WHERE` clause.
//!
//! The action_id stays in the persisted body for diagnostic value and as
//! a defence-in-depth match against a possible future backend that emits
//! a stale version after a reclaim (none does today). The CAS version is
//! the primary guard.
//!
//! Path layout:
//! - `/ledger/inbound/<sha256-of-fingerprint>.json` — one record per
//!   fingerprint, body is a JSON-serialized [`ProductInboundAction`].
//!
//! Recovery contract (`begin_or_replay`):
//! - Settled / DeduplicatedReplay row → `Replay`.
//! - Non-terminal row fresh within the recovery lease → `Transient`.
//! - Non-terminal row past the lease → reclaim via Version CAS; on
//!   `VersionMismatch` (another caller beat us) surface `Transient` so the
//!   caller retries.
//! - Absent row → ordinary `Absent` CAS put.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use ironclaw_product_workflow::{
    ActionFingerprintKey, ActionPhase, IdempotencyDecision, IdempotencyLedger,
    ProductInboundAction, ProductWorkflowError,
};
use sha2::{Digest, Sha256};

use crate::error::transient;
use crate::recovery::DEFAULT_RECOVERY_LEASE;

/// Bounded retry budget for read-then-CAS-write loops. Sized to absorb a
/// small burst of contending writers; the recovery-lease contract and the
/// `superseded` rejection path keep us from spinning indefinitely against a
/// truly stuck row.
const MAX_CAS_RETRIES: usize = 5;

/// Filesystem-backed durable idempotency ledger.
///
/// Construct with a [`ScopedFilesystem`] over any [`RootFilesystem`]
/// implementation (libSQL, Postgres, in-memory, HSM-decorated, …). Tenant
/// isolation comes from the [`MountView`](ironclaw_host_api::MountView) the
/// composition layer hands the scoped filesystem at construction time — the
/// ledger does not own a tenant id.
pub struct FilesystemIdempotencyLedger<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    recovery_lease: Duration,
}

impl<F> FilesystemIdempotencyLedger<F>
where
    F: RootFilesystem,
{
    /// Construct with the [`DEFAULT_RECOVERY_LEASE`].
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self::with_recovery_lease(filesystem, DEFAULT_RECOVERY_LEASE)
    }

    /// Construct with an explicit recovery-lease TTL. A non-terminal row older
    /// than this is eligible for reclaim on the next `begin_or_replay` call
    /// for the same fingerprint.
    pub fn with_recovery_lease(
        filesystem: Arc<ScopedFilesystem<F>>,
        recovery_lease: Duration,
    ) -> Self {
        Self {
            filesystem,
            recovery_lease,
        }
    }

    async fn read(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<(ProductInboundAction, VersionedEntry)>, ProductWorkflowError> {
        let Some(versioned) = self
            .filesystem
            .get(scope, path)
            .await
            .map_err(map_fs_error)?
        else {
            return Ok(None);
        };
        let action: ProductInboundAction = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| transient(format!("invalid ledger row body: {e}")))?;
        Ok(Some((action, versioned)))
    }

    async fn write(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        action: &ProductInboundAction,
        cas: CasExpectation,
    ) -> Result<(), ProductWorkflowError> {
        let body = serde_json::to_vec(action)
            .map_err(|e| transient(format!("serialize ledger row: {e}")))?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.put_with_byte_fallback(scope, path, entry, cas).await
    }

    /// Write `entry` with the given CAS expectation, falling back to a
    /// metadata-stripped opaque write + `CasExpectation::Any` for backends
    /// that reject record-shape entries or non-`Any` CAS (e.g.
    /// `LocalFilesystem`). Mirrors the same fallback used by other
    /// filesystem-backed stores in the workspace so every byte-only mount
    /// stays writeable.
    async fn put_with_byte_fallback(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<(), ProductWorkflowError> {
        match self.filesystem.put(scope, path, entry.clone(), cas).await {
            Ok(_) => Ok(()),
            Err(FilesystemError::Unsupported { .. }) => {
                let opaque = Entry::bytes(entry.body).with_content_type(entry.content_type);
                self.filesystem
                    .put(scope, path, opaque, CasExpectation::Any)
                    .await
                    .map(|_| ())
                    .map_err(map_fs_error)
            }
            Err(error) => Err(map_fs_error(error)),
        }
    }
}

#[async_trait]
impl<F> IdempotencyLedger for FilesystemIdempotencyLedger<F>
where
    F: RootFilesystem,
{
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let scope = ResourceScope::system();
        let path = ledger_path(&fingerprint)?;
        let lease_chrono = chrono::Duration::from_std(self.recovery_lease)
            .map_err(|e| transient(format!("recovery lease out of range: {e}")))?;

        for _ in 0..MAX_CAS_RETRIES {
            // Read first so we can branch on phase before writing. The
            // INSERT-first pattern the SQL impls used was a workaround for
            // SQL's lack of typed conflict semantics; the FS surface gives
            // us `VersionMismatch` directly. CAS::Absent on the put still
            // closes the TOCTOU window between this read and the write.
            match self.read(&scope, &path).await? {
                Some((existing, versioned)) => match existing.phase {
                    ActionPhase::Settled | ActionPhase::DeduplicatedReplay => {
                        return Ok(IdempotencyDecision::Replay(existing));
                    }
                    ActionPhase::Received | ActionPhase::Dispatched => {
                        let stale_threshold = received_at - lease_chrono;
                        if existing.received_at >= stale_threshold {
                            return Err(transient(
                                "idempotency fingerprint already in flight; retry after recovery lease",
                            ));
                        }
                        // Stale reclaim — Version CAS guarantees only one
                        // racing reclaimer wins. The loser observes
                        // VersionMismatch and either retries (might find the
                        // winner's fresh row → Transient) or exhausts the
                        // retry budget.
                        let claimed = ProductInboundAction::begin(fingerprint.clone(), received_at);
                        match self
                            .write(
                                &scope,
                                &path,
                                &claimed,
                                CasExpectation::Version(versioned.version),
                            )
                            .await
                        {
                            Ok(()) => {
                                tracing::warn!(
                                    fingerprint = ?claimed.fingerprint,
                                    prior_received_at = %existing.received_at,
                                    lease_secs = self.recovery_lease.as_secs(),
                                    "reclaimed stale non-terminal ledger row after recovery lease elapsed"
                                );
                                return Ok(IdempotencyDecision::New(claimed));
                            }
                            Err(ProductWorkflowError::Transient { reason })
                                if reason.contains("version") =>
                            {
                                continue;
                            }
                            Err(err) => return Err(err),
                        }
                    }
                },
                None => {
                    // No row — fresh claim. CAS::Absent rejects concurrent
                    // writers; on VersionMismatch we go around and the next
                    // iteration sees the row written by the winner.
                    let action = ProductInboundAction::begin(fingerprint.clone(), received_at);
                    match self
                        .write(&scope, &path, &action, CasExpectation::Absent)
                        .await
                    {
                        Ok(()) => return Ok(IdempotencyDecision::New(action)),
                        Err(ProductWorkflowError::Transient { reason })
                            if reason.contains("version") =>
                        {
                            continue;
                        }
                        Err(err) => return Err(err),
                    }
                }
            }
        }
        Err(transient(
            "idempotency fingerprint contended past retry budget; retry later",
        ))
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let scope = ResourceScope::system();
        let path = ledger_path(&action.fingerprint)?;

        for _ in 0..MAX_CAS_RETRIES {
            let Some((existing, versioned)) = self.read(&scope, &path).await? else {
                return Err(transient(
                    "idempotency reservation was superseded before terminal settle",
                ));
            };
            // Action-id mismatch ⇒ the row was reclaimed by another caller
            // between our begin and our settle. Surface superseded
            // immediately rather than spinning the retry loop — no amount
            // of retries reattaches us to a row we no longer own.
            if existing.action_id != action.action_id {
                return Err(transient(
                    "idempotency reservation was superseded before terminal settle",
                ));
            }
            // Idempotent re-settle: if the row is already terminal and
            // already carries our action_id, the prior settle succeeded
            // (probably an at-least-once retry from the workflow layer).
            if matches!(
                existing.phase,
                ActionPhase::Settled | ActionPhase::DeduplicatedReplay
            ) {
                return Ok(());
            }
            // Persist the terminal transition. Version CAS protects against
            // a reclaim race: if another caller wrote between our read and
            // our write (which would only happen on a Version-CAS reclaim
            // — the action_id check above would catch a value change but
            // not a same-value-different-version sequence in theory), the
            // put fails with VersionMismatch and we re-read, finding the
            // mismatched action_id on the next pass.
            match self
                .write(
                    &scope,
                    &path,
                    &action,
                    CasExpectation::Version(versioned.version),
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(ProductWorkflowError::Transient { reason }) if reason.contains("version") => {
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
        Err(transient(
            "idempotency settle contended past retry budget; retry later",
        ))
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let scope = ResourceScope::system();
        let path = ledger_path(&action.fingerprint)?;

        // Release is a silent no-op for stale callers: if the row is
        // missing, terminal, or owned by a different action_id, we leave
        // it alone. This matches the in-memory ledger contract and means
        // a stale worker resuming after a recovery-lease reclaim cannot
        // discard the new owner's reservation.
        let Some((existing, _versioned)) = self.read(&scope, &path).await? else {
            return Ok(());
        };
        if existing.action_id != action.action_id {
            return Ok(());
        }
        if matches!(
            existing.phase,
            ActionPhase::Settled | ActionPhase::DeduplicatedReplay
        ) {
            return Ok(());
        }
        // Best-effort delete. The narrow race between our action_id check
        // and the delete itself is the same race the SQL implementations
        // had (SQL `DELETE` does not take a version either); the
        // consequence is bounded because a freshly-reclaimed row that we
        // erroneously delete causes the new owner's settle to fail with
        // `superseded`, which the protocol layer treats as Transient and
        // retries. A future fix would extend the trait surface with a
        // CAS-aware delete.
        self.filesystem
            .delete(&scope, &path)
            .await
            .map_err(map_fs_error)?;
        Ok(())
    }
}

/// Deterministic path for a fingerprint: `/ledger/inbound/<hex-sha256>.json`.
///
/// Hashing the fingerprint keeps the path short, stable, and PII-free; the
/// fingerprint itself contains the external-actor id and event id which we
/// don't want surfaced on `list_dir` results. The full fingerprint is still
/// stored inside the entry body for diagnostic value.
fn ledger_path(fingerprint: &ActionFingerprintKey) -> Result<ScopedPath, ProductWorkflowError> {
    let mut hasher = Sha256::new();
    hasher.update(fingerprint.adapter_id.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(fingerprint.installation_id.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(fingerprint.external_actor_ref.kind().as_bytes());
    hasher.update(b"\0");
    hasher.update(fingerprint.external_actor_ref.id().as_bytes());
    hasher.update(b"\0");
    hasher.update(fingerprint.source_binding_key.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(fingerprint.external_event_id.as_str().as_bytes());
    let digest = hex::encode(hasher.finalize());
    ScopedPath::new(format!("/ledger/inbound/{digest}.json"))
        .map_err(|e| transient(format!("invalid ledger path: {e}")))
}

/// Map a filesystem error to a `ProductWorkflowError`. The retry-loop
/// branches above detect `VersionMismatch` via the substring "version" in
/// the rendered reason (the universal-FS error type does not export a
/// stable discriminant we can match on across feature combos). All other
/// errors are surfaced as opaque transients so backend detail (host paths,
/// driver-specific reasons) does not cross the workflow boundary.
fn map_fs_error(error: FilesystemError) -> ProductWorkflowError {
    match error {
        FilesystemError::VersionMismatch { .. } => transient("ledger row version mismatch"),
        other => transient(format!("ledger filesystem error: {other}")),
    }
}
