use crate::request::github_request;
use crate::validation::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn list_issues(
    owner: &str,
    repo: &str,
    state: Option<&str>,
    labels: Option<Vec<String>>,
    assignee: Option<&str>,
    milestone: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let state = state.unwrap_or("open");
    match state {
        "open" | "closed" | "all" => {}
        _ => return Err("invalid_state".to_string()),
    }
    validate_page(page)?;
    validate_limit(limit)?;
    validate_name_list(labels.as_deref(), "labels")?;
    if let Some(assignee) = assignee {
        validate_input_length(assignee, "assignee")?;
    }
    if let Some(milestone) = milestone {
        validate_milestone_filter(milestone)?;
    }
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);

    let mut path = format!(
        "/repos/{}/{}/issues?state={}&per_page={}",
        encoded_owner,
        encoded_repo,
        url_encode_query(state),
        limit
    );
    if let Some(labels) = labels {
        path.push_str("&labels=");
        path.push_str(&url_encode_query(&labels.join(",")));
    }
    if let Some(assignee) = assignee {
        path.push_str("&assignee=");
        path.push_str(&url_encode_query(assignee));
    }
    if let Some(milestone) = milestone {
        path.push_str("&milestone=");
        path.push_str(&url_encode_query(milestone));
    }
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

fn validate_milestone_filter(milestone: &str) -> Result<(), String> {
    validate_input_length(milestone, "milestone")?;
    if milestone == "none" || milestone == "*" || milestone.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(());
    }
    Err("invalid_milestone".to_string())
}

pub(crate) fn create_issue(
    owner: &str,
    repo: &str,
    title: &str,
    body: Option<&str>,
    labels: Option<Vec<String>>,
    assignees: Option<Vec<String>>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_input_length(title, "title")?;
    if let Some(b) = body {
        validate_input_length(b, "body")?;
    }
    validate_name_list(labels.as_deref(), "labels")?;
    validate_name_list(assignees.as_deref(), "assignees")?;

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
    if let Some(assignees) = assignees {
        req_body["assignees"] = serde_json::json!(assignees);
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

fn validate_name_list(values: Option<&[String]>, field_name: &str) -> Result<(), String> {
    let Some(values) = values else {
        return Ok(());
    };
    if values.len() > 100 {
        return Err(format!(
            "Invalid {field_name}: at most 100 values are allowed"
        ));
    }
    for value in values {
        if value.is_empty() {
            return Err(format!("Invalid {field_name}: values cannot be empty"));
        }
        validate_input_length(value, field_name)?;
        if value.chars().count() > 100 {
            return Err(format!(
                "Invalid {field_name}: value exceeds maximum length of 100 characters"
            ));
        }
    }
    Ok(())
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
