//! Agent-callable tools for managing skills (prompt-level extensions).
//!
//! Four tools for discovering, installing, listing, and removing skills
//! entirely through conversation, following the extension_tools pattern.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::context::JobContext;
use crate::tools::tool::{
    ApprovalRequirement, EngineCompatibility, Tool, ToolError, ToolOutput, require_str,
};
use ironclaw_skills::catalog::{
    SkillCatalog, catalog_entry_is_installed, resolve_catalog_slug_for_name,
};
use ironclaw_skills::registry::SkillRegistry;

const MAX_CHAIN_DEPS: usize = 10;
const MAX_DOWNLOAD_BYTES: usize = 10 * 1024 * 1024;
const MAX_ZIP_ENTRY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_UNZIPPED_BYTES: u64 = 20 * 1024 * 1024;

/// Hard cap on the chain-installer BFS queue to prevent unbounded growth
/// from nested `requires.skills` fan-out. Even though we stop enqueueing
/// once `attempted >= MAX_CHAIN_DEPS`, this is a defense-in-depth bound in
/// case a future refactor (parallel fetching, retries) changes that
/// invariant.
const MAX_CHAIN_QUEUE: usize = MAX_CHAIN_DEPS * 10;

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub(crate) struct SkillFetchError {
    status: Option<u16>,
    message: String,
}

impl SkillFetchError {
    fn from_message(message: impl Into<String>) -> Self {
        Self {
            status: None,
            message: message.into(),
        }
    }

    fn from_http_status(status: u16, url: &str) -> Self {
        Self {
            status: Some(status),
            message: format!("Skill fetch returned HTTP {status}: {url}"),
        }
    }

    fn is_missing_dependency(&self) -> bool {
        matches!(self.status, Some(404 | 410))
    }
}

