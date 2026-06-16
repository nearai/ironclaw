//! Descriptor-driven per-route body limit for the WebChat v2 native
//! surface.
//!
//! `ironclaw_webui_v2::webui_v2_routes()` carries a [`BodyLimitPolicy`]
//! per route (16 KiB for `create_thread`, 14 MiB for `send_message`, 4
//! KiB for `cancel_run`/`resolve_gate`, `NoBody` for the read /
//! streaming routes). The v2 crate's CLAUDE.md designates enforcement
//! as host-composition responsibility; this module is that enforcement.
//!
//! Wiring:
//!
//! - This middleware runs **before** auth so an oversized payload is
//!   rejected without spending a bearer-validation step.
//! - It runs **after** the outer `RequestBodyLimitLayer` global cap
//!   that [`crate::webui_serve::webui_v2_app`] keeps as a defense in
//!   depth for paths that don't match any v2 descriptor (e.g. axum's
//!   404 fallback). Per-route enforcement is strictly tighter than that
//!   global cap.
//!
//! `BodyLimitPolicy` has only two variants today (`NoBody` and
//! `Limited { max_bytes }`); both are supported. The `match` is
//! exhaustive on a host-api enum, so a new variant added upstream would
//! fail to compile rather than silently disable enforcement.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ironclaw_host_api::ingress::{BodyLimitPolicy, IngressRouteDescriptor};

use crate::webui_route_match::{network_method_to_axum, parse_pattern, segments_match};

#[derive(Debug, Clone)]
struct RouteBodyLimit {
    route_id: String,
    method: Method,
    segments: Vec<Option<String>>,
    policy: ResolvedBodyPolicy,
}

#[derive(Debug, Clone, Copy)]
enum ResolvedBodyPolicy {
    NoBody,
    Limited { max_bytes: u64 },
}

/// Shared state for [`enforce_body_limit`]. Cheap to clone — the inner
/// route table is `Arc`-shared across every per-request invocation.
#[derive(Clone)]
pub(crate) struct BodyLimitState {
    routes: Arc<Vec<RouteBodyLimit>>,
}

impl std::fmt::Debug for BodyLimitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BodyLimitState")
            .field("routes", &self.routes.len())
            .finish_non_exhaustive()
    }
}

/// Resolve the v2 descriptor set into a fixed lookup table consumed by
/// [`enforce_body_limit`]. Composition is infallible today because both
/// [`BodyLimitPolicy`] variants are supported; if upstream adds a new
/// variant this `match` fails to compile, which is the fail-closed
/// contract the gateway needs.
pub(crate) fn build_body_limit_state(descriptors: &[IngressRouteDescriptor]) -> BodyLimitState {
    let routes = descriptors
        .iter()
        .map(|descriptor| {
            let policy = match descriptor.policy().body_limit() {
                BodyLimitPolicy::NoBody => ResolvedBodyPolicy::NoBody,
                BodyLimitPolicy::Limited { max_bytes } => ResolvedBodyPolicy::Limited {
                    max_bytes: max_bytes.get(),
                },
            };
            RouteBodyLimit {
                route_id: descriptor.route_id().as_str().to_string(),
                method: network_method_to_axum(descriptor.method()),
                segments: parse_pattern(descriptor.route_pattern().as_str()),
                policy,
            }
        })
        .collect();
    BodyLimitState {
        routes: Arc::new(routes),
    }
}

fn match_route<'a>(
    routes: &'a [RouteBodyLimit],
    method: &Method,
    path: &str,
) -> Option<&'a RouteBodyLimit> {
    routes
        .iter()
        .find(|route| route.method == *method && segments_match(&route.segments, path))
}

fn declared_content_length(request: &Request) -> Option<u64> {
    request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.parse::<u64>().ok())
}

fn too_large_for(policy: ResolvedBodyPolicy) -> Response {
    let detail: &'static str = match policy {
        ResolvedBodyPolicy::NoBody => "Request body not allowed for this route.",
        ResolvedBodyPolicy::Limited { .. } => "Request body exceeds the route's body limit.",
    };
    (StatusCode::PAYLOAD_TOO_LARGE, detail).into_response()
}

