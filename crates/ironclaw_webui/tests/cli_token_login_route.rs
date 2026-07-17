//! Caller-level tests for the CLI-token `/login?token=` route (B4).
//!
//! Drives the unauthenticated `Router` returned by
//! [`build_cli_token_login`] through `tower::ServiceExt::oneshot`,
//! mirroring `google_oauth_routes.rs`'s pattern for the OAuth
//! callback: a valid token mints a session and redirects the SPA
//! with a one-time `login_ticket`, which `POST /auth/session/exchange`
//! then redeems for the real bearer (the SPA's existing
//! `exchangeLoginTicket` call, per `crates/ironclaw_webui/frontend/
//! src/lib/api.ts:747-767` — no new frontend code). A wrong token
//! 401s and mints no ticket; a redeemed ticket is single-use.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use chrono::Duration as ChronoDuration;
use http_body_util::BodyExt;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_webui::{
    CliTokenLoginConfig, EnvBearerAuthenticator, SessionAuthenticator, SessionStore,
    build_cli_token_login, signed_session_store,
};
use secrecy::SecretString;
use serde::Deserialize;
use tower::ServiceExt;

const CLI_TOKEN: &str = "cli-secret-token";
const OPERATOR_USER: &str = "operator";

fn tenant() -> TenantId {
    TenantId::new("tenant-a").expect("tenant")
}

fn build_router() -> (axum::Router, Arc<dyn SessionStore>) {
    let session_store = signed_session_store(
        &SecretString::from("operator-secret".to_string()),
        &tenant(),
    );
    let authenticator = Arc::new(
        EnvBearerAuthenticator::new(
            SecretString::from(CLI_TOKEN.to_string()),
            UserId::new(OPERATOR_USER).expect("user"),
        )
        .expect("env bearer authenticator"),
    );
    let config = CliTokenLoginConfig::new(tenant(), authenticator, session_store.clone())
        .with_session_lifetime(ChronoDuration::hours(1));
    (build_cli_token_login(config).router, session_store)
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.expect("collect body").to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

#[derive(Deserialize)]
struct SessionExchangeResponse {
    token: String,
}

fn login_request(token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("/login?token={token}"))
        .body(Body::empty())
        .expect("request")
}

fn ticket_from_location(location: &str) -> String {
    let query = location.split_once('?').expect("query").1;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("login_ticket=") {
            return urlencoding::decode(value).expect("urldecode").into_owned();
        }
    }
    panic!("no login_ticket in {location}");
}

async fn exchange_ticket(router: axum::Router, ticket: &str) -> axum::response::Response {
    router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/session/exchange")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "ticket": ticket }).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot")
}

#[tokio::test]
async fn valid_token_redirects_with_ticket_that_exchanges_for_an_authenticating_bearer() {
    let (router, session_store) = build_router();

    let login = router
        .clone()
        .oneshot(login_request(CLI_TOKEN))
        .await
        .expect("oneshot");
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let location = login
        .headers()
        .get(header::LOCATION)
        .expect("Location header")
        .to_str()
        .expect("utf-8")
        .to_string();
    assert!(location.starts_with("/?login_ticket="), "got {location}");

    let ticket = ticket_from_location(&location);
    let exchange = exchange_ticket(router.clone(), &ticket).await;
    assert_eq!(exchange.status(), StatusCode::OK);
    let body = body_string(exchange.into_body()).await;
    let payload: SessionExchangeResponse = serde_json::from_str(&body).expect("json");
    assert!(!payload.token.is_empty());

    // The exchanged bearer must actually authenticate against the
    // session store the route minted through — proving the redirect's
    // ticket really carries a live session, not just an opaque value.
    let record = session_store
        .lookup(&payload.token)
        .await
        .expect("lookup")
        .expect("session must resolve");
    assert_eq!(record.user_id.as_str(), OPERATOR_USER);
    assert_eq!(record.tenant_id.as_str(), "tenant-a");

    // Single-use: redeeming the same ticket again must fail.
    let replay = exchange_ticket(router, &ticket).await;
    assert_eq!(
        replay.status(),
        StatusCode::UNAUTHORIZED,
        "login ticket must be single-use",
    );
}

