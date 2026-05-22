use std::collections::HashSet;
use std::sync::Arc;

use ironclaw_loop_support::{
    SkillBundleId, SkillBundleSource, SkillBundleSourceError, SkillFilePath,
};
use ironclaw_turns::run_profile::LoopRunContext;
use thiserror::Error;

use super::SkillActivationRequest;

/// Bundle file returned through the first-party skill asset reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBundleAsset {
    pub bundle_id: SkillBundleId,
    pub path: SkillFilePath,
    pub bytes: Vec<u8>,
}

impl SkillBundleAsset {
    pub fn into_utf8(self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.bytes)
    }
}

/// Scoped reader for files under already activated skill bundles.
///
/// This reader does not expose host paths. It accepts only validated
/// bundle-relative paths and delegates final storage policy to
/// [`SkillBundleSource`].
#[derive(Debug)]
pub struct SkillBundleAssetReader<S>
where
    S: SkillBundleSource + ?Sized,
{
    bundle_source: Arc<S>,
    active_bundles: HashSet<SkillBundleId>,
}

impl<S> Clone for SkillBundleAssetReader<S>
where
    S: SkillBundleSource + ?Sized,
{
    fn clone(&self) -> Self {
        Self {
            bundle_source: Arc::clone(&self.bundle_source),
            active_bundles: self.active_bundles.clone(),
        }
    }
}

