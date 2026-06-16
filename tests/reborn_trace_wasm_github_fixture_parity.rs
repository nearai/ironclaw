#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::time::Duration;

use ironclaw_host_api::{CapabilityId, NetworkMethod};
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_network::NetworkHttpRequest;
use ironclaw_turns::TurnStatus;
use reborn_support::{
    harness::{HarnessWaitConfig, RebornBinaryE2EHarness},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};
use serde_json::json;

#[tokio::test]
async fn reborn_trace_advertises_github_v2_wasm_capabilities() {
    let expected_capabilities =
        reborn_support::github::capability_ids().expect("valid GitHub capability ids");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::AssertProviderToolsThenResponse {
            capability_ids: expected_capabilities,
            response: HostManagedModelResponse::assistant_reply("github wasm trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_github_issue_capabilities(
        "room-trace-github-wasm",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-trace-github-wasm", "show GitHub issue tools")
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            HarnessWaitConfig {
                timeout: Duration::from_secs(15),
                poll_interval: Duration::from_millis(10),
            },
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply("github wasm trace complete")
        .await
        .expect("final reply");

    assert_eq!(
        harness.capability_invocations(),
        Vec::new(),
        "advertisement trace must not call live GitHub or execute the WASM module"
    );
    let requests = harness.model_requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.role == HostManagedModelMessageRole::User
                && message.content.contains("show GitHub issue tools")),
        "trace should exercise the real inbound user-to-model path"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_executes_github_v2_wasm_capability_matrix() {
    let capability_calls = github_capability_calls();
    let expected_capabilities =
        reborn_support::github::capability_ids().expect("valid GitHub capability ids");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: capability_calls.clone(),
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("github wasm matrix complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_github_issue_capabilities(
        "room-trace-github-wasm-matrix",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-github-wasm-matrix",
            "exercise every GitHub WASM capability",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            HarnessWaitConfig {
                timeout: Duration::from_secs(15),
                poll_interval: Duration::from_millis(10),
            },
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply("github wasm matrix complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), capability_calls.len());
    for capability_id in expected_capabilities {
        assert!(
            invocations
                .iter()
                .any(|invocation| invocation.capability_id == capability_id),
            "missing invocation for {}",
            capability_id.as_str()
        );
    }

    let requests = harness.network_http_requests();
    assert_requests_match(&requests, expected_github_http_requests());
    assert_eq!(requests.len(), expected_github_http_requests().len());
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

struct ExpectedGithubHttpRequest {
    method: NetworkMethod,
    url: &'static str,
    body: Option<serde_json::Value>,
}

fn github_capability_calls() -> Vec<RebornScriptedProviderToolCall> {
    vec![
        call(
            "github.get_repo",
            "get-repo",
            json!({"owner": "nearai", "repo": "ironclaw"}),
        ),
        call(
            "github.create_repo",
            "create-repo",
            json!({
                "name": "reborn-fixture",
                "description": "fixture repo",
                "private": true,
                "auto_init": true,
                "gitignore_template": "Rust",
                "license_template": "mit",
                "org": "nearai"
            }),
        ),
        call(
            "github.list_issues",
            "list-issues",
            json!({"owner": "nearai", "repo": "ironclaw", "state": "closed", "limit": 7, "page": 2}),
        ),
        call(
            "github.create_issue",
            "create-issue",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "title": "matrix issue",
                "body": "body",
                "labels": ["qa", "reborn"]
            }),
        ),
        call(
            "github.get_issue",
            "get-issue",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 42}),
        ),
        call(
            "github.list_issue_comments",
            "list-issue-comments",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 42, "limit": 5, "page": 3}),
        ),
        call(
            "github.create_issue_comment",
            "create-issue-comment",
            issue_comment_input(),
        ),
        call(
            "github.list_pull_requests",
            "list-prs",
            json!({"owner": "nearai", "repo": "ironclaw", "state": "all", "limit": 9, "page": 4}),
        ),
        call(
            "github.create_pull_request",
            "create-pr",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "title": "matrix pr",
                "head": "feature/matrix",
                "base": "main",
                "body": "body",
                "draft": true
            }),
        ),
        call(
            "github.get_pull_request",
            "get-pr",
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280}),
        ),
        call(
            "github.get_pull_request_files",
            "get-pr-files",
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280}),
        ),
        call(
            "github.create_pr_review",
            "create-pr-review",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "body": "review body",
                "event": "COMMENT"
            }),
        ),
        call(
            "github.list_pull_request_comments",
            "list-pr-comments",
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280, "limit": 6, "page": 2}),
        ),
        call(
            "github.reply_pull_request_comment",
            "reply-pr-comment",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "comment_id": 123456789_u64,
                "body": "reply"
            }),
        ),
        call(
            "github.get_pull_request_reviews",
            "get-pr-reviews",
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280, "limit": 8, "page": 3}),
        ),
        call(
            "github.get_combined_status",
            "get-status",
            json!({"owner": "nearai", "repo": "ironclaw", "ref": "feature/matrix"}),
        ),
        call(
            "github.merge_pull_request",
            "merge-pr",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "commit_title": "merge title",
                "commit_message": "merge body",
                "merge_method": "squash"
            }),
        ),
        call(
            "github.list_repos",
            "list-repos",
            json!({"username": "nearai", "limit": 11, "page": 2}),
        ),
        call(
            "github.search_repositories",
            "search-repos",
            json!({"query": "org:nearai ironclaw", "limit": 12, "page": 3, "sort": "updated", "order": "desc"}),
        ),
        call(
            "github.search_code",
            "search-code",
            json!({"query": "repo:nearai/ironclaw path:src Tool", "limit": 12, "page": 3, "sort": "updated", "order": "desc"}),
        ),
        call(
            "github.search_issues_pull_requests",
            "search-issues-prs",
            json!({"query": "repo:nearai/ironclaw is:pr", "limit": 12, "page": 3, "sort": "updated", "order": "desc"}),
        ),
        call(
            "github.list_branches",
            "list-branches",
            json!({"owner": "nearai", "repo": "ironclaw", "protected": true, "limit": 13, "page": 2}),
        ),
        call(
            "github.create_branch",
            "create-branch",
            json!({"owner": "nearai", "repo": "ironclaw", "branch": "feature/matrix", "from_ref": "main"}),
        ),
        call(
            "github.get_file_content",
            "get-file",
            json!({"owner": "nearai", "repo": "ironclaw", "path": "docs/replay.md", "ref": "feature/matrix"}),
        ),
        call(
            "github.create_or_update_file",
            "write-file",
            file_write_input(),
        ),
        call("github.delete_file", "delete-file", file_delete_input()),
        call(
            "github.list_releases",
            "list-releases",
            json!({"owner": "nearai", "repo": "ironclaw", "limit": 14, "page": 2}),
        ),
        call(
            "github.create_release",
            "create-release",
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
        ),
        call(
            "github.trigger_workflow",
            "trigger-workflow",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "workflow_id": "ci.yml",
                "ref": "main",
                "inputs": {"suite": "smoke"}
            }),
        ),
        call(
            "github.get_workflow_runs",
            "get-workflow-runs",
            json!({"owner": "nearai", "repo": "ironclaw", "workflow_id": "ci.yml", "limit": 15, "page": 2}),
        ),
        call(
            "github.fork_repo",
            "fork-repo",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "organization": "nearai-labs",
                "name": "ironclaw-fork",
                "default_branch_only": true
            }),
        ),
        call(
            "github.handle_webhook",
            "handle-webhook",
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
        ),
    ]
}

