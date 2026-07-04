//! Jobs converter (v1 `agent_jobs` / `job_actions` / `job_events`).
//!
//! Reborn has no general-purpose job store: background execution is either the
//! sandboxed process runtime (`ironclaw_processes`, not a historical job log) or
//! trigger run history (`trigger_run_history`, which has no public insert API).
//! Historical v1 jobs therefore have no Reborn target — each is enumerated and
//! recorded as a loss so the operator sees exactly what did not carry over.

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
    let jobs = src
        .db
        .list_agent_jobs()
        .await
        .map_err(|e| MigrationError::ReadSource {
            domain: "agent_jobs".into(),
            reason: e.to_string(),
        })?;
    for job in jobs {
        report.record_loss(
            Domain::Job,
            job.id.to_string(),
            "*",
            LossReason::NoTargetConcept,
            "Reborn has no general job store; historical agent_jobs (and their \
             job_actions/job_events) have no migration target"
                .to_string(),
        );
    }
    Ok(())
}
