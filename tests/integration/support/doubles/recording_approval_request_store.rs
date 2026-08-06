/// Test double substituting the production `ApprovalRequestStorePort` impl
/// (`ApprovalRequestStore`, `crates/kernel/ironclaw_approvals/src/lib.rs`).
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{ids::ApprovalRequestId, resource::ResourceScope};

/// Records `(ApprovalRequestId, ResourceScope)` on `save_pending`, then delegates
/// every method to the inner store. Synthetic local-dev capabilities (e.g.
/// `outbound_delivery_target_set`) persist approval requests directly to the
/// store rather than through the host runtime, so [`RecordingHostRuntime`]
/// never captures their scope — wrapping the store they write through
/// restores the `pending_approval_scopes` bookkeeping `approve_standalone_gate`
/// / `deny_standalone_gate` depend on. Delegation is total, so the inner store
/// stays the single source of truth.
pub(crate) struct RecordingApprovalRequestStore {
    pub(crate) inner: Arc<dyn ironclaw_approvals::ApprovalRequestStorePort>,
    pub(crate) pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
}

#[async_trait]
impl ironclaw_approvals::ApprovalRequestStorePort for RecordingApprovalRequestStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ironclaw_host_api::approval::ApprovalRequest,
    ) -> Result<ironclaw_approvals::ApprovalRecord, ironclaw_approvals::ApprovalStoreError> {
        self.pending_approval_scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(request.id, scope.clone());
        self.inner.save_pending(scope, request).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ironclaw_approvals::ApprovalRecord>, ironclaw_approvals::ApprovalStoreError>
    {
        self.inner.get(scope, request_id).await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_approvals::ApprovalRecord, ironclaw_approvals::ApprovalStoreError> {
        self.inner.approve(scope, request_id).await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_approvals::ApprovalRecord, ironclaw_approvals::ApprovalStoreError> {
        self.inner.deny(scope, request_id).await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_approvals::ApprovalRecord, ironclaw_approvals::ApprovalStoreError> {
        self.inner.discard_pending(scope, request_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ironclaw_approvals::ApprovalRecord>, ironclaw_approvals::ApprovalStoreError>
    {
        self.inner.records_for_scope(scope).await
    }
}
