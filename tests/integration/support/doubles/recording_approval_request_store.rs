/// Test double substituting the production `ApprovalRequestStore` impls
/// (`InMemoryApprovalRequestStore` / `FilesystemApprovalRequestStore`,
/// `crates/ironclaw_run_state/src/lib.rs`).
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{ApprovalRequestId, ResourceScope};

/// Records `(ApprovalRequestId, ResourceScope)` on `save_pending`, then delegates
/// every method to the inner store. Synthetic local-dev capabilities (e.g.
/// `outbound_delivery_target_set`) persist their approval requests directly to
/// the approval store rather than through the host runtime, so
/// [`RecordingHostRuntime`] (which only observes host-runtime-level gates) never
/// captures their scope. Wrapping the store the synthetic capability writes
/// through restores the same `pending_approval_scopes` bookkeeping the
/// `approve_local_dev_gate` / `deny_local_dev_gate` lookups depend on. Delegation
/// is total, so the inner store the evidence/approve/deny paths read stays the
/// single source of truth.
pub(crate) struct RecordingApprovalRequestStore {
    pub(crate) inner: Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
    pub(crate) pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
}

#[async_trait]
impl ironclaw_run_state::ApprovalRequestStore for RecordingApprovalRequestStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ironclaw_host_api::approval::ApprovalRequest,
    ) -> Result<ironclaw_run_state::ApprovalRecord, ironclaw_run_state::RunStateError> {
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
    ) -> Result<Option<ironclaw_run_state::ApprovalRecord>, ironclaw_run_state::RunStateError> {
        self.inner.get(scope, request_id).await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_run_state::ApprovalRecord, ironclaw_run_state::RunStateError> {
        self.inner.approve(scope, request_id).await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_run_state::ApprovalRecord, ironclaw_run_state::RunStateError> {
        self.inner.deny(scope, request_id).await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_run_state::ApprovalRecord, ironclaw_run_state::RunStateError> {
        self.inner.discard_pending(scope, request_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ironclaw_run_state::ApprovalRecord>, ironclaw_run_state::RunStateError> {
        self.inner.records_for_scope(scope).await
    }
}
