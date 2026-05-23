use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ScopedPath;
use ironclaw_turns::run_profile::LoopRunContext;

use crate::{SkillActivationSelectionError, activation::SetupMarkerSource};

pub(crate) struct FilesystemSetupMarkerSource<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> std::fmt::Debug for FilesystemSetupMarkerSource<F>
where
    F: RootFilesystem + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemSetupMarkerSource")
            .field("filesystem", &self.filesystem)
            .finish()
    }
}

impl<F> FilesystemSetupMarkerSource<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }
}

#[async_trait]
impl<F> SetupMarkerSource for FilesystemSetupMarkerSource<F>
where
    F: RootFilesystem + 'static,
{
    async fn satisfied_setup_markers(
        &self,
        run_context: &LoopRunContext,
        markers: &HashSet<String>,
    ) -> Result<HashSet<String>, SkillActivationSelectionError> {
        let scope = run_context.scope.to_resource_scope();
        let mut satisfied = HashSet::new();
        for marker in markers {
            let Some(path) = workspace_setup_marker_path(marker) else {
                continue;
            };
            match self.filesystem.stat(&scope, &path).await {
                Ok(_) => {
                    satisfied.insert(marker.clone());
                }
                Err(FilesystemError::NotFound { .. }) => {}
                Err(_) => return Err(SkillActivationSelectionError::SourceUnavailable),
            }
        }
        Ok(satisfied)
    }
}

fn workspace_setup_marker_path(marker: &str) -> Option<ScopedPath> {
    let marker = marker.trim_start_matches('/');
    if marker.is_empty() {
        tracing::debug!("ignoring empty skill setup marker");
        return None;
    }
    match ScopedPath::new(format!("/workspace/{marker}")) {
        Ok(path) => Some(path),
        Err(reason) => {
            tracing::debug!(%marker, %reason, "ignoring invalid skill setup marker");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_setup_marker_path_ignores_invalid_markers() {
        assert!(workspace_setup_marker_path("").is_none());
        assert!(workspace_setup_marker_path("../escape").is_none());
        assert_eq!(
            workspace_setup_marker_path("/markers/setup.done")
                .expect("valid setup marker path")
                .as_str(),
            "/workspace/markers/setup.done"
        );
    }
}
