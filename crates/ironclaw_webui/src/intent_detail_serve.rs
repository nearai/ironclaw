//! The authenticated intent-detail read (attested-signing Phase C §C3).
//!
//! `GET /api/webchat/v2/intents/{intent_id}` — what the `/review/:intentId`
//! SPA page calls once the public link has redirected it there and the session
//! layer has established who is asking.
//!
//! ## The token showed nothing; this is where authorization happens
//!
//! The public `/intent/{token}` route only turns a token into a redirect. It
//! reveals no transaction detail, because a link forwarded into a group chat
//! would otherwise expose one. Every authorization for this flow lands here,
//! against a session, in
//! [`ironclaw_attestation::authorize_view`]: the session user must equal the
//! intent's bound approver AND the session tenant must equal the intent's
//! tenant. A token holder who is not the approver gets exactly what a stranger
//! gets.
//!
//! ## Uniform 404, same as the public route
//!
//! Unknown id, wrong tenant, wrong user, expired, and backend failure are one
//! response. Anything else turns this into an oracle for which intent ids
//! exist and who their approvers are — and an authenticated attacker probing
//! ids is exactly the caller this endpoint has to assume.
//!
//! ## The signing-context sibling
//!
//! `GET /api/webchat/v2/intents/{intent_id}/signing-context` serves the
//! ERC-7730 descriptor for the same intent, under the same authorization. It
//! exists because the SPA has a zero-remote-origins CSP and cannot reach
//! Ledger's context service itself (§D3). Its refusal is not an error: a
//! transaction with no descriptor returns `{"clear_signing": "unavailable"}`
//! with HTTP 200, and the page must render "this cannot be clear-signed"
//! rather than offering a blind-sign path.
//!
//! ## What the DTO deliberately omits
//!
//! The signature, the review-token hash, and the agent key id never leave the
//! server. The page renders a transaction for a human to compare against their
//! device screen; none of those three help with that, and each is material an
//! attacker would rather have. `approved_tx_hash` IS included: it is the value
//! the Ledger will display, so the human needs it to compare.

use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};

use ironclaw_attestation::{
    DecodedTransaction, IntentId, IntentRecord, IntentStore, ReviewCaller, authorize_view,
};
use ironclaw_attested_runtime::{
    DescriptorKey, DescriptorLookup, DescriptorSource, TtlDescriptorCache,
};
use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ProductSurfaceCaller;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};
use ironclaw_signing_provider::{TenantId as SigningTenantId, UserId as SigningUserId};
use serde::Serialize;

use crate::ProtectedRouteMount;

/// The detail path. `{intent_id}` is a placeholder in the route id.
pub const INTENT_DETAIL_PATH: &str = "/api/webchat/v2/intents/{intent_id}";

/// The clear-signing descriptor path for the same intent.
pub const SIGNING_CONTEXT_PATH: &str = "/api/webchat/v2/intents/{intent_id}/signing-context";

/// Per-caller ceiling. Generous for a human reading one page, tight enough that
/// an authenticated session cannot sweep the id space quickly.
const INTENT_DETAIL_MAX_REQUESTS: std::num::NonZero<u32> =
    std::num::NonZero::new(60).expect("nonzero literal"); // safety: const-evaluated — a zero literal fails the build, never runtime
const INTENT_DETAIL_RATE_WINDOW_SECONDS: std::num::NonZero<u32> =
    std::num::NonZero::new(60).expect("nonzero literal"); // safety: const-evaluated — a zero literal fails the build, never runtime

#[derive(Clone)]
struct IntentDetailState {
    intents: Arc<dyn IntentStore>,
    /// Descriptors for the clear-signing sibling route. Shared so the TTL cache
    /// is process-wide rather than per-request.
    descriptors: Arc<TtlDescriptorCache<Arc<dyn DescriptorSource>>>,
}

/// What the review page renders.
///
/// Sanitized by construction: there is no field here the server would have to
/// remember to strip. See the module note on what is omitted and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct IntentDetailDto {
    /// The intent's id, echoed so the page can assert it got what it asked for.
    pub intent_id: String,
    /// Lifecycle projection: `pending`, `approved`, `rejected`, `expired`.
    pub state: String,
    /// CAIP-2 chain id, so the page can name the network.
    pub chain_id: String,
    /// The hash the device will display. The human compares this against their
    /// Ledger screen, so it is the one credential-shaped value that belongs
    /// here.
    pub approved_tx_hash: String,
    /// When the approval window closes (unix millis), for the countdown.
    pub expires_at_ms: i64,
    /// The decoded transaction, exactly as the authoritative decode produced
    /// it. The page renders it; it does not re-derive anything from it.
    pub decoded_tx: serde_json::Value,
}

