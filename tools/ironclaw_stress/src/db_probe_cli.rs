use ironclaw_stress::db_probe as core;

pub(crate) use ironclaw_stress::db_probe::{
    DbProbeDelta, DbProbeSnapshot, DbProbeSummary, DbWriteMeasurement, StatsScope, summarize,
    summarize_measurement,
};
#[cfg(test)]
pub(crate) use ironclaw_stress::db_probe::{
    LibSqlTableWrites, PostgresStatementCalls, PostgresTableWrites, aggregate_statement_calls,
    install_libsql_write_counters, pg_stat_statements_reset_supported,
    pg_stat_statements_unavailable, sanitize_postgres_error, try_capture_libsql,
};

use core::DbProbeConfig;

use crate::{Args, Backend};

fn config(args: &Args) -> Result<DbProbeConfig, String> {
    match args.backend {
        Backend::Libsql => Ok(DbProbeConfig::libsql(
            args.libsql_path
                .clone()
                .unwrap_or_else(crate::default_libsql_path),
            args.db_write_reset_stats,
        )),
        Backend::Postgres => Ok(DbProbeConfig::postgres(
            crate::resolve_postgres_url(args)?,
            args.db_write_reset_stats,
        )),
    }
}

pub(crate) async fn capture(args: &Args) -> DbProbeSnapshot {
    let config = match config(args) {
        Ok(config) => config,
        Err(error) => {
            return DbProbeSnapshot {
                error: Some(format!("postgres probe failed: {error}")),
                ..DbProbeSnapshot::default()
            };
        }
    };
    core::capture_unmeasured(&config).await
}

pub(crate) async fn begin_measurement(args: &Args) -> Result<DbProbeSnapshot, String> {
    core::begin(&config(args)?).await
}

pub(crate) async fn capture_measurement(args: &Args) -> Result<DbProbeSnapshot, String> {
    core::capture_settled(&config(args)?).await
}

pub(crate) async fn finish_measurement(args: &Args) -> Result<(), String> {
    if matches!(args.backend, Backend::Postgres) {
        return Ok(());
    }
    core::finish(&config(args)?).await
}
