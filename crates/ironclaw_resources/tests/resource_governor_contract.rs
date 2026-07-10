// arch-exempt: large_file, resource governor contract suite decomposition, plan #5662
use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

use tempfile::tempdir;

use ironclaw_host_api::*;
use ironclaw_resources::*;
use rust_decimal_macros::dec;

#[derive(Clone)]
struct AlwaysFailingStore;

impl ResourceGovernorStore for AlwaysFailingStore {
    fn update<T, F>(&self, _update: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        Err(ResourceError::Storage {
            reason: "forced durable write failure".to_string(),
        })
    }

    fn inspect<T, F>(&self, _inspect: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        Err(ResourceError::Storage {
            reason: "forced durable read failure".to_string(),
        })
    }
}

struct RejectAppendFilesystem<F> {
    inner: F,
    append_calls: std::sync::atomic::AtomicUsize,
}

impl<F> RejectAppendFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            append_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn append_calls(&self) -> usize {
        self.append_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl<F> ironclaw_filesystem::RootFilesystem for RejectAppendFilesystem<F>
where
    F: ironclaw_filesystem::RootFilesystem,
{
    fn capabilities(&self) -> ironclaw_filesystem::BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: ironclaw_filesystem::Entry,
        cas: ironclaw_filesystem::CasExpectation,
    ) -> Result<ironclaw_filesystem::RecordVersion, ironclaw_filesystem::FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<ironclaw_filesystem::VersionedEntry>, ironclaw_filesystem::FilesystemError>
    {
        self.inner.get(path).await
    }

    async fn list_dir(
        &self,
        path: &VirtualPath,
    ) -> Result<Vec<ironclaw_filesystem::DirEntry>, ironclaw_filesystem::FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(
        &self,
        path: &VirtualPath,
    ) -> Result<ironclaw_filesystem::FileStat, ironclaw_filesystem::FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), ironclaw_filesystem::FilesystemError> {
        self.inner.delete(path).await
    }

    async fn append(
        &self,
        path: &VirtualPath,
        _payload: Vec<u8>,
    ) -> Result<ironclaw_filesystem::SeqNo, ironclaw_filesystem::FilesystemError> {
        self.append_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(ironclaw_filesystem::FilesystemError::Unsupported {
            path: path.clone(),
            operation: ironclaw_filesystem::FilesystemOperation::Append,
        })
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        _payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<ironclaw_filesystem::SeqNo>, ironclaw_filesystem::FilesystemError> {
        self.append_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(ironclaw_filesystem::FilesystemError::Unsupported {
            path: path.clone(),
            operation: ironclaw_filesystem::FilesystemOperation::Append,
        })
    }
}

struct BlockFirstAppendFilesystem<F> {
    inner: F,
    append_calls: std::sync::atomic::AtomicUsize,
    first_append_started: (std::sync::Mutex<bool>, std::sync::Condvar),
    release_first_append: (std::sync::Mutex<bool>, std::sync::Condvar),
}

impl<F> BlockFirstAppendFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            append_calls: std::sync::atomic::AtomicUsize::new(0),
            first_append_started: (std::sync::Mutex::new(false), std::sync::Condvar::new()),
            release_first_append: (std::sync::Mutex::new(false), std::sync::Condvar::new()),
        }
    }

    fn wait_for_first_append(&self) {
        let (lock, cvar) = &self.first_append_started;
        let mut started = lock.lock().expect("first append started lock");
        while !*started {
            started = cvar.wait(started).expect("first append started cvar");
        }
    }

    fn release_first_append(&self) {
        let (lock, cvar) = &self.release_first_append;
        *lock.lock().expect("first append release lock") = true;
        cvar.notify_all();
    }

    fn maybe_block_first_append(&self) {
        let call = self
            .append_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call != 0 {
            return;
        }
        {
            let (lock, cvar) = &self.first_append_started;
            *lock.lock().expect("first append started lock") = true;
            cvar.notify_all();
        }
        let (lock, cvar) = &self.release_first_append;
        let mut released = lock.lock().expect("first append release lock");
        while !*released {
            released = cvar.wait(released).expect("first append release cvar");
        }
    }
}

#[async_trait::async_trait]
impl<F> ironclaw_filesystem::RootFilesystem for BlockFirstAppendFilesystem<F>
where
    F: ironclaw_filesystem::RootFilesystem,
{
    fn capabilities(&self) -> ironclaw_filesystem::BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: ironclaw_filesystem::Entry,
        cas: ironclaw_filesystem::CasExpectation,
    ) -> Result<ironclaw_filesystem::RecordVersion, ironclaw_filesystem::FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<ironclaw_filesystem::VersionedEntry>, ironclaw_filesystem::FilesystemError>
    {
        self.inner.get(path).await
    }

    async fn list_dir(
        &self,
        path: &VirtualPath,
    ) -> Result<Vec<ironclaw_filesystem::DirEntry>, ironclaw_filesystem::FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(
        &self,
        path: &VirtualPath,
    ) -> Result<ironclaw_filesystem::FileStat, ironclaw_filesystem::FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), ironclaw_filesystem::FilesystemError> {
        self.inner.delete(path).await
    }

    async fn append(
        &self,
        path: &VirtualPath,
        payload: Vec<u8>,
    ) -> Result<ironclaw_filesystem::SeqNo, ironclaw_filesystem::FilesystemError> {
        self.maybe_block_first_append();
        self.inner.append(path, payload).await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<ironclaw_filesystem::SeqNo>, ironclaw_filesystem::FilesystemError> {
        self.maybe_block_first_append();
        self.inner.append_batch(path, payloads).await
    }
}

#[test]
fn persistent_trait_set_limit_surfaces_storage_errors() {
    let governor: Arc<dyn ResourceGovernor> =
        Arc::new(PersistentResourceGovernor::new(AlwaysFailingStore));
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let error = governor
        .set_limit(
            ResourceAccount::tenant(scope.tenant_id),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap_err();

    assert!(
        matches!(error, ResourceError::Storage { reason } if reason == "forced durable write failure")
    );
}

#[test]
fn storage_errors_display_sanitized_message_without_backend_details() {
    let error = ResourceError::Storage {
        reason: "postgres://user:secret@localhost/db failed under /tmp/private".to_string(),
    };
    let rendered = error.to_string();

    assert_eq!(rendered, "resource governor storage error");
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("/tmp/private"));
}

#[test]
fn reserve_succeeds_when_budget_is_available() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_usd(dec!(1.00))
                .set_max_concurrency_slots(2),
        )
        .unwrap();

    let reservation = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default()
                .set_usd(dec!(0.25))
                .set_concurrency_slots(1),
        )
        .unwrap();

    assert_eq!(reservation.scope, scope);
    assert_eq!(reservation.estimate.usd, Some(dec!(0.25)));
    assert_eq!(governor.reserved_for(&account).usd, dec!(0.25));
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);
}

#[test]
fn reserve_with_id_uses_requested_identifier_and_rejects_duplicates() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let reservation_id = ResourceReservationId::new();
    let estimate = ResourceEstimate::default().set_concurrency_slots(1);

    let reservation = governor
        .reserve_with_id(scope.clone(), estimate.clone(), reservation_id)
        .unwrap();

    assert_eq!(reservation.id, reservation_id);
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);
    assert!(matches!(
        governor.reserve_with_id(scope, estimate, reservation_id),
        Err(ResourceError::ReservationAlreadyExists { id }) if id == reservation_id
    ));
}

#[test]
fn reserve_with_id_rejects_negative_usd_estimates() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let err = governor
        .reserve_with_id(
            scope,
            ResourceEstimate::default().set_usd(dec!(-100.00)),
            ResourceReservationId::new(),
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::InvalidEstimate {
            dimension: ResourceDimension::Usd,
            reason: "must be non-negative"
        }
    ));
    assert_eq!(governor.reserved_for(&account).usd, dec!(0));
}

#[test]
fn reconcile_rejects_negative_usd_actuals_without_closing_reservation() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let reservation = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.25)))
        .unwrap();

    let err = governor
        .reconcile(
            reservation.id,
            ResourceUsage::default().set_usd(dec!(-100.00)),
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::InvalidEstimate {
            dimension: ResourceDimension::Usd,
            reason: "must be non-negative"
        }
    ));
    assert_eq!(governor.reserved_for(&account).usd, dec!(0.25));
    assert_eq!(governor.usage_for(&account).usd, dec!(0));
    assert!(governor.release(reservation.id).is_ok());
}

