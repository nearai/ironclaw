//! Skills management API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use futures::future::join_all;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::*;

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

async fn skill_info(skill: ironclaw_skills::types::LoadedSkill) -> SkillInfo {
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
    let bundle_path = bundle_dir.as_ref().map(|path| path.display().to_string());

    SkillInfo {
        name: skill.manifest.name.clone(),
        description: skill.manifest.description.clone(),
        version: skill.manifest.version.clone(),
        trust: skill.trust.to_string(),
        source: format!("{:?}", skill.source),
        keywords: skill.manifest.activation.keywords.clone(),
        usage_hint: Some(format!(
            "Type `/{}` in chat to force-activate this skill.",
            skill.manifest.name
        )),
        setup_hint: skill_setup_hint(&skill),
        bundle_path,
        install_source_url: install_meta.and_then(|meta| meta.source_url),
        has_requirements,
        has_scripts,
    }
}

pub async fn skills_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<SkillListResponse>, (StatusCode, String)> {
    let registry = Arc::clone(state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?);

    let skill_snapshot = {
        let guard = registry.read().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Skill registry lock poisoned: {}", e),
            )
        })?;
        guard.skills().to_vec()
    };

    let skills: Vec<SkillInfo> = join_all(skill_snapshot.into_iter().map(skill_info)).await;

    let count = skills.len();
    Ok(Json(SkillListResponse { skills, count }))
}

