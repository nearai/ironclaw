//! File operation tools for reading, writing, and navigating the filesystem.
//!
//! These tools provide controlled access to the filesystem with:
//! - Path validation and sandboxing
//! - Size limits on read/write operations
//! - Support for common development tasks

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_safety::sensitive_paths::is_sensitive_path;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::sync::RwLock;

use crate::context::JobContext;
use crate::tools::builtin::file_edit_guard::{self, MatchMethod, SharedReadFileState};
use crate::tools::builtin::file_history::FileHistory;
use crate::tools::builtin::path_utils::{DEFAULT_EXCLUDED_DIRS, validate_path};
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};
use crate::workspace::paths as ws_paths;

/// Well-known workspace filenames that must go through memory_write, not write_file.
///
/// If the LLM tries to write one of these via the filesystem tool we reject
/// immediately and point it at the correct tool.
const WORKSPACE_FILES: &[&str] = &[
    ws_paths::HEARTBEAT,
    ws_paths::MEMORY,
    ws_paths::IDENTITY,
    ws_paths::SOUL,
    ws_paths::AGENTS,
    ws_paths::USER,
    ws_paths::README,
];

/// Check whether `path` resolves to a workspace file that should be written
/// through `memory_write` instead of `write_file`.
fn is_workspace_path(path: &str) -> bool {
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(path);

    WORKSPACE_FILES.contains(&filename)
        || path.starts_with("daily/")
        || path.starts_with("context/")
}

/// Maximum file size for reading (10MB).
const MAX_READ_SIZE: u64 = 10 * 1024 * 1024;

/// Default line limit when no offset/limit is specified.
const DEFAULT_LINE_LIMIT: usize = 2000;

/// Device paths that must not be read (would hang or produce infinite output).
const BLOCKED_DEVICE_PATHS: &[&str] = &[
    "/dev/zero",
    "/dev/urandom",
    "/dev/random",
    "/proc/kcore",
    "/proc/kmem",
    "/dev/null",
    "/dev/stdin",
    "/dev/stdout",
    "/dev/stderr",
];

/// Maximum file size for apply_patch operations (10MB).
const MAX_PATCH_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum file size for writing (5MB).
const MAX_WRITE_SIZE: usize = 5 * 1024 * 1024;

/// Maximum directory listing entries.
const MAX_DIR_ENTRIES: usize = 500;

/// Read file contents tool.
#[derive(Debug, Default)]
pub struct ReadFileTool {
    base_dir: Option<PathBuf>,
    read_state: Option<SharedReadFileState>,
}

impl ReadFileTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_read_state(mut self, state: SharedReadFileState) -> Self {
        self.read_state = Some(state);
        self
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file from the LOCAL FILESYSTEM. **Always read a file before editing it.** \
         NOT for workspace memory paths (use memory_read for those). \
         Returns content with line numbers. Default limit is 2000 lines. \
         For large files, use offset and limit for partial reads."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-indexed, optional)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (optional)"
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

        let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = params.get("limit").and_then(|v| v.as_u64());
        let has_explicit_range = offset > 0 || limit.is_some();

        let start = std::time::Instant::now();

        let path = validate_path(path_str, self.base_dir.as_deref())?;

        // Check sensitive path blocklist (after canonicalization in validate_path)
        if is_sensitive_path(&path) {
            return Err(ToolError::ExecutionFailed(
                "Access denied: this file may contain credentials. \
                 Use `secret_list` and `secret_create` to manage credentials securely."
                    .to_string(),
            ));
        }

        // Block device paths that would hang or produce infinite output.
        // Check the resolved path to prevent bypasses via relative paths or symlinks.
        let resolved_str = path.to_string_lossy();
        if BLOCKED_DEVICE_PATHS
            .iter()
            .any(|p| resolved_str.starts_with(p))
            || (resolved_str.starts_with("/proc/") && resolved_str.contains("/fd/"))
        {
            return Err(ToolError::InvalidParameters(format!(
                "Reading device/proc paths is not allowed: {}",
                path.display()
            )));
        }

