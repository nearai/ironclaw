//! E2E test: real github WASM tool with parameter coercion via TestRig.
//!
//! Loads the compiled github WASM binary into the test rig, replays an LLM
//! trace that sends string-typed numeric params, and verifies the WASM tool
//! constructs the correct HTTP API call via `http_exchanges` in the trace.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use ironclaw::llm::recording::{HttpExchange, HttpExchangeRequest, HttpExchangeResponse};

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{
        LlmTrace, TraceExpects, TraceResponse, TraceStep, TraceToolCall,
    };

    const GITHUB_WASM: &str = "tools-src/github/target/wasm32-wasip2/release/github_tool.wasm";
    const GITHUB_CAPS: &str = "tools-src/github/github-tool.capabilities.json";

    fn github_ok(body: &str) -> HttpExchangeResponse {
        HttpExchangeResponse {
            status: 200,
            headers: vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("x-ratelimit-remaining".to_string(), "100".to_string()),
            ],
            body: body.to_string(),
        }
    }

    fn skip_if_no_wasm() -> bool {
        if !std::path::Path::new(GITHUB_WASM).exists() {
            eprintln!(
                "Skipping: github WASM binary not found at {GITHUB_WASM}. \
                 Build with: cargo build -p github-tool --target wasm32-wasip2 --release"
            );
            true
        } else {
            false
        }
    }

    /// LLM sends `limit: "50"` (string) to `list_issues`. Coercion converts it
    /// to integer, and the WASM tool must call `GET /repos/.../issues?...&per_page=50`.
    #[tokio::test]
    async fn wasm_github_list_issues_coerces_string_limit() {
        if skip_if_no_wasm() {
            return;
        }

        let expected_url =
            "https://api.github.com/repos/nearai/ironclaw/issues?state=open&per_page=50";

        let trace = LlmTrace {
            model_name: "test-wasm-coercion-list-issues".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "List issues in nearai/ironclaw with limit 50".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_1".to_string(),
                                name: "github".to_string(),
                                arguments: json!({
                                    "action": "list_issues",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "state": "open",
                                    "limit": "50"
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
                            content: "Found 1 issue.".to_string(),
                            input_tokens: 150,
                            output_tokens: 10,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![HttpExchange {
                request: HttpExchangeRequest {
                    method: "GET".to_string(),
                    url: expected_url.to_string(),
                    headers: vec![],
                    body: None,
                },
                response: github_ok(r#"[{"number":1,"title":"Test issue","state":"open"}]"#),
            }],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("github", GITHUB_WASM, Some(GITHUB_CAPS))
            .build()
            .await;

        rig.send_message("List issues in nearai/ironclaw with limit 50")
            .await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;
        rig.verify_trace_expects(&trace, &responses);

        rig.shutdown();
    }

    /// LLM sends `issue_number: "42"` (string) to `get_issue`. Coercion converts
    /// it to integer, and the URL must contain `/issues/42`.
    #[tokio::test]
    async fn wasm_github_get_issue_coerces_string_issue_number() {
        if skip_if_no_wasm() {
            return;
        }

        let expected_url = "https://api.github.com/repos/nearai/ironclaw/issues/42";

        let trace = LlmTrace {
            model_name: "test-wasm-coercion-get-issue".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Get issue 42 from nearai/ironclaw".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_2".to_string(),
                                name: "github".to_string(),
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
            http_exchanges: vec![HttpExchange {
                request: HttpExchangeRequest {
                    method: "GET".to_string(),
                    url: expected_url.to_string(),
                    headers: vec![],
                    body: None,
                },
                response: github_ok(r#"{"number":42,"title":"Test","state":"open","body":"desc"}"#),
            }],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("github", GITHUB_WASM, Some(GITHUB_CAPS))
            .build()
            .await;

        rig.send_message("Get issue 42 from nearai/ironclaw").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;
        rig.verify_trace_expects(&trace, &responses);

        rig.shutdown();
    }

    /// LLM sends `limit: "25"` (string) to `list_pull_requests`. URL must
    /// contain `per_page=25`.
    #[tokio::test]
    async fn wasm_github_list_prs_coerces_string_limit() {
        if skip_if_no_wasm() {
            return;
        }

        let expected_url =
            "https://api.github.com/repos/nearai/ironclaw/pulls?state=open&per_page=25";

        let trace = LlmTrace {
            model_name: "test-wasm-coercion-list-prs".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "List PRs in nearai/ironclaw".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_3".to_string(),
                                name: "github".to_string(),
                                arguments: json!({
                                    "action": "list_pull_requests",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "limit": "25"
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
                            content: "Found PRs.".to_string(),
                            input_tokens: 100,
                            output_tokens: 10,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![HttpExchange {
                request: HttpExchangeRequest {
                    method: "GET".to_string(),
                    url: expected_url.to_string(),
                    headers: vec![],
                    body: None,
                },
                response: github_ok(r#"[{"number":1,"title":"Test PR","state":"open"}]"#),
            }],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("github", GITHUB_WASM, Some(GITHUB_CAPS))
            .build()
            .await;

        rig.send_message("List PRs in nearai/ironclaw").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;
        rig.verify_trace_expects(&trace, &responses);

        rig.shutdown();
    }
}
