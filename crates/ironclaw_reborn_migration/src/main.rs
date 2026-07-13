//! Same-release migration companion for `ironclaw-reborn migrate v1`.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, ensure};
use clap::{ArgGroup, Args, Parser, Subcommand};
use ironclaw_reborn_composition::{RebornMigrationTargetStore, resolve_reborn_migration_target};
use ironclaw_reborn_config::RebornBootConfig;
use ironclaw_reborn_migration::{
    ApplyAcknowledgements, Disposition, MIGRATION_PROTOCOL_VERSION, MigrationManifest,
    MigrationOptions, MigrationSecretInputs, MigrationStatus, SourceDb, TargetStore,
    apply_migration, manifest_target_matches, plan_migration, resume_migration, verify_migration,
};
use secrecy::SecretString;
use serde::Serialize;

const HANDSHAKE_SCHEMA: &str = "ironclaw.reborn.migration-companion/v1";
const SOURCE_POSTGRES_ENV: &str = "MIGRATION_SOURCE_POSTGRES";
const SOURCE_MASTER_KEY_ENV: &str = "MIGRATION_SOURCE_SECRET_MASTER_KEY";
const ERROR_FORMAT_ENV: &str = "IRONCLAW_REBORN_MIGRATION_ERROR_FORMAT";
const TARGET_STATE_FILE: &str = ".v1-migration-state.json";
const TARGET_STATE_SCHEMA: &str = "ironclaw.reborn.migration-state/v1";

#[derive(Parser)]
#[command(name = "ironclaw-reborn-migration", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Machine-readable compatibility handshake used by ironclaw-reborn.
    #[command(name = "__handshake", hide = true)]
    Handshake,
    /// Migrate an IronClaw v1 installation.
    V1(V1Command),
}

#[derive(Args)]
struct V1Command {
    #[command(subcommand)]
    operation: V1Operation,
}

#[derive(Subcommand)]
enum V1Operation {
    /// Inventory a v1 source without opening or creating the Reborn target.
    Plan(PlanArgs),
    /// Apply a reviewed plan to a fresh staged Reborn target.
    Apply(ApplyArgs),
    /// Resume an interrupted apply using the same stopped-source snapshot.
    Resume(ResumeArgs),
    /// Verify an applied target without activating workers or ingress.
    Verify(VerifyArgs),
    /// Inspect a migration manifest without opening either database.
    Status(StatusArgs),
}

#[derive(Args)]
struct SourceArgs {
    /// WAL-consistent libSQL/SQLite snapshot.
    #[arg(long, value_name = "SNAPSHOT", group = "source")]
    source_libsql: Option<PathBuf>,

    /// Read the PostgreSQL snapshot URL from MIGRATION_SOURCE_POSTGRES.
    #[arg(long, group = "source")]
    source_postgres: bool,

    /// v1 home containing persistent files outside the database snapshot.
    #[arg(long, value_name = "PATH")]
    source_home: Option<PathBuf>,
}

#[derive(Args)]
#[command(group(
    ArgGroup::new("source")
        .required(true)
        .multiple(false)
))]
struct PlanArgs {
    #[command(flatten)]
    source: SourceArgs,

    /// Destination for the versioned migration manifest.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,

    /// Return failure when the inventory contains unsupported or blocked data.
    #[arg(long)]
    strict: bool,
}

#[derive(Args)]
#[command(group(
    ArgGroup::new("source")
        .required(true)
        .multiple(false)
))]
struct ApplyArgs {
    #[command(flatten)]
    source: SourceArgs,

    /// Reviewed migration plan to apply and update.
    #[arg(long, value_name = "PATH")]
    plan: PathBuf,

    /// Confirm that the v1 process and every other source writer are stopped.
    #[arg(long, required = true)]
    confirm_v1_stopped: bool,

    /// Confirm that the selected source is a consistent operator-created snapshot.
    #[arg(long, required = true)]
    confirm_source_snapshot: bool,
}

#[derive(Args)]
#[command(group(
    ArgGroup::new("source")
        .required(true)
        .multiple(false)
))]
struct ResumeArgs {
    #[command(flatten)]
    source: SourceArgs,

    /// Migration manifest to resume and update.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,

    /// Confirm that the v1 process and every other source writer remain stopped.
    #[arg(long, required = true)]
    confirm_v1_stopped: bool,

