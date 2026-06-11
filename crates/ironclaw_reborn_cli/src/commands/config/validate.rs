//! `ironclaw-reborn config validate` — parse and validate a blueprint, resolve
//! its file references, and print the resulting lockfile.
//!
//! Slice 3 of epic #3036. This is the read-only operator surface: it proves a
//! blueprint parses, contains no inline secrets, and that every `text_ref` /
//! `brief_ref` resolves and hashes. The live `apply` / `diff` subcommands land
//! once the per-domain reconcilers and settings repo exist.

use std::path::{Path, PathBuf};

use clap::Args;
use ironclaw_blueprint::{Blueprint, Lockfile, parse};

use crate::context::RebornCliContext;

/// Repo-local convention (mirrors `.claude/`): a project carries its blueprint
/// at `.ironclaw/blueprint.toml`, so `config validate` with no path resolves it
/// automatically from the current directory.
const REPO_LOCAL_BLUEPRINT: &str = ".ironclaw/blueprint.toml";

#[derive(Debug, Args)]
pub(crate) struct ConfigValidateCommand {
    /// Path to a `blueprint.toml`, or a directory containing
    /// `.ironclaw/blueprint.toml`. Defaults to the current directory.
    path: Option<PathBuf>,
}

impl ConfigValidateCommand {
    pub(crate) fn execute(self, _context: RebornCliContext) -> anyhow::Result<()> {
        let summary = validate_blueprint(self.path.as_deref())?;
        summary.print();
        Ok(())
    }
}

/// Outcome of a successful validation — what `execute` prints, factored out so
/// it can be driven directly in tests.
#[derive(Debug)]
pub(crate) struct ValidationSummary {
    pub file: PathBuf,
    pub blueprint: Blueprint,
    pub lockfile: Lockfile,
}

impl ValidationSummary {
    fn print(&self) {
        println!("✓ {} is valid", self.file.display());
        println!("  api_version: {}", self.blueprint.api_version);

        let scope = &self.blueprint.scope;
        let scope_desc = [
            scope.tenant.as_ref().map(|v| format!("tenant={v}")),
            scope.user.as_ref().map(|v| format!("user={v}")),
            scope.project.as_ref().map(|v| format!("project={v}")),
            scope.agent.as_ref().map(|v| format!("agent={v}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        println!(
            "  scope: {}",
            if scope_desc.is_empty() {
                "system".to_string()
            } else {
                scope_desc.join(", ")
            }
        );

        println!(
            "  sections: {} extensions, {} skills, {} missions, {} projects",
            self.blueprint.extensions.len(),
            self.blueprint.skills.len(),
            self.blueprint.missions.len(),
            self.blueprint.projects.len(),
        );

        if self.lockfile.files.is_empty() {
            println!("  files: none referenced");
        } else {
            println!("  files ({}):", self.lockfile.files.len());
            for file in &self.lockfile.files {
                // First 12 hex chars are enough for a human-readable digest.
                println!("    {}  {}", &file.sha256[..12], file.path);
            }
        }
    }
}

/// Resolve, read, parse, validate, and lock a blueprint. Fail-loud at every
/// step; the error carries the offending path from the parser.
pub(crate) fn validate_blueprint(path: Option<&Path>) -> anyhow::Result<ValidationSummary> {
    let file = resolve_blueprint_path(path)?;
    let root = file.parent().unwrap_or_else(|| Path::new("."));

    let source = std::fs::read_to_string(&file)
        .map_err(|e| anyhow::anyhow!("cannot read blueprint {}: {e}", file.display()))?;
    let blueprint = parse(&source).map_err(|e| anyhow::anyhow!("{e}"))?;
    let lockfile = blueprint
        .resolve_lockfile(root)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(ValidationSummary {
        file,
        blueprint,
        lockfile,
    })
}

/// Turn the optional path argument into a concrete blueprint file:
///
/// - a file path is used as-is;
/// - a directory resolves to `<dir>/.ironclaw/blueprint.toml`;
/// - no argument resolves to `./.ironclaw/blueprint.toml`.
fn resolve_blueprint_path(path: Option<&Path>) -> anyhow::Result<PathBuf> {
    let candidate = match path {
        Some(p) if p.is_file() => return Ok(p.to_path_buf()),
        Some(p) if p.is_dir() => p.join(REPO_LOCAL_BLUEPRINT),
        Some(p) => {
            // A path was given but does not exist — report it directly rather
            // than silently falling back to the repo-local default.
            anyhow::bail!("blueprint path does not exist: {}", p.display());
        }
        None => {
            let cwd = std::env::current_dir()
                .map_err(|e| anyhow::anyhow!("cannot determine current directory: {e}"))?;
            cwd.join(REPO_LOCAL_BLUEPRINT)
        }
    };
    if candidate.is_file() {
        Ok(candidate)
    } else {
        anyhow::bail!("no blueprint found at {}", candidate.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&path, body).expect("write");
        path
    }

    const VALID: &str = r#"
api_version = "ironclaw.config/v1"
kind = "Blueprint"
[scope]
user = "self"
[system_prompt]
text_ref = "files/prompt.md"
"#;

    #[test]
    fn validates_repo_local_blueprint_from_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), REPO_LOCAL_BLUEPRINT, VALID);
        write(dir.path(), ".ironclaw/files/prompt.md", "hi");

        let summary = validate_blueprint(Some(dir.path())).expect("validates");
        assert_eq!(summary.blueprint.api_version, "ironclaw.config/v1");
        assert_eq!(summary.lockfile.files.len(), 1);
        assert_eq!(summary.lockfile.files[0].path, "files/prompt.md");
    }

    #[test]
    fn validates_explicit_file_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "bp.toml", VALID);
        write(dir.path(), "files/prompt.md", "hi");

        let summary = validate_blueprint(Some(&file)).expect("validates");
        assert_eq!(summary.lockfile.files.len(), 1);
    }

    #[test]
    fn errors_on_missing_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope.toml");
        let err = validate_blueprint(Some(&missing)).expect_err("missing path errors");
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn surfaces_inline_secret_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bad = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
                   [providers.anthropic]\napi_key = \"sk-proj-abcdef1234567890abcdef1234\"\n";
        let file = write(dir.path(), "bp.toml", bad);
        let err = validate_blueprint(Some(&file)).expect_err("inline secret errors");
        assert!(err.to_string().contains("providers.anthropic.api_key"));
    }
}
