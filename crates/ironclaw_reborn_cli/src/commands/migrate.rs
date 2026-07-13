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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MigrationLifecycleStatus {
    Planned,
    Applying,
    Failed,
    Applied,
    Verifying,
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct MigrationStateRecord {
    schema_version: String,
    migration_protocol_version: u32,
    run_id: String,
    status: String,
    profile: String,
    target_backend: String,
    target_locator_fingerprint: String,
    tenant_id: String,
    agent_id: String,
}

impl MigrationLifecycleStatus {
    pub(crate) fn parse(status: &str) -> anyhow::Result<Self> {
        match status {
            "planned" => Ok(Self::Planned),
            "applying" => Ok(Self::Applying),
            "failed" => Ok(Self::Failed),
            "applied" => Ok(Self::Applied),
            "verifying" => Ok(Self::Verifying),
            "verified" => Ok(Self::Verified),
            _ => anyhow::bail!(
                "Reborn target is quarantined because v1 migration status `{status}` is unknown"
            ),
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Applying => "applying",
            Self::Failed => "failed",
            Self::Applied => "applied",
            Self::Verifying => "verifying",
            Self::Verified => "verified",
        }
    }

    const fn is_activation_safe(self) -> bool {
        matches!(self, Self::Planned | Self::Verified)
    }
}

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

    /// Fail after writing for blockers or nonzero archive/re-auth/reinstall/unsupported data.
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
    activation_status_allowed(status)
}

pub(crate) fn read_activation_state_status(
    context: &RebornCliContext,
) -> anyhow::Result<Option<MigrationLifecycleStatus>> {
    let marker = context
        .boot_config()
        .home()
        .path()
        .join(MIGRATION_STATE_MARKER_FILE);
    let local = read_local_target_state(&marker)?;
    let shared = read_shared_target_state(context)?;
    if local.is_none() && shared.is_none() {
        return Ok(None);
    }
    let binding = current_target_binding(context)?;
    if let Some(record) = local.as_ref() {
        validate_state_binding(record, &binding)?;
    }
    if let Some(record) = shared.as_ref() {
        validate_state_binding(record, &binding)?;
    }
    match (local, shared) {
        (None, None) => Ok(None),
        (Some(_), None) => anyhow::bail!(
            "Reborn target is quarantined because its local v1 migration marker has no matching target-owned durable state"
        ),
        (Some(local), Some(shared)) => {
            ensure!(
                local == shared,
                "Reborn target is quarantined because local and durable v1 migration state do not match"
            );
            Ok(Some(MigrationLifecycleStatus::parse(&shared.status)?))
        }
        (None, Some(shared)) => Ok(Some(MigrationLifecycleStatus::parse(&shared.status)?)),
    }
}

