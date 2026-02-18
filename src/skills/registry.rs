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

    #[error("Skill '{name}' already exists")]
    AlreadyExists { name: String },

    #[error("Cannot remove skill '{name}': {reason}")]
    CannotRemove { name: String, reason: String },

    #[error("Failed to write skill file {path}: {reason}")]
    WriteError { path: String, reason: String },
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
                .discover_from_dir(ws_dir, SkillTrust::Trusted, SkillSource::Workspace)
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
        // ~4 bytes per token for English prose = ~0.25 tokens per byte
        let approx_tokens = (prompt_content.len() as f64 * 0.25) as usize;
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

    /// Check if a skill with the given name is loaded.
    pub fn has(&self, name: &str) -> bool {
        self.skills.iter().any(|s| s.manifest.name == name)
    }

    /// Find a skill by name.
    pub fn find_by_name(&self, name: &str) -> Option<&LoadedSkill> {
        self.skills.iter().find(|s| s.manifest.name == name)
    }

    /// Perform the disk I/O and loading for a skill install.
    ///
    /// This is a static method so it doesn't borrow `&self`, allowing callers
    /// to drop their registry lock before awaiting.
    pub async fn prepare_install_to_disk(
        user_dir: &Path,
        skill_name: &str,
        normalized_content: &str,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        let skill_dir = user_dir.join(skill_name);
        tokio::fs::create_dir_all(&skill_dir).await.map_err(|e| {
            SkillRegistryError::WriteError {
                path: skill_dir.display().to_string(),
                reason: e.to_string(),
            }
        })?;

        let skill_path = skill_dir.join("SKILL.md");
        tokio::fs::write(&skill_path, normalized_content)
            .await
            .map_err(|e| SkillRegistryError::WriteError {
                path: skill_path.display().to_string(),
                reason: e.to_string(),
            })?;

        // Load by re-reading from disk (validates round-trip)
        let source = SkillSource::User(skill_dir);
        // Use a temporary registry-less load (load_skill_md_standalone)
        load_skill_md_standalone(&skill_path, SkillTrust::Installed, source).await
    }

    /// Commit a prepared skill into the in-memory registry.
    ///
    /// This is a fast, synchronous operation that only adds to the Vec.
    /// Call after `prepare_install` completes.
    pub fn commit_install(
        &mut self,
        name: &str,
        skill: LoadedSkill,
    ) -> Result<(), SkillRegistryError> {
        // Re-check for duplicates (another thread may have installed between prepare and commit)
        if self.has(name) {
            return Err(SkillRegistryError::AlreadyExists {
                name: name.to_string(),
            });
        }
        self.skills.push(skill);
        tracing::info!("Installed skill: {}", name);
        Ok(())
    }

    /// Install a skill at runtime from SKILL.md content.
    ///
    /// Convenience method that parses, writes to disk, and commits in-memory.
    /// When called through tool execution where a lock is involved, prefer using
    /// `prepare_install_to_disk` + `commit_install` separately to minimize lock
    /// hold time.
    pub async fn install_skill(&mut self, content: &str) -> Result<String, SkillRegistryError> {
        let normalized = normalize_line_endings(content);
        let parsed = parse_skill_md(&normalized).map_err(|e: SkillParseError| match e {
            SkillParseError::InvalidName { ref name } => SkillRegistryError::ParseError {
                name: name.clone(),
                reason: e.to_string(),
            },
            _ => SkillRegistryError::ParseError {
                name: "(install)".to_string(),
                reason: e.to_string(),
            },
        })?;
        let skill_name = parsed.manifest.name.clone();
        if self.has(&skill_name) {
            return Err(SkillRegistryError::AlreadyExists { name: skill_name });
        }
        let user_dir = self.user_dir.clone();
        let (name, skill) =
            Self::prepare_install_to_disk(&user_dir, &skill_name, &normalized).await?;
        self.commit_install(&name, skill)?;
        Ok(name)
    }

    /// Validate that a skill can be removed and return its filesystem path.
    ///
    /// Performs validation without modifying state. Callers can then do async
    /// filesystem cleanup without holding the registry lock, and call
    /// `commit_remove` afterward.
    pub fn validate_remove(&self, name: &str) -> Result<PathBuf, SkillRegistryError> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.manifest.name == name)
            .ok_or_else(|| SkillRegistryError::NotFound(name.to_string()))?;

        let skill = &self.skills[idx];

        match &skill.source {
            SkillSource::User(path) => Ok(path.clone()),
            SkillSource::Workspace(_) => Err(SkillRegistryError::CannotRemove {
                name: name.to_string(),
                reason: "workspace skills cannot be removed via this interface".to_string(),
            }),
            SkillSource::Bundled(_) => Err(SkillRegistryError::CannotRemove {
                name: name.to_string(),
                reason: "bundled skills cannot be removed".to_string(),
            }),
            SkillSource::Registry { .. } => Err(SkillRegistryError::CannotRemove {
                name: name.to_string(),
                reason: "registry skills should be uninstalled, not removed".to_string(),
            }),
        }
    }

    /// Remove a skill's files from disk (async I/O).
    ///
    /// Call after `validate_remove` and before `commit_remove`.
    pub async fn delete_skill_files(path: &Path) -> Result<(), SkillRegistryError> {
        let skill_md = path.join("SKILL.md");
        if tokio::fs::try_exists(&skill_md).await.unwrap_or(false) {
            tokio::fs::remove_file(&skill_md).await.map_err(|e| {
                SkillRegistryError::WriteError {
                    path: skill_md.display().to_string(),
                    reason: e.to_string(),
                }
            })?;
            // Remove the directory if empty
            let _ = tokio::fs::remove_dir(path).await;
        }
        Ok(())
    }

    /// Remove a skill from the in-memory registry.
    ///
    /// Fast synchronous operation. Call after filesystem cleanup.
    pub fn commit_remove(&mut self, name: &str) -> Result<(), SkillRegistryError> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.manifest.name == name)
            .ok_or_else(|| SkillRegistryError::NotFound(name.to_string()))?;

        self.skills.remove(idx);
        tracing::info!("Removed skill: {}", name);
        Ok(())
    }

    /// Remove a skill by name.
    ///
    /// Convenience method that combines validation, file deletion, and in-memory
    /// removal. When called through tool execution, prefer using the split
    /// validate/delete/commit methods to minimize lock hold time.
    pub async fn remove_skill(&mut self, name: &str) -> Result<(), SkillRegistryError> {
        let path = self.validate_remove(name)?;
        Self::delete_skill_files(&path).await?;
        self.commit_remove(name)
    }

    /// Clear all loaded skills and re-discover from disk.
    pub async fn reload(&mut self) -> Vec<String> {
        self.skills.clear();
        self.discover_all().await
    }

    /// Get the user skills directory path.
    pub fn user_dir(&self) -> &Path {
        &self.user_dir
    }
}

