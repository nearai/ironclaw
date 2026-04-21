//! Deprecated CLI alias for `ironclaw migrate`.

#[cfg(feature = "migrate")]
/// **Deprecated**: use [`MigrateCommand`](crate::cli::migrate::MigrateCommand) instead.
/// This alias will be removed in a future release.
pub type ImportCommand = crate::cli::migrate::MigrateCommand;

#[cfg(feature = "migrate")]
pub async fn run_import_command(
    cmd: &ImportCommand,
    config: &crate::config::Config,
) -> anyhow::Result<()> {
    crate::cli::run_migrate_command(cmd, config).await
}

#[cfg(not(feature = "migrate"))]
pub async fn run_import_command(_cmd: &(), _config: &crate::config::Config) -> anyhow::Result<()> {
    anyhow::bail!("Migration feature not enabled. Compile with --features migrate")
}
