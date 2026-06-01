use std::sync::Arc;

use ironclaw_wasm::{
    PreparedWitTool, RecordingWasmHostHttp, WasmHttpResponse, WitToolExecution, WitToolHost,
    WitToolRequest, WitToolRuntime, WitToolRuntimeConfig,
};
use serde_json::json;

#[test]
fn bundled_github_wasm_routes_every_capability_to_the_expected_github_contract() {
    let harness = GitHubWasmHarness::new();
    for case in github_capability_cases() {
        let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
            status: 200,
            headers_json: "{}".to_string(),
            body: br#"{"object":{"sha":"abc123"},"ok":true}"#.to_vec(),
        }));

        let execution = harness.execute(case.capability_id, case.input, http.clone());
        assert_eq!(
            execution.error.as_deref(),
            None,
            "{} failed with {execution:?}",
            case.capability_id
        );

        let requests = http.requests().unwrap();
        assert_eq!(
            requests.len(),
            case.requests.len(),
            "{} emitted unexpected request count",
            case.capability_id
        );

        for (request, expected) in requests.iter().zip(case.requests) {
            assert_eq!(request.method, expected.method, "{}", case.capability_id);
            assert_eq!(request.url, expected.url, "{}", case.capability_id);
            assert_eq!(request.timeout_ms, Some(10_000), "{}", case.capability_id);

            match expected.body {
                None => assert_eq!(
                    request.body, None,
                    "{} should not send a request body",
                    case.capability_id
                ),
                Some(expected_body) => assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(
                        request.body.as_deref().expect("request body")
                    )
                    .unwrap(),
                    expected_body,
                    "{} request body mismatch",
                    case.capability_id
                ),
            }

            let headers: serde_json::Value = serde_json::from_str(&request.headers_json).unwrap();
            assert_eq!(
                headers["User-Agent"], "IronClaw-GitHub-Reborn-WASM",
                "{}",
                case.capability_id
            );
            assert_eq!(
                headers["X-GitHub-Api-Version"], "2026-03-10",
                "{}",
                case.capability_id
            );
        }
    }
}

#[test]
fn bundled_github_wasm_normalizes_webhook_without_github_egress() {
    let harness = GitHubWasmHarness::new();
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: Vec::new(),
    }));
    let execution = harness.execute(
        "github.handle_webhook",
        json!({
            "webhook": {
                "headers": {
                    "X-GitHub-Event": "issue_comment",
                    "X-GitHub-Delivery": "delivery-123"
                },
                "body_json": {
                    "action": "created",
                    "repository": {"full_name": "nearai/ironclaw"},
                    "issue": {
                        "number": 4280,
                        "pull_request": {"url": "https://api.github.com/repos/nearai/ironclaw/pulls/4280"}
                    },
                    "comment": {"id": 99, "body": "looks good"}
                }
            }
        }),
        http.clone(),
    );

    assert_eq!(execution.error, None);
    assert!(http.requests().unwrap().is_empty());
    let output: serde_json::Value =
        serde_json::from_str(execution.output_json.as_deref().unwrap()).unwrap();
    assert_eq!(output["accepted"], json!(true));
    assert_eq!(
        output["emit_events"][0]["event_type"],
        json!("pr.comment.created")
    );
    assert_eq!(
        output["emit_events"][0]["payload"]["delivery_id"],
        json!("delivery-123")
    );
    assert_eq!(
        output["emit_events"][0]["payload"]["pr_number"],
        json!(4280)
    );
}

struct CapabilityCase {
    capability_id: &'static str,
    input: serde_json::Value,
    requests: Vec<ExpectedRequest>,
}

struct ExpectedRequest {
    method: &'static str,
    url: &'static str,
    body: Option<serde_json::Value>,
}

impl ExpectedRequest {
    fn get(url: &'static str) -> Self {
        Self {
            method: "GET",
            url,
            body: None,
        }
    }

    fn json(method: &'static str, url: &'static str, body: serde_json::Value) -> Self {
        Self {
            method,
            url,
            body: Some(body),
        }
    }
}