pub async fn skills_search_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Json(req): Json<SkillSearchRequest>,
) -> Result<Json<SkillSearchResponse>, (StatusCode, String)> {
    let registry = Arc::clone(state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?);

    let catalog = Arc::clone(state.skill_catalog.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "ClawHub registry is not enabled (CLAWHUB_ENABLED=false)".to_string(),
    ))?);

    // Search ClawHub catalog
    let catalog_outcome = catalog.search(&req.query).await;
    let catalog_error = catalog_outcome.error.clone();

    // Enrich top results with detail data (stars, downloads, owner)
    let mut entries = catalog_outcome.results;
    catalog.enrich_search_results(&mut entries, 5).await;

    let query_lower = req.query.to_lowercase();
    let (installed_names, matching_skills): (
        Vec<String>,
        Vec<ironclaw_skills::types::LoadedSkill>,
    ) = {
        let guard = registry.read().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Skill registry lock poisoned: {}", e),
            )
        })?;
        let installed_names: Vec<String> = guard
            .skills()
            .iter()
            .map(|s| s.manifest.name.clone())
            .collect();
        let matching_skills = guard
            .skills()
            .iter()
            .filter(|s| {
                s.manifest.name.to_lowercase().contains(&query_lower)
                    || s.manifest.description.to_lowercase().contains(&query_lower)
            })
            .cloned()
            .collect();
        (installed_names, matching_skills)
    };
    let installed: Vec<SkillInfo> = join_all(matching_skills.into_iter().map(skill_info)).await;

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

    let name = req.name.as_deref().unwrap_or("");
    tracing::info!(user_id = %user.user_id, skill = %name, "skill install requested");

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    let mut resolved_download_key = None;
    let install_payload = if let Some(ref raw) = req.content {
        crate::tools::builtin::skill_tools::SkillInstallPayload {
            skill_md: raw.clone(),
            ..crate::tools::builtin::skill_tools::SkillInstallPayload::default()
        }
    } else if let Some(ref url) = req.url {
        // URL installs require ClawHub to be enabled
        if state.skill_catalog.is_none() {
            return Err((
                StatusCode::FORBIDDEN,
                "URL-based skill installs are disabled (ClawHub registry is not enabled)"
                    .to_string(),
            ));
        }
        // Fetch from explicit URL (with SSRF protection)
        crate::tools::builtin::skill_tools::fetch_skill_payload(url)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    } else if let Some(ref catalog) = state.skill_catalog {
        let download_key = if let Some(slug) = req.slug.as_deref().filter(|s| !s.is_empty()) {
            slug.to_string()
        } else if name.contains('/') {
            name.to_string()
        } else {
            let outcome = catalog.search(name).await;
            match ironclaw_skills::catalog::resolve_catalog_slug_for_name(name, &outcome.results) {
                Ok(Some(resolved)) => resolved,
                Ok(None) => {
                    let reason = outcome
                        .error
                        .unwrap_or_else(|| "no unique catalog match was found".to_string());
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Could not resolve skill name '{}' to a catalog slug: {}",
                            name, reason
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
            "Provide 'content' to install a skill (ClawHub registry is not enabled)".to_string(),
        )));
    };

    let normalized = ironclaw_skills::normalize_line_endings(&install_payload.skill_md);
    let requested_identifier =
        install_requested_identifier(name, req.slug.as_deref(), resolved_download_key.as_deref());

    // Parse, check duplicates, and get install_dir under a brief read lock.
    let (user_dir, skill_name_from_parse, install_content) = {
        let guard = registry.read().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Skill registry lock poisoned: {}", e),
            )
        })?;

        let (skill_name, install_content) =
            ironclaw_skills::registry::SkillRegistry::resolve_install_content(
                &normalized,
                Some(requested_identifier),
            )
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        if guard.has(&skill_name) {
            return Ok(Json(ActionResponse::fail(format!(
                "Skill '{}' already exists",
                skill_name
            ))));
        }

        (
            guard.install_target_dir().to_path_buf(),
            skill_name,
            install_content,
        )
    };

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
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Commit: brief write lock for in-memory addition
    let mut guard = registry.write().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Skill registry lock poisoned: {}", e),
        )
    })?;

    match guard.commit_install(&skill_name, loaded_skill) {
        Ok(()) => Ok(Json(ActionResponse::ok(format!(
            "Skill '{}' installed",
            skill_name
        )))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
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

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    // Validate removal under a brief read lock
    let skill_path = {
        let guard = registry.read().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Skill registry lock poisoned: {}", e),
            )
        })?;
        guard
            .validate_remove(&name)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    };

    // Delete files from disk (async I/O, no lock held)
    ironclaw_skills::registry::SkillRegistry::delete_skill_files(&skill_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Remove from in-memory registry under a brief write lock
    let mut guard = registry.write().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Skill registry lock poisoned: {}", e),
        )
    })?;

    match guard.commit_remove(&name) {
        Ok(()) => Ok(Json(ActionResponse::ok(format!(
            "Skill '{}' removed",
            name
        )))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn catalog_entry_matches_installed_slug_suffix() {
        let installed = vec!["mortgage-calculator".to_string()];

        assert!(ironclaw_skills::catalog::catalog_entry_is_installed(
            "finance/mortgage-calculator",
            "Mortgage Calculator",
            &installed,
        ));
    }

    #[test]
    fn catalog_entry_matches_installed_display_name() {
        let installed = vec!["Mortgage Calculator".to_string()];

        assert!(ironclaw_skills::catalog::catalog_entry_is_installed(
            "finance/mortgage-calculator",
            "Mortgage Calculator",
            &installed,
        ));
    }

    #[test]
    fn catalog_entry_does_not_match_unrelated_installed_skill() {
        let installed = vec!["budget-planner".to_string()];

        assert!(!ironclaw_skills::catalog::catalog_entry_is_installed(
            "finance/mortgage-calculator",
            "Mortgage Calculator",
            &installed,
        ));
    }

    #[test]
    fn catalog_entry_matches_owner_aware_normalized_install_name() {
        let installed = vec!["finance-mortgage-calculator".to_string()];

        assert!(ironclaw_skills::catalog::catalog_entry_is_installed(
            "finance/mortgage-calculator",
            "Mortgage Calculator",
            &installed,
        ));
    }

    #[test]
    fn install_requested_identifier_prefers_resolved_slug_for_manual_name_installs() {
        assert_eq!(
            super::install_requested_identifier(
                "Mortgage Calculator",
                None,
                Some("finance/mortgage-calculator"),
            ),
            "finance/mortgage-calculator"
        );
    }

    #[tokio::test]
    async fn skill_info_reports_bundle_files() {
        let install_dir = tempfile::tempdir().expect("tempdir");
        let metadata = ironclaw_skills::registry::InstalledSkillMetadata {
            source_url: Some("https://example.com/skill".to_string()),
            source_subdir: None,
        };
        let extra_files = vec![
            ironclaw_skills::registry::InstallFile {
                relative_path: Path::new("requirements.txt").to_path_buf(),
                contents: b"httpx==0.27.0\n".to_vec(),
            },
            ironclaw_skills::registry::InstallFile {
                relative_path: Path::new("scripts/run.py").to_path_buf(),
                contents: b"print('ok')\n".to_vec(),
            },
        ];

        let (_, skill) = ironclaw_skills::registry::SkillRegistry::prepare_install_bundle_to_disk(
            install_dir.path(),
            "demo-skill",
            "---\nname: demo-skill\ndescription: Demo\nversion: 1.0.0\n---\n\n# Demo\n",
            &extra_files,
            Some(&metadata),
        )
        .await
        .expect("install bundle");

        let info = super::skill_info(skill).await;
        assert!(info.has_requirements);
        assert!(info.has_scripts);
        assert_eq!(
            info.install_source_url.as_deref(),
            Some("https://example.com/skill")
        );
        assert!(
            info.bundle_path
                .as_deref()
                .is_some_and(|path| path.ends_with("demo-skill"))
        );
    }
}
