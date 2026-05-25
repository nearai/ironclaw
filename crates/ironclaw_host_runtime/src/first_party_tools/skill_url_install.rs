use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

use ironclaw_host_api::{
    NetworkMethod, NetworkPolicy, ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgressError,
    RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest, RuntimeKind,
};
use serde_json::Value;

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

const SKILL_URL_RESPONSE_BODY_LIMIT_BYTES: u64 = 10 * 1024 * 1024;
const SKILL_URL_FETCH_TIMEOUT_MS: u32 = 10_000;
const MAX_ZIP_ENTRY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_UNZIPPED_BYTES: u64 = 20 * 1024 * 1024;
const MAX_GITHUB_PATH_SEGMENTS: usize = 32;
const MAX_ZIP_FILE_ENTRIES: usize = ironclaw_skills::MAX_INSTALL_BUNDLE_FILES * 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkillUrlPayload {
    pub(super) content: String,
    pub(super) files: Vec<SkillUrlPayloadFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkillUrlPayloadFile {
    pub(super) path: PathBuf,
    pub(super) contents: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ZipSkillBundle {
    skill_md: String,
    files: Vec<SkillUrlPayloadFile>,
    bundle_subdir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubRepoRef {
    owner: String,
    repo: String,
    branch: String,
    subdir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubRepoRequest {
    owner: String,
    repo: String,
    tree_segments: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubBlobRequest {
    owner: String,
    repo: String,
    blob_segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FetchedBytes {
    status: u16,
    body: Vec<u8>,
}

pub(super) async fn fetch_skill_url_payload(
    request: &FirstPartyCapabilityRequest,
    url: &str,
    usage: &mut ResourceUsage,
) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
    let parsed = validate_skill_url(url)?;
    fetch_skill_payload_from_url(request, &parsed, usage).await
}

async fn fetch_url_bytes(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
) -> Result<Vec<u8>, FirstPartyCapabilityError> {
    fetch_url_bytes_with_headers(request, url, usage, Vec::new()).await
}

async fn fetch_url_bytes_with_headers(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
    headers: Vec<(String, String)>,
) -> Result<Vec<u8>, FirstPartyCapabilityError> {
    let response = fetch_url_response(request, url, usage, headers).await?;
    if !(200..300).contains(&response.status) {
        return Err(
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone()),
        );
    }
    Ok(response.body)
}

async fn fetch_url_response(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
    headers: Vec<(String, String)>,
) -> Result<FetchedBytes, FirstPartyCapabilityError> {
    let egress = request
        .services
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
        .clone();
    let http_request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method: NetworkMethod::Get,
        url: url.to_string(),
        headers,
        body: Vec::new(),
        network_policy: NetworkPolicy::default(),
        credential_injections: Vec::new(),
        response_body_limit: Some(SKILL_URL_RESPONSE_BODY_LIMIT_BYTES),
        timeout_ms: Some(SKILL_URL_FETCH_TIMEOUT_MS),
    };
    let response = tokio::task::spawn_blocking(move || egress.execute(http_request))
        .await
        .map_err(|error| {
            if error.is_panic() {
                tracing::error!("skill URL fetch egress worker panicked");
            }
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        })?
        .map_err(|error| skill_url_fetch_error(error, usage))?;
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(response.request_bytes);
    Ok(FetchedBytes {
        status: response.status,
        body: response.body,
    })
}

fn validate_skill_url(url: &str) -> Result<url::Url, FirstPartyCapabilityError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    Ok(parsed)
}

async fn fetch_skill_payload_from_url(
    request: &FirstPartyCapabilityRequest,
    parsed: &url::Url,
    usage: &mut ResourceUsage,
) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
    if let Some(blob) = parse_github_blob_ref(parsed) {
        let raw_url = resolve_github_blob_download_url(request, blob, usage).await?;
        let bytes = fetch_url_bytes(request, &raw_url, usage).await?;
        return Ok(SkillUrlPayload {
            content: String::from_utf8(bytes).map_err(|_| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                    .with_usage(usage.clone())
            })?,
            files: Vec::new(),
        });
    }

    if let Some(repo) = parse_github_repo_ref(parsed) {
        return fetch_github_repo_payload(request, parsed.as_str(), repo, usage).await;
    }

    let bytes = fetch_url_bytes(request, parsed, usage).await?;
    if bytes.starts_with(b"PK\x03\x04") {
        let bundle = extract_skill_bundle_from_zip_blocking(bytes, None).await?;
        return Ok(SkillUrlPayload {
            content: bundle.skill_md,
            files: bundle.files,
        });
    }

    Ok(SkillUrlPayload {
        content: String::from_utf8(bytes).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone())
        })?,
        files: Vec::new(),
    })
}

