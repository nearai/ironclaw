//! Skill loader: fetch manifests from URLs, GitHub repos, or local files.

use std::path::Path;

use crate::skills::{SkillError, SkillManifest};

/// Loads skill manifests from various sources.
pub struct SkillLoader {
    client: reqwest::Client,
}

impl SkillLoader {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Load a skill from a URL (raw TOML content).
    ///
    /// Supports:
    /// - Direct URLs to `.toml` files
    /// - `file://` URLs for local files
    /// - GitHub blob URLs (auto-converted to raw)
    pub async fn load_from_url(&self, url: &str) -> Result<SkillManifest, SkillError> {
        // Handle file:// URLs
        if let Some(path) = url.strip_prefix("file://") {
            return self.load_from_file(Path::new(path));
        }

        let raw_url = normalize_github_url(url);

        let response = self
            .client
            .get(&raw_url)
            .header("Accept", "text/plain")
            .send()
            .await
            .map_err(|e| SkillError::LoadError {
                location: raw_url.clone(),
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            return Err(SkillError::LoadError {
                location: raw_url,
                reason: format!("HTTP {}", response.status()),
            });
        }

        let content = response.text().await.map_err(|e| SkillError::LoadError {
            location: raw_url,
            reason: e.to_string(),
        })?;

        SkillManifest::from_toml(&content)
    }

    /// Load a skill from a local file path.
    pub fn load_from_file(&self, path: &Path) -> Result<SkillManifest, SkillError> {
        let content = std::fs::read_to_string(path).map_err(|e| SkillError::LoadError {
            location: path.display().to_string(),
            reason: e.to_string(),
        })?;

        SkillManifest::from_toml(&content)
    }
}

impl Default for SkillLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a GitHub blob URL to a raw content URL.
///
/// `github.com/user/repo/blob/main/skill.toml`
///   -> `raw.githubusercontent.com/user/repo/main/skill.toml`
fn normalize_github_url(url: &str) -> String {
    if url.contains("github.com") && url.contains("/blob/") {
        url.replace("github.com", "raw.githubusercontent.com")
            .replace("/blob/", "/")
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::skills::loader::{SkillLoader, normalize_github_url};

    #[test]
    fn test_normalize_github_blob_url() {
        let url = "https://github.com/alice/skills/blob/main/pr-review.skill.toml";
        let raw = normalize_github_url(url);
        assert_eq!(
            raw,
            "https://raw.githubusercontent.com/alice/skills/main/pr-review.skill.toml"
        );
    }

    #[test]
    fn test_normalize_already_raw() {
        let url = "https://raw.githubusercontent.com/alice/skills/main/pr-review.skill.toml";
        let raw = normalize_github_url(url);
        assert_eq!(raw, url);
    }

    #[test]
    fn test_normalize_non_github() {
        let url = "https://example.com/skills/my-skill.toml";
        let raw = normalize_github_url(url);
        assert_eq!(raw, url);
    }

    #[test]
    fn test_load_from_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.skill.toml");
        std::fs::write(
            &path,
            r#"
[skill]
name = "file-test"
version = "1.0.0"
description = "From file"

[prompt]
content = "Do stuff."
"#,
        )
        .expect("write");

        let loader = SkillLoader::new();
        let manifest = loader.load_from_file(&path).expect("load");
        assert_eq!(manifest.name(), "file-test");
    }

    #[test]
    fn test_load_from_file_not_found() {
        let loader = SkillLoader::new();
        assert!(
            loader
                .load_from_file(std::path::Path::new("/nonexistent.toml"))
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_load_from_file_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.skill.toml");
        std::fs::write(
            &path,
            r#"
[skill]
name = "file-url-test"
version = "1.0.0"
description = "From file URL"

[prompt]
content = "Do stuff."
"#,
        )
        .expect("write");

        let loader = SkillLoader::new();
        let url = format!("file://{}", path.display());
        let manifest = loader.load_from_url(&url).await.expect("load");
        assert_eq!(manifest.name(), "file-url-test");
    }
}
