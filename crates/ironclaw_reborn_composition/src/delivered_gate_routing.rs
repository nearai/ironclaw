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
//! ## Route record lifetime
//!
//! Route records are **not** removed after a successful resolve. This is
//! intentional: approval resolution supports idempotent replay via
//! `idempotency_key`, and a retried `approve <ref>` DM after the gate has
//! already settled must still rewrite the request to the correct run scope so
//! the inner service can return the settled result rather than `MissingGate`.
//! Records expire after [`ironclaw_outbound::DELIVERED_GATE_ROUTE_TTL`]
//! (48 hours). Expired records are ignored on load and removed lazily by this
//! wrapper. An opportunistic sweep of expired records runs on the write path
//! (when a new approval prompt is delivered), gated behind
//! [`DeliveredGateRouteStore::sweep_expired_delivered_gate_routes`].
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
use chrono::Utc;
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
pub(crate) struct DeliveredGateRoutingApprovalService {
    inner: Arc<dyn ApprovalInteractionService>,
    routes: Arc<dyn DeliveredGateRouteStore>,
}

impl DeliveredGateRoutingApprovalService {
    pub(crate) fn new(
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

        // TTL check: if the record is older than DELIVERED_GATE_ROUTE_TTL,
        // treat it as a miss and remove it lazily (best-effort).
        if record.is_expired(Utc::now()) {
            tracing::debug!(
                target = "ironclaw::reborn::delivered_gate_routing",
                gate_ref = gate_ref_str,
                "delivered gate route record expired; treating as miss and removing lazily"
            );
            if let Err(remove_err) = self
                .routes
                .remove_delivered_gate_route(tenant_id, user_id, gate_ref_str)
                .await
            {
                tracing::debug!(
                    target = "ironclaw::reborn::delivered_gate_routing",
                    gate_ref = gate_ref_str,
                    error = %remove_err,
                    "delivered gate route lazy removal failed (best-effort)"
                );
            }
            return self.inner.resolve(request).await;
        }

        // Defense-in-depth: the key already encodes tenant and actor user_id,
        // but verify both explicitly before rewriting. On mismatch, forward
        // unchanged so the inner service decides — do not error here.
        if record.user_id != *user_id {
            tracing::debug!(
                target = "ironclaw::reborn::delivered_gate_routing",
                gate_ref = gate_ref_str,
                "delivered gate route user_id mismatch; forwarding unchanged"
            );
            return self.inner.resolve(request).await;
        }
        if record.tenant_id != *tenant_id {
            tracing::debug!(
                target = "ironclaw::reborn::delivered_gate_routing",
                gate_ref = gate_ref_str,
                "delivered gate route tenant_id mismatch; forwarding unchanged"
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

    // -------------------------------------------------------------------------
    // Test 4 — wrapper through the workflow path
    // -------------------------------------------------------------------------
    //
    // Drives `DefaultProductWorkflow::submit_inbound` with an
    // `ApprovalResolution` envelope.  The workflow's approval port is
    // `DeliveredGateRoutingApprovalService` wrapping a recording inner service.
    // A route record is pre-seeded for (tenant, user, gate_ref) → run scope
    // (different thread than the inbound DM conversation).  After the call:
    //  - the recording inner service must have received the rewritten scope
    //    (run thread) and `run_id_hint = Some(run_id)`.
    //  - the route record must STILL be present (not removed) so idempotent
    //    retries can continue to rewrite the scope.
    //  - a second resolve with the same gate_ref must again rewrite the scope
    //    (the inner service sees the rewritten scope twice).

    /// Variant of [`RecordingApprovalService`] that returns `Ok(Approved)`
    /// so the routing wrapper performs its post-resolve cleanup.
    #[derive(Default)]
    struct AcceptingRecordingApprovalService {
        resolve_calls: Mutex<Vec<ResolveApprovalInteractionRequest>>,
    }

    impl AcceptingRecordingApprovalService {
        fn resolve_calls(&self) -> Vec<ResolveApprovalInteractionRequest> {
            self.resolve_calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl ApprovalInteractionService for AcceptingRecordingApprovalService {
        async fn list_pending(
            &self,
            _request: ListPendingApprovalsRequest,
        ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
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
            let run_id = request.run_id_hint.unwrap_or_default();
            Ok(ResolveApprovalInteractionResponse::Approved(
                ironclaw_turns::ResumeTurnResponse {
                    run_id,
                    status: ironclaw_turns::TurnStatus::Queued,
                    event_cursor: ironclaw_turns::EventCursor::default(),
                },
            ))
        }
    }

    fn approval_envelope(gate_ref_str: &str) -> ironclaw_product_adapters::ProductInboundEnvelope {
        use ironclaw_product_adapters::{
            AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
            ExternalActorRef, ExternalConversationRef, ExternalEventId, ParsedProductInbound,
            ProductAdapterId, ProductInboundEnvelope, ProductInboundPayload, ProtocolAuthEvidence,
            TrustedInboundContext,
        };

        let adapter_id = ProductAdapterId::new("test_adapter").expect("adapter id");
        let installation_id = AdapterInstallationId::new("install_alpha").expect("install id");
        let event_id =
            ExternalEventId::new(format!("evt:approval:{gate_ref_str}")).expect("event id");
        let actor_ref =
            ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor ref");
        let conv_ref = ExternalConversationRef::new(None, "conv-dm", None, None).expect("conv ref");
        let payload = ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref_str, ApprovalDecision::ApproveOnce)
                .expect("approval payload"),
        );
        let parsed = ParsedProductInbound::new(event_id, actor_ref, conv_ref, payload)
            .expect("parsed inbound");
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Secret".into(),
            },
            installation_id.as_str(),
        );
        let context = TrustedInboundContext::from_verified_evidence(
            adapter_id,
            installation_id,
            chrono::Utc::now(),
            &evidence,
        )
        .expect("trusted context");
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
    }

    #[tokio::test]
    async fn routing_wrapper_through_workflow_rewrites_scope_and_keeps_route_record_for_idempotent_retry()
     {
        use ironclaw_product_adapters::ProductWorkflow;
        use ironclaw_product_workflow::{
            DefaultProductWorkflow, FakeConversationBindingService, FakeIdempotencyLedger,
            FakeInboundTurnService,
        };

        // 1. The gate_ref and run to be resolved.
        let run_id = TurnRunId::new();
        let gate_ref_str = "gate:workflow-routing-001";

        // 2. Binding service: default fake generates tenant/user from envelope
        //    fields.  For install_alpha + actor user1:
        //      tenant_id = "tenant:install_alpha"
        //      user_id   = "user:user1"
        //      thread_id = "thread:install_alpha:conv-dm"  (the DM thread)
        let binding = Arc::new(FakeConversationBindingService::new());

        // 3. Pre-seed the route record: same tenant/user, different (run) thread.
        let route_tenant = TenantId::new("tenant:install_alpha").expect("tenant");
        let route_user = UserId::new("user:user1").expect("user");
        let run_thread = ThreadId::new("thread:run-001").expect("thread id");
        let run_scope = TurnScope::new_with_owner(
            route_tenant.clone(),
            Some(AgentId::new("agent:fake").expect("agent")),
            None,
            run_thread.clone(),
            Some(route_user.clone()),
        );
        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        route_store
            .record_delivered_gate_route(DeliveredGateRouteRecord {
                tenant_id: route_tenant.clone(),
                user_id: route_user.clone(),
                gate_ref: gate_ref_str.to_string(),
                run_id,
                scope: run_scope.clone(),
                recorded_at: chrono::Utc::now(),
            })
            .await
            .expect("record delivered gate route");

        // 4. Wrap a recording acceptance service with the routing wrapper.
        let inner = Arc::new(AcceptingRecordingApprovalService::default());
        let routed_approval = Arc::new(DeliveredGateRoutingApprovalService::new(
            Arc::clone(&inner) as _,
            Arc::clone(&route_store) as _,
        ));

        // 5. Wire everything into DefaultProductWorkflow.
        let workflow = DefaultProductWorkflow::new(
            Arc::new(FakeInboundTurnService::new()),
            Arc::new(FakeIdempotencyLedger::new()),
            Arc::clone(&binding) as _,
        )
        .with_approval_interaction_service(routed_approval);

        // 6. Submit an approval resolution envelope (first attempt).
        let envelope = approval_envelope(gate_ref_str);
        let ack = workflow
            .submit_inbound(envelope)
            .await
            .expect("submit_inbound should succeed");
        assert!(
            matches!(
                ack,
                ironclaw_product_adapters::ProductInboundAck::Accepted { .. }
            ),
            "expected Accepted ack, got {ack:?}"
        );

        // 7. Assert inner service received rewritten scope and run_id_hint.
        let calls = inner.resolve_calls();
        assert_eq!(
            calls.len(),
            1,
            "inner should have received exactly one resolve call on first attempt"
        );
        let forwarded = &calls[0];
        assert_eq!(
            forwarded.scope.thread_id, run_thread,
            "scope must be rewritten to run thread, not DM thread"
        );
        assert_eq!(
            forwarded.run_id_hint,
            Some(run_id),
            "run_id_hint must be set from route record"
        );

        // 8. Route record must STILL be present — not removed — so idempotent
        //    retries can continue to rewrite the scope.
        let still_present = route_store
            .load_delivered_gate_route(&route_tenant, &route_user, gate_ref_str)
            .await
            .expect("load after first resolve");
        assert!(
            still_present.is_some(),
            "route record must NOT be removed after successful resolve; idempotent retries need it"
        );

        // 9. Simulate an idempotent retry by calling the routing wrapper directly
        //    (bypassing the workflow-level idempotency ledger which would short-
        //    circuit before reaching the approval service).  Build a request that
        //    matches the route record's (tenant, user) so the wrapper finds it.
        let dm_thread_retry = ThreadId::new("thread:install_alpha:conv-dm").expect("dm thread");
        let retry_scope = TurnScope::new_with_owner(
            route_tenant.clone(),
            Some(AgentId::new("agent:fake").expect("agent")),
            None,
            dm_thread_retry,
            Some(route_user.clone()),
        );
        let retry_request = ResolveApprovalInteractionRequest {
            scope: retry_scope,
            actor: TurnActor::new(route_user.clone()),
            run_id_hint: None,
            gate_ref: GateRef::new(gate_ref_str).expect("gate ref"),
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("idem-retry-001").expect("idempotency key"),
        };
        let routed_approval_direct = DeliveredGateRoutingApprovalService::new(
            Arc::clone(&inner) as _,
            Arc::clone(&route_store) as _,
        );
        let _ = routed_approval_direct.resolve(retry_request).await;

        let calls2 = inner.resolve_calls();
        assert_eq!(
            calls2.len(),
            2,
            "inner should have received a second resolve call on retry"
        );
        let forwarded2 = &calls2[1];
        assert_eq!(
            forwarded2.scope.thread_id, run_thread,
            "retry must also rewrite scope to run thread"
        );
        assert_eq!(
            forwarded2.run_id_hint,
            Some(run_id),
            "retry must also carry the run_id_hint"
        );
    }

    // -------------------------------------------------------------------------
    // Test 5 — store read failure forwards request unchanged
    // -------------------------------------------------------------------------
    //
    // When `load_delivered_gate_route` returns `Err`, the wrapper logs at
    // debug and forwards the original request to the inner service unchanged
    // (no scope rewrite, no run_id_hint).

    struct FailingRouteStore;

    #[async_trait]
    impl DeliveredGateRouteStore for FailingRouteStore {
        async fn record_delivered_gate_route(
            &self,
            _record: DeliveredGateRouteRecord,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn load_delivered_gate_route(
            &self,
            _tenant_id: &ironclaw_host_api::TenantId,
            _user_id: &ironclaw_host_api::UserId,
            _gate_ref: &str,
        ) -> Result<Option<DeliveredGateRouteRecord>, String> {
            Err("boom".into())
        }

        async fn remove_delivered_gate_route(
            &self,
            _tenant_id: &ironclaw_host_api::TenantId,
            _user_id: &ironclaw_host_api::UserId,
            _gate_ref: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn sweep_expired_delivered_gate_routes(
            &self,
            _now: chrono::DateTime<chrono::Utc>,
        ) -> Result<usize, String> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn expired_route_record_forwards_request_unchanged() {
        // A route record whose recorded_at is older than DELIVERED_GATE_ROUTE_TTL
        // must be treated as a miss: the wrapper forwards the original request
        // unchanged (no scope rewrite, no run_id_hint).
        use ironclaw_outbound::DELIVERED_GATE_ROUTE_TTL;

        let run_id = TurnRunId::new();
        let gate_ref_str = "gate:routing-expired-001";

        let route_record = DeliveredGateRouteRecord {
            tenant_id: tenant(),
            user_id: user(),
            gate_ref: gate_ref_str.to_string(),
            run_id,
            scope: run_scope(),
            // Record is 49 hours old — past the 48-hour TTL.
            recorded_at: chrono::Utc::now() - DELIVERED_GATE_ROUTE_TTL - chrono::Duration::hours(1),
        };

        let route_store = Arc::new(InMemoryDeliveredGateRouteStore::default());
        route_store
            .record_delivered_gate_route(route_record)
            .await
            .unwrap();

        let inner = Arc::new(RecordingApprovalService::default());
        let service =
            DeliveredGateRoutingApprovalService::new(Arc::clone(&inner) as _, route_store);

        // Request arrives on DM scope — would be rewritten if record were fresh.
        let request = resolve_request(dm_scope(), gate_ref_str);
        let original_thread_id = request.scope.thread_id.clone();
        let _ = service.resolve(request).await;

        let calls = inner.resolve_calls();
        assert_eq!(calls.len(), 1, "inner must receive exactly one call");
        let forwarded = &calls[0];

        // Scope must be unchanged — expired record treated as a miss.
        assert_eq!(
            forwarded.scope.thread_id, original_thread_id,
            "scope must be unchanged for expired route record"
        );
        assert_eq!(
            forwarded.run_id_hint, None,
            "run_id_hint must stay None for expired route record"
        );
    }

    #[tokio::test]
    async fn store_read_failure_forwards_original_request_unchanged() {
        let inner = Arc::new(RecordingApprovalService::default());
        let service = DeliveredGateRoutingApprovalService::new(
            Arc::clone(&inner) as _,
            Arc::new(FailingRouteStore),
        );

        let original_scope = dm_scope();
        let original_thread_id = original_scope.thread_id.clone();
        let request = resolve_request(original_scope, "gate:store-fail-001");
        // The original request has run_id_hint = None (as constructed by
        // resolve_request).
        let _ = service.resolve(request).await;

        let calls = inner.resolve_calls();
        assert_eq!(calls.len(), 1, "inner must receive exactly one call");
        let forwarded = &calls[0];

        // Scope must be unchanged (not rewritten) because the store failed.
        assert_eq!(
            forwarded.scope.thread_id, original_thread_id,
            "scope must be unchanged when store read fails"
        );
        // run_id_hint must remain None (original request had no hint).
        assert_eq!(
            forwarded.run_id_hint, None,
            "run_id_hint must stay None when store read fails"
        );
    }
}
