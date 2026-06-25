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
        | "invalid_direction"
        | "invalid_milestone"
        | "invalid_order"
        | "invalid_page"
        | "invalid_limit"
        | "invalid_labels"
        | "invalid_label"
        | "invalid_comments"
        | "invalid_thread_id"
        | "Invalid owner or repo name"
        | "Invalid repository name"
        | "Invalid org name"
        | "Invalid fork name"
        | "Invalid path: relative path segments not allowed"
        | "Invalid path: empty segment not allowed"
        | "Unsupported from_ref: use a branch or tag ref, not a raw commit SHA"
        | "Unsupported from_ref: only refs/heads/* and refs/tags/* are supported"
        | "Source ref response missing object.sha" => "input",
        code if is_string_validation_input_error(code) => "input",
        "github_api_body_limit" => "output_too_large",
        "github_api_timeout" => "executor",
        "github_api_egress_denied" | "github_api_redirect_denied" => "network_denied",
        "github_api_error_status_401" => "auth_required",
        "github_api_error_status_403" | "github_api_error_status_429" => "client",
        _ => "operation_failed",
    }
}

fn is_string_validation_input_error(code: &str) -> bool {
    code.starts_with("Invalid labels:")
        || code.starts_with("Invalid assignees:")
        || code.starts_with("invalid_comments:")
}

export!(GitHubTool);

