//! Skills management API handlers.

use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::handlers::skill_registry_scope::scoped_skill_registry;
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::*;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

static SKILL_MUTATION_LOCKS: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
static SKILL_CONTENT_SAFETY: std::sync::LazyLock<ironclaw_safety::Sanitizer> =
    std::sync::LazyLock::new(ironclaw_safety::Sanitizer::new);
const MAX_SKILL_SEARCH_QUERY_BYTES: usize = 1024;

fn install_requested_identifier<'a>(
    name: &'a str,
    explicit_slug: Option<&'a str>,
    resolved_download_key: Option<&'a str>,
) -> &'a str {
    explicit_slug
        .filter(|s| !s.is_empty())
        .or(resolved_download_key.filter(|s| !s.is_empty()))
        .unwrap_or(name)
}

fn skill_setup_hint(skill: &ironclaw_skills::types::LoadedSkill) -> Option<String> {
    let mut hints = Vec::new();
    if !skill.manifest.requires.env.is_empty() {
        hints.push(format!(
            "Requires env vars: {}",
            skill.manifest.requires.env.join(", ")
        ));
    }
    if !skill.manifest.requires.bins.is_empty() {
        hints.push(format!(
            "Requires binaries on PATH: {}",
            skill.manifest.requires.bins.join(", ")
        ));
    }
    (!hints.is_empty()).then(|| hints.join(" · "))
}

fn skill_source_kind(source: &ironclaw_skills::types::SkillSource) -> SkillSourceKind {
    match source {
        ironclaw_skills::types::SkillSource::Workspace(_) => SkillSourceKind::Workspace,
        ironclaw_skills::types::SkillSource::User(_) => SkillSourceKind::User,
        ironclaw_skills::types::SkillSource::Installed(_) => SkillSourceKind::Installed,
        ironclaw_skills::types::SkillSource::Bundled(_) => SkillSourceKind::System,
    }
}

fn skill_is_user_managed(source: &ironclaw_skills::types::SkillSource) -> bool {
    matches!(
        source,
        ironclaw_skills::types::SkillSource::User(_)
            | ironclaw_skills::types::SkillSource::Installed(_)
    )
}

fn skill_can_delete(source: &ironclaw_skills::types::SkillSource) -> bool {
    matches!(
        source,
        ironclaw_skills::types::SkillSource::User(_)
            | ironclaw_skills::types::SkillSource::Installed(_)
    )
}

fn skill_registry_error_response(
    status: StatusCode,
    error: ironclaw_skills::SkillRegistryError,
) -> (StatusCode, String) {
    use ironclaw_skills::SkillRegistryError;

    match error {
        SkillRegistryError::ReadError { .. }
        | SkillRegistryError::WriteError { .. }
        | SkillRegistryError::SymlinkDetected { .. } => {
            tracing::warn!(error = %error, "skill filesystem operation failed");
            (status, "Can't access this skill".to_string())
        }
        other => (status, other.to_string()),
    }
}

fn validate_skill_search_query(query: &str) -> Result<(), (StatusCode, String)> {
    if query.len() > MAX_SKILL_SEARCH_QUERY_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Skill search query must be at most {MAX_SKILL_SEARCH_QUERY_BYTES} bytes"),
        ));
    }
    Ok(())
}

fn validate_skill_content_safety(content: &str) -> Result<(), (StatusCode, String)> {
    ironclaw_safety::validate_trusted_trigger_prompt(&*SKILL_CONTENT_SAFETY, content).map_err(
        |error| {
            tracing::warn!(
                reason = error.reason(),
                "skill content rejected by safety scan"
            );
            (
                StatusCode::BAD_REQUEST,
                "Skill content was rejected by the safety scan".to_string(),
            )
        },
    )
}

async fn skill_mutation_guard(
    state: &GatewayState,
    user: &crate::channels::web::auth::UserIdentity,
) -> Result<tokio::sync::OwnedMutexGuard<()>, (StatusCode, String)> {
    let lock_key = if !state.multi_tenant_mode {
        "shared".to_string()
    } else {
        format!("user:{}", user.user_id)
    };
    let lock = {
        let mut locks = SKILL_MUTATION_LOCKS.lock().map_err(|error| {
            tracing::error!("Skill mutation lock map poisoned: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Can't access skills right now".to_string(),
            )
        })?;
        locks.retain(|_, weak| weak.strong_count() > 0);
        if let Some(existing) = locks.get(&lock_key).and_then(Weak::upgrade) {
            existing
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(lock_key, Arc::downgrade(&lock));
            lock
        }
    };
    Ok(lock.lock_owned().await)
}

