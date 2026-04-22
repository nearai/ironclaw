//! Content search tool powered by ripgrep.
//!
//! Provides structured code search with multiple output modes, pagination,
//! and regex support. Shells out to `rg` (ripgrep) rather than embedding
//! a search engine, consistent with how `ShellTool` delegates to `sh`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::process::Command;

use ironclaw_safety::sensitive_paths::is_sensitive_path;

use crate::context::JobContext;
use crate::tools::builtin::path_utils::validate_path;
use crate::tools::builtin::shell::SAFE_ENV_VARS;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDiscoverySummary, ToolDomain, ToolError, ToolOutput, require_str,
};

/// Maximum output size before truncation (64KB, same as ShellTool).
const MAX_OUTPUT_SIZE: usize = 64 * 1024;

/// Default head limit for output lines/entries.
const DEFAULT_HEAD_LIMIT: usize = 250;

/// Grep tool for searching file contents using ripgrep.
#[derive(Debug, Default)]
pub struct GrepTool {
    base_dir: Option<PathBuf>,
}

impl GrepTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }
}

/// Build a safe environment map with only allowed variables.
fn safe_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    for &key in SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.to_string(), val);
        }
    }
    env
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regex patterns. Powered by ripgrep. \
         Three output modes: content (matching lines with context), \
         files_with_matches (file paths only, default), count (match counts per file). \
         Use this instead of shell with 'grep' or 'rg'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to current directory)"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. '*.rs', '*.{ts,tsx}')"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output mode: content (matching lines), files_with_matches (paths only, default), count (match counts)"
                },
                "context": {
                    "type": "integer",
                    "description": "Lines of context before and after each match (-C)"
                },
                "before_context": {
                    "type": "integer",
                    "description": "Lines of context before each match (-B)"
                },
                "after_context": {
                    "type": "integer",
                    "description": "Lines of context after each match (-A)"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case insensitive search (default false)"
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Maximum output lines/entries (default 250, pass 0 for unlimited)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip first N lines/entries (default 0)"
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Enable multiline matching where . matches newlines"
                },
                "type_filter": {
                    "type": "string",
                    "description": "File type filter (e.g. 'py', 'js', 'rust')"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let pattern = require_str(&params, "pattern")?;
        let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let glob_filter = params.get("glob").and_then(|v| v.as_str());
        let output_mode = params
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches");
        let context = params.get("context").and_then(|v| v.as_u64());
        let before_context = params.get("before_context").and_then(|v| v.as_u64());
        let after_context = params.get("after_context").and_then(|v| v.as_u64());
        let case_insensitive = params
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let head_limit = params
            .get("head_limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let multiline = params
            .get("multiline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let type_filter = params.get("type_filter").and_then(|v| v.as_str());

        let start = std::time::Instant::now();

        // Validate search path
        let search_path = validate_path(path_str, self.base_dir.as_deref())?;
        if is_sensitive_path(&search_path) {
            return Err(ToolError::ExecutionFailed(
                "Access denied: search path may contain credentials. \
                 Use `secret_list` and `secret_create` to manage credentials securely."
                    .to_string(),
            ));
        }

        // Build rg command
        let mut cmd = Command::new("rg");
        cmd.env_clear();
        for (key, val) in safe_env() {
            cmd.env(&key, &val);
        }

        // Re-inject explicitly approved environment from the job context,
        // mirroring ShellTool's behavior so tools can access required credentials.
        for (key, val) in _ctx.extra_env.as_ref() {
            cmd.env(key, val);
        }

        // Always-on flags
        cmd.arg("--color").arg("never");
        cmd.arg("--no-heading");
        cmd.arg("--glob").arg("!.git");
        cmd.arg("--glob").arg("!node_modules");
        cmd.arg("--glob").arg("!target");

        // Output mode
        match output_mode {
            "files_with_matches" => {
                cmd.arg("--files-with-matches");
            }
            "count" => {
                cmd.arg("--count");
            }
            "content" => {
                cmd.arg("-n"); // Line numbers
            }
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "Invalid output_mode '{}'. Must be: content, files_with_matches, or count",
                    output_mode
                )));
            }
        }

        // Context lines
        if let Some(c) = context {
            cmd.arg("-C").arg(c.to_string());
        } else {
            if let Some(b) = before_context {
                cmd.arg("-B").arg(b.to_string());
            }
            if let Some(a) = after_context {
                cmd.arg("-A").arg(a.to_string());
            }
        }

        // Flags
        if case_insensitive {
            cmd.arg("-i");
        }
        if multiline {
            cmd.arg("-U");
            cmd.arg("--multiline-dotall");
        }

        // Filters
        if let Some(g) = glob_filter {
            cmd.arg("--glob").arg(g);
        }
        if let Some(t) = type_filter {
            cmd.arg("--type").arg(t);
        }

        // Pattern (use -e to avoid it being parsed as a flag)
        cmd.arg("-e").arg(pattern);

        // Search path
        cmd.arg(&search_path);

        // Execute with timeout
        let output = tokio::time::timeout(Duration::from_secs(30), cmd.output())
            .await
            .map_err(|_| ToolError::Timeout(Duration::from_secs(30)))?
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ToolError::ExecutionFailed(
                        "ripgrep (rg) is not installed. Install it from: \
                         https://github.com/BurntSushi/ripgrep#installation"
                            .to_string(),
                    )
                } else {
                    ToolError::ExecutionFailed(format!("Failed to execute rg: {}", e))
                }
            })?;

        // rg exit code 1 = no matches (not an error)
        // rg exit code 2 = actual error
        if output.status.code() == Some(2) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed(format!(
                "ripgrep error: {}",
                stderr.trim()
            )));
        }

        let raw_output = String::from_utf8_lossy(&output.stdout);

        // Truncate if too large
        let truncated_output = if raw_output.len() > MAX_OUTPUT_SIZE {
            // Find a safe char boundary to truncate at
            let mut end = MAX_OUTPUT_SIZE;
            while end > 0 && !raw_output.is_char_boundary(end) {
                end -= 1;
            }
            &raw_output[..end]
        } else {
            &raw_output
        };

        // Split into lines and apply pagination
        let lines: Vec<&str> = truncated_output.lines().collect();
        let effective_limit = match head_limit {
            Some(0) => lines.len(), // 0 = unlimited
            Some(n) => n,
            None => DEFAULT_HEAD_LIMIT,
        };

        let paginated: Vec<&str> = lines
            .iter()
            .skip(offset)
            .take(effective_limit)
            .copied()
            .collect();
        let was_truncated =
            raw_output.len() > MAX_OUTPUT_SIZE || lines.len() > offset + effective_limit;

        // Build output based on mode
        let result = match output_mode {
            "files_with_matches" => {
                // Collect all file entries with mtime using bounded parallelism,
                // then sort globally by mtime and paginate.
                let all_paths: Vec<String> = lines
                    .iter()
                    .map(|line| line.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect();

                let mut file_entries: Vec<(String, SystemTime)> =
                    Vec::with_capacity(all_paths.len());
                let mut join_set = tokio::task::JoinSet::new();
                let mut pending = all_paths.into_iter();
                let max_concurrency = 64usize;

                // Seed initial batch
                for path in pending.by_ref().take(max_concurrency) {
                    let sp = search_path.clone();
                    join_set.spawn(async move {
                        let mtime = tokio::fs::metadata(&path)
                            .await
                            .and_then(|m| m.modified())
                            .unwrap_or(UNIX_EPOCH);
                        let relative = std::path::Path::new(&path)
                            .strip_prefix(&sp)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| path.clone());
                        (relative, mtime)
                    });
                }

                // Drain results, refilling to maintain concurrency
                while let Some(join_result) = join_set.join_next().await {
                    if let Ok(entry) = join_result {
                        file_entries.push(entry);
                    }
                    if let Some(path) = pending.next() {
                        let sp = search_path.clone();
                        join_set.spawn(async move {
                            let mtime = tokio::fs::metadata(&path)
                                .await
                                .and_then(|m| m.modified())
                                .unwrap_or(UNIX_EPOCH);
                            let relative = std::path::Path::new(&path)
                                .strip_prefix(&sp)
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_else(|_| path.clone());
                            (relative, mtime)
                        });
                    }
                }

                file_entries.sort_by_key(|entry| std::cmp::Reverse(entry.1));

                // Apply pagination after sorting
                let total_count = file_entries.len();
                let files: Vec<String> = file_entries
                    .into_iter()
                    .skip(offset)
                    .take(effective_limit)
                    .map(|(path, _)| path)
                    .collect();
                let count = files.len();
                let was_truncated =
                    raw_output.len() > MAX_OUTPUT_SIZE || total_count > offset + effective_limit;

                serde_json::json!({
                    "files": files,
                    "count": count,
                    "truncated": was_truncated
                })
            }
            "count" => {
                let mut counts: Vec<serde_json::Value> = Vec::new();
                let mut total: u64 = 0;

                for line in &paginated {
                    if let Some((file, count_str)) = line.rsplit_once(':') {
                        let count = count_str.trim().parse::<u64>().unwrap_or(0);
                        let relative = std::path::Path::new(file)
                            .strip_prefix(&search_path)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| file.to_string());
                        total += count;
                        counts.push(serde_json::json!({
                            "file": relative,
                            "count": count
                        }));
                    }
                }

                serde_json::json!({
                    "counts": counts,
                    "total": total,
                    "truncated": was_truncated
                })
            }
            _ => {
                // "content" mode — relativize paths per-line using strip_prefix
                // to avoid false positives from substring replacement in file content
                let search_prefix = format!("{}/", search_path.display());
                let content: String = paginated
                    .iter()
                    .map(|line| line.strip_prefix(search_prefix.as_str()).unwrap_or(line))
                    .collect::<Vec<_>>()
                    .join("\n");

                serde_json::json!({
                    "content": content,
                    "truncated": was_truncated
                })
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // File content could contain anything
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn discovery_summary(&self) -> Option<ToolDiscoverySummary> {
        Some(ToolDiscoverySummary {
            notes: vec![
                "Use this instead of shell with 'grep' or 'rg'".into(),
                "Default output_mode is files_with_matches (paths only)".into(),
                "Use content mode with context lines for reading matches in place".into(),
                "head_limit defaults to 250; pass 0 for unlimited".into(),
            ],
            examples: vec![
                serde_json::json!({"pattern": "fn main", "output_mode": "content", "context": 3}),
                serde_json::json!({"pattern": "TODO", "glob": "*.rs"}),
                serde_json::json!({"pattern": "import", "type_filter": "py", "output_mode": "count"}),
            ],
            ..Default::default()
        })
    }
}

