//! Public RFC 7591 Client ID Metadata Document (CIMD) route.

use axum::{
    Json,
    extract::{Path, State},
};
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressJustification, IngressPolicy, IngressPolicyParts, ListenerClass, RateLimitPolicy,
    RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};

use super::{
    OAUTH_CALLBACK_MAX_REQUESTS, OAUTH_RATE_WINDOW_SECONDS, ProductAuthRouteFailure,
    ProductAuthRouteState,
};

pub(super) const PATH: &str = "/api/reborn/product-auth/oauth/{provider}/client-metadata.json";
pub(super) const ROUTE_ID: &str = "product_auth.oauth.client_metadata";

pub(super) async fn handler(
    State(state): State<ProductAuthRouteState>,
    Path(provider): Path<String>,
) -> Result<Json<ironclaw_auth::OAuthClientMetadataDocument>, ProductAuthRouteFailure> {
    state
        .auth_engine()?
        .client_metadata_document(&provider)
        .map(Json)
        .map_err(ProductAuthRouteFailure::from)
}

pub(super) fn policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: IngressAuthPolicy::Public {
            justification: IngressJustification::new(
                "oauth client metadata",
                "Authorization servers must fetch the configured secret-free client metadata document before a user can authenticate",
            )
            .expect("client metadata justification must validate"), // safety: fixed non-empty protocol justification.
        },
        scope_source: ironclaw_host_api::ingress::IngressScopeSource::PublicRoute,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::PerIp,
            max_requests: OAUTH_CALLBACK_MAX_REQUESTS,
            window_seconds: OAUTH_RATE_WINDOW_SECONDS,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::NoEffect,
    })
    .expect("client metadata policy must validate") // safety: public, no-body, no-effect metadata read with a bounded per-IP rate limit.
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
    };
    use ironclaw_host_api::{
        ids::TenantId,
        ingress::{AllowedEffectPath, IngressAuthPolicy},
    };
    use ironclaw_secrets::SecretStore;
    use tower::ServiceExt as _;

    use super::super::{
        ProductAuthRouteState, product_auth_route_descriptors, product_auth_route_mount,
        tests::{
            NoopDispatcher, PanickingDcrEgress, test_engine_with_resolver_and_callback_base,
            test_vendor_recipe,
        },
    };

    #[tokio::test]
    async fn client_metadata_document_is_public_and_self_consistent() {
        let recipe = test_vendor_recipe(true, None);
        let engine = test_engine_with_resolver_and_callback_base(
            Arc::new(ironclaw_auth::StaticAuthRecipeResolver::new(vec![recipe])),
            Arc::new(PanickingDcrEgress),
            Arc::new(SecretStore::ephemeral()),
            "https://ironclaw.example/api/reborn/product-auth/oauth",
        );
        let product_auth =
            ironclaw_auth::RebornProductAuthServices::in_memory_for_test(Arc::new(NoopDispatcher))
                .with_auth_engine(engine);
        let state = ProductAuthRouteState::new(
            Arc::new(product_auth),
            TenantId::new("tenant-alpha").expect("tenant"),
            None,
            None,
        );
        let mount = product_auth_route_mount(state);

        let response = mount
            .public
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/reborn/product-auth/oauth/vendorco/client-metadata.json")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("metadata JSON");
        assert_eq!(
            document["client_id"],
            "https://ironclaw.example/api/reborn/product-auth/oauth/vendorco/client-metadata.json"
        );
        assert_eq!(document["client_name"], "Ironclaw");
        assert_eq!(
            document["redirect_uris"],
            serde_json::json!([
                "https://ironclaw.example/api/reborn/product-auth/oauth/vendorco/callback"
            ])
        );
        assert_eq!(
            document["grant_types"],
            serde_json::json!(["authorization_code", "refresh_token"])
        );
        assert_eq!(document["response_types"], serde_json::json!(["code"]));
        assert_eq!(document["token_endpoint_auth_method"], "none");
        assert_eq!(document.as_object().expect("object").len(), 6);

        let descriptor = product_auth_route_descriptors()
            .into_iter()
            .find(|descriptor| descriptor.route_pattern().as_str() == super::PATH)
            .expect("client metadata route descriptor");
        assert!(matches!(
            descriptor.policy().auth(),
            IngressAuthPolicy::Public { .. }
        ));
        assert_eq!(
            descriptor.policy().scope_source(),
            ironclaw_host_api::ingress::IngressScopeSource::PublicRoute
        );
        assert_eq!(
            descriptor.policy().effect_path(),
            &AllowedEffectPath::NoEffect
        );
    }
}
