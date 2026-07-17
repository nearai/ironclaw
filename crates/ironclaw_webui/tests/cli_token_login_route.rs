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
use axum::http::{Request, StatusCode, header};
use chrono::Duration as ChronoDuration;
use http_body_util::BodyExt;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_webui::{
    CliTokenLoginConfig, EnvBearerAuthenticator, SessionStore, build_cli_token_login,
    signed_session_store,
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
