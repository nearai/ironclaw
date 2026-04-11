//! Caller-level tests for version tracking and downgrade detection (PR #2314).
//!
//! These tests drive `crate::version::run_startup_version_check_with` — the
//! exact function that `AppBuilder::init_database()` calls during startup —
//! against a real in-memory libSQL backend. This exercises the full wiring
//! chain (startup helper -> settings store persistence -> transition match
//! arms -> tracing), not just the pure helper `compare_semver`, per the
//! "Test Through the Caller, Not Just the Helper" rule in
//! `.claude/rules/testing.md`.
//!
//! The transition matrix covered is:
//!   - first boot    (no persisted version) -> Fresh
//!   - same version restart                  -> Unchanged
//!   - upgrade (0.23.0 -> 0.24.0)             -> Upgraded { previous }
//!   - downgrade (0.24.0 -> 0.23.0)           -> Downgraded { previous }
//!
//! Persistence is asserted across each call so we also verify the side
//! effect the wiring relies on: after every boot, the DB contains the
//! current-version string, not the old one.
//!
//! **Backends:** Only libSQL is covered here because the version store is
//! dual-backend at the trait level (`SettingsStore`) and libSQL is
//! available in default features. The Postgres implementation delegates to
//! the same trait method through `check_and_record_version_with`, so this
//! provides trait-level coverage; a dedicated Postgres integration test can
//! be added if we ever observe a dialect-specific bug.

use ironclaw::db::Database;
use ironclaw::db::SettingsStore;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::version::{VersionTransition, read_previous_version, run_startup_version_check_with};
use tempfile::TempDir;

const SYSTEM_USER: &str = "system";
const VERSION_KEY: &str = "ironclaw.version";

/// Create a fresh libSQL backend (local file) with migrations applied.
///
/// We use `new_local` instead of `new_memory` because libSQL in-memory
/// databases do not share state between the fresh connections that
/// `LibSqlBackend::connect()` creates per-operation. A tempfile gives us
/// real cross-connection persistence matching production behaviour.
async fn fresh_backend() -> (LibSqlBackend, TempDir) {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let path = tmp.path().join("version-test.db");
    let backend = LibSqlBackend::new_local(&path)
        .await
        .expect("failed to create libSQL backend");
    backend
        .run_migrations()
        .await
        .expect("failed to run libSQL migrations");
    (backend, tmp)
}

/// Assert the DB currently reports the given persisted version string.
async fn assert_persisted_version(backend: &LibSqlBackend, expected: &str) {
    let stored = backend
        .get_setting(SYSTEM_USER, VERSION_KEY)
        .await
        .expect("get_setting failed")
        .expect("version setting missing");
    assert_eq!(
        stored.as_str(),
        Some(expected),
        "persisted version should be {expected}, was {stored:?}"
    );
}

#[tokio::test]
async fn startup_version_first_boot_is_fresh_and_persists() {
    let (backend, _tmp) = fresh_backend().await;

    // Sanity: nothing persisted yet. read_previous_version is what the
    // gateway status handler uses to surface the pre-boot version.
    let pre = read_previous_version(&backend)
        .await
        .expect("read_previous_version failed");
    assert!(
        pre.is_none(),
        "expected no persisted version on a fresh DB, got {pre:?}"
    );

    let transition = run_startup_version_check_with(&backend, "0.24.0").await;
    assert_eq!(transition, VersionTransition::Fresh);

    // Side effect: the current version must be persisted for next boot.
    assert_persisted_version(&backend, "0.24.0").await;
}

#[tokio::test]
async fn startup_version_same_version_restart_is_unchanged() {
    let (backend, _tmp) = fresh_backend().await;

    // First boot establishes the baseline.
    let first = run_startup_version_check_with(&backend, "0.24.0").await;
    assert_eq!(first, VersionTransition::Fresh);

    // Second boot with the same version must not flag a transition.
    let second = run_startup_version_check_with(&backend, "0.24.0").await;
    assert_eq!(second, VersionTransition::Unchanged);

    assert_persisted_version(&backend, "0.24.0").await;
}

#[tokio::test]
async fn startup_version_upgrade_is_detected_and_persisted() {
    let (backend, _tmp) = fresh_backend().await;

    // Boot the old version first.
    let first = run_startup_version_check_with(&backend, "0.23.0").await;
    assert_eq!(first, VersionTransition::Fresh);

    // Capture the "previous" view the gateway reads — it must match the old
    // version at this point, before the upgrade overwrites it.
    let seen_by_gateway = read_previous_version(&backend)
        .await
        .expect("read_previous_version failed");
    assert_eq!(seen_by_gateway.as_deref(), Some("0.23.0"));

    // Simulate deploying the new binary.
    let transition = run_startup_version_check_with(&backend, "0.24.0").await;
    assert_eq!(
        transition,
        VersionTransition::Upgraded {
            previous: "0.23.0".to_string()
        }
    );

    // DB now holds the new version for the next boot.
    assert_persisted_version(&backend, "0.24.0").await;
}

#[tokio::test]
async fn startup_version_downgrade_is_detected_and_persisted() {
    let (backend, _tmp) = fresh_backend().await;

    // Establish a newer-version baseline.
    let first = run_startup_version_check_with(&backend, "0.24.0").await;
    assert_eq!(first, VersionTransition::Fresh);

    // Now a Docker restart accidentally pulls an older image — this is the
    // whole failure class the feature is protecting against.
    let transition = run_startup_version_check_with(&backend, "0.23.0").await;
    assert_eq!(
        transition,
        VersionTransition::Downgraded {
            previous: "0.24.0".to_string()
        }
    );

    // DB now reflects the actually-running (downgraded) version so the
    // *next* boot reads a consistent state. Importantly, the "previous"
    // value is clobbered — this is intentional, the single-slot model only
    // tracks the immediately-prior version.
    assert_persisted_version(&backend, "0.23.0").await;
}

#[tokio::test]
async fn startup_version_full_transition_matrix_end_to_end() {
    // Drives a single DB through every transition the startup wiring can
    // produce. Regression guard for off-by-one or state-leak bugs where a
    // later call accidentally changes the previous transition's classification.
    let (backend, _tmp) = fresh_backend().await;

    // 1. First boot -> Fresh
    assert_eq!(
        run_startup_version_check_with(&backend, "0.22.0").await,
        VersionTransition::Fresh
    );
    assert_persisted_version(&backend, "0.22.0").await;

    // 2. Same-version restart -> Unchanged
    assert_eq!(
        run_startup_version_check_with(&backend, "0.22.0").await,
        VersionTransition::Unchanged
    );

    // 3. Minor upgrade -> Upgraded
    assert_eq!(
        run_startup_version_check_with(&backend, "0.23.0").await,
        VersionTransition::Upgraded {
            previous: "0.22.0".to_string()
        }
    );
    assert_persisted_version(&backend, "0.23.0").await;

    // 4. Major-ish upgrade -> Upgraded
    assert_eq!(
        run_startup_version_check_with(&backend, "1.0.0").await,
        VersionTransition::Upgraded {
            previous: "0.23.0".to_string()
        }
    );

    // 5. Accidental downgrade -> Downgraded
    assert_eq!(
        run_startup_version_check_with(&backend, "0.23.0").await,
        VersionTransition::Downgraded {
            previous: "1.0.0".to_string()
        }
    );
    assert_persisted_version(&backend, "0.23.0").await;

    // 6. Recovery to the correct binary -> Upgraded again.
    assert_eq!(
        run_startup_version_check_with(&backend, "1.0.0").await,
        VersionTransition::Upgraded {
            previous: "0.23.0".to_string()
        }
    );
}
