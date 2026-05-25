//! Host-owned route descriptors for the Reborn WebChat v2 surface.
//!
//! Host composition consumes [`webui_v2_routes`] and mounts the matching
//! handler from [`crate::handlers`] under each descriptor's pattern. The
//! descriptor is the contract: changing a route's policy here changes what
//! host composition enforces before the handler runs.

use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass,
    RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
};
use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
use std::num::{NonZeroU32, NonZeroU64};

pub const WEBUI_V2_ROUTE_CREATE_THREAD: &str = "webui.v2.create_thread";
pub const WEBUI_V2_ROUTE_SEND_MESSAGE: &str = "webui.v2.send_message";
pub const WEBUI_V2_ROUTE_LIST_THREADS: &str = "webui.v2.list_threads";
pub const WEBUI_V2_ROUTE_GET_TIMELINE: &str = "webui.v2.get_timeline";
pub const WEBUI_V2_ROUTE_STREAM_EVENTS: &str = "webui.v2.stream_events";
pub const WEBUI_V2_ROUTE_STREAM_EVENTS_WS: &str = "webui.v2.stream_events_ws";
pub const WEBUI_V2_ROUTE_CANCEL_RUN: &str = "webui.v2.cancel_run";
pub const WEBUI_V2_ROUTE_RESOLVE_GATE: &str = "webui.v2.resolve_gate";
pub const WEBUI_V2_ROUTE_SETUP_EXTENSION: &str = "webui.v2.setup_extension";

pub const WEBUI_V2_PATTERN_CREATE_THREAD: &str = "/api/webchat/v2/threads";
pub const WEBUI_V2_PATTERN_LIST_THREADS: &str = "/api/webchat/v2/threads";
pub const WEBUI_V2_PATTERN_SEND_MESSAGE: &str = "/api/webchat/v2/threads/{thread_id}/messages";
pub const WEBUI_V2_PATTERN_GET_TIMELINE: &str = "/api/webchat/v2/threads/{thread_id}/timeline";
pub const WEBUI_V2_PATTERN_STREAM_EVENTS: &str = "/api/webchat/v2/threads/{thread_id}/events";
pub const WEBUI_V2_PATTERN_STREAM_EVENTS_WS: &str = "/api/webchat/v2/threads/{thread_id}/ws";
pub const WEBUI_V2_PATTERN_CANCEL_RUN: &str =
    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel";
pub const WEBUI_V2_PATTERN_RESOLVE_GATE: &str =
    "/api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve";
pub const WEBUI_V2_PATTERN_SETUP_EXTENSION: &str =
    "/api/webchat/v2/extensions/{extension_name}/setup";

/// Return the canonical [`IngressRouteDescriptor`] set for the WebChat v2
/// beta route surface.
///
/// Host composition calls this once at startup, validates the descriptors
/// against its own mount table, and refuses to bind any route whose policy
/// the host cannot enforce.
pub fn webui_v2_routes() -> Vec<IngressRouteDescriptor> {
    vec![
        create_thread_descriptor(),
        send_message_descriptor(),
        list_threads_descriptor(),
        get_timeline_descriptor(),
        stream_events_descriptor(),
        stream_events_ws_descriptor(),
        cancel_run_descriptor(),
        resolve_gate_descriptor(),
        setup_extension_descriptor(),
    ]
}

fn create_thread_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_CREATE_THREAD,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_CREATE_THREAD,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn send_message_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SEND_MESSAGE,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_SEND_MESSAGE,
        mutation_policy(
            // Message bodies carry user content. 1 MiB is the same cap the
            // existing turn admission layer enforces.
            body_limit_kib(1024),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::TurnCoordinator,
        ),
    )
}

fn get_timeline_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_GET_TIMELINE,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_GET_TIMELINE,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn stream_events_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_STREAM_EVENTS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_STREAM_EVENTS,
        read_policy(
            stream_rate_limit(),
            AuditTraceClass::StreamingSubscription,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::Sse,
        ),
    )
}

fn cancel_run_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_CANCEL_RUN,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_CANCEL_RUN,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::TurnCoordinator,
        ),
    )
}

fn resolve_gate_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_RESOLVE_GATE,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_RESOLVE_GATE,
        mutation_policy(
            body_limit_kib(4),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::TurnCoordinator,
        ),
    )
}

fn list_threads_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_LIST_THREADS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_LIST_THREADS,
        read_policy(
            read_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProjectionOnly,
            StreamingMode::None,
        ),
    )
}

