use std::path::{Path, PathBuf};

use ironclaw_blueprint::parse;
use thiserror::Error;

/// Repo-local convention (mirrors `.claude/`): a project carries its blueprint
/// at `.ironclaw/blueprint.toml`, so `config validate` with no path resolves it
/// automatically from the current directory.
pub const REPO_LOCAL_BLUEPRINT: &str = ".ironclaw/blueprint.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlueprintValidationSummary {
    pub file: PathBuf,
    pub api_version: String,
    pub scope: Vec<String>,
    pub sections: BlueprintSectionCounts,
    pub files: Vec<BlueprintReferencedFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlueprintSectionCounts {
    pub extensions: usize,
    pub skills: usize,
    pub missions: usize,
    pub projects: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlueprintReferencedFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Error)]
pub enum BlueprintValidationError {
    #[error("blueprint path does not exist: {0}")]
    MissingPath(String),
    #[error("no blueprint found at {0}")]
    MissingBlueprint(String),
    #[error("cannot read blueprint {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Lockfile(String),
}

/// Resolve, read, parse, validate, and lock a blueprint. Fail-loud at every
/// step; the error carries the offending path from the parser.
pub fn validate_blueprint(
    path: Option<&Path>,
) -> Result<BlueprintValidationSummary, BlueprintValidationError> {
    let file = resolve_blueprint_path(path)?;
    let root = file.parent().unwrap_or_else(|| Path::new("."));

    let source =
        std::fs::read_to_string(&file).map_err(|source| BlueprintValidationError::Read {
            path: file.clone(),
            source,
        })?;
    let blueprint =
        parse(&source).map_err(|error| BlueprintValidationError::Parse(error.to_string()))?;
    let lockfile = blueprint
        .resolve_lockfile(root)
        .map_err(|error| BlueprintValidationError::Lockfile(error.to_string()))?;

    let scope = [
        blueprint
            .scope
            .tenant
            .as_ref()
            .map(|v| format!("tenant={v}")),
        blueprint.scope.user.as_ref().map(|v| format!("user={v}")),
        blueprint
            .scope
            .project
            .as_ref()
            .map(|v| format!("project={v}")),
        blueprint.scope.agent.as_ref().map(|v| format!("agent={v}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let sections = BlueprintSectionCounts {
        extensions: blueprint.extensions.len(),
        skills: blueprint.skills.len(),
        missions: blueprint.missions.len(),
        projects: blueprint.projects.len(),
    };
    let files = lockfile
        .files
        .into_iter()
        .map(|file| BlueprintReferencedFile {
            path: file.path,
            sha256: file.sha256,
        })
        .collect();

    Ok(BlueprintValidationSummary {
        file,
        api_version: blueprint.api_version,
        scope,
        sections,
        files,
    })
}

/// Turn the optional path argument into a concrete blueprint file:
///
/// - a file path is used as-is;
/// - a directory resolves to `<dir>/.ironclaw/blueprint.toml`;
/// - no argument resolves to `./.ironclaw/blueprint.toml`.
fn resolve_blueprint_path(path: Option<&Path>) -> Result<PathBuf, BlueprintValidationError> {
    let candidate = match path {
        Some(p) if p.is_file() => return Ok(p.to_path_buf()),
        Some(p) if p.is_dir() => p.join(REPO_LOCAL_BLUEPRINT),
        Some(p) => {
            return Err(BlueprintValidationError::MissingPath(
                p.display().to_string(),
            ));
        }
        None => PathBuf::from(REPO_LOCAL_BLUEPRINT),
    };
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(BlueprintValidationError::MissingBlueprint(
            candidate.display().to_string(),
        ))
    }
}
