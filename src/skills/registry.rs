//! Skill registry for discovering, loading, and managing available skills.
//!
//! Skills are discovered from two filesystem locations:
//! 1. Workspace skills directory (`<workspace>/skills/`) -- Trusted
//! 2. User skills directory (`~/.ironclaw/skills/`) -- Trusted
//!
//! Both flat (`skills/SKILL.md`) and subdirectory (`skills/<name>/SKILL.md`)
//! layouts are supported. Earlier locations win on name collision (workspace
//! overrides user). Uses async I/O throughout to avoid blocking the tokio runtime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::skills::gating::check_requirements;
use crate::skills::parser::{SkillParseError, parse_skill_md};
use crate::skills::{
    GatingRequirements, LoadedSkill, MAX_PROMPT_FILE_SIZE, SkillSource, SkillTrust,
    normalize_line_endings,
};

/// Maximum number of skills that can be discovered from a single directory.
/// Prevents resource exhaustion from a directory with thousands of entries.
const MAX_DISCOVERED_SKILLS: usize = 100;

/// Error type for skill registry operations.
#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryError {
    #[error("Skill not found: {0}")]
    NotFound(String),

    #[error("Failed to read skill file {path}: {reason}")]
    ReadError { path: String, reason: String },

    #[error("Failed to parse SKILL.md for '{name}': {reason}")]
    ParseError { name: String, reason: String },

    #[error("Skill file too large for '{name}': {size} bytes (max {max} bytes)")]
    FileTooLarge { name: String, size: u64, max: u64 },

    #[error("Symlink detected in skills directory: {path}")]
    SymlinkDetected { path: String },

    #[error("Skill '{name}' failed gating: {reason}")]
    GatingFailed { name: String, reason: String },

    #[error(
        "Skill '{name}' prompt exceeds token budget: ~{approx_tokens} tokens but declares max_context_tokens={declared}"
    )]
    TokenBudgetExceeded {
        name: String,
        approx_tokens: usize,
        declared: usize,
    },
}

/// Registry of available skills.
pub struct SkillRegistry {
    /// Loaded skills keyed by name.
    skills: Vec<LoadedSkill>,
    /// User skills directory (~/.ironclaw/skills/).
    user_dir: PathBuf,
    /// Optional workspace skills directory.
    workspace_dir: Option<PathBuf>,
}

impl SkillRegistry {
    /// Create a new skill registry.
    pub fn new(user_dir: PathBuf) -> Self {
        Self {
            skills: Vec::new(),
            user_dir,
            workspace_dir: None,
        }
    }

    /// Set a workspace skills directory.
    pub fn with_workspace_dir(mut self, dir: PathBuf) -> Self {
        self.workspace_dir = Some(dir);
        self
    }

    /// Discover and load skills from all configured directories.
    ///
    /// Discovery order (earlier wins on name collision):
    /// 1. Workspace skills directory (if set) -- Trusted
    /// 2. User skills directory -- Trusted
    pub async fn discover_all(&mut self) -> Vec<String> {
        let mut loaded_names: Vec<String> = Vec::new();
        let mut seen: HashMap<String, ()> = HashMap::new();

        // 1. Workspace skills (highest priority)
        if let Some(ref ws_dir) = self.workspace_dir.clone() {
            let ws_skills = self
                .discover_from_dir(ws_dir, SkillTrust::Trusted, |p| SkillSource::Workspace(p))
                .await;
            for (name, skill) in ws_skills {
                if seen.contains_key(&name) {
                    continue;
                }
                seen.insert(name.clone(), ());
                loaded_names.push(name);
                self.skills.push(skill);
            }
        }

        // 2. User skills
        let user_dir = self.user_dir.clone();
        let user_skills = self
            .discover_from_dir(&user_dir, SkillTrust::Trusted, SkillSource::User)
            .await;
        for (name, skill) in user_skills {
            if seen.contains_key(&name) {
                tracing::debug!("Skipping user skill '{}' (overridden by workspace)", name);
                continue;
            }
            seen.insert(name.clone(), ());
            loaded_names.push(name);
            self.skills.push(skill);
        }

        loaded_names
    }

