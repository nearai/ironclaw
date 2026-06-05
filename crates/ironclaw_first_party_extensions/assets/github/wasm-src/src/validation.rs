use crate::types::GitCommitIdentity;

const MAX_TEXT_LENGTH: usize = 65536;
const MAX_SEARCH_QUERY_LENGTH: usize = 512;
const MAX_REPOSITORY_SEGMENT_LENGTH: usize = 100;

/// Validate input length to prevent oversized payloads.
pub(crate) fn validate_input_length(s: &str, field_name: &str) -> Result<(), String> {
    if s.len() > MAX_TEXT_LENGTH {
        return Err(format!(
            "Input '{}' exceeds maximum length of {} characters",
            field_name, MAX_TEXT_LENGTH
        ));
    }
    Ok(())
}

/// Percent-encode a string for safe use in URL path segments.
/// Encodes everything except alphanumeric, hyphen, underscore, and dot.
pub(crate) fn url_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

/// Percent-encode a string for use as a URL query parameter value.
/// Currently identical to `url_encode_path`.
pub(crate) fn url_encode_query(s: &str) -> String {
    url_encode_path(s)
}

/// Validate that a path segment doesn't contain dangerous characters.
/// Returns true if the segment is safe to use.
pub(crate) fn validate_path_segment(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('/')
        && !s.contains("..")
        && !s.contains('?')
        && !s.contains('#')
        && !s.chars().any(|c| c.is_control() || c.is_whitespace())
}

pub(crate) fn validate_repo_path(path: &str) -> Result<(), String> {
    validate_input_length(path, "path")?;
    for segment in path.split('/') {
        if segment == ".." || segment == "." {
            return Err("Invalid path: relative path segments not allowed".into());
        }
        if segment.is_empty() {
            return Err("Invalid path: empty segment not allowed".into());
        }
    }
    Ok(())
}

