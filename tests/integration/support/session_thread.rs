use std::sync::Arc;

use ironclaw_filesystem::{
    CompositeRootFilesystem, InMemoryBackend, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ThreadId, VirtualPath,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendFinalizedAssistantMessageRequest, AppendToolResultReferenceRequest, ContextMessages,
    ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest,
    FilesystemSessionThreadService, LoadContextMessagesRequest, LoadContextWindowRequest,
    MessageContent, RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadError,
    SessionThreadRecord, SessionThreadService, SummaryArtifact, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    UpdateAssistantDraftRequest, UpdateToolResultReferenceRequest,
};
use thiserror::Error;

pub const TRANSCRIPT_FAILURE_SECRET: &str = "sk-TRANSCRIPT0123456789SECRET";

#[derive(Debug, Error)]
pub enum RebornThreadHarnessError {
    #[error("invalid mount view: {0}")]
    MountView(#[from] ironclaw_host_api::HostApiError),
    #[error("thread service failed: {0}")]
    Thread(#[from] ironclaw_threads::SessionThreadError),
    #[error("thread history does not contain final assistant reply containing {0:?}")]
    MissingFinalReply(String),
}

/// Thin harness over `FilesystemSessionThreadService<F>` for asserting thread history.
///
/// Defaults to `InMemoryBackend` (CAS-capable, models the production DB-backed filesystem)
/// rather than the byte-only `DiskFilesystem` (see e3e155803). The integration tier uses
/// `RebornThreadHarness<CompositeRootFilesystem>` via `filesystem_shared_composite`, mounted
/// on the per-`build()` production-path composite.
pub struct RebornThreadHarness<F = InMemoryBackend>
where
    F: RootFilesystem,
{
    pub scope: ThreadScope,
    pub service: Arc<FilesystemSessionThreadService<F>>,
    backend: Arc<F>,
    /// Backing `TempDir` for disk-persisting tiers (e.g. `CompositeRootFilesystem`);
    /// `None` for in-memory tiers.
    root: Option<Arc<tempfile::TempDir>>,
    /// Path prefix before `/tenants/...`. Default `InMemoryBackend` tier uses `"/engine"`;
    /// integration-tier `CompositeRootFilesystem` uses `""` (production composite layout).
    root_prefix: String,
}

/// Test-only runtime transcript seam that rejects final assistant persistence
/// before creating a draft. All reads and inbound writes still use the real
/// filesystem-backed service, so whole-turn tests exercise the production
/// scheduler, loop host, driver, failure persistence, and projection path.
pub struct FailingFinalizedAssistantThreadService {
    inner: Arc<dyn SessionThreadService>,
}

impl FailingFinalizedAssistantThreadService {
    pub fn new(inner: Arc<dyn SessionThreadService>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl SessionThreadService for FailingFinalizedAssistantThreadService {
    async fn ensure_thread(
        &self,
        request: EnsureThreadRequest,
    ) -> Result<SessionThreadRecord, SessionThreadError> {
        self.inner.ensure_thread(request).await
    }

    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, SessionThreadError> {
        self.inner.accept_inbound_message(request).await
    }

    async fn replay_accepted_inbound_message(
        &self,
        request: ReplayAcceptedInboundMessageRequest,
    ) -> Result<Option<AcceptedInboundMessageReplay>, SessionThreadError> {
        self.inner.replay_accepted_inbound_message(request).await
    }

    async fn mark_message_submitted(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        turn_id: String,
        turn_run_id: String,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_submitted(scope, thread_id, message_id, turn_id, turn_run_id)
            .await
    }

    async fn mark_message_rejected_busy(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .mark_message_rejected_busy(scope, thread_id, message_id)
            .await
    }

    async fn append_assistant_draft(
        &self,
        request: AppendAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_assistant_draft(request).await
    }

    async fn append_finalized_assistant_message(
        &self,
        request: AppendFinalizedAssistantMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        Err(SessionThreadError::Backend(format!(
            "write rejected for raw transcript {:?} using token {TRANSCRIPT_FAILURE_SECRET}",
            request.content.as_text()
        )))
    }

    async fn append_tool_result_reference(
        &self,
        request: AppendToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_tool_result_reference(request).await
    }

    async fn append_capability_display_preview(
        &self,
        request: AppendCapabilityDisplayPreviewRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.append_capability_display_preview(request).await
    }

    async fn update_tool_result_reference(
        &self,
        request: UpdateToolResultReferenceRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_tool_result_reference(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraftRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        content: MessageContent,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner
            .finalize_assistant_message(scope, thread_id, message_id, content)
            .await
    }

    async fn redact_message(
        &self,
        request: RedactMessageRequest,
    ) -> Result<ThreadMessageRecord, SessionThreadError> {
        self.inner.redact_message(request).await
    }

    async fn load_context_window(
        &self,
        request: LoadContextWindowRequest,
    ) -> Result<ContextWindow, SessionThreadError> {
        self.inner.load_context_window(request).await
    }

    async fn load_context_messages(
        &self,
        request: LoadContextMessagesRequest,
    ) -> Result<ContextMessages, SessionThreadError> {
        self.inner.load_context_messages(request).await
    }

    async fn list_thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, SessionThreadError> {
        self.inner.list_thread_history(request).await
    }

    async fn create_summary_artifact(
        &self,
        request: CreateSummaryArtifactRequest,
    ) -> Result<SummaryArtifact, SessionThreadError> {
        self.inner.create_summary_artifact(request).await
    }
}

impl<F: RootFilesystem> RebornThreadHarness<F> {
    pub fn reopened(&self) -> Result<Self, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at(&self.root_prefix, Arc::clone(&self.backend))?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope: self.scope.clone(),
            service,
            backend: Arc::clone(&self.backend),
            root: self.root.clone(),
            root_prefix: self.root_prefix.clone(),
        })
    }

    pub fn service_instance(
        &self,
    ) -> Result<FilesystemSessionThreadService<F>, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at(&self.root_prefix, Arc::clone(&self.backend))?;
        Ok(FilesystemSessionThreadService::new(scoped))
    }

    pub async fn history(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadMessageRecord>, RebornThreadHarnessError> {
        Ok(self
            .service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.scope.clone(),
                thread_id,
            })
            .await?
            .messages)
    }