    /// Confirm that the selected source is the plan's consistent snapshot.
    #[arg(long, required = true)]
    confirm_source_snapshot: bool,
}

#[derive(Args)]
#[command(group(
    ArgGroup::new("source")
        .required(true)
        .multiple(false)
))]
struct VerifyArgs {
    #[command(flatten)]
    source: SourceArgs,

    /// Applied migration manifest to verify and update.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,
}

#[derive(Args)]
struct StatusArgs {
    /// Migration manifest to inspect.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,

    /// Print the complete redacted manifest JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
struct Handshake<'a> {
    schema_version: &'a str,
    protocol_version: u32,
    release_version: &'a str,
}

struct ResolvedRun {
    options: MigrationOptions,
    target_master_key: Option<SecretString>,
    target_state_path: PathBuf,
}

struct ResolvedTarget {
    store: TargetStore,
    tenant_id: ironclaw_host_api::TenantId,
    agent_id: ironclaw_host_api::AgentId,
    master_key: Option<SecretString>,
    profile: String,
    state_path: PathBuf,
}

#[derive(Serialize)]
struct TargetMigrationState<'a> {
    schema_version: &'static str,
    migration_protocol_version: u32,
    release_version: &'static str,
    run_id: String,
    status: &'static str,
    profile: &'a str,
    manifest: &'a Path,
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

    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if std::env::var(ERROR_FORMAT_ENV).as_deref() == Ok("json") {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "schema_version": "ironclaw.reborn.migration-error/v1",
                        "code": "migration_failed",
                        "message": error.to_string(),
                    })
                );
            } else {
                eprintln!("migration failed: {error:#}");
            }
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Handshake => emit_handshake(),
        Command::V1(command) => run_v1(command.operation).await,
    }
}

fn emit_handshake() -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&Handshake {
            schema_version: HANDSHAKE_SCHEMA,
            protocol_version: MIGRATION_PROTOCOL_VERSION,
            release_version: env!("CARGO_PKG_VERSION"),
        })?
    );
    Ok(())
}

