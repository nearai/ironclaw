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

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, Filter, IndexKey, IndexKind, IndexName,
    IndexSpec, IndexValue, Page, RootFilesystem, ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{ScopedPath, TenantId, ThreadId};
use ironclaw_turns::{TurnActor, TurnScope};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::validation::{
    validate_advance_request, validate_delivery_attempt, validate_delivery_identity,
    validate_delivery_status_request, validate_policy, validate_subscription_identity,
    validate_subscription_record, validate_subscription_request,
};
use crate::{
    AdvanceSubscriptionCursorRequest, LoadSubscriptionCursorRequest, OutboundDeliveryAttempt,
    OutboundDeliveryId, OutboundError, OutboundStateStore, ProjectionSubscriptionId,
    ProjectionSubscriptionRecord, ThreadNotificationPolicy, UpdateDeliveryStatusRequest,
};

/// Maximum number of compare-and-swap retries on a read-then-write path
/// before surfacing the conflict as a permanent backend failure. Sized to
/// absorb a small burst of concurrent writers without spinning indefinitely;
/// progression invariants (e.g. cursor must not move backwards) are
/// re-validated on every iteration so a regression breaks the loop early
/// rather than ricocheting between racing writers.
const MAX_CAS_RETRIES: usize = 5;

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
        path: &ScopedPath,
        value: &T,
        tenant: &TenantId,
        cas: CasExpectation,
    ) -> Result<(), OutboundError> {
        let body = serde_json::to_vec(value).map_err(|_| OutboundError::Serialization)?;
        // Defense-in-depth: tag the entry with the tenant id so admin-tier
        // queries can filter by tenant and a path-rewriting bug surfaces as
        // a query-time mismatch rather than silent cross-tenant leakage.
        // See docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md.
        let entry = Entry::bytes(body)
            .with_content_type(ContentType::json())
            .with_indexed(tenant_id_index_key(), tenant_id_index_value(tenant));
        self.put_with_byte_fallback(path, entry, cas).await
    }

    /// Like [`put_json`] but additionally projects an indexed scope value so
    /// backends with index support can answer `query(Filter::Eq { scope })`
    /// without materializing every delivery row (audit finding F2). The
    /// `tenant_id` lives in the [`ScopedFilesystem`] mount prefix, not in
    /// this index value — the index discriminates between scopes _within_ a
    /// tenant-scoped subtree.
    async fn put_delivery_attempt_indexed(
        &self,
        path: &ScopedPath,
        attempt: &OutboundDeliveryAttempt,
        cas: CasExpectation,
    ) -> Result<(), OutboundError> {
        let body = serde_json::to_vec(attempt).map_err(|_| OutboundError::Serialization)?;
        let entry = Entry::bytes(body)
            .with_content_type(ContentType::json())
            .with_indexed(
                delivery_scope_index_key(),
                delivery_scope_index_value(&attempt.scope),
            )
            // Defense-in-depth tenant projection — see `put_json` and
            // `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
            .with_indexed(
                tenant_id_index_key(),
                tenant_id_index_value(&attempt.scope.tenant_id),
            );
        self.put_with_byte_fallback(path, entry, cas).await
    }

    /// Write `entry` with the given CAS expectation, falling back to a
    /// metadata-stripped opaque write + `CasExpectation::Any` for backends
    /// that reject record-shape entries or non-`Any` CAS (e.g.
    /// `LocalFilesystem`). Mirrors
    /// [`ironclaw_processes::filesystem_store::put_with_byte_fallback`] so
    /// every byte-only mount in the workspace stays writeable through the
    /// new filesystem stores.
    async fn put_with_byte_fallback(
        &self,
        path: &ScopedPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<(), OutboundError> {
        match self.filesystem.put(path, entry.clone(), cas).await {
            Ok(_) => Ok(()),
            Err(FilesystemError::Unsupported { .. }) => {
                let opaque = Entry::bytes(entry.body).with_content_type(entry.content_type);
                self.filesystem
                    .put(path, opaque, CasExpectation::Any)
                    .await
                    .map(|_| ())
                    .map_err(map_fs_error)
            }
            Err(error) => Err(map_fs_error(error)),
        }
    }

    /// Declare the `scope` exact-equality index on the deliveries prefix.
    /// Idempotent across the deliveries mount lifetime; tolerates backends
    /// that don't materialize indexes (e.g. byte-only `LocalFilesystem`),
    /// because the in-memory `query` evaluator still filters on
    /// `Entry::indexed` even without a declared index.
    async fn ensure_delivery_scope_index(&self) -> Result<(), OutboundError> {
        let root = deliveries_root()?;
        let name = IndexName::new(DELIVERY_SCOPE_INDEX_NAME).map_err(|_| OutboundError::Backend)?;
        let spec = IndexSpec::new(name, vec![delivery_scope_index_key()], IndexKind::Exact);
        match self.filesystem.ensure_index(&root, &spec).await {
            Ok(()) => Ok(()),
            // Match the engine store's pattern (`ensure_exact_index` in
            // `ironclaw_engine::store::filesystem`): backends without index
            // support are still usable for reads/writes; the query path
            // degrades on those mounts but does not fail closed.
            Err(FilesystemError::Unsupported { .. }) => Ok(()),
            Err(error) => Err(map_fs_error(error)),
        }
    }

    /// Declare the `tenant_id` exact-equality index on `root`. Mirrors
    /// [`Self::ensure_delivery_scope_index`] but for the defense-in-depth
    /// tenant projection — see
    /// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
    /// Idempotent across the mount lifetime; tolerates byte-only backends.
    async fn ensure_tenant_id_index(&self, root: &ScopedPath) -> Result<(), OutboundError> {
        let name = IndexName::new(TENANT_ID_INDEX_NAME).map_err(|_| OutboundError::Backend)?;
        let spec = IndexSpec::new(name, vec![tenant_id_index_key()], IndexKind::Exact);
        match self.filesystem.ensure_index(root, &spec).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::Unsupported { .. }) => Ok(()),
            Err(error) => Err(map_fs_error(error)),
        }
    }

    /// Read the current versioned entry at `path` and decode its body as `T`.
    /// Returns the parsed value alongside the version token that can be passed
    /// back as [`CasExpectation::Version`] to detect concurrent writers.
    async fn get_versioned_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<(T, VersionedEntry)>, OutboundError> {
        let Some(versioned) = self.filesystem.get(path).await.map_err(map_fs_error)? else {
            return Ok(None);
        };
        let parsed = serde_json::from_slice(&versioned.entry.body)
            .map_err(|_| OutboundError::Serialization)?;
        Ok(Some((parsed, versioned)))
    }

    async fn get_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<T>, OutboundError> {
        Ok(self
            .get_versioned_json::<T>(path)
            .await?
            .map(|(value, _)| value))
    }
}