/// Build the protected detail mount, including the clear-signing sibling.
pub fn intent_detail_mount(
    intents: Arc<dyn IntentStore>,
    descriptors: Arc<TtlDescriptorCache<Arc<dyn DescriptorSource>>>,
) -> ProtectedRouteMount {
    let router = Router::new()
        .route(INTENT_DETAIL_PATH, get(handle_intent_detail))
        .route(SIGNING_CONTEXT_PATH, get(handle_signing_context))
        .with_state(IntentDetailState {
            intents,
            descriptors,
        });
    ProtectedRouteMount::new(
        router,
        vec![intent_detail_descriptor(), signing_context_descriptor()],
    )
}

/// Serve the ERC-7730 descriptor for an intent, or say plainly that there is
/// none.
///
/// Authorization is identical to the detail read — an unauthorized caller gets
/// the same uniform 404, so this route cannot be used to probe which intents
/// exist. Beyond that gate the response is always 200: "no descriptor" is an
/// ANSWER, not a failure, and the page renders it as a blocked ceremony.
async fn handle_signing_context(
    State(state): State<IntentDetailState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(intent_id): Path<String>,
) -> Response {
    let intent_id = IntentId::from_string(intent_id);
    let tenant = SigningTenantId::new(caller.tenant_id.as_str());
    let user = SigningUserId::new(caller.user_id.as_str());

    let record = match state.intents.get(&tenant, &intent_id).await {
        Ok(record) => record,
        Err(error) => {
            tracing::debug!(%error, "signing-context lookup did not resolve");
            return not_found();
        }
    };
    let review_caller = ReviewCaller {
        user: &user,
        tenant: &tenant,
    };
    let record = match authorize_view(&record, review_caller, now_unix_millis()) {
        Ok(record) => record,
        Err(_) => return not_found(),
    };

    let intent = record.intent.intent();
    let key = descriptor_key_for(intent.chain_id.as_str(), &intent.decoded_tx);
    // No contract call to describe (a plain transfer or a deployment) is
    // reported as unavailable, exactly like a missing descriptor: either way
    // the device cannot render fields, so the ceremony blocks.
    let lookup = match key {
        Some(key) => state.descriptors.lookup(&key, now_unix_millis()).await,
        None => DescriptorLookup::NotAvailable,
    };

    let body = match lookup {
        DescriptorLookup::Available { descriptor } => serde_json::json!({
            "clear_signing": "available",
            "descriptor": descriptor,
        }),
        DescriptorLookup::NotAvailable => serde_json::json!({
            "clear_signing": "unavailable",
        }),
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// The descriptor key for a decoded transaction, when it describes a call.
fn descriptor_key_for(chain_id: &str, decoded: &DecodedTransaction) -> Option<DescriptorKey> {
    match decoded {
        DecodedTransaction::Evm(tx) => {
            let to = tx
                .to
                .as_ref()
                .map(|address| format!("0x{}", hex::encode(address.0)));
            DescriptorKey::from_call(chain_id, to.as_deref(), &tx.data)
        }
        // Clear signing is EVM-only in v1; other chains block rather than
        // silently claiming a descriptor applies.
        _ => None,
    }
}

fn signing_context_descriptor() -> IngressRouteDescriptor {
    let policy = IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: INTENT_DETAIL_MAX_REQUESTS,
            window_seconds: INTENT_DETAIL_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::NoEffect,
    })
    .expect("signing context policy must validate"); // safety: same validated shape as the detail route.
    IngressRouteDescriptor::new(
        "webui.v2.intent_signing_context".to_string(),
        NetworkMethod::Get,
        SIGNING_CONTEXT_PATH.to_string(),
        policy,
    )
    .expect("signing context route descriptor must validate at startup") // safety: crate-local literals.
}

