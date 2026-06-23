use crate::api::*;
use crate::types::{GitHubAction, ToolContext};
use crate::webhook::handle_webhook;

pub(crate) fn execute_inner(params: &str, context: Option<&str>) -> Result<String, String> {
    let action_name = action_from_context(context)?;
    let action_params = params_with_action(params, action_name)?;
    let action: GitHubAction =
        serde_json::from_value(action_params).map_err(|_| "invalid_parameters".to_string())?;

    match action {
        GitHubAction::GetRepo { owner, repo } => get_repo(&owner, &repo),
        GitHubAction::CreateRepo {
            name,
            description,
            private,
            auto_init,
            gitignore_template,
            license_template,
            org,
        } => create_repo(
            &name,
            description.as_deref(),
            private.unwrap_or(false),
            auto_init.unwrap_or(false),
            gitignore_template.as_deref(),
            license_template.as_deref(),
            org.as_deref(),
        ),
        GitHubAction::ListIssues {
            owner,
            repo,
            state,
            labels,
            assignee,
            milestone,
            page,
            limit,
        } => list_issues(
            &owner,
            &repo,
            state.as_deref(),
            labels,
            assignee.as_deref(),
            milestone.as_deref(),
            page,
            limit,
        ),
        GitHubAction::CreateIssue {
            owner,
            repo,
            title,
            body,
            labels,
            assignees,
        } => create_issue(&owner, &repo, &title, body.as_deref(), labels, assignees),
        GitHubAction::GetIssue {
            owner,
            repo,
            issue_number,
        } => get_issue(&owner, &repo, issue_number),
        GitHubAction::ListIssueComments {
            owner,
            repo,
            issue_number,
            page,
            limit,
        } => list_issue_comments(&owner, &repo, issue_number, page, limit),
        GitHubAction::CreateIssueComment {
            owner,
            repo,
            issue_number,
            body,
        } => create_issue_comment(&owner, &repo, issue_number, &body),
        GitHubAction::ListPullRequests {
            owner,
            repo,
            state,
            head,
            base,
            sort,
            direction,
            page,
            limit,
        } => list_pull_requests(
            &owner,
            &repo,
            state.as_deref(),
            head.as_deref(),
            base.as_deref(),
            sort.as_deref(),
            direction.as_deref(),
            page,
            limit,
        ),
        GitHubAction::CreatePullRequest {
            owner,
            repo,
            title,
            head,
            base,
            body,
            draft,
        } => create_pull_request(
            &owner,
            &repo,
            &title,
            &head,
            &base,
            body.as_deref(),
            draft.unwrap_or(false),
        ),
        GitHubAction::GetPullRequest {
            owner,
            repo,
            pr_number,
        } => get_pull_request(&owner, &repo, pr_number),
        GitHubAction::GetPullRequestFiles {
            owner,
            repo,
            pr_number,
            page,
            limit,
        } => get_pull_request_files(&owner, &repo, pr_number, page, limit),
        GitHubAction::CreatePrReview {
            owner,
            repo,
            pr_number,
            body,
            event,
        } => create_pr_review(&owner, &repo, pr_number, &body, event),
        GitHubAction::ListPullRequestComments {
            owner,
            repo,
            pr_number,
            page,
            limit,
        } => list_pull_request_comments(&owner, &repo, pr_number, page, limit),
        GitHubAction::ReplyPullRequestComment {
            owner,
            repo,
            pr_number,
            comment_id,
            body,
        } => reply_pull_request_comment(&owner, &repo, pr_number, comment_id, &body),
        GitHubAction::GetPullRequestReviews {
            owner,
            repo,
            pr_number,
            page,
            limit,
        } => get_pull_request_reviews(&owner, &repo, pr_number, page, limit),
        GitHubAction::GetCombinedStatus { owner, repo, r#ref } => {
            get_combined_status(&owner, &repo, &r#ref)
        }
        GitHubAction::MergePullRequest {
            owner,
            repo,
            pr_number,
            commit_title,
            commit_message,
            merge_method,
            sha,
        } => merge_pull_request(
            &owner,
            &repo,
            pr_number,
            commit_title.as_deref(),
            commit_message.as_deref(),
            merge_method,
            sha.as_deref(),
        ),
        GitHubAction::GetAuthenticatedUser {} => get_authenticated_user(),
        GitHubAction::ListRepos {
            username,
            repo_type,
            page,
            limit,
        } => list_repos(username.as_deref(), repo_type, page, limit),
        GitHubAction::SearchRepositories {
            query,
            page,
            limit,
            sort,
            order,
        } => search_repositories(&query, page, limit, sort.as_deref(), order.as_deref()),
        GitHubAction::SearchCode {
            query,
            page,
            limit,
            sort,
            order,
        } => search_code(&query, page, limit, sort.as_deref(), order.as_deref()),
        GitHubAction::SearchIssuesPullRequests {
            query,
            repository,
            owner,
            repo,
            author,
            assignee,
            involves,
            state,
            issue_type,
            page,
            limit,
            sort,
            order,
        } => search_issues_pull_requests(
            query.as_deref(),
            repository.as_deref(),
            owner.as_deref(),
            repo.as_deref(),
            author.as_deref(),
            assignee.as_deref(),
            involves.as_deref(),
            state.as_deref(),
            issue_type.as_deref(),
            page,
            limit,
            sort.as_deref(),
            order.as_deref(),
        ),
        GitHubAction::ListBranches {
            owner,
            repo,
            protected,
            page,
            limit,
        } => list_branches(&owner, &repo, protected, page, limit),
        GitHubAction::CreateBranch {
            owner,
            repo,
            branch,
            from_ref,
        } => create_branch(&owner, &repo, &branch, &from_ref),
        GitHubAction::GetFileContent {
            owner,
            repo,
            path,
            r#ref,
        } => get_file_content(&owner, &repo, &path, r#ref.as_deref()),
        GitHubAction::CreateOrUpdateFile {
            owner,
            repo,
            path,
            message,
            content,
            sha,
            branch,
            committer,
            author,
        } => create_or_update_file(
            &owner,
            &repo,
            &path,
            &message,
            &content,
            sha.as_deref(),
            branch.as_deref(),
            committer,
            author,
        ),
        GitHubAction::DeleteFile {
            owner,
            repo,
            path,
            message,
            sha,
            branch,
            committer,
            author,
        } => delete_file(
            &owner,
            &repo,
            &path,
            &message,
            &sha,
            branch.as_deref(),
            committer,
            author,
        ),
        GitHubAction::ListReleases {
            owner,
            repo,
            page,
            limit,
        } => list_releases(&owner, &repo, page, limit),
        GitHubAction::CreateRelease {
            owner,
            repo,
            tag_name,
            target_commitish,
            name,
            body,
            draft,
            prerelease,
            generate_release_notes,
        } => create_release(
            &owner,
            &repo,
            &tag_name,
            target_commitish.as_deref(),
            name.as_deref(),
            body.as_deref(),
            draft.unwrap_or(false),
            prerelease.unwrap_or(false),
            generate_release_notes.unwrap_or(false),
        ),
        GitHubAction::TriggerWorkflow {
            owner,
            repo,
            workflow_id,
            r#ref,
            inputs,
        } => trigger_workflow(&owner, &repo, &workflow_id, &r#ref, inputs),
        GitHubAction::GetWorkflowRuns {
            owner,
            repo,
            workflow_id,
            page,
            limit,
        } => get_workflow_runs(&owner, &repo, workflow_id.as_deref(), page, limit),
        GitHubAction::ForkRepo {
            owner,
            repo,
            organization,
            name,
            default_branch_only,
        } => fork_repo(
            &owner,
            &repo,
            organization.as_deref(),
            name.as_deref(),
            default_branch_only,
        ),
        GitHubAction::HandleWebhook { webhook } => handle_webhook(webhook),
    }
}

pub(crate) fn action_from_context(context: Option<&str>) -> Result<&'static str, String> {
    let context = context.ok_or_else(|| "missing_invocation_context".to_string())?;
    let context: ToolContext =
        serde_json::from_str(context).map_err(|_| "invalid_invocation_context".to_string())?;
    match context.capability_id.as_str() {
        "github.get_repo" => Ok("get_repo"),
        "github.create_repo" => Ok("create_repo"),
        "github.list_issues" => Ok("list_issues"),
        "github.create_issue" => Ok("create_issue"),
        "github.get_issue" => Ok("get_issue"),
        "github.list_issue_comments" => Ok("list_issue_comments"),
        "github.create_issue_comment" | "github.comment_issue" => Ok("create_issue_comment"),
        "github.list_pull_requests" => Ok("list_pull_requests"),
        "github.create_pull_request" => Ok("create_pull_request"),
        "github.get_pull_request" => Ok("get_pull_request"),
        "github.get_pull_request_files" => Ok("get_pull_request_files"),
        "github.create_pr_review" => Ok("create_pr_review"),
        "github.list_pull_request_comments" => Ok("list_pull_request_comments"),
        "github.reply_pull_request_comment" => Ok("reply_pull_request_comment"),
        "github.get_pull_request_reviews" => Ok("get_pull_request_reviews"),
        "github.get_combined_status" => Ok("get_combined_status"),
        "github.merge_pull_request" => Ok("merge_pull_request"),
        "github.get_authenticated_user" => Ok("get_authenticated_user"),
        "github.list_repos" => Ok("list_repos"),
        "github.search_repositories" => Ok("search_repositories"),
        "github.search_code" => Ok("search_code"),
        "github.search_issues" | "github.search_issues_pull_requests" => {
            Ok("search_issues_pull_requests")
        }
        "github.list_branches" => Ok("list_branches"),
        "github.create_branch" => Ok("create_branch"),
        "github.get_file_content" => Ok("get_file_content"),
        "github.create_or_update_file" => Ok("create_or_update_file"),
        "github.delete_file" => Ok("delete_file"),
        "github.list_releases" => Ok("list_releases"),
        "github.create_release" => Ok("create_release"),
        "github.trigger_workflow" => Ok("trigger_workflow"),
        "github.get_workflow_runs" => Ok("get_workflow_runs"),
        "github.fork_repo" => Ok("fork_repo"),
        "github.handle_webhook" => Ok("handle_webhook"),
        _ => Err("unsupported_github_capability".to_string()),
    }
}

fn params_with_action(params: &str, action: &str) -> Result<serde_json::Value, String> {
    let mut params: serde_json::Value =
        serde_json::from_str(params).map_err(|_| "invalid_parameters".to_string())?;
    let obj = params
        .as_object_mut()
        .ok_or_else(|| "invalid_parameters".to_string())?;
    if obj.contains_key("action") {
        return Err("invalid_parameters".to_string());
    }
    obj.insert(
        "action".to_string(),
        serde_json::Value::String(action.to_string()),
    );
    Ok(params)
}
