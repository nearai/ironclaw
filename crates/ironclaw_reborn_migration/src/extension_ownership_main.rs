//! Explicit one-time rewrite of every installed extension to user ownership.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_reborn_migration::{
    ExtensionOwnershipMigrationOptions, TargetStore, run_extension_ownership_migration,
};
use secrecy::SecretString;

/// Assign every installed extension to every existing user in one tenant.
/// Stop the IronClaw server before applying this migration.
#[derive(Parser)]
#[command(name = "ironclaw-reborn-extension-ownership-migration", version, about)]
struct Cli {
    /// Reborn libSQL/SQLite database path.
    #[arg(
        long,
        conflicts_with = "target_postgres",
        env = "MIGRATION_TARGET_LIBSQL"
    )]
    target_libsql: Option<PathBuf>,

    /// Reborn PostgreSQL connection URL.
    #[arg(
        long,
        conflicts_with = "target_libsql",
        env = "MIGRATION_TARGET_POSTGRES"
    )]
    target_postgres: Option<String>,

    /// Tenant whose installed extensions and user directory are migrated.
    #[arg(long)]
    tenant_id: String,

    /// User absent from the persisted directory that must own every extension.
    /// Repeat this flag for multiple bootstrap users.
    #[arg(long = "include-user")]
    include_users: Vec<String>,

    /// Print the planned rewrite without changing installation state.
    #[arg(long)]
    dry_run: bool,
}

impl Cli {
    fn into_options(self) -> anyhow::Result<ExtensionOwnershipMigrationOptions> {
        let target = match (self.target_libsql, self.target_postgres) {
            (Some(path), None) => TargetStore::LibSql { path },
            (None, Some(url)) => TargetStore::Postgres {
                url: SecretString::from(url),
            },
            _ => anyhow::bail!("exactly one of --target-libsql / --target-postgres is required"),
        };
        let tenant_id = TenantId::new(self.tenant_id)?;
        let include_users = self
            .include_users
            .into_iter()
            .map(UserId::new)
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(ExtensionOwnershipMigrationOptions {
            target,
            tenant_id,
            include_users,
            dry_run: self.dry_run,
        })
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("extension ownership migration failed: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let options = Cli::parse().into_options()?;
    let report = run_extension_ownership_migration(options).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
