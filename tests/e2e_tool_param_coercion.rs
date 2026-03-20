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
}
