use crate::request::github_request;
use crate::types::IssueState;
use crate::validation::*;

const MAX_ASSIGNEES_PER_REQUEST: usize = 10;

// arch-exempt: too_many_args, issue listing keeps GitHub filter args explicit, plan #5171
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
        if !labels.is_empty() {
            path.push_str("&labels=");
            path.push_str(&url_encode_query(&labels.join(",")));
        }
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
    let response = github_request("GET", &path, None)?;
    filter_pull_requests_from_issues_response(&response)
}

fn filter_pull_requests_from_issues_response(response: &str) -> Result<String, String> {
    let issues: Vec<serde_json::Value> =
        serde_json::from_str(response).map_err(|_| "github_api_invalid_json".to_string())?;
    let issues = issues
        .into_iter()
        .filter(|issue| issue.get("pull_request").is_none())
        .collect::<Vec<_>>();
    serde_json::to_string(&issues).map_err(|_| "github_api_invalid_json".to_string())
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
    milestone: Option<u32>,
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
    if let Some(milestone) = milestone {
        validate_positive_number(milestone, "milestone")?;
        req_body["milestone"] = serde_json::json!(milestone);
    }
    if let Some(labels) = labels {
        req_body["labels"] = serde_json::json!(labels);
    }
    if let Some(assignees) = assignees {
        req_body["assignees"] = serde_json::json!(assignees);
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

// arch-exempt: too_many_args, issue update mirrors GitHub's patchable fields, plan #5171
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_issue(
    owner: &str,
    repo: &str,
    issue_number: u32,
    title: Option<&str>,
    body: Option<Option<&str>>,
    state: Option<IssueState>,
    milestone: Option<Option<u32>>,
    labels: Option<Vec<String>>,
    assignees: Option<Vec<String>>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    if let Some(title) = title {
        validate_input_length(title, "title")?;
    }
    if let Some(Some(body)) = body {
        validate_input_length(body, "body")?;
    }
    validate_name_list(labels.as_deref(), "labels")?;
    validate_name_list(assignees.as_deref(), "assignees")?;
    if let Some(Some(milestone)) = milestone {
        validate_positive_number(milestone, "milestone")?;
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
    if let Some(milestone) = milestone {
        req_body["milestone"] = serde_json::json!(milestone);
    }
    if let Some(labels) = labels {
        req_body["labels"] = serde_json::json!(labels);
    }
    if let Some(assignees) = assignees {
        req_body["assignees"] = serde_json::json!(assignees);
    }
    if req_body.as_object().is_some_and(|body| body.is_empty()) {
        return Err("invalid_parameters".to_string());
    }

    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/issues/{}",
        encoded_owner, encoded_repo, issue_number
    );
    github_request("PATCH", &path, Some(req_body.to_string()))
}

pub(crate) fn add_issue_labels(
    owner: &str,
    repo: &str,
    issue_number: u32,
    labels: Vec<String>,
) -> Result<String, String> {
    issue_name_list_request(
        "POST",
        owner,
        repo,
        issue_number,
        "labels",
        labels,
        "labels",
    )
}

pub(crate) fn remove_issue_label(
    owner: &str,
    repo: &str,
    issue_number: u32,
    name: &str,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_label_name(name)?;
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let encoded_name = url_encode_path(name);
    let path = format!(
        "/repos/{}/{}/issues/{}/labels/{}",
        encoded_owner, encoded_repo, issue_number, encoded_name
    );
    github_request("DELETE", &path, None)
}

pub(crate) fn add_issue_assignees(
    owner: &str,
    repo: &str,
    issue_number: u32,
    assignees: Vec<String>,
) -> Result<String, String> {
    validate_assignee_request_limit(&assignees)?;
    issue_name_list_request(
        "POST",
        owner,
        repo,
        issue_number,
        "assignees",
        assignees,
        "assignees",
    )
}

fn validate_assignee_request_limit(assignees: &[String]) -> Result<(), String> {
    if assignees.len() > MAX_ASSIGNEES_PER_REQUEST {
        return Err(format!(
            "Invalid assignees: at most {MAX_ASSIGNEES_PER_REQUEST} values are allowed"
        ));
    }
    Ok(())
}

pub(crate) fn remove_issue_assignees(
    owner: &str,
    repo: &str,
    issue_number: u32,
    assignees: Vec<String>,
) -> Result<String, String> {
    issue_name_list_request(
        "DELETE",
        owner,
        repo,
        issue_number,
        "assignees",
        assignees,
        "assignees",
    )
}

fn issue_name_list_request(
    method: &str,
    owner: &str,
    repo: &str,
    issue_number: u32,
    endpoint: &str,
    values: Vec<String>,
    field_name: &str,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_name_list(Some(values.as_slice()), field_name)?;
    if values.is_empty() {
        return Err(format!("Invalid {field_name}: values cannot be empty"));
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!(
        "/repos/{}/{}/issues/{}/{}",
        encoded_owner, encoded_repo, issue_number, endpoint
    );
    let mut req_body = serde_json::Map::new();
    req_body.insert(field_name.to_string(), serde_json::json!(values));
    let req_body = serde_json::Value::Object(req_body);
    github_request(method, &path, Some(req_body.to_string()))
}

fn validate_positive_number(value: u32, field_name: &str) -> Result<(), String> {
    if value == 0 {
        return Err(format!("invalid_{field_name}"));
    }
    Ok(())
}

fn validate_label_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("invalid_label".to_string());
    }
    validate_input_length(name, "label")?;
    if name.chars().count() > 100 {
        return Err("invalid_label".to_string());
    }
    Ok(())
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
    validate_page(page)?;
    validate_limit(limit)?;
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