pub(crate) fn encode_repo_path(path: &str) -> String {
    path.split('/')
        .map(url_encode_path)
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn validate_git_ref(ref_name: &str, field_name: &str) -> Result<(), String> {
    if ref_name.is_empty() {
        return Err(format!("Invalid {field_name}: cannot be empty"));
    }
    if ref_name.contains("..")
        || ref_name.contains(':')
        || ref_name.contains('?')
        || ref_name.contains('[')
        || ref_name.contains('\\')
        || ref_name.contains('^')
        || ref_name.contains('~')
        || ref_name.contains("@{")
        || ref_name.contains("//")
        || ref_name.starts_with('/')
        || ref_name.ends_with('/')
        || ref_name.starts_with('.')
        || ref_name.ends_with('.')
        || ref_name.ends_with(".lock")
        || ref_name.chars().any(|c| c.is_control() || c == ' ')
    {
        return Err(format!(
            "Invalid {field_name}: must be a valid branch, tag, or ref name"
        ));
    }
    Ok(())
}

pub(crate) fn normalize_ref_lookup(ref_name: &str) -> Result<String, String> {
    validate_git_ref(ref_name, "from_ref")?;
    if is_full_commit_sha(ref_name) {
        return Err(
            "Unsupported from_ref: use a branch or tag ref, not a raw commit SHA".to_string(),
        );
    }
    if let Some(stripped) = ref_name.strip_prefix("refs/heads/") {
        return Ok(format!("heads/{stripped}"));
    }
    if let Some(stripped) = ref_name.strip_prefix("refs/tags/") {
        return Ok(format!("tags/{stripped}"));
    }
    if ref_name.starts_with("refs/") {
        return Err(
            "Unsupported from_ref: only refs/heads/* and refs/tags/* are supported".to_string(),
        );
    }
    if ref_name.starts_with("heads/") || ref_name.starts_with("tags/") {
        return Ok(ref_name.to_string());
    }
    Ok(format!("heads/{ref_name}"))
}

fn is_full_commit_sha(ref_name: &str) -> bool {
    ref_name.len() == 40 && ref_name.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(crate) fn normalize_branch_ref(branch: &str) -> Result<String, String> {
    validate_git_ref(branch, "branch")?;
    if branch.starts_with("refs/heads/") {
        return Ok(branch.to_string());
    }
    if branch.starts_with("refs/") {
        return Err("Invalid branch ref: only refs/heads/* is allowed".to_string());
    }
    let branch = branch.strip_prefix("heads/").unwrap_or(branch);
    if branch.starts_with("tags/") {
        return Err("Invalid branch ref: tags/* is not a branch".to_string());
    }
    Ok(format!("refs/heads/{branch}"))
}

pub(crate) fn append_search_params(
    path: &mut String,
    page: Option<u32>,
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), String> {
    if let Some(p) = page {
        path.push_str(&format!("&page={p}"));
    }
    if let Some(sort) = sort {
        validate_input_length(sort, "sort")?;
        path.push_str("&sort=");
        path.push_str(&url_encode_query(sort));
    }
    if let Some(order) = order {
        if !matches!(order, "asc" | "desc") {
            return Err("Invalid order: must be 'asc' or 'desc'".into());
        }
        path.push_str("&order=");
        path.push_str(order);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_issue_search_query(
    query: Option<&str>,
    repository: Option<&str>,
    owner: Option<&str>,
    repo: Option<&str>,
    author: Option<&str>,
    assignee: Option<&str>,
    involves: Option<&str>,
    state: Option<&str>,
    issue_type: Option<&str>,
) -> Result<String, String> {
    let mut parts = Vec::new();
    if repository.is_some() && (owner.is_some() || repo.is_some()) {
        return Err("invalid_repository".to_string());
    }
    if let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) {
        validate_search_query_length(query)?;
        parts.push(query.to_string());
    }
    if let Some(repository) = repository {
        let (owner, repo) = repository
            .split_once('/')
            .ok_or_else(|| "invalid_repository".to_string())?;
        validate_repository_qualifier(owner, repo)?;
        parts.push(format!("repo:{owner}/{repo}"));
    }
    match (owner, repo) {
        (Some(owner), Some(repo)) => {
            validate_repository_qualifier(owner, repo)?;
            parts.push(format!("repo:{owner}/{repo}"));
        }
        (None, Some(repo)) => {
            let (owner, repo) = repo
                .split_once('/')
                .ok_or_else(|| "invalid_repository".to_string())?;
            validate_repository_qualifier(owner, repo)?;
            parts.push(format!("repo:{owner}/{repo}"));
        }
        (None, None) => {}
        (Some(_), None) => return Err("invalid_repository".to_string()),
    }
    push_search_qualifier(&mut parts, "author", author)?;
    push_search_qualifier(&mut parts, "assignee", assignee)?;
    push_search_qualifier(&mut parts, "involves", involves)?;
    if let Some(state) = state {
        validate_search_state(state)?;
        parts.push(format!("state:{state}"));
    }
    if let Some(issue_type) = issue_type {
        validate_search_type(issue_type)?;
        parts.push(format!("is:{issue_type}"));
    }
    if parts.is_empty() {
        return Err("invalid_query_empty".to_string());
    }
    let query = parts.join(" ");
    validate_search_query_length(&query)?;
    Ok(query)
}

fn push_search_qualifier(
    parts: &mut Vec<String>,
    qualifier: &str,
    value: Option<&str>,
) -> Result<(), String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if validate_search_qualifier_value(value) {
        parts.push(format!("{qualifier}:{value}"));
        Ok(())
    } else {
        Err(format!("invalid_{qualifier}"))
    }
}

fn validate_repository_qualifier(owner: &str, repo: &str) -> Result<(), String> {
    if validate_repository_segment(owner) && validate_repository_segment(repo) {
        Ok(())
    } else {
        Err("invalid_repository".to_string())
    }
}

fn validate_repository_segment(value: &str) -> bool {
    validate_path_segment(value)
        && value.len() <= MAX_REPOSITORY_SEGMENT_LENGTH
        && !value.contains(':')
}

fn validate_search_qualifier_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REPOSITORY_SEGMENT_LENGTH
        && !value.contains(char::is_whitespace)
        && !value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, ':' | '"' | '(' | ')'))
}

fn validate_search_query_length(query: &str) -> Result<(), String> {
    if query.len() > MAX_SEARCH_QUERY_LENGTH {
        Err("invalid_query_too_large".to_string())
    } else {
        Ok(())
    }
}

pub(crate) fn validate_search_state(state: &str) -> Result<(), String> {
    match state {
        "open" | "closed" => Ok(()),
        _ => Err("invalid_state".to_string()),
    }
}

pub(crate) fn validate_search_type(issue_type: &str) -> Result<(), String> {
    match issue_type {
        "issue" | "pr" => Ok(()),
        _ => Err("invalid_type".to_string()),
    }
}

pub(crate) fn validate_search_sort(sort: Option<&str>) -> Result<(), String> {
    match sort {
        None | Some("comments" | "created" | "updated") => Ok(()),
        Some(_) => Err("invalid_sort".to_string()),
    }
}

pub(crate) fn validate_search_page(page: Option<u32>) -> Result<(), String> {
    match page {
        None | Some(1..=100) => Ok(()),
        Some(_) => Err("invalid_page".to_string()),
    }
}

pub(crate) fn validate_search_limit(limit: Option<u32>) -> Result<(), String> {
    match limit {
        None | Some(1..=100) => Ok(()),
        Some(_) => Err("invalid_limit".to_string()),
    }
}

pub(crate) fn validate_commit_identity(
    identity: &GitCommitIdentity,
    field_name: &str,
) -> Result<(), String> {
    validate_input_length(&identity.name, &format!("{field_name}.name"))?;
    validate_input_length(&identity.email, &format!("{field_name}.email"))?;
    Ok(())
}
