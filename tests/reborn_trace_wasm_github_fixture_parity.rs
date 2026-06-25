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
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "state": "closed",
                "labels": ["qa", "reborn"],
                "assignee": "henry",
                "milestone": "12",
                "limit": 7,
                "page": 2
            }),
        ),
        call(
            "github.create_issue",
            "create-issue",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "title": "matrix issue",
                "body": "body",
                "milestone": 7,
                "labels": ["qa", "reborn"],
                "assignees": ["henry"]
            }),
        ),
        call(
            "github.update_issue",
            "update-issue",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "issue_number": 42,
                "state": "closed",
                "labels": ["qa"],
                "assignees": ["henry"],
                "milestone": 7
            }),
        ),
        call(
            "github.add_issue_labels",
            "add-issue-labels",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 42, "labels": ["api", "reborn"]}),
        ),
        call(
            "github.remove_issue_label",
            "remove-issue-label",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 42, "name": "needs review"}),
        ),
        call(
            "github.add_issue_assignees",
            "add-issue-assignees",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 42, "assignees": ["henry"]}),
        ),
        call(
            "github.remove_issue_assignees",
            "remove-issue-assignees",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 42, "assignees": ["henry"]}),
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
        // Compatibility alias for github.create_issue_comment — routes through the
        // same WASM path and must stay model-callable (visibility = "model").
        call(
            "github.comment_issue",
            "comment-issue",
            json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 77, "body": "alias comment"}),
        ),
        call(
            "github.list_pull_requests",
            "list-prs",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "state": "all",
                "head": "henry:fix/github-tool-api-correctness",
                "base": "main",
                "sort": "updated",
                "direction": "asc",
                "limit": 9,
                "page": 4
            }),
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
                "head_repo": "ironclaw-fork",
                "maintainer_can_modify": true,
                "draft": true
            }),
        ),
        call(
            "github.update_pull_request",
            "update-pr",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "state": "closed",
                "base": "release",
                "maintainer_can_modify": false
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
            json!({"owner": "nearai", "repo": "ironclaw", "pr_number": 4280, "limit": 10, "page": 2}),
        ),
        call(
            "github.create_pr_review",
            "create-pr-review",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "body": "review body",
                "event": "COMMENT",
                "commit_id": "abc123def4567890abc123def4567890abc123de",
                "comments": [
                    {"path": "src/lib.rs", "body": "inline", "line": 10, "side": "RIGHT"}
                ]
            }),
        ),
        call(
            "github.list_pull_request_comments",
            "list-pr-comments",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "sort": "updated",
                "direction": "desc",
                "since": "2026-06-23T00:00:00Z",
                "limit": 6,
                "page": 2
            }),
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
            "github.list_pull_request_review_threads",
            "list-pr-review-threads",
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "first": 12,
                "after": "cursor-1"
            }),
        ),
        call(
            "github.resolve_review_thread",
            "resolve-review-thread",
            json!({"thread_id": "PRRT_kwDOExample"}),
        ),
        call(
            "github.unresolve_review_thread",
            "unresolve-review-thread",
            json!({"thread_id": "PRRT_kwDOExample"}),
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
                "merge_method": "squash",
                "sha": "abc123def4567890abc123def4567890abc123de"
            }),
        ),
        call(
            "github.get_authenticated_user",
            "get-authenticated-user",
            json!({}),
        ),
        call(
            "github.list_repos",
            "list-repos",
            json!({"type": "member", "limit": 11, "page": 2}),
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
        // Compatibility alias for github.search_issues_pull_requests — routes through
        // the same WASM path and must stay model-callable (visibility = "model").
        call(
            "github.search_issues",
            "search-issues",
            json!({"query": "repo:nearai/ironclaw is:issue", "limit": 12, "page": 3, "sort": "updated", "order": "desc"}),
        ),
        call(
            "github.search_issues_pull_requests",
            "search-issues-prs",
            json!({"query": "repo:nearai/ironclaw is:pr", "limit": 12, "page": 3, "sort": "reactions-heart", "order": "desc"}),
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
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "workflow_id": "ci.yml",
                "event": "pull_request",
                "status": "failure",
                "head_sha": "abc123def4567890abc123def4567890abc123de",
                "limit": 15,
                "page": 2
            }),
        ),
        call(
            "github.get_workflow_run_jobs",
            "get-workflow-run-jobs",
            json!({"owner": "nearai", "repo": "ironclaw", "run_id": 12345, "filter": "all", "limit": 16, "page": 2}),
        ),
        call(
            "github.get_workflow_run_artifacts",
            "get-workflow-run-artifacts",
            json!({"owner": "nearai", "repo": "ironclaw", "run_id": 12345, "name": "coverage", "direction": "asc", "limit": 17, "page": 3}),
        ),
        call(
            "github.rerun_failed_workflow_run_jobs",
            "rerun-failed-workflow-run-jobs",
            json!({"owner": "nearai", "repo": "ironclaw", "run_id": 12345, "enable_debug_logging": true}),
        ),
        call(
            "github.rerun_workflow_job",
            "rerun-workflow-job",
            json!({"owner": "nearai", "repo": "ironclaw", "job_id": 67890, "enable_debugger": true}),
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
            "https://api.github.com/repos/nearai/ironclaw/issues?state=closed&per_page=7&page=1&labels=qa%2Creborn&assignee=henry&milestone=12",
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/issues",
            json!({"title": "matrix issue", "body": "body", "milestone": 7, "labels": ["qa", "reborn"], "assignees": ["henry"]}),
        ),
        request(
            "PATCH",
            "https://api.github.com/repos/nearai/ironclaw/issues/42",
            json!({"state": "closed", "labels": ["qa"], "assignees": ["henry"], "milestone": 7}),
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/issues/42/labels",
            json!({"labels": ["api", "reborn"]}),
        ),
        delete("https://api.github.com/repos/nearai/ironclaw/issues/42/labels/needs%20review"),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/issues/42/assignees",
            json!({"assignees": ["henry"]}),
        ),
        request(
            "DELETE",
            "https://api.github.com/repos/nearai/ironclaw/issues/42/assignees",
            json!({"assignees": ["henry"]}),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/issues/42"),
        get("https://api.github.com/repos/nearai/ironclaw/issues/42/comments?per_page=5&page=3"),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/issues/42/comments",
            json!({"body": "matrix comment"}),
        ),
        // github.comment_issue alias → identical create-comment endpoint, distinct issue.
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/issues/77/comments",
            json!({"body": "alias comment"}),
        ),
        get(
            "https://api.github.com/repos/nearai/ironclaw/pulls?state=all&per_page=9&head=henry%3Afix%2Fgithub-tool-api-correctness&base=main&sort=updated&direction=asc&page=4",
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/pulls",
            json!({
                "title": "matrix pr",
                "head": "feature/matrix",
                "base": "main",
                "body": "body",
                "head_repo": "ironclaw-fork",
                "maintainer_can_modify": true,
                "draft": true
            }),
        ),
        request(
            "PATCH",
            "https://api.github.com/repos/nearai/ironclaw/pulls/4280",
            json!({"state": "closed", "base": "release", "maintainer_can_modify": false}),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/pulls/4280"),
        get("https://api.github.com/repos/nearai/ironclaw/pulls/4280/files?per_page=10&page=2"),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/pulls/4280/reviews",
            json!({
                "body": "review body",
                "event": "COMMENT",
                "commit_id": "abc123def4567890abc123def4567890abc123de",
                "comments": [
                    {"path": "src/lib.rs", "body": "inline", "line": 10, "side": "RIGHT"}
                ]
            }),
        ),
        get(
            "https://api.github.com/repos/nearai/ironclaw/pulls/4280/comments?per_page=6&sort=updated&direction=desc&since=2026-06-23T00%3A00%3A00Z&page=2",
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/pulls/4280/comments/123456789/replies",
            json!({"body": "reply"}),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/pulls/4280/reviews?per_page=8&page=3"),
        request(
            "POST",
            "https://api.github.com/graphql",
            review_threads_query_body(),
        ),
        request(
            "POST",
            "https://api.github.com/graphql",
            resolve_review_thread_body(),
        ),
        request(
            "POST",
            "https://api.github.com/graphql",
            unresolve_review_thread_body(),
        ),
        get("https://api.github.com/repos/nearai/ironclaw/commits/feature%2Fmatrix/status"),
        request(
            "PUT",
            "https://api.github.com/repos/nearai/ironclaw/pulls/4280/merge",
            json!({
                "merge_method": "squash",
                "commit_title": "merge title",
                "commit_message": "merge body",
                "sha": "abc123def4567890abc123def4567890abc123de"
            }),
        ),
        get("https://api.github.com/user"),
        get("https://api.github.com/user/repos?per_page=11&type=member&page=2"),
        get(
            "https://api.github.com/search/repositories?q=org%3Anearai%20ironclaw&per_page=12&page=3&sort=updated&order=desc",
        ),
        get(
            "https://api.github.com/search/code?q=repo%3Anearai%2Fironclaw%20path%3Asrc%20Tool&per_page=12&page=3&sort=updated&order=desc",
        ),
        // github.search_issues alias → identical search/issues endpoint, distinct query.
        get(
            "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Aissue&per_page=12&page=3&sort=updated&order=desc",
        ),
        get(
            "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Apr&per_page=12&page=3&sort=reactions-heart&order=desc",
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
            "https://api.github.com/repos/nearai/ironclaw/actions/workflows/ci.yml/runs?per_page=15&event=pull_request&status=failure&head_sha=abc123def4567890abc123def4567890abc123de&page=2",
        ),
        get(
            "https://api.github.com/repos/nearai/ironclaw/actions/runs/12345/jobs?per_page=16&filter=all&page=2",
        ),
        get(
            "https://api.github.com/repos/nearai/ironclaw/actions/runs/12345/artifacts?per_page=17&name=coverage&direction=asc&page=3",
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/actions/runs/12345/rerun-failed-jobs",
            json!({"enable_debug_logging": true}),
        ),
        request(
            "POST",
            "https://api.github.com/repos/nearai/ironclaw/actions/jobs/67890/rerun",
            json!({"enable_debugger": true}),
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
    let mut matched = vec![false; actual.len()];
    for expected_request in expected {
        let Some((index, request)) = actual.iter().enumerate().find(|(index, request)| {
            !matched[*index]
                && request.method == expected_request.method
                && request.url == expected_request.url
                && request_body_matches(request, expected_request.body.as_ref())
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
        matched[index] = true;
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

fn request_body_matches(
    request: &NetworkHttpRequest,
    expected_body: Option<&serde_json::Value>,
) -> bool {
    match expected_body {
        Some(expected_body) => serde_json::from_slice::<serde_json::Value>(&request.body)
            .map(|actual_body| actual_body == *expected_body)
            .unwrap_or(false),
        None => request.body.is_empty(),
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

fn delete(url: &'static str) -> ExpectedGithubHttpRequest {
    ExpectedGithubHttpRequest {
        method: NetworkMethod::Delete,
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
        "PATCH" => NetworkMethod::Patch,
        "DELETE" => NetworkMethod::Delete,
        _ => unreachable!("unsupported test method"),
    };
    ExpectedGithubHttpRequest {
        method,
        url,
        body: Some(body),
    }
}

fn review_threads_query_body() -> serde_json::Value {
    json!({
        "query": r#"
query($owner: String!, $repo: String!, $number: Int!, $first: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: $first, after: $after) {
        nodes {
          id
          isResolved
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
  }
}
"#,
        "variables": {
            "owner": "nearai",
            "repo": "ironclaw",
            "number": 4280,
            "first": 12,
            "after": "cursor-1"
        }
    })
}

fn resolve_review_thread_body() -> serde_json::Value {
    json!({
        "query": r#"
mutation($threadId: ID!) {
  resolveReviewThread(input: { threadId: $threadId }) {
    thread {
      id
      isResolved
    }
  }
}
"#,
        "variables": {"threadId": "PRRT_kwDOExample"}
    })
}

fn unresolve_review_thread_body() -> serde_json::Value {
    json!({
        "query": r#"
mutation($threadId: ID!) {
  unresolveReviewThread(input: { threadId: $threadId }) {
    thread {
      id
      isResolved
    }
  }
}
"#,
        "variables": {"threadId": "PRRT_kwDOExample"}
    })
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
