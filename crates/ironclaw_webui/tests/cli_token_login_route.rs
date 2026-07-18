//! Caller-level tests for the CLI-token `/login?token=` route (B4).
//!
//! Drives the unauthenticated `Router` from [`build_cli_token_login`] via
//! `tower::ServiceExt::oneshot`, mirroring `google_oauth_routes.rs`'s
//! OAuth-callback pattern:
//! - valid token → mints a session, redirects with a one-time `login_ticket`
//! - `POST /auth/session/exchange` redeems it for the real bearer (same
//!   contract the SPA's `exchangeLoginTicket` already uses)
//! - wrong token → 401, no ticket minted; a redeemed ticket is single-use

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

    // Must authenticate against the store the route minted through — not
    // just an opaque value.
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

    // Store was never populated by the failed attempt.
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
// `build_router` above only proves the exchange returns a well-formed
// bearer, not that it authenticates anything. Front a minimal protected
// route with the real prod auth layer, [`SessionAuthenticator`] (same as
// `serve.rs`'s `WebuiServeConfig`), fed the SAME `session_store` the login
// mount mints through.
// - deliberately NOT the full `webui_v2_app` composition
//   `session_round_trip.rs` uses — that facade is scoped to the OAuth
//   round-trip; duplicating it here is unneeded machinery.
// - `SessionAuthenticator` IS the seam that decides "does this bearer
//   authenticate" in production, so a route behind it suffices.

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
    // Reuses the real WebuiAuthenticator::authenticate contract (same call
    // as webui_serve's authenticate_request) rather than reimplementing.
    if ironclaw_webui::WebuiAuthenticator::authenticate(&*authenticator, token)
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

// USER-DECIDED LAW: webui-token auth = operator/admin, whether via raw
// `Authorization: Bearer` or this `/login?token=` link. A session minted
// through this route must authenticate with `operator_webui_config = true`
// so the caller gets the same admin capabilities as a raw bearer check.
#[tokio::test]
async fn exchanged_bearer_from_cli_token_login_is_operator_capable() {
    let (router, session_store) = build_router();
    let authenticator = SessionAuthenticator::new(session_store);

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
    let ticket = ticket_from_location(&location);

    let exchange = exchange_ticket(router, &ticket).await;
    assert_eq!(exchange.status(), StatusCode::OK);
    let body = body_string(exchange.into_body()).await;
    let payload: SessionExchangeResponse = serde_json::from_str(&body).expect("json");

    let auth = ironclaw_webui::WebuiAuthenticator::authenticate(&authenticator, &payload.token)
        .await
        .expect("exchanged bearer must authenticate");
    assert!(
        auth.capabilities.operator_webui_config,
        "a session minted via the CLI-token /login link, from a token that \
         verified against the operator-capable authenticator, must \
         authenticate with operator capabilities",
    );
}

// Non-operator mirror of `exchanged_bearer_from_cli_token_login_is_operator_capable`:
// `EnvBearerAuthenticator::authenticate` always returns an operator
// `WebuiAuthentication`, so it alone can't prove `login_handler` actually
// carries the token's own `operator_webui_config` bit through to
// `create_session` rather than hardcoding `true`. This authenticator
// returns a non-operator identity so a session minted through it must NOT
// come out operator-capable.
struct NonOperatorAuthenticator {
    token: &'static str,
    user_id: UserId,
}

#[async_trait::async_trait]
impl ironclaw_webui::WebuiAuthenticator for NonOperatorAuthenticator {
    async fn authenticate(&self, candidate: &str) -> Option<ironclaw_webui::WebuiAuthentication> {
        (candidate == self.token)
            .then(|| ironclaw_webui::WebuiAuthentication::user(self.user_id.clone()))
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn exchanged_bearer_from_a_non_operator_authenticator_is_not_operator_capable() {
    let session_store = signed_session_store(
        &SecretString::from("operator-secret".to_string()),
        &tenant(),
    );
    let authenticator = Arc::new(NonOperatorAuthenticator {
        token: CLI_TOKEN,
        user_id: UserId::new("member").expect("user"),
    });
    let config = CliTokenLoginConfig::new(tenant(), authenticator, session_store.clone())
        .with_session_lifetime(ChronoDuration::hours(1));
    let router = build_cli_token_login(config).router;
    let session_authenticator = SessionAuthenticator::new(session_store);

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
    let ticket = ticket_from_location(&location);

    let exchange = exchange_ticket(router, &ticket).await;
    assert_eq!(exchange.status(), StatusCode::OK);
    let body = body_string(exchange.into_body()).await;
    let payload: SessionExchangeResponse = serde_json::from_str(&body).expect("json");

    let auth =
        ironclaw_webui::WebuiAuthenticator::authenticate(&session_authenticator, &payload.token)
            .await
            .expect("exchanged bearer must authenticate");
    assert!(
        !auth.capabilities.operator_webui_config,
        "a session minted from a non-operator-capable authenticator must never come out \
         operator-capable — login_handler must carry the token's own \
         `operator_webui_config` bit through, not hardcode `true`",
    );
}

#[tokio::test]
async fn exchanged_bearer_authenticates_a_protected_route() {
    let (login_router, session_store) = build_router();
    let protected_router = build_protected_router(session_store);
    let app = login_router.merge(protected_router.clone());

    // RED first: proves the route enforces auth before a later 200 means
    // anything.
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
