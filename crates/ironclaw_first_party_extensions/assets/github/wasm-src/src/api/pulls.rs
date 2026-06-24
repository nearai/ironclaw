use crate::request::github_request;
use crate::types::{
    Direction, MergeMethod, PrReviewCommentInput, PrReviewEvent, PullRequestCommentSort,
    PullRequestState,
};
use crate::validation::*;

// arch-exempt: too_many_args, pull listing keeps GitHub filters separate for callers, plan #5171
#[allow(clippy::too_many_arguments)]
pub(crate) fn list_pull_requests(
    owner: &str,
    repo: &str,
    state: Option<&str>,
    head: Option<&str>,
    base: Option<&str>,
    sort: Option<&str>,
    direction: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let state = state.unwrap_or("open");
    match state {
        "open" | "closed" | "all" => {}
        _ => return Err("invalid_state".to_string()),
    }
    validate_pull_request_sort(sort)?;
    validate_direction(direction)?;
    if let Some(head) = head {
        validate_input_length(head, "head")?;
    }
    if let Some(base) = base {
        validate_input_length(base, "base")?;
    }
    validate_page(page)?;
    validate_limit(limit)?;
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let encoded_state = url_encode_query(state);

    let mut path = format!(
        "/repos/{}/{}/pulls?state={}&per_page={}",
        encoded_owner, encoded_repo, encoded_state, limit
    );
    if let Some(head) = head {
        path.push_str("&head=");
        path.push_str(&url_encode_query(head));
    }
    if let Some(base) = base {
        path.push_str("&base=");
        path.push_str(&url_encode_query(base));
    }
    if let Some(sort) = sort {
        path.push_str("&sort=");
        path.push_str(sort);
    }
    if let Some(direction) = direction {
        path.push_str("&direction=");
        path.push_str(direction);
    }
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }

    github_request("GET", &path, None)
}

// arch-exempt: too_many_args, pull create mirrors GitHub's create-pr payload fields, plan #5171
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_pull_request(
    owner: &str,
    repo: &str,
    title: Option<&str>,
    head: &str,
    base: &str,
    body: Option<&str>,
    issue: Option<u32>,
    head_repo: Option<&str>,
    maintainer_can_modify: Option<bool>,
    draft: bool,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    if issue.is_none() && title.is_none_or(|value| value.is_empty()) {
        return Err("invalid_parameters".to_string());
    }
    if let Some(title) = title {
        validate_input_length(title, "title")?;
    }
    validate_input_length(head, "head")?;
    validate_input_length(base, "base")?;
    if let Some(b) = body {
        validate_input_length(b, "body")?;
    }
    if let Some(head_repo) = head_repo {
        if !validate_path_segment(head_repo) {
            return Err("Invalid repository name".into());
        }
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/pulls", encoded_owner, encoded_repo);
    let mut req_body = serde_json::json!({
        "head": head,
        "base": base,
        "draft": draft,
    });
    if let Some(title) = title {
        req_body["title"] = serde_json::json!(title);
    }
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }
    if let Some(issue) = issue {
        req_body["issue"] = serde_json::json!(issue);
    }
    if let Some(head_repo) = head_repo {
        req_body["head_repo"] = serde_json::json!(head_repo);
    }
    if let Some(maintainer_can_modify) = maintainer_can_modify {
        req_body["maintainer_can_modify"] = serde_json::json!(maintainer_can_modify);
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

// arch-exempt: too_many_args, pull update exposes GitHub's patchable fields directly, plan #5171
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_pull_request(
    owner: &str,
    repo: &str,
    pr_number: u32,
    title: Option<&str>,
    body: Option<&str>,
    state: Option<PullRequestState>,
    base: Option<&str>,
    maintainer_can_modify: Option<bool>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    if let Some(title) = title {
        validate_input_length(title, "title")?;
    }
    if let Some(body) = body {
        validate_input_length(body, "body")?;
    }
    if let Some(base) = base {
        validate_input_length(base, "base")?;
    }

    let mut req_body = serde_json::json!({});
    if let Some(title) = title {
        req_body["title"] = serde_json::json!(title);
    }
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }
    if let Some(state) = state {
        req_body["state"] = serde_json::json!(state.as_str());
    }
    if let Some(base) = base {
        req_body["base"] = serde_json::json!(base);
    }
    if let Some(maintainer_can_modify) = maintainer_can_modify {
        req_body["maintainer_can_modify"] = serde_json::json!(maintainer_can_modify);
    }
    if req_body.as_object().is_some_and(|body| body.is_empty()) {
        return Err("invalid_parameters".to_string());
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/pulls/{}",
        encoded_owner, encoded_repo, pr_number
    );
    github_request("PATCH", &path, Some(req_body.to_string()))
}

pub(crate) fn get_pull_request(owner: &str, repo: &str, pr_number: u32) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    github_request(
        "GET",
        &format!(
            "/repos/{}/{}/pulls/{}",
            encoded_owner, encoded_repo, pr_number
        ),
        None,
    )
}

