// arch-exempt: large_file, mechanical DiskFilesystem->DiskFilesystem Bucket-2 rename (arch-simplification §4.4), no logic change, plan #6168
//! Filesystem-backed [`OutboundStateStore`] implementation.
//!
//! Persists outbound metadata under a fixed [`ScopedPath`] tree rooted at the
//! `/outbound` mount alias, using the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface accessed
//! through a [`ScopedFilesystem`]. The [`MountView`](ironclaw_host_api::MountView)
//! wired by composition resolves `/outbound` to a tenant/user-scoped
//! [`VirtualPath`](ironclaw_host_api::VirtualPath) (e.g.
//! `/tenants/<tenant_id>/users/<user_id>/outbound`) and enforces per-grant ACL
//! before any backend dispatch — so tenant isolation is structural rather than
//! a convention this code has to remember.
//!
//! Adding this alongside the SQL backends gives every operator the option of
//! mounting outbound state on the universal filesystem fabric (libSQL,
//! Postgres, in-memory, or HSM-decorated) without reaching back into a
//! per-crate driver.
//!
//! Per-record paths (alias-relative under `/outbound`):
//! - `/outbound/policies/<thread-scope-key>.json` — thread notification
//!   policy keyed by `(tenant, agent?, project?, thread)`.
//! - `/outbound/subscriptions/<subscription-key>.json` — projection
//!   subscription cursor keyed by `(subscription_id, actor, scope, thread)`.
//!   The key is a deterministic hash so the path doesn't leak the actor on
//!   list operations.
//! - `/outbound/deliveries/<delivery_id>.json` — delivery attempt keyed by
//!   `delivery_id`. An indexed `scope` projection allows
//!   `list_delivery_attempts(scope)` to filter within the tenant-scoped
//!   subtree without materializing every row.
//! - `/outbound/communication-preferences/<sha256(v2-scoped-key)>.json` —
//!   scoped communication preference row keyed by a hashed
//!   `CommunicationPreferenceKey`. Reply-target refs remain candidates and do
//!   not grant send authority.
//! - `/outbound/delivered-gate-routes/<sha256(tenant|user|gate_ref)>.json` —
//!   cross-thread approval routing record keyed by `(tenant_id, user_id,
//!   gate_ref)`. Written when an approval prompt is delivered to a personal
//!   target on a different thread from the run; used by the routing wrapper to
//!   rewrite DM replies to the correct run scope.
//! - `/outbound/run-final-reply-targets/<turn-run-id>.json` — exact,
//!   actor-scoped final-reply destination for one run. The record contains
//!   metadata only and is revalidated against current target authority before
//!   provider egress.
//! - `/outbound/run-final-reply-handoffs/<event-cursor>-<turn-run-id>.json` —
//!   minimal rebuildable projection keys for completed-run delivery. The
//!   sibling metadata cursor records how far the authoritative turn-event log
//!   has been materialized; neither contains reply content or target data.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FileType, FilesystemError,
    Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue, Page, RootFilesystem,
    ScopedFilesystem, VersionedEntry, cas_update,
};
use ironclaw_host_api::{ResourceScope, ScopedPath, TenantId, ThreadId, UserId};
use ironclaw_turns::{EventCursor as TurnEventCursor, TurnActor, TurnScope};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::validation::{
    validate_advance_request, validate_communication_preference, validate_delivery_attempt,
    validate_delivery_identity, validate_delivery_status_request, validate_policy,
    validate_subscription_identity, validate_subscription_record, validate_subscription_request,
};
use crate::{
    AdvanceSubscriptionCursorRequest, CommunicationPreferenceKey, CommunicationPreferenceRecord,
    CommunicationPreferenceRepository, CommunicationPreferenceVersion, DeliveredGateRouteRecord,
    DeliveredGateRouteStore, DeliveryDefaultScope, LoadSubscriptionCursorRequest,
    MAX_RUN_DELIVERY_CLEANUP_RECORDS, MAX_RUN_FINAL_REPLY_HANDOFF_PAGE, OutboundDeliveryAttempt,
    OutboundDeliveryId, OutboundDeliveryStatus, OutboundError, OutboundStateStore,
    ProjectionSubscriptionId, ProjectionSubscriptionRecord, RunDeliveryCleanupRecord,
    RunDeliveryCleanupRequest, RunFinalReplyHandoffRecord, RunFinalReplyTargetRecord,
    RunFinalReplyTargetRequest, ThreadNotificationPolicy, TriggeredRunDeliveryRecord,
    TriggeredRunDeliveryStore, UpdateDeliveryStatusRequest, VersionedCommunicationPreferenceRecord,
    WriteCommunicationPreferenceRequest,
};

/// Maximum number of compare-and-swap retries on a read-then-write path
/// before surfacing the conflict as a permanent backend failure. Sized to
/// absorb a small burst of concurrent writers without spinning indefinitely;
/// progression invariants (e.g. cursor must not move backwards) are
/// re-validated on every iteration so a regression breaks the loop early
/// rather than ricocheting between racing writers.
const MAX_CAS_RETRIES: usize = 5;

/// Maximum number of CAS retries for conversation-index read-modify-writes.
/// Tighter than `MAX_CAS_RETRIES` because index writes are best-effort
/// (callers treat route-store errors as non-fatal) and we want to bound spin
/// time across a small burst of concurrent gate deliveries to the same
/// conversation.
const MAX_CONV_IDX_CAS_RETRIES: usize = 3;

/// Indexed projection key for the scope of a delivery attempt. The value is a
/// hash of `(tenant, agent?, project?, thread)` — the same key
/// [`thread_scope_key`] computes for policy paths — so backends without
/// composite-index support can serve `list_delivery_attempts(scope)` with a
/// single equality lookup (audit finding F2). The `tenant_id` itself moves
/// into the path prefix via the [`ScopedFilesystem`] mount, so this index is
/// only ever used to discriminate within an already tenant-scoped subtree.
const DELIVERY_SCOPE_INDEX_KEY: &str = "scope";
const DELIVERY_SCOPE_INDEX_NAME: &str = "outbound_delivery_scope";
const DELIVERIES_ROOT: &str = "/outbound/deliveries";
const COMMUNICATION_PREFERENCES_ROOT: &str = "/outbound/communication-preferences";

/// Indexed projection key for the tenant id, written alongside every
/// outbound write as a defense-in-depth measure beyond path-prefix
/// scoping. See `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`
/// — path-prefix scoping is the primary isolation boundary; this
/// projection lets admin-tier queries filter explicitly by tenant and
/// turns a path-rewriting bug into a query-time mismatch.
const TENANT_ID_INDEX_KEY: &str = "tenant_id";
const TENANT_ID_INDEX_NAME: &str = "outbound_by_tenant";
const POLICIES_ROOT: &str = "/outbound/policies";
const SUBSCRIPTIONS_ROOT: &str = "/outbound/subscriptions";
const TRIGGERED_RUN_DELIVERY_ROOT: &str = "/outbound/triggered-run-delivery";
const DELIVERED_GATE_ROUTES_ROOT: &str = "/outbound/delivered-gate-routes";
const DELIVERED_GATE_ROUTES_CONV_IDX_ROOT: &str = "/outbound/delivered-gate-routes/conv-idx";
const RUN_DELIVERY_CLEANUP_ROOT: &str = "/outbound/run-delivery-cleanup";
const RUN_FINAL_REPLY_TARGETS_ROOT: &str = "/outbound/run-final-reply-targets";
const RUN_FINAL_REPLY_HANDOFFS_ROOT: &str = "/outbound/run-final-reply-handoffs";
const RUN_FINAL_REPLY_HANDOFF_CURSOR_PATH: &str =
    "/outbound/run-final-reply-handoff-meta/cursor.json";
