//! `ironclaw-reborn config validate` — parse and validate a blueprint, resolve
//! its file references, and print the resulting lockfile.
//!
//! Slice 3 of epic #3036. This is the read-only operator surface: it proves a
//! blueprint parses, contains no inline secrets, and that every `text_ref` /
//! `brief_ref` resolves and hashes. The live `apply` / `diff` subcommands land
//! once the per-domain reconcilers and settings repo exist.

#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use clap::Args;
use ironclaw_reborn_composition::{BlueprintValidationSummary, validate_blueprint};

#[cfg(test)]
use ironclaw_reborn_composition::REPO_LOCAL_BLUEPRINT;

use crate::context::RebornCliContext;

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

trait PrintValidationSummary {
    fn print(&self);
}

impl PrintValidationSummary for BlueprintValidationSummary {
    fn print(&self) {
        println!("✓ {} is valid", self.file.display());
        println!("  api_version: {}", self.api_version);

        println!(
            "  scope: {}",
            if self.scope.is_empty() {
                "system".to_string()
            } else {
                self.scope.join(", ")
            }
        );

        println!(
            "  sections: {} extensions, {} skills, {} missions, {} projects",
            self.sections.extensions,
            self.sections.skills,
            self.sections.missions,
            self.sections.projects,
        );

        if self.files.is_empty() {
            println!("  files: none referenced");
        } else {
            println!("  files ({}):", self.files.len());
            for file in &self.files {
                // First 12 hex chars are enough for a human-readable digest.
                let digest = file.sha256.chars().take(12).collect::<String>();
                println!("    {}  {}", digest, file.path);
            }
        }
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
        assert_eq!(summary.api_version, "ironclaw.config/v1");
        assert_eq!(summary.files.len(), 1);
        assert_eq!(summary.files[0].path, "files/prompt.md");
    }

    #[test]
    fn validates_explicit_file_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = write(dir.path(), "bp.toml", VALID);
        write(dir.path(), "files/prompt.md", "hi");

        let summary = validate_blueprint(Some(&file)).expect("validates");
        assert_eq!(summary.files.len(), 1);
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
