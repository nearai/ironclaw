//! Descriptor-driven per-route rate-limit middleware for the WebChat v2
//! native surface.
//!
//! `crate::webui_v2::webui_v2_routes()` returns an
//! [`IngressRouteDescriptor`] per route, each carrying a
//! [`RateLimitPolicy`] (mutation 60/60, read 120/60, stream 30/60 in
//! the current beta). The v2 crate's CLAUDE.md explicitly designates
//! enforcement of these policies as a host-composition responsibility;
//! this module is that enforcement.
//!
//! Design choices:
//!
//! - **Sliding window per descriptor-declared bucket** — authenticated
//!   routes use `(route, caller)`, while public callback-style routes
//!   use route/global/IP buckets that do not need caller identity.
//! - **Supported scopes:** `PerCaller` for authenticated routes and
//!   `PerRoute` / `PerIp` / `Global` for public callback-style routes
//!   that have no authenticated caller extension yet. `PerIp` uses the
//!   transport peer address injected by the host-owned ingress
//!   (`ConnectInfo<SocketAddr>`); it never trusts `X-Forwarded-For` or
//!   `X-Real-IP` headers. `PerTenant` remains an explicit `Err` at
//!   composition time so a future policy change cannot silently degrade
//!   enforcement.
//! - **Sharded LRU eviction** — counters live in 16 independent
//!   `Mutex<LruCache>` shards picked by a hash of the resolved bucket key.
//!   Each shard is capped at 512 entries (16 × 512 = 8192-entry total
//!   budget). Concurrent requests for different callers very rarely
//!   contend on the same shard's mutex; a single caller's bursts
//!   serialize against their own shard only. Evicted entries simply
//!   reset their window; a caller that loses its counter and then
//!   bursts is no worse off than a brand-new caller.
//! - **Disabled routes pass through.** A descriptor with
//!   `RateLimitPolicy::Disabled` (the v2 beta does not have any, but
//!   the type allows it) records no counters and never returns 429.
//! - **Explicitly-marked downstream 429s are refunded.** A handler can
//!   opt a specific rejection out of this middleware's charge by calling
//!   [`mark_rate_limit_refundable`] on its response — used today by the
//!   SSE per-caller concurrency cap in `webui_v2::sse_capacity`, which
//!   reflects a resource limit unrelated to request-volume abuse. The
//!   marker is a response *extension* (server-side-only metadata, never
//!   serialized onto the wire) that this middleware consumes and removes
//!   before returning — it never reaches the client either way.
//!   Refunding is opt-in and keyed off this explicit marker, **not**
//!   the bare `429` status code: this protected surface also carries
//!   system-wide admission-control 429s (`TurnErrorCategory::
//!   AdmissionRejected` / `CapacityExceeded`, mapped in
//!   `ironclaw_product_workflow::reborn_services::map_turn_error`) that
//!   must keep draining the caller's budget precisely during the
//!   overload they exist to signal — refunding those would let a
//!   caller flooding the system during an outage dodge the very limit
//!   meant to contain it.

use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ironclaw_host_api::ingress::{IngressRouteDescriptor, RateLimitPolicy, RateLimitScope};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use lru::LruCache;

use crate::webui_route_match::{network_method_to_axum, parse_pattern, segments_match};

/// Number of sharded counter maps. Each authenticated request takes
/// exactly one shard's mutex, so contention scales as 1/SHARD_COUNT in
/// the limit. 16 is the standard cache-line-friendly value; benchmarks
/// can move this if a higher-tenancy deployment demands more.
const SHARD_COUNT: usize = 16;

/// Hard cap on the number of `(route, caller)` counter entries kept in
/// memory **per shard**. Sized so the SHARD_COUNT × this product
/// matches the original 8_192-entry budget (8_192 / 16 = 512). A
/// caller's counter lives in exactly one shard; cross-shard eviction
/// is independent.
///
/// Stored as a `NonZeroUsize` const so the runtime constructor can avoid
/// runtime extraction from a value the compiler can prove is non-zero.
const RATE_LIMIT_PER_SHARD_CAPACITY: NonZeroUsize = match NonZeroUsize::new(512) {
    Some(value) => value,
    // SAFETY: 512 is a non-zero compile-time constant; the match arm
    // is unreachable. Written with an explicit match so production code avoids
    // runtime value extraction from a value known to be non-zero.
    None => unreachable!(),
};