#[test]
fn usd_tally_saturates_instead_of_panicking_on_decimal_overflow() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(rust_decimal::Decimal::MAX),
        )
        .unwrap();
    governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(1)))
        .unwrap();

    assert_eq!(
        governor.reserved_for(&account).usd,
        rust_decimal::Decimal::MAX
    );
}

#[test]
fn usd_limit_check_denies_instead_of_panicking_on_decimal_overflow() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_usd(rust_decimal::Decimal::MAX),
        )
        .unwrap();

    governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(rust_decimal::Decimal::MAX),
        )
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(1)))
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account && denial.dimension == ResourceDimension::Usd
    ));
}

#[test]
fn reserve_denies_when_usd_limit_would_be_exceeded() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_usd(dec!(0.50)),
        )
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.75)))
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account && denial.dimension == ResourceDimension::Usd
    ));
}

#[test]
fn reserve_denies_runtime_quota_even_without_usd() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_wall_clock_ms(1_000)
                .set_max_process_count(1),
        )
        .unwrap();

    let err = governor
        .reserve(
            scope,
            ResourceEstimate::default()
                .set_wall_clock_ms(2_000)
                .set_process_count(1),
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account && denial.dimension == ResourceDimension::WallClockMs
    ));
}

#[test]
fn active_reservations_consume_concurrency_until_released() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_concurrency_slots(1),
        )
        .unwrap();

    let first = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_concurrency_slots(1),
        )
        .unwrap();
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 1);

    let second = governor.reserve(
        scope.clone(),
        ResourceEstimate::default().set_concurrency_slots(1),
    );
    assert!(matches!(
        second,
        Err(ResourceError::LimitExceeded { denial, .. })
            if denial.dimension == ResourceDimension::ConcurrencySlots
    ));

    governor.release(first.id).unwrap();
    assert_eq!(governor.reserved_for(&account).concurrency_slots, 0);

    governor
        .reserve(scope, ResourceEstimate::default().set_concurrency_slots(1))
        .unwrap();
}

#[test]
fn concurrent_reservations_cannot_oversubscribe_scope() {
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor
        .set_limit(
            account,
            ResourceLimits::default().set_max_concurrency_slots(1),
        )
        .unwrap();

    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let governor = Arc::clone(&governor);
        let barrier = Arc::clone(&barrier);
        let mut scope = scope.clone();
        scope.invocation_id = InvocationId::new();
        handles.push(thread::spawn(move || {
            barrier.wait();
            governor
                .reserve(scope, ResourceEstimate::default().set_concurrency_slots(1))
                .is_ok()
        }));
    }

    let successes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|success| *success)
        .count();
    assert_eq!(successes, 1);
}

#[test]
fn reconcile_records_actual_usage_and_closes_reservation() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_usd(dec!(1.00))
                .set_max_concurrency_slots(1),
        )
        .unwrap();

    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate::default()
                .set_usd(dec!(0.75))
                .set_concurrency_slots(1),
        )
        .unwrap();

    let receipt = governor
        .reconcile(
            reservation.id,
            ResourceUsage {
                usd: dec!(0.20),
                input_tokens: 10,
                output_tokens: 20,
                wall_clock_ms: 100,
                output_bytes: 50,
                network_egress_bytes: 0,
                process_count: 1,
            },
        )
        .unwrap();

    assert_eq!(receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account).usd, dec!(0.20));
    assert_eq!(governor.usage_for(&account).input_tokens, 10);
    assert!(matches!(
        governor.reconcile(reservation.id, ResourceUsage::default()),
        Err(ResourceError::ReservationClosed { .. })
    ));
    assert!(matches!(
        governor.release(reservation.id),
        Err(ResourceError::ReservationClosed {
            status: ReservationStatus::Reconciled,
            ..
        })
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account).usd, dec!(0.20));
}

#[test]
fn release_frees_reserved_capacity_without_recording_spend() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_usd(dec!(1.00))
                .set_max_concurrency_slots(1),
        )
        .unwrap();

    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate::default()
                .set_usd(dec!(0.75))
                .set_concurrency_slots(1),
        )
        .unwrap();

    let receipt = governor.release(reservation.id).unwrap();
    assert_eq!(receipt.status, ReservationStatus::Released);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
    assert!(matches!(
        governor.release(reservation.id),
        Err(ResourceError::ReservationClosed { .. })
    ));
}

#[test]
fn unknown_reservation_cannot_be_reconciled_or_released() {
    let governor = InMemoryResourceGovernor::new();
    let unknown = ResourceReservationId::new();

    assert!(matches!(
        governor.reconcile(unknown, ResourceUsage::default()),
        Err(ResourceError::UnknownReservation { id }) if id == unknown
    ));
    assert!(matches!(
        governor.release(unknown),
        Err(ResourceError::UnknownReservation { id }) if id == unknown
    ));
}

#[test]
fn tenant_limit_applies_across_projects() {
    let governor = InMemoryResourceGovernor::new();
    let project_a = sample_scope("tenant1", "user1", Some("project_a"));
    let project_b = sample_scope("tenant1", "user1", Some("project_b"));
    let tenant_account = ResourceAccount::tenant(project_a.tenant_id.clone());
    governor
        .set_limit(
            tenant_account.clone(),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();

    governor
        .reserve(project_a, ResourceEstimate::default().set_usd(dec!(0.75)))
        .unwrap();

    let err = governor
        .reserve(project_b, ResourceEstimate::default().set_usd(dec!(0.50)))
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == tenant_account && denial.dimension == ResourceDimension::Usd
    ));
}

#[test]
fn resource_governor_enforces_agent_scoped_limits_independently() {
    let governor = InMemoryResourceGovernor::new();
    let tenant = TenantId::new("tenant1").unwrap();
    let user = UserId::new("user1").unwrap();
    let agent_a = AgentId::new("agent-a").unwrap();
    let agent_b = AgentId::new("agent-b").unwrap();
    governor
        .set_limit(
            ResourceAccount::agent(tenant.clone(), user.clone(), None, agent_a.clone()),
            ResourceLimits::default().set_max_output_bytes(10),
        )
        .unwrap();

    let estimate = ResourceEstimate::default().set_output_bytes(8);
    governor
        .reserve(
            sample_scope_with_agent("tenant1", "user1", None, Some("agent-a")),
            estimate.clone(),
        )
        .unwrap();
    governor
        .reserve(
            sample_scope_with_agent("tenant1", "user1", None, Some("agent-b")),
            estimate.clone(),
        )
        .unwrap();

    let denial = governor
        .reserve(
            sample_scope_with_agent("tenant1", "user1", None, Some("agent-a")),
            estimate,
        )
        .unwrap_err();

    assert!(matches!(denial, ResourceError::LimitExceeded { .. }));
    assert_eq!(
        governor.reserved_for(&ResourceAccount::agent(tenant, user, None, agent_a)),
        ResourceTally {
            output_bytes: 8,
            ..ResourceTally::default()
        }
    );
    assert_eq!(
        governor.reserved_for(&ResourceAccount::agent(
            TenantId::new("tenant1").unwrap(),
            UserId::new("user1").unwrap(),
            None,
            agent_b,
        )),
        ResourceTally {
            output_bytes: 8,
            ..ResourceTally::default()
        }
    );
}

#[test]
fn persistent_governor_reloads_active_holds_and_usage_from_store() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_usd(dec!(1.00))
                .set_max_concurrency_slots(1),
        )
        .unwrap();
    let active = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default()
                .set_usd(dec!(0.20))
                .set_concurrency_slots(1),
        )
        .unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let concurrency_denial = reloaded
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_concurrency_slots(1),
        )
        .unwrap_err();
    assert!(matches!(
        concurrency_denial,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account
                && denial.dimension == ResourceDimension::ConcurrencySlots
                && denial.active_reserved == ResourceValue::Integer(1)
    ));

    reloaded
        .reconcile(active.id, ResourceUsage::default().set_usd(dec!(0.95)))
        .unwrap();

    let reloaded_again = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let usd_denial = reloaded_again
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.10)))
        .unwrap_err();
    assert!(matches!(
        usd_denial,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account
                && denial.dimension == ResourceDimension::Usd
                && denial.current_usage == ResourceValue::Decimal(dec!(0.95))
    ));
}

