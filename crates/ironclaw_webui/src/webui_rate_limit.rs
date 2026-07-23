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
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Mints a unique, monotonically increasing id every time a [`Window`]
    /// entry is (re)created — a fresh key insert or a window-expiry reset.
    /// [`refund_charge`] validates this instead of `window_start` alone: an
    /// entry evicted from the shard's LRU under load and then reinserted
    /// for the same key within the same wall-clock second would otherwise
    /// coincidentally match on `window_start`, letting a delayed refund for
    /// the evicted charge credit the unrelated replacement entry (PR #6592
    /// review). A single counter shared across shards is simplest and the
    /// contention is negligible next to the shard mutex each charge/refund
    /// already takes.
    next_generation: Arc<AtomicU64>,
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
    /// Unique id minted by [`RateLimitState::next_generation`] when this
    /// window instance was (re)created. Refunds validate this — see the
    /// field doc on `next_generation` for why `window_start` alone isn't
    /// sufficient.
    generation: u64,
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
        next_generation: Arc::new(AtomicU64::new(0)),
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
    let charged_generation = {
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
            generation: state.next_generation.fetch_add(1, Ordering::Relaxed),
        });

        if now.saturating_sub(window_entry.window_start) >= window_seconds {
            // Window expired — start a new one. Charge the current
            // request against the fresh budget. This is logically a new
            // window instance, so it gets a fresh generation too — a
            // refund arriving after a legitimate rollover for the same
            // caller must not credit the new window any more than one
            // arriving after an LRU eviction should.
            window_entry.window_start = now;
            window_entry.remaining = max_requests.saturating_sub(1);
            window_entry.generation = state.next_generation.fetch_add(1, Ordering::Relaxed);
            Some(window_entry.generation)
        } else if window_entry.remaining == 0 {
            None
        } else {
            window_entry.remaining -= 1;
            Some(window_entry.generation)
        }
    };

    let Some(charged_generation) = charged_generation else {
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
            charged_generation,
            max_requests,
        );
    }

    response
}

/// Credits back a charge made by [`enforce_rate_limit`] when the downstream
/// handler marked its own `429` response refundable (see
/// [`mark_rate_limit_refundable`]). Only refunds into the exact window
/// instance it charged — validated by `generation`, a per-entry token minted
/// by [`RateLimitState::next_generation`] whenever a [`Window`] is (re)created
/// (fresh insert or window-expiry reset). A plain `window_start` comparison
/// is only second-resolution: if the charged entry is evicted from the
/// shard's LRU under load and the same caller then makes a brand-new,
/// unrelated request within the same wall-clock second, the replacement
/// entry's `window_start` can coincidentally match the original charge's.
/// `generation` is unique per (re)creation regardless of timing, so it
/// can't be fooled by that coincidence — it subsumes the `window_start`
/// check rather than complementing it. If the entry is missing (evicted and
/// never reinserted) or its generation has moved on (rolled over or
/// replaced), this is a no-op rather than mis-crediting the wrong window.
fn refund_charge(
    shard: &Mutex<LruCache<CounterKey, Window>>,
    key: &CounterKey,
    charged_generation: u64,
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
        && window_entry.generation == charged_generation
        && window_entry.remaining < max_requests
    {
        window_entry.remaining += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{TenantId, UserId};

    /// `pub(super)`: reused by the sibling `webui_rate_limit_router_contract_test` module.
    pub(super) fn caller(tenant: &str, user: &str) -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new(tenant).expect("tenant"),
            UserId::new(user).expect("user"),
            None,
            None,
        )
    }

    /// `pub(super)`: reused by the sibling `webui_rate_limit_refund_test` module.
    pub(super) fn limited_state(max: u32, window_secs: u32) -> RateLimitState {
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
            next_generation: Arc::new(AtomicU64::new(0)),
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
            generation: state.next_generation.fetch_add(1, Ordering::Relaxed),
        });
        if now.saturating_sub(window_entry.window_start) >= window_seconds {
            window_entry.window_start = now;
            window_entry.remaining = max.saturating_sub(1);
            window_entry.generation = state.next_generation.fetch_add(1, Ordering::Relaxed);
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
}

#[cfg(test)]
#[path = "webui_rate_limit_router_contract_test.rs"]
mod webui_rate_limit_router_contract_test;

// Finding C6 (PR #6592 review, decompose >1000-line file): the
// refund-specific `mod tests` helpers/tests moved to this sibling
// module — see its module doc for why it stays under `src/` rather
// than `tests/`.
#[cfg(test)]
#[path = "webui_rate_limit_refund_test.rs"]
mod webui_rate_limit_refund_test;
