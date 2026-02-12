//! Skill registry for discovering, loading, and managing available skills.
//!
//! Skills are discovered from the local filesystem (`~/.ironclaw/skills/`) and loaded
//! into memory with manifest parsing, content hashing, and security scanning.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::skills::scanner::SkillScanner;
use crate::skills::{LoadedSkill, SkillManifest, SkillSource, SkillTrust};

/// Error type for skill registry operations.
#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryError {
    #[error("Skill not found: {0}")]
    NotFound(String),

    #[error("Failed to read skill directory {path}: {reason}")]
    ReadError { path: String, reason: String },

    #[error("Invalid manifest in skill '{name}': {reason}")]
    InvalidManifest { name: String, reason: String },

    #[error("Skill '{name}' blocked by scanner: {reason}")]
    Blocked { name: String, reason: String },

    #[error("Integrity check failed for skill '{name}': expected {expected}, got {actual}")]
    IntegrityMismatch {
        name: String,
        expected: String,
        actual: String,
    },
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

        if !self.local_dir.exists() {
            tracing::debug!("Skills directory does not exist: {:?}", self.local_dir);
            return loaded;
        }

        let entries = match std::fs::read_dir(&self.local_dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!("Failed to read skills directory {:?}: {}", self.local_dir, e);
                return loaded;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("skill.toml");
            let prompt_path = path.join("prompt.md");

            if !manifest_path.exists() || !prompt_path.exists() {
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
        // Read manifest
        let manifest_content =
            std::fs::read_to_string(manifest_path).map_err(|e| SkillRegistryError::ReadError {
                path: manifest_path.display().to_string(),
                reason: e.to_string(),
            })?;

        let manifest: SkillManifest = toml::from_str(&manifest_content).map_err(|e| {
            SkillRegistryError::InvalidManifest {
                name: manifest_path.display().to_string(),
                reason: e.to_string(),
            }
        })?;

        // Read prompt content
        let prompt_content =
            std::fs::read_to_string(prompt_path).map_err(|e| SkillRegistryError::ReadError {
                path: prompt_path.display().to_string(),
                reason: e.to_string(),
            })?;

        // Compute content hash
        let content_hash = compute_hash(&prompt_content);

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
fn compute_hash(content: &str) -> String {
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
}
