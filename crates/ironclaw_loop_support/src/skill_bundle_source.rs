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
    /// Lists portable skill bundles visible to the supplied run context.
    ///
    /// Implementations should return descriptors only for bundles that the host
    /// authorizes for `run_context`. An empty vector means no bundles are
    /// available; source-level failures should be reported with
    /// [`SkillBundleSourceError`].
    async fn list_skill_bundles(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Vec<SkillBundleDescriptor>, SkillBundleSourceError>;

    /// Reads a bundle-relative file from a previously listed skill bundle.
    ///
    /// `path` is a validated [`SkillFilePath`], never a raw host path. Sources
    /// should enforce bundle visibility, reject access outside the virtual
    /// bundle root, and return [`SkillBundleSourceError::ContentTooLarge`] when
    /// host policy refuses the requested content size.
    async fn read_skill_bundle_file(
        &self,
        run_context: &LoopRunContext,
        bundle_id: &SkillBundleId,
        path: &SkillFilePath,
    ) -> Result<Vec<u8>, SkillBundleSourceError>;
}

/// Host-approved scope from which a skill bundle was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillSourceKind {
    System,
    TenantShared,
    User,
}

impl SkillSourceKind {
    /// Returns the stable string identifier used for descriptor ordering and display.
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

impl Ord for SkillSourceKind {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for SkillSourceKind {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct SkillBundleId {
    source_kind: SkillSourceKind,
    name: String,
}

impl SkillBundleId {
    /// Builds a virtual bundle identifier from a host source and validated skill name.
    pub fn new(
        source_kind: SkillSourceKind,
        name: impl AsRef<str>,
    ) -> Result<Self, SkillBundleSourceError> {
        let name = name.as_ref();
        if name.trim() != name || !validate_skill_name(name) {
            return Err(SkillBundleSourceError::InvalidBundleId);
        }
        Ok(Self {
            source_kind,
            name: name.to_string(),
        })
    }

    /// Returns the host source scope for this bundle.
    pub fn source_kind(&self) -> SkillSourceKind {
        self.source_kind
    }

    /// Returns the validated skill name component of this bundle id.
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
    /// Builds a validated bundle-relative file path.
    pub fn new(path: impl AsRef<str>) -> Result<Self, SkillBundleSourceError> {
        let path = path.as_ref();
        if path.trim() != path
            || path.is_empty()
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

    /// Returns the canonical descriptor path for a bundle's `SKILL.md` file.
    pub fn skill_md() -> Self {
        Self(SKILL_MD_FILE.to_string())
    }

    /// Returns the validated bundle-relative path string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkillFilePath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Provenance metadata for a skill bundle descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBundleProvenance {
    pub source_kind: SkillSourceKind,
    pub content_hash: Option<String>,
}

impl SkillBundleProvenance {
    /// Creates provenance for a bundle from the supplied source scope.
    pub fn new(source_kind: SkillSourceKind) -> Self {
        Self {
            source_kind,
            content_hash: None,
        }
    }

    /// Attaches an optional content hash supplied by the host source.
    #[must_use]
    pub fn with_content_hash(mut self, content_hash: impl Into<String>) -> Self {
        self.content_hash = Some(content_hash.into());
        self
    }
}

/// Public metadata describing one portable skill bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBundleDescriptor {
    id: SkillBundleId,
    skill_md_path: SkillFilePath,
    trust: Option<SkillTrust>,
    visibility: Option<SkillVisibility>,
    provenance: SkillBundleProvenance,
    description: String,
}

impl SkillBundleDescriptor {
    /// Creates a descriptor with the default `SKILL.md` descriptor path.
    pub fn new(
        id: SkillBundleId,
        trust: Option<SkillTrust>,
        visibility: Option<SkillVisibility>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            provenance: SkillBundleProvenance::new(id.source_kind()),
            id,
            skill_md_path: SkillFilePath::skill_md(),
            trust,
            visibility,
            description: description.into(),
        }
    }

    /// Overrides the descriptor file path for bundles whose manifest is nested.
    #[must_use]
    pub fn with_skill_md_path(mut self, path: SkillFilePath) -> Self {
        self.skill_md_path = path;
        self
    }

    /// Overrides provenance metadata supplied by the host source.
    ///
    /// The descriptor id remains the source-kind authority; this method preserves
    /// other provenance fields while aligning provenance source kind with the id.
    #[must_use]
    pub fn with_provenance(mut self, mut provenance: SkillBundleProvenance) -> Self {
        provenance.source_kind = self.id.source_kind();
        self.provenance = provenance;
        self
    }

    /// Attaches the safe manifest description that may be shown before prompt loading.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Returns the bundle id.
    pub fn id(&self) -> &SkillBundleId {
        &self.id
    }

    /// Returns the validated bundle-relative `SKILL.md` path.
    pub fn skill_md_path(&self) -> &SkillFilePath {
        &self.skill_md_path
    }

    /// Returns the trust metadata declared by the host source, if any.
    pub fn trust(&self) -> Option<&SkillTrust> {
        self.trust.as_ref()
    }

    /// Returns visibility metadata declared by the host source, if any.
    pub fn visibility(&self) -> Option<&SkillVisibility> {
        self.visibility.as_ref()
    }

    /// Returns provenance metadata for this descriptor.
    pub fn provenance(&self) -> &SkillBundleProvenance {
        &self.provenance
    }

