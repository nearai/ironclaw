use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use anyhow::{Context, ensure};
use clap::{ArgGroup, Args, Subcommand};
use serde::Deserialize;

use crate::context::RebornCliContext;
use crate::context::V1MigrationSourceCandidate;

#[cfg(not(windows))]
const COMPANION_FILE_STEM: &str = "ironclaw-reborn-migration";
const COMPANION_HANDSHAKE_SCHEMA: &str = "ironclaw.reborn.migration-companion/v1";
const COMPANION_PROTOCOL_VERSION: u32 = 1;
const COMPANION_ERROR_FORMAT_ENV: &str = "IRONCLAW_REBORN_MIGRATION_ERROR_FORMAT";
pub(crate) const MIGRATION_STATE_MARKER_FILE: &str = ".v1-migration-state.json";
const MIGRATION_STATE_MARKER_SCHEMA: &str = "ironclaw.reborn.migration-state/v1";

/// Migrate persisted state into Reborn.
#[derive(Debug, Args)]
pub(crate) struct MigrateCommand {
    #[command(subcommand)]
    target: MigrationTarget,
}

#[derive(Debug, Subcommand)]
enum MigrationTarget {
    /// Migrate an IronClaw v1 installation.
    V1(V1MigrationCommand),
}

#[derive(Debug, Args)]
struct V1MigrationCommand {
    #[command(subcommand)]
    operation: V1MigrationOperation,
}

#[derive(Debug, Subcommand)]
enum V1MigrationOperation {
    /// Inventory a v1 snapshot and write a reviewable migration manifest.
    Plan(PlanArgs),
    /// Apply a reviewed plan into a fresh staged Reborn target.
    Apply(ApplyArgs),
    /// Resume an interrupted apply using its migration manifest.
    Resume(ResumeArgs),
    /// Verify an applied target with structural durable-store readback.
    Verify(VerifyArgs),
    /// Show the current migration status recorded in a manifest.
    Status(StatusArgs),
}

#[derive(Debug, Args)]
struct SourceArgs {
    /// WAL-consistent libSQL/SQLite snapshot to inventory.
    #[arg(long, value_name = "SNAPSHOT", group = "source")]
    source_libsql: Option<PathBuf>,

    /// Read the PostgreSQL snapshot URL from MIGRATION_SOURCE_POSTGRES.
    #[arg(long, group = "source")]
    source_postgres: bool,

    /// v1 home whose persistent artifacts belong to this source snapshot.
    #[arg(long, value_name = "PATH")]
    source_home: Option<PathBuf>,
}

#[derive(Debug, Args)]
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

    /// Fail after writing when inventory is non-live, unsupported, or blocked.
    #[arg(long)]
    strict: bool,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("source")
        .required(true)
        .multiple(false)
))]
struct ApplyArgs {
    #[command(flatten)]
    source: SourceArgs,

    /// Reviewed migration plan to apply.
    #[arg(long, value_name = "PATH")]
    plan: PathBuf,

    /// Confirm that v1 is stopped and the plan's source is a consistent snapshot.
    #[arg(long, required = true)]
    confirm_v1_stopped: bool,

    /// Confirm that the selected source was created as a consistent snapshot.
    #[arg(long, required = true)]
    confirm_source_snapshot: bool,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("source")
        .required(true)
        .multiple(false)
))]
struct ResumeArgs {
    #[command(flatten)]
    source: SourceArgs,

    /// Migration manifest to resume.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,

    /// Confirm that v1 remains stopped.
    #[arg(long, required = true)]
    confirm_v1_stopped: bool,

    /// Confirm that the selected source is the plan's consistent snapshot.
    #[arg(long, required = true)]
    confirm_source_snapshot: bool,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("source")
        .required(true)
        .multiple(false)
))]
struct VerifyArgs {
    #[command(flatten)]
    source: SourceArgs,

    /// Migration manifest to verify.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,
}

