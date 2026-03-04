//! Memory hygiene: automatic cleanup of stale workspace documents.
//!
//! Runs on a configurable cadence and deletes daily log entries older
//! than the retention period. Identity files (`IDENTITY.md`, `SOUL.md`,
//! etc.) are never touched.
//!
//! A global [`AtomicBool`] guard prevents concurrent hygiene passes, which
//! avoids TOCTOU races on the state file and Windows file-locking errors
//! (OS error 1224) when multiple heartbeat ticks fire before the first
//! pass completes.
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │               Hygiene Pass                   │
//! │                                              │
//! │  0. Acquire RUNNING guard (skip if held)     │
//! │  1. Check cadence (skip if ran recently)     │
//! │  2. Save state (claim the cadence window)    │
//! │  3. List daily/ documents                    │
//! │  4. Delete those older than retention_days   │
//! │  5. Log summary                              │
//! └─────────────────────────────────────────────┘
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bootstrap::ironclaw_base_dir;
use crate::workspace::Workspace;

/// Global guard preventing concurrent hygiene passes.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Configuration for workspace hygiene.
#[derive(Debug, Clone)]
pub struct HygieneConfig {
    /// Whether hygiene is enabled at all.
    pub enabled: bool,
    /// Documents in `daily/` older than this many days are deleted.
    pub retention_days: u32,
    /// Minimum hours between hygiene passes.
    pub cadence_hours: u32,
    /// Directory to store state file (default: `~/.ironclaw`).
    pub state_dir: PathBuf,
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 30,
            cadence_hours: 12,
            state_dir: ironclaw_base_dir(),
        }
    }
}

/// Persisted state for tracking hygiene cadence.
#[derive(Debug, Serialize, Deserialize)]
struct HygieneState {
    last_run: DateTime<Utc>,
}

/// Summary of what a hygiene pass cleaned up.
#[derive(Debug, Default)]
pub struct HygieneReport {
    /// Number of daily log documents deleted.
    pub daily_logs_deleted: u32,
    /// Whether the run was skipped (cadence not yet elapsed).
    pub skipped: bool,
}

impl HygieneReport {
    /// True if any cleanup work was done.
    pub fn had_work(&self) -> bool {
        self.daily_logs_deleted > 0
    }
}

/// Run a hygiene pass if the cadence has elapsed.
///
/// This is best-effort: failures are logged but never propagate. The
/// agent should not crash because cleanup failed.
///
/// An [`AtomicBool`] guard ensures only one pass runs at a time, and the
/// state file is written *before* cleanup so that concurrent callers that
/// slip past the guard still see an up-to-date cadence timestamp.
pub async fn run_if_due(workspace: &Workspace, config: &HygieneConfig) -> HygieneReport {
    if !config.enabled {
        return HygieneReport {
            skipped: true,
            ..Default::default()
        };
    }

    // Prevent concurrent passes. If another task is already running,
    // skip immediately.
    if RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        tracing::debug!("memory hygiene: skipping (another pass is running)");
        return HygieneReport {
            skipped: true,
            ..Default::default()
        };
    }

    // Ensure the guard is released when we return.
    let _guard = RunningGuard;

    let state_file = config.state_dir.join("memory_hygiene_state.json");

    // Check cadence
    if let Some(state) = load_state(&state_file) {
        let elapsed = Utc::now().signed_duration_since(state.last_run);
        let cadence = chrono::Duration::hours(i64::from(config.cadence_hours));
        if elapsed < cadence {
            tracing::debug!(
                hours_since_last = elapsed.num_hours(),
                cadence_hours = config.cadence_hours,
                "memory hygiene: skipping (cadence not elapsed)"
            );
            return HygieneReport {
                skipped: true,
                ..Default::default()
            };
        }
    }

    // Save state *before* cleanup to claim the cadence window and prevent
    // TOCTOU races where another task reads stale state.
    save_state(&state_file);

    tracing::info!(
        retention_days = config.retention_days,
        "memory hygiene: starting cleanup pass"
    );

    let mut report = HygieneReport::default();

    // Delete old daily logs
    match cleanup_daily_logs(workspace, config.retention_days).await {
        Ok(count) => report.daily_logs_deleted = count,
        Err(e) => tracing::warn!("memory hygiene: failed to clean daily logs: {e}"),
    }

    if report.had_work() {
        tracing::info!(
            daily_logs_deleted = report.daily_logs_deleted,
            "memory hygiene: cleanup complete"
        );
    } else {
        tracing::debug!("memory hygiene: nothing to clean");
    }

    report
}

