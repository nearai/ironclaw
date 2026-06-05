//! Durable product workflow [`IdempotencyLedger`] storage adapters.

#![cfg_attr(
    not(any(feature = "libsql", feature = "postgres")),
    allow(dead_code, unused_imports)
)]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexValue,
    Page, RecordKind, RecordVersion, RootFilesystem, ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{ResourceScope, ScopedPath, VirtualPath};
use ironclaw_product_workflow::{
    ActionFingerprintKey, ActionPhase, IdempotencyDecision, IdempotencyLedger,
    ProductInboundAction, ProductWorkflowError,
};

const DEFAULT_IN_FLIGHT_LEASE: Duration = Duration::seconds(60);
const DEFAULT_LEDGER_ROOT: &str = "/engine/product_workflow/idempotency/actions";
const ACTION_RECORD_KIND: &str = "product_workflow_action";
const PRUNE_LEASE_RECORD_KIND: &str = "product_workflow_prune_lease";
const PRUNE_LEASE_SECONDS: i64 = 30;

struct FilesystemIdempotencyLedger {
    filesystem: Arc<dyn LedgerFilesystem>,
    root: LedgerRoot,
    in_flight_lease: Duration,
    settled_entry_limit: Option<NonZeroUsize>,
    settled_prune_interval: NonZeroUsize,
    settled_since_prune: AtomicUsize,
}

impl FilesystemIdempotencyLedger {
    fn new_root(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self::with_root_lease(filesystem, DEFAULT_IN_FLIGHT_LEASE)
    }

    fn with_root_lease(filesystem: Arc<dyn RootFilesystem>, in_flight_lease: Duration) -> Self {
        Self {
            filesystem: Arc::new(RootLedgerFilesystem { filesystem }),
            root: LedgerRoot::Root(default_ledger_root()),
            in_flight_lease,
            settled_entry_limit: None,
            settled_prune_interval: NonZeroUsize::new(1).expect("non-zero literal"), // safety: static literal is non-zero.
            settled_since_prune: AtomicUsize::new(0),
        }
    }

