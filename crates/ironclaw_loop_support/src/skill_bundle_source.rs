use async_trait::async_trait;
use ironclaw_skills::{SkillTrust, validate_skill_name};
use ironclaw_turns::run_profile::{LoopRunContext, SkillVisibility};
use thiserror::Error;

const SKILL_MD_FILE: &str = "SKILL.md";

/// Host-owned source of portable skill bundles.
///
/// Implementations may enumerate scoped filesystems, extension state, or other
/// host-approved stores. This port only exposes virtual bundle identifiers and
/// bundle-relative file paths so callers cannot observe raw host paths or
/// backend internals.
#[async_trait]
pub trait SkillBundleSource: Send + Sync {
    async fn list_skill_bundles(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError>;

    async fn read_skill_bundle_file(
        &self,
        run_context: &LoopRunContext,
        bundle_id: &SkillBundleId,
        path: &SkillFilePath,
    ) -> Result<Vec<u8>, SkillBundleSourceError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum SkillSourceKind {
    System,
    User,
    TenantShared,
}

impl SkillSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::TenantShared => "tenant_shared",
        }
    }
}

impl std::fmt::Display for SkillSourceKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct SkillBundleId {
    source_kind: SkillSourceKind,
    name: String,
}

impl SkillBundleId {
    pub fn new(
        source_kind: SkillSourceKind,
        name: impl AsRef<str>,
    ) -> Result<Self, SkillBundleSourceError> {
        let name = name.as_ref().trim();
        if !validate_skill_name(name) {
            return Err(SkillBundleSourceError::InvalidBundleId);
        }
        Ok(Self {
            source_kind,
            name: name.to_string(),
        })
    }

    pub fn source_kind(&self) -> SkillSourceKind {
        self.source_kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for SkillBundleId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.source_kind, self.name)
    }
}

/// Bundle-relative file path.
///
/// This is intentionally not a host path. It rejects absolute paths, URL-like
/// strings, Windows drive prefixes, backslashes, empty components, and `.`/`..`
/// components so future filesystem-backed sources can join it to a scoped
/// virtual root without mount escape ambiguity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct SkillFilePath(String);