#[derive(Debug, Args)]
struct StatusArgs {
    /// Migration manifest to inspect.
    #[arg(long, value_name = "PATH")]
    manifest: PathBuf,

    /// Emit the machine-readable status document.
    #[arg(long)]
    json: bool,
}

impl MigrateCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        let args = self.target.forward_args();
        launch_companion(args)
    }
}

pub(crate) fn plan_detected_v1(
    source: V1MigrationSourceCandidate,
    source_home: &Path,
    manifest: &Path,
) -> anyhow::Result<()> {
    let mut args = vec![OsString::from("v1"), OsString::from("plan")];
    match source {
        V1MigrationSourceCandidate::LibSql(path) => {
            push_path_option(&mut args, "--source-libsql", path)
        }
        V1MigrationSourceCandidate::PostgresEnvironment => {
            args.push(OsString::from("--source-postgres"));
        }
    }
    push_path_option(&mut args, "--source-home", source_home.to_path_buf());
    push_path_option(&mut args, "--manifest", manifest.to_path_buf());
    launch_companion(args)
}

pub(crate) fn ensure_activation_allowed(context: &RebornCliContext) -> anyhow::Result<()> {
    let status = read_activation_state_status(context)?;
    let Some(status) = status else {
        return Ok(());
    };
    activation_status_allowed(&status)
}

pub(crate) fn read_activation_state_status(
    context: &RebornCliContext,
) -> anyhow::Result<Option<String>> {
    let marker = context
        .boot_config()
        .home()
        .path()
        .join(MIGRATION_STATE_MARKER_FILE);
    let local = read_local_target_state_status(&marker)?;
    let shared = read_shared_target_state_status(context)?;
    Ok(most_restrictive_status(local, shared))
}

fn most_restrictive_status(local: Option<String>, shared: Option<String>) -> Option<String> {
    let quarantined =
        |status: &str| matches!(status, "applying" | "failed" | "applied" | "verifying");
    match (local, shared) {
        (Some(_), Some(shared)) if quarantined(&shared) => Some(shared),
        (Some(local), Some(_)) if quarantined(&local) => Some(local),
        (_, Some(shared)) => Some(shared),
        (local, None) => local,
    }
}

fn read_local_target_state_status(marker: &Path) -> anyhow::Result<Option<String>> {
    let body = match fs::read_to_string(marker) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect the target-owned v1 migration state at {}",
                    marker.display()
                )
            });
        }
    };
    let document: serde_json::Value = serde_json::from_str(&body).with_context(|| {
        format!(
            "target-owned v1 migration state at {} is invalid; keep the target quarantined and inspect the migration manifest recorded in that marker",
            marker.display()
        )
    })?;
    validate_state_header(
        document
            .get("schema_version")
            .and_then(serde_json::Value::as_str),
        document
            .get("migration_protocol_version")
            .and_then(serde_json::Value::as_u64),
    )?;
    let status = document
        .get("status")
        .and_then(serde_json::Value::as_str)
        .context("v1 migration state marker has no status")?;
    Ok(Some(validate_state_status(status)?.to_owned()))
}

fn validate_state_header(schema: Option<&str>, protocol: Option<u64>) -> anyhow::Result<()> {
    ensure!(
        schema == Some(MIGRATION_STATE_MARKER_SCHEMA),
        "Reborn target is quarantined because its v1 migration state marker has an unknown schema"
    );
    ensure!(
        protocol == Some(u64::from(COMPANION_PROTOCOL_VERSION)),
        "Reborn target is quarantined because its v1 migration state marker has an incompatible protocol"
    );
    Ok(())
}

fn validate_state_status(status: &str) -> anyhow::Result<&str> {
    ensure!(
        matches!(
            status,
            "planned" | "applying" | "failed" | "applied" | "verifying" | "verified"
        ),
        "Reborn target is quarantined because v1 migration status `{status}` is unknown"
    );
    Ok(status)
}

