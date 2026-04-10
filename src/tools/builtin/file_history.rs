//! File modification history for tracking and undoing filesystem changes.
//!
//! Provides a per-session in-memory history of file modifications made by tools
//! like `apply_patch` and `write_file`, enabling file-level undo.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::context::JobContext;
use crate::tools::builtin::path_utils::validate_path;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};

/// Maximum number of snapshots to keep by default.
const DEFAULT_MAX_SNAPSHOTS: usize = 50;

/// A snapshot of a file's content before modification.
#[derive(Debug, Clone)]
pub struct FileSnapshot {
    /// Unique snapshot ID.
    pub id: Uuid,
    /// Job/session that created this snapshot.
    pub job_id: Uuid,
    /// Absolute path to the file.
    pub path: PathBuf,
    /// File content before the modification (raw bytes for binary support).
    /// `None` means the file did not exist before the change.
    pub content_before: Option<Vec<u8>>,
    /// When the snapshot was taken.
    pub timestamp: DateTime<Utc>,
    /// Which tool made the change (e.g., "apply_patch", "write_file").
    pub tool_name: String,
    /// Turn number when the change was made.
    pub turn_number: usize,
}

/// In-memory history of file modifications for the current session.
#[derive(Debug)]
pub struct FileHistory {
    snapshots: VecDeque<FileSnapshot>,
    max_snapshots: usize,
}

impl Default for FileHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl FileHistory {
    /// Create a new file history with default capacity.
    pub fn new() -> Self {
        Self {
            snapshots: VecDeque::new(),
            max_snapshots: DEFAULT_MAX_SNAPSHOTS,
        }
    }

    /// Create a new file history with custom capacity.
    pub fn with_max_snapshots(max_snapshots: usize) -> Self {
        Self {
            snapshots: VecDeque::new(),
            max_snapshots,
        }
    }

    /// Take a snapshot of a file's content before modification.
    ///
    /// Reads the file at `path` and stores its content. If the file does not
    /// exist, the snapshot records that absence so `file_undo` can remove the
    /// newly-created file.
    pub async fn snapshot(
        &mut self,
        job_id: Uuid,
        path: &Path,
        tool_name: &str,
        turn_number: usize,
    ) -> Result<Option<Uuid>, ToolError> {
        // Read as raw bytes to support binary files.
        let content_before = match tokio::fs::read(path).await {
            Ok(c) => Some(c),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "Failed to snapshot file before modification: {}",
                    e
                )));
            }
        };

        let id = Uuid::new_v4();
        let snapshot = FileSnapshot {
            id,
            job_id,
            path: path.to_path_buf(),
            content_before,
            timestamp: Utc::now(),
            tool_name: tool_name.to_string(),
            turn_number,
        };

        self.snapshots.push_back(snapshot);

        // Evict oldest if over capacity
        while self.snapshots.len() > self.max_snapshots {
            self.snapshots.pop_front();
        }

        Ok(Some(id))
    }

    /// Get the most recent snapshot for a file path within a specific job.
    pub fn latest_snapshot_for(&self, job_id: Uuid, path: &Path) -> Option<&FileSnapshot> {
        self.snapshots
            .iter()
            .rev()
            .find(|s| s.job_id == job_id && s.path == path)
    }

    /// Get all snapshots for a file path within a specific job, newest first.
    pub fn snapshots_for(&self, job_id: Uuid, path: &Path) -> Vec<&FileSnapshot> {
        let mut result: Vec<_> = self
            .snapshots
            .iter()
            .filter(|s| s.job_id == job_id && s.path == path)
            .collect();
        result.reverse();
        result
    }

    /// Restore a file to its most recent snapshot within the given job and remove it.
    ///
    /// Returns the snapshot that was restored, or `None` if no snapshot exists.
    pub async fn restore_latest(
        &mut self,
        job_id: Uuid,
        path: &Path,
    ) -> Result<Option<FileSnapshot>, ToolError> {
        let idx = self
            .snapshots
            .iter()
            .rposition(|s| s.job_id == job_id && s.path == path);

        let Some(idx) = idx else {
            return Ok(None);
        };

        let Some(snapshot) = self.snapshots.remove(idx) else {
            return Ok(None);
        };

        match &snapshot.content_before {
            Some(content) => {
                tokio::fs::write(&snapshot.path, content)
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!("Failed to restore file: {}", e))
                    })?;
            }
            None => match tokio::fs::remove_file(&snapshot.path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Failed to remove file during restore: {}",
                        e
                    )));
                }
            },
        }

        Ok(Some(snapshot))
    }

    /// Number of snapshots currently stored.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether the history is empty.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

