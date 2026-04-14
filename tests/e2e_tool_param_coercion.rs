//! E2E trace tests: schema-guided tool parameter normalization.
//!
//! These regressions run through the real agent loop with stub tools that
//! mirror Google Sheets / Google Docs write payload shapes. The model sends
//! quoted JSON container values, and the runtime must normalize them before
//! tool execution.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::json;

    use ironclaw::context::JobContext;
    use ironclaw::tools::{Tool, ToolError, ToolOutput};

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{
        LlmTrace, TraceExpects, TraceResponse, TraceStep, TraceToolCall,
    };

    struct SheetsWriteFixtureTool;

    #[async_trait]
    impl Tool for SheetsWriteFixtureTool {
        fn name(&self) -> &str {
            "google_sheets_write_fixture"
        }

        fn description(&self) -> &str {
            "Test fixture for Sheets-style values writes"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "spreadsheet_id": { "type": "string" },
                    "range": { "type": "string" },
                    "values": {
                        "type": "array",
                        "items": {
                            "type": "array",
                            "items": { "type": "integer" }
                        }
                    }
                },
                "required": ["spreadsheet_id", "range", "values"]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            let rows = params
                .get("values")
                .and_then(|v| v.as_array())
                .ok_or_else(|| ToolError::InvalidParameters("values must be an array".into()))?;

            let mut sum = 0_i64;
            for row in rows {
                let cells = row.as_array().ok_or_else(|| {
                    ToolError::InvalidParameters("each row must be an array".into())
                })?;
                for cell in cells {
                    sum += cell.as_i64().ok_or_else(|| {
                        ToolError::InvalidParameters("all cells must be integers".into())
                    })?;
                }
            }

            Ok(ToolOutput::success(
                json!({
                    "rows": rows.len(),
                    "sum": sum
                }),
                Duration::from_millis(1),
            ))
        }

        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    struct DocsBatchUpdateFixtureTool;

    #[async_trait]
    impl Tool for DocsBatchUpdateFixtureTool {
        fn name(&self) -> &str {
            "google_docs_batch_update_fixture"
        }

        fn description(&self) -> &str {
            "Test fixture for Docs-style batchUpdate requests"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "document_id": { "type": "string" },
                    "requests": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "insert_text": {
                                    "type": "object",
                                    "properties": {
                                        "location": {
                                            "type": "object",
                                            "properties": {
                                                "index": { "type": "integer" }
                                            },
                                            "required": ["index"]
                                        },
                                        "text": { "type": "string" },
                                        "bold": { "type": "boolean" }
                                    },
                                    "required": ["location", "text", "bold"]
                                }
                            },
                            "required": ["insert_text"]
                        }
                    }
                },
                "required": ["document_id", "requests"]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            let requests = params
                .get("requests")
                .and_then(|v| v.as_array())
                .ok_or_else(|| ToolError::InvalidParameters("requests must be an array".into()))?;

            let mut indexes = Vec::new();
            let mut bold_count = 0_usize;
            for request in requests {
                let insert = request
                    .get("insert_text")
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("insert_text must be an object".into())
                    })?;
                let index = insert
                    .get("location")
                    .and_then(|v| v.get("index"))
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters("location.index must be an integer".into())
                    })?;
                if insert
                    .get("bold")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    bold_count += 1;
                }
                indexes.push(index);
            }

            Ok(ToolOutput::success(
                json!({
                    "request_count": requests.len(),
                    "indexes": indexes,
                    "bold_count": bold_count
                }),
                Duration::from_millis(1),
            ))
        }

        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn e2e_normalizes_stringified_google_sheets_values() {
        let trace = LlmTrace {
            model_name: "test-coercion-sheets".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Append these rows to the sheet".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_sheets".to_string(),
                                name: "google_sheets_write_fixture".to_string(),
                                arguments: json!({
                                    "spreadsheet_id": "sheet-123",
                                    "range": "Sheet1!A1:B2",
                                    "values": "[[\"1\",2],[\"3\",\"4\"]]"
                                }),
                            }],
                            input_tokens: 100,
                            output_tokens: 25,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "The sheet write succeeded with 2 rows and sum 10."
                                .to_string(),
                            input_tokens: 120,
                            output_tokens: 20,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects {
                response_contains: vec!["2 rows".to_string(), "sum 10".to_string()],
                response_not_contains: Vec::new(),
                response_matches: None,
                tools_used: vec!["google_sheets_write_fixture".to_string()],
                tools_not_used: Vec::new(),
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                tool_results_contain: std::collections::HashMap::new(),
                tools_order: vec!["google_sheets_write_fixture".to_string()],
            },
            repo: Default::default(),
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![Arc::new(SheetsWriteFixtureTool)])
            .build()
            .await;

        rig.send_message("Append these rows to the sheet").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);
        let tool_results = rig.tool_results();
        assert!(
            tool_results
                .iter()
                .any(|(name, preview)| name == "google_sheets_write_fixture"
                    && preview.contains("\"rows\"")
                    && preview.contains("2")
                    && preview.contains("\"sum\"")
                    && preview.contains("10")),
            "expected normalized sheet result preview, got {tool_results:?}"
        );

        rig.shutdown();
    }

    #[tokio::test]
    async fn e2e_normalizes_stringified_google_docs_requests() {
        let trace = LlmTrace {
            model_name: "test-coercion-docs".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Apply these edits to the doc".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_docs".to_string(),
                                name: "google_docs_batch_update_fixture".to_string(),
                                arguments: json!({
                                    "document_id": "doc-456",
                                    "requests": "[{\"insert_text\":{\"location\":{\"index\":\"1\"},\"text\":\"Hello\",\"bold\":\"true\"}},{\"insert_text\":{\"location\":{\"index\":5},\"text\":\" world\",\"bold\":\"false\"}}]"
                                }),
                            }],
                            input_tokens: 140,
                            output_tokens: 30,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "The doc update succeeded with 2 requests at indexes 1 and 5."
                                .to_string(),
                            input_tokens: 180,
                            output_tokens: 24,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects {
                response_contains: vec!["2 requests".to_string(), "indexes 1 and 5".to_string()],
                response_not_contains: Vec::new(),
                response_matches: None,
                tools_used: vec!["google_docs_batch_update_fixture".to_string()],
                tools_not_used: Vec::new(),
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                tool_results_contain: std::collections::HashMap::new(),
                tools_order: vec!["google_docs_batch_update_fixture".to_string()],
            },
            repo: Default::default(),
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![Arc::new(DocsBatchUpdateFixtureTool)])
            .build()
            .await;

        rig.send_message("Apply these edits to the doc").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);
        let tool_results = rig.tool_results();
        assert!(
            tool_results
                .iter()
                .any(|(name, preview)| name == "google_docs_batch_update_fixture"
                    && preview.contains("\"request_count\"")
                    && preview.contains("2")
                    && preview.contains("\"bold_count\"")
                    && preview.contains("1")),
            "expected normalized docs result preview, got {tool_results:?}"
        );

        rig.shutdown();
    }

    /// Fixture tool that mirrors the github WASM tool's `oneOf` discriminated
    /// union schema. Uses `#[serde(tag = "action")]` deserialization — exactly
    /// what the real tool does — so if coercion fails the test reproduces:
    /// `invalid type: string "100", expected u32`
    struct GitHubFixtureTool;

    #[derive(Debug, Deserialize)]
    #[serde(tag = "action")]
    enum GitHubFixtureAction {
        #[serde(rename = "list_issues")]
        ListIssues {
            owner: String,
            repo: String,
            #[serde(default)]
            state: Option<String>,
            #[serde(default)]
            limit: Option<u32>,
        },
        #[serde(rename = "get_issue")]
        GetIssue {
            owner: String,
            repo: String,
            issue_number: u32,
        },
        #[serde(rename = "list_pull_requests")]
        ListPullRequests {
            owner: String,
            repo: String,
            #[serde(default)]
            limit: Option<u32>,
            #[serde(default)]
            page: Option<u32>,
        },
        #[serde(rename = "create_pull_request")]
        CreatePullRequest {
            owner: String,
            repo: String,
            title: String,
            head: String,
            base: String,
            #[serde(default)]
            draft: Option<bool>,
        },
    }

    use serde::Deserialize;

    #[async_trait]
    impl Tool for GitHubFixtureTool {
        fn name(&self) -> &str {
            "github_fixture"
        }

        fn description(&self) -> &str {
            "Fixture mirroring the github WASM tool's oneOf schema"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "required": ["action"],
                "oneOf": [
                    {
                        "properties": {
                            "action": { "const": "list_issues" },
                            "owner": { "type": "string" },
                            "repo": { "type": "string" },
                            "state": { "type": "string", "enum": ["open", "closed", "all"] },
                            "limit": { "type": "integer", "default": 30 }
                        },
                        "required": ["action", "owner", "repo"]
                    },
                    {
                        "properties": {
                            "action": { "const": "get_issue" },
                            "owner": { "type": "string" },
                            "repo": { "type": "string" },
                            "issue_number": { "type": "integer" }
                        },
                        "required": ["action", "owner", "repo", "issue_number"]
                    },
                    {
                        "properties": {
                            "action": { "const": "list_pull_requests" },
                            "owner": { "type": "string" },
                            "repo": { "type": "string" },
                            "limit": { "type": "integer", "default": 30 },
                            "page": { "type": "integer" }
                        },
                        "required": ["action", "owner", "repo"]
                    },
                    {
                        "properties": {
                            "action": { "const": "create_pull_request" },
                            "owner": { "type": "string" },
                            "repo": { "type": "string" },
                            "title": { "type": "string" },
                            "head": { "type": "string" },
                            "base": { "type": "string" },
                            "draft": { "type": "boolean", "default": false }
                        },
                        "required": ["action", "owner", "repo", "title", "head", "base"]
                    }
                ]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            // Deserialize exactly like the real github WASM tool does.
            // Without coercion, this fails: `invalid type: string "100", expected u32`
            let action: GitHubFixtureAction = serde_json::from_value(params).map_err(|e| {
                ToolError::InvalidParameters(format!("serde deserialization failed: {e}"))
            })?;

            let result = match action {
                GitHubFixtureAction::ListIssues {
                    owner,
                    repo,
                    state,
                    limit,
                } => json!({
                    "action": "list_issues",
                    "owner": owner,
                    "repo": repo,
                    "state": state.unwrap_or_else(|| "open".to_string()),
                    "limit": limit.unwrap_or(30),
                }),
                GitHubFixtureAction::GetIssue {
                    owner,
                    repo,
                    issue_number,
                } => json!({
                    "action": "get_issue",
                    "owner": owner,
                    "repo": repo,
                    "issue_number": issue_number,
                }),
                GitHubFixtureAction::ListPullRequests {
                    owner,
                    repo,
                    limit,
                    page,
                } => json!({
                    "action": "list_pull_requests",
                    "owner": owner,
                    "repo": repo,
                    "limit": limit.unwrap_or(30),
                    "page": page.unwrap_or(1),
                }),
                GitHubFixtureAction::CreatePullRequest {
                    owner,
                    repo,
                    title,
                    head,
                    base,
                    draft,
                } => json!({
                    "action": "create_pull_request",
                    "owner": owner,
                    "repo": repo,
                    "title": title,
                    "head": head,
                    "base": base,
                    "draft": draft.unwrap_or(false),
                }),
            };

            Ok(ToolOutput::success(result, Duration::from_millis(1)))
        }

        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    /// Reproduces the exact bug: LLM sends `limit: "100"` and `issue_number: "42"`
    /// as strings to a `oneOf` discriminated union schema. Without coercion support
    /// for combinators, serde fails with `invalid type: string "100", expected u32`.
    #[tokio::test]
    async fn e2e_coerces_oneof_discriminated_union_params() {
        let trace = LlmTrace {
            model_name: "test-coercion-oneof".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "List issues in nearai/ironclaw with limit 100".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_list".to_string(),
                                name: "github_fixture".to_string(),
                                // LLM sends numeric params as strings — the exact bug
                                arguments: json!({
                                    "action": "list_issues",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "state": "open",
                                    "limit": "100"
                                }),
                            }],
                            input_tokens: 100,
                            output_tokens: 30,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Found issues in nearai/ironclaw with limit 100.".to_string(),
                            input_tokens: 150,
                            output_tokens: 20,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects {
                tools_used: vec!["github_fixture".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            repo: Default::default(),
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![Arc::new(GitHubFixtureTool)])
            .build()
            .await;

        rig.send_message("List issues in nearai/ironclaw with limit 100")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);
        let tool_results = rig.tool_results();
        assert!(
            tool_results
                .iter()
                .any(|(name, preview)| name == "github_fixture"
                    && preview.contains("\"limit\"")
                    && preview.contains("100")),
            "expected coerced list_issues result, got {tool_results:?}"
        );

        rig.shutdown();
    }

    /// Tests a second oneOf variant with different string-to-integer coercions:
    /// `issue_number: "42"` must be coerced to match the `get_issue` variant.
    #[tokio::test]
    async fn e2e_coerces_oneof_get_issue_variant() {
        let trace = LlmTrace {
            model_name: "test-coercion-oneof-issue".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Get issue 42 from nearai/ironclaw".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_issue".to_string(),
                                name: "github_fixture".to_string(),
                                arguments: json!({
                                    "action": "get_issue",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "issue_number": "42"
                                }),
                            }],
                            input_tokens: 80,
                            output_tokens: 20,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Issue 42 retrieved.".to_string(),
                            input_tokens: 100,
                            output_tokens: 10,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects {
                tools_used: vec!["github_fixture".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            repo: Default::default(),
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![Arc::new(GitHubFixtureTool)])
            .build()
            .await;

        rig.send_message("Get issue 42 from nearai/ironclaw").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);
        let tool_results = rig.tool_results();
        assert!(
            tool_results
                .iter()
                .any(|(name, preview)| name == "github_fixture"
                    && preview.contains("\"issue_number\"")
                    && preview.contains("42")),
            "expected coerced get_issue result, got {tool_results:?}"
        );

        rig.shutdown();
    }

    /// Tests boolean coercion in a oneOf variant: `draft: "true"` must become
    /// a boolean for the `create_pull_request` variant.
    #[tokio::test]
    async fn e2e_coerces_oneof_boolean_in_variant() {
        let trace = LlmTrace {
            model_name: "test-coercion-oneof-bool".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Create a draft PR".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_pr".to_string(),
                                name: "github_fixture".to_string(),
                                arguments: json!({
                                    "action": "create_pull_request",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "title": "Fix coercion",
                                    "head": "fix/coercion",
                                    "base": "main",
                                    "draft": "true"
                                }),
                            }],
                            input_tokens: 90,
                            output_tokens: 25,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Draft PR created.".to_string(),
                            input_tokens: 110,
                            output_tokens: 10,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: Vec::new(),
            expects: TraceExpects {
                tools_used: vec!["github_fixture".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            repo: Default::default(),
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![Arc::new(GitHubFixtureTool)])
            .build()
            .await;

        rig.send_message("Create a draft PR").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        rig.verify_trace_expects(&trace, &responses);
        let tool_results = rig.tool_results();
        assert!(
            tool_results
                .iter()
                .any(|(name, preview)| name == "github_fixture"
                    && preview.contains("\"draft\"")
                    && preview.contains("true")),
            "expected coerced create_pull_request result with draft=true, got {tool_results:?}"
        );

        rig.shutdown();
    }
}
