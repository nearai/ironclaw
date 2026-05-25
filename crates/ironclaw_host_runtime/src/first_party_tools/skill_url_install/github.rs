use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
};

use ironclaw_host_api::{ResourceUsage, RuntimeDispatchErrorKind};
use serde::Deserialize;

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{
    MAX_GITHUB_CONTENT_DIRS, MAX_GITHUB_PATH_SEGMENTS, SkillUrlPayload, bundle::BundleCollector,
    bundle::normalize_archive_path, fetch_url_bytes, fetch_url_bytes_with_headers,
    validate_derived_fetch_url, zip_bundle::extract_skill_bundle_blocking,
};

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

pub(super) async fn fetch_payload_if_supported(
    request: &FirstPartyCapabilityRequest,
    parsed: &url::Url,
    usage: &mut ResourceUsage,
) -> Result<Option<SkillUrlPayload>, FirstPartyCapabilityError> {
    if let Some(blob) = parse_github_blob_ref(parsed) {
        let raw_url = resolve_github_blob_download_url(request, blob, usage).await?;
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
        return fetch_github_repo_payload(request, parsed.as_str(), repo, usage)
            .await
            .map(Some);
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
        let mut segments = url.path_segments_mut().map_err(|_| {
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
) -> Result<T, FirstPartyCapabilityError> {
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
    let meta: GitHubRepoMetadata = fetch_github_api_json(request, &api_url, usage).await?;
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

    let commits: Vec<GitHubCommitListItem> =
        fetch_github_api_json(request, &commits_url, usage).await?;
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
) -> Result<(String, Vec<String>), FirstPartyCapabilityError> {
    if segments.is_empty() || segments.len() > MAX_GITHUB_PATH_SEGMENTS {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    for namespace in ["heads", "tags"] {
        let mut candidates = Vec::new();
        let refs_url = build_github_matching_refs_url(owner, repo, namespace, &segments[0])?;
        let refs: Vec<GitHubRefItem> = fetch_github_api_json(request, &refs_url, usage).await?;
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
) -> Result<GitHubRepoRef, FirstPartyCapabilityError> {
    validate_github_repo_components(&repo.owner, &repo.repo)?;
    let branch = match repo.tree_segments {
        Some(segments) => {
            let (candidate_ref, remaining) = resolve_github_ref_from_segments(
                request,
                &repo.owner,
                &repo.repo,
                &segments,
                usage,
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
    let (candidate_ref, remaining) = resolve_github_ref_from_segments(
        request,
        &blob.owner,
        &blob.repo,
        &blob.blob_segments,
        usage,
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
    let metadata: GitHubContentFile = fetch_github_api_json(request, &contents_url, usage).await?;
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
    validate_derived_fetch_url(&download_url)
}

async fn fetch_github_repo_payload(
    request: &FirstPartyCapabilityRequest,
    source_url: &str,
    repo_request: GitHubRepoRequest,
    usage: &mut ResourceUsage,
) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
    let repo = resolve_github_tree_request(request, repo_request, usage).await?;
    if repo.subdir.is_some() {
        return fetch_github_contents_bundle_payload(request, source_url, repo, usage).await;
    }
    let commit_sha =
        resolve_github_ref_commit_sha(request, &repo.owner, &repo.repo, &repo.branch, usage)
            .await?;
    let archive_url = validate_derived_fetch_url(&format!(
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
    usage: &mut ResourceUsage,
) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
    let root_subdir = repo
        .subdir
        .as_deref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;
    let root_path = normalize_archive_path(Path::new(root_subdir))?;
    let mut directories = VecDeque::from([root_subdir.to_string()]);
    let mut visited_directories = 0usize;
    let mut seen_files = HashSet::<PathBuf>::new();
    let mut collector = BundleCollector::new(root_path);

    while let Some(directory) = directories.pop_front() {
        visited_directories += 1;
        if visited_directories > MAX_GITHUB_CONTENT_DIRS {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::OutputTooLarge,
            ));
        }
        let contents_url =
            build_github_contents_url(&repo.owner, &repo.repo, Some(&directory), &repo.branch)?;
        let entries: Vec<GitHubContentFile> =
            fetch_github_api_json(request, &contents_url, usage).await?;
        for entry in entries {
            let entry_path = entry.path.ok_or_else(|| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
            })?;
            match entry.entry_type.as_str() {
                "dir" => directories.push_back(entry_path),
                "file" => {
                    let path = normalize_archive_path(Path::new(&entry_path))?;
                    let relative = path.strip_prefix(Path::new(root_subdir)).map_err(|_| {
                        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                    })?;
                    if relative.as_os_str().is_empty() {
                        continue;
                    }
                    if !seen_files.insert(relative.to_path_buf()) {
                        return Err(FirstPartyCapabilityError::new(
                            RuntimeDispatchErrorKind::InputEncode,
                        ));
                    }
                    let download_url = entry.download_url.ok_or_else(|| {
                        FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                    })?;
                    let download_url = validate_derived_fetch_url(&download_url)?;
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
        source_subdir = root_subdir,
        bundle_file_count = payload.files.len(),
        "skill URL install fetched GitHub contents bundle"
    );
    Ok(payload)
}
