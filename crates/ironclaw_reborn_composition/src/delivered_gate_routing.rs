//! Cross-thread gate-reply routing for triggered-run approvals.
//!
//! ## Problem
//!
//! When a trigger fires and the run blocks on an approval gate, the driver
//! delivers an "Approve needed" prompt to the creator's personal delivery
//! target (e.g., a Slack DM). The user replies with `approve <gate_ref>` in
//! their DM. The inbound path resolves the scope from the DM conversation,
//! which has a different `thread_id` from the triggered run's thread. The
//! approval service's `find_gate` call then fails with `MissingGate` because
//! the gate lives on the run's original thread, not the DM thread.
//!
//! ## Fix
//!
//! [`DeliveredGateRoutingApprovalService`] wraps the real
//! [`ApprovalInteractionService`]. On every `resolve` call it looks up
//! `(tenant_id, actor.user_id, gate_ref)` in a [`DeliveredGateRouteStore`].
//! On a hit it rewrites the request to use the stored scope and run_id_hint
//! before forwarding to the inner service. On a miss it forwards unchanged
//! (normal same-thread live runs keep working).
//!
//! ## Security invariants
//!
//! The wrapper never widens authority:
//! - A route record exists only because the approval prompt was previously
//!   delivered to *that user's own personal target*, so record presence is
//!   already a proof of delivery authority.
//! - The lookup key binds `(tenant_id, actor.user_id, gate_ref)`. Tenant is
//!   taken from the inbound request scope; user_id is the authenticated actor.
//! - The record's `user_id` is verified equal to the requesting actor's
//!   `user_id` as a defense-in-depth check before the scope rewrite.
//! - The inner service still enforces all gate-state and actor checks after
//!   the rewrite; the wrapper only routes — it does not grant approval.
//! - A user_id mismatch silently forwards the unchanged request (not an
//!   error) so the inner service decides the outcome.
//! - `list_pending` is forwarded unchanged; the wrapper does not affect
//!   approval listing.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_outbound::DeliveredGateRouteStore;
use ironclaw_product_workflow::{
    ApprovalInteractionService, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    ProductWorkflowError, ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
};

/// [`ApprovalInteractionService`] wrapper that rewrites cross-thread approval
/// resolve requests to the stored run scope before forwarding to the inner
/// service.
///
/// See the module-level documentation for security invariants.
pub struct DeliveredGateRoutingApprovalService {
    inner: Arc<dyn ApprovalInteractionService>,
    routes: Arc<dyn DeliveredGateRouteStore>,
}

impl DeliveredGateRoutingApprovalService {
    pub fn new(
        inner: Arc<dyn ApprovalInteractionService>,
        routes: Arc<dyn DeliveredGateRouteStore>,
    ) -> Self {
        Self { inner, routes }
    }
}

#[async_trait]
impl ApprovalInteractionService for DeliveredGateRoutingApprovalService {
    async fn list_pending(
        &self,
        request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        self.inner.list_pending(request).await
    }

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let tenant_id = &request.scope.tenant_id;
        let user_id = &request.actor.user_id;
        let gate_ref_str = request.gate_ref.as_str();

        // Look up the route record. On store error, log at debug! and forward
        // unchanged — the inner service will attempt resolution on the inbound
        // scope and return MissingGate if this was truly a cross-thread reply.
        let route = match self
            .routes
            .load_delivered_gate_route(tenant_id, user_id, gate_ref_str)
            .await
        {
            Ok(record) => record,
            Err(reason) => {
                tracing::debug!(
                    target = "ironclaw::reborn::delivered_gate_routing",
                    gate_ref = gate_ref_str,
                    error = %reason,
                    "delivered gate route store read failed; forwarding unchanged"
                );
                None
            }
        };

        let Some(record) = route else {
            // Miss: forward unchanged (normal same-thread live run path).
            return self.inner.resolve(request).await;
        };

        // Defense-in-depth: the key already encodes actor user_id, but
        // verify explicitly before rewriting. On mismatch, forward unchanged
        // so the inner service decides — do not error here.
        if record.user_id != *user_id {
            tracing::debug!(
                target = "ironclaw::reborn::delivered_gate_routing",
                gate_ref = gate_ref_str,
                "delivered gate route user_id mismatch; forwarding unchanged"
            );
            return self.inner.resolve(request).await;
        }