fn activation_status_allowed(status: &str) -> anyhow::Result<()> {
    match status {
        "planned" | "verified" => Ok(()),
        "applying" | "failed" | "applied" | "verifying" => anyhow::bail!(
            "Reborn target is quarantined because v1 migration status is `{status}`; do not start live workers or ingress until migration verification records `verified`"
        ),
        _ => anyhow::bail!(
            "Reborn target is quarantined because v1 migration status `{status}` is unknown"
        ),
    }
}

#[cfg(feature = "postgres")]
fn read_shared_target_state_status(context: &RebornCliContext) -> anyhow::Result<Option<String>> {
    use ironclaw_reborn_composition::RebornMigrationTargetStore;

    let target =
        ironclaw_reborn_composition::resolve_reborn_migration_target(context.boot_config())
            .context("failed to resolve the Reborn target for migration quarantine inspection")?;
    let url = match target.store {
        RebornMigrationTargetStore::Postgres { url } => url,
        #[cfg(feature = "libsql")]
        RebornMigrationTargetStore::LibSql { .. } => return Ok(None),
    };
    crate::runtime::block_on_cli(async move {
        let pool = ironclaw_reborn_composition::open_reborn_postgres_pool(url).context(
            "failed to open the Reborn PostgreSQL target for migration quarantine inspection",
        )?;
        let client = pool
            .get()
            .await
            .context("failed to inspect shared PostgreSQL migration quarantine state")?;
        let relation: Option<String> = client
            .query_one("SELECT to_regclass('reborn_migration_state')::text", &[])
            .await
            .context("failed to inspect shared PostgreSQL migration quarantine schema")?
            .try_get(0)
            .context("invalid shared PostgreSQL migration quarantine schema result")?;
        if relation.is_none() {
            return Ok::<Option<String>, anyhow::Error>(None);
        }
        let row = client
            .query_opt(
                "SELECT schema_version, migration_protocol_version, status \
                 FROM reborn_migration_state WHERE singleton = TRUE",
                &[],
            )
            .await
            .context("failed to read shared PostgreSQL migration quarantine state")?
            .context("shared PostgreSQL migration quarantine table has no singleton state")?;
        let schema: String = row
            .try_get(0)
            .context("invalid shared PostgreSQL migration quarantine schema version")?;
        let protocol: i64 = row
            .try_get(1)
            .context("invalid shared PostgreSQL migration quarantine protocol version")?;
        let status: String = row
            .try_get(2)
            .context("invalid shared PostgreSQL migration quarantine status")?;
        let protocol = u64::try_from(protocol)
            .context("shared PostgreSQL migration quarantine protocol version is negative")?;
        validate_state_header(Some(&schema), Some(protocol))?;
        Ok::<Option<String>, anyhow::Error>(Some(validate_state_status(&status)?.to_owned()))
    })
}

#[cfg(not(feature = "postgres"))]
fn read_shared_target_state_status(_context: &RebornCliContext) -> anyhow::Result<Option<String>> {
    Ok(None)
}

fn launch_companion(args: Vec<OsString>) -> anyhow::Result<()> {
    let companion = resolve_companion(
        &std::env::current_exe()
            .context("failed to locate the running ironclaw-reborn executable")?,
    )?;
    verify_handshake(&companion)?;

    let status = Command::new(&companion)
        .args(args)
        .env(COMPANION_ERROR_FORMAT_ENV, "json")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| {
            format!(
                "failed to start the Reborn migration companion at {}",
                companion.display()
            )
        })?;

    propagate_status(status)
}

impl MigrationTarget {
    fn forward_args(self) -> Vec<OsString> {
        match self {
            Self::V1(command) => command.forward_args(),
        }
    }
}

