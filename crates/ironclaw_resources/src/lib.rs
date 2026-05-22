//! Resource reservation governor for IronClaw Reborn.
//!
//! `ironclaw_resources` enforces the host-level reservation protocol used by
//! runtime lanes before they spend money or consume scarce sandbox capacity:
//! reserve estimated resources, execute work, then reconcile actual usage or
//! release the unused hold.
//!
//! Durable persistence is provided by [`FilesystemResourceGovernorStore`]
//! over a [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem). The
//! `RootFilesystem` choice (libSQL-backed, PostgreSQL-backed, in-memory, or
//! local-disk) is made at the filesystem layer — the consumer-store level no
//! longer carries per-backend impls. See
//! `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
//!
//! Persistent governors fail closed when snapshot reads, writes, locks, or
//! schema validation fail. Callers must handle [`ResourceError::Storage`] the
//! same way as quota denials: do not start costed or quota-limited work until a
//! reservation operation succeeds.
#![warn(unreachable_pub)]

mod event;
mod filesystem_gate_store;
mod filesystem_store;
mod gate;
mod period;

pub use event::{BudgetEvent, BudgetEventSink, InMemoryBudgetEventSink, NoOpBudgetEventSink};
pub use filesystem_gate_store::FilesystemBudgetGateStore;
pub use filesystem_store::FilesystemResourceGovernorStore;
pub use gate::{
    BudgetApprovalGate, BudgetGateError, BudgetGateId, BudgetGateOutcome, BudgetGateStatus,
    BudgetGateStore, InMemoryBudgetGateStore,
};
pub use period::{
    BudgetPeriod, BudgetThresholds, BudgetThresholdsError, PeriodUnit, period_bounds,
    period_has_rolled_over,
};

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Duration, Utc};
use fs2::FileExt;

use ironclaw_host_api::ReservationStatus;
use ironclaw_host_api::{
    AgentId, MissionId, ProjectId, ResourceEstimate, ResourceReservationId, ResourceScope,
    ResourceUsage, TenantId, ThreadId, UserId,
};
pub use ironclaw_host_api::{ResourceReceipt, ResourceReservation};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Source of `now` for governor period accounting.
///
/// The default [`SystemClock`] returns `Utc::now()`. Tests inject a
/// [`FakeClock`] (see test helpers) for deterministic period boundaries.
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> DateTime<Utc>;
}

/// Default wall-clock implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Test-only fixed-or-advanceable clock.
#[derive(Debug, Clone)]
pub struct FakeClock {
    inner: Arc<Mutex<DateTime<Utc>>>,
}

impl FakeClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(now)),
        }
    }

    pub fn advance(&self, by: chrono::Duration) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard += by;
    }

    pub fn set(&self, now: DateTime<Utc>) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = now;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Durable account level that can carry resource limits and ledgers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ResourceAccount {
    Tenant {
        tenant_id: TenantId,
    },
    User {
        tenant_id: TenantId,
        user_id: UserId,
    },
    Project {
        tenant_id: TenantId,
        user_id: UserId,
        project_id: ProjectId,
    },
    Agent {
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        agent_id: AgentId,
    },
    Mission {
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        mission_id: MissionId,
    },
    Thread {
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        mission_id: Option<MissionId>,
        thread_id: ThreadId,
    },
}

impl ResourceAccount {
    pub fn tenant(tenant_id: TenantId) -> Self {
        Self::Tenant { tenant_id }
    }

    pub fn user(tenant_id: TenantId, user_id: UserId) -> Self {
        Self::User { tenant_id, user_id }
    }

    pub fn project(tenant_id: TenantId, user_id: UserId, project_id: ProjectId) -> Self {
        Self::Project {
            tenant_id,
            user_id,
            project_id,
        }
    }

    pub fn agent(
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        agent_id: AgentId,
    ) -> Self {
        Self::Agent {
            tenant_id,
            user_id,
            project_id,
            agent_id,
        }
    }

    pub fn mission(
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        mission_id: MissionId,
    ) -> Self {
        Self::Mission {
            tenant_id,
            user_id,
            project_id,
            mission_id,
        }
    }

    pub fn thread(
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        mission_id: Option<MissionId>,
        thread_id: ThreadId,
    ) -> Self {
        Self::Thread {
            tenant_id,
            user_id,
            project_id,
            mission_id,
            thread_id,
        }
    }

    /// Returns every account whose limit applies to this scope, from broadest to
    /// narrowest.
    ///
    /// A reservation succeeds only if every account returned by this cascade
    /// remains within its limit. Deeper accounts do not override shallower
    /// accounts; tenant, user, project, agent, mission, and thread limits all
    /// apply when present.
    pub fn cascade(scope: &ResourceScope) -> Vec<Self> {
        let mut accounts = vec![
            Self::tenant(scope.tenant_id.clone()),
            Self::user(scope.tenant_id.clone(), scope.user_id.clone()),
        ];

        if let Some(project_id) = &scope.project_id {
            accounts.push(Self::project(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                project_id.clone(),
            ));
        }

        if let Some(agent_id) = &scope.agent_id {
            accounts.push(Self::agent(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                scope.project_id.clone(),
                agent_id.clone(),
            ));
        }

        if let Some(mission_id) = &scope.mission_id {
            accounts.push(Self::mission(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                scope.project_id.clone(),
                mission_id.clone(),
            ));
        }

        if let Some(thread_id) = &scope.thread_id {
            accounts.push(Self::thread(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                scope.project_id.clone(),
                scope.mission_id.clone(),
                thread_id.clone(),
            ));
        }

        accounts
    }
}

/// Optional maximums for each resource dimension.
///
/// **Zero semantics:** for any dimension, `Some(zero)` means **unlimited**
/// (explicit opt-out of enforcement). `None` is the same as the unset/
/// uninstalled limit and also means unlimited. To deny *any* spending in a
/// dimension, set the limit to a small non-zero value rather than zero.
/// This convention exists so configuration files can express "no budget cap
/// for this account" with a plain `0` rather than dropping the key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    pub max_usd: Option<Decimal>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_wall_clock_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub max_network_egress_bytes: Option<u64>,
    pub max_process_count: Option<u32>,
    pub max_concurrency_slots: Option<u32>,
    /// Period over which `max_*` limits accumulate. Defaults to
    /// [`BudgetPeriod::PerInvocation`] for backwards-compatible behavior
    /// with v1 limits that did not carry a period.
    #[serde(default)]
    pub period: BudgetPeriod,
    /// Graduated-intervention thresholds. Defaults to
    /// [`BudgetThresholds::DISABLED`] so that pre-existing limits never
    /// emit warnings or approval gates without explicit opt-in.
    #[serde(default)]
    pub thresholds: BudgetThresholds,
}

impl ResourceLimits {
    /// True when every dimension is unbounded (None or explicit zero).
    pub fn is_unlimited(&self) -> bool {
        is_decimal_unlimited(self.max_usd)
            && is_integer_unlimited(self.max_input_tokens)
            && is_integer_unlimited(self.max_output_tokens)
            && is_integer_unlimited(self.max_wall_clock_ms)
            && is_integer_unlimited(self.max_output_bytes)
            && is_integer_unlimited(self.max_network_egress_bytes)
            && is_integer_unlimited(self.max_process_count.map(u64::from))
            && is_integer_unlimited(self.max_concurrency_slots.map(u64::from))
    }
}

fn is_decimal_unlimited(value: Option<Decimal>) -> bool {
    match value {
        None => true,
        Some(v) => v <= Decimal::ZERO,
    }
}

fn is_integer_unlimited(value: Option<u64>) -> bool {
    matches!(value, None | Some(0))
}

/// Resource dimension that may deny a reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceDimension {
    Usd,
    InputTokens,
    OutputTokens,
    WallClockMs,
    OutputBytes,
    NetworkEgressBytes,
    ProcessCount,
    ConcurrencySlots,
}

