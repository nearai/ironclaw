//! `ironclaw-reborn-migration` — convert IronClaw v1 / engine-v2 state into the
//! Reborn state substrate.
//!
//! Thin CLI wrapper over [`ironclaw_reborn_migration::run_migration`]. The
//! conversion engine lives in the library so it can later be reused inside
//! `ironclaw-reborn` startup (documented follow-up).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_reborn_migration::{
    MigrationOptions, MigrationReport, SourceDb, TargetStore, run_migration,
};
use secrecy::SecretString;

/// Migrate IronClaw v1 / engine-v2 persisted state into Reborn state.
///
/// Deliberately no `Debug` derive: this struct holds the secrets master key and
/// PostgreSQL connection URLs (which embed `user:password@host`) as plain
/// strings, so a stray `{cli:?}` must not be able to leak them. `clap::Parser`
/// does not require `Debug`.
#[derive(Parser)]
#[command(name = "ironclaw-reborn-migration", version, about)]
struct Cli {
    /// v1 source: path to a libSQL/SQLite database file.
    #[arg(
        long,
        conflicts_with = "source_postgres",
        env = "MIGRATION_SOURCE_LIBSQL"
    )]
    source_libsql: Option<PathBuf>,

    /// v1 source: PostgreSQL connection URL.
    #[arg(
        long,
        conflicts_with = "source_libsql",
        env = "MIGRATION_SOURCE_POSTGRES"
    )]
    source_postgres: Option<String>,

    /// Reborn target: path to the libSQL store to write (created if absent).
    #[arg(
        long,
        conflicts_with = "target_postgres",
        env = "MIGRATION_TARGET_LIBSQL"
    )]
    target_libsql: Option<PathBuf>,

    /// Reborn target: PostgreSQL connection URL.
    #[arg(
        long,
        conflicts_with = "target_libsql",
        env = "MIGRATION_TARGET_POSTGRES"
    )]
    target_postgres: Option<String>,

    /// Reborn tenant all migrated state is written under.
    #[arg(long, default_value = "default")]
    tenant_id: String,

    /// Reborn agent migrated threads/triggers/memory are scoped to.
    #[arg(long, default_value = "default")]
    agent_id: String,

    /// Secrets master key (needed only to migrate secrets). Prefer the env var.
    #[arg(long, env = "MIGRATION_SECRET_MASTER_KEY")]
    secret_master_key: Option<String>,

    /// Resolve the v1 secrets master key from SECRETS_MASTER_KEY or OS keychain.
    #[arg(long, conflicts_with = "secret_master_key")]
    resolve_secret_master_key: bool,

    /// Report what would be migrated without writing to the Reborn store.
    #[arg(long)]
    dry_run: bool,

    /// Write the JSON report to this path (otherwise printed to stdout).
    #[arg(long)]
    report: Option<PathBuf>,
}

impl Cli {
    async fn into_options(self) -> anyhow::Result<(MigrationOptions, Option<PathBuf>)> {
        let source = match (self.source_libsql, self.source_postgres) {
            (Some(path), None) => SourceDb::LibSql { path },
            (None, Some(url)) => SourceDb::Postgres {
                url: SecretString::from(url),
            },
            _ => anyhow::bail!("exactly one of --source-libsql / --source-postgres is required"),
        };
        let target = match (self.target_libsql, self.target_postgres) {
            (Some(path), None) => TargetStore::LibSql { path },
            (None, Some(url)) => TargetStore::Postgres {
                url: SecretString::from(url),
            },
            _ => anyhow::bail!("exactly one of --target-libsql / --target-postgres is required"),
        };
        let tenant_id = TenantId::new(self.tenant_id)?;
        let agent_id = AgentId::new(self.agent_id)?;
        // Only touch the env/keychain when the user asked us to resolve; an
        // explicit `--secret-master-key` (or neither flag) never consults it.
        let resolved = if self.resolve_secret_master_key {
            ironclaw_secrets::keychain::resolve_master_key_material().await
        } else {
            Ok(None)
        };
        let secret_master_key = select_secret_master_key(
            self.secret_master_key,
            self.resolve_secret_master_key,
            resolved,
        )?;
        let options = MigrationOptions {
            source,
            target,
            tenant_id,
            agent_id,
            secret_master_key,
            dry_run: self.dry_run,
        };
        Ok((options, self.report))
    }
}

