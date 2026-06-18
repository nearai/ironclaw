use std::num::{NonZeroU32, NonZeroU64};

use ironclaw_host_api::NetworkMethod;
use ironclaw_host_api::ingress::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
    IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
    IngressScopeSource, ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode,
    WebSocketOriginPolicy,
};

/// Maximum accepted chat-completion request body. Sized to admit base64-inline
/// images (vision, #4644). Single source of truth for both the route
/// descriptor's ingress `body_limit` (below) and the in-workflow body check in
/// `chat_workflow.rs`, so the two can never drift apart.
pub(crate) const MAX_CHAT_BODY_BYTES: usize = 14 * 1024 * 1024;

pub const OPENAI_COMPAT_ROUTE_CHAT_COMPLETIONS: &str = "openai.compat.chat_completions";
pub const OPENAI_COMPAT_ROUTE_RESPONSES_API_CREATE: &str = "openai.compat.responses_api.create";
pub const OPENAI_COMPAT_ROUTE_RESPONSES_V1_CREATE: &str = "openai.compat.responses_v1.create";
pub const OPENAI_COMPAT_ROUTE_RESPONSES_API_RETRIEVE: &str = "openai.compat.responses_api.retrieve";
pub const OPENAI_COMPAT_ROUTE_RESPONSES_V1_RETRIEVE: &str = "openai.compat.responses_v1.retrieve";
pub const OPENAI_COMPAT_ROUTE_RESPONSES_API_CANCEL: &str = "openai.compat.responses_api.cancel";
pub const OPENAI_COMPAT_ROUTE_RESPONSES_V1_CANCEL: &str = "openai.compat.responses_v1.cancel";

pub const OPENAI_COMPAT_PATTERN_CHAT_COMPLETIONS: &str = "/v1/chat/completions";
pub const OPENAI_COMPAT_PATTERN_RESPONSES_API_CREATE: &str = "/api/v1/responses";
pub const OPENAI_COMPAT_PATTERN_RESPONSES_V1_CREATE: &str = "/v1/responses";
pub const OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM: &str = "/api/v1/responses/{response_id}";
pub const OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM: &str = "/v1/responses/{response_id}";
pub const OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM_CANCEL: &str =
    "/api/v1/responses/{response_id}/cancel";
pub const OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM_CANCEL: &str =
    "/v1/responses/{response_id}/cancel";

pub fn openai_compat_routes() -> Vec<IngressRouteDescriptor> {
    vec![
        chat_completions_descriptor(),
        responses_api_create_descriptor(),
        responses_v1_create_descriptor(),
        responses_api_retrieve_descriptor(),
        responses_v1_retrieve_descriptor(),
        responses_api_cancel_descriptor(),
        responses_v1_cancel_descriptor(),
    ]
}

fn chat_completions_descriptor() -> IngressRouteDescriptor {
    descriptor(
        OPENAI_COMPAT_ROUTE_CHAT_COMPLETIONS,
        NetworkMethod::Post,
        OPENAI_COMPAT_PATTERN_CHAT_COMPLETIONS,
        // Admits base64-inline images (vision, #4644); the per-image decoded
        // ceiling is enforced in the workflow. Both this ingress cap and the
        // in-workflow body check read the single `MAX_CHAT_BODY_BYTES` source of
        // truth so they can't drift apart.
        create_policy(body_limit_kib((MAX_CHAT_BODY_BYTES / 1024) as u64)),
    )
}

fn responses_api_create_descriptor() -> IngressRouteDescriptor {
    descriptor(
        OPENAI_COMPAT_ROUTE_RESPONSES_API_CREATE,
        NetworkMethod::Post,
        OPENAI_COMPAT_PATTERN_RESPONSES_API_CREATE,
        create_policy(body_limit_kib(1024)),
    )
}

fn responses_v1_create_descriptor() -> IngressRouteDescriptor {
    descriptor(
        OPENAI_COMPAT_ROUTE_RESPONSES_V1_CREATE,
        NetworkMethod::Post,
        OPENAI_COMPAT_PATTERN_RESPONSES_V1_CREATE,
        create_policy(body_limit_kib(1024)),
    )
}