impl std::fmt::Display for ResourceDimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Usd => "usd",
            Self::InputTokens => "input_tokens",
            Self::OutputTokens => "output_tokens",
            Self::WallClockMs => "wall_clock_ms",
            Self::OutputBytes => "output_bytes",
            Self::NetworkEgressBytes => "network_egress_bytes",
            Self::ProcessCount => "process_count",
            Self::ConcurrencySlots => "concurrency_slots",
        })
    }
}

/// Comparable amount for denial details.
///
/// Uses adjacent tagging because `Decimal` serializes as a JSON string via
/// `serde-with-str`, which is incompatible with `serde(tag = …)` internal
/// tagging on newtype variants (rust-lang/serde#1402). Adjacent tagging keeps
/// the `kind` discriminator while embedding the inner value under `value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ResourceValue {
    Decimal(Decimal),
    Integer(u64),
}

/// Structured reservation denial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDenial {
    pub account: ResourceAccount,
    pub dimension: ResourceDimension,
    pub limit: ResourceValue,
    pub current_usage: ResourceValue,
    pub active_reserved: ResourceValue,
    pub requested: ResourceValue,
}

/// Reservation pause: utilization would cross the configured pause-threshold
/// before reaching the hard limit. Callers route this through their
/// approval surface (foreground modal, background notification, CLI) before
/// retrying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceApprovalNeeded {
    pub account: ResourceAccount,
    pub dimension: ResourceDimension,
    pub limit: ResourceValue,
    pub current_usage: ResourceValue,
    pub active_reserved: ResourceValue,
    pub requested: ResourceValue,
    /// Fraction in `[0.0, 1.0]` (may exceed 1.0 in pathological cases) —
    /// `(usage + reserved + requested) / limit`.
    pub utilization: f64,
    /// When the current period naturally rolls over and the gate would
    /// resolve without user action. `None` for `PerInvocation`.
    pub period_end: Option<DateTime<Utc>>,
}

/// Threshold-crossing event surfaced from `reserve()`.
///
/// Warnings do *not* deny the reservation. They tell callers the account
/// has crossed [`BudgetThresholds::warn_at`] but is still below
/// [`BudgetThresholds::pause_at`]; UI surfaces typically render a chip
/// change but allow the work to proceed.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetWarning {
    pub account: ResourceAccount,
    pub dimension: ResourceDimension,
    pub utilization: f64,
    pub limit: ResourceValue,
    pub period_end: Option<DateTime<Utc>>,
}

/// Resource governor errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ResourceError {
    /// Hard cap exceeded for a dimension. `warnings` carries any threshold
    /// crossings that fired before this terminal denial; downstream audit
    /// sinks emit them as their own `BudgetEvent::Warned` records so the
    /// user sees the warn signal that preceded the stop.
    #[error("resource limit exceeded for {dimension} at {account:?}", account = denial.account, dimension = denial.dimension)]
    LimitExceeded {
        denial: Box<ResourceDenial>,
        warnings: Vec<BudgetWarning>,
    },
    /// Reservation would push utilization past the configured pause
    /// threshold. The work is not denied; callers must surface an approval
    /// gate, capture the user's decision, and retry the reservation (with
    /// an extended limit or after the period rolls over). `warnings`
    /// carries any other dimensions that crossed warn before the pause.
    #[error("resource budget approval required for {dimension} at {account:?}", account = needed.account, dimension = needed.dimension)]
    RequiresApproval {
        needed: Box<ResourceApprovalNeeded>,
        warnings: Vec<BudgetWarning>,
    },
    #[error("resource reservation {id} already exists")]
    ReservationAlreadyExists { id: ResourceReservationId },
    #[error("invalid resource estimate for {dimension}: {reason}")]
    InvalidEstimate {
        dimension: ResourceDimension,
        reason: &'static str,
    },
    #[error("resource reservation {id} does not match requested scope or estimate")]
    ReservationMismatch { id: ResourceReservationId },
    #[error("unknown resource reservation {id}")]
    UnknownReservation { id: ResourceReservationId },
    #[error("resource reservation {id} is already {status:?}")]
    ReservationClosed {
        id: ResourceReservationId,
        status: ReservationStatus,
    },
    /// Durable storage or snapshot schema validation failed. Governors must
    /// fail closed when this is returned.
    #[error("resource governor storage error")]
    Storage { reason: String },
}

/// Aggregated resource usage/reservation tally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceTally {
    pub usd: Decimal,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_clock_ms: u64,
    pub output_bytes: u64,
    pub network_egress_bytes: u64,
    pub process_count: u32,
    pub concurrency_slots: u32,
}

impl ResourceTally {
    fn from_estimate(estimate: &ResourceEstimate) -> Self {
        Self {
            usd: estimate.usd.unwrap_or_default(),
            input_tokens: estimate.input_tokens.unwrap_or_default(),
            output_tokens: estimate.output_tokens.unwrap_or_default(),
            wall_clock_ms: estimate.wall_clock_ms.unwrap_or_default(),
            output_bytes: estimate.output_bytes.unwrap_or_default(),
            network_egress_bytes: estimate.network_egress_bytes.unwrap_or_default(),
            process_count: estimate.process_count.unwrap_or_default(),
            concurrency_slots: estimate.concurrency_slots.unwrap_or_default(),
        }
    }

    fn from_usage(usage: &ResourceUsage) -> Self {
        Self {
            usd: usage.usd,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            wall_clock_ms: usage.wall_clock_ms,
            output_bytes: usage.output_bytes,
            network_egress_bytes: usage.network_egress_bytes,
            process_count: usage.process_count,
            concurrency_slots: 0,
        }
    }

    fn add_assign(&mut self, other: &Self) {
        self.usd = self.usd.checked_add(other.usd).unwrap_or(Decimal::MAX);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.wall_clock_ms = self.wall_clock_ms.saturating_add(other.wall_clock_ms);
        self.output_bytes = self.output_bytes.saturating_add(other.output_bytes);
        self.network_egress_bytes = self
            .network_egress_bytes
            .saturating_add(other.network_egress_bytes);
        self.process_count = self.process_count.saturating_add(other.process_count);
        self.concurrency_slots = self
            .concurrency_slots
            .saturating_add(other.concurrency_slots);
    }

    fn sub_assign(&mut self, other: &Self) {
        self.usd = self
            .usd
            .checked_sub(other.usd)
            .map(|value| value.max(Decimal::ZERO))
            .unwrap_or(Decimal::ZERO);
        self.input_tokens = self.input_tokens.saturating_sub(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_sub(other.output_tokens);
        self.wall_clock_ms = self.wall_clock_ms.saturating_sub(other.wall_clock_ms);
        self.output_bytes = self.output_bytes.saturating_sub(other.output_bytes);
        self.network_egress_bytes = self
            .network_egress_bytes
            .saturating_sub(other.network_egress_bytes);
        self.process_count = self.process_count.saturating_sub(other.process_count);
        self.concurrency_slots = self
            .concurrency_slots
            .saturating_sub(other.concurrency_slots);
    }
}

/// Successful reservation with any threshold-crossing warnings that fired
/// during cascade evaluation.
#[derive(Debug, Clone)]
pub struct ReservationOutcome {
    pub reservation: ResourceReservation,
    pub warnings: Vec<BudgetWarning>,
}

/// Snapshot of one account's current period + utilization for UI/audit.
#[derive(Debug, Clone)]
pub struct AccountSnapshot {
    pub account: ResourceAccount,
    pub limits: Option<ResourceLimits>,
    pub ledger: PeriodLedger,
}

/// Synchronous resource governor contract.
///
/// Persistent implementations may return [`ResourceError::Storage`] from any
/// method when durable reads, writes, locking, serialization, or schema
/// validation fail. Callers must treat storage failures as fail-closed and avoid
/// starting quota-limited work without a successful reservation.
pub trait ResourceGovernor: Send + Sync {
    /// Sets or replaces limits for a scoped resource account without mutating existing reservations.
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError>;

