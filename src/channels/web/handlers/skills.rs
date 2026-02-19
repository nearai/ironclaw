//! Skills management API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

pub async fn skills_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SkillListResponse>, (StatusCode, String)> {
    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    let guard = registry.read().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Skill registry lock poisoned: {}", e),
        )
    })?;

    let skills: Vec<SkillInfo> = guard
        .skills()
        .iter()
        .map(|s| SkillInfo {
            name: s.manifest.name.clone(),
            description: s.manifest.description.clone(),
            version: s.manifest.version.clone(),
            trust: s.trust.to_string(),
            source: format!("{:?}", s.source),
            keywords: s.manifest.activation.keywords.clone(),
        })
        .collect();

    let count = skills.len();
    Ok(Json(SkillListResponse { skills, count }))
}

pub async fn skills_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SkillSearchRequest>,
) -> Result<Json<SkillSearchResponse>, (StatusCode, String)> {
    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    let catalog = state.skill_catalog.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skill catalog not available".to_string(),
    ))?;

    // Search ClawHub catalog
    let catalog_results = catalog.search(&req.query).await;
    let catalog_json: Vec<serde_json::Value> = catalog_results
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "slug": e.slug,
                "name": e.name,
                "description": e.description,
                "version": e.version,
                "score": e.score,
            })
        })
        .collect();

    // Search local skills
    let query_lower = req.query.to_lowercase();
    let installed: Vec<SkillInfo> = {
        let guard = registry.read().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Skill registry lock poisoned: {}", e),
            )
        })?;
        guard
            .skills()
            .iter()
            .filter(|s| {
                s.manifest.name.to_lowercase().contains(&query_lower)
                    || s.manifest.description.to_lowercase().contains(&query_lower)
            })
            .map(|s| SkillInfo {
                name: s.manifest.name.clone(),
                description: s.manifest.description.clone(),
                version: s.manifest.version.clone(),
                trust: s.trust.to_string(),
                source: format!("{:?}", s.source),
                keywords: s.manifest.activation.keywords.clone(),
            })
            .collect()
    };

    Ok(Json(SkillSearchResponse {
        catalog: catalog_json,
        installed,
        registry_url: catalog.registry_url().to_string(),
    }))
}

pub async fn skills_install_handler(
    State(state): State<Arc<GatewayState>>,
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

    let registry = state.skill_registry.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Skills system not enabled".to_string(),
    ))?;

    let content = if let Some(ref raw) = req.content {
        raw.clone()
    } else if let Some(ref url) = req.url {
        // Fetch from explicit URL (with SSRF protection)
        crate::tools::builtin::skill_tools::fetch_skill_content(url)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    } else if let Some(ref catalog) = state.skill_catalog {
        let url = crate::skills::catalog::skill_download_url(catalog.registry_url(), &req.name);
        crate::tools::builtin::skill_tools::fetch_skill_content(&url)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?
    } else {
        return Ok(Json(ActionResponse::fail(
            "Provide 'content' or 'url' to install a skill".to_string(),
        )));
    };

    // Parse, check duplicates, and get user_dir under a brief read lock.
    let (user_dir, skill_name_from_parse) = {
        let guard = registry.read().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Skill registry lock poisoned: {}", e),
            )
        })?;

        let normalized = crate::skills::normalize_line_endings(&content);
        let parsed = crate::skills::parser::parse_skill_md(&normalized)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let skill_name = parsed.manifest.name.clone();

        if guard.has(&skill_name) {
            return Ok(Json(ActionResponse::fail(format!(
                "Skill '{}' already exists",
                skill_name
            ))));
        }

        (guard.user_dir().to_path_buf(), skill_name)
    };

    // Perform async I/O (write to disk, load) with no lock held.
    let normalized = crate::skills::normalize_line_endings(&content);
    let (skill_name, loaded_skill) =
        crate::skills::registry::SkillRegistry::prepare_install_to_disk(
            &user_dir,
            &skill_name_from_parse,
            &normalized,
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
    crate::skills::registry::SkillRegistry::delete_skill_files(&skill_path)
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
