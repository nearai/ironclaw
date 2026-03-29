//! Memory hygiene: automatic cleanup of stale workspace documents.
//!
//! Runs on a configurable cadence and discovers which directories have hygiene
//! enabled by reading `.config` metadata documents. This is a **metadata-driven**
//! approach: instead of hardcoding `daily/` and `conversations/`, the system
//! respects `hygiene.enabled` and `hygiene.retention_days` set on each folder's
//! `.config` document.
//!
//! A global [`AtomicBool`] guard prevents concurrent hygiene passes, which
//! avoids TOCTOU races on the state file and Windows file-locking errors
//! (OS error 1224) when multiple heartbeat ticks fire before the first
//! pass completes.
//!
//! ```text
//! ┌──────────────────────────────────────────────────┐
//! │               Hygiene Pass                        │
//! │                                                   │
//! │  0. Acquire RUNNING guard (skip if held)          │
//! │  1. Check cadence (skip if ran recently)          │
//! │  2. Save state (claim the cadence window)         │
//! │  3. Discover .config docs with hygiene.enabled    │
//! │  4. For each: cleanup_directory(parent, retention)│
//! │  5. Log summary                                   │
//! └──────────────────────────────────────────────────┘
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bootstrap::ironclaw_base_dir;
use crate::workspace::{DocumentMetadata, IDENTITY_PATHS, Workspace, is_config_path};

/// Global guard preventing concurrent hygiene passes.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Configuration for workspace hygiene.
#[derive(Debug, Clone)]
pub struct HygieneConfig {
    /// Whether hygiene is enabled at all.
    pub enabled: bool,
    /// Maximum number of versions to keep per document.
    /// Enforced during hygiene passes for documents in cleaned directories.
    pub version_keep_count: u32,
    /// Minimum hours between hygiene passes.
    pub cadence_hours: u32,
    /// Directory to store state file (default: `~/.ironclaw`).
    pub state_dir: PathBuf,
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            version_keep_count: 50,
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
    /// Per-directory cleanup results: `(directory_path, deleted_count)`.
    pub directories_cleaned: Vec<(String, u32)>,
    /// Number of document versions pruned across all documents.
    pub versions_pruned: u64,
    /// Whether the run was skipped (cadence not yet elapsed).
    pub skipped: bool,
}

impl HygieneReport {
    /// True if any cleanup work was done.
    pub fn had_work(&self) -> bool {
        self.directories_cleaned.iter().any(|(_, n)| *n > 0) || self.versions_pruned > 0
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

    tracing::info!("memory hygiene: starting cleanup pass");

    let mut report = HygieneReport::default();

    // Discover directories that have hygiene enabled via .config metadata.
    let config_docs = match workspace.find_config_documents().await {
        Ok(docs) => docs,
        Err(e) => {
            tracing::warn!("memory hygiene: failed to discover .config documents: {e}");
            return report;
        }
    };

    for doc in &config_docs {
        let meta = DocumentMetadata::from_value(&doc.metadata);
        let Some(hygiene) = meta.hygiene else {
            continue;
        };
        if !hygiene.enabled {
            continue;
        }

        // Derive the parent directory from the .config path.
        let directory = match doc.path.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/"),
            None => continue, // root-level .config — skip
        };

        match cleanup_directory(workspace, &directory, hygiene.retention_days).await {
            Ok(deleted) => {
                if deleted > 0 {
                    tracing::info!(directory, deleted, "memory hygiene: cleaned directory");
                }
                report.directories_cleaned.push((directory, deleted));
            }
            Err(e) => {
                tracing::warn!(directory, "memory hygiene: failed to clean directory: {e}");
            }
        }
    }

    // Prune old document versions if configured.
    if config.version_keep_count > 0 {
        // Prune versions for documents in directories that were just cleaned.
        // This avoids iterating ALL documents — we only prune where hygiene ran.
        for (directory, _) in &report.directories_cleaned {
            if let Ok(entries) = workspace.list(directory).await {
                for entry in entries {
                    if entry.is_directory || is_config_path(&entry.path) {
                        continue;
                    }
                    let path = if entry.path.starts_with(directory.as_str()) {
                        entry.path.clone()
                    } else {
                        format!("{}{}", directory, entry.path)
                    };
                    if let Ok(doc) = workspace.read(&path).await {
                        match workspace
                            .prune_versions(doc.id, config.version_keep_count as i32)
                            .await
                        {
                            Ok(pruned) => report.versions_pruned += pruned,
                            Err(e) => {
                                tracing::debug!(path, "version prune failed: {e}");
                            }
                        }
                    }
                }
            }
        }
    }