    /// Reserves estimated resources before costed/quota-limited work starts.
    ///
    /// A reservation succeeds only if every account in [`ResourceAccount::cascade`]
    /// would remain within its limits. Limits at deeper accounts do not override
    /// shallower limits; tenant, user, project, agent, mission, and thread limits
    /// all apply when present.
    ///
    /// Returns just the reservation handle; any threshold-crossing warnings
    /// are discarded. New callers should prefer
    /// [`ResourceGovernor::reserve_with_outcome`] to receive warnings.
    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError> {
        self.reserve_with_outcome(scope, estimate)
            .map(|outcome| outcome.reservation)
    }

    /// Reserves estimated resources with a caller-supplied reservation id for obligation handoff.
    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError> {
        self.reserve_with_id_and_outcome(scope, estimate, reservation_id)
            .map(|outcome| outcome.reservation)
    }

    /// Reserve, returning any threshold-crossing warnings alongside the
    /// reservation handle. Production callers that surface budget UI go
    /// through this method so the warning list reaches the event sink.
    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ReservationOutcome, ResourceError>;

    /// Like [`Self::reserve_with_outcome`] but with a caller-supplied
    /// reservation id for obligation handoff.
    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ReservationOutcome, ResourceError>;

    /// Reconciles an active reservation with actual usage and releases reserved capacity exactly once.
    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError>;

    /// Releases an active reservation without usage when work is cancelled or fails before reconciliation.
    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError>;

    /// Read current account state (limits + period ledger) for UI/audit.
    /// Returns `None` if no limit was ever set and no reservation has ever
    /// touched the account.
    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<AccountSnapshot>, ResourceError>;

    /// Convenience helper that returns the recorded spend tally for an
    /// account. Default impl reads through [`Self::account_snapshot`].
    /// Implementations that hold spend directly may override for cheaper
    /// access. Returns the zero tally when the account has never been
    /// touched.
    fn usage_for(&self, account: &ResourceAccount) -> Result<ResourceTally, ResourceError> {
        Ok(self
            .account_snapshot(account)?
            .map(|snapshot| snapshot.ledger.spent)
            .unwrap_or_default())
    }
}

/// Snapshot schema version.
///
/// **v1** (deprecated, read-only-compat) — `usage_by_account` and
/// `reserved_by_account` HashMaps with no period concept.
/// **v2** (current) — adds `ResourceLimits::period` and
/// `ResourceLimits::thresholds`, plus per-account `period_anchors` carrying
/// the current period's end instant for rollover.
///
/// Migration: v1 snapshots are accepted on read. The first write rewrites
/// them in v2 shape. v1 entries are treated as `PerInvocation` with
/// `BudgetThresholds::DISABLED` — no behavior change unless callers
/// explicitly install new-shape limits.
const RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_VERSION: u32 = 2;
const RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_V1_ACCEPTED: u32 = 1;

/// Serializable governor snapshot stored by durable stores.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResourceGovernorSnapshot {
    schema_version: u32,
    state: ResourceState,
}

impl Default for ResourceGovernorSnapshot {
    fn default() -> Self {
        Self {
            schema_version: RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_VERSION,
            state: ResourceState::default(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceGovernorSnapshotSerde {
    #[serde(default = "current_resource_governor_snapshot_schema_version")]
    schema_version: u32,
    state: ResourceState,
}

impl<'de> Deserialize<'de> for ResourceGovernorSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let snapshot = ResourceGovernorSnapshotSerde::deserialize(deserializer)?;
        match snapshot.schema_version {
            RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_VERSION => Ok(Self {
                schema_version: snapshot.schema_version,
                state: snapshot.state,
            }),
            RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_V1_ACCEPTED => {
                // v1 → v2 in-place: existing `usage_by_account` /
                // `reserved_by_account` entries keep their values; period
                // anchors are absent so accounts fall back to
                // `PerInvocation` semantics until callers explicitly
                // install a new-shape limit. Rewritten as v2 on next save.
                Ok(Self {
                    schema_version: RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_VERSION,
                    state: snapshot.state,
                })
            }
            other => Err(serde::de::Error::custom(format!(
                "unsupported resource governor snapshot schema version {other}; expected {} (current) or {} (v1, migrated on first write)",
                RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_VERSION,
                RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_V1_ACCEPTED
            ))),
        }
    }
}

fn current_resource_governor_snapshot_schema_version() -> u32 {
    RESOURCE_GOVERNOR_SNAPSHOT_SCHEMA_VERSION
}

/// Transactional storage primitive for [`PersistentResourceGovernor`].
///
/// Implementations must serialize the whole closure with any other readers or
/// writers over the same account-wide ledger before writing the updated
/// snapshot back durably.
pub trait ResourceGovernorStore: Send + Sync + 'static {
    fn update<T, F>(&self, update: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static;
}

/// File-backed resource-governor store using a stable sidecar lock file around
/// each load/update/atomic-rename transaction.
#[derive(Debug, Clone)]
pub struct JsonFileResourceGovernorStore {
    path: PathBuf,
}

impl JsonFileResourceGovernorStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl ResourceGovernorStore for JsonFileResourceGovernorStore {
    fn update<T, F>(&self, update: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(storage_error)?;
        }

        let lock_path = lock_path_for(&self.path);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(storage_error)?;
        lock_file.lock_exclusive().map_err(storage_error)?;

        let result = update_file_snapshot(&self.path, update);
        let unlock_result = lock_file.unlock().map_err(storage_error);
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }
}

fn lock_path_for(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    PathBuf::from(lock_path)
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut temp_path = path.as_os_str().to_owned();
    temp_path.push(format!(".{}.tmp", ResourceReservationId::new()));
    PathBuf::from(temp_path)
}

fn update_file_snapshot<T, F>(path: &Path, update: F) -> Result<T, ResourceError>
where
    F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError>,
{
    let mut snapshot = read_file_snapshot(path)?;
    let value = update(&mut snapshot)?;
    write_file_snapshot_atomically(path, &snapshot)?;
    Ok(value)
}

fn read_file_snapshot(path: &Path) -> Result<ResourceGovernorSnapshot, ResourceError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(ResourceGovernorSnapshot::default());
        }
        Err(error) => return Err(storage_error(error)),
    };
    let mut contents = String::new();
    file.read_to_string(&mut contents).map_err(storage_error)?;
    if contents.trim().is_empty() {
        Ok(ResourceGovernorSnapshot::default())
    } else {
        serde_json::from_str(&contents).map_err(snapshot_decode_error)
    }
}

fn write_file_snapshot_atomically(
    path: &Path,
    snapshot: &ResourceGovernorSnapshot,
) -> Result<(), ResourceError> {
    let temp_path = temp_path_for(path);
    let encoded = serde_json::to_vec_pretty(snapshot).map_err(storage_error)?;
    let write_result = write_temp_snapshot(&temp_path, &encoded)
        .and_then(|()| replace_file_atomically(&temp_path, path))
        .and_then(|()| sync_parent_dir(path));
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    write_result
}

fn write_temp_snapshot(temp_path: &Path, encoded: &[u8]) -> Result<(), ResourceError> {
    let mut temp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .map_err(storage_error)?;
    temp_file.write_all(encoded).map_err(storage_error)?;
    temp_file.write_all(b"\n").map_err(storage_error)?;
    temp_file.sync_all().map_err(storage_error)
}

#[cfg(not(windows))]
fn replace_file_atomically(temp_path: &Path, path: &Path) -> Result<(), ResourceError> {
    std::fs::rename(temp_path, path).map_err(storage_error)
}

