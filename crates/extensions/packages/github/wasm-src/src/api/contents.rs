use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

use crate::request::github_request;
use crate::types::GitCommitIdentity;
use crate::validation::*;

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

// arch-exempt: too_many_args, file write inputs stay split to mirror GitHub payload shape, plan #5171
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

// arch-exempt: too_many_args, delete path needs the same GitHub file-write parameters, plan #5171
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
