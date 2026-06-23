use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

use crate::request::github_request;
use crate::types::{GitCommitIdentity, MergeMethod, PrReviewEvent};
use crate::validation::*;

pub(crate) fn get_repo(owner: &str, repo: &str) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    github_request(
        "GET",
        &format!("/repos/{}/{}", encoded_owner, encoded_repo),
        None,
    )
}

pub(crate) fn create_repo(
    name: &str,
    description: Option<&str>,
    private: bool,
    auto_init: bool,
    gitignore_template: Option<&str>,
    license_template: Option<&str>,
    org: Option<&str>,
) -> Result<String, String> {
    if !validate_path_segment(name) {
        return Err("Invalid repository name".into());
    }
    validate_input_length(name, "name")?;
    if let Some(description) = description {
        validate_input_length(description, "description")?;
    }
    if let Some(template) = gitignore_template {
        validate_input_length(template, "gitignore_template")?;
    }
    if let Some(template) = license_template {
        validate_input_length(template, "license_template")?;
    }
    if let Some(org) = org {
        if !validate_path_segment(org) {
            return Err("Invalid org name".into());
        }
    }

    let path = if let Some(org) = org {
        format!("/orgs/{}/repos", url_encode_path(org))
    } else {
        "/user/repos".to_string()
    };

    let mut req_body = serde_json::json!({
        "name": name,
        "private": private,
        "auto_init": auto_init,
    });
    if let Some(description) = description {
        req_body["description"] = serde_json::json!(description);
    }
    if let Some(template) = gitignore_template {
        req_body["gitignore_template"] = serde_json::json!(template);
    }
    if let Some(template) = license_template {
        req_body["license_template"] = serde_json::json!(template);
    }

    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn fork_repo(
    owner: &str,
    repo: &str,
    organization: Option<&str>,
    name: Option<&str>,
    default_branch_only: Option<bool>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(owner, "owner")?;
    validate_input_length(repo, "repo")?;
    if let Some(org) = organization {
        validate_input_length(org, "organization")?;
        if !validate_path_segment(org) {
            return Err("Invalid org name".into());
        }
    }
    if let Some(n) = name {
        validate_input_length(n, "name")?;
        if !validate_path_segment(n) {
            return Err("Invalid fork name".into());
        }
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/forks", encoded_owner, encoded_repo);

    let mut req_body = serde_json::json!({});
    if let Some(org) = organization {
        req_body["organization"] = serde_json::json!(org);
    }
    if let Some(n) = name {
        req_body["name"] = serde_json::json!(n);
    }
    if let Some(only) = default_branch_only {
        req_body["default_branch_only"] = serde_json::json!(only);
    }

    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn list_issues(
    owner: &str,
    repo: &str,
    state: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let state = state.unwrap_or("open");
    let search_state = match state {
        "open" | "closed" => Some(state),
        "all" => None,
        _ => return Err("invalid_state".to_string()),
    };
    validate_search_page(page)?;
    validate_search_limit(limit)?;
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let query = build_issue_search_query(
        None,
        None,
        Some(owner),
        Some(repo),
        None,
        None,
        None,
        search_state,
        Some("issue"),
    )?;

    let mut path = format!(
        "/search/issues?q={}&per_page={}",
        url_encode_query(&query),
        limit
    );
    append_search_params(&mut path, page, Some("created"), Some("desc"))?;

    let response = github_request("GET", &path, None)?;
    issue_items_from_search_response(&response)
}

pub(crate) fn issue_items_from_search_response(response: &str) -> Result<String, String> {
    let response: serde_json::Value = serde_json::from_str(response).map_err(|error| {
        format!("github_api_invalid_json: failed to parse issue search response: {error}")
    })?;
    let items = response
        .get("items")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            "github_api_invalid_json: issue search response missing items array".to_string()
        })?;
    serde_json::to_string(items).map_err(|error| {
        format!("github_api_invalid_json: failed to serialize issue search items: {error}")
    })
}

pub(crate) fn create_issue(
    owner: &str,
    repo: &str,
    title: &str,
    body: Option<&str>,
    labels: Option<Vec<String>>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(title, "title")?;
    if let Some(b) = body {
        validate_input_length(b, "body")?;
    }
    if let Some(labels) = &labels {
        if labels.len() > 100 {
            return Err("Invalid labels: at most 100 labels are allowed".into());
        }
        for label in labels {
            if label.is_empty() {
                return Err("Invalid labels: labels cannot be empty".into());
            }
            validate_input_length(label, "labels")?;
            if label.chars().count() > 100 {
                return Err(
                    "Invalid labels: label exceeds maximum length of 100 characters".into(),
                );
            }
        }
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/issues", encoded_owner, encoded_repo);
    let mut req_body = serde_json::json!({
        "title": title,
    });
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }
    if let Some(labels) = labels {
        req_body["labels"] = serde_json::json!(labels);
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn get_issue(owner: &str, repo: &str, issue_number: u32) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    github_request(
        "GET",
        &format!(
            "/repos/{}/{}/issues/{}",
            encoded_owner, encoded_repo, issue_number
        ),
        None,
    )
}

pub(crate) fn list_issue_comments(
    owner: &str,
    repo: &str,
    issue_number: u32,
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
        "/repos/{}/{}/issues/{}/comments?per_page={}",
        encoded_owner, encoded_repo, issue_number, limit
    );
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

pub(crate) fn create_issue_comment(
    owner: &str,
    repo: &str,
    issue_number: u32,
    body: &str,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(body, "body")?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/issues/{}/comments",
        encoded_owner, encoded_repo, issue_number
    );
    let req_body = serde_json::json!({ "body": body });
    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn list_pull_requests(
    owner: &str,
    repo: &str,
    state: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let state = state.unwrap_or("open");
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let encoded_state = url_encode_query(state);

    let mut path = format!(
        "/repos/{}/{}/pulls?state={}&per_page={}",
        encoded_owner, encoded_repo, encoded_state, limit
    );
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
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    github_request(
        "GET",
        &format!(
            "/repos/{}/{}/pulls/{}/files",
            encoded_owner, encoded_repo, pr_number
        ),
        None,
    )
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
    github_request("PUT", &path, Some(req_body.to_string()))
}

pub(crate) fn get_authenticated_user() -> Result<String, String> {
    github_request("GET", "/user", None)
}

pub(crate) fn list_repos(
    username: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let mut path = match username.map(str::trim).filter(|value| !value.is_empty()) {
        Some(username) if !is_authenticated_user_alias(username) => {
            if !validate_path_segment(username) {
                return Err("Invalid username".into());
            }
            let encoded_username = url_encode_path(username);
            format!("/users/{}/repos?per_page={}", encoded_username, limit)
        }
        _ => format!("/user/repos?per_page={}", limit),
    };
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

fn is_authenticated_user_alias(username: &str) -> bool {
    username.eq_ignore_ascii_case("me") || username.eq_ignore_ascii_case("@me")
}

pub(crate) fn search_repositories(
    query: &str,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    validate_input_length(query, "query")?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/repositories?q={}&per_page={}",
        url_encode_query(query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}

pub(crate) fn search_code(
    query: &str,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    validate_input_length(query, "query")?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/code?q={}&per_page={}",
        url_encode_query(query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_issues_pull_requests(
    query: Option<&str>,
    repository: Option<&str>,
    owner: Option<&str>,
    repo: Option<&str>,
    author: Option<&str>,
    assignee: Option<&str>,
    involves: Option<&str>,
    state: Option<&str>,
    issue_type: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<String, String> {
    let query = build_issue_search_query(
        query, repository, owner, repo, author, assignee, involves, state, issue_type,
    )?;
    validate_search_page(page)?;
    validate_search_limit(limit)?;
    validate_search_sort(sort)?;
    let limit = limit.unwrap_or(30).min(100);
    let mut path = format!(
        "/search/issues?q={}&per_page={}",
        url_encode_query(&query),
        limit
    );
    append_search_params(&mut path, page, sort, order)?;
    github_request("GET", &path, None)
}

pub(crate) fn list_branches(
    owner: &str,
    repo: &str,
    protected: Option<bool>,
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
        "/repos/{}/{}/branches?per_page={}",
        encoded_owner, encoded_repo, limit
    );
    if let Some(protected) = protected {
        path.push_str("&protected=");
        path.push_str(if protected { "true" } else { "false" });
    }
    if let Some(page) = page {
        path.push_str(&format!("&page={page}"));
    }
    github_request("GET", &path, None)
}

pub(crate) fn create_branch(
    owner: &str,
    repo: &str,
    branch: &str,
    from_ref: &str,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(branch, "branch")?;
    validate_input_length(from_ref, "from_ref")?;

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let source_ref = normalize_ref_lookup(from_ref)?;
    let source_path = format!(
        "/repos/{}/{}/git/ref/{}",
        encoded_owner,
        encoded_repo,
        encode_repo_path(&source_ref)
    );
    let source_ref_resp = github_request("GET", &source_path, None)?;
    let source_ref_json: serde_json::Value = serde_json::from_str(&source_ref_resp)
        .map_err(|e| format!("Invalid GitHub response for source ref: {e}"))?;
    let sha = source_ref_json
        .pointer("/object/sha")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Source ref response missing object.sha".to_string())?;

    let req_body = serde_json::json!({
        "ref": normalize_branch_ref(branch)?,
        "sha": sha,
    });
    let path = format!("/repos/{}/{}/git/refs", encoded_owner, encoded_repo);
    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn get_file_content(
    owner: &str,
    repo: &str,
    path: &str,
    r#ref: Option<&str>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_repo_path(path)?;
    // Validate ref if provided
    if let Some(r#ref) = r#ref {
        validate_git_ref(r#ref, "ref")?;
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_path = encode_repo_path(path);

    let url_path = if let Some(r#ref) = r#ref {
        let encoded_ref = url_encode_query(r#ref);
        format!(
            "/repos/{}/{}/contents/{}?ref={}",
            encoded_owner, encoded_repo, encoded_path, encoded_ref
        )
    } else {
        format!(
            "/repos/{}/{}/contents/{}",
            encoded_owner, encoded_repo, encoded_path
        )
    };
    github_request("GET", &url_path, None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_or_update_file(
    owner: &str,
    repo: &str,
    path: &str,
    message: &str,
    content: &str,
    sha: Option<&str>,
    branch: Option<&str>,
    committer: Option<GitCommitIdentity>,
    author: Option<GitCommitIdentity>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_repo_path(path)?;
    validate_input_length(message, "message")?;
    validate_input_length(content, "content")?;
    if let Some(branch) = branch {
        validate_git_ref(branch, "branch")?;
    }
    if let Some(sha) = sha {
        validate_input_length(sha, "sha")?;
    }
    if let Some(committer) = &committer {
        validate_commit_identity(committer, "committer")?;
    }
    if let Some(author) = &author {
        validate_commit_identity(author, "author")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_path = encode_repo_path(path);
    let mut req_body = serde_json::json!({
        "message": message,
        "content": BASE64_STANDARD.encode(content.as_bytes()),
    });
    if let Some(sha) = sha {
        req_body["sha"] = serde_json::json!(sha);
    }
    if let Some(branch) = branch {
        req_body["branch"] = serde_json::json!(branch);
    }
    if let Some(committer) = committer {
        req_body["committer"] =
            serde_json::to_value(committer).map_err(|e| format!("Invalid committer: {e}"))?;
    }
    if let Some(author) = author {
        req_body["author"] =
            serde_json::to_value(author).map_err(|e| format!("Invalid author: {e}"))?;
    }

    let path = format!(
        "/repos/{}/{}/contents/{}",
        encoded_owner, encoded_repo, encoded_path
    );
    github_request("PUT", &path, Some(req_body.to_string()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn delete_file(
    owner: &str,
    repo: &str,
    path: &str,
    message: &str,
    sha: &str,
    branch: Option<&str>,
    committer: Option<GitCommitIdentity>,
    author: Option<GitCommitIdentity>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_repo_path(path)?;
    validate_input_length(message, "message")?;
    validate_input_length(sha, "sha")?;
    if let Some(branch) = branch {
        validate_git_ref(branch, "branch")?;
    }
    if let Some(committer) = &committer {
        validate_commit_identity(committer, "committer")?;
    }
    if let Some(author) = &author {
        validate_commit_identity(author, "author")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_path = encode_repo_path(path);
    let mut req_body = serde_json::json!({
        "message": message,
        "sha": sha,
    });
    if let Some(branch) = branch {
        req_body["branch"] = serde_json::json!(branch);
    }
    if let Some(committer) = committer {
        req_body["committer"] =
            serde_json::to_value(committer).map_err(|e| format!("Invalid committer: {e}"))?;
    }
    if let Some(author) = author {
        req_body["author"] =
            serde_json::to_value(author).map_err(|e| format!("Invalid author: {e}"))?;
    }

    let path = format!(
        "/repos/{}/{}/contents/{}",
        encoded_owner, encoded_repo, encoded_path
    );
    github_request("DELETE", &path, Some(req_body.to_string()))
}

pub(crate) fn list_releases(
    owner: &str,
    repo: &str,
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
        "/repos/{}/{}/releases?per_page={}",
        encoded_owner, encoded_repo, limit
    );
    if let Some(page) = page {
        path.push_str(&format!("&page={page}"));
    }
    github_request("GET", &path, None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_release(
    owner: &str,
    repo: &str,
    tag_name: &str,
    target_commitish: Option<&str>,
    name: Option<&str>,
    body: Option<&str>,
    draft: bool,
    prerelease: bool,
    generate_release_notes: bool,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_git_ref(tag_name, "tag_name")?;
    if let Some(target_commitish) = target_commitish {
        validate_git_ref(target_commitish, "target_commitish")?;
    }
    if let Some(name) = name {
        validate_input_length(name, "name")?;
    }
    if let Some(body) = body {
        validate_input_length(body, "body")?;
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{}/{}/releases", encoded_owner, encoded_repo);
    let mut req_body = serde_json::json!({
        "tag_name": tag_name,
        "draft": draft,
        "prerelease": prerelease,
        "generate_release_notes": generate_release_notes,
    });
    if let Some(target_commitish) = target_commitish {
        req_body["target_commitish"] = serde_json::json!(target_commitish);
    }
    if let Some(name) = name {
        req_body["name"] = serde_json::json!(name);
    }
    if let Some(body) = body {
        req_body["body"] = serde_json::json!(body);
    }

    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn trigger_workflow(
    owner: &str,
    repo: &str,
    workflow_id: &str,
    r#ref: &str,
    inputs: Option<serde_json::Value>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    // Validate inputs size if present
    if let Some(valid_inputs) = &inputs {
        let inputs_str = valid_inputs.to_string();
        validate_input_length(&inputs_str, "inputs")?;
    }

    // Validate workflow_id - must be a safe filename
    if workflow_id.contains('/') || workflow_id.contains("..") || workflow_id.contains(':') {
        return Err("Invalid workflow_id: must be a filename or numeric ID".into());
    }
    validate_git_ref(r#ref, "ref")?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_workflow_id = url_encode_path(workflow_id);
    let path = format!(
        "/repos/{}/{}/actions/workflows/{}/dispatches",
        encoded_owner, encoded_repo, encoded_workflow_id
    );
    let mut req_body = serde_json::json!({
        "ref": r#ref,
    });
    if let Some(inputs) = inputs {
        req_body["inputs"] = inputs;
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

pub(crate) fn get_workflow_runs(
    owner: &str,
    repo: &str,
    workflow_id: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    // Validate workflow_id if provided
    if let Some(wid) = workflow_id {
        if wid.contains('/') || wid.contains("..") || wid.contains(':') {
            return Err("Invalid workflow_id: must be a filename or numeric ID".into());
        }
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let mut path = if let Some(workflow_id) = workflow_id {
        let encoded_workflow_id = url_encode_path(workflow_id);
        format!(
            "/repos/{}/{}/actions/workflows/{}/runs?per_page={}",
            encoded_owner, encoded_repo, encoded_workflow_id, limit
        )
    } else {
        format!(
            "/repos/{}/{}/actions/runs?per_page={}",
            encoded_owner, encoded_repo, limit
        )
    };
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}