    /// Returns the safe manifest description supplied by the host source.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the deterministic ordering key used by [`Ord`].
    pub fn ordering_key(&self) -> (SkillSourceKind, &str, &str) {
        (
            self.id.source_kind(),
            self.id.name(),
            self.skill_md_path.as_str(),
        )
    }
}

impl Ord for SkillBundleDescriptor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ordering_key().cmp(&other.ordering_key())
    }
}

impl PartialOrd for SkillBundleDescriptor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Sorts descriptors by source scope, bundle name, then bundle-relative manifest path.
pub fn sort_skill_bundle_descriptors(descriptors: &mut [SkillBundleDescriptor]) {
    descriptors.sort();
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillBundleSourceError {
    #[error("skill bundle source unavailable")]
    SourceUnavailable,
    #[error("skill bundle id is invalid")]
    InvalidBundleId,
    #[error("skill bundle file path is invalid")]
    InvalidFilePath,
    #[error("skill bundle content is invalid")]
    InvalidSkillBundle,
    #[error("skill bundle manifest is not valid UTF-8")]
    BundleUtf8DecodeFailed,
    #[error("skill bundle manifest failed to parse")]
    ManifestParseFailed,
    #[error("skill bundle source has duplicate source-kind roots")]
    DuplicateSourceKind,
    #[error("skill bundle not found")]
    BundleNotFound,
    #[error("skill bundle file not found")]
    FileNotFound,
    #[error("skill bundle access denied")]
    PermissionDenied,
    #[error("skill bundle content too large")]
    ContentTooLarge,
    #[error("skill bundle source scan limit exceeded")]
    BundleScanLimitExceeded,
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
            format!("{name} description"),
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

        for invalid in [
            "",
            " code-review",
            "code-review ",
            "has space",
            "../escape",
            "/absolute",
            "ümlaut",
        ] {
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
            " references/policy.md",
            "references/policy.md ",
            "/skills/code-review/SKILL.md",
            "../SKILL.md",
            "references/../SKILL.md",
            "references/./SKILL.md",
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
    fn skill_bundle_descriptor_can_override_provenance_with_content_hash() {
        let descriptor = SkillBundleDescriptor::new(
            id(SkillSourceKind::User, "code-review"),
            Some(SkillTrust::Trusted),
            Some(SkillVisibility::Visible),
            "code-review description",
        )
        .with_skill_md_path(SkillFilePath::new("nested/SKILL.md").unwrap())
        .with_provenance(
            SkillBundleProvenance::new(SkillSourceKind::TenantShared)
                .with_content_hash("sha256:abc123"),
        );

        assert_eq!(descriptor.id().source_kind(), SkillSourceKind::User);
        assert_eq!(descriptor.id().name(), "code-review");
        assert_eq!(descriptor.skill_md_path().as_str(), "nested/SKILL.md");
        assert_eq!(descriptor.trust(), Some(&SkillTrust::Trusted));
        assert_eq!(descriptor.visibility(), Some(&SkillVisibility::Visible));
        assert_eq!(descriptor.provenance().source_kind, SkillSourceKind::User);
        assert_eq!(
            descriptor.provenance().content_hash.as_deref(),
            Some("sha256:abc123")
        );
    }

    #[test]
    fn skill_bundle_source_errors_render_stable_messages() {
        assert_eq!(
            SkillBundleSourceError::SourceUnavailable.to_string(),
            "skill bundle source unavailable"
        );
        assert_eq!(
            SkillBundleSourceError::InvalidBundleId.to_string(),
            "skill bundle id is invalid"
        );
        assert_eq!(
            SkillBundleSourceError::InvalidFilePath.to_string(),
            "skill bundle file path is invalid"
        );
        assert_eq!(
            SkillBundleSourceError::InvalidSkillBundle.to_string(),
            "skill bundle content is invalid"
        );
        assert_eq!(
            SkillBundleSourceError::BundleUtf8DecodeFailed.to_string(),
            "skill bundle manifest is not valid UTF-8"
        );
        assert_eq!(
            SkillBundleSourceError::ManifestParseFailed.to_string(),
            "skill bundle manifest failed to parse"
        );
        assert_eq!(
            SkillBundleSourceError::DuplicateSourceKind.to_string(),
            "skill bundle source has duplicate source-kind roots"
        );
        assert_eq!(
            SkillBundleSourceError::BundleNotFound.to_string(),
            "skill bundle not found"
        );
        assert_eq!(
            SkillBundleSourceError::FileNotFound.to_string(),
            "skill bundle file not found"
        );
        assert_eq!(
            SkillBundleSourceError::PermissionDenied.to_string(),
            "skill bundle access denied"
        );
        assert_eq!(
            SkillBundleSourceError::ContentTooLarge.to_string(),
            "skill bundle content too large"
        );
        assert_eq!(
            SkillBundleSourceError::BundleScanLimitExceeded.to_string(),
            "skill bundle source scan limit exceeded"
        );
        assert_eq!(
            SkillBundleSourceError::Internal.to_string(),
            "skill bundle source internal error"
        );
    }

    #[test]
    fn skill_source_kind_sort_matches_stable_string_ordering() {
        let mut source_kinds = vec![
            SkillSourceKind::User,
            SkillSourceKind::TenantShared,
            SkillSourceKind::System,
        ];

        source_kinds.sort();

        assert_eq!(
            source_kinds,
            vec![
                SkillSourceKind::System,
                SkillSourceKind::TenantShared,
                SkillSourceKind::User,
            ]
        );
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
                    descriptor.id().source_kind(),
                    descriptor.id().name(),
                    descriptor.skill_md_path()
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
