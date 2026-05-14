//! Shared `ActionPhase` ↔ wire-string mapping for the durable ledger.
//!
//! Both `ledger_libsql.rs` and `ledger_postgres.rs` need to render and parse
//! the snake_case spelling of [`ActionPhase`] when reading and writing the
//! `phase` column. The mapping is identical across backends — the migration
//! columns store the same `snake_case` text — so the helpers live here in
//! one place to remove the drift risk gemini flagged on `derive_user_id`
//! (see PR #3590, zmanian's review item #5).

use ironclaw_product_workflow::{ActionPhase, ProductWorkflowError};

use crate::error::transient;

/// Render an [`ActionPhase`] as the canonical snake_case wire string. Keep in
/// lock-step with [`parse_phase`] below — any new variant added to
/// `ActionPhase` must be added here too, or this stops compiling.
pub(crate) fn phase_to_str(phase: ActionPhase) -> &'static str {
    match phase {
        ActionPhase::Received => "received",
        ActionPhase::Dispatched => "dispatched",
        ActionPhase::Settled => "settled",
        ActionPhase::DeduplicatedReplay => "deduplicated_replay",
    }
}

/// Parse a column value back into [`ActionPhase`]. Exhaustive on the
/// persisted wire spelling; an unknown variant collapses to a transient
/// error so the protocol layer retries rather than corrupting workflow
/// state.
pub(crate) fn parse_phase(value: &str) -> Result<ActionPhase, ProductWorkflowError> {
    match value {
        "received" => Ok(ActionPhase::Received),
        "dispatched" => Ok(ActionPhase::Dispatched),
        "settled" => Ok(ActionPhase::Settled),
        "deduplicated_replay" => Ok(ActionPhase::DeduplicatedReplay),
        other => Err(transient(format!("invalid phase '{other}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_every_variant() {
        for phase in [
            ActionPhase::Received,
            ActionPhase::Dispatched,
            ActionPhase::Settled,
            ActionPhase::DeduplicatedReplay,
        ] {
            let s = phase_to_str(phase);
            let parsed = parse_phase(s).expect("roundtrip");
            assert_eq!(phase, parsed, "phase {phase:?} did not roundtrip");
        }
    }

    #[test]
    fn unknown_phase_is_transient() {
        let err = parse_phase("definitely_not_a_phase").expect_err("must reject");
        assert!(matches!(err, ProductWorkflowError::Transient { .. }));
    }
}