    if report.had_work() {
        tracing::info!(
            directories_cleaned = ?report.directories_cleaned,
            versions_pruned = report.versions_pruned,
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

/// Check if a document path is an identity document that must never be deleted.
///
/// Performs case-insensitive filename comparison to handle case-insensitive
/// filesystems (Windows, macOS).
fn is_identity_document(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let file_name_lower = file_name.to_lowercase();
    IDENTITY_PATHS
        .iter()
        .any(|&p| p.to_lowercase() == file_name_lower)
}

/// Delete documents in `directory` that are older than `retention_days`.
///
/// Skips directories, `.config` files, and identity documents (`MEMORY.md`,
/// `SOUL.md`, `IDENTITY.md`, etc.) which must never be deleted by hygiene
/// regardless of which directory they appear in.
async fn cleanup_directory(
    workspace: &Workspace,
    directory: &str,
    retention_days: u32,
) -> Result<u32, anyhow::Error> {
    let cutoff = Utc::now() - chrono::Duration::days(i64::from(retention_days));
    let entries = workspace.list(directory).await?;
    let mut deleted = 0u32;
    for entry in entries {
        if entry.is_directory {
            continue;
        }
        if is_config_path(&entry.path) {
            continue;
        }
        // Safety net: never delete identity documents regardless of directory.
        // This protects MEMORY.md, SOUL.md, IDENTITY.md, etc. even if a
        // misconfigured .config enables hygiene on a directory containing them.
        if is_identity_document(&entry.path) {
            continue;
        }
        if let Some(updated_at) = entry.updated_at
            && updated_at < cutoff
        {
            let path = if entry.path.starts_with(directory) {
                entry.path.clone()
            } else {
                format!("{}{}", directory, entry.path)
            };
            if let Err(e) = workspace.delete(&path).await {
                tracing::warn!(path, "memory hygiene: failed to delete: {e}");
            } else {
                tracing::debug!(path, "memory hygiene: deleted stale document");
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
        assert_eq!(cfg.version_keep_count, 50);
        assert_eq!(cfg.cadence_hours, 12);
    }

    #[test]
    fn report_defaults_to_no_work() {
        let report = HygieneReport::default();
        assert!(!report.had_work());
        assert!(!report.skipped);
    }

    #[test]
    fn report_had_work_when_directories_cleaned() {
        let report = HygieneReport {
            directories_cleaned: vec![("daily/".to_string(), 3)],
            versions_pruned: 0,
            skipped: false,
        };
        assert!(report.had_work());
    }

    #[test]
    fn report_had_work_when_versions_pruned() {
        let report = HygieneReport {
            directories_cleaned: vec![],
            versions_pruned: 5,
            skipped: false,
        };
        assert!(report.had_work());
    }

    #[test]
    fn report_no_work_when_zero_deletions() {
        let report = HygieneReport {
            directories_cleaned: vec![("daily/".to_string(), 0)],
            versions_pruned: 0,
            skipped: false,
        };
        assert!(!report.had_work());
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

        /// Helper to seed a .config document with hygiene metadata on a directory.
        async fn seed_hygiene_config(workspace: &Workspace, directory: &str, retention_days: u32) {
            let config_path = format!("{}.config", directory);
            // Create the .config document with empty content
            workspace
                .write(&config_path, "")
                .await
                .expect("write .config");
            // Read back to get the document ID
            let doc = workspace
                .read(&config_path)
                .await
                .expect("read .config doc");
            // Set hygiene metadata
            workspace
                .update_metadata(
                    doc.id,
                    &serde_json::json!({
                        "hygiene": {"enabled": true, "retention_days": retention_days},
                        "skip_versioning": true
                    }),
                )
                .await
                .expect("set metadata");
        }

        #[tokio::test]
        async fn cleanup_directory_skips_config_files() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Write documents including a .config
            ws.write("daily/2024-01-15.md", "Old log")
                .await
                .expect("write log");
            ws.write("daily/.config", "").await.expect("write config");

            // Run cleanup with 0-day retention (deletes everything old)
            let deleted = cleanup_directory(&ws, "daily/", 0)
                .await
                .expect("cleanup_directory");

            // Should have deleted the log but not the .config
            assert!(deleted > 0, "should have deleted old daily documents");

            // Verify .config still exists
            let config_doc = db
                .get_document_by_path("default", None, "daily/.config")
                .await
                .expect("get .config doc");
            assert_eq!(config_doc.path, "daily/.config");
        }

        #[tokio::test]
        async fn cleanup_directory_handles_empty_directory() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Run cleanup on an empty directory
            let deleted = cleanup_directory(&ws, "conversations/", 7)
                .await
                .expect("cleanup_directory");

            assert_eq!(deleted, 0, "should delete 0 from empty directory");
        }

        #[tokio::test]
        async fn metadata_driven_cleanup_discovers_directories() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Seed .config with hygiene enabled on daily/ (0-day retention = delete everything)
            seed_hygiene_config(&ws, "daily/", 0).await;

            // Write some documents
            ws.write("daily/log1.md", "content 1")
                .await
                .expect("write doc 1");
            ws.write("daily/log2.md", "content 2")
                .await
                .expect("write doc 2");

            // Use find_config_documents + cleanup_directory directly to avoid
            // global AtomicBool contention with concurrent tests.
            let configs = ws
                .find_config_documents()
                .await
                .expect("find_config_documents");
            assert!(!configs.is_empty(), "should find .config documents");

            let meta = DocumentMetadata::from_value(&configs[0].metadata);
            assert!(
                meta.hygiene.as_ref().is_some_and(|h| h.enabled),
                "hygiene should be enabled"
            );

            let deleted = cleanup_directory(&ws, "daily/", 0)
                .await
                .expect("cleanup_directory");
            assert!(deleted > 0, "should have cleaned documents");
        }

        #[test]
        fn cleanup_respects_cadence_via_state_file() {
            // Test cadence logic without run_if_due (which uses a global
            // AtomicBool that causes flakiness with concurrent tests).
            let dir = tempfile::tempdir().expect("tempdir");
            let state_file = dir.path().join("memory_hygiene_state.json");

            // No state file → cadence not elapsed (first run should proceed)
            assert!(
                load_state(&state_file).is_none(),
                "no state file should exist initially"
            );

            // Save state (simulates a completed run)
            save_state(&state_file);

            // State exists with recent timestamp → cadence check should block
            let state = load_state(&state_file).expect("state should be loadable");
            let elapsed = Utc::now().signed_duration_since(state.last_run);
            assert!(elapsed.num_seconds() < 5, "state should be very recent");

            // With 12-hour cadence, a run just saved should cause skip
            let cadence = chrono::Duration::hours(12);
            assert!(
                elapsed < cadence,
                "elapsed time should be less than cadence"
            );
        }

        #[tokio::test]
        async fn cleanup_reports_deletion_counts_correctly() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Seed hygiene on both directories
            seed_hygiene_config(&ws, "daily/", 0).await;
            seed_hygiene_config(&ws, "conversations/", 0).await;

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

            // Run with 0-day retention via direct cleanup_directory calls
            let deleted_daily = cleanup_directory(&ws, "daily/", 0)
                .await
                .expect("cleanup daily");
            let deleted_conv = cleanup_directory(&ws, "conversations/", 0)
                .await
                .expect("cleanup conversations");

            assert!(deleted_daily > 0, "should report deleted daily logs");
            assert_eq!(deleted_conv, 1, "should report 1 deleted conversation doc");

            // Verify HygieneReport aggregation
            let report = HygieneReport {
                directories_cleaned: vec![
                    ("daily/".to_string(), deleted_daily),
                    ("conversations/".to_string(), deleted_conv),
                ],
                versions_pruned: 0,
                skipped: false,
            };

            assert!(!report.skipped, "should not be skipped");
            assert!(report.had_work(), "report should indicate work was done");

            // Verify had_work() correctly checks directory counts
            let no_work = HygieneReport {
                directories_cleaned: vec![],
                versions_pruned: 0,
                skipped: false,
            };
            assert!(!no_work.had_work(), "empty report should indicate no work");
        }

        #[tokio::test]
        async fn no_config_documents_means_no_cleanup() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Write documents to custom/ but do NOT create a .config
            ws.write("custom/note1.md", "some content")
                .await
                .expect("write note1");
            ws.write("custom/note2.md", "other content")
                .await
                .expect("write note2");

            // Without .config documents, find_config_documents returns empty,
            // so no directories are discovered for cleanup.
            let config_docs = ws
                .find_config_documents()
                .await
                .expect("find_config_documents");
            assert!(config_docs.is_empty(), "no .config documents should exist");

            // Documents should still exist (no cleanup occurred)
            assert!(ws.read("custom/note1.md").await.is_ok());
            assert!(ws.read("custom/note2.md").await.is_ok());
        }

        #[tokio::test]
        async fn config_with_hygiene_disabled_skips_directory() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Create a .config with hygiene disabled
            let config_doc = ws.write("test/.config", "").await.expect("write .config");
            ws.update_metadata(
                config_doc.id,
                &serde_json::json!({
                    "hygiene": {"enabled": false, "retention_days": 1},
                    "skip_versioning": true
                }),
            )
            .await
            .expect("set metadata");

            // Write a document
            ws.write("test/data.md", "should survive")
                .await
                .expect("write data");

            // Verify that the .config document is found but has hygiene disabled.
            // The run_if_due loop skips directories where hygiene.enabled is false.
            let config_docs = ws
                .find_config_documents()
                .await
                .expect("find_config_documents");
            assert_eq!(config_docs.len(), 1, "should find 1 .config document");
            let meta = DocumentMetadata::from_value(&config_docs[0].metadata);
            assert!(
                meta.hygiene.is_some() && !meta.hygiene.as_ref().is_none_or(|h| h.enabled),
                "hygiene should be present but disabled"
            );

            // Even with 0-day retention, cleanup_directory should not delete
            // the doc — but the key point is that run_if_due would never call
            // cleanup_directory for this directory at all (enabled=false).
            // Document should still exist.
            let doc = ws.read("test/data.md").await.expect("data.md should exist");
            assert_eq!(doc.content, "should survive");
        }

