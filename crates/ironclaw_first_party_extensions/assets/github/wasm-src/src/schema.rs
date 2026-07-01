pub(crate) fn schema() -> String {
    let schemas = GITHUB_SCHEMAS
        .iter()
        .copied()
        .map(schema_value)
        .collect::<Vec<_>>();
    serde_json::json!({
        "type": "object",
        "oneOf": schemas,
    })
    .to_string()
}

pub(crate) fn action_name_from_capability_id(capability_id: &str) -> Option<String> {
    let action = capability_id.strip_prefix("github.")?;
    let action = match action {
        "comment_issue" => "create_issue_comment",
        "search_issues" => "search_issues_pull_requests",
        action => action,
    };
    GITHUB_SCHEMAS
        .iter()
        .any(|schema| schema_action_name(schema).as_deref() == Some(action))
        .then(|| action.to_string())
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

fn schema_action_name(schema: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(schema).ok()?;
    value
        .get("title")?
        .as_str()?
        .strip_prefix("GitHub ")?
        .strip_suffix(" input")
        .map(str::to_string)
}

const GITHUB_SCHEMAS: &[&str] = &[
    include_str!("../../schemas/github/get_repo.input.v1.json"),
    include_str!("../../schemas/github/create_repo.input.v1.json"),
    include_str!("../../schemas/github/list_issues.input.v1.json"),
    include_str!("../../schemas/github/create_issue.input.v1.json"),
    include_str!("../../schemas/github/update_issue.input.v1.json"),
    include_str!("../../schemas/github/add_issue_labels.input.v1.json"),
    include_str!("../../schemas/github/remove_issue_label.input.v1.json"),
    include_str!("../../schemas/github/add_issue_assignees.input.v1.json"),
    include_str!("../../schemas/github/remove_issue_assignees.input.v1.json"),
    include_str!("../../schemas/github/get_issue.input.v1.json"),
    include_str!("../../schemas/github/list_issue_comments.input.v1.json"),
    include_str!("../../schemas/github/create_issue_comment.input.v1.json"),
    include_str!("../../schemas/github/comment_issue.input.v1.json"),
    include_str!("../../schemas/github/list_pull_requests.input.v1.json"),
    include_str!("../../schemas/github/create_pull_request.input.v1.json"),
    include_str!("../../schemas/github/update_pull_request.input.v1.json"),
    include_str!("../../schemas/github/get_pull_request.input.v1.json"),
    include_str!("../../schemas/github/get_pull_request_files.input.v1.json"),
    include_str!("../../schemas/github/create_pr_review.input.v1.json"),
    include_str!("../../schemas/github/list_pull_request_comments.input.v1.json"),
    include_str!("../../schemas/github/reply_pull_request_comment.input.v1.json"),
    include_str!("../../schemas/github/get_pull_request_reviews.input.v1.json"),
    include_str!("../../schemas/github/list_pull_request_review_threads.input.v1.json"),
    include_str!("../../schemas/github/resolve_review_thread.input.v1.json"),
    include_str!("../../schemas/github/unresolve_review_thread.input.v1.json"),
    include_str!("../../schemas/github/get_combined_status.input.v1.json"),
    include_str!("../../schemas/github/merge_pull_request.input.v1.json"),
    include_str!("../../schemas/github/get_authenticated_user.input.v1.json"),
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
    include_str!("../../schemas/github/get_workflow_run_jobs.input.v1.json"),
    include_str!("../../schemas/github/get_workflow_run_artifacts.input.v1.json"),
    include_str!("../../schemas/github/rerun_failed_workflow_run_jobs.input.v1.json"),
    include_str!("../../schemas/github/rerun_workflow_job.input.v1.json"),
    include_str!("../../schemas/github/fork_repo.input.v1.json"),
    include_str!("../../schemas/github/handle_webhook.input.v1.json"),
];