fn parse_github_blob_ref(parsed: &url::Url) -> Option<GitHubBlobRequest> {
    if parsed.host_str()? != "github.com" {
        return None;
    }
    let parts = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 5 || parts[2] != "blob" {
        return None;
    }
    let repo = parts[1].trim_end_matches(".git").to_string();
    if repo.is_empty() {
        return None;
    }
    Some(GitHubBlobRequest {
        owner: parts[0].to_string(),
        repo,
        blob_segments: parts[3..]
            .iter()
            .map(|segment| (*segment).to_string())
            .collect(),
    })
}

fn parse_github_repo_ref(parsed: &url::Url) -> Option<GitHubRepoRequest> {
    if parsed.host_str()? != "github.com" {
        return None;
    }
    let parts = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let owner = parts[0].to_string();
    let repo = parts[1].trim_end_matches(".git").to_string();
    if repo.is_empty() {
        return None;
    }
    if parts.len() == 2 {
        return Some(GitHubRepoRequest {
            owner,
            repo,
            tree_segments: None,
        });
    }
    if parts.len() >= 4 && parts[2] == "tree" {
        return Some(GitHubRepoRequest {
            owner,
            repo,
            tree_segments: Some(
                parts[3..]
                    .iter()
                    .map(|segment| (*segment).to_string())
                    .collect(),
            ),
        });
    }
    None
}

fn is_safe_github_component(component: &str) -> bool {
    !component.is_empty()
        && component
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn validate_github_repo_components(
    owner: &str,
    repo: &str,
) -> Result<(), FirstPartyCapabilityError> {
    if is_safe_github_component(owner) && is_safe_github_component(repo) {
        Ok(())
    } else {
        Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ))
    }
}

fn validate_derived_fetch_url(url: &str) -> Result<url::Url, FirstPartyCapabilityError> {
    validate_skill_url(url)
}

fn build_github_api_base_url(
    owner: &str,
    repo: &str,
) -> Result<url::Url, FirstPartyCapabilityError> {
    validate_github_repo_components(owner, repo)?;
    validate_derived_fetch_url(&format!("https://api.github.com/repos/{owner}/{repo}"))
}

fn build_github_contents_url(
    owner: &str,
    repo: &str,
    path: Option<&str>,
    git_ref: &str,
) -> Result<url::Url, FirstPartyCapabilityError> {
    let mut url = build_github_api_base_url(owner, repo)?;
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        segments.push("contents");
        if let Some(path) = path {
            for segment in path.split('/').filter(|segment| !segment.is_empty()) {
                segments.push(segment);
            }
        }
    }
    url.query_pairs_mut().append_pair("ref", git_ref);
    Ok(url)
}

fn github_api_headers() -> Vec<(String, String)> {
    vec![
        (
            "User-Agent".to_string(),
            "ironclaw-skill-install".to_string(),
        ),
        (
            "Accept".to_string(),
            "application/vnd.github+json".to_string(),
        ),
    ]
}

async fn fetch_github_api_value(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
) -> Result<Value, FirstPartyCapabilityError> {
    let bytes = fetch_url_bytes_with_headers(request, url, usage, github_api_headers()).await?;
    serde_json::from_slice(&bytes).map_err(|_| {
        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            .with_usage(usage.clone())
    })
}

async fn resolve_github_default_branch(
    request: &FirstPartyCapabilityRequest,
    owner: &str,
    repo: &str,
    usage: &mut ResourceUsage,
) -> Result<String, FirstPartyCapabilityError> {
    let api_url = build_github_api_base_url(owner, repo)?;
    let meta = fetch_github_api_value(request, &api_url, usage).await?;
    meta.get("default_branch")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))
}

async fn resolve_github_ref_commit_sha(
    request: &FirstPartyCapabilityRequest,
    owner: &str,
    repo: &str,
    git_ref: &str,
    usage: &mut ResourceUsage,
) -> Result<String, FirstPartyCapabilityError> {
    let mut commits_url = build_github_api_base_url(owner, repo)?;
    {
        let mut segments = commits_url.path_segments_mut().map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        segments.push("commits");
    }
    commits_url
        .query_pairs_mut()
        .append_pair("sha", git_ref)
        .append_pair("per_page", "1");

    let commits = fetch_github_api_value(request, &commits_url, usage).await?;
    let sha = commits
        .as_array()
        .and_then(|commits| commits.first())
        .and_then(|commit| commit.get("sha"))
        .and_then(Value::as_str)
        .filter(|sha| !sha.trim().is_empty())
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;
    if !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::OperationFailed,
        ));
    }
    Ok(sha.to_string())
}