async fn run_v1(operation: V1Operation) -> anyhow::Result<()> {
    match operation {
        V1Operation::Plan(command) => {
            let run = resolve_run(command.source)?;
            let manifest = plan_migration(&run.options).await?;
            manifest.write_atomic(&command.manifest, false)?;
            println!("{}", manifest.to_json()?);

            if command.strict && manifest_has_strict_loss(&manifest) {
                anyhow::bail!(
                    "strict migration planning found unsupported data or blockers; review {}",
                    command.manifest.display()
                );
            }
            eprintln!(
                "migration plan written: {} ({} inventory categories)",
                command.manifest.display(),
                manifest.inventory.len()
            );
            Ok(())
        }
        V1Operation::Apply(command) => {
            let run = resolve_run(command.source)?;
            let manifest = read_manifest(&command.plan)?;
            let applying = manifest.transition(MigrationStatus::Applying)?;
            write_target_state(&run.target_state_path, &applying, &command.plan)?;
            applying.write_atomic(&command.plan, true)?;
            let result = apply_migration(
                run.options,
                &manifest,
                migration_secrets(run.target_master_key)?,
                ApplyAcknowledgements {
                    source_is_stopped: command.confirm_v1_stopped,
                    source_is_snapshot: command.confirm_source_snapshot,
                },
            )
            .await;
            let report = match result {
                Ok(report) => report,
                Err(error) => {
                    let failed = applying.transition(MigrationStatus::Failed)?;
                    write_target_state(&run.target_state_path, &failed, &command.plan)?;
                    failed.write_atomic(&command.plan, true)?;
                    return Err(error.into());
                }
            };
            persist_report_manifest(&report, &command.plan)?;
            write_target_state(
                &run.target_state_path,
                report
                    .manifest
                    .as_ref()
                    .context("migration apply completed without an updated manifest")?,
                &command.plan,
            )?;
            println!("{}", report.to_json()?);
            Ok(())
        }
        V1Operation::Resume(command) => {
            let run = resolve_run(command.source)?;
            let manifest = read_manifest(&command.manifest)?;
            let applying = if manifest.status == MigrationStatus::Applying {
                manifest.clone()
            } else {
                manifest.transition(MigrationStatus::Applying)?
            };
            write_target_state(&run.target_state_path, &applying, &command.manifest)?;
            applying.write_atomic(&command.manifest, true)?;
            let result = resume_migration(
                run.options,
                &manifest,
                migration_secrets(run.target_master_key)?,
                ApplyAcknowledgements {
                    source_is_stopped: command.confirm_v1_stopped,
                    source_is_snapshot: command.confirm_source_snapshot,
                },
            )
            .await;
            let report = match result {
                Ok(report) => report,
                Err(error) => {
                    let failed = applying.transition(MigrationStatus::Failed)?;
                    write_target_state(&run.target_state_path, &failed, &command.manifest)?;
                    failed.write_atomic(&command.manifest, true)?;
                    return Err(error.into());
                }
            };
            persist_report_manifest(&report, &command.manifest)?;
            write_target_state(
                &run.target_state_path,
                report
                    .manifest
                    .as_ref()
                    .context("migration resume completed without an updated manifest")?,
                &command.manifest,
            )?;
            println!("{}", report.to_json()?);
            Ok(())
        }
        V1Operation::Verify(command) => {
            let run = resolve_run(command.source)?;
            let manifest = read_manifest(&command.manifest)?;
            let verifying = match manifest.status {
                MigrationStatus::Verifying | MigrationStatus::Verified => manifest.clone(),
                _ => manifest.transition(MigrationStatus::Verifying)?,
            };
            if manifest.status != MigrationStatus::Verified {
                write_target_state(&run.target_state_path, &verifying, &command.manifest)?;
                verifying.write_atomic(&command.manifest, true)?;
            }
            let verified = match verify_migration(&run.options, &manifest).await {
                Ok(verified) => verified,
                Err(error) => {
                    if verifying.status == MigrationStatus::Verifying {
                        let failed = verifying.transition(MigrationStatus::Failed)?;
                        write_target_state(&run.target_state_path, &failed, &command.manifest)?;
                        failed.write_atomic(&command.manifest, true)?;
                    }
                    return Err(error.into());
                }
            };
            write_target_state(&run.target_state_path, &verified, &command.manifest)?;
            verified.write_atomic(&command.manifest, true)?;
            println!("{}", verified.to_json()?);
            ensure!(
                verified.status == MigrationStatus::Verified,
                "verification did not reach the verified state; the target remains quarantined and must not be started"
            );
            Ok(())
        }
        V1Operation::Status(command) => {
            let manifest = read_manifest(&command.manifest)?;
            let target = resolve_target()?;
            let target_matches = manifest_target_matches(&target.store, &manifest);
            if command.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "target_fingerprint_match": target_matches,
                        "manifest": manifest,
                    }))?
                );
            } else {
                println!("IronClaw Reborn v1 migration");
                println!("status: {}", status_label(manifest.status));
                println!("run_id: {}", manifest.run_id);
                println!("profile: {}", manifest.scope.profile);
                println!("tenant_id: {}", manifest.scope.tenant_id);
                println!("agent_id: {}", manifest.scope.agent_id);
                println!("inventory_categories: {}", manifest.inventory.len());
                println!("target_fingerprint_match: {target_matches}");
                println!("manifest: {}", command.manifest.display());
            }
            Ok(())
        }
    }
}

fn resolve_run(source: SourceArgs) -> anyhow::Result<ResolvedRun> {
    let source_home = source.source_home.clone();
    let source = resolve_source(source)?;
    let target = resolve_target()?;
    Ok(ResolvedRun {
        options: MigrationOptions {
            source,
            source_home,
            target: target.store,
            profile: target.profile.clone(),
            tenant_id: target.tenant_id,
            agent_id: target.agent_id,
            secret_master_key: None,
            dry_run: false,
        },
        target_master_key: target.master_key,
        target_state_path: target.state_path,
    })
}

fn resolve_target() -> anyhow::Result<ResolvedTarget> {
    let boot = RebornBootConfig::resolve_from_env()
        .context("failed to resolve the Reborn migration target configuration")?;
    let state_path = boot.home().path().join(TARGET_STATE_FILE);
    let target = resolve_reborn_migration_target(&boot)
        .context("failed to resolve the production Reborn migration target")?;
    let store = match target.store {
        #[cfg(feature = "libsql")]
        RebornMigrationTargetStore::LibSql { path } => TargetStore::LibSql { path },
        #[cfg(feature = "postgres")]
        RebornMigrationTargetStore::Postgres { url } => TargetStore::Postgres { url },
    };
    Ok(ResolvedTarget {
        store,
        tenant_id: target.tenant_id,
        agent_id: target.agent_id,
        master_key: target.target_master_key,
        profile: target.profile.as_str().to_owned(),
        state_path,
    })
}

