//! Memory tools for persistent workspace memory.
//!
//! These tools allow the agent to:
//! - Search past memories, decisions, and context
//! - Read and write files in the workspace
//!
//! # Usage
//!
//! The agent should use `memory_search` before answering questions about
//! prior work, decisions, dates, people, preferences, or todos.
//!
//! Use `memory_write` to persist important facts that should be remembered
//! across sessions.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};
use crate::workspace::{Workspace, paths};

/// Identity files that the LLM must not overwrite via tool calls.
/// These are loaded into the system prompt and could be used for prompt
/// injection if an attacker tricks the agent into overwriting them.
const PROTECTED_IDENTITY_FILES: &[&str] =
    &[paths::IDENTITY, paths::SOUL, paths::AGENTS, paths::USER];

/// Detect paths that are clearly local filesystem references, not workspace-memory docs.
///
/// Examples:
/// - `/Users/.../file.md` (Unix absolute)
/// - `C:\Users\...` or `D:/work/...` (Windows absolute)
/// - `~/notes.md` (home expansion shorthand)
fn looks_like_filesystem_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }

    if Path::new(path).is_absolute() || path.starts_with("~/") {
        return true;
    }

    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Tool for searching workspace memory.
///
/// Performs hybrid search (FTS + semantic) across all memory documents.
/// The agent should call this tool before answering questions about
/// prior work, decisions, preferences, or any historical context.
pub struct MemorySearchTool {
    workspace: Arc<Workspace>,
}

