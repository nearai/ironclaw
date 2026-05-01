//! `ironclaw backup` — portable state snapshots.
//!
//! First commit only ships `--quick`: bundles the libSQL database and the
//! TOML config into a single zip with a JSON manifest. No secrets, no skill
//! bundles, no installed extensions; those land in `--full` (separate PR),
//! together with `ironclaw import` for restore.
//!
//! ## Quick mode contents
//!
//! ```text
//! manifest.json          (JSON, see [`Manifest`])
//! data/ironclaw.db       (libSQL database, WAL-checkpointed)
//! config.toml            (the active TOML config, if present)
//! ```
//!
//! The db is checkpointed with `PRAGMA wal_checkpoint(TRUNCATE)` before the
//! file copy so the snapshot is self-contained without `-wal` / `-shm`
//! sidecar files.

use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;

/// Args for `ironclaw backup`.
#[derive(Args, Debug, Clone)]
pub struct BackupArgs {
    /// Quick mode: db + active config.toml (no secrets, no skill bundles, no
    /// installed extensions). The default and only mode supported in this
    /// commit. Reserved as a flag so `--full` can be added without breaking
    /// existing scripts.
    #[arg(long)]
    pub quick: bool,

    /// Output path for the zip. Defaults to
    /// `~/ironclaw-backup-<ISO8601>.zip`.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Optional human-readable label baked into the manifest (e.g. the
    /// migration source: `pre-crab-shack`).
    #[arg(long)]
    pub label: Option<String>,
}

/// JSON manifest stored at the top of every backup archive.
#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub ironclaw_version: String,
    pub schema_version: i64,
    pub created_at: String,
    pub hostname: String,
    pub label: Option<String>,
    pub mode: String,
    pub components: Vec<String>,
}

/// Source paths the `--quick` mode tries to bundle.
#[derive(Debug, Clone)]
struct QuickSources {
    db_path: PathBuf,
    config_path: PathBuf,
}

impl QuickSources {
    /// Resolve from environment + IronClaw defaults. Falls back to
    /// `~/.ironclaw/{ironclaw.db,config.toml}` when overrides are unset.
    ///
    /// Honors `LIBSQL_PATH` and `--config <PATH>` (passed in by caller).
    fn resolve(config_override: Option<&Path>) -> Self {
        let db_path = std::env::var_os("LIBSQL_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(crate::config::default_libsql_path);

        // Use compute_ironclaw_base_dir() (env-reading) instead of
        // Settings::default_toml_path() (LazyLock-cached) so a process-level
        // IRONCLAW_BASE_DIR override is honored at command time. Matches the
        // env-reading behavior of LIBSQL_PATH above for consistency.
        let config_path = config_override.map(PathBuf::from).unwrap_or_else(|| {
            crate::bootstrap::compute_ironclaw_base_dir().join("config.toml")
        });

        Self {
            db_path,
            config_path,
        }
    }
}

/// Top-level entry point wired into the CLI dispatch in `src/main.rs`.
pub async fn run_backup_command(
    args: BackupArgs,
    config_path_override: Option<&Path>,
) -> Result<()> {
    if !args.quick {
        anyhow::bail!(
            "this commit only supports `ironclaw backup --quick`. \
             Full-mode bundles (skills, installed extensions, secrets) \
             are tracked as a follow-up."
        );
    }

    let sources = QuickSources::resolve(config_path_override);
    let output_path = match args.output {
        Some(p) => p,
        None => default_output_path()?,
    };

    if !sources.db_path.exists() {
        anyhow::bail!(
            "database not found at {} — cannot back up. \
             Run `ironclaw onboard` first or set LIBSQL_PATH.",
            sources.db_path.display()
        );
    }
    if !sources.config_path.exists() {
        anyhow::bail!(
            "config file not found at {} — cannot back up. \
             Run `ironclaw config init` first or pass --config <PATH>.",
            sources.config_path.display()
        );
    }

    // WAL checkpoint is required for self-containment: without it the .db
    // file may be missing recently committed pages still parked in the
    // -wal sidecar. Fail loudly rather than silently produce a stale
    // snapshot.
    checkpoint_wal(&sources.db_path)
        .await
        .with_context(|| format!("WAL checkpoint failed on {}", sources.db_path.display()))?;

    let schema_version = read_schema_version(&sources.db_path)
        .await
        .with_context(|| {
            format!(
                "reading schema_version from {} (latest applied migration)",
                sources.db_path.display()
            )
        })?;

    let manifest = Manifest {
        ironclaw_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version,
        created_at: Utc::now().to_rfc3339(),
        hostname: hostname(),
        label: args.label,
        mode: "quick".into(),
        components: vec!["db".into(), "config".into()],
    };

    write_quick_archive(&output_path, &sources, &manifest)
        .with_context(|| format!("failed to write backup archive {}", output_path.display()))?;

    println!("Backup written: {}", output_path.display());
    println!(
        "  ironclaw_version: {}\n  schema_version: {}\n  components: {}",
        manifest.ironclaw_version,
        manifest.schema_version,
        manifest.components.join(",")
    );
    Ok(())
}

/// Compose `~/ironclaw-backup-<ISO8601>.zip`.
fn default_output_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    // ISO8601, but with `:` swapped to `-` to keep the filename portable on
    // case-insensitive filesystems (cosmetic; full ISO8601 timestamp is in
    // the manifest).
    let stamp = Utc::now()
        .format("%Y-%m-%dT%H-%M-%SZ")
        .to_string();
    Ok(home.join(format!("ironclaw-backup-{stamp}.zip")))
}