/// Error returned when [`build_rate_limit_state`] cannot accept a
/// descriptor — typically because the host has shipped a scope that
/// the gateway doesn't yet implement.
#[derive(Debug, thiserror::Error)]
pub enum RateLimitConfigError {
    #[error(
        "rate-limit scope {scope:?} on route `{route_id}` is not supported by the WebUI gateway \
         composition; supported scopes are PerCaller, PerRoute, PerIp, and Global"
    )]
    UnsupportedScope {
        route_id: String,
        scope: RateLimitScope,
    },
}

/// Per-route policy resolved from a descriptor at composition time.
#[derive(Debug, Clone)]
struct RouteLimit {
    route_id: String,
    method: Method,
    /// Pattern split into segments. Each entry is either a static
    /// literal (the leading slash is stripped) or `None` to mark a
    /// wildcard `{name}` slot. Stored once at composition time so the
    /// hot-path matcher does not re-parse the pattern per request.
    segments: Vec<Option<String>>,
    policy: ResolvedPolicy,
}

#[derive(Debug, Clone, Copy)]
enum ResolvedPolicy {
    Limited {
        scope: RateLimitScope,
        max_requests: u32,
        window: Duration,
    },
    Disabled,
}

/// Shared state for [`enforce_rate_limit`]. Cheap to clone — the
/// inner counter maps are sharded across [`SHARD_COUNT`] independent
/// `Mutex<LruCache<…>>`s, picked by a hash of the resolved bucket key, so
/// concurrent rate-limit checks for different callers don't contend
/// on the same mutex. Each shard's lock is held only for the window
/// update + counter decrement — microseconds in the warm path.
#[derive(Clone)]
pub(crate) struct RateLimitState {
    routes: Arc<Vec<RouteLimit>>,
    shards: Arc<Vec<Mutex<LruCache<CounterKey, Window>>>>,
}

impl std::fmt::Debug for RateLimitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitState")
            .field("routes", &self.routes.len())
            .field("shards", &self.shards.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CounterKey {
    route_idx: usize,
    /// Stable limiter bucket. For authenticated routes this is the
    /// caller identity formatted as `tenant\x1fuser`; for public callback
    /// routes it is route/global/IP-scoped and contains no user material.
    bucket_key: String,
}

#[derive(Debug)]
struct Window {
    /// Number of accepted requests in the current window.
    remaining: u32,
    /// Epoch second at which the current window started.
    window_start: u64,
}

#[derive(Debug)]
enum CounterKeyError {
    Misconfigured,
}

/// Resolve the v2 descriptor set into a fixed lookup table consumed by
/// [`enforce_rate_limit`]. Returns `Err` for unsupported policy shapes
/// so a regression in the descriptor surface fails composition rather
/// than silently dropping enforcement.
pub(crate) fn build_rate_limit_state(
    descriptors: &[IngressRouteDescriptor],
) -> Result<RateLimitState, RateLimitConfigError> {
    let mut routes = Vec::with_capacity(descriptors.len());
    for descriptor in descriptors {
        let route_id = descriptor.route_id().as_str().to_string();
        let policy = resolve_policy(&route_id, descriptor.policy().rate_limit())?;
        let method = network_method_to_axum(descriptor.method());
        let segments = parse_pattern(descriptor.route_pattern().as_str());
        routes.push(RouteLimit {
            route_id,
            method,
            segments,
            policy,
        });
    }

    let shards = (0..SHARD_COUNT)
        .map(|_| Mutex::new(LruCache::new(RATE_LIMIT_PER_SHARD_CAPACITY)))
        .collect::<Vec<_>>();
    Ok(RateLimitState {
        routes: Arc::new(routes),
        shards: Arc::new(shards),
    })
}

/// Pick the shard for a given limiter bucket. Uses `DefaultHasher` for
/// uniform-enough distribution across 16 buckets; we don't need
/// adversarial resistance because per-caller buckets come from
/// host-authenticated caller identity and public route/global buckets
/// are fixed by descriptor metadata.
fn shard_index(bucket_key: &str) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bucket_key.hash(&mut hasher);
    (hasher.finish() as usize) % SHARD_COUNT
}