/// Axum middleware enforcing the descriptor's [`BodyLimitPolicy`] for
/// every matched v2 route. Unmatched paths pass through — the outer
/// global `RequestBodyLimitLayer` (set in `webui_serve`) still caps
/// them as defense in depth.
pub(crate) async fn enforce_body_limit(
    State(state): State<BodyLimitState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(route) = match_route(&state.routes, request.method(), request.uri().path()) else {
        return next.run(request).await;
    };

    let max_bytes_u64: u64 = match route.policy {
        ResolvedBodyPolicy::NoBody => 0,
        ResolvedBodyPolicy::Limited { max_bytes } => max_bytes,
    };

    // Reject upfront when the client advertised a Content-Length that
    // already exceeds the policy. Saves us from buffering bytes we'd
    // drop anyway.
    if let Some(declared) = declared_content_length(&request)
        && declared > max_bytes_u64
    {
        tracing::debug!(
            target = "ironclaw::reborn::webui_body_limit",
            route_id = %route.route_id,
            declared,
            limit = max_bytes_u64,
            "rejecting oversized request by declared Content-Length",
        );
        return too_large_for(route.policy);
    }

    // Cast is safe: the largest descriptor cap is 14 MiB and the
    // workspace targets are 64-bit platforms. `usize::try_from` returns
    // an error only on 32-bit platforms with a > 4 GiB cap, which the
    // v2 descriptors do not declare.
    let max_bytes_usize = match usize::try_from(max_bytes_u64) {
        Ok(value) => value,
        Err(_) => {
            tracing::debug!(
                target = "ironclaw::reborn::webui_body_limit",
                route_id = %route.route_id,
                limit = max_bytes_u64,
                "body limit exceeds usize; rejecting as if oversized",
            );
            return too_large_for(route.policy);
        }
    };

    // Buffer with the descriptor cap as the hard limit. `to_bytes`
    // reads up to `limit` bytes and returns an error if the body
    // produced more. Buffering is acceptable because every v2 route
    // caps at ≤ 14 MiB and the v2 handlers already buffer via
    // `Json<T>` downstream.
    let (parts, body) = request.into_parts();
    let buffered = match axum::body::to_bytes(body, max_bytes_usize).await {
        Ok(bytes) => bytes,
        Err(_) => {
            tracing::debug!(
                target = "ironclaw::reborn::webui_body_limit",
                route_id = %route.route_id,
                limit = max_bytes_u64,
                "rejecting body that exceeded the per-route cap during read",
            );
            return too_large_for(route.policy);
        }
    };

    // Defensive: `to_bytes` should already enforce the bound, but if
    // a future axum release loosens that contract the explicit length
    // check still fails closed.
    if buffered.len() as u64 > max_bytes_u64 {
        return too_large_for(route.policy);
    }

    let request = Request::from_parts(parts, Body::from(buffered));
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ingress::{
        AllowedEffectPath, AuditTraceClass, CorsPolicy, IngressAuthPolicy, IngressAuthScheme,
        IngressPolicy, IngressPolicyParts, IngressRouteDescriptor, ListenerClass, RateLimitPolicy,
        RateLimitScope, StreamingMode, WebSocketOriginPolicy,
    };
    use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
    use std::num::{NonZeroU32, NonZeroU64};

    fn limited_policy(max_bytes_kib: u64) -> IngressPolicy {
        let max_bytes = NonZeroU64::new(max_bytes_kib * 1024).expect("nz");
        IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::LocalGateway,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::BearerToken],
            },
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::Limited { max_bytes },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::PerCaller,
                max_requests: NonZeroU32::new(60).expect("nz"),
                window_seconds: NonZeroU32::new(60).expect("nz"),
            },
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        })
        .expect("policy")
    }

    fn nobody_policy() -> IngressPolicy {
        IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::LocalGateway,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::BearerToken],
            },
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::PerCaller,
                max_requests: NonZeroU32::new(60).expect("nz"),
                window_seconds: NonZeroU32::new(60).expect("nz"),
            },
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProjectionOnly,
        })
        .expect("policy")
    }

    fn descriptor(
        route_id: &str,
        method: NetworkMethod,
        pattern: &str,
        policy: IngressPolicy,
    ) -> IngressRouteDescriptor {
        IngressRouteDescriptor::new(route_id.to_string(), method, pattern.to_string(), policy)
            .expect("descriptor")
    }

    #[test]
    fn build_body_limit_state_accepts_webui_v2_descriptors() {
        let descriptors = ironclaw_webui_v2::webui_v2_routes();
        let state = build_body_limit_state(&descriptors);
        assert_eq!(
            state.routes.len(),
            descriptors.len(),
            "every descriptor produced a RouteBodyLimit entry",
        );
        // Locks in the descriptor contract: send_message must be 14 MiB
        // (text + base64 inline attachments), get_timeline and
        // stream_events must be NoBody. A regression that flips these
        // would trip here before reaching production.
        let send = state
            .routes
            .iter()
            .find(|r| r.route_id == "webui.v2.send_message")
            .expect("send_message route");
        assert!(matches!(
            send.policy,
            ResolvedBodyPolicy::Limited { max_bytes } if max_bytes == 14 * 1024 * 1024,
        ));
        let timeline = state
            .routes
            .iter()
            .find(|r| r.route_id == "webui.v2.get_timeline")
            .expect("get_timeline route");
        assert!(matches!(timeline.policy, ResolvedBodyPolicy::NoBody));
    }

    #[test]
    fn build_body_limit_state_preserves_per_route_caps_from_descriptors() {
        let descriptors = vec![
            descriptor(
                "test.small",
                NetworkMethod::Post,
                "/api/small",
                limited_policy(4),
            ),
            descriptor(
                "test.large",
                NetworkMethod::Post,
                "/api/large",
                limited_policy(1024),
            ),
            descriptor(
                "test.read",
                NetworkMethod::Get,
                "/api/read",
                nobody_policy(),
            ),
        ];
        let state = build_body_limit_state(&descriptors);
        let small = state
            .routes
            .iter()
            .find(|r| r.route_id == "test.small")
            .unwrap();
        let large = state
            .routes
            .iter()
            .find(|r| r.route_id == "test.large")
            .unwrap();
        let read = state
            .routes
            .iter()
            .find(|r| r.route_id == "test.read")
            .unwrap();
        assert!(
            matches!(small.policy, ResolvedBodyPolicy::Limited { max_bytes } if max_bytes == 4 * 1024)
        );
        assert!(
            matches!(large.policy, ResolvedBodyPolicy::Limited { max_bytes } if max_bytes == 1024 * 1024)
        );
        assert!(matches!(read.policy, ResolvedBodyPolicy::NoBody));
    }
}
