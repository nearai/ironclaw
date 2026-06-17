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

/// Reserved tenant/user id used by [`ResourceScope::system`] for filesystem
/// operations that have no real per-tenant scope (migrations, admin
/// tooling). Contains an ASCII Unit-Separator control character (`\x1f`)
/// which `TenantId::new` / `UserId::new` reject during validation, so no
/// caller-supplied identifier can ever collide with it.
pub const SYSTEM_RESERVED_ID: &str = "\x1fSYSTEM\x1f";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceScope {
    // SECURITY: `ResourceScope` is a TRUSTED-PERSISTENCE shape. It is serialized
    // into durable records (e.g. system-scoped secret entries) and read back; it
    // is NEVER deserialized from an untrusted HTTP request body. The
    // WebUI/product request DTOs carry no `tenant_id`/`user_id`/`scope` field,
    // and the caller scope is stamped host-side from trusted installation config
    // plus the authenticator's verified `UserId` (see
    // `webui_serve::authenticate_request` and the rule in
    // `crates/ironclaw_product_workflow/CLAUDE.md`), so a browser body cannot
    // influence it. Do not add a `ResourceScope` (or bare `TenantId`/`UserId`)
    // field to any untrusted request DTO.
    //
    // The system sentinel ([`SYSTEM_RESERVED_ID`]) carries control bytes that
    // `TenantId`/`UserId` validation rejects, so [`ResourceScope::system`] builds
    // it via `from_trusted`. A persisted system scope must therefore round-trip,
    // but the trusted exception stays scoped to these two fields only — the
    // shared id `Deserialize` keeps rejecting control bytes everywhere else
    // (locked by `system_sentinel_is_rejected_for_bare_ids`), so untrusted input
    // can never mint a sentinel-bearing id or collide with the reserved system
    // identity on any other axis.
    #[serde(deserialize_with = "deserialize_system_aware_tenant_id")]
    pub tenant_id: TenantId,
    #[serde(deserialize_with = "deserialize_system_aware_user_id")]
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub invocation_id: InvocationId,
}

fn deserialize_system_aware_tenant_id<'de, D>(deserializer: D) -> Result<TenantId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    if raw == SYSTEM_RESERVED_ID {
        Ok(TenantId::from_trusted(raw))
    } else {
        TenantId::new(raw).map_err(serde::de::Error::custom)
    }
}

fn deserialize_system_aware_user_id<'de, D>(deserializer: D) -> Result<UserId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    if raw == SYSTEM_RESERVED_ID {
        Ok(UserId::from_trusted(raw))
    } else {
        UserId::new(raw).map_err(serde::de::Error::custom)
    }
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

    /// Synthetic scope for system-level filesystem operations that have no
    /// real per-tenant identity (master-key checks, migrations, admin
    /// tooling). Uses [`SYSTEM_RESERVED_ID`] for both tenant and user, which
    /// validation rejects, so no user-supplied identifier can collide.
    pub fn system() -> Self {
        Self {
            tenant_id: TenantId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
            user_id: UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    /// True iff this scope is the system sentinel (see [`Self::system`]).
    pub fn is_system(&self) -> bool {
        self.tenant_id.as_str() == SYSTEM_RESERVED_ID && self.user_id.as_str() == SYSTEM_RESERVED_ID
    }

    /// Copy of this scope with the transient `mission_id`/`thread_id`
    /// sub-scope cleared. This clears mission/thread only: the owner identity
    /// (tenant/user/agent/project) and `invocation_id` are left unchanged, so it
    /// does not by itself reduce the scope to a pure owner identity.
    ///
    /// This is a neutral scope-narrowing primitive: it makes no claim about
    /// what the narrowed scope is *used* for. Policy crates that own an
    /// ownership contract (e.g. credential-account ownership in `ironclaw_auth`)
    /// build on top of this; the meaning of the narrowing lives there, not here.
    pub fn without_thread_and_mission(&self) -> Self {
        Self {
            mission_id: None,
            thread_id: None,
            ..self.clone()
        }
    }
}

/// Origin of a background reservation. Distinguishes heartbeats, routines,
/// missions, container jobs, and user-initiated work so per-kind budgets
/// can be tracked separately within the same user's daily budget.
///
/// **Contract-only for now:** schedulers that pre-date this enum still
/// open reservations through plain [`ResourceScope`]. As the Reborn
/// runtime grows native heartbeats/routines, those call sites will pass
/// the kind through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundKind {
    /// Periodic heartbeat tick (proactive memory / status checks).
    HeartbeatTick,
    /// User-defined lightweight routine.
    RoutineLightweight,
    /// User-defined standard routine (heavier per-fire budget).
    RoutineStandard,
    /// Multi-step mission tick.
    MissionTick,
    /// One-shot container job (e.g., sandboxed shell).
    ContainerJob,
    /// Explicitly user-triggered work that is not scheduled.
    UserInitiated,
}

impl BackgroundKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HeartbeatTick => "heartbeat_tick",
            Self::RoutineLightweight => "routine_lightweight",
            Self::RoutineStandard => "routine_standard",
            Self::MissionTick => "mission_tick",
            Self::ContainerJob => "container_job",
            Self::UserInitiated => "user_initiated",
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The system scope is built from a reserved id that fails normal
    /// validation, so it must still survive a serde round-trip — otherwise any
    /// persisted system-scoped record (e.g. an operator-wide secret entry)
    /// serializes but cannot be read back. Regression for the WebUI NEAR AI
    /// "save returns service_unavailable" bug.
    #[test]
    fn system_scope_survives_json_round_trip() {
        let scope = ResourceScope::system();
        let json = serde_json::to_string(&scope).expect("serialize system scope");
        let restored: ResourceScope =
            serde_json::from_str(&json).expect("deserialize system scope");
        assert!(restored.is_system());
        assert_eq!(restored.tenant_id.as_str(), SYSTEM_RESERVED_ID);
        assert_eq!(restored.user_id.as_str(), SYSTEM_RESERVED_ID);
    }

    /// The trusted-sentinel exception must not widen into a general bypass. The
    /// JSON is built via `serde_json::to_string` so the control byte becomes a
    /// proper `\uXXXX` escape; a raw control byte would be rejected at JSON parse
    /// time, before id validation runs, and pass the assertion for the wrong
    /// reason. An ordinary control-bearing id is still rejected by the validator.
    #[test]
    fn other_control_character_ids_are_still_rejected() {
        let json = serde_json::to_string("\u{1f}not-the-sentinel\u{1f}").expect("encode");
        assert!(serde_json::from_str::<TenantId>(&json).is_err());
    }

    /// The exception lives only on `ResourceScope`'s tenant/user fields, not on
    /// the shared id `Deserialize`. The exact system sentinel must NOT deserialize
    /// into a bare id type (here `TenantId` and `AgentId`), so it can never be
    /// minted from untrusted input or collide with the system identity elsewhere.
    #[test]
    fn system_sentinel_is_rejected_for_bare_ids() {
        let json = serde_json::to_string(SYSTEM_RESERVED_ID).expect("encode sentinel");
        assert!(
            serde_json::from_str::<TenantId>(&json).is_err(),
            "bare TenantId must not accept the system sentinel"
        );
        assert!(
            serde_json::from_str::<AgentId>(&json).is_err(),
            "AgentId must not accept the system sentinel"
        );
    }
}
