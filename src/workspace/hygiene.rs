//! Memory hygiene: automatic cleanup of stale workspace documents.
//!
//! Runs on a configurable cadence and deletes daily log entries and conversation
//! documents older than their respective retention periods. Identity files
//! (`IDENTITY.md`, `SOUL.md`, etc.) are never touched.
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
//! │  4. Delete those older than daily_retention  │
//! │  5. List conversations/ documents            │
//! │  6. Delete those older than conversation_ret │
//! │  7. Log summary                              │
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

/// Paths that must never be deleted by hygiene, regardless of age.
const IDENTITY_PATHS: &[&str] = &[
    crate::workspace::document::paths::MEMORY,
    crate::workspace::document::paths::IDENTITY,
    crate::workspace::document::paths::SOUL,
    crate::workspace::document::paths::AGENTS,
    crate::workspace::document::paths::USER,
    crate::workspace::document::paths::HEARTBEAT,
    crate::workspace::document::paths::README,
    crate::workspace::document::paths::TOOLS,
    crate::workspace::document::paths::BOOTSTRAP,
];

/// Check if a document path is an identity document that must never be deleted.
///
/// Performs case-insensitive comparison to handle case-insensitive filesystems
/// (Windows, macOS) and prevent accidental deletion of identity docs with
/// different casing (e.g., memory.md, MEMORY.MD, Memory.md).
fn is_identity_path(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let file_name_lower = file_name.to_lowercase();
    IDENTITY_PATHS
        .iter()
        .any(|&p| p.to_lowercase() == file_name_lower)
}

/// Configuration for workspace hygiene.
#[derive(Debug, Clone)]
pub struct HygieneConfig {
    /// Whether hygiene is enabled at all.
    pub enabled: bool,
    /// Documents in `daily/` older than this many days are deleted.
    pub daily_retention_days: u32,
    /// Documents in `conversations/` older than this many days are deleted.
    pub conversation_retention_days: u32,
    /// Minimum hours between hygiene passes.
    pub cadence_hours: u32,
    /// Directory to store state file (default: `~/.ironclaw`).
    pub state_dir: PathBuf,
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            daily_retention_days: 30,
            conversation_retention_days: 7,
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
    /// Number of conversation documents deleted.
    pub conversation_docs_deleted: u32,
    /// Whether the run was skipped (cadence not yet elapsed).
    pub skipped: bool,
}

impl HygieneReport {
    /// True if any cleanup work was done.
    pub fn had_work(&self) -> bool {
        self.daily_logs_deleted > 0 || self.conversation_docs_deleted > 0
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
        daily_retention_days = config.daily_retention_days,
        conversation_retention_days = config.conversation_retention_days,
        "memory hygiene: starting cleanup pass"
    );

    let mut report = HygieneReport::default();

    // Delete old daily logs
    match cleanup_daily_logs(workspace, config.daily_retention_days).await {
        Ok(count) => report.daily_logs_deleted = count,
        Err(e) => tracing::warn!("memory hygiene: failed to clean daily logs: {e}"),
    }

    // Delete old conversation documents
    match cleanup_conversation_docs(workspace, config.conversation_retention_days).await {
        Ok(count) => report.conversation_docs_deleted = count,
        Err(e) => tracing::warn!("memory hygiene: failed to clean conversation docs: {e}"),
    }