/// Resolve an intent for its bound approver, or a uniform 404.
async fn handle_intent_detail(
    State(state): State<IntentDetailState>,
    Extension(caller): Extension<ProductSurfaceCaller>,
    Path(intent_id): Path<String>,
) -> Response {
    let intent_id = IntentId::from_string(intent_id);
    // The session carries host-api identities; the attestation stores speak the
    // signing-provider newtypes. Bridge once, here, rather than letting either
    // spelling leak into the other layer.
    let tenant = SigningTenantId::new(caller.tenant_id.as_str());
    let user = SigningUserId::new(caller.user_id.as_str());

    // Tenant-qualified at the store, then authorized against the session. The
    // store read alone is not the authorization: it proves the intent belongs
    // to the caller's tenant, not that the caller is its approver.
    let record = match state.intents.get(&tenant, &intent_id).await {
        Ok(record) => record,
        Err(error) => {
            tracing::debug!(%error, "intent detail lookup did not resolve");
            return not_found();
        }
    };

    let caller = ReviewCaller {
        user: &user,
        tenant: &tenant,
    };
    match authorize_view(&record, caller, now_unix_millis()) {
        Ok(record) => match detail_of(record) {
            Ok(dto) => (StatusCode::OK, Json(dto)).into_response(),
            Err(error) => {
                tracing::debug!(%error, "intent detail did not serialize");
                not_found()
            }
        },
        // Wrong tenant, wrong user, and expired are one answer.
        Err(_) => not_found(),
    }
}

/// Project a record onto the DTO.
fn detail_of(record: &IntentRecord) -> Result<IntentDetailDto, serde_json::Error> {
    let intent = record.intent.intent();
    Ok(IntentDetailDto {
        intent_id: intent.intent_id.as_str().to_string(),
        state: state_str(record.state).to_string(),
        chain_id: intent.chain_id.as_str().to_string(),
        approved_tx_hash: hex::encode(intent.approved_tx_hash.as_bytes()),
        expires_at_ms: intent.expires_at_ms,
        decoded_tx: serde_json::to_value(&intent.decoded_tx)?,
    })
}

fn state_str(state: ironclaw_attestation::IntentState) -> &'static str {
    use ironclaw_attestation::IntentState;
    match state {
        IntentState::Pending => "pending",
        IntentState::Approved => "approved",
        IntentState::Rejected => "rejected",
        IntentState::Expired => "expired",
    }
}

/// The single refusal shape, carrying no body.
fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

