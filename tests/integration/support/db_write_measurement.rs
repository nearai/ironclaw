use std::{future::Future, time::Instant};

use ironclaw_event_log::{EventStreamKey, ReadScope};
use ironclaw_processes::ProcessJournalKind;
use ironclaw_stress::db_probe::{
    DbProbeConfig, DbProbeError, DbProbeSummary, DbProbeTarget, DbWriteMeasurement, StatsScope,
    begin, capture_settled, finish, postgres_relation_write_totals, summarize_measurement,
};
use ironclaw_turns::{TurnRunId, process_projection::process_id_from_turn_run_id};
use serde::Serialize;

use super::builder::{RebornIntegrationHarness, StorageReopen};
use super::group::HarnessResult;

const REQUIRED_ROOT_FILESYSTEM_FAMILIES: [&str; 4] = [
    "root_filesystem_entries",
    "root_filesystem_events",
    "root_filesystem_ordered_index_rows",
    "root_filesystem_sequences",
];
// Canonical workloads fit in one journal page; this assertion intentionally
// reads one page so a larger workload must raise the bound explicitly.
const PROCESS_JOURNAL_ASSERTION_PAGE_SIZE: usize = 1_023;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MeasuredStorageBackend {
    Libsql,
    Postgres,
}

impl MeasuredStorageBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Libsql => "libsql",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalDbWriteMeasurement {
    workload: &'static str,
    tool_calls: usize,
}