async fn github_ref_path_exists(
    request: &FirstPartyCapabilityRequest,
    owner: &str,
    repo: &str,
    git_ref: &str,
    path: Option<&str>,
    usage: &mut ResourceUsage,
) -> Result<bool, FirstPartyCapabilityError> {
    let contents_url = build_github_contents_url(owner, repo, path, git_ref)?;
    let response = fetch_url_response(request, &contents_url, usage, github_api_headers()).await?;
    match response.status {
        200..=299 => Ok(true),
        404 => Ok(false),
        _ => Err(
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone()),
        ),
    }
}

async fn resolve_github_tree_request(
    request: &FirstPartyCapabilityRequest,
    repo: GitHubRepoRequest,
    usage: &mut ResourceUsage,
) -> Result<GitHubRepoRef, FirstPartyCapabilityError> {
    validate_github_repo_components(&repo.owner, &repo.repo)?;
    let branch = match repo.tree_segments {
        Some(segments) => {
            if segments.is_empty() || segments.len() > MAX_GITHUB_PATH_SEGMENTS {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::InputEncode,
                ));
            }
            for split in (1..=segments.len()).rev() {
                let candidate_ref = segments[..split].join("/");
                let candidate_subdir =
                    (split < segments.len()).then(|| segments[split..].join("/"));
                if github_ref_path_exists(
                    request,
                    &repo.owner,
                    &repo.repo,
                    &candidate_ref,
                    candidate_subdir.as_deref(),
                    usage,
                )
                .await?
                {
                    return Ok(GitHubRepoRef {
                        owner: repo.owner,
                        repo: repo.repo,
                        branch: candidate_ref,
                        subdir: candidate_subdir,
                    });
                }
            }
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OperationFailed,
            ));
        }
        None => resolve_github_default_branch(request, &repo.owner, &repo.repo, usage).await?,
    };

    Ok(GitHubRepoRef {
        owner: repo.owner,
        repo: repo.repo,
        branch,
        subdir: None,
    })
}

async fn resolve_github_blob_download_url(
    request: &FirstPartyCapabilityRequest,
    blob: GitHubBlobRequest,
    usage: &mut ResourceUsage,
) -> Result<url::Url, FirstPartyCapabilityError> {
    validate_github_repo_components(&blob.owner, &blob.repo)?;
    if blob.blob_segments.len() < 2 || blob.blob_segments.len() > MAX_GITHUB_PATH_SEGMENTS {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    for split in (1..blob.blob_segments.len()).rev() {
        let candidate_ref = blob.blob_segments[..split].join("/");
        let candidate_path = blob.blob_segments[split..].join("/");
        let contents_url = build_github_contents_url(
            &blob.owner,
            &blob.repo,
            Some(&candidate_path),
            &candidate_ref,
        )?;
        let response =
            fetch_url_response(request, &contents_url, usage, github_api_headers()).await?;
        if response.status == 404 {
            continue;
        }
        if !(200..300).contains(&response.status) {
            return Err(
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                    .with_usage(usage.clone()),
            );
        }
        let metadata: Value = serde_json::from_slice(&response.body).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone())
        })?;
        if metadata.get("type").and_then(Value::as_str) != Some("file") {
            continue;
        }
        let Some(download_url) = metadata.get("download_url").and_then(Value::as_str) else {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OperationFailed,
            ));
        };
        return validate_derived_fetch_url(download_url);
    }
    Err(FirstPartyCapabilityError::new(
        RuntimeDispatchErrorKind::OperationFailed,
    ))
}

async fn fetch_github_repo_payload(
    request: &FirstPartyCapabilityRequest,
    source_url: &str,
    repo_request: GitHubRepoRequest,
    usage: &mut ResourceUsage,
) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
    let repo = resolve_github_tree_request(request, repo_request, usage).await?;
    let commit_sha =
        resolve_github_ref_commit_sha(request, &repo.owner, &repo.repo, &repo.branch, usage)
            .await?;
    let archive_url = validate_derived_fetch_url(&format!(
        "https://codeload.github.com/{}/{}/legacy.zip/{}",
        repo.owner, repo.repo, commit_sha
    ))?;
    let bytes = fetch_url_bytes(request, &archive_url, usage).await?;
    let bundle = extract_skill_bundle_from_zip_blocking(bytes, repo.subdir).await?;
    tracing::debug!(
        source_url,
        source_subdir = ?bundle.bundle_subdir,
        bundle_file_count = bundle.files.len(),
        "skill URL install fetched GitHub bundle"
    );
    Ok(SkillUrlPayload {
        content: bundle.skill_md,
        files: bundle.files,
    })
}

