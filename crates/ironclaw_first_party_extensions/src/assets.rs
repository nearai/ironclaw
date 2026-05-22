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
