//! Refund-specific regression tests for PR #6592's `mark_rate_limit_refundable`
//! / `refund_charge` mechanism, split out of `webui_rate_limit.rs`'s
//! `mod tests` (Finding C6) to keep that file under the repo's 1000-line
//! decomposition threshold. Intentionally NOT under `tests/` — like
//! `webui_rate_limit_router_contract_test.rs` (see that file's module doc
//! for the full rationale), this is a unit-level test module that exercises
//! `pub(crate)`-only middleware internals (`RateLimitState`, `RouteLimit`,
//! `ResolvedPolicy`, `refund_charge`, `enforce_rate_limit`); moving it to
//! `tests/` would force exporting those internals just for this suite.

use super::tests::{caller, limited_state};
use super::*;

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
    // entry whose generation differs from what the (stale) caller
    // believes it charged — e.g. the window rolled over since, minting
    // a fresh generation for the new window instance.
    {
        let mut guard = shard.lock().expect("lock");
        guard.get_or_insert_mut(key.clone(), || Window {
            remaining: 1,
            window_start: 1_000,
            generation: 7,
        });
    }
    let stale_charged_generation = 3; // does not match the live entry's generation (7)

    refund_charge(shard, &key, stale_charged_generation, 3);

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
    refund_charge(shard, &key, 0, 3);

    let guard = shard.lock().expect("lock");
    assert!(
        guard.peek(&key).is_none(),
        "refund must not create an entry for an uncharged/evicted key"
    );
}

/// Regression for PR #6592 review comment ("Refund can credit a
/// replacement entry after LRU eviction"): a plain `window_start`
/// equality check is only second-resolution. If the original charge's
/// entry is evicted from the shard's LRU (other callers on the same
/// shard churning it out under load) and the SAME caller then makes a
/// brand-new, unrelated request within the same wall-clock second, the
/// replacement entry's `window_start` can coincidentally match the
/// original charge's `window_start` — so a `window_start`-only guard
/// would let a delayed refund for the evicted charge incorrectly credit
/// the unrelated replacement entry. The per-entry `generation` token
/// is unique per (re)creation regardless of any `window_start`
/// coincidence, so it must not be fooled by this.
#[test]
fn refund_charge_does_not_credit_replacement_entry_after_eviction_same_second() {
    let state = limited_state(3, 60);
    let alice = caller("tenant-alpha", "alice");
    let key = CounterKey {
        route_idx: 0,
        bucket_key: caller_key(&alice),
    };
    let shard = &state.shards[shard_index(&key.bucket_key)];
    let window_start = 1_000;

    // Original charge: insert alice's entry at generation 1, remember
    // that generation as `refund_charge` would (this is exactly
    // `charged_generation` in `enforce_rate_limit`).
    {
        let mut guard = shard.lock().expect("lock");
        guard.get_or_insert_mut(key.clone(), || Window {
            remaining: 2,
            window_start,
            generation: 1,
        });
    }
    let charged_generation = 1;

    // Simulate LRU eviction of alice's entry (other callers on the same
    // shard churning through their 512-entry cap under load), then a
    // brand-new, unrelated request from alice within the same second
    // reinserting a fresh entry — same `window_start` by coincidence,
    // but a new, distinct `generation` (2).
    {
        let mut guard = shard.lock().expect("lock");
        guard.pop(&key);
        guard.get_or_insert_mut(key.clone(), || Window {
            remaining: 1,
            window_start,
            generation: 2,
        });
    }

    // The delayed refund for the ORIGINAL (now-evicted, generation 1)
    // charge arrives late and must not credit the unrelated
    // replacement entry (generation 2) it happens to collide with on
    // `window_start`.
    refund_charge(shard, &key, charged_generation, 3);

    let guard = shard.lock().expect("lock");
    let entry = guard.peek(&key).expect("replacement entry still present");
    assert_eq!(
        entry.remaining, 1,
        "refund for an evicted charge must not credit an unrelated \
         replacement entry that coincidentally shares window_start"
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
    let generation = 42;

    {
        let mut guard = shard.lock().expect("lock");
        guard.get_or_insert_mut(key.clone(), || Window {
            remaining: 3,
            window_start,
            generation,
        });
    }

    // Two refunds for the same charge (a duplicate/racing refund)
    // must not push `remaining` past `max_requests`.
    refund_charge(shard, &key, generation, 3);
    refund_charge(shard, &key, generation, 3);

    let guard = shard.lock().expect("lock");
    let entry = guard.peek(&key).expect("entry still present");
    assert_eq!(
        entry.remaining, 3,
        "refund must not credit a window past max_requests"
    );
}