fn responses_api_retrieve_descriptor() -> IngressRouteDescriptor {
    descriptor(
        OPENAI_COMPAT_ROUTE_RESPONSES_API_RETRIEVE,
        NetworkMethod::Get,
        OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM,
        retrieve_policy(),
    )
}

fn responses_v1_retrieve_descriptor() -> IngressRouteDescriptor {
    descriptor(
        OPENAI_COMPAT_ROUTE_RESPONSES_V1_RETRIEVE,
        NetworkMethod::Get,
        OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM,
        retrieve_policy(),
    )
}

fn responses_api_cancel_descriptor() -> IngressRouteDescriptor {
    descriptor(
        OPENAI_COMPAT_ROUTE_RESPONSES_API_CANCEL,
        NetworkMethod::Post,
        OPENAI_COMPAT_PATTERN_RESPONSES_API_ITEM_CANCEL,
        cancel_policy(),
    )
}

fn responses_v1_cancel_descriptor() -> IngressRouteDescriptor {
    descriptor(
        OPENAI_COMPAT_ROUTE_RESPONSES_V1_CANCEL,
        NetworkMethod::Post,
        OPENAI_COMPAT_PATTERN_RESPONSES_V1_ITEM_CANCEL,
        cancel_policy(),
    )
}

fn create_policy(body_limit: BodyLimitPolicy) -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit,
        rate_limit: rate_limit_per_caller(60, 60),
        cors: CorsPolicy::HostConfiguredAllowlist,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::Sse,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("OpenAI-compatible create policy must validate") // safety: crate-local constants declare LocalGateway + bearer + AuthenticatedCaller with host-mediated ProductWorkflow effects and SSE response support
}

fn retrieve_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: BodyLimitPolicy::NoBody,
        rate_limit: rate_limit_per_caller(120, 60),
        cors: CorsPolicy::HostConfiguredAllowlist,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProjectionOnly,
    })
    .expect("OpenAI-compatible retrieve policy must validate") // safety: read-only projection route uses required bearer auth and no request body
}

fn cancel_policy() -> IngressPolicy {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::LocalGateway,
        auth: bearer_required(),
        scope_source: IngressScopeSource::AuthenticatedCaller,
        body_limit: body_limit_kib(4),
        rate_limit: rate_limit_per_caller(60, 60),
        cors: CorsPolicy::HostConfiguredAllowlist,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::UserAction,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .expect("OpenAI-compatible cancel policy must validate") // safety: cancel is host-mediated through ProductWorkflow and requires authenticated caller scope
}

fn descriptor(
    route_id: &str,
    method: NetworkMethod,
    pattern: &str,
    policy: IngressPolicy,
) -> IngressRouteDescriptor {
    IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
        .expect("OpenAI-compatible route descriptor must validate") // safety: route ids and patterns are crate-local literals locked by descriptor contract tests
}

fn bearer_required() -> IngressAuthPolicy {
    IngressAuthPolicy::Required {
        schemes: vec![IngressAuthScheme::BearerToken],
    }
}

fn body_limit_kib(kib: u64) -> BodyLimitPolicy {
    let max_bytes = kib
        .checked_mul(1024)
        .and_then(NonZeroU64::new)
        .expect("OpenAI-compatible body limit must be non-zero"); // safety: all call sites pass crate-local positive constants small enough to multiply by 1024
    BodyLimitPolicy::Limited { max_bytes }
}

fn rate_limit_per_caller(max: u32, window_secs: u32) -> RateLimitPolicy {
    RateLimitPolicy::Limited {
        scope: RateLimitScope::PerCaller,
        max_requests: NonZeroU32::new(max).expect("max_requests must be non-zero"), // safety: all call sites pass crate-local positive constants
        window_seconds: NonZeroU32::new(window_secs).expect("window_seconds must be non-zero"), // safety: all call sites pass crate-local positive constants
    }
}