/// Check if ripgrep is available on the system.
#[cfg(test)]
fn is_rg_available() -> bool {
    std::process::Command::new("rg")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Skip test if rg is not installed.
    macro_rules! require_rg {
        () => {
            if !is_rg_available() {
                eprintln!("SKIPPING: ripgrep (rg) not installed");
                return;
            }
        };
    }

    fn create_search_tree(dir: &std::path::Path) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/main.rs"),
            "fn main() {\n    println!(\"hello\");\n    // TODO: add args\n}\n",
        )
        .unwrap();
        fs::write(
            dir.join("src/lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n// TODO: add tests\n",
        )
        .unwrap();
        fs::write(dir.join("README.md"), "# Project\n\nA simple project.\n").unwrap();
    }

    #[tokio::test]
    async fn test_grep_basic_pattern() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        create_search_tree(dir.path());

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"pattern": "TODO", "path": dir.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn test_grep_regex_pattern() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        create_search_tree(dir.path());

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": r"fn\s+\w+",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "content"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let content = result.result.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("fn main"));
        assert!(content.contains("fn add"));
    }

    #[tokio::test]
    async fn test_grep_files_with_matches_mode() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        create_search_tree(dir.path());

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "fn",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "files_with_matches"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 2);
        let paths: Vec<&str> = files.iter().map(|f| f.as_str().unwrap()).collect();
        assert!(paths.iter().any(|p| p.contains("main.rs")));
        assert!(paths.iter().any(|p| p.contains("lib.rs")));
    }

    #[tokio::test]
    async fn test_grep_content_mode() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        create_search_tree(dir.path());

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "println",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "content"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let content = result.result.get("content").unwrap().as_str().unwrap();
        // Should contain line numbers
        assert!(content.contains("println"));
    }

    #[tokio::test]
    async fn test_grep_count_mode() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        create_search_tree(dir.path());

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "TODO",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "count"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let total = result.result.get("total").unwrap().as_u64().unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn test_grep_case_insensitive() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "Hello\nhello\nHELLO\n").unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "hello",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "count",
                    "case_insensitive": true
                }),
                &ctx,
            )
            .await
            .unwrap();

        let total = result.result.get("total").unwrap().as_u64().unwrap();
        assert_eq!(total, 3);
    }

    #[tokio::test]
    async fn test_grep_context_lines() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("test.txt"),
            "line1\nline2\nTARGET\nline4\nline5\n",
        )
        .unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "TARGET",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "content",
                    "context": 1
                }),
                &ctx,
            )
            .await
            .unwrap();

        let content = result.result.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("line2"));
        assert!(content.contains("TARGET"));
        assert!(content.contains("line4"));
    }

    #[tokio::test]
    async fn test_grep_before_after_context() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "a\nb\nc\nMATCH\nd\ne\nf\n").unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "MATCH",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "content",
                    "before_context": 2,
                    "after_context": 1
                }),
                &ctx,
            )
            .await
            .unwrap();

        let content = result.result.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("b"));
        assert!(content.contains("c"));
        assert!(content.contains("MATCH"));
        assert!(content.contains("d"));
    }

    #[tokio::test]
    async fn test_grep_glob_filter() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        create_search_tree(dir.path());

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        // Search only .rs files
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "TODO",
                    "path": dir.path().to_str().unwrap(),
                    "glob": "*.rs"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        for f in files {
            assert!(f.as_str().unwrap().ends_with(".rs"));
        }
    }

    #[tokio::test]
    async fn test_grep_head_limit() {
        require_rg!();
        let dir = TempDir::new().unwrap();

        // Create many files with matches
        for i in 0..20 {
            fs::write(
                dir.path().join(format!("file{}.txt", i)),
                format!("MATCH line {}", i),
            )
            .unwrap();
        }

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "MATCH",
                    "path": dir.path().to_str().unwrap(),
                    "head_limit": 5
                }),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 5);
        assert!(result.result.get("truncated").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_grep_offset() {
        require_rg!();
        let dir = TempDir::new().unwrap();

        for i in 0..10 {
            fs::write(dir.path().join(format!("f{:02}.txt", i)), "MATCH").unwrap();
        }

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "MATCH",
                    "path": dir.path().to_str().unwrap(),
                    "offset": 5,
                    "head_limit": 100
                }),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 5);
    }

    #[tokio::test]
    async fn test_grep_offset_and_limit() {
        require_rg!();
        let dir = TempDir::new().unwrap();

        for i in 0..20 {
            fs::write(dir.path().join(format!("f{:02}.txt", i)), "MATCH").unwrap();
        }

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "MATCH",
                    "path": dir.path().to_str().unwrap(),
                    "offset": 5,
                    "head_limit": 3
                }),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert!(result.result.get("truncated").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "nothing here").unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "NONEXISTENT_PATTERN_xyz123",
                    "path": dir.path().to_str().unwrap()
                }),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn test_grep_unicode_content() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "Hello world\nBonjour monde\n").unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "Bonjour",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "content"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let content = result.result.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("Bonjour"));
    }

    #[tokio::test]
    async fn test_grep_path_validation() {
        require_rg!();
        let dir = TempDir::new().unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "test",
                    "path": "../../etc"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_grep_large_output_truncation() {
        require_rg!();
        let dir = TempDir::new().unwrap();

        // Create a file with many matching lines
        let content: String = (0..5000)
            .map(|i| format!("MATCH line number {}\n", i))
            .collect();
        fs::write(dir.path().join("large.txt"), &content).unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "MATCH",
                    "path": dir.path().to_str().unwrap(),
                    "output_mode": "content",
                    "head_limit": 0
                }),
                &ctx,
            )
            .await
            .unwrap();

        // Output should be truncated at some point
        let content = result.result.get("content").unwrap().as_str().unwrap();
        assert!(content.len() <= MAX_OUTPUT_SIZE + 100); // Some slack for line boundaries
    }

    #[tokio::test]
    async fn test_grep_shell_metacharacters_safe() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("test.txt"), "safe content").unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        // This should not execute as a shell command
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": ";rm -rf /",
                    "path": dir.path().to_str().unwrap()
                }),
                &ctx,
            )
            .await;

        // Should succeed (no matches) or fail safely, not execute the injection
        assert!(result.is_ok() || matches!(result.unwrap_err(), ToolError::ExecutionFailed(_)));
    }

    #[tokio::test]
    async fn test_grep_type_filter() {
        require_rg!();
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("code.py"), "def hello(): pass").unwrap();
        fs::write(dir.path().join("code.rs"), "fn hello() {}").unwrap();

        let tool = GrepTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "hello",
                    "path": dir.path().to_str().unwrap(),
                    "type_filter": "py"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].as_str().unwrap().contains("code.py"));
    }
}