#[cfg(windows)]
fn replace_file_atomically(temp_path: &Path, path: &Path) -> Result<(), ResourceError> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temp_path = path_to_nul_terminated_wide(temp_path)?;
    let target_path = path_to_nul_terminated_wide(path)?;
    // SAFETY: Both arguments are valid NUL-terminated UTF-16 buffers that live
    // for the duration of the call. The temp file lives beside the target, so
    // MoveFileExW performs an atomic same-volume replacement when the target
    // already exists instead of failing like std::fs::rename on Windows.
    let moved = unsafe {
        MoveFileExW(
            temp_path.as_ptr(),
            target_path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(storage_error(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn path_to_nul_terminated_wide(path: &Path) -> Result<Vec<u16>, ResourceError> {
    use std::os::windows::ffi::OsStrExt;

    let absolute = std::path::absolute(path).map_err(storage_error)?;
    let mut wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(ResourceError::Storage {
            reason: "path contains an interior NUL byte".to_string(),
        });
    }

    normalize_windows_path_separators(&mut wide);
    if !has_windows_namespace_prefix(&wide) {
        wide = verbatim_windows_path(wide);
    }
    wide.push(0);
    Ok(wide)
}

#[cfg(windows)]
fn has_windows_namespace_prefix(wide: &[u16]) -> bool {
    wide.len() >= 4
        && is_windows_path_separator(wide[0])
        && is_windows_path_separator(wide[1])
        && (wide[2] == b'?' as u16 || wide[2] == b'.' as u16)
        && is_windows_path_separator(wide[3])
}

#[cfg(windows)]
fn normalize_windows_path_separators(wide: &mut [u16]) {
    for code_unit in wide {
        if *code_unit == b'/' as u16 {
            *code_unit = b'\\' as u16;
        }
    }
}

#[cfg(windows)]
fn verbatim_windows_path(wide: Vec<u16>) -> Vec<u16> {
    if wide.len() >= 2 && is_windows_path_separator(wide[0]) && is_windows_path_separator(wide[1]) {
        let mut prefixed = wide_literal(r"\\?\UNC\");
        prefixed.extend_from_slice(&wide[2..]);
        prefixed
    } else if wide.len() >= 3 && wide[1] == b':' as u16 && is_windows_path_separator(wide[2]) {
        let mut prefixed = wide_literal(r"\\?\");
        prefixed.extend_from_slice(&wide);
        prefixed
    } else {
        wide
    }
}

#[cfg(windows)]
fn wide_literal(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

#[cfg(windows)]
fn is_windows_path_separator(code_unit: u16) -> bool {
    code_unit == b'\\' as u16 || code_unit == b'/' as u16
}

fn sync_parent_dir(path: &Path) -> Result<(), ResourceError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    normalize_parent_dir_sync_result(File::open(parent).and_then(|dir| dir.sync_all()))
}

fn normalize_parent_dir_sync_result(result: std::io::Result<()>) -> Result<(), ResourceError> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if is_unsupported_parent_dir_sync_error(&error) => Ok(()),
        Err(error) => Err(storage_error(error)),
    }
}

fn is_unsupported_parent_dir_sync_error(error: &std::io::Error) -> bool {
    if matches!(error.kind(), ErrorKind::Unsupported) {
        return true;
    }

    #[cfg(windows)]
    {
        const ERROR_INVALID_FUNCTION: i32 = 1;
        const ERROR_ACCESS_DENIED: i32 = 5;
        if matches!(error.kind(), ErrorKind::PermissionDenied)
            || matches!(
                error.raw_os_error(),
                Some(ERROR_INVALID_FUNCTION) | Some(ERROR_ACCESS_DENIED)
            )
        {
            return true;
        }
    }

    false
}

pub(crate) fn storage_error(error: impl std::fmt::Display) -> ResourceError {
    ResourceError::Storage {
        reason: error.to_string(),
    }
}

pub(crate) fn snapshot_decode_error(error: impl std::fmt::Display) -> ResourceError {
    ResourceError::Storage {
        reason: format!("malformed resource governor snapshot: {error}"),
    }
}

/// Durable resource governor backed by a transactional [`ResourceGovernorStore`].
#[derive(Debug)]
pub struct PersistentResourceGovernor<S>
where
    S: ResourceGovernorStore,
{
    store: S,
    clock: Arc<dyn Clock>,
}

impl<S> PersistentResourceGovernor<S>
where
    S: ResourceGovernorStore,
{
    pub fn new(store: S) -> Self {
        Self {
            store,
            clock: Arc::new(SystemClock),
        }
    }

    /// Construct with a custom clock. The clock is only consulted on
    /// mutating operations; if you replace it after construction, in-flight
    /// reservations keep their original anchors.
    pub fn with_clock(store: S, clock: Arc<dyn Clock>) -> Self {
        Self { store, clock }
    }

    pub fn try_set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        let now = self.clock.now();
        self.store.update(move |snapshot| {
            set_limit_in_state(&mut snapshot.state, account, limits, now);
            Ok(())
        })
    }

    pub fn reserved_for(&self, account: &ResourceAccount) -> Result<ResourceTally, ResourceError> {
        let account = account.clone();
        let now = self.clock.now();
        self.store.update(move |snapshot| {
            advance_period_if_rolled_over(&mut snapshot.state, &account, now);
            Ok(snapshot
                .state
                .reserved_by_account
                .get(&account)
                .cloned()
                .unwrap_or_default())
        })
    }

    pub fn usage_for(&self, account: &ResourceAccount) -> Result<ResourceTally, ResourceError> {
        let account = account.clone();
        let now = self.clock.now();
        self.store.update(move |snapshot| {
            advance_period_if_rolled_over(&mut snapshot.state, &account, now);
            Ok(snapshot
                .state
                .usage_by_account
                .get(&account)
                .cloned()
                .unwrap_or_default())
        })
    }
}

impl<S> ResourceGovernor for PersistentResourceGovernor<S>
where
    S: ResourceGovernorStore,
{
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        self.try_set_limit(account, limits)
    }

    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ReservationOutcome, ResourceError> {
        self.reserve_with_id_and_outcome(scope, estimate, ResourceReservationId::new())
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ReservationOutcome, ResourceError> {
        let now = self.clock.now();
        self.store.update(move |snapshot| {
            reserve_with_outcome_in_state(&mut snapshot.state, scope, estimate, reservation_id, now)
        })
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        let now = self.clock.now();
        self.store.update(move |snapshot| {
            reconcile_in_state(&mut snapshot.state, reservation_id, actual, now)
        })
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        let now = self.clock.now();
        self.store
            .update(move |snapshot| release_in_state(&mut snapshot.state, reservation_id, now))
    }

    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<AccountSnapshot>, ResourceError> {
        let account = account.clone();
        let now = self.clock.now();
        self.store.update(move |snapshot| {
            Ok(account_snapshot_in_state(
                &mut snapshot.state,
                &account,
                now,
            ))
        })
    }
}

/// In-memory governor used by early Reborn contract tests.
#[derive(Debug)]
pub struct InMemoryResourceGovernor {
    state: Mutex<ResourceState>,
    clock: Arc<dyn Clock>,
    /// Optional sink that receives `BudgetEvent`s as reservations,
    /// reconciliations, warnings, and denials happen. Wired by composition;
    /// defaults to [`NoOpBudgetEventSink`] so the governor stays usable
    /// without observability.
    event_sink: Arc<dyn BudgetEventSink>,
}

impl Default for InMemoryResourceGovernor {
    fn default() -> Self {
        Self {
            state: Mutex::new(ResourceState::default()),
            clock: Arc::new(SystemClock),
            event_sink: Arc::new(NoOpBudgetEventSink),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ResourceState {
    limits: HashMap<ResourceAccount, ResourceLimits>,
    reserved_by_account: HashMap<ResourceAccount, ResourceTally>,
    usage_by_account: HashMap<ResourceAccount, ResourceTally>,
    reservations: HashMap<ResourceReservationId, ReservationRecord>,
    /// Per-account period anchors. `period_end_at_anchor[acc]` is the UTC
    /// instant at which `usage_by_account[acc]` was last advanced; any
    /// `now >= period_end_at_anchor[acc]` triggers a fresh window before the
    /// next limit check. Missing entries inherit `PerInvocation` semantics
    /// (no carry-over). Storage is best-effort and recomputed from
    /// `ResourceLimits::period` on each mutation; v1 snapshots that lack
    /// this field migrate transparently.
    period_anchors: HashMap<ResourceAccount, DateTime<Utc>>,
}

/// Snapshot of accumulated period-scoped spend + reserved.
///
/// Returned by [`ResourceGovernor`] query helpers so callers can render
/// utilization in UI without re-implementing the period-rollover rules.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PeriodLedger {
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub spent: ResourceTally,
    pub reserved: ResourceTally,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ReservationRecord {
    reservation: ResourceReservation,
    accounts: Vec<ResourceAccount>,
    tally: ResourceTally,
    status: ReservationStatus,
    actual: Option<ResourceUsage>,
}

#[derive(Deserialize)]
#[serde(remote = "ResourceScope", deny_unknown_fields)]
struct StrictResourceScope {
    tenant_id: TenantId,
    user_id: UserId,
    #[serde(default)]
    agent_id: Option<AgentId>,
    #[serde(default)]
    project_id: Option<ProjectId>,
    #[serde(default)]
    mission_id: Option<MissionId>,
    #[serde(default)]
    thread_id: Option<ThreadId>,
    invocation_id: ironclaw_host_api::InvocationId,
}

#[derive(Deserialize)]
#[serde(remote = "ResourceEstimate", deny_unknown_fields)]
struct StrictResourceEstimate {
    #[serde(default)]
    usd: Option<Decimal>,
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    wall_clock_ms: Option<u64>,
    #[serde(default)]
    output_bytes: Option<u64>,
    #[serde(default)]
    network_egress_bytes: Option<u64>,
    #[serde(default)]
    process_count: Option<u32>,
    #[serde(default)]
    concurrency_slots: Option<u32>,
}

#[derive(Deserialize)]
#[serde(remote = "ResourceUsage", deny_unknown_fields)]
struct StrictResourceUsage {
    usd: Decimal,
    input_tokens: u64,
    output_tokens: u64,
    wall_clock_ms: u64,
    output_bytes: u64,
    network_egress_bytes: u64,
    process_count: u32,
}

#[derive(Deserialize)]
#[serde(remote = "ResourceReservation", deny_unknown_fields)]
struct StrictResourceReservation {
    id: ResourceReservationId,
    #[serde(with = "StrictResourceScope")]
    scope: ResourceScope,
    #[serde(with = "StrictResourceEstimate")]
    estimate: ResourceEstimate,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReservationRecordSerde {
    #[serde(with = "StrictResourceReservation")]
    reservation: ResourceReservation,
    accounts: Vec<ResourceAccount>,
    tally: ResourceTally,
    status: ReservationStatus,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_strict_resource_usage"
    )]
    actual: Option<ResourceUsage>,
}

impl<'de> Deserialize<'de> for ReservationRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = ReservationRecordSerde::deserialize(deserializer)?;
        Ok(Self {
            reservation: value.reservation,
            accounts: value.accounts,
            tally: value.tally,
            status: value.status,
            actual: value.actual,
        })
    }
}

fn deserialize_optional_strict_resource_usage<'de, D>(
    deserializer: D,
) -> Result<Option<ResourceUsage>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct StrictResourceUsageOption(#[serde(with = "StrictResourceUsage")] ResourceUsage);

    Option::<StrictResourceUsageOption>::deserialize(deserializer)
        .map(|value| value.map(|wrapper| wrapper.0))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceStateSerde {
    limits: Vec<(ResourceAccount, ResourceLimits)>,
    reserved_by_account: Vec<(ResourceAccount, ResourceTally)>,
    usage_by_account: Vec<(ResourceAccount, ResourceTally)>,
    reservations: Vec<(ResourceReservationId, ReservationRecord)>,
    /// Per-account period anchors, populated on v2 snapshots and absent on
    /// v1 snapshots. v1 → v2 migration treats a missing entry as the
    /// default `BudgetPeriod::PerInvocation` semantics (no carry-over).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    period_anchors: Option<Vec<(ResourceAccount, DateTime<Utc>)>>,
}

impl Serialize for ResourceState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ResourceStateSerde {
            limits: self
                .limits
                .iter()
                .map(|(account, limits)| (account.clone(), limits.clone()))
                .collect(),
            reserved_by_account: self
                .reserved_by_account
                .iter()
                .map(|(account, tally)| (account.clone(), tally.clone()))
                .collect(),
            usage_by_account: self
                .usage_by_account
                .iter()
                .map(|(account, tally)| (account.clone(), tally.clone()))
                .collect(),
            reservations: self
                .reservations
                .iter()
                .map(|(id, record)| (*id, record.clone()))
                .collect(),
            period_anchors: if self.period_anchors.is_empty() {
                None
            } else {
                Some(
                    self.period_anchors
                        .iter()
                        .map(|(account, anchor)| (account.clone(), *anchor))
                        .collect(),
                )
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ResourceState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = ResourceStateSerde::deserialize(deserializer)?;
        Ok(Self {
            limits: value.limits.into_iter().collect(),
            reserved_by_account: value.reserved_by_account.into_iter().collect(),
            usage_by_account: value.usage_by_account.into_iter().collect(),
            reservations: value.reservations.into_iter().collect(),
            period_anchors: value
                .period_anchors
                .map(|entries| entries.into_iter().collect())
                .unwrap_or_default(),
        })
    }
}

impl InMemoryResourceGovernor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            state: Mutex::new(ResourceState::default()),
            clock,
            event_sink: Arc::new(NoOpBudgetEventSink),
        }
    }

    /// Plug in an audit/SSE sink. Every `reserve`, `reconcile`, `release`,
    /// warning, and approval/denial emits a [`BudgetEvent`] to this sink.
    /// Calls are best-effort and synchronous; sinks must be cheap (forward
    /// to a `broadcast` channel for SSE).
    pub fn with_event_sink(mut self, sink: Arc<dyn BudgetEventSink>) -> Self {
        self.event_sink = sink;
        self
    }

    pub fn reserved_for(&self, account: &ResourceAccount) -> ResourceTally {
        let now = self.clock.now();
        let mut state = self.lock_state();
        advance_period_if_rolled_over(&mut state, account, now);
        state
            .reserved_by_account
            .get(account)
            .cloned()
            .unwrap_or_default()
    }

    pub fn usage_for(&self, account: &ResourceAccount) -> ResourceTally {
        let now = self.clock.now();
        let mut state = self.lock_state();
        advance_period_if_rolled_over(&mut state, account, now);
        state
            .usage_by_account
            .get(account)
            .cloned()
            .unwrap_or_default()
    }

    fn lock_state(&self) -> MutexGuard<'_, ResourceState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ResourceGovernor for InMemoryResourceGovernor {
    fn set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        let now = self.clock.now();
        set_limit_in_state(&mut self.lock_state(), account.clone(), limits, now);
        self.event_sink
            .emit(BudgetEvent::LimitChanged { account, at: now });
        Ok(())
    }

    fn reserve_with_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ReservationOutcome, ResourceError> {
        self.reserve_with_id_and_outcome(scope, estimate, ResourceReservationId::new())
    }

    fn reserve_with_id_and_outcome(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ReservationOutcome, ResourceError> {
        let now = self.clock.now();
        let result = reserve_with_outcome_in_state(
            &mut self.lock_state(),
            scope,
            estimate,
            reservation_id,
            now,
        );
        emit_reserve_events(self.event_sink.as_ref(), &result, now);
        result
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        let now = self.clock.now();
        let result = reconcile_in_state(&mut self.lock_state(), reservation_id, actual, now);
        if let Ok(receipt) = &result {
            self.event_sink.emit(BudgetEvent::Reconciled {
                account: most_specific_account(&receipt.scope),
                receipt: receipt.clone(),
                at: now,
            });
        }
        result
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        let now = self.clock.now();
        let result = release_in_state(&mut self.lock_state(), reservation_id, now);
        if let Ok(receipt) = &result {
            self.event_sink.emit(BudgetEvent::Released {
                account: most_specific_account(&receipt.scope),
                receipt: receipt.clone(),
                at: now,
            });
        }
        result
    }

    fn account_snapshot(
        &self,
        account: &ResourceAccount,
    ) -> Result<Option<AccountSnapshot>, ResourceError> {
        let now = self.clock.now();
        Ok(account_snapshot_in_state(
            &mut self.lock_state(),
            account,
            now,
        ))
    }
}

