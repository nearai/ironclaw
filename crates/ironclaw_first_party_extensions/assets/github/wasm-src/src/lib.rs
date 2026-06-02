//! First-party Reborn GitHub WASM tool.
//!
//! Ports the v1 GitHub WASM capability surface to the Reborn product capability
//! model. The host selects the operation via the invocation context capability id
//! and mediates GitHub credentials through HTTP egress; this component never
//! reads or constructs a GitHub token.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../../../wit/tool.wit",
});

mod api;
mod dispatch;
mod request;
mod schema;
mod types;
mod validation;
mod webhook;

struct GitHubTool;

impl exports::near::agent::tool::Guest for GitHubTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match dispatch::execute_inner(&req.params, req.context.as_deref()) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(error) => exports::near::agent::tool::Response {
                output: None,
                error: Some(guest_error_payload(&error)),
            },
        }
    }

    fn schema() -> String {
        schema::schema()
    }

    fn description() -> String {
        "First-party GitHub Reborn tool: repositories, issues, pull requests, reviews/comments, search, branches, code reads, file writes, releases, workflow dispatch/runs, forks, and webhook normalization. GitHub credentials are injected only by host HTTP egress."
            .to_string()
    }
}

fn guest_error_payload(code: &str) -> String {
    serde_json::json!({
        "code": code,
        "kind": guest_error_kind(code),
    })
    .to_string()
}

fn guest_error_kind(code: &str) -> &'static str {
    match code {
        "AuthRequired" => "auth_required",
        "missing_invocation_context"
        | "invalid_invocation_context"
        | "unsupported_github_capability"
        | "invalid_parameters"
        | "invalid_repository"
        | "invalid_query_empty"
        | "invalid_query_too_large"
        | "invalid_author"
        | "invalid_assignee"
        | "invalid_involves"
        | "invalid_state"
        | "invalid_type"
        | "invalid_sort"
        | "invalid_order"
        | "invalid_page"
        | "invalid_limit"
        | "invalid_labels"
        | "Invalid owner or repo name"
        | "Invalid repository name"
        | "Invalid org name"
        | "Invalid fork name"
        | "Invalid username"
        | "Invalid path: relative path segments not allowed"
        | "Invalid path: empty segment not allowed"
        | "Unsupported from_ref: use a branch or tag ref, not a raw commit SHA"
        | "Unsupported from_ref: only refs/heads/* and refs/tags/* are supported"
        | "Source ref response missing object.sha" => "input",
        "github_api_body_limit" => "output_too_large",
        "github_api_timeout" => "executor",
        "github_api_egress_denied" | "github_api_redirect_denied" => "network_denied",
        "github_api_error_status_401" => "auth_required",
        "github_api_error_status_403" | "github_api_error_status_429" => "client",
        _ => "operation_failed",
    }
}

export!(GitHubTool);

