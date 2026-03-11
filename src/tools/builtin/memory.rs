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
         or provide a custom path for arbitrary file creation."
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
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let content = require_str(&params, "content")?;

        let target = params
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("daily_log");

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
                self.workspace
                    .append_daily_log(content)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;
                format!("daily/{}.md", chrono::Utc::now().format("%Y-%m-%d"))
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
         (use read_file for those). Works with identity files, heartbeat checklist, \
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

// ========== Structured Fact Tools ==========

use crate::db::Database;

/// Tool for searching structured facts in the memory_facts table.
///
/// Uses hybrid FTS + semantic search to find relevant facts.
pub struct MemoryFactsSearchTool {
    db: Arc<dyn Database>,
    workspace: Arc<Workspace>,
}

impl MemoryFactsSearchTool {
    /// Create a new memory facts search tool.
    pub fn new(db: Arc<dyn Database>, workspace: Arc<Workspace>) -> Self {
        Self { db, workspace }
    }
}

#[async_trait]
impl Tool for MemoryFactsSearchTool {
    fn name(&self) -> &str {
        "memory_facts_search"
    }

    fn description(&self) -> &str {
        "Search structured facts extracted from past conversations. Returns atomic \
         knowledge items (preferences, learned facts, procedures, context). Use this \
         alongside memory_search for the most complete recall of past information."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query"
                },
                "category": {
                    "type": "string",
                    "description": "Optional category filter: preference, learned, procedural, context",
                    "enum": ["preference", "learned", "procedural", "context"]
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results (default: 10, max: 50)",
                    "default": 10,
                    "minimum": 1,
                    "maximum": 50
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?;
        let category = params.get("category").and_then(|v| v.as_str());
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(50) as usize;

        let user_id = &ctx.user_id;

        // If category filter is specified, use category-based retrieval
        if let Some(cat) = category {
            let facts = self
                .db
                .get_facts_by_category(user_id, None, cat, limit)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Fact search failed: {}", e)))?;

            let output = serde_json::json!({
                "query": query,
                "category_filter": cat,
                "results": facts.iter().map(|f| serde_json::json!({
                    "id": f.id.to_string(),
                    "content": f.content,
                    "category": f.category,
                    "confidence": f.confidence,
                    "updated_at": f.updated_at.to_rfc3339(),
                })).collect::<Vec<_>>(),
                "result_count": facts.len(),
            });
            return Ok(ToolOutput::success(output, start.elapsed()));
        }

        // Try embedding-enhanced search if embeddings are available
        let embedding = if let Some(embedder) = self.workspace.embeddings() {
            embedder.embed(query).await.ok()
        } else {
            None
        };

        let results = self
            .db
            .search_facts(user_id, None, query, embedding.as_deref(), limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Fact search failed: {}", e)))?;

        let output = serde_json::json!({
            "query": query,
            "results": results.iter().map(|r| serde_json::json!({
                "id": r.fact.id.to_string(),
                "content": r.fact.content,
                "category": r.fact.category,
                "confidence": r.fact.confidence,
                "score": r.score,
                "updated_at": r.fact.updated_at.to_rfc3339(),
            })).collect::<Vec<_>>(),
            "result_count": results.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal memory
    }
}

/// Tool for managing structured facts (add/update/delete).
pub struct MemoryFactsManageTool {
    db: Arc<dyn Database>,
}

impl MemoryFactsManageTool {
    /// Create a new memory facts manage tool.
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for MemoryFactsManageTool {
    fn name(&self) -> &str {
        "memory_facts_manage"
    }

    fn description(&self) -> &str {
        "Manage structured facts: add new facts, update existing ones, or delete outdated facts. \
         Facts are atomic knowledge items automatically extracted from conversations. Use this \
         to manually curate the fact store when auto-extraction misses or gets something wrong."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: add, update, or delete",
                    "enum": ["add", "update", "delete"]
                },
                "id": {
                    "type": "string",
                    "description": "Fact ID (required for update and delete)"
                },
                "content": {
                    "type": "string",
                    "description": "The fact content (required for add, optional for update)"
                },
                "category": {
                    "type": "string",
                    "description": "Category: preference, learned, procedural, context (required for add)",
                    "enum": ["preference", "learned", "procedural", "context"]
                },
                "confidence": {
                    "type": "number",
                    "description": "Confidence score 0.0-1.0 (default: 1.0)",
                    "default": 1.0,
                    "minimum": 0.0,
                    "maximum": 1.0
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let action = require_str(&params, "action")?;
        let user_id = &ctx.user_id;

        match action {
            "add" => {
                let content = require_str(&params, "content")?;
                let category = params
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("learned");
                let confidence = params
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0) as f32;

                if content.trim().is_empty() {
                    return Err(ToolError::InvalidParameters(
                        "content cannot be empty".to_string(),
                    ));
                }

                let id = uuid::Uuid::new_v4();
                self.db
                    .upsert_fact(
                        id,
                        user_id,
                        None,
                        content,
                        category,
                        confidence,
                        None,
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Add fact failed: {}", e)))?;

                let output = serde_json::json!({
                    "status": "added",
                    "id": id.to_string(),
                    "content": content,
                    "category": category,
                    "confidence": confidence,
                });
                Ok(ToolOutput::success(output, start.elapsed()))
            }
            "update" => {
                let id_str = require_str(&params, "id")?;
                let id: uuid::Uuid = id_str
                    .parse()
                    .map_err(|_| ToolError::InvalidParameters("invalid fact ID".to_string()))?;

                let content = require_str(&params, "content")?;
                let confidence = params
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0) as f32;

                self.db
                    .update_fact(id, content, confidence)
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!("Update fact failed: {}", e))
                    })?;

                let output = serde_json::json!({
                    "status": "updated",
                    "id": id.to_string(),
                    "content": content,
                    "confidence": confidence,
                });
                Ok(ToolOutput::success(output, start.elapsed()))
            }
            "delete" => {
                let id_str = require_str(&params, "id")?;
                let id: uuid::Uuid = id_str
                    .parse()
                    .map_err(|_| ToolError::InvalidParameters("invalid fact ID".to_string()))?;

                self.db.delete_fact(id).await.map_err(|e| {
                    ToolError::ExecutionFailed(format!("Delete fact failed: {}", e))
                })?;

                let output = serde_json::json!({
                    "status": "deleted",
                    "id": id.to_string(),
                });
                Ok(ToolOutput::success(output, start.elapsed()))
            }
            _ => Err(ToolError::InvalidParameters(format!(
                "unknown action '{}': use add, update, or delete",
                action
            ))),
        }
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
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