#[test]
fn persistent_governor_unlimited_fast_path_avoids_durable_writes_until_finite_limit() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path))
        .with_unlimited_fast_path();
    let reservation = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default()
                .set_usd(dec!(0.20))
                .set_concurrency_slots(1),
        )
        .unwrap();
    assert!(
        !path.exists(),
        "unlimited fast path should not create the durable governor snapshot"
    );

    governor
        .reconcile(reservation.id, ResourceUsage::default().set_usd(dec!(0.20)))
        .unwrap();
    assert!(
        matches!(
            governor.release(reservation.id),
            Err(ResourceError::ReservationClosed {
                status: ReservationStatus::Reconciled,
                ..
            })
        ),
        "same-process lifecycle checks are still enforced"
    );
    assert!(
        !path.exists(),
        "reconcile on the unlimited fast path should not create the durable snapshot"
    );

    let active = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_concurrency_slots(1),
        )
        .unwrap();
    assert!(
        !path.exists(),
        "active unlimited fast-path reservations should stay process-local before finite limits"
    );

    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_concurrency_slots(1),
        )
        .unwrap();
    let denied = governor
        .reserve(scope, ResourceEstimate::default().set_concurrency_slots(1))
        .unwrap_err();
    assert!(matches!(
        denied,
        ResourceError::LimitExceeded {
            denial,
            ..
        } if denial.dimension == ResourceDimension::ConcurrencySlots
    ));
    governor.release(active.id).unwrap();
}

#[test]
fn persistent_governor_unlimited_fast_path_ignores_legacy_durable_activity() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
    let user_account = ResourceAccount::user(scope.tenant_id.clone(), scope.user_id.clone());
    let unrelated_account = ResourceAccount::tenant(TenantId::new("tenant-unrelated").unwrap());

    let legacy_governor =
        PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    legacy_governor
        .set_limit(tenant_account.clone(), ResourceLimits::default())
        .unwrap();
    let legacy_reservation = legacy_governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default()
                .set_usd(dec!(0.20))
                .set_concurrency_slots(1),
        )
        .unwrap();
    legacy_governor
        .reconcile(
            legacy_reservation.id,
            ResourceUsage::default().set_usd(dec!(0.20)),
        )
        .unwrap();
    let legacy_usage = legacy_governor.usage_for(&tenant_account).unwrap();
    assert_eq!(
        legacy_usage.usd,
        dec!(0.20),
        "test setup must leave durable usage for the fast path to ignore"
    );
    let before_fast_path = fs::read_to_string(&path).unwrap();
    let before_fast_path_json: serde_json::Value = serde_json::from_str(&before_fast_path).unwrap();
    assert!(
        before_fast_path_json["state"]["usage_by_account"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty()),
        "legacy fixture must contain durable usage so the test exercises accounting cleanup"
    );
    assert!(
        before_fast_path_json["state"]["reservations"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty()),
        "legacy fixture must contain durable reservations so the test exercises accounting cleanup"
    );

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path))
        .with_unlimited_fast_path();
    governor
        .set_limit(unrelated_account, ResourceLimits::default())
        .unwrap();
    assert_eq!(
        governor.usage_for(&user_account).unwrap(),
        ResourceTally::default(),
        "unlimited set_limit must not seed local fast-path state with legacy durable usage"
    );
    let after_unlimited_limit = fs::read_to_string(&path).unwrap();

    let reservation = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default()
                .set_usd(dec!(0.10))
                .set_concurrency_slots(1),
        )
        .unwrap();
    governor
        .reconcile(reservation.id, ResourceUsage::default().set_usd(dec!(0.10)))
        .unwrap();

    assert!(
        matches!(
            governor.release(reservation.id),
            Err(ResourceError::ReservationClosed {
                status: ReservationStatus::Reconciled,
                ..
            })
        ),
        "same-process lifecycle checks should still use local state"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        after_unlimited_limit,
        "legacy durable usage/reservations should not force unlimited fast-path writes"
    );

    governor
        .set_limit(
            tenant_account.clone(),
            ResourceLimits::default().set_max_usd(dec!(10.00)),
        )
        .unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let snapshot = reloaded.account_snapshot(&tenant_account).unwrap().unwrap();
    assert_eq!(
        snapshot.ledger.spent.usd,
        dec!(0.30),
        "finite-limit transition should preserve legacy durable usage and merge fast-path usage"
    );
}

#[test]
fn persistent_governor_serializes_concurrent_reservations_across_handles() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(
            account,
            ResourceLimits::default().set_max_concurrency_slots(1),
        )
        .unwrap();

    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        let mut scope = scope.clone();
        scope.invocation_id = InvocationId::new();
        handles.push(thread::spawn(move || {
            let governor =
                PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(path));
            barrier.wait();
            governor
                .reserve(scope, ResourceEstimate::default().set_concurrency_slots(1))
                .is_ok()
        }));
    }

    let successes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|success| *success)
        .count();
    assert_eq!(successes, 1);
}

#[test]
fn persistent_governor_writes_versioned_snapshot_schema() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id);

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(account, ResourceLimits::default().set_max_usd(dec!(1.00)))
        .unwrap();

    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(snapshot["schema_version"], serde_json::json!(3));
}

#[test]
fn persistent_governor_upgrades_legacy_unversioned_snapshot() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(
        &path,
        r#"{
            "state": {
                "limits": [],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            }
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();

    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(snapshot["schema_version"], serde_json::json!(3));
}

#[test]
fn persistent_governor_rejects_malformed_snapshot_with_storage_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(&path, "{not valid json").unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("malformed resource governor snapshot")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_snapshot_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(
        &path,
        r#"{
            "schema_version": 1,
            "state": {
                "limits": [],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            },
            "unexpected": true
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_persisted_resource_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(
        &path,
        r#"{
            "schema_version": 1,
            "state": {
                "limits": [
                    [
                        { "Tenant": { "tenant_id": "tenant1" } },
                        { "max_usd": "1.00", "unexpected_limit": true }
                    ]
                ],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            }
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_reservation_scope_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .reserve(scope.clone(), ResourceEstimate::default())
        .unwrap();

    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    snapshot["state"]["reservations"].as_array_mut().unwrap()[0][1]["reservation"]["scope"]["unexpected_scope"] =
        serde_json::json!(true);
    fs::write(&path, serde_json::to_string_pretty(&snapshot).unwrap()).unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = reloaded
        .try_set_limit(
            ResourceAccount::tenant(scope.tenant_id),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_reservation_estimate_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(0.20)),
        )
        .unwrap();

    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    snapshot["state"]["reservations"].as_array_mut().unwrap()[0][1]["reservation"]["estimate"]["unexpected_estimate"] =
        serde_json::json!(true);
    fs::write(&path, serde_json::to_string_pretty(&snapshot).unwrap()).unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = reloaded
        .try_set_limit(
            ResourceAccount::tenant(scope.tenant_id),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_unknown_reservation_actual_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let reservation = governor
        .reserve(scope.clone(), ResourceEstimate::default())
        .unwrap();
    governor
        .reconcile(reservation.id, ResourceUsage::default().set_usd(dec!(0.20)))
        .unwrap();

    let mut snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    snapshot["state"]["reservations"].as_array_mut().unwrap()[0][1]["actual"]["unexpected_actual"] =
        serde_json::json!(true);
    fs::write(&path, serde_json::to_string_pretty(&snapshot).unwrap()).unwrap();

    let reloaded = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = reloaded
        .try_set_limit(
            ResourceAccount::tenant(scope.tenant_id),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("unknown field")
    ));
}

#[test]
fn persistent_governor_rejects_partial_snapshot_fields() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(&path, r#"{"schema_version": 1}"#).unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason } if reason.contains("missing field")
    ));
}

#[test]
fn persistent_governor_rejects_unsupported_snapshot_schema_version() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("resource-governor.json");
    fs::write(
        &path,
        r#"{
            "schema_version": 999,
            "state": {
                "limits": [],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            }
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    let error = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ResourceError::Storage { reason }
            if reason.contains("unsupported resource governor snapshot schema version 999")
    ));
}

