//! `BudgetGate` adapter over the host's `CostGuard`.
//!
//! Implements the engine's mission `BudgetGate` trait so a mission fire is
//! refused when the user has exhausted their daily LLM budget. The mission
//! manager calls `allow_mission_fire` before each spawn; a `false` return
//! aborts the spawn without consuming the mission's daily quota.

use std::sync::Arc;

use ironclaw_engine::{BudgetGate, MissionId};

use crate::agent::cost_guard::CostGuard;

/// Adapts a host `CostGuard` to the engine `BudgetGate` trait.
pub struct CostGuardBudgetGate {
    cost_guard: Arc<CostGuard>,
}

impl CostGuardBudgetGate {
    pub fn new(cost_guard: Arc<CostGuard>) -> Self {
        Self { cost_guard }
    }
}

#[async_trait::async_trait]
impl BudgetGate for CostGuardBudgetGate {
    async fn allow_mission_fire(&self, user_id: &str, mission_id: MissionId) -> bool {
        match self.cost_guard.check_allowed_for_user(user_id).await {
            Ok(()) => true,
            Err(error) => {
                tracing::debug!(
                    user_id = %user_id,
                    mission_id = %mission_id,
                    error = %error,
                    "mission fire refused — cost guard denies user"
                );
                false
            }
        }
    }
}