fn now_unix_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn intent_detail_descriptor() -> IngressRouteDescriptor {
    let policy = IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::BearerToken],
        },
        scope_source: IngressScopeSource::AuthenticatedCaller,
        // A GET with no body.
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: INTENT_DETAIL_MAX_REQUESTS,
            window_seconds: INTENT_DETAIL_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        // A human opened their review page.
        audit: AuditTraceClass::UserAction,
        // A read: it resolves nothing and claims nothing.
        effect_path: AllowedEffectPath::NoEffect,
    })
    .expect("intent detail policy must validate"); // safety: local-gateway + bearer auth + NoEffect + no-body is a validated shape.
    IngressRouteDescriptor::new(
        "webui.v2.intent_detail".to_string(),
        NetworkMethod::Get,
        INTENT_DETAIL_PATH.to_string(),
        policy,
    )
    .expect("intent detail route descriptor must validate at startup") // safety: id/pattern are crate-local literals; the policy comes from the helper above.
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use ironclaw_attestation::{
        AgentKeyId, DecodedTransaction, EvmAddress, EvmTransaction, INTENT_SIGNATURE_LEN,
        InMemoryIntentStore, IntentRecord, IntentState, RenderingSchemaVersion, ReviewTokenHash,
        UnsignedIntent,
    };
    use ironclaw_signing_provider::{ApprovedTxHash, ChainId, GateRef, TenantId, UserId};
    use tower::ServiceExt as _;

    const ID: &str = "01J0000000000000000000DETAIL";

    fn record(
        tenant: &str,
        approver: &str,
        state: IntentState,
        expires_at_ms: i64,
    ) -> IntentRecord {
        let intent = UnsignedIntent {
            intent_id: IntentId::from_string(ID),
            tenant: TenantId::new(tenant),
            agent_key_id: AgentKeyId::new(TenantId::new(tenant), "agent-1", 1),
            approver: UserId::new(approver),
            chain_id: ChainId::new("eip155:11155111"),
            approved_tx_hash: ApprovedTxHash::from_bytes([0x77; 32]),
            decoded_tx: DecodedTransaction::Evm(EvmTransaction {
                chain_id: 11155111,
                nonce: 7,
                tx_type: 2,
                to: Some(EvmAddress([0x99; 20])),
                value: vec![],
                data: vec![],
                gas_limit: 21_000,
                gas_price: None,
                max_fee_per_gas: Some(vec![0x09]),
                max_priority_fee_per_gas: Some(vec![0x3b]),
                access_list: vec![],
                max_fee_per_blob_gas: None,
                blob_versioned_hashes: vec![],
            }),
            created_at_ms: 0,
            expires_at_ms,
            schema_version: RenderingSchemaVersion::CURRENT,
        };
        let mut record = IntentRecord::pending(
            intent.into_signed([0xEE; INTENT_SIGNATURE_LEN]),
            GateRef::new("gate:attested-detail"),
            ReviewTokenHash::of_token("a-secret-review-token"),
        );
        record.state = state;
        record
    }

    /// Far enough out that the wall clock inside the handler cannot expire it.
    fn far_future() -> i64 {
        now_unix_millis() + 86_400_000
    }

    async fn get_as(store: Arc<dyn IntentStore>, tenant: &str, user: &str, id: &str) -> Response {
        // Host-api identities: this is the session shape the auth layer
        // installs, which is exactly what the handler has to bridge.
        let caller = ProductSurfaceCaller::new(
            ironclaw_host_api::TenantId::new(tenant).expect("test tenant id"),
            ironclaw_host_api::UserId::new(user).expect("test user id"),
            None,
            None,
        );
        let request = Request::builder()
            .uri(format!("/api/webchat/v2/intents/{id}"))
            .extension(caller)
            .body(Body::empty())
            .expect("request");
        intent_detail_mount(store, descriptors_for_test())
            .router
            .oneshot(request)
            .await
            .expect("response")
    }

    /// Tests default to no descriptor service: the blocked outcome is the
    /// default everywhere, including here.
    fn descriptors_for_test() -> Arc<TtlDescriptorCache<Arc<dyn DescriptorSource>>> {
        Arc::new(TtlDescriptorCache::new(
            Arc::new(ironclaw_attested_runtime::UnconfiguredDescriptorSource)
                as Arc<dyn DescriptorSource>,
            60_000,
            5_000,
        ))
    }

    async fn store_with(record: IntentRecord) -> Arc<dyn IntentStore> {
        let store = InMemoryIntentStore::new();
        store.put(record).await.expect("put");
        Arc::new(store)
    }

    async fn body_of(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[tokio::test]
    async fn the_bound_approver_sees_the_transaction() {
        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Pending,
            far_future(),
        ))
        .await;
        let response = get_as(store, "tenant-a", "alice", ID).await;
        assert_eq!(response.status(), StatusCode::OK);

        let body = body_of(response).await;
        assert_eq!(body["intent_id"], ID);
        assert_eq!(body["state"], "pending");
        assert_eq!(body["chain_id"], "eip155:11155111");
        // The hash the device will show, so the human can compare it.
        assert_eq!(body["approved_tx_hash"], "77".repeat(32));
        assert_eq!(body["decoded_tx"]["nonce"], 7);
    }

    /// The whole point of Q4: holding the link is not being the approver.
    #[tokio::test]
    async fn a_different_user_in_the_same_tenant_gets_the_uniform_404() {
        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Pending,
            far_future(),
        ))
        .await;
        let response = get_as(store, "tenant-a", "mallory", ID).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_cross_tenant_caller_gets_the_uniform_404() {
        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Pending,
            far_future(),
        ))
        .await;
        // Same user name under another tenant is a different principal.
        let response = get_as(store, "tenant-b", "alice", ID).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Unknown id, wrong user, wrong tenant, and expired must be ONE response —
    /// otherwise an authenticated caller can probe which ids exist and who
    /// approves them.
    #[tokio::test]
    async fn every_refusal_is_indistinguishable() {
        let live = record("tenant-a", "alice", IntentState::Pending, far_future());
        let expired = record("tenant-a", "alice", IntentState::Pending, 1);

        let cases: Vec<(&str, Response)> = vec![
            (
                "unknown id",
                get_as(
                    store_with(live.clone()).await,
                    "tenant-a",
                    "alice",
                    "01J000000000000000000ABSENT",
                )
                .await,
            ),
            (
                "wrong user",
                get_as(store_with(live.clone()).await, "tenant-a", "mallory", ID).await,
            ),
            (
                "wrong tenant",
                get_as(store_with(live).await, "tenant-b", "alice", ID).await,
            ),
            (
                "expired",
                get_as(store_with(expired).await, "tenant-a", "alice", ID).await,
            ),
        ];

        for (label, response) in cases {
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{label} must be the uniform refusal"
            );
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("body");
            assert!(
                bytes.is_empty(),
                "{label} must carry no body to distinguish it"
            );
        }
    }

    /// The DTO is sanitized by construction. If a future field lands on it,
    /// this fails and forces the question to be answered deliberately.
    #[tokio::test]
    async fn the_response_never_carries_the_signature_token_hash_or_key_id() {
        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Pending,
            far_future(),
        ))
        .await;
        let body = body_of(get_as(store, "tenant-a", "alice", ID).await).await;

        let rendered = body.to_string();
        assert!(
            !rendered.contains(&"ee".repeat(32)),
            "the intent signature must not reach the page"
        );
        assert!(
            !rendered.contains("review_token"),
            "the review token hash must not reach the page"
        );
        assert!(
            !rendered.contains("agent_key_id") && !rendered.contains("agent-1"),
            "the agent key id must not reach the page"
        );

        let object = body.as_object().expect("a JSON object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "approved_tx_hash",
                "chain_id",
                "decoded_tx",
                "expires_at_ms",
                "intent_id",
                "state"
            ],
            "a new field on the review DTO must be a deliberate decision"
        );
    }

    async fn get_signing_context(
        store: Arc<dyn IntentStore>,
        descriptors: Arc<TtlDescriptorCache<Arc<dyn DescriptorSource>>>,
        tenant: &str,
        user: &str,
    ) -> Response {
        let caller = ProductSurfaceCaller::new(
            ironclaw_host_api::TenantId::new(tenant).expect("test tenant id"),
            ironclaw_host_api::UserId::new(user).expect("test user id"),
            None,
            None,
        );
        let request = Request::builder()
            .uri(format!("/api/webchat/v2/intents/{ID}/signing-context"))
            .extension(caller)
            .body(Body::empty())
            .expect("request");
        intent_detail_mount(store, descriptors)
            .router
            .oneshot(request)
            .await
            .expect("response")
    }

    /// A source that always has a descriptor, to prove the available branch is
    /// reachable and that the blocked branch is not merely the only one wired.
    struct AlwaysAvailable;

    #[async_trait::async_trait]
    impl DescriptorSource for AlwaysAvailable {
        async fn lookup(&self, _key: &DescriptorKey) -> DescriptorLookup {
            DescriptorLookup::Available {
                descriptor: serde_json::json!({ "context": { "contract": {} } }),
            }
        }
    }

    /// THE fail-closed property (§D3). No descriptor service wired means every
    /// ceremony blocks — with a 200 carrying an explicit "unavailable", never
    /// an error a client might treat as "proceed anyway".
    #[tokio::test]
    async fn no_descriptor_service_blocks_the_ceremony_explicitly() {
        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Pending,
            far_future(),
        ))
        .await;
        let response =
            get_signing_context(store, descriptors_for_test(), "tenant-a", "alice").await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_of(response).await;
        assert_eq!(body["clear_signing"], "unavailable");
        assert!(
            body.get("descriptor").is_none(),
            "a blocked ceremony must carry nothing the client could sign from"
        );
    }

    /// The descriptor key is derived from the TRANSACTION, never asked for by
    /// the client — so a transaction with nothing to describe stays blocked
    /// even against a source that would happily answer anything.
    #[tokio::test]
    async fn a_transaction_with_no_call_to_describe_stays_blocked_against_any_source() {
        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Pending,
            far_future(),
        ))
        .await;
        let descriptors: Arc<TtlDescriptorCache<Arc<dyn DescriptorSource>>> =
            Arc::new(TtlDescriptorCache::new(
                Arc::new(AlwaysAvailable) as Arc<dyn DescriptorSource>,
                60_000,
                5_000,
            ));
        let response = get_signing_context(store, descriptors, "tenant-a", "alice").await;
        assert_eq!(response.status(), StatusCode::OK);

        // The fixture is a bare transfer: no selector, nothing to describe. An
        // always-available source cannot rescue it, because no key is ever
        // built to ask with.
        assert_eq!(body_of(response).await["clear_signing"], "unavailable");
    }

    /// The signing-context route must not become a softer way to probe intents
    /// than the detail route it sits beside.
    #[tokio::test]
    async fn the_signing_context_route_refuses_a_non_approver_identically() {
        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Pending,
            far_future(),
        ))
        .await;
        let response =
            get_signing_context(store, descriptors_for_test(), "tenant-a", "mallory").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Pending,
            far_future(),
        ))
        .await;
        let response =
            get_signing_context(store, descriptors_for_test(), "tenant-b", "alice").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// A terminal intent still renders — the approver who just signed should
    /// see the outcome rather than a 404. (Only `authorize_proof_submission`
    /// refuses terminal states; viewing is not submitting.)
    #[tokio::test]
    async fn a_resolved_intent_still_renders_its_outcome() {
        let store = store_with(record(
            "tenant-a",
            "alice",
            IntentState::Approved,
            far_future(),
        ))
        .await;
        let response = get_as(store, "tenant-a", "alice", ID).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_of(response).await["state"], "approved");
    }
}
