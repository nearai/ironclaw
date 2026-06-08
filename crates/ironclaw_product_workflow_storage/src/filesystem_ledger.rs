//! Filesystem-backed product workflow [`IdempotencyLedger`] storage adapters.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use futures_util::StreamExt;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, IndexKey, IndexValue, Page, RecordKind,
    RecordVersion, RootFilesystem, ScopedFilesystem,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{AgentId, InvocationId, ProjectId, TenantId, UserId};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use ironclaw_product_workflow::{
    ActionFingerprintKey, ActionPhase, IdempotencyDecision, IdempotencyLedger,
    ProductInboundAction, ProductWorkflowError,
};

mod path;

use path::{
    action_path, default_scoped_ledger_root, prune_lease_path, scoped_ledger_root_for_scope,
};

const DEFAULT_IN_FLIGHT_LEASE: Duration = Duration::seconds(60);
const ACTION_RECORD_KIND: &str = "product_workflow_action";
const PRUNE_LEASE_RECORD_KIND: &str = "product_workflow_prune_lease";
const PRUNE_LEASE_SECONDS: i64 = 30;
const PRUNE_DELETE_CONCURRENCY: usize = 16;

struct FilesystemIdempotencyLedger<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
    root: ScopedPath,
    in_flight_lease: Duration,
    settled_entry_limit: Option<NonZeroUsize>,
    settled_prune_interval: NonZeroUsize,
    settled_since_prune: AtomicUsize,
}

