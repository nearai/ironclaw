//! Sanity contract for [`webui_v2_routes`].
//!
//! Locks the route table to:
//! - exactly six routes
//! - method + pattern fixed (host composition relies on these to mount)
//! - bearer-token auth required on every route
//! - per-caller rate limiting on every route
//! - SSE streaming exposed only on `stream_events`
//! - no route exposes a public scope source

use ironclaw_host_api::ingress::{
    IngressAuthPolicy, IngressAuthScheme, IngressRouteDescriptor, RateLimitPolicy, StreamingMode,
};
use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
use ironclaw_webui_v2::{
    WEBUI_V2_ROUTE_CANCEL_RUN, WEBUI_V2_ROUTE_CREATE_THREAD, WEBUI_V2_ROUTE_GET_TIMELINE,
    WEBUI_V2_ROUTE_RESOLVE_GATE, WEBUI_V2_ROUTE_SEND_MESSAGE, WEBUI_V2_ROUTE_STREAM_EVENTS,
    webui_v2_routes,
};

#[test]
fn route_table_has_exactly_six_routes() {
    let routes = webui_v2_routes();
    assert_eq!(routes.len(), 6, "expected exactly six WebChat v2 routes");

    let ids: Vec<&str> = routes
        .iter()
        .map(|d| d.route_id().as_str())
        .collect::<Vec<_>>();
    for expected in [
        WEBUI_V2_ROUTE_CREATE_THREAD,
        WEBUI_V2_ROUTE_SEND_MESSAGE,
        WEBUI_V2_ROUTE_GET_TIMELINE,
        WEBUI_V2_ROUTE_STREAM_EVENTS,
        WEBUI_V2_ROUTE_CANCEL_RUN,
        WEBUI_V2_ROUTE_RESOLVE_GATE,
    ] {
        assert!(
            ids.contains(&expected),
            "missing route {expected:?} from {ids:?}"
        );
    }
}

#[test]
fn every_route_requires_bearer_auth_and_authenticated_caller_scope() {
    for route in webui_v2_routes() {
        match route.policy().auth() {
            IngressAuthPolicy::Required { schemes } => {
                assert!(
                    schemes.contains(&IngressAuthScheme::BearerToken),
                    "route {} must require BearerToken auth",
                    route.route_id()
                );
            }
            IngressAuthPolicy::Public { .. } => panic!(
                "route {} must not be public; WebChat v2 is authenticated-only",
                route.route_id()
            ),
        }
        assert_eq!(
            route.policy().scope_source(),
            IngressScopeSource::AuthenticatedCaller,
            "route {} must source scope from the authenticated caller",
            route.route_id()
        );
    }
}

#[test]
fn every_route_carries_a_per_caller_rate_limit() {
    for route in webui_v2_routes() {
        match route.policy().rate_limit() {
            RateLimitPolicy::Limited { scope, .. } => {
                use ironclaw_host_api::ingress::RateLimitScope;
                assert_eq!(
                    *scope,
                    RateLimitScope::PerCaller,
                    "route {} must rate-limit per caller",
                    route.route_id()
                );
            }
            RateLimitPolicy::Disabled { .. } => {
                panic!("route {} must not disable rate limiting", route.route_id())
            }
        }
    }
}

#[test]
fn only_stream_events_uses_sse() {
    let by_id = route_lookup();
    assert_eq!(
        by_id[WEBUI_V2_ROUTE_STREAM_EVENTS].policy().streaming(),
        StreamingMode::Sse
    );
    for non_streaming in [
        WEBUI_V2_ROUTE_CREATE_THREAD,
        WEBUI_V2_ROUTE_SEND_MESSAGE,
        WEBUI_V2_ROUTE_GET_TIMELINE,
        WEBUI_V2_ROUTE_CANCEL_RUN,
        WEBUI_V2_ROUTE_RESOLVE_GATE,
    ] {
        assert_eq!(
            by_id[non_streaming].policy().streaming(),
            StreamingMode::None,
            "non-streaming route {non_streaming} must not declare a streaming mode"
        );
    }
}

#[test]
fn mutations_are_post_reads_are_get() {
    let by_id = route_lookup();
    for (id, method) in [
        (WEBUI_V2_ROUTE_CREATE_THREAD, NetworkMethod::Post),
        (WEBUI_V2_ROUTE_SEND_MESSAGE, NetworkMethod::Post),
        (WEBUI_V2_ROUTE_GET_TIMELINE, NetworkMethod::Get),
        (WEBUI_V2_ROUTE_STREAM_EVENTS, NetworkMethod::Get),
        (WEBUI_V2_ROUTE_CANCEL_RUN, NetworkMethod::Post),
        (WEBUI_V2_ROUTE_RESOLVE_GATE, NetworkMethod::Post),
    ] {
        assert_eq!(by_id[id].method(), method, "route {id} must use {method:?}");
    }
}

fn route_lookup() -> std::collections::HashMap<String, IngressRouteDescriptor> {
    webui_v2_routes()
        .into_iter()
        .map(|d| (d.route_id().as_str().to_string(), d))
        .collect()
}