impl MemorySearchTool {
    /// Create a new memory search tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search past memories, decisions, and context. MUST be called before answering \
         questions about prior work, decisions, dates, people, preferences, or todos. \
         Returns relevant snippets with relevance scores."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Use natural language to describe what you're looking for."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5, max: 20)",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let query = require_str(&params, "query")?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(20) as usize;

        let results = self
            .workspace
            .search(query, limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Search failed: {}", e)))?;

        let result_count = results.len();
        let output = serde_json::json!({
            "query": query,
            "results": results.into_iter().map(|r| serde_json::json!({
                "content": r.content,
                "score": r.score,
                "path": r.document_path,
                "document_id": r.document_id.to_string(),
                "is_hybrid_match": r.is_hybrid(),
            })).collect::<Vec<_>>(),
            "result_count": result_count,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal memory, trusted content
    }
}

/// Tool for writing to workspace memory.
///
/// Use this to persist important information that should be remembered
/// across sessions: decisions, preferences, facts, lessons learned.
pub struct MemoryWriteTool {
    workspace: Arc<Workspace>,
}

impl MemoryWriteTool {
    /// Create a new memory write tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Write to persistent memory (database-backed, NOT the local filesystem). \
         Use for important facts, decisions, preferences, or lessons learned that should \
         be remembered across sessions. Targets: 'memory' for curated long-term facts, \
         'daily_log' for timestamped session notes, 'heartbeat' for the periodic \
         checklist (HEARTBEAT.md), 'bootstrap' to clear the first-run ritual file, \
         or provide a custom workspace path for arbitrary file creation. \
         Never pass absolute filesystem paths like '/Users/...' or 'C:\\...'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to write to memory. Be concise but include relevant context."
                },
                "target": {
                    "type": "string",
                    "description": "Where to write: 'memory' for MEMORY.md, 'daily_log' for today's log, 'heartbeat' for HEARTBEAT.md checklist, 'bootstrap' to clear BOOTSTRAP.md (content is ignored; the file is always cleared), or a path like 'projects/alpha/notes.md'",
                    "default": "daily_log"
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to existing content. If false, replace entirely.",
                    "default": true
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let content = require_str(&params, "content")?;

        let target = params
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("daily_log");

        if looks_like_filesystem_path(target) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' looks like a local filesystem path. memory_write only works with workspace-memory paths. \
                 Use write_file for filesystem writes. For opening files in an editor, use shell with: open \"<absolute_path>\".",
                target
            )));
        }

        // Bootstrap target: clear BOOTSTRAP.md to mark first-run ritual complete.
        // Handled early because it accepts empty content (unlike other targets).
        if target == "bootstrap" {
            // Write empty content to effectively disable the bootstrap injection.
            // system_prompt_for_context() skips empty files.
            self.workspace
                .write(paths::BOOTSTRAP, "")
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;

            let output = serde_json::json!({
                "status": "cleared",
                "path": paths::BOOTSTRAP,
                "message": "BOOTSTRAP.md cleared. First-run ritual will not repeat.",
            });

            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        if content.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "content cannot be empty".to_string(),
            ));
        }

        // Reject writes to identity files that are loaded into the system prompt.
        // An attacker could use prompt injection to trick the agent into overwriting
        // these, poisoning future conversations.
        if PROTECTED_IDENTITY_FILES.contains(&target) {
            return Err(ToolError::NotAuthorized(format!(
                "writing to '{}' is not allowed (identity file protected from tool writes)",
                target,
            )));
        }

        let append = params
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let path = match target {
            "memory" => {
                if append {
                    self.workspace
                        .append_memory(content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                } else {
                    self.workspace
                        .write(paths::MEMORY, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                }
                paths::MEMORY.to_string()
            }
            "daily_log" => {
                let tz = crate::timezone::parse_timezone(&ctx.user_timezone)
                    .unwrap_or(chrono_tz::Tz::UTC);
                self.workspace
                    .append_daily_log_tz(content, tz)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?
            }
            "heartbeat" => {
                if append {
                    self.workspace
                        .append(paths::HEARTBEAT, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                } else {
                    self.workspace
                        .write(paths::HEARTBEAT, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                }
                paths::HEARTBEAT.to_string()
            }
            path => {
                // Protect identity files from LLM overwrites (prompt injection defense).
                // These files are injected into the system prompt, so poisoning them
                // would let an attacker rewrite the agent's core instructions.
                let normalized = path.trim_start_matches('/');
                if PROTECTED_IDENTITY_FILES
                    .iter()
                    .any(|p| normalized.eq_ignore_ascii_case(p))
                {
                    return Err(ToolError::NotAuthorized(format!(
                        "writing to '{}' is not allowed (identity file protected from tool access)",
                        path
                    )));
                }

                if append {
                    self.workspace
                        .append(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                } else {
                    self.workspace
                        .write(path, content)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                }
                path.to_string()
            }
        };

        let output = serde_json::json!({
            "status": "written",
            "path": path,
            "append": append,
            "content_length": content.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for reading workspace files.
///
/// Use this to read the full content of any file in the workspace.
pub struct MemoryReadTool {
    workspace: Arc<Workspace>,
}

impl MemoryReadTool {
    /// Create a new memory read tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemoryReadTool {
    fn name(&self) -> &str {
        "memory_read"
    }

    fn description(&self) -> &str {
        "Read a file from the workspace memory (database-backed storage). \
         Use this to read files shown by memory_tree. NOT for local filesystem files \
         (use read_file for those). Do not pass absolute paths like '/Users/...' or 'C:\\...'. \
         Works with identity files, heartbeat checklist, \
         memory, daily logs, or any custom workspace path."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file (e.g., 'MEMORY.md', 'daily/2024-01-15.md', 'projects/alpha/notes.md')"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let path = require_str(&params, "path")?;

        if looks_like_filesystem_path(path) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' looks like a local filesystem path. memory_read only works with workspace-memory paths. \
                 Use read_file for filesystem reads. For opening files in an editor, use shell with: open \"<absolute_path>\".",
                path
            )));
        }

        let doc = self
            .workspace
            .read(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Read failed: {}", e)))?;

        let output = serde_json::json!({
            "path": doc.path,
            "content": doc.content,
            "word_count": doc.word_count(),
            "updated_at": doc.updated_at.to_rfc3339(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal memory
    }
}

#[cfg(test)]
mod path_routing_tests {
    use super::looks_like_filesystem_path;

    #[test]
    fn detects_filesystem_paths() {
        assert!(looks_like_filesystem_path("/Users/nige/file.md"));
        assert!(looks_like_filesystem_path("C:\\Users\\nige\\file.md"));
        assert!(looks_like_filesystem_path("D:/work/file.md"));
        assert!(looks_like_filesystem_path("~/notes.md"));
    }

    #[test]
    fn allows_workspace_memory_paths() {
        assert!(!looks_like_filesystem_path("MEMORY.md"));
        assert!(!looks_like_filesystem_path("daily/2026-03-11.md"));
        assert!(!looks_like_filesystem_path("projects/alpha/notes.md"));
    }
}

/// Tool for viewing workspace structure as a tree.
///
/// Returns a hierarchical view of files and directories with configurable depth.
pub struct MemoryTreeTool {
    workspace: Arc<Workspace>,
}

impl MemoryTreeTool {
    /// Create a new memory tree tool.
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }

    /// Recursively build tree structure.
    ///
    /// Returns a compact format where directories end with `/` and may have children.
    async fn build_tree(
        &self,
        path: &str,
        current_depth: usize,
        max_depth: usize,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        if current_depth > max_depth {
            return Ok(Vec::new());
        }

        let entries = self
            .workspace
            .list(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Tree failed: {}", e)))?;

        let mut result = Vec::new();
        for entry in entries {
            // Directories end with `/`, files don't
            let display_path = if entry.is_directory {
                format!("{}/", entry.name())
            } else {
                entry.name().to_string()
            };

            if entry.is_directory && current_depth < max_depth {
                let children =
                    Box::pin(self.build_tree(&entry.path, current_depth + 1, max_depth)).await?;
                if children.is_empty() {
                    result.push(serde_json::Value::String(display_path));
                } else {
                    result.push(serde_json::json!({ display_path: children }));
                }
            } else {
                result.push(serde_json::Value::String(display_path));
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl Tool for MemoryTreeTool {
    fn name(&self) -> &str {
        "memory_tree"
    }

    fn description(&self) -> &str {
        "View the workspace memory structure as a tree (database-backed storage). \
         Use memory_read to read files shown here, NOT read_file. \
         The workspace is separate from the local filesystem."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Root path to start from (empty string for workspace root)",
                    "default": ""
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum depth to traverse (1 = immediate children only)",
                    "default": 1,
                    "minimum": 1,
                    "maximum": 10
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");

        let depth = params
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, 10) as usize;

        let tree = self.build_tree(path, 1, depth).await?;

        // Compact output: just the tree array
        Ok(ToolOutput::success(
            serde_json::Value::Array(tree),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    fn make_test_workspace() -> Arc<Workspace> {
        Arc::new(Workspace::new(
            "test_user",
            deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
                tokio_postgres::Config::new(),
                tokio_postgres::NoTls,
            ))
            .build()
            .unwrap(),
        ))
    }

    #[test]
    fn test_memory_search_schema() {
        let workspace = make_test_workspace();
        let tool = MemorySearchTool::new(workspace);

        assert_eq!(tool.name(), "memory_search");
        assert!(!tool.requires_sanitization());

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"query".into())
        );
    }

    #[test]
    fn test_memory_write_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryWriteTool::new(workspace);

        assert_eq!(tool.name(), "memory_write");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["content"].is_object());
        assert!(schema["properties"]["target"].is_object());
        assert!(schema["properties"]["append"].is_object());
    }

    #[test]
    fn test_memory_read_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryReadTool::new(workspace);

        assert_eq!(tool.name(), "memory_read");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"path".into())
        );
    }

    #[test]
    fn test_memory_tree_schema() {
        let workspace = make_test_workspace();
        let tool = MemoryTreeTool::new(workspace);

        assert_eq!(tool.name(), "memory_tree");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["depth"].is_object());
        assert_eq!(schema["properties"]["depth"]["default"], 1);
    }
}
