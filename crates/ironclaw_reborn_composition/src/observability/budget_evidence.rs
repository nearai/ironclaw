use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_runner::loop_exit_applier::ResourceGateEvidenceStore;
use ironclaw_turns::{LoopGateRef, TurnError, TurnScope};

pub(crate) fn budget_gate_evidence(
    budget_gate_store: Arc<dyn ironclaw_resources::BudgetGateStore>,
) -> Arc<dyn ResourceGateEvidenceStore> {
    Arc::new(BudgetGateEvidence { budget_gate_store })
}

struct BudgetGateEvidence {
    budget_gate_store: Arc<dyn ironclaw_resources::BudgetGateStore>,
}

#[async_trait]
impl ResourceGateEvidenceStore for BudgetGateEvidence {
    async fn pending_resource_gate(
        &self,
        scope: &TurnScope,
        gate_ref: &LoopGateRef,
    ) -> Result<bool, TurnError> {
        let Some(gate_id) = budget_gate_id_from_gate_ref(gate_ref)? else {
            return Ok(false);
        };
        let record = self
            .budget_gate_store
            .get(&scope.to_resource_scope(), gate_id)
            .map_err(|error| TurnError::Unavailable {
                reason: format!("budget gate evidence lookup failed: {error}"),
            })?;
        Ok(record
            .map(|record| record.status == ironclaw_resources::BudgetGateStatus::Pending)
            .unwrap_or(false))
    }
}

fn budget_gate_id_from_gate_ref(
    gate_ref: &LoopGateRef,
) -> Result<Option<ironclaw_resources::BudgetGateId>, TurnError> {
    let Some(value) = gate_ref.as_str().strip_prefix("gate:budget-") else {
        return Ok(None);
    };
    let id = uuid::Uuid::parse_str(value).map_err(|error| TurnError::InvalidRequest {
        reason: format!("invalid budget gate ref `{}`: {error}", gate_ref.as_str()),
    })?;
    Ok(Some(ironclaw_resources::BudgetGateId::from_uuid(id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_gate_ref_parser_rejects_malformed_budget_ref() {
        let gate_ref =
            ironclaw_turns::LoopGateRef::new("gate:budget-not-a-uuid").expect("gate ref");
        let error = budget_gate_id_from_gate_ref(&gate_ref)
            .expect_err("malformed budget gate refs should fail loudly");

        assert!(
            matches!(
                error,
                TurnError::InvalidRequest { ref reason }
                if reason.contains("invalid budget gate ref")
                    && reason.contains("gate:budget-not-a-uuid")
            ),
            "unexpected error: {error:?}"
        );
    }
}