pub(crate) fn get_pull_request_files(
    owner: &str,
    repo: &str,
    pr_number: u32,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_page(page)?;
    validate_limit(limit)?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/pulls/{}/files?per_page={}",
        encoded_owner, encoded_repo, pr_number, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

pub(crate) fn create_pr_review(
    owner: &str,
    repo: &str,
    pr_number: u32,
    body: &str,
    event: PrReviewEvent,
    commit_id: Option<&str>,
    comments: Option<Vec<PrReviewCommentInput>>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(body, "body")?;
    if let Some(commit_id) = commit_id {
        validate_input_length(commit_id, "commit_id")?;
    }
    if let Some(comments) = comments.as_deref() {
        validate_review_comments(comments)?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/pulls/{}/reviews",
        encoded_owner, encoded_repo, pr_number
    );
    let req_body = serde_json::json!({
        "body": body,
        "event": event.as_str(),
    });
    let mut req_body = req_body;
    if let Some(commit_id) = commit_id {
        req_body["commit_id"] = serde_json::json!(commit_id);
    }
    if let Some(comments) = comments {
        req_body["comments"] =
            serde_json::to_value(comments).map_err(|e| format!("invalid_comments: {e}"))?;
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

// arch-exempt: too_many_args, pull comment listing keeps sort and paging parameters explicit, plan #5171
#[allow(clippy::too_many_arguments)]
pub(crate) fn list_pull_request_comments(
    owner: &str,
    repo: &str,
    pr_number: u32,
    sort: Option<PullRequestCommentSort>,
    direction: Option<Direction>,
    since: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_page(page)?;
    validate_limit(limit)?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/pulls/{}/comments?per_page={}",
        encoded_owner, encoded_repo, pr_number, limit
    );
    if let Some(sort) = sort {
        path.push_str("&sort=");
        path.push_str(sort.as_str());
    }
    if let Some(direction) = direction {
        path.push_str("&direction=");
        path.push_str(direction.as_str());
    }
    if let Some(since) = since {
        validate_input_length(since, "since")?;
        path.push_str("&since=");
        path.push_str(&url_encode_query(since));
    }
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

fn validate_review_comments(comments: &[PrReviewCommentInput]) -> Result<(), String> {
    if comments.len() > 100 {
        return Err("invalid_comments".to_string());
    }
    for comment in comments {
        validate_repo_path(&comment.path)?;
        validate_input_length(&comment.body, "body")?;
        if comment.body.is_empty() {
            return Err("invalid_comments".to_string());
        }
        let has_position = comment.position.is_some();
        let has_line = comment.line.is_some();
        let has_side = comment.side.is_some();
        let has_start_line = comment.start_line.is_some();
        let has_start_side = comment.start_side.is_some();
        if has_position {
            if has_line || has_side || has_start_line || has_start_side {
                return Err("invalid_comments".to_string());
            }
        } else {
            if !(has_line && has_side) {
                return Err("invalid_comments".to_string());
            }
            if has_start_line ^ has_start_side {
                return Err("invalid_comments".to_string());
            }
        }
    }
    Ok(())
}

pub(crate) fn reply_pull_request_comment(
    owner: &str,
    repo: &str,
    pr_number: u32,
    comment_id: u64,
    body: &str,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(body, "body")?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/pulls/{}/comments/{}/replies",
        encoded_owner, encoded_repo, pr_number, comment_id
    );
    let req_body = serde_json::json!({ "body": body });
    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn get_pull_request_reviews(
    owner: &str,
    repo: &str,
    pr_number: u32,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_page(page)?;
    validate_limit(limit)?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/pulls/{}/reviews?per_page={}",
        encoded_owner, encoded_repo, pr_number, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

pub(crate) fn get_combined_status(owner: &str, repo: &str, r#ref: &str) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(r#ref, "ref")?;
    validate_git_ref(r#ref, "ref")?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_ref = url_encode_path(r#ref);
    let path = format!(
        "/repos/{}/{}/commits/{}/status",
        encoded_owner, encoded_repo, encoded_ref
    );
    github_request("GET", &path, None)
}

pub(crate) fn merge_pull_request(
    owner: &str,
    repo: &str,
    pr_number: u32,
    commit_title: Option<&str>,
    commit_message: Option<&str>,
    merge_method: Option<MergeMethod>,
    sha: Option<&str>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    if let Some(v) = commit_title {
        validate_input_length(v, "commit_title")?;
    }
    if let Some(v) = commit_message {
        validate_input_length(v, "commit_message")?;
    }
    if let Some(v) = sha {
        validate_input_length(v, "sha")?;
    }
    let method = merge_method.unwrap_or(MergeMethod::Merge);

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/pulls/{}/merge",
        encoded_owner, encoded_repo, pr_number
    );
    let mut req_body = serde_json::json!({
        "merge_method": method.as_str(),
    });
    if let Some(v) = commit_title {
        req_body["commit_title"] = serde_json::json!(v);
    }
    if let Some(v) = commit_message {
        req_body["commit_message"] = serde_json::json!(v);
    }
    if let Some(v) = sha {
        req_body["sha"] = serde_json::json!(v);
    }
    github_request("PUT", &path, Some(req_body.to_string()))
}