impl<F> FilesystemIdempotencyLedger<F>
where
    F: RootFilesystem + 'static,
{
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn new_root(filesystem: Arc<F>) -> Self {
        Self::with_root_lease(filesystem, DEFAULT_IN_FLIGHT_LEASE)
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn with_root_lease(filesystem: Arc<F>, in_flight_lease: Duration) -> Self {
        let root = default_scoped_ledger_root();
        Self {
            filesystem: root_scoped_filesystem(filesystem, &root),
            scope: root_scope(),
            root,
            in_flight_lease,
            settled_entry_limit: None,
            settled_prune_interval: NonZeroUsize::new(1).expect("non-zero literal"), // safety: static literal is non-zero.
            settled_since_prune: AtomicUsize::new(0),
        }
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn with_root(filesystem: Arc<F>, root: VirtualPath, in_flight_lease: Duration) -> Self {
        let root = ScopedPath::new(root.as_str()).expect("virtual root is a valid scoped path"); // safety: both path types use the same absolute path grammar.
        Self {
            filesystem: root_scoped_filesystem(filesystem, &root),
            scope: root_scope(),
            root,
            in_flight_lease,
            settled_entry_limit: None,
            settled_prune_interval: NonZeroUsize::new(1).expect("non-zero literal"), // safety: static literal is non-zero.
            settled_since_prune: AtomicUsize::new(0),
        }
    }

    fn new_scoped(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        in_flight_lease: Duration,
    ) -> Self {
        let root = scoped_ledger_root_for_scope(default_scoped_ledger_root(), &scope);
        Self {
            filesystem,
            scope,
            root,
            in_flight_lease,
            settled_entry_limit: None,
            settled_prune_interval: NonZeroUsize::new(1).expect("non-zero literal"), // safety: static literal is non-zero.
            settled_since_prune: AtomicUsize::new(0),
        }
    }

    fn with_scoped_root(
        filesystem: Arc<ScopedFilesystem<F>>,
        scope: ResourceScope,
        root: ScopedPath,
        in_flight_lease: Duration,
    ) -> Self {
        let root = scoped_ledger_root_for_scope(root, &scope);
        Self {
            filesystem,
            scope,
            root,
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
            .put(
                &self.scope,
                &path,
                entry_for_action(&action)?,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) => return Ok(IdempotencyDecision::New(action)),
            Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(error) => return Err(filesystem_error("reserve action", error)),
        }

        loop {
            let Some((prior, version)) = load_action(&self.filesystem, &self.scope, &path).await?
            else {
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
                    &self.scope,
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
            let Some((current, version)) =
                load_action(&self.filesystem, &self.scope, &path).await?
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
                    &self.scope,
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
            let Some((current, version)) =
                load_action(&self.filesystem, &self.scope, &path).await?
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
                    &self.scope,
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
        let terminal_may_exceed_limit = self.terminal_actions_may_exceed(limit).await?;
        if !terminal_may_exceed_limit {
            return Ok(());
        }
        if !self.try_acquire_prune_lease().await? {
            return Ok(());
        }
        if terminal_may_exceed_limit {
            let mut terminal = self.load_actions(&terminal_action_filter()?).await?;
            if terminal.len() > limit.get() {
                terminal.sort_by(|left, right| {
                    left.received_at
                        .cmp(&right.received_at)
                        .then_with(|| left.action_id.as_uuid().cmp(&right.action_id.as_uuid()))
                });
                let prune_count = terminal.len() - limit.get();
                self.prune_terminal_actions(terminal.into_iter().take(prune_count).collect())
                    .await?;
            }
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
            .is_multiple_of(interval)
    }

    async fn try_acquire_prune_lease(&self) -> Result<bool, ProductWorkflowError> {
        let path = prune_lease_path(&self.root)?;
        let entry = prune_lease_entry(Utc::now() + Duration::seconds(PRUNE_LEASE_SECONDS))?;
        match self
            .filesystem
            .put(&self.scope, &path, entry.clone(), CasExpectation::Absent)
            .await
        {
            Ok(_) => return Ok(true),
            Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(error) => return Err(filesystem_error("acquire prune lease", error)),
        }

        let Some(existing) = self
            .filesystem
            .get(&self.scope, &path)
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
            .put(
                &self.scope,
                &path,
                entry,
                CasExpectation::Version(existing.version),
            )
            .await
        {
            Ok(_) => Ok(true),
            Err(FilesystemError::VersionMismatch { .. }) => Ok(false),
            Err(error) => Err(filesystem_error("renew prune lease", error)),
        }
    }

    async fn terminal_actions_may_exceed(
        &self,
        limit: NonZeroUsize,
    ) -> Result<bool, ProductWorkflowError> {
        let mut seen = 0usize;
        let mut offset = 0;
        let filter = terminal_action_filter()?;
        loop {
            let remaining = limit.get().saturating_add(1).saturating_sub(seen);
            if remaining == 0 {
                return Ok(true);
            }
            let page_limit = remaining.min(Page::MAX_LIMIT as usize) as u32;
            let entries = self
                .filesystem
                .query(
                    &self.scope,
                    &self.root,
                    &filter,
                    Page::new(offset, page_limit),
                )
                .await
                .map_err(|error| filesystem_error("probe terminal actions", error))?;
            let received = entries.len();
            seen = seen.saturating_add(received);
            if seen > limit.get() {
                return Ok(true);
            }
            if received < page_limit as usize {
                return Ok(false);
            }
            offset += u64::from(page_limit);
        }
    }

    async fn prune_terminal_actions(
        &self,
        actions: Vec<ProductInboundAction>,
    ) -> Result<(), ProductWorkflowError> {
        let results = futures_util::stream::iter(
            actions
                .into_iter()
                .map(|action| async move { self.prune_terminal_action_if_current(&action).await }),
        )
        .buffer_unordered(PRUNE_DELETE_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        for result in results {
            result?;
        }
        Ok(())
    }

    async fn prune_terminal_action_if_current(
        &self,
        action: &ProductInboundAction,
    ) -> Result<(), ProductWorkflowError> {
        let path = action_path(&self.root, &action.fingerprint)?;
        let Some((current, _version)) = load_action(&self.filesystem, &self.scope, &path).await?
        else {
            return Ok(());
        };
        if current.action_id != action.action_id || !current.is_terminal() {
            return Ok(());
        }
        match self.filesystem.delete(&self.scope, &path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(filesystem_error("prune terminal action", error)),
        }
    }

    async fn load_actions(
        &self,
        filter: &Filter,
    ) -> Result<Vec<ProductInboundAction>, ProductWorkflowError> {
        let mut actions = Vec::new();
        let mut offset = 0;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = self
                .filesystem
                .query(&self.scope, &self.root, filter, page)
                .await
                .map_err(|error| filesystem_error("query actions", error))?;
            let received = entries.len();
            for entry in entries {
                let action: ProductInboundAction = entry
                    .entry
                    .parse_json()
                    .map_err(|error| durable_error("deserialize action", error))?;
                actions.push(action);
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
    inner: FilesystemIdempotencyLedger<F>,
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
    inner: FilesystemIdempotencyLedger<LibSqlRootFilesystem>,
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
    inner: FilesystemIdempotencyLedger<PostgresRootFilesystem>,
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

fn settled_prune_interval_for(limit: NonZeroUsize) -> NonZeroUsize {
    NonZeroUsize::new((limit.get() / 10).max(1)).expect("non-zero derived interval") // safety: max(1) guarantees a non-zero value.
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn root_scoped_filesystem<F>(filesystem: Arc<F>, root: &ScopedPath) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem + 'static,
{
    let alias = root_mount_alias(root);
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new(alias.as_str()).expect("root mount alias is valid"), // safety: root_mount_alias returns "/" or a single absolute path segment from an existing ScopedPath.
        VirtualPath::new(alias).expect("root mount target is valid"), // safety: root_mount_alias returns an absolute virtual path accepted by VirtualPath.
        MountPermissions::read_write_list_delete(),
    )])
    .expect("root ledger mount view is valid"); // safety: the mount view contains one read-write grant with validated alias and target.
    Arc::new(ScopedFilesystem::with_fixed_view(filesystem, mounts))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn root_mount_alias(root: &ScopedPath) -> String {
    let mut parts = root.as_str().split('/').filter(|part| !part.is_empty());
    let Some(first) = parts.next() else {
        return "/".to_string();
    };
    format!("/{first}")
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn root_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant:product-workflow-storage-root")
            .expect("static tenant id is valid"), // safety: static literal uses the validated tenant id grammar.
        user_id: UserId::new("user:product-workflow-storage-root")
            .expect("static user id is valid"), // safety: static literal uses the validated user id grammar.
        agent_id: Some(
            AgentId::new("agent:product-workflow-storage-root").expect("static agent id is valid"), // safety: static literal uses the validated agent id grammar.
        ),
        project_id: Some(
            ProjectId::new("project:product-workflow-storage-root")
                .expect("static project id is valid"), // safety: static literal uses the validated project id grammar.
        ),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
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

async fn load_action<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
) -> Result<Option<(ProductInboundAction, RecordVersion)>, ProductWorkflowError>
where
    F: RootFilesystem,
{
    let Some(entry) = filesystem
        .get(scope, path)
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

fn terminal_action_filter() -> Result<Filter, ProductWorkflowError> {
    Ok(Filter::Or(vec![
        phase_filter(ActionPhase::Settled)?,
        phase_filter(ActionPhase::DeduplicatedReplay)?,
    ]))
}

fn phase_filter(phase: ActionPhase) -> Result<Filter, ProductWorkflowError> {
    Ok(Filter::Eq {
        key: index_key("phase")?,
        value: text(phase_label(phase)),
    })
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
        .ok_or_else(|| {
            durable_error(
                "deserialize prune lease expiration",
                "missing expires_at_ms",
            )
        })?;
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