/// Filesystem-backed reload contract — replaces the deleted libSQL /
/// Postgres `*_persistent_governor_reloads_active_holds_and_usage_from_store`
/// tests. Backend choice is now a property of the underlying
/// `RootFilesystem`; this test exercises the on-disk snapshot
/// round-trip through `ScopedFilesystem` so durability across reopen is
/// covered by the same surface (a single `FilesystemResourceGovernorStore`
/// constructed twice over the same backing store).
#[tokio::test]
async fn filesystem_persistent_governor_reloads_active_holds_and_usage_from_store() {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let backend = std::sync::Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = std::sync::Arc::new(ScopedFilesystem::with_fixed_view(
        std::sync::Arc::clone(&backend),
        mounts,
    ));

    let store = FilesystemResourceGovernorStore::new(std::sync::Arc::clone(&scoped));

    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let governor = PersistentResourceGovernor::new(store);
    governor
        .try_set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_usd(dec!(1.00))
                .set_max_concurrency_slots(1),
        )
        .unwrap();
    let active = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_concurrency_slots(1),
        )
        .unwrap();

    // Reload from the same on-disk snapshot via a fresh
    // FilesystemResourceGovernorStore handle over the same ScopedFilesystem.
    let reloaded = PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(
        std::sync::Arc::clone(&scoped),
    ));
    let concurrency_denial = reloaded
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_concurrency_slots(1),
        )
        .unwrap_err();
    assert!(matches!(
        concurrency_denial,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account && denial.dimension == ResourceDimension::ConcurrencySlots
    ));

    reloaded
        .reconcile(active.id, ResourceUsage::default().set_usd(dec!(0.95)))
        .unwrap();
    let usd_denial = reloaded
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.10)))
        .unwrap_err();
    assert!(matches!(
        usd_denial,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account
                && denial.dimension == ResourceDimension::Usd
                && denial.current_usage == ResourceValue::Decimal(dec!(0.95))
    ));
}

#[tokio::test]
async fn filesystem_resource_governor_replays_journaled_holds_and_usage_after_restart() {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let backend = Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        mounts,
    ));

    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let governor = FilesystemResourceGovernor::new(Arc::clone(&scoped));
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let active = governor
        .reserve(
            scope.clone(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();

    let reloaded = FilesystemResourceGovernor::new(Arc::clone(&scoped));
    let concurrency_denial = reloaded
        .reserve(
            scope.clone(),
            ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap_err();
    assert!(matches!(
        concurrency_denial,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account && denial.dimension == ResourceDimension::ConcurrencySlots
    ));

    reloaded
        .reconcile(
            active.id,
            ResourceUsage {
                usd: dec!(0.80),
                ..ResourceUsage::default()
            },
        )
        .unwrap();

    let reloaded_again = FilesystemResourceGovernor::new(scoped);
    let snapshot = reloaded_again.account_snapshot(&account).unwrap().unwrap();
    assert_eq!(snapshot.ledger.spent.usd, dec!(0.80));
    assert_eq!(snapshot.ledger.reserved.concurrency_slots, 0);
}

#[tokio::test]
async fn filesystem_resource_governor_serializes_concurrent_reservations_on_shared_handle() {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let backend = Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        mounts,
    ));

    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let governor = Arc::new(FilesystemResourceGovernor::new(Arc::clone(&scoped)));
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_concurrency_slots: Some(1),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let workers = 16;
    let barrier = Arc::new(Barrier::new(workers));
    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let governor = Arc::clone(&governor);
            let barrier = Arc::clone(&barrier);
            let mut request_scope = scope.clone();
            request_scope.invocation_id = InvocationId::new();
            thread::spawn(move || {
                barrier.wait();
                governor
                    .reserve(
                        request_scope,
                        ResourceEstimate {
                            concurrency_slots: Some(1),
                            ..ResourceEstimate::default()
                        },
                    )
                    .is_ok()
            })
        })
        .collect();

    let successes = handles
        .into_iter()
        .map(|handle| handle.join().expect("reservation thread joins"))
        .filter(|accepted| *accepted)
        .count();
    assert_eq!(
        successes, 1,
        "shared filesystem governor handle must not oversubscribe concurrency"
    );

    let reloaded = FilesystemResourceGovernor::new(scoped);
    let snapshot = reloaded.account_snapshot(&account).unwrap().unwrap();
    assert_eq!(snapshot.ledger.reserved.concurrency_slots, 1);
}

#[tokio::test]
async fn filesystem_resource_governor_releases_account_gate_before_delta_ack() {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let backend = Arc::new(BlockFirstAppendFilesystem::new(InMemoryBackend::new()));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        mounts,
    ));

    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let governor = Arc::new(FilesystemResourceGovernor::new(scoped));
    let estimate = ResourceEstimate {
        concurrency_slots: Some(1),
        ..ResourceEstimate::default()
    };

    let first = {
        let governor = Arc::clone(&governor);
        let scope = scope.clone();
        let estimate = estimate.clone();
        thread::spawn(move || governor.reserve(scope, estimate).map(|_| ()))
    };
    backend.wait_for_first_append();

    let second = {
        let governor = Arc::clone(&governor);
        let mut scope = scope.clone();
        scope.invocation_id = InvocationId::new();
        let estimate = estimate.clone();
        thread::spawn(move || governor.reserve(scope, estimate).map(|_| ()))
    };

    thread::sleep(Duration::from_millis(50));
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = {
        let governor = Arc::clone(&governor);
        let account = account.clone();
        thread::spawn(move || {
            let tally = governor
                .reserved_for(&account)
                .expect("reserved tally remains readable while append is blocked");
            tx.send(tally.concurrency_slots)
                .expect("send reserved tally");
        })
    };

    let observed = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("account gate should be released before durable append ack");
    backend.release_first_append();
    first
        .join()
        .expect("first reserve thread joins")
        .expect("first reserve succeeds");
    second
        .join()
        .expect("second reserve thread joins")
        .expect("second reserve succeeds");
    reader.join().expect("reader thread joins");

    assert_eq!(
        observed, 2,
        "both reservations should be visible in memory while the first append ack is pending"
    );
}

#[tokio::test]
async fn filesystem_resource_governor_fails_closed_and_poisoned_after_delta_append_error() {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let backend = Arc::new(RejectAppendFilesystem::new(InMemoryBackend::new()));
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        mounts,
    ));

    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let governor = FilesystemResourceGovernor::new(scoped);

    let error = governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(1.00)),
                ..ResourceLimits::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(error, ResourceError::Storage { .. }),
        "delta append failure must surface as storage error: {error:?}"
    );
    assert_eq!(backend.append_calls(), 1);

    let poisoned = governor.account_snapshot(&account).unwrap_err();
    assert!(
        matches!(poisoned, ResourceError::Storage { .. }),
        "authority must fail closed after a durable journal error: {poisoned:?}"
    );
}

/// Regression: a byte-only `RootFilesystem` (one that rejects `put` when
/// `Entry::kind` is set) must surface `CasUpdateError::CasUnsupported` ->
/// `ResourceError::Storage` via `map_cas_error` (cas_snapshot.rs:243-258)
/// rather than silently succeeding with a blind overwrite. Today this
/// crate's filesystem-store tests only exercise `InMemoryBackend`, which
/// supports versioned CAS and therefore never takes the
/// `CasUnsupported` branch.
///
/// `LocalFilesystem` is used here because it is the canonical byte-only
/// `RootFilesystem`: its `put` impl rejects entries with
/// `entry.kind.is_some()`, which `cas_update` maps to `CasUnsupported`.
/// Mirrors `ironclaw_run_state`'s
/// `filesystem_approval_store_fails_closed_on_byte_only_backend`
/// regression
/// (crates/ironclaw_run_state/tests/run_state_contract.rs:1027-1048) for
/// the resources crate's CAS snapshot stores.
#[tokio::test]
async fn filesystem_resource_governor_store_fails_closed_on_byte_only_backend() {
    use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{
        HostPath, MountAlias, MountGrant, MountPermissions, MountView, VirtualPath,
    };

    let dir = tempdir().expect("temp dir");
    let mut local_fs = LocalFilesystem::new();
    local_fs
        .mount_local(
            VirtualPath::new("/tenants").expect("virtual root"),
            HostPath::from_path_buf(dir.path().to_path_buf()),
        )
        .expect("mount /tenants at temp dir");

    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(local_fs),
        mounts,
    ));

    let governor = PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(scoped));

    let err = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(
        matches!(&err, ResourceError::Storage { reason } if reason.contains("compare-and-swap")),
        "expected Storage(CasUnsupported) from byte-only LocalFilesystem but got {err:?}",
    );
}