/// Shared file history for injection into tools.
pub type SharedFileHistory = Arc<RwLock<FileHistory>>;

/// Create a new shared file history.
pub fn shared_file_history() -> SharedFileHistory {
    Arc::new(RwLock::new(FileHistory::new()))
}

/// Tool for undoing file modifications by restoring from history.
pub struct FileUndoTool {
    history: SharedFileHistory,
    base_dir: Option<PathBuf>,
}

impl FileUndoTool {
    pub fn new(history: SharedFileHistory) -> Self {
        Self {
            history,
            base_dir: None,
        }
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }
}

impl std::fmt::Debug for FileUndoTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileUndoTool")
            .field("base_dir", &self.base_dir)
            .finish()
    }
}

#[async_trait]
impl Tool for FileUndoTool {
    fn name(&self) -> &str {
        "file_undo"
    }

    fn description(&self) -> &str {
        "Undo the most recent modification to a file by restoring its previous content. \
         Only works for changes made by apply_patch or write_file in the current session."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to restore"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = require_str(&params, "path")?;
        let start = std::time::Instant::now();

        let path = validate_path(path_str, self.base_dir.as_deref())?;

        let mut history = self.history.write().await;
        let restored = history.restore_latest(ctx.job_id, &path).await?;

        match restored {
            Some(snapshot) => {
                let result = serde_json::json!({
                    "path": path.display().to_string(),
                    "restored_from_turn": snapshot.turn_number,
                    "tool_that_modified": snapshot.tool_name,
                    "snapshot_timestamp": snapshot.timestamp.to_rfc3339(),
                    "success": true
                });
                Ok(ToolOutput::success(result, start.elapsed()))
            }
            None => Err(ToolError::ExecutionFailed(format!(
                "No file history found for {}. Only changes made by apply_patch or \
                 write_file in the current session can be undone.",
                path.display()
            ))),
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_job_id() -> Uuid {
        Uuid::new_v4()
    }

    #[tokio::test]
    async fn test_snapshot_and_restore() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "original content").unwrap();
        let job = test_job_id();

        let mut history = FileHistory::new();

        // Take snapshot
        let id = history
            .snapshot(job, &file_path, "apply_patch", 1)
            .await
            .unwrap();
        assert!(id.is_some());

        // Modify file externally (simulating what apply_patch would do)
        std::fs::write(&file_path, "modified content").unwrap();

        // Restore
        let restored = history.restore_latest(job, &file_path).await.unwrap();
        assert!(restored.is_some());
        let snapshot = restored.unwrap();
        assert_eq!(snapshot.tool_name, "apply_patch");
        assert_eq!(snapshot.turn_number, 1);

        // File should be back to original
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "original content");
    }

    #[tokio::test]
    async fn test_multiple_snapshots_same_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "v1").unwrap();
        let job = test_job_id();

        let mut history = FileHistory::new();
        history
            .snapshot(job, &file_path, "apply_patch", 1)
            .await
            .unwrap();

        std::fs::write(&file_path, "v2").unwrap();
        history
            .snapshot(job, &file_path, "apply_patch", 2)
            .await
            .unwrap();

        std::fs::write(&file_path, "v3").unwrap();

        // Restore should go to v2 (latest snapshot)
        let restored = history.restore_latest(job, &file_path).await.unwrap();
        assert!(restored.is_some());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "v2");

        // Restore again should go to v1
        let restored = history.restore_latest(job, &file_path).await.unwrap();
        assert!(restored.is_some());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "v1");
    }

    #[tokio::test]
    async fn test_max_snapshots_eviction() {
        let dir = TempDir::new().unwrap();
        let mut history = FileHistory::with_max_snapshots(3);
        let job = test_job_id();

        for i in 0..5 {
            let file_path = dir.path().join(format!("file{}.txt", i));
            std::fs::write(&file_path, format!("content {}", i)).unwrap();
            history
                .snapshot(job, &file_path, "write_file", i)
                .await
                .unwrap();
        }

        // Should only keep 3 most recent
        assert_eq!(history.len(), 3);

        // Oldest (file0, file1) should be gone
        let file0 = dir.path().join("file0.txt");
        assert!(history.latest_snapshot_for(job, &file0).is_none());

        // Newest (file4) should still be there
        let file4 = dir.path().join("file4.txt");
        assert!(history.latest_snapshot_for(job, &file4).is_some());
    }

    #[tokio::test]
    async fn test_snapshot_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("does_not_exist.txt");
        let job = test_job_id();

        let mut history = FileHistory::new();
        let id = history
            .snapshot(job, &file_path, "write_file", 1)
            .await
            .unwrap();

        assert!(id.is_some());
        assert_eq!(history.len(), 1);
        let snapshot = history.latest_snapshot_for(job, &file_path).unwrap();
        assert!(snapshot.content_before.is_none());
    }

    #[tokio::test]
    async fn test_restore_removes_newly_created_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("new_file.txt");
        let job = test_job_id();

        let mut history = FileHistory::new();
        history
            .snapshot(job, &file_path, "write_file", 1)
            .await
            .unwrap();

        std::fs::write(&file_path, "created later").unwrap();
        assert!(file_path.exists());

        let restored = history.restore_latest(job, &file_path).await.unwrap();
        assert!(restored.is_some());
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn test_restore_to_specific_turn() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        let job = test_job_id();

        std::fs::write(&file_path, "turn1").unwrap();
        let mut history = FileHistory::new();
        history
            .snapshot(job, &file_path, "apply_patch", 1)
            .await
            .unwrap();

        std::fs::write(&file_path, "turn2").unwrap();
        history
            .snapshot(job, &file_path, "apply_patch", 2)
            .await
            .unwrap();

        std::fs::write(&file_path, "turn3").unwrap();

        // Get all snapshots for the file
        let snapshots = history.snapshots_for(job, &file_path);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].turn_number, 2); // newest first
        assert_eq!(snapshots[1].turn_number, 1);
    }

    #[tokio::test]
    async fn test_file_undo_tool_execute() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("code.rs");
        std::fs::write(&file_path, "fn main() {}").unwrap();

        let history = shared_file_history();
        let ctx = JobContext::default();

        // Simulate a modification with snapshot
        {
            let mut h = history.write().await;
            h.snapshot(ctx.job_id, &file_path, "apply_patch", 1)
                .await
                .unwrap();
        }
        std::fs::write(&file_path, "fn main() { println!(\"hello\"); }").unwrap();

        let tool = FileUndoTool::new(Arc::clone(&history)).with_base_dir(dir.path().to_path_buf());

        // Reuse the same ctx so file_undo sees the same job_id as the snapshot
        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "fn main() {}");
    }

    #[tokio::test]
    async fn test_file_undo_no_history() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "content").unwrap();

        let history = shared_file_history();
        let tool = FileUndoTool::new(history).with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("No file history found"));
    }

    #[tokio::test]
    async fn test_integration_patch_then_undo() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("main.rs");
        let original = "fn greet() {\n    println!(\"hello\");\n}\n";
        std::fs::write(&file_path, original).unwrap();

        let history = shared_file_history();
        let ctx = JobContext::default();

        // Step 1: Snapshot before "patching"
        {
            let mut h = history.write().await;
            h.snapshot(ctx.job_id, &file_path, "apply_patch", 1)
                .await
                .unwrap();
        }

        // Step 2: Simulate patch
        let modified = "fn greet() {\n    println!(\"goodbye\");\n}\n";
        std::fs::write(&file_path, modified).unwrap();

        // Step 3: Verify modified
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, modified);

        // Step 4: Undo via tool (reuse same ctx so job_id matches)
        let tool = FileUndoTool::new(Arc::clone(&history)).with_base_dir(dir.path().to_path_buf());

        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());

        // Step 5: Content should be original
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, original);
    }
}
