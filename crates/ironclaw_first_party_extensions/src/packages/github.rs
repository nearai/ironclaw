//! GitHub extension package — repo/issue/PR/release/workflow tools over a WASM
//! executor, host-mediated egress. Assets: input/output JSON schemas, per-tool
//! prompt docs, and the tool WASM module.

use std::borrow::Cow;

use super::{PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const ID: &str = "github";

const MANIFEST: &str = include_str!("../../assets/github/manifest.toml");
const WASM: &[u8] = include_bytes!("../../assets/github/wasm/github_tool.wasm");

pub(super) fn bundle() -> PackageBundle {
    PackageBundle {
        id: ID,
        display_name: "GitHub",
        manifest_toml: Cow::Borrowed(MANIFEST),
        assets: assets(),
        onboarding: Some(PackageOnboarding {
            instructions: "GitHub needs a personal access token before its \
                repository and pull request tools can run."
                .to_string(),
            credential_instructions: Some(
                "Create a GitHub personal access token with the repository \
                permissions you want IronClaw to use, then paste it here."
                    .to_string(),
            ),
            setup_url: Some("https://github.com/settings/personal-access-tokens/new".to_string()),
            credential_next_step: "After saving the token, IronClaw finishes GitHub installation \
                automatically and publishes its tools."
                .to_string(),
        }),
        // WASM tool package: trust comes from the extension registry, not an
        // admin local-manifest effect grant.
        trust_effects: None,
    }
}

fn assets() -> Vec<super::PackageAsset> {
    macro_rules! github_schema_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("schemas/github/", $path),
                include_bytes!(concat!("../../assets/github/schemas/github/", $path)),
            )
        };
    }
    macro_rules! github_prompt_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("prompts/github/", $path),
                include_bytes!(concat!("../../assets/github/prompts/github/", $path)),
            )
        };
    }

    vec![
        bytes_asset("manifest.toml", MANIFEST.as_bytes()),
        github_schema_asset!("add_issue_assignees.input.v1.json"),
        github_schema_asset!("add_issue_labels.input.v1.json"),
        github_schema_asset!("comment_issue.input.v1.json"),
        github_schema_asset!("comment_issue.output.v1.json"),
        github_schema_asset!("create_branch.input.v1.json"),
        github_schema_asset!("create_issue.input.v1.json"),
        github_schema_asset!("create_issue_comment.input.v1.json"),
        github_schema_asset!("create_or_update_file.input.v1.json"),
        github_schema_asset!("create_pr_review.input.v1.json"),
        github_schema_asset!("create_pull_request.input.v1.json"),
        github_schema_asset!("create_release.input.v1.json"),
        github_schema_asset!("create_repo.input.v1.json"),
        github_schema_asset!("delete_file.input.v1.json"),
        github_schema_asset!("fork_repo.input.v1.json"),
        github_schema_asset!("get_combined_status.input.v1.json"),
        github_schema_asset!("get_file_content.input.v1.json"),
        github_schema_asset!("get_issue.input.v1.json"),
        github_schema_asset!("get_job_logs.input.v1.json"),
        github_schema_asset!("get_issue.output.v1.json"),
        github_schema_asset!("get_pull_request.input.v1.json"),
        github_schema_asset!("get_pull_request_files.input.v1.json"),
        github_schema_asset!("get_pull_request_reviews.input.v1.json"),
        github_schema_asset!("get_repo.input.v1.json"),
        github_schema_asset!("get_authenticated_user.input.v1.json"),
        github_schema_asset!("get_workflow_run_artifacts.input.v1.json"),
        github_schema_asset!("get_workflow_run_jobs.input.v1.json"),
        github_schema_asset!("get_workflow_runs.input.v1.json"),
        github_schema_asset!("handle_webhook.input.v1.json"),
        github_schema_asset!("list_branches.input.v1.json"),
        github_schema_asset!("list_issue_comments.input.v1.json"),
        github_schema_asset!("list_issues.input.v1.json"),
        github_schema_asset!("list_pull_request_comments.input.v1.json"),
        github_schema_asset!("list_pull_request_review_threads.input.v1.json"),
        github_schema_asset!("list_pull_requests.input.v1.json"),
        github_schema_asset!("list_releases.input.v1.json"),
        github_schema_asset!("list_repos.input.v1.json"),
        github_schema_asset!("merge_pull_request.input.v1.json"),
        github_schema_asset!("raw_output.v1.json"),
        github_schema_asset!("remove_issue_assignees.input.v1.json"),
        github_schema_asset!("remove_issue_label.input.v1.json"),
        github_schema_asset!("reply_pull_request_comment.input.v1.json"),
        github_schema_asset!("rerun_failed_workflow_run_jobs.input.v1.json"),
        github_schema_asset!("rerun_workflow_job.input.v1.json"),
        github_schema_asset!("resolve_review_thread.input.v1.json"),
        github_schema_asset!("search_code.input.v1.json"),
        github_schema_asset!("search_issues.input.v1.json"),
        github_schema_asset!("search_issues.output.v1.json"),
        github_schema_asset!("search_issues_pull_requests.input.v1.json"),
        github_schema_asset!("search_repositories.input.v1.json"),
        github_schema_asset!("trigger_workflow.input.v1.json"),
        github_schema_asset!("unresolve_review_thread.input.v1.json"),
        github_schema_asset!("update_issue.input.v1.json"),
        github_schema_asset!("update_pull_request.input.v1.json"),
        github_prompt_asset!("add_issue_assignees.md"),
        github_prompt_asset!("add_issue_labels.md"),
        github_prompt_asset!("comment_issue.md"),
        github_prompt_asset!("create_branch.md"),
        github_prompt_asset!("create_issue.md"),
        github_prompt_asset!("create_issue_comment.md"),
        github_prompt_asset!("create_or_update_file.md"),
        github_prompt_asset!("create_pr_review.md"),
        github_prompt_asset!("create_pull_request.md"),
        github_prompt_asset!("create_release.md"),
        github_prompt_asset!("create_repo.md"),
        github_prompt_asset!("delete_file.md"),
        github_prompt_asset!("fork_repo.md"),
        github_prompt_asset!("get_combined_status.md"),
        github_prompt_asset!("get_file_content.md"),
        github_prompt_asset!("get_issue.md"),
        github_prompt_asset!("get_job_logs.md"),
        github_prompt_asset!("get_pull_request.md"),
        github_prompt_asset!("get_pull_request_files.md"),
        github_prompt_asset!("get_pull_request_reviews.md"),
        github_prompt_asset!("get_repo.md"),
        github_prompt_asset!("get_authenticated_user.md"),
        github_prompt_asset!("get_workflow_run_artifacts.md"),
        github_prompt_asset!("get_workflow_run_jobs.md"),
        github_prompt_asset!("get_workflow_runs.md"),
        github_prompt_asset!("handle_webhook.md"),
        github_prompt_asset!("list_branches.md"),
        github_prompt_asset!("list_issue_comments.md"),
        github_prompt_asset!("list_issues.md"),
        github_prompt_asset!("list_pull_request_comments.md"),
        github_prompt_asset!("list_pull_request_review_threads.md"),
        github_prompt_asset!("list_pull_requests.md"),
        github_prompt_asset!("list_releases.md"),
        github_prompt_asset!("list_repos.md"),
        github_prompt_asset!("merge_pull_request.md"),
        github_prompt_asset!("remove_issue_assignees.md"),
        github_prompt_asset!("remove_issue_label.md"),
        github_prompt_asset!("reply_pull_request_comment.md"),
        github_prompt_asset!("rerun_failed_workflow_run_jobs.md"),
        github_prompt_asset!("rerun_workflow_job.md"),
        github_prompt_asset!("resolve_review_thread.md"),
        github_prompt_asset!("search_code.md"),
        github_prompt_asset!("search_issues.md"),
        github_prompt_asset!("search_issues_pull_requests.md"),
        github_prompt_asset!("search_repositories.md"),
        github_prompt_asset!("trigger_workflow.md"),
        github_prompt_asset!("unresolve_review_thread.md"),
        github_prompt_asset!("update_issue.md"),
        github_prompt_asset!("update_pull_request.md"),
        bytes_asset("wasm/github_tool.wasm", WASM),
    ]
}