/// Translate a `Result<ReservationOutcome, ResourceError>` to a stream of
/// `BudgetEvent`s. Emits `Warned` for every warning regardless of the
/// terminal outcome, then either `Reserved` (success), `ApprovalRequested`
/// (pause), or `Denied` (hard cap).
fn emit_reserve_events(
    sink: &dyn BudgetEventSink,
    result: &Result<ReservationOutcome, ResourceError>,
    at: DateTime<Utc>,
) {
    let warnings: &[BudgetWarning] = match result {
        Ok(outcome) => &outcome.warnings,
        Err(ResourceError::RequiresApproval { warnings, .. }) => warnings,
        Err(ResourceError::LimitExceeded { warnings, .. }) => warnings,
        Err(_) => &[],
    };
    for warning in warnings {
        sink.emit(BudgetEvent::Warned {
            warning: warning.clone(),
            at,
        });
    }
    match result {
        Ok(outcome) => {
            sink.emit(BudgetEvent::Reserved {
                account: most_specific_account(&outcome.reservation.scope),
                reservation: outcome.reservation.clone(),
                warnings: outcome.warnings.clone(),
                at,
            });
        }
        Err(ResourceError::RequiresApproval { needed, .. }) => {
            sink.emit(BudgetEvent::ApprovalRequested {
                needed: (**needed).clone(),
                at,
            });
        }
        Err(ResourceError::LimitExceeded { denial, .. }) => {
            sink.emit(BudgetEvent::Denied {
                denial: (**denial).clone(),
                at,
            });
        }
        Err(_) => {}
    }
}

