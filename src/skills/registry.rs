//! Skill registry for discovering, loading, and managing available skills.
//!
//! Skills are discovered from the local filesystem (`~/.ironclaw/skills/`) and loaded
//! into memory with manifest parsing, content hashing, and security scanning.
//! Uses async I/O throughout to avoid blocking the tokio runtime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::skills::scanner::SkillScanner;
use crate::skills::{
    LoadedSkill, MAX_PROMPT_FILE_SIZE, SkillManifest, SkillSource, SkillTrust,
    normalize_line_endings, validate_skill_name,
};

/// Error type for skill registry operations.
#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryError {
    #[error("Skill not found: {0}")]
    NotFound(String),

    #[error("Failed to read skill directory {path}: {reason}")]
    ReadError { path: String, reason: String },

    #[error("Invalid manifest in skill '{name}': {reason}")]
    InvalidManifest { name: String, reason: String },

    #[error("Invalid skill name '{name}': must match [a-zA-Z0-9][a-zA-Z0-9._-]{{0,63}}")]
    InvalidName { name: String },

    #[error("Skill '{name}' blocked by scanner: {reason}")]
    Blocked { name: String, reason: String },

    #[error("Integrity check failed for skill '{name}': expected {expected}, got {actual}")]
    IntegrityMismatch {
        name: String,
        expected: String,
        actual: String,
    },

    #[error("Integrity hash required for {trust} skill '{name}' but not provided")]
    IntegrityRequired { name: String, trust: String },

    #[error("Prompt file too large for skill '{name}': {size} bytes (max {max} bytes)")]
    PromptTooLarge { name: String, size: u64, max: u64 },
}

/// Registry of available skills.
pub struct SkillRegistry {
    /// Loaded skills keyed by name.
    skills: Arc<RwLock<HashMap<String, LoadedSkill>>>,
    /// Scanner for checking skill content.
    scanner: SkillScanner,
    /// Base directory for local skills.
    local_dir: PathBuf,
}

impl SkillRegistry {
    /// Create a new skill registry.
    pub fn new(local_dir: PathBuf) -> Self {
        Self {
            skills: Arc::new(RwLock::new(HashMap::new())),
            scanner: SkillScanner::new(),
            local_dir,
        }
    }