async fn skill_info(
    skill: ironclaw_skills::types::LoadedSkill,
    can_manage_skills: bool,
) -> SkillInfo {
    let bundle_dir = match &skill.source {
        ironclaw_skills::types::SkillSource::Workspace(path)
        | ironclaw_skills::types::SkillSource::User(path)
        | ironclaw_skills::types::SkillSource::Installed(path)
        | ironclaw_skills::types::SkillSource::Bundled(path) => Some(path.clone()),
    };
    let install_meta = match &bundle_dir {
        Some(path) => ironclaw_skills::registry::SkillRegistry::read_install_metadata(path).await,
        None => None,
    };
    let has_requirements = match &bundle_dir {
        Some(path) => tokio::fs::try_exists(path.join("requirements.txt"))
            .await
            .unwrap_or(false),
        None => false,
    };
    let has_scripts = match &bundle_dir {
        Some(path) => tokio::fs::metadata(path.join("scripts"))
            .await
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false),
        None => false,
    };
    let source_kind = skill_source_kind(&skill.source);
    let can_edit = can_manage_skills && skill_is_user_managed(&skill.source);
    let can_delete = can_manage_skills && skill_can_delete(&skill.source);

    SkillInfo {
        name: skill.manifest.name.clone(),
        description: skill.manifest.description.clone(),
        version: skill.manifest.version.clone(),
        trust: skill.trust.to_string(),
        source: source_kind.as_str().to_string(),
        source_kind,
        keywords: skill.manifest.activation.keywords.clone(),
        usage_hint: Some(format!(
            "Type `/{}` in chat to force-activate this skill.",
            skill.manifest.name
        )),
        setup_hint: skill_setup_hint(&skill),
        bundle_path: None,
        install_source_url: install_meta.and_then(|meta| meta.source_url),
        has_requirements,
        has_scripts,
        can_edit,
        can_delete,
    }
}

async fn skill_infos(
    skills: Vec<ironclaw_skills::types::LoadedSkill>,
    can_manage_skills: bool,
) -> Vec<SkillInfo> {
    let mut infos = Vec::with_capacity(skills.len());
    for skill in skills {
        infos.push(skill_info(skill, can_manage_skills).await);
    }
    infos
}

pub async fn skills_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<SkillListResponse>, (StatusCode, String)> {
    let registry = scoped_skill_registry(&state, &user).await?;
    let skill_snapshot = registry.skills_snapshot()?;

    let skills = skill_infos(skill_snapshot, true).await;

    let count = skills.len();
    Ok(Json(SkillListResponse { skills, count }))
}

pub async fn skills_search_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<SkillSearchRequest>,
) -> Result<Json<SkillSearchResponse>, (StatusCode, String)> {
    validate_skill_search_query(&req.query)?;
    let registry = scoped_skill_registry(&state, &user).await?;

    let catalog = Arc::clone(state.skill_catalog.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skill catalog not available".to_string(),
    ))?);

    // Search ClawHub catalog
    let catalog_outcome = catalog.search(&req.query).await;
    let catalog_error = catalog_outcome.error.clone();

    // Enrich top results with detail data (stars, downloads, owner)
    let mut entries = catalog_outcome.results;
    catalog.enrich_search_results(&mut entries, 5).await;

    let query_lower = req.query.to_lowercase();
    let skill_snapshot = registry.skills_snapshot()?;
    let installed_names: Vec<String> = skill_snapshot
        .iter()
        .map(|s| s.manifest.name.clone())
        .collect();
    let matching_skills: Vec<ironclaw_skills::types::LoadedSkill> = skill_snapshot
        .into_iter()
        .filter(|s| {
            s.manifest.name.to_lowercase().contains(&query_lower)
                || s.manifest.description.to_lowercase().contains(&query_lower)
        })
        .collect();
    let installed = skill_infos(matching_skills, true).await;

    let catalog_json: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            let is_installed = ironclaw_skills::catalog::catalog_entry_is_installed(
                &e.slug,
                &e.name,
                &installed_names,
            );
            serde_json::json!({
                "slug": e.slug,
                "name": e.name,
                "description": e.description,
                "version": e.version,
                "score": e.score,
                "updatedAt": e.updated_at,
                "stars": e.stars,
                "downloads": e.downloads,
                "owner": e.owner,
                "installed": is_installed,
            })
        })
        .collect();

    Ok(Json(SkillSearchResponse {
        catalog: catalog_json,
        installed,
        registry_url: catalog.registry_url().to_string(),
        catalog_error,
    }))
}