/// The deepest account in the cascade — the one whose limits are the
/// "owning" cap for this reservation. Used for `Reserved`/`Reconciled`/
/// `Released` events so subscribers can route per-thread/per-project.
fn most_specific_account(scope: &ResourceScope) -> ResourceAccount {
    ResourceAccount::cascade(scope)
        .into_iter()
        .next_back()
        .unwrap_or_else(|| ResourceAccount::tenant(scope.tenant_id.clone()))
}

fn set_limit_in_state(
    state: &mut ResourceState,
    account: ResourceAccount,
    limits: ResourceLimits,
    now: DateTime<Utc>,
) {
    // Advance any existing ledger to the freshly-configured period boundary.
    // A period change resets accumulated `usage_by_account` for the account
    // because the new period semantics may differ from the prior shape.
    let prior_period = state.limits.get(&account).map(|l| l.period.clone());
    let new_period = limits.period.clone();
    if prior_period.as_ref() != Some(&new_period) {
        state.usage_by_account.remove(&account);
        state.period_anchors.remove(&account);
    }
    state.limits.insert(account.clone(), limits);
    // Set initial period anchor so subsequent reserves see correct bounds.
    let (_, period_end) = period_bounds(&new_period, now);
    state.period_anchors.insert(account, period_end);
}

fn advance_period_if_rolled_over(
    state: &mut ResourceState,
    account: &ResourceAccount,
    now: DateTime<Utc>,
) {
    let Some(limits) = state.limits.get(account) else {
        return;
    };
    let period = limits.period.clone();
    if matches!(period, BudgetPeriod::PerInvocation) {
        // PerInvocation has no accumulating ledger; nothing to roll over.
        return;
    }
    let needs_advance = state
        .period_anchors
        .get(account)
        .map(|anchor| period_has_rolled_over(*anchor, now))
        .unwrap_or(true);
    if needs_advance {
        state.usage_by_account.remove(account);
        let (_, new_end) = period_bounds(&period, now);
        state.period_anchors.insert(account.clone(), new_end);
    }
}

fn reserve_with_outcome_in_state(
    state: &mut ResourceState,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    reservation_id: ResourceReservationId,
    now: DateTime<Utc>,
) -> Result<ReservationOutcome, ResourceError> {
    validate_estimate(&estimate)?;

    if state.reservations.contains_key(&reservation_id) {
        return Err(ResourceError::ReservationAlreadyExists { id: reservation_id });
    }
    let accounts = ResourceAccount::cascade(&scope);
    let requested = ResourceTally::from_estimate(&estimate);

    // Roll over any periods whose anchors have passed before checking limits.
    for account in &accounts {
        advance_period_if_rolled_over(state, account, now);
    }

    let mut warnings = Vec::new();
    for account in &accounts {
        if let Some(limits) = state.limits.get(account) {
            let usage = state
                .usage_by_account
                .get(account)
                .cloned()
                .unwrap_or_default();
            let reserved = state
                .reserved_by_account
                .get(account)
                .cloned()
                .unwrap_or_default();
            let period_end = state.period_anchors.get(account).copied();
            match evaluate_cascade_for_account(
                account, limits, &usage, &reserved, &requested, period_end,
            )? {
                CascadeOutcome::Allow(mut acc_warnings) => warnings.append(&mut acc_warnings),
                CascadeOutcome::RequiresApproval {
                    warnings: mut acc_warnings,
                    needed,
                } => {
                    warnings.append(&mut acc_warnings);
                    return Err(ResourceError::RequiresApproval {
                        needed: Box::new(needed),
                        warnings,
                    });
                }
                CascadeOutcome::Deny {
                    warnings: mut acc_warnings,
                    denial,
                } => {
                    warnings.append(&mut acc_warnings);
                    return Err(ResourceError::LimitExceeded {
                        denial: Box::new(denial),
                        warnings,
                    });
                }
            }
        }
    }

    let reservation = ResourceReservation {
        id: reservation_id,
        scope,
        estimate,
    };

    for account in &accounts {
        state
            .reserved_by_account
            .entry(account.clone())
            .or_default()
            .add_assign(&requested);
    }

    state.reservations.insert(
        reservation.id,
        ReservationRecord {
            reservation: reservation.clone(),
            accounts,
            tally: requested,
            status: ReservationStatus::Active,
            actual: None,
        },
    );

    Ok(ReservationOutcome {
        reservation,
        warnings,
    })
}

fn reconcile_in_state(
    state: &mut ResourceState,
    reservation_id: ResourceReservationId,
    actual: ResourceUsage,
    now: DateTime<Utc>,
) -> Result<ResourceReceipt, ResourceError> {
    let mut record = state
        .reservations
        .remove(&reservation_id)
        .ok_or(ResourceError::UnknownReservation { id: reservation_id })?;

    if record.status != ReservationStatus::Active {
        let status = record.status;
        state.reservations.insert(reservation_id, record);
        return Err(ResourceError::ReservationClosed {
            id: reservation_id,
            status,
        });
    }

    if let Err(error) = validate_usage(&actual) {
        state.reservations.insert(reservation_id, record);
        return Err(error);
    }

    for account in &record.accounts {
        advance_period_if_rolled_over(state, account, now);
        state
            .reserved_by_account
            .entry(account.clone())
            .or_default()
            .sub_assign(&record.tally);
        state
            .usage_by_account
            .entry(account.clone())
            .or_default()
            .add_assign(&ResourceTally::from_usage(&actual));
    }

    record.status = ReservationStatus::Reconciled;
    record.actual = Some(actual.clone());
    let receipt = ResourceReceipt {
        id: reservation_id,
        scope: record.reservation.scope.clone(),
        status: ReservationStatus::Reconciled,
        estimate: record.reservation.estimate.clone(),
        actual: Some(actual),
    };
    state.reservations.insert(reservation_id, record);
    Ok(receipt)
}

