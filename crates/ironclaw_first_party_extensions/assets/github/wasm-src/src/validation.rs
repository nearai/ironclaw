use crate::types::GitCommitIdentity;

const MAX_TEXT_LENGTH: usize = 65536;

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

pub(crate) fn validate_commit_identity(
    identity: &GitCommitIdentity,
    field_name: &str,
) -> Result<(), String> {
    validate_input_length(&identity.name, &format!("{field_name}.name"))?;
    validate_input_length(&identity.email, &format!("{field_name}.email"))?;
    Ok(())
}
