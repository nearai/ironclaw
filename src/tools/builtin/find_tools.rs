//! `find_tools` — keyword search over the ACTIVE tool registry.
//!
//! When tool retrieval narrows the model-facing tool list for efficiency, a
//! needed tool may not be advertised on a given turn. Tool *execution* is not
//! gated by the advertised list (the dispatcher looks calls up in the full
//! registry), so the model only needs to learn a hidden tool's name to call
//! it. This tool is the always-in-core discovery path: it searches every
//! registered tool by keyword and returns names + parameter schemas the model
//! can then call directly.

use std::sync::Weak;

use async_trait::async_trait;
use ironclaw_llm::ToolDefinition;
use serde_json::json;

use crate::context::JobContext;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Rank registered tool definitions against a keyword `query`.
///
/// Pure and registry-free so it is unit-testable. A tool matches if any query
/// word (≥2 chars) is a substring of its name or description. Name matches
/// score higher than description matches; ties break on name for determinism.
/// An empty/whitespace query yields no matches (callers require a real query).
pub fn rank_tools(defs: Vec<ToolDefinition>, query: &str, limit: usize) -> Vec<ToolDefinition> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().filter(|w| w.len() >= 2).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(i32, ToolDefinition)> = defs
        .into_iter()
        .filter_map(|d| {
            let name = d.name.to_lowercase();
            let desc = d.description.to_lowercase();
            let mut score = 0;
            for w in &words {
                if name.contains(w) {
                    score += 2;
                }
                if desc.contains(w) {
                    score += 1;
                }
            }
            if score > 0 {
                Some((score, d))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(limit).map(|(_, d)| d).collect()
}

/// `find_tools` discovery tool. Holds a weak ref to the registry so it can
/// enumerate the full tool set at call time (mirrors `ToolInfoTool`).
pub struct FindToolsTool {
    registry: Weak<ToolRegistry>,
}

impl FindToolsTool {
    pub fn new(registry: Weak<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for FindToolsTool {
    fn name(&self) -> &str {
        "find_tools"
    }

    fn description(&self) -> &str {
        "Search ALL your available tools by keyword to find one you need. Your visible tool \
         list may be narrowed for efficiency, so if you need a capability that isn't in your \
         current tools — browsing the web, running code, sending a message, reading files, \
         checking news, etc. — call find_tools with a keyword FIRST instead of saying you \
         can't do it. Returns matching tool names + their parameters, which you can then call \
         directly."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword(s) describing the capability you need (e.g. \"browser\", \"news web\", \"run python\")."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max tools to return (default 10, max 50).",
                    "default": 10
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
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if query.is_empty() {
            return Err(ToolError::InvalidParameters(
                "find_tools requires a non-empty 'query'".to_string(),
            ));
        }
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .clamp(1, 50) as usize;

        let registry = self.registry.upgrade().ok_or_else(|| {
            ToolError::ExecutionFailed("tool registry is no longer available for find_tools".to_string())
        })?;
        let defs = registry.tool_definitions().await;
        let matches = rank_tools(defs, &query, limit);
        let count = matches.len();
        let results: Vec<serde_json::Value> = matches
            .into_iter()
            .map(|d| json!({ "name": d.name, "description": d.description, "parameters": d.parameters }))
            .collect();
        let hint = if count == 0 {
            "No tools matched that keyword. Try a broader or different term."
        } else {
            "Call any listed tool by name directly — these are callable now."
        };
        Ok(ToolOutput::success(
            json!({ "query": query, "results": results, "count": count, "hint": hint }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, desc: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: desc.to_string(),
            parameters: json!({"type": "object"}),
        }
    }

    fn fixture() -> Vec<ToolDefinition> {
        vec![
            def("playwright_browser_navigate", "Navigate the browser to a URL"),
            def("memory_search", "Search stored memory"),
            def("system_time", "Get the current time"),
            def("wm_get", "Get a world-monitor news/economic snapshot value"),
        ]
    }

    #[test]
    fn matches_keyword_in_name() {
        let out = rank_tools(fixture(), "browser", 10);
        let names: Vec<&str> = out.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["playwright_browser_navigate"]);
    }

    #[test]
    fn matches_keyword_in_description() {
        // "news" appears only in wm_get's description.
        let out = rank_tools(fixture(), "news", 10);
        let names: Vec<&str> = out.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["wm_get"]);
    }

    #[test]
    fn name_match_outranks_description_match() {
        // Query "browser" hits the navigate name (score 2); nothing else.
        // Query with two words where one hits a name and one a desc: name wins.
        let defs = vec![
            def("web_browser_open", "open a page"),      // name hit on "browser"
            def("fetch_url", "download a web browser page"), // desc hit on "browser"/"web"
        ];
        let out = rank_tools(defs, "browser", 10);
        assert_eq!(out[0].name, "web_browser_open");
    }

    #[test]
    fn limit_caps_results() {
        let defs = vec![
            def("web_a", "web tool a"),
            def("web_b", "web tool b"),
            def("web_c", "web tool c"),
        ];
        let out = rank_tools(defs, "web", 2);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn no_match_and_empty_query_return_empty() {
        assert!(rank_tools(fixture(), "quantum", 10).is_empty());
        assert!(rank_tools(fixture(), "   ", 10).is_empty());
    }
}
