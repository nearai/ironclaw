//! E2E test: real github WASM tool with parameter coercion via TestRig.
//!
//! Loads the compiled github WASM binary into the test rig, replays an LLM
//! trace that sends string-typed numeric params, and verifies the WASM tool
//! constructs the correct HTTP API call via `http_exchanges` in the trace.
//!
//! These tests are `#[ignore]` by default because they require a pre-compiled
//! WASM binary. Build it with:
//!   cargo build -p github-tool --target wasm32-wasip2 --release
//! Then run with:
//!   cargo test --features libsql --test e2e_wasm_github_coercion -- --ignored

#[cfg(feature = "libsql")]
mod support;

/// Note on URL verification: the `ReplayingHttpInterceptor` logs warnings on
/// URL mismatch but still returns the canned response. The real verification is
/// that the tool succeeds end-to-end: coercion produced the correct typed
/// parameters, serde deserialization succeeded, and the WASM tool constructed a
/// valid HTTP request. A URL mismatch warning in logs does not indicate test
/// failure — it is a soft check only.
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

    fn github_exchange(
        method: &str,
        url: &str,
        body: Option<String>,
        response_body: &str,
    ) -> HttpExchange {
        HttpExchange {
            request: HttpExchangeRequest {
                method: method.to_string(),
                url: url.to_string(),
                headers: vec![],
                body,
            },
            response: github_ok(response_body),
        }
    }

    async fn run_trace(trace: LlmTrace, prompt: &str) {
        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_wasm_tool("github", GITHUB_WASM, Some(GITHUB_CAPS.into()))
            .build()
            .await;

        rig.send_message(prompt).await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;
        rig.verify_trace_expects(&trace, &responses);

        rig.shutdown();
    }

    /// LLM sends `limit: "50"` (string) to `list_issues`. Coercion converts it
    /// to integer, and the WASM tool must call `GET /repos/.../issues?...&per_page=50`.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_list_issues_coerces_string_limit() {
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

        run_trace(trace, "List issues in nearai/ironclaw with limit 50").await;
    }

    /// LLM sends `issue_number: "42"` (string) to `get_issue`. Coercion converts
    /// it to integer, and the URL must contain `/issues/42`.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_get_issue_coerces_string_issue_number() {
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

        run_trace(trace, "Get issue 42 from nearai/ironclaw").await;
    }

    /// LLM sends `limit: "25"` (string) to `list_pull_requests`. URL must
    /// contain `per_page=25`.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_list_prs_coerces_string_limit() {
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

        run_trace(trace, "List PRs in nearai/ironclaw").await;
    }

    /// LLM sends pagination as strings to `search_code`. Coercion converts them
    /// to integers, and the tool must construct the expected search query.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_search_code_coerces_string_pagination() {
        let expected_url = "https://api.github.com/search/code?q=repo%3Anearai%2Fironclaw%20path%3Asrc%20Tool&per_page=10&page=2&sort=indexed&order=desc";

        let trace = LlmTrace {
            model_name: "test-wasm-coercion-search-code".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Search code in nearai/ironclaw".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_4".to_string(),
                                name: "github".to_string(),
                                arguments: json!({
                                    "action": "search_code",
                                    "query": "repo:nearai/ironclaw path:src Tool",
                                    "limit": "10",
                                    "page": "2",
                                    "sort": "indexed",
                                    "order": "desc"
                                }),
                            }],
                            input_tokens: 120,
                            output_tokens: 40,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Found matching code results.".to_string(),
                            input_tokens: 180,
                            output_tokens: 12,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![github_exchange(
                "GET",
                expected_url,
                None,
                r#"{"total_count":1,"items":[{"name":"lib.rs","path":"src/lib.rs"}]}"#,
            )],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        run_trace(trace, "Search code in nearai/ironclaw").await;
    }

    /// `create_repo` must target the org repos endpoint and send the expected
    /// JSON body for repository creation.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_create_repo_posts_expected_payload() {
        let expected_url = "https://api.github.com/orgs/nearai/repos";
        let expected_body = json!({
            "name": "github-tool-replay",
            "private": true,
            "auto_init": true,
            "description": "Replay-created repo",
            "gitignore_template": "Rust",
            "license_template": "mit"
        })
        .to_string();

        let trace = LlmTrace {
            model_name: "test-wasm-create-repo".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Create a private repo for nearai".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_5".to_string(),
                                name: "github".to_string(),
                                arguments: json!({
                                    "action": "create_repo",
                                    "name": "github-tool-replay",
                                    "description": "Replay-created repo",
                                    "private": true,
                                    "auto_init": true,
                                    "gitignore_template": "Rust",
                                    "license_template": "mit",
                                    "org": "nearai"
                                }),
                            }],
                            input_tokens: 110,
                            output_tokens: 40,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Repository created.".to_string(),
                            input_tokens: 150,
                            output_tokens: 12,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![github_exchange(
                "POST",
                expected_url,
                Some(expected_body),
                r#"{"name":"github-tool-replay","private":true}"#,
            )],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        run_trace(trace, "Create a private repo for nearai").await;
    }

    /// `create_branch` should fetch the source ref SHA and then create the new
    /// branch ref in a second request.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_create_branch_replays_two_step_ref_flow() {
        let source_ref_url = "https://api.github.com/repos/nearai/ironclaw/git/ref/heads/main";
        let create_ref_url = "https://api.github.com/repos/nearai/ironclaw/git/refs";
        let create_ref_body = json!({
            "ref": "refs/heads/feature/replay-test",
            "sha": "abc123def4567890abc123def4567890abc123de"
        })
        .to_string();

        let trace = LlmTrace {
            model_name: "test-wasm-create-branch".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Create a replay branch from main".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_6".to_string(),
                                name: "github".to_string(),
                                arguments: json!({
                                    "action": "create_branch",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "branch": "feature/replay-test",
                                    "from_ref": "main"
                                }),
                            }],
                            input_tokens: 100,
                            output_tokens: 35,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Branch created.".to_string(),
                            input_tokens: 145,
                            output_tokens: 10,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![
                github_exchange(
                    "GET",
                    source_ref_url,
                    None,
                    r#"{"ref":"refs/heads/main","object":{"sha":"abc123def4567890abc123def4567890abc123de"}}"#,
                ),
                github_exchange(
                    "POST",
                    create_ref_url,
                    Some(create_ref_body),
                    r#"{"ref":"refs/heads/feature/replay-test"}"#,
                ),
            ],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        run_trace(trace, "Create a replay branch from main").await;
    }

    /// `create_or_update_file` must target the contents API and send base64
    /// encoded file contents in the request body.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_create_or_update_file_puts_contents_payload() {
        let expected_url = "https://api.github.com/repos/nearai/ironclaw/contents/docs/replay.md";
        let expected_body = json!({
            "message": "Add replay doc",
            "content": "IyBSZXBsYXkgZG9jCg==",
            "branch": "feature/replay-test",
            "committer": {
                "name": "IronClaw Bot",
                "email": "bot@example.com"
            }
        })
        .to_string();

        let trace = LlmTrace {
            model_name: "test-wasm-create-or-update-file".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Write docs/replay.md".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_7".to_string(),
                                name: "github".to_string(),
                                arguments: json!({
                                    "action": "create_or_update_file",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "path": "docs/replay.md",
                                    "message": "Add replay doc",
                                    "content": "# Replay doc\n",
                                    "branch": "feature/replay-test",
                                    "committer": {
                                        "name": "IronClaw Bot",
                                        "email": "bot@example.com"
                                    }
                                }),
                            }],
                            input_tokens: 120,
                            output_tokens: 40,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "File written.".to_string(),
                            input_tokens: 160,
                            output_tokens: 10,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![github_exchange(
                "PUT",
                expected_url,
                Some(expected_body),
                r#"{"content":{"path":"docs/replay.md"},"commit":{"sha":"deadbeef"}}"#,
            )],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        run_trace(trace, "Write docs/replay.md").await;
    }

    /// `delete_file` must call DELETE on the contents endpoint with the blob SHA
    /// and commit metadata in the request body.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_delete_file_deletes_contents_with_sha() {
        let expected_url = "https://api.github.com/repos/nearai/ironclaw/contents/docs/replay.md";
        let expected_body = json!({
            "message": "Remove replay doc",
            "sha": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "branch": "feature/replay-test"
        })
        .to_string();

        let trace = LlmTrace {
            model_name: "test-wasm-delete-file".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Delete docs/replay.md".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_8".to_string(),
                                name: "github".to_string(),
                                arguments: json!({
                                    "action": "delete_file",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "path": "docs/replay.md",
                                    "message": "Remove replay doc",
                                    "sha": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                                    "branch": "feature/replay-test"
                                }),
                            }],
                            input_tokens: 110,
                            output_tokens: 36,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "File deleted.".to_string(),
                            input_tokens: 150,
                            output_tokens: 10,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![github_exchange(
                "DELETE",
                expected_url,
                Some(expected_body),
                r#"{"commit":{"sha":"feedface"}}"#,
            )],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        run_trace(trace, "Delete docs/replay.md").await;
    }

    /// `create_release` should post release metadata to the releases endpoint.
    #[tokio::test]
    #[ignore] // requires pre-compiled WASM binary
    async fn wasm_github_create_release_posts_expected_payload() {
        let expected_url = "https://api.github.com/repos/nearai/ironclaw/releases";
        let expected_body = json!({
            "tag_name": "v1.2.3",
            "draft": false,
            "prerelease": true,
            "generate_release_notes": true,
            "target_commitish": "main",
            "name": "Replay Release",
            "body": "Generated during replay testing"
        })
        .to_string();

        let trace = LlmTrace {
            model_name: "test-wasm-create-release".to_string(),
            turns: vec![crate::support::trace_llm::TraceTurn {
                user_input: "Create a prerelease".to_string(),
                steps: vec![
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::ToolCalls {
                            tool_calls: vec![TraceToolCall {
                                id: "call_gh_9".to_string(),
                                name: "github".to_string(),
                                arguments: json!({
                                    "action": "create_release",
                                    "owner": "nearai",
                                    "repo": "ironclaw",
                                    "tag_name": "v1.2.3",
                                    "target_commitish": "main",
                                    "name": "Replay Release",
                                    "body": "Generated during replay testing",
                                    "draft": false,
                                    "prerelease": true,
                                    "generate_release_notes": true
                                }),
                            }],
                            input_tokens: 125,
                            output_tokens: 40,
                        },
                        expected_tool_results: Vec::new(),
                    },
                    TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "Release created.".to_string(),
                            input_tokens: 170,
                            output_tokens: 10,
                        },
                        expected_tool_results: Vec::new(),
                    },
                ],
                expects: TraceExpects::default(),
            }],
            memory_snapshot: Vec::new(),
            http_exchanges: vec![github_exchange(
                "POST",
                expected_url,
                Some(expected_body),
                r#"{"id":1,"tag_name":"v1.2.3"}"#,
            )],
            expects: TraceExpects {
                tools_used: vec!["github".to_string()],
                all_tools_succeeded: Some(true),
                max_tool_calls: Some(1),
                min_responses: Some(1),
                ..Default::default()
            },
            steps: Vec::new(),
        };

        run_trace(trace, "Create a prerelease").await;
    }
}
