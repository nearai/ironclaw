pub(crate) fn schema() -> String {
    let schemas = [
        include_str!("../../schemas/github/get_repo.input.v1.json"),
        include_str!("../../schemas/github/create_repo.input.v1.json"),
        include_str!("../../schemas/github/list_issues.input.v1.json"),
        include_str!("../../schemas/github/create_issue.input.v1.json"),
        include_str!("../../schemas/github/get_issue.input.v1.json"),
        include_str!("../../schemas/github/list_issue_comments.input.v1.json"),
        include_str!("../../schemas/github/create_issue_comment.input.v1.json"),
        include_str!("../../schemas/github/comment_issue.input.v1.json"),
        include_str!("../../schemas/github/list_pull_requests.input.v1.json"),
        include_str!("../../schemas/github/create_pull_request.input.v1.json"),
        include_str!("../../schemas/github/get_pull_request.input.v1.json"),
        include_str!("../../schemas/github/get_pull_request_files.input.v1.json"),
        include_str!("../../schemas/github/create_pr_review.input.v1.json"),
        include_str!("../../schemas/github/list_pull_request_comments.input.v1.json"),
        include_str!("../../schemas/github/reply_pull_request_comment.input.v1.json"),
        include_str!("../../schemas/github/get_pull_request_reviews.input.v1.json"),
        include_str!("../../schemas/github/get_combined_status.input.v1.json"),
        include_str!("../../schemas/github/merge_pull_request.input.v1.json"),
        include_str!("../../schemas/github/list_repos.input.v1.json"),
        include_str!("../../schemas/github/search_repositories.input.v1.json"),
        include_str!("../../schemas/github/search_code.input.v1.json"),
        include_str!("../../schemas/github/search_issues.input.v1.json"),
        include_str!("../../schemas/github/search_issues_pull_requests.input.v1.json"),
        include_str!("../../schemas/github/list_branches.input.v1.json"),
        include_str!("../../schemas/github/create_branch.input.v1.json"),
        include_str!("../../schemas/github/get_file_content.input.v1.json"),
        include_str!("../../schemas/github/create_or_update_file.input.v1.json"),
        include_str!("../../schemas/github/delete_file.input.v1.json"),
        include_str!("../../schemas/github/list_releases.input.v1.json"),
        include_str!("../../schemas/github/create_release.input.v1.json"),
        include_str!("../../schemas/github/trigger_workflow.input.v1.json"),
        include_str!("../../schemas/github/get_workflow_runs.input.v1.json"),
        include_str!("../../schemas/github/fork_repo.input.v1.json"),
        include_str!("../../schemas/github/handle_webhook.input.v1.json"),
    ]
    .into_iter()
    .map(schema_value)
    .collect::<Vec<_>>();
    serde_json::json!({
        "type": "object",
        "oneOf": schemas,
    })
    .to_string()
}

fn schema_value(schema: &str) -> serde_json::Value {
    match serde_json::from_str(schema) {
        Ok(value) => value,
        Err(error) => serde_json::json!({
            "not": {},
            "description": format!("invalid bundled GitHub schema: {error}"),
        }),
    }
}
