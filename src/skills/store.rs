//! Persistent storage for installed skills.
//!
//! Skills are stored as `.skill.toml` files in `~/.ironclaw/skills/`.
//! Approval state (BLAKE3 hash of prompt at approval time) is tracked
//! in `.approvals.json` alongside the manifests.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::skills::analyzer::AnalysisVerdict;
use crate::skills::{SkillError, SkillManifest};

/// On-disk approval record for a single skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillApproval {
    /// BLAKE3 hash of the prompt content at approval time.
    pub prompt_hash: String, // hex-encoded
    pub approved_at: DateTime<Utc>,
    pub analysis_verdict: AnalysisVerdict,
}

/// A skill with its approval state.
pub struct StoredSkill {
    pub manifest: SkillManifest,
    pub approval: Option<SkillApproval>,
}

/// Manages the `~/.ironclaw/skills/` directory.
pub struct SkillStore {
    skills_dir: PathBuf,
}

/// Contents of `.approvals.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ApprovalsFile {
    #[serde(flatten)]
    approvals: HashMap<String, SkillApproval>,
}

impl SkillStore {
    /// Create a new store pointing to the given directory.
    ///
    /// Creates the directory if it doesn't exist.
    pub fn new(skills_dir: PathBuf) -> Result<Self, SkillError> {
        if !skills_dir.exists() {
            std::fs::create_dir_all(&skills_dir)?;
        }
        Ok(Self { skills_dir })
    }

    /// Save a skill manifest to disk.
    pub fn save(&self, manifest: &SkillManifest) -> Result<(), SkillError> {
        let path = self.manifest_path(manifest.name());
        let toml = manifest.to_toml()?;
        std::fs::write(&path, toml)?;
        Ok(())
    }

    /// Load a skill by name.
    pub fn load(&self, name: &str) -> Result<StoredSkill, SkillError> {
        let path = self.manifest_path(name);
        if !path.exists() {
            return Err(SkillError::NotFound {
                name: name.to_string(),
            });
        }

        let content = std::fs::read_to_string(&path)?;
        let manifest = SkillManifest::from_toml(&content)?;
        let approval = self.load_approval(name);

        Ok(StoredSkill { manifest, approval })
    }

    /// Remove a skill from disk.
    pub fn remove(&self, name: &str) -> Result<(), SkillError> {
        let path = self.manifest_path(name);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        // Also remove approval
        let mut approvals = self.load_approvals();
        approvals.approvals.remove(name);
        self.save_approvals(&approvals)?;

        Ok(())
    }

    /// List all installed skill names.
    pub fn list(&self) -> Result<Vec<String>, SkillError> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name.ends_with(".skill.toml") {
                names.push(name.trim_end_matches(".skill.toml").to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    /// List all installed skills with their full data.
    pub fn list_all(&self) -> Result<Vec<StoredSkill>, SkillError> {
        let names = self.list()?;
        let mut skills = Vec::new();
        for name in names {
            match self.load(&name) {
                Ok(skill) => skills.push(skill),
                Err(e) => {
                    tracing::warn!("Failed to load skill '{}': {}", name, e);
                }
            }
        }
        Ok(skills)
    }

    /// Record user approval for a skill.
    pub fn approve(
        &self,
        name: &str,
        prompt_content: &str,
        verdict: AnalysisVerdict,
    ) -> Result<(), SkillError> {
        let hash = blake3::hash(prompt_content.as_bytes());

        let approval = SkillApproval {
            prompt_hash: hash.to_hex().to_string(),
            approved_at: Utc::now(),
            analysis_verdict: verdict,
        };

        let mut approvals = self.load_approvals();
        approvals.approvals.insert(name.to_string(), approval);
        self.save_approvals(&approvals)?;

        Ok(())
    }

    /// Check if a skill's approval is still valid (content hasn't changed).
    ///
    /// Returns the approval hash bytes if valid, or None if the skill
    /// was never approved or the content has changed since approval.
    pub fn check_approval(&self, name: &str, current_prompt: &str) -> Option<[u8; 32]> {
        let approval = self.load_approval(name)?;
        let current_hash = blake3::hash(current_prompt.as_bytes());
        let current_hex = current_hash.to_hex().to_string();

        if approval.prompt_hash == current_hex {
            Some(*current_hash.as_bytes())
        } else {
            None
        }
    }

    /// Find a skill by its slash command binding.
    pub fn find_by_command(&self, command: &str) -> Result<Option<StoredSkill>, SkillError> {
        let skills = self.list_all()?;
        Ok(skills
            .into_iter()
            .find(|s| s.manifest.command() == Some(command)))
    }

    fn manifest_path(&self, name: &str) -> PathBuf {
        self.skills_dir.join(format!("{}.skill.toml", name))
    }

    fn approvals_path(&self) -> PathBuf {
        self.skills_dir.join(".approvals.json")
    }

    fn load_approvals(&self) -> ApprovalsFile {
        let path = self.approvals_path();
        if !path.exists() {
            return ApprovalsFile::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => ApprovalsFile::default(),
        }
    }

    fn load_approval(&self, name: &str) -> Option<SkillApproval> {
        let approvals = self.load_approvals();
        approvals.approvals.get(name).cloned()
    }

    fn save_approvals(&self, approvals: &ApprovalsFile) -> Result<(), SkillError> {
        let json =
            serde_json::to_string_pretty(approvals).map_err(|e| SkillError::Serialization {
                reason: e.to_string(),
            })?;
        std::fs::write(self.approvals_path(), json)?;
        Ok(())
    }
}

/// Compute the BLAKE3 hash of prompt content as raw bytes.
pub fn hash_prompt(content: &str) -> [u8; 32] {
    *blake3::hash(content.as_bytes()).as_bytes()
}

/// Default skills directory path.
pub fn default_skills_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".ironclaw").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".ironclaw/skills"))
}

