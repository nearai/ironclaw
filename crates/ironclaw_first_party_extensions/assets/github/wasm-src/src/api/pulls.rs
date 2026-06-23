use crate::request::github_request;
use crate::types::{MergeMethod, PrReviewEvent};
use crate::validation::*;

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

pub(crate) fn create_pull_request(
    owner: &str,
    repo: &str,
    title: &str,
    head: &str,
    base: &str,
    body: Option<&str>,
    draft: bool,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(title, "title")?;
    validate_input_length(head, "head")?;
    validate_input_length(base, "base")?;
    if let Some(b) = body {
        validate_input_length(b, "body")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/pulls", encoded_owner, encoded_repo);
    let mut req_body = serde_json::json!({
        "title": title,
        "head": head,
        "base": base,
        "draft": draft,
    });
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }
    github_request("POST", &path, Some(req_body.to_string()))
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
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(body, "body")?;

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
    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn list_pull_request_comments(
    owner: &str,
    repo: &str,
    pr_number: u32,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/repos/{}/{}/pulls/{}/comments?per_page={}",
        encoded_owner, encoded_repo, pr_number, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
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