    fn with_root(
        filesystem: Arc<dyn RootFilesystem>,
        root: VirtualPath,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            filesystem: Arc::new(RootLedgerFilesystem { filesystem }),
            root: LedgerRoot::Root(root),
            in_flight_lease,
            settled_entry_limit: None,
            settled_prune_interval: NonZeroUsize::new(1).expect("non-zero literal"), // safety: static literal is non-zero.
            settled_since_prune: AtomicUsize::new(0),
        }
    }

    fn new_scoped<F>(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        in_flight_lease: Duration,
    ) -> Self
    where
        F: RootFilesystem + 'static,
    {
        Self {
            filesystem: Arc::new(ScopedLedgerFilesystem { filesystem, scope }),
            root: LedgerRoot::Scoped(default_scoped_ledger_root()),
            in_flight_lease,
            settled_entry_limit: None,
            settled_prune_interval: NonZeroUsize::new(1).expect("non-zero literal"), // safety: static literal is non-zero.
            settled_since_prune: AtomicUsize::new(0),
        }
    }

    fn with_scoped_root<F>(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        root: ScopedPath,
        in_flight_lease: Duration,
    ) -> Self
    where
        F: RootFilesystem + 'static,
    {
        Self {
            filesystem: Arc::new(ScopedLedgerFilesystem { filesystem, scope }),
            root: LedgerRoot::Scoped(root),
            in_flight_lease,
            settled_entry_limit: None,
            settled_prune_interval: NonZeroUsize::new(1).expect("non-zero literal"), // safety: static literal is non-zero.
            settled_since_prune: AtomicUsize::new(0),
        }
    }

    fn with_settled_entry_limit(mut self, limit: NonZeroUsize) -> Self {
        self.settled_entry_limit = Some(limit);
        self.settled_prune_interval = settled_prune_interval_for(limit);
        self
    }

    fn with_settled_prune_interval(mut self, interval: NonZeroUsize) -> Self {
        self.settled_prune_interval = interval;
        self
    }

    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        let path = action_path(&self.root, &fingerprint)?;
        let action = ProductInboundAction::begin(fingerprint, received_at);
        match self
            .filesystem
            .put(&path, entry_for_action(&action)?, CasExpectation::Absent)
            .await
        {
            Ok(_) => return Ok(IdempotencyDecision::New(action)),
            Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(error) => return Err(filesystem_error("reserve action", error)),
        }

        loop {
            let Some((prior, version)) = load_action(self.filesystem.as_ref(), &path).await? else {
                return Err(transient("idempotency ledger conflict row disappeared"));
            };
            if prior.is_terminal() {
                return Ok(IdempotencyDecision::Replay(prior));
            }
            if fresh_in_flight(&prior, received_at, self.in_flight_lease) {
                return Err(in_flight_error());
            }

            let replacement = ProductInboundAction::begin(prior.fingerprint.clone(), received_at);
            match self
                .filesystem
                .put(
                    &path,
                    entry_for_action(&replacement)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => return Ok(IdempotencyDecision::New(replacement)),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("reclaim action", error)),
            }
        }
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let path = action_path(&self.root, &action.fingerprint)?;
        loop {
            let Some((current, version)) = load_action(self.filesystem.as_ref(), &path).await?
            else {
                return Err(transient(
                    "idempotency reservation missing before terminal settle",
                ));
            };
            if current.is_terminal() {
                if current.action_id == action.action_id {
                    return Ok(());
                }
                return Err(transient(
                    "idempotency reservation was superseded before terminal settle",
                ));
            }
            if current.action_id != action.action_id {
                return Err(transient(
                    "idempotency reservation was superseded before terminal settle",
                ));
            }

            match self
                .filesystem
                .put(
                    &path,
                    entry_for_action(&action)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => {
                    if self.should_prune_after_settle()
                        && let Err(error) = self.prune_settled_entries().await
                    {
                        // silent-ok: settled-action pruning is retention cleanup; future settles retry.
                        tracing::warn!(
                            error = %error,
                            "product workflow idempotency ledger failed to prune settled entries"
                        );
                    }
                    return Ok(());
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("settle action", error)),
            }
        }
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        let path = action_path(&self.root, &action.fingerprint)?;
        loop {
            let Some((current, version)) = load_action(self.filesystem.as_ref(), &path).await?
            else {
                return Ok(());
            };
            if current.is_terminal() || current.action_id != action.action_id {
                return Ok(());
            }

            let mut released = current;
            released.received_at = expired_received_at(released.received_at, self.in_flight_lease);
            match self
                .filesystem
                .put(
                    &path,
                    entry_for_action(&released)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(filesystem_error("release action", error)),
            }
        }
    }

    async fn prune_settled_entries(&self) -> Result<(), ProductWorkflowError> {
        let Some(limit) = self.settled_entry_limit else {
            return Ok(());
        };
        let mut settled = self.load_settled_actions().await?;
        if settled.len() <= limit.get() {
            return Ok(());
        }
        if !self.try_acquire_prune_lease().await? {
            return Ok(());
        }
        settled.sort_by(|left, right| {
            left.received_at
                .cmp(&right.received_at)
                .then_with(|| left.action_id.as_uuid().cmp(&right.action_id.as_uuid()))
        });
        let prune_count = settled.len() - limit.get();
        for action in settled.into_iter().take(prune_count) {
            self.prune_settled_action_if_current(&action).await?;
        }
        Ok(())
    }

    fn should_prune_after_settle(&self) -> bool {
        if self.settled_entry_limit.is_none() {
            return false;
        }
        let interval = self.settled_prune_interval.get();
        self.settled_since_prune
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
            % interval
            == 0
    }

    async fn try_acquire_prune_lease(&self) -> Result<bool, ProductWorkflowError> {
        let path = prune_lease_path(&self.root)?;
        let entry = prune_lease_entry(Utc::now() + Duration::seconds(PRUNE_LEASE_SECONDS))?;
        match self
            .filesystem
            .put(&path, entry.clone(), CasExpectation::Absent)
            .await
        {
            Ok(_) => return Ok(true),
            Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(error) => return Err(filesystem_error("acquire prune lease", error)),
        }

        let Some(existing) = self
            .filesystem
            .get(&path)
            .await
            .map_err(|error| filesystem_error("load prune lease", error))?
        else {
            return Ok(false);
        };
        if prune_lease_is_fresh(&existing.entry)? {
            return Ok(false);
        }
        match self
            .filesystem
            .put(&path, entry, CasExpectation::Version(existing.version))
            .await
        {
            Ok(_) => Ok(true),
            Err(FilesystemError::VersionMismatch { .. }) => Ok(false),
            Err(error) => Err(filesystem_error("renew prune lease", error)),
        }
    }

    async fn prune_settled_action_if_current(
        &self,
        action: &ProductInboundAction,
    ) -> Result<(), ProductWorkflowError> {
        let path = action_path(&self.root, &action.fingerprint)?;
        let Some((current, _version)) = load_action(self.filesystem.as_ref(), &path).await? else {
            return Ok(());
        };
        if current.action_id != action.action_id || !matches!(current.phase, ActionPhase::Settled) {
            return Ok(());
        }
        match self.filesystem.delete(&path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(filesystem_error("prune settled action", error)),
        }
    }

    async fn load_settled_actions(
        &self,
    ) -> Result<Vec<ProductInboundAction>, ProductWorkflowError> {
        let mut actions = Vec::new();
        let mut offset = 0;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = self
                .filesystem
                .query(&self.root, &settled_action_filter()?, page)
                .await
                .map_err(|error| filesystem_error("query settled actions", error))?;
            let received = entries.len();
            for entry in entries {
                let action: ProductInboundAction = entry
                    .entry
                    .parse_json()
                    .map_err(|error| durable_error("deserialize action", error))?;
                if matches!(action.phase, ActionPhase::Settled) {
                    actions.push(action);
                }
            }
            if received < Page::MAX_LIMIT as usize {
                return Ok(actions);
            }
            offset += u64::from(Page::MAX_LIMIT);
        }
    }
}