impl V1MigrationCommand {
    fn forward_args(self) -> Vec<OsString> {
        let mut args = vec![OsString::from("v1")];
        match self.operation {
            V1MigrationOperation::Plan(command) => {
                args.push(OsString::from("plan"));
                push_source_args(&mut args, command.source);
                push_path_option(&mut args, "--manifest", command.manifest);
                if command.strict {
                    args.push(OsString::from("--strict"));
                }
            }
            V1MigrationOperation::Apply(command) => {
                args.push(OsString::from("apply"));
                push_source_args(&mut args, command.source);
                push_path_option(&mut args, "--plan", command.plan);
                if command.confirm_v1_stopped {
                    args.push(OsString::from("--confirm-v1-stopped"));
                }
                if command.confirm_source_snapshot {
                    args.push(OsString::from("--confirm-source-snapshot"));
                }
            }
            V1MigrationOperation::Resume(command) => {
                args.push(OsString::from("resume"));
                push_source_args(&mut args, command.source);
                push_path_option(&mut args, "--manifest", command.manifest);
                if command.confirm_v1_stopped {
                    args.push(OsString::from("--confirm-v1-stopped"));
                }
                if command.confirm_source_snapshot {
                    args.push(OsString::from("--confirm-source-snapshot"));
                }
            }
            V1MigrationOperation::Verify(command) => {
                args.push(OsString::from("verify"));
                push_source_args(&mut args, command.source);
                push_path_option(&mut args, "--manifest", command.manifest);
            }
            V1MigrationOperation::Status(command) => {
                args.push(OsString::from("status"));
                push_path_option(&mut args, "--manifest", command.manifest);
                if command.json {
                    args.push(OsString::from("--json"));
                }
            }
        }
        args
    }
}

fn push_source_args(args: &mut Vec<OsString>, source: SourceArgs) {
    if let Some(path) = source.source_libsql {
        push_path_option(args, "--source-libsql", path);
    } else if source.source_postgres {
        args.push(OsString::from("--source-postgres"));
    }
    if let Some(path) = source.source_home {
        push_path_option(args, "--source-home", path);
    }
}

fn push_path_option(args: &mut Vec<OsString>, option: &'static str, value: PathBuf) {
    args.push(OsString::from(option));
    args.push(value.into_os_string());
}

#[derive(Debug, Deserialize)]
struct CompanionHandshake {
    schema_version: String,
    protocol_version: u32,
    release_version: String,
}

fn companion_file_name() -> &'static OsStr {
    #[cfg(windows)]
    {
        OsStr::new("ironclaw-reborn-migration.exe")
    }
    #[cfg(not(windows))]
    {
        OsStr::new(COMPANION_FILE_STEM)
    }
}

fn resolve_companion(current_exe: &Path) -> anyhow::Result<PathBuf> {
    let bin_dir = current_exe.parent().with_context(|| {
        format!(
            "cannot resolve the installation directory for {}",
            current_exe.display()
        )
    })?;
    let companion = bin_dir.join(companion_file_name());
    let metadata = fs::symlink_metadata(&companion).with_context(|| {
        format!(
            "the Reborn migration companion is missing at {}; reinstall or upgrade Reborn with migration support",
            companion.display()
        )
    })?;
    ensure!(
        !metadata.file_type().is_symlink() && metadata.is_file(),
        "refusing migration companion at {} because it is not a regular, non-symlink file",
        companion.display()
    );

    verify_companion_permissions(current_exe, &companion, &metadata)?;
    Ok(companion)
}

#[cfg(unix)]
fn verify_companion_permissions(
    current_exe: &Path,
    companion: &Path,
    companion_metadata: &fs::Metadata,
) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let current_metadata = fs::metadata(current_exe).with_context(|| {
        format!(
            "failed to inspect the running Reborn executable at {}",
            current_exe.display()
        )
    })?;
    ensure!(
        current_metadata.uid() == companion_metadata.uid(),
        "refusing migration companion at {} because it is not owned by the Reborn binary owner",
        companion.display()
    );
    ensure!(
        companion_metadata.mode() & 0o022 == 0,
        "refusing migration companion at {} because it is group- or world-writable",
        companion.display()
    );
    let install_dir = companion
        .parent()
        .context("migration companion has no installation directory")?;
    let install_metadata = fs::metadata(install_dir).with_context(|| {
        format!(
            "failed to inspect the Reborn installation directory at {}",
            install_dir.display()
        )
    })?;
    ensure!(
        install_metadata.uid() == current_metadata.uid() && install_metadata.mode() & 0o022 == 0,
        "refusing migration companion because its installation directory {} is writable by another user",
        install_dir.display()
    );
    Ok(())
}