fn release_in_state(
    state: &mut ResourceState,
    reservation_id: ResourceReservationId,
    now: DateTime<Utc>,
) -> Result<ResourceReceipt, ResourceError> {
    let mut record = state
        .reservations
        .remove(&reservation_id)
        .ok_or(ResourceError::UnknownReservation { id: reservation_id })?;

    if record.status != ReservationStatus::Active {
        let status = record.status;
        state.reservations.insert(reservation_id, record);
        return Err(ResourceError::ReservationClosed {
            id: reservation_id,
            status,
        });
    }

    for account in &record.accounts {
        advance_period_if_rolled_over(state, account, now);
        state
            .reserved_by_account
            .entry(account.clone())
            .or_default()
            .sub_assign(&record.tally);
    }

    record.status = ReservationStatus::Released;
    let receipt = ResourceReceipt {
        id: reservation_id,
        scope: record.reservation.scope.clone(),
        status: ReservationStatus::Released,
        estimate: record.reservation.estimate.clone(),
        actual: None,
    };
    state.reservations.insert(reservation_id, record);
    Ok(receipt)
}

fn account_snapshot_in_state(
    state: &mut ResourceState,
    account: &ResourceAccount,
    now: DateTime<Utc>,
) -> Option<AccountSnapshot> {
    advance_period_if_rolled_over(state, account, now);
    let limits = state.limits.get(account).cloned();
    let reserved = state
        .reserved_by_account
        .get(account)
        .cloned()
        .unwrap_or_default();
    let spent = state
        .usage_by_account
        .get(account)
        .cloned()
        .unwrap_or_default();
    if limits.is_none() && reserved == ResourceTally::default() && spent == ResourceTally::default()
    {
        return None;
    }
    let period = limits
        .as_ref()
        .map(|l| l.period.clone())
        .unwrap_or_default();
    // Rolling24h is anchored to when the limit was set (or last rolled
    // over) — not to `now`. Derive the start from the stored anchor so
    // the snapshot reports the window the ledger is actually accumulating
    // against. For Calendar/PerInvocation, `period_bounds(now)` already
    // returns the correct wall-clock-anchored window.
    let (period_start, period_end) = match (state.period_anchors.get(account), &period) {
        (Some(end), BudgetPeriod::Rolling24h) => (*end - Duration::hours(24), *end),
        _ => period_bounds(&period, now),
    };
    Some(AccountSnapshot {
        account: account.clone(),
        limits,
        ledger: PeriodLedger {
            period_start,
            period_end,
            spent,
            reserved,
        },
    })
}

fn validate_estimate(estimate: &ResourceEstimate) -> Result<(), ResourceError> {
    if let Some(usd) = estimate.usd
        && usd < Decimal::ZERO
    {
        return Err(ResourceError::InvalidEstimate {
            dimension: ResourceDimension::Usd,
            reason: "must be non-negative",
        });
    }

    Ok(())
}

fn validate_usage(usage: &ResourceUsage) -> Result<(), ResourceError> {
    if usage.usd < Decimal::ZERO {
        return Err(ResourceError::InvalidEstimate {
            dimension: ResourceDimension::Usd,
            reason: "must be non-negative",
        });
    }

    Ok(())
}

/// Result of evaluating one account in the cascade.
///
/// Each variant carries the *accumulated* warnings produced across every
/// dimension and threshold check, not just the first one. Earlier
/// implementations short-circuited on the first non-`Allow` intervention,
/// which silently dropped warnings the UI / audit sink should still see —
/// see the #3841 follow-up "report accumulated metrics before pausing".
enum CascadeOutcome {
    /// Reservation allowed, optionally with warning entries (warn threshold
    /// crossed but pause threshold not yet).
    Allow(Vec<BudgetWarning>),
    /// Pause threshold crossed — caller must surface an approval gate.
    /// `warnings` are dimensions on this same account that crossed the warn
    /// threshold before the pause point was hit.
    RequiresApproval {
        warnings: Vec<BudgetWarning>,
        needed: ResourceApprovalNeeded,
    },
    /// Hard limit exceeded — fail closed.
    /// `warnings` are dimensions on this same account that crossed the warn
    /// or pause threshold before this terminal denial fired.
    Deny {
        warnings: Vec<BudgetWarning>,
        denial: ResourceDenial,
    },
}

/// Evaluate one account in the cascade. Hard denial wins over approval
/// requirement (a 100% overrun is never "ask the user", it's "stop"). In
/// either terminal outcome we still carry through every warning the
/// dimensions produced so the audit sink / UI can render the full picture.
fn evaluate_cascade_for_account(
    account: &ResourceAccount,
    limits: &ResourceLimits,
    usage: &ResourceTally,
    reserved: &ResourceTally,
    requested: &ResourceTally,
    period_end: Option<DateTime<Utc>>,
) -> Result<CascadeOutcome, ResourceError> {
    // Collect warnings across every dimension first. Even if a later step
    // hard-denies or pauses, these warnings should still surface to the
    // event sink so users see the warn signal that preceded the terminal.
    let (warnings, approval) =
        check_thresholds_all_interventions(account, limits, usage, reserved, requested, period_end);
    if let Some(denial) = check_limits_first_denial(account, limits, usage, reserved, requested) {
        return Ok(CascadeOutcome::Deny { warnings, denial });
    }
    if let Some(needed) = approval {
        return Ok(CascadeOutcome::RequiresApproval { warnings, needed });
    }
    Ok(CascadeOutcome::Allow(warnings))
}

/// Threshold-driven intervention (warn or pause-with-approval).
enum ThresholdIntervention {
    Warning(BudgetWarning),
    Approval(ResourceApprovalNeeded),
}

/// Returns the first denied dimension in canonical resource order.
///
/// `Some(0)` or `Some(<0)` in any dimension is treated as unlimited (see
/// [`ResourceLimits`] docstring); `None` is also unlimited.
fn check_limits_first_denial(
    account: &ResourceAccount,
    limits: &ResourceLimits,
    usage: &ResourceTally,
    reserved: &ResourceTally,
    requested: &ResourceTally,
) -> Option<ResourceDenial> {
    check_decimal(
        account,
        ResourceDimension::Usd,
        limits.max_usd,
        usage.usd,
        reserved.usd,
        requested.usd,
    )
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::InputTokens,
            limits.max_input_tokens,
            usage.input_tokens,
            reserved.input_tokens,
            requested.input_tokens,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::OutputTokens,
            limits.max_output_tokens,
            usage.output_tokens,
            reserved.output_tokens,
            requested.output_tokens,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::WallClockMs,
            limits.max_wall_clock_ms,
            usage.wall_clock_ms,
            reserved.wall_clock_ms,
            requested.wall_clock_ms,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::OutputBytes,
            limits.max_output_bytes,
            usage.output_bytes,
            reserved.output_bytes,
            requested.output_bytes,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::NetworkEgressBytes,
            limits.max_network_egress_bytes,
            usage.network_egress_bytes,
            reserved.network_egress_bytes,
            requested.network_egress_bytes,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::ProcessCount,
            limits.max_process_count.map(u64::from),
            u64::from(usage.process_count),
            u64::from(reserved.process_count),
            u64::from(requested.process_count),
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::ConcurrencySlots,
            limits.max_concurrency_slots.map(u64::from),
            u64::from(usage.concurrency_slots),
            u64::from(reserved.concurrency_slots),
            u64::from(requested.concurrency_slots),
        )
    })
}