/// Scoped-filesystem-backed product workflow idempotency ledger.
///
/// Construct with the same [`ScopedFilesystem`] handle used by the Reborn host
/// stores. The supplied [`ResourceScope`] is passed to the filesystem for every
/// operation so the filesystem's mount resolver owns any tenant/user rewriting.
pub struct RebornFilesystemIdempotencyLedger<F>
where
    F: RootFilesystem,
{
    inner: FilesystemIdempotencyLedger,
    _filesystem: std::marker::PhantomData<F>,
}

impl<F> RebornFilesystemIdempotencyLedger<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, scope: ResourceScope) -> Self {
        Self::with_in_flight_lease(filesystem, scope, DEFAULT_IN_FLIGHT_LEASE)
    }

    pub fn with_in_flight_lease(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::new_scoped(filesystem, scope, in_flight_lease),
            _filesystem: std::marker::PhantomData,
        }
    }

    pub fn with_root(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        root: ScopedPath,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_scoped_root(
                filesystem,
                scope,
                root,
                in_flight_lease,
            ),
            _filesystem: std::marker::PhantomData,
        }
    }

    pub fn with_settled_entry_limit(mut self, limit: NonZeroUsize) -> Self {
        self.inner = self.inner.with_settled_entry_limit(limit);
        self
    }

    pub fn with_settled_prune_interval(mut self, interval: NonZeroUsize) -> Self {
        self.inner = self.inner.with_settled_prune_interval(interval);
        self
    }
}

#[async_trait]
impl<F> IdempotencyLedger for RebornFilesystemIdempotencyLedger<F>
where
    F: RootFilesystem + 'static,
{
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        self.inner.begin_or_replay(fingerprint, received_at).await
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.settle(action).await
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.release(action).await
    }
}

/// libSQL-backed product workflow idempotency ledger using the shared
/// SQL filesystem backend for persistence.
#[cfg(feature = "libsql")]
pub struct RebornLibSqlIdempotencyLedger {
    inner: FilesystemIdempotencyLedger,
}

#[cfg(feature = "libsql")]
impl RebornLibSqlIdempotencyLedger {
    pub fn new(filesystem: Arc<LibSqlRootFilesystem>) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::new_root(filesystem),
        }
    }

    pub fn with_in_flight_lease(
        filesystem: Arc<LibSqlRootFilesystem>,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_root_lease(filesystem, in_flight_lease),
        }
    }

    pub fn with_root(
        filesystem: Arc<LibSqlRootFilesystem>,
        root: VirtualPath,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_root(filesystem, root, in_flight_lease),
        }
    }

    pub fn with_settled_entry_limit(mut self, limit: NonZeroUsize) -> Self {
        self.inner = self.inner.with_settled_entry_limit(limit);
        self
    }

    pub fn with_settled_prune_interval(mut self, interval: NonZeroUsize) -> Self {
        self.inner = self.inner.with_settled_prune_interval(interval);
        self
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl IdempotencyLedger for RebornLibSqlIdempotencyLedger {
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        self.inner.begin_or_replay(fingerprint, received_at).await
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.settle(action).await
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.release(action).await
    }
}