/// RAII guard that clears the [`RUNNING`] flag on drop.
struct RunningGuard;

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Delete daily log documents older than `retention_days`.
async fn cleanup_daily_logs(
    workspace: &Workspace,
    retention_days: u32,
) -> Result<u32, anyhow::Error> {
    let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
    let entries = workspace.list("daily/").await?;

    let mut deleted = 0u32;
    for entry in entries {
        if entry.is_directory {
            continue;
        }

        // Check if the document is old enough to delete
        if let Some(updated_at) = entry.updated_at
            && updated_at < cutoff
        {
            let path = if entry.path.starts_with("daily/") {
                entry.path.clone()
            } else {
                format!("daily/{}", entry.path)
            };

            if let Err(e) = workspace.delete(&path).await {
                tracing::warn!(path, "memory hygiene: failed to delete: {e}");
            } else {
                tracing::debug!(path, "memory hygiene: deleted old daily log");
                deleted += 1;
            }
        }
    }

    Ok(deleted)
}

fn state_path_dir(state_file: &std::path::Path) -> Option<&std::path::Path> {
    state_file.parent()
}

fn load_state(path: &std::path::Path) -> Option<HygieneState> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save state using atomic write (write to temp file, then rename).
///
/// This avoids partial writes and Windows file-locking errors (OS error
/// 1224) when multiple processes try to write the same file.
fn save_state(path: &std::path::Path) {
    let state = HygieneState {
        last_run: Utc::now(),
    };
    if let Some(dir) = state_path_dir(path)
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        tracing::warn!("memory hygiene: failed to create state dir: {e}");
        return;
    }
    let Ok(json) = serde_json::to_string_pretty(&state) else {
        return;
    };

    // Write to a temp file in the same directory, then atomically rename.
    let tmp_path = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp_path, &json) {
        tracing::warn!("memory hygiene: failed to write temp state: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        tracing::warn!("memory hygiene: failed to rename state file: {e}");
        // Clean up temp file on rename failure
        let _ = std::fs::remove_file(&tmp_path);
    }
}

#[cfg(test)]
mod tests {
    use crate::workspace::hygiene::*;

    #[test]
    fn default_config_is_reasonable() {
        let cfg = HygieneConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.retention_days, 30);
        assert_eq!(cfg.cadence_hours, 12);
    }

    #[test]
    fn report_defaults_to_no_work() {
        let report = HygieneReport::default();
        assert!(!report.had_work());
        assert!(!report.skipped);
    }

    #[test]
    fn report_had_work_when_deleted() {
        let report = HygieneReport {
            daily_logs_deleted: 3,
            skipped: false,
        };
        assert!(report.had_work());
    }

    #[test]
    fn load_state_returns_none_for_missing_file() {
        assert!(load_state(std::path::Path::new("/tmp/nonexistent_hygiene.json")).is_none());
    }

    #[test]
    fn save_and_load_state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hygiene_state.json");

        save_state(&path);
        let state = load_state(&path).expect("state should be loadable after save");

        // Should be within the last second
        let elapsed = Utc::now().signed_duration_since(state.last_run);
        assert!(elapsed.num_seconds() < 2);
    }

    #[test]
    fn save_state_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("state.json");

        save_state(&path);
        assert!(path.exists());
    }

    #[test]
    fn save_state_is_atomic_no_tmp_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let tmp = dir.path().join("state.json.tmp");

        save_state(&path);
        assert!(path.exists(), "state file should exist");
        assert!(!tmp.exists(), "temp file should be cleaned up after rename");

        // Verify the content is valid JSON
        let state = load_state(&path).expect("saved state should be loadable");
        let elapsed = Utc::now().signed_duration_since(state.last_run);
        assert!(elapsed.num_seconds() < 2);
    }

    /// Regression test for issue #495: concurrent hygiene passes should be
    /// serialized by the AtomicBool guard.
    #[test]
    fn running_guard_prevents_reentry() {
        // Simulate acquiring the guard
        assert!(
            RUNNING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "first acquisition should succeed"
        );

        // Second acquisition should fail
        assert!(
            RUNNING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err(),
            "second acquisition should fail while first is held"
        );

        // Release
        RUNNING.store(false, Ordering::SeqCst);

        // Now it should succeed again
        assert!(
            RUNNING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "acquisition should succeed after release"
        );
        RUNNING.store(false, Ordering::SeqCst);
    }
}