#[cfg(test)]
mod tests {
    use super::guest_error_kind;
    use super::GitHubTool;
    use crate::dispatch::{action_from_context, execute_inner};
    use crate::exports::near::agent::tool::Guest;
    use crate::request::{sanitize_host_error, test_support};
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
        assert_eq!(
            action_from_context(Some(r#"{"capability_id":"github.get_authenticated_user"}"#))
                .unwrap(),
            "get_authenticated_user"
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
    fn schema_exposes_bug1_parameters() {
        let schema = GitHubTool::schema();
        let parsed: serde_json::Value =
            serde_json::from_str(&schema).expect("schema should be valid JSON");
        let schemas = parsed["oneOf"].as_array().expect("schema oneOf");

        let find_schema = |title: &str| {
            schemas
                .iter()
                .find(|schema| schema["title"] == title)
                .unwrap_or_else(|| panic!("missing schema {title}"))
        };

        assert!(
            find_schema("GitHub get_pull_request_files input")["properties"]["page"].is_object()
        );
        assert!(
            find_schema("GitHub get_pull_request_files input")["properties"]["page"]["maximum"]
                .is_null()
        );
        assert_eq!(
            find_schema("GitHub list_issues input")["properties"]["page"]["maximum"],
            10
        );
        assert!(find_schema("GitHub list_issues input")["properties"]["labels"].is_object());
        assert!(find_schema("GitHub merge_pull_request input")["properties"]["sha"].is_object());
        assert!(find_schema("GitHub create_issue input")["properties"]["assignees"].is_object());
        assert_eq!(
            find_schema("GitHub add_issue_assignees input")["properties"]["assignees"]["maxItems"],
            10
        );
        assert!(find_schema("GitHub create_issue input")["properties"]["milestone"].is_object());
        assert!(find_schema("GitHub update_issue input")["properties"]["state"].is_object());
        assert!(find_schema("GitHub update_pull_request input")["properties"]["state"].is_object());
        assert!(
            find_schema("GitHub list_pull_request_review_threads input")["properties"]["first"]
                .is_object()
        );
        assert!(
            find_schema("GitHub get_workflow_run_jobs input")["properties"]["run_id"].is_object()
        );
        assert_eq!(
            find_schema("GitHub list_repos input")["properties"]["type"]["enum"],
            json!(["all", "owner", "public", "private", "member"])
        );
        assert!(
            find_schema("GitHub list_repos input")["properties"]["username"].is_null(),
            "list_repos schema should not expose username"
        );
        assert!(find_schema("GitHub list_pull_requests input")["properties"]["head"].is_object());
        assert!(
            find_schema("GitHub get_file_content input")["properties"]["path"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("base64"))
        );
        assert!(
            find_schema("GitHub search_issues_pull_requests input")["properties"]["sort"]["enum"]
                .as_array()
                .is_some_and(
                    |values| values.iter().any(|value| value == "reactions-heart")
                        && !values.iter().any(|value| value == "author-date")
                        && !values.iter().any(|value| value == "committer-date")
                )
        );
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
    fn guest_error_kind_classifies_string_validation_errors_as_input() {
        for code in [
            "Invalid labels: values cannot be empty",
            "Invalid assignees: values cannot be empty",
            "Invalid assignees: at most 10 values are allowed",
            "Invalid assignees: at most 100 values are allowed",
            "invalid_comments: comments serialization failed",
        ] {
            assert_eq!(guest_error_kind(code), "input", "{code}");
        }
    }

    #[test]
    fn list_issues_uses_native_repo_endpoint_with_filters() {
        test_support::set_responses([
            Ok(json!([
                {
                    "number": 4808,
                    "title": "previous issue"
                },
                {
                    "number": 4807,
                    "title": "previous pull request",
                    "pull_request": {
                        "url": "https://api.github.com/repos/nearai/ironclaw/pulls/4807"
                    }
                }
            ])
            .to_string()),
            Ok(json!([
                {
                    "number": 4806,
                    "title": "skipped issue"
                },
                {
                    "number": 4805,
                    "title": "pull request",
                    "pull_request": {
                        "url": "https://api.github.com/repos/nearai/ironclaw/pulls/4805"
                    }
                }
            ])
            .to_string()),
            Ok(json!([
                {
                    "number": 4804,
                    "title": "issue one"
                },
                {
                    "number": 4803,
                    "title": "issue two"
                }
            ])
            .to_string()),
        ]);

        let output = execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","state":"open","labels":["bug","api"],"assignee":"henry","milestone":"12","page":2,"limit":2}"#,
            Some(r#"{"capability_id":"github.list_issues"}"#),
        )
        .expect("github.list_issues should call native issues endpoint");

        let requests = test_support::requests();
        assert_eq!(requests.len(), 3);
        assert!(requests
            .iter()
            .all(|request| request.method == "GET" && request.body.is_none()));
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/issues?state=open&per_page=2&page=1&labels=bug%2Capi&assignee=henry&milestone=12"
        );
        assert_eq!(
            requests[1].path,
            "/repos/nearai/ironclaw/issues?state=open&per_page=2&page=2&labels=bug%2Capi&assignee=henry&milestone=12"
        );
        assert_eq!(
            requests[2].path,
            "/repos/nearai/ironclaw/issues?state=open&per_page=2&page=3&labels=bug%2Capi&assignee=henry&milestone=12"
        );

        let parsed: serde_json::Value =
            serde_json::from_str(&output).expect("list_issues output should be JSON");
        assert_eq!(
            parsed,
            json!([
                {
                    "number": 4804,
                    "title": "issue one"
                },
                {
                    "number": 4803,
                    "title": "issue two"
                }
            ])
        );
    }

    #[test]
    fn list_issues_all_state_uses_native_state_param() {
        test_support::set_response(Ok(json!([]).to_string()));

        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","state":"all","limit":1}"#,
            Some(r#"{"capability_id":"github.list_issues"}"#),
        )
        .expect("github.list_issues should accept state=all");

        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/issues?state=all&per_page=1&page=1"
        );
    }

    #[test]
    fn list_issues_bounds_logical_page_and_raw_page_scan() {
        test_support::set_response(Ok(json!([]).to_string()));

        assert_eq!(
            execute_inner(
                r#"{"owner":"nearai","repo":"ironclaw","page":11}"#,
                Some(r#"{"capability_id":"github.list_issues"}"#),
            )
            .unwrap_err(),
            "invalid_page"
        );
        assert!(
            test_support::requests().is_empty(),
            "oversized logical issue page should fail before egress"
        );

        let pr_only_page = || {
            Ok(json!([{
                "number": 4805,
                "title": "pull request",
                "pull_request": {
                    "url": "https://api.github.com/repos/nearai/ironclaw/pulls/4805"
                }
            }])
            .to_string())
        };
        test_support::set_responses([
            pr_only_page(),
            pr_only_page(),
            pr_only_page(),
            pr_only_page(),
            pr_only_page(),
            pr_only_page(),
            pr_only_page(),
            pr_only_page(),
            pr_only_page(),
            pr_only_page(),
        ]);

        assert_eq!(
            execute_inner(
                r#"{"owner":"nearai","repo":"ironclaw","limit":1}"#,
                Some(r#"{"capability_id":"github.list_issues"}"#),
            )
            .unwrap_err(),
            "github_issue_page_scan_limit"
        );
        assert_eq!(
            test_support::requests().len(),
            10,
            "scan limit should bound upstream page requests"
        );
    }

    #[test]
    fn list_issues_rejects_milestone_titles_before_egress() {
        test_support::set_response(Ok(json!([]).to_string()));

        assert_eq!(
            execute_inner(
                r#"{"owner":"nearai","repo":"ironclaw","milestone":"v1"}"#,
                Some(r#"{"capability_id":"github.list_issues"}"#),
            )
            .unwrap_err(),
            "invalid_milestone"
        );
        assert!(
            test_support::requests().is_empty(),
            "invalid milestone title should be rejected before egress"
        );
    }

    #[test]
    fn review_threads_reject_invalid_first_and_blank_thread_id_before_egress() {
        for (capability, input, expected_error) in [
            (
                "github.list_pull_request_review_threads",
                r#"{"owner":"nearai","repo":"ironclaw","pr_number":12,"first":0}"#,
                "invalid_limit",
            ),
            (
                "github.list_pull_request_review_threads",
                r#"{"owner":"nearai","repo":"ironclaw","pr_number":12,"first":101}"#,
                "invalid_limit",
            ),
            (
                "github.resolve_review_thread",
                r#"{"thread_id":""}"#,
                "invalid_thread_id",
            ),
        ] {
            test_support::set_response(Ok(json!({"data": {}}).to_string()));

            assert_eq!(
                execute_inner(
                    input,
                    Some(&format!(r#"{{"capability_id":"{capability}"}}"#))
                )
                .unwrap_err(),
                expected_error
            );
            assert!(
                test_support::requests().is_empty(),
                "validation error {expected_error} should happen before egress"
            );
        }
    }

    #[test]
    fn get_pull_request_files_uses_page_and_limit() {
        test_support::set_response(Ok(json!([]).to_string()));

        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","pr_number":42,"page":3,"limit":50}"#,
            Some(r#"{"capability_id":"github.get_pull_request_files"}"#),
        )
        .expect("github.get_pull_request_files should accept pagination");

        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/pulls/42/files?per_page=50&page=3"
        );
    }

    #[test]
    fn list_pull_requests_uses_filters_sort_and_direction() {
        test_support::set_response(Ok(json!([]).to_string()));

        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","state":"all","head":"henry:fix","base":"main","sort":"updated","direction":"asc","page":101,"limit":12}"#,
            Some(r#"{"capability_id":"github.list_pull_requests"}"#),
        )
        .expect("github.list_pull_requests should accept filters");

        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/pulls?state=all&per_page=12&head=henry%3Afix&base=main&sort=updated&direction=asc&page=101"
        );
    }

    #[test]
    fn list_pull_requests_rejects_invalid_sort_direction_and_pagination() {
        for (input, expected_error) in [
            (
                r#"{"owner":"nearai","repo":"ironclaw","state":"invalid"}"#,
                "invalid_state",
            ),
            (
                r#"{"owner":"nearai","repo":"ironclaw","sort":"comments"}"#,
                "invalid_sort",
            ),
            (
                r#"{"owner":"nearai","repo":"ironclaw","direction":"sideways"}"#,
                "invalid_direction",
            ),
            (
                r#"{"owner":"nearai","repo":"ironclaw","page":0}"#,
                "invalid_page",
            ),
            (
                r#"{"owner":"nearai","repo":"ironclaw","limit":0}"#,
                "invalid_limit",
            ),
        ] {
            test_support::set_response(Ok(json!([]).to_string()));

            assert_eq!(
                execute_inner(
                    input,
                    Some(r#"{"capability_id":"github.list_pull_requests"}"#)
                )
                .unwrap_err(),
                expected_error
            );
            assert!(
                test_support::requests().is_empty(),
                "validation error {expected_error} should happen before egress"
            );
        }
    }

    #[test]
    fn merge_pull_request_sends_optional_sha() {
        test_support::set_response(Ok(json!({"merged": true}).to_string()));

        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","pr_number":42,"merge_method":"squash","sha":"abc123"}"#,
            Some(r#"{"capability_id":"github.merge_pull_request"}"#),
        )
        .expect("github.merge_pull_request should accept sha");

        let requests = test_support::requests();
        assert_eq!(requests[0].method, "PUT");
        assert_eq!(requests[0].path, "/repos/nearai/ironclaw/pulls/42/merge");
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body["merge_method"], "squash");
        assert_eq!(body["sha"], "abc123");
    }

    #[test]
    fn create_issue_sends_assignees_and_milestone() {
        test_support::set_response(Ok(json!({"number": 12}).to_string()));

        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","title":"bug","milestone":7,"assignees":["henry"],"labels":["api"]}"#,
            Some(r#"{"capability_id":"github.create_issue"}"#),
        )
        .expect("github.create_issue should accept assignees");

        let requests = test_support::requests();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/repos/nearai/ironclaw/issues");
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body["title"], "bug");
        assert_eq!(body["milestone"], 7);
        assert_eq!(body["assignees"], json!(["henry"]));
        assert_eq!(body["labels"], json!(["api"]));
    }

    #[test]
    fn list_issues_omits_empty_labels_filter() {
        test_support::set_response(Ok(json!([]).to_string()));

        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","labels":[],"limit":2}"#,
            Some(r#"{"capability_id":"github.list_issues"}"#),
        )
        .expect("github.list_issues should accept empty label filters");

        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/issues?state=open&per_page=2&page=1"
        );
    }

    #[test]
    fn issue_mutation_tools_use_native_endpoints() {
        for (capability, input, expected_method, expected_path, expected_body) in [
            (
                "github.update_issue",
                r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"state":"closed","labels":["bug"],"assignees":["henry"],"milestone":7}"#,
                "PATCH",
                "/repos/nearai/ironclaw/issues/42",
                json!({"state":"closed","labels":["bug"],"assignees":["henry"],"milestone":7}),
            ),
            (
                "github.add_issue_labels",
                r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"labels":["api","reborn"]}"#,
                "POST",
                "/repos/nearai/ironclaw/issues/42/labels",
                json!({"labels":["api","reborn"]}),
            ),
            (
                "github.add_issue_assignees",
                r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"assignees":["henry"]}"#,
                "POST",
                "/repos/nearai/ironclaw/issues/42/assignees",
                json!({"assignees":["henry"]}),
            ),
            (
                "github.remove_issue_assignees",
                r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"assignees":["henry"]}"#,
                "DELETE",
                "/repos/nearai/ironclaw/issues/42/assignees",
                json!({"assignees":["henry"]}),
            ),
        ] {
            test_support::set_response(Ok(json!({}).to_string()));

            execute_inner(
                input,
                Some(&format!(r#"{{"capability_id":"{capability}"}}"#)),
            )
            .expect("issue mutation should dispatch");

            let requests = test_support::requests();
            assert_eq!(requests[0].method, expected_method);
            assert_eq!(requests[0].path, expected_path);
            let body: serde_json::Value =
                serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
            assert_eq!(body, expected_body);
        }

        test_support::set_response(Ok(json!({}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"name":"needs review"}"#,
            Some(r#"{"capability_id":"github.remove_issue_label"}"#),
        )
        .expect("remove label should dispatch");
        let requests = test_support::requests();
        assert_eq!(requests[0].method, "DELETE");
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/issues/42/labels/needs%20review"
        );
        assert!(requests[0].body.is_none());

        test_support::set_response(Ok(json!({}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"milestone":null}"#,
            Some(r#"{"capability_id":"github.update_issue"}"#),
        )
        .expect("update issue should allow clearing milestone");
        let requests = test_support::requests();
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({"milestone": null}));
    }

    #[test]
    fn add_issue_assignees_rejects_more_than_ten_assignees_before_egress() {
        let assignees = (0..11)
            .map(|index| format!("user-{index}"))
            .collect::<Vec<_>>();
        let input = json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "issue_number": 42,
            "assignees": assignees,
        })
        .to_string();
        test_support::set_response(Ok(json!({}).to_string()));

        assert_eq!(
            execute_inner(
                &input,
                Some(r#"{"capability_id":"github.add_issue_assignees"}"#)
            )
            .unwrap_err(),
            "Invalid assignees: at most 10 values are allowed"
        );
        assert!(
            test_support::requests().is_empty(),
            "too many assignees should fail before egress"
        );
    }

    #[test]
    fn update_issue_allows_clearing_body_and_rejects_oversized_body_before_egress() {
        test_support::set_response(Ok(json!({}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"body":null}"#,
            Some(r#"{"capability_id":"github.update_issue"}"#),
        )
        .expect("update issue should allow clearing body");
        let requests = test_support::requests();
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({"body": null}));

        test_support::set_response(Ok(json!({}).to_string()));
        let oversized_body = "a".repeat(65537);
        let input = format!(
            r#"{{"owner":"nearai","repo":"ironclaw","issue_number":42,"body":"{}"}}"#,
            oversized_body
        );
        let error = execute_inner(&input, Some(r#"{"capability_id":"github.update_issue"}"#))
            .expect_err("oversized body should be rejected");
        assert_eq!(
            error,
            "Input 'body' exceeds maximum length of 65536 characters"
        );
        assert!(
            test_support::requests().is_empty(),
            "oversized body should fail before egress"
        );
    }

    #[test]
    fn update_issue_rejects_missing_mutation_fields_before_egress() {
        let error = execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","issue_number":42}"#,
            Some(r#"{"capability_id":"github.update_issue"}"#),
        )
        .expect_err("update issue should require at least one mutable field");
        assert_eq!(error, "invalid_parameters");
        assert!(
            test_support::requests().is_empty(),
            "missing mutation fields should fail before egress"
        );
    }

    #[test]
    fn update_issue_preserves_empty_labels_and_assignees_arrays() {
        test_support::set_response(Ok(json!({}).to_string()));

        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"labels":[],"assignees":[]}"#,
            Some(r#"{"capability_id":"github.update_issue"}"#),
        )
        .expect("update issue should accept empty labels and assignees arrays");

        let requests = test_support::requests();
        assert_eq!(requests[0].method, "PATCH");
        assert_eq!(requests[0].path, "/repos/nearai/ironclaw/issues/42");
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({"labels": [], "assignees": []}));
    }

    #[test]
    fn pull_request_workflow_tools_use_native_endpoints() {
        test_support::set_response(Ok(json!({"number": 12}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","head":"feature","base":"main","issue":42,"head_repo":"ironclaw-fork","maintainer_can_modify":true}"#,
            Some(r#"{"capability_id":"github.create_pull_request"}"#),
        )
        .expect("create PR should accept issue and fork fields");
        let requests = test_support::requests();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/repos/nearai/ironclaw/pulls");
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body["issue"], 42);
        assert_eq!(body["head_repo"], "ironclaw-fork");
        assert_eq!(body["maintainer_can_modify"], true);
        assert!(body.get("title").is_none());

        test_support::set_response(Ok(json!({"number": 12}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","pr_number":12,"state":"closed","base":"release","maintainer_can_modify":false}"#,
            Some(r#"{"capability_id":"github.update_pull_request"}"#),
        )
        .expect("update PR should dispatch");
        let requests = test_support::requests();
        assert_eq!(requests[0].method, "PATCH");
        assert_eq!(requests[0].path, "/repos/nearai/ironclaw/pulls/12");
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"state":"closed","base":"release","maintainer_can_modify":false})
        );

        test_support::set_response(Ok(json!({"id": 1}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","pr_number":12,"body":"review","event":"COMMENT","commit_id":"abc123","comments":[{"path":"src/lib.rs","body":"inline","line":10,"side":"RIGHT"}]}"#,
            Some(r#"{"capability_id":"github.create_pr_review"}"#),
        )
        .expect("create PR review should accept inline comments");
        let requests = test_support::requests();
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(requests[0].path, "/repos/nearai/ironclaw/pulls/12/reviews");
        assert_eq!(body["commit_id"], "abc123");
        assert_eq!(body["comments"][0]["path"], "src/lib.rs");
        assert_eq!(body["comments"][0]["line"], 10);

        for invalid_comments in [
            r#"[{"path":"src/lib.rs","body":"inline"}]"#,
            r#"[{"path":"src/lib.rs","body":"inline","line":10}]"#,
            r#"[{"path":"src/lib.rs","body":"inline","position":1,"line":10,"side":"RIGHT"}]"#,
            r#"[{"path":"src/lib.rs","body":"inline","line":10,"side":"RIGHT","start_line":9}]"#,
        ] {
            test_support::set_response(Ok(json!({"id": 1}).to_string()));
            let input = format!(
                r#"{{"owner":"nearai","repo":"ironclaw","pr_number":12,"body":"review","event":"COMMENT","comments":{invalid_comments}}}"#
            );
            assert_eq!(
                execute_inner(
                    &input,
                    Some(r#"{"capability_id":"github.create_pr_review"}"#)
                )
                .unwrap_err(),
                "invalid_comments"
            );
            assert!(
                test_support::requests().is_empty(),
                "invalid inline review comments should fail before egress"
            );
        }

        test_support::set_response(Ok(json!([]).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","pr_number":12,"sort":"updated","direction":"desc","since":"2026-06-23T00:00:00Z","limit":2,"page":3}"#,
            Some(r#"{"capability_id":"github.list_pull_request_comments"}"#),
        )
        .expect("list PR comments should accept filters");
        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/pulls/12/comments?per_page=2&sort=updated&direction=desc&since=2026-06-23T00%3A00%3A00Z&page=3"
        );
    }

    #[test]
    fn review_threads_use_graphql_endpoint() {
        test_support::set_response(Ok(json!({"data": {}}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","pr_number":12,"first":10,"after":"cursor"}"#,
            Some(r#"{"capability_id":"github.list_pull_request_review_threads"}"#),
        )
        .expect("list review threads should dispatch");
        let requests = test_support::requests();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/graphql");
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        let query = body["query"].as_str().unwrap();
        let compact_query: String = query.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(query.contains("reviewThreads"));
        assert!(
            !compact_query.contains("comments(") && !compact_query.contains("comments{"),
            "thread listing should not hydrate per-thread comments"
        );
        assert_eq!(body["variables"]["owner"], "nearai");
        assert_eq!(body["variables"]["first"], 10);

        test_support::set_response(Ok(json!({"data": {}}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","pr_number":12}"#,
            Some(r#"{"capability_id":"github.list_pull_request_review_threads"}"#),
        )
        .expect("list review threads should apply the default page size");
        let requests = test_support::requests();
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body["variables"]["first"], 30);
        assert_eq!(body["variables"]["after"], serde_json::Value::Null);

        test_support::set_response(Ok(json!({"data": {}}).to_string()));
        execute_inner(
            r#"{"thread_id":"PRRT_kwDOExample"}"#,
            Some(r#"{"capability_id":"github.resolve_review_thread"}"#),
        )
        .expect("resolve review thread should dispatch");
        let requests = test_support::requests();
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert!(body["query"]
            .as_str()
            .unwrap()
            .contains("resolveReviewThread"));
        assert_eq!(body["variables"]["threadId"], "PRRT_kwDOExample");
    }

    #[test]
    fn update_pull_request_rejects_oversized_title_body_and_base_before_egress() {
        for (field, input, expected_error) in [
            (
                "title",
                format!(
                    r#"{{"owner":"nearai","repo":"ironclaw","pr_number":12,"state":"closed","title":"{}"}}"#,
                    "a".repeat(65537)
                ),
                "Input 'title' exceeds maximum length of 65536 characters",
            ),
            (
                "body",
                format!(
                    r#"{{"owner":"nearai","repo":"ironclaw","pr_number":12,"state":"closed","body":"{}"}}"#,
                    "a".repeat(65537)
                ),
                "Input 'body' exceeds maximum length of 65536 characters",
            ),
            (
                "base",
                format!(
                    r#"{{"owner":"nearai","repo":"ironclaw","pr_number":12,"state":"closed","base":"{}"}}"#,
                    "a".repeat(65537)
                ),
                "Input 'base' exceeds maximum length of 65536 characters",
            ),
        ] {
            test_support::set_response(Ok(json!({"number": 12}).to_string()));

            assert_eq!(
                execute_inner(
                    &input,
                    Some(r#"{"capability_id":"github.update_pull_request"}"#)
                )
                .unwrap_err(),
                expected_error,
                "{field} should reject oversized input"
            );
            assert!(
                test_support::requests().is_empty(),
                "oversized {field} should fail before egress"
            );
        }
    }

    #[test]
    fn update_pull_request_rejects_missing_mutation_fields_before_egress() {
        let error = execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","pr_number":12}"#,
            Some(r#"{"capability_id":"github.update_pull_request"}"#),
        )
        .expect_err("update pull request should require at least one mutable field");
        assert_eq!(error, "invalid_parameters");
        assert!(
            test_support::requests().is_empty(),
            "missing mutation fields should fail before egress"
        );
    }

    #[test]
    fn review_threads_reject_oversized_after_and_thread_id_before_egress() {
        for (capability, input, expected_error) in [
            (
                "github.list_pull_request_review_threads",
                format!(
                    r#"{{"owner":"nearai","repo":"ironclaw","pr_number":12,"first":10,"after":"{}"}}"#,
                    "a".repeat(65537)
                ),
                "Input 'after' exceeds maximum length of 65536 characters",
            ),
            (
                "github.resolve_review_thread",
                format!(r#"{{"thread_id":"{}"}}"#, "a".repeat(65537)),
                "Input 'thread_id' exceeds maximum length of 65536 characters",
            ),
            (
                "github.unresolve_review_thread",
                format!(r#"{{"thread_id":"{}"}}"#, "a".repeat(65537)),
                "Input 'thread_id' exceeds maximum length of 65536 characters",
            ),
        ] {
            test_support::set_response(Ok(json!({"data": {}}).to_string()));

            assert_eq!(
                execute_inner(
                    input.as_str(),
                    Some(&format!(r#"{{"capability_id":"{capability}"}}"#))
                )
                .unwrap_err(),
                expected_error
            );
            assert!(
                test_support::requests().is_empty(),
                "oversized validation for {capability} should happen before egress"
            );
        }
    }

    #[test]
    fn workflow_action_tools_use_native_endpoints() {
        test_support::set_response(Ok(json!({"workflow_runs": []}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","workflow_id":"ci.yml","head_sha":"abc123","event":"pull_request","status":"failure","exclude_pull_requests":false,"limit":5,"page":2}"#,
            Some(r#"{"capability_id":"github.get_workflow_runs"}"#),
        )
        .expect("workflow runs should accept filters");
        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/actions/workflows/ci.yml/runs?per_page=5&event=pull_request&status=failure&exclude_pull_requests=false&head_sha=abc123&page=2"
        );

        test_support::set_response(Ok(json!({"jobs": []}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","run_id":123,"filter":"all","limit":6,"page":2}"#,
            Some(r#"{"capability_id":"github.get_workflow_run_jobs"}"#),
        )
        .expect("workflow jobs should dispatch");
        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/actions/runs/123/jobs?per_page=6&filter=all&page=2"
        );

        test_support::set_response(Ok(json!({"artifacts": []}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","run_id":123,"name":"coverage","direction":"asc","limit":7,"page":3}"#,
            Some(r#"{"capability_id":"github.get_workflow_run_artifacts"}"#),
        )
        .expect("workflow artifacts should dispatch");
        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/actions/runs/123/artifacts?per_page=7&name=coverage&direction=asc&page=3"
        );

        test_support::set_response(Ok(json!({"jobs": []}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","run_id":123}"#,
            Some(r#"{"capability_id":"github.get_workflow_run_jobs"}"#),
        )
        .expect("workflow jobs should use default pagination");
        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/actions/runs/123/jobs?per_page=30"
        );

        test_support::set_response(Ok(json!({"artifacts": []}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","run_id":123}"#,
            Some(r#"{"capability_id":"github.get_workflow_run_artifacts"}"#),
        )
        .expect("workflow artifacts should use default pagination");
        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/actions/runs/123/artifacts?per_page=30"
        );

        test_support::set_response(Ok(json!({"status": 201}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","run_id":123,"enable_debug_logging":true}"#,
            Some(r#"{"capability_id":"github.rerun_failed_workflow_run_jobs"}"#),
        )
        .expect("rerun failed jobs should dispatch");
        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/actions/runs/123/rerun-failed-jobs"
        );
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({"enable_debug_logging": true}));

        test_support::set_response(Ok(json!({"status": 201}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","run_id":123}"#,
            Some(r#"{"capability_id":"github.rerun_failed_workflow_run_jobs"}"#),
        )
        .expect("rerun failed jobs should send an empty body when debug logging is omitted");
        let requests = test_support::requests();
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({}));

        test_support::set_response(Ok(json!({"status": 201}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","job_id":456,"enable_debugger":true}"#,
            Some(r#"{"capability_id":"github.rerun_workflow_job"}"#),
        )
        .expect("rerun workflow job should dispatch");
        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/actions/jobs/456/rerun"
        );
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({"enable_debugger": true}));

        test_support::set_response(Ok(json!({"status": 201}).to_string()));
        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","job_id":456}"#,
            Some(r#"{"capability_id":"github.rerun_workflow_job"}"#),
        )
        .expect("rerun workflow job should send an empty body when debug flags are omitted");
        let requests = test_support::requests();
        let body: serde_json::Value =
            serde_json::from_str(requests[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body, json!({}));
    }

    #[test]
    fn workflow_run_jobs_and_artifacts_reject_invalid_page_or_limit_before_egress() {
        for (capability, input, expected_error) in [
            (
                "github.get_workflow_run_jobs",
                r#"{"owner":"nearai","repo":"ironclaw","run_id":123,"page":0}"#,
                "invalid_page",
            ),
            (
                "github.get_workflow_run_jobs",
                r#"{"owner":"nearai","repo":"ironclaw","run_id":123,"limit":0}"#,
                "invalid_limit",
            ),
            (
                "github.get_workflow_run_artifacts",
                r#"{"owner":"nearai","repo":"ironclaw","run_id":123,"page":0}"#,
                "invalid_page",
            ),
            (
                "github.get_workflow_run_artifacts",
                r#"{"owner":"nearai","repo":"ironclaw","run_id":123,"limit":101}"#,
                "invalid_limit",
            ),
        ] {
            test_support::set_response(Ok(json!({}).to_string()));

            assert_eq!(
                execute_inner(
                    input,
                    Some(&format!(r#"{{"capability_id":"{capability}"}}"#))
                )
                .unwrap_err(),
                expected_error
            );
            assert!(
                test_support::requests().is_empty(),
                "validation error {expected_error} should happen before egress"
            );
        }
    }

    #[test]
    fn paginated_repo_issue_and_pull_tools_reject_invalid_page_or_limit_before_egress() {
        for (capability, input, expected_error) in [
            (
                "github.list_issue_comments",
                r#"{"owner":"nearai","repo":"ironclaw","issue_number":42,"page":0}"#,
                "invalid_page",
            ),
            (
                "github.list_pull_request_comments",
                r#"{"owner":"nearai","repo":"ironclaw","pr_number":42,"limit":0}"#,
                "invalid_limit",
            ),
            (
                "github.get_pull_request_reviews",
                r#"{"owner":"nearai","repo":"ironclaw","pr_number":42,"page":0}"#,
                "invalid_page",
            ),
            (
                "github.list_branches",
                r#"{"owner":"nearai","repo":"ironclaw","limit":0}"#,
                "invalid_limit",
            ),
            (
                "github.list_releases",
                r#"{"owner":"nearai","repo":"ironclaw","page":0}"#,
                "invalid_page",
            ),
            (
                "github.search_code",
                r#"{"query":"repo:nearai/ironclaw","limit":0}"#,
                "invalid_limit",
            ),
            (
                "github.search_issues_pull_requests",
                r#"{"query":"repo:nearai/ironclaw","page":0}"#,
                "invalid_page",
            ),
        ] {
            test_support::set_response(Ok(json!({}).to_string()));

            assert_eq!(
                execute_inner(
                    input,
                    Some(&format!(r#"{{"capability_id":"{capability}"}}"#))
                )
                .unwrap_err(),
                expected_error
            );
            assert!(
                test_support::requests().is_empty(),
                "validation error {expected_error} should happen before egress"
            );
        }
    }

    #[test]
    fn create_issue_rejects_empty_or_too_many_labels_and_assignees() {
        let too_many_assignees: Vec<String> =
            (0..101).map(|index| format!("user-{index}")).collect();
        let too_many_assignees_input = json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "title": "bug",
            "assignees": too_many_assignees,
        })
        .to_string();

        for (input, expected_error) in [
            (
                r#"{"owner":"nearai","repo":"ironclaw","title":"bug","labels":[""]}"#.to_string(),
                "Invalid labels: values cannot be empty",
            ),
            (
                r#"{"owner":"nearai","repo":"ironclaw","title":"bug","assignees":[""]}"#
                    .to_string(),
                "Invalid assignees: values cannot be empty",
            ),
            (
                too_many_assignees_input,
                "Invalid assignees: at most 100 values are allowed",
            ),
        ] {
            test_support::set_response(Ok(json!({"number": 12}).to_string()));

            assert_eq!(
                execute_inner(&input, Some(r#"{"capability_id":"github.create_issue"}"#))
                    .unwrap_err(),
                expected_error
            );
            assert!(
                test_support::requests().is_empty(),
                "validation error {expected_error} should happen before egress"
            );
        }
    }

    #[test]
    fn list_repos_uses_authenticated_endpoint_by_default() {
        test_support::set_response(Ok(json!([]).to_string()));

        execute_inner(r#"{}"#, Some(r#"{"capability_id":"github.list_repos"}"#))
            .expect("github.list_repos should list authenticated user repos");

        let requests = test_support::requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].body, None);
        assert_eq!(requests[0].path, "/user/repos?per_page=30");
    }

    #[test]
    fn list_repos_appends_type_for_authenticated_user() {
        test_support::set_response(Ok(json!([]).to_string()));

        execute_inner(
            r#"{"type":"member","limit":2}"#,
            Some(r#"{"capability_id":"github.list_repos"}"#),
        )
        .expect("github.list_repos should accept type");

        let requests = test_support::requests();
        assert_eq!(requests[0].path, "/user/repos?per_page=2&type=member");
    }

    #[test]
    fn list_repos_appends_page_when_provided() {
        test_support::set_response(Ok(json!([]).to_string()));

        execute_inner(
            r#"{"page":2,"limit":7}"#,
            Some(r#"{"capability_id":"github.list_repos"}"#),
        )
        .expect("github.list_repos should accept page");

        let requests = test_support::requests();
        assert_eq!(requests[0].path, "/user/repos?per_page=7&page=2");
    }

    #[test]
    fn list_repos_appends_supported_types_for_authenticated_user() {
        for (repo_type, expected_path) in [
            ("all", "/user/repos?per_page=3&type=all"),
            ("owner", "/user/repos?per_page=3&type=owner"),
            ("public", "/user/repos?per_page=3&type=public"),
            ("private", "/user/repos?per_page=3&type=private"),
            ("member", "/user/repos?per_page=3&type=member"),
        ] {
            test_support::set_response(Ok(json!([]).to_string()));

            execute_inner(
                &format!(r#"{{"type":"{repo_type}","limit":3}}"#),
                Some(r#"{"capability_id":"github.list_repos"}"#),
            )
            .expect("github.list_repos should accept authenticated-user repo types");

            let requests = test_support::requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].path, expected_path);
        }
    }

    #[test]
    fn list_repos_rejects_invalid_page_or_limit() {
        for (input, expected_error) in [
            (r#"{"page":0}"#, "invalid_page"),
            (r#"{"limit":0}"#, "invalid_limit"),
        ] {
            test_support::set_response(Ok(json!([]).to_string()));

            assert_eq!(
                execute_inner(input, Some(r#"{"capability_id":"github.list_repos"}"#)).unwrap_err(),
                expected_error
            );
            assert!(
                test_support::requests().is_empty(),
                "validation error {expected_error} should happen before egress"
            );
        }
    }

    #[test]
    fn search_repositories_rejects_invalid_page_or_limit_before_egress() {
        for (input, expected_error) in [
            (
                r#"{"query":"repo:nearai/ironclaw","page":0}"#,
                "invalid_page",
            ),
            (
                r#"{"query":"repo:nearai/ironclaw","limit":0}"#,
                "invalid_limit",
            ),
        ] {
            test_support::set_response(Ok(json!([]).to_string()));

            assert_eq!(
                execute_inner(
                    input,
                    Some(r#"{"capability_id":"github.search_repositories"}"#)
                )
                .unwrap_err(),
                expected_error
            );
            assert!(
                test_support::requests().is_empty(),
                "validation error {expected_error} should happen before egress"
            );
        }
    }

    #[test]
    fn list_repos_rejects_username_field_before_egress() {
        test_support::set_response(Ok(json!([]).to_string()));

        assert_eq!(
            execute_inner(
                r#"{"username":"nearai","limit":11,"page":2}"#,
                Some(r#"{"capability_id":"github.list_repos"}"#),
            )
            .unwrap_err(),
            "invalid_parameters"
        );
        assert!(
            test_support::requests().is_empty(),
            "username should no longer deserialize for github.list_repos"
        );
    }

    #[test]
    fn get_authenticated_user_uses_user_endpoint() {
        test_support::set_response(Ok(json!({
            "login": "serrrfirat",
            "type": "User"
        })
        .to_string()));

        let output = execute_inner(
            r#"{}"#,
            Some(r#"{"capability_id":"github.get_authenticated_user"}"#),
        )
        .expect("github.get_authenticated_user should return authenticated user");
        let output: serde_json::Value =
            serde_json::from_str(&output).expect("mock output should be JSON");
        assert_eq!(output["login"], "serrrfirat");
        assert_eq!(output["type"], "User");

        let requests = test_support::requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].body, None);
        assert_eq!(requests[0].path, "/user");
    }

    #[test]
    fn get_file_content_uses_ref_query() {
        test_support::set_response(Ok(json!({
            "path": "src/lib.rs",
            "encoding": "base64",
            "content": "Zm4gbWFpbigpIHt9"
        })
        .to_string()));

        execute_inner(
            r#"{"owner":"nearai","repo":"ironclaw","path":"src/lib.rs","ref":"main"}"#,
            Some(r#"{"capability_id":"github.get_file_content"}"#),
        )
        .expect("github.get_file_content should fetch content");

        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/repos/nearai/ironclaw/contents/src/lib.rs?ref=main"
        );
    }

    #[test]
    fn search_issues_pull_requests_accepts_wider_sort() {
        test_support::set_response(Ok(json!({"items": []}).to_string()));

        execute_inner(
            r#"{"repo":"nearai/ironclaw","type":"pr","sort":"reactions-heart","order":"desc","limit":5}"#,
            Some(r#"{"capability_id":"github.search_issues_pull_requests"}"#),
        )
        .expect("github.search_issues_pull_requests should accept GitHub issue-search sort");

        let requests = test_support::requests();
        assert_eq!(
            requests[0].path,
            "/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Apr&per_page=5&sort=reactions-heart&order=desc"
        );
    }

    #[test]
    fn search_issues_pull_requests_rejects_commit_search_sort() {
        let err = execute_inner(
            r#"{"repo":"nearai/ironclaw","type":"pr","sort":"author-date"}"#,
            Some(r#"{"capability_id":"github.search_issues_pull_requests"}"#),
        )
        .unwrap_err();

        assert!(err.contains("invalid_sort"));
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
