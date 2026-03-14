//! Session search tool for finding past conversations.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::db::SessionSearchStore;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Tool for searching past session summaries via FTS.
pub struct SessionSearchTool {
    store: Arc<dyn SessionSearchStore>,
}

impl SessionSearchTool {
    pub fn new(store: Arc<dyn SessionSearchStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search past conversation sessions by keyword. Returns summaries of matching \
         sessions with topics and tools used. Use this to recall prior work, decisions, \
         or context from previous conversations."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (keywords to match against session summaries)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 5)",
                    "default": 5
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
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        let limit = limit.min(20); // cap at 20

        let user_id = &ctx.user_id;

        let results = self
            .store
            .search_sessions_fts(user_id, query, limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Session search failed: {e}")))?;

        if results.is_empty() {
            return Ok(ToolOutput::text(
                format!("No sessions found matching '{query}'."),
                start.elapsed(),
            ));
        }

        let mut output = format!("Found {} matching session(s):\n\n", results.len());
        for (i, r) in results.iter().enumerate() {
            output.push_str(&format!(
                "{}. **{}** (score: {:.2})\n   Topics: {}\n   Tools: {}\n   Messages: {}\n\n",
                i + 1,
                r.summary.chars().take(200).collect::<String>(),
                r.score,
                if r.topics.is_empty() {
                    "none".to_string()
                } else {
                    r.topics.join(", ")
                },
                if r.tool_names.is_empty() {
                    "none".to_string()
                } else {
                    r.tool_names.join(", ")
                },
                r.message_count,
            ));
        }

        Ok(ToolOutput::text(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // internal data, no external content
    }
}
