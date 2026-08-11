//! The **writer** half of the host-authored checkpoint-rejection envelope.
//!
//! The category→summary data tables, and the *reader* that revalidates a
//! persisted envelope before projecting it, moved to
//! `ironclaw_host_api::failure::summary` with WS1.7 (PROPOSAL §6.1.1). This
//! half could not follow them: it is typed on `ironclaw_agent_loop`'s
//! `CheckpointKind` and `ironclaw_loop_contracts`' `LoopSafeSummary`, and
//! `ironclaw_host_api` may hold no internal dependency.
//!
//! The envelope's literals therefore have exactly one definition — the shared
//! constants in `host_api::failure::summary` — and the round-trip test below
//! drives writer→reader across the crate boundary for every `CheckpointKind`,
//! so the split halves cannot drift.

use ironclaw_agent_loop::state::CheckpointKind;
use ironclaw_host_api::failure::summary::{
    CHECKPOINT_REJECTION_CAUSE_SEPARATOR, CHECKPOINT_REJECTION_PREFIX,
    CHECKPOINT_REJECTION_REMEDIATION,
};
use ironclaw_loop_contracts::LoopSafeSummary;

pub(crate) fn checkpoint_rejection_host_explanation(
    stage: CheckpointKind,
    cause: &LoopSafeSummary,
) -> String {
    format!(
        "{CHECKPOINT_REJECTION_PREFIX}{}{CHECKPOINT_REJECTION_CAUSE_SEPARATOR}{}{CHECKPOINT_REJECTION_REMEDIATION}",
        checkpoint_stage_name(stage),
        cause.as_str(),
    )
}

fn checkpoint_stage_name(stage: CheckpointKind) -> &'static str {
    match stage {
        CheckpointKind::BeforeModel => "pre-model",
        CheckpointKind::BeforeSideEffect => "pre-side-effect",
        CheckpointKind::BeforeBlock => "pre-block",
        CheckpointKind::Final => "final",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::failure::summary::checkpoint_rejection_host_explanation_from_detail;
    use ironclaw_host_api::result_meta::{MODEL_DIAGNOSTIC_MAX_BYTES, ModelDiagnostic};

    #[test]
    fn checkpoint_rejection_explanation_is_bounded_and_provenance_validated() {
        let cause = LoopSafeSummary::new("a".repeat(512)).expect("maximum safe summary");
        let explanation =
            checkpoint_rejection_host_explanation(CheckpointKind::BeforeModel, &cause);

        assert!(explanation.len() <= MODEL_DIAGNOSTIC_MAX_BYTES);
        assert!(ModelDiagnostic::new(explanation.clone()).is_ok());
        assert_eq!(
            checkpoint_rejection_host_explanation_from_detail(Some(&explanation)),
            Some(explanation)
        );
        assert_eq!(
            checkpoint_rejection_host_explanation_from_detail(Some(
                "The host rejected the unknown checkpoint because safe cause. No model or capability ran after the rejection. Start a new run. If this repeats, ask an operator to inspect checkpoint storage and run-profile compatibility."
            )),
            None
        );
    }

    /// WS1.7 split this envelope's writer (here) from its reader
    /// (`host_api::failure::summary`), so the stage vocabulary the writer
    /// renders and the closed set the reader admits are now defined in two
    /// crates. The pre-split round-trip covered only `BeforeModel`; this drives
    /// every `CheckpointKind` through writer→reader so a new or renamed stage
    /// cannot start failing revalidation silently — which would degrade the
    /// projection to the pinned fallback rather than error.
    #[test]
    fn checkpoint_rejection_explanation_round_trips_every_stage() {
        let cause = LoopSafeSummary::new("the host rejected it").expect("safe summary");
        for stage in [
            CheckpointKind::BeforeModel,
            CheckpointKind::BeforeSideEffect,
            CheckpointKind::BeforeBlock,
            CheckpointKind::Final,
        ] {
            let explanation = checkpoint_rejection_host_explanation(stage, &cause);
            assert_eq!(
                checkpoint_rejection_host_explanation_from_detail(Some(&explanation)),
                Some(explanation.clone()),
                "{stage:?} must survive envelope revalidation across the crate boundary"
            );
        }
    }
}