fn resolve_policy(
    route_id: &str,
    policy: &RateLimitPolicy,
) -> Result<ResolvedPolicy, RateLimitConfigError> {
    match policy {
        RateLimitPolicy::Disabled { .. } => Ok(ResolvedPolicy::Disabled),
        RateLimitPolicy::Limited {
            scope,
            max_requests,
            window_seconds,
        } => match scope {
            RateLimitScope::PerCaller
            | RateLimitScope::PerRoute
            | RateLimitScope::PerIp
            | RateLimitScope::Global => Ok(ResolvedPolicy::Limited {
                scope: *scope,
                max_requests: max_requests.get(),
                window: Duration::from_secs(u64::from(window_seconds.get())),
            }),
            other => Err(RateLimitConfigError::UnsupportedScope {
                route_id: route_id.to_string(),
                scope: *other,
            }),
        },
    }
}

fn request_counter_key(
    route_idx: usize,
    route: &RouteLimit,
    request: &Request,
) -> Result<CounterKey, CounterKeyError> {
    let ResolvedPolicy::Limited { scope, .. } = route.policy else {
        return Err(CounterKeyError::Misconfigured);
    };

    let bucket_key = match scope {
        RateLimitScope::PerCaller => {
            let Some(caller) = request.extensions().get::<WebUiAuthenticatedCaller>() else {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_rate_limit",
                    route_id = %route.route_id,
                    "per-caller rate-limit reached without an authenticated caller — \
                     auth middleware must run first",
                );
                return Err(CounterKeyError::Misconfigured);
            };
            caller_key(caller)
        }
        RateLimitScope::PerRoute => format!("route\x1f{}", route.route_id),
        RateLimitScope::PerIp => {
            let Some(connect_info) = request.extensions().get::<ConnectInfo<SocketAddr>>() else {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_rate_limit",
                    route_id = %route.route_id,
                    "per-ip rate-limit reached without host-provided ConnectInfo — \
                     host ingress must inject transport peer addresses",
                );
                return Err(CounterKeyError::Misconfigured);
            };
            format!("peer_ip\x1f{}", connect_info.0.ip())
        }
        RateLimitScope::Global => "global".to_string(),
        RateLimitScope::PerTenant => {
            tracing::debug!(
                target = "ironclaw::reborn::webui_rate_limit",
                route_id = %route.route_id,
                scope = ?scope,
                "unsupported rate-limit scope reached runtime after composition",
            );
            return Err(CounterKeyError::Misconfigured);
        }
    };

    let route_idx = if scope == RateLimitScope::Global {
        usize::MAX
    } else {
        route_idx
    };

    Ok(CounterKey {
        route_idx,
        bucket_key,
    })
}

/// Build the `(method, path)` → route index lookup for one request.
fn match_route(routes: &[RouteLimit], method: &Method, path: &str) -> Option<usize> {
    routes
        .iter()
        .enumerate()
        .find(|(_, route)| route.method == *method && segments_match(&route.segments, path))
        .map(|(idx, _)| idx)
}

fn caller_key(caller: &WebUiAuthenticatedCaller) -> String {
    // \x1F (unit separator) is rejected by `TenantId` / `UserId` newtypes
    // at construction time, so it can never appear inside a valid id —
    // safe to use as the join delimiter for a flat key.
    format!(
        "{}\x1f{}",
        caller.tenant_id.as_str(),
        caller.user_id.as_str()
    )
}

/// Marker a handler attaches to its own `429` response (via
/// [`mark_rate_limit_refundable`]) to opt that specific rejection out of
/// [`enforce_rate_limit`]'s charge — see the "Explicitly-marked downstream
/// 429s are refunded" module doc. Carried as a response *extension*, not a
/// header: extensions are server-side-only metadata that never serialize
/// onto the wire, so there is no risk of this internal signal leaking to
/// the actual HTTP client.
#[derive(Clone)]
struct RateLimitRefundable;

/// Marks a handler's own `429` response as refundable against the caller's
/// [`enforce_rate_limit`] budget — use for a rejection that reflects a
/// resource limit unrelated to request-volume abuse (e.g. the SSE
/// per-caller concurrency cap in `webui_v2::sse_capacity`). Do **not** use
/// for a system-wide capacity/admission-control signal: those must keep
/// draining the caller's budget during the overload they exist to contain.
pub(crate) fn mark_rate_limit_refundable(mut response: Response) -> Response {
    response.extensions_mut().insert(RateLimitRefundable);
    response
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        // SystemTime before UNIX_EPOCH is undefined territory for a
        // server clock; treat it as t=0 so the next request restarts a
        // fresh window. The window length itself is independent of
        // this anchor so over-restarting is the worst outcome.
        .unwrap_or(0)
}

