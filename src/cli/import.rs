//! `ironclaw import` — restore from `ironclaw backup` archives, plus
//! migration paths from other AI systems.
//!
//! This module hosts two unrelated import flavors that share a common
//! parent subcommand:
//!
//! - `ironclaw import backup <PATH>` — companion to `ironclaw backup`,
//!   restoring a `--quick` archive (db + config + manifest) onto the
//!   local IronClaw base dir. Always available.
//! - `ironclaw import openclaw [--path …]` — migration from OpenClaw
//!   (memory, history, settings, credentials). Gated behind the
//!   `import` feature flag because it pulls in re-embedding deps.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(feature = "import")]
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Subcommand;

#[cfg(feature = "import")]
use crate::import::ImportOptions;
#[cfg(feature = "import")]
use crate::import::openclaw::OpenClawImporter;

/// Import data into IronClaw. Currently supports restoring from a backup
/// archive (always available) and migrating from OpenClaw (feature-gated).
#[derive(Subcommand, Debug, Clone)]
pub enum ImportCommand {
    /// Import from OpenClaw (memory, history, settings, credentials)
    #[cfg(feature = "import")]
    Openclaw {
        /// Path to OpenClaw directory (default: ~/.openclaw)
        #[arg(long)]
        path: Option<PathBuf>,

        /// Dry-run mode: show what would be imported without writing
        #[arg(long)]
        dry_run: bool,

        /// Re-embed memory if dimensions don't match target provider
        #[arg(long)]
        re_embed: bool,

        /// User ID for imported data (default: 'default')
        #[arg(long)]
        user_id: Option<String>,
    },