impl From<SkillFetchError> for ToolError {
    fn from(value: SkillFetchError) -> Self {
        ToolError::ExecutionFailed(value.to_string())
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SkillInstallPayload {
    pub(crate) skill_md: String,
    pub(crate) extra_files: Vec<ironclaw_skills::registry::InstallFile>,
    pub(crate) install_metadata: Option<ironclaw_skills::registry::InstalledSkillMetadata>,
}

#[derive(Debug)]
struct ZipSkillBundle {
    skill_md: String,
    extra_files: Vec<ironclaw_skills::registry::InstallFile>,
    bundle_subdir: Option<String>,
}

#[derive(Debug, Clone)]
struct GitHubRepoRef {
    owner: String,
    repo: String,
    branch: String,
    subdir: Option<String>,
}

#[derive(Debug, Clone)]
struct GitHubRepoRequest {
    owner: String,
    repo: String,
    tree_segments: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
struct GitHubBlobRequest {
    owner: String,
    repo: String,
    blob_segments: Vec<String>,
}

fn is_safe_github_component(component: &str) -> bool {
    !component.is_empty()
        && component
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn validate_github_repo_components(owner: &str, repo: &str) -> Result<(), SkillFetchError> {
    if !is_safe_github_component(owner) {
        return Err(SkillFetchError::from_message(format!(
            "Invalid GitHub owner in skill URL: {}",
            owner
        )));
    }
    if !is_safe_github_component(repo) {
        return Err(SkillFetchError::from_message(format!(
            "Invalid GitHub repository in skill URL: {}",
            repo
        )));
    }
    Ok(())
}

fn validate_github_repo_ref(repo: &GitHubRepoRef) -> Result<(), SkillFetchError> {
    validate_github_repo_components(&repo.owner, &repo.repo)
}

fn validate_derived_fetch_url(url: &str) -> Result<reqwest::Url, SkillFetchError> {
    validate_fetch_url(url).map_err(|e| SkillFetchError::from_message(e.to_string()))
}

fn validate_payload_skill_size(
    payload: SkillInstallPayload,
) -> Result<SkillInstallPayload, SkillFetchError> {
    if payload.skill_md.len() as u64 > ironclaw_skills::MAX_PROMPT_FILE_SIZE {
        return Err(SkillFetchError::from_message(format!(
            "Skill content too large: {} bytes (max {} bytes)",
            payload.skill_md.len(),
            ironclaw_skills::MAX_PROMPT_FILE_SIZE
        )));
    }
    Ok(payload)
}

#[derive(Debug, Default)]
struct ChainInstallReport {
    installed: Vec<String>,
    failed: Vec<String>,
    missing: Vec<String>,
    skipped: Vec<String>,
    pending_explicit_install: Vec<String>,
}

impl ChainInstallReport {
    fn has_warnings(&self) -> bool {
        !self.failed.is_empty()
            || !self.missing.is_empty()
            || !self.skipped.is_empty()
            || !self.pending_explicit_install.is_empty()
    }
}

/// Acquire a read lock on the skill registry, recovering from poisoning.
///
/// `std::sync::RwLock` becomes "poisoned" if a writer panics while holding it,
/// after which every subsequent `.read()` / `.write()` returns `Err`. The
/// skill registry only ever holds replace-on-success state (commits happen
/// after successful disk writes and validation), so a poisoned lock is safe
/// to recover from — failing every future `skill_install` call is a worse
/// outcome than ignoring the panic. We log loudly so the underlying panic
/// stays visible.
fn registry_read(
    registry: &Arc<std::sync::RwLock<SkillRegistry>>,
) -> std::sync::RwLockReadGuard<'_, SkillRegistry> {
    registry.read().unwrap_or_else(|poison| {
        tracing::error!(
            "skill registry RwLock was poisoned (a previous writer panicked); \
             recovering — skill state may be from before the panic"
        );
        poison.into_inner()
    })
}

/// Acquire a write lock on the skill registry, recovering from poisoning.
/// See [`registry_read`] for the rationale.
fn registry_write(
    registry: &Arc<std::sync::RwLock<SkillRegistry>>,
) -> std::sync::RwLockWriteGuard<'_, SkillRegistry> {
    registry.write().unwrap_or_else(|poison| {
        tracing::error!(
            "skill registry RwLock was poisoned (a previous writer panicked); \
             recovering — skill state may be from before the panic"
        );
        poison.into_inner()
    })
}

async fn install_missing_skill_dependencies<F, Fut>(
    registry: &Arc<std::sync::RwLock<SkillRegistry>>,
    registry_url: &str,
    required_skills: Vec<String>,
    fetcher: F,
) -> Result<ChainInstallReport, ToolError>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<SkillInstallPayload, SkillFetchError>>,
{
    let (user_dir, initial_missing) = {
        let guard = registry_read(registry);
        let missing = required_skills
            .into_iter()
            .filter(|name| !guard.has(name))
            .collect::<Vec<_>>();
        (guard.install_target_dir().to_path_buf(), missing)
    };

    let mut report = ChainInstallReport::default();
    let mut queue: VecDeque<String> = initial_missing.into_iter().collect();
    let mut queued_or_seen: HashSet<String> = queue.iter().cloned().collect();
    let mut attempted = 0usize;

    while let Some(dep_name) = queue.pop_front() {
        if !ironclaw_skills::validate_skill_name(&dep_name) {
            report
                .failed
                .push(format!("{}: invalid skill dependency name", dep_name));
            continue;
        }

        // Check whether the dep was satisfied by an earlier iteration (or
        // another concurrent install) BEFORE applying the cap, so already-
        // installed deps don't count toward `skipped_dependencies`.
        {
            let guard = registry_read(registry);
            if guard.has(&dep_name) {
                continue;
            }
        }

        if attempted >= MAX_CHAIN_DEPS {
            report.skipped.push(dep_name);
            continue;
        }

        attempted += 1;

        let download_url = ironclaw_skills::catalog::skill_download_url(registry_url, &dep_name);
        match fetcher(download_url).await {
            Ok(dep_bundle) => {
                let normalized = ironclaw_skills::normalize_line_endings(&dep_bundle.skill_md);
                match ironclaw_skills::registry::SkillRegistry::prepare_install_bundle_to_disk(
                    &user_dir,
                    &dep_name,
                    &normalized,
                    &dep_bundle.extra_files,
                    dep_bundle.install_metadata.as_ref(),
                )
                .await
                {
                    Ok((name, skill)) => {
                        // Dependency-confusion guard: the `name` returned
                        // by `prepare_install_to_disk` comes from the
                        // downloaded manifest, not from the `dep_name` we
                        // requested. A hostile catalog entry could publish
                        // a skill whose manifest declares a DIFFERENT name
                        // (e.g., we asked for "dep-a" but the manifest says
                        // `name: evil-skill`). Reject and clean up the
                        // on-disk write — callers rely on the requested
                        // dep name matching what gets installed.
                        if name != dep_name {
                            let orphan_dir = user_dir.join(&name);
                            if let Err(cleanup_err) = tokio::fs::remove_dir_all(&orphan_dir).await {
                                tracing::debug!(
                                    "chain install: failed to clean up mismatched-name dir {}: {}",
                                    orphan_dir.display(),
                                    cleanup_err
                                );
                            }
                            report.failed.push(format!(
                                "{}: manifest declares name '{}' (dependency-confusion guard)",
                                dep_name, name
                            ));
                            continue;
                        }
                        let nested_required = skill.manifest.requires.skills.clone();
                        // Take the write lock in a tightly scoped block so the
                        // (non-Send) RwLockWriteGuard is dropped before any
                        // subsequent `.await`.
                        enum CommitOutcome {
                            Installed,
                            Duplicate,
                            Failed(String),
                        }
                        let outcome: CommitOutcome = {
                            let mut guard = registry_write(registry);
                            if guard.has(&name) {
                                CommitOutcome::Duplicate
                            } else {
                                match guard.commit_install(&name, skill) {
                                    Ok(()) => CommitOutcome::Installed,
                                    Err(e) => CommitOutcome::Failed(e.to_string()),
                                }
                            }
                        };
                        match outcome {
                            CommitOutcome::Installed => {
                                report.installed.push(name);
                                // Only enqueue nested deps if we still have
                                // attempt budget left — otherwise we grow the
                                // queue for items we'll never fetch, which a
                                // malicious manifest could exploit to blow
                                // out memory on `queued_or_seen`. Belt and
                                // braces: also enforce MAX_CHAIN_QUEUE.
                                if attempted < MAX_CHAIN_DEPS {
                                    for nested_dep in nested_required {
                                        if queue.len() >= MAX_CHAIN_QUEUE {
                                            tracing::warn!(
                                                "chain install: queue hit MAX_CHAIN_QUEUE={}; dropping further nested deps",
                                                MAX_CHAIN_QUEUE
                                            );
                                            break;
                                        }
                                        if queued_or_seen.insert(nested_dep.clone()) {
                                            queue.push_back(nested_dep);
                                        }
                                    }
                                }
                            }
                            CommitOutcome::Duplicate => {
                                // Another concurrent install committed first.
                                // Clean up the on-disk skill dir we just wrote
                                // so it doesn't become an orphan that
                                // drift-monitors will flag later.
                                let orphan_dir = user_dir.join(&name);
                                if let Err(cleanup_err) =
                                    tokio::fs::remove_dir_all(&orphan_dir).await
                                {
                                    tracing::debug!(
                                        "chain install: failed to clean up orphan skill dir {}: {}",
                                        orphan_dir.display(),
                                        cleanup_err
                                    );
                                }
                            }
                            CommitOutcome::Failed(e) => {
                                report.failed.push(format!("{}: {}", dep_name, e))
                            }
                        }
                    }
                    Err(e) => report.failed.push(format!("{}: {}", dep_name, e)),
                }
            }
            Err(e) => {
                if e.is_missing_dependency() {
                    report.missing.push(dep_name);
                } else {
                    report.failed.push(format!("{}: {}", dep_name, e));
                }
            }
        }
    }

    Ok(report)
}

fn build_skill_install_output(
    installed_name: &str,
    report: &ChainInstallReport,
) -> serde_json::Value {
    let status = if report.has_warnings() {
        "installed_with_warnings"
    } else {
        "installed"
    };
    let message = if report.has_warnings() {
        format!(
            "Skill '{}' installed with warnings. It will activate when matching keywords are detected.",
            installed_name
        )
    } else {
        format!(
            "Skill '{}' installed successfully. It will activate when matching keywords are detected.",
            installed_name
        )
    };

    let mut output = serde_json::json!({
        "name": installed_name,
        "status": status,
        "trust": "installed",
        "message": message,
    });

    append_chain_install_report_fields(&mut output, report);

    output
}

fn build_already_installed_output(name: &str, report: &ChainInstallReport) -> serde_json::Value {
    let status = if report.has_warnings() {
        "already_installed_with_warnings"
    } else {
        "already_installed"
    };
    let message = if report.has_warnings() {
        format!(
            "Skill '{}' is already active; dependency installation finished with warnings.",
            name
        )
    } else if report.installed.is_empty() {
        format!("Skill '{}' is already active — no install needed.", name)
    } else {
        format!(
            "Skill '{}' is already active; companion skills were installed.",
            name
        )
    };

    let mut output = serde_json::json!({
        "name": name,
        "status": status,
        "trust": "installed",
        "message": message,
    });

    append_chain_install_report_fields(&mut output, report);

    output
}

fn append_chain_install_report_fields(output: &mut serde_json::Value, report: &ChainInstallReport) {
    if !report.installed.is_empty() {
        output["chain_installed"] = serde_json::json!(&report.installed);
    }
    if !report.failed.is_empty() {
        output["chain_install_failed"] = serde_json::json!(&report.failed);
    }
    if !report.missing.is_empty() {
        output["missing_dependencies"] = serde_json::json!(&report.missing);
        output["missing_dependencies_message"] = serde_json::json!(format!(
            "These required skills could not be found in the catalog and need manual installation: {}",
            report.missing.join(", ")
        ));
    }
    if !report.skipped.is_empty() {
        output["skipped_dependencies"] = serde_json::json!(&report.skipped);
        output["skipped_dependencies_message"] = serde_json::json!(format!(
            "{} dependency chain hit the MAX_CHAIN_DEPS={} *attempt* cap (intentional bound on fetch time from large/malicious manifests). These deps were not attempted and must be installed manually with a follow-up `skill_install` call: {}",
            report.skipped.len(),
            MAX_CHAIN_DEPS,
            report.skipped.join(", ")
        ));
    }
    if !report.pending_explicit_install.is_empty() {
        output["pending_dependency_install"] = serde_json::json!(&report.pending_explicit_install);
        output["pending_dependency_install_message"] = serde_json::json!(format!(
            "Companion skills were not installed automatically. Re-run skill_install with install_dependencies=true to approve installing: {}",
            report.pending_explicit_install.join(", ")
        ));
    }
}

// ── skill_list ──────────────────────────────────────────────────────────

pub struct SkillListTool {
    registry: Arc<std::sync::RwLock<SkillRegistry>>,
}

impl SkillListTool {
    pub fn new(registry: Arc<std::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillListTool {
    fn name(&self) -> &str {
        "skill_list"
    }

    fn description(&self) -> &str {
        "List all loaded skills with their trust level, source, and activation keywords."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "verbose": {
                    "type": "boolean",
                    "description": "Include extra detail (tags, content_hash, version)",
                    "default": false
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let verbose = params
            .get("verbose")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let guard = self
            .registry
            .read()
            .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;

        let skills: Vec<serde_json::Value> = guard
            .skills()
            .iter()
            .map(|s| {
                let mut entry = serde_json::json!({
                    "name": s.manifest.name,
                    "description": s.manifest.description,
                    "trust": s.trust.to_string(),
                    "source": format!("{:?}", s.source),
                    "keywords": s.manifest.activation.keywords,
                });

                if verbose && let Some(obj) = entry.as_object_mut() {
                    obj.insert(
                        "version".to_string(),
                        serde_json::Value::String(s.manifest.version.clone()),
                    );
                    obj.insert(
                        "tags".to_string(),
                        serde_json::json!(s.manifest.activation.tags),
                    );
                    obj.insert(
                        "content_hash".to_string(),
                        serde_json::Value::String(s.content_hash.clone()),
                    );
                    obj.insert(
                        "max_context_tokens".to_string(),
                        serde_json::json!(s.manifest.activation.max_context_tokens),
                    );
                }

                entry
            })
            .collect();

        let output = serde_json::json!({
            "skills": skills,
            "count": skills.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_search ────────────────────────────────────────────────────────

pub struct SkillSearchTool {
    registry: Arc<std::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
}

impl SkillSearchTool {
    pub fn new(
        registry: Arc<std::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
    ) -> Self {
        Self { registry, catalog }
    }
}

#[async_trait]
impl Tool for SkillSearchTool {
    fn name(&self) -> &str {
        "skill_search"
    }

    fn description(&self) -> &str {
        "Search for skills in the ClawHub catalog and among locally loaded skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (name, keyword, or description fragment)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?;

        // Search the ClawHub catalog (async, best-effort)
        let catalog_outcome = self.catalog.search(query).await;
        let catalog_error = catalog_outcome.error.clone();

        // Enrich top results with detail data (stars, downloads, owner)
        let mut catalog_entries = catalog_outcome.results;
        self.catalog
            .enrich_search_results(&mut catalog_entries, 5)
            .await;

        // Search locally loaded skills
        let installed_names: Vec<String> = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .skills()
                .iter()
                .map(|s| s.manifest.name.clone())
                .collect()
        };

        // Mark catalog entries that are already installed
        let catalog_json: Vec<serde_json::Value> = catalog_entries
            .iter()
            .map(|entry| {
                let is_installed =
                    catalog_entry_is_installed(&entry.slug, &entry.name, &installed_names);
                serde_json::json!({
                    "slug": entry.slug,
                    "name": entry.name,
                    "description": entry.description,
                    "version": entry.version,
                    "score": entry.score,
                    "installed": is_installed,
                    "stars": entry.stars,
                    "downloads": entry.downloads,
                    "owner": entry.owner,
                })
            })
            .collect();

        // Find matching local skills (simple substring match)
        let query_lower = query.to_lowercase();
        let local_matches: Vec<serde_json::Value> = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .skills()
                .iter()
                .filter(|s| {
                    s.manifest.name.to_lowercase().contains(&query_lower)
                        || s.manifest.description.to_lowercase().contains(&query_lower)
                        || s.manifest
                            .activation
                            .keywords
                            .iter()
                            .any(|k| k.to_lowercase().contains(&query_lower))
                })
                .map(|s| {
                    serde_json::json!({
                        "name": s.manifest.name,
                        "description": s.manifest.description,
                        "trust": s.trust.to_string(),
                    })
                })
                .collect()
        };

        let mut output = serde_json::json!({
            "catalog": catalog_json,
            "catalog_count": catalog_json.len(),
            "installed": local_matches,
            "installed_count": local_matches.len(),
            "registry_url": self.catalog.registry_url(),
        });
        if let Some(err) = catalog_error {
            output["catalog_error"] = serde_json::Value::String(err);
        }

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_install ───────────────────────────────────────────────────────

pub struct SkillInstallTool {
    registry: Arc<std::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
}

impl SkillInstallTool {
    pub fn new(
        registry: Arc<std::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
    ) -> Self {
        Self { registry, catalog }
    }
}

async fn resolve_catalog_download_key(
    catalog: &SkillCatalog,
    name: &str,
    slug: Option<&str>,
) -> Result<String, ToolError> {
    if let Some(slug) = slug.filter(|s| !s.is_empty()) {
        return Ok(slug.to_string());
    }

    if name.contains('/') {
        return Ok(name.to_string());
    }

    let outcome = catalog.search(name).await;
    match resolve_catalog_slug_for_name(name, &outcome.results) {
        Ok(Some(resolved)) => Ok(resolved),
        Ok(None) => {
            let reason = outcome
                .error
                .unwrap_or_else(|| "no unique catalog match was found".to_string());
            Err(ToolError::ExecutionFailed(format!(
                "Could not resolve skill name '{}' to a catalog slug: {}",
                name, reason
            )))
        }
        Err(e) => Err(ToolError::ExecutionFailed(e.to_string())),
    }
}

#[async_trait]
impl Tool for SkillInstallTool {
    fn name(&self) -> &str {
        "skill_install"
    }

    fn description(&self) -> &str {
        "Install a skill from SKILL.md content, a URL, or by name from the ClawHub catalog."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name or slug (from search results)"
                },
                "slug": {
                    "type": "string",
                    "description": "Registry slug from catalog search results; preferred when installing from ClawHub"
                },
                "url": {
                    "type": "string",
                    "description": "Direct URL to a SKILL.md file"
                },
                "content": {
                    "type": "string",
                    "description": "Raw SKILL.md content to install directly"
                },
                "install_dependencies": {
                    "type": "boolean",
                    "description": "When true, also install companion skills declared in requires.skills. Defaults to false so dependency installs stay explicit in the approved tool call.",
                    "default": false
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;
        let install_dependencies = params
            .get("install_dependencies")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut requested_identifier = params
            .get("slug")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        // Idempotent: if a skill with this name is already loaded (from any
        // source — local SKILL.md, bundled, previously installed), avoid
        // reinstalling the top-level skill. Dependency installs are not a
        // no-op: when explicitly requested, walk the loaded skill's companion
        // list instead of returning early.
        let loaded_required_skills = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            if let Some(loaded_skill) = guard.find_by_name(name) {
                let required_skills = loaded_skill.manifest.requires.skills.clone();
                if install_dependencies && !required_skills.is_empty() {
                    Some(required_skills)
                } else {
                    let report = ChainInstallReport::default();
                    return Ok(ToolOutput::success(
                        build_already_installed_output(name, &report),
                        start.elapsed(),
                    ));
                }
            } else {
                None
            }
        };

        if let Some(required_skills) = loaded_required_skills {
            let chain_report = install_missing_skill_dependencies(
                &self.registry,
                self.catalog.registry_url(),
                required_skills,
                |url| async move { fetch_skill_payload(&url).await },
            )
            .await?;

            return Ok(ToolOutput::success(
                build_already_installed_output(name, &chain_report),
                start.elapsed(),
            ));
        }

        let install_payload = if let Some(raw) = params.get("content").and_then(|v| v.as_str()) {
            // Direct content provided
            SkillInstallPayload {
                skill_md: raw.to_string(),
                ..SkillInstallPayload::default()
            }
        } else if let Some(url) = params
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            // Fetch from explicit URL
            fetch_skill_payload(url).await.map_err(ToolError::from)?
        } else {
            // Look up in catalog and fetch
            let download_key = resolve_catalog_download_key(
                self.catalog.as_ref(),
                name,
                requested_identifier.as_deref(),
            )
            .await?;
            requested_identifier = Some(download_key.clone());
            let download_url = ironclaw_skills::catalog::skill_download_url(
                self.catalog.registry_url(),
                &download_key,
            );
            fetch_skill_payload(&download_url)
                .await
                .map_err(ToolError::from)?
        };

        let normalized = ironclaw_skills::normalize_line_endings(&install_payload.skill_md);

        // Check for duplicates and get install_dir under a brief read lock.
        let (user_dir, skill_name_from_parse, install_content) = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;

            let (skill_name, install_content) =
                ironclaw_skills::registry::SkillRegistry::resolve_install_content(
                    &normalized,
                    requested_identifier.as_deref(),
                )
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            if guard.has(&skill_name) {
                return Err(ToolError::ExecutionFailed(format!(
                    "Skill '{}' already exists",
                    skill_name
                )));
            }

            (
                guard.install_target_dir().to_path_buf(),
                skill_name,
                install_content,
            )
        };

        // Perform async I/O (write to disk, validate round-trip) with no lock held.
        let (skill_name, loaded_skill) =
            ironclaw_skills::registry::SkillRegistry::prepare_install_bundle_to_disk(
                &user_dir,
                &skill_name_from_parse,
                &install_content,
                &install_payload.extra_files,
                install_payload.install_metadata.as_ref(),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        // Commit the in-memory addition under a brief write lock. The
        // earlier `guard.has()` check was under a released read lock with
        // async I/O (prepare_install_to_disk) in between, so we MUST
        // re-check under the write lock to close the TOCTOU window — a
        // concurrent `skill_install` for the same name can finish during
        // the window and leave us double-committing.
        enum CommitResult {
            Installed(String, Vec<String>),
            AlreadyInstalled,
        }
        let commit_result: CommitResult = {
            let mut guard = registry_write(&self.registry);
            if guard.has(&skill_name) {
                CommitResult::AlreadyInstalled
            } else {
                let reqs = loaded_skill.manifest.requires.clone();
                guard
                    .commit_install(&skill_name, loaded_skill)
                    .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
                CommitResult::Installed(skill_name, reqs.skills)
            }
        };

        let (installed_name, required_skills) = match commit_result {
            CommitResult::Installed(name, skills) => (name, skills),
            CommitResult::AlreadyInstalled => {
                // A concurrent install won the race. Clean up the on-disk
                // copy we wrote during `prepare_install_to_disk` so it
                // doesn't become an orphan, then return the idempotent
                // response.
                let orphan_dir = user_dir.join(&skill_name_from_parse);
                if let Err(cleanup_err) = tokio::fs::remove_dir_all(&orphan_dir).await {
                    tracing::debug!(
                        "skill_install: failed to clean up orphan skill dir {}: {}",
                        orphan_dir.display(),
                        cleanup_err
                    );
                }
                return Ok(ToolOutput::success(
                    serde_json::json!({
                        "name": skill_name_from_parse,
                        "status": "already_installed",
                        "trust": "installed",
                        "message": format!(
                            "Skill '{}' was already installed by a concurrent call — no install needed.",
                            skill_name_from_parse
                        ),
                    }),
                    start.elapsed(),
                ));
            }
        };

        let chain_report = if required_skills.is_empty() {
            ChainInstallReport::default()
        } else if !install_dependencies {
            let missing_required_skills = {
                let guard = self
                    .registry
                    .read()
                    .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
                required_skills
                    .into_iter()
                    .filter(|skill| !guard.has(skill))
                    .collect::<Vec<_>>()
            };

            ChainInstallReport {
                pending_explicit_install: missing_required_skills,
                ..Default::default()
            }
        } else {
            install_missing_skill_dependencies(
                &self.registry,
                self.catalog.registry_url(),
                required_skills,
                |url| async move { fetch_skill_payload(&url).await },
            )
            .await?
        };

        let output = build_skill_install_output(&installed_name, &chain_report);

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        let install_deps = params
            .get("install_dependencies")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // No-op shortcut: if a skill with this name is already loaded (bundled,
        // user, workspace, or previously installed), `execute` will return
        // `already_installed` without touching the catalog. Asking for approval
        // on a guaranteed no-op is pure friction, so we mirror the idempotent
        // path here. Dependency-chain installs may still fetch companion
        // skills, so they must go through the approval path below.
        if !install_deps
            && let Some(name) = params.get("name").and_then(|v| v.as_str())
            && !name.is_empty()
            && let Ok(guard) = self.registry.read()
            && guard.has(name)
        {
            return ApprovalRequirement::Never;
        }

        // Chain installs pull up to MAX_CHAIN_DEPS additional skills, each
        // with its own prompt-injection surface. When the LLM sets
        // `install_dependencies=true` we force a per-call approval prompt
        // instead of honoring the auto-approve allowlist — the single
        // `skill_install` approval the user previously granted covered one
        // skill, not an unbounded companion set. Single-skill installs
        // retain the normal `UnlessAutoApproved` behavior so routine flows
        // don't regress.
        if install_deps {
            ApprovalRequirement::Always
        } else {
            ApprovalRequirement::UnlessAutoApproved
        }
    }
}

/// Validate that a URL is safe to fetch (SSRF prevention).
///
/// Rejects:
/// - Non-HTTPS URLs (except in tests)
/// - URLs pointing to private, loopback, or link-local IP addresses
/// - URLs without a host
pub fn validate_fetch_url(url_str: &str) -> Result<reqwest::Url, ToolError> {
    let parsed = reqwest::Url::parse(url_str)
        .map_err(|e| ToolError::ExecutionFailed(format!("Invalid URL '{}': {}", url_str, e)))?;

    // Require HTTPS
    if parsed.scheme() != "https" {
        return Err(ToolError::ExecutionFailed(format!(
            "Only HTTPS URLs are allowed for skill fetching, got scheme '{}'",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host()
        .ok_or_else(|| ToolError::ExecutionFailed("URL has no host".to_string()))?;

    // Check if host is an IP address and reject private ranges.
    // Use reqwest::Url host variants to get proper IpAddr values -- host_str()
    // returns bracketed IPv6 (e.g. "[::1]") which IpAddr cannot parse.
    // Unwrap IPv4-mapped IPv6 addresses (e.g. ::ffff:192.168.1.1) to catch
    // SSRF bypasses that encode private IPv4 addresses as IPv6.
    if let Some(ip) = host_ip_addr(&host) {
        validate_fetch_ip(&ip, &host.to_string())?;
    }

    // Reject common internal hostnames, including FQDN forms with a trailing dot.
    let host_lower = normalize_domain(host.to_string().as_str()).to_lowercase();
    if host_lower == "localhost"
        || host_lower == "metadata.google.internal"
        || host_lower.ends_with(".internal")
        || host_lower.ends_with(".local")
    {
        return Err(ToolError::ExecutionFailed(format!(
            "URL points to an internal hostname: {}",
            host
        )));
    }

    Ok(parsed)
}

fn host_ip_addr(host: &url::Host<&str>) -> Option<std::net::IpAddr> {
    match host {
        url::Host::Ipv4(v4) => Some(std::net::IpAddr::V4(*v4)),
        url::Host::Ipv6(v6) => Some(normalize_ip(std::net::IpAddr::V6(*v6))),
        url::Host::Domain(_) => None,
    }
}

fn normalize_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(std::net::IpAddr::V4)
            .unwrap_or(std::net::IpAddr::V6(v6)),
        other => other,
    }
}

fn validate_fetch_ip(ip: &std::net::IpAddr, display_host: &str) -> Result<(), ToolError> {
    if ip.is_loopback() || ip.is_unspecified() || is_private_ip(ip) || is_link_local_ip(ip) {
        return Err(ToolError::ExecutionFailed(format!(
            "URL points to a private/loopback/link-local address: {}",
            display_host
        )));
    }

    Ok(())
}

fn normalize_domain(host: &str) -> &str {
    host.trim_end_matches('.')
}

fn validate_resolved_addrs(host: &str, addrs: &[std::net::SocketAddr]) -> Result<(), ToolError> {
    if addrs.is_empty() {
        return Err(ToolError::ExecutionFailed(format!(
            "DNS resolution returned no addresses for {}",
            host
        )));
    }

    for addr in addrs {
        let ip = normalize_ip(addr.ip());
        validate_fetch_ip(&ip, host)?;
    }

    Ok(())
}

fn build_fetch_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("ironclaw/0.1")
        .redirect(reqwest::redirect::Policy::none())
}

async fn build_safe_fetch_client(parsed: &reqwest::Url) -> Result<reqwest::Client, ToolError> {
    let host = parsed
        .host()
        .ok_or_else(|| ToolError::ExecutionFailed("URL has no host".to_string()))?;

    match host {
        url::Host::Ipv4(_) | url::Host::Ipv6(_) => build_fetch_client_builder()
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {}", e))),
        url::Host::Domain(domain) => {
            let lookup_host = normalize_domain(domain);
            let port = parsed
                .port_or_known_default()
                .ok_or_else(|| ToolError::ExecutionFailed("URL has no valid port".to_string()))?;

            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((lookup_host, port))
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!(
                        "DNS resolution failed for {}: {}",
                        lookup_host, e
                    ))
                })?
                .collect();

            validate_resolved_addrs(domain, &addrs)?;

            build_fetch_client_builder()
                .resolve_to_addrs(domain, &addrs)
                .build()
                .map_err(|e| ToolError::ExecutionFailed(format!("HTTP client error: {}", e)))
        }
    }
}

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16
            v4.is_private() || v4.is_link_local()
        }
        std::net::IpAddr::V6(v6) => {
            // Unique local (fc00::/7)
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00
        }
    }
}

fn is_link_local_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            // fe80::/10
            let segments = v6.segments();
            (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

fn parse_github_blob_ref(parsed: &reqwest::Url) -> Option<GitHubBlobRequest> {
    if parsed.host_str()? != "github.com" {
        return None;
    }

    let parts: Vec<_> = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
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

fn parse_github_repo_ref(parsed: &reqwest::Url) -> Option<GitHubRepoRequest> {
    if parsed.host_str()? != "github.com" {
        return None;
    }

    let parts: Vec<_> = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
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

async fn fetch_url_bytes(parsed: &reqwest::Url) -> Result<Vec<u8>, SkillFetchError> {
    let client = build_safe_fetch_client(parsed)
        .await
        .map_err(|e| SkillFetchError::from_message(e.to_string()))?;
    let response = client.get(parsed.clone()).send().await.map_err(|e| {
        SkillFetchError::from_message(format!("Failed to fetch skill from {}: {}", parsed, e))
    })?;

    if !response.status().is_success() {
        return Err(SkillFetchError::from_http_status(
            response.status().as_u16(),
            parsed.as_str(),
        ));
    }

    let bytes = response.bytes().await.map_err(|e| {
        SkillFetchError::from_message(format!("Failed to read response body: {}", e))
    })?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return Err(SkillFetchError::from_message(format!(
            "Response too large: {} bytes (max {} bytes)",
            bytes.len(),
            MAX_DOWNLOAD_BYTES
        )));
    }

    Ok(bytes.to_vec())
}

fn build_github_api_base_url(owner: &str, repo: &str) -> Result<reqwest::Url, SkillFetchError> {
    validate_github_repo_components(owner, repo)?;
    validate_derived_fetch_url(&format!("https://api.github.com/repos/{owner}/{repo}"))
}

fn build_github_contents_url(
    owner: &str,
    repo: &str,
    path: Option<&str>,
    git_ref: &str,
) -> Result<reqwest::Url, SkillFetchError> {
    let mut url = build_github_api_base_url(owner, repo)?;
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            SkillFetchError::from_message("Failed to build GitHub contents URL".to_string())
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

async fn fetch_github_api_response(
    url: &reqwest::Url,
    context: &str,
) -> Result<reqwest::Response, SkillFetchError> {
    let client = build_safe_fetch_client(url)
        .await
        .map_err(|e| SkillFetchError::from_message(e.to_string()))?;
    let response = client.get(url.clone()).send().await.map_err(|e| {
        SkillFetchError::from_message(format!("Failed to {context} via {url}: {e}"))
    })?;

    if !response.status().is_success() {
        return Err(SkillFetchError::from_http_status(
            response.status().as_u16(),
            url.as_str(),
        ));
    }

    Ok(response)
}

async fn resolve_github_default_branch(owner: &str, repo: &str) -> Result<String, SkillFetchError> {
    #[derive(serde::Deserialize)]
    struct RepoMetadata {
        default_branch: String,
    }

    let api_url = build_github_api_base_url(owner, repo)?;
    let response = fetch_github_api_response(&api_url, "resolve the default branch").await?;
    let meta = response
        .json::<RepoMetadata>()
        .await
        .map_err(|e| SkillFetchError::from_message(format!("Invalid GitHub repo metadata: {e}")))?;
    if meta.default_branch.trim().is_empty() {
        return Err(SkillFetchError::from_message(
            "GitHub repo metadata did not include a default branch".to_string(),
        ));
    }
    Ok(meta.default_branch)
}

async fn resolve_github_ref_commit_sha(
    owner: &str,
    repo: &str,
    git_ref: &str,
) -> Result<String, SkillFetchError> {
    #[derive(serde::Deserialize)]
    struct CommitSummary {
        sha: String,
    }

    let mut commits_url = build_github_api_base_url(owner, repo)?;
    {
        let mut segments = commits_url.path_segments_mut().map_err(|_| {
            SkillFetchError::from_message("Failed to build GitHub commits URL".to_string())
        })?;
        segments.push("commits");
    }
    commits_url
        .query_pairs_mut()
        .append_pair("sha", git_ref)
        .append_pair("per_page", "1");

    let response = fetch_github_api_response(&commits_url, "resolve the GitHub ref").await?;
    let commits = response.json::<Vec<CommitSummary>>().await.map_err(|e| {
        SkillFetchError::from_message(format!("Invalid GitHub commit metadata: {e}"))
    })?;
    let sha = commits
        .into_iter()
        .next()
        .map(|commit| commit.sha)
        .filter(|sha| !sha.trim().is_empty())
        .ok_or_else(|| {
            SkillFetchError::from_message(format!(
                "GitHub ref '{git_ref}' did not resolve to a commit"
            ))
        })?;

    if !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(SkillFetchError::from_message(format!(
            "GitHub returned an invalid commit SHA for ref '{git_ref}'"
        )));
    }

    Ok(sha)
}

async fn github_ref_path_exists(
    owner: &str,
    repo: &str,
    git_ref: &str,
    path: Option<&str>,
) -> Result<bool, SkillFetchError> {
    let contents_url = build_github_contents_url(owner, repo, path, git_ref)?;
    match fetch_github_api_response(&contents_url, "resolve the GitHub path").await {
        Ok(_) => Ok(true),
        Err(err) if matches!(err.status, Some(404)) => Ok(false),
        Err(err) => Err(err),
    }
}

async fn resolve_github_tree_request(
    repo: GitHubRepoRequest,
) -> Result<GitHubRepoRef, SkillFetchError> {
    validate_github_repo_components(&repo.owner, &repo.repo)?;

    let branch = match repo.tree_segments {
        Some(segments) => {
            if segments.is_empty() {
                return Err(SkillFetchError::from_message(
                    "GitHub tree URL is missing a branch or tag name".to_string(),
                ));
            }

            for split in (1..=segments.len()).rev() {
                let candidate_ref = segments[..split].join("/");
                let candidate_subdir =
                    (split < segments.len()).then(|| segments[split..].join("/"));
                if github_ref_path_exists(
                    &repo.owner,
                    &repo.repo,
                    &candidate_ref,
                    candidate_subdir.as_deref(),
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

            return Err(SkillFetchError::from_message(
                "Could not resolve the GitHub tree URL to a valid ref and subdirectory".to_string(),
            ));
        }
        None => resolve_github_default_branch(&repo.owner, &repo.repo).await?,
    };

    Ok(GitHubRepoRef {
        owner: repo.owner,
        repo: repo.repo,
        branch,
        subdir: None,
    })
}

async fn resolve_github_blob_download_url(
    blob: GitHubBlobRequest,
) -> Result<reqwest::Url, SkillFetchError> {
    #[derive(serde::Deserialize)]
    struct GitHubContentsFile {
        r#type: String,
        download_url: Option<String>,
    }

    validate_github_repo_components(&blob.owner, &blob.repo)?;
    if blob.blob_segments.len() < 2 {
        return Err(SkillFetchError::from_message(
            "GitHub blob URL is missing a ref or file path".to_string(),
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
            match fetch_github_api_response(&contents_url, "resolve the GitHub blob").await {
                Ok(response) => response,
                Err(err) if matches!(err.status, Some(404)) => continue,
                Err(err) => return Err(err),
            };
        let metadata = response.json::<GitHubContentsFile>().await.map_err(|e| {
            SkillFetchError::from_message(format!("Invalid GitHub blob metadata: {e}"))
        })?;
        if metadata.r#type != "file" {
            continue;
        }
        let download_url = metadata.download_url.ok_or_else(|| {
            SkillFetchError::from_message(
                "GitHub blob metadata did not include a raw download URL".to_string(),
            )
        })?;
        return validate_derived_fetch_url(&download_url);
    }

    Err(SkillFetchError::from_message(
        "Could not resolve the GitHub blob URL to a valid ref and file path".to_string(),
    ))
}

fn normalize_archive_path(path: &Path) -> Result<PathBuf, ToolError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "ZIP archive contains unsafe path: {}",
                    path.display()
                )));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(ToolError::ExecutionFailed(
            "ZIP archive entry resolved to empty path".to_string(),
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
) -> Result<ZipSkillBundle, ToolError> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to open ZIP archive: {e}")))?;

    let mut raw_paths = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid ZIP entry: {e}")))?;
        if file.is_dir() {
            continue;
        }
        raw_paths.push(normalize_archive_path(Path::new(file.name()))?);
    }

    let strip_root = strip_common_archive_root(&raw_paths);
    let mut files = Vec::<(PathBuf, Vec<u8>)>::new();
    let mut skill_dirs = HashSet::<PathBuf>::new();
    let mut total_unzipped_bytes = 0u64;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|e| ToolError::ExecutionFailed(format!("Invalid ZIP entry: {e}")))?;
        if file.is_dir() {
            continue;
        }
        // Reject early if the declared size already blows the budget — this is
        // a cheap reject for honestly-labelled archives. The authoritative
        // check is the post-read length comparison below, because a malicious
        // archive can forge `size()` (the `zip` crate surfaces central-directory
        // metadata, which is attacker-controlled).
        if file.size() > MAX_ZIP_ENTRY_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "ZIP entry too large to decompress safely: {}",
                file.name()
            )));
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

        // Cap the actual read at MAX_ZIP_ENTRY_BYTES + 1 so a forged metadata
        // size cannot trick us into materializing an unbounded decompressed
        // payload. If we hit the limit, the next byte exists — the entry is
        // oversized regardless of what the header claims.
        let mut contents = Vec::new();
        (&mut file)
            .take(MAX_ZIP_ENTRY_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to read ZIP entry {}: {e}",
                    path.display()
                ))
            })?;
        if contents.len() as u64 > MAX_ZIP_ENTRY_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "ZIP entry too large to decompress safely: {}",
                entry_name
            )));
        }

        total_unzipped_bytes = total_unzipped_bytes
            .checked_add(contents.len() as u64)
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "ZIP archive decompressed size overflowed safety budget".to_string(),
                )
            })?;
        if total_unzipped_bytes > MAX_TOTAL_UNZIPPED_BYTES {
            return Err(ToolError::ExecutionFailed(format!(
                "ZIP archive expands to {} bytes (max {} bytes)",
                total_unzipped_bytes, MAX_TOTAL_UNZIPPED_BYTES
            )));
        }

        if path.file_name().is_some_and(|name| name == "SKILL.md") {
            skill_dirs.insert(path.parent().unwrap_or(Path::new("")).to_path_buf());
        }
        files.push((path, contents));
    }

    let requested_dir = if let Some(subdir) = requested_subdir {
        let normalized = normalize_archive_path(Path::new(subdir))?;
        if !skill_dirs.contains(&normalized) {
            return Err(ToolError::ExecutionFailed(format!(
                "ZIP archive does not contain SKILL.md under {}",
                normalized.display()
            )));
        }
        normalized
    } else {
        match skill_dirs.len() {
            0 => {
                return Err(ToolError::ExecutionFailed(
                    "ZIP archive does not contain SKILL.md".to_string(),
                ));
            }
            1 => skill_dirs.into_iter().next().unwrap_or_default(),
            _ => {
                let mut dirs = skill_dirs
                    .iter()
                    .map(|dir| dir.display().to_string())
                    .collect::<Vec<_>>();
                dirs.sort();
                return Err(ToolError::ExecutionFailed(format!(
                    "ZIP archive contains multiple skills; specify a subdirectory URL instead: {}",
                    dirs.join(", ")
                )));
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
                return Err(ToolError::ExecutionFailed(format!(
                    "SKILL.md in archive is too large: {} bytes (max {} bytes)",
                    contents.len(),
                    ironclaw_skills::MAX_PROMPT_FILE_SIZE
                )));
            }
            skill_md = Some(String::from_utf8(contents).map_err(|e| {
                ToolError::ExecutionFailed(format!("SKILL.md in archive is not valid UTF-8: {e}"))
            })?);
            continue;
        }
        extra_files.push(ironclaw_skills::registry::InstallFile {
            relative_path: relative.to_path_buf(),
            contents,
        });
    }

    let skill_md = skill_md.ok_or_else(|| {
        ToolError::ExecutionFailed("ZIP archive does not contain SKILL.md".to_string())
    })?;

    Ok(ZipSkillBundle {
        skill_md,
        extra_files,
        bundle_subdir: (!requested_dir.as_os_str().is_empty())
            .then(|| requested_dir.display().to_string()),
    })
}