#[async_trait]
impl<F> OutboundStateStore for FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        validate_policy(&policy)?;
        let path = policy_path(&policy.scope)?;
        self.ensure_tenant_id_index(&policies_root()?).await?;
        // Notification policy puts are blind overwrites — the caller owns the
        // full policy and there is no read-then-write progression invariant to
        // protect. `CasExpectation::Any` is correct here; the read-then-write
        // paths below carry their own version expectations (audit finding F1).
        self.put_json(&path, &policy, &policy.scope.tenant_id, CasExpectation::Any)
            .await
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        let path = policy_path(&scope)?;
        match self.get_json::<ThreadNotificationPolicy>(&path).await? {
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
        self.ensure_tenant_id_index(&subscriptions_root()?).await?;
        // CAS retry loop (audit finding F1): a concurrent writer could insert
        // or update the same subscription record between our read and put,
        // racing the "cursor must not move backwards" check. Re-read on
        // version mismatch and re-validate identity + progression on every
        // attempt so a regressing cursor breaks the loop rather than letting
        // a later writer overwrite it.
        for _ in 0..MAX_CAS_RETRIES {
            let (cas, existing) = match self
                .get_versioned_json::<ProjectionSubscriptionRecord>(&path)
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
                .put_json(&path, &record, &record.scope.stream.tenant_id, cas)
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
        let Some(record) = self.get_json::<ProjectionSubscriptionRecord>(&path).await? else {
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
        self.ensure_tenant_id_index(&subscriptions_root()?).await?;
        // CAS retry loop (audit finding F1): the audit specifically calls
        // this out as the load-bearing site for the "cursor must not move
        // backwards" invariant. On every retry we re-read the existing
        // record and re-run `validate_advance_request`, which re-checks
        // progression against the current persisted cursor. If a racing
        // writer already advanced the cursor past `request.cursor.runtime`,
        // the second-pass validation surfaces `InvalidRequest` rather than
        // silently overwriting forward progress with stale state.
        for _ in 0..MAX_CAS_RETRIES {
            let Some((mut record, versioned)) = self
                .get_versioned_json::<ProjectionSubscriptionRecord>(&path)
                .await?
            else {
                return Err(OutboundError::SubscriptionScopeMismatch);
            };
            validate_advance_request(&record, &request)?;
            record.cursor = Some(request.cursor.clone());
            match self
                .put_json(
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
        self.ensure_delivery_scope_index().await?;
        self.ensure_tenant_id_index(&deliveries_root()?).await?;
        let path = delivery_path(&attempt.delivery_id)?;
        // CAS retry loop (audit finding F1): two concurrent writers racing the
        // same `delivery_id` (e.g. an at-least-once orchestrator retry firing
        // twice) must not both succeed at the first-write branch. We use
        // `CasExpectation::Absent` for the initial insert so the second
        // writer's put fails with `CasConflict`; the loop then re-reads and
        // falls into the identity-validate branch, which treats a matching
        // existing record as a duplicate-OK no-op.
        for _ in 0..MAX_CAS_RETRIES {
            if let Some(existing) = self.get_json::<OutboundDeliveryAttempt>(&path).await? {
                validate_delivery_identity(&existing, &attempt)?;
                return Ok(());
            }
            match self
                .put_delivery_attempt_indexed(&path, &attempt, CasExpectation::Absent)
                .await
            {
                Ok(()) => return Ok(()),
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
        // CAS retry loop (audit finding F1): two concurrent status writes
        // (e.g. delivered + failed firing simultaneously after a retry) race
        // the read-modify-write. Re-read on version mismatch so the second
        // writer sees the first writer's state instead of clobbering it with
        // stale fields. The write side reuses
        // `put_delivery_attempt_indexed` so the `scope` projection is
        // preserved on status mutations (audit finding F2).
        for _ in 0..MAX_CAS_RETRIES {
            let Some((mut attempt, versioned)) = self
                .get_versioned_json::<OutboundDeliveryAttempt>(&path)
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
        // Audit finding F2: previously this was a `list_dir` + N+1
        // `get_json` per row, with no indexed projection. Now we declare an
        // exact-equality index on the `scope` indexed key (the same hash
        // `thread_scope_key` uses for policy paths) and let the backend
        // serve `Filter::Eq` natively.
        //
        // Audit finding F3: the previous `list_dir` call was unpaginated;
        // SQL backends issue `LIMIT Page::MAX_LIMIT` on the list_dir
        // translation and would silently truncate past 1024 deliveries.
        // Drain the paginated query in a loop until a short page arrives,
        // mirroring the engine store's `query_all` pattern.
        self.ensure_delivery_scope_index().await?;
        let root = deliveries_root()?;
        let filter = Filter::Eq {
            key: delivery_scope_index_key(),
            value: delivery_scope_index_value(&scope),
        };
        let mut deliveries: Vec<OutboundDeliveryAttempt> = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = match self.filesystem.query(&root, &filter, page).await {
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
}

fn policy_path(scope: &TurnScope) -> Result<ScopedPath, OutboundError> {
    let key = thread_scope_key(scope);
    ScopedPath::new(format!("/outbound/policies/{key}.json")).map_err(|_| OutboundError::Backend)
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

fn deliveries_root() -> Result<ScopedPath, OutboundError> {
    ScopedPath::new(DELIVERIES_ROOT).map_err(|_| OutboundError::Backend)
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
mod tests {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::TurnScope;

    use super::{SCOPE_NONE_SENTINEL, thread_scope_key};

    fn tenant() -> TenantId {
        TenantId::new("tenant-scope-key").unwrap()
    }

    fn thread() -> ThreadId {
        ThreadId::new("thread-scope-key").unwrap()
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
