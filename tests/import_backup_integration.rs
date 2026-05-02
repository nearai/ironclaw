//! Integration tests for `ironclaw import backup`.
//!
//! Drives the public entry point against synthetic backup archives that
//! match the on-the-wire shape `ironclaw backup --quick` produces:
//!   manifest.json + data/ironclaw.db + config.toml.
//!
//! Process-level state (env vars, the `IRONCLAW_BASE_DIR` LazyLock) is
//! shared across all tests in this binary, so we serialize via
//! `ENV_LOCK` and prefer test-scoped temp dirs piped through env vars.

#![cfg(feature = "libsql")]

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use ironclaw::cli::run_import_backup;
// `Database` is brought in for the `run_migrations` trait method on
// `LibSqlBackend`, which we use both to seed a real payload db inside
// the synthetic archive and to reopen the restored snapshot for
// content assertions.
use ironclaw::db::Database;

/// Serialize all tests that touch process env. Tests that do not touch
/// env still take the lock to avoid interleaving with ones that do —
/// cheaper than reasoning about which arms touch which keys.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// RAII env override. Restores prior values on drop, panic-safe.
struct EnvGuard {
    entries: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn set(mut self, key: &'static str, val: impl AsRef<Path>) -> Self {
        let prior = std::env::var_os(key);
        // SAFETY: tests serialize via ENV_LOCK; no other thread
        // mutates the environment concurrently.
        unsafe {
            std::env::set_var(key, val.as_ref());
        }
        self.entries.push((key, prior));
        self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prior) in self.entries.drain(..) {
            // SAFETY: same lock contract as `set`. Drop runs even on
            // panic so the next test starts from a clean baseline.
            unsafe {
                match prior {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

/// Build a real (migrated) libSQL db at `db_path`. Used as the payload
/// inside the synthetic archive.
async fn build_real_libsql_db(db_path: &Path) {
    let backend = ironclaw::db::libsql::LibSqlBackend::new_local(db_path)
        .await
        .expect("LibSqlBackend::new_local");
    backend
        .run_migrations()
        .await
        .expect("run_migrations on payload db");
}

/// Compose a synthetic backup archive at `archive_out` containing:
///   - `manifest.json` (top of zip)
///   - `data/ironclaw.db` (a real, migrated libSQL file)
///   - `config.toml` (placeholder content)
async fn build_archive(
    archive_out: &Path,
    payload_db: &Path,
    payload_config: &str,
    mode: &str,
    version: &str,
) {
    build_real_libsql_db(payload_db).await;

    let f = fs::File::create(archive_out).expect("create archive");
    let mut zip = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 1. manifest.json — pinned at index 0 to match the backup writer.
    zip.start_file("manifest.json", opts).expect("start manifest.json");
    let manifest = serde_json::json!({
        "ironclaw_version": version,
        "schema_version": 25,
        "created_at": Utc::now().to_rfc3339(),
        "hostname": "test-host",
        "label": "import-integration",
        "mode": mode,
        "components": ["db", "config"],
    });
    zip.write_all(
        serde_json::to_string_pretty(&manifest)
            .expect("manifest serialize")
            .as_bytes(),
    )
    .expect("write manifest");

    // 2. data/ironclaw.db
    zip.start_file("data/ironclaw.db", opts).expect("start db");
    let db_bytes = fs::read(payload_db).expect("read payload db");
    zip.write_all(&db_bytes).expect("write db bytes");

    // 3. config.toml
    zip.start_file("config.toml", opts).expect("start config.toml");
    zip.write_all(payload_config.as_bytes()).expect("write config");

    zip.finish().expect("finalize zip");
}

#[tokio::test]
async fn dry_run_validates_and_does_not_write() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    let archive = tmp.path().join("snap.zip");
    let payload_db = tmp.path().join("payload.db");

    build_archive(
        &archive,
        &payload_db,
        "[agent]\nname = \"from-archive\"\n",
        "quick",
        env!("CARGO_PKG_VERSION"),
    )
    .await;

    let _guard = EnvGuard::new().set("IRONCLAW_BASE_DIR", &base);

    run_import_backup(&archive, /* force */ false, /* dry_run */ true)
        .await
        .expect("dry run should succeed for a valid same-major archive");

    // No write should land at the canonical paths under the base dir.
    assert!(!base.join("ironclaw.db").exists(), "dry run wrote db");
    assert!(
        !base.join("config.toml").exists(),
        "dry run wrote config"
    );
}

#[tokio::test]
async fn restore_lands_db_and_config_when_target_is_empty() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    let archive = tmp.path().join("snap.zip");
    let payload_db = tmp.path().join("payload.db");

    let archive_config = "[agent]\nname = \"from-archive\"\n";
    build_archive(
        &archive,
        &payload_db,
        archive_config,
        "quick",
        env!("CARGO_PKG_VERSION"),
    )
    .await;

    let _guard = EnvGuard::new().set("IRONCLAW_BASE_DIR", &base);

    run_import_backup(&archive, false, false)
        .await
        .expect("restore should succeed");

    let restored_db = base.join("ironclaw.db");
    let restored_cfg = base.join("config.toml");
    assert!(restored_db.exists(), "restored db is missing at expected path");
    assert!(
        restored_cfg.exists(),
        "restored config is missing at expected path"
    );

    // Config content survives the round trip verbatim.
    let got = fs::read_to_string(&restored_cfg).unwrap();
    assert_eq!(got, archive_config);

    // The restored db has the seeded migrations table from
    // `run_migrations` baked in. Sanity-check by reopening.
    let backend = ironclaw::db::libsql::LibSqlBackend::new_local(&restored_db)
        .await
        .expect("open restored db");
    let conn = backend.connect().await.expect("connect restored");
    let mut rows = conn
        .query("SELECT COUNT(*) FROM _migrations", ())
        .await
        .expect("read _migrations");
    let row = rows
        .next()
        .await
        .expect("rows.next")
        .expect("at least one row");
    let count: i64 = row.get(0).expect("count col");
    assert!(count > 0, "restored db has no _migrations rows");

    // No pre-import sidecar files exist when there was nothing to
    // displace.
    let sidecars: Vec<PathBuf> = fs::read_dir(&base)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(".pre-import-"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        sidecars.is_empty(),
        "unexpected pre-import sidecars: {sidecars:?}"
    );
}

#[tokio::test]
async fn restore_renames_existing_files_aside() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    let archive = tmp.path().join("snap.zip");
    let payload_db = tmp.path().join("payload.db");

    build_archive(
        &archive,
        &payload_db,
        "from-archive",
        "quick",
        env!("CARGO_PKG_VERSION"),
    )
    .await;

    // Pre-create files at the targets so the importer must move them
    // aside.
    fs::write(base.join("ironclaw.db"), b"PRIOR DB CONTENT").unwrap();
    fs::write(base.join("config.toml"), b"PRIOR CONFIG CONTENT").unwrap();

    let _guard = EnvGuard::new().set("IRONCLAW_BASE_DIR", &base);

    run_import_backup(&archive, false, false)
        .await
        .expect("restore over existing files should succeed");

    // Canonical paths now hold the archive content.
    assert_eq!(
        fs::read_to_string(base.join("config.toml")).unwrap(),
        "from-archive"
    );

    // Pre-import sidecars exist with the original content.
    let entries: Vec<_> = fs::read_dir(&base)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    let pre_db_sidecar = entries
        .iter()
        .find(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("ironclaw.db.pre-import-")
        })
        .expect("db pre-import sidecar must exist");
    let pre_cfg_sidecar = entries
        .iter()
        .find(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("config.toml.pre-import-")
        })
        .expect("config pre-import sidecar must exist");

    assert_eq!(fs::read(pre_db_sidecar).unwrap(), b"PRIOR DB CONTENT");
    assert_eq!(
        fs::read(pre_cfg_sidecar).unwrap(),
        b"PRIOR CONFIG CONTENT"
    );
}