fn github_capability_cases() -> Vec<CapabilityCase> {
    vec![
        case(
            "github.get_repo",
            json!({"owner": "nearai", "repo": "ironclaw"}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw",
            )],
        ),
        case(
            "github.create_repo",
            json!({
                "name": "reborn-fixture",
                "description": "fixture repo",
                "private": true,
                "auto_init": true,
                "gitignore_template": "Rust",
                "license_template": "mit",
                "org": "nearai"
            }),
            vec![ExpectedRequest::json(
                "POST",
                "https://api.github.com/orgs/nearai/repos",
                json!({
                    "name": "reborn-fixture",
                    "description": "fixture repo",
                    "private": true,
                    "auto_init": true,
                    "gitignore_template": "Rust",
                    "license_template": "mit"
                }),
            )],
        ),
        case(
            "github.list_issues",
            json!({"owner": "nearai", "repo": "ironclaw", "state": "closed", "limit": 7, "page": 2}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/issues?state=closed&per_page=7&page=2",
            )],
        ),
        case(
            "github.create_issue",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "title": "matrix issue",
                "body": "body",
                "labels": ["qa", "reborn"]
            }),
            vec![ExpectedRequest::json(
                "POST",
                "https://api.github.com/repos/nearai/ironclaw/issues",
                json!({"title": "matrix issue", "body": "body", "labels": ["qa", "reborn"]}),
            )],
        ),
        case(
            "github.get_issue",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 42}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/issues/42",
            )],
        ),
        case(
            "github.list_issue_comments",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 42, "limit": 5, "page": 3}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/issues/42/comments?per_page=5&page=3",
            )],
        ),
        case(
            "github.create_issue_comment",
            issue_comment_input(),
            vec![issue_comment_request()],
        ),
        case(
            "github.comment_issue",
            issue_comment_input(),
            vec![issue_comment_request()],
        ),
        case(
            "github.list_pull_requests",
            json!({"owner": "nearai", "repo": "ironclaw", "state": "all", "limit": 9, "page": 4}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/pulls?state=all&per_page=9&page=4",
            )],
        ),
        case(
            "github.create_pull_request",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "title": "matrix pr",
                "head": "feature/matrix",
                "base": "main",
                "body": "body",
                "draft": true
            }),
            vec![ExpectedRequest::json(
                "POST",
                "https://api.github.com/repos/nearai/ironclaw/pulls",
                json!({
                    "title": "matrix pr",
                    "head": "feature/matrix",
                    "base": "main",
                    "body": "body",
                    "draft": true
                }),
            )],
        ),
        case(
            "github.get_pull_request",
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/pulls/4280",
            )],
        ),
        case(
            "github.get_pull_request_files",
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/pulls/4280/files",
            )],
        ),
        case(
            "github.create_pr_review",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "body": "review body",
                "event": "COMMENT"
            }),
            vec![ExpectedRequest::json(
                "POST",
                "https://api.github.com/repos/nearai/ironclaw/pulls/4280/reviews",
                json!({"body": "review body", "event": "COMMENT"}),
            )],
        ),
        case(
            "github.list_pull_request_comments",
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280, "limit": 6, "page": 2}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/pulls/4280/comments?per_page=6&page=2",
            )],
        ),
        case(
            "github.reply_pull_request_comment",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "comment_id": 123456789_u64,
                "body": "reply"
            }),
            vec![ExpectedRequest::json(
                "POST",
                "https://api.github.com/repos/nearai/ironclaw/pulls/4280/comments/123456789/replies",
                json!({"body": "reply"}),
            )],
        ),
        case(
            "github.get_pull_request_reviews",
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280, "limit": 8, "page": 3}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/pulls/4280/reviews?per_page=8&page=3",
            )],
        ),
        case(
            "github.get_combined_status",
            json!({"owner": "nearai", "repo": "ironclaw", "ref": "feature/matrix"}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/commits/feature%2Fmatrix/status",
            )],
        ),
        case(
            "github.merge_pull_request",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "commit_title": "merge title",
                "commit_message": "merge body",
                "merge_method": "squash"
            }),
            vec![ExpectedRequest::json(
                "PUT",
                "https://api.github.com/repos/nearai/ironclaw/pulls/4280/merge",
                json!({
                    "merge_method": "squash",
                    "commit_title": "merge title",
                    "commit_message": "merge body"
                }),
            )],
        ),
        case(
            "github.list_repos",
            json!({"username": "nearai", "limit": 11, "page": 2}),
            vec![ExpectedRequest::get(
                "https://api.github.com/users/nearai/repos?per_page=11&page=2",
            )],
        ),
        search_case(
            "github.search_repositories",
            "https://api.github.com/search/repositories?q=org%3Anearai%20ironclaw&per_page=12&page=3&sort=updated&order=desc",
        ),
        search_case(
            "github.search_code",
            "https://api.github.com/search/code?q=repo%3Anearai%2Fironclaw%20path%3Asrc%20Tool&per_page=12&page=3&sort=updated&order=desc",
        ),
        search_case(
            "github.search_issues_pull_requests",
            "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Apr&per_page=12&page=3&sort=updated&order=desc",
        ),
        search_case(
            "github.search_issues",
            "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Aissue&per_page=12&page=3&sort=updated&order=desc",
        ),
        case(
            "github.list_branches",
            json!({"owner": "nearai", "repo": "ironclaw", "protected": true, "limit": 13, "page": 2}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/branches?per_page=13&protected=true&page=2",
            )],
        ),
        case(
            "github.create_branch",
            json!({"owner": "nearai", "repo": "ironclaw", "branch": "feature/matrix", "from_ref": "main"}),
            vec![
                ExpectedRequest::get(
                    "https://api.github.com/repos/nearai/ironclaw/git/ref/heads/main",
                ),
                ExpectedRequest::json(
                    "POST",
                    "https://api.github.com/repos/nearai/ironclaw/git/refs",
                    json!({"ref": "refs/heads/feature/matrix", "sha": "abc123"}),
                ),
            ],
        ),
        case(
            "github.get_file_content",
            json!({"owner": "nearai", "repo": "ironclaw", "path": "docs/replay.md", "ref": "feature/matrix"}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/contents/docs/replay.md?ref=feature%2Fmatrix",
            )],
        ),
        case(
            "github.create_or_update_file",
            file_write_input(),
            vec![ExpectedRequest::json(
                "PUT",
                "https://api.github.com/repos/nearai/ironclaw/contents/docs/replay.md",
                json!({
                    "message": "write replay",
                    "content": "aGVsbG8=",
                    "sha": "abc123",
                    "branch": "feature/matrix",
                    "committer": {"name": "Commit Bot", "email": "commit@example.com"},
                    "author": {"name": "Author Bot", "email": "author@example.com"}
                }),
            )],
        ),
        case(
            "github.delete_file",
            file_delete_input(),
            vec![ExpectedRequest::json(
                "DELETE",
                "https://api.github.com/repos/nearai/ironclaw/contents/docs/replay.md",
                json!({
                    "message": "delete replay",
                    "sha": "abc123",
                    "branch": "feature/matrix",
                    "committer": {"name": "Commit Bot", "email": "commit@example.com"},
                    "author": {"name": "Author Bot", "email": "author@example.com"}
                }),
            )],
        ),
        case(
            "github.list_releases",
            json!({"owner": "nearai", "repo": "ironclaw", "limit": 14, "page": 2}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/releases?per_page=14&page=2",
            )],
        ),
        case(
            "github.create_release",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "tag_name": "v1.2.3",
                "target_commitish": "main",
                "name": "v1.2.3",
                "body": "release notes",
                "draft": true,
                "prerelease": false,
                "generate_release_notes": true
            }),
            vec![ExpectedRequest::json(
                "POST",
                "https://api.github.com/repos/nearai/ironclaw/releases",
                json!({
                    "tag_name": "v1.2.3",
                    "target_commitish": "main",
                    "name": "v1.2.3",
                    "body": "release notes",
                    "draft": true,
                    "prerelease": false,
                    "generate_release_notes": true
                }),
            )],
        ),
        case(
            "github.trigger_workflow",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "workflow_id": "ci.yml",
                "ref": "main",
                "inputs": {"suite": "smoke"}
            }),
            vec![ExpectedRequest::json(
                "POST",
                "https://api.github.com/repos/nearai/ironclaw/actions/workflows/ci.yml/dispatches",
                json!({"ref": "main", "inputs": {"suite": "smoke"}}),
            )],
        ),
        case(
            "github.get_workflow_runs",
            json!({"owner": "nearai", "repo": "ironclaw", "workflow_id": "ci.yml", "limit": 15, "page": 2}),
            vec![ExpectedRequest::get(
                "https://api.github.com/repos/nearai/ironclaw/actions/workflows/ci.yml/runs?per_page=15&page=2",
            )],
        ),
        case(
            "github.fork_repo",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "organization": "nearai-labs",
                "name": "ironclaw-fork",
                "default_branch_only": true
            }),
            vec![ExpectedRequest::json(
                "POST",
                "https://api.github.com/repos/nearai/ironclaw/forks",
                json!({
                    "organization": "nearai-labs",
                    "name": "ironclaw-fork",
                    "default_branch_only": true
                }),
            )],
        ),
    ]
}

