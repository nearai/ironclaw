//! Memory hygiene: automatic cleanup of stale workspace documents.
//!
//! Runs on a configurable cadence and deletes daily log entries older
//! than the retention period. Identity files (`IDENTITY.md`, `SOUL.md`,
//! etc.) are never touched.
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │               Hygiene Pass                   │
//! │                                              │
//! │  1. Check cadence (skip if ran recently)     │
//! │  2. List daily/ documents                    │
//! │  3. Delete those older than retention_days   │
//! │  4. Log summary                              │
//! └─────────────────────────────────────────────┘
//! ```

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::workspace::Workspace;

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
    /// Maximum documents to re-index per hygiene pass.
    ///
    /// Re-indexing involves embedding API calls, so we process in batches
    /// to avoid overwhelming the API or blocking startup.
    pub reindex_batch_size: usize,
}

impl Default for HygieneConfig {
    fn default() -> Self {
        let state_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ironclaw");

        Self {
            enabled: true,
            retention_days: 30,
            cadence_hours: 12,
            state_dir,
            reindex_batch_size: 20, // Process 20 docs per pass (~2 seconds with 100ms throttle)
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
    /// Number of documents re-indexed due to stale chunks.
    pub docs_reindexed: usize,
    /// Number of documents that failed to re-index.
    pub reindex_failed: usize,
    /// True if more stale documents remain after this batch.
    pub reindex_has_more: bool,
    /// Whether the run was skipped (cadence not yet elapsed).
    pub skipped: bool,
}

impl HygieneReport {
    /// True if any cleanup work was done.
    pub fn had_work(&self) -> bool {
        self.daily_logs_deleted > 0 || self.docs_reindexed > 0
    }
}

/// Run a hygiene pass if the cadence has elapsed.
///
/// This is best-effort: failures are logged but never propagate. The
/// agent should not crash because cleanup failed.
pub async fn run_if_due(workspace: &Workspace, config: &HygieneConfig) -> HygieneReport {
    if !config.enabled {
        return HygieneReport {
            skipped: true,
            ..Default::default()
        };
    }

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

    tracing::info!(
        retention_days = config.retention_days,
        reindex_batch_size = config.reindex_batch_size,
        "memory hygiene: starting cleanup pass"
    );

    let mut report = HygieneReport::default();

    // Delete old daily logs
    match cleanup_daily_logs(workspace, config.retention_days).await {
        Ok(count) => report.daily_logs_deleted = count,
        Err(e) => tracing::warn!("memory hygiene: failed to clean daily logs: {e}"),
    }

    // Re-index documents with stale chunks
    match workspace.reindex_stale_chunks(config.reindex_batch_size).await {
        Ok(reindex_report) => {
            report.docs_reindexed = reindex_report.processed;
            report.reindex_failed = reindex_report.failed;
            report.reindex_has_more = reindex_report.has_more;
        }
        Err(e) => tracing::warn!("memory hygiene: failed to re-index stale chunks: {e}"),
    }

    if report.had_work() {
        tracing::info!(
            daily_logs_deleted = report.daily_logs_deleted,
            docs_reindexed = report.docs_reindexed,
            reindex_failed = report.reindex_failed,
            reindex_has_more = report.reindex_has_more,
            "memory hygiene: cleanup complete"
        );
    } else {
        tracing::debug!("memory hygiene: nothing to clean");
    }

    // Save state (best-effort)
    save_state(&state_file);

    report
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

fn save_state(path: &std::path::Path) {
    let state = HygieneState {
        last_run: Utc::now(),
    };
    if let Some(dir) = state_path_dir(path) {
        std::fs::create_dir_all(dir).ok();
    }
    if let Ok(json) = serde_json::to_string_pretty(&state)
        && let Err(e) = std::fs::write(path, json)
    {
        tracing::warn!("memory hygiene: failed to save state: {e}");
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
            ..Default::default()
        };
        assert!(report.had_work());
    }

    #[test]
    fn report_had_work_when_reindexed() {
        let report = HygieneReport {
            docs_reindexed: 5,
            ..Default::default()
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
}