/// Run `PRAGMA wal_checkpoint(TRUNCATE)` against the libSQL DB so the
/// snapshot is self-contained.
#[cfg(feature = "libsql")]
async fn checkpoint_wal(db_path: &Path) -> Result<()> {
    let db = libsql::Builder::new_local(db_path)
        .build()
        .await
        .with_context(|| format!("opening libSQL db at {}", db_path.display()))?;
    let conn = db.connect().context("opening libSQL connection")?;
    // PRAGMA wal_checkpoint(TRUNCATE) returns a 3-column row
    // (busy, log, checkpointed). libSQL's `execute()` rejects statements
    // that produce rows with "Execute returned rows", so we use `query()`
    // and let the resulting Rows handle drop without iteration —
    // execution still completes (the checkpoint runs eagerly during the
    // call, not lazily during row iteration).
    conn.query("PRAGMA wal_checkpoint(TRUNCATE)", ())
        .await
        .context("PRAGMA wal_checkpoint(TRUNCATE) failed")?;
    Ok(())
}

#[cfg(not(feature = "libsql"))]
async fn checkpoint_wal(_db_path: &Path) -> Result<()> {
    // libSQL isn't compiled in; the backup still works, but the on-disk db
    // may have a sidecar -wal/-shm if another process is writing. We skip
    // silently rather than fail: the command must remain useful in
    // postgres-only builds.
    Ok(())
}

/// Read `MAX(version)` from the libSQL `_migrations` table.
#[cfg(feature = "libsql")]
async fn read_schema_version(db_path: &Path) -> Result<i64> {
    let db = libsql::Builder::new_local(db_path)
        .build()
        .await
        .with_context(|| format!("opening libSQL db at {}", db_path.display()))?;
    let conn = db.connect().context("opening libSQL connection")?;
    let mut rows = conn
        .query("SELECT COALESCE(MAX(version), 0) FROM _migrations", ())
        .await
        .context("querying _migrations")?;
    let row = rows
        .next()
        .await
        .context("reading _migrations result")?
        .context("_migrations returned no row")?;
    Ok(row.get::<i64>(0).unwrap_or(0))
}

#[cfg(not(feature = "libsql"))]
async fn read_schema_version(_db_path: &Path) -> Result<i64> {
    Ok(0)
}

