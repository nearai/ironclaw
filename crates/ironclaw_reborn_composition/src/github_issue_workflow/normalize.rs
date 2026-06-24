//! Provider-response normalization helpers for the GitHub issue workflow.
//!
//! Each `normalize_*` function converts a raw provider JSON payload into the
//! strongly-typed workflow snapshot it backs, surfacing
//! [`GithubIssueWorkflowError::ProviderRead`] on malformed input. The
//! `required_*` / `optional_*` / `json_at_path` helpers are the shared JSON
//! accessors the normalizers build on.

use chrono::{DateTime, Utc};
use ironclaw_github_issue_workflow::{
    GithubActorSnapshot, GithubCheckConclusion, GithubCommentRef, GithubIssueCommentSnapshot,
    GithubIssueProviderSnapshot, GithubIssueSearchHit, GithubIssueWorkflowError,
    GithubPullRequestCheckSnapshot, GithubPullRequestRef, GithubPullRequestSnapshot,
    GithubReviewCommentSnapshot,
};
use serde_json::Value as JsonValue;

use super::{
    GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID, GITHUB_GET_AUTHENTICATED_USER_CAPABILITY_ID,
    GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID, GITHUB_GET_ISSUE_CAPABILITY_ID,
    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID, GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
    GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID, GITHUB_SEARCH_ISSUES_CAPABILITY_ID,
};

pub(super) fn normalize_issue_search_hits(
    value: &JsonValue,
    owner: &str,
    repo: &str,
) -> Result<Vec<GithubIssueSearchHit>, GithubIssueWorkflowError> {
    let items = match value {
        JsonValue::Array(items) => items,
        _ => required_array(value, &["items"], GITHUB_SEARCH_ISSUES_CAPABILITY_ID)?,
    };
    items
        .iter()
        .map(|item| {
            let number = required_u64(item, &["number"], GITHUB_SEARCH_ISSUES_CAPABILITY_ID)?;
            Ok(GithubIssueSearchHit {
                owner: owner.to_string(),
                repo: repo.to_string(),
                number,
                node_id: optional_string(item, &[&["node_id"]]),
                url: issue_like_url(item, owner, repo, number),
                default_branch: optional_string(
                    item,
                    &[
                        &["repository", "default_branch"],
                        &["base", "repo", "default_branch"],
                        &["default_branch"],
                    ],
                )
                .unwrap_or_default(),
                updated_at: optional_rfc3339_datetime(item, &[&["updated_at"]]),
            })
        })
        .collect()
}

