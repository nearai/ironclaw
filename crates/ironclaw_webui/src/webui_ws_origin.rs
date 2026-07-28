//! Descriptor-driven `Origin` enforcement for WebChat v2 routes that
//! declare a [`WebSocketOriginPolicy`] other than `NotApplicable`.
//!
//! The CORS layer composed by [`crate::webui_serve::webui_v2_app`]
//! handles ordinary XHR pre-flight, but the browser does NOT issue a
//! pre-flight before a WebSocket upgrade — it just opens a new
//! connection and sends `Origin` directly on the upgrade request.
//! Same-origin enforcement on WS therefore has to run inline.
//!
//! Today the v2 surface has exactly one WS descriptor
//! (`stream_events_ws`); the middleware ignores any path the
//! descriptor table doesn't claim. A future descriptor adding more WS
//! routes is picked up automatically.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ironclaw_host_api::ingress::{IngressRouteDescriptor, StreamingMode, WebSocketOriginPolicy};

use crate::webui_route_match::{network_method_to_axum, parse_pattern, segments_match};

#[derive(Debug, Clone)]
struct WsRouteOriginRule {
    method: axum::http::Method,
    segments: Vec<Option<String>>,
    policy: WebSocketOriginPolicy,
}

/// Shared state for [`enforce_websocket_origin`]. Cheap to clone.
#[derive(Clone)]
pub(crate) struct WebSocketOriginState {
    routes: Arc<Vec<WsRouteOriginRule>>,
    /// Stringified allow-list — reused from the same source the CORS
    /// layer consumes (`WebuiServeConfig::allowed_origins`). The
    /// configured `HeaderValue` is converted to `&str` once at
    /// composition time; values that don't round-trip are dropped
    /// here too (a malformed entry could not pass through `CorsLayer`
    /// either).
    allowed_origins: Arc<Vec<String>>,
    /// Optional canonical host (`WebuiServeConfig::canonical_host`).
    /// When set, the `SameOriginRequired` policy compares `Origin`
    /// against this value instead of trusting the client-supplied
    /// `Host` header — protects against reverse-proxy
    /// passthrough-Host misconfigurations.
    canonical_host: Option<Arc<String>>,
}

impl std::fmt::Debug for WebSocketOriginState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketOriginState")
            .field("ws_routes", &self.routes.len())
            .field("allowed_origins", &self.allowed_origins.len())
            .field("canonical_host_set", &self.canonical_host.is_some())
            .finish()
    }
}

/// Build the lookup table consumed by [`enforce_websocket_origin`].
/// Only descriptors with `streaming == WebSocket` are recorded; the
/// middleware short-circuits other paths.
pub(crate) fn build_websocket_origin_state(
    descriptors: &[IngressRouteDescriptor],
    allowed_origins: &[axum::http::HeaderValue],
    canonical_host: Option<String>,
) -> WebSocketOriginState {
    let routes = descriptors
        .iter()
        .filter(|descriptor| descriptor.policy().streaming() == StreamingMode::WebSocket)
        .map(|descriptor| WsRouteOriginRule {
            method: network_method_to_axum(descriptor.method()),
            segments: parse_pattern(descriptor.route_pattern().as_str()),
            policy: descriptor.policy().websocket_origin(),
        })
        .collect();
    let allowed_origins = allowed_origins
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    WebSocketOriginState {
        routes: Arc::new(routes),
        allowed_origins: Arc::new(allowed_origins),
        canonical_host: canonical_host.map(Arc::new),
    }
}

fn match_ws_route<'a>(
    routes: &'a [WsRouteOriginRule],
    method: &axum::http::Method,
    path: &str,
) -> Option<&'a WsRouteOriginRule> {
    routes
        .iter()
        .find(|route| route.method == *method && segments_match(&route.segments, path))
}