#[tokio::test]
async fn rejects_cross_major_without_force() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    let archive = tmp.path().join("snap.zip");
    let payload_db = tmp.path().join("payload.db");

    // Force the archive to claim a major-version that cannot match the
    // running binary regardless of CARGO_PKG_VERSION drift.
    build_archive(&archive, &payload_db, "irrelevant", "quick", "99.0.0").await;

    let _guard = EnvGuard::new().set("IRONCLAW_BASE_DIR", &base);

    let err = run_import_backup(&archive, /* force */ false, false)
        .await
        .expect_err("must refuse cross-major restore without --force");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("--force"),
        "error must mention --force escape hatch: {msg}"
    );

    // No files were written.
    assert!(!base.join("ironclaw.db").exists());
    assert!(!base.join("config.toml").exists());

    // With --force, the same archive is accepted.
    run_import_backup(&archive, /* force */ true, false)
        .await
        .expect("--force overrides cross-major check");
    assert!(base.join("ironclaw.db").exists());
    assert!(base.join("config.toml").exists());
}

#[tokio::test]
async fn rejects_non_quick_mode_archive() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    let archive = tmp.path().join("snap.zip");
    let payload_db = tmp.path().join("payload.db");

    build_archive(
        &archive,
        &payload_db,
        "irrelevant",
        "full",
        env!("CARGO_PKG_VERSION"),
    )
    .await;

    let _guard = EnvGuard::new().set("IRONCLAW_BASE_DIR", &base);

    let err = run_import_backup(&archive, false, false)
        .await
        .expect_err("must refuse non-quick archive in this commit");
    let msg = format!("{err:#}");
    assert!(msg.contains("'full'") || msg.contains("only 'quick'"), "{msg}");
}

#[tokio::test]
async fn rejects_archive_missing_required_entries() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let archive = tmp.path().join("incomplete.zip");

    // Build a zip that only has manifest.json — no db, no config.
    let f = fs::File::create(&archive).expect("create archive");
    let mut zip = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("manifest.json", opts).unwrap();
    let m = serde_json::json!({
        "ironclaw_version": env!("CARGO_PKG_VERSION"),
        "schema_version": 25,
        "created_at": Utc::now().to_rfc3339(),
        "hostname": "h",
        "label": null,
        "mode": "quick",
        "components": ["db", "config"],
    });
    zip.write_all(serde_json::to_string(&m).unwrap().as_bytes())
        .unwrap();
    zip.finish().unwrap();

    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    let _guard = EnvGuard::new().set("IRONCLAW_BASE_DIR", &base);

    let err = run_import_backup(&archive, false, false)
        .await
        .expect_err("must refuse archive missing data/ironclaw.db");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("data/ironclaw.db") || msg.contains("missing entry"),
        "error must mention the missing entry: {msg}"
    );
}
