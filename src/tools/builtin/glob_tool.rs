//! Fast file pattern matching tool.
//!
//! Provides glob-based file discovery without shelling out to `find` or `ls`.
//! Results are sorted by modification time (newest first) and common
//! non-essential directories are excluded by default.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use glob::{MatchOptions, glob_with};

use ironclaw_safety::sensitive_paths::is_sensitive_path;

use crate::context::JobContext;
use crate::tools::builtin::path_utils::{DEFAULT_EXCLUDED_DIRS, validate_path};
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDiscoverySummary, ToolDomain, ToolError, ToolOutput, require_str,
};

/// Maximum results returned by default.
const DEFAULT_MAX_RESULTS: usize = 200;

/// Glob tool for fast file pattern matching.
#[derive(Debug, Default)]
pub struct GlobTool {
    base_dir: Option<PathBuf>,
}

impl GlobTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }
}

/// Check if any component of a path is in the exclusion list.
fn is_excluded_path(path: &std::path::Path) -> bool {
    path.components().any(|c| {
        if let std::path::Component::Normal(name) = c
            && let Some(name_str) = name.to_str()
        {
            return DEFAULT_EXCLUDED_DIRS.contains(&name_str);
        }
        false
    })
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Fast file pattern matching. Find files by glob pattern (e.g. `**/*.rs`, `src/**/*.ts`). \
         Results sorted by modification time (newest first). \
         Use this instead of shell with `find` or `ls -R`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g. '**/*.rs', 'src/**/*.ts', '*.toml')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to current directory)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default 200)"
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
        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_MAX_RESULTS as u64) as usize;

        let start = std::time::Instant::now();

        // Reject absolute patterns and patterns with `..` that could escape the search root
        if std::path::Path::new(pattern).is_absolute() {
            return Err(ToolError::InvalidParameters(
                "Absolute glob patterns are not allowed. Use the 'path' parameter to set the search root.".to_string(),
            ));
        }
        if std::path::Path::new(pattern)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(ToolError::InvalidParameters(
                "Glob patterns containing parent directory traversal ('..') are not allowed."
                    .to_string(),
            ));
        }

        // Validate search root
        let search_root = validate_path(path_str, self.base_dir.as_deref())?;
        if is_sensitive_path(&search_root) {
            return Err(ToolError::ExecutionFailed(
                "Access denied: search root may contain credentials. \
                 Use `secret_list` and `secret_create` to manage credentials securely."
                    .to_string(),
            ));
        }

        // Construct full glob pattern
        let full_pattern = search_root.join(pattern);
        let full_pattern_str = full_pattern.to_string_lossy().to_string();

        // Run glob + metadata collection in spawn_blocking to avoid blocking the tokio executor
        let search_root_clone = search_root.clone();
        let files = tokio::task::spawn_blocking(move || {
            let options = MatchOptions {
                case_sensitive: true,
                require_literal_separator: false,
                require_literal_leading_dot: false,
            };

            let entries = glob_with(&full_pattern_str, options).map_err(|e| {
                ToolError::InvalidParameters(format!("Invalid glob pattern: {}", e))
            })?;

            let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
            for entry in entries {
                let path = match entry {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                if path.is_dir() {
                    continue;
                }

                // Defense-in-depth: ensure path is within search root
                let Ok(relative) = path.strip_prefix(&search_root_clone) else {
                    continue;
                };
                if is_excluded_path(relative) || is_sensitive_path(&path) {
                    continue;
                }

                let mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH);

                files.push((path, mtime));
            }

            Ok::<_, ToolError>(files)
        })
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Glob task failed: {}", e)))??;

        // Sort by mtime descending (newest first)
        let mut files = files;
        files.sort_by_key(|x| std::cmp::Reverse(x.1));

        // Truncate
        let truncated = files.len() > max_results;
        files.truncate(max_results);

        // Convert to relative paths
        let file_paths: Vec<String> = files
            .iter()
            .map(|(p, _)| {
                p.strip_prefix(&search_root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();

        let count = file_paths.len();
        let duration_ms = start.elapsed().as_millis() as u64;

        let result = serde_json::json!({
            "files": file_paths,
            "count": count,
            "truncated": truncated,
            "duration_ms": duration_ms
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // File paths only
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn discovery_summary(&self) -> Option<ToolDiscoverySummary> {
        Some(ToolDiscoverySummary {
            notes: vec![
                "Use this instead of shell with 'find' or 'ls -R'".into(),
                "Results sorted by modification time (newest first)".into(),
                "Excludes .git, node_modules, target, __pycache__ by default".into(),
            ],
            examples: vec![
                serde_json::json!({"pattern": "**/*.rs", "path": "src"}),
                serde_json::json!({"pattern": "*.toml"}),
            ],
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use tempfile::TempDir;

    fn create_test_tree(dir: &std::path::Path) {
        fs::create_dir_all(dir.join("src/sub")).unwrap();
        fs::write(dir.join("root.txt"), "root").unwrap();
        fs::write(dir.join("src/a.rs"), "fn a() {}").unwrap();
        fs::write(dir.join("src/b.rs"), "fn b() {}").unwrap();
        fs::write(dir.join("src/sub/c.rs"), "fn c() {}").unwrap();
        fs::write(dir.join("config.toml"), "[config]").unwrap();
    }

    #[tokio::test]
    async fn test_glob_basic_pattern() {
        let dir = TempDir::new().unwrap();
        create_test_tree(dir.path());

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "*.txt"}), &ctx)
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].as_str().unwrap(), "root.txt");
    }

    #[tokio::test]
    async fn test_glob_recursive_pattern() {
        let dir = TempDir::new().unwrap();
        create_test_tree(dir.path());

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "**/*.rs"}), &ctx)
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 3);
    }

    #[tokio::test]
    async fn test_glob_sorted_by_mtime() {
        let dir = TempDir::new().unwrap();

        // Create files with staggered timestamps.
        // 1100ms ensures different mtimes even on filesystems with 1s granularity.
        fs::write(dir.path().join("old.txt"), "old").unwrap();
        thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(dir.path().join("new.txt"), "new").unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "*.txt"}), &ctx)
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 2);
        // Newest first
        assert_eq!(files[0].as_str().unwrap(), "new.txt");
        assert_eq!(files[1].as_str().unwrap(), "old.txt");
    }

    #[tokio::test]
    async fn test_glob_excludes_git_dir() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        fs::write(dir.path().join(".git/objects/abc.txt"), "git data").unwrap();
        fs::write(dir.path().join("normal.txt"), "normal").unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "**/*.txt"}), &ctx)
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].as_str().unwrap(), "normal.txt");
    }

    #[tokio::test]
    async fn test_glob_excludes_node_modules() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(dir.path().join("node_modules/pkg/index.js"), "module").unwrap();
        fs::write(dir.path().join("index.js"), "main").unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "**/*.js"}), &ctx)
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].as_str().unwrap(), "index.js");
    }

    #[tokio::test]
    async fn test_glob_excludes_target() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::write(dir.path().join("target/debug/binary"), "elf").unwrap();
        fs::write(dir.path().join("src.rs"), "fn main() {}").unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "**/*"}), &ctx)
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.as_str().unwrap()).collect();
        assert!(!paths.iter().any(|p| p.contains("target")));
    }

    #[tokio::test]
    async fn test_glob_max_results_truncation() {
        let dir = TempDir::new().unwrap();
        for i in 0..10 {
            fs::write(dir.path().join(format!("file{}.txt", i)), "content").unwrap();
        }

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"pattern": "*.txt", "max_results": 5}),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        assert_eq!(files.len(), 5);
        assert!(result.result.get("truncated").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_glob_custom_max_results() {
        let dir = TempDir::new().unwrap();
        for i in 0..3 {
            fs::write(dir.path().join(format!("f{}.txt", i)), "c").unwrap();
        }

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"pattern": "*.txt", "max_results": 2}),
                &ctx,
            )
            .await
            .unwrap();

        let count = result.result.get("count").unwrap().as_u64().unwrap();
        assert_eq!(count, 2);
        assert!(result.result.get("truncated").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let dir = TempDir::new().unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "*.nonexistent"}), &ctx)
            .await
            .unwrap();

        let count = result.result.get("count").unwrap().as_u64().unwrap();
        assert_eq!(count, 0);
        assert!(!result.result.get("truncated").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_glob_with_path_param() {
        let dir = TempDir::new().unwrap();
        create_test_tree(dir.path());

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "*.rs",
                    "path": dir.path().join("src").to_str().unwrap()
                }),
                &ctx,
            )
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        // Only direct children of src/, not recursive
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn test_glob_empty_dir() {
        let dir = TempDir::new().unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "**/*"}), &ctx)
            .await
            .unwrap();

        let count = result.result.get("count").unwrap().as_u64().unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_glob_unicode_filenames() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("hello_world.txt"), "en").unwrap();
        fs::write(dir.path().join("data.txt"), "other").unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "*.txt"}), &ctx)
            .await
            .unwrap();

        let count = result.result.get("count").unwrap().as_u64().unwrap();
        assert!(count >= 1);
    }

    #[tokio::test]
    async fn test_glob_invalid_pattern() {
        let dir = TempDir::new().unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"pattern": "[invalid"}), &ctx)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Invalid glob pattern"));
    }

    #[tokio::test]
    async fn test_glob_path_validation() {
        let dir = TempDir::new().unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"pattern": "*.txt", "path": "../../etc"}),
                &ctx,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_glob_hidden_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".hidden"), "secret").unwrap();
        fs::write(dir.path().join("visible.txt"), "public").unwrap();

        let tool = GlobTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        // Pattern `*` should match visible files
        let result = tool
            .execute(serde_json::json!({"pattern": "*"}), &ctx)
            .await
            .unwrap();

        let files = result.result.get("files").unwrap().as_array().unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.as_str().unwrap()).collect();
        assert!(paths.contains(&"visible.txt"));
    }
}