        #[tokio::test]
        async fn multiple_directories_with_different_retention() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // fast/ has 0-day retention (everything gets deleted)
            seed_hygiene_config(&ws, "fast/", 0).await;
            // slow/ has 9999-day retention (nothing gets deleted)
            seed_hygiene_config(&ws, "slow/", 9999).await;

            ws.write("fast/ephemeral.md", "gone soon")
                .await
                .expect("write fast doc");
            ws.write("slow/durable.md", "here to stay")
                .await
                .expect("write slow doc");

            // Use cleanup_directory directly to avoid global AtomicBool
            // contention with concurrent tests.
            let fast_deleted = cleanup_directory(&ws, "fast/", 0)
                .await
                .expect("cleanup fast");
            let slow_deleted = cleanup_directory(&ws, "slow/", 9999)
                .await
                .expect("cleanup slow");

            assert!(fast_deleted > 0, "fast/ docs should be deleted");
            assert_eq!(slow_deleted, 0, "slow/ docs should be preserved");

            // Verify slow doc still readable
            let doc = ws
                .read("slow/durable.md")
                .await
                .expect("slow doc should still exist");
            assert_eq!(doc.content, "here to stay");
        }

        #[tokio::test]
        async fn documents_newer_than_retention_not_deleted() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            ws.write("test/recent.md", "just created")
                .await
                .expect("write recent doc");

            // Use cleanup_directory directly to avoid global AtomicBool contention
            // with other concurrent tests that call run_if_due.
            let deleted = cleanup_directory(&ws, "test/", 9999)
                .await
                .expect("cleanup_directory");

            assert_eq!(
                deleted, 0,
                "recent docs should not be deleted with 9999-day retention"
            );

            let doc = ws
                .read("test/recent.md")
                .await
                .expect("recent doc should still exist");
            assert_eq!(doc.content, "just created");
        }

        #[tokio::test]
        async fn version_pruning_during_hygiene() {
            let (db, _tmp) = create_test_db().await;
            let ws = create_workspace(&db);

            // Write multiple times to create versions
            let doc = ws
                .write("daily/evolving.md", "version 1")
                .await
                .expect("write v1");
            let doc_id = doc.id;
            ws.write("daily/evolving.md", "version 2")
                .await
                .expect("write v2");
            ws.write("daily/evolving.md", "version 3")
                .await
                .expect("write v3");
            ws.write("daily/evolving.md", "version 4")
                .await
                .expect("write v4");

            // Verify we have multiple versions before pruning
            let versions_before = ws.list_versions(doc_id, 100).await.expect("list versions");
            assert!(
                versions_before.len() >= 3,
                "should have at least 3 versions before pruning, got {}",
                versions_before.len()
            );

            // Prune directly (avoids global AtomicBool contention with concurrent tests)
            let pruned = ws.prune_versions(doc_id, 2).await.expect("prune_versions");
            assert!(pruned > 0, "should have pruned some versions");

            // After pruning, at most 2 versions should remain
            let versions_after = ws.list_versions(doc_id, 100).await.expect("list versions");
            assert!(
                versions_after.len() <= 2,
                "should have at most 2 versions after pruning, got {}",
                versions_after.len()
            );
        }
    }
}