/// Resolve the effective secrets master key from the two mutually-exclusive CLI
/// inputs (`--secret-master-key` vs `--resolve-secret-master-key`) and the
/// outcome of the keychain/env lookup.
///
/// Fail-loud: when the user explicitly asked us to resolve the key and the
/// lookup **errored**, the migration aborts. Swallowing that error and
/// returning `None` would silently skip every secret and leave the target in a
/// broken half-migrated state, violating the crate's "nothing is silently
/// dropped" invariant (see `.claude/rules/error-handling.md`). A successful
/// lookup that simply found no key (`Ok(None)`) is a legitimate absence — the
/// secrets converter records it as a reported loss, not a swallowed failure.
fn select_secret_master_key(
    explicit: Option<String>,
    resolve: bool,
    resolved: Result<Option<SecretString>, ironclaw_secrets::SecretError>,
) -> anyhow::Result<Option<SecretString>> {
    match (explicit, resolve) {
        (Some(key), false) => Ok(Some(SecretString::from(key))),
        (None, true) => resolved
            .map_err(|error| anyhow::anyhow!("failed to resolve secrets master key: {error}")),
        (None, false) => Ok(None),
        (Some(_), true) => unreachable!("clap conflict prevents both secret key sources"),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("migration failed: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let (options, report_path) = cli.into_options().await?;
    let dry_run = options.dry_run;

    let report = run_migration(options).await?;
    emit_report(&report, report_path.as_deref()).await?;

    let losses = report.lossy.len();
    if dry_run {
        eprintln!("dry run complete: {} lossy item(s) reported", losses);
    } else {
        eprintln!("migration complete: {} lossy item(s) reported", losses);
    }
    Ok(())
}

async fn emit_report(
    report: &MigrationReport,
    path: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let json = report.to_json()?;
    match path {
        Some(path) => tokio::fs::write(path, json).await?,
        None => println!("{json}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_secrets::SecretError;
    use secrecy::ExposeSecret;

    #[test]
    fn explicit_key_resolution_failure_aborts_rather_than_silently_skipping() {
        // User asked us to resolve the key (`--resolve-secret-master-key`) and
        // the lookup errored. This must surface as an error, not a silent
        // `None` that would skip every secret. Regression for the swallowed
        // `tracing::warn!` + `return None` fail-loud violation.
        let result = select_secret_master_key(
            None,
            true,
            Err(SecretError::KeychainError(
                "keychain unavailable".to_string(),
            )),
        );
        let error = result.expect_err("resolution failure must abort the migration");
        assert!(
            error
                .to_string()
                .contains("failed to resolve secrets master key"),
            "error should name the resolution failure, got: {error}"
        );
    }

    #[test]
    fn explicit_key_resolution_success_carries_the_resolved_key() {
        let result = select_secret_master_key(
            None,
            true,
            Ok(Some(SecretString::from("resolved-key".to_string()))),
        );
        let key = result.expect("successful resolution should not error");
        assert_eq!(key.expect("key present").expose_secret(), "resolved-key");
    }

    #[test]
    fn absent_resolved_key_is_a_legitimate_none_not_an_error() {
        // A successful lookup that found nothing is not a failure — secrets are
        // reported as unmigrated by the converter, not swallowed here.
        let result = select_secret_master_key(None, true, Ok(None));
        assert!(
            result.expect("Ok(None) is not an error").is_none(),
            "no key found must flow through as None"
        );
    }

    #[test]
    fn explicit_master_key_flag_never_consults_resolution() {
        let result = select_secret_master_key(Some("literal-key".to_string()), false, Ok(None));
        let key = result.expect("explicit key path must not error");
        assert_eq!(key.expect("key present").expose_secret(), "literal-key");
    }
}