/// Load a single SKILL.md file without requiring a SkillRegistry instance.
///
/// This is used by `prepare_install_to_disk` to avoid borrowing the registry
/// across async boundaries.
async fn load_skill_md_standalone(
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

    let raw_content = String::from_utf8(raw_bytes).map_err(|e| SkillRegistryError::ReadError {
        path: path.display().to_string(),
        reason: format!("Invalid UTF-8: {}", e),
    })?;

    let normalized_content = normalize_line_endings(&raw_content);

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

    let approx_tokens = (prompt_content.len() as f64 * 0.25) as usize;
    let declared = manifest.activation.max_context_tokens;
    if declared > 0 && approx_tokens > declared * 2 {
        return Err(SkillRegistryError::TokenBudgetExceeded {
            name: manifest.name.clone(),
            approx_tokens,
            declared,
        });
    }

    let content_hash = compute_hash(&prompt_content);
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

    #[tokio::test]
    async fn test_has_and_find_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nPrompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;

        assert!(registry.has("my-skill"));
        assert!(!registry.has("nonexistent"));
        assert!(registry.find_by_name("my-skill").is_some());
        assert!(registry.find_by_name("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_install_skill_from_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let content =
            "---\nname: test-install\ndescription: Installed skill\n---\n\nInstalled prompt.\n";
        let name = registry.install_skill(content).await.unwrap();

        assert_eq!(name, "test-install");
        assert!(registry.has("test-install"));
        assert_eq!(registry.count(), 1);

        // Verify file was written to disk
        let skill_path = dir.path().join("test-install").join("SKILL.md");
        assert!(skill_path.exists());
    }

    #[tokio::test]
    async fn test_install_duplicate_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let content = "---\nname: dup-skill\n---\n\nPrompt.\n";
        registry.install_skill(content).await.unwrap();

        let result = registry.install_skill(content).await;
        assert!(matches!(
            result,
            Err(SkillRegistryError::AlreadyExists { .. })
        ));
    }

    #[tokio::test]
    async fn test_remove_user_skill() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let content = "---\nname: removable\n---\n\nPrompt.\n";
        registry.install_skill(content).await.unwrap();
        assert!(registry.has("removable"));

        registry.remove_skill("removable").await.unwrap();
        assert!(!registry.has("removable"));
        assert_eq!(registry.count(), 0);
    }

    #[tokio::test]
    async fn test_remove_workspace_skill_rejected() {
        let user_dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();

        let ws_skill = ws_dir.path().join("ws-skill");
        fs::create_dir(&ws_skill).unwrap();
        fs::write(
            ws_skill.join("SKILL.md"),
            "---\nname: ws-skill\n---\n\nWorkspace prompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
            .with_workspace_dir(ws_dir.path().to_path_buf());
        registry.discover_all().await;

        let result = registry.remove_skill("ws-skill").await;
        assert!(matches!(
            result,
            Err(SkillRegistryError::CannotRemove { .. })
        ));
    }

    #[tokio::test]
    async fn test_remove_nonexistent_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let result = registry.remove_skill("nonexistent").await;
        assert!(matches!(result, Err(SkillRegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_reload_clears_and_rediscovers() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("persist-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: persist-skill\n---\n\nPrompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;
        assert_eq!(registry.count(), 1);

        let loaded = registry.reload().await;
        assert_eq!(loaded, vec!["persist-skill"]);
        assert_eq!(registry.count(), 1);
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