/// Walk every dimension, collecting every warning along the way. Returns
/// the first pause-approval (a single account can only be paused on one
/// dimension at a time, but every warning that fired before / alongside
/// the pause is still in `warnings`).
fn check_thresholds_all_interventions(
    account: &ResourceAccount,
    limits: &ResourceLimits,
    usage: &ResourceTally,
    reserved: &ResourceTally,
    requested: &ResourceTally,
    period_end: Option<DateTime<Utc>>,
) -> (Vec<BudgetWarning>, Option<ResourceApprovalNeeded>) {
    let mut warnings: Vec<BudgetWarning> = Vec::new();
    let mut approval: Option<ResourceApprovalNeeded> = None;
    if limits.thresholds.pause_at >= 1.0 && limits.thresholds.warn_at >= 1.0 {
        return (warnings, approval);
    }
    let mut absorb = |intervention: ThresholdIntervention| match intervention {
        ThresholdIntervention::Warning(warning) => warnings.push(warning),
        ThresholdIntervention::Approval(needed) => {
            if approval.is_none() {
                approval = Some(needed);
            }
        }
    };
    if let Some(intervention) = threshold_decimal(ThresholdInputs {
        account,
        dimension: ResourceDimension::Usd,
        limit: limits.max_usd,
        usage: usage.usd,
        reserved: reserved.usd,
        requested: requested.usd,
        thresholds: limits.thresholds,
        period_end,
    }) {
        absorb(intervention);
    }
    for (dimension, limit, usage_v, reserved_v, requested_v) in [
        (
            ResourceDimension::InputTokens,
            limits.max_input_tokens,
            usage.input_tokens,
            reserved.input_tokens,
            requested.input_tokens,
        ),
        (
            ResourceDimension::OutputTokens,
            limits.max_output_tokens,
            usage.output_tokens,
            reserved.output_tokens,
            requested.output_tokens,
        ),
        (
            ResourceDimension::WallClockMs,
            limits.max_wall_clock_ms,
            usage.wall_clock_ms,
            reserved.wall_clock_ms,
            requested.wall_clock_ms,
        ),
        (
            ResourceDimension::OutputBytes,
            limits.max_output_bytes,
            usage.output_bytes,
            reserved.output_bytes,
            requested.output_bytes,
        ),
        (
            ResourceDimension::NetworkEgressBytes,
            limits.max_network_egress_bytes,
            usage.network_egress_bytes,
            reserved.network_egress_bytes,
            requested.network_egress_bytes,
        ),
        (
            ResourceDimension::ProcessCount,
            limits.max_process_count.map(u64::from),
            u64::from(usage.process_count),
            u64::from(reserved.process_count),
            u64::from(requested.process_count),
        ),
        (
            ResourceDimension::ConcurrencySlots,
            limits.max_concurrency_slots.map(u64::from),
            u64::from(usage.concurrency_slots),
            u64::from(reserved.concurrency_slots),
            u64::from(requested.concurrency_slots),
        ),
    ] {
        if let Some(intervention) = threshold_integer(ThresholdInputs {
            account,
            dimension,
            limit,
            usage: usage_v,
            reserved: reserved_v,
            requested: requested_v,
            thresholds: limits.thresholds,
            period_end,
        }) {
            absorb(intervention);
        }
    }
    (warnings, approval)
}

fn check_decimal(
    account: &ResourceAccount,
    dimension: ResourceDimension,
    limit: Option<Decimal>,
    usage: Decimal,
    reserved: Decimal,
    requested: Decimal,
) -> Option<ResourceDenial> {
    // 0 (or negative) = unlimited per the convention in `ResourceLimits`.
    let limit = limit.filter(|v| *v > Decimal::ZERO)?;
    let exceeds = match usage
        .checked_add(reserved)
        .and_then(|subtotal| subtotal.checked_add(requested))
    {
        Some(total) => total > limit,
        None => true,
    };
    if exceeds {
        Some(ResourceDenial {
            account: account.clone(),
            dimension,
            limit: ResourceValue::Decimal(limit),
            current_usage: ResourceValue::Decimal(usage),
            active_reserved: ResourceValue::Decimal(reserved),
            requested: ResourceValue::Decimal(requested),
        })
    } else {
        None
    }
}

fn check_integer(
    account: &ResourceAccount,
    dimension: ResourceDimension,
    limit: Option<u64>,
    usage: u64,
    reserved: u64,
    requested: u64,
) -> Option<ResourceDenial> {
    // 0 = unlimited per the convention in `ResourceLimits`.
    let limit = limit.filter(|v| *v > 0)?;
    if usage.saturating_add(reserved).saturating_add(requested) > limit {
        Some(ResourceDenial {
            account: account.clone(),
            dimension,
            limit: ResourceValue::Integer(limit),
            current_usage: ResourceValue::Integer(usage),
            active_reserved: ResourceValue::Integer(reserved),
            requested: ResourceValue::Integer(requested),
        })
    } else {
        None
    }
}

/// Inputs to threshold evaluation. Bundled so the dimension-typed helpers stay
/// inside clippy's `too_many_arguments` default and so the cascade can pass
/// per-dimension snapshots without re-spelling six positional parameters.
struct ThresholdInputs<'a, T> {
    account: &'a ResourceAccount,
    dimension: ResourceDimension,
    limit: Option<T>,
    usage: T,
    reserved: T,
    requested: T,
    thresholds: BudgetThresholds,
    period_end: Option<DateTime<Utc>>,
}

fn threshold_decimal(inputs: ThresholdInputs<'_, Decimal>) -> Option<ThresholdIntervention> {
    let ThresholdInputs {
        account,
        dimension,
        limit,
        usage,
        reserved,
        requested,
        thresholds,
        period_end,
    } = inputs;
    let limit = limit.filter(|v| *v > Decimal::ZERO)?;
    let total = usage.checked_add(reserved)?.checked_add(requested)?;
    let utilization = decimal_to_f64(total) / decimal_to_f64(limit);
    // A threshold at or above 1.0 is "disabled": utilization that hits 1.0
    // is already a hard deny, so the only useful pause point is strictly
    // below 1.0. Approval at exactly 100% utilization fires when pause_at
    // is set below 1.0 (e.g. the recommended 0.90 default).
    if thresholds.pause_at < 1.0 && utilization >= thresholds.pause_at {
        return Some(ThresholdIntervention::Approval(ResourceApprovalNeeded {
            account: account.clone(),
            dimension,
            limit: ResourceValue::Decimal(limit),
            current_usage: ResourceValue::Decimal(usage),
            active_reserved: ResourceValue::Decimal(reserved),
            requested: ResourceValue::Decimal(requested),
            utilization,
            period_end,
        }));
    }
    if thresholds.warn_at < 1.0 && utilization >= thresholds.warn_at {
        return Some(ThresholdIntervention::Warning(BudgetWarning {
            account: account.clone(),
            dimension,
            utilization,
            limit: ResourceValue::Decimal(limit),
            period_end,
        }));
    }
    None
}

fn threshold_integer(inputs: ThresholdInputs<'_, u64>) -> Option<ThresholdIntervention> {
    let ThresholdInputs {
        account,
        dimension,
        limit,
        usage,
        reserved,
        requested,
        thresholds,
        period_end,
    } = inputs;
    let limit = limit.filter(|v| *v > 0)?;
    let total = usage.saturating_add(reserved).saturating_add(requested);
    let utilization = total as f64 / limit as f64;
    if thresholds.pause_at < 1.0 && utilization >= thresholds.pause_at {
        return Some(ThresholdIntervention::Approval(ResourceApprovalNeeded {
            account: account.clone(),
            dimension,
            limit: ResourceValue::Integer(limit),
            current_usage: ResourceValue::Integer(usage),
            active_reserved: ResourceValue::Integer(reserved),
            requested: ResourceValue::Integer(requested),
            utilization,
            period_end,
        }));
    }
    if thresholds.warn_at < 1.0 && utilization >= thresholds.warn_at {
        return Some(ThresholdIntervention::Warning(BudgetWarning {
            account: account.clone(),
            dimension,
            utilization,
            limit: ResourceValue::Integer(limit),
            period_end,
        }));
    }
    None
}

fn decimal_to_f64(d: Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    d.to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_snapshot_replace_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resources.json");
        let temp_path = temp_path_for(&path);
        std::fs::write(&path, b"old").unwrap();
        write_temp_snapshot(&temp_path, b"new").unwrap();

        replace_file_atomically(&temp_path, &path).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert!(!temp_path.exists());
    }

    #[test]
    fn unsupported_parent_directory_sync_is_best_effort() {
        let error = std::io::Error::new(ErrorKind::Unsupported, "directory sync unsupported");

        assert!(normalize_parent_dir_sync_result(Err(error)).is_ok());
    }
}
