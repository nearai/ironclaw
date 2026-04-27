//! Resource reservation governor for IronClaw Reborn.
//!
//! `ironclaw_resources` enforces the host-level reservation protocol used by
//! runtime lanes before they spend money or consume scarce sandbox capacity:
//! reserve estimated resources, execute work, then reconcile actual usage or
//! release the unused hold.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use ironclaw_host_api::{
    AgentId, MissionId, ProjectId, ResourceEstimate, ResourceReservationId, ResourceScope,
    ResourceUsage, TenantId, ThreadId, UserId,
};
pub use ironclaw_host_api::{ReservationStatus, ResourceReceipt, ResourceReservation};
use rust_decimal::Decimal;
use thiserror::Error;

/// Durable account level that can carry resource limits and ledgers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceLimits {
    pub max_usd: Option<Decimal>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_wall_clock_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub max_network_egress_bytes: Option<u64>,
    pub max_process_count: Option<u32>,
    pub max_concurrency_slots: Option<u32>,
}

/// Resource dimension that may deny a reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Resource governor errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResourceError {
    #[error("resource limit exceeded for {dimension} at {account:?}", account = .0.account, dimension = .0.dimension)]
    LimitExceeded(Box<ResourceDenial>),
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
}

/// Aggregated resource usage/reservation tally.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

/// Synchronous resource governor contract.
pub trait ResourceGovernor: Send + Sync {
    /// Sets or replaces limits for a scoped resource account without mutating existing reservations.
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits);

    /// Reserves estimated resources before costed/quota-limited work starts.
    ///
    /// A reservation succeeds only if every account in [`ResourceAccount::cascade`]
    /// would remain within its limits. Limits at deeper accounts do not override
    /// shallower limits; tenant, user, project, agent, mission, and thread limits
    /// all apply when present.
    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError>;

    /// Reserves estimated resources with a caller-supplied reservation id for obligation handoff.
    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError>;

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
}

/// In-memory governor used by early Reborn contract tests.
#[derive(Debug, Default)]
pub struct InMemoryResourceGovernor {
    state: Mutex<ResourceState>,
}

#[derive(Debug, Default)]
struct ResourceState {
    limits: HashMap<ResourceAccount, ResourceLimits>,
    reserved_by_account: HashMap<ResourceAccount, ResourceTally>,
    usage_by_account: HashMap<ResourceAccount, ResourceTally>,
    reservations: HashMap<ResourceReservationId, ReservationRecord>,
}

#[derive(Debug, Clone)]
struct ReservationRecord {
    reservation: ResourceReservation,
    accounts: Vec<ResourceAccount>,
    tally: ResourceTally,
    status: ReservationStatus,
    actual: Option<ResourceUsage>,
}

impl InMemoryResourceGovernor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reserved_for(&self, account: &ResourceAccount) -> ResourceTally {
        self.lock_state()
            .reserved_by_account
            .get(account)
            .cloned()
            .unwrap_or_default()
    }

    pub fn usage_for(&self, account: &ResourceAccount) -> ResourceTally {
        self.lock_state()
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
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits) {
        self.lock_state().limits.insert(account, limits);
    }

    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError> {
        self.reserve_with_id(scope, estimate, ResourceReservationId::new())
    }

    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError> {
        validate_estimate(&estimate)?;

        let mut state = self.lock_state();
        if state.reservations.contains_key(&reservation_id) {
            return Err(ResourceError::ReservationAlreadyExists { id: reservation_id });
        }
        let accounts = ResourceAccount::cascade(&scope);
        let requested = ResourceTally::from_estimate(&estimate);

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
                if let Some(denial) = check_limits(account, limits, &usage, &reserved, &requested) {
                    return Err(ResourceError::LimitExceeded(Box::new(denial)));
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

        Ok(reservation)
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        let mut state = self.lock_state();
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

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        let mut state = self.lock_state();
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

/// Returns the first denied dimension in canonical resource order.
///
/// This intentionally reports one denial rather than aggregating all failed
/// dimensions so callers have a deterministic, compact failure reason.
fn check_limits(
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

fn check_decimal(
    account: &ResourceAccount,
    dimension: ResourceDimension,
    limit: Option<Decimal>,
    usage: Decimal,
    reserved: Decimal,
    requested: Decimal,
) -> Option<ResourceDenial> {
    let limit = limit?;
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
    let limit = limit?;
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