    if report.had_work() {
        tracing::info!(
            daily_logs_deleted = report.daily_logs_deleted,
            conversation_docs_deleted = report.conversation_docs_deleted,
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

        // Never delete identity documents
        if is_identity_path(&entry.path) {
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

/// Delete conversation documents older than `retention_days`.
async fn cleanup_conversation_docs(
    workspace: &Workspace,
    retention_days: u32,
) -> Result<u32, anyhow::Error> {
    let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
    let entries = workspace.list("conversations/").await?;

    let mut deleted = 0u32;
    for entry in entries {
        if entry.is_directory {
            continue;
        }

        // Never delete identity documents
        if is_identity_path(&entry.path) {
            continue;
        }

        // Check if the document is old enough to delete
        if let Some(updated_at) = entry.updated_at
            && updated_at < cutoff
        {
            let path = if entry.path.starts_with("conversations/") {
                entry.path.clone()
            } else {
                format!("conversations/{}", entry.path)
            };

            if let Err(e) = workspace.delete(&path).await {
                tracing::warn!(
                    path,
                    "memory hygiene: failed to delete conversation doc: {e}"
                );
            } else {
                tracing::debug!(path, "memory hygiene: deleted old conversation doc");
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
    use std::sync::Mutex;

    use crate::workspace::hygiene::*;

    /// Serialize tests that touch the global `RUNNING` AtomicBool so they
    /// don't interfere with each other when `cargo test` runs in parallel.
    static RUNNING_TESTS: Mutex<()> = Mutex::new(());

    #[test]
    fn default_config_is_reasonable() {
        let cfg = HygieneConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.daily_retention_days, 30);
        assert_eq!(cfg.conversation_retention_days, 7);
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
            conversation_docs_deleted: 0,
            skipped: false,
        };
        assert!(report.had_work());
    }

    #[test]
    fn report_had_work_when_conversation_deleted() {
        let report = HygieneReport {
            daily_logs_deleted: 0,
            conversation_docs_deleted: 2,
            skipped: false,
        };
        assert!(report.had_work());
    }

    #[test]
    fn is_identity_path_excludes_sacred_docs() {
        for name in [
            "MEMORY.md",
            "IDENTITY.md",
            "SOUL.md",
            "AGENTS.md",
            "USER.md",
            "HEARTBEAT.md",
            "README.md",
            "TOOLS.md",
            "BOOTSTRAP.md",
        ] {
            assert!(is_identity_path(name), "{name} should be excluded");
            assert!(
                is_identity_path(&format!("conversations/{name}")),
                "conversations/{name} should be excluded via path"
            );
        }
    }

    #[test]
    fn is_identity_path_case_insensitive() {
        // Verify case-insensitive matching for case-insensitive filesystems
        assert!(
            is_identity_path("memory.md"),
            "lowercase memory.md should be excluded"
        );
        assert!(
            is_identity_path("Memory.md"),
            "mixed case Memory.md should be excluded"
        );
        assert!(
            is_identity_path("MEMORY.MD"),
            "uppercase MEMORY.MD should be excluded"
        );
        assert!(
            is_identity_path("identity.md"),
            "lowercase identity.md should be excluded"
        );
        assert!(
            is_identity_path("conversations/soul.md"),
            "conversations/soul.md should be excluded"
        );
        assert!(
            is_identity_path("conversations/SOUL.MD"),
            "conversations/SOUL.MD should be excluded"
        );
    }

    #[test]
    fn is_identity_path_allows_normal_docs() {
        for path in [
            "daily/2024-01-01.md",
            "conversations/chat-abc.md",
            "notes.md",
        ] {
            assert!(!is_identity_path(path), "{path} should not be excluded");
        }
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
        let _lock = RUNNING_TESTS.lock().unwrap();

        // Reset the global flag to ensure a clean state
        RUNNING.store(false, Ordering::SeqCst);

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

    // ================================================================
    // Async integration tests (require libsql backend)
    // ================================================================

    #[cfg(feature = "libsql")]
    mod async_tests {
        use super::*;
        use crate::db::Database;
        use std::sync::Arc;

        /// Helper to create a test database with migrations.
        async fn create_test_db() -> (Arc<dyn crate::db::Database>, tempfile::TempDir) {
            use crate::db::libsql::LibSqlBackend;

            let temp_dir = tempfile::tempdir().expect("tempdir");
            let db_path = temp_dir.path().join("test_hygiene.db");
            let backend = LibSqlBackend::new_local(&db_path)
                .await
                .expect("LibSqlBackend::new_local");
            backend.run_migrations().await.expect("run_migrations");
            let db: Arc<dyn Database> = Arc::new(backend);
            (db, temp_dir)
        }

        /// Helper to create a workspace from a test database.
        fn create_workspace(db: &Arc<dyn Database>) -> Arc<Workspace> {
            Arc::new(Workspace::new_with_db("default", db.clone()))
        }

        #[tokio::test]
        async fn cleanup_daily_logs_preserves_identity_documents() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Write several regular documents (non-identity)
            ws.write("daily/2024-01-15.md", "Old log")
                .await
                .expect("write log 1");
            ws.write("daily/2024-01-20.md", "Another log")
                .await
                .expect("write log 2");

            // Write an identity document
            ws.write("MEMORY.md", "Long-term curated memory")
                .await
                .expect("write identity");

            // List before cleanup
            let before = ws.list("daily/").await.expect("list before");
            let daily_count_before = before.iter().filter(|e| !e.is_directory).count();
            assert!(daily_count_before >= 2, "should have at least 2 daily logs");

            // Run cleanup with 0-day retention (deletes everything old)
            // This tests that even with aggressive cleanup, identity docs survive
            let deleted = cleanup_daily_logs(&ws, 0)
                .await
                .expect("cleanup_daily_logs");

            // Should have deleted some documents (the daily logs)
            assert!(deleted > 0, "should have deleted old daily documents");

            // Verify identity doc still exists
            let identity = db
                .get_document_by_path("default", None, "MEMORY.md")
                .await
                .expect("get identity doc");
            assert_eq!(identity.path, "MEMORY.md");
            assert_eq!(identity.content, "Long-term curated memory");
        }

        #[tokio::test]
        async fn cleanup_conversation_docs_handles_empty_directory() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Run cleanup on an empty directory (conversations/ doesn't exist)
            let deleted = cleanup_conversation_docs(&ws, 7)
                .await
                .expect("cleanup_conversation_docs");

            // Should delete 0 (nothing to delete)
            assert_eq!(deleted, 0, "should delete 0 from empty directory");
        }

        #[tokio::test]
        async fn cleanup_respects_cadence_prevents_concurrent_runs() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            let config = HygieneConfig {
                enabled: true,
                daily_retention_days: 30,
                conversation_retention_days: 7,
                cadence_hours: 12,
                state_dir: _tmp.path().to_path_buf(),
            };

            // First run should succeed
            let report1 = run_if_due(&ws, &config).await;
            assert!(!report1.skipped, "first run should not be skipped");

            // Second run immediately should be skipped (cadence not elapsed)
            let report2 = run_if_due(&ws, &config).await;
            assert!(report2.skipped, "second run should be skipped by cadence");

            // Report structure should be correct
            assert_eq!(
                report1.daily_logs_deleted + report1.conversation_docs_deleted,
                0,
                "first run should have clean counts"
            );
        }

        #[tokio::test]
        async fn cleanup_reports_deletion_counts_correctly() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Write some documents
            ws.write("daily/log1.md", "content 1")
                .await
                .expect("write doc 1");
            ws.write("daily/log2.md", "content 2")
                .await
                .expect("write doc 2");
            ws.write("conversations/chat1.md", "content 3")
                .await
                .expect("write doc 3");

            // Run with 0-day retention to delete everything non-identity
            let deleted_daily = cleanup_daily_logs(&ws, 0).await.expect("cleanup daily");
            let deleted_conv = cleanup_conversation_docs(&ws, 0)
                .await
                .expect("cleanup conversations");

            // Both should report deletions
            assert!(deleted_daily > 0, "should report deleted daily logs");
            assert_eq!(deleted_conv, 1, "should report 1 deleted conversation doc");

            // Create a HygieneReport and verify aggregation works
            let report = HygieneReport {
                daily_logs_deleted: deleted_daily,
                conversation_docs_deleted: deleted_conv,
                skipped: false,
            };

            // Verify HygieneReport structure
            assert!(!report.skipped, "should not be skipped");
            assert!(report.had_work(), "report should indicate work was done");
            assert!(
                report.daily_logs_deleted > 0 || report.conversation_docs_deleted > 0,
                "report should have at least one deletion count > 0"
            );

            // Verify had_work() correctly combines both counts
            let no_work = HygieneReport {
                daily_logs_deleted: 0,
                conversation_docs_deleted: 0,
                skipped: false,
            };
            assert!(!no_work.had_work(), "empty report should indicate no work");
        }
    }
}