/// PostgreSQL-backed product workflow idempotency ledger using the shared
/// SQL filesystem backend for persistence.
#[cfg(feature = "postgres")]
pub struct RebornPostgresIdempotencyLedger {
    inner: FilesystemIdempotencyLedger,
}

#[cfg(feature = "postgres")]
impl RebornPostgresIdempotencyLedger {
    pub fn new(filesystem: Arc<PostgresRootFilesystem>) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::new_root(filesystem),
        }
    }

    pub fn with_in_flight_lease(
        filesystem: Arc<PostgresRootFilesystem>,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_root_lease(filesystem, in_flight_lease),
        }
    }

    pub fn with_root(
        filesystem: Arc<PostgresRootFilesystem>,
        root: VirtualPath,
        in_flight_lease: Duration,
    ) -> Self {
        Self {
            inner: FilesystemIdempotencyLedger::with_root(filesystem, root, in_flight_lease),
        }
    }

    pub fn with_settled_entry_limit(mut self, limit: NonZeroUsize) -> Self {
        self.inner = self.inner.with_settled_entry_limit(limit);
        self
    }

    pub fn with_settled_prune_interval(mut self, interval: NonZeroUsize) -> Self {
        self.inner = self.inner.with_settled_prune_interval(interval);
        self
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl IdempotencyLedger for RebornPostgresIdempotencyLedger {
    async fn begin_or_replay(
        &self,
        fingerprint: ActionFingerprintKey,
        received_at: DateTime<Utc>,
    ) -> Result<IdempotencyDecision, ProductWorkflowError> {
        self.inner.begin_or_replay(fingerprint, received_at).await
    }

    async fn settle(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.settle(action).await
    }

    async fn release(&self, action: ProductInboundAction) -> Result<(), ProductWorkflowError> {
        self.inner.release(action).await
    }
}

#[async_trait]
trait LedgerFilesystem: Send + Sync {
    async fn put(
        &self,
        path: &LedgerPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError>;

    async fn get(&self, path: &LedgerPath) -> Result<Option<VersionedEntry>, FilesystemError>;

    async fn query(
        &self,
        root: &LedgerRoot,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError>;

    async fn delete(&self, path: &LedgerPath) -> Result<(), FilesystemError>;
}

struct RootLedgerFilesystem {
    filesystem: Arc<dyn RootFilesystem>,
}

#[async_trait]
impl LedgerFilesystem for RootLedgerFilesystem {
    async fn put(
        &self,
        path: &LedgerPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let LedgerPath::Root(path) = path else {
            return Err(ledger_path_kind_error(FilesystemOperation::WriteFile));
        };
        self.filesystem.put(path, entry, cas).await
    }

    async fn get(&self, path: &LedgerPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let LedgerPath::Root(path) = path else {
            return Err(ledger_path_kind_error(FilesystemOperation::ReadFile));
        };
        self.filesystem.get(path).await
    }

    async fn query(
        &self,
        root: &LedgerRoot,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let LedgerRoot::Root(root) = root else {
            return Err(ledger_path_kind_error(FilesystemOperation::Query));
        };
        self.filesystem.query(root, filter, page).await
    }

    async fn delete(&self, path: &LedgerPath) -> Result<(), FilesystemError> {
        let LedgerPath::Root(path) = path else {
            return Err(ledger_path_kind_error(FilesystemOperation::Delete));
        };
        self.filesystem.delete(path).await
    }
}

struct ScopedLedgerFilesystem<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
}

#[async_trait]
impl<F> LedgerFilesystem for ScopedLedgerFilesystem<F>
where
    F: RootFilesystem + 'static,
{
    async fn put(
        &self,
        path: &LedgerPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let LedgerPath::Scoped(path) = path else {
            return Err(ledger_path_kind_error(FilesystemOperation::WriteFile));
        };
        self.filesystem.put(&self.scope, path, entry, cas).await
    }

    async fn get(&self, path: &LedgerPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let LedgerPath::Scoped(path) = path else {
            return Err(ledger_path_kind_error(FilesystemOperation::ReadFile));
        };
        self.filesystem.get(&self.scope, path).await
    }

    async fn query(
        &self,
        root: &LedgerRoot,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let LedgerRoot::Scoped(root) = root else {
            return Err(ledger_path_kind_error(FilesystemOperation::Query));
        };
        self.filesystem.query(&self.scope, root, filter, page).await
    }

    async fn delete(&self, path: &LedgerPath) -> Result<(), FilesystemError> {
        let LedgerPath::Scoped(path) = path else {
            return Err(ledger_path_kind_error(FilesystemOperation::Delete));
        };
        self.filesystem.delete(&self.scope, path).await
    }
}

enum LedgerRoot {
    Root(VirtualPath),
    Scoped(ScopedPath),
}

impl LedgerRoot {
    fn as_str(&self) -> &str {
        match self {
            Self::Root(path) => path.as_str(),
            Self::Scoped(path) => path.as_str(),
        }
    }
}

enum LedgerPath {
    Root(VirtualPath),
    Scoped(ScopedPath),
}

fn settled_prune_interval_for(limit: NonZeroUsize) -> NonZeroUsize {
    NonZeroUsize::new((limit.get() / 10).max(1)).expect("non-zero derived interval") // safety: max(1) guarantees a non-zero value.
}

fn transient(reason: impl Into<String>) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: reason.into(),
    }
}