fn read_local_target_state(marker: &Path) -> anyhow::Result<Option<MigrationStateRecord>> {
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
    let document: MigrationStateRecord = serde_json::from_str(&body).with_context(|| {
        format!(
            "target-owned v1 migration state at {} is invalid; keep the target quarantined and inspect the migration manifest recorded in that marker",
            marker.display()
        )
    })?;
    validate_state_header(
        Some(&document.schema_version),
        Some(u64::from(document.migration_protocol_version)),
    )?;
    MigrationLifecycleStatus::parse(&document.status)?;
    Ok(Some(document))
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

fn activation_status_allowed(status: MigrationLifecycleStatus) -> anyhow::Result<()> {
    if status.is_activation_safe() {
        Ok(())
    } else {
        anyhow::bail!(
            "Reborn target is quarantined because v1 migration status is `{}`; do not start live workers or ingress until migration verification records `verified`",
            status.as_str()
        )
    }
}

#[derive(Debug)]
struct CurrentTargetBinding {
    profile: String,
    target_backend: &'static str,
    target_locator_fingerprint: String,
    tenant_id: String,
    agent_id: String,
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn current_target_binding(context: &RebornCliContext) -> anyhow::Result<CurrentTargetBinding> {
    use ironclaw_reborn_composition::RebornMigrationTargetStore;

    let target =
        ironclaw_reborn_composition::resolve_reborn_migration_target(context.boot_config())
            .context("failed to resolve the Reborn target for migration quarantine inspection")?;
    let (target_backend, target_locator_fingerprint) = match &target.store {
        #[cfg(feature = "postgres")]
        RebornMigrationTargetStore::Postgres { url } => (
            "postgres",
            ironclaw_reborn_composition::migration_postgres_locator_fingerprint(url)
                .context("failed to identify configured PostgreSQL migration target")?,
        ),
        #[cfg(feature = "libsql")]
        RebornMigrationTargetStore::LibSql { path } => (
            "libsql",
            ironclaw_reborn_composition::migration_libsql_locator_fingerprint(path),
        ),
    };
    Ok(CurrentTargetBinding {
        profile: target.profile.as_str().to_string(),
        target_backend,
        target_locator_fingerprint,
        tenant_id: target.tenant_id.to_string(),
        agent_id: target.agent_id.to_string(),
    })
}

#[cfg(not(any(feature = "postgres", feature = "libsql")))]
fn current_target_binding(_context: &RebornCliContext) -> anyhow::Result<CurrentTargetBinding> {
    anyhow::bail!("migration target inspection requires a binary built with libsql or postgres")
}

fn validate_state_binding(
    record: &MigrationStateRecord,
    binding: &CurrentTargetBinding,
) -> anyhow::Result<()> {
    ensure!(
        record.profile == binding.profile
            && record.target_backend == binding.target_backend
            && record.target_locator_fingerprint == binding.target_locator_fingerprint
            && record.tenant_id == binding.tenant_id
            && record.agent_id == binding.agent_id,
        "Reborn target is quarantined because v1 migration state does not match the configured target profile or scope"
    );
    Ok(())
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn read_shared_target_state(
    context: &RebornCliContext,
) -> anyhow::Result<Option<MigrationStateRecord>> {
    use ironclaw_reborn_composition::RebornMigrationTargetStore;

    let target = match ironclaw_reborn_composition::resolve_reborn_migration_target(
        context.boot_config(),
    ) {
        Ok(target) => target,
        Err(_) => return Ok(None),
    };
    match target.store {
        #[cfg(feature = "postgres")]
        RebornMigrationTargetStore::Postgres { url } => crate::runtime::block_on_cli(async move {
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
                return Ok::<Option<MigrationStateRecord>, anyhow::Error>(None);
            }
            let row = client
                .query_opt(
                    "SELECT schema_version, migration_protocol_version, run_id, status, profile, \
                        target_backend, target_locator_fingerprint, tenant_id, agent_id \
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
            let run_id: String = row
                .try_get(2)
                .context("invalid shared PostgreSQL migration run id")?;
            let status: String = row
                .try_get(3)
                .context("invalid shared PostgreSQL migration quarantine status")?;
            let profile: String = row.try_get(4).context("invalid shared migration profile")?;
            let target_backend: String = row.try_get(5).context("invalid shared target backend")?;
            let target_locator_fingerprint: String = row
                .try_get(6)
                .context("invalid shared target fingerprint")?;
            let tenant_id: String = row.try_get(7).context("invalid shared tenant id")?;
            let agent_id: String = row.try_get(8).context("invalid shared agent id")?;
            let protocol = u64::try_from(protocol)
                .context("shared PostgreSQL migration quarantine protocol version is negative")?;
            validate_state_header(Some(&schema), Some(protocol))?;
            MigrationLifecycleStatus::parse(&status)?;
            Ok::<Option<MigrationStateRecord>, anyhow::Error>(Some(MigrationStateRecord {
                schema_version: schema,
                migration_protocol_version: u32::try_from(protocol)
                    .context("shared migration protocol version is too large")?,
                run_id,
                status,
                profile,
                target_backend,
                target_locator_fingerprint,
                tenant_id,
                agent_id,
            }))
        }),
        #[cfg(feature = "libsql")]
        RebornMigrationTargetStore::LibSql { path } => read_libsql_target_state(path),
    }
}

#[cfg(not(any(feature = "postgres", feature = "libsql")))]
fn read_shared_target_state(
    _context: &RebornCliContext,
) -> anyhow::Result<Option<MigrationStateRecord>> {
    Ok(None)
}

#[cfg(feature = "libsql")]
fn read_libsql_target_state(path: PathBuf) -> anyhow::Result<Option<MigrationStateRecord>> {
    if !path.is_file() {
        return Ok(None);
    }
    crate::runtime::block_on_cli(async move {
        let database = libsql::Builder::new_local(&path)
            .flags(libsql::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .build()
            .await
            .context("failed to open libSQL migration quarantine state")?;
        let connection = database
            .connect()
            .context("failed to connect libSQL target")?;
        let mut schema = connection
            .query(
                "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'reborn_migration_state'",
                (),
            )
            .await
            .context("failed to inspect libSQL migration quarantine schema")?;
        if schema
            .next()
            .await
            .context("failed to read libSQL schema")?
            .is_none()
        {
            return Ok::<Option<MigrationStateRecord>, anyhow::Error>(None);
        }
        let mut rows = connection
            .query(
                "SELECT schema_version, migration_protocol_version, run_id, status, profile,
                        target_backend, target_locator_fingerprint, tenant_id, agent_id
                 FROM reborn_migration_state WHERE singleton = 1",
                (),
            )
            .await
            .context("failed to read libSQL migration quarantine state")?;
        let row = rows
            .next()
            .await
            .context("failed to read libSQL migration state row")?
            .context("libSQL migration quarantine table has no singleton state")?;
        let record = MigrationStateRecord {
            schema_version: row.get(0).context("invalid libSQL migration schema")?,
            migration_protocol_version: u32::try_from(
                row.get::<i64>(1)
                    .context("invalid libSQL migration protocol")?,
            )
            .context("invalid libSQL migration protocol")?,
            run_id: row.get(2).context("invalid libSQL migration run id")?,
            status: row.get(3).context("invalid libSQL migration status")?,
            profile: row.get(4).context("invalid libSQL migration profile")?,
            target_backend: row.get(5).context("invalid libSQL target backend")?,
            target_locator_fingerprint: row.get(6).context("invalid libSQL target fingerprint")?,
            tenant_id: row.get(7).context("invalid libSQL tenant id")?,
            agent_id: row.get(8).context("invalid libSQL agent id")?,
        };
        validate_state_header(
            Some(&record.schema_version),
            Some(u64::from(record.migration_protocol_version)),
        )?;
        MigrationLifecycleStatus::parse(&record.status)?;
        Ok::<Option<MigrationStateRecord>, anyhow::Error>(Some(record))
    })
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

    fn marker(release_version: &str, status: &str, binding: &CurrentTargetBinding) -> String {
        serde_json::json!({
            "schema_version": MIGRATION_STATE_MARKER_SCHEMA,
            "migration_protocol_version": COMPANION_PROTOCOL_VERSION,
            "release_version": release_version,
            "run_id": "01JTESTMIGRATIONRUN0000000000",
            "status": status,
            "profile": binding.profile,
            "target_backend": binding.target_backend,
            "target_locator_fingerprint": binding.target_locator_fingerprint,
            "tenant_id": binding.tenant_id,
            "agent_id": binding.agent_id,
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
    #[cfg(feature = "libsql")]
    fn verified_marker_without_matching_durable_state_stays_quarantined() {
        let (_tmp, context) = RebornCliContext::test_context();
        let binding = current_target_binding(&context).expect("target binding");
        let path = context
            .boot_config()
            .home()
            .path()
            .join(MIGRATION_STATE_MARKER_FILE);
        fs::create_dir_all(context.boot_config().home().path()).expect("create home");
        fs::write(&path, marker("previous-release", "verified", &binding)).expect("write marker");

        let error = ensure_activation_allowed(&context)
            .expect_err("a local marker alone must not authorize activation");
        assert!(
            error
                .to_string()
                .contains("no matching target-owned durable state")
        );
    }

    #[test]
    fn state_binding_rejects_changed_scope() {
        let binding = CurrentTargetBinding {
            profile: "local-dev".to_string(),
            target_backend: "libsql",
            target_locator_fingerprint: "target-fingerprint".to_string(),
            tenant_id: "tenant-a".to_string(),
            agent_id: "agent-a".to_string(),
        };
        let mut record: MigrationStateRecord =
            serde_json::from_str(&marker("release", "verified", &binding)).expect("marker record");
        record.tenant_id = "tenant-b".to_string();
        assert!(validate_state_binding(&record, &binding).is_err());
    }
}