#[cfg(not(unix))]
fn verify_companion_permissions(
    _current_exe: &Path,
    _companion: &Path,
    _companion_metadata: &fs::Metadata,
) -> anyhow::Result<()> {
    Ok(())
}

fn verify_handshake(companion: &Path) -> anyhow::Result<()> {
    let output = Command::new(companion)
        .arg("__handshake")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| {
            format!(
                "failed to query the migration companion at {}",
                companion.display()
            )
        })?;
    ensure!(
        output.status.success(),
        "migration companion handshake failed at {}: {}",
        companion.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let handshake: CompanionHandshake = serde_json::from_slice(&output.stdout)
        .context("migration companion returned an invalid handshake document")?;
    ensure!(
        handshake.schema_version == COMPANION_HANDSHAKE_SCHEMA,
        "migration companion protocol schema mismatch: expected {}, found {}",
        COMPANION_HANDSHAKE_SCHEMA,
        handshake.schema_version
    );
    ensure!(
        handshake.protocol_version == COMPANION_PROTOCOL_VERSION,
        "migration companion protocol mismatch: expected {}, found {}",
        COMPANION_PROTOCOL_VERSION,
        handshake.protocol_version
    );
    ensure!(
        handshake.release_version == env!("CARGO_PKG_VERSION"),
        "migration companion release mismatch: ironclaw-reborn is {}, companion is {}; install both executables from the same release",
        env!("CARGO_PKG_VERSION"),
        handshake.release_version
    );
    Ok(())
}

fn propagate_status(status: ExitStatus) -> anyhow::Result<()> {
    if status.success() {
        return Ok(());
    }

    if let Some(code) = status.code() {
        std::process::exit(code);
    }

    anyhow::bail!("migration companion terminated without an exit code")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(release_version: &str, status: &str) -> String {
        serde_json::json!({
            "schema_version": MIGRATION_STATE_MARKER_SCHEMA,
            "migration_protocol_version": COMPANION_PROTOCOL_VERSION,
            "release_version": release_version,
            "status": status,
        })
        .to_string()
    }

    #[test]
    fn plan_forwarding_never_contains_database_urls_or_keys() {
        let command = V1MigrationCommand {
            operation: V1MigrationOperation::Plan(PlanArgs {
                source: SourceArgs {
                    source_libsql: None,
                    source_postgres: true,
                    source_home: Some(PathBuf::from("v1-home")),
                },
                manifest: PathBuf::from("manifest.json"),
                strict: true,
            }),
        };

        assert_eq!(
            command.forward_args(),
            vec![
                "v1",
                "plan",
                "--source-postgres",
                "--source-home",
                "v1-home",
                "--manifest",
                "manifest.json",
                "--strict",
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verified_marker_from_compatible_release_allows_activation() {
        let (_tmp, context) = RebornCliContext::test_context();
        let path = context
            .boot_config()
            .home()
            .path()
            .join(MIGRATION_STATE_MARKER_FILE);
        fs::create_dir_all(context.boot_config().home().path()).expect("create home");
        fs::write(&path, marker("previous-release", "verified")).expect("write marker");

        ensure_activation_allowed(&context).expect("compatible verified marker should activate");
        assert_eq!(
            read_activation_state_status(&context)
                .expect("read marker")
                .as_deref(),
            Some("verified")
        );
    }

    #[test]
    fn shared_quarantine_dominates_stale_local_verified_state() {
        assert_eq!(
            most_restrictive_status(Some("verified".to_string()), Some("applying".to_string()))
                .as_deref(),
            Some("applying")
        );
    }
}