/// Mirrors `filesystem_resource_governor_store_fails_closed_on_byte_only_backend`
/// for `FilesystemBudgetGateStore`. Both stores route through the same
/// shared `CasSnapshotStore` encoder (cas_snapshot.rs:221-227) and
/// `map_cas_error` (cas_snapshot.rs:243-258), so a byte-only backend must
/// fail closed for budget-gate writes too rather than blind-overwriting a
/// pending gate.
#[tokio::test]
async fn filesystem_budget_gate_store_fails_closed_on_byte_only_backend() {
    use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{
        HostPath, MountAlias, MountGrant, MountPermissions, MountView, VirtualPath,
    };

    let dir = tempdir().expect("temp dir");
    let mut local_fs = LocalFilesystem::new();
    local_fs
        .mount_local(
            VirtualPath::new("/tenants").expect("virtual root"),
            HostPath::from_path_buf(dir.path().to_path_buf()),
        )
        .expect("mount /tenants at temp dir");

    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(local_fs),
        mounts,
    ));

    let store = FilesystemBudgetGateStore::new(scoped);
    let scope = sample_scope("tenant1", "user1", None);
    let gate = BudgetApprovalGate {
        id: BudgetGateId::new(),
        needed: ResourceApprovalNeeded {
            account: ResourceAccount::tenant(scope.tenant_id.clone()),
            dimension: ResourceDimension::Usd,
            limit: ResourceValue::Decimal(dec!(10)),
            current_usage: ResourceValue::Decimal(dec!(0)),
            active_reserved: ResourceValue::Decimal(dec!(0)),
            requested: ResourceValue::Decimal(dec!(9)),
            utilization: 0.91,
            period_end: None,
        },
        opened_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
        status: BudgetGateStatus::Pending,
    };

    let err = store.open(&scope, gate).unwrap_err();
    assert!(
        matches!(&err, BudgetGateError::Storage { reason } if reason.contains("compare-and-swap")),
        "expected Storage(CasUnsupported) from byte-only LocalFilesystem but got {err:?}",
    );
}

/// Backend wrapper that races *every* versioned `put` against a watched
/// path, ported verbatim (mechanics) from `ironclaw_secrets`'s
/// `AlwaysRacingBackend`
/// (crates/ironclaw_secrets/src/filesystem_store.rs:2085-2127) for the PR
/// #5234 review follow-up (Medium): no resource-caller test in this crate
/// drove a *persistent* `FilesystemError::VersionMismatch` through
/// `FilesystemResourceGovernorStore`/`FilesystemBudgetGateStore` to pin
/// `map_cas_error`'s `CasUpdateError::RetriesExhausted` ->
/// `ResourceError::Storage` mapping (cas_snapshot.rs:265-267). The
/// byte-only tests above only exercise `CasUnsupported`; the helper crate
/// (`ironclaw_secrets`) separately pins persistent `VersionMismatch`, but
/// nothing here did for a resource-governor caller.
///
/// On every `put` against the watched path with a `CasExpectation::Version`
/// precondition, an out-of-band write under `Any` bumps the stored version
/// first, so the delegated put always observes a stale version and returns
/// `VersionMismatch` — driving `cas_update` past `FILESYSTEM_CAS_RETRIES`
/// (32) on every attempt rather than just the first, so the retry budget is
/// exhausted instead of recovered.
struct PersistentVersionMismatchBackend {
    inner: Arc<ironclaw_filesystem::InMemoryBackend>,
    watched: String,
    races: std::sync::atomic::AtomicUsize,
}

impl PersistentVersionMismatchBackend {
    fn new(
        inner: Arc<ironclaw_filesystem::InMemoryBackend>,
        watched: ironclaw_host_api::VirtualPath,
    ) -> Self {
        Self {
            inner,
            watched: watched.as_str().to_string(),
            races: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn races(&self) -> usize {
        self.races.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ironclaw_filesystem::RootFilesystem for PersistentVersionMismatchBackend {
    fn capabilities(&self) -> ironclaw_filesystem::BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &ironclaw_host_api::VirtualPath,
        entry: ironclaw_filesystem::Entry,
        cas: ironclaw_filesystem::CasExpectation,
    ) -> Result<ironclaw_filesystem::RecordVersion, ironclaw_filesystem::FilesystemError> {
        let should_race = path.as_str() == self.watched
            && matches!(cas, ironclaw_filesystem::CasExpectation::Version(_));
        if should_race && let Some(current) = self.inner.get(path).await? {
            self.races.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = self
                .inner
                .put(
                    path,
                    current.entry,
                    ironclaw_filesystem::CasExpectation::Any,
                )
                .await;
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(
        &self,
        path: &ironclaw_host_api::VirtualPath,
    ) -> Result<Option<ironclaw_filesystem::VersionedEntry>, ironclaw_filesystem::FilesystemError>
    {
        self.inner.get(path).await
    }

    async fn list_dir(
        &self,
        path: &ironclaw_host_api::VirtualPath,
    ) -> Result<Vec<ironclaw_filesystem::DirEntry>, ironclaw_filesystem::FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(
        &self,
        path: &ironclaw_host_api::VirtualPath,
    ) -> Result<ironclaw_filesystem::FileStat, ironclaw_filesystem::FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(
        &self,
        path: &ironclaw_host_api::VirtualPath,
    ) -> Result<(), ironclaw_filesystem::FilesystemError> {
        self.inner.delete(path).await
    }

    async fn query(
        &self,
        path: &ironclaw_host_api::VirtualPath,
        filter: &ironclaw_filesystem::Filter,
        page: ironclaw_filesystem::Page,
    ) -> Result<Vec<ironclaw_filesystem::VersionedEntry>, ironclaw_filesystem::FilesystemError>
    {
        self.inner.query(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &ironclaw_host_api::VirtualPath,
        spec: &ironclaw_filesystem::IndexSpec,
    ) -> Result<(), ironclaw_filesystem::FilesystemError> {
        self.inner.ensure_index(path, spec).await
    }
}

/// Drives a *persistent* `VersionMismatch` through
/// `FilesystemResourceGovernorStore::try_set_limit` and pins the
/// `CasUpdateError::RetriesExhausted` -> `ResourceError::Storage` mapping
/// (cas_snapshot.rs:265-267, PR #5234 review follow-up, Medium). Companion
/// to `filesystem_resource_governor_store_fails_closed_on_byte_only_backend`
/// above, which pins the sibling `CasUnsupported` branch.
#[tokio::test]
async fn filesystem_resource_governor_store_surfaces_storage_error_on_persistent_version_mismatch()
{
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, ScopedPath};

    let inner = Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("alias"),
        VirtualPath::new("/tenants/tenant1/users/user1/resources").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");

    // Resolve the snapshot's virtual path the same way the production store
    // does (alias-relative `/resources/snapshot.json` under the store's
    // default scope, `ResourceScope::system()`), so the wrapper below races
    // the exact path `FilesystemResourceGovernorStore` writes.
    let bootstrap_scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&inner),
        mounts.clone(),
    ));
    let watched = bootstrap_scoped
        .resolve(
            &ResourceScope::system(),
            &ScopedPath::new("/resources/snapshot.json".to_string()).expect("scoped path"),
        )
        .expect("resolve snapshot path");

    // Seed the snapshot file via a plain (non-racing) store first. The
    // very first write to an absent path goes through
    // `CasExpectation::Absent`, not `Version(_)` (cas.rs:328-331), so the
    // wrapper — which only races a `Version(_)` precondition — would never
    // see a race on a from-scratch snapshot. Bootstrapping ensures the
    // mutation under test lands on an *existing* snapshot whose every
    // retry attempt carries `CasExpectation::Version(_)`.
    PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(Arc::clone(
        &bootstrap_scoped,
    )))
    .try_set_limit(
        ResourceAccount::tenant(TenantId::new("tenant-bootstrap").unwrap()),
        ResourceLimits::default(),
    )
    .expect("bootstrap write to seed the snapshot");

    let racing = Arc::new(PersistentVersionMismatchBackend::new(
        Arc::clone(&inner),
        watched,
    ));
    let racing_scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&racing),
        mounts,
    ));
    let governor =
        PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(racing_scoped));

    let err = governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default(),
        )
        .unwrap_err();

    assert!(
        matches!(&err, ResourceError::Storage { reason } if reason.contains("retries exhausted")),
        "expected Storage(RetriesExhausted) from a backend that perpetually \
         races the CAS version but got {err:?}",
    );
    assert_eq!(
        racing.races(),
        ironclaw_filesystem::FILESYSTEM_CAS_RETRIES,
        "every retry attempt must have raced the same path"
    );
}

