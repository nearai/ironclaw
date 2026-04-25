//! Resource scope, estimate, usage, and quota contracts.
//!
//! `ironclaw_resources` owns enforcement, but this module defines the shared
//! shapes used by callers and audit records. [`ResourceScope`] captures the
//! tenant/user/project/mission/thread/invocation cascade. [`ResourceEstimate`]
//! and [`ResourceUsage`] describe budgeted work, while [`SandboxQuota`] and
//! [`ResourceCeiling`] describe runtime limits that sandbox providers enforce.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    InvocationId, MissionId, ProjectId, ResourceReservationId, TenantId, ThreadId, UserId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub invocation_id: InvocationId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceEstimate {
    pub usd: Option<Decimal>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub wall_clock_ms: Option<u64>,
    pub output_bytes: Option<u64>,
    pub network_egress_bytes: Option<u64>,
    pub process_count: Option<u32>,
    pub concurrency_slots: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub usd: Decimal,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_clock_ms: u64,
    pub output_bytes: u64,
    pub network_egress_bytes: u64,
    pub process_count: u32,
}

/// Active reservation returned by a resource governor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceReservation {
    pub id: ResourceReservationId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
}

/// Reservation lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationStatus {
    Active,
    Reconciled,
    Released,
}

/// Receipt returned when a resource reservation is reconciled or released.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceReceipt {
    pub id: ResourceReservationId,
    pub scope: ResourceScope,
    pub status: ReservationStatus,
    pub estimate: ResourceEstimate,
    pub actual: Option<ResourceUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceProfile {
    pub default_estimate: ResourceEstimate,
    pub hard_ceiling: Option<ResourceCeiling>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCeiling {
    pub max_usd: Option<Decimal>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_wall_clock_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub sandbox: Option<SandboxQuota>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxQuota {
    pub cpu_time_ms: Option<u64>,
    pub memory_bytes: Option<u64>,
    pub disk_bytes: Option<u64>,
    pub network_egress_bytes: Option<u64>,
    pub process_count: Option<u32>,
}