fn stream_events_ws_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_STREAM_EVENTS_WS,
        NetworkMethod::Get,
        WEBUI_V2_PATTERN_STREAM_EVENTS_WS,
        ws_read_policy(
            stream_rate_limit(),
            AuditTraceClass::StreamingSubscription,
            AllowedEffectPath::ProjectionOnly,
        ),
    )
}

fn setup_extension_descriptor() -> IngressRouteDescriptor {
    descriptor(
        WEBUI_V2_ROUTE_SETUP_EXTENSION,
        NetworkMethod::Post,
        WEBUI_V2_PATTERN_SETUP_EXTENSION,
        mutation_policy(
            body_limit_kib(16),
            mutation_rate_limit(),
            AuditTraceClass::UserAction,
            AllowedEffectPath::ProductWorkflow,
        ),
    )
}

fn ws_read_policy(
    rate_limit: RateLimitPolicy,
    audit: AuditTraceClass,
    effect_path: AllowedEffectPath,
) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit,
        cors: CorsPolicy::SameOriginOnly,
        // WS upgrade is gated by host composition's same-origin
        // check; declared here so the descriptor is the contract a
        // future allowlist-based deployment overrides.
        websocket_origin: WebSocketOriginPolicy::SameOriginRequired,
        streaming: StreamingMode::WebSocket,
        audit,
        effect_path,
    })
    .expect("webui v2 WS read policy must validate") // safety: combination LocalGateway + bearer + AuthenticatedCaller + WebSocket + SameOriginRequired is a permitted shape; other parts are crate-local constants
}

fn descriptor(
    route_id: &str,
    method: NetworkMethod,
    pattern: &str,
    policy: IngressPolicy,
) -> IngressRouteDescriptor {
    IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
        .expect("webui v2 route descriptor must validate at startup") // safety: route_id/pattern are crate-local literals known to satisfy IngressRouteId / IngressRoutePattern; policy is constructed by sibling helpers that validate their own inputs
}

fn mutation_policy(
    body_limit: BodyLimitPolicy,
    rate_limit: RateLimitPolicy,
    audit: AuditTraceClass,
    effect_path: AllowedEffectPath,
) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit,
        rate_limit,
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit,
        effect_path,
    })
    .expect("webui v2 mutation policy must validate") // safety: all parts are crate-local constants; the combination (LocalGateway + bearer required + AuthenticatedCaller + None streaming) is a permitted shape, locked in by the descriptor contract test
}

fn read_policy(
    rate_limit: RateLimitPolicy,
    audit: AuditTraceClass,
    effect_path: AllowedEffectPath,
    streaming: StreamingMode,
) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit,
        cors: CorsPolicy::SameOriginOnly,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming,
        audit,
        effect_path,
    })
    .expect("webui v2 read policy must validate") // safety: streaming is either None or Sse (both permitted with bearer + AuthenticatedCaller); other parts are crate-local constants
}

fn bearer_required() -> IngressAuthPolicy {
    IngressAuthPolicy::Required {
        schemes: vec![IngressAuthScheme::BearerToken],
    }
}

fn body_limit_kib(kib: u64) -> BodyLimitPolicy {
    let bytes = kib
        .checked_mul(1024)
        .and_then(NonZeroU64::new)
        .expect("body limit must be non-zero"); // safety: all call sites pass crate-local positive constants (4, 16, 1024); overflow at u64 * 1024 is impossible for these
    BodyLimitPolicy::Limited { max_bytes: bytes }
}

fn mutation_rate_limit() -> RateLimitPolicy {
    rate_limit_per_caller(60, 60)
}

fn read_rate_limit() -> RateLimitPolicy {
    rate_limit_per_caller(120, 60)
}

fn stream_rate_limit() -> RateLimitPolicy {
    // SSE sessions are long-lived; the per-tenant/user concurrency
    // cap (3 streams, enforced in `WebUiV2State::SseCapacity`) does
    // the real bounding. The request-rate window here is just for
    // burst protection against reconnect storms — bumped to 60/60s
    // (parity with the mutation budget) so a browser doing
    // exponential-backoff reconnects after a transient outage does
    // not lock itself out before the concurrency cap recycles.
    rate_limit_per_caller(60, 60)
}

fn rate_limit_per_caller(max: u32, window_secs: u32) -> RateLimitPolicy {
    RateLimitPolicy::Limited {
        scope: RateLimitScope::PerCaller,
        max_requests: NonZeroU32::new(max).expect("max_requests must be non-zero"), // safety: all call sites pass crate-local positive constants (12, 60, 120)
        window_seconds: NonZeroU32::new(window_secs).expect("window_seconds must be non-zero"), // safety: all call sites pass crate-local positive constants (60)
    }
}
