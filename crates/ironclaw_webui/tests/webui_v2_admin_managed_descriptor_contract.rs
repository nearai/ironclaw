use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthScheme,
    IngressScopeSource, ListenerClass, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_webui::webui_v2::{WEBUI_V2_ROUTE_ADMIN_CREATE_MANAGED_USER, webui_v2_routes};
use std::num::NonZeroU64;

#[test]
fn managed_user_creation_route_is_admin_product_workflow_mutation() {
    let route = webui_v2_routes()
        .into_iter()
        .find(|route| route.route_id().as_str() == WEBUI_V2_ROUTE_ADMIN_CREATE_MANAGED_USER)
        .expect("managed-user route");

    assert_eq!(route.method(), NetworkMethod::Post);
    assert_eq!(
        route.route_pattern().as_str(),
        "/api/webchat/v2/admin/agents"
    );
    let policy = route.policy();
    assert_eq!(policy.listener_class(), ListenerClass::LocalGateway);
    assert!(matches!(
        policy.auth(),
        ironclaw_host_api::ingress::IngressAuthPolicy::Required { schemes }
            if schemes == &vec![IngressAuthScheme::BearerToken]
    ));
    assert_eq!(
        policy.scope_source(),
        IngressScopeSource::AuthenticatedCaller
    );
    assert_eq!(
        policy.body_limit(),
        BodyLimitPolicy::Limited {
            max_bytes: NonZeroU64::new(16 * 1024).expect("non-zero body limit")
        }
    );
    assert!(matches!(
        policy.rate_limit(),
        ironclaw_host_api::ingress::RateLimitPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests,
            window_seconds,
        } if max_requests.get() == 60 && window_seconds.get() == 60
    ));
    assert_eq!(policy.cors(), CorsPolicy::SameOriginOnly);
    assert_eq!(
        policy.websocket_origin(),
        WebSocketOriginPolicy::NotApplicable
    );
    assert_eq!(policy.streaming(), StreamingMode::None);
    assert_eq!(policy.audit(), AuditTraceClass::UserAction);
    assert_eq!(policy.effect_path(), &AllowedEffectPath::ProductWorkflow);
}