#[cfg(test)]
mod tests {
    use super::GitHubTool;
    use crate::dispatch::{action_from_context, execute_inner};
    use crate::exports::near::agent::tool::Guest;
    use crate::request::sanitize_host_error;
    use crate::types::{GitHubAction, GitHubWebhookRequest};
    use crate::validation::{normalize_ref_lookup, validate_repo_path};
    use crate::webhook::handle_webhook;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn operation_comes_from_host_context_not_param_shape() {
        assert_eq!(
            action_from_context(Some(r#"{"capability_id":"github.get_issue"}"#)).unwrap(),
            "get_issue"
        );
        assert_eq!(
            action_from_context(Some(r#"{"capability_id":"github.comment_issue"}"#)).unwrap(),
            "create_issue_comment"
        );
    }

    #[test]
    fn operation_rejects_missing_or_unknown_context() {
        assert_eq!(
            action_from_context(None).unwrap_err(),
            "missing_invocation_context"
        );
        assert_eq!(
            action_from_context(Some(r#"{"capability_id":"github.unknown"}"#)).unwrap_err(),
            "unsupported_github_capability"
        );
    }

    #[test]
    fn serde_rejects_unknown_fields_before_egress() {
        assert_eq!(
            execute_inner(
                r#"{"query":"repo:nearai/ironclaw","extra":"ignored?"}"#,
                Some(r#"{"capability_id":"github.search_issues"}"#),
            )
            .unwrap_err(),
            "invalid_parameters"
        );
    }

    #[test]
    fn serde_accepts_common_pr_number_aliases() {
        let action: GitHubAction = serde_json::from_value(json!({
            "action": "get_pull_request",
            "owner": "nearai",
            "repo": "ironclaw",
            "number": 4286
        }))
        .expect("number should be accepted as a pull request number alias");
        assert!(matches!(
            action,
            GitHubAction::GetPullRequest {
                pr_number: 4286,
                ..
            }
        ));

        let action: GitHubAction = serde_json::from_value(json!({
            "action": "get_pull_request_files",
            "owner": "nearai",
            "repo": "ironclaw",
            "pull_number": 4286
        }))
        .expect("pull_number should be accepted as a pull request number alias");
        assert!(matches!(
            action,
            GitHubAction::GetPullRequestFiles {
                pr_number: 4286,
                ..
            }
        ));
    }

    #[test]
    fn validates_static_schema_json() {
        let schema = GitHubTool::schema();
        let parsed: serde_json::Value =
            serde_json::from_str(&schema).expect("schema should be valid JSON");
        assert_eq!(parsed["type"], "object");
        assert!(parsed["oneOf"]
            .as_array()
            .is_some_and(|schemas| schemas.len() >= 30));
    }

    #[test]
    fn sanitizes_host_egress_errors_without_leaking_details() {
        assert_eq!(
            sanitize_host_error("missing token ghp_secret_value"),
            "AuthRequired"
        );
        assert_eq!(
            sanitize_host_error("deadline exceeded"),
            "github_api_timeout"
        );
        assert_eq!(
            sanitize_host_error("redirect blocked"),
            "github_api_redirect_denied"
        );
        assert_eq!(
            sanitize_host_error("response body too large"),
            "github_api_body_limit"
        );
        assert_eq!(
            sanitize_host_error("host not allowed"),
            "github_api_egress_denied"
        );
        assert_eq!(
            sanitize_host_error("connection reset with token ghp_secret_value"),
            "AuthRequired"
        );
    }

    #[test]
    fn normalize_ref_lookup_handles_branch_tag_and_unsupported_refs() {
        assert_eq!(
            normalize_ref_lookup("refs/heads/main").unwrap(),
            "heads/main"
        );
        assert_eq!(
            normalize_ref_lookup("refs/tags/v1.0.0").unwrap(),
            "tags/v1.0.0"
        );
        assert_eq!(normalize_ref_lookup("heads/dev").unwrap(), "heads/dev");
        assert_eq!(normalize_ref_lookup("tags/v2").unwrap(), "tags/v2");
        assert_eq!(
            normalize_ref_lookup("feature/reborn").unwrap(),
            "heads/feature/reborn"
        );
        assert_eq!(
            normalize_ref_lookup("refs/remotes/origin/main").unwrap_err(),
            "Unsupported from_ref: only refs/heads/* and refs/tags/* are supported"
        );
        assert_eq!(
            normalize_ref_lookup("0123456789abcdef0123456789abcdef01234567").unwrap_err(),
            "Unsupported from_ref: use a branch or tag ref, not a raw commit SHA"
        );
    }

    #[test]
    fn validate_repo_path_rejects_relative_segments() {
        assert_eq!(
            validate_repo_path("../src/main.rs").unwrap_err(),
            "Invalid path: relative path segments not allowed"
        );
        assert_eq!(
            validate_repo_path("src/./main.rs").unwrap_err(),
            "Invalid path: relative path segments not allowed"
        );
        assert_eq!(
            validate_repo_path("src//main.rs").unwrap_err(),
            "Invalid path: empty segment not allowed"
        );
        assert!(validate_repo_path("src/main.rs").is_ok());
    }

    #[test]
    fn handle_webhook_rejects_missing_event_or_body() {
        assert_eq!(
            handle_webhook(GitHubWebhookRequest {
                headers: HashMap::new(),
                body_json: Some(json!({}))
            })
            .unwrap_err(),
            "Missing X-GitHub-Event header"
        );

        let mut headers = HashMap::new();
        headers.insert("X-GitHub-Event".to_string(), "issues".to_string());
        assert_eq!(
            handle_webhook(GitHubWebhookRequest {
                headers,
                body_json: None
            })
            .unwrap_err(),
            "Missing webhook.body_json"
        );
    }

    #[test]
    fn handle_webhook_normalizes_pull_request_opened_event() {
        let mut headers = HashMap::new();
        headers.insert("X-GitHub-Event".to_string(), "pull_request".to_string());

        let response = handle_webhook(GitHubWebhookRequest {
            headers,
            body_json: Some(json!({
                "action": "opened",
                "repository": {
                    "full_name": "nearai/ironclaw",
                    "owner": {"login": "nearai"}
                },
                "pull_request": {
                    "number": 4280,
                    "state": "open",
                    "merged": false,
                    "draft": true,
                    "base": {"ref": "reborn-integration"},
                    "head": {"ref": "codex/reborn-github-capabilities"}
                },
                "sender": {"login": "reviewer"}
            })),
        })
        .expect("pull_request webhook should normalize");

        let parsed: serde_json::Value =
            serde_json::from_str(&response).expect("webhook response should be JSON");
        let payload = &parsed["emit_events"][0]["payload"];
        assert_eq!(parsed["emit_events"][0]["event_type"], json!("pr.opened"));
        assert_eq!(payload["pr_number"], json!(4280));
        assert_eq!(payload["pr_state"], json!("open"));
        assert_eq!(payload["pr_merged"], json!(false));
        assert_eq!(payload["pr_draft"], json!(true));
        assert_eq!(payload["base_branch"], json!("reborn-integration"));
        assert_eq!(
            payload["head_branch"],
            json!("codex/reborn-github-capabilities")
        );
    }

    #[test]
    fn handle_webhook_normalizes_check_run_event() {
        let mut headers = HashMap::new();
        headers.insert("X-GitHub-Event".to_string(), "check_run".to_string());

        let response = handle_webhook(GitHubWebhookRequest {
            headers,
            body_json: Some(json!({
                "action": "completed",
                "repository": {
                    "full_name": "nearai/ironclaw",
                    "owner": {"login": "nearai"}
                },
                "check_run": {
                    "status": "completed",
                    "conclusion": "success"
                }
            })),
        })
        .expect("check_run webhook should normalize");

        let parsed: serde_json::Value =
            serde_json::from_str(&response).expect("webhook response should be JSON");
        let payload = &parsed["emit_events"][0]["payload"];
        assert_eq!(
            parsed["emit_events"][0]["event_type"],
            json!("ci.check_run.completed")
        );
        assert_eq!(payload["ci_status"], json!("completed"));
        assert_eq!(payload["ci_conclusion"], json!("success"));
    }

    #[test]
    fn handle_webhook_normalizes_pr_comment_event() {
        let mut headers = HashMap::new();
        headers.insert("X-GitHub-Event".to_string(), "issue_comment".to_string());
        headers.insert("X-GitHub-Delivery".to_string(), "delivery-123".to_string());

        let response = handle_webhook(GitHubWebhookRequest {
            headers,
            body_json: Some(json!({
                "action": "created",
                "repository": {"full_name": "nearai/ironclaw"},
                "issue": {
                    "number": 4280,
                    "pull_request": {"url": "https://api.github.com/repos/nearai/ironclaw/pulls/4280"}
                },
                "comment": {"id": 99, "body": "looks good"}
            })),
        })
        .expect("webhook should normalize");

        let parsed: serde_json::Value =
            serde_json::from_str(&response).expect("webhook response should be JSON");
        assert_eq!(parsed["accepted"], json!(true));
        assert_eq!(parsed["emit_events"][0]["source"], json!("github"));
        assert_eq!(
            parsed["emit_events"][0]["event_type"],
            json!("pr.comment.created")
        );
        assert_eq!(
            parsed["emit_events"][0]["payload"]["delivery_id"],
            json!("delivery-123")
        );
        assert_eq!(
            parsed["emit_events"][0]["payload"]["repository_name"],
            json!("nearai/ironclaw")
        );
        assert_eq!(
            parsed["emit_events"][0]["payload"]["pr_number"],
            json!(4280)
        );
    }
}