pub(super) fn normalize_issue_snapshot(
    value: &JsonValue,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<GithubIssueProviderSnapshot, GithubIssueWorkflowError> {
    Ok(GithubIssueProviderSnapshot {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        node_id: optional_string(value, &[&["node_id"]]),
        url: issue_like_url(value, owner, repo, number),
        default_branch: optional_string(
            value,
            &[
                &["repository", "default_branch"],
                &["base", "repo", "default_branch"],
                &["default_branch"],
            ],
        )
        .unwrap_or_default(),
        title: required_string(value, &["title"], GITHUB_GET_ISSUE_CAPABILITY_ID)?.to_string(),
        body: optional_string(value, &[&["body"]]).unwrap_or_default(),
        state: required_string(value, &["state"], GITHUB_GET_ISSUE_CAPABILITY_ID)?.to_string(),
        author_login: optional_string(value, &[&["user", "login"], &["author", "login"]]),
        labels: optional_labels(value),
        updated_at: optional_rfc3339_datetime(value, &[&["updated_at"]]),
    })
}

pub(super) fn normalize_actor_snapshot(
    value: &JsonValue,
) -> Result<GithubActorSnapshot, GithubIssueWorkflowError> {
    Ok(GithubActorSnapshot {
        login: required_string(
            value,
            &["login"],
            GITHUB_GET_AUTHENTICATED_USER_CAPABILITY_ID,
        )?
        .to_string(),
        node_id: optional_string(value, &[&["node_id"]]),
    })
}

pub(super) fn normalize_issue_comments(
    value: &JsonValue,
    issue: &ironclaw_github_issue_workflow::GithubIssueRef,
) -> Result<Vec<GithubIssueCommentSnapshot>, GithubIssueWorkflowError> {
    let comments = match value {
        JsonValue::Array(items) => items,
        _ => required_array(value, &["items"], GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID)?,
    };

    comments
        .iter()
        .map(|comment| {
            Ok(GithubIssueCommentSnapshot {
                comment: normalize_comment_ref(
                    comment,
                    Some(issue),
                    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
                )?,
                body: optional_string(comment, &[&["body"]]).unwrap_or_default(),
                author_login: optional_string(comment, &[&["user", "login"], &["author", "login"]])
                    .unwrap_or_default(),
                created_at: required_datetime(
                    comment,
                    &[&["created_at"]],
                    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
                )?,
                updated_at: required_datetime(
                    comment,
                    &[&["updated_at"], &["created_at"]],
                    GITHUB_LIST_ISSUE_COMMENTS_CAPABILITY_ID,
                )?,
            })
        })
        .collect()
}

pub(super) fn normalize_comment_ref(
    value: &JsonValue,
    issue: Option<&ironclaw_github_issue_workflow::GithubIssueRef>,
    capability_id: &str,
) -> Result<GithubCommentRef, GithubIssueWorkflowError> {
    let url = if let Some(url) = optional_string(value, &[&["html_url"], &["url"]]) {
        url
    } else if let (Some(issue), Some(comment_id)) =
        (issue, value.get("id").and_then(JsonValue::as_u64))
    {
        format!("{}#issuecomment-{comment_id}", issue.url)
    } else if let Some(issue) = issue {
        issue.url.clone()
    } else {
        return Err(invalid_output(
            capability_id,
            "comment response is missing url",
        ));
    };

    Ok(GithubCommentRef {
        node_id: optional_string(value, &[&["node_id"]]),
        url,
    })
}

pub(super) fn normalize_pull_request_snapshots(
    value: &JsonValue,
    owner: &str,
    repo: &str,
) -> Result<Vec<GithubPullRequestSnapshot>, GithubIssueWorkflowError> {
    let items = match value {
        JsonValue::Array(items) => items,
        _ => required_array(value, &["items"], GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID)?,
    };
    items
        .iter()
        .map(|item| {
            normalize_pull_request_snapshot(
                item,
                owner,
                repo,
                GITHUB_LIST_PULL_REQUESTS_CAPABILITY_ID,
            )
        })
        .collect()
}

pub(super) fn normalize_pull_request_snapshot(
    value: &JsonValue,
    owner: &str,
    repo: &str,
    capability_id: &str,
) -> Result<GithubPullRequestSnapshot, GithubIssueWorkflowError> {
    Ok(GithubPullRequestSnapshot {
        pull_request: normalize_pull_request_ref_with_capability(
            value,
            owner,
            repo,
            capability_id,
        )?,
        title: optional_string(value, &[&["title"]]).unwrap_or_default(),
        body: optional_string(value, &[&["body"]]).unwrap_or_default(),
        state: optional_string(value, &[&["state"]]).unwrap_or_else(|| "unknown".to_string()),
        draft: optional_bool(value, &[&["draft"]]).unwrap_or(false),
        merged: optional_bool(value, &[&["merged"]])
            .or_else(|| value.get("merged_at").map(|merged_at| !merged_at.is_null()))
            .unwrap_or(false),
        updated_at: optional_rfc3339_datetime(value, &[&["updated_at"]]),
    })
}

pub(super) fn normalize_pull_request_ref(
    value: &JsonValue,
    owner: &str,
    repo: &str,
) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
    normalize_pull_request_ref_with_capability(
        value,
        owner,
        repo,
        GITHUB_CREATE_PULL_REQUEST_CAPABILITY_ID,
    )
}

fn normalize_pull_request_ref_with_capability(
    value: &JsonValue,
    owner: &str,
    repo: &str,
    capability_id: &str,
) -> Result<GithubPullRequestRef, GithubIssueWorkflowError> {
    let number = required_u64(value, &["number"], capability_id)?;
    let head_branch =
        optional_string(value, &[&["head", "ref"], &["head_branch"]]).ok_or_else(|| {
            invalid_output(capability_id, "pull request response is missing head.ref")
        })?;

    Ok(GithubPullRequestRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
        node_id: optional_string(value, &[&["node_id"]]),
        url: optional_string(value, &[&["html_url"], &["url"]])
            .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/pull/{number}")),
        head_branch,
        head_sha: optional_string(value, &[&["head", "sha"], &["head_sha"]]),
    })
}

pub(super) fn normalize_combined_status_checks(
    value: &JsonValue,
    fallback_head_sha: Option<&str>,
    limit: usize,
) -> Result<Vec<GithubPullRequestCheckSnapshot>, GithubIssueWorkflowError> {
    let statuses = match value {
        JsonValue::Array(items) => items,
        _ => required_array(
            value,
            &["statuses"],
            GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
        )?,
    };
    statuses
        .iter()
        .take(limit)
        .map(|status| {
            let suite_or_run_id = optional_u64(status, &[&["id"]])
                .map(|id| id.to_string())
                .or_else(|| optional_string(status, &[&["node_id"], &["context"]]))
                .ok_or_else(|| {
                    invalid_output(
                        GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
                        "combined status item is missing id or context",
                    )
                })?;
            let head_sha = optional_string(status, &[&["sha"]])
                .or_else(|| fallback_head_sha.map(ToString::to_string))
                .ok_or_else(|| {
                    invalid_output(
                        GITHUB_GET_COMBINED_STATUS_CAPABILITY_ID,
                        "combined status item is missing sha",
                    )
                })?;
            let conclusion = optional_string(status, &[&["state"]])
                .map(|state| GithubCheckConclusion::from_provider(&state))
                .unwrap_or(GithubCheckConclusion::Unknown);
            Ok(GithubPullRequestCheckSnapshot {
                suite_or_run_id,
                name: optional_string(status, &[&["context"], &["name"]]).unwrap_or_default(),
                head_sha,
                conclusion,
                completed_at: optional_rfc3339_datetime(
                    status,
                    &[&["updated_at"], &["created_at"]],
                ),
                details_url: optional_string(status, &[&["target_url"], &["url"]]),
            })
        })
        .collect()
}

