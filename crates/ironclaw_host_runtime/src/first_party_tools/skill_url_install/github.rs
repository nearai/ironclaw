use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
};

use ironclaw_host_api::{ResourceUsage, RuntimeDispatchErrorKind};
use serde::Deserialize;

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    MAX_GITHUB_CONTENT_API_REQUESTS, MAX_GITHUB_CONTENT_API_RESPONSE_BYTES,
    MAX_GITHUB_PATH_SEGMENTS, SkillUrlPayload, bundle::BundleCollector,
    bundle::normalize_archive_path, fetch_url_bytes, fetch_url_response, validate_skill_url,
    zip_bundle::extract_skill_bundle_blocking,
};

const MAX_GITHUB_API_REQUESTS: usize = 24;

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

#[derive(Debug, Deserialize)]
struct GitHubRepoMetadata {
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubCommitListItem {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRefItem {
    #[serde(rename = "ref")]
    git_ref: String,
}

#[derive(Debug, Deserialize)]
struct GitHubContentFile {
    #[serde(rename = "type")]
    entry_type: String,
    path: Option<String>,
    download_url: Option<String>,
}

#[derive(Debug, Default)]
struct GitHubApiBudget {
    calls: usize,
}

impl GitHubApiBudget {
    fn consume(&mut self) -> Result<(), FirstPartyCapabilityError> {
        if self.calls >= MAX_GITHUB_API_REQUESTS {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        self.calls += 1;
        Ok(())
    }
}

pub(super) async fn fetch_payload_if_supported(
    request: &FirstPartyCapabilityRequest,
    parsed: &url::Url,
    usage: &mut ResourceUsage,
) -> Result<Option<SkillUrlPayload>, FirstPartyCapabilityError> {
    let mut api_budget = GitHubApiBudget::default();
    if let Some(blob) = parse_github_blob_ref(parsed) {
        let raw_url =
            resolve_github_blob_download_url(request, blob, usage, &mut api_budget).await?;
        let bytes = fetch_url_bytes(request, &raw_url, usage).await?;
        return Ok(Some(SkillUrlPayload {
            content: String::from_utf8(bytes).map_err(|_| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                    .with_usage(usage.clone())
            })?,
            files: Vec::new(),
        }));
    }

    if let Some(repo) = parse_github_repo_ref(parsed) {
        return fetch_github_repo_payload(request, parsed.as_str(), repo, usage, &mut api_budget)
            .await
            .map(Some);
    }

    if parsed.host_str() == Some("github.com") {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }

    Ok(None)
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

fn build_github_api_base_url(
    owner: &str,
    repo: &str,
) -> Result<url::Url, FirstPartyCapabilityError> {
    validate_github_repo_components(owner, repo)?;
    validate_skill_url(&format!("https://api.github.com/repos/{owner}/{repo}"))
}

fn build_github_contents_url(
    owner: &str,
    repo: &str,
    path: Option<&str>,
    git_ref: &str,
) -> Result<url::Url, FirstPartyCapabilityError> {
    let mut url = build_github_api_base_url(owner, repo)?;
    {
        let mut segments = url.path_segments_mut().map_err(|error| {
            tracing::debug!(
                ?error,
                url_context = "github_contents",
                "failed to build GitHub URL path"
            );
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

fn build_github_raw_url(
    owner: &str,
    repo: &str,
    git_ref: &str,
    path: &Path,
) -> Result<url::Url, FirstPartyCapabilityError> {
    validate_github_repo_components(owner, repo)?;
    let mut url = validate_skill_url("https://raw.githubusercontent.com")?;
    {
        let mut segments = url.path_segments_mut().map_err(|error| {
            tracing::debug!(
                ?error,
                url_context = "github_raw",
                "failed to build GitHub URL path"
            );
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        segments.push(owner);
        segments.push(repo);
        segments.push(git_ref);
        for segment in path.iter() {
            let segment = segment.to_str().ok_or_else(|| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
            })?;
            segments.push(segment);
        }
    }
    Ok(url)
}

fn build_github_matching_refs_url(
    owner: &str,
    repo: &str,
    namespace: &str,
    prefix: &str,
) -> Result<url::Url, FirstPartyCapabilityError> {
    if !matches!(namespace, "heads" | "tags") || !is_safe_github_component(prefix) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    let mut url = build_github_api_base_url(owner, repo)?;
    {
        let mut segments = url.path_segments_mut().map_err(|error| {
            tracing::debug!(
                ?error,
                url_context = "github_matching_refs",
                "failed to build GitHub URL path"
            );
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        segments.push("git");
        segments.push("matching-refs");
        segments.push(namespace);
        segments.push(prefix);
    }
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

async fn fetch_github_api_json<T: for<'de> Deserialize<'de>>(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<T, FirstPartyCapabilityError> {
    let (value, _) = fetch_github_api_json_with_body_len(request, url, usage, api_budget).await?;
    Ok(value)
}

async fn fetch_github_api_json_with_body_len<T: for<'de> Deserialize<'de>>(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<(T, u64), FirstPartyCapabilityError> {
    api_budget.consume()?;
    let response = fetch_url_response(request, url, usage, github_api_headers()).await?;
    if !(200..300).contains(&response.status) {
        return Err(
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone()),
        );
    }
    let body_len = response.body.len() as u64;
    let value = serde_json::from_slice(&response.body).map_err(|_| {
        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            .with_usage(usage.clone())
    })?;
    Ok((value, body_len))
}

async fn fetch_github_contents_dir(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<(Vec<GitHubContentFile>, u64), FirstPartyCapabilityError> {
    api_budget.consume()?;
    let response = fetch_url_response(request, url, usage, github_api_headers()).await?;
    if !(200..300).contains(&response.status) {
        return Err(
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone()),
        );
    }
    if response
        .body
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_none_or(|byte| byte != b'[')
    {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    let body_len = response.body.len() as u64;
    let value = serde_json::from_slice(&response.body).map_err(|_| {
        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            .with_usage(usage.clone())
    })?;
    Ok((value, body_len))
}

async fn resolve_github_default_branch(
    request: &FirstPartyCapabilityRequest,
    owner: &str,
    repo: &str,
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<String, FirstPartyCapabilityError> {
    let api_url = build_github_api_base_url(owner, repo)?;
    let meta: GitHubRepoMetadata =
        fetch_github_api_json(request, &api_url, usage, api_budget).await?;
    meta.default_branch
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))
}

async fn resolve_github_ref_commit_sha(
    request: &FirstPartyCapabilityRequest,
    owner: &str,
    repo: &str,
    git_ref: &str,
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<String, FirstPartyCapabilityError> {
    let mut commits_url = build_github_api_base_url(owner, repo)?;
    {
        let mut segments = commits_url.path_segments_mut().map_err(|error| {
            tracing::debug!(
                ?error,
                url_context = "github_commits",
                "failed to build GitHub URL path"
            );
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
        })?;
        segments.push("commits");
    }
    commits_url
        .query_pairs_mut()
        .append_pair("sha", git_ref)
        .append_pair("per_page", "1");

    let commits: Vec<GitHubCommitListItem> =
        fetch_github_api_json(request, &commits_url, usage, api_budget).await?;
    let sha = commits
        .first()
        .map(|commit| commit.sha.as_str())
        .filter(|sha| !sha.trim().is_empty())
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;
    if !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::OperationFailed,
        ));
    }
    Ok(sha.to_string())
}

async fn resolve_github_ref_from_segments(
    request: &FirstPartyCapabilityRequest,
    owner: &str,
    repo: &str,
    segments: &[String],
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<(String, Vec<String>), FirstPartyCapabilityError> {
    if segments.is_empty() || segments.len() > MAX_GITHUB_PATH_SEGMENTS {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    for namespace in ["heads", "tags"] {
        let mut candidates = Vec::new();
        let refs_url = build_github_matching_refs_url(owner, repo, namespace, &segments[0])?;
        let refs: Vec<GitHubRefItem> =
            fetch_github_api_json(request, &refs_url, usage, api_budget).await?;
        let prefix = format!("refs/{namespace}/");
        for item in refs {
            let Some(git_ref) = item.git_ref.strip_prefix(&prefix) else {
                continue;
            };
            let ref_segments = git_ref.split('/').collect::<Vec<_>>();
            if ref_segments.len() > segments.len() {
                continue;
            }
            if ref_segments
                .iter()
                .zip(segments.iter())
                .all(|(left, right)| *left == right)
            {
                candidates.push((git_ref.to_string(), ref_segments.len()));
            }
        }
        if let Some((git_ref, consumed)) =
            candidates.into_iter().max_by_key(|(_, consumed)| *consumed)
        {
            return Ok((git_ref, segments[consumed..].to_vec()));
        }
    }
    Err(FirstPartyCapabilityError::new(
        RuntimeDispatchErrorKind::OperationFailed,
    ))
}

async fn resolve_github_tree_request(
    request: &FirstPartyCapabilityRequest,
    repo: GitHubRepoRequest,
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<GitHubRepoRef, FirstPartyCapabilityError> {
    validate_github_repo_components(&repo.owner, &repo.repo)?;
    if let Some(segments) = repo.tree_segments {
        let (candidate_ref, remaining) = resolve_github_ref_from_segments(
            request,
            &repo.owner,
            &repo.repo,
            &segments,
            usage,
            api_budget,
        )
        .await?;
        let candidate_subdir = (!remaining.is_empty()).then(|| remaining.join("/"));
        return Ok(GitHubRepoRef {
            owner: repo.owner,
            repo: repo.repo,
            branch: candidate_ref,
            subdir: candidate_subdir,
        });
    }

    let branch =
        resolve_github_default_branch(request, &repo.owner, &repo.repo, usage, api_budget).await?;

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
    api_budget: &mut GitHubApiBudget,
) -> Result<url::Url, FirstPartyCapabilityError> {
    validate_github_repo_components(&blob.owner, &blob.repo)?;
    if blob.blob_segments.len() < 2 || blob.blob_segments.len() > MAX_GITHUB_PATH_SEGMENTS {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    let (candidate_ref, remaining) = resolve_github_ref_from_segments(
        request,
        &blob.owner,
        &blob.repo,
        &blob.blob_segments,
        usage,
        api_budget,
    )
    .await?;
    if remaining.is_empty() {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    let candidate_path = remaining.join("/");
    let contents_url = build_github_contents_url(
        &blob.owner,
        &blob.repo,
        Some(&candidate_path),
        &candidate_ref,
    )?;
    let metadata: GitHubContentFile =
        fetch_github_api_json(request, &contents_url, usage, api_budget).await?;
    if metadata.entry_type != "file" {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::OperationFailed,
        ));
    }
    let Some(download_url) = metadata.download_url else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::OperationFailed,
        ));
    };
    let parsed = validate_skill_url(&download_url)?;
    if parsed.host_str() != Some("raw.githubusercontent.com") {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    Ok(parsed)
}

async fn fetch_github_repo_payload(
    request: &FirstPartyCapabilityRequest,
    source_url: &str,
    repo_request: GitHubRepoRequest,
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
    let repo = resolve_github_tree_request(request, repo_request, usage, api_budget).await?;
    let commit_sha = resolve_github_ref_commit_sha(
        request,
        &repo.owner,
        &repo.repo,
        &repo.branch,
        usage,
        api_budget,
    )
    .await?;
    if repo.subdir.is_some() {
        return fetch_github_contents_bundle_payload(
            request,
            source_url,
            repo,
            &commit_sha,
            usage,
            api_budget,
        )
        .await;
    }
    let archive_url = validate_skill_url(&format!(
        "https://codeload.github.com/{}/{}/legacy.zip/{}",
        repo.owner, repo.repo, commit_sha
    ))?;
    let bytes = fetch_url_bytes(request, &archive_url, usage).await?;
    let bundle = extract_skill_bundle_blocking(bytes, repo.subdir).await?;
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

async fn fetch_github_contents_bundle_payload(
    request: &FirstPartyCapabilityRequest,
    source_url: &str,
    repo: GitHubRepoRef,
    commit_sha: &str,
    usage: &mut ResourceUsage,
    api_budget: &mut GitHubApiBudget,
) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
    let root_subdir = repo
        .subdir
        .as_deref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;
    let root_path = normalize_archive_path(Path::new(root_subdir))?;
    let root_dir = normalized_archive_path_to_string(&root_path)?;
    if !root_dir.split('/').all(is_safe_github_component) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    let mut directories = VecDeque::from([root_path.clone()]);
    let mut visited_directories = 0usize;
    let mut contents_response_bytes = 0u64;
    let mut seen_files = HashSet::<PathBuf>::new();
    let mut collector = BundleCollector::new(root_path);

    while let Some(directory) = directories.pop_front() {
        visited_directories += 1;
        if visited_directories > MAX_GITHUB_CONTENT_API_REQUESTS {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        let directory_path = normalized_archive_path_to_string(&directory)?;
        let contents_url =
            build_github_contents_url(&repo.owner, &repo.repo, Some(&directory_path), commit_sha)?;
        let (entries, response_bytes) =
            fetch_github_contents_dir(request, &contents_url, usage, api_budget).await?;
        contents_response_bytes = contents_response_bytes
            .checked_add(response_bytes)
            .ok_or_else(|| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputTooLarge)
            })?;
        if contents_response_bytes > MAX_GITHUB_CONTENT_API_RESPONSE_BYTES {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        for entry in entries {
            let entry_path = entry.path.ok_or_else(|| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?;
            match entry.entry_type.as_str() {
                "dir" => {
                    let path = normalize_archive_path(Path::new(&entry_path))?;
                    collector.relative_path(&path)?;
                    directories.push_back(path);
                }
                "file" => {
                    let path = normalize_archive_path(Path::new(&entry_path))?;
                    let Some(relative) = collector.relative_path(&path)? else {
                        continue;
                    };
                    if !seen_files.insert(relative) {
                        return Err(FirstPartyCapabilityError::new(
                            RuntimeDispatchErrorKind::InputEncode,
                        ));
                    }
                    let download_url =
                        build_github_raw_url(&repo.owner, &repo.repo, commit_sha, &path)?;
                    let bytes = fetch_url_bytes(request, &download_url, usage).await?;
                    collector.push_file(path, bytes)?;
                }
                _ => {}
            }
        }
    }

    let payload = collector.finish()?;
    tracing::debug!(
        source_url,
        source_subdir = root_dir,
        bundle_file_count = payload.files.len(),
        "skill URL install fetched GitHub contents bundle"
    );
    Ok(payload)
}

fn normalized_archive_path_to_string(path: &Path) -> Result<String, FirstPartyCapabilityError> {
    let mut segments = Vec::new();
    for segment in path.iter() {
        segments.push(segment.to_str().ok_or_else(|| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
        })?);
    }
    if segments.is_empty() {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    Ok(segments.join("/"))
}