    /// Discover and load local skills from the configured directory.
    ///
    /// Scans `~/.ironclaw/skills/` for directories containing `skill.toml` + `prompt.md`.
    /// Skills that fail to load are logged and skipped.
    pub async fn discover_local(&self) -> Vec<String> {
        let mut loaded = Vec::new();

        let local_dir = self.local_dir.clone();
        if !tokio::fs::try_exists(&local_dir).await.unwrap_or(false) {
            tracing::debug!("Skills directory does not exist: {:?}", local_dir);
            return loaded;
        }

        let mut entries = match tokio::fs::read_dir(&local_dir).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!("Failed to read skills directory {:?}: {}", local_dir, e);
                return loaded;
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let is_dir = tokio::fs::metadata(&path)
                .await
                .map(|m| m.is_dir())
                .unwrap_or(false);
            if !is_dir {
                continue;
            }

            let manifest_path = path.join("skill.toml");
            let prompt_path = path.join("prompt.md");

            let manifest_exists = tokio::fs::try_exists(&manifest_path).await.unwrap_or(false);
            let prompt_exists = tokio::fs::try_exists(&prompt_path).await.unwrap_or(false);

            if !manifest_exists || !prompt_exists {
                tracing::debug!(
                    "Skipping {:?}: missing skill.toml or prompt.md",
                    path.file_name().unwrap_or_default()
                );
                continue;
            }

            match self
                .load_skill(&manifest_path, &prompt_path, SkillTrust::Local)
                .await
            {
                Ok(name) => {
                    tracing::info!("Loaded local skill: {}", name);
                    loaded.push(name);
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

        loaded
    }

    /// Load a skill from manifest and prompt files.
    pub async fn load_skill(
        &self,
        manifest_path: &Path,
        prompt_path: &Path,
        trust: SkillTrust,
    ) -> Result<String, SkillRegistryError> {
        // Check prompt file size before reading
        let prompt_metadata =
            tokio::fs::metadata(prompt_path)
                .await
                .map_err(|e| SkillRegistryError::ReadError {
                    path: prompt_path.display().to_string(),
                    reason: e.to_string(),
                })?;

        if prompt_metadata.len() > MAX_PROMPT_FILE_SIZE {
            // We don't know the name yet, use filename
            let name = prompt_path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            return Err(SkillRegistryError::PromptTooLarge {
                name,
                size: prompt_metadata.len(),
                max: MAX_PROMPT_FILE_SIZE,
            });
        }

        // Read manifest (async)
        let manifest_content = tokio::fs::read_to_string(manifest_path)
            .await
            .map_err(|e| SkillRegistryError::ReadError {
                path: manifest_path.display().to_string(),
                reason: e.to_string(),
            })?;

        let mut manifest: SkillManifest =
            toml::from_str(&manifest_content).map_err(|e| SkillRegistryError::InvalidManifest {
                name: manifest_path.display().to_string(),
                reason: e.to_string(),
            })?;

        // Validate skill name
        if !validate_skill_name(&manifest.skill.name) {
            return Err(SkillRegistryError::InvalidName {
                name: manifest.skill.name.clone(),
            });
        }

        // Enforce keyword/pattern limits to prevent scoring manipulation
        manifest.activation.enforce_limits();

        // Read prompt content (async)
        let raw_prompt = tokio::fs::read_to_string(prompt_path).await.map_err(|e| {
            SkillRegistryError::ReadError {
                path: prompt_path.display().to_string(),
                reason: e.to_string(),
            }
        })?;

        // Normalize line endings before hashing for cross-platform consistency
        let prompt_content = normalize_line_endings(&raw_prompt);

        // Compute content hash
        let content_hash = compute_hash(&prompt_content);

        // Require integrity hash for non-Local trust tiers
        if trust != SkillTrust::Local && manifest.integrity.prompt_hash.is_none() {
            return Err(SkillRegistryError::IntegrityRequired {
                name: manifest.skill.name.clone(),
                trust: trust.to_string(),
            });
        }

        // Verify integrity if hash is specified in manifest
        if let Some(ref expected_hash) = manifest.integrity.prompt_hash {
            if *expected_hash != content_hash {
                return Err(SkillRegistryError::IntegrityMismatch {
                    name: manifest.skill.name.clone(),
                    expected: expected_hash.clone(),
                    actual: content_hash,
                });
            }
        }

        // Run security scanner
        let scan_result = self.scanner.scan(&prompt_content);

        if scan_result.blocked && trust != SkillTrust::Local {
            return Err(SkillRegistryError::Blocked {
                name: manifest.skill.name.clone(),
                reason: scan_result.summary,
            });
        }

        // For local skills, log warnings but don't block
        if scan_result.blocked {
            tracing::warn!(
                "Local skill '{}' has scanner warnings (loaded anyway due to local trust): {}",
                manifest.skill.name,
                scan_result.summary
            );
        }

        // Pre-compile regex patterns at load time to avoid per-message compilation
        let compiled_patterns = LoadedSkill::compile_patterns(&manifest.activation.patterns);

        let name = manifest.skill.name.clone();
        let source_dir = manifest_path
            .parent()
            .unwrap_or(manifest_path)
            .to_path_buf();

        let skill = LoadedSkill {
            manifest,
            prompt_content,
            trust,
            source: SkillSource::Local(source_dir),
            content_hash,
            scan_warnings: scan_result.warning_messages(),
            compiled_patterns,
        };

        // Register
        let mut skills = self.skills.write().await;
        skills.insert(name.clone(), skill);

        Ok(name)
    }

    /// Get all available (loaded) skills.
    pub async fn available(&self) -> Vec<LoadedSkill> {
        let skills = self.skills.read().await;
        skills.values().cloned().collect()
    }

    /// Get a specific skill by name.
    pub async fn get(&self, name: &str) -> Option<LoadedSkill> {
        let skills = self.skills.read().await;
        skills.get(name).cloned()
    }

    /// Remove a skill by name.
    pub async fn remove(&self, name: &str) -> bool {
        let mut skills = self.skills.write().await;
        skills.remove(name).is_some()
    }

    /// Get the number of loaded skills.
    pub async fn count(&self) -> usize {
        let skills = self.skills.read().await;
        skills.len()
    }

    /// Get the scanner for external use.
    pub fn scanner(&self) -> &SkillScanner {
        &self.scanner
    }
}

/// Compute SHA-256 hash of content in the format "sha256:hex...".
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{:x}", result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_discover_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_local().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_discover_nonexistent_dir() {
        let registry = SkillRegistry::new(PathBuf::from("/nonexistent/skills"));
        let loaded = registry.discover_local().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_load_valid_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("test-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
name = "test-skill"
description = "A test skill"
tags = ["test"]

[activation]
keywords = ["test"]
"#,
        )
        .unwrap();

        fs::write(
            skill_dir.join("prompt.md"),
            "You are a helpful test assistant.",
        )
        .unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_local().await;

        assert_eq!(loaded, vec!["test-skill"]);
        assert_eq!(registry.count().await, 1);

        let skill = registry.get("test-skill").await.unwrap();
        assert_eq!(skill.trust, SkillTrust::Local);
        assert_eq!(skill.prompt_content, "You are a helpful test assistant.");
    }