#[tokio::test]
async fn wrong_token_is_rejected_and_mints_no_ticket() {
    let (router, _session_store) = build_router();

    let login = router
        .clone()
        .oneshot(login_request("not-the-token"))
        .await
        .expect("oneshot");
    assert_eq!(login.status(), StatusCode::UNAUTHORIZED);
    assert!(
        login.headers().get(header::LOCATION).is_none(),
        "a rejected login must not redirect (no ticket minted)",
    );

    // No ticket exists to exchange — a made-up value must also fail,
    // proving the store was never populated by the failed attempt.
    let exchange = exchange_ticket(router, "made-up-ticket").await;
    assert_eq!(exchange.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_token_is_rejected() {
    let (router, _session_store) = build_router();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/login")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ─── post-exchange bearer authorizes a protected route ────────────────
//
// `build_router` above only proves the ticket exchange returns a
// well-formed bearer — it never proves that bearer actually
// authenticates anything. Compose the SAME `session_store` the login
// mount mints through with the real production auth-verifying layer,
// [`SessionAuthenticator`] (the one `serve.rs` wires into
// `WebuiServeConfig` — see `session_round_trip.rs`'s `build_app` for the
// analogous OAuth-side wiring), fronting a minimal protected route. This
// is deliberately NOT the full `webui_v2_app` + `RebornServicesApi`
// composition `session_round_trip.rs` uses — that facade stub is
// scoped to proving the OAuth session round-trip through a real v2
// handler, and duplicating its ~30-method trait impl here just to prove
// "does this bearer authenticate" would be machinery this test doesn't
// need. `SessionAuthenticator` IS the seam that decides that question in
// production; a route behind it is enough to prove the login mount's
// bearer is a real, working session and not just an opaque string.

const PROTECTED_PATH: &str = "/protected/ping";

async fn require_session_bearer(
    State(authenticator): State<Arc<SessionAuthenticator>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let Some(token) = token else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // Reuses the real `WebuiAuthenticator::authenticate` contract — the
    // same call `authenticate_request` makes in
    // `ironclaw_reborn_composition::webui::webui_serve` — rather than
    // reimplementing session validation.
    if ironclaw_reborn_composition::WebuiAuthenticator::authenticate(&*authenticator, token)
        .await
        .is_none()
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

fn build_protected_router(session_store: Arc<dyn SessionStore>) -> axum::Router {
    let authenticator = Arc::new(SessionAuthenticator::new(session_store));
    axum::Router::new()
        .route(
            PROTECTED_PATH,
            axum::routing::get(|| async { StatusCode::OK }),
        )
        .route_layer(middleware::from_fn_with_state(
            authenticator,
            require_session_bearer,
        ))
}

fn protected_request(bearer: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(PROTECTED_PATH);
    if let Some(bearer) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    builder.body(Body::empty()).expect("request")
}

#[tokio::test]
async fn exchanged_bearer_authenticates_a_protected_route() {
    let (login_router, session_store) = build_router();
    let protected_router = build_protected_router(session_store);
    let app = login_router.merge(protected_router.clone());

    // RED first: an unauthenticated request against the protected route
    // must 401 — proves the route actually enforces the auth-verifying
    // layer before the rest of this test relies on a 200 meaning
    // something.
    let unauthenticated = protected_router
        .clone()
        .oneshot(protected_request(None))
        .await
        .expect("oneshot");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let login = app
        .clone()
        .oneshot(login_request(CLI_TOKEN))
        .await
        .expect("oneshot");
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let location = login
        .headers()
        .get(header::LOCATION)
        .expect("Location header")
        .to_str()
        .expect("utf-8")
        .to_string();
    let ticket = ticket_from_location(&location);

    let exchange = exchange_ticket(app.clone(), &ticket).await;
    assert_eq!(exchange.status(), StatusCode::OK);
    let body = body_string(exchange.into_body()).await;
    let payload: SessionExchangeResponse = serde_json::from_str(&body).expect("json");
    assert!(!payload.token.is_empty());

    // GREEN: the exchanged bearer must authenticate the protected route.
    let authenticated = app
        .oneshot(protected_request(Some(&payload.token)))
        .await
        .expect("oneshot");
    assert_eq!(
        authenticated.status(),
        StatusCode::OK,
        "the login mount's exchanged bearer must authorize a request through the real \
         SessionAuthenticator layer, not just decode as JSON",
    );
}