fn sample_scope(tenant: &str, user: &str, project: Option<&str>) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: project.map(|value| ProjectId::new(value).unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_scope_with_agent(
    tenant: &str,
    user: &str,
    project: Option<&str>,
    agent: Option<&str>,
) -> ResourceScope {
    let mut scope = sample_scope(tenant, user, project);
    scope.agent_id = agent.map(|id| AgentId::new(id).unwrap());
    scope
}

#[test]
fn project_and_agent_limits_both_apply_without_override() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope_with_agent("tenant1", "user1", Some("project1"), Some("agent1"));
    let project_account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    let agent_account = ResourceAccount::agent(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone(),
        scope.agent_id.clone().unwrap(),
    );

    governor
        .set_limit(
            project_account.clone(),
            ResourceLimits::default().set_max_usd(dec!(0.50)),
        )
        .unwrap();
    governor
        .set_limit(
            agent_account.clone(),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();

    let err = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(0.75)),
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == project_account && denial.dimension == ResourceDimension::Usd
    ));

    governor
        .set_limit(
            project_account,
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();
    governor
        .set_limit(
            agent_account.clone(),
            ResourceLimits::default().set_max_usd(dec!(0.50)),
        )
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.75)))
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == agent_account && denial.dimension == ResourceDimension::Usd
    ));
}

#[test]
fn reservation_and_usage_are_charged_to_full_scope_cascade() {
    let governor = InMemoryResourceGovernor::new();
    let mut scope = sample_scope("tenant1", "user1", Some("project1"));
    scope.mission_id = Some(MissionId::new("mission1").unwrap());
    scope.thread_id = Some(ThreadId::new("thread1").unwrap());

    let accounts = ResourceAccount::cascade(&scope);
    assert_eq!(accounts.len(), 5);

    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate::default()
                .set_usd(dec!(0.10))
                .set_output_bytes(100)
                .set_concurrency_slots(1),
        )
        .unwrap();

    for account in &accounts {
        let reserved = governor.reserved_for(account);
        assert_eq!(reserved.usd, dec!(0.10));
        assert_eq!(reserved.output_bytes, 100);
        assert_eq!(reserved.concurrency_slots, 1);
    }

    governor
        .reconcile(
            reservation.id,
            ResourceUsage {
                usd: dec!(0.06),
                input_tokens: 11,
                output_tokens: 7,
                wall_clock_ms: 55,
                output_bytes: 80,
                network_egress_bytes: 9,
                process_count: 1,
            },
        )
        .unwrap();

    for account in &accounts {
        assert_eq!(governor.reserved_for(account), ResourceTally::default());
        let usage = governor.usage_for(account);
        assert_eq!(usage.usd, dec!(0.06));
        assert_eq!(usage.input_tokens, 11);
        assert_eq!(usage.output_tokens, 7);
        assert_eq!(usage.wall_clock_ms, 55);
        assert_eq!(usage.output_bytes, 80);
        assert_eq!(usage.network_egress_bytes, 9);
        assert_eq!(usage.process_count, 1);
        assert_eq!(usage.concurrency_slots, 0);
    }
}

#[test]
fn project_limit_denies_leaf_even_when_tenant_allows() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let tenant = ResourceAccount::tenant(scope.tenant_id.clone());
    let project = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor
        .set_limit(tenant, ResourceLimits::default().set_max_usd(dec!(10.00)))
        .unwrap();
    governor
        .set_limit(
            project.clone(),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(1.50)))
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == project && denial.dimension == ResourceDimension::Usd
    ));
}

#[test]
fn reconciled_usage_counts_against_future_reservations() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();

    let reservation = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(0.20)),
        )
        .unwrap();
    governor
        .reconcile(reservation.id, ResourceUsage::default().set_usd(dec!(0.80)))
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.30)))
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account
                && denial.dimension == ResourceDimension::Usd
                && denial.current_usage == ResourceValue::Decimal(dec!(0.80))
                && denial.active_reserved == ResourceValue::Decimal(dec!(0))
                && denial.requested == ResourceValue::Decimal(dec!(0.30))
    ));
}

#[test]
fn active_reserved_and_usage_appear_in_denial_details() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();

    let completed = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(0.40)),
        )
        .unwrap();
    governor
        .reconcile(completed.id, ResourceUsage::default().set_usd(dec!(0.40)))
        .unwrap();

    governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(0.30)),
        )
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.40)))
        .unwrap_err();

    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account
                && denial.dimension == ResourceDimension::Usd
                && denial.limit == ResourceValue::Decimal(dec!(1.00))
                && denial.current_usage == ResourceValue::Decimal(dec!(0.40))
                && denial.active_reserved == ResourceValue::Decimal(dec!(0.30))
                && denial.requested == ResourceValue::Decimal(dec!(0.40))
    ));
}

#[test]
fn actual_usage_above_estimate_is_recorded_and_blocks_future_work() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();

    let reservation = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(0.20)),
        )
        .unwrap();
    governor
        .reconcile(reservation.id, ResourceUsage::default().set_usd(dec!(0.95)))
        .unwrap();

    assert_eq!(governor.usage_for(&account).usd, dec!(0.95));
    assert!(matches!(
        governor.reserve(
            scope,
            ResourceEstimate::default().set_usd(dec!(0.10)),
        ),
        Err(ResourceError::LimitExceeded { denial, .. })
            if denial.current_usage == ResourceValue::Decimal(dec!(0.95))
    ));
}

#[test]
fn closed_reservations_reject_cross_lifecycle_operations_with_status() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));

    let reconciled = governor
        .reserve(scope.clone(), ResourceEstimate::default())
        .unwrap();
    governor
        .reconcile(reconciled.id, ResourceUsage::default())
        .unwrap();
    assert!(matches!(
        governor.release(reconciled.id),
        Err(ResourceError::ReservationClosed {
            status: ReservationStatus::Reconciled,
            ..
        })
    ));

    let released = governor
        .reserve(scope, ResourceEstimate::default())
        .unwrap();
    governor.release(released.id).unwrap();
    assert!(matches!(
        governor.reconcile(released.id, ResourceUsage::default()),
        Err(ResourceError::ReservationClosed {
            status: ReservationStatus::Released,
            ..
        })
    ));
}

#[test]
fn non_usd_dimensions_can_deny_reservations() {
    assert_denied_dimension(
        ResourceLimits::default().set_max_input_tokens(10),
        ResourceEstimate::default().set_input_tokens(11),
        ResourceDimension::InputTokens,
    );
    assert_denied_dimension(
        ResourceLimits::default().set_max_output_tokens(10),
        ResourceEstimate::default().set_output_tokens(11),
        ResourceDimension::OutputTokens,
    );
    assert_denied_dimension(
        ResourceLimits::default().set_max_output_bytes(10),
        ResourceEstimate::default().set_output_bytes(11),
        ResourceDimension::OutputBytes,
    );
    assert_denied_dimension(
        ResourceLimits::default().set_max_network_egress_bytes(10),
        ResourceEstimate::default().set_network_egress_bytes(11),
        ResourceDimension::NetworkEgressBytes,
    );
    assert_denied_dimension(
        ResourceLimits::default().set_max_process_count(1),
        ResourceEstimate::default().set_process_count(2),
        ResourceDimension::ProcessCount,
    );
}