    #[tokio::test]
    async fn test_skip_dir_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("incomplete-skill");
        fs::create_dir(&skill_dir).unwrap();
        // Only prompt, no manifest
        fs::write(skill_dir.join("prompt.md"), "test").unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_local().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_integrity_check_pass() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = "Test content";
        let hash = compute_hash(prompt);

        let skill_dir = dir.path().join("hash-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("skill.toml"),
            format!(
                r#"
[skill]
name = "hash-skill"

[integrity]
prompt_hash = "{}"
"#,
                hash
            ),
        )
        .unwrap();

        fs::write(skill_dir.join("prompt.md"), prompt).unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_local().await;
        assert_eq!(loaded, vec!["hash-skill"]);
    }

    #[tokio::test]
    async fn test_integrity_check_fail() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("bad-hash");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
name = "bad-hash"

[integrity]
prompt_hash = "sha256:0000000000000000"
"#,
        )
        .unwrap();

        fs::write(skill_dir.join("prompt.md"), "Different content").unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_local().await;
        assert!(loaded.is_empty()); // Should fail integrity check
    }

    #[tokio::test]
    async fn test_remove_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("removable");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            "[skill]\nname = \"removable\"",
        )
        .unwrap();
        fs::write(skill_dir.join("prompt.md"), "test").unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_local().await;
        assert_eq!(registry.count().await, 1);

        assert!(registry.remove("removable").await);
        assert_eq!(registry.count().await, 0);
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

    #[tokio::test]
    async fn test_invalid_skill_name_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("bad-name");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            "[skill]\nname = \"has spaces in name\"",
        )
        .unwrap();
        fs::write(skill_dir.join("prompt.md"), "test").unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_local().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_prompt_file_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("big-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            "[skill]\nname = \"big-skill\"",
        )
        .unwrap();
        // Write a file larger than MAX_PROMPT_FILE_SIZE
        let big_content = "x".repeat((MAX_PROMPT_FILE_SIZE + 1) as usize);
        fs::write(skill_dir.join("prompt.md"), &big_content).unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_local().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_line_ending_normalization() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("crlf-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            "[skill]\nname = \"crlf-skill\"",
        )
        .unwrap();
        // Write with CRLF endings
        fs::write(skill_dir.join("prompt.md"), "line1\r\nline2\r\n").unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_local().await;

        let skill = registry.get("crlf-skill").await.unwrap();
        // Content should be normalized to LF
        assert_eq!(skill.prompt_content, "line1\nline2\n");
    }

    #[tokio::test]
    async fn test_integrity_required_for_verified() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("no-hash");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(skill_dir.join("skill.toml"), "[skill]\nname = \"no-hash\"").unwrap();
        fs::write(skill_dir.join("prompt.md"), "test").unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        let result = registry
            .load_skill(
                &skill_dir.join("skill.toml"),
                &skill_dir.join("prompt.md"),
                SkillTrust::Verified,
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Integrity hash required"));
    }

    #[tokio::test]
    async fn test_compiled_patterns_cached() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("regex-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.toml"),
            r#"
[skill]
name = "regex-skill"

[activation]
keywords = ["test"]
patterns = ["(?i)\\bwrite\\b.*\\bemail\\b", "[invalid"]
"#,
        )
        .unwrap();
        fs::write(skill_dir.join("prompt.md"), "test").unwrap();

        let registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_local().await;

        let skill = registry.get("regex-skill").await.unwrap();
        // Only valid patterns should be compiled (1 out of 2)
        assert_eq!(skill.compiled_patterns.len(), 1);
    }
}