/// Axum middleware that enforces the per-route rate limits resolved by
/// [`build_rate_limit_state`]. Authenticated routes run this after the
/// bearer-auth middleware so the [`WebUiAuthenticatedCaller`] extension
/// is available; public callback routes use route/global buckets that do
/// not require a caller extension. Returns 429 when the bucket has
/// exhausted the route's window; otherwise passes through.
pub(crate) async fn enforce_rate_limit(
    State(state): State<RateLimitState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(route_idx) = match_route(&state.routes, request.method(), request.uri().path()) else {
        // Unknown path — the v2 router will fall through to 404. No
        // rate-limit decision applies because there is no policy to
        // consult.
        return next.run(request).await;
    };
    let route = &state.routes[route_idx];

    let (max_requests, window) = match route.policy {
        ResolvedPolicy::Disabled => return next.run(request).await,
        ResolvedPolicy::Limited {
            max_requests,
            window,
            ..
        } => (max_requests, window),
    };

    let key = match request_counter_key(route_idx, route, &request) {
        Ok(key) => key,
        Err(CounterKeyError::Misconfigured) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Rate-limit middleware misconfigured",
            )
                .into_response();
        }
    };

    let now = now_epoch_secs();
    let window_seconds = window.as_secs().max(1);

    let shard_idx = shard_index(&key.bucket_key);
    let charged_window_start = {
        let mut guard = match state.shards[shard_idx].lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_rate_limit",
                    "rate-limit LRU mutex poisoned — recovering",
                );
                poisoned.into_inner()
            }
        };

        let window_entry = guard.get_or_insert_mut(key.clone(), || Window {
            remaining: max_requests,
            window_start: now,
        });

        if now.saturating_sub(window_entry.window_start) >= window_seconds {
            // Window expired — start a new one. Charge the current
            // request against the fresh budget.
            window_entry.window_start = now;
            window_entry.remaining = max_requests.saturating_sub(1);
            Some(window_entry.window_start)
        } else if window_entry.remaining == 0 {
            None
        } else {
            window_entry.remaining -= 1;
            Some(window_entry.window_start)
        }
    };

    let Some(charged_window_start) = charged_window_start else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.",
        )
            .into_response();
    };

    let mut response = next.run(request).await;

    if response
        .extensions_mut()
        .remove::<RateLimitRefundable>()
        .is_some()
    {
        refund_charge(
            &state.shards[shard_idx],
            &key,
            charged_window_start,
            max_requests,
        );
    }

    response
}