async fn fetch_github_repo_payload(
    source_url: &str,
    repo_request: GitHubRepoRequest,
) -> Result<SkillInstallPayload, SkillFetchError> {
    let repo = resolve_github_tree_request(repo_request).await?;
    validate_github_repo_ref(&repo)?;
    let commit_sha = resolve_github_ref_commit_sha(&repo.owner, &repo.repo, &repo.branch).await?;

    let archive_url = validate_derived_fetch_url(&format!(
        "https://codeload.github.com/{}/{}/legacy.zip/{}",
        repo.owner, repo.repo, commit_sha
    ))?;
    let bytes = fetch_url_bytes(&archive_url).await?;
    let bundle = extract_skill_bundle_from_zip(&bytes, repo.subdir.as_deref())
        .map_err(|e| SkillFetchError::from_message(e.to_string()))?;

    validate_payload_skill_size(SkillInstallPayload {
        skill_md: bundle.skill_md,
        extra_files: bundle.extra_files,
        install_metadata: Some(ironclaw_skills::registry::InstalledSkillMetadata {
            source_url: Some(source_url.to_string()),
            source_subdir: bundle.bundle_subdir.or(repo.subdir),
        }),
    })
}

pub(crate) async fn fetch_skill_payload(url: &str) -> Result<SkillInstallPayload, SkillFetchError> {
    let parsed =
        validate_fetch_url(url).map_err(|e| SkillFetchError::from_message(e.to_string()))?;

    if let Some(blob) = parse_github_blob_ref(&parsed) {
        let raw_url = resolve_github_blob_download_url(blob).await?;
        let bytes = fetch_url_bytes(&raw_url).await?;
        let skill_md = String::from_utf8(bytes).map_err(|e| {
            SkillFetchError::from_message(format!("Response is not valid UTF-8: {e}"))
        })?;
        return validate_payload_skill_size(SkillInstallPayload {
            skill_md,
            install_metadata: Some(ironclaw_skills::registry::InstalledSkillMetadata {
                source_url: Some(url.to_string()),
                source_subdir: None,
            }),
            ..SkillInstallPayload::default()
        });
    }

    if let Some(repo) = parse_github_repo_ref(&parsed) {
        return fetch_github_repo_payload(url, repo).await;
    }

    let bytes = fetch_url_bytes(&parsed).await?;
    let payload = if bytes.starts_with(b"PK\x03\x04") {
        let bundle = extract_skill_bundle_from_zip(&bytes, None)
            .map_err(|e| SkillFetchError::from_message(e.to_string()))?;
        SkillInstallPayload {
            skill_md: bundle.skill_md,
            extra_files: bundle.extra_files,
            ..SkillInstallPayload::default()
        }
    } else {
        SkillInstallPayload {
            skill_md: String::from_utf8(bytes).map_err(|e| {
                SkillFetchError::from_message(format!("Response is not valid UTF-8: {e}"))
            })?,
            ..SkillInstallPayload::default()
        }
    };

    validate_payload_skill_size(payload)
}

