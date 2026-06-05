use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{StreamExt, stream};
use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ScopedPath;
use ironclaw_turns::run_profile::LoopRunContext;

use crate::{SkillActivationSelectionError, activation::SetupMarkerSource};

const MAX_CONCURRENT_SETUP_MARKER_STATS: usize = 16;

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
        let filesystem = Arc::clone(&self.filesystem);
        let satisfied = stream::iter(markers.iter().cloned())
            .map(|marker| {
                let filesystem = Arc::clone(&filesystem);
                let scope = scope.clone();
                async move {
                    let path = workspace_setup_marker_path(&marker)?;
                    match filesystem.stat(&scope, &path).await {
                        Ok(_) => Some(marker),
                        Err(FilesystemError::NotFound { .. }) => None,
                        Err(error) => {
                            tracing::debug!(
                                %marker,
                                %error,
                                "treating unavailable skill setup marker as unsatisfied"
                            );
                            None
                        }
                    }
                }
            })
            .buffer_unordered(MAX_CONCURRENT_SETUP_MARKER_STATS)
            .filter_map(|marker| async move { marker })
            .collect::<HashSet<_>>()
            .await;
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
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{AgentId, MountView, ProjectId, TenantId};
    use ironclaw_turns::{
        AcceptedMessageRef, TurnActor, TurnId, TurnRunId, TurnScope,
        run_profile::{
            InMemoryRunProfileResolver, RunProfileResolutionRequest, RunProfileResolver,
        },
    };

    async fn run_context() -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("run profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new("tenant-a").expect("valid tenant"),
                Some(AgentId::new("agent-a").expect("valid agent")),
                Some(ProjectId::new("project-a").expect("valid project")),
                ironclaw_host_api::ThreadId::new("thread-a").expect("valid thread"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
        .with_accepted_message_ref(
            AcceptedMessageRef::new("msg:setup-marker").expect("valid message ref"),
        )
        .with_actor(TurnActor::new(
            ironclaw_host_api::UserId::new("user-a").expect("valid user"),
        ))
    }

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

    #[tokio::test]
    async fn filesystem_setup_marker_source_treats_stat_errors_as_unsatisfied() {
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::default()),
            MountView::new(Vec::new()).expect("empty mount view"),
        ));
        let source = FilesystemSetupMarkerSource::new(filesystem);
        let markers = HashSet::from(["markers/setup.done".to_string()]);

        let satisfied = source
            .satisfied_setup_markers(&run_context().await, &markers)
            .await
            .expect("marker probe errors should not fail activation");

        assert!(satisfied.is_empty());
    }
}