fn durable_error(operation: &'static str, error: impl std::fmt::Display) -> ProductWorkflowError {
    let error_type = std::any::type_name_of_val(&error);
    tracing::error!(
        operation,
        error_type,
        "product workflow idempotency ledger failed"
    );
    transient(format!("idempotency ledger failed to {operation}"))
}

fn filesystem_error(operation: &'static str, error: FilesystemError) -> ProductWorkflowError {
    durable_error(operation, error)
}

fn fresh_in_flight(
    action: &ProductInboundAction,
    received_at: DateTime<Utc>,
    lease: Duration,
) -> bool {
    !action.is_terminal() && action.received_at + lease > received_at
}

fn in_flight_error() -> ProductWorkflowError {
    transient("idempotency fingerprint already in flight; retry after recovery lease")
}

fn expired_received_at(received_at: DateTime<Utc>, lease: Duration) -> DateTime<Utc> {
    received_at - lease - Duration::seconds(1)
}

async fn load_action(
    filesystem: &dyn LedgerFilesystem,
    path: &LedgerPath,
) -> Result<Option<(ProductInboundAction, RecordVersion)>, ProductWorkflowError> {
    let Some(entry) = filesystem
        .get(path)
        .await
        .map_err(|error| filesystem_error("load action", error))?
    else {
        return Ok(None);
    };
    let action = entry
        .entry
        .parse_json()
        .map_err(|error| durable_error("deserialize action", error))?;
    Ok(Some((action, entry.version)))
}

fn entry_for_action(action: &ProductInboundAction) -> Result<Entry, ProductWorkflowError> {
    let payload =
        serde_json::to_value(action).map_err(|error| durable_error("serialize action", error))?;
    let kind = RecordKind::new(ACTION_RECORD_KIND)
        .map_err(|error| durable_error("construct action record kind", error))?;
    let entry = Entry::record(kind, &payload)
        .map_err(|error| durable_error("serialize action entry", error))?
        .with_indexed(
            index_key("adapter_id")?,
            text(action.fingerprint.adapter_id.as_str()),
        )
        .with_indexed(
            index_key("installation_id")?,
            text(action.fingerprint.installation_id.as_str()),
        )
        .with_indexed(
            index_key("external_actor_kind")?,
            text(action.fingerprint.external_actor_ref.kind()),
        )
        .with_indexed(
            index_key("external_actor_id")?,
            text(action.fingerprint.external_actor_ref.id()),
        )
        .with_indexed(
            index_key("source_binding_key")?,
            text(action.fingerprint.source_binding_key.as_str()),
        )
        .with_indexed(
            index_key("external_event_id")?,
            text(action.fingerprint.external_event_id.as_str()),
        )
        .with_indexed(index_key("phase")?, text(phase_label(action.phase)))
        .with_indexed(
            index_key("received_at_ms")?,
            IndexValue::I64(action.received_at.timestamp_millis()),
        );
    Ok(entry)
}