#[cfg(test)]
mod tests {
    use crate::skills::analyzer::AnalysisVerdict;
    use crate::skills::manifest::SkillManifest;
    use crate::skills::store::{SkillStore, hash_prompt};

    fn test_manifest(name: &str) -> SkillManifest {
        let toml = format!(
            r#"
[skill]
name = "{name}"
version = "1.0.0"
description = "Test skill"

[prompt]
content = "Do the thing."
"#
        );
        SkillManifest::from_toml(&toml).expect("test manifest should parse")
    }

    fn test_manifest_with_command(name: &str, command: &str) -> SkillManifest {
        let toml = format!(
            r#"
[skill]
name = "{name}"
version = "1.0.0"
description = "Test skill"
command = "{command}"
activation = "command"

[prompt]
content = "Do the thing."
"#
        );
        SkillManifest::from_toml(&toml).expect("test manifest should parse")
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SkillStore::new(dir.path().to_path_buf()).expect("store");

        let manifest = test_manifest("save-test");
        store.save(&manifest).expect("save");

        let loaded = store.load("save-test").expect("load");
        assert_eq!(loaded.manifest.name(), "save-test");
        assert!(loaded.approval.is_none());
    }

    #[test]
    fn test_load_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SkillStore::new(dir.path().to_path_buf()).expect("store");

        assert!(store.load("nonexistent").is_err());
    }

    #[test]
    fn test_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SkillStore::new(dir.path().to_path_buf()).expect("store");

        store.save(&test_manifest("alpha")).expect("save");
        store.save(&test_manifest("beta")).expect("save");

        let names = store.list().expect("list");
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_remove() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SkillStore::new(dir.path().to_path_buf()).expect("store");

        store.save(&test_manifest("removeme")).expect("save");
        assert!(store.load("removeme").is_ok());

        store.remove("removeme").expect("remove");
        assert!(store.load("removeme").is_err());
    }

    #[test]
    fn test_approval_flow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SkillStore::new(dir.path().to_path_buf()).expect("store");

        let manifest = test_manifest("approved");
        store.save(&manifest).expect("save");

        // Not approved yet
        assert!(store.check_approval("approved", "Do the thing.").is_none());

        // Approve it
        store
            .approve("approved", "Do the thing.", AnalysisVerdict::Pass)
            .expect("approve");

        // Now it should be approved
        let hash = store.check_approval("approved", "Do the thing.");
        assert!(hash.is_some());

        // Change the content, approval should be invalidated
        assert!(
            store
                .check_approval("approved", "Do something else.")
                .is_none()
        );
    }

    #[test]
    fn test_find_by_command() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SkillStore::new(dir.path().to_path_buf()).expect("store");

        store
            .save(&test_manifest_with_command("pr-review", "review"))
            .expect("save");
        store
            .save(&test_manifest_with_command("debug-skill", "debug"))
            .expect("save");

        let found = store
            .find_by_command("review")
            .expect("find")
            .expect("should find");
        assert_eq!(found.manifest.name(), "pr-review");

        let not_found = store.find_by_command("nonexistent").expect("find");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_hash_prompt_deterministic() {
        let hash1 = hash_prompt("hello world");
        let hash2 = hash_prompt("hello world");
        assert_eq!(hash1, hash2);

        let hash3 = hash_prompt("different content");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_creates_dir_if_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("deep").join("nested").join("skills");
        assert!(!nested.exists());

        let store = SkillStore::new(nested.clone()).expect("store");
        store.save(&test_manifest("test")).expect("save");

        assert!(nested.exists());
    }
}