    pub async fn assert_final_reply(
        &self,
        thread_id: ThreadId,
        text: &str,
    ) -> Result<(), RebornThreadHarnessError> {
        let history = self.history(thread_id).await?;
        let found = history
            .iter()
            .rev()
            .find(|message| {
                message.kind == ironclaw_threads::MessageKind::Assistant
                    && message.status == ironclaw_threads::MessageStatus::Finalized
            })
            .is_some_and(|message| {
                message
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains(text))
            });
        if found {
            Ok(())
        } else {
            Err(RebornThreadHarnessError::MissingFinalReply(
                text.to_string(),
            ))
        }
    }
}

/// `InMemoryBackend`-specific constructors (default tier).
impl RebornThreadHarness<InMemoryBackend> {
    pub fn filesystem_temp(scope: ThreadScope) -> Result<Self, RebornThreadHarnessError> {
        let backend = Arc::new(InMemoryBackend::new());
        Self::filesystem_shared_backend(scope, backend)
    }

    pub fn filesystem_shared_backend(
        scope: ThreadScope,
        backend: Arc<InMemoryBackend>,
    ) -> Result<Self, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at("/engine", Arc::clone(&backend))?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope,
            service,
            backend,
            root: None,
            root_prefix: "/engine".to_string(),
        })
    }
}

/// `CompositeRootFilesystem`-specific constructor (integration tier).
impl RebornThreadHarness<CompositeRootFilesystem> {
    /// Harness backed by a shared production-path composite; threads land at
    /// `/tenants/{tenant}/users/{user}/threads` (visible via `mount_local_dev_database_roots`).
    /// `root` (also held by `GroupSharedStorage::turn_root`) keeps the `TempDir` alive so
    /// on-disk libsql data persists across `reopened()` calls.
    pub fn filesystem_shared_composite(
        scope: ThreadScope,
        backend: Arc<CompositeRootFilesystem>,
        root: Arc<tempfile::TempDir>,
    ) -> Result<Self, RebornThreadHarnessError> {
        let scoped = scoped_threads_fs_at("", Arc::clone(&backend))?;
        let service = Arc::new(FilesystemSessionThreadService::new(scoped));
        Ok(Self {
            scope,
            service,
            backend,
            root: Some(root),
            root_prefix: String::new(),
        })
    }
}

/// Build the scoped thread filesystem.
///
/// The `/threads` mount resolves **per filesystem operation** from that op's `ResourceScope`
/// (via `ThreadScope::to_resource_scope`), not fixed at construction — so one service instance
/// serves every owner's subtree, letting a group's shared runtime resolve a second actor's
/// thread (issue #5479). Single-owner tests are unaffected (path is byte-identical).
///
/// `root_prefix` precedes `/tenants/...`: `"/engine"` for the default `InMemoryBackend` tier,
/// `""` for the integration tier.
fn scoped_threads_fs_at<F>(
    root_prefix: &str,
    backend: Arc<F>,
) -> Result<Arc<ScopedFilesystem<F>>, ironclaw_host_api::HostApiError>
where
    F: RootFilesystem,
{
    let root_prefix = root_prefix.to_owned();
    Ok(Arc::new(ScopedFilesystem::new(backend, move |scope| {
        threads_mount_view(&root_prefix, scope)
    })))
}

/// The single `/threads` mount grant for one operation's `ResourceScope`.
///
/// System-scoped ops carry the `SYSTEM_RESERVED_ID` sentinel (control bytes, not path-safe)
/// in the tenant/user segment; mapped to the harness's `_system` segment — matching
/// production's `resource_scope_path_segment` only in SHAPE, not value (prod uses
/// `__system__`). Deliberately not switched, to avoid rewriting every existing fixture
/// path; see `path_segment`.
pub(crate) fn threads_mount_view(
    root_prefix: &str,
    scope: &ironclaw_host_api::ResourceScope,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    let tenant_id = path_segment(scope.tenant_id.as_str());
    let user_id = path_segment(scope.user_id.as_str());
    let target = format!("{root_prefix}/tenants/{tenant_id}/users/{user_id}/threads");
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/threads")?,
        VirtualPath::new(target)?,
        MountPermissions::read_write_list_delete(),
    )])
}

/// Path-safe segment for one scope axis: `SYSTEM_RESERVED_ID` becomes `_system` (not
/// production's `__system__` — see `threads_mount_view`); everything else passes through
/// verbatim.
pub(crate) fn path_segment(value: &str) -> &str {
    if value == ironclaw_host_api::SYSTEM_RESERVED_ID {
        "_system"
    } else {
        value
    }
}