    /// Discover skills from a single directory.
    ///
    /// Supports both layouts:
    /// - Flat: `dir/SKILL.md` (skill name derived from parent dir or file stem)
    /// - Subdirectory: `dir/<name>/SKILL.md`
    async fn discover_from_dir<F>(
        &self,
        dir: &Path,
        trust: SkillTrust,
        make_source: F,
    ) -> Vec<(String, LoadedSkill)>
    where
        F: Fn(PathBuf) -> SkillSource,
    {
        let mut results = Vec::new();

        if !tokio::fs::try_exists(dir).await.unwrap_or(false) {
            tracing::debug!("Skills directory does not exist: {:?}", dir);
            return results;
        }

        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!("Failed to read skills directory {:?}: {}", dir, e);
                return results;
            }
        };

        let mut count = 0usize;
        while let Ok(Some(entry)) = entries.next_entry().await {
            if count >= MAX_DISCOVERED_SKILLS {
                tracing::warn!(
                    "Skill discovery cap reached ({} skills), skipping remaining",
                    MAX_DISCOVERED_SKILLS
                );
                break;
            }

            let path = entry.path();
            let meta = match tokio::fs::symlink_metadata(&path).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!("Failed to stat {:?}: {}", path, e);
                    continue;
                }
            };

            // Reject symlinks
            if meta.is_symlink() {
                tracing::warn!(
                    "Skipping symlink in skills directory: {:?}",
                    path.file_name().unwrap_or_default()
                );
                continue;
            }

            // Case 1: Subdirectory containing SKILL.md
            if meta.is_dir() {
                let skill_md = path.join("SKILL.md");
                if tokio::fs::try_exists(&skill_md).await.unwrap_or(false) {
                    count += 1;
                    let source = make_source(path.clone());
                    match self.load_skill_md(&skill_md, trust, source).await {
                        Ok((name, skill)) => {
                            tracing::info!("Loaded skill: {}", name);
                            results.push((name, skill));
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load skill from {:?}: {}",
                                path.file_name().unwrap_or_default(),
                                e
                            );
                        }
                    }
                }
                continue;
            }

            // Case 2: Flat SKILL.md directly in the directory
            if meta.is_file()
                && let Some(fname) = path.file_name().and_then(|f| f.to_str())
                && fname == "SKILL.md"
            {
                count += 1;
                let source = make_source(dir.to_path_buf());
                match self.load_skill_md(&path, trust, source).await {
                    Ok((name, skill)) => {
                        tracing::info!("Loaded skill: {}", name);
                        results.push((name, skill));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load skill from {:?}: {}", fname, e);
                    }
                }
            }
        }

        results
    }

    /// Load a single SKILL.md file.
    async fn load_skill_md(
        &self,
        path: &Path,
        trust: SkillTrust,
        source: SkillSource,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        // Check for symlink at the file level
        let file_meta =
            tokio::fs::symlink_metadata(path)
                .await
                .map_err(|e| SkillRegistryError::ReadError {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                })?;

        if file_meta.is_symlink() {
            return Err(SkillRegistryError::SymlinkDetected {
                path: path.display().to_string(),
            });
        }

        // Read and check size
        let raw_bytes = tokio::fs::read(path)
            .await
            .map_err(|e| SkillRegistryError::ReadError {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;

        if raw_bytes.len() as u64 > MAX_PROMPT_FILE_SIZE {
            return Err(SkillRegistryError::FileTooLarge {
                name: path.display().to_string(),
                size: raw_bytes.len() as u64,
                max: MAX_PROMPT_FILE_SIZE,
            });
        }

        let raw_content =
            String::from_utf8(raw_bytes).map_err(|e| SkillRegistryError::ReadError {
                path: path.display().to_string(),
                reason: format!("Invalid UTF-8: {}", e),
            })?;

        // Normalize line endings before parsing to handle CRLF
        let normalized_content = normalize_line_endings(&raw_content);

        // Parse SKILL.md
        let parsed = parse_skill_md(&normalized_content).map_err(|e: SkillParseError| match e {
            SkillParseError::InvalidName { ref name } => SkillRegistryError::ParseError {
                name: name.clone(),
                reason: e.to_string(),
            },
            _ => SkillRegistryError::ParseError {
                name: path.display().to_string(),
                reason: e.to_string(),
            },
        })?;

        let manifest = parsed.manifest;
        let prompt_content = parsed.prompt_content;

        // Check gating requirements
        if let Some(ref meta) = manifest.metadata
            && let Some(ref openclaw) = meta.openclaw
        {
            let gating = check_requirements(&openclaw.requires);
            if !gating.passed {
                return Err(SkillRegistryError::GatingFailed {
                    name: manifest.name.clone(),
                    reason: gating.failures.join("; "),
                });
            }
        }

        // Check token budget (reject if prompt is > 2x declared budget)
        let approx_tokens = (prompt_content.len() as f64 * 0.75) as usize;
        let declared = manifest.activation.max_context_tokens;
        if declared > 0 && approx_tokens > declared * 2 {
            return Err(SkillRegistryError::TokenBudgetExceeded {
                name: manifest.name.clone(),
                approx_tokens,
                declared,
            });
        }

        // Compute content hash
        let content_hash = compute_hash(&prompt_content);

        // Compile regex patterns
        let compiled_patterns = LoadedSkill::compile_patterns(&manifest.activation.patterns);

        let name = manifest.name.clone();
        let skill = LoadedSkill {
            manifest,
            prompt_content,
            trust,
            source,
            content_hash,
            compiled_patterns,
        };

        Ok((name, skill))
    }

    /// Get all loaded skills.
    pub fn skills(&self) -> &[LoadedSkill] {
        &self.skills
    }

    /// Get the number of loaded skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }
}

/// Compute SHA-256 hash of content in the format "sha256:hex...".
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{:x}", result)
}