impl CanonicalDbWriteMeasurement {
    pub const fn new(workload: &'static str, tool_calls: usize) -> Self {
        Self {
            workload,
            tool_calls,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CanonicalDbWriteMeasurementReport {
    pub backend: MeasuredStorageBackend,
    pub workload_duration_ms: u64,
    #[serde(flatten)]
    pub summary: DbProbeSummary,
}

impl CanonicalDbWriteMeasurementReport {
    pub fn assert_minimum_duration(&self, minimum: std::time::Duration) -> HarnessResult<()> {
        let minimum_ms = u64::try_from(minimum.as_millis()).unwrap_or(u64::MAX);
        if self.workload_duration_ms < minimum_ms {
            return Err(format!(
                "measured workload lasted {}ms, shorter than the required {}ms",
                self.workload_duration_ms, minimum_ms
            )
            .into());
        }
        Ok(())
    }

    pub fn assert_nonzero_root_filesystem_families(&self) -> HarnessResult<()> {
        let postgres_writes = match self.backend {
            MeasuredStorageBackend::Libsql => None,
            MeasuredStorageBackend::Postgres => Some(postgres_relation_write_totals(
                &self.summary.delta.postgres_table_writes,
            )),
        };
        for table in REQUIRED_ROOT_FILESYSTEM_FAMILIES {
            let writes = match self.backend {
                MeasuredStorageBackend::Libsql => self
                    .summary
                    .delta
                    .libsql_table_writes
                    .iter()
                    .find(|row| row.table == table)
                    .map(|row| row.inserts + row.updates + row.deletes),
                MeasuredStorageBackend::Postgres => postgres_writes
                    .as_ref()
                    .and_then(|writes| writes.get(table).copied()),
            }
            .unwrap_or_default();
            if writes <= 0 {
                return Err(format!(
                    "{} workload produced no writes for required table family {table}",
                    self.backend.as_str()
                )
                .into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DbWriteMeasurementError {
    #[error("database probe begin failed: {0}")]
    Begin(#[source] DbProbeError),
    #[error("database probe begin failed: {primary}; cleanup also failed: {cleanup}")]
    BeginAndCleanup {
        #[source]
        primary: DbProbeError,
        cleanup: DbProbeError,
    },
    #[error("measured workload failed: {0}")]
    Workload(#[source] BoxError),
    #[error("measured workload failed: {primary}; database probe cleanup also failed: {cleanup}")]
    WorkloadAndCleanup {
        #[source]
        primary: BoxError,
        cleanup: DbProbeError,
    },
    #[error("database probe capture failed: {0}")]
    Capture(#[source] DbProbeError),
    #[error("database probe capture failed: {primary}; cleanup also failed: {cleanup}")]
    CaptureAndCleanup {
        #[source]
        primary: DbProbeError,
        cleanup: DbProbeError,
    },
    #[error("database probe cleanup failed: {0}")]
    Cleanup(#[source] DbProbeError),
}

impl DbWriteMeasurementError {
    pub fn cleanup_error(&self) -> Option<&DbProbeError> {
        match self {
            Self::BeginAndCleanup { cleanup, .. }
            | Self::WorkloadAndCleanup { cleanup, .. }
            | Self::CaptureAndCleanup { cleanup, .. } => Some(cleanup),
            Self::Cleanup(error) => Some(error),
            Self::Begin(_) | Self::Workload(_) | Self::Capture(_) => None,
        }
    }
}

pub async fn measure_db_writes<T, Workload, WorkloadFuture>(
    config: &DbProbeConfig,
    metadata: CanonicalDbWriteMeasurement,
    workload: Workload,
) -> Result<(CanonicalDbWriteMeasurementReport, T), DbWriteMeasurementError>
where
    Workload: FnOnce() -> WorkloadFuture,
    WorkloadFuture: Future<Output = Result<T, BoxError>>,
{
    let before = match begin(config).await {
        Ok(before) => before,
        Err(primary) => {
            return match finish(config).await {
                Ok(()) => Err(DbWriteMeasurementError::Begin(primary)),
                Err(cleanup) => Err(DbWriteMeasurementError::BeginAndCleanup { primary, cleanup }),
            };
        }
    };

    let started = Instant::now();
    let workload_output = match workload().await {
        Ok(output) => output,
        Err(primary) => {
            return match finish(config).await {
                Ok(()) => Err(DbWriteMeasurementError::Workload(primary)),
                Err(cleanup) => {
                    Err(DbWriteMeasurementError::WorkloadAndCleanup { primary, cleanup })
                }
            };
        }
    };
    let workload_duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let after = match capture_settled(config).await {
        Ok(after) => after,
        Err(primary) => {
            return match finish(config).await {
                Ok(()) => Err(DbWriteMeasurementError::Capture(primary)),
                Err(cleanup) => {
                    Err(DbWriteMeasurementError::CaptureAndCleanup { primary, cleanup })
                }
            };
        }
    };
    finish(config)
        .await
        .map_err(DbWriteMeasurementError::Cleanup)?;

    let backend = match config.target() {
        DbProbeTarget::LibSql { .. } => MeasuredStorageBackend::Libsql,
        DbProbeTarget::Postgres { .. } => MeasuredStorageBackend::Postgres,
    };
    let report = CanonicalDbWriteMeasurementReport {
        backend,
        workload_duration_ms,
        summary: summarize_measurement(
            before,
            after,
            None,
            DbWriteMeasurement {
                workload: metadata.workload.to_string(),
                tool_calls_per_turn: metadata.tool_calls,
                idle_observation_seconds: 0,
                reset_stats: config.reset_stats(),
                stats_scope: if config.reset_stats() {
                    StatsScope::ExplicitResetCurrentDatabase
                } else {
                    StatsScope::SnapshotDeltaCurrentDatabase
                },
            },
        ),
    };
    Ok((report, workload_output))
}

impl RebornIntegrationHarness {
    pub fn db_probe_config(&self, reset_stats: bool) -> HarnessResult<DbProbeConfig> {
        match &self._shared.storage_reopen {
            StorageReopen::LibSql { db_path } => {
                Ok(DbProbeConfig::libsql(db_path.clone(), reset_stats))
            }
            StorageReopen::Postgres { database_url, .. } => {
                Ok(DbProbeConfig::postgres(database_url.clone(), reset_stats))
            }
            StorageReopen::None => {
                Err("DB-write measurement requires LibSql or Postgres storage".into())
            }
        }
    }

    pub async fn assert_process_heartbeat_count(
        &self,
        run_id: TurnRunId,
        minimum: usize,
    ) -> HarnessResult<()> {
        let page = self
            ._shared
            .process_system
            .journal()
            .read_process_journal_after(
                &self.turn_scope.to_resource_scope(),
                Some(&self.binding.actor_user_id),
                None,
                PROCESS_JOURNAL_ASSERTION_PAGE_SIZE,
            )
            .await
            .map_err(|error| format!("read process journal: {error}"))?;
        let process_id = process_id_from_turn_run_id(run_id);
        let actual = page
            .entries
            .iter()
            .filter(|entry| {
                entry.process_id == process_id && entry.kind == ProcessJournalKind::Heartbeat
            })
            .count();
        if actual < minimum {
            return Err(format!(
                "expected at least {minimum} process heartbeats for run {run_id}, found {actual}"
            )
            .into());
        }
        Ok(())
    }

    pub async fn assert_durable_event_count_at_least(&self, minimum: usize) -> HarnessResult<()> {
        let event_log = self
            ._shared
            .durable_event_log
            .as_ref()
            .ok_or("durable milestone event store is not wired")?;
        let stream = EventStreamKey::new(
            self.binding.tenant_id.clone(),
            self.binding.actor_user_id.clone(),
            self.binding.agent_id.clone(),
        );
        let replay = event_log
            .read_after_cursor(&stream, &ReadScope::any(), None, 4096)
            .await
            .map_err(|error| format!("read durable milestone events: {error}"))?;
        if replay.entries.len() < minimum {
            return Err(format!(
                "expected at least {minimum} durable milestone events, found {}",
                replay.entries.len()
            )
            .into());
        }
        Ok(())
    }
}
