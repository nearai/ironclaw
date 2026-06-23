#![cfg(all(feature = "github-issue-workflow-beta", feature = "test-support"))]

mod github_issue_workflow_provider {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_github_issue_workflow::{
        CreateDraftPullRequestInput, CreateIssueCommentInput, GetAuthenticatedWorkflowActorInput,
        GetGithubIssueInput, GetPullRequestInput, GithubIssueRef, GithubIssueWorkflowError,
        GithubProviderAccountRef, ListPullRequestChecksInput, ListPullRequestReviewCommentsInput,
        ListPullRequestsInput, SearchGithubIssuesInput,
    };
    use ironclaw_reborn_composition::test_support::{
        GithubIssueWorkflowCapabilityDispatchErrorForTest,
        GithubIssueWorkflowCapabilityDispatchRequestForTest,
        GithubIssueWorkflowCapabilityDispatcherForTest,
        github_issue_workflow_provider_port_for_test,
    };
    use serde_json::{Value, json};

    #[tokio::test]
    async fn search_open_bug_issues_invokes_search_issues_with_expected_query() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!([
            {
                "number": 42,
                "html_url": "https://github.com/nearai/ironclaw/issues/42",
                "updated_at": "2026-06-22T10:30:00Z"
            }
        ]))));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher.clone(),
        );
        let query = "repo:nearai/ironclaw is:issue state:open label:bug".to_string();

        let hits = port
            .search_open_bug_issues(SearchGithubIssuesInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                query: query.clone(),
                limit: 5,
            })
            .await
            .expect("search succeeds");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].owner, "nearai");
        assert_eq!(hits[0].repo, "ironclaw");
        assert_eq!(hits[0].number, 42);

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, "github.search_issues");
        assert_eq!(
            requests[0].provider_account_ref,
            provider_account("input-account")
        );
        assert_eq!(requests[0].input, json!({ "query": query, "limit": 5 }));
    }

    #[tokio::test]
    async fn get_issue_normalizes_issue_author_login() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!({
            "number": 42,
            "html_url": "https://github.com/nearai/ironclaw/issues/42",
            "title": "Fix flaky workflow",
            "body": "The workflow occasionally stalls.",
            "state": "open",
            "user": { "login": "core-dev" },
            "labels": [{ "name": "bug" }],
            "updated_at": "2026-06-22T10:30:00Z",
            "repository": { "default_branch": "main" }
        }))));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher.clone(),
        );

        let issue = port
            .get_issue(GetGithubIssueInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                number: 42,
            })
            .await
            .expect("issue loads");

        assert_eq!(issue.author_login.as_deref(), Some("core-dev"));

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, "github.get_issue");
        assert_eq!(
            requests[0].provider_account_ref,
            provider_account("input-account")
        );
        assert_eq!(
            requests[0].input,
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "issue_number": 42,
            })
        );
    }

    #[tokio::test]
    async fn create_claim_comment_invokes_comment_issue_with_marker_body() {
        let marker_body =
            "<!-- ironclaw:github-bug-workflow:claim:run-123 -->\nClaimed.".to_string();
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!({
            "id": 44,
            "html_url": "https://github.com/nearai/ironclaw/issues/42#issuecomment-44"
        }))));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher.clone(),
        );

        let comment = port
            .create_issue_comment(CreateIssueCommentInput {
                issue: issue_ref(),
                body: marker_body.clone(),
            })
            .await
            .expect("comment succeeds");

        assert_eq!(
            comment.url,
            "https://github.com/nearai/ironclaw/issues/42#issuecomment-44"
        );
        assert_eq!(comment.node_id, None);

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, "github.comment_issue");
        assert_eq!(
            requests[0].input,
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "issue_number": 42,
                "body": marker_body,
            })
        );
    }

    #[tokio::test]
    async fn create_draft_pr_invokes_create_pull_request_with_draft_true() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!({
            "number": 4280,
            "html_url": "https://github.com/nearai/ironclaw/pull/4280",
            "head": {
                "ref": "codex/github-bug-workflow",
                "sha": "abc123"
            }
        }))));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher.clone(),
        );

        let pr = port
            .create_draft_pull_request(CreateDraftPullRequestInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                title: "Fix workflow adapter".to_string(),
                body: Some("Implements the provider adapter.".to_string()),
                head_branch: "codex/github-bug-workflow".to_string(),
                base_branch: "main".to_string(),
            })
            .await
            .expect("draft pr succeeds");

        assert_eq!(pr.number, 4280);
        assert_eq!(pr.head_branch, "codex/github-bug-workflow");
        assert_eq!(pr.head_sha.as_deref(), Some("abc123"));

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, "github.create_pull_request");
        assert_eq!(
            requests[0].provider_account_ref,
            provider_account("input-account")
        );
        assert_eq!(
            requests[0].input,
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "title": "Fix workflow adapter",
                "head": "codex/github-bug-workflow",
                "base": "main",
                "body": "Implements the provider adapter.",
                "draft": true,
            })
        );
    }

    #[tokio::test]
    async fn list_pull_requests_invokes_provider_list_with_account_ref() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!([
            {
                "number": 4280,
                "html_url": "https://github.com/nearai/ironclaw/pull/4280",
                "node_id": "PR_kwDONode",
                "title": "Fix bug",
                "body": "<!-- marker -->",
                "state": "open",
                "draft": true,
                "merged": false,
                "updated_at": "2026-06-22T11:00:00Z",
                "head": {
                    "ref": "codex/github-bug-workflow",
                    "sha": "abc123"
                }
            }
        ]))));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher.clone(),
        );

        let pulls = port
            .list_pull_requests(ListPullRequestsInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                state: "open".to_string(),
                limit: 25,
            })
            .await
            .expect("list pull requests succeeds");

        assert_eq!(pulls.len(), 1);
        assert_eq!(pulls[0].pull_request.number, 4280);
        assert_eq!(pulls[0].pull_request.head_sha.as_deref(), Some("abc123"));
        assert!(pulls[0].draft);

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, "github.list_pull_requests");
        assert_eq!(
            requests[0].provider_account_ref,
            provider_account("input-account")
        );
        assert_eq!(
            requests[0].input,
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "state": "open",
                "page": 1,
                "limit": 25,
            })
        );
    }

    #[tokio::test]
    async fn get_pull_request_invokes_provider_get_and_normalizes_snapshot() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!({
            "number": 4280,
            "html_url": "https://github.com/nearai/ironclaw/pull/4280",
            "title": "Fix bug",
            "body": "PR body",
            "state": "closed",
            "draft": false,
            "merged": true,
            "updated_at": "2026-06-22T12:00:00Z",
            "head": {
                "ref": "codex/github-bug-workflow",
                "sha": "def456"
            }
        }))));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher.clone(),
        );

        let pull = port
            .get_pull_request(GetPullRequestInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                number: 4280,
            })
            .await
            .expect("get pull request succeeds");

        assert_eq!(pull.pull_request.number, 4280);
        assert_eq!(pull.pull_request.head_sha.as_deref(), Some("def456"));
        assert!(pull.merged);

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, "github.get_pull_request");
        assert_eq!(
            requests[0].input,
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
            })
        );
    }

    #[tokio::test]
    async fn list_pull_request_checks_reads_combined_status_for_head_sha() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!({
            "state": "failure",
            "sha": "def456",
            "statuses": [
                {
                    "id": 7001,
                    "context": "clippy",
                    "state": "failure",
                    "target_url": "https://github.com/nearai/ironclaw/actions/runs/7001",
                    "updated_at": "2026-06-22T12:15:00Z"
                }
            ]
        }))));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher.clone(),
        );

        let checks = port
            .list_pull_request_checks(ListPullRequestChecksInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                pull_request_number: 4280,
                head_sha: Some("def456".to_string()),
                limit: 10,
            })
            .await
            .expect("list checks succeeds");

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].suite_or_run_id, "7001");
        assert_eq!(checks[0].head_sha, "def456");
        assert!(checks[0].conclusion.is_failure());

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, "github.get_combined_status");
        assert_eq!(
            requests[0].input,
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "ref": "def456",
            })
        );
    }

    #[tokio::test]
    async fn list_pull_request_review_comments_invokes_provider_comments() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!([
            {
                "id": 9001,
                "node_id": "PRRC_kwDONode",
                "html_url": "https://github.com/nearai/ironclaw/pull/4280#discussion_r9001",
                "body": "Please add a regression test.",
                "user": { "login": "reviewer" },
                "created_at": "2026-06-22T12:20:00Z",
                "updated_at": "2026-06-22T12:21:00Z"
            }
        ]))));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher.clone(),
        );

        let comments = port
            .list_pull_request_review_comments(ListPullRequestReviewCommentsInput {
                provider_account_ref: provider_account("input-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                pull_request_number: 4280,
                since: None,
                limit: 20,
            })
            .await
            .expect("list review comments succeeds");

        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author_login, "reviewer");
        assert_eq!(
            comments[0].comment.node_id.as_deref(),
            Some("PRRC_kwDONode")
        );

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].capability_id,
            "github.list_pull_request_comments"
        );
        assert_eq!(
            requests[0].input,
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "pr_number": 4280,
                "page": 1,
                "limit": 20,
            })
        );
    }

    #[tokio::test]
    async fn provider_adapter_redacts_backend_error() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Err(
            GithubIssueWorkflowCapabilityDispatchErrorForTest::Backend {
                kind: "backend".to_string(),
                message: "raw provider body: {\"token\":\"ghp_secret\"}".to_string(),
            },
        )));
        let port = github_issue_workflow_provider_port_for_test(
            provider_account("configured-account"),
            dispatcher,
        );

        let error = port
            .search_open_bug_issues(SearchGithubIssuesInput {
                provider_account_ref: provider_account("configured-account"),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                query: "repo:nearai/ironclaw is:issue".to_string(),
                limit: 1,
            })
            .await
            .expect_err("search should fail");

        assert!(matches!(
            error,
            GithubIssueWorkflowError::ProviderRead { .. }
        ));
        let rendered = error.to_string();
        assert!(rendered.contains("GitHub provider read failed"));
        assert!(!rendered.contains("ghp_secret"));
        assert!(!rendered.contains("raw provider body"));
    }

    #[tokio::test]
    async fn provider_adapter_uses_configured_account_ref() {
        let dispatcher = Arc::new(RecordingDispatcher::with_response(Ok(json!({
            "login": "serrrfirat",
            "node_id": "MDQ6VXNlcjE="
        }))));
        let configured = provider_account("configured-account");
        let port =
            github_issue_workflow_provider_port_for_test(configured.clone(), dispatcher.clone());

        let actor = port
            .get_authenticated_workflow_actor(GetAuthenticatedWorkflowActorInput {
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
            })
            .await
            .expect("actor lookup succeeds");

        assert_eq!(actor.login, "serrrfirat");

        let requests = dispatcher.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].capability_id, "github.get_authenticated_user");
        assert_eq!(requests[0].provider_account_ref, configured);
        assert_eq!(requests[0].input, json!({}));
    }

    #[derive(Debug)]
    struct RecordingDispatcher {
        responses: Mutex<Vec<Result<Value, GithubIssueWorkflowCapabilityDispatchErrorForTest>>>,
        requests: Mutex<Vec<GithubIssueWorkflowCapabilityDispatchRequestForTest>>,
    }

    impl RecordingDispatcher {
        fn with_response(
            response: Result<Value, GithubIssueWorkflowCapabilityDispatchErrorForTest>,
        ) -> Self {
            Self {
                responses: Mutex::new(vec![response]),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<GithubIssueWorkflowCapabilityDispatchRequestForTest> {
            self.requests.lock().expect("requests mutex").clone()
        }
    }

    #[async_trait]
    impl GithubIssueWorkflowCapabilityDispatcherForTest for RecordingDispatcher {
        async fn dispatch(
            &self,
            request: GithubIssueWorkflowCapabilityDispatchRequestForTest,
        ) -> Result<Value, GithubIssueWorkflowCapabilityDispatchErrorForTest> {
            self.requests.lock().expect("requests mutex").push(request);
            self.responses.lock().expect("responses mutex").remove(0)
        }
    }

    fn provider_account(account_id: &str) -> GithubProviderAccountRef {
        GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: account_id.to_string(),
        }
    }

    fn issue_ref() -> GithubIssueRef {
        GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 42,
            node_id: Some("I_kwDONode".to_string()),
            url: "https://github.com/nearai/ironclaw/issues/42".to_string(),
            default_branch: "main".to_string(),
        }
    }
}