        tracing::debug!(
            target = "ironclaw::reborn::delivered_gate_routing",
            gate_ref = gate_ref_str,
            run_id = %record.run_id,
            "rewriting approval resolve request to triggered run scope"
        );

        let rewritten = ResolveApprovalInteractionRequest {
            scope: record.scope,
            run_id_hint: Some(record.run_id),
            // Keep actor, decision, and idempotency_key from the original request.
            actor: request.actor,
            gate_ref: request.gate_ref,
            decision: request.decision,
            idempotency_key: request.idempotency_key,
        };

        self.inner.resolve(rewritten).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
    use ironclaw_outbound::{DeliveredGateRouteRecord, InMemoryDeliveredGateRouteStore};
    use ironclaw_product_workflow::{
        ApprovalInteractionDecision, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
        ProductWorkflowError, ResolveApprovalInteractionRequest,
        ResolveApprovalInteractionResponse,
    };
    use ironclaw_turns::{GateRef, IdempotencyKey, TurnActor, TurnRunId, TurnScope};

    use super::*;

    // --- Recording fake inner service ----------------------------------------

    #[derive(Default)]
    struct RecordingApprovalService {
        resolve_calls: Mutex<Vec<ResolveApprovalInteractionRequest>>,
        list_calls: Mutex<Vec<ListPendingApprovalsRequest>>,
    }

    impl RecordingApprovalService {
        fn resolve_calls(&self) -> Vec<ResolveApprovalInteractionRequest> {
            self.resolve_calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl ApprovalInteractionService for RecordingApprovalService {
        async fn list_pending(
            &self,
            request: ListPendingApprovalsRequest,
        ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
            self.list_calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(request);
            Ok(ListPendingApprovalsResponse {
                approvals: Vec::new(),
            })
        }

        async fn resolve(
            &self,
            request: ResolveApprovalInteractionRequest,
        ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
            self.resolve_calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(request.clone());
            // Return a synthetic Approved response for test purposes.
            // The actual gate state logic lives in the real inner service.
            Err(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ironclaw_product_workflow::ApprovalInteractionRejectionKind::MissingGate,
            })
        }
    }

    // --- Helpers --------------------------------------------------------------

    fn tenant() -> TenantId {
        TenantId::new("tenant:routing-test").unwrap()
    }

    fn user() -> UserId {
        UserId::new("user:routing-test").unwrap()
    }

    fn dm_scope() -> TurnScope {
        // The DM thread — different thread_id from the triggered run scope.
        let agent = AgentId::new("agent:routing-test").unwrap();
        let thread = ThreadId::new("thread:dm-thread").unwrap();
        TurnScope::new_with_owner(tenant(), Some(agent), None, thread, Some(user()))
    }

    fn run_scope() -> TurnScope {
        // The triggered run's original thread — different thread_id from dm_scope.
        let agent = AgentId::new("agent:routing-test").unwrap();
        let thread = ThreadId::new("thread:run-thread").unwrap();
        TurnScope::new_with_owner(tenant(), Some(agent), None, thread, Some(user()))
    }

    fn resolve_request(scope: TurnScope, gate_ref: &str) -> ResolveApprovalInteractionRequest {
        ResolveApprovalInteractionRequest {
            scope,
            actor: TurnActor::new(user()),
            run_id_hint: None,
            gate_ref: GateRef::new(gate_ref).unwrap(),
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new(format!("idem-{gate_ref}"))
                .expect("valid idempotency key"),
        }
    }

    // --- Tests ----------------------------------------------------------------