fn write_target_state(
    path: &Path,
    manifest: &MigrationManifest,
    manifest_path: &Path,
) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("target migration state has no parent directory")?;
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create target migration state directory {}",
            parent.display()
        )
    })?;
    let absolute_manifest = std::path::absolute(manifest_path).with_context(|| {
        format!(
            "failed to resolve migration manifest path {}",
            manifest_path.display()
        )
    })?;
    let document = serde_json::to_vec_pretty(&TargetMigrationState {
        schema_version: TARGET_STATE_SCHEMA,
        migration_protocol_version: MIGRATION_PROTOCOL_VERSION,
        release_version: env!("CARGO_PKG_VERSION"),
        run_id: manifest.run_id.to_string(),
        status: status_label(manifest.status),
        profile: &manifest.scope.profile,
        manifest: &absolute_manifest,
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(TARGET_STATE_FILE);
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).with_context(|| {
        format!(
            "failed to create temporary target migration state {}",
            temporary.display()
        )
    })?;
    if let Err(error) = file
        .write_all(&document)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
    {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).context("failed to persist target migration state");
    }
    drop(file);
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).with_context(|| {
            format!(
                "failed to install target migration state at {}",
                path.display()
            )
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn resolve_source(source: SourceArgs) -> anyhow::Result<SourceDb> {
    match (source.source_libsql, source.source_postgres) {
        (Some(path), false) => Ok(SourceDb::LibSql { path }),
        (None, true) => Ok(SourceDb::Postgres {
            url: required_secret_env(SOURCE_POSTGRES_ENV)?,
        }),
        _ => anyhow::bail!("exactly one v1 source selector is required"),
    }
}

fn migration_secrets(
    target_master_key: Option<SecretString>,
) -> anyhow::Result<MigrationSecretInputs> {
    Ok(MigrationSecretInputs {
        source_master_key: optional_secret_env(SOURCE_MASTER_KEY_ENV)?,
        target_master_key,
    })
}

fn required_secret_env(name: &'static str) -> anyhow::Result<SecretString> {
    optional_secret_env(name)?
        .with_context(|| format!("required environment variable {name} is not set"))
}

fn optional_secret_env(name: &'static str) -> anyhow::Result<Option<SecretString>> {
    match std::env::var(name) {
        Ok(value) => {
            ensure!(
                !value.trim().is_empty(),
                "environment variable {name} is empty"
            );
            Ok(Some(SecretString::from(value)))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("environment variable {name} is not valid UTF-8")
        }
    }
}

fn read_manifest(path: &Path) -> anyhow::Result<MigrationManifest> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read migration manifest at {}", path.display()))?;
    let manifest: MigrationManifest =
        serde_json::from_str(&body).context("migration manifest is not valid versioned JSON")?;
    manifest.validate_plan_hash()?;
    ensure!(
        manifest.migration_protocol_version == MIGRATION_PROTOCOL_VERSION,
        "migration manifest protocol {} is not supported by this companion (expected {})",
        manifest.migration_protocol_version,
        MIGRATION_PROTOCOL_VERSION
    );
    Ok(manifest)
}

fn persist_report_manifest(
    report: &ironclaw_reborn_migration::MigrationReport,
    path: &Path,
) -> anyhow::Result<()> {
    let manifest = report
        .manifest
        .as_ref()
        .context("migration lifecycle completed without an updated manifest")?;
    manifest.write_atomic(path, true)?;
    Ok(())
}

fn manifest_has_strict_loss(manifest: &MigrationManifest) -> bool {
    manifest.inventory.iter().any(|entry| {
        entry.blocker.is_some()
            || matches!(
                entry.disposition,
                Disposition::ArchiveOnly
                    | Disposition::RequiresReauth
                    | Disposition::RequiresReinstall
                    | Disposition::Unsupported
                    | Disposition::UnsupportedUnknown
            )
    })
}

const fn status_label(status: MigrationStatus) -> &'static str {
    match status {
        MigrationStatus::Planned => "planned",
        MigrationStatus::Applying => "applying",
        MigrationStatus::Failed => "failed",
        MigrationStatus::Applied => "applied",
        MigrationStatus::Verifying => "verifying",
        MigrationStatus::Verified => "verified",
    }
}
