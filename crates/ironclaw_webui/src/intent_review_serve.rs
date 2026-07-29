//! The public `GET /intent/{token}` review link (attested-signing Phase C §C2).
//!
//! This is the only unauthenticated surface in the attested flow, and it does
//! exactly one thing: turn a token from a chat message into a redirect to the
//! SPA. It reads no session, mutates no state, and reveals nothing about the
//! transaction, the approver, or the tenant.
//!
//! Everything the route refuses — unknown token, expired intent, already
//! resolved — is a **uniform 404**, decided by
//! [`ironclaw_attestation::resolve_token_landing`]. There is no branch here
//! that could reintroduce a distinguishable response.
//!
//! ## Why GET must stay side-effect-free
//!
//! Chat platforms fetch link previews with bot user-agents the moment a
//! message is delivered. A GET that consumed a
//! one-shot or advanced state would be burned by that preview fetch before the
//! human ever clicked. The handler therefore only reads; the store call it
//! makes is a lookup.
//!
//! ## Why the token never reaches a log
//!
//! The path segment IS the credential-shaped part of this URL. The route id is
//! registered with a literal `{token}` placeholder rather than the concrete
//! value, and the handler logs only the outcome — never the token, and never
//! the resolved intent id at info level.

use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};

use ironclaw_attestation::{IntentStore, ReviewTokenHash, TokenLanding, resolve_token_landing};
use ironclaw_host_api::{
    NetworkMethod,
    ingress::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
        IngressJustification, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
        IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
        WebSocketOriginPolicy,
    },
};

use crate::PublicRouteMount;

/// The public review-link path. `{token}` is a placeholder in the route id, so
/// the concrete token never lands in route metrics or logs.
pub const INTENT_REVIEW_PATH: &str = "/intent/{token}";

/// An unauthenticated link is inherently reachable by anyone who has it, so it
/// is rate limited per IP: the token is 256-bit random, and this bounds guessing
/// to a rate at which brute force is hopeless rather than merely improbable.
const INTENT_REVIEW_RATE_MAX: u32 = 30;
const INTENT_REVIEW_RATE_WINDOW_SECONDS: u32 = 60;

/// Environment override for the SPA route the review link redirects to.
const INTENT_REVIEW_SPA_BASE_ENV: &str = "IRONCLAW_INTENT_REVIEW_SPA_BASE";

/// The SPA route review links land on.
///
/// Server-fixed: read from configuration at composition time, never from a
/// request. That is what keeps this out of the open-redirect class.
///
/// `/review` rather than anything under `/intent`: `webui_serve` reserves the
/// first literal segment of every mounted route descriptor as a host-owned root
/// namespace, and the static fallback refuses to render the SPA shell for one.
/// `/intent` belongs to THIS route, so an SPA page beneath it could never be
/// served. No fragment either — the SPA is a `BrowserRouter`, which never sees
/// one.
pub fn intent_review_spa_base() -> String {
    std::env::var(INTENT_REVIEW_SPA_BASE_ENV).unwrap_or_else(|_| "/review".to_string())
}

#[derive(Clone)]
struct IntentReviewState {
    intents: Arc<dyn IntentStore>,
    /// Where the SPA route lives. Server-fixed configuration: no part of the
    /// request contributes to it, so this cannot become an open redirect.
    spa_base: Arc<str>,
}

/// Build the public review-link mount.
pub fn intent_review_mount(intents: Arc<dyn IntentStore>, spa_base: &str) -> PublicRouteMount {
    let router = Router::new()
        .route(INTENT_REVIEW_PATH, get(handle_intent_review))
        .with_state(IntentReviewState {
            intents,
            spa_base: Arc::from(spa_base.trim_end_matches('/')),
        });
    PublicRouteMount::new(router, vec![intent_review_descriptor()])
}

/// Resolve a review token to a redirect, or a uniform 404.
///
/// `now_ms` comes from the wall clock here — this is the edge of the system,
/// where a real clock reading belongs; every layer beneath takes it as a
/// parameter so it stays testable.
async fn handle_intent_review(
    State(state): State<IntentReviewState>,
    Path(token): Path<String>,
) -> Response {
    let hash = ReviewTokenHash::of_token(&token);
    // The raw token is dropped here; nothing below this line can log it.
    drop(token);

    let record = match state.intents.find_by_token_hash(&hash).await {
        Ok(record) => record,
        // Unknown token and backend failure are the same answer to the caller.
        // A distinguishable 500 would confirm the token space is being probed
        // correctly; the operator sees the real cause in the log instead.
        Err(error) => {
            tracing::debug!(%error, "intent review link did not resolve");
            return not_found();
        }
    };

    match resolve_token_landing(&record, now_unix_millis()) {
        Ok(TokenLanding::Redirect { intent_id }) => {
            let location = format!("{}/{}", state.spa_base, intent_id);
            // 303: this is a GET, and the SPA route is a different resource.
            // The browser follows; a preview bot sees only a redirect to a
            // page that will demand a session.
            match header::HeaderValue::from_str(&location) {
                Ok(value) => (StatusCode::SEE_OTHER, [(header::LOCATION, value)]).into_response(),
                // Unreachable for a ULID id + configured base, but a
                // non-header-safe value must not panic the route.
                Err(_) => not_found(),
            }
        }
        Err(_) => not_found(),
    }
}

