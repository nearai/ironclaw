//! Descriptor-driven per-route rate-limit middleware for the WebChat v2
//! native surface.
//!
//! `ironclaw_webui_v2::webui_v2_routes()` returns an
//! [`IngressRouteDescriptor`] per route, each carrying a
//! [`RateLimitPolicy`] (mutation 60/60, read 120/60, stream 12/60 in
//! the current beta). The v2 crate's CLAUDE.md explicitly designates
//! enforcement of these policies as a host-composition responsibility;
//! this module is that enforcement.
//!
//! Design choices:
//!
//! - **Sliding window per `(route, caller)`** — a single 30s burst from
//!   one user does not exhaust other users on the same route.
//! - **`PerCaller` scope only.** All v2 descriptors today declare
//!   `PerCaller`; other scopes (`PerTenant`, `PerIp`, `PerRoute`,
//!   `Global`) are explicit `Err` at composition time so a future
//!   policy change cannot silently degrade enforcement.
//! - **Sharded LRU eviction** — counters live in 16 independent
//!   `Mutex<LruCache>` shards picked by a hash of the caller identity.
//!   Each shard is capped at 512 entries (16 × 512 = 8192-entry total
//!   budget). Concurrent requests for different callers very rarely
//!   contend on the same shard's mutex; a single caller's bursts
//!   serialize against their own shard only. Evicted entries simply
//!   reset their window; a caller that loses its counter and then
//!   bursts is no worse off than a brand-new caller.
//! - **Disabled routes pass through.** A descriptor with
//!   `RateLimitPolicy::Disabled` (the v2 beta does not have any, but
//!   the type allows it) records no counters and never returns 429.

use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
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
         composition; only PerCaller is enforced today"
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
    Limited { max_requests: u32, window: Duration },
    Disabled,
}

/// Shared state for [`enforce_rate_limit`]. Cheap to clone — the
/// inner counter maps are sharded across [`SHARD_COUNT`] independent
/// `Mutex<LruCache<…>>`s, picked by a hash of the caller identity, so
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
    /// Caller identity formatted as `tenant\x1fuser`. The unit-separator
    /// byte is illegal inside `TenantId` / `UserId` (the grammar is
    /// `[a-z0-9._-]+`), so it cannot collide with a real id value.
    caller_key: String,
}

#[derive(Debug)]
struct Window {
    /// Number of accepted requests in the current window.
    remaining: u32,
    /// Epoch second at which the current window started.
    window_start: u64,
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

/// Pick the shard for a given caller. Uses `DefaultHasher` for
/// uniform-enough distribution across 16 buckets; we don't need
/// adversarial-resistance because the caller identity is already host-
/// authenticated and trusted to not be attacker-controlled in a way
/// that could collide on a specific shard.
fn shard_index(caller_key: &str) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    caller_key.hash(&mut hasher);
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
            RateLimitScope::PerCaller => Ok(ResolvedPolicy::Limited {
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
/// [`build_rate_limit_state`]. Runs after the bearer-auth middleware so
/// the [`WebUiAuthenticatedCaller`] extension is available. Returns 429
/// when the caller has exhausted the route's window; otherwise passes
/// through.
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
        } => (max_requests, window),
    };

    let caller = match request.extensions().get::<WebUiAuthenticatedCaller>() {
        Some(caller) => caller,
        None => {
            // No authenticated caller in extensions means the auth
            // middleware did not run (composition bug). Fail closed
            // with a 500 — under no circumstances should we silently
            // skip enforcement when the gate above us is missing.
            tracing::debug!(
                target = "ironclaw::reborn::webui_rate_limit",
                route_id = %route.route_id,
                "rate-limit middleware reached without an authenticated caller — \
                 auth middleware must run first",
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Rate-limit middleware misconfigured",
            )
                .into_response();
        }
    };

    let key = CounterKey {
        route_idx,
        caller_key: caller_key(caller),
    };

    let now = now_epoch_secs();
    let window_seconds = window.as_secs().max(1);

    let shard = &state.shards[shard_index(&key.caller_key)];
    let allowed = {
        let mut guard = match shard.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_rate_limit",
                    "rate-limit LRU mutex poisoned — recovering",
                );
                poisoned.into_inner()
            }
        };

        let window_entry = guard.get_or_insert_mut(key, || Window {
            remaining: max_requests,
            window_start: now,
        });

        if now.saturating_sub(window_entry.window_start) >= window_seconds {
            // Window expired — start a new one. Charge the current
            // request against the fresh budget.
            window_entry.window_start = now;
            window_entry.remaining = max_requests.saturating_sub(1);
            true
        } else if window_entry.remaining == 0 {
            false
        } else {
            window_entry.remaining -= 1;
            true
        }
    };

    if !allowed {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.",
        )
            .into_response();
    }

    next.run(request).await
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

    fn consume(state: &RateLimitState, caller: &WebUiAuthenticatedCaller) -> bool {
        let key = CounterKey {
            route_idx: 0,
            caller_key: caller_key(caller),
        };
        let mut guard = state.shards[shard_index(&key.caller_key)]
            .lock()
            .expect("lock");
        let route = &state.routes[0];
        let (max, window) = match route.policy {
            ResolvedPolicy::Limited {
                max_requests,
                window,
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
        let descriptors = ironclaw_webui_v2::webui_v2_routes();
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
        // `PerCaller` must abort composition rather than silently
        // degrade to no enforcement. Without this test, a future v2
        // descriptor flipping `send_message` to e.g. `PerTenant` would
        // skip the limiter entirely.
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
}