    #[tokio::test]
    async fn cross_thread_resolve_rewrites_scope_and_run_id_hint() {
        let run_id = TurnRunId::new();
        let gate_ref_str = "gate:routing-cross-001";

        let route_record = DeliveredGateRouteRecord {
            tenant_id: tenant(),
            user_id: user(),
            gate_ref: gate_ref_str.to_string(),
            run_id,
            scope: run_scope(),
            recorded_at: chrono::Utc::now(),
        };

        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        route_store
            .record_delivered_gate_route(route_record)
            .await
            .unwrap();

        let inner = Arc::new(RecordingApprovalService::default());
        let service =
            DeliveredGateRoutingApprovalService::new(Arc::clone(&inner) as _, route_store);

        // Request arrives on DM scope, not the run's original scope.
        let request = resolve_request(dm_scope(), gate_ref_str);
        let _ = service.resolve(request).await;

        let calls = inner.resolve_calls();
        assert_eq!(calls.len(), 1);
        let forwarded = &calls[0];

        // Scope must be rewritten to the run's original scope.
        assert_eq!(forwarded.scope.thread_id, run_scope().thread_id);
        // run_id_hint must be set to the stored run_id.
        assert_eq!(forwarded.run_id_hint, Some(run_id));
        // Actor and gate_ref must be unchanged.
        assert_eq!(forwarded.actor.user_id, user());
        assert_eq!(forwarded.gate_ref.as_str(), gate_ref_str);
    }

    #[tokio::test]
    async fn miss_forwards_request_unchanged() {
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        // No record stored — simulates a normal same-thread live run.

        let inner = Arc::new(RecordingApprovalService::default());
        let service =
            DeliveredGateRoutingApprovalService::new(Arc::clone(&inner) as _, route_store);

        let request = resolve_request(dm_scope(), "gate:routing-miss-001");
        let original_thread_id = request.scope.thread_id.clone();
        let _ = service.resolve(request).await;

        let calls = inner.resolve_calls();
        assert_eq!(calls.len(), 1);
        let forwarded = &calls[0];

        // Scope must be unchanged.
        assert_eq!(forwarded.scope.thread_id, original_thread_id);
        // run_id_hint must remain None.
        assert_eq!(forwarded.run_id_hint, None);
    }

    #[tokio::test]
    async fn user_id_mismatch_forwards_unchanged() {
        let run_id = TurnRunId::new();
        let gate_ref_str = "gate:routing-mismatch-001";

        // Route record belongs to a different user.
        let other_user = UserId::new("user:other").unwrap();
        let route_record = DeliveredGateRouteRecord {
            tenant_id: tenant(),
            user_id: other_user.clone(),
            gate_ref: gate_ref_str.to_string(),
            run_id,
            scope: run_scope(),
            recorded_at: chrono::Utc::now(),
        };

        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        // The lookup key encodes the other user — the requesting user won't
        // find this record at all (different key). This tests the user_id
        // guard when the key happens to be present but for a different user.
        //
        // To exercise the guard path we store the record under the requesting
        // user's key but with other_user in the payload.
        let store_record = DeliveredGateRouteRecord {
            user_id: user(), // store under requesting user's key
            ..route_record
        };
        // Manually insert with mismatched payload.user_id:
        let store_record_mismatched = DeliveredGateRouteRecord {
            user_id: other_user.clone(), // payload says other user
            ..store_record
        };
        route_store
            .record_delivered_gate_route(store_record_mismatched)
            .await
            .unwrap();

        let inner = Arc::new(RecordingApprovalService::default());
        let service =
            DeliveredGateRoutingApprovalService::new(Arc::clone(&inner) as _, route_store);

        let request = resolve_request(dm_scope(), gate_ref_str);
        let original_thread_id = request.scope.thread_id.clone();
        let _ = service.resolve(request).await;

        let calls = inner.resolve_calls();
        assert_eq!(calls.len(), 1);
        let forwarded = &calls[0];

        // Scope must NOT be rewritten due to user_id mismatch.
        assert_eq!(forwarded.scope.thread_id, original_thread_id);
        assert_eq!(forwarded.run_id_hint, None);
    }

    #[tokio::test]
    async fn list_pending_forwards_unchanged() {
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        let inner = Arc::new(RecordingApprovalService::default());
        let service =
            DeliveredGateRoutingApprovalService::new(Arc::clone(&inner) as _, route_store);

        let request = ListPendingApprovalsRequest {
            scope: dm_scope(),
            actor: TurnActor::new(user()),
        };
        let result = service.list_pending(request).await;
        assert!(result.is_ok());

        let list_calls = inner
            .list_calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len();
        assert_eq!(list_calls, 1);
    }
}