        // Check file size
        let metadata = fs::metadata(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Cannot access file: {}", e)))?;

        if metadata.len() > MAX_READ_SIZE {
            return Err(ToolError::ExecutionFailed(format!(
                "File too large ({} bytes). Maximum is {} bytes. Use offset/limit for partial reads.",
                metadata.len(),
                MAX_READ_SIZE
            )));
        }

        // Binary file detection: read first 8KB and check for null bytes,
        // but skip the check for UTF-16LE files (which legitimately contain null bytes).
        {
            let probe_size = 8192u64.min(metadata.len()) as usize;
            if probe_size > 0 {
                let mut f = tokio::fs::File::open(&path)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Cannot open file: {}", e)))?;
                let mut probe = vec![0u8; probe_size];
                let n = f
                    .read(&mut probe)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Cannot read file: {}", e)))?;
                let is_utf16le = n >= 2 && probe[0] == 0xFF && probe[1] == 0xFE;
                if !is_utf16le && probe[..n].contains(&0) {
                    return Err(ToolError::ExecutionFailed(format!(
                        "File appears to be binary (contains null bytes): {}",
                        path.display()
                    )));
                }
            }
        }

        // Read file using encoding-aware path (handles UTF-16LE with BOM)
        let (content, _encoding, _line_ending) =
            file_edit_guard::read_file_with_encoding(&path).await?;

        // Apply offset and limit
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start_line = if offset > 0 {
            offset.saturating_sub(1)
        } else {
            0
        };

        let (end_line, truncated_by_default) = if let Some(lim) = limit {
            ((start_line + lim as usize).min(total_lines), false)
        } else if !has_explicit_range && total_lines > DEFAULT_LINE_LIMIT {
            // Apply default 2000-line limit when no offset/limit specified
            (DEFAULT_LINE_LIMIT.min(total_lines), true)
        } else {
            (total_lines, false)
        };

        let selected_lines: Vec<String> = lines[start_line..end_line]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>6}│ {}", start_line + i + 1, line))
            .collect();

        // Record the read for staleness detection.
        // Mark as partial when the user provided an explicit range OR the default
        // 2000-line cap truncated the output — either way the caller hasn't seen
        // the full file, so edits should require a full read first.
        if let Some(ref read_state) = self.read_state {
            let mtime = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
            let partial = has_explicit_range || truncated_by_default;
            let mut state = read_state.write().await;
            state.record_read(ctx.job_id, &path, mtime, partial);
        }

        let result = serde_json::json!({
            "content": selected_lines.join("\n"),
            "total_lines": total_lines,
            "lines_shown": end_line - start_line,
            "truncated_by_default": truncated_by_default,
            "path": path.display().to_string()
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // File content could contain anything
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }
}

/// Write file contents tool.
#[derive(Debug, Default)]
pub struct WriteFileTool {
    base_dir: Option<PathBuf>,
    file_history: Option<Arc<RwLock<FileHistory>>>,
    read_state: Option<SharedReadFileState>,
}

impl WriteFileTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_file_history(mut self, history: Arc<RwLock<FileHistory>>) -> Self {
        self.file_history = Some(history);
        self
    }