    /// Restore an `ironclaw backup` archive (currently `--quick` mode
    /// only) onto the local IronClaw base dir. Existing files are
    /// renamed to `<path>.pre-import-<ISO8601>` before replacement so
    /// the operator can roll back manually if needed.
    Backup {
        /// Path to the backup zip (the file produced by
        /// `ironclaw backup --quick`).
        archive: PathBuf,

        /// Bypass the major-version-skew safety check. Required when
        /// the archive was produced by an `ironclaw` whose major
        /// version differs from the running binary; defaults to the
        /// safe rejection.
        #[arg(long)]
        force: bool,

        /// Validate the archive and print the restore plan, but do
        /// not modify any files.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Run an import command.
pub async fn run_import_command(
    cmd: &ImportCommand,
    config: &crate::config::Config,
) -> anyhow::Result<()> {
    match cmd {
        #[cfg(feature = "import")]
        ImportCommand::Openclaw {
            path,
            dry_run,
            re_embed,
            user_id,
        } => run_import_openclaw(config, path.clone(), *dry_run, *re_embed, user_id.clone()).await,

        ImportCommand::Backup {
            archive,
            force,
            dry_run,
        } => {
            // The backup-restore path operates on local files; it does
            // not need a live database connection from `config`. We
            // accept the param for signature symmetry with the
            // openclaw arm.
            let _ = config;
            run_import_backup(archive, *force, *dry_run).await
        }
    }
}

// =====================================================================
// `ironclaw import backup` — always-on restore for the backup format.
// =====================================================================

/// Manifest header read from `manifest.json` at the top of the zip.
/// Mirrors the shape that `ironclaw backup` writes.
#[derive(Debug, serde::Deserialize)]
struct ImportManifest {
    ironclaw_version: String,
    schema_version: i64,
    #[allow(dead_code)]
    created_at: String,
    hostname: String,
    label: Option<String>,
    mode: String,
    components: Vec<String>,
}

/// Top-level dispatch for `ironclaw import backup <PATH>`. Public so
/// integration tests can drive it without faking a `Config`.
pub async fn run_import_backup(archive: &Path, force: bool, dry_run: bool) -> Result<()> {
    if !archive.exists() {
        anyhow::bail!("archive not found at {}", archive.display());
    }

    let manifest = read_manifest_from_zip(archive)
        .with_context(|| format!("reading manifest from {}", archive.display()))?;

    validate_manifest(&manifest, force)?;

    let base = crate::bootstrap::compute_ironclaw_base_dir();
    let db_target = base.join("ironclaw.db");
    let config_target = base.join("config.toml");

    println!("ironclaw import — restore plan");
    println!("  archive:           {}", archive.display());
    println!("  ironclaw_version:  {}", manifest.ironclaw_version);
    println!("  schema_version:    {}", manifest.schema_version);
    println!("  hostname:          {}", manifest.hostname);
    if let Some(label) = manifest.label.as_deref() {
        println!("  label:             {label}");
    }
    println!("  components:        {}", manifest.components.join(","));
    println!();
    println!("  -> {}", db_target.display());
    println!("  -> {}", config_target.display());

    if dry_run {
        println!();
        println!("[DRY RUN] No files were modified.");
        return Ok(());
    }

    extract_with_rollback(archive, &db_target, &config_target)
        .with_context(|| format!("extracting {}", archive.display()))?;

    // Re-open the restored libSQL db and apply any migrations newer
    // than what the archive carried. This is what makes round-trip
    // safe across binary updates: a backup taken on schema_version=25
    // restored onto a binary that expects 26 will catch up cleanly.
    #[cfg(feature = "libsql")]
    apply_pending_migrations(&db_target).await.with_context(|| {
        format!(
            "running pending migrations on restored db {}",
            db_target.display()
        )
    })?;

    println!();
    println!("Restore complete.");
    Ok(())
}

/// Reject archives that were produced by an incompatible binary.
///
/// In this commit only `--quick` mode is restorable; later commits will
/// add `--full`. Major-version skew is rejected without `--force`
/// because the on-disk shape of the secrets store, the workspace
/// document encoding, and the WASM channel layout can change between
/// majors and we don't auto-rewrite them.
fn validate_manifest(manifest: &ImportManifest, force: bool) -> Result<()> {
    if manifest.mode != "quick" {
        anyhow::bail!(
            "archive mode is '{}'; only 'quick' is supported by this binary. \
             Re-run `ironclaw backup --quick` on the source host.",
            manifest.mode
        );
    }

    let current = env!("CARGO_PKG_VERSION");
    let current_major = major(current);
    let archive_major = major(&manifest.ironclaw_version);

    if current_major != archive_major && !force {
        anyhow::bail!(
            "archive was produced by ironclaw {}; current binary is {} (different major). \
             Cross-major-version restore is risky and refused without --force.",
            manifest.ironclaw_version,
            current
        );
    }

    Ok(())
}

/// Parse the leading numeric component of a SemVer-shaped version string.
/// Returns 0 for unparseable input — over-permissive on purpose so a
/// malformed manifest version does not crash the importer ahead of the
/// `--force` gate that the operator can always reach for.
fn major(version: &str) -> u32 {
    version
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Read and parse `manifest.json` from the top of the archive.
fn read_manifest_from_zip(archive: &Path) -> Result<ImportManifest> {
    let f = std::fs::File::open(archive)
        .with_context(|| format!("opening archive {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(f))
        .with_context(|| format!("reading zip {}", archive.display()))?;

    let mut entry = zip
        .by_name("manifest.json")
        .context("archive is missing manifest.json — not a recognized ironclaw backup")?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).context("reading manifest.json bytes")?;
    let manifest: ImportManifest =
        serde_json::from_str(&buf).context("parsing manifest.json as JSON")?;
    Ok(manifest)
}

/// Extract the archive over the targets atomically-as-possible:
///
/// 1. Stream entries to sibling staging paths.
/// 2. Move any pre-existing target out of the way (`<path>.pre-import-<ts>`).
/// 3. Rename staging paths into place. If step 3 fails partway through,
///    restore the pre-existing files.
fn extract_with_rollback(
    archive: &Path,
    db_target: &Path,
    config_target: &Path,
) -> Result<()> {
    if let Some(parent) = db_target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir {}", parent.display()))?;
        }
    }
    if let Some(parent) = config_target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir {}", parent.display()))?;
        }
    }