fn case(
    capability_id: &'static str,
    input: serde_json::Value,
    requests: Vec<ExpectedRequest>,
) -> CapabilityCase {
    CapabilityCase {
        capability_id,
        input,
        requests,
    }
}

fn search_case(capability_id: &'static str, expected_url: &'static str) -> CapabilityCase {
    let query = match capability_id {
        "github.search_repositories" => "org:nearai ironclaw",
        "github.search_code" => "repo:nearai/ironclaw path:src Tool",
        "github.search_issues_pull_requests" => "repo:nearai/ironclaw is:pr",
        "github.search_issues" => "repo:nearai/ironclaw is:issue",
        _ => unreachable!("unknown search capability"),
    };
    case(
        capability_id,
        json!({"query": query, "limit": 12, "page": 3, "sort": "updated", "order": "desc"}),
        vec![ExpectedRequest::get(expected_url)],
    )
}

fn issue_comment_input() -> serde_json::Value {
    json!({
        "owner": "nearai",
        "repo": "ironclaw",
        "issue_number": 42,
        "body": "matrix comment"
    })
}

fn issue_comment_request() -> ExpectedRequest {
    ExpectedRequest::json(
        "POST",
        "https://api.github.com/repos/nearai/ironclaw/issues/42/comments",
        json!({"body": "matrix comment"}),
    )
}