    pub fn with_read_state(mut self, state: SharedReadFileState) -> Self {
        self.read_state = Some(state);
        self
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file on the LOCAL FILESYSTEM. **Only use for creating new files \
         or complete rewrites.** For targeted edits, use apply_patch instead — it's safer \
         and more efficient. NOT for workspace memory (use memory_write for that). \
         Creates parent directories automatically."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = require_str(&params, "path")?;

        // Reject workspace paths: these live in the database, not on disk.
        if is_workspace_path(path_str) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is a workspace memory file. Use the memory_write tool instead of write_file. \
                 For HEARTBEAT.md use target='heartbeat', for MEMORY.md use target='memory'.",
                path_str
            )));
        }

        let content = require_str(&params, "content")?;

        let start = std::time::Instant::now();

        // Check content size
        if content.len() > MAX_WRITE_SIZE {
            return Err(ToolError::InvalidParameters(format!(
                "Content too large ({} bytes). Maximum is {} bytes.",
                content.len(),
                MAX_WRITE_SIZE
            )));
        }

        let path = validate_path(path_str, self.base_dir.as_deref())?;

        // Block writes to sensitive credential files
        if is_sensitive_path(&path) {
            return Err(ToolError::ExecutionFailed(
                "Access denied: cannot write to credential files. \
                 Use `secret_list` and `secret_create` to manage credentials securely."
                    .to_string(),
            ));
        }

        // Staleness check for existing files — log a warning but don't hard-fail.
        // write_file replaces the entire file content, so the risk of overwriting
        // unseen content is lower than apply_patch (which does substring replacement).
        // Use async metadata instead of blocking path.exists().
        if let Ok(existing_meta) = fs::metadata(&path).await
            && let Some(ref read_state) = self.read_state
        {
            let current_mtime = existing_meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            let state = read_state.read().await;
            if let Err(e) = state.check_before_edit(ctx.job_id, &path, current_mtime) {
                tracing::debug!("write_file staleness warning: {}", e);
            }
        }

        // Snapshot existing file before overwriting (for file_undo)
        if let Some(ref history) = self.file_history {
            let mut h = history.write().await;
            if let Err(e) = h.snapshot(ctx.job_id, &path, "write_file").await {
                tracing::debug!("file_history snapshot failed before write_file: {}", e);
            }
        }

        // Create parent directories
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to create directories: {}", e))
            })?;
        }

        // Write file
        fs::write(&path, content)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write file: {}", e)))?;

        // Update read state mtime so subsequent edits don't see stale timestamps
        if let Some(ref read_state) = self.read_state
            && let Ok(m) = fs::metadata(&path).await
            && let Ok(mtime) = m.modified()
        {
            let mut state = read_state.write().await;
            state.update_mtime(ctx.job_id, &path, mtime);
        }

        let result = serde_json::json!({
            "path": path.display().to_string(),
            "bytes_written": content.len(),
            "success": true
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false // We're writing, not reading external data
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

/// List directory contents tool.
#[derive(Debug, Default)]
pub struct ListDirTool {
    base_dir: Option<PathBuf>,
}

impl ListDirTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List contents of a directory on the LOCAL FILESYSTEM. NOT for workspace memory \
         (use memory_tree for that). Shows files and subdirectories with their sizes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list (defaults to current directory)"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "If true, list contents recursively (default false)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum depth for recursive listing (default 3)"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let recursive = params
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_depth = params
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;

        let start = std::time::Instant::now();

        let path = validate_path(path_str, self.base_dir.as_deref())?;

        // Block listing sensitive credential directories
        if is_sensitive_path(&path) {
            return Err(ToolError::ExecutionFailed(
                "Access denied: this directory may contain credentials. \
                 Use `secret_list` and `secret_create` to manage credentials securely."
                    .to_string(),
            ));
        }

        let mut entries = Vec::new();
        list_dir_inner(&path, &path, recursive, max_depth, 0, &mut entries).await?;

        // Sort entries
        entries.sort_by(|a, b| {
            let a_is_dir = a.ends_with('/');
            let b_is_dir = b.ends_with('/');
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.cmp(b),
            }
        });

        let truncated = entries.len() > MAX_DIR_ENTRIES;
        if truncated {
            entries.truncate(MAX_DIR_ENTRIES);
        }

        let result = serde_json::json!({
            "path": path.display().to_string(),
            "entries": entries,
            "count": entries.len(),
            "truncated": truncated
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Directory listings are safe
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }
}

/// Recursively list directory contents.
async fn list_dir_inner(
    base: &Path,
    path: &Path,
    recursive: bool,
    max_depth: usize,
    current_depth: usize,
    entries: &mut Vec<String>,
) -> Result<(), ToolError> {
    if entries.len() >= MAX_DIR_ENTRIES {
        return Ok(());
    }

    let mut dir = fs::read_dir(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read directory: {}", e)))?;

    while let Some(entry) = dir
        .next_entry()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read entry: {}", e)))?
    {
        if entries.len() >= MAX_DIR_ENTRIES {
            break;
        }

        let entry_path = entry.path();
        let relative = entry_path
            .strip_prefix(base)
            .unwrap_or(&entry_path)
            .to_string_lossy();

        let metadata = entry.metadata().await.ok();
        let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());

        let display = if is_dir {
            format!("{}/", relative)
        } else {
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            format!("{} ({})", relative, format_size(size))
        };

        entries.push(display);

        if recursive && is_dir && current_depth < max_depth {
            // Skip sensitive credential directories during recursive traversal
            if is_sensitive_path(&entry_path) {
                // Replace the last entry with an annotated version so the user
                // knows the directory was intentionally skipped.
                if let Some(last) = entries.last_mut() {
                    *last = format!("{} [sensitive - access blocked]", relative);
                }
                continue;
            }

            // Skip common non-essential directories
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !DEFAULT_EXCLUDED_DIRS.contains(&name_str.as_ref()) {
                Box::pin(list_dir_inner(
                    base,
                    &entry_path,
                    recursive,
                    max_depth,
                    current_depth + 1,
                    entries,
                ))
                .await?;
            }
        }
    }

    Ok(())
}

/// Format file size in human-readable form.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Apply patch tool for targeted file edits.
#[derive(Debug, Default)]
pub struct ApplyPatchTool {
    base_dir: Option<PathBuf>,
    file_history: Option<Arc<RwLock<FileHistory>>>,
    read_state: Option<SharedReadFileState>,
}

impl ApplyPatchTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_file_history(mut self, history: Arc<RwLock<FileHistory>>) -> Self {
        self.file_history = Some(history);
        self
    }

    pub fn with_read_state(mut self, state: SharedReadFileState) -> Self {
        self.read_state = Some(state);
        self
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply targeted edits to a file using search/replace. **Prefer this over write_file** \
         for modifying existing files — it sends only the changed portion. \
         The old_string must match exactly (including whitespace and indentation). \
         The edit will FAIL if old_string is not unique — provide more context to make it unique, \
         or set replace_all=true. **You must read_file before editing.** \
         When editing text from read_file output, preserve the exact indentation (tabs/spaces) \
         as it appears after the line number prefix."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace (must be unique in the file unless replace_all=true)"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace it with (must differ from old_string)"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace all occurrences (default false, replaces first only)"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = require_str(&params, "path")?;

        // Reject workspace paths
        if is_workspace_path(path_str) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is a workspace memory file. Use the memory_write tool instead of apply_patch.",
                path_str
            )));
        }

        let old_string = require_str(&params, "old_string")?;
        let new_string = require_str(&params, "new_string")?;

        // Reject no-op edits
        if old_string == new_string {
            return Err(ToolError::InvalidParameters(
                "old_string and new_string are identical. No edit needed.".to_string(),
            ));
        }

        let replace_all = params
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let start = std::time::Instant::now();

        let path = validate_path(path_str, self.base_dir.as_deref())?;

        // Block patches to sensitive credential files
        if is_sensitive_path(&path) {
            return Err(ToolError::ExecutionFailed(
                "Access denied: cannot modify credential files. \
                 Use `secret_list` and `secret_create` to manage credentials securely."
                    .to_string(),
            ));
        }

        // Check file size
        let metadata = fs::metadata(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Cannot access file: {}", e)))?;

        if metadata.len() > MAX_PATCH_SIZE {
            return Err(ToolError::ExecutionFailed(format!(
                "File too large ({} bytes). Maximum for apply_patch is {} bytes.",
                metadata.len(),
                MAX_PATCH_SIZE
            )));
        }

        // Check read-before-edit guard and staleness
        if let Some(ref read_state) = self.read_state {
            let current_mtime = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
            let state = read_state.read().await;
            state.check_before_edit(ctx.job_id, &path, current_mtime)?;
        }

        // Read current content with encoding detection
        let (content, encoding, line_ending) =
            file_edit_guard::read_file_with_encoding(&path).await?;

        // Try to find the old_string, with fuzzy matching fallbacks
        let (match_count, match_method) = file_edit_guard::count_matches(&content, old_string);
        if match_count == 0 {
            let preview = if old_string.len() > 200 {
                let truncated: String = old_string.chars().take(200).collect();
                format!("{truncated}...")
            } else {
                old_string.to_string()
            };
            return Err(ToolError::ExecutionFailed(format!(
                "String to replace not found in {}.\n\
                 old_string:\n{}\n\n\
                 Make sure old_string matches the file content exactly, \
                 including whitespace and indentation.",
                path.display(),
                preview
            )));
        }

        // Uniqueness validation
        if !replace_all && match_count > 1 {
            return Err(ToolError::ExecutionFailed(format!(
                "Found {} matches for the specified text in {}. \
                 Provide more context in old_string to make it unique, or set replace_all=true.",
                match_count,
                path.display()
            )));
        }

        // Snapshot before modification (for file_undo)
        if let Some(ref history) = self.file_history {
            let mut h = history.write().await;
            if let Err(e) = h.snapshot(ctx.job_id, &path, "apply_patch").await {
                tracing::debug!("file_history snapshot failed before apply_patch: {}", e);
            }
        }

        // Apply replacement using actual match ranges so fuzzy replace_all
        // updates every normalized variant, not only the first byte-identical one.
        let new_content = if replace_all {
            let mut matches = Vec::new();
            let mut search_offset = 0usize;
            while let Some(m) =
                file_edit_guard::find_match_from(&content, old_string, search_offset)
            {
                if m.end <= m.start {
                    return Err(ToolError::ExecutionFailed(
                        "Internal error: apply_patch produced an empty match.".to_string(),
                    ));
                }
                matches.push((m.start, m.end));
                search_offset = m.end;
            }

            if matches.len() != match_count {
                return Err(ToolError::ExecutionFailed(format!(
                    "Internal error: expected {} matches in {}, but found {} while applying replacements.",
                    match_count,
                    path.display(),
                    matches.len()
                )));
            }

            let mut rebuilt = String::with_capacity(content.len());
            let mut last = 0usize;
            for (start, end) in matches {
                rebuilt.push_str(&content[last..start]);
                rebuilt.push_str(new_string);
                last = end;
            }
            rebuilt.push_str(&content[last..]);
            rebuilt
        } else {
            let m = file_edit_guard::find_match(&content, old_string).ok_or_else(|| {
                ToolError::ExecutionFailed(format!(
                    "String to replace not found in {} while applying the edit.",
                    path.display()
                ))
            })?;

            let mut rebuilt =
                String::with_capacity(content.len() - m.actual.len() + new_string.len());
            rebuilt.push_str(&content[..m.start]);
            rebuilt.push_str(new_string);
            rebuilt.push_str(&content[m.end..]);
            rebuilt
        };

        let replacements = if replace_all { match_count } else { 1 };

        // Write back with original encoding and line endings
        file_edit_guard::write_file_with_encoding(&path, &new_content, encoding, line_ending)
            .await?;

        // Update read state mtime
        if let Some(ref read_state) = self.read_state
            && let Ok(m) = fs::metadata(&path).await
            && let Ok(mtime) = m.modified()
        {
            let mut state = read_state.write().await;
            state.update_mtime(ctx.job_id, &path, mtime);
        }

        let mut result = serde_json::json!({
            "path": path.display().to_string(),
            "replacements": replacements,
            "success": true
        });
        if match_method != MatchMethod::Exact {
            result["match_method"] = serde_json::json!(format!("{:?}", match_method));
        }

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false // We're writing, not reading external data
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::builtin::path_utils::normalize_lexical;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_read_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line 1\nline 2\nline 3\n").unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let content = result.result.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("line 1"));
        assert!(content.contains("line 2"));
    }

    #[tokio::test]
    async fn test_write_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("new_file.txt");

        let tool = WriteFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "content": "hello world"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_apply_patch() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("code.rs");
        std::fs::write(&file_path, "fn main() {\n    println!(\"old\");\n}\n").unwrap();

        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "println!(\"old\")",
                    "new_string": "println!(\"new\")"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("println!(\"new\")"));
    }

    #[tokio::test]
    async fn test_write_file_rejects_workspace_paths() {
        let dir = TempDir::new().unwrap();
        let tool = WriteFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let workspace_files = &[
            "HEARTBEAT.md",
            "MEMORY.md",
            "IDENTITY.md",
            "SOUL.md",
            "AGENTS.md",
            "USER.md",
            "README.md",
        ];

        for filename in workspace_files {
            let path = dir.path().join(filename);
            let err = tool
                .execute(
                    serde_json::json!({
                        "path": path.to_str().unwrap(),
                        "content": "test"
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();

            let msg = err.to_string();
            assert!(
                msg.contains("memory_write"),
                "Rejection for {} should mention memory_write, got: {}",
                filename,
                msg
            );
        }

        // daily/ and context/ prefixes should also be rejected
        for prefix_path in &["daily/2024-01-15.md", "context/vision.md"] {
            let err = tool
                .execute(
                    serde_json::json!({
                        "path": prefix_path,
                        "content": "test"
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();

            assert!(
                err.to_string().contains("memory_write"),
                "Rejection for {} should mention memory_write",
                prefix_path
            );
        }

        // Regular files should still work
        let regular_path = dir.path().join("normal.txt");
        let result = tool
            .execute(
                serde_json::json!({
                    "path": regular_path.to_str().unwrap(),
                    "content": "fine"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file1.txt"), "content").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let tool = ListDirTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": dir.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let entries = result.result.get("entries").unwrap().as_array().unwrap();
        assert!(entries.len() >= 2);
    }

    #[test]
    fn test_normalize_lexical() {
        // Basic .. resolution
        assert_eq!(
            normalize_lexical(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        // Multiple .. components
        assert_eq!(
            normalize_lexical(Path::new("/a/b/c/../../d")),
            PathBuf::from("/a/d")
        );
        // . components stripped
        assert_eq!(
            normalize_lexical(Path::new("/a/./b/./c")),
            PathBuf::from("/a/b/c")
        );
        // Cannot escape root
        assert_eq!(
            normalize_lexical(Path::new("/a/../../..")),
            PathBuf::from("/")
        );
    }

    #[test]
    fn test_validate_path_rejects_traversal_nonexistent_parent() {
        // The critical test: writing to ../../outside/newdir/file with base_dir
        // set should be rejected even when the parent directory does not exist
        // (i.e. canonicalize() cannot resolve it).
        let dir = TempDir::new().unwrap();
        let evil_path = format!(
            "{}/../../outside/newdir/file.txt",
            dir.path().to_str().unwrap()
        );
        let result = validate_path(&evil_path, Some(dir.path()));
        assert!(
            result.is_err(),
            "Should reject traversal via non-existent parent, got: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_rejects_relative_traversal() {
        let dir = TempDir::new().unwrap();
        let result = validate_path("../../etc/passwd", Some(dir.path()));
        assert!(
            result.is_err(),
            "Should reject relative traversal, got: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_allows_valid_nested_write() {
        let dir = TempDir::new().unwrap();
        let result = validate_path("subdir/newfile.txt", Some(dir.path()));
        assert!(
            result.is_ok(),
            "Should allow nested writes within sandbox: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_allows_dot_dot_within_sandbox() {
        // a/b/../c resolves to a/c which is still inside the sandbox
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        let result = validate_path("a/b/../c.txt", Some(dir.path()));
        assert!(
            result.is_ok(),
            "Should allow .. that stays within sandbox: {:?}",
            result
        );
    }

    // Unit tests for is_sensitive_path live in crates/ironclaw_safety/src/sensitive_paths.rs.
    // Only integration-level tests that exercise the tool's execute() method belong here.

    #[tokio::test]
    async fn test_list_dir_blocks_sensitive_directory() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let ssh_dir = tmp.path().join(".ssh");
        std::fs::create_dir(&ssh_dir).expect("create .ssh dir");
        std::fs::write(ssh_dir.join("id_rsa"), "fake-key").expect("write fake key");

        let tool = ListDirTool::new();
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({"path": ssh_dir.to_string_lossy().as_ref()}),
                &ctx,
            )
            .await;

        assert!(err.is_err(), "listing a .ssh directory should be blocked");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("Access denied") || msg.contains("credentials"),
            "expected sensitive path error, got: {msg}"
        );
    }

    // --- ReadFileTool enhancement tests ---

    #[tokio::test]
    async fn test_read_file_default_limit_2000() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("big.txt");
        let content: String = (1..=3000).map(|i| format!("line {}\n", i)).collect();
        std::fs::write(&file_path, &content).unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let lines_shown = result.result.get("lines_shown").unwrap().as_u64().unwrap();
        assert_eq!(lines_shown, 2000);
        assert!(
            result
                .result
                .get("truncated_by_default")
                .unwrap()
                .as_bool()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_read_file_explicit_limit_overrides() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        let content: String = (1..=500).map(|i| format!("line {}\n", i)).collect();
        std::fs::write(&file_path, &content).unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap(), "limit": 100}),
                &ctx,
            )
            .await
            .unwrap();

        let lines_shown = result.result.get("lines_shown").unwrap().as_u64().unwrap();
        assert_eq!(lines_shown, 100);
        assert!(
            !result
                .result
                .get("truncated_by_default")
                .unwrap()
                .as_bool()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_read_file_binary_rejected() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("binary.bin");
        let mut content = vec![0u8; 100];
        content[50] = 0; // null byte
        content[0] = b'H';
        content[1] = b'i';
        std::fs::write(&file_path, &content).unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("binary"));
    }

    #[tokio::test]
    async fn test_read_file_blocks_dev_paths() {
        let tool = ReadFileTool::new();
        let ctx = JobContext::default();

        for dev_path in &["/dev/zero", "/dev/urandom", "/dev/null"] {
            let err = tool
                .execute(serde_json::json!({"path": dev_path}), &ctx)
                .await
                .unwrap_err();

            assert!(
                err.to_string().contains("not allowed"),
                "Should block {}: {}",
                dev_path,
                err
            );
        }
    }

    #[tokio::test]
    async fn test_read_file_blocks_proc_fd() {
        let tool = ReadFileTool::new();
        let ctx = JobContext::default();

        let err = tool
            .execute(serde_json::json!({"path": "/proc/self/fd/0"}), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("not allowed"));
    }

    #[tokio::test]
    async fn test_read_file_truncated_flag() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("small.txt");
        std::fs::write(&file_path, "just one line\n").unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            !result
                .result
                .get("truncated_by_default")
                .unwrap()
                .as_bool()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_read_file_utf8_emoji_not_binary() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("emoji.txt");
        std::fs::write(
            &file_path,
            "Hello \u{1F30D}! Rust is great. Caf\u{00E9} r\u{00E9}sum\u{00E9}.",
        )
        .unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await;

        assert!(
            result.is_ok(),
            "UTF-8 text with special chars should not be detected as binary"
        );
    }

    #[tokio::test]
    async fn test_read_file_offset_without_limit() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("lines.txt");
        let content: String = (1..=3000).map(|i| format!("line {}\n", i)).collect();
        std::fs::write(&file_path, &content).unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        // Explicit offset should NOT trigger the 2000-line default
        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap(), "offset": 2990}),
                &ctx,
            )
            .await
            .unwrap();

        let lines_shown = result.result.get("lines_shown").unwrap().as_u64().unwrap();
        // Should read remaining ~10 lines, not cap at 2000
        assert!(lines_shown <= 11);
        assert!(
            !result
                .result
                .get("truncated_by_default")
                .unwrap()
                .as_bool()
                .unwrap()
        );
    }

    // --- ApplyPatchTool enhancement tests ---

    #[tokio::test]
    async fn test_apply_patch_rejects_workspace_paths() {
        let dir = TempDir::new().unwrap();
        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        for filename in &["MEMORY.md", "HEARTBEAT.md", "daily/2024-01-15.md"] {
            let err = tool
                .execute(
                    serde_json::json!({
                        "path": filename,
                        "old_string": "old",
                        "new_string": "new"
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();

            assert!(
                err.to_string().contains("memory_write"),
                "Should reject workspace path {}: {}",
                filename,
                err
            );
        }
    }

    #[tokio::test]
    async fn test_apply_patch_ambiguous_match_error() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world\nhello world\nhello world\n").unwrap();

        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let err = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "hello world",
                    "new_string": "goodbye"
                }),
                &ctx,
            )
            .await
            .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("3 matches"),
            "Should report 3 matches: {}",
            msg
        );
        assert!(
            msg.contains("replace_all"),
            "Should suggest replace_all: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_apply_patch_ambiguous_with_replace_all() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world\nhello world\nhello world\n").unwrap();

        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "hello world",
                    "new_string": "goodbye",
                    "replace_all": true
                }),
                &ctx,
            )
            .await
            .unwrap();

        let replacements = result.result.get("replacements").unwrap().as_u64().unwrap();
        assert_eq!(replacements, 3);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(!content.contains("hello world"));
    }

    #[tokio::test]
    async fn test_apply_patch_replace_all_rewrites_all_fuzzy_variants() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(
            &file_path,
            "hello world  \nhello world\n\nhello world\nhello world   \n",
        )
        .unwrap();

        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "hello world\nhello world\n",
                    "new_string": "goodbye\ngoodbye\n",
                    "replace_all": true
                }),
                &ctx,
            )
            .await
            .unwrap();

        let replacements = result.result.get("replacements").unwrap().as_u64().unwrap();
        assert_eq!(replacements, 2);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(!content.contains("hello world"));
        assert!(content.contains("goodbye\ngoodbye\n"));
    }

    #[tokio::test]
    async fn test_apply_patch_single_match_succeeds() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "unique line\nanother line\n").unwrap();

        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "unique line",
                    "new_string": "replaced line"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_apply_patch_overlapping_pattern() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "aaa").unwrap();

        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        // "aa" appears twice in "aaa" (overlapping), but str::matches counts non-overlapping
        // which is 1 match. So this should succeed.
        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "aa",
                    "new_string": "bb"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
    }
}
