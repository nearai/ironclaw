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
use ironclaw_host_api::UserId;
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
pub trait BudgetGateStore: Send + Sync + std::fmt::Debug {
    fn open(&self, gate: BudgetApprovalGate) -> Result<(), BudgetGateError>;
    fn resolve(
        &self,
        id: BudgetGateId,
        outcome: BudgetGateOutcome,
        at: DateTime<Utc>,
    ) -> Result<BudgetApprovalGate, BudgetGateError>;
    fn expire_pending_older_than(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError>;
    fn get(&self, id: BudgetGateId) -> Result<Option<BudgetApprovalGate>, BudgetGateError>;
    fn list_pending(&self) -> Result<Vec<BudgetApprovalGate>, BudgetGateError>;
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
    fn open(&self, gate: BudgetApprovalGate) -> Result<(), BudgetGateError> {
        let mut guard = self.lock();
        guard.insert(gate.id, gate);
        Ok(())
    }

    fn resolve(
        &self,
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

    fn get(&self, id: BudgetGateId) -> Result<Option<BudgetApprovalGate>, BudgetGateError> {
        let guard = self.lock();
        Ok(guard.get(&id).cloned())
    }

    fn list_pending(&self) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
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
        let gate = sample_gate();
        let id = gate.id;
        store.open(gate.clone()).unwrap();
        let got = store.get(id).unwrap().unwrap();
        assert_eq!(got.id, id);
        assert!(matches!(got.status, BudgetGateStatus::Pending));
    }

    #[test]
    fn approve_resolves_gate_with_increased_limit() {
        let store = InMemoryBudgetGateStore::new();
        let gate = sample_gate();
        let id = gate.id;
        store.open(gate).unwrap();
        let user = UserId::new("alice").unwrap();
        let new_limits = ResourceLimits {
            max_usd: Some(Decimal::from(50)),
            ..ResourceLimits::default()
        };
        let resolved = store
            .resolve(
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
    fn cancel_resolves_gate_as_cancelled() {
        let store = InMemoryBudgetGateStore::new();
        let gate = sample_gate();
        let id = gate.id;
        store.open(gate).unwrap();
        let user = UserId::new("bob").unwrap();
        let resolved = store
            .resolve(
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
        let gate = sample_gate();
        let id = gate.id;
        store.open(gate).unwrap();
        let user = UserId::new("alice").unwrap();
        store
            .resolve(
                id,
                BudgetGateOutcome::Cancel { by: user.clone() },
                Utc::now(),
            )
            .unwrap();
        let err = store
            .resolve(id, BudgetGateOutcome::Cancel { by: user }, Utc::now())
            .unwrap_err();
        assert!(matches!(err, BudgetGateError::AlreadyResolved { .. }));
    }

    #[test]
    fn resolve_unknown_gate_fails_with_unknown() {
        let store = InMemoryBudgetGateStore::new();
        let user = UserId::new("alice").unwrap();
        let unknown_id = BudgetGateId::new();
        let err = store
            .resolve(
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
        let mut gate = sample_gate();
        gate.expires_at = Utc::now() - chrono::Duration::hours(1);
        let id = gate.id;
        store.open(gate).unwrap();
        let expired = store.expire_pending_older_than(Utc::now()).unwrap();
        assert_eq!(expired.len(), 1);
        let after = store.get(id).unwrap().unwrap();
        assert!(matches!(after.status, BudgetGateStatus::Expired { .. }));
    }

    #[test]
    fn list_pending_excludes_resolved_gates() {
        let store = InMemoryBudgetGateStore::new();
        let pending = sample_gate();
        let resolved = sample_gate();
        let resolved_id = resolved.id;
        store.open(pending).unwrap();
        store.open(resolved).unwrap();
        store
            .resolve(
                resolved_id,
                BudgetGateOutcome::Cancel {
                    by: UserId::new("alice").unwrap(),
                },
                Utc::now(),
            )
            .unwrap();
        let pending_list = store.list_pending().unwrap();
        assert_eq!(pending_list.len(), 1);
        assert_ne!(pending_list[0].id, resolved_id);
    }
}