fn file_write_input() -> serde_json::Value {
    json!({
        "owner": "nearai",
        "repo": "ironclaw",
        "path": "docs/replay.md",
        "message": "write replay",
        "content": "hello",
        "sha": "abc123",
        "branch": "feature/matrix",
        "committer": {"name": "Commit Bot", "email": "commit@example.com"},
        "author": {"name": "Author Bot", "email": "author@example.com"}
    })
}

fn file_delete_input() -> serde_json::Value {
    json!({
        "owner": "nearai",
        "repo": "ironclaw",
        "path": "docs/replay.md",
        "message": "delete replay",
        "sha": "abc123",
        "branch": "feature/matrix",
        "committer": {"name": "Commit Bot", "email": "commit@example.com"},
        "author": {"name": "Author Bot", "email": "author@example.com"}
    })
}

struct GitHubWasmHarness {
    runtime: WitToolRuntime,
    prepared: PreparedWitTool,
}

impl GitHubWasmHarness {
    fn new() -> Self {
        let runtime = WitToolRuntime::new(WitToolRuntimeConfig::default()).unwrap();
        let wasm_bytes =
            std::fs::read(github_wasm_path()).expect("first-party GitHub WASM must be built");
        let prepared = runtime.prepare("github", &wasm_bytes).unwrap();
        Self { runtime, prepared }
    }

    fn execute(
        &self,
        capability_id: &str,
        input: serde_json::Value,
        http: Arc<RecordingWasmHostHttp>,
    ) -> WitToolExecution {
        self.runtime
            .execute(
                &self.prepared,
                WitToolHost::deny_all().with_http(http),
                WitToolRequest::new(input.to_string()).with_context(
                    json!({
                        "capability_id": capability_id,
                    })
                    .to_string(),
                ),
            )
            .unwrap()
    }
}

fn github_wasm_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates/ironclaw_first_party_extensions/assets/github/wasm/github_tool.wasm")
}
