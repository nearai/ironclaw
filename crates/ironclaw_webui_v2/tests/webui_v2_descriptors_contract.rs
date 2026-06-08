//! Sanity contract for [`webui_v2_routes`].
//!
//! Locks the full host-owned ingress policy surface per route. Host
//! composition relies on these descriptors to mount the router and apply
//! its auth / CORS / body-limit / rate-limit / audit middleware; any drift
//! in method, pattern, listener class, auth scheme, scope source, body
//! limit, rate limit max/window/scope, CORS, websocket origin, streaming
//! mode, audit class, or allowed effect path is a behavior change the
//! host cannot enforce silently.

use std::collections::HashMap;
use std::num::{NonZeroU32, NonZeroU64};

use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressRouteDescriptor, ListenerClass, RateLimitPolicy, RateLimitScope,
    StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
use ironclaw_webui_v2::{
    WEBUI_V2_ROUTE_ACTIVATE_EXTENSION, WEBUI_V2_ROUTE_CANCEL_RUN,
    WEBUI_V2_ROUTE_COMPLETE_NEARAI_WALLET_LOGIN, WEBUI_V2_ROUTE_CREATE_THREAD,
    WEBUI_V2_ROUTE_DELETE_LLM_PROVIDER, WEBUI_V2_ROUTE_DELETE_THREAD,
    WEBUI_V2_ROUTE_GET_EXTENSION_SETUP, WEBUI_V2_ROUTE_GET_LLM_CONFIG, WEBUI_V2_ROUTE_GET_SESSION,
    WEBUI_V2_ROUTE_GET_TIMELINE, WEBUI_V2_ROUTE_INSTALL_EXTENSION, WEBUI_V2_ROUTE_LIST_AUTOMATIONS,
    WEBUI_V2_ROUTE_LIST_CONNECTABLE_CHANNELS, WEBUI_V2_ROUTE_LIST_EXTENSION_REGISTRY,
    WEBUI_V2_ROUTE_LIST_EXTENSIONS, WEBUI_V2_ROUTE_LIST_LLM_MODELS, WEBUI_V2_ROUTE_LIST_THREADS,
    WEBUI_V2_ROUTE_REMOVE_EXTENSION, WEBUI_V2_ROUTE_RESOLVE_GATE, WEBUI_V2_ROUTE_SEND_MESSAGE,
    WEBUI_V2_ROUTE_SET_ACTIVE_LLM, WEBUI_V2_ROUTE_SETUP_EXTENSION,
    WEBUI_V2_ROUTE_START_CODEX_LOGIN, WEBUI_V2_ROUTE_START_NEARAI_LOGIN,
    WEBUI_V2_ROUTE_STREAM_EVENTS, WEBUI_V2_ROUTE_STREAM_EVENTS_WS,
    WEBUI_V2_ROUTE_TEST_LLM_CONNECTION, WEBUI_V2_ROUTE_UPSERT_LLM_PROVIDER, webui_v2_routes,
};

/// Expected policy surface for one route. Everything host composition
/// reads off the descriptor lands here so the test fails the moment any
/// of those fields drift from this table.
struct Expected {
    route_id: &'static str,
    method: NetworkMethod,
    pattern: &'static str,
    listener_class: ListenerClass,
    auth_schemes: &'static [IngressAuthScheme],
    scope_source: IngressScopeSource,
    body_limit: BodyLimitPolicy,
    rate_limit_max: u32,
    rate_limit_window_seconds: u32,
    rate_limit_scope: RateLimitScope,
    cors: CorsPolicy,
    websocket_origin: WebSocketOriginPolicy,
    streaming: StreamingMode,
    audit: AuditTraceClass,
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
            route_id: WEBUI_V2_ROUTE_GET_SESSION,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/session",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_CREATE_THREAD,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/threads",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(16),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_SEND_MESSAGE,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/threads/{thread_id}/messages",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(1024),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::TurnCoordinator,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_DELETE_THREAD,
            method: NetworkMethod::Delete,
            pattern: "/api/webchat/v2/threads/{thread_id}",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_GET_TIMELINE,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/threads/{thread_id}/timeline",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_STREAM_EVENTS,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/threads/{thread_id}/events",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 30,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::Sse,
            audit: AuditTraceClass::StreamingSubscription,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_CANCEL_RUN,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::TurnCoordinator,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_RESOLVE_GATE,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::TurnCoordinator,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_LIST_THREADS,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/threads",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_STREAM_EVENTS_WS,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/threads/{thread_id}/ws",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 30,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::SameOriginRequired,
            streaming: StreamingMode::WebSocket,
            audit: AuditTraceClass::StreamingSubscription,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_LIST_AUTOMATIONS,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/automations",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_LIST_CONNECTABLE_CHANNELS,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/channels/connectable",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_LIST_EXTENSIONS,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/extensions",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_LIST_EXTENSION_REGISTRY,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/extensions/registry",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_INSTALL_EXTENSION,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/extensions/install",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(16),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_ACTIVATE_EXTENSION,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/extensions/{package_id}/activate",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_REMOVE_EXTENSION,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/extensions/{package_id}/remove",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_GET_EXTENSION_SETUP,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/extensions/{package_id}/setup",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_SETUP_EXTENSION,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/extensions/{package_id}/setup",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(16),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_GET_LLM_CONFIG,
            method: NetworkMethod::Get,
            pattern: "/api/webchat/v2/llm/providers",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit_max: 120,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_UPSERT_LLM_PROVIDER,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/llm/providers",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(16),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_DELETE_LLM_PROVIDER,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/llm/providers/{provider_id}/delete",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_SET_ACTIVE_LLM,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/llm/active",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_TEST_LLM_CONNECTION,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/llm/test-connection",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(16),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_LIST_LLM_MODELS,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/llm/list-models",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(16),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_START_NEARAI_LOGIN,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/llm/nearai/login",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_COMPLETE_NEARAI_WALLET_LOGIN,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/llm/nearai/wallet",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
        Expected {
            route_id: WEBUI_V2_ROUTE_START_CODEX_LOGIN,
            method: NetworkMethod::Post,
            pattern: "/api/webchat/v2/llm/codex/login",
            listener_class: ListenerClass::LocalGateway,
            auth_schemes: &[IngressAuthScheme::BearerToken],
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: body_limit_kib(4),
            rate_limit_max: 60,
            rate_limit_window_seconds: 60,
            rate_limit_scope: RateLimitScope::PerCaller,
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        },
    ]
}