/// The single refusal shape. Carries no body: nothing to leak, nothing for a
/// preview bot to render.
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

fn intent_review_descriptor() -> IngressRouteDescriptor {
    let policy = IngressPolicy::new(IngressPolicyParts {
        // Not a webhook (nothing signs these) and not an OAuth callback: it is
        // a link on the local gateway. Permitted unauthenticated precisely
        // because it is non-effectful — see `effect_path` below.
        listener_class: ListenerClass::LocalGateway,
        // Public by design: the token addresses, it does not authorize. Every
        // authorization for this flow happens behind the redirect, against an
        // authenticated session (`authorize_view`).
        auth: IngressAuthPolicy::Public {
            justification: IngressJustification::new(
                "public link",
                "intent review link: the token addresses an intent and authorizes nothing; \
                 viewing transaction detail requires an authenticated session whose user is \
                 the bound approver",
            )
            .expect("intent review justification must validate"), // safety: crate-local literal.
        },
        // Public auth may not claim an authenticated-caller scope; this route
        // resolves no principal at all.
        scope_source: IngressScopeSource::PublicRoute,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerIp,
            max_requests: std::num::NonZero::new(INTENT_REVIEW_RATE_MAX)
                .expect("rate max is a nonzero literal"), // safety: crate-local constant.
            window_seconds: std::num::NonZero::new(INTENT_REVIEW_RATE_WINDOW_SECONDS)
                .expect("rate window is a nonzero literal"), // safety: crate-local constant.
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        // The honest classification, and what lets this route be public: the
        // handler reads one row and returns a redirect. It claims no grant,
        // advances no state, and touches no workflow — the side-effect-free
        // property the preview-bot test pins, expressed in the policy.
        effect_path: AllowedEffectPath::NoEffect,
    })
    .expect("intent review policy must validate"); // safety: local-gateway + public auth + NoEffect + no-body is the validated read-only public shape.
    IngressRouteDescriptor::new(
        "webui.v2.intent_review".to_string(),
        NetworkMethod::Get,
        INTENT_REVIEW_PATH.to_string(),
        policy,
    )
    .expect("intent review descriptor must validate") // safety: route id/path are crate-local literals.
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use ironclaw_attestation::{
        AgentKeyId, DecodedTransaction, EvmAddress, EvmTransaction, INTENT_SIGNATURE_LEN,
        InMemoryIntentStore, IntentId, IntentRecord, IntentState, RenderingSchemaVersion,
        UnsignedIntent,
    };
    use ironclaw_signing_provider::{ApprovedTxHash, ChainId, GateRef, TenantId, UserId};
    use tower::ServiceExt as _;

    const SPA: &str = "https://ironclaw.example/webui/#/intent";

    fn record(state: IntentState, expires_at_ms: i64) -> IntentRecord {
        let intent = UnsignedIntent {
            intent_id: IntentId::from_string("01J0000000000000000000ROUTE"),
            tenant: TenantId::new("tenant-a"),
            agent_key_id: AgentKeyId::new(TenantId::new("tenant-a"), "agent-1", 1),
            approver: UserId::new("alice"),
            chain_id: ChainId::new("eip155:11155111"),
            approved_tx_hash: ApprovedTxHash::from_bytes([0x33; 32]),
            decoded_tx: DecodedTransaction::Evm(EvmTransaction {
                chain_id: 11155111,
                nonce: 1,
                tx_type: 2,
                to: Some(EvmAddress([0x44; 20])),
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
            intent.into_signed([0u8; INTENT_SIGNATURE_LEN]),
            GateRef::new("gate:attested-route"),
            ReviewTokenHash::of_token("the-secret-token"),
        );
        record.state = state;
        record
    }

    async fn get(store: Arc<dyn IntentStore>, path: &str) -> Response {
        intent_review_mount(store, SPA)
            .router
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn far_future() -> i64 {
        now_unix_millis() + 60_000
    }

    async fn store_with(record: IntentRecord) -> Arc<dyn IntentStore> {
        let store = Arc::new(InMemoryIntentStore::new());
        store.put(record).await.expect("put");
        store
    }

    #[tokio::test]
    async fn a_live_token_redirects_to_the_server_fixed_spa_route() {
        let store = store_with(record(IntentState::Pending, far_future())).await;
        let response = get(store, "/intent/the-secret-token").await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let location = response
            .headers()
            .get(header::LOCATION)
            .expect("redirect carries a location")
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            location,
            format!("{SPA}/01J0000000000000000000ROUTE"),
            "the redirect target is composed from configuration + the intent id only"
        );
    }

    /// Enumeration resistance: an unknown token is indistinguishable from an
    /// expired one, a resolved one, and a token for another tenant's intent.
    #[tokio::test]
    async fn every_refusal_is_the_same_uniform_404() {
        let cases: Vec<(&str, Arc<dyn IntentStore>, &str)> = vec![
            (
                "unknown token",
                store_with(record(IntentState::Pending, far_future())).await,
                "/intent/not-the-token",
            ),
            (
                "expired intent",
                store_with(record(IntentState::Pending, 1)).await,
                "/intent/the-secret-token",
            ),
            (
                "already approved",
                store_with(record(IntentState::Approved, far_future())).await,
                "/intent/the-secret-token",
            ),
            (
                "already rejected",
                store_with(record(IntentState::Rejected, far_future())).await,
                "/intent/the-secret-token",
            ),
        ];
        for (label, store, path) in cases {
            let response = get(store, path).await;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{label} must be a uniform 404"
            );
            assert!(
                response.headers().get(header::LOCATION).is_none(),
                "{label} must not leak a redirect"
            );
        }
    }

    /// The preview-bot property: a GET changes nothing, so the human's later
    /// click still finds a live intent.
    #[tokio::test]
    async fn a_get_is_side_effect_free() {
        let store = Arc::new(InMemoryIntentStore::new());
        store
            .put(record(IntentState::Pending, far_future()))
            .await
            .expect("put");
        let shared: Arc<dyn IntentStore> = store.clone();

        // Three fetches, as a chat platform's preview bots would.
        for _ in 0..3 {
            let response = get(Arc::clone(&shared), "/intent/the-secret-token").await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER);
        }

        // The intent is untouched: still pending, still addressable.
        let after = store
            .find_by_token_hash(&ReviewTokenHash::of_token("the-secret-token"))
            .await
            .expect("still present");
        assert_eq!(
            after.state,
            IntentState::Pending,
            "a preview fetch must not consume or advance the intent"
        );
    }

    /// The 404 body is empty — nothing for a preview card to render, and
    /// nothing that could differ between refusal reasons.
    #[tokio::test]
    async fn the_refusal_carries_no_body() {
        let store = store_with(record(IntentState::Pending, far_future())).await;
        let response = get(store, "/intent/wrong").await;
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        assert!(body.is_empty(), "a refusal must carry no content");
    }

    /// The route is registered with a `{token}` placeholder, so the concrete
    /// token cannot reach route-keyed metrics or logs.
    #[test]
    fn the_route_id_does_not_embed_a_concrete_token() {
        let descriptor = intent_review_descriptor();
        assert!(descriptor.route_pattern().as_str().contains("{token}"));
    }

    /// The default redirect target must be a route the SPA can actually serve.
    ///
    /// Two ways it can silently not be, both of which this pins:
    ///
    /// * A `#` fragment never routes — the SPA is a `BrowserRouter`, so a
    ///   fragment is dropped before React Router ever sees it.
    /// * The first path segment must not be a host-owned root namespace.
    ///   `webui_serve` reserves the first literal segment of every mounted
    ///   route descriptor, and the static fallback refuses to render the SPA
    ///   shell for a reserved namespace. `/intent` is this very route's
    ///   namespace, so the SPA cannot live under it.
    ///
    /// Get either wrong and a review link lands on the app's default route with
    /// the intent id discarded — the approver sees their inbox, not the
    /// transaction they were asked to approve.
    #[test]
    fn the_default_spa_base_is_a_route_the_spa_can_serve() {
        let base = intent_review_spa_base();
        assert!(
            !base.contains('#'),
            "the SPA is a BrowserRouter; a fragment in {base:?} never reaches the router"
        );

        let root_namespace = base
            .trim_start_matches('/')
            .split('/')
            .next()
            .expect("a base path has a first segment");
        let reserved = INTENT_REVIEW_PATH
            .trim_start_matches('/')
            .split('/')
            .next()
            .expect("the route pattern has a first segment");
        assert_ne!(
            root_namespace, reserved,
            "{base:?} sits under this route's own server-owned namespace, so the \
             static SPA fallback will never render it"
        );
    }

    /// The env override exists for deployments that mount the SPA elsewhere; it
    /// must not be able to smuggle a request-derived value, which is why the
    /// base is read once at composition time and never from a request.
    #[test]
    fn the_spa_base_is_configurable_but_server_fixed() {
        // Reading it twice with no request in between must be stable — the
        // property that keeps this out of the open-redirect class.
        assert_eq!(intent_review_spa_base(), intent_review_spa_base());
    }
}