const RUN_FINAL_REPLY_HANDOFF_ORDER_INDEX_KEY: &str = "handoff_order";
const RUN_FINAL_REPLY_HANDOFF_ORDER_INDEX_NAME: &str = "run_final_reply_handoff_order";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct RunFinalReplyHandoffCursorRecord {
    event_cursor: TurnEventCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RunDeliveryCleanupSnapshot {
    request: RunDeliveryCleanupRequest,
    records: Vec<RunDeliveryCleanupRecord>,
}

/// Filesystem-backed outbound store. Construct with a [`ScopedFilesystem`]
/// over any [`RootFilesystem`] implementation (libSQL, Postgres, in-memory,
/// HSM-decorated, …) — the store doesn't care which. Tenant isolation is
/// enforced by the [`MountView`](ironclaw_host_api::MountView) the
/// composition layer hands the scoped filesystem at construction time.
pub struct FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    async fn put_json<T: Serialize>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        value: &T,
        tenant: &TenantId,
        cas: CasExpectation,
    ) -> Result<(), OutboundError> {
        let body = serde_json::to_vec(value).map_err(|_| OutboundError::Serialization)?;
        let entry = Entry::bytes(body)
            .with_content_type(ContentType::json())
            .with_indexed(tenant_id_index_key(), tenant_id_index_value(tenant));
        self.put_with_byte_fallback(scope, path, entry, cas).await
    }

    /// Like [`put_json`] but additionally projects an indexed scope value so
    /// backends with index support can answer `query(Filter::Eq { scope })`
    /// without materializing every delivery row (audit finding F2). The
    /// `tenant_id` lives in the [`ScopedFilesystem`] mount prefix, not in
    /// this index value — the index discriminates between scopes _within_ a
    /// tenant-scoped subtree.
    async fn put_delivery_attempt_indexed(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        attempt: &OutboundDeliveryAttempt,
        cas: CasExpectation,
    ) -> Result<(), OutboundError> {
        let entry = delivery_attempt_entry(attempt)?;
        self.put_with_byte_fallback(scope, path, entry, cas).await
    }

    async fn ensure_run_final_reply_handoff_index(&self) -> Result<(), OutboundError> {
        let root = run_final_reply_handoffs_root()?;
        let name = IndexName::new(RUN_FINAL_REPLY_HANDOFF_ORDER_INDEX_NAME)
            .map_err(|_| OutboundError::Backend)?;
        let spec = IndexSpec::new(
            name,
            vec![run_final_reply_handoff_order_index_key()?],
            IndexKind::Exact,
        );
        self.filesystem
            .ensure_index(&ResourceScope::system(), &root, &spec)
            .await
            .map_err(map_fs_error)
    }

    /// Write `entry` with the given CAS expectation, falling back to a
    /// metadata-stripped opaque write + `CasExpectation::Any` for backends
    /// that reject record-shape entries or non-`Any` CAS (e.g.
    /// `DiskFilesystem`). Mirrors
    /// [`ironclaw_processes::filesystem_store::put_with_byte_fallback`] so
    /// every byte-only mount in the workspace stays writeable through the
    /// new filesystem stores.
    async fn put_with_byte_fallback(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<(), OutboundError> {
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

    async fn ensure_delivery_scope_index(
        &self,
        scope: &ResourceScope,
    ) -> Result<(), OutboundError> {
        let root = deliveries_root()?;
        let name = IndexName::new(DELIVERY_SCOPE_INDEX_NAME).map_err(|_| OutboundError::Backend)?;
        let spec = IndexSpec::new(name, vec![delivery_scope_index_key()], IndexKind::Exact);
        match self.filesystem.ensure_index(scope, &root, &spec).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::Unsupported { .. }) => Ok(()),
            Err(error) => Err(map_fs_error(error)),
        }
    }

    async fn ensure_tenant_id_index(
        &self,
        scope: &ResourceScope,
        root: &ScopedPath,
    ) -> Result<(), OutboundError> {
        let name = IndexName::new(TENANT_ID_INDEX_NAME).map_err(|_| OutboundError::Backend)?;
        let spec = IndexSpec::new(name, vec![tenant_id_index_key()], IndexKind::Exact);
        match self.filesystem.ensure_index(scope, root, &spec).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::Unsupported { .. }) => Ok(()),
            Err(error) => Err(map_fs_error(error)),
        }
    }

    async fn get_versioned_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<(T, VersionedEntry)>, OutboundError> {
        let Some(versioned) = self
            .filesystem
            .get(scope, path)
            .await
            .map_err(map_fs_error)?
        else {
            return Ok(None);
        };
        let parsed = serde_json::from_slice(&versioned.entry.body)
            .map_err(|_| OutboundError::Serialization)?;
        Ok(Some((parsed, versioned)))
    }

    async fn get_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<T>, OutboundError> {
        Ok(self
            .get_versioned_json::<T>(scope, path)
            .await?
            .map(|(value, _)| value))
    }

    async fn write_delivered_gate_route_conversation_indexes(
        &self,
        record: &DeliveredGateRouteRecord,
    ) -> Result<(), String> {
        // Index files use the system resource scope: the store instance is
        // mounted per (tenant, user), and the index path hash binds the
        // tenant. The primary record is NOT read through this scope — the
        // index entry carries the identity triple so the lookup reloads it
        // via load_delivered_gate_route with the same tenant+user scope the
        // write path used.
        let resource_scope = ResourceScope::system();
        let new_entry = DeliveredGateRouteConversationIndexRouteEntry {
            tenant_id: record.tenant_id.clone(),
            user_id: record.user_id.clone(),
            gate_ref: record.gate_ref.clone(),
        };
        for conversation_fingerprint in &record.delivered_conversation_fingerprints {
            let idx_path = delivered_gate_route_conv_idx_path(
                &record.tenant_id,
                &record.user_id,
                conversation_fingerprint,
            )
            .map_err(|e| format!("delivered gate route conversation index path: {e}"))?;
            let entry_to_add = new_entry.clone();
            self.retry_conv_idx(&resource_scope, &idx_path, |mut routes| {
                if !routes.iter().any(|r| r == &entry_to_add) {
                    routes.push(entry_to_add.clone());
                }
                ConvIdxUpdate::Write(routes)
            })
            .await
            .map_err(|e| format!("delivered gate route conversation index write: {e}"))?;
        }
        Ok(())
    }

    async fn delete_delivered_gate_route_conversation_indexes(
        &self,
        record: &DeliveredGateRouteRecord,
        conversation_fingerprints: &[String],
    ) -> Result<(), String> {
        let resource_scope = ResourceScope::system();
        let entry_to_remove = DeliveredGateRouteConversationIndexRouteEntry {
            tenant_id: record.tenant_id.clone(),
            user_id: record.user_id.clone(),
            gate_ref: record.gate_ref.clone(),
        };
        for conversation_fingerprint in conversation_fingerprints {
            let idx_path = delivered_gate_route_conv_idx_path(
                &record.tenant_id,
                &record.user_id,
                conversation_fingerprint,
            )
            .map_err(|e| format!("delivered gate route conversation index path: {e}"))?;
            let entry = entry_to_remove.clone();
            self.retry_conv_idx(&resource_scope, &idx_path, |mut routes| {
                routes.retain(|r| r != &entry);
                if routes.is_empty() {
                    ConvIdxUpdate::Delete
                } else {
                    ConvIdxUpdate::Write(routes)
                }
            })
            .await
            .map_err(|e| {
                format!("delivered gate route conversation index write after remove: {e}")
            })?;
        }
        Ok(())
    }

    /// Read-modify-write a conversation index file under versioned CAS, with a
    /// bounded retry loop to absorb concurrent writes to the same fingerprint.
    ///
    /// `merge` receives the current route list (empty if the file is absent)
    /// and returns one of:
    /// - [`ConvIdxUpdate::Write`] — serialize and write back the updated list.
    /// - [`ConvIdxUpdate::Delete`] — the list is empty; garbage-collect the
    ///   index file.
    ///
    /// The CAS expectation is:
    /// - `CasExpectation::Absent` when the file did not exist at read time.
    /// - `CasExpectation::Version(v)` when the file existed at read time.
    ///
    /// On a `CasConflict` the loop re-reads and re-applies `merge`; after
    /// `MAX_CONV_IDX_CAS_RETRIES` attempts the error is surfaced as a
    /// `String` and callers log it at best-effort.
    ///
    /// ## Delete handling
    ///
    /// The filesystem layer has no versioned-delete operation — `ScopedFilesystem::delete`
    /// accepts no CAS argument. An unversioned delete on a now-empty index
    /// races with a concurrent writer that added a sibling route between our
    /// read and our delete: writer B's entry would be silently removed.
    ///
    /// To close the race we instead CAS-write an empty `V2 { routes: [] }` file
    /// back with the version we read. A concurrent writer that won the slot
    /// first will cause a `CasConflict`, which the loop handles by re-reading
    /// and re-applying `merge` — the re-read will see the sibling entry and
    /// return `Write` rather than `Delete`. Empty index files left behind are
    /// harmless: the lookup's `into_routes()` returns an empty vec, which the
    /// caller treats as a miss. A background sweep or the next write to the
    /// same path will overwrite them.
    ///
    /// Note on `put_with_byte_fallback`: when the underlying mount is a
    /// `DiskFilesystem`, the fallback leg strips the CAS expectation and
    /// retries with `CasExpectation::Any`. That means the versioned-CAS
    /// guarantee is not available for local-filesystem mounts; concurrent
    /// writes on that backend still risk last-write-wins. This is accepted
    /// because (a) local-filesystem mounts are dev/test only and (b) the
    /// `DiskFilesystem` has no version-tracking sidecar yet. The fallback
    /// is kept to avoid breaking those mounts entirely; production
    /// (libSQL/Postgres) backends honour the CAS expectation.
    async fn retry_conv_idx(
        &self,
        resource_scope: &ResourceScope,
        idx_path: &ScopedPath,
        mut merge: impl FnMut(Vec<DeliveredGateRouteConversationIndexRouteEntry>) -> ConvIdxUpdate,
    ) -> Result<(), OutboundError> {
        for _ in 0..MAX_CONV_IDX_CAS_RETRIES {
            // Read the current file and capture its version for CAS.
            let (routes, cas) = match self
                .get_versioned_json::<DeliveredGateRouteConversationIndexFile>(
                    resource_scope,
                    idx_path,
                )
                .await?
            {
                Some((file, versioned)) => (
                    file.into_routes(),
                    CasExpectation::Version(versioned.version),
                ),
                None => (Vec::new(), CasExpectation::Absent),
            };

            // Determine what to write back (or skip if already absent).
            let write_back = match merge(routes) {
                ConvIdxUpdate::Write(updated) => {
                    DeliveredGateRouteConversationIndexFile::from_routes(updated)
                }
                ConvIdxUpdate::Delete => {
                    // The filesystem layer has no versioned delete. CAS-write an
                    // empty routes list instead so a concurrent sibling-add wins
                    // the conflict and the retry loop re-merges rather than
                    // blindly wiping the file. Empty index files are filtered out
                    // on read and are garbage-collected lazily.
                    if cas == CasExpectation::Absent {
                        // File was already absent at read time; nothing to do.
                        return Ok(());
                    }
                    DeliveredGateRouteConversationIndexFile::from_routes(Vec::new())
                }
            };

            let body = serde_json::to_vec(&write_back).map_err(|_| OutboundError::Serialization)?;
            let entry = Entry::bytes(body).with_content_type(ContentType::json());
            match self
                .put_with_byte_fallback(resource_scope, idx_path, entry, cas)
                .await
            {
                Ok(()) => return Ok(()),
                Err(OutboundError::CasConflict) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(OutboundError::Backend)
    }

    /// Writes preference records with versioned CAS only.
    ///
    /// This intentionally bypasses the byte-only fallback used by non-CAS
    /// helpers: preference updates must fail closed when the mount cannot
    /// preserve the expected version.
    async fn put_json_strict_cas<T: Serialize>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        value: &T,
        tenant: &TenantId,
        cas: CasExpectation,
    ) -> Result<CommunicationPreferenceVersion, OutboundError> {
        let body = serde_json::to_vec(value).map_err(|_| OutboundError::Serialization)?;
        let entry = Entry::bytes(body)
            .with_content_type(ContentType::json())
            .with_indexed(tenant_id_index_key(), tenant_id_index_value(tenant));
        self.filesystem
            .put(scope, path, entry, cas)
            .await
            .map(|version| CommunicationPreferenceVersion::from_raw(version.get()))
            .map_err(map_fs_error)
    }
}

#[async_trait]
impl<F> CommunicationPreferenceRepository for FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    async fn load_communication_preference(
        &self,
        key: CommunicationPreferenceKey,
    ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
        let path = communication_preference_path(&key)?;
        let resource_scope = communication_preference_resource_scope(&key.scope);
        let Some((record, versioned)) = self
            .get_versioned_json::<CommunicationPreferenceRecord>(&resource_scope, &path)
            .await?
        else {
            return Ok(None);
        };
        if record.key() != key {
            return Err(OutboundError::Backend);
        }
        Ok(Some(VersionedCommunicationPreferenceRecord {
            record,
            version: CommunicationPreferenceVersion::from_raw(versioned.version.get()),
        }))
    }

    async fn write_communication_preference(
        &self,
        request: WriteCommunicationPreferenceRequest,
    ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
        validate_communication_preference(&request.record)?;
        let key = request.record.key();
        let path = communication_preference_path(&key)?;
        let resource_scope = communication_preference_resource_scope(&key.scope);
        self.ensure_tenant_id_index(&resource_scope, &communication_preferences_root()?)
            .await?;
        let cas = match request.expected_version {
            Some(version) => CasExpectation::Version(
                ironclaw_filesystem::RecordVersion::from_backend(version.raw()),
            ),
            None => CasExpectation::Absent,
        };
        let version = self
            .put_json_strict_cas(
                &resource_scope,
                &path,
                &request.record,
                request.record.scope.tenant_id(),
                cas,
            )
            .await?;
        Ok(VersionedCommunicationPreferenceRecord {
            record: request.record,
            version,
        })
    }
}