impl<S> SkillBundleAssetReader<S>
where
    S: SkillBundleSource + ?Sized,
{
    pub fn new(
        bundle_source: Arc<S>,
        active_bundles: impl IntoIterator<Item = SkillBundleId>,
    ) -> Self {
        Self {
            bundle_source,
            active_bundles: active_bundles.into_iter().collect(),
        }
    }

    pub fn active_bundles(&self) -> impl Iterator<Item = &SkillBundleId> {
        self.active_bundles.iter()
    }

    pub async fn read_file(
        &self,
        run_context: &LoopRunContext,
        bundle_id: &SkillBundleId,
        path: impl AsRef<str>,
    ) -> Result<SkillBundleAsset, SkillBundleAssetReadError> {
        if !self.active_bundles.contains(bundle_id) {
            return Err(SkillBundleAssetReadError::InactiveSkill {
                bundle_id: bundle_id.clone(),
            });
        }
        let path = SkillFilePath::new(path).map_err(SkillBundleAssetReadError::from_source)?;
        let bytes = self
            .bundle_source
            .read_skill_bundle_file(run_context, bundle_id, &path)
            .await
            .map_err(SkillBundleAssetReadError::from_source)?;
        Ok(SkillBundleAsset {
            bundle_id: bundle_id.clone(),
            path,
            bytes,
        })
    }

    pub async fn read_file_for_activation(
        &self,
        run_context: &LoopRunContext,
        activation: &SkillActivationRequest,
        path: impl AsRef<str>,
    ) -> Result<SkillBundleAsset, SkillBundleAssetReadError> {
        let bundle_id = match &activation.bundle_id {
            Some(bundle_id) => bundle_id.clone(),
            None => {
                let source = activation.source.ok_or_else(|| {
                    SkillBundleAssetReadError::UnresolvedActivation {
                        name: activation.name.clone(),
                    }
                })?;
                SkillBundleId::new(source, &activation.name)
                    .map_err(SkillBundleAssetReadError::from_source)?
            }
        };
        self.read_file(run_context, &bundle_id, path).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillBundleAssetReadError {
    #[error("skill activation '{name}' is unresolved")]
    UnresolvedActivation { name: String },
    #[error("skill bundle '{bundle_id}' is not active for this plan")]
    InactiveSkill { bundle_id: SkillBundleId },
    #[error("skill bundle asset source unavailable")]
    SourceUnavailable,
    #[error("skill bundle asset path is invalid")]
    InvalidPath,
    #[error("skill bundle asset content is invalid")]
    InvalidBundle,
    #[error("skill bundle asset not found")]
    NotFound,
    #[error("skill bundle asset access denied")]
    PermissionDenied,
    #[error("skill bundle asset content too large")]
    ContentTooLarge,
    #[error("skill bundle asset source internal error")]
    Internal,
}

impl SkillBundleAssetReadError {
    fn from_source(error: SkillBundleSourceError) -> Self {
        match error {
            SkillBundleSourceError::SourceUnavailable => Self::SourceUnavailable,
            SkillBundleSourceError::InvalidBundleId | SkillBundleSourceError::InvalidFilePath => {
                Self::InvalidPath
            }
            SkillBundleSourceError::InvalidSkillBundle
            | SkillBundleSourceError::BundleUtf8DecodeFailed
            | SkillBundleSourceError::ManifestParseFailed => Self::InvalidBundle,
            SkillBundleSourceError::BundleNotFound | SkillBundleSourceError::FileNotFound => {
                Self::NotFound
            }
            SkillBundleSourceError::PermissionDenied => Self::PermissionDenied,
            SkillBundleSourceError::ContentTooLarge
            | SkillBundleSourceError::BundleScanLimitExceeded => Self::ContentTooLarge,
            SkillBundleSourceError::DuplicateSourceKind | SkillBundleSourceError::Internal => {
                Self::Internal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_loop_support::{SkillFilePath, SkillSourceKind};
    use ironclaw_turns::{
        TurnId, TurnRunId, TurnScope,
        run_profile::{
            InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest,
            RunProfileResolver,
        },
    };

    use super::*;

    struct StaticSkillBundleSource {
        files: HashMap<(SkillSourceKind, String, String), Vec<u8>>,
        errors: HashMap<(SkillSourceKind, String, String), SkillBundleSourceError>,
    }

    impl StaticSkillBundleSource {
        fn new() -> Self {
            Self {
                files: HashMap::new(),
                errors: HashMap::new(),
            }
        }

        fn with_file(mut self, bundle_id: &SkillBundleId, path: &str, bytes: &[u8]) -> Self {
            self.files.insert(
                (
                    bundle_id.source_kind(),
                    bundle_id.name().to_string(),
                    path.to_string(),
                ),
                bytes.to_vec(),
            );
            self
        }

        fn with_error(
            mut self,
            bundle_id: &SkillBundleId,
            path: &str,
            error: SkillBundleSourceError,
        ) -> Self {
            self.errors.insert(
                (
                    bundle_id.source_kind(),
                    bundle_id.name().to_string(),
                    path.to_string(),
                ),
                error,
            );
            self
        }
    }

    #[async_trait]
    impl SkillBundleSource for StaticSkillBundleSource {
        async fn list_skill_bundles(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<ironclaw_loop_support::SkillBundleDescriptor>, SkillBundleSourceError>
        {
            Ok(Vec::new())
        }

        async fn read_skill_bundle_file(
            &self,
            _run_context: &LoopRunContext,
            bundle_id: &SkillBundleId,
            path: &SkillFilePath,
        ) -> Result<Vec<u8>, SkillBundleSourceError> {
            let key = (
                bundle_id.source_kind(),
                bundle_id.name().to_string(),
                path.as_str().to_string(),
            );
            if let Some(error) = self.errors.get(&key) {
                return Err(error.clone());
            }
            self.files
                .get(&key)
                .cloned()
                .ok_or(SkillBundleSourceError::FileNotFound)
        }
    }

    async fn run_context() -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("resolve run profile");
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new("tenant-a").unwrap(),
                Some(AgentId::new("agent-a").unwrap()),
                Some(ProjectId::new("project-a").unwrap()),
                ThreadId::new("thread-a").unwrap(),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }

    #[tokio::test]
    async fn read_file_rejects_inactive_bundle_before_source_read() {
        let active = SkillBundleId::new(SkillSourceKind::User, "active-helper").unwrap();
        let inactive = SkillBundleId::new(SkillSourceKind::User, "inactive-helper").unwrap();
        let source = Arc::new(StaticSkillBundleSource::new().with_file(
            &inactive,
            "references/policy.md",
            b"should not be exposed",
        ));
        let reader = SkillBundleAssetReader::new(source, vec![active.clone()]);
        let active_bundles = reader.active_bundles().cloned().collect::<Vec<_>>();

        assert_eq!(active_bundles, vec![active]);
        let error = reader
            .read_file(&run_context().await, &inactive, "references/policy.md")
            .await
            .expect_err("inactive bundle should be rejected");

        assert_eq!(
            error,
            SkillBundleAssetReadError::InactiveSkill {
                bundle_id: inactive
            }
        );
    }

    #[tokio::test]
    async fn read_file_maps_active_bundle_source_errors() {
        let active = SkillBundleId::new(SkillSourceKind::User, "active-helper").unwrap();
        let source = Arc::new(
            StaticSkillBundleSource::new()
                .with_error(
                    &active,
                    "references/denied.md",
                    SkillBundleSourceError::PermissionDenied,
                )
                .with_error(
                    &active,
                    "references/large.md",
                    SkillBundleSourceError::ContentTooLarge,
                ),
        );
        let reader = SkillBundleAssetReader::new(source, vec![active.clone()]);
        let context = run_context().await;

        for (path, expected) in [
            ("references/missing.md", SkillBundleAssetReadError::NotFound),
            (
                "references/denied.md",
                SkillBundleAssetReadError::PermissionDenied,
            ),
            (
                "references/large.md",
                SkillBundleAssetReadError::ContentTooLarge,
            ),
        ] {
            let error = reader
                .read_file(&context, &active, path)
                .await
                .expect_err("active bundle source error should be mapped");

            assert_eq!(error, expected);
        }
    }
}