pub(super) fn normalize_review_comments(
    value: &JsonValue,
) -> Result<Vec<GithubReviewCommentSnapshot>, GithubIssueWorkflowError> {
    let comments = match value {
        JsonValue::Array(items) => items,
        _ => required_array(
            value,
            &["items"],
            GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
        )?,
    };

    comments
        .iter()
        .map(|comment| {
            Ok(GithubReviewCommentSnapshot {
                comment: normalize_comment_ref(
                    comment,
                    None,
                    GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
                )?,
                body: optional_string(comment, &[&["body"]]).unwrap_or_default(),
                author_login: optional_string(comment, &[&["user", "login"], &["author", "login"]])
                    .unwrap_or_default(),
                created_at: required_datetime(
                    comment,
                    &[&["created_at"]],
                    GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
                )?,
                updated_at: required_datetime(
                    comment,
                    &[&["updated_at"], &["created_at"]],
                    GITHUB_LIST_PULL_REQUEST_COMMENTS_CAPABILITY_ID,
                )?,
            })
        })
        .collect()
}

fn issue_like_url(value: &JsonValue, owner: &str, repo: &str, number: u64) -> String {
    optional_string(value, &[&["html_url"], &["url"]])
        .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/issues/{number}"))
}

fn optional_labels(value: &JsonValue) -> Vec<String> {
    value
        .get("labels")
        .and_then(JsonValue::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| match label {
                    JsonValue::String(name) => Some(name.clone()),
                    JsonValue::Object(_) => label
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .map(ToString::to_string),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn required_array<'a>(
    value: &'a JsonValue,
    path: &[&str],
    capability_id: &str,
) -> Result<&'a Vec<JsonValue>, GithubIssueWorkflowError> {
    json_at_path(value, path)
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            invalid_output(
                capability_id,
                &format!("missing array `{}`", path.join(".")),
            )
        })
}

fn required_string<'a>(
    value: &'a JsonValue,
    path: &[&str],
    capability_id: &str,
) -> Result<&'a str, GithubIssueWorkflowError> {
    json_at_path(value, path)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            invalid_output(
                capability_id,
                &format!("missing string `{}`", path.join(".")),
            )
        })
}

fn required_u64(
    value: &JsonValue,
    path: &[&str],
    capability_id: &str,
) -> Result<u64, GithubIssueWorkflowError> {
    json_at_path(value, path)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| {
            invalid_output(
                capability_id,
                &format!("missing integer `{}`", path.join(".")),
            )
        })
}

fn required_datetime(
    value: &JsonValue,
    paths: &[&[&str]],
    capability_id: &str,
) -> Result<DateTime<Utc>, GithubIssueWorkflowError> {
    optional_rfc3339_datetime(value, paths).ok_or_else(|| {
        invalid_output(
            capability_id,
            &format!("missing timestamp `{}`", paths[0].join(".")),
        )
    })
}

fn optional_rfc3339_datetime(value: &JsonValue, paths: &[&[&str]]) -> Option<DateTime<Utc>> {
    optional_string(value, paths).and_then(|timestamp| {
        DateTime::parse_from_rfc3339(&timestamp)
            .ok()
            .map(|parsed| parsed.with_timezone(&Utc))
    })
}

fn optional_bool(value: &JsonValue, paths: &[&[&str]]) -> Option<bool> {
    paths
        .iter()
        .find_map(|path| json_at_path(value, path).and_then(JsonValue::as_bool))
}

fn optional_u64(value: &JsonValue, paths: &[&[&str]]) -> Option<u64> {
    paths
        .iter()
        .find_map(|path| json_at_path(value, path).and_then(JsonValue::as_u64))
}

fn optional_string(value: &JsonValue, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| json_at_path(value, path).and_then(JsonValue::as_str))
        .map(ToString::to_string)
}

fn json_at_path<'a>(value: &'a JsonValue, path: &[&str]) -> Option<&'a JsonValue> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn invalid_output(capability_id: &str, detail: &str) -> GithubIssueWorkflowError {
    GithubIssueWorkflowError::ProviderRead {
        reason: format!("GitHub capability {capability_id} returned invalid output: {detail}"),
    }
}