    let f = std::fs::File::open(archive)
        .with_context(|| format!("opening archive {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(f))
        .with_context(|| format!("reading zip {}", archive.display()))?;

    let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let db_staged = with_extension_suffix(db_target, ".restore-staged");
    let config_staged = with_extension_suffix(config_target, ".restore-staged");
    let db_backup = with_extension_suffix(db_target, &format!(".pre-import-{stamp}"));
    let config_backup = with_extension_suffix(config_target, &format!(".pre-import-{stamp}"));

    extract_entry(&mut zip, "data/ironclaw.db", &db_staged)
        .context("extracting data/ironclaw.db")?;
    extract_entry(&mut zip, "config.toml", &config_staged)
        .context("extracting config.toml")?;

    let db_existed = db_target.exists();
    let config_existed = config_target.exists();
    if db_existed {
        std::fs::rename(db_target, &db_backup).with_context(|| {
            format!(
                "moving existing db aside: {} -> {}",
                db_target.display(),
                db_backup.display()
            )
        })?;
    }
    if config_existed {
        if let Err(e) = std::fs::rename(config_target, &config_backup) {
            // Roll back the db rename to keep the on-disk state coherent.
            if db_existed {
                let _ = std::fs::rename(&db_backup, db_target);
            }
            return Err(anyhow::Error::from(e).context(format!(
                "moving existing config aside: {} -> {}",
                config_target.display(),
                config_backup.display()
            )));
        }
    }

    let promote = (|| -> std::io::Result<()> {
        std::fs::rename(&db_staged, db_target)?;
        std::fs::rename(&config_staged, config_target)?;
        Ok(())
    })();

    if let Err(e) = promote {
        // Best-effort rollback: clean up staged + restore originals.
        let _ = std::fs::remove_file(&db_staged);
        let _ = std::fs::remove_file(&config_staged);
        if db_existed {
            let _ = std::fs::rename(&db_backup, db_target);
        }
        if config_existed {
            let _ = std::fs::rename(&config_backup, config_target);
        }
        return Err(anyhow::Error::from(e).context("promoting staged files"));
    }

    if db_existed {
        println!("  pre-import db saved:     {}", db_backup.display());
    }
    if config_existed {
        println!("  pre-import config saved: {}", config_backup.display());
    }
    Ok(())
}

fn with_extension_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn extract_entry(
    zip: &mut zip::ZipArchive<io::BufReader<std::fs::File>>,
    name: &str,
    out: &Path,
) -> Result<()> {
    let mut entry = zip
        .by_name(name)
        .with_context(|| format!("archive is missing entry '{name}'"))?;
    let f = std::fs::File::create(out)
        .with_context(|| format!("creating {}", out.display()))?;
    let mut writer = io::BufWriter::with_capacity(64 * 1024, f);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = entry.read(&mut buf).context("reading zip entry")?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).context("writing staged file")?;
    }
    writer.flush().context("flushing staged file")?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn apply_pending_migrations(db_path: &Path) -> Result<()> {
    // `run_migrations` is a method on the `Database` supertrait —
    // bring it into scope at the call site so the trait method
    // resolves on the concrete `LibSqlBackend`.
    use crate::db::Database;
    let backend = crate::db::libsql::LibSqlBackend::new_local(db_path)
        .await
        .with_context(|| format!("opening restored libSQL db at {}", db_path.display()))?;
    backend
        .run_migrations()
        .await
        .context("running pending migrations on restored db")?;
    Ok(())
}

// =====================================================================
// `ironclaw import openclaw` — feature-gated migration from OpenClaw.
// =====================================================================

/// Run the OpenClaw import.
#[cfg(feature = "import")]
async fn run_import_openclaw(
    config: &crate::config::Config,
    openclaw_path: Option<PathBuf>,
    dry_run: bool,
    re_embed: bool,
    user_id: Option<String>,
) -> anyhow::Result<()> {
    use secrecy::SecretString;

    // Determine OpenClaw path
    let openclaw_path = if let Some(path) = openclaw_path {
        path
    } else if let Some(path) = OpenClawImporter::detect() {
        path
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".openclaw")
    };

    let user_id = user_id.unwrap_or_else(|| "default".to_string());

    println!("🔍 OpenClaw Import");
    println!("  Path: {}", openclaw_path.display());
    println!("  User: {}", user_id);
    if dry_run {
        println!("  Mode: DRY RUN (no data will be written)");
    }
    println!();

    // Initialize database
    let db = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to initialize database: {}", e))?;

