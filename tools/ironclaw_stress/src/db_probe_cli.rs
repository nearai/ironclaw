use ironclaw_stress::db_probe::{self as core, DbProbeConfig, DbProbeError, DbProbeSnapshot};

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

fn measurement_config(args: &Args) -> Result<DbProbeConfig, DbProbeError> {
    config(args).map_err(|message| {
        let backend = match args.backend {
            Backend::Libsql => "libsql",
            Backend::Postgres => "postgres",
        };
        DbProbeError::operation(
            backend,
            "resolve target",
            format!("{backend} probe failed: {message}"),
        )
    })
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

pub(crate) async fn begin_measurement(args: &Args) -> Result<DbProbeSnapshot, DbProbeError> {
    core::begin(&measurement_config(args)?).await
}

pub(crate) async fn capture_measurement(args: &Args) -> Result<DbProbeSnapshot, DbProbeError> {
    core::capture_settled(&measurement_config(args)?).await
}

pub(crate) async fn finish_measurement(args: &Args) -> Result<(), DbProbeError> {
    if matches!(args.backend, Backend::Postgres) {
        return Ok(());
    }
    core::finish(&measurement_config(args)?).await
}