fn route_lookup() -> HashMap<String, IngressRouteDescriptor> {
    webui_v2_routes()
        .into_iter()
        .map(|d| (d.route_id().as_str().to_string(), d))
        .collect()
}

#[test]
fn route_table_has_exactly_the_expected_routes() {
    let routes = webui_v2_routes();
    let expected = expected_table();
    assert_eq!(
        routes.len(),
        expected.len(),
        "expected {} WebChat v2 routes, found {}",
        expected.len(),
        routes.len()
    );

    let actual_ids: Vec<String> = routes
        .iter()
        .map(|d| d.route_id().as_str().to_string())
        .collect();
    for row in &expected {
        assert!(
            actual_ids.iter().any(|id| id == row.route_id),
            "expected route {} missing from {:?}",
            row.route_id,
            actual_ids
        );
    }
}

#[test]
fn every_descriptor_matches_the_locked_policy_surface() {
    let actual = route_lookup();
    for row in expected_table() {
        let route = actual
            .get(row.route_id)
            .unwrap_or_else(|| panic!("route {} missing from descriptor table", row.route_id));
        let policy = route.policy();

        assert_eq!(route.method(), row.method, "route {}: method", row.route_id);
        assert_eq!(
            route.route_pattern().as_str(),
            row.pattern,
            "route {}: pattern",
            row.route_id
        );
        assert_eq!(
            policy.listener_class(),
            row.listener_class,
            "route {}: listener class",
            row.route_id
        );
        match policy.auth() {
            IngressAuthPolicy::Required { schemes } => {
                let expected = row.auth_schemes.to_vec();
                assert_eq!(
                    schemes.clone(),
                    expected,
                    "route {}: auth schemes",
                    row.route_id
                );
            }
            IngressAuthPolicy::Public { .. } => panic!(
                "route {} must require bearer auth; descriptor is Public",
                row.route_id
            ),
        }
        assert_eq!(
            policy.scope_source(),
            row.scope_source,
            "route {}: scope source",
            row.route_id
        );
        assert_eq!(
            policy.body_limit(),
            row.body_limit,
            "route {}: body limit",
            row.route_id
        );
        match policy.rate_limit() {
            RateLimitPolicy::Limited {
                scope,
                max_requests,
                window_seconds,
            } => {
                assert_eq!(
                    *scope, row.rate_limit_scope,
                    "route {}: rate scope",
                    row.route_id
                );
                assert_eq!(
                    *max_requests,
                    NonZeroU32::new(row.rate_limit_max).expect("non-zero max"),
                    "route {}: rate max_requests",
                    row.route_id
                );
                assert_eq!(
                    *window_seconds,
                    NonZeroU32::new(row.rate_limit_window_seconds).expect("non-zero window"),
                    "route {}: rate window_seconds",
                    row.route_id
                );
            }
            RateLimitPolicy::Disabled { .. } => {
                panic!("route {}: rate limit must not be Disabled", row.route_id)
            }
        }
        assert_eq!(policy.cors(), row.cors, "route {}: CORS", row.route_id);
        assert_eq!(
            policy.websocket_origin(),
            row.websocket_origin,
            "route {}: websocket origin policy",
            row.route_id
        );
        assert_eq!(
            policy.streaming(),
            row.streaming,
            "route {}: streaming mode",
            row.route_id
        );
        assert_eq!(
            policy.audit(),
            row.audit,
            "route {}: audit class",
            row.route_id
        );
        assert_eq!(
            policy.effect_path(),
            &row.effect_path,
            "route {}: effect path",
            row.route_id
        );
    }
}
