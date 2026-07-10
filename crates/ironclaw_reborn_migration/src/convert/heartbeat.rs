//! Heartbeat converter (v1 `heartbeat_state`).
//!
//! Reborn's periodic execution is expressed through triggers/the poller, not a
//! persisted per-user heartbeat row. There is no durable heartbeat-state target
//! to migrate into, so the presence of v1 heartbeat state is recorded as a loss
//! (its cadence should be re-established as a Reborn scheduled trigger).

use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;

pub(crate) async fn run(
    src: &V1Source,
    _tgt: &mut RebornTarget,
    _options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    // heartbeat_state is keyed per (user_id, agent_id); enumerate distinct users
    // and record the gap. No typed all-user read API exists for heartbeat state.
    for user_id in src.distinct_users().await? {
        report.record_loss(
            Domain::Heartbeat,
            user_id,
            "heartbeat_state",
            LossReason::NoTargetConcept,
            "Reborn has no durable heartbeat-state record; re-establish periodic \
             execution as a scheduled trigger"
                .to_string(),
        );
    }
    Ok(())
}
