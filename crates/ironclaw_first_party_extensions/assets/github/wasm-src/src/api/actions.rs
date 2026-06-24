use crate::request::github_request;
use crate::types::{Direction, WorkflowJobFilter, WorkflowRunStatus};
use crate::validation::*;

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

// arch-exempt: too_many_args, action-run query fans out across many optional filters, plan #5171
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_workflow_runs(
    owner: &str,
    repo: &str,
    workflow_id: Option<&str>,
    actor: Option<&str>,
    branch: Option<&str>,
    event: Option<&str>,
    status: Option<WorkflowRunStatus>,
    created: Option<&str>,
    exclude_pull_requests: Option<bool>,
    check_suite_id: Option<u64>,
    head_sha: Option<&str>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    validate_page(page)?;
    validate_limit(limit)?;
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
    append_optional_query_str(&mut path, "actor", actor)?;
    append_optional_query_str(&mut path, "branch", branch)?;
    append_optional_query_str(&mut path, "event", event)?;
    if let Some(status) = status {
        append_query_pair(&mut path, "status", status.as_str());
    }
    append_optional_query_str(&mut path, "created", created)?;
    if let Some(exclude_pull_requests) = exclude_pull_requests {
        append_query_pair(
            &mut path,
            "exclude_pull_requests",
            if exclude_pull_requests {
                "true"
            } else {
                "false"
            },
        );
    }
    if let Some(check_suite_id) = check_suite_id {
        append_query_pair(&mut path, "check_suite_id", &check_suite_id.to_string());
    }
    append_optional_query_str(&mut path, "head_sha", head_sha)?;
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
    github_request("GET", &path, None)
}

pub(crate) fn get_workflow_run_jobs(
    owner: &str,
    repo: &str,
    run_id: u64,
    filter: Option<WorkflowJobFilter>,
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
        "/repos/{}/{}/actions/runs/{}/jobs?per_page={}",
        encoded_owner, encoded_repo, run_id, limit
    );
    if let Some(filter) = filter {
        append_query_pair(&mut path, "filter", filter.as_str());
    }
    if let Some(page) = page {
        path.push_str(&format!("&page={page}"));
    }
    github_request("GET", &path, None)
}

pub(crate) fn get_workflow_run_artifacts(
    owner: &str,
    repo: &str,
    run_id: u64,
    name: Option<&str>,
    direction: Option<Direction>,
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
        "/repos/{}/{}/actions/runs/{}/artifacts?per_page={}",
        encoded_owner, encoded_repo, run_id, limit
    );
    append_optional_query_str(&mut path, "name", name)?;
    if let Some(direction) = direction {
        append_query_pair(&mut path, "direction", direction.as_str());
    }
    if let Some(page) = page {
        path.push_str(&format!("&page={page}"));
    }
    github_request("GET", &path, None)
}

pub(crate) fn rerun_failed_workflow_run_jobs(
    owner: &str,
    repo: &str,
    run_id: u64,
    enable_debug_logging: Option<bool>,
) -> Result<String, String> {
    workflow_rerun_request(
        owner,
        repo,
        &format!("/actions/runs/{run_id}/rerun-failed-jobs"),
        enable_debug_logging,
        None,
    )
}

pub(crate) fn rerun_workflow_job(
    owner: &str,
    repo: &str,
    job_id: u64,
    enable_debug_logging: Option<bool>,
    enable_debugger: Option<bool>,
) -> Result<String, String> {
    workflow_rerun_request(
        owner,
        repo,
        &format!("/actions/jobs/{job_id}/rerun"),
        enable_debug_logging,
        enable_debugger,
    )
}

fn workflow_rerun_request(
    owner: &str,
    repo: &str,
    suffix: &str,
    enable_debug_logging: Option<bool>,
    enable_debugger: Option<bool>,
) -> Result<String, String> {
    if !validate_path_segment(owner) || !validate_path_segment(repo) {
        return Err("Invalid owner or repo name".into());
    }
    let encoded_owner = url_encode_path(owner);
    let encoded_repo = url_encode_path(repo);
    let path = format!("/repos/{encoded_owner}/{encoded_repo}{suffix}");
    let mut req_body = serde_json::json!({});
    if let Some(enable_debug_logging) = enable_debug_logging {
        req_body["enable_debug_logging"] = serde_json::json!(enable_debug_logging);
    }
    if let Some(enable_debugger) = enable_debugger {
        req_body["enable_debugger"] = serde_json::json!(enable_debugger);
    }
    github_request("POST", &path, Some(req_body.to_string()))
}

fn append_optional_query_str(
    path: &mut String,
    key: &str,
    value: Option<&str>,
) -> Result<(), String> {
    if let Some(value) = value {
        validate_input_length(value, key)?;
        append_query_pair(path, key, value);
    }
    Ok(())
}

fn append_query_pair(path: &mut String, key: &str, value: &str) {
    path.push('&');
    path.push_str(key);
    path.push('=');
    path.push_str(&url_encode_query(value));
}