    // Initialize secrets store with master key from env or keychain
    let secrets_crypto = if let Ok(master_key_hex) = std::env::var("SECRETS_MASTER_KEY") {
        Arc::new(
            crate::secrets::SecretsCrypto::new(SecretString::from(master_key_hex))
                .map_err(|e| anyhow::anyhow!("Failed to initialize secrets: {}", e))?,
        )
    } else {
        match crate::secrets::keychain::get_master_key().await {
            Ok(key_bytes) => {
                let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                Arc::new(
                    crate::secrets::SecretsCrypto::new(SecretString::from(key_hex))
                        .map_err(|e| anyhow::anyhow!("Failed to initialize secrets: {}", e))?,
                )
            }
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "No secrets master key found. Set SECRETS_MASTER_KEY env var or run 'ironclaw onboard' first."
                ));
            }
        }
    };

    let secrets: Arc<dyn crate::secrets::SecretsStore> = Arc::new(
        crate::secrets::InMemorySecretsStore::new(secrets_crypto.clone()),
    );

    // Initialize workspace
    let workspace = crate::workspace::Workspace::new_with_db(user_id.clone(), db.clone());

    let opts = ImportOptions {
        openclaw_path,
        dry_run,
        re_embed,
        user_id,
    };

    let importer = OpenClawImporter::new(db, workspace, secrets, opts);
    let stats = importer.import().await?;

    // Print results
    println!("Import Complete");
    println!();
    println!("Summary:");
    println!("  Documents:    {}", stats.documents);
    println!("  Chunks:       {}", stats.chunks);
    println!("  Conversations: {}", stats.conversations);
    println!("  Messages:     {}", stats.messages);
    println!("  Settings:     {}", stats.settings);
    println!("  Secrets:      {}", stats.secrets);
    if stats.skipped > 0 {
        println!("  Skipped:      {}", stats.skipped);
    }
    if stats.re_embed_queued > 0 {
        println!("  Re-embed queued: {}", stats.re_embed_queued);
    }
    println!();
    println!("Total imported: {}", stats.total_imported());

    if dry_run {
        println!();
        println!("[DRY RUN] No data was written.");
    }

    Ok(())
}

// =====================================================================
// Tests for the backup-restore path. The openclaw path has its own
// integration tests under `tests/openclaw_*` that we don't duplicate here.
// =====================================================================

#[cfg(all(test, feature = "libsql"))]
mod tests {
    use super::*;

    #[test]
    fn major_parses_leading_component() {
        assert_eq!(major("0.27.0"), 0);
        assert_eq!(major("1.2.3"), 1);
        assert_eq!(major("12"), 12);
        assert_eq!(major("garbage"), 0);
        assert_eq!(major(""), 0);
    }

    #[test]
    fn validate_rejects_non_quick_mode() {
        let m = ImportManifest {
            ironclaw_version: env!("CARGO_PKG_VERSION").to_string(),
            schema_version: 25,
            created_at: "2026-05-01T00:00:00Z".into(),
            hostname: "h".into(),
            label: None,
            mode: "full".into(),
            components: vec!["db".into(), "config".into(), "skills".into()],
        };
        let err = validate_manifest(&m, false).unwrap_err().to_string();
        assert!(err.contains("'full'"), "{err}");
    }

    #[test]
    fn validate_rejects_cross_major_without_force() {
        let m = ImportManifest {
            // Deliberately use a major that won't match CARGO_PKG_VERSION.
            ironclaw_version: "99.0.0".into(),
            schema_version: 25,
            created_at: "2026-05-01T00:00:00Z".into(),
            hostname: "h".into(),
            label: None,
            mode: "quick".into(),
            components: vec!["db".into(), "config".into()],
        };
        let err = validate_manifest(&m, false).unwrap_err().to_string();
        assert!(err.contains("--force"), "{err}");
        // With --force, the same manifest is accepted.
        validate_manifest(&m, true).expect("force overrides version check");
    }

    #[test]
    fn validate_accepts_same_major() {
        let m = ImportManifest {
            ironclaw_version: env!("CARGO_PKG_VERSION").to_string(),
            schema_version: 25,
            created_at: "2026-05-01T00:00:00Z".into(),
            hostname: "h".into(),
            label: None,
            mode: "quick".into(),
            components: vec!["db".into(), "config".into()],
        };
        validate_manifest(&m, false).expect("same-major archive is valid");
    }
}