pub async fn skills_install_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: axum::http::HeaderMap,
    Json(req): Json<SkillInstallRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Require explicit confirmation header to prevent accidental installs.
    // Chat tools have requires_approval(); this is the equivalent for the web API.
    if headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill install requires X-Confirm-Action: true header".to_string(),
        ));
    }

    tracing::info!(user_id = %user.user_id, skill = %req.name, "skill install requested");

    // dispatch-exempt: web skill management mirrors the approved skill_install tool path.
    let _mutation_guard = skill_mutation_guard(&state, &user).await?;
    let mut scoped_registry = scoped_skill_registry(&state, &user).await?;

    let mut resolved_download_key = None;
    let install_payload = if let Some(ref raw) = req.content {
        crate::tools::builtin::skill_tools::SkillInstallPayload {
            skill_md: raw.clone(),
            ..crate::tools::builtin::skill_tools::SkillInstallPayload::default()
        }
    } else if let Some(ref url) = req.url {
        // Fetch from explicit URL (with SSRF protection)
        crate::tools::builtin::skill_tools::fetch_skill_payload(url)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    } else if let Some(ref catalog) = state.skill_catalog {
        let download_key = if let Some(slug) = req.slug.as_deref().filter(|s| !s.is_empty()) {
            slug.to_string()
        } else if req.name.contains('/') {
            req.name.clone()
        } else {
            let outcome = catalog.search(&req.name).await;
            match ironclaw_skills::catalog::resolve_catalog_slug_for_name(
                &req.name,
                &outcome.results,
            ) {
                Ok(Some(resolved)) => resolved,
                Ok(None) => {
                    let reason = outcome
                        .error
                        .unwrap_or_else(|| "no unique catalog match was found".to_string());
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Could not resolve skill name '{}' to a catalog slug: {}",
                            req.name, reason
                        ),
                    ));
                }
                Err(e) => return Err((StatusCode::BAD_REQUEST, e.to_string())),
            }
        };
        let url =
            ironclaw_skills::catalog::skill_download_url(catalog.registry_url(), &download_key);
        resolved_download_key = Some(download_key);
        crate::tools::builtin::skill_tools::fetch_skill_payload(&url)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?
    } else {
        return Ok(Json(ActionResponse::fail(
            "Provide 'content' or 'url' to install a skill".to_string(),
        )));
    };

    let normalized = ironclaw_skills::normalize_line_endings(&install_payload.skill_md);
    let requested_identifier = install_requested_identifier(
        &req.name,
        req.slug.as_deref(),
        resolved_download_key.as_deref(),
    );

    // Parse, check duplicates, and get install_dir under a brief read lock.
    let (skill_name_from_parse, install_content) =
        ironclaw_skills::registry::SkillRegistry::resolve_install_content(
            &normalized,
            Some(requested_identifier),
        )
        .map_err(|e| skill_registry_error_response(StatusCode::BAD_REQUEST, e))?;
    validate_skill_content_safety(&install_content)?;

    if scoped_registry.has(&skill_name_from_parse)? {
        return Ok(Json(ActionResponse::fail(format!(
            "Skill '{}' already exists",
            skill_name_from_parse
        ))));
    }

    let user_dir = scoped_registry.install_target_dir()?;

    // Perform async I/O (write to disk, load) with no lock held.
    let (skill_name, loaded_skill) =
        ironclaw_skills::registry::SkillRegistry::prepare_install_bundle_to_disk(
            &user_dir,
            &skill_name_from_parse,
            &install_content,
            &install_payload.extra_files,
            install_payload.install_metadata.as_ref(),
        )
        .await
        .map_err(|e| skill_registry_error_response(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Commit: brief write lock for in-memory addition
    let commit_result = scoped_registry.commit_install(&skill_name, loaded_skill)?;

    match commit_result {
        Ok(()) => Ok(Json(ActionResponse::ok(format!(
            "Skill '{}' installed",
            skill_name
        )))),
        Err(e) => Ok(Json(ActionResponse::fail(
            skill_registry_error_response(StatusCode::BAD_REQUEST, e).1,
        ))),
    }
}

pub async fn skills_remove_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Require explicit confirmation header to prevent accidental removals.
    if headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill removal requires X-Confirm-Action: true header".to_string(),
        ));
    }

    tracing::info!(user_id = %user.user_id, skill = %name, "skill remove requested");

    // dispatch-exempt: web skill management mirrors the approved skill_remove tool path.
    let _mutation_guard = skill_mutation_guard(&state, &user).await?;
    let mut scoped_registry = scoped_skill_registry(&state, &user).await?;

    // Validate removal under a brief read lock
    let skill_path = scoped_registry
        .validate_remove(&name)?
        .map_err(|e| skill_registry_error_response(StatusCode::BAD_REQUEST, e))?;

    // Delete files from disk (async I/O, no lock held)
    ironclaw_skills::registry::SkillRegistry::delete_skill_files(&skill_path)
        .await
        .map_err(|e| skill_registry_error_response(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Remove from in-memory registry under a brief write lock
    let commit_result = scoped_registry.commit_remove(&name)?;

    match commit_result {
        Ok(()) => Ok(Json(ActionResponse::ok(format!(
            "Skill '{}' removed",
            name
        )))),
        Err(e) => Ok(Json(ActionResponse::fail(
            skill_registry_error_response(StatusCode::BAD_REQUEST, e).1,
        ))),
    }
}

pub async fn skills_get_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<SkillContentResponse>, (StatusCode, String)> {
    let scoped_registry = scoped_skill_registry(&state, &user).await?;

    let (skill_path, _, _) = scoped_registry
        .validate_update(&name)?
        .map_err(|e| skill_registry_error_response(StatusCode::BAD_REQUEST, e))?;

    let content =
        ironclaw_skills::registry::SkillRegistry::read_skill_content_for_update(&skill_path, &name)
            .await
            .map_err(|e| skill_registry_error_response(StatusCode::BAD_REQUEST, e))?;

    Ok(Json(SkillContentResponse { name, content }))
}

pub async fn skills_update_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<SkillUpdateRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Require explicit confirmation header to prevent accidental edits.
    if headers
        .get("x-confirm-action")
        .and_then(|v| v.to_str().ok())
        != Some("true")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Skill update requires X-Confirm-Action: true header".to_string(),
        ));
    }

    tracing::info!(user_id = %user.user_id, skill = %name, "skill update requested");

    if req.content.len() as u64 > ironclaw_skills::MAX_PROMPT_FILE_SIZE {
        return Err(skill_registry_error_response(
            StatusCode::BAD_REQUEST,
            ironclaw_skills::SkillRegistryError::FileTooLarge {
                name: name.clone(),
                size: req.content.len() as u64,
                max: ironclaw_skills::MAX_PROMPT_FILE_SIZE,
            },
        ));
    }

    validate_skill_content_safety(&req.content)?;

    // dispatch-exempt: web skill management mirrors the approved skill_update tool path.
    let _mutation_guard = skill_mutation_guard(&state, &user).await?;
    let mut scoped_registry = scoped_skill_registry(&state, &user).await?;

    let (skill_path, trust, source) = scoped_registry
        .validate_update(&name)?
        .map_err(|e| skill_registry_error_response(StatusCode::BAD_REQUEST, e))?;

    let loaded_skill = ironclaw_skills::registry::SkillRegistry::prepare_update_to_disk(
        &skill_path,
        &name,
        &req.content,
        trust,
        source,
    )
    .await
    .map_err(|e| skill_registry_error_response(StatusCode::BAD_REQUEST, e))?;

    let commit_result = scoped_registry.commit_update(&name, loaded_skill)?;

    match commit_result {
        Ok(()) => Ok(Json(ActionResponse::ok(format!(
            "Skill '{}' updated",
            name
        )))),
        Err(e) => Ok(Json(ActionResponse::fail(
            skill_registry_error_response(StatusCode::BAD_REQUEST, e).1,
        ))),
    }
}

#[cfg(test)]
mod tests;