fn expected_github_http_requests() -> Vec<ExpectedGithubHttpRequest> {
    vec![
        get("https://api.github.com/repos/nearai/ironclaw"),
        request(
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
        ),
        get(
            "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20state%3Aclosed%20is%3Aissue&per_page=7&page=2&sort=created&order=desc",
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/issues",
            json!({"title": "matrix issue", "body": "body", "labels": ["qa", "reborn"]}),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/issues/42"),
        get("https://api.github.com/repos/nearai/ironclaw/issues/42/comments?per_page=5&page=3"),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/issues/42/comments",
            json!({"body": "matrix comment"}),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/pulls?state=all&per_page=9&page=4"),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/pulls",
            json!({
                "title": "matrix pr",
                "head": "feature/matrix",
                "base": "main",
                "body": "body",
                "draft": true
            }),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/pulls/4280"),
        get("https://api.github.com/repos/nearai/ironclaw/pulls/4280/files"),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/pulls/4280/reviews",
            json!({"body": "review body", "event": "COMMENT"}),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/pulls/4280/comments?per_page=6&page=2"),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/pulls/4280/comments/123456789/replies",
            json!({"body": "reply"}),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/pulls/4280/reviews?per_page=8&page=3"),
        get("https://api.github.com/repos/nearai/ironclaw/commits/feature%2Fmatrix/status"),
        request(
            "PUT",
            "https://api.github.com/repos/nearai/ironclaw/pulls/4280/merge",
            json!({
                "merge_method": "squash",
                "commit_title": "merge title",
                "commit_message": "merge body"
            }),
        ),
        get("https://api.github.com/users/nearai/repos?per_page=11&page=2"),
        get(
            "https://api.github.com/search/repositories?q=org%3Anearai%20ironclaw&per_page=12&page=3&sort=updated&order=desc",
        ),
        get(
            "https://api.github.com/search/code?q=repo%3Anearai%2Fironclaw%20path%3Asrc%20Tool&per_page=12&page=3&sort=updated&order=desc",
        ),
        get(
            "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Apr&per_page=12&page=3&sort=updated&order=desc",
        ),
        get(
            "https://api.github.com/repos/nearai/ironclaw/branches?per_page=13&protected=true&page=2",
        ),
        get("https://api.github.com/repos/nearai/ironclaw/git/ref/heads/main"),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/git/refs",
            json!({"ref": "refs/heads/feature/matrix", "sha": "abc123def4567890abc123def4567890abc123de"}),
        ),
        get(
            "https://api.github.com/repos/nearai/ironclaw/contents/docs/replay.md?ref=feature%2Fmatrix",
        ),
        request(
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
        ),
        request(
            "DELETE",
            "https://api.github.com/repos/nearai/ironclaw/contents/docs/replay.md",
            json!({
                "message": "delete replay",
                "sha": "abc123",
                "branch": "feature/matrix",
                "committer": {"name": "Commit Bot", "email": "commit@example.com"},
                "author": {"name": "Author Bot", "email": "author@example.com"}
            }),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/releases?per_page=14&page=2"),
        request(
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
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/actions/workflows/ci.yml/dispatches",
            json!({"ref": "main", "inputs": {"suite": "smoke"}}),
        ),
        get(
            "https://api.github.com/repos/nearai/ironclaw/actions/workflows/ci.yml/runs?per_page=15&page=2",
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/forks",
            json!({
                "organization": "nearai-labs",
                "name": "ironclaw-fork",
                "default_branch_only": true
            }),
        ),
    ]
}

fn assert_requests_match(actual: &[NetworkHttpRequest], expected: Vec<ExpectedGithubHttpRequest>) {
    for expected_request in expected {
        let Some((index, request)) = actual.iter().enumerate().find(|(_, request)| {
            request.method == expected_request.method && request.url == expected_request.url
        }) else {
            panic!(
                "missing GitHub HTTP request {}; actual requests: {:#?}",
                expected_request.url,
                actual
                    .iter()
                    .map(|request| (&request.method, request.url.as_str()))
                    .collect::<Vec<_>>()
            );
        };
        if let Some(expected_body) = expected_request.body {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&request.body).unwrap(),
                expected_body,
                "request body mismatch for {}",
                request.url
            );
        } else {
            assert!(
                request.body.is_empty(),
                "expected empty request body for {}",
                request.url
            );
        }
        assert_eq!(request.timeout_ms, Some(10_000), "{}", request.url);
        assert_ne!(
            request
                .headers
                .iter()
                .find(|(name, _)| name == "User-Agent"),
            None,
            "{} missing User-Agent header",
            request.url
        );
        assert!(index < actual.len());
    }
}

fn call(
    capability_id: &str,
    call_id: &'static str,
    arguments: serde_json::Value,
) -> RebornScriptedProviderToolCall {
    RebornScriptedProviderToolCall::new(
        CapabilityId::new(capability_id).expect("valid capability id"),
        call_id.replace('-', "_"),
        arguments,
    )
}

fn get(url: &'static str) -> ExpectedGithubHttpRequest {
    ExpectedGithubHttpRequest {
        method: NetworkMethod::Get,
        url,
        body: None,
    }
}

fn request(
    method: &'static str,
    url: &'static str,
    body: serde_json::Value,
) -> ExpectedGithubHttpRequest {
    let method = match method {
        "POST" => NetworkMethod::Post,
        "PUT" => NetworkMethod::Put,
        "DELETE" => NetworkMethod::Delete,
        _ => unreachable!("unsupported test method"),
    };
    ExpectedGithubHttpRequest {
        method,
        url,
        body: Some(body),
    }
}

fn issue_comment_input() -> serde_json::Value {
    json!({
        "owner": "nearai",
        "repo": "ironclaw",
        "issue_number": 42,
        "body": "matrix comment"
    })
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
