//! Resource scope, estimate, usage, and quota contracts.
//!
//! `ironclaw_resources` owns enforcement, but this module defines the shared
//! shapes used by callers and audit records. [`ResourceScope`] captures the
//! tenant/user/agent/project/mission/thread/invocation cascade. [`ResourceEstimate`]
//! and [`ResourceUsage`] describe budgeted work, while [`SandboxQuota`] and
//! [`ResourceCeiling`] describe runtime limits that sandbox providers enforce.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    AgentId, HostApiError, InvocationId, MissionId, ProjectId, TenantId, ThreadId, UserId,
};

/// Canonical local/single-user tenant id.
pub const LOCAL_DEFAULT_TENANT_ID: &str = "default";
/// Canonical local/single-user default agent id.
pub const LOCAL_DEFAULT_AGENT_ID: &str = "default";
/// Canonical local/single-user default bootstrap project id.
pub const LOCAL_DEFAULT_PROJECT_ID: &str = "bootstrap";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub invocation_id: InvocationId,
}

impl ResourceScope {
    /// Build the canonical local/single-user scope.
    ///
    /// This intentionally uses concrete `default` tenant/agent ids and the
    /// `bootstrap` project. Optional `None` scopes remain reserved for
    /// deliberately unscoped/shared records, not for the normal local default.
    pub fn local_default(
        user_id: UserId,
        invocation_id: InvocationId,
    ) -> Result<Self, HostApiError> {
        Ok(Self {
            tenant_id: TenantId::new(LOCAL_DEFAULT_TENANT_ID)?,
            user_id,
            agent_id: Some(AgentId::new(LOCAL_DEFAULT_AGENT_ID)?),
            project_id: Some(ProjectId::new(LOCAL_DEFAULT_PROJECT_ID)?),
            mission_id: None,
            thread_id: None,
            invocation_id,
        })
    }
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

/// Active reservation returned by a resource governor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceReservation {
    pub id: crate::ResourceReservationId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
}

/// Reservation lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReservationStatus {
    Active,
    Reconciled,
    Released,
}

/// Receipt returned when a reservation is reconciled or released.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceReceipt {
    pub id: crate::ResourceReservationId,
    pub scope: ResourceScope,
    pub status: ReservationStatus,
    pub estimate: ResourceEstimate,
    pub actual: Option<ResourceUsage>,
}