/// Hostname, falling back to `unknown` rather than failing the backup.
fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Stream-write the zip to `output`. Manifest first, then db, then config.
///
/// We write to `<output>.tmp` and rename atomically on success so a crashed
/// backup never leaves a half-written archive at the canonical path.
fn write_quick_archive(
    output: &Path,
    sources: &QuickSources,
    manifest: &Manifest,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir {}", parent.display()))?;
        }
    }

    let tmp = with_extension_suffix(output, ".tmp");
    {
        let file = File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        // 1. manifest.json — small, written first so consumers can validate
        //    before paying the deflate cost on the db.
        zip.start_file("manifest.json", opts)
            .context("zip: starting manifest.json")?;
        let manifest_bytes =
            serde_json::to_vec_pretty(manifest).context("serializing manifest")?;
        zip.write_all(&manifest_bytes)
            .context("zip: writing manifest.json")?;

        // 2. data/ironclaw.db — streamed, never buffered.
        zip.start_file("data/ironclaw.db", opts)
            .context("zip: starting data/ironclaw.db")?;
        stream_file(&sources.db_path, &mut zip)
            .with_context(|| format!("copying {} into zip", sources.db_path.display()))?;

        // 3. config.toml — caller already verified it exists, so a missing
        //    file here is a TOCTOU (someone removed it during the backup);
        //    surface the IO error rather than skip silently.
        zip.start_file("config.toml", opts)
            .context("zip: starting config.toml")?;
        stream_file(&sources.config_path, &mut zip)
            .with_context(|| format!("copying {} into zip", sources.config_path.display()))?;

        zip.finish().context("finalizing zip")?;
    }

    fs::rename(&tmp, output)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), output.display()))?;
    Ok(())
}

/// Append a suffix to a path's filename (`/foo/bar.zip` + `.tmp` =
/// `/foo/bar.zip.tmp`).
fn with_extension_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Stream-copy a file into any `Write`. 64 KiB chunks; never buffers the
/// whole file in memory.
fn stream_file<W: Write>(src: &Path, dst: &mut W) -> io::Result<()> {
    let f = File::open(src)?;
    let mut reader = BufReader::with_capacity(64 * 1024, f);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_serializes_quick_mode_with_required_fields() {
        let m = Manifest {
            ironclaw_version: "0.1.2".into(),
            schema_version: 25,
            created_at: "2026-05-01T00:00:00+00:00".into(),
            hostname: "test-host".into(),
            label: Some("pre-crab-shack".into()),
            mode: "quick".into(),
            components: vec!["db".into(), "config".into()],
        };
        let v: serde_json::Value = serde_json::from_slice(
            &serde_json::to_vec(&m).expect("serialize"),
        )
        .expect("parse roundtrip");
        assert_eq!(v["mode"], "quick");
        assert_eq!(v["schema_version"], 25);
        assert_eq!(v["ironclaw_version"], "0.1.2");
        assert_eq!(v["label"], "pre-crab-shack");
        assert_eq!(v["components"], serde_json::json!(["db", "config"]));
    }

    #[test]
    fn with_extension_suffix_appends() {
        let p = PathBuf::from("/tmp/foo.zip");
        assert_eq!(
            with_extension_suffix(&p, ".tmp"),
            PathBuf::from("/tmp/foo.zip.tmp")
        );
    }

    #[test]
    fn default_output_path_uses_iso8601_stamp() {
        let p = default_output_path().expect("default path");
        let name = p.file_name().expect("filename").to_string_lossy().into_owned();
        assert!(
            name.starts_with("ironclaw-backup-") && name.ends_with(".zip"),
            "unexpected default name: {name}"
        );
        // Stamp: ironclaw-backup-YYYY-MM-DDTHH-MM-SSZ.zip = 16 + 20 + 4 = 40
        assert_eq!(name.len(), 40, "stamp width drifted: {name}");
    }
}