/// Credits back a charge made by [`enforce_rate_limit`] when the downstream
/// handler marked its own `429` response refundable (see
/// [`mark_rate_limit_refundable`]). Only refunds into the exact window it
/// charged (`window_start` must be unchanged); if the window rolled over or
/// the entry was evicted under LRU pressure in the meantime, this is a
/// no-op rather than mis-crediting a different window.
fn refund_charge(
    shard: &Mutex<LruCache<CounterKey, Window>>,
    key: &CounterKey,
    charged_window_start: u64,
    max_requests: u32,
) {
    let mut guard = match shard.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::debug!(
                target = "ironclaw::reborn::webui_rate_limit",
                "rate-limit LRU mutex poisoned during refund — recovering",
            );
            poisoned.into_inner()
        }
    };
    if let Some(window_entry) = guard.get_mut(key)
        && window_entry.window_start == charged_window_start
        && window_entry.remaining < max_requests
    {
        window_entry.remaining += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{TenantId, UserId};

    fn caller(tenant: &str, user: &str) -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new(tenant).expect("tenant"),
            UserId::new(user).expect("user"),
            None,
            None,
        )
    }

    fn limited_state(max: u32, window_secs: u32) -> RateLimitState {
        let route = RouteLimit {
            route_id: "test.route".into(),
            method: Method::POST,
            segments: parse_pattern("/api/test"),
            policy: ResolvedPolicy::Limited {
                scope: RateLimitScope::PerCaller,
                max_requests: max,
                window: Duration::from_secs(u64::from(window_secs)),
            },
        };
        let shards = (0..SHARD_COUNT)
            .map(|_| Mutex::new(LruCache::new(RATE_LIMIT_PER_SHARD_CAPACITY)))
            .collect::<Vec<_>>();
        RateLimitState {
            routes: Arc::new(vec![route]),
            shards: Arc::new(shards),
        }
    }

    fn limited_route_with_scope(scope: RateLimitScope) -> RouteLimit {
        RouteLimit {
            route_id: "test.route".into(),
            method: Method::GET,
            segments: parse_pattern("/api/test"),
            policy: ResolvedPolicy::Limited {
                scope,
                max_requests: 2,
                window: Duration::from_secs(60),
            },
        }
    }

    fn consume(state: &RateLimitState, caller: &WebUiAuthenticatedCaller) -> bool {
        let key = CounterKey {
            route_idx: 0,
            bucket_key: caller_key(caller),
        };
        let mut guard = state.shards[shard_index(&key.bucket_key)]
            .lock()
            .expect("lock");
        let route = &state.routes[0];
        let (max, window) = match route.policy {
            ResolvedPolicy::Limited {
                max_requests,
                window,
                ..
            } => (max_requests, window),
            ResolvedPolicy::Disabled => return true,
        };
        let now = now_epoch_secs();
        let window_seconds = window.as_secs().max(1);
        let window_entry = guard.get_or_insert_mut(key, || Window {
            remaining: max,
            window_start: now,
        });
        if now.saturating_sub(window_entry.window_start) >= window_seconds {
            window_entry.window_start = now;
            window_entry.remaining = max.saturating_sub(1);
            true
        } else if window_entry.remaining == 0 {
            false
        } else {
            window_entry.remaining -= 1;
            true
        }
    }

    #[test]
    fn limit_blocks_after_max_requests_for_same_caller() {
        let state = limited_state(3, 60);
        let alice = caller("tenant-alpha", "alice");
        assert!(consume(&state, &alice));
        assert!(consume(&state, &alice));
        assert!(consume(&state, &alice));
        assert!(
            !consume(&state, &alice),
            "fourth request should be rejected"
        );
    }

    #[test]
    fn distinct_callers_have_independent_budgets() {
        let state = limited_state(2, 60);
        let alice = caller("tenant-alpha", "alice");
        let bob = caller("tenant-alpha", "bob");
        assert!(consume(&state, &alice));
        assert!(consume(&state, &alice));
        assert!(!consume(&state, &alice), "alice exhausted");
        assert!(consume(&state, &bob), "bob still has budget");
        assert!(consume(&state, &bob));
        assert!(!consume(&state, &bob), "bob exhausted");
    }

    #[test]
    fn build_rate_limit_state_accepts_webui_v2_descriptors() {
        let descriptors = crate::webui_v2::webui_v2_routes();
        let state = build_rate_limit_state(&descriptors).expect("v2 descriptors must be accepted");
        assert_eq!(
            state.routes.len(),
            descriptors.len(),
            "every descriptor produced a RouteLimit entry",
        );
    }

    #[test]
    fn unsupported_scope_is_rejected_at_composition() {
        // Regression guard for the fail-closed branch in
        // `resolve_policy`: a descriptor whose rate-limit scope is not
        // implemented by this gateway must abort composition rather
        // than silently degrade to no enforcement. Without this test, a
        // future v2 descriptor flipping `send_message` to e.g.
        // `PerTenant` would skip the limiter entirely.
        use ironclaw_host_api::NetworkMethod;
        use ironclaw_host_api::ingress::{
            AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
            IngressAuthScheme, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
            IngressScopeSource, ListenerClass, StreamingMode, WebSocketOriginPolicy,
        };
        use std::num::{NonZeroU32, NonZeroU64};

        let policy = IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::LocalGateway,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::BearerToken],
            },
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(1024).expect("nz"),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::PerTenant,
                max_requests: NonZeroU32::new(60).expect("nz"),
                window_seconds: NonZeroU32::new(60).expect("nz"),
            },
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        })
        .expect("policy must construct");

        let descriptor = IngressRouteDescriptor::new(
            "test.unsupported_scope".to_string(),
            NetworkMethod::Post,
            "/api/test".to_string(),
            policy,
        )
        .expect("descriptor must construct");

        let err =
            build_rate_limit_state(&[descriptor]).expect_err("PerTenant scope must be rejected");
        match err {
            RateLimitConfigError::UnsupportedScope { route_id, scope } => {
                assert_eq!(route_id, "test.unsupported_scope");
                assert!(matches!(scope, RateLimitScope::PerTenant));
            }
        }
    }

    #[test]
    fn per_ip_uses_host_peer_address_not_forwarded_headers() {
        let route = limited_route_with_scope(RateLimitScope::PerIp);
        let mut request = Request::builder()
            .method(Method::GET)
            .uri("/api/test")
            .header("x-forwarded-for", "198.51.100.10")
            .header("x-real-ip", "198.51.100.11")
            .body(axum::body::Body::empty())
            .expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 10], 443))));

        let key = request_counter_key(0, &route, &request).expect("counter key");
        assert_eq!(key.bucket_key, "peer_ip\x1f203.0.113.10");
    }

    #[test]
    fn per_ip_without_host_peer_address_fails_closed() {
        let route = limited_route_with_scope(RateLimitScope::PerIp);
        let request = Request::builder()
            .method(Method::GET)
            .uri("/api/test")
            .body(axum::body::Body::empty())
            .expect("request");

        assert!(matches!(
            request_counter_key(0, &route, &request),
            Err(CounterKeyError::Misconfigured)
        ));
    }

    /// Shared harness for the two `enforce_rate_limit` + downstream-429
    /// regression tests below: a real axum app with the middleware wired
    /// exactly as production does (`middleware::from_fn_with_state`, per
    /// `webui_serve.rs`), fronting a handler that always 429s — marking the
    /// response refundable or not depending on `mark_refundable`.
    fn refund_test_app(
        max_requests: u32,
        succeed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        mark_refundable: bool,
    ) -> axum::Router {
        use axum::Router;
        use axum::extract::State as AxumState;
        use axum::middleware;
        use axum::response::IntoResponse;
        use axum::routing::post;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Clone)]
        struct HandlerState {
            succeed: Arc<AtomicBool>,
            mark_refundable: bool,
        }

        async fn handler(AxumState(state): AxumState<HandlerState>) -> Response {
            if state.succeed.load(Ordering::SeqCst) {
                StatusCode::OK.into_response()
            } else if state.mark_refundable {
                mark_rate_limit_refundable(StatusCode::TOO_MANY_REQUESTS.into_response())
            } else {
                StatusCode::TOO_MANY_REQUESTS.into_response()
            }
        }

        // `limited_state` registers its route as POST — match it here, or
        // `match_route` silently no-ops the limiter for the whole test
        // (mismatched method looks like an unrelated route to the matcher).
        Router::new()
            .route("/api/test", post(handler))
            .with_state(HandlerState {
                succeed,
                mark_refundable,
            })
            .route_layer(middleware::from_fn_with_state(
                limited_state(max_requests, 60),
                enforce_rate_limit,
            ))
    }

    fn refund_test_request(alice: &WebUiAuthenticatedCaller) -> Request<axum::body::Body> {
        use axum::body::Body;
        use axum::http::{Method as HttpMethod, Request};

        let mut request = Request::builder()
            .method(HttpMethod::POST)
            .uri("/api/test")
            .body(Body::empty())
            .expect("request");
        request.extensions_mut().insert(alice.clone());
        request
    }

    /// Regression test for issue #6581: a downstream handler rejecting a
    /// request with its own 429 marked refundable (e.g. the SSE per-caller
    /// concurrency cap in `webui_v2::sse_capacity`, via
    /// `mark_rate_limit_refundable`) must not also drain the caller's
    /// per-route rate-limit budget.
    #[tokio::test]
    async fn refundable_downstream_429_does_not_consume_rate_limit_budget() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use tower::ServiceExt;

        let succeed = Arc::new(AtomicBool::new(false));
        let app = refund_test_app(2, succeed.clone(), true);
        let alice = caller("tenant-alpha", "alice");

        // max_requests is 2, but fire 5 requests against the always-429
        // handler. If the budget were spent on these, the middleware itself
        // would start rejecting before the handler is even reached.
        for attempt in 0..5 {
            let response = app
                .clone()
                .oneshot(refund_test_request(&alice))
                .await
                .expect("oneshot");
            assert_eq!(
                response.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "attempt {attempt} should reach the handler and get its 429"
            );
        }

        // Now let the handler succeed. This proves the budget was never
        // actually spent by the downstream refundable 429s above.
        succeed.store(true, Ordering::SeqCst);
        let response = app
            .clone()
            .oneshot(refund_test_request(&alice))
            .await
            .expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "refundable downstream 429s must not have consumed the rate-limit budget"
        );
    }

    /// Security regression: a downstream 429 that is NOT marked refundable
    /// — e.g. the turn-submission admission-control rejections mapped in
    /// `ironclaw_product_workflow::reborn_services::map_turn_error`
    /// (`TurnErrorCategory::AdmissionRejected` / `CapacityExceeded`) — must
    /// keep draining the caller's budget. Refunding these would let a
    /// caller flooding the system during an overload dodge the very limit
    /// meant to contain it.
    #[tokio::test]
    async fn unmarked_downstream_429_still_consumes_rate_limit_budget() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use tower::ServiceExt;

        let succeed = Arc::new(AtomicBool::new(false));
        let app = refund_test_app(2, succeed.clone(), false);
        let alice = caller("tenant-alpha", "alice");

        // Two unmarked 429s spend the whole (max_requests = 2) budget.
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(refund_test_request(&alice))
                .await
                .expect("oneshot");
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        }

        // Even once the handler would succeed, the middleware itself now
        // rejects because the budget was genuinely spent.
        succeed.store(true, Ordering::SeqCst);
        let response = app
            .clone()
            .oneshot(refund_test_request(&alice))
            .await
            .expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "unmarked downstream 429s must still consume the rate-limit budget"
        );
    }

    #[test]
    fn refund_charge_noops_when_window_rolled_over_since_charge() {
        let state = limited_state(3, 60);
        let alice = caller("tenant-alpha", "alice");
        let key = CounterKey {
            route_idx: 0,
            bucket_key: caller_key(&alice),
        };
        let shard = &state.shards[shard_index(&key.bucket_key)];

        // Simulate a charge made against a now-stale window: insert an
        // entry whose window_start is different from what the (stale)
        // caller believes it charged.
        {
            let mut guard = shard.lock().expect("lock");
            guard.get_or_insert_mut(key.clone(), || Window {
                remaining: 1,
                window_start: 1_000,
            });
        }
        let stale_charged_window_start = 500; // does not match the 1_000 above

        refund_charge(shard, &key, stale_charged_window_start, 3);

        let guard = shard.lock().expect("lock");
        let entry = guard.peek(&key).expect("entry still present");
        assert_eq!(
            entry.remaining, 1,
            "refund into a rolled-over window must be a no-op"
        );
    }

    #[test]
    fn refund_charge_noops_when_key_missing_from_cache() {
        let state = limited_state(3, 60);
        let alice = caller("tenant-alpha", "alice");
        let key = CounterKey {
            route_idx: 0,
            bucket_key: caller_key(&alice),
        };
        let shard = &state.shards[shard_index(&key.bucket_key)];

        // No entry was ever inserted for this key (never charged, or
        // evicted under LRU pressure) — refund must not fabricate one.
        refund_charge(shard, &key, now_epoch_secs(), 3);

        let guard = shard.lock().expect("lock");
        assert!(
            guard.peek(&key).is_none(),
            "refund must not create an entry for an uncharged/evicted key"
        );
    }

    #[test]
    fn refund_charge_does_not_credit_past_max_requests() {
        let state = limited_state(3, 60);
        let alice = caller("tenant-alpha", "alice");
        let key = CounterKey {
            route_idx: 0,
            bucket_key: caller_key(&alice),
        };
        let shard = &state.shards[shard_index(&key.bucket_key)];
        let window_start = now_epoch_secs();

        {
            let mut guard = shard.lock().expect("lock");
            guard.get_or_insert_mut(key.clone(), || Window {
                remaining: 3,
                window_start,
            });
        }

        // Two refunds for the same charge (a duplicate/racing refund)
        // must not push `remaining` past `max_requests`.
        refund_charge(shard, &key, window_start, 3);
        refund_charge(shard, &key, window_start, 3);

        let guard = shard.lock().expect("lock");
        let entry = guard.peek(&key).expect("entry still present");
        assert_eq!(
            entry.remaining, 3,
            "refund must not credit a window past max_requests"
        );
    }
}