async fn extract_skill_bundle_from_zip_blocking(
    data: Vec<u8>,
    requested_subdir: Option<String>,
) -> Result<ZipSkillBundle, FirstPartyCapabilityError> {
    tokio::task::spawn_blocking(move || {
        extract_skill_bundle_from_zip(&data, requested_subdir.as_deref())
    })
    .await
    .map_err(|error| {
        if error.is_panic() {
            tracing::error!("skill URL ZIP extraction worker panicked");
        }
        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
    })?
}

fn normalize_archive_path(path: &Path) -> Result<PathBuf, FirstPartyCapabilityError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::InputEncode,
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    Ok(normalized)
}

fn strip_common_archive_root(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut root: Option<std::ffi::OsString> = None;
    let mut has_nested = false;
    for path in paths {
        let mut components = path.components();
        let Some(Component::Normal(first)) = components.next() else {
            return None;
        };
        has_nested |= components.next().is_some();
        match &root {
            Some(existing) if existing != first => return None,
            None => root = Some(first.to_os_string()),
            _ => {}
        }
    }
    if !has_nested {
        return None;
    }
    root.map(PathBuf::from)
}

fn extract_skill_bundle_from_zip(
    data: &[u8],
    requested_subdir: Option<&str>,
) -> Result<ZipSkillBundle, FirstPartyCapabilityError> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;

    let mut raw_paths = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        if !file.is_dir() {
            if raw_paths.len() >= MAX_ZIP_FILE_ENTRIES {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OutputTooLarge,
                ));
            }
            raw_paths.push(normalize_archive_path(Path::new(file.name()))?);
        }
    }
    let strip_root = strip_common_archive_root(&raw_paths);
    let mut files = Vec::<(PathBuf, Vec<u8>)>::new();
    let mut seen_paths = std::collections::HashSet::<PathBuf>::new();
    let mut skill_dirs = std::collections::HashSet::<PathBuf>::new();
    let mut total_unzipped_bytes = 0u64;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        if file.is_dir() {
            continue;
        }
        if file.size() > MAX_ZIP_ENTRY_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        let entry_name = file.name().to_string();
        let mut path = normalize_archive_path(Path::new(&entry_name))?;
        if let Some(root) = &strip_root
            && let Ok(stripped) = path.strip_prefix(root)
        {
            path = stripped.to_path_buf();
        }
        if path.as_os_str().is_empty() {
            continue;
        }
        if !seen_paths.insert(path.clone()) {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::InputEncode,
            ));
        }

        let mut contents = Vec::new();
        (&mut file)
            .take(MAX_ZIP_ENTRY_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|_| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?;
        if contents.len() as u64 > MAX_ZIP_ENTRY_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        total_unzipped_bytes = total_unzipped_bytes
            .checked_add(contents.len() as u64)
            .ok_or_else(|| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputTooLarge)
            })?;
        if total_unzipped_bytes > MAX_TOTAL_UNZIPPED_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        if path.file_name().is_some_and(|name| name == "SKILL.md") {
            skill_dirs.insert(path.parent().unwrap_or(Path::new("")).to_path_buf());
        }
        files.push((path, contents));
    }

    let requested_dir = if let Some(subdir) = requested_subdir {
        let normalized = normalize_archive_path(Path::new(subdir))?;
        if !skill_dirs.contains(&normalized) {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OperationFailed,
            ));
        }
        normalized
    } else {
        match skill_dirs.len() {
            0 => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OperationFailed,
                ));
            }
            1 => skill_dirs.into_iter().next().unwrap_or_default(),
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::InputEncode,
                ));
            }
        }
    };

    let mut skill_md = None;
    let mut extra_files = Vec::new();
    for (path, contents) in files {
        let Ok(relative) = path.strip_prefix(&requested_dir) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        if relative == Path::new("SKILL.md") {
            if contents.len() as u64 > ironclaw_skills::MAX_PROMPT_FILE_SIZE {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OutputTooLarge,
                ));
            }
            skill_md = Some(String::from_utf8(contents).map_err(|_| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?);
            continue;
        }
        extra_files.push(SkillUrlPayloadFile {
            path: relative.to_path_buf(),
            contents,
        });
        if extra_files.len() > ironclaw_skills::MAX_INSTALL_BUNDLE_FILES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
    }

    let skill_md = skill_md
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;
    Ok(ZipSkillBundle {
        skill_md,
        files: extra_files,
        bundle_subdir: (!requested_dir.as_os_str().is_empty())
            .then(|| requested_dir.display().to_string()),
    })
}

fn skill_url_fetch_error(
    error: RuntimeHttpEgressError,
    usage: &mut ResourceUsage,
) -> FirstPartyCapabilityError {
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(error.request_bytes());
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::PolicyDenied => RuntimeDispatchErrorKind::PolicyDenied,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OperationFailed,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    FirstPartyCapabilityError::new(kind).with_usage(usage.clone())
}
