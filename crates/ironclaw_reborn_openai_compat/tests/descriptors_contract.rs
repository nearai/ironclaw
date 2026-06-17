use std::collections::BTreeSet;
use std::num::{NonZeroU32, NonZeroU64};

use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressRouteDescriptor, ListenerClass, RateLimitPolicy, RateLimitScope,
    StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
use ironclaw_reborn_openai_compat::{
    OPENAI_COMPAT_ROUTE_CHAT_COMPLETIONS, OPENAI_COMPAT_ROUTE_RESPONSES_API_CANCEL,
    OPENAI_COMPAT_ROUTE_RESPONSES_API_CREATE, OPENAI_COMPAT_ROUTE_RESPONSES_API_RETRIEVE,
    OPENAI_COMPAT_ROUTE_RESPONSES_V1_CANCEL, OPENAI_COMPAT_ROUTE_RESPONSES_V1_CREATE,
    OPENAI_COMPAT_ROUTE_RESPONSES_V1_RETRIEVE, openai_compat_routes,
};

#[derive(Debug)]
struct Expected {
    route_id: &'static str,
    method: NetworkMethod,
    pattern: &'static str,
    body_limit: BodyLimitPolicy,
    rate_limit_max: u32,
    streaming: StreamingMode,
    effect_path: AllowedEffectPath,
}

fn body_limit_kib(kib: u64) -> BodyLimitPolicy {
    BodyLimitPolicy::Limited {
        max_bytes: NonZeroU64::new(kib * 1024).expect("non-zero body limit"),
    }
}

fn expected_table() -> Vec<Expected> {
    vec![
        Expected {
            route_id: OPENAI_COMPAT_ROUTE_CHAT_COMPLETIONS,
            method: NetworkMethod::Post,
            pattern: "/v1/chat/completions",
            // 14 MiB to admit base64-inline images (vision, #4644).
            body_limit: body_limit_kib(14 * 1024),
            rate_limit_max: 60,
            streaming: StreamingMode::Sse,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: OPENAI_COMPAT_ROUTE_RESPONSES_API_CREATE,
            method: NetworkMethod::Post,
            pattern: "/api/v1/responses",
            body_limit: body_limit_kib(1024),
            rate_limit_max: 60,
            streaming: StreamingMode::Sse,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: OPENAI_COMPAT_ROUTE_RESPONSES_V1_CREATE,
            method: NetworkMethod::Post,
            pattern: "/v1/responses",
            body_limit: body_limit_kib(1024),
            rate_limit_max: 60,
            streaming: StreamingMode::Sse,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: OPENAI_COMPAT_ROUTE_RESPONSES_API_RETRIEVE,
            method: NetworkMethod::Get,
            pattern: "/api/v1/responses/{response_id}",
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            streaming: StreamingMode::None,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: OPENAI_COMPAT_ROUTE_RESPONSES_V1_RETRIEVE,
            method: NetworkMethod::Get,
            pattern: "/v1/responses/{response_id}",
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            streaming: StreamingMode::None,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: OPENAI_COMPAT_ROUTE_RESPONSES_API_CANCEL,
            method: NetworkMethod::Post,
            pattern: "/api/v1/responses/{response_id}/cancel",
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            streaming: StreamingMode::None,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: OPENAI_COMPAT_ROUTE_RESPONSES_V1_CANCEL,
            method: NetworkMethod::Post,
            pattern: "/v1/responses/{response_id}/cancel",
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            streaming: StreamingMode::None,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
    ]
}

#[test]
fn route_descriptors_lock_host_owned_ingress_policy() {
    let routes = openai_compat_routes();
    assert_eq!(routes.len(), 7);

    let mut seen = BTreeSet::new();
    for expected in expected_table() {
        let descriptor = routes
            .iter()
            .find(|route| route.route_id().as_str() == expected.route_id)
            .unwrap_or_else(|| panic!("missing route {}", expected.route_id));
        assert!(seen.insert(expected.route_id));
        assert_descriptor(descriptor, &expected);
    }
}

fn assert_descriptor(descriptor: &IngressRouteDescriptor, expected: &Expected) {
    assert_eq!(descriptor.method(), expected.method);
    assert_eq!(descriptor.route_pattern().as_str(), expected.pattern);

    let policy = descriptor.policy();
    assert_eq!(policy.listener_class(), ListenerClass::LocalGateway);
    assert_eq!(
        policy.scope_source(),
        IngressScopeSource::AuthenticatedCaller
    );
    assert_eq!(policy.body_limit(), expected.body_limit);
    assert_eq!(policy.cors(), CorsPolicy::HostConfiguredAllowlist);
    assert_eq!(
        policy.websocket_origin(),
        WebSocketOriginPolicy::NotApplicable
    );
    assert_eq!(policy.streaming(), expected.streaming);
    assert_eq!(policy.audit(), AuditTraceClass::UserAction);
    assert_eq!(policy.effect_path(), &expected.effect_path);

    match policy.auth() {
        IngressAuthPolicy::Required { schemes } => {
            assert_eq!(schemes, &[IngressAuthScheme::BearerToken]);
        }
        IngressAuthPolicy::Public { .. } => panic!("OpenAI-compatible API must require auth"),
    }

    match policy.rate_limit() {
        RateLimitPolicy::Limited {
            scope,
            max_requests,
            window_seconds,
        } => {
            assert_eq!(*scope, RateLimitScope::PerCaller);
            assert_eq!(
                *max_requests,
                NonZeroU32::new(expected.rate_limit_max).expect("non-zero")
            );
            assert_eq!(*window_seconds, NonZeroU32::new(60).expect("non-zero"));
        }
        RateLimitPolicy::Disabled { .. } => {
            panic!("OpenAI-compatible API must keep route rate limits enabled");
        }
    }
}

#[test]
fn descriptors_reject_unknown_fields_through_host_api_contract() {
    let descriptor = openai_compat_routes()
        .into_iter()
        .next()
        .expect("descriptor");
    let mut value = serde_json::to_value(descriptor).expect("serialize descriptor");
    value
        .as_object_mut()
        .expect("descriptor object")
        .insert("surprise".to_string(), serde_json::json!(true));

    let err = serde_json::from_value::<IngressRouteDescriptor>(value)
        .expect_err("unknown descriptor fields must reject");
    assert!(err.to_string().contains("unknown field"));
}