/// Helper to check gating for a `GatingRequirements`. Useful for callers that
/// don't have the full skill loaded yet.
pub fn check_gating(requirements: &GatingRequirements) -> crate::skills::gating::GatingResult {
    check_requirements(requirements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_discover_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_discover_nonexistent_dir() {
        let mut registry = SkillRegistry::new(PathBuf::from("/nonexistent/skills"));
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_load_subdirectory_layout() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("test-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\nactivation:\n  keywords: [\"test\"]\n---\n\nYou are a helpful test assistant.\n",
        ).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(loaded, vec!["test-skill"]);
        assert_eq!(registry.count(), 1);

        let skill = &registry.skills()[0];
        assert_eq!(skill.trust, SkillTrust::Trusted);
        assert!(skill.prompt_content.contains("helpful test assistant"));
    }

    #[tokio::test]
    async fn test_workspace_overrides_user() {
        let user_dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();

        // Create skill in user dir
        let user_skill = user_dir.path().join("my-skill");
        fs::create_dir(&user_skill).unwrap();
        fs::write(
            user_skill.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nUser version.\n",
        )
        .unwrap();

        // Create same-named skill in workspace dir
        let ws_skill = ws_dir.path().join("my-skill");
        fs::create_dir(&ws_skill).unwrap();
        fs::write(
            ws_skill.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nWorkspace version.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
            .with_workspace_dir(ws_dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(loaded, vec!["my-skill"]);
        assert_eq!(registry.count(), 1);
        assert!(registry.skills()[0].prompt_content.contains("Workspace"));
    }

    #[tokio::test]
    async fn test_gating_failure_skips_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("gated-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: gated-skill\nmetadata:\n  openclaw:\n    requires:\n      bins: [\"__nonexistent_bin__\"]\n---\n\nGated prompt.\n",
        ).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_symlink_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("real-skill");
        fs::create_dir(&real_dir).unwrap();
        fs::write(
            real_dir.join("SKILL.md"),
            "---\nname: real-skill\n---\n\nTest.\n",
        )
        .unwrap();

        let skills_dir = dir.path().join("skills");
        fs::create_dir(&skills_dir).unwrap();
        std::os::unix::fs::symlink(&real_dir, skills_dir.join("linked-skill")).unwrap();

        let mut registry = SkillRegistry::new(skills_dir);
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_file_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("big-skill");
        fs::create_dir(&skill_dir).unwrap();

        let big_content = format!(
            "---\nname: big-skill\n---\n\n{}",
            "x".repeat((MAX_PROMPT_FILE_SIZE + 1) as usize)
        );
        fs::write(skill_dir.join("SKILL.md"), &big_content).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_invalid_skill_md_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("bad-skill");
        fs::create_dir(&skill_dir).unwrap();

        // Missing frontmatter
        fs::write(skill_dir.join("SKILL.md"), "Just plain text").unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_line_ending_normalization() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("crlf-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\r\nname: crlf-skill\r\n---\r\n\r\nline1\r\nline2\r\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;

        assert_eq!(registry.count(), 1);
        let skill = &registry.skills()[0];
        assert_eq!(skill.prompt_content, "line1\nline2\n");
    }

    #[tokio::test]
    async fn test_token_budget_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("big-prompt");
        fs::create_dir(&skill_dir).unwrap();

        let big_prompt = "word ".repeat(4000);
        let content = format!(
            "---\nname: big-prompt\nactivation:\n  max_context_tokens: 100\n---\n\n{}",
            big_prompt
        );
        fs::write(skill_dir.join("SKILL.md"), &content).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let h1 = compute_hash("hello world");
        let h2 = compute_hash("hello world");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn test_compute_hash_different_content() {
        let h1 = compute_hash("hello");
        let h2 = compute_hash("world");
        assert_ne!(h1, h2);
    }
}