fn settled_action_filter() -> Result<Filter, ProductWorkflowError> {
    Ok(Filter::Eq {
        key: index_key("phase")?,
        value: text(phase_label(ActionPhase::Settled)),
    })
}

fn prune_lease_path(root: &LedgerRoot) -> Result<LedgerPath, ProductWorkflowError> {
    let path = format!(
        "{}/_control/prune_lease.json",
        root.as_str().trim_end_matches('/')
    );
    match root {
        LedgerRoot::Root(_) => VirtualPath::new(path)
            .map(LedgerPath::Root)
            .map_err(|error| durable_error("construct prune lease path", error)),
        LedgerRoot::Scoped(_) => ScopedPath::new(path)
            .map(LedgerPath::Scoped)
            .map_err(|error| durable_error("construct prune lease path", error)),
    }
}

fn prune_lease_entry(expires_at: DateTime<Utc>) -> Result<Entry, ProductWorkflowError> {
    let payload = serde_json::json!({
        "expires_at_ms": expires_at.timestamp_millis(),
    });
    let kind = RecordKind::new(PRUNE_LEASE_RECORD_KIND)
        .map_err(|error| durable_error("construct prune lease record kind", error))?;
    Entry::record(kind, &payload)
        .map_err(|error| durable_error("serialize prune lease entry", error))
}

fn prune_lease_is_fresh(entry: &Entry) -> Result<bool, ProductWorkflowError> {
    let payload: serde_json::Value = entry
        .parse_json()
        .map_err(|error| durable_error("deserialize prune lease", error))?;
    let expires_at_ms = payload
        .get("expires_at_ms")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    Ok(expires_at_ms > Utc::now().timestamp_millis())
}

fn index_key(value: &'static str) -> Result<IndexKey, ProductWorkflowError> {
    IndexKey::new(value).map_err(|error| durable_error("construct action index key", error))
}

fn text(value: &str) -> IndexValue {
    IndexValue::Text(value.to_string())
}

fn phase_label(phase: ActionPhase) -> &'static str {
    match phase {
        ActionPhase::Received => "received",
        ActionPhase::Dispatched => "dispatched",
        ActionPhase::Settled => "settled",
        ActionPhase::DeduplicatedReplay => "deduplicated_replay",
    }
}

fn action_path(
    root: &LedgerRoot,
    fingerprint: &ActionFingerprintKey,
) -> Result<LedgerPath, ProductWorkflowError> {
    let path = format!(
        "{}/{}/{}/{}/{}/{}/{}.json",
        root.as_str().trim_end_matches('/'),
        hex_component(fingerprint.adapter_id.as_str()),
        hex_component(fingerprint.installation_id.as_str()),
        hex_component(fingerprint.external_actor_ref.kind()),
        hex_component(fingerprint.external_actor_ref.id()),
        hex_component(fingerprint.source_binding_key.as_str()),
        hex_component(fingerprint.external_event_id.as_str())
    );
    match root {
        LedgerRoot::Root(_) => VirtualPath::new(path)
            .map(LedgerPath::Root)
            .map_err(|error| durable_error("construct action path", error)),
        LedgerRoot::Scoped(_) => ScopedPath::new(path)
            .map(LedgerPath::Scoped)
            .map_err(|error| durable_error("construct action path", error)),
    }
}

fn default_ledger_root() -> VirtualPath {
    // safety: DEFAULT_LEDGER_ROOT is a static absolute virtual path literal.
    VirtualPath::new(DEFAULT_LEDGER_ROOT).expect("default ledger root is a valid virtual path")
}

fn default_scoped_ledger_root() -> ScopedPath {
    // safety: DEFAULT_LEDGER_ROOT is also valid in the scoped path grammar.
    ScopedPath::new(DEFAULT_LEDGER_ROOT).expect("default ledger root is a valid scoped path")
}

fn ledger_path_kind_error(operation: FilesystemOperation) -> FilesystemError {
    FilesystemError::Backend {
        path: default_ledger_root(),
        operation,
        reason: "ledger path kind did not match filesystem adapter".to_string(),
    }
}

fn hex_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