#[async_trait]
impl<F> OutboundStateStore for FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    async fn put_run_delivery_cleanup(
        &self,
        record: RunDeliveryCleanupRecord,
    ) -> Result<(), OutboundError> {
        record
            .validate()
            .map_err(|reason| OutboundError::InvalidRequest { reason })?;
        let request = record.request();
        let path = run_delivery_cleanup_path(&request)?;
        let resource_scope = request.scope.to_resource_scope();
        self.ensure_tenant_id_index(&resource_scope, &run_delivery_cleanup_root()?)
            .await?;
        cas_update(
            self.filesystem.as_ref(),
            &resource_scope,
            &path,
            decode_run_delivery_cleanup_snapshot,
            encode_run_delivery_cleanup_snapshot,
            move |current: Option<RunDeliveryCleanupSnapshot>| {
                let request = request.clone();
                let record = record.clone();
                async move {
                    let mut snapshot = current.unwrap_or_else(|| RunDeliveryCleanupSnapshot {
                        request: request.clone(),
                        records: Vec::new(),
                    });
                    validate_run_delivery_cleanup_snapshot(&snapshot, &request)?;
                    if snapshot.records.contains(&record) {
                        return Ok(CasApply::new(snapshot, ()));
                    }
                    if snapshot.records.len() >= MAX_RUN_DELIVERY_CLEANUP_RECORDS {
                        return Err(OutboundError::InvalidRequest {
                            reason: "run delivery cleanup record limit exceeded",
                        });
                    }
                    snapshot.records.push(record);
                    Ok(CasApply::new(snapshot, ()))
                }
            },
        )
        .await
        .map_err(map_cleanup_cas_error)
    }

    async fn load_run_delivery_cleanup(
        &self,
        request: RunDeliveryCleanupRequest,
    ) -> Result<Vec<RunDeliveryCleanupRecord>, OutboundError> {
        let path = run_delivery_cleanup_path(&request)?;
        let resource_scope = request.scope.to_resource_scope();
        let Some(snapshot) = self
            .get_json::<RunDeliveryCleanupSnapshot>(&resource_scope, &path)
            .await?
        else {
            return Ok(Vec::new());
        };
        if snapshot.request != request {
            return Err(OutboundError::AccessDenied);
        }
        validate_run_delivery_cleanup_snapshot(&snapshot, &request)?;
        Ok(snapshot.records)
    }

    async fn complete_run_delivery_cleanup(
        &self,
        record: &RunDeliveryCleanupRecord,
    ) -> Result<(), OutboundError> {
        record
            .validate()
            .map_err(|reason| OutboundError::InvalidRequest { reason })?;
        let request = record.request();
        let path = run_delivery_cleanup_path(&request)?;
        let resource_scope = request.scope.to_resource_scope();
        self.ensure_tenant_id_index(&resource_scope, &run_delivery_cleanup_root()?)
            .await?;
        let record = record.clone();
        cas_update(
            self.filesystem.as_ref(),
            &resource_scope,
            &path,
            decode_run_delivery_cleanup_snapshot,
            encode_run_delivery_cleanup_snapshot,
            move |current: Option<RunDeliveryCleanupSnapshot>| {
                let request = request.clone();
                let record = record.clone();
                async move {
                    let Some(mut snapshot) = current else {
                        return Ok(CasApply::no_op(
                            RunDeliveryCleanupSnapshot {
                                request,
                                records: Vec::new(),
                            },
                            (),
                        ));
                    };
                    validate_run_delivery_cleanup_snapshot(&snapshot, &request)?;
                    if !snapshot.records.contains(&record) {
                        return Ok(CasApply::no_op(snapshot, ()));
                    }
                    snapshot.records.retain(|existing| existing != &record);
                    if snapshot.records.is_empty() {
                        return Ok(CasApply::delete(snapshot, ()));
                    }
                    Ok(CasApply::new(snapshot, ()))
                }
            },
        )
        .await
        .map_err(map_cleanup_cas_error)
    }

    async fn put_run_final_reply_target(
        &self,
        record: RunFinalReplyTargetRecord,
    ) -> Result<(), OutboundError> {
        let path = run_final_reply_target_path(record.run_id)?;
        let resource_scope = record.scope.to_resource_scope();
        self.ensure_tenant_id_index(
            &resource_scope,
            &ScopedPath::new(RUN_FINAL_REPLY_TARGETS_ROOT).map_err(|_| OutboundError::Backend)?,
        )
        .await?;
        for _ in 0..MAX_CAS_RETRIES {
            if let Some(existing) = self
                .get_json::<RunFinalReplyTargetRecord>(&resource_scope, &path)
                .await?
            {
                return if existing == record {
                    Ok(())
                } else {
                    Err(OutboundError::InvalidRequest {
                        reason: "final reply target is already sealed for this run",
                    })
                };
            }
            match self
                .put_json(
                    &resource_scope,
                    &path,
                    &record,
                    &record.scope.tenant_id,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }

    async fn load_run_final_reply_target(
        &self,
        request: RunFinalReplyTargetRequest,
    ) -> Result<Option<RunFinalReplyTargetRecord>, OutboundError> {
        let path = run_final_reply_target_path(request.run_id)?;
        let resource_scope = request.scope.to_resource_scope();
        let Some(record) = self
            .get_json::<RunFinalReplyTargetRecord>(&resource_scope, &path)
            .await?
        else {
            return Ok(None);
        };
        if record.run_id != request.run_id
            || record.scope != request.scope
            || record.actor != request.actor
        {
            return Ok(None);
        }
        Ok(Some(record))
    }

    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        validate_policy(&policy)?;
        let path = policy_path(&policy.scope)?;
        let resource_scope = policy.scope.to_resource_scope();
        self.ensure_tenant_id_index(&resource_scope, &policies_root()?)
            .await?;
        self.put_json(
            &resource_scope,
            &path,
            &policy,
            &policy.scope.tenant_id,
            CasExpectation::Any,
        )
        .await
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        let path = policy_path(&scope)?;
        let resource_scope = scope.to_resource_scope();
        match self
            .get_json::<ThreadNotificationPolicy>(&resource_scope, &path)
            .await?
        {
            Some(policy) => Ok(policy),
            None => Ok(ThreadNotificationPolicy::default_for_scope(scope)),
        }
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        validate_subscription_record(&record)?;
        let path = subscription_path(
            &record.subscription_id,
            &record.actor,
            &record.scope,
            &record.thread_id,
        )?;
        // Outbound subscription records carry their full identity in the
        // record body (subscription_id, actor, scope hash); the path is
        // already a tenant-aware sha256 so the per-tenant FS rewrite isn't
        // needed. Route through the system scope.
        let resource_scope = ResourceScope::system();
        self.ensure_tenant_id_index(&resource_scope, &subscriptions_root()?)
            .await?;
        for _ in 0..MAX_CAS_RETRIES {
            let (cas, existing) = match self
                .get_versioned_json::<ProjectionSubscriptionRecord>(&resource_scope, &path)
                .await?
            {
                Some((existing, versioned)) => {
                    (CasExpectation::Version(versioned.version), Some(existing))
                }
                None => (CasExpectation::Absent, None),
            };
            if let Some(existing) = existing.as_ref() {
                validate_subscription_identity(existing, &record)?;
            }
            match self
                .put_json(
                    &resource_scope,
                    &path,
                    &record,
                    &record.scope.stream.tenant_id,
                    cas,
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        let path = subscription_path(
            &request.subscription_id,
            &request.actor,
            &request.scope,
            &request.thread_id,
        )?;
        let resource_scope = ResourceScope::system();
        let Some(record) = self
            .get_json::<ProjectionSubscriptionRecord>(&resource_scope, &path)
            .await?
        else {
            return Ok(None);
        };
        validate_subscription_request(&record, &request)?;
        Ok(record.cursor)
    }

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        let path = subscription_path(
            &request.subscription_id,
            &request.actor,
            &request.cursor.scope,
            &request.thread_id,
        )?;
        let resource_scope = ResourceScope::system();
        self.ensure_tenant_id_index(&resource_scope, &subscriptions_root()?)
            .await?;
        for _ in 0..MAX_CAS_RETRIES {
            let Some((mut record, versioned)) = self
                .get_versioned_json::<ProjectionSubscriptionRecord>(&resource_scope, &path)
                .await?
            else {
                return Err(OutboundError::SubscriptionScopeMismatch);
            };
            validate_advance_request(&record, &request)?;
            record.cursor = Some(request.cursor.clone());
            match self
                .put_json(
                    &resource_scope,
                    &path,
                    &record,
                    &record.scope.stream.tenant_id,
                    CasExpectation::Version(versioned.version),
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        validate_delivery_attempt(&attempt)?;
        let resource_scope = attempt.scope.to_resource_scope();
        self.ensure_delivery_scope_index(&resource_scope).await?;
        self.ensure_tenant_id_index(&resource_scope, &deliveries_root()?)
            .await?;
        let path = delivery_path(&attempt.delivery_id)?;
        for _ in 0..MAX_CAS_RETRIES {
            if let Some(existing) = self
                .get_json::<OutboundDeliveryAttempt>(&resource_scope, &path)
                .await?
            {
                validate_delivery_identity(&existing, &attempt)?;
                return Ok(());
            }
            match self
                .put_delivery_attempt_indexed(
                    &resource_scope,
                    &path,
                    &attempt,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }

    async fn claim_delivery_attempt_for_send(
        &self,
        request: crate::ClaimDeliveryAttemptForSendRequest,
    ) -> Result<bool, OutboundError> {
        let path = delivery_path(&request.delivery_id)?;
        let resource_scope = request.scope.to_resource_scope();
        for _ in 0..MAX_CAS_RETRIES {
            let Some((mut attempt, versioned)) = self
                .get_versioned_json::<OutboundDeliveryAttempt>(&resource_scope, &path)
                .await?
            else {
                return Err(OutboundError::DeliveryNotFound);
            };
            if attempt.scope != request.scope {
                return Err(OutboundError::SubscriptionScopeMismatch);
            }
            if attempt.status != OutboundDeliveryStatus::Prepared {
                return Ok(false);
            }
            attempt.status = OutboundDeliveryStatus::Sending;
            attempt.failure_kind = None;
            let entry = delivery_attempt_entry(&attempt)?;
            // This ownership transition must never use the byte-only
            // last-write-wins fallback: a backend without versioned CAS is
            // incapable of proving sole vendor-egress ownership and must
            // fail closed instead.
            match self
                .filesystem
                .put(
                    &resource_scope,
                    &path,
                    entry,
                    CasExpectation::Version(versioned.version),
                )
                .await
                .map_err(map_fs_error)
            {
                Ok(_) => return Ok(true),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }

    async fn recover_interrupted_delivery_attempt(
        &self,
        request: crate::RecoverInterruptedDeliveryRequest,
    ) -> Result<bool, OutboundError> {
        let path = delivery_path(&request.delivery_id)?;
        let resource_scope = request.scope.to_resource_scope();
        for _ in 0..MAX_CAS_RETRIES {
            let Some((mut attempt, versioned)) = self
                .get_versioned_json::<OutboundDeliveryAttempt>(&resource_scope, &path)
                .await?
            else {
                return Err(OutboundError::DeliveryNotFound);
            };
            if attempt.scope != request.scope {
                return Err(OutboundError::SubscriptionScopeMismatch);
            }
            // Re-verify `Sending` inside the same CAS read the write commits
            // against. A stale recovery list snapshot must never overwrite a
            // terminal result (`Delivered`/`Failed`) that a different worker
            // wrote after completing egress, so recovery no-ops for any attempt
            // that already advanced past `Sending`.
            if attempt.status != OutboundDeliveryStatus::Sending {
                return Ok(false);
            }
            attempt.status = OutboundDeliveryStatus::Unknown;
            attempt.failure_kind = None;
            let entry = delivery_attempt_entry(&attempt)?;
            match self
                .filesystem
                .put(
                    &resource_scope,
                    &path,
                    entry,
                    CasExpectation::Version(versioned.version),
                )
                .await
                .map_err(map_fs_error)
            {
                Ok(_) => return Ok(true),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        validate_delivery_status_request(&request)?;
        let path = delivery_path(&request.delivery_id)?;
        let resource_scope = request.scope.to_resource_scope();
        for _ in 0..MAX_CAS_RETRIES {
            let Some((mut attempt, versioned)) = self
                .get_versioned_json::<OutboundDeliveryAttempt>(&resource_scope, &path)
                .await?
            else {
                return Err(OutboundError::DeliveryNotFound);
            };
            if attempt.scope != request.scope {
                return Err(OutboundError::SubscriptionScopeMismatch);
            }
            attempt.status = request.status;
            attempt.failure_kind = request.failure_kind;
            match self
                .put_delivery_attempt_indexed(
                    &resource_scope,
                    &path,
                    &attempt,
                    CasExpectation::Version(versioned.version),
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        let resource_scope = scope.to_resource_scope();
        self.ensure_delivery_scope_index(&resource_scope).await?;
        let root = deliveries_root()?;
        let filter = Filter::Eq {
            key: delivery_scope_index_key(),
            value: delivery_scope_index_value(&scope),
        };
        let mut deliveries: Vec<OutboundDeliveryAttempt> = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = match self
                .filesystem
                .query(&resource_scope, &root, &filter, page)
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => break,
                Err(error) => return Err(map_fs_error(error)),
            };
            let received = entries.len();
            for versioned in entries {
                let attempt: OutboundDeliveryAttempt =
                    serde_json::from_slice(&versioned.entry.body)
                        .map_err(|_| OutboundError::Serialization)?;
                // Defence-in-depth: the index value is a hash, so distinct
                // scopes hashing to the same bucket is collision-resistant
                // but not impossible. Drop any row whose persisted scope
                // doesn't exactly match the query scope.
                if scope_matches(&attempt.scope, &scope) {
                    deliveries.push(attempt);
                }
            }
            if received < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset.saturating_add(received as u64);
        }
        deliveries.sort_by_key(|attempt| (attempt.attempted_at, attempt.delivery_id.to_string()));
        Ok(deliveries)
    }

    async fn put_run_final_reply_handoff(
        &self,
        record: RunFinalReplyHandoffRecord,
    ) -> Result<(), OutboundError> {
        self.ensure_run_final_reply_handoff_index().await?;
        let resource_scope = ResourceScope::system();
        let path = run_final_reply_handoff_path(&record)?;
        for _ in 0..MAX_CAS_RETRIES {
            if let Some(existing) = self
                .get_json::<RunFinalReplyHandoffRecord>(&resource_scope, &path)
                .await?
            {
                return if existing == record {
                    Ok(())
                } else {
                    Err(OutboundError::InvalidRequest {
                        reason: "final reply handoff event key conflicts with an existing record",
                    })
                };
            }
            let body = serde_json::to_vec(&record).map_err(|_| OutboundError::Serialization)?;
            let entry = Entry::bytes(body)
                .with_content_type(ContentType::json())
                .with_indexed(
                    run_final_reply_handoff_order_index_key()?,
                    IndexValue::Text(run_final_reply_handoff_order_key(&record)),
                )
                .with_indexed(
                    tenant_id_index_key(),
                    tenant_id_index_value(&record.scope.tenant_id),
                );
            match self
                .filesystem
                .put(&resource_scope, &path, entry, CasExpectation::Absent)
                .await
                .map(|_| ())
                .map_err(map_fs_error)
            {
                Ok(()) => return Ok(()),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }

    async fn list_pending_run_final_reply_handoffs(
        &self,
        limit: usize,
    ) -> Result<Vec<RunFinalReplyHandoffRecord>, OutboundError> {
        self.list_pending_run_final_reply_handoffs_after(None, limit)
            .await
    }

    async fn list_pending_run_final_reply_handoffs_after(
        &self,
        after: Option<&RunFinalReplyHandoffRecord>,
        limit: usize,
    ) -> Result<Vec<RunFinalReplyHandoffRecord>, OutboundError> {
        if limit == 0 || limit > MAX_RUN_FINAL_REPLY_HANDOFF_PAGE {
            return Err(OutboundError::InvalidRequest {
                reason: "final reply handoff page limit is invalid",
            });
        }
        self.ensure_run_final_reply_handoff_index().await?;
        let root = run_final_reply_handoffs_root()?;
        let filter = Filter::Range {
            key: run_final_reply_handoff_order_index_key()?,
            lo: IndexValue::Text(
                after
                    .map(run_final_reply_handoff_order_key)
                    .unwrap_or_else(|| "00000000000000000000-".to_string()),
            ),
            hi: IndexValue::Text("~".to_string()),
        };
        let continuation_row = if after.is_some() { 1 } else { 0 };
        let query_limit = limit.saturating_add(continuation_row);
        let page_limit = u32::try_from(query_limit).map_err(|_| OutboundError::InvalidRequest {
            reason: "final reply handoff page limit is invalid",
        })?;
        let entries = self
            .filesystem
            .query(
                &ResourceScope::system(),
                &root,
                &filter,
                Page::first(page_limit),
            )
            .await
            .map_err(map_fs_error)?;
        let mut records = entries
            .into_iter()
            .map(|versioned| {
                serde_json::from_slice::<RunFinalReplyHandoffRecord>(&versioned.entry.body)
                    .map_err(|_| OutboundError::Serialization)
            })
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by_key(|record| (record.event_cursor, record.run_id));
        if let Some(after) = after {
            records.retain(|record| {
                (record.event_cursor, record.run_id) > (after.event_cursor, after.run_id)
            });
        }
        records.truncate(limit);
        Ok(records)
    }

    async fn complete_run_final_reply_handoff(
        &self,
        record: &RunFinalReplyHandoffRecord,
    ) -> Result<(), OutboundError> {
        let resource_scope = ResourceScope::system();
        let path = run_final_reply_handoff_path(record)?;
        let Some(existing) = self
            .get_json::<RunFinalReplyHandoffRecord>(&resource_scope, &path)
            .await?
        else {
            return Ok(());
        };
        if existing != *record {
            return Err(OutboundError::Backend);
        }
        match self.filesystem.delete(&resource_scope, &path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_fs_error(error)),
        }
    }

    async fn load_run_final_reply_handoff_cursor(&self) -> Result<TurnEventCursor, OutboundError> {
        let path = run_final_reply_handoff_cursor_path()?;
        Ok(self
            .get_json::<RunFinalReplyHandoffCursorRecord>(&ResourceScope::system(), &path)
            .await?
            .map(|record| record.event_cursor)
            .unwrap_or_default())
    }

    async fn advance_run_final_reply_handoff_cursor(
        &self,
        cursor: TurnEventCursor,
    ) -> Result<(), OutboundError> {
        let resource_scope = ResourceScope::system();
        let path = run_final_reply_handoff_cursor_path()?;
        for _ in 0..MAX_CAS_RETRIES {
            let (current, cas) = match self
                .get_versioned_json::<RunFinalReplyHandoffCursorRecord>(&resource_scope, &path)
                .await?
            {
                Some((record, versioned)) => {
                    if record.event_cursor >= cursor {
                        return Ok(());
                    }
                    (
                        record.event_cursor,
                        CasExpectation::Version(versioned.version),
                    )
                }
                None => (TurnEventCursor::default(), CasExpectation::Absent),
            };
            let record = RunFinalReplyHandoffCursorRecord {
                event_cursor: current.max(cursor),
            };
            let body = serde_json::to_vec(&record).map_err(|_| OutboundError::Serialization)?;
            let entry = Entry::bytes(body).with_content_type(ContentType::json());
            match self
                .filesystem
                .put(&resource_scope, &path, entry, cas)
                .await
                .map(|_| ())
                .map_err(map_fs_error)
            {
                Ok(()) => return Ok(()),
                Err(OutboundError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(OutboundError::Backend)
    }
}

fn policy_path(scope: &TurnScope) -> Result<ScopedPath, OutboundError> {
    let key = thread_scope_key(scope);
    ScopedPath::new(format!("/outbound/policies/{key}.json")).map_err(|_| OutboundError::Backend)
}

fn run_final_reply_target_path(
    run_id: ironclaw_turns::TurnRunId,
) -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(format!("{RUN_FINAL_REPLY_TARGETS_ROOT}/{run_id}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn run_delivery_cleanup_path(
    request: &RunDeliveryCleanupRequest,
) -> Result<ScopedPath, OutboundError> {
    let serialized = serde_json::to_vec(request).map_err(|_| OutboundError::Serialization)?;
    let digest = hex::encode(Sha256::digest(serialized));
    ScopedPath::new(format!("{RUN_DELIVERY_CLEANUP_ROOT}/{digest}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn run_delivery_cleanup_root() -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(RUN_DELIVERY_CLEANUP_ROOT).map_err(|_| OutboundError::Backend)
}

fn decode_run_delivery_cleanup_snapshot(
    bytes: &[u8],
) -> Result<RunDeliveryCleanupSnapshot, OutboundError> {
    serde_json::from_slice(bytes).map_err(|_| OutboundError::Serialization)
}

fn validate_run_delivery_cleanup_snapshot(
    snapshot: &RunDeliveryCleanupSnapshot,
    request: &RunDeliveryCleanupRequest,
) -> Result<(), OutboundError> {
    if &snapshot.request != request {
        return Err(OutboundError::AccessDenied);
    }
    if snapshot.records.len() > MAX_RUN_DELIVERY_CLEANUP_RECORDS
        || snapshot
            .records
            .iter()
            .any(|record| record.request() != snapshot.request || record.validate().is_err())
    {
        return Err(OutboundError::Serialization);
    }
    Ok(())
}

fn encode_run_delivery_cleanup_snapshot(
    snapshot: &RunDeliveryCleanupSnapshot,
) -> Result<Entry, OutboundError> {
    let body = serde_json::to_vec(snapshot).map_err(|_| OutboundError::Serialization)?;
    Ok(Entry::bytes(body)
        .with_content_type(ContentType::json())
        .with_indexed(
            tenant_id_index_key(),
            tenant_id_index_value(&snapshot.request.scope.tenant_id),
        ))
}

fn map_cleanup_cas_error(error: CasUpdateError<OutboundError>) -> OutboundError {
    match error {
        CasUpdateError::Apply(error) => error,
        CasUpdateError::Timeout
        | CasUpdateError::RetriesExhausted
        | CasUpdateError::CasUnsupported
        | CasUpdateError::Backend(_) => OutboundError::Backend,
    }
}

fn run_final_reply_handoffs_root() -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(RUN_FINAL_REPLY_HANDOFFS_ROOT).map_err(|_| OutboundError::Backend)
}

fn run_final_reply_handoff_cursor_path() -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(RUN_FINAL_REPLY_HANDOFF_CURSOR_PATH).map_err(|_| OutboundError::Backend)
}

fn run_final_reply_handoff_path(
    record: &RunFinalReplyHandoffRecord,
) -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(format!(
        "{RUN_FINAL_REPLY_HANDOFFS_ROOT}/{:020}-{}.json",
        record.event_cursor.0, record.run_id
    ))
    .map_err(|_| OutboundError::Backend)
}

fn run_final_reply_handoff_order_index_key() -> Result<IndexKey, OutboundError> {
    IndexKey::new(RUN_FINAL_REPLY_HANDOFF_ORDER_INDEX_KEY).map_err(|_| OutboundError::Backend)
}

fn run_final_reply_handoff_order_key(record: &RunFinalReplyHandoffRecord) -> String {
    format!("{:020}-{}", record.event_cursor.0, record.run_id)
}

fn subscription_path(
    subscription_id: &ProjectionSubscriptionId,
    actor: &TurnActor,
    scope: &ProjectionScope,
    thread_id: &ThreadId,
) -> Result<ScopedPath, OutboundError> {
    #[derive(Serialize)]
    struct SubscriptionIdentity<'a> {
        subscription_id: &'a ProjectionSubscriptionId,
        actor: &'a TurnActor,
        scope: &'a ProjectionScope,
        thread_id: &'a ThreadId,
    }
    let identity = SubscriptionIdentity {
        subscription_id,
        actor,
        scope,
        thread_id,
    };
    let serialized = serde_json::to_string(&identity).map_err(|_| OutboundError::Serialization)?;
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    let digest = hex::encode(hasher.finalize());
    ScopedPath::new(format!("/outbound/subscriptions/{digest}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn delivery_path(delivery_id: &OutboundDeliveryId) -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(format!("/outbound/deliveries/{delivery_id}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn delivery_attempt_entry(attempt: &OutboundDeliveryAttempt) -> Result<Entry, OutboundError> {
    let body = serde_json::to_vec(attempt).map_err(|_| OutboundError::Serialization)?;
    Ok(Entry::bytes(body)
        .with_content_type(ContentType::json())
        .with_indexed(
            delivery_scope_index_key(),
            delivery_scope_index_value(&attempt.scope),
        )
        .with_indexed(
            tenant_id_index_key(),
            tenant_id_index_value(&attempt.scope.tenant_id),
        ))
}

fn delivered_gate_route_path(
    tenant_id: &TenantId,
    user_id: &UserId,
    gate_ref: &str,
) -> Result<ScopedPath, OutboundError> {
    let mut hasher = Sha256::new();
    update_hash_part(&mut hasher, tenant_id.as_str());
    update_hash_part(&mut hasher, user_id.as_str());
    update_hash_part(&mut hasher, gate_ref);
    let digest = hex::encode(hasher.finalize());
    ScopedPath::new(format!("{DELIVERED_GATE_ROUTES_ROOT}/{digest}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn delivered_gate_route_conv_idx_path(
    tenant_id: &TenantId,
    user_id: &UserId,
    conversation_fingerprint: &str,
) -> Result<ScopedPath, OutboundError> {
    let mut hasher = Sha256::new();
    update_hash_part(&mut hasher, tenant_id.as_str());
    update_hash_part(&mut hasher, user_id.as_str());
    update_hash_part(&mut hasher, conversation_fingerprint);
    let digest = hex::encode(hasher.finalize());
    ScopedPath::new(format!(
        "{DELIVERED_GATE_ROUTES_CONV_IDX_ROOT}/{digest}.json"
    ))
    .map_err(|_| OutboundError::Backend)
}

/// Outcome of a [`FilesystemOutboundStateStore::retry_conv_idx`] merge
/// callback: either write back an updated route list or delete the now-empty
/// index file.
enum ConvIdxUpdate {
    Write(Vec<DeliveredGateRouteConversationIndexRouteEntry>),
    Delete,
}

/// Identity triple for one route entry in a conversation index file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DeliveredGateRouteConversationIndexRouteEntry {
    tenant_id: TenantId,
    user_id: UserId,
    gate_ref: String,
}

/// Value stored in a conversation index file.
///
/// The v2 format stores a `routes` array so a single conversation fingerprint
/// can map to multiple pending gates. The v1 format stored the fields at the
/// top level (a single `{tenant_id, user_id, gate_ref}` object). Both formats
/// are accepted on read for backward-compatible rehydration of existing records;
/// only the v2 format is written.
///
/// Wire rule: snake_case, no rename. New field additions must `#[serde(default)]`.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum DeliveredGateRouteConversationIndexFile {
    /// v2: one-to-many, written by this code.
    V2 {
        routes: Vec<DeliveredGateRouteConversationIndexRouteEntry>,
    },
    /// v1: single-entry, written by code prior to one-to-many upgrade.
    V1(DeliveredGateRouteConversationIndexRouteEntry),
}

impl DeliveredGateRouteConversationIndexFile {
    fn into_routes(self) -> Vec<DeliveredGateRouteConversationIndexRouteEntry> {
        match self {
            Self::V2 { routes } => routes,
            Self::V1(entry) => vec![entry],
        }
    }

    fn from_routes(routes: Vec<DeliveredGateRouteConversationIndexRouteEntry>) -> Self {
        Self::V2 { routes }
    }
}

fn communication_preference_path(
    key: &CommunicationPreferenceKey,
) -> Result<ScopedPath, OutboundError> {
    let mut hasher = Sha256::new();
    hasher.update(b"v2:");
    hash_delivery_default_scope(&mut hasher, &key.scope);
    let digest = hex::encode(hasher.finalize());
    ScopedPath::new(format!("{COMMUNICATION_PREFERENCES_ROOT}/{digest}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn hash_delivery_default_scope(hasher: &mut Sha256, scope: &DeliveryDefaultScope) {
    match scope {
        DeliveryDefaultScope::Personal { tenant_id, user_id } => {
            update_hash_part(hasher, "personal");
            update_hash_part(hasher, tenant_id.as_str());
            update_hash_part(hasher, user_id.as_str());
        }
        DeliveryDefaultScope::SharedAgent {
            tenant_id,
            agent_id,
            project_id,
        } => {
            update_hash_part(hasher, "shared_agent");
            update_hash_part(hasher, tenant_id.as_str());
            update_hash_part(hasher, agent_id.as_str());
            match project_id {
                Some(project_id) => {
                    update_hash_part(hasher, "project");
                    update_hash_part(hasher, project_id.as_str());
                }
                None => update_hash_part(hasher, "no_project"),
            }
        }
    }
}

fn update_hash_part(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn communication_preference_resource_scope(scope: &DeliveryDefaultScope) -> ResourceScope {
    let mut resource_scope = ResourceScope::system();
    match scope {
        DeliveryDefaultScope::Personal { tenant_id, user_id } => {
            resource_scope.tenant_id = tenant_id.clone();
            resource_scope.user_id = user_id.clone();
        }
        DeliveryDefaultScope::SharedAgent {
            tenant_id,
            agent_id,
            project_id,
        } => {
            resource_scope.tenant_id = tenant_id.clone();
            resource_scope.agent_id = Some(agent_id.clone());
            resource_scope.project_id = project_id.clone();
        }
    }
    resource_scope
}

fn deliveries_root() -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(DELIVERIES_ROOT).map_err(|_| OutboundError::Backend)
}

fn communication_preferences_root() -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(COMMUNICATION_PREFERENCES_ROOT).map_err(|_| OutboundError::Backend)
}

fn policies_root() -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(POLICIES_ROOT).map_err(|_| OutboundError::Backend)
}

fn subscriptions_root() -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(SUBSCRIPTIONS_ROOT).map_err(|_| OutboundError::Backend)
}

fn tenant_id_index_key() -> IndexKey {
    // safety: `TENANT_ID_INDEX_KEY` is the constant identifier `"tenant_id"`,
    // a simple `[A-Za-z_][A-Za-z0-9_]*` identifier — `IndexKey::new`
    // cannot fail on this input.
    static KEY: std::sync::OnceLock<IndexKey> = std::sync::OnceLock::new();
    KEY.get_or_init(|| match IndexKey::new(TENANT_ID_INDEX_KEY) {
        Ok(key) => key,
        Err(_) => unreachable!(
            "TENANT_ID_INDEX_KEY must satisfy IndexKey::new grammar — \
             update the constant or grammar"
        ),
    })
    .clone()
}

fn tenant_id_index_value(tenant: &TenantId) -> IndexValue {
    IndexValue::Text(tenant.as_str().to_string())
}

fn delivery_scope_index_key() -> IndexKey {
    // safety: `DELIVERY_SCOPE_INDEX_KEY` is the constant identifier `"scope"`,
    // which is statically known to satisfy `IndexKey::new`'s
    // `[A-Za-z_][A-Za-z0-9_]*` grammar; if the grammar ever changes such that
    // this constructor fails, the regression surfaces at the first call site
    // of this function (covered by every CAS/index test in this crate).
    static KEY: std::sync::OnceLock<IndexKey> = std::sync::OnceLock::new();
    KEY.get_or_init(|| match IndexKey::new(DELIVERY_SCOPE_INDEX_KEY) {
        Ok(key) => key,
        Err(_) => unreachable!(
            "DELIVERY_SCOPE_INDEX_KEY must satisfy IndexKey::new grammar — \
             update the constant or grammar"
        ),
    })
    .clone()
}

#[async_trait]
impl<F> TriggeredRunDeliveryStore for FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    async fn record_triggered_run_delivery(
        &self,
        record: TriggeredRunDeliveryRecord,
    ) -> Result<(), String> {
        let run_id_str = record.run_id.to_string();
        let path = ScopedPath::new(format!("{TRIGGERED_RUN_DELIVERY_ROOT}/{run_id_str}.json"))
            .map_err(|e| format!("triggered run delivery path: {e}"))?;
        // Delivery records are written once per run and are tenant-scoped by
        // the filesystem mount. Use system scope so the put is always
        // authorized regardless of which user-scoped mount is active.
        let resource_scope = ResourceScope::system();
        let body = serde_json::to_vec(&record)
            .map_err(|e| format!("triggered run delivery serialize: {e}"))?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.put_with_byte_fallback(&resource_scope, &path, entry, CasExpectation::Any)
            .await
            .map_err(|e| format!("triggered run delivery write: {e}"))
    }

    async fn load_triggered_run_delivery(
        &self,
        run_id: ironclaw_turns::TurnRunId,
    ) -> Result<Option<TriggeredRunDeliveryRecord>, String> {
        let run_id_str = run_id.to_string();
        let path = ScopedPath::new(format!("{TRIGGERED_RUN_DELIVERY_ROOT}/{run_id_str}.json"))
            .map_err(|e| format!("triggered run delivery path: {e}"))?;
        let resource_scope = ResourceScope::system();
        match self
            .filesystem
            .get(&resource_scope, &path)
            .await
            .map_err(|e| format!("triggered run delivery read: {e}"))?
        {
            Some(versioned) => {
                let record: TriggeredRunDeliveryRecord =
                    serde_json::from_slice(&versioned.entry.body)
                        .map_err(|e| format!("triggered run delivery deserialize: {e}"))?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }
}

#[async_trait]
impl<F> DeliveredGateRouteStore for FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    async fn record_delivered_gate_route(
        &self,
        record: DeliveredGateRouteRecord,
    ) -> Result<(), String> {
        let path = delivered_gate_route_path(&record.tenant_id, &record.user_id, &record.gate_ref)
            .map_err(|e| format!("delivered gate route path: {e}"))?;
        let resource_scope =
            delivered_gate_route_resource_scope(&record.tenant_id, &record.user_id);
        let old_record = self
            .get_json::<DeliveredGateRouteRecord>(&resource_scope, &path)
            .await
            .map_err(|e| format!("delivered gate route read old record: {e}"))?;
        let body = serde_json::to_vec(&record)
            .map_err(|e| format!("delivered gate route serialize: {e}"))?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.put_with_byte_fallback(&resource_scope, &path, entry, CasExpectation::Any)
            .await
            .map_err(|e| format!("delivered gate route write: {e}"))?;
        self.write_delivered_gate_route_conversation_indexes(&record)
            .await?;
        // Stale-index cleanup runs LAST (write-then-cleanup): a crash earlier
        // in this sequence leaves either the old index or a dangling one, and
        // the lookup's membership check turns a dangling index into a
        // harmless miss. Deleting first would instead leave the gate with no
        // conversation index at all, silently disabling bare-reply routing.
        if let Some(old_record) = old_record {
            let new_fingerprints: std::collections::HashSet<String> = record
                .delivered_conversation_fingerprints
                .iter()
                .cloned()
                .collect();
            let stale_fingerprints: Vec<String> = old_record
                .delivered_conversation_fingerprints
                .iter()
                .filter(|&fingerprint| !new_fingerprints.contains(fingerprint))
                .cloned()
                .collect();
            self.delete_delivered_gate_route_conversation_indexes(&old_record, &stale_fingerprints)
                .await?;
        }
        Ok(())
    }

    async fn load_delivered_gate_route(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        gate_ref: &str,
    ) -> Result<Option<DeliveredGateRouteRecord>, String> {
        let path = delivered_gate_route_path(tenant_id, user_id, gate_ref)
            .map_err(|e| format!("delivered gate route path: {e}"))?;
        let resource_scope = delivered_gate_route_resource_scope(tenant_id, user_id);
        match self
            .filesystem
            .get(&resource_scope, &path)
            .await
            .map_err(|e| format!("delivered gate route read: {e}"))?
        {
            Some(versioned) => {
                let record: DeliveredGateRouteRecord =
                    serde_json::from_slice(&versioned.entry.body)
                        .map_err(|e| format!("delivered gate route deserialize: {e}"))?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    async fn load_delivered_gate_route_by_conversation_fingerprint(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        conversation_fingerprint: &str,
    ) -> Result<Vec<DeliveredGateRouteRecord>, String> {
        let idx_path =
            delivered_gate_route_conv_idx_path(tenant_id, user_id, conversation_fingerprint)
                .map_err(|e| format!("delivered gate route conversation index path: {e}"))?;
        // Only the index file is read with the system scope (its path hash
        // binds the (tenant, user)). Primary records are reloaded through
        // load_delivered_gate_route so reads use the same tenant+user
        // resource scope as the write path.
        let resource_scope = ResourceScope::system();
        let index_file = match self
            .get_json::<DeliveredGateRouteConversationIndexFile>(&resource_scope, &idx_path)
            .await
            .map_err(|e| format!("delivered gate route conversation index read: {e}"))?
        {
            Some(index_file) => index_file,
            None => return Ok(Vec::new()),
        };
        let entries = index_file.into_routes();
        // Cap as defense-in-depth. The index is now per-(tenant, user),
        // so legitimate overflow is implausible; the cap guards against a
        // corrupt or adversarially written index file.
        let entries = entries
            .into_iter()
            .take(crate::delivered_gate_routes::DELIVERED_GATE_ROUTE_CONVERSATION_LOOKUP_CAP);
        let mut records = Vec::new();
        for entry in entries {
            // Tenant + user defense-in-depth: the index path already binds
            // (tenant, user) in its hash, so a mismatch here means the file
            // was written by a different key version or was corrupted. Skip.
            if entry.tenant_id != *tenant_id || entry.user_id != *user_id {
                continue;
            }
            let record = match self
                .load_delivered_gate_route(&entry.tenant_id, &entry.user_id, &entry.gate_ref)
                .await?
            {
                Some(record) => record,
                None => continue,
            };
            // Membership check: a stale index (crash between record write and
            // index cleanup) may point at a record that no longer lists this
            // conversation. Treat it as a miss rather than routing a reply from
            // a conversation the gate was never delivered to.
            if !record
                .delivered_conversation_fingerprints
                .iter()
                .any(|delivered| delivered == conversation_fingerprint)
            {
                continue;
            }
            records.push(record);
        }
        Ok(records)
    }

    async fn remove_delivered_gate_route(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        gate_ref: &str,
    ) -> Result<(), String> {
        let path = delivered_gate_route_path(tenant_id, user_id, gate_ref)
            .map_err(|e| format!("delivered gate route path: {e}"))?;
        let resource_scope = delivered_gate_route_resource_scope(tenant_id, user_id);
        // Read the old record first so we know which conversation indexes to
        // clean up. Then delete the primary record, then remove the indexes.
        //
        // This order is deliberate: a crash between the primary delete and the
        // index cleanup leaves a dangling index entry pointing at a record that
        // no longer exists. The lookup's membership check turns that into a
        // harmless miss. The inverse order (delete indexes first) would leave a
        // primary record with no index, silently disabling bare-reply routing —
        // the worse failure mode.
        let old_record = self
            .get_json::<DeliveredGateRouteRecord>(&resource_scope, &path)
            .await
            .map_err(|e| format!("delivered gate route read before delete: {e}"))?;
        match self.filesystem.delete(&resource_scope, &path).await {
            Ok(()) => {}
            Err(FilesystemError::NotFound { .. }) => {}
            Err(e) => return Err(format!("delivered gate route delete: {e}")),
        }
        if let Some(old_record) = old_record {
            let fingerprints: Vec<String> = old_record.delivered_conversation_fingerprints.to_vec();
            self.delete_delivered_gate_route_conversation_indexes(&old_record, &fingerprints)
                .await?;
        }
        Ok(())
    }

    /// Sweep expired route records from the filesystem store.
    ///
    /// # Scope limitation
    ///
    /// The filesystem store is constructed with a per-(tenant, user) mount
    /// view. This sweep can only list and delete files that are reachable
    /// through the mount aliases on the current filesystem instance — i.e.,
    /// files belonging to the tenant + user the store was created for. It
    /// cannot enumerate records belonging to other tenant/user combinations.
    /// The opportunistic sweep on write is therefore scoped to the triggering
    /// user; a background sweep covering all users would require a separate
    /// admin-scoped store or a direct backend scan. This limitation is
    /// accepted for now.
    ///
    /// Best-effort per-file: one unreadable or undeserializable file does not
    /// abort the sweep. A missing directory returns `Ok(0)`.
    async fn sweep_expired_delivered_gate_routes(
        &self,
        now: DateTime<Utc>,
    ) -> Result<usize, String> {
        let root = ScopedPath::new(DELIVERED_GATE_ROUTES_ROOT)
            .map_err(|e| format!("delivered gate route sweep root path: {e}"))?;
        // Use the system resource scope so the mount resolver picks up the
        // tenant-scoped virtual root; the isolation boundary is enforced by
        // the mount, not by a specific user_id in the ResourceScope here.
        let resource_scope = ResourceScope::system();
        let entries = match self.filesystem.list_dir(&resource_scope, &root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(0),
            Err(e) => return Err(format!("delivered gate route sweep list_dir: {e}")),
        };

        let mut removed = 0usize;
        for entry in entries {
            if entry.file_type != FileType::File {
                continue;
            }
            // Reconstruct the ScopedPath from the file name.
            let file_path =
                match ScopedPath::new(format!("{DELIVERED_GATE_ROUTES_ROOT}/{}", entry.name)) {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::debug!(
                            target = "ironclaw::outbound::filesystem_store",
                            name = %entry.name,
                            "delivered gate route sweep: skipping entry with invalid scoped path"
                        );
                        continue;
                    }
                };
            // Read the file; skip if unreadable.
            let versioned = match self.filesystem.get(&resource_scope, &file_path).await {
                Ok(Some(v)) => v,
                Ok(None) => continue,
                Err(e) => {
                    tracing::debug!(
                        target = "ironclaw::outbound::filesystem_store",
                        name = %entry.name,
                        error = %e,
                        "delivered gate route sweep: skipping unreadable file"
                    );
                    continue;
                }
            };
            // Deserialize; skip if undeserializable.
            let record: crate::DeliveredGateRouteRecord =
                match serde_json::from_slice(&versioned.entry.body) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::debug!(
                            target = "ironclaw::outbound::filesystem_store",
                            name = %entry.name,
                            error = %e,
                            "delivered gate route sweep: skipping undeserializable file"
                        );
                        continue;
                    }
                };
            if !record.is_expired(now) {
                continue;
            }
            let fingerprints: Vec<String> = record.delivered_conversation_fingerprints.to_vec();
            if let Err(e) = self
                .delete_delivered_gate_route_conversation_indexes(&record, &fingerprints)
                .await
            {
                // silent-ok: stale index entries are filtered by the membership
                // check and re-swept next pass.
                tracing::debug!(
                    target = "ironclaw::outbound::filesystem_store",
                    name = %entry.name,
                    error = %e,
                    "delivered gate route sweep: failed to delete conversation indexes (best-effort)"
                );
            }
            // Delete the expired file.
            match self.filesystem.delete(&resource_scope, &file_path).await {
                Ok(()) => removed += 1,
                Err(FilesystemError::NotFound { .. }) => {
                    // Already gone — count it as removed.
                    removed += 1;
                }
                Err(e) => {
                    // silent-ok: the record will be re-visited on the next sweep
                    // and filtered out at lookup time (is_expired check).
                    tracing::debug!(
                        target = "ironclaw::outbound::filesystem_store",
                        name = %entry.name,
                        error = %e,
                        "delivered gate route sweep: failed to delete expired file (best-effort)"
                    );
                }
            }
        }
        Ok(removed)
    }
}

/// Resource scope for delivered-gate route records. Carries the real tenant
/// and user so the mount's path-prefix isolation applies structurally, on top
/// of the hash-keyed file name (which also binds tenant + user + gate_ref).
fn delivered_gate_route_resource_scope(tenant_id: &TenantId, user_id: &UserId) -> ResourceScope {
    let mut resource_scope = ResourceScope::system();
    resource_scope.tenant_id = tenant_id.clone();
    resource_scope.user_id = user_id.clone();
    resource_scope
}

fn delivery_scope_index_value(scope: &TurnScope) -> IndexValue {
    // Reuse `thread_scope_key`'s hash so the same scope hashes consistently
    // across the policy path and the delivery scope index. The hash is
    // collision-resistant against the legal-id grammar, and the F6 sentinel
    // fix guarantees `None` agent/project no longer collide with literal ids.
    IndexValue::Text(thread_scope_key(scope))
}

/// Sentinel used in [`thread_scope_key`] to distinguish `agent_id = None` /
/// `project_id = None` from a literal id value. `\x1F` (ASCII unit-separator)
/// is a control character; `validate_scope_id` in `ironclaw_host_api` rejects
/// every ASCII control character via `has_forbidden_control`, so no real
/// `AgentId` / `ProjectId` can ever contain it. Using the previous `"_"`
/// sentinel collided with a legal id of literally `"_"` (audit finding F6),
/// silently hashing two distinct scopes to the same key.
const SCOPE_NONE_SENTINEL: &str = "\x1f";

fn thread_scope_key(scope: &TurnScope) -> String {
    let agent = scope
        .agent_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| SCOPE_NONE_SENTINEL.to_string());
    let project = scope
        .project_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| SCOPE_NONE_SENTINEL.to_string());
    let serialized = format!(
        "{}|{}|{}|{}",
        scope.tenant_id, agent, project, scope.thread_id
    );
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    hex::encode(hasher.finalize())
}

fn scope_matches(left: &TurnScope, right: &TurnScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.thread_id == right.thread_id
}

fn map_fs_error(error: FilesystemError) -> OutboundError {
    // The outbound CLAUDE.md guardrails forbid leaking backend error detail
    // strings. The FilesystemError variants already keep host paths internal,
    // but we collapse to OutboundError::Backend to honour the crate's no-leak
    // contract. The one exception is `VersionMismatch`: read-then-write paths
    // need a typed conflict variant so the bounded retry loop can match on it
    // discriminator-wise (audit finding F5).
    match error {
        FilesystemError::VersionMismatch { .. } => OutboundError::CasConflict,
        _ => OutboundError::Backend,
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // contract tests construct the store under test
mod tests {
    use std::sync::Arc;

    use chrono::{Duration, Utc};
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
        ThreadId, UserId, VirtualPath,
    };
    use ironclaw_turns::{TurnRunId, TurnScope};

    use super::{
        DeliveredGateRouteConversationIndexFile, FilesystemOutboundStateStore, SCOPE_NONE_SENTINEL,
        thread_scope_key,
    };
    use crate::{DeliveredGateRouteRecord, DeliveredGateRouteStore};

    /// Build a `ScopedFilesystem<InMemoryBackend>` with full permissions on the
    /// `/outbound` alias, mapped to a fixed tenant+user-scoped virtual root.
    /// The `target_root` mirrors how composition wires the outbound filesystem
    /// mount.
    fn build_scoped_fs_for_gate_routes(
        backend: Arc<InMemoryBackend>,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let target_root = format!(
            "/engine/tenants/{}/users/{}/outbound",
            tenant_id.as_str(),
            user_id.as_str()
        );
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/outbound").expect("alias"),
            VirtualPath::new(&target_root).expect("virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    /// Build a `FilesystemOutboundStateStore` backed by an `InMemoryBackend`
    /// for testing the `DeliveredGateRouteStore` implementation.
    fn build_gate_route_store(
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> FilesystemOutboundStateStore<InMemoryBackend> {
        let backend = Arc::new(InMemoryBackend::new());
        FilesystemOutboundStateStore::new(build_scoped_fs_for_gate_routes(
            backend, tenant_id, user_id,
        ))
    }

    /// Build a minimal `DeliveredGateRouteRecord` for the given identities.
    fn gate_route_record(
        tenant_id: TenantId,
        user_id: UserId,
        gate_ref: &str,
        run_id: TurnRunId,
        scope: TurnScope,
    ) -> DeliveredGateRouteRecord {
        DeliveredGateRouteRecord {
            tenant_id,
            user_id,
            gate_ref: gate_ref.to_string(),
            run_id,
            scope,
            recorded_at: Utc::now(),
            delivered_conversation_fingerprints: Vec::new(),
        }
    }

    #[tokio::test]
    async fn filesystem_gate_route_store_round_trip_record_load_remove() {
        // Test: record → load (hash-path + JSON round-trip) → remove → load returns None.
        // Also verifies that removing a missing record is Ok (idempotent).
        let tenant_id = TenantId::new("fs-gate-route-tenant").expect("tenant");
        let user_id = UserId::new("fs-gate-route-user").expect("user");
        let agent_id = AgentId::new("fs-gate-route-agent").expect("agent");
        let thread_id = ThreadId::new("fs-gate-route-thread").expect("thread");
        let gate_ref = "gate:fs-route-test-001";

        let store = build_gate_route_store(&tenant_id, &user_id);

        let run_id = TurnRunId::new();
        // Use a TurnScope with explicit owner to mirror triggered-run delivery.
        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(agent_id),
            None,
            thread_id,
            Some(user_id.clone()),
        );
        let record = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            gate_ref,
            run_id,
            scope.clone(),
        );

        // 1. remove of a missing record must be Ok (idempotent).
        store
            .remove_delivered_gate_route(&tenant_id, &user_id, gate_ref)
            .await
            .expect("remove of absent record must be Ok");

        // 2. Record → load round-trip (hash path + JSON encoding).
        store
            .record_delivered_gate_route(record.clone())
            .await
            .expect("record must succeed");

        let loaded = store
            .load_delivered_gate_route(&tenant_id, &user_id, gate_ref)
            .await
            .expect("load must not error")
            .expect("record must be present after recording");

        assert_eq!(loaded.tenant_id, tenant_id, "tenant_id round-trips");
        assert_eq!(loaded.user_id, user_id, "user_id round-trips");
        assert_eq!(loaded.gate_ref, gate_ref, "gate_ref round-trips");
        assert_eq!(loaded.run_id, run_id, "run_id round-trips");
        assert_eq!(
            loaded.scope.thread_id, scope.thread_id,
            "scope thread_id round-trips"
        );

        // 3. remove → load returns None.
        store
            .remove_delivered_gate_route(&tenant_id, &user_id, gate_ref)
            .await
            .expect("remove must succeed");

        let after_remove = store
            .load_delivered_gate_route(&tenant_id, &user_id, gate_ref)
            .await
            .expect("load after remove must not error");
        assert!(after_remove.is_none(), "record must be absent after remove");

        // 4. Idempotent second remove is also Ok.
        store
            .remove_delivered_gate_route(&tenant_id, &user_id, gate_ref)
            .await
            .expect("second remove of absent record must be Ok");
    }

    fn gate_route_conversation_fingerprint(thread_id: &str) -> String {
        format!("fingerprint:{thread_id}")
    }

    #[tokio::test]
    async fn filesystem_gate_route_conversation_lookup_round_trip() {
        // Test: record with a delivered conversation ref → lookup by that ref
        // returns the record; lookup by an undelivered ref returns None;
        // remove cleans the index so the lookup misses afterwards.
        let tenant_id = TenantId::new("fs-conv-idx-tenant").expect("tenant");
        let user_id = UserId::new("fs-conv-idx-user").expect("user");
        let thread_id = ThreadId::new("fs-conv-idx-thread").expect("thread");
        let gate_ref = "gate:fs-conv-idx-001";
        let conv_a = gate_route_conversation_fingerprint("thread-a");
        let conv_b = gate_route_conversation_fingerprint("thread-b");

        let store = build_gate_route_store(&tenant_id, &user_id);
        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );
        let mut record = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            gate_ref,
            TurnRunId::new(),
            scope,
        );
        record.delivered_conversation_fingerprints = vec![conv_a.clone()];

        store
            .record_delivered_gate_route(record.clone())
            .await
            .expect("record must succeed");

        let hits = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_a)
            .await
            .expect("lookup must not error");
        assert_eq!(
            hits.len(),
            1,
            "delivered conversation must resolve to the record"
        );
        assert_eq!(hits[0].gate_ref, gate_ref);
        assert_eq!(hits[0].user_id, user_id);

        let miss = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_b)
            .await
            .expect("lookup must not error");
        assert!(miss.is_empty(), "undelivered conversation must miss");

        store
            .remove_delivered_gate_route(&tenant_id, &user_id, gate_ref)
            .await
            .expect("remove must succeed");
        let after_remove = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_a)
            .await
            .expect("lookup must not error");
        assert!(after_remove.is_empty(), "remove must clean the index");
    }

    #[tokio::test]
    async fn filesystem_gate_route_stale_conversation_index_is_harmless_miss() {
        // Simulates a crash between primary-record write and stale-index
        // cleanup: an index file points at a record that no longer lists the
        // indexed conversation. The membership check must turn that dangling
        // index into a miss, not a route.
        let tenant_id = TenantId::new("fs-stale-idx-tenant").expect("tenant");
        let user_id = UserId::new("fs-stale-idx-user").expect("user");
        let thread_id = ThreadId::new("fs-stale-idx-thread").expect("thread");
        let gate_ref = "gate:fs-stale-idx-001";
        let conv_a = gate_route_conversation_fingerprint("thread-a");
        let conv_b = gate_route_conversation_fingerprint("thread-b");

        let store = build_gate_route_store(&tenant_id, &user_id);
        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );
        let mut record = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            gate_ref,
            TurnRunId::new(),
            scope,
        );
        record.delivered_conversation_fingerprints = vec![conv_a.clone()];

        store
            .record_delivered_gate_route(record.clone())
            .await
            .expect("record must succeed");

        // Forge a dangling index: same identity triple, but pointing the
        // conv_b fingerprint at a primary record that only lists conv_a.
        let mut doctored = record.clone();
        doctored.delivered_conversation_fingerprints = vec![conv_b.clone()];
        store
            .write_delivered_gate_route_conversation_indexes(&doctored)
            .await
            .expect("index write must succeed");

        let stale = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_b)
            .await
            .expect("lookup must not error");
        assert!(
            stale.is_empty(),
            "dangling index must be a miss, not a route"
        );

        // The legitimate conversation still resolves.
        let hit = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_a)
            .await
            .expect("lookup must not error");
        assert!(!hit.is_empty(), "delivered conversation must still resolve");
    }

    #[tokio::test]
    async fn filesystem_gate_route_post_write_pre_cleanup_dangling_index_is_harmless_miss() {
        // Simulates the crash window inside `remove_delivered_gate_route`:
        // the primary record has been deleted (route B is gone) but the OLD
        // conversation-index for conversation A still points at it.  A lookup
        // by conversation A must return empty (membership check: record is
        // absent → miss), and a lookup by conversation B (the one the updated
        // record was delivered to) must also return empty after the primary is
        // gone.
        //
        // Concrete scenario:
        //   1. Record route for (tenant, user, gate) delivered to conv_a.
        //   2. Update the same key to conv_b; conv_a index is cleaned up by the
        //      store (normal overwrite path).
        //   3. Simulate the crash: raw-write the OLD conv_a index back so it
        //      again points at the record (which now only lists conv_b).
        //   4. Assert conv_a lookup is a miss (membership check filters it out).
        //   5. Assert conv_b lookup returns the route.
        let tenant_id = TenantId::new("fs-crash-window-tenant").expect("tenant");
        let user_id = UserId::new("fs-crash-window-user").expect("user");
        let thread_id = ThreadId::new("fs-crash-window-thread").expect("thread");
        let gate_ref = "gate:fs-crash-window-001";
        let conv_a = gate_route_conversation_fingerprint("thread-crash-a");
        let conv_b = gate_route_conversation_fingerprint("thread-crash-b");

        let store = build_gate_route_store(&tenant_id, &user_id);
        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );

        // Step 1: record the route delivered to conv_a.
        let mut record_a = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            gate_ref,
            TurnRunId::new(),
            scope.clone(),
        );
        record_a.delivered_conversation_fingerprints = vec![conv_a.clone()];
        store
            .record_delivered_gate_route(record_a.clone())
            .await
            .expect("initial record must succeed");

        // Step 2: overwrite the route with conv_b (normal path; conv_a index is cleaned).
        let mut record_b = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            gate_ref,
            TurnRunId::new(),
            scope.clone(),
        );
        record_b.delivered_conversation_fingerprints = vec![conv_b.clone()];
        store
            .record_delivered_gate_route(record_b.clone())
            .await
            .expect("overwrite to conv_b must succeed");

        // Confirm conv_b is live and conv_a is gone after the normal overwrite.
        let before_crash = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_a)
            .await
            .expect("lookup must not error");
        assert!(
            before_crash.is_empty(),
            "conv_a must be gone after overwrite to conv_b"
        );

        // Step 3: simulate the crash window by injecting a dangling conv_a index
        // back (mirrors the pattern from filesystem_gate_route_stale_conversation_index_is_harmless_miss).
        // record_a still lists conv_a, so write_delivered_gate_route_conversation_indexes
        // re-adds the conv_a index even though the primary record now lists conv_b.
        store
            .write_delivered_gate_route_conversation_indexes(&record_a)
            .await
            .expect("raw index injection must succeed");

        // Step 4: conv_a lookup must be a miss — the primary record lists conv_b,
        // so the membership check filters out the dangling index.
        let after_crash_a = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_a)
            .await
            .expect("lookup must not error");
        assert!(
            after_crash_a.is_empty(),
            "dangling conv_a index after crash must be a miss, not a route"
        );

        // Step 5: conv_b lookup must still return the live route.
        let after_crash_b = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_b)
            .await
            .expect("lookup must not error");
        assert!(
            !after_crash_b.is_empty(),
            "conv_b must still resolve after crash-window injection"
        );
        assert_eq!(
            after_crash_b[0].gate_ref, gate_ref,
            "conv_b route must match the recorded gate_ref"
        );
    }

    #[tokio::test]
    async fn filesystem_gate_route_overwrite_cleans_stale_conversation_index() {
        // Re-recording the same gate with a different conversation set must
        // drop indexes for conversations no longer delivered to.
        let tenant_id = TenantId::new("fs-overwrite-idx-tenant").expect("tenant");
        let user_id = UserId::new("fs-overwrite-idx-user").expect("user");
        let thread_id = ThreadId::new("fs-overwrite-idx-thread").expect("thread");
        let gate_ref = "gate:fs-overwrite-idx-001";
        let conv_a = gate_route_conversation_fingerprint("thread-a");
        let conv_b = gate_route_conversation_fingerprint("thread-b");

        let store = build_gate_route_store(&tenant_id, &user_id);
        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );
        let mut record = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            gate_ref,
            TurnRunId::new(),
            scope,
        );
        record.delivered_conversation_fingerprints = vec![conv_a.clone()];
        store
            .record_delivered_gate_route(record.clone())
            .await
            .expect("first record must succeed");

        record.delivered_conversation_fingerprints = vec![conv_b.clone()];
        store
            .record_delivered_gate_route(record)
            .await
            .expect("second record must succeed");

        let stale = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_a)
            .await
            .expect("lookup must not error");
        assert!(stale.is_empty(), "replaced conversation index must be gone");

        let fresh = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &conv_b)
            .await
            .expect("lookup must not error");
        assert_eq!(fresh.len(), 1, "new conversation must resolve");
        assert_eq!(fresh[0].gate_ref, gate_ref);
    }

    #[tokio::test]
    async fn filesystem_gate_route_cleanup_preserves_reused_conversation_index() {
        // Cleanup for an old route must not delete a conversation index that has
        // since been repointed to a newer route for the same conversation.
        let tenant_id = TenantId::new("fs-reused-idx-tenant").expect("tenant");
        let user_id = UserId::new("fs-reused-idx-user").expect("user");
        let thread_id = ThreadId::new("fs-reused-idx-thread").expect("thread");
        let shared = gate_route_conversation_fingerprint("thread-shared");

        let store = build_gate_route_store(&tenant_id, &user_id);
        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );
        let mut old = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:fs-reused-old",
            TurnRunId::new(),
            scope.clone(),
        );
        old.delivered_conversation_fingerprints = vec![shared.clone()];
        let mut new = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:fs-reused-new",
            TurnRunId::new(),
            scope,
        );
        new.delivered_conversation_fingerprints = vec![shared.clone()];

        store
            .record_delivered_gate_route(old)
            .await
            .expect("old record must succeed");
        store
            .record_delivered_gate_route(new.clone())
            .await
            .expect("new record must succeed");
        store
            .remove_delivered_gate_route(&tenant_id, &user_id, "gate:fs-reused-old")
            .await
            .expect("old remove must succeed");

        let loaded = store
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_id, &shared)
            .await
            .expect("lookup must not error");
        assert_eq!(
            loaded.len(),
            1,
            "shared conversation must still resolve to new route"
        );
        assert_eq!(loaded[0].gate_ref, new.gate_ref);
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant-scope-key").unwrap()
    }

    fn thread() -> ThreadId {
        ThreadId::new("thread-scope-key").unwrap()
    }

    #[tokio::test]
    async fn filesystem_gate_route_sweep_removes_expired_keeps_fresh() {
        let tenant_id = TenantId::new("sweep-tenant").expect("tenant");
        let user_id = UserId::new("sweep-user").expect("user");
        let agent_id = AgentId::new("sweep-agent").expect("agent");
        let thread_id = ThreadId::new("sweep-thread").expect("thread");

        let store = build_gate_route_store(&tenant_id, &user_id);
        let now = Utc::now();

        // Build two records: one fresh, one expired.
        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(agent_id),
            None,
            thread_id,
            Some(user_id.clone()),
        );

        let fresh = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:sweep-fs-fresh",
            TurnRunId::new(),
            scope.clone(),
        );
        // Override recorded_at to be well within TTL.
        let fresh = crate::DeliveredGateRouteRecord {
            recorded_at: now,
            ..fresh
        };

        let expired = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:sweep-fs-expired",
            TurnRunId::new(),
            scope.clone(),
        );
        // Override recorded_at to be past TTL.
        let expired = crate::DeliveredGateRouteRecord {
            recorded_at: now - Duration::hours(49),
            ..expired
        };

        store
            .record_delivered_gate_route(fresh.clone())
            .await
            .expect("record fresh");
        store
            .record_delivered_gate_route(expired.clone())
            .await
            .expect("record expired");

        let removed = store
            .sweep_expired_delivered_gate_routes(now)
            .await
            .expect("sweep succeeds");
        assert_eq!(removed, 1, "exactly one expired record removed");

        // Fresh record is still loadable.
        let still_there = store
            .load_delivered_gate_route(&tenant_id, &user_id, "gate:sweep-fs-fresh")
            .await
            .expect("load after sweep")
            .expect("fresh record must survive sweep");
        assert_eq!(still_there.gate_ref, "gate:sweep-fs-fresh");

        // Expired record is gone.
        let gone = store
            .load_delivered_gate_route(&tenant_id, &user_id, "gate:sweep-fs-expired")
            .await
            .expect("load after sweep for expired");
        assert!(gone.is_none(), "expired record must be absent after sweep");
    }

    #[tokio::test]
    async fn filesystem_gate_route_sweep_empty_directory_returns_zero() {
        let tenant_id = TenantId::new("sweep-empty-tenant").expect("tenant");
        let user_id = UserId::new("sweep-empty-user").expect("user");
        let store = build_gate_route_store(&tenant_id, &user_id);

        let removed = store
            .sweep_expired_delivered_gate_routes(Utc::now())
            .await
            .expect("sweep on empty directory");
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    async fn filesystem_two_routes_same_conversation_both_retrievable() {
        // Store contract: two distinct routes sharing one conversation
        // fingerprint must both appear in the Vec returned by the conversation
        // lookup. Removing one must leave the other.
        let tenant_id = TenantId::new("fs-two-routes-tenant").expect("tenant");
        let user_id = UserId::new("fs-two-routes-user").expect("user");
        let thread_id = ThreadId::new("fs-two-routes-thread").expect("thread");
        let shared_conv = gate_route_conversation_fingerprint("thread-shared-two");

        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );
        let build_store = || build_gate_route_store(&tenant_id, &user_id);
        let store = build_store();

        let mut rec_a = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:fs-two-a",
            TurnRunId::new(),
            scope.clone(),
        );
        rec_a.delivered_conversation_fingerprints = vec![shared_conv.clone()];

        let mut rec_b = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:fs-two-b",
            TurnRunId::new(),
            scope,
        );
        rec_b.delivered_conversation_fingerprints = vec![shared_conv.clone()];

        store
            .record_delivered_gate_route(rec_a.clone())
            .await
            .expect("record a");
        store
            .record_delivered_gate_route(rec_b.clone())
            .await
            .expect("record b");

        let mut routes = store
            .load_delivered_gate_route_by_conversation_fingerprint(
                &tenant_id,
                &user_id,
                &shared_conv,
            )
            .await
            .expect("lookup must not error");
        routes.sort_by(|a, b| a.gate_ref.cmp(&b.gate_ref));
        assert_eq!(routes.len(), 2, "both routes must be retrievable");
        assert_eq!(routes[0].gate_ref, "gate:fs-two-a");
        assert_eq!(routes[1].gate_ref, "gate:fs-two-b");

        // Removing one route must leave the sibling.
        store
            .remove_delivered_gate_route(&tenant_id, &user_id, "gate:fs-two-a")
            .await
            .expect("remove a");

        let after = store
            .load_delivered_gate_route_by_conversation_fingerprint(
                &tenant_id,
                &user_id,
                &shared_conv,
            )
            .await
            .expect("lookup after remove");
        assert_eq!(after.len(), 1, "sibling must survive removal");
        assert_eq!(after[0].gate_ref, "gate:fs-two-b");
    }

    #[tokio::test]
    async fn filesystem_gate_route_conv_lookup_capped_at_32_entries() {
        let tenant_id = TenantId::new("fs-conv-cap-tenant").expect("tenant");
        let user_id = UserId::new("fs-conv-cap-user").expect("user");
        let thread_id = ThreadId::new("fs-conv-cap-thread").expect("thread");
        let shared_conv = gate_route_conversation_fingerprint("thread-cap-shared");
        let store = build_gate_route_store(&tenant_id, &user_id);
        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );

        for idx in 0..33 {
            let mut record = gate_route_record(
                tenant_id.clone(),
                user_id.clone(),
                &format!("gate:fs-cap-{idx:02}"),
                TurnRunId::new(),
                scope.clone(),
            );
            record.delivered_conversation_fingerprints = vec![shared_conv.clone()];
            store
                .record_delivered_gate_route(record)
                .await
                .expect("record route");
        }

        let results = store
            .load_delivered_gate_route_by_conversation_fingerprint(
                &tenant_id,
                &user_id,
                &shared_conv,
            )
            .await
            .expect("lookup must not error");
        assert!(
            results.len() <= 32,
            "lookup must cap returned records at 32, got {}",
            results.len()
        );
    }

    #[test]
    fn filesystem_old_v1_conversation_index_format_rehydrates() {
        // Wire-compat: old single-entry index files (v1 format) must still
        // be parseable after the one-to-many upgrade.
        let v1_json = serde_json::json!({
            "tenant_id": "tenant:v1-compat",
            "user_id": "user:v1-compat",
            "gate_ref": "gate:v1-compat-ref",
        });
        let file: DeliveredGateRouteConversationIndexFile =
            serde_json::from_value(v1_json).expect("v1 single-entry must rehydrate");
        let routes = file.into_routes();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].gate_ref, "gate:v1-compat-ref");
    }

    /// Build two `FilesystemOutboundStateStore` instances sharing the same
    /// `InMemoryBackend`, allowing a second writer to simulate a concurrent
    /// mid-flight mutation of the same index files.
    fn build_shared_backend_stores(
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> (
        FilesystemOutboundStateStore<InMemoryBackend>,
        FilesystemOutboundStateStore<InMemoryBackend>,
    ) {
        let backend = Arc::new(InMemoryBackend::new());
        let store_a = FilesystemOutboundStateStore::new(build_scoped_fs_for_gate_routes(
            backend.clone(),
            tenant_id,
            user_id,
        ));
        let store_b = FilesystemOutboundStateStore::new(build_scoped_fs_for_gate_routes(
            backend, tenant_id, user_id,
        ));
        (store_a, store_b)
    }

    #[tokio::test]
    async fn filesystem_conv_idx_cas_retry_merges_concurrent_writes() {
        // Regression for the dropped-entry race: two concurrent gate deliveries
        // to the same conversation fingerprint must both be retrievable even if
        // they race to update the conversation index.
        //
        // The CAS-retried index path guarantees that a second writer re-reads
        // the index after a conflict, merges its entry into the existing set,
        // and writes back — rather than overwriting with only its own entry.
        // We exercise this by having two stores share the same backend and
        // write to the same conversation fingerprint sequentially; the second
        // store's write must not drop the first store's entry.
        let tenant_id = TenantId::new("fs-cas-retry-tenant").expect("tenant");
        let user_id = UserId::new("fs-cas-retry-user").expect("user");
        let thread_id = ThreadId::new("fs-cas-retry-thread").expect("thread");
        let shared_conv = "fingerprint:cas-retry-shared";

        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );

        let (store_a, store_b) = build_shared_backend_stores(&tenant_id, &user_id);

        let mut rec_a = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:cas-retry-a",
            TurnRunId::new(),
            scope.clone(),
        );
        rec_a.delivered_conversation_fingerprints = vec![shared_conv.to_string()];

        let mut rec_b = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:cas-retry-b",
            TurnRunId::new(),
            scope,
        );
        rec_b.delivered_conversation_fingerprints = vec![shared_conv.to_string()];

        // Both stores record their gate routes (writing to the shared backend).
        // Because the conversation index file is shared, the second write must
        // merge rather than overwrite.
        store_a
            .record_delivered_gate_route(rec_a.clone())
            .await
            .expect("store_a record must succeed");
        store_b
            .record_delivered_gate_route(rec_b.clone())
            .await
            .expect("store_b record must succeed");

        // Both routes must appear in the conversation lookup.
        let mut routes = store_a
            .load_delivered_gate_route_by_conversation_fingerprint(
                &tenant_id,
                &user_id,
                shared_conv,
            )
            .await
            .expect("lookup must not error");
        routes.sort_by(|a, b| a.gate_ref.cmp(&b.gate_ref));
        assert_eq!(
            routes.len(),
            2,
            "both concurrent routes must be retrievable after CAS-retried index writes"
        );
        assert_eq!(routes[0].gate_ref, "gate:cas-retry-a");
        assert_eq!(routes[1].gate_ref, "gate:cas-retry-b");
    }

    #[tokio::test]
    async fn filesystem_conv_idx_stale_version_write_retries_and_merges() {
        // Simulate the exact interleaving that caused the dropped-entry race:
        //   1. store_a writes rec_a → conv index version 1.
        //   2. store_b (sharing the same backend) immediately writes rec_b →
        //      conv index version 2 (this is the "external mutation" between
        //      store_a's next read and write).
        //   3. store_a re-records rec_a. The retry_conv_idx loop reads v2,
        //      sees rec_a already present (no-op merge), and writes v3 only if
        //      the set changed — confirming the CAS path handles the re-read
        //      correctly.
        //   4. Both routes sharing the fingerprint must remain retrievable.
        let tenant_id = TenantId::new("fs-stale-ver-tenant").expect("tenant");
        let user_id = UserId::new("fs-stale-ver-user").expect("user");
        let thread_id = ThreadId::new("fs-stale-ver-thread").expect("thread");
        let shared_conv = "fingerprint:stale-ver-shared";

        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );

        let (store_a, store_b) = build_shared_backend_stores(&tenant_id, &user_id);

        let mut rec_a = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:stale-ver-a",
            TurnRunId::new(),
            scope.clone(),
        );
        rec_a.delivered_conversation_fingerprints = vec![shared_conv.to_string()];

        let mut rec_b = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:stale-ver-b",
            TurnRunId::new(),
            scope,
        );
        rec_b.delivered_conversation_fingerprints = vec![shared_conv.to_string()];

        // Step 1: store_a writes rec_a (conv index version 1).
        store_a
            .record_delivered_gate_route(rec_a.clone())
            .await
            .expect("initial write of rec_a must succeed");

        // Step 2: store_b writes rec_b (conv index version 2) — simulates a
        // concurrent writer mutating the index while store_a is about to
        // re-record.
        store_b
            .record_delivered_gate_route(rec_b.clone())
            .await
            .expect("concurrent write of rec_b must succeed");

        // Step 3: store_a re-records rec_a. The index already carries rec_a
        // (written in step 1) so the merge is a no-op, but the CAS path must
        // not lose rec_b which was added in step 2.
        store_a
            .record_delivered_gate_route(rec_a.clone())
            .await
            .expect("re-record of rec_a must succeed via CAS retry");

        // Both routes sharing the conversation fingerprint must be present.
        let mut routes = store_a
            .load_delivered_gate_route_by_conversation_fingerprint(
                &tenant_id,
                &user_id,
                shared_conv,
            )
            .await
            .expect("lookup must not error");
        routes.sort_by(|a, b| a.gate_ref.cmp(&b.gate_ref));
        assert_eq!(
            routes.len(),
            2,
            "both entries must be present after interleaved CAS writes; \
             the stale-version write must not have dropped the sibling entry"
        );
        assert_eq!(routes[0].gate_ref, "gate:stale-ver-a");
        assert_eq!(routes[1].gate_ref, "gate:stale-ver-b");
    }

    #[tokio::test]
    async fn filesystem_conv_idx_delete_vs_add_race_preserves_sibling() {
        // Regression for the CAS-delete hole: writer A empties its entry from
        // the conversation index (transition: Write → empty → Delete). Before
        // the fix, Delete used an unversioned filesystem.delete() call. Writer B
        // concurrently adds a sibling entry. Under the old code B's entry could
        // be silently wiped; under the new code a Delete issues a
        // CAS-versioned write of an empty file, so B's concurrent add wins the
        // conflict and the retry loop re-reads a non-empty set and issues Write
        // instead.
        //
        // We simulate this by:
        //   1. store_a writes rec_a (conv index has rec_a).
        //   2. store_b writes rec_b (conv index has rec_a + rec_b).
        //   3. store_a removes rec_a (merge returns Delete because only rec_a
        //      was present in store_a's snapshot). The CAS write of the empty
        //      file conflicts with store_b's rec_b; the retry re-reads
        //      [rec_b], removes rec_a (already gone), returns Write([rec_b]).
        //   4. Lookup must return rec_b.
        let tenant_id = TenantId::new("fs-delete-race-tenant").expect("tenant");
        let user_id = UserId::new("fs-delete-race-user").expect("user");
        let thread_id = ThreadId::new("fs-delete-race-thread").expect("thread");
        let shared_conv = "fingerprint:delete-race-shared";

        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_id.clone()),
        );

        let (store_a, store_b) = build_shared_backend_stores(&tenant_id, &user_id);

        let mut rec_a = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:delete-race-a",
            TurnRunId::new(),
            scope.clone(),
        );
        rec_a.delivered_conversation_fingerprints = vec![shared_conv.to_string()];

        let mut rec_b = gate_route_record(
            tenant_id.clone(),
            user_id.clone(),
            "gate:delete-race-b",
            TurnRunId::new(),
            scope,
        );
        rec_b.delivered_conversation_fingerprints = vec![shared_conv.to_string()];

        // Step 1: store_a records rec_a → conv index = [rec_a].
        store_a
            .record_delivered_gate_route(rec_a.clone())
            .await
            .expect("store_a initial write must succeed");

        // Step 2: store_b records rec_b → conv index = [rec_a, rec_b].
        store_b
            .record_delivered_gate_route(rec_b.clone())
            .await
            .expect("store_b concurrent write must succeed");

        // Step 3: store_a removes rec_a. Its snapshot (from step 1) only sees
        // [rec_a], so merge returns Delete. The CAS-versioned empty-file write
        // conflicts with store_b's version; the retry loop re-reads [rec_a,
        // rec_b], removes rec_a, and writes back [rec_b].
        store_a
            .remove_delivered_gate_route(&tenant_id, &user_id, "gate:delete-race-a")
            .await
            .expect("store_a remove must succeed");

        // Step 4: only rec_b must survive.
        let remaining = store_a
            .load_delivered_gate_route_by_conversation_fingerprint(
                &tenant_id,
                &user_id,
                shared_conv,
            )
            .await
            .expect("lookup must not error");
        assert_eq!(
            remaining.len(),
            1,
            "sibling rec_b must survive delete-vs-add race; got: {remaining:?}"
        );
        assert_eq!(remaining[0].gate_ref, "gate:delete-race-b");
    }

    #[tokio::test]
    async fn filesystem_conv_idx_user_isolation_sibling_user_invisible() {
        // Fix 2: the conversation index is now keyed per (tenant, user), so
        // user A's routes in a shared conversation must not appear in user B's
        // lookup and vice versa.
        let tenant_id = TenantId::new("fs-user-isolation-tenant").expect("tenant");
        let user_a = UserId::new("fs-user-isolation-a").expect("user_a");
        let user_b = UserId::new("fs-user-isolation-b").expect("user_b");
        let thread_id = ThreadId::new("fs-user-isolation-thread").expect("thread");
        let shared_conv = "fingerprint:user-isolation-shared";

        // Each store is scoped to its own user, but shares the same backend.
        let backend = Arc::new(InMemoryBackend::new());
        let store_a = FilesystemOutboundStateStore::new(build_scoped_fs_for_gate_routes(
            backend.clone(),
            &tenant_id,
            &user_a,
        ));
        let store_b = FilesystemOutboundStateStore::new(build_scoped_fs_for_gate_routes(
            backend, &tenant_id, &user_b,
        ));

        let scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            None,
            None,
            thread_id,
            Some(user_a.clone()),
        );

        let mut rec_a = gate_route_record(
            tenant_id.clone(),
            user_a.clone(),
            "gate:user-isolation-a",
            TurnRunId::new(),
            scope.clone(),
        );
        rec_a.delivered_conversation_fingerprints = vec![shared_conv.to_string()];

        let mut rec_b = gate_route_record(
            tenant_id.clone(),
            user_b.clone(),
            "gate:user-isolation-b",
            TurnRunId::new(),
            scope,
        );
        rec_b.delivered_conversation_fingerprints = vec![shared_conv.to_string()];

        store_a
            .record_delivered_gate_route(rec_a.clone())
            .await
            .expect("user_a record must succeed");
        store_b
            .record_delivered_gate_route(rec_b.clone())
            .await
            .expect("user_b record must succeed");

        // user_a lookup must return only rec_a.
        let routes_a = store_a
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_a, shared_conv)
            .await
            .expect("lookup for user_a must not error");
        assert_eq!(
            routes_a.len(),
            1,
            "user_a lookup must return exactly 1 route; got {routes_a:?}"
        );
        assert_eq!(
            routes_a[0].user_id, user_a,
            "user_a result must be owned by user_a"
        );

        // user_b lookup must return only rec_b.
        let routes_b = store_b
            .load_delivered_gate_route_by_conversation_fingerprint(&tenant_id, &user_b, shared_conv)
            .await
            .expect("lookup for user_b must not error");
        assert_eq!(
            routes_b.len(),
            1,
            "user_b lookup must return exactly 1 route; got {routes_b:?}"
        );
        assert_eq!(
            routes_b[0].user_id, user_b,
            "user_b result must be owned by user_b"
        );
    }

    #[test]
    fn sentinel_is_control_character_rejected_by_scope_id_validators() {
        // Audit finding F6: the sentinel for `agent_id = None` /
        // `project_id = None` must be a value that no legal scope id can
        // ever contain. `validate_scope_id` (`ironclaw_host_api::ids`)
        // rejects every ASCII control character, so a single byte in the
        // C0 control range is safe to use as a path-illegal sentinel.
        assert_eq!(SCOPE_NONE_SENTINEL, "\x1f");
        assert!(AgentId::new(SCOPE_NONE_SENTINEL).is_err());
        assert!(ProjectId::new(SCOPE_NONE_SENTINEL).is_err());
    }

    #[test]
    fn underscore_agent_id_no_longer_collides_with_none_sentinel() {
        // Audit finding F6 regression test: before the sentinel fix, a
        // scope with `agent_id = Some("_")` hashed to the same key as a
        // scope with `agent_id = None`, because the sentinel was literally
        // `"_"`. With `\x1F` as the sentinel — rejected by `AgentId::new`
        // — no legal agent_id can collide with the absence marker.
        let underscore_agent =
            TurnScope::new(tenant(), Some(AgentId::new("_").unwrap()), None, thread());
        let none_agent = TurnScope::new(tenant(), None, None, thread());
        assert_ne!(
            thread_scope_key(&underscore_agent),
            thread_scope_key(&none_agent),
        );
    }
}