fn origin_header_value(request: &Request) -> Option<String> {
    request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn origin_matches_host(origin: &str, host: &str) -> bool {
    // Same-origin: the Origin's `host:port` (after stripping `scheme://`)
    // matches the Host header. Browsers always send a fully-qualified
    // Origin on WS upgrade, so we only need to strip the scheme.
    let stripped = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .unwrap_or(origin);
    stripped == host
}

fn forbidden_origin(detail: &'static str) -> Response {
    // Security event — log at info! so production deployments with
    // `RUST_LOG=info` (the default) see WS-origin rejections in their
    // request logs. info!/warn! is safe inside HTTP handlers; the
    // CLAUDE.md REPL/TUI restriction is about background tasks and
    // interactive CLI surfaces, not gateway request handling.
    tracing::info!(
        target = "ironclaw::reborn::webui_ws_origin",
        detail,
        "rejecting WebSocket upgrade with disallowed Origin",
    );
    (StatusCode::FORBIDDEN, detail).into_response()
}

/// Axum middleware enforcing each WS descriptor's
/// [`WebSocketOriginPolicy`] before the request reaches the handler.
/// Non-WS paths pass through.
pub(crate) async fn enforce_websocket_origin(
    State(state): State<WebSocketOriginState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(route) = match_ws_route(&state.routes, request.method(), request.uri().path()) else {
        return next.run(request).await;
    };
    match route.policy {
        WebSocketOriginPolicy::NotApplicable => next.run(request).await,
        WebSocketOriginPolicy::LocalhostAllowed => {
            // Permits any localhost-looking origin OR explicit allowlist
            // entry. Useful for local dev where the browser may not
            // always send a Host the listener can mirror.
            let Some(origin) = origin_header_value(&request) else {
                return forbidden_origin("WebSocket upgrade requires Origin header");
            };
            let allowed = origin_is_localhost(&origin)
                || state.allowed_origins.iter().any(|entry| entry == &origin);
            if allowed {
                next.run(request).await
            } else {
                forbidden_origin("WebSocket Origin not in the host-configured allowlist")
            }
        }
        WebSocketOriginPolicy::HostConfiguredAllowlist => {
            let Some(origin) = origin_header_value(&request) else {
                return forbidden_origin("WebSocket upgrade requires Origin header");
            };
            if state.allowed_origins.iter().any(|entry| entry == &origin) {
                next.run(request).await
            } else {
                forbidden_origin("WebSocket Origin not in the host-configured allowlist")
            }
        }
        WebSocketOriginPolicy::SameOriginRequired => {
            let Some(origin) = origin_header_value(&request) else {
                return forbidden_origin("WebSocket upgrade requires Origin header");
            };
            // Prefer the operator-configured canonical host over the
            // client-supplied `Host` header. A reverse proxy that
            // forwards an attacker-controlled Host would otherwise let
            // the same-origin check pass for a forged Origin. Falls
            // back to `Host` only when canonical_host is unset.
            let host_for_compare: Option<&str> = state
                .canonical_host
                .as_deref()
                .map(String::as_str)
                .or_else(|| {
                    // Host header is read from the request — wrap in
                    // the let-else dance once here so the optional
                    // logic stays compact.
                    request
                        .headers()
                        .get(header::HOST)
                        .and_then(|value| value.to_str().ok())
                });
            let Some(host) = host_for_compare else {
                return forbidden_origin(
                    "WebSocket upgrade requires Host header (or canonical_host config)",
                );
            };
            // Host installations that front WS through a different
            // origin (reverse proxy / CDN) can still opt in via the
            // allowed_origins list.
            let allowed = origin_matches_host(&origin, host)
                || state.allowed_origins.iter().any(|entry| entry == &origin);
            if allowed {
                next.run(request).await
            } else {
                forbidden_origin("WebSocket Origin must match the request Host")
            }
        }
    }
}

fn origin_is_localhost(origin: &str) -> bool {
    let stripped = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .unwrap_or(origin);
    // Strip optional port suffix. IPv6 literals are bracketed
    // (`[::1]:3000`), so the host portion is everything inside the
    // brackets; for IPv4 / DNS names we split on the last `:`.
    let host = if let Some(rest) = stripped.strip_prefix('[') {
        // `[::1]:3000` → `::1`; `[::1]` → `::1`.
        match rest.find(']') {
            Some(end) => &rest[..end], // safety: find returns a UTF-8 character boundary.
            None => return false,      // malformed
        }
    } else if let Some(idx) = stripped.rfind(':') {
        // `127.0.0.1:3000` → `127.0.0.1`. Take a slice that excludes
        // the port. The colon-only check below catches the no-port
        // case for IPv6 literals already.
        &stripped[..idx] // safety: rfind returns a UTF-8 character boundary.
    } else {
        stripped
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
        || host.starts_with("127.")
        || host == "::ffff:127.0.0.1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_matches_host_strips_scheme() {
        assert!(origin_matches_host("http://example.test", "example.test"));
        assert!(origin_matches_host(
            "https://example.test:443",
            "example.test:443"
        ));
        assert!(!origin_matches_host(
            "http://evil.example.test",
            "example.test"
        ));
    }

    #[test]
    fn origin_is_localhost_recognizes_common_forms() {
        assert!(origin_is_localhost("http://localhost"));
        assert!(origin_is_localhost("http://127.0.0.1:3000"));
        assert!(origin_is_localhost("https://localhost:8443"));
        // IPv6 literals must parse correctly with and without port.
        // Browsers serialize IPv6 origins with brackets per RFC 6454.
        assert!(origin_is_localhost("http://[::1]"));
        assert!(origin_is_localhost("http://[::1]:3000"));
        assert!(origin_is_localhost("http://[::ffff:127.0.0.1]"));
        // Loopback /8 — `127.x.y.z` is the entire IPv4 loopback block.
        assert!(origin_is_localhost("http://127.0.0.42"));
        // Non-loopback rejections.
        assert!(!origin_is_localhost("http://attacker.test"));
        assert!(!origin_is_localhost("http://192.168.1.1"));
        // Malformed bracketed origin must NOT pass.
        assert!(!origin_is_localhost("http://[no-close"));
    }

    #[test]
    fn build_state_collects_only_ws_descriptors() {
        let descriptors = crate::webui_v2::webui_v2_routes();
        let state = build_websocket_origin_state(&descriptors, &[], None);
        assert!(
            !state.routes.is_empty(),
            "the v2 descriptor set must declare at least one WS route",
        );
        for route in state.routes.iter() {
            assert!(
                matches!(
                    route.policy,
                    WebSocketOriginPolicy::SameOriginRequired
                        | WebSocketOriginPolicy::HostConfiguredAllowlist
                        | WebSocketOriginPolicy::LocalhostAllowed
                ),
                "WS descriptor must declare a meaningful WebSocketOriginPolicy",
            );
        }
    }
}
