//! Caller-level contract for the OpenAI-compatible `GET /v1/models` route.
//!
//! Ports the v1 OpenAI-compatible proxy's `test_models_endpoint` and
//! `test_models_no_auth` behaviors onto the reborn route + host-injected
//! catalog port, and locks the fail-closed (`501`) shape when no catalog is
//! wired.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use http::Request;
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product::{AuthRequirement, ProtocolAuthEvidence};
use ironclaw_reborn_openai_compat::{
    OpenAiCompatActorScope, OpenAiCompatAuthenticatedCaller, OpenAiCompatHttpError,
    OpenAiCompatModelCatalog, OpenAiCompatModelEntry, OpenAiCompatRouterState,
    openai_compat_router, openai_compat_router_with_state,
};
use serde_json::Value;
use tower::ServiceExt;

struct StaticModelCatalog {
    entries: Vec<OpenAiCompatModelEntry>,
}

#[async_trait]
impl OpenAiCompatModelCatalog for StaticModelCatalog {
    async fn list_models(
        &self,
        _caller: &OpenAiCompatAuthenticatedCaller,
    ) -> Result<Vec<OpenAiCompatModelEntry>, OpenAiCompatHttpError> {
        Ok(self.entries.clone())
    }
}

#[tokio::test]
async fn models_endpoint_returns_openai_list_for_authenticated_caller() {
    for path in ["/v1/models", "/api/v1/models"] {
        let catalog = Arc::new(StaticModelCatalog {
            entries: vec![
                OpenAiCompatModelEntry::new("gpt-reborn"),
                OpenAiCompatModelEntry::new("claude").with_owner("anthropic"),
            ],
        });
        let router = openai_compat_router_with_state(OpenAiCompatRouterState::with_models(catalog))
            .layer(axum::Extension(caller()));

        let response = router.oneshot(get_request(path)).await.expect("response");

        assert_eq!(response.status(), http::StatusCode::OK, "{path}");
        let body = json_body(response).await;
        assert_eq!(body["object"], "list", "{path}");
        assert_eq!(body["data"].as_array().expect("data").len(), 2, "{path}");
        assert_eq!(body["data"][0]["id"], "gpt-reborn", "{path}");
        assert_eq!(body["data"][0]["object"], "model", "{path}");
        assert_eq!(body["data"][0]["owned_by"], "ironclaw", "{path}");
        assert_eq!(body["data"][1]["owned_by"], "anthropic", "{path}");
    }
}

#[tokio::test]
async fn models_endpoint_without_caller_returns_401_before_catalog() {
    let catalog = Arc::new(StaticModelCatalog {
        entries: vec![OpenAiCompatModelEntry::new("gpt-reborn")],
    });

    for path in ["/v1/models", "/api/v1/models"] {
        let router =
            openai_compat_router_with_state(OpenAiCompatRouterState::with_models(catalog.clone()));

        let response = router.oneshot(get_request(path)).await.expect("response");

        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED, "{path}");
        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], "authentication_required", "{path}");
    }
}

#[tokio::test]
async fn models_endpoint_without_catalog_fails_closed_501() {
    for path in ["/v1/models", "/api/v1/models"] {
        let router = openai_compat_router().layer(axum::Extension(caller()));

        let response = router.oneshot(get_request(path)).await.expect("response");

        assert_eq!(
            response.status(),
            http::StatusCode::NOT_IMPLEMENTED,
            "{path}"
        );
        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], "unsupported", "{path}");
    }
}

fn get_request(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("request")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}

fn caller() -> OpenAiCompatAuthenticatedCaller {
    OpenAiCompatAuthenticatedCaller::new(
        OpenAiCompatActorScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new("user-a").expect("user"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
        ),
        ProtocolAuthEvidence::test_verified_for_tenant(
            AuthRequirement::BearerToken,
            "user-a",
            TenantId::new("tenant-a").expect("tenant"),
        ),
    )
    .expect("caller")
}