#[allow(dead_code)]
/// Backward-compatible wrapper used by older tests that only care about SKILL.md.
pub(crate) async fn fetch_skill_content(url: &str) -> Result<String, SkillFetchError> {
    Ok(fetch_skill_payload(url).await?.skill_md)
}

#[allow(dead_code)]
/// Extract `SKILL.md` from a ZIP archive returned by the ClawHub download API.
fn extract_skill_from_zip(data: &[u8]) -> Result<String, ToolError> {
    Ok(extract_skill_bundle_from_zip(data, None)?.skill_md)
}

// ── skill_remove ────────────────────────────────────────────────────────

pub struct SkillRemoveTool {
    registry: Arc<std::sync::RwLock<SkillRegistry>>,
}

impl SkillRemoveTool {
    pub fn new(registry: Arc<std::sync::RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillRemoveTool {
    fn name(&self) -> &str {
        "skill_remove"
    }

    fn description(&self) -> &str {
        "Permanently remove an installed skill from disk. This action cannot be undone — \
         the skill files will be deleted."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to remove"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let name = require_str(&params, "name")?;

        // Validate removal and get the filesystem path under a brief read lock.
        let skill_path = {
            let guard = self
                .registry
                .read()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .validate_remove(name)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
        };

        // Delete files from disk (async I/O, no lock held).
        ironclaw_skills::registry::SkillRegistry::delete_skill_files(&skill_path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        // Remove from in-memory registry under a brief write lock.
        {
            let mut guard = self
                .registry
                .write()
                .map_err(|e| ToolError::ExecutionFailed(format!("Lock poisoned: {}", e)))?;
            guard
                .commit_remove(name)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        }

        let output = serde_json::json!({
            "name": name,
            "status": "removed",
            "message": format!("Skill '{}' has been removed.", name),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V1Only
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_registry() -> Arc<std::sync::RwLock<SkillRegistry>> {
        let dir = tempfile::tempdir().unwrap();
        // Keep the tempdir so it lives for the test duration
        let path = dir.keep();
        Arc::new(std::sync::RwLock::new(SkillRegistry::new(path)))
    }

    fn test_catalog() -> Arc<SkillCatalog> {
        Arc::new(SkillCatalog::with_url("http://127.0.0.1:1"))
    }

    fn skill_content(name: &str, required_skills: &[&str]) -> String {
        let requires_block = if required_skills.is_empty() {
            String::new()
        } else {
            let skills = required_skills
                .iter()
                .map(|skill| format!("    - {}", skill))
                .collect::<Vec<_>>()
                .join("\n");
            format!("requires:\n  skills:\n{}\n", skills)
        };

        format!(
            "---\nname: {name}\ndescription: {name} description\n{requires_block}---\n\n#{name}\n"
        )
    }

    #[test]
    fn test_skill_list_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillListTool::new(test_registry());
        assert_eq!(tool.name(), "skill_list");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
    }

    #[test]
    fn test_skill_search_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillSearchTool::new(test_registry(), test_catalog());
        assert_eq!(tool.name(), "skill_search");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("query").is_some());
    }

    #[test]
    fn test_skill_install_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillInstallTool::new(test_registry(), test_catalog());
        assert_eq!(tool.name(), "skill_install");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        assert!(schema["properties"].get("slug").is_some());
        assert!(schema["properties"].get("url").is_some());
        assert!(schema["properties"].get("content").is_some());
    }

    /// Regression: when a persona bundle is already loaded (via bundled
    /// content, user dir, or prior install), an LLM force-activation like
    /// `/ceo-setup` used to trigger a redundant `skill_install` call which
    /// tripped the approval prompt even though `execute` would immediately
    /// return `already_installed`. The shortcut in `requires_approval`
    /// prevents that user-visible friction.
    #[tokio::test]
    async fn skill_install_skips_approval_when_already_loaded() {
        use crate::tools::tool::ApprovalRequirement;

        let registry = test_registry();
        let (name, loaded) = {
            let dir = registry.read().unwrap().install_target_dir().to_path_buf();
            SkillRegistry::prepare_install_to_disk(
                &dir,
                "ceo-setup",
                &skill_content("ceo-setup", &[]),
            )
            .await
            .expect("prepare should succeed")
        };
        registry
            .write()
            .unwrap()
            .commit_install(&name, loaded)
            .expect("commit should succeed");
        let tool = SkillInstallTool::new(Arc::clone(&registry), test_catalog());

        assert_eq!(
            tool.requires_approval(&serde_json::json!({"name": "ceo-setup"})),
            ApprovalRequirement::Never,
            "already-loaded skills must not prompt for approval"
        );

        // Sanity: an unknown name still follows the normal gated path.
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"name": "not-loaded"})),
            ApprovalRequirement::UnlessAutoApproved,
        );

        // Sanity: install_dependencies=true still forces approval even when
        // the top-level skill is loaded, because companions may not be.
        assert_eq!(
            tool.requires_approval(&serde_json::json!({
                "name": "ceo-setup",
                "install_dependencies": true,
            })),
            ApprovalRequirement::Always,
            "install_dependencies=true must prompt even if the main skill is loaded",
        );
    }

    #[tokio::test]
    async fn skill_install_execute_honors_dependencies_when_already_loaded() {
        let registry = test_registry();
        let (name, loaded) = {
            let dir = registry.read().unwrap().install_target_dir().to_path_buf();
            SkillRegistry::prepare_install_to_disk(
                &dir,
                "bundle",
                &skill_content("bundle", &["dep-a"]),
            )
            .await
            .expect("prepare should succeed")
        };
        registry
            .write()
            .unwrap()
            .commit_install(&name, loaded)
            .expect("commit should succeed");
        let tool = SkillInstallTool::new(Arc::clone(&registry), test_catalog());

        let output = tool
            .execute(
                serde_json::json!({
                    "name": "bundle",
                    "install_dependencies": true,
                }),
                &JobContext::default(),
            )
            .await
            .expect("execute should succeed even when dependency fetch fails");

        assert_eq!(output.result["status"], "already_installed_with_warnings");
        let failures = output.result["chain_install_failed"]
            .as_array()
            .expect("dependency install should have been attempted");
        let failure = failures[0].as_str().unwrap();
        assert!(failure.starts_with("dep-a:"));
        assert!(failure.contains("Only HTTPS URLs are allowed"));

        let guard = registry.read().unwrap();
        assert!(guard.has("bundle"));
        assert!(!guard.has("dep-a"));
    }

    #[test]
    fn test_find_catalog_slug_for_display_name() {
        let entries = vec![ironclaw_skills::catalog::CatalogEntry {
            slug: "finance/mortgage-calculator".to_string(),
            name: "Mortgage Calculator".to_string(),
            description: String::new(),
            version: String::new(),
            score: 1.0,
            updated_at: None,
            stars: None,
            downloads: None,
            installs_current: None,
            owner: None,
        }];

        assert_eq!(
            resolve_catalog_slug_for_name("Mortgage Calculator", &entries)
                .unwrap()
                .as_deref(),
            Some("finance/mortgage-calculator")
        );
        assert_eq!(
            resolve_catalog_slug_for_name("mortgage-calculator", &entries)
                .unwrap()
                .as_deref(),
            Some("finance/mortgage-calculator")
        );
    }

    #[test]
    fn test_resolve_catalog_slug_for_display_name_is_ambiguous() {
        let entries = vec![
            ironclaw_skills::catalog::CatalogEntry {
                slug: "alice/mortgage-calculator".to_string(),
                name: "Mortgage Calculator".to_string(),
                description: String::new(),
                version: String::new(),
                score: 1.0,
                updated_at: None,
                stars: None,
                downloads: None,
                installs_current: None,
                owner: None,
            },
            ironclaw_skills::catalog::CatalogEntry {
                slug: "bob/mortgage-calculator".to_string(),
                name: "Mortgage Calculator".to_string(),
                description: String::new(),
                version: String::new(),
                score: 0.9,
                updated_at: None,
                stars: None,
                downloads: None,
                installs_current: None,
                owner: None,
            },
        ];

        assert!(resolve_catalog_slug_for_name("Mortgage Calculator", &entries).is_err());
    }

    #[test]
    fn test_skill_remove_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillRemoveTool::new(test_registry());
        assert_eq!(tool.name(), "skill_remove");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
    }

    #[test]
    fn skill_remove_always_requires_approval_regardless_of_params() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = SkillRemoveTool::new(test_registry());

        let test_cases = vec![
            ("no params", serde_json::json!({})),
            ("empty name", serde_json::json!({"name": ""})),
            (
                "deployment skill",
                serde_json::json!({"name": "deployment"}),
            ),
            ("custom skill", serde_json::json!({"name": "custom-skill"})),
            (
                "with extra fields",
                serde_json::json!({"name": "skill", "extra": "field"}),
            ),
        ];

        for (case_name, params) in test_cases {
            assert_eq!(
                tool.requires_approval(&params),
                ApprovalRequirement::Always,
                "skill_remove must always require approval for case: {}",
                case_name
            );
        }
    }

    #[test]
    fn test_validate_fetch_url_allows_https() {
        assert!(super::validate_fetch_url("https://clawhub.ai/api/v1/download?slug=foo").is_ok());
    }

    #[test]
    fn test_validate_fetch_url_rejects_http() {
        let err = super::validate_fetch_url("http://example.com/skill.md").unwrap_err();
        assert!(err.to_string().contains("Only HTTPS"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_private_ip() {
        let err = super::validate_fetch_url("https://192.168.1.1/skill.md").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_loopback() {
        let err = super::validate_fetch_url("https://127.0.0.1/skill.md").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_localhost() {
        let err = super::validate_fetch_url("https://localhost/skill.md").unwrap_err();
        assert!(err.to_string().contains("internal hostname"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_localhost_fqdn() {
        let err = super::validate_fetch_url("https://localhost./skill.md").unwrap_err();
        assert!(err.to_string().contains("internal hostname"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_metadata_endpoint() {
        let err =
            super::validate_fetch_url("https://169.254.169.254/latest/meta-data/").unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_internal_domain() {
        let err =
            super::validate_fetch_url("https://metadata.google.internal/something").unwrap_err();
        assert!(err.to_string().contains("internal hostname"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_file_scheme() {
        let err = super::validate_fetch_url("file:///etc/passwd").unwrap_err();
        assert!(err.to_string().contains("Only HTTPS"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_ipv4_mapped_ipv6_loopback() {
        let err = super::validate_fetch_url("https://[::ffff:127.0.0.1]/skill.md").unwrap_err();
        assert!(err.to_string().contains("private") || err.to_string().contains("loopback"));
    }

    #[test]
    fn test_validate_fetch_url_rejects_ipv6_loopback() {
        let err = super::validate_fetch_url("https://[::1]/skill.md").unwrap_err();
        assert!(err.to_string().contains("private") || err.to_string().contains("loopback"));
    }

    #[test]
    fn test_parse_github_blob_ref_preserves_slashed_ref_segments() {
        let parsed = reqwest::Url::parse(
            "https://github.com/nearai/ironclaw/blob/feature/foo/skills/demo/SKILL.md",
        )
        .unwrap();

        let blob = super::parse_github_blob_ref(&parsed).expect("blob ref");
        assert_eq!(blob.owner, "nearai");
        assert_eq!(blob.repo, "ironclaw");
        assert_eq!(
            blob.blob_segments,
            vec!["feature", "foo", "skills", "demo", "SKILL.md"]
        );
    }

    #[test]
    fn test_parse_github_repo_ref_preserves_slashed_tree_segments() {
        let parsed =
            reqwest::Url::parse("https://github.com/nearai/ironclaw/tree/feature/foo/skills/demo")
                .unwrap();

        let repo = super::parse_github_repo_ref(&parsed).expect("repo ref");
        assert_eq!(repo.owner, "nearai");
        assert_eq!(repo.repo, "ironclaw");
        assert_eq!(
            repo.tree_segments,
            Some(vec![
                "feature".to_string(),
                "foo".to_string(),
                "skills".to_string(),
                "demo".to_string(),
            ])
        );
    }

    #[test]
    fn test_validate_github_repo_components_rejects_unsafe_segments() {
        let err = super::validate_github_repo_components("nearai", "../ironclaw").unwrap_err();
        assert!(err.to_string().contains("Invalid GitHub repository"));
    }

    #[test]
    fn test_validate_resolved_addrs_rejects_loopback_hostname() {
        let addrs = vec![
            "127.0.0.1:443".parse::<std::net::SocketAddr>().unwrap(),
            "[::1]:443".parse::<std::net::SocketAddr>().unwrap(),
        ];

        let err = super::validate_resolved_addrs("example.com", &addrs).unwrap_err();
        assert!(err.to_string().contains("private") || err.to_string().contains("loopback"));
    }

    #[test]
    fn test_validate_resolved_addrs_allows_public_hostname() {
        let addrs = vec![
            "8.8.8.8:443".parse::<std::net::SocketAddr>().unwrap(),
            "[2606:4700:4700::1111]:443"
                .parse::<std::net::SocketAddr>()
                .unwrap(),
        ];

        assert!(super::validate_resolved_addrs("example.com", &addrs).is_ok());
    }

    #[test]
    fn test_extract_skill_from_zip_deflate() {
        let skill_md = b"---\nname: test\n---\n# Test Skill\n";
        let zip = build_zip_archive(&[("SKILL.md", skill_md)], zip::CompressionMethod::Deflated);

        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, "---\nname: test\n---\n# Test Skill\n");
    }

    #[test]
    fn test_extract_skill_from_zip_store() {
        let skill_md = b"---\nname: stored\n---\n# Stored\n";
        let zip = build_zip_archive(&[("SKILL.md", skill_md)], zip::CompressionMethod::Stored);

        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, "---\nname: stored\n---\n# Stored\n");
    }

    #[test]
    fn test_extract_skill_bundle_from_github_repo_zip() {
        use std::io::Write;

        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file(
                "Pika-Skills-main/pikastream-video-meeting/SKILL.md",
                options,
            )
            .unwrap();
        writer
            .write_all(b"---\nname: pikastream-video-meeting\n---\n\n# Skill\n")
            .unwrap();
        writer
            .start_file(
                "Pika-Skills-main/pikastream-video-meeting/requirements.txt",
                options,
            )
            .unwrap();
        writer.write_all(b"requests>=2.32.5\n").unwrap();
        writer
            .start_file(
                "Pika-Skills-main/pikastream-video-meeting/scripts/run.py",
                options,
            )
            .unwrap();
        writer.write_all(b"print('ok')\n").unwrap();
        let zip = writer.finish().unwrap().into_inner();

        let bundle = super::extract_skill_bundle_from_zip(&zip, None).unwrap();
        assert_eq!(
            bundle.bundle_subdir.as_deref(),
            Some("pikastream-video-meeting")
        );
        assert!(bundle.skill_md.contains("pikastream-video-meeting"));
        assert_eq!(bundle.extra_files.len(), 2);
        assert!(
            bundle
                .extra_files
                .iter()
                .any(|f| f.relative_path == Path::new("requirements.txt"))
        );
        assert!(
            bundle
                .extra_files
                .iter()
                .any(|f| f.relative_path == Path::new("scripts/run.py"))
        );
    }

    #[test]
    fn test_extract_skill_bundle_from_zip_rejects_multiple_skills_without_subdir() {
        use std::io::Write;

        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("bundle-main/skill-a/SKILL.md", options)
            .unwrap();
        writer.write_all(b"---\nname: skill-a\n---\n").unwrap();
        writer
            .start_file("bundle-main/skill-b/SKILL.md", options)
            .unwrap();
        writer.write_all(b"---\nname: skill-b\n---\n").unwrap();
        let zip = writer.finish().unwrap().into_inner();

        let err = super::extract_skill_bundle_from_zip(&zip, None).unwrap_err();
        assert!(
            err.to_string().contains("multiple skills"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_extract_skill_bundle_from_zip_rejects_large_total_unzipped_size() {
        use std::io::Write;

        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("bundle-main/skill-a/SKILL.md", options)
            .unwrap();
        writer
            .write_all(b"---\nname: skill-a\n---\n\nPrompt\n")
            .unwrap();
        for idx in 0..11 {
            writer
                .start_file(format!("bundle-main/skill-a/blob-{idx}.bin"), options)
                .unwrap();
            writer.write_all(&vec![b'x'; 2 * 1024 * 1024]).unwrap();
        }
        let zip = writer.finish().unwrap().into_inner();

        let err = super::extract_skill_bundle_from_zip(&zip, Some("skill-a")).unwrap_err();
        assert!(
            err.to_string().contains("expands to"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_extract_skill_bundle_from_zip_rejects_oversized_skill_md() {
        use std::io::Write;

        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("bundle-main/skill-a/SKILL.md", options)
            .unwrap();
        writer
            .write_all(&vec![
                b'a';
                (ironclaw_skills::MAX_PROMPT_FILE_SIZE as usize) + 1
            ])
            .unwrap();
        let zip = writer.finish().unwrap().into_inner();

        let err = super::extract_skill_bundle_from_zip(&zip, Some("skill-a")).unwrap_err();
        assert!(
            err.to_string().contains("SKILL.md in archive is too large"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_chain_install_recurses_into_transitive_skill_dependencies() {
        let registry = test_registry();
        let registry_url = "https://clawhub.example";

        let dep_a_url = ironclaw_skills::catalog::skill_download_url(registry_url, "dep-a");
        let dep_b_url = ironclaw_skills::catalog::skill_download_url(registry_url, "dep-b");

        let responses = Arc::new(HashMap::from([
            (dep_a_url, skill_content("dep-a", &["dep-b"])),
            (dep_b_url, skill_content("dep-b", &[])),
        ]));

        let report = install_missing_skill_dependencies(
            &registry,
            registry_url,
            vec!["dep-a".to_string()],
            {
                let responses = Arc::clone(&responses);
                move |url| {
                    let responses = Arc::clone(&responses);
                    async move {
                        responses
                            .get(&url)
                            .map(|skill_md| SkillInstallPayload {
                                skill_md: skill_md.clone(),
                                ..SkillInstallPayload::default()
                            })
                            .ok_or_else(|| SkillFetchError::from_http_status(404, &url))
                    }
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(
            report.installed,
            vec!["dep-a".to_string(), "dep-b".to_string()]
        );
        assert!(report.failed.is_empty());
        assert!(report.missing.is_empty());
        assert!(report.skipped.is_empty());

        let guard = registry.read().unwrap();
        assert!(guard.has("dep-a"));
        assert!(guard.has("dep-b"));
    }

    #[tokio::test]
    async fn test_chain_install_round_trip_picks_up_pending_deps() {
        // Regression test for PR #1736 review (serrrfirat, 3058525543):
        // verify that the `install_dependencies=false` → `install_dependencies=true`
        // two-step flow actually picks up the pending deps on the second
        // call.
        //
        // Setup: a bundle skill `bundle` declaring two companions `dep-a`
        // and `dep-b`. We install `bundle` itself into the registry first
        // (simulating the outcome of the first `skill_install` call with
        // `install_dependencies=false`, where the tool would record
        // `pending_dependency_install=[dep-a, dep-b]`). We then call
        // `install_missing_skill_dependencies` with that pending list — the
        // same code path the second `skill_install(install_dependencies=true)`
        // would take — and assert the deps are now installed.
        let registry = test_registry();
        let registry_url = "https://clawhub.example";

        let (bundle_name, bundle_loaded) = {
            let content = skill_content("bundle", &["dep-a", "dep-b"]);
            let dir = registry.read().unwrap().install_target_dir().to_path_buf();
            SkillRegistry::prepare_install_to_disk(&dir, "bundle", &content)
                .await
                .unwrap()
        };
        registry
            .write()
            .unwrap()
            .commit_install(&bundle_name, bundle_loaded)
            .unwrap();

        // After the first "install" the bundle is present but the companions are not.
        {
            let guard = registry.read().unwrap();
            assert!(guard.has("bundle"));
            assert!(!guard.has("dep-a"));
            assert!(!guard.has("dep-b"));
        }

        // Second "install": re-drive with the pending list via the helper
        // that `SkillInstallTool::execute` uses when `install_dependencies=true`.
        let dep_a_url = ironclaw_skills::catalog::skill_download_url(registry_url, "dep-a");
        let dep_b_url = ironclaw_skills::catalog::skill_download_url(registry_url, "dep-b");
        let responses = Arc::new(HashMap::from([
            (dep_a_url, skill_content("dep-a", &[])),
            (dep_b_url, skill_content("dep-b", &[])),
        ]));

        let report = install_missing_skill_dependencies(
            &registry,
            registry_url,
            vec!["dep-a".to_string(), "dep-b".to_string()],
            {
                let responses = Arc::clone(&responses);
                move |url| {
                    let responses = Arc::clone(&responses);
                    async move {
                        responses
                            .get(&url)
                            .map(|skill_md| SkillInstallPayload {
                                skill_md: skill_md.clone(),
                                ..SkillInstallPayload::default()
                            })
                            .ok_or_else(|| SkillFetchError::from_http_status(404, &url))
                    }
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(
            report.installed,
            vec!["dep-a".to_string(), "dep-b".to_string()]
        );
        assert!(report.failed.is_empty());
        assert!(report.missing.is_empty());
        assert!(report.skipped.is_empty());
        assert!(report.pending_explicit_install.is_empty());

        let guard = registry.read().unwrap();
        assert!(guard.has("bundle"));
        assert!(guard.has("dep-a"));
        assert!(guard.has("dep-b"));
    }

    #[tokio::test]
    async fn test_chain_install_treats_http_404_as_missing_dependency() {
        let registry = test_registry();
        let report = install_missing_skill_dependencies(
            &registry,
            "https://clawhub.example",
            vec!["missing-skill".to_string()],
            |url| async move { Err(SkillFetchError::from_http_status(404, &url)) },
        )
        .await
        .unwrap();

        assert!(report.installed.is_empty());
        assert!(report.failed.is_empty());
        assert_eq!(report.missing, vec!["missing-skill".to_string()]);
        assert!(report.skipped.is_empty());
    }

    #[tokio::test]
    async fn test_chain_install_rejects_invalid_dependency_names() {
        let registry = test_registry();
        let report = install_missing_skill_dependencies(
            &registry,
            "https://clawhub.example",
            vec!["../../escape".to_string()],
            |_url| async move { Ok(SkillInstallPayload::default()) },
        )
        .await
        .unwrap();

        assert!(report.installed.is_empty());
        assert!(report.missing.is_empty());
        assert!(report.skipped.is_empty());
        assert_eq!(
            report.failed,
            vec!["../../escape: invalid skill dependency name".to_string()]
        );
    }

    #[test]
    fn test_build_skill_install_output_uses_warning_status_for_partial_failures() {
        let report = ChainInstallReport {
            installed: vec!["dep-a".to_string()],
            failed: vec!["dep-b: network error".to_string()],
            missing: vec!["dep-c".to_string()],
            skipped: vec!["dep-d".to_string()],
            pending_explicit_install: Vec::new(),
        };

        let output = build_skill_install_output("bundle", &report);

        assert_eq!(output["status"], "installed_with_warnings");
        assert_eq!(output["chain_installed"], serde_json::json!(["dep-a"]));
        assert_eq!(output["missing_dependencies"], serde_json::json!(["dep-c"]));
        assert_eq!(output["skipped_dependencies"], serde_json::json!(["dep-d"]));
    }

    #[test]
    fn test_build_skill_install_output_reports_pending_dependency_install() {
        let report = ChainInstallReport {
            pending_explicit_install: vec!["dep-a".to_string(), "dep-b".to_string()],
            ..Default::default()
        };

        let output = build_skill_install_output("bundle", &report);

        assert_eq!(output["status"], "installed_with_warnings");
        assert_eq!(
            output["pending_dependency_install"],
            serde_json::json!(["dep-a", "dep-b"])
        );
    }

    #[test]
    fn test_extract_skill_from_zip_missing_skill_md() {
        let zip = build_zip_archive(&[("_meta.json", b"{}")], zip::CompressionMethod::Stored);

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(err.to_string().contains("does not contain SKILL.md"));
    }

    // ── ZIP extraction security regression tests ────────────────────────

    fn build_zip_archive(
        entries: &[(&str, &[u8])],
        compression: zip::CompressionMethod,
    ) -> Vec<u8> {
        use std::io::Write;

        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default().compression_method(compression);
        for (file_name, content) in entries {
            writer.start_file(*file_name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn build_zip_entry_store(file_name: &str, content: &[u8]) -> Vec<u8> {
        build_zip_archive(&[(file_name, content)], zip::CompressionMethod::Stored)
    }

    #[test]
    fn test_zip_extract_valid_skill() {
        let content = b"---\nname: hello\n---\n# Hello Skill\nDoes things.\n";
        let zip = build_zip_entry_store("SKILL.md", content);
        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, std::str::from_utf8(content).unwrap());
    }

    #[test]
    fn test_zip_extract_ignores_non_skill_entries() {
        // ZIP with README.md and src/main.rs but no SKILL.md -- should error.
        let zip = build_zip_archive(
            &[("README.md", b"# Readme"), ("src/main.rs", b"fn main() {}")],
            zip::CompressionMethod::Stored,
        );

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(
            err.to_string().contains("does not contain SKILL.md"),
            "Expected 'does not contain SKILL.md' error, got: {}",
            err
        );
    }

    #[test]
    fn test_zip_extract_path_traversal_rejected() {
        // Parent components are invalid and must be rejected during path normalization.
        let content = b"---\nname: evil\n---\n# Malicious path traversal\n";
        let zip = build_zip_entry_store("../../SKILL.md", content);

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(
            err.to_string().contains("unsafe path"),
            "Path traversal entry should be rejected during normalization, got: {}",
            err
        );
    }

    #[test]
    fn test_zip_extract_nested_single_skill_supported() {
        // A ZIP containing a single nested skill directory should still extract SKILL.md.
        let content = b"---\nname: nested\n---\n# Nested\n";
        let zip = build_zip_entry_store("subdir/SKILL.md", content);

        let result = super::extract_skill_from_zip(&zip).unwrap();
        assert_eq!(result, std::str::from_utf8(content).unwrap());
    }

    #[test]
    fn test_zip_extract_oversized_rejected() {
        let oversized_body = vec![b'x'; (super::MAX_ZIP_ENTRY_BYTES as usize) + 1];
        let zip = build_zip_archive(
            &[("blob.bin", oversized_body.as_slice())],
            zip::CompressionMethod::Stored,
        );

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(
            err.to_string()
                .contains("ZIP entry too large to decompress safely"),
            "Oversized entry should be rejected, got: {}",
            err
        );
    }

    #[test]
    fn test_zip_extract_forged_metadata_rejected() {
        // A malicious ZIP can report a tiny `uncompressed_size` in the central
        // directory (which `ZipFile::size()` exposes) while shipping a much
        // larger actual payload that `read_to_end()` still yields. The
        // extractor's decompression budget must be enforced against the
        // *actual* bytes read, not the attacker-controlled metadata —
        // otherwise a few KB of archive can expand to arbitrary memory.
        //
        // Build an honest oversized Stored ZIP, then rewrite the
        // `uncompressed_size` fields in both the local file header and the
        // central-directory header to claim 10 bytes.
        let oversized_body = vec![b'A'; (super::MAX_ZIP_ENTRY_BYTES as usize) + 1];
        let mut zip = build_zip_archive(
            &[("SKILL.md", oversized_body.as_slice())],
            zip::CompressionMethod::Stored,
        );

        // Local file header signature 0x04034b50 (little-endian 50 4B 03 04);
        // uncompressed_size is at offset 22 within the header.
        let lfh_sig: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
        let lfh_offset = zip
            .windows(4)
            .position(|w| w == lfh_sig)
            .expect("local file header not found");
        zip[lfh_offset + 22..lfh_offset + 26].copy_from_slice(&10u32.to_le_bytes());

        // Central-directory file header signature 0x02014b50; uncompressed_size
        // is at offset 24 within the header. `ZipFile::size()` reads from here.
        let cdh_sig: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
        let cdh_offset = zip
            .windows(4)
            .position(|w| w == cdh_sig)
            .expect("central-directory header not found");
        zip[cdh_offset + 24..cdh_offset + 28].copy_from_slice(&10u32.to_le_bytes());

        let err = super::extract_skill_from_zip(&zip).unwrap_err();
        assert!(
            err.to_string()
                .contains("ZIP entry too large to decompress safely"),
            "Forged-metadata ZIP must be rejected based on actual bytes read, got: {}",
            err
        );
    }

    // ── SSRF prevention regression tests ────────────────────────────────

    #[test]
    fn test_is_private_ip_blocks_loopback() {
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        // is_private_ip checks v4.is_private() which does NOT include loopback,
        // but validate_fetch_url checks is_loopback() separately. Test the full flow.
        assert!(loopback.is_loopback());
        // Also verify via validate_fetch_url
        assert!(super::validate_fetch_url("https://127.0.0.1/skill.md").is_err());
    }

    #[test]
    fn test_is_private_ip_blocks_private_ranges() {
        let cases: Vec<(&str, bool)> = vec![
            ("10.0.0.1", true),
            ("10.255.255.255", true),
            ("172.16.0.1", true),
            ("172.31.255.255", true),
            ("192.168.1.1", true),
            ("192.168.0.0", true),
        ];
        for (ip_str, expect_private) in cases {
            let ip: std::net::IpAddr = ip_str.parse().unwrap();
            assert_eq!(
                super::is_private_ip(&ip),
                expect_private,
                "Expected is_private_ip({}) = {}",
                ip_str,
                expect_private
            );
        }
    }

    #[test]
    fn test_is_private_ip_blocks_link_local() {
        // 169.254.0.0/16 range (link-local)
        let cases = vec!["169.254.1.1", "169.254.0.1", "169.254.255.255"];
        for ip_str in cases {
            let ip: std::net::IpAddr = ip_str.parse().unwrap();
            // is_private_ip includes v4.is_link_local()
            assert!(
                super::is_private_ip(&ip),
                "Expected is_private_ip({}) = true (link-local)",
                ip_str
            );
        }
    }

    #[test]
    fn test_is_private_ip_allows_public() {
        let public_ips = vec!["8.8.8.8", "1.1.1.1", "93.184.216.34", "151.101.1.67"];
        for ip_str in public_ips {
            let ip: std::net::IpAddr = ip_str.parse().unwrap();
            assert!(
                !super::is_private_ip(&ip),
                "Expected is_private_ip({}) = false (public IP)",
                ip_str
            );
            assert!(!ip.is_loopback(), "Expected {} is not loopback", ip_str);
        }
    }

    #[test]
    fn test_is_private_ip_blocks_ipv4_mapped_ipv6() {
        // Test the IPv4-mapped unwrapping logic end-to-end through
        // validate_fetch_url. IPv6 URLs like https://[::ffff:127.0.0.1]/path
        // must be correctly detected as private/loopback.

        // ::ffff:127.0.0.1 mapped -> 127.0.0.1 (loopback) -- must be blocked
        let err = super::validate_fetch_url("https://[::ffff:127.0.0.1]/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("loopback"),
            "IPv4-mapped loopback should be blocked, got: {}",
            err
        );

        // ::ffff:192.168.1.1 mapped -> 192.168.1.1 (private) -- must be blocked
        let err = super::validate_fetch_url("https://[::ffff:192.168.1.1]/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("loopback"),
            "IPv4-mapped private should be blocked, got: {}",
            err
        );

        // ::ffff:10.0.0.1 mapped -> 10.0.0.1 (private) -- must be blocked
        let err = super::validate_fetch_url("https://[::ffff:10.0.0.1]/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("loopback"),
            "IPv4-mapped 10.x should be blocked, got: {}",
            err
        );

        // ::ffff:8.8.8.8 mapped -> 8.8.8.8 (public) -- must be allowed
        assert!(
            super::validate_fetch_url("https://[::ffff:8.8.8.8]/skill.md").is_ok(),
            "IPv4-mapped public IP should be allowed"
        );

        // Pure IPv6 loopback ::1 -- must be blocked
        let err = super::validate_fetch_url("https://[::1]/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("loopback"),
            "IPv6 loopback should be blocked, got: {}",
            err
        );
    }

    #[test]
    fn test_is_restricted_host_blocks_metadata() {
        // Cloud metadata endpoint (AWS/GCP/Azure style)
        let err =
            super::validate_fetch_url("https://169.254.169.254/latest/meta-data/").unwrap_err();
        assert!(
            err.to_string().contains("private") || err.to_string().contains("link-local"),
            "Metadata IP should be blocked, got: {}",
            err
        );

        // GCP metadata hostname
        let err =
            super::validate_fetch_url("https://metadata.google.internal/something").unwrap_err();
        assert!(
            err.to_string().contains("internal hostname"),
            "metadata.google.internal should be blocked, got: {}",
            err
        );

        // Generic .internal domain
        let err = super::validate_fetch_url("https://service.internal/api").unwrap_err();
        assert!(
            err.to_string().contains("internal hostname"),
            ".internal domains should be blocked, got: {}",
            err
        );

        // .local domain
        let err = super::validate_fetch_url("https://myhost.local/skill.md").unwrap_err();
        assert!(
            err.to_string().contains("internal hostname"),
            ".local domains should be blocked, got: {}",
            err
        );
    }

    #[test]
    fn test_is_restricted_host_allows_normal() {
        let allowed = vec![
            "https://github.com/repo/SKILL.md",
            "https://clawhub.dev/api/v1/download?slug=foo",
            "https://raw.githubusercontent.com/user/repo/main/SKILL.md",
            "https://example.com/skills/deploy.md",
        ];
        for url in allowed {
            assert!(
                super::validate_fetch_url(url).is_ok(),
                "Expected validate_fetch_url({}) to succeed",
                url
            );
        }
    }

    #[test]
    fn test_empty_url_param_is_treated_as_absent() {
        // LLMs sometimes pass "" for optional parameters instead of omitting them.
        // Before the fix, url: "" would match Some("") and attempt to fetch from an
        // empty URL (failing with an invalid URL error) instead of falling through to
        // the catalog lookup. The full execute path cannot be tested here without a
        // real catalog and database, so this test verifies the parameter filtering
        // behaviour directly.
        let params = serde_json::json!({"name": "my-skill", "url": ""});
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        assert!(
            url.is_none(),
            "empty url string should be treated as absent"
        );
    }
}
