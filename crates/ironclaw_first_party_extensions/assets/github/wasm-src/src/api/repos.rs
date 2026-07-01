use crate::request::github_request;
use crate::types::RepoListType;
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

pub(crate) fn get_authenticated_user() -> Result<String, String> {
    github_request("GET", "/user", None)
}

pub(crate) fn list_repos(
    repo_type: Option<RepoListType>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<String, String> {
    validate_page(page)?;
    validate_limit(limit)?;
    let limit = limit.unwrap_or(30).min(100); // Cap at 100
    let mut path = format!("/user/repos?per_page={}", limit);
    if let Some(repo_type) = repo_type {
        path.push_str("&type=");
        path.push_str(repo_type.as_str());
    }
    if let Some(p) = page {
        path.push_str(&format!("&page={}", p));
    }
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
    validate_page(page)?;
    validate_limit(limit)?;
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

pub(crate) fn list_releases(
    owner: &str,
    repo: &str,
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
        "/repos/{}/{}/releases?per_page={}",
        encoded_owner, encoded_repo, limit
    );
    if let Some(page) = page {
        path.push_str(&format!("&page={page}"));
    }
    github_request("GET", &path, None)
}

// arch-exempt: too_many_args, release creation keeps GitHub's release knobs explicit, plan #5171
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
