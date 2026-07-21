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
        let secret_master_key = match (self.secret_master_key, self.resolve_secret_master_key) {
            (Some(key), false) => Some(SecretString::from(key)),
            (None, true) => match ironclaw_secrets::keychain::resolve_master_key_material().await {
                Ok(key) => key,
                Err(error) => {
                    tracing::warn!(
                        "failed to resolve secrets master key; secrets will be reported as unmigrated: {error}"
                    );
                    None
                }
            },
            (None, false) => None,
            (Some(_), true) => unreachable!("clap conflict prevents both secret key sources"),
        };
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