fn assert_denied_dimension(
    limits: ResourceLimits,
    estimate: ResourceEstimate,
    expected: ResourceDimension,
) {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant1", "user1", Some("project1"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(account.clone(), limits).unwrap();

    let err = governor.reserve(scope, estimate).unwrap_err();
    assert!(matches!(
        err,
        ResourceError::LimitExceeded { denial, .. }
            if denial.account == account && denial.dimension == expected
    ));
}

// =====================================================================
// Phase 0 — period, thresholds, 0=unlimited (cost-based budgeting)
// =====================================================================

#[test]
fn zero_usd_limit_treated_as_unlimited() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant-zero", "user-zero", Some("project-zero"));
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(account, ResourceLimits::default().set_max_usd(dec!(0)))
        .unwrap();
    // A reservation that would clearly exceed any non-zero cap still succeeds
    // because 0 is the "explicit no cap" sentinel.
    governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(1_000_000)))
        .unwrap();
}

#[test]
fn zero_integer_limit_treated_as_unlimited() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant-zero2", "user-zero2", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account,
            ResourceLimits::default().set_max_concurrency_slots(0),
        )
        .unwrap();
    governor
        .reserve(
            scope,
            ResourceEstimate::default().set_concurrency_slots(u32::MAX),
        )
        .unwrap();
}

#[test]
fn reserve_with_outcome_returns_warning_above_warn_threshold_below_pause() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant-warn", "user-warn", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                thresholds: BudgetThresholds {
                    warn_at: 0.75,
                    pause_at: 0.90,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let outcome = governor
        .reserve_with_outcome(scope, ResourceEstimate::default().set_usd(dec!(8.00)))
        .unwrap();
    assert_eq!(outcome.warnings.len(), 1);
    assert_eq!(outcome.warnings[0].account, account);
    assert_eq!(outcome.warnings[0].dimension, ResourceDimension::Usd);
    assert!(outcome.warnings[0].utilization >= 0.75);
    assert!(outcome.warnings[0].utilization < 0.90);
}

#[test]
fn reserve_returns_requires_approval_above_pause_below_hard_limit() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant-pause", "user-pause", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                thresholds: BudgetThresholds {
                    warn_at: 0.75,
                    pause_at: 0.90,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(9.50)))
        .unwrap_err();
    match err {
        ResourceError::RequiresApproval { needed, .. } => {
            assert_eq!(needed.account, account);
            assert_eq!(needed.dimension, ResourceDimension::Usd);
            assert!(needed.utilization >= 0.90);
            assert!(needed.utilization < 1.0);
        }
        other => panic!("expected RequiresApproval, got {other:?}"),
    }
}

#[test]
fn hard_limit_overrun_returns_limit_exceeded_not_requires_approval() {
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant-hard", "user-hard", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                thresholds: BudgetThresholds {
                    warn_at: 0.75,
                    pause_at: 0.90,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(11.00)))
        .unwrap_err();
    assert!(matches!(err, ResourceError::LimitExceeded { .. }));
}

#[test]
fn account_snapshot_returns_none_for_untouched_account() {
    let governor = InMemoryResourceGovernor::new();
    let account = ResourceAccount::tenant(TenantId::new("untouched").unwrap());
    assert!(governor.account_snapshot(&account).unwrap().is_none());
}

#[test]
fn account_snapshot_reports_current_period_and_spend() {
    let clock = FakeClock::new(chrono::Utc::now());
    let governor = InMemoryResourceGovernor::with_clock(Arc::new(clock.clone()));
    let scope = sample_scope("tenant-snap", "user-snap", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_usd(dec!(5.00))
                .set_period(BudgetPeriod::Rolling24h),
        )
        .unwrap();

    let reservation = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.50)))
        .unwrap();
    governor
        .reconcile(reservation.id, ResourceUsage::default().set_usd(dec!(0.50)))
        .unwrap();

    let snapshot = governor.account_snapshot(&account).unwrap().unwrap();
    assert_eq!(snapshot.account, account);
    assert_eq!(snapshot.ledger.spent.usd, dec!(0.50));
    assert!(snapshot.ledger.period_end > snapshot.ledger.period_start);
}

#[test]
fn rolling_24h_snapshot_reports_anchored_window_not_now_window() {
    // Regression: account_snapshot used to call `period_bounds(now)`, which
    // anchors Rolling24h at the *current* wall clock. After advancing the
    // FakeClock the reported window slides with it, breaking the UI
    // contract that the window covers the ledger's actual accumulation.
    let start = chrono::DateTime::parse_from_rfc3339("2026-05-21T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let clock = FakeClock::new(start);
    let governor = InMemoryResourceGovernor::with_clock(Arc::new(clock.clone()));
    let scope = sample_scope("tenant-roll", "user-roll", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor
        .set_limit(
            account.clone(),
            ResourceLimits::default()
                .set_max_usd(dec!(5.00))
                .set_period(BudgetPeriod::Rolling24h),
        )
        .unwrap();

    let initial = governor.account_snapshot(&account).unwrap().unwrap();
    let initial_end = initial.ledger.period_end;

    // Advance 6 hours, still within the same Rolling24h window. The
    // reported end should not move — it was anchored at set_limit time.
    clock.advance(chrono::Duration::hours(6));
    let later = governor.account_snapshot(&account).unwrap().unwrap();
    assert_eq!(
        later.ledger.period_end, initial_end,
        "Rolling24h window must stay anchored to set_limit time, not slide with `now`",
    );
    assert_eq!(
        later.ledger.period_end - later.ledger.period_start,
        chrono::Duration::hours(24)
    );
}

#[test]
fn threshold_pause_fires_at_exactly_100_percent_when_pause_below_one() {
    // Regression: previously the threshold check required `utilization < 1.0`,
    // so the exact 100% case (e.g. requested = remaining) silently fell
    // through without raising approval. With pause_at < 1.0 we should
    // surface RequiresApproval rather than letting it slip past as Allow.
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant-100pct", "user-100pct", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                thresholds: BudgetThresholds {
                    warn_at: 0.75,
                    pause_at: 0.90,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    // Exactly 100% utilization: usage 0, requested 10.00 against a $10 cap.
    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(10.00)))
        .unwrap_err();
    match err {
        ResourceError::RequiresApproval { needed, .. } => {
            assert_eq!(needed.dimension, ResourceDimension::Usd);
            assert!(needed.utilization >= 1.0 - f64::EPSILON);
        }
        other => panic!("expected RequiresApproval at exactly 100% utilization, got {other:?}"),
    }
}

#[test]
fn pause_threshold_of_one_disables_approval_and_allows_under_hard_cap() {
    // pause_at == 1.0 means "approval disabled" — under the hard limit
    // we should reserve cleanly with no approval intervention. The old
    // `utilization < 1.0` shortcut accidentally produced the right
    // behavior here; the new `pause_at < 1.0` rule must preserve it.
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant-disabled", "user-disabled", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor
        .set_limit(
            account,
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                thresholds: BudgetThresholds {
                    warn_at: 1.0,
                    pause_at: 1.0,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    // 95% of cap; pause_at = 1.0 disables approval, hard limit not yet hit.
    let outcome = governor
        .reserve_with_outcome(scope, ResourceEstimate::default().set_usd(dec!(9.50)))
        .unwrap();
    assert!(outcome.warnings.is_empty());
}

#[test]
fn calendar_day_period_resets_at_local_midnight() {
    // 2026-05-21 23:00 UTC → 16:00 PDT. Spend $4 (~80% of $5 daily) and
    // verify that advancing to 09:00 UTC (~02:00 PDT next day) resets.
    let tz = chrono_tz::America::Los_Angeles;
    let day1_evening = chrono::DateTime::parse_from_rfc3339("2026-05-21T23:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let day2_morning_utc = chrono::DateTime::parse_from_rfc3339("2026-05-22T09:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let clock = FakeClock::new(day1_evening);
    let governor = InMemoryResourceGovernor::with_clock(Arc::new(clock.clone()));
    let scope = sample_scope("tenant-cal", "user-cal", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(5.00)),
                period: BudgetPeriod::Calendar {
                    tz,
                    unit: PeriodUnit::Day,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let r1 = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(4.00)),
        )
        .unwrap();
    governor
        .reconcile(r1.id, ResourceUsage::default().set_usd(dec!(4.00)))
        .unwrap();

    // 80% spent in the day-1 window. Same window: another $1.50 should hard-deny.
    let denied = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(1.50)),
        )
        .unwrap_err();
    assert!(matches!(denied, ResourceError::LimitExceeded { .. }));

    // Advance the clock past LA midnight into day 2. New period, full budget.
    clock.set(day2_morning_utc);
    governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(4.00)))
        .unwrap();
}

