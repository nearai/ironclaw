//! Budget approval gates.
//!
//! When a reservation crosses the pause threshold the loop opens a gate
//! and pauses the turn (foreground modal / background notification — both
//! resolve via the same store). A gate has three terminal states:
//!
//! - **Approved** with an increased limit → the loop's resume path calls
//!   `set_limit` with the new ceiling and retries the reservation.
//! - **Cancelled** → the loop transitions to `Failed { BudgetCancelled }`.
//! - **Expired** (after a configurable timeout, default 24h) → same as
//!   cancelled but with a distinct failure kind so audit can tell apart
//!   "user said no" from "user never replied".
//!
//! Persistence mirrors the resource-governor snapshot pattern: a
//! transactional store backed by `ScopedFilesystem` or, for tests, an
//! in-memory map.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{ResourceScope, UserId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::{ResourceApprovalNeeded, ResourceLimits};

/// Stable identifier for a single budget approval gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BudgetGateId(uuid::Uuid);

impl BudgetGateId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn from_uuid(value: uuid::Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl std::fmt::Display for BudgetGateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One pending or terminal budget approval gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetApprovalGate {
    pub id: BudgetGateId,
    pub needed: ResourceApprovalNeeded,
    pub opened_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: BudgetGateStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BudgetGateStatus {
    Pending,
    Approved {
        increased_limit: ResourceLimits,
        by: UserId,
        at: DateTime<Utc>,
    },
    Cancelled {
        by: UserId,
        at: DateTime<Utc>,
    },
    Expired {
        at: DateTime<Utc>,
    },
}

/// Wire decoder for the externally persisted budget-gate status shape.
///
/// Keep this in lockstep with [`BudgetGateStatus`] variants: each terminal
/// state has required fields, and variants must reject fields owned by other
/// states so durable snapshots cannot silently drift across releases. The
/// persisted wire shapes accepted here are pinned by decode tests below; audit
/// existing snapshots before tightening or renaming those fields.
impl<'de> Deserialize<'de> for BudgetGateStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum StatusKind {
            Pending,
            Approved,
            Cancelled,
            Expired,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StatusWire {
            kind: StatusKind,
            #[serde(default)]
            increased_limit: Option<ResourceLimits>,
            #[serde(default)]
            by: Option<UserId>,
            #[serde(default)]
            at: Option<DateTime<Utc>>,
        }

        fn required<T, E>(value: Option<T>, field: &'static str) -> Result<T, E>
        where
            E: serde::de::Error,
        {
            value.ok_or_else(|| E::missing_field(field))
        }

        let wire = StatusWire::deserialize(deserializer)?;
        match wire.kind {
            StatusKind::Pending => {
                if wire.increased_limit.is_some() || wire.by.is_some() || wire.at.is_some() {
                    return Err(serde::de::Error::custom(
                        "pending budget gate status must not include terminal fields",
                    ));
                }
                Ok(Self::Pending)
            }
            StatusKind::Approved => Ok(Self::Approved {
                increased_limit: required(wire.increased_limit, "increased_limit")?,
                by: required(wire.by, "by")?,
                at: required(wire.at, "at")?,
            }),
            StatusKind::Cancelled => {
                if wire.increased_limit.is_some() {
                    return Err(serde::de::Error::custom(
                        "cancelled budget gate status must not include increased_limit",
                    ));
                }
                Ok(Self::Cancelled {
                    by: required(wire.by, "by")?,
                    at: required(wire.at, "at")?,
                })
            }
            StatusKind::Expired => {
                if wire.increased_limit.is_some() || wire.by.is_some() {
                    return Err(serde::de::Error::custom(
                        "expired budget gate status must not include approval fields",
                    ));
                }
                Ok(Self::Expired {
                    at: required(wire.at, "at")?,
                })
            }
        }
    }
}

impl BudgetGateStatus {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// User-supplied resolution of a pending gate.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetGateOutcome {
    Approve {
        increased_limit: ResourceLimits,
        by: UserId,
    },
    Cancel {
        by: UserId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BudgetGateError {
    #[error("budget gate {id} is unknown")]
    Unknown { id: BudgetGateId },
    #[error("budget gate {id} is already resolved")]
    AlreadyResolved { id: BudgetGateId },
    #[error("budget gate storage error: {reason}")]
    Storage { reason: String },
}

impl crate::cas_snapshot::StorageError for BudgetGateError {
    fn storage(reason: String) -> Self {
        Self::Storage { reason }
    }
}

/// Transactional store for budget approval gates.
///
/// Every operation takes the caller's [`ResourceScope`] so a single
/// shared store instance can naturally route work to the right tenant
/// path under a multi-tenant deployment. The current
/// [`InMemoryBudgetGateStore`] is single-snapshot and ignores scope
/// (suitable for tests / single-tenant local-dev), while the
/// filesystem-backed store uses the scope to write under the correct
/// tenant's mount view (review feedback Thermo-Nuclear #2: scope at the
/// store-operation boundary, not at construction).
pub trait BudgetGateStore: Send + Sync + std::fmt::Debug {
    fn open(&self, scope: &ResourceScope, gate: BudgetApprovalGate) -> Result<(), BudgetGateError>;
    fn resolve(
        &self,
        scope: &ResourceScope,
        id: BudgetGateId,
        outcome: BudgetGateOutcome,
        at: DateTime<Utc>,
    ) -> Result<BudgetApprovalGate, BudgetGateError>;
    fn expire_pending_older_than(
        &self,
        scope: &ResourceScope,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError>;
    fn get(
        &self,
        scope: &ResourceScope,
        id: BudgetGateId,
    ) -> Result<Option<BudgetApprovalGate>, BudgetGateError>;
    fn list_pending(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError>;
}

/// In-memory store used by tests and the local-dev runtime. Production
/// composition can swap in a filesystem-backed store mirroring the
/// resource-governor snapshot shape (deferred to a follow-up).
#[derive(Debug, Default)]
pub struct InMemoryBudgetGateStore {
    gates: Mutex<HashMap<BudgetGateId, BudgetApprovalGate>>,
}

impl InMemoryBudgetGateStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<BudgetGateId, BudgetApprovalGate>> {
        self.gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl BudgetGateStore for InMemoryBudgetGateStore {
    fn open(
        &self,
        _scope: &ResourceScope,
        gate: BudgetApprovalGate,
    ) -> Result<(), BudgetGateError> {
        let mut guard = self.lock();
        guard.insert(gate.id, gate);
        Ok(())
    }

    fn resolve(
        &self,
        _scope: &ResourceScope,
        id: BudgetGateId,
        outcome: BudgetGateOutcome,
        at: DateTime<Utc>,
    ) -> Result<BudgetApprovalGate, BudgetGateError> {
        let mut guard = self.lock();
        let gate = guard.get_mut(&id).ok_or(BudgetGateError::Unknown { id })?;
        if gate.status.is_terminal() {
            return Err(BudgetGateError::AlreadyResolved { id });
        }
        gate.status = match outcome {
            BudgetGateOutcome::Approve {
                increased_limit,
                by,
            } => BudgetGateStatus::Approved {
                increased_limit,
                by,
                at,
            },
            BudgetGateOutcome::Cancel { by } => BudgetGateStatus::Cancelled { by, at },
        };
        Ok(gate.clone())
    }

    fn expire_pending_older_than(
        &self,
        _scope: &ResourceScope,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
        let mut guard = self.lock();
        let mut expired = Vec::new();
        for gate in guard.values_mut() {
            if matches!(gate.status, BudgetGateStatus::Pending) && gate.expires_at <= cutoff {
                gate.status = BudgetGateStatus::Expired { at: cutoff };
                expired.push(gate.clone());
            }
        }
        Ok(expired)
    }

    fn get(
        &self,
        _scope: &ResourceScope,
        id: BudgetGateId,
    ) -> Result<Option<BudgetApprovalGate>, BudgetGateError> {
        let guard = self.lock();
        Ok(guard.get(&id).cloned())
    }

    fn list_pending(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
        let guard = self.lock();
        Ok(guard
            .values()
            .filter(|g| matches!(g.status, BudgetGateStatus::Pending))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResourceAccount, ResourceDimension, ResourceValue};
    use ironclaw_host_api::TenantId;
    use rust_decimal::Decimal;

    fn sample_needed() -> ResourceApprovalNeeded {
        ResourceApprovalNeeded {
            account: ResourceAccount::tenant(TenantId::new("t").unwrap()),
            dimension: ResourceDimension::Usd,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            current_usage: ResourceValue::Decimal(Decimal::from(0)),
            active_reserved: ResourceValue::Decimal(Decimal::from(0)),
            requested: ResourceValue::Decimal(Decimal::from(9)),
            utilization: 0.91,
            period_end: None,
        }
    }

    fn sample_gate() -> BudgetApprovalGate {
        BudgetApprovalGate {
            id: BudgetGateId::new(),
            needed: sample_needed(),
            opened_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
            status: BudgetGateStatus::Pending,
        }
    }

    #[test]
    fn open_then_get_returns_pending_gate() {
        let store = InMemoryBudgetGateStore::new();
        let scope = ResourceScope::system();
        let gate = sample_gate();
        let id = gate.id;
        store.open(&scope, gate.clone()).unwrap();
        let got = store.get(&scope, id).unwrap().unwrap();
        assert_eq!(got.id, id);
        assert!(matches!(got.status, BudgetGateStatus::Pending));
    }

    #[test]
    fn approve_resolves_gate_with_increased_limit() {
        let store = InMemoryBudgetGateStore::new();
        let scope = ResourceScope::system();
        let gate = sample_gate();
        let id = gate.id;
        store.open(&scope, gate).unwrap();
        let user = UserId::new("alice").unwrap();
        let new_limits = ResourceLimits::default().set_max_usd(Decimal::from(50));
        let resolved = store
            .resolve(
                &scope,
                id,
                BudgetGateOutcome::Approve {
                    increased_limit: new_limits.clone(),
                    by: user.clone(),
                },
                Utc::now(),
            )
            .unwrap();
        assert!(matches!(
            resolved.status,
            BudgetGateStatus::Approved { ref by, .. } if by == &user
        ));
    }

    #[test]
    fn approved_status_decodes_with_numeric_thresholds() {
        let raw = r#"{
            "kind": "approved",
            "increased_limit": {
                "max_usd": "1000.00",
                "max_input_tokens": null,
                "max_output_tokens": null,
                "max_wall_clock_ms": null,
                "max_output_bytes": null,
                "max_network_egress_bytes": null,
                "max_process_count": null,
                "max_concurrency_slots": null,
                "period": { "kind": "rolling24h" },
                "thresholds": {
                    "warn_at": 1.0,
                    "pause_at": 1.0
                }
            },
            "by": "alice",
            "at": "2026-07-03T14:18:49.505189Z"
        }"#;

        let status: BudgetGateStatus = serde_json::from_str(raw).unwrap();

        assert!(matches!(
            status,
            BudgetGateStatus::Approved {
                increased_limit: ResourceLimits {
                    max_usd: Some(limit),
                    thresholds,
                    ..
                },
                ..
            } if limit == Decimal::from(1000)
                && thresholds.warn_at == 1.0
                && thresholds.pause_at == 1.0
        ));
    }

    #[test]
    fn status_decode_accepts_persisted_wire_shapes() {
        let persisted_shapes = [
            serde_json::json!({
                "kind": "pending"
            }),
            serde_json::json!({
                "kind": "cancelled",
                "by": "alice",
                "at": "2026-07-03T14:18:49.505189Z"
            }),
            serde_json::json!({
                "kind": "expired",
                "at": "2026-07-03T14:18:49.505189Z"
            }),
        ];

        for raw in persisted_shapes {
            serde_json::from_value::<BudgetGateStatus>(raw).unwrap();
        }
    }

    #[test]
    fn status_decode_rejects_invalid_field_combinations() {
        let valid_limit = serde_json::json!({
            "max_usd": "1000.00",
            "max_input_tokens": null,
            "max_output_tokens": null,
            "max_wall_clock_ms": null,
            "max_output_bytes": null,
            "max_network_egress_bytes": null,
            "max_process_count": null,
            "max_concurrency_slots": null,
            "period": { "kind": "rolling24h" },
            "thresholds": {
                "warn_at": 1.0,
                "pause_at": 1.0
            }
        });
        let cases = [
            (
                "pending status with terminal fields",
                serde_json::json!({
                    "kind": "pending",
                    "by": "alice"
                }),
            ),
            (
                "approved status missing required actor",
                serde_json::json!({
                    "kind": "approved",
                    "increased_limit": valid_limit.clone(),
                    "at": "2026-07-03T14:18:49.505189Z"
                }),
            ),
            (
                "cancelled status with increased limit",
                serde_json::json!({
                    "kind": "cancelled",
                    "increased_limit": valid_limit.clone(),
                    "by": "alice",
                    "at": "2026-07-03T14:18:49.505189Z"
                }),
            ),
            (
                "expired status with approval fields",
                serde_json::json!({
                    "kind": "expired",
                    "by": "alice",
                    "at": "2026-07-03T14:18:49.505189Z"
                }),
            ),
        ];

        for (label, raw) in cases {
            assert!(
                serde_json::from_value::<BudgetGateStatus>(raw).is_err(),
                "{label} must be rejected"
            );
        }
    }

    #[test]
    fn cancel_resolves_gate_as_cancelled() {
        let store = InMemoryBudgetGateStore::new();
        let scope = ResourceScope::system();
        let gate = sample_gate();
        let id = gate.id;
        store.open(&scope, gate).unwrap();
        let user = UserId::new("bob").unwrap();
        let resolved = store
            .resolve(
                &scope,
                id,
                BudgetGateOutcome::Cancel { by: user.clone() },
                Utc::now(),
            )
            .unwrap();
        assert!(matches!(
            resolved.status,
            BudgetGateStatus::Cancelled { ref by, .. } if by == &user
        ));
    }

    #[test]
    fn second_resolve_fails_already_resolved() {
        let store = InMemoryBudgetGateStore::new();
        let scope = ResourceScope::system();
        let gate = sample_gate();
        let id = gate.id;
        store.open(&scope, gate).unwrap();
        let user = UserId::new("alice").unwrap();
        store
            .resolve(
                &scope,
                id,
                BudgetGateOutcome::Cancel { by: user.clone() },
                Utc::now(),
            )
            .unwrap();
        let err = store
            .resolve(
                &scope,
                id,
                BudgetGateOutcome::Cancel { by: user },
                Utc::now(),
            )
            .unwrap_err();
        assert!(matches!(err, BudgetGateError::AlreadyResolved { .. }));
    }

    #[test]
    fn resolve_unknown_gate_fails_with_unknown() {
        let store = InMemoryBudgetGateStore::new();
        let scope = ResourceScope::system();
        let user = UserId::new("alice").unwrap();
        let unknown_id = BudgetGateId::new();
        let err = store
            .resolve(
                &scope,
                unknown_id,
                BudgetGateOutcome::Cancel { by: user },
                Utc::now(),
            )
            .unwrap_err();
        assert!(matches!(err, BudgetGateError::Unknown { .. }));
    }

    #[test]
    fn expire_pending_older_than_marks_stale_gates_expired() {
        let store = InMemoryBudgetGateStore::new();
        let scope = ResourceScope::system();
        let mut gate = sample_gate();
        gate.expires_at = Utc::now() - chrono::Duration::hours(1);
        let id = gate.id;
        store.open(&scope, gate).unwrap();
        let expired = store.expire_pending_older_than(&scope, Utc::now()).unwrap();
        assert_eq!(expired.len(), 1);
        let after = store.get(&scope, id).unwrap().unwrap();
        assert!(matches!(after.status, BudgetGateStatus::Expired { .. }));
    }

    #[test]
    fn list_pending_excludes_resolved_gates() {
        let store = InMemoryBudgetGateStore::new();
        let scope = ResourceScope::system();
        let pending = sample_gate();
        let resolved = sample_gate();
        let resolved_id = resolved.id;
        store.open(&scope, pending).unwrap();
        store.open(&scope, resolved).unwrap();
        store
            .resolve(
                &scope,
                resolved_id,
                BudgetGateOutcome::Cancel {
                    by: UserId::new("alice").unwrap(),
                },
                Utc::now(),
            )
            .unwrap();
        let pending_list = store.list_pending(&scope).unwrap();
        assert_eq!(pending_list.len(), 1);
        assert_ne!(pending_list[0].id, resolved_id);
    }
}