impl SkillFilePath {
    pub fn new(path: impl AsRef<str>) -> Result<Self, SkillBundleSourceError> {
        let path = path.as_ref().trim();
        if path.is_empty()
            || path.starts_with('/')
            || path.starts_with('~')
            || path.contains('\\')
            || path.contains(':')
            || path
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | "..") || has_control(part))
        {
            return Err(SkillBundleSourceError::InvalidFilePath);
        }
        Ok(Self(path.to_string()))
    }

    pub fn skill_md() -> Self {
        Self(SKILL_MD_FILE.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkillFilePath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBundleProvenance {
    pub source_kind: SkillSourceKind,
    pub content_hash: Option<String>,
}

impl SkillBundleProvenance {
    pub fn new(source_kind: SkillSourceKind) -> Self {
        Self {
            source_kind,
            content_hash: None,
        }
    }

    pub fn with_content_hash(mut self, content_hash: impl Into<String>) -> Self {
        self.content_hash = Some(content_hash.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBundleDescriptor {
    pub id: SkillBundleId,
    pub skill_md_path: SkillFilePath,
    pub trust: Option<SkillTrust>,
    pub visibility: Option<SkillVisibility>,
    pub provenance: SkillBundleProvenance,
}

impl SkillBundleDescriptor {
    pub fn new(
        id: SkillBundleId,
        trust: Option<SkillTrust>,
        visibility: Option<SkillVisibility>,
    ) -> Self {
        Self {
            provenance: SkillBundleProvenance::new(id.source_kind()),
            id,
            skill_md_path: SkillFilePath::skill_md(),
            trust,
            visibility,
        }
    }

    pub fn with_skill_md_path(mut self, path: SkillFilePath) -> Self {
        self.skill_md_path = path;
        self
    }

    pub fn with_provenance(mut self, provenance: SkillBundleProvenance) -> Self {
        self.provenance = provenance;
        self
    }

    pub fn ordering_key(&self) -> (&'static str, &str, &str) {
        (
            self.id.source_kind().as_str(),
            self.id.name(),
            self.skill_md_path.as_str(),
        )
    }
}

pub fn sort_skill_bundle_descriptors(descriptors: &mut [SkillBundleDescriptor]) {
    descriptors.sort_by(|left, right| left.ordering_key().cmp(&right.ordering_key()));
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillBundleSourceError {
    #[error("skill bundle source unavailable")]
    SourceUnavailable,
    #[error("skill bundle id is invalid")]
    InvalidBundleId,
    #[error("skill bundle file path is invalid")]
    InvalidFilePath,
    #[error("skill bundle not found")]
    BundleNotFound,
    #[error("skill bundle file not found")]
    FileNotFound,
    #[error("skill bundle access denied")]
    PermissionDenied,
    #[error("skill bundle content too large")]
    ContentTooLarge,
    #[error("skill bundle source internal error")]
    Internal,
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(source_kind: SkillSourceKind, name: &str) -> SkillBundleId {
        SkillBundleId::new(source_kind, name).unwrap()
    }

    fn descriptor(source_kind: SkillSourceKind, name: &str) -> SkillBundleDescriptor {
        SkillBundleDescriptor::new(
            id(source_kind, name),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
        )
    }

    #[test]
    fn skill_bundle_id_validates_skill_names() {
        assert_eq!(
            SkillBundleId::new(SkillSourceKind::User, "code-review")
                .unwrap()
                .to_string(),
            "user:code-review"
        );

        for invalid in ["", "has space", "../escape", "/absolute", "ümlaut"] {
            assert!(SkillBundleId::new(SkillSourceKind::User, invalid).is_err());
        }
    }

    #[test]
    fn skill_file_path_accepts_bundle_relative_files() {
        assert_eq!(SkillFilePath::skill_md().as_str(), "SKILL.md");
        assert_eq!(
            SkillFilePath::new("references/policy.md").unwrap().as_str(),
            "references/policy.md"
        );
    }

    #[test]
    fn skill_file_path_rejects_mount_escape_and_raw_host_shapes() {
        for invalid in [
            "",
            "/skills/code-review/SKILL.md",
            "../SKILL.md",
            "references/../SKILL.md",
            "references//SKILL.md",
            "C:/Users/alice/SKILL.md",
            "https://example.test/SKILL.md",
            "~/skills/SKILL.md",
            "references\\SKILL.md",
            "references/\u{0000}/SKILL.md",
        ] {
            assert!(
                SkillFilePath::new(invalid).is_err(),
                "{invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn skill_bundle_descriptors_sort_deterministically_without_host_paths() {
        let mut descriptors = vec![
            descriptor(SkillSourceKind::User, "beta"),
            descriptor(SkillSourceKind::System, "beta"),
            descriptor(SkillSourceKind::TenantShared, "alpha"),
            descriptor(SkillSourceKind::User, "alpha")
                .with_skill_md_path(SkillFilePath::new("nested/SKILL.md").unwrap()),
            descriptor(SkillSourceKind::User, "alpha"),
        ];

        sort_skill_bundle_descriptors(&mut descriptors);

        let ordered: Vec<String> = descriptors
            .iter()
            .map(|descriptor| {
                format!(
                    "{}:{}:{}",
                    descriptor.id.source_kind(),
                    descriptor.id.name(),
                    descriptor.skill_md_path
                )
            })
            .collect();
        assert_eq!(
            ordered,
            vec![
                "system:beta:SKILL.md",
                "tenant_shared:alpha:SKILL.md",
                "user:alpha:SKILL.md",
                "user:alpha:nested/SKILL.md",
                "user:beta:SKILL.md",
            ]
        );
    }
}