#[test]
fn rolling_24h_period_resets_after_anchor_passes() {
    let now = chrono::Utc::now();
    let clock = FakeClock::new(now);
    let governor = InMemoryResourceGovernor::with_clock(Arc::new(clock.clone()));
    let scope = sample_scope("tenant-roll", "user-roll", None);
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    governor
        .set_limit(
            account,
            ResourceLimits::default()
                .set_max_usd(dec!(5.00))
                .set_period(BudgetPeriod::Rolling24h),
        )
        .unwrap();

    let r = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(4.50)),
        )
        .unwrap();
    governor
        .reconcile(r.id, ResourceUsage::default().set_usd(dec!(4.50)))
        .unwrap();
    // After 24h+1m the window has rolled over.
    clock.advance(chrono::Duration::hours(24) + chrono::Duration::minutes(1));
    governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(4.50)))
        .unwrap();
}

#[test]
fn cascade_reports_first_failing_account_in_user_project_order() {
    // User has $1 limit, project has $0.50 (more restrictive); request $0.75
    // must deny at project, not user, because cascade walks broadest →
    // narrowest and project comes after user.
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope("tenant-cas", "user-cas", Some("project-cas"));
    let user_account = ResourceAccount::user(scope.tenant_id.clone(), scope.user_id.clone());
    let project_account = ResourceAccount::project(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.project_id.clone().unwrap(),
    );
    governor
        .set_limit(
            user_account,
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();
    governor
        .set_limit(
            project_account.clone(),
            ResourceLimits::default().set_max_usd(dec!(0.50)),
        )
        .unwrap();

    let err = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.75)))
        .unwrap_err();
    match err {
        ResourceError::LimitExceeded { denial, .. } => {
            assert_eq!(denial.account, project_account);
        }
        other => panic!("expected project-level denial, got {other:?}"),
    }
}

/// Regression for #3841 follow-up "report accumulated metrics before pausing":
/// when one dimension crosses warn while another dimension hard-denies, the
/// terminal denial must carry the warning so audit/SSE consumers see both.
#[test]
fn limit_exceeded_carries_warnings_from_other_dimensions() {
    let governor = InMemoryResourceGovernor::new();
    let scope = ResourceScope {
        tenant_id: TenantId::new("tenant-cascade-warn").unwrap(),
        user_id: UserId::new("cascade-warn-user").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let account = ResourceAccount::user(scope.tenant_id.clone(), scope.user_id.clone());
    // Two dimensions on the same account: USD is well under cap (no warn),
    // input_tokens sits at 80% (above warn=0.5), output_tokens overruns by
    // 200% (hard deny). The terminal denial on output_tokens must still
    // include the input_tokens warning.
    governor
        .set_limit(
            account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                max_input_tokens: Some(100),
                max_output_tokens: Some(10),
                thresholds: BudgetThresholds {
                    warn_at: 0.5,
                    pause_at: 0.95,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    // Burn 80 input tokens of prior usage so the next reservation crosses warn.
    let prior = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_input_tokens(80),
        )
        .unwrap();
    governor
        .reconcile(prior.id, ResourceUsage::default().set_input_tokens(80))
        .unwrap();
    // Now request 20 output_tokens (cap 10, hard deny) + 5 input_tokens
    // (running total 85, warn at 0.5 fires).
    let err = governor
        .reserve(
            scope,
            ResourceEstimate::default()
                .set_input_tokens(5)
                .set_output_tokens(20),
        )
        .unwrap_err();
    let warnings = match err {
        ResourceError::LimitExceeded {
            ref denial,
            ref warnings,
        } => {
            assert_eq!(
                denial.dimension,
                ResourceDimension::OutputTokens,
                "denial must fire on the over-cap dimension"
            );
            warnings.clone()
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    };
    assert!(
        warnings
            .iter()
            .any(|w| w.dimension == ResourceDimension::InputTokens),
        "input_tokens warning must accompany the output_tokens denial — got {warnings:?}"
    );
}

/// Regression for #3841 follow-up "A2: project BudgetEvent into the gateway
/// event stream". A reserve that crosses warn → pause must emit at least
/// one Warned event before the ApprovalRequested event, and a hard-cap
/// overrun must emit Denied while preserving prior warnings.
#[test]
fn governor_emits_budget_events_through_event_sink() {
    use std::sync::Arc;

    use ironclaw_resources::{
        BudgetEvent, BudgetPeriod, BudgetThresholds, InMemoryBudgetEventSink,
    };
    let sink = Arc::new(InMemoryBudgetEventSink::new());
    let governor = InMemoryResourceGovernor::default().with_event_sink(sink.clone());

    let scope = ResourceScope {
        tenant_id: TenantId::new("tenant-sink").unwrap(),
        user_id: UserId::new("sink-user").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let account = ResourceAccount::user(scope.tenant_id.clone(), scope.user_id.clone());

    governor
        .set_limit(
            account,
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                period: BudgetPeriod::Rolling24h,
                thresholds: BudgetThresholds {
                    warn_at: 0.5,
                    pause_at: 0.9,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    assert!(
        sink.snapshot()
            .iter()
            .any(|e| matches!(e, BudgetEvent::LimitChanged { .. })),
        "set_limit must emit LimitChanged"
    );

    // Reserve a small amount — no warn yet.
    let outcome = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(2.00)),
        )
        .unwrap();
    governor
        .reconcile(outcome.id, ResourceUsage::default().set_usd(dec!(2.00)))
        .unwrap();
    assert!(
        sink.snapshot()
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reserved { .. })),
        "reserve must emit Reserved"
    );
    assert!(
        sink.snapshot()
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reconciled { .. })),
        "reconcile must emit Reconciled"
    );

    // Drain so the next assertions only see new events.
    sink.drain();

    // Now reserve enough to cross warn (0.5) but still under pause.
    let _ = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(4.00)),
        )
        .unwrap();
    let warn_events = sink.drain();
    assert!(
        warn_events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Warned { .. })),
        "crossing warn threshold must emit Warned — got {warn_events:?}"
    );

    // Push into pause (0.9 of $10 with $6 already spent + new $3 = $9 → 90%).
    let approval = governor
        .reserve(
            scope.clone(),
            ResourceEstimate::default().set_usd(dec!(3.00)),
        )
        .unwrap_err();
    assert!(matches!(approval, ResourceError::RequiresApproval { .. }));
    let pause_events = sink.drain();
    assert!(
        pause_events
            .iter()
            .any(|e| matches!(e, BudgetEvent::ApprovalRequested { .. })),
        "pause threshold must emit ApprovalRequested — got {pause_events:?}"
    );

    // Push over the hard cap.
    let denial = governor
        .reserve(scope, ResourceEstimate::default().set_usd(dec!(100.00)))
        .unwrap_err();
    assert!(matches!(denial, ResourceError::LimitExceeded { .. }));
    let deny_events = sink.drain();
    assert!(
        deny_events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Denied { .. })),
        "hard cap must emit Denied — got {deny_events:?}"
    );
}

#[test]
fn schema_v1_snapshot_migrates_in_place_on_load() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("v1.json");
    fs::write(
        &path,
        r#"{
            "schema_version": 1,
            "state": {
                "limits": [],
                "reserved_by_account": [],
                "usage_by_account": [],
                "reservations": []
            }
        }"#,
    )
    .unwrap();

    let governor = PersistentResourceGovernor::new(JsonFileResourceGovernorStore::new(&path));
    governor
        .try_set_limit(
            ResourceAccount::tenant(TenantId::new("tenant1").unwrap()),
            ResourceLimits::default().set_max_usd(dec!(1.00)),
        )
        .unwrap();

    // After the first successful mutation, the file is rewritten as the
    // current snapshot schema.
    let snapshot: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(snapshot["schema_version"], serde_json::json!(3));
}

#[test]
fn thresholds_validation_rejects_pause_below_warn() {
    assert!(
        BudgetThresholds {
            warn_at: 0.9,
            pause_at: 0.5
        }
        .validate()
        .is_err()
    );
}
