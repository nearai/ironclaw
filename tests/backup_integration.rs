//! End-to-end test for `ironclaw backup --quick`.
//!
//! Verifies the three contractual properties from the spec:
//! 1. The command produces a valid zip at the expected path.
//! 2. The zip contains exactly `manifest.json`, `data/ironclaw.db`,
//!    `config.toml`.
//! 3. The manifest fields are populated (version, schema_version,
//!    mode = "quick", created_at parses as ISO8601).
//!
//! These tests do NOT require the full app: they exercise the public API
//! exposed by `ironclaw::cli::backup`.

#![cfg(feature = "libsql")]

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::DateTime;
use ironclaw::cli::backup::{BackupArgs, run_backup_command};

/// Serializes env-var manipulation across the three integration tests in this
/// binary. cargo runs tests inside a single binary in parallel by default, so
/// without this mutex one test could see another's HOME / IRONCLAW_BASE_DIR.
/// Using a `Mutex<()>` (no `parking_lot` dep) keeps the test runtime simple.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Spin up a fake `~/.ironclaw` rooted at `tempdir` so we never touch the
/// real user state. Drops the env var on `Drop`.
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(key: &'static str, val: &std::path::Path) -> Self {
        // Safety: tests run in a single thread inside one binary, but the
        // CLI uses a `LazyLock` so we still set this *before* the first
        // `ironclaw_base_dir()` call. Each test is its own integration
        // binary anyway.
        unsafe { std::env::set_var(key, val); }
        Self { keys: vec![key] }
    }
    fn add(mut self, key: &'static str, val: &std::path::Path) -> Self {
        unsafe { std::env::set_var(key, val); }
        self.keys.push(key);
        self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for k in &self.keys {
            unsafe { std::env::remove_var(k); }
        }
    }
}

/// Write a minimal libSQL DB at `path` with the `_migrations` row IronClaw
/// uses to track schema_version. Avoids the heavy backend setup so each
/// integration test runs in <1s.
async fn make_fake_db(path: &std::path::Path) {
    let db = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("build libsql db");
    let conn = db.connect().expect("connect libsql");
    conn.execute(
        "CREATE TABLE _migrations (version INTEGER PRIMARY KEY, name TEXT, applied_at TEXT)",
        (),
    )
    .await
    .expect("create _migrations");
    conn.execute(
        "INSERT INTO _migrations(version, name, applied_at) VALUES (?, ?, datetime('now'))",
        libsql::params![25_i64, "test_v25"],
    )
    .await
    .expect("seed migration");
}

#[tokio::test]
async fn quick_backup_produces_zip_with_expected_entries_and_manifest() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    let db_path = base.join("ironclaw.db");
    let cfg_path = base.join("config.toml");
    let cfg_contents = "[agent]\nname = \"test-agent\"\n";
    fs::write(&cfg_path, cfg_contents).unwrap();
    make_fake_db(&db_path).await;

    let _guard = EnvGuard::set("IRONCLAW_BASE_DIR", &base)
        .add("LIBSQL_PATH", &db_path)
        // Avoid spurious hostname differences on CI runners.
        .add("HOSTNAME", std::path::Path::new("ironclaw-test"));

    let out = tmp.path().join("snap.zip");
    let args = BackupArgs {
        quick: true,
        output: Some(out.clone()),
        label: Some("integration-test".into()),
    };

    run_backup_command(args, None)
        .await
        .expect("backup --quick should succeed");

    // (a) zip exists and is non-empty.
    let meta = fs::metadata(&out).expect("zip exists");
    assert!(meta.len() > 0, "zip is empty");

    // (b) zip contains exactly manifest.json, data/ironclaw.db, config.toml.
    let f = fs::File::open(&out).expect("open zip");
    let mut zip = zip::ZipArchive::new(f).expect("read zip");
    let mut names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).expect("entry").name().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "config.toml".to_string(),
            "data/ironclaw.db".to_string(),
            "manifest.json".to_string(),
        ],
        "unexpected zip entry list"
    );

    // (c) manifest fields populated.
    let mut manifest_file = zip.by_name("manifest.json").expect("manifest.json");
    let mut buf = String::new();
    manifest_file.read_to_string(&mut buf).expect("read manifest");
    drop(manifest_file);
    let v: serde_json::Value = serde_json::from_str(&buf).expect("manifest json");
    assert_eq!(v["mode"], "quick");
    assert_eq!(v["label"], "integration-test");
    assert_eq!(
        v["ironclaw_version"].as_str().unwrap(),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        v["schema_version"].as_i64().expect("schema_version int"),
        25
    );
    let created_at = v["created_at"].as_str().expect("created_at str");
    DateTime::parse_from_rfc3339(created_at)
        .unwrap_or_else(|e| panic!("created_at is not ISO8601: {created_at} ({e})"));
    assert_eq!(
        v["components"]
            .as_array()
            .expect("components array")
            .len(),
        2
    );

    // Bonus invariant: the db blob in the zip matches the on-disk db.
    let mut db_in_zip = zip.by_name("data/ironclaw.db").expect("db entry");
    let mut zip_bytes = Vec::new();
    db_in_zip.read_to_end(&mut zip_bytes).unwrap();
    drop(db_in_zip);
    let on_disk = fs::read(&db_path).expect("read on-disk db");
    assert_eq!(
        zip_bytes, on_disk,
        "db blob in zip should match the on-disk db file"
    );

    // Config preserved verbatim.
    let mut cfg_in_zip = zip.by_name("config.toml").expect("config entry");
    let mut got = String::new();
    cfg_in_zip.read_to_string(&mut got).unwrap();
    assert_eq!(got, cfg_contents);
}

#[tokio::test]
async fn quick_backup_default_output_path_uses_iso8601() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    let db_path = base.join("ironclaw.db");
    fs::write(&base.join("config.toml"), "").unwrap();
    make_fake_db(&db_path).await;

    // Pin HOME so the default-path computation lands in our temp dir.
    let _guard = EnvGuard::set("IRONCLAW_BASE_DIR", &base)
        .add("LIBSQL_PATH", &db_path)
        .add("HOME", tmp.path());

    let args = BackupArgs {
        quick: true,
        output: None,
        label: None,
    };

    run_backup_command(args, None)
        .await
        .expect("backup --quick should succeed");

    // Find the auto-named backup in $HOME.
    let entries: Vec<PathBuf> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.starts_with("ironclaw-backup-") && n.ends_with(".zip"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one default-named backup, got {entries:?}"
    );
}

#[tokio::test]
async fn quick_backup_fails_when_db_missing() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("ironclaw_base");
    fs::create_dir_all(&base).unwrap();
    // No db file written.

    let _guard = EnvGuard::set("IRONCLAW_BASE_DIR", &base)
        .add("LIBSQL_PATH", &base.join("ironclaw.db"));

    let args = BackupArgs {
        quick: true,
        output: Some(tmp.path().join("snap.zip")),
        label: None,
    };

    let err = run_backup_command(args, None)
        .await
        .expect_err("backup should fail when db is missing");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("database not found"),
        "unexpected error: {msg}"
    );
}
