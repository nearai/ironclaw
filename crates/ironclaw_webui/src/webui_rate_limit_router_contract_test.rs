//! Caller-level contract test for PR #6592 review comment "Missing
//! production test for refundable SSE capacity 429s". Split into its
//! own file to keep webui_rate_limit.rs under the repo's 1000-line
//! decomposition threshold — this single test's minimal RebornServicesApi
//! fake accounts for most of its size.
use super::tests::caller;
use super::*;

/// Caller-level contract test for the PR #6592 review comment "Missing
/// production test for refundable SSE capacity 429s". The two
/// `refund_test_app` tests above front `enforce_rate_limit` with a
/// synthetic handler that always 429s, and the `SseCapacity` cap tests
/// (`webui_v2::handlers` contract suite) inject the caller `Extension`
/// directly, bypassing this middleware entirely. Neither proves that
/// exhausting the SSE per-caller concurrency cap through the real,
/// fully-wired route — `enforce_rate_limit` in front of the real
/// `stream_events` handler and a real `SseCapacity` — actually refunds
/// the caller's rate-limit budget end to end. This test drives that
/// exact combination.
#[tokio::test]
async fn sse_capacity_429_through_real_stream_events_handler_is_refunded() {
    use crate::webui_v2::{WebUiV2State, webui_v2_router};
    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use axum::middleware;
    use ironclaw_product_workflow::{
        LifecyclePackageRef, RebornCancelRunResponse, RebornCreateThreadResponse,
        RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
        RebornExtensionListResponse, RebornExtensionRegistryResponse, RebornGetRunStateRequest,
        RebornGetRunStateResponse, RebornListAutomationsResponse, RebornListThreadsResponse,
        RebornOutboundDeliveryTargetListResponse, RebornOutboundPreferencesResponse,
        RebornResolveGateResponse, RebornRetryRunResponse, RebornServicesApi, RebornServicesError,
        RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse,
        RebornSkillActionResponse, RebornSkillContentResponse, RebornSkillListResponse,
        RebornSkillSearchResponse, RebornStreamEventsRequest, RebornStreamEventsResponse,
        RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
        WebUiCancelRunRequest, WebUiCreateThreadRequest, WebUiListAutomationsRequest,
        WebUiListThreadsRequest, WebUiResolveGateRequest, WebUiRetryRunRequest,
        WebUiSendMessageRequest, WebUiSetupExtensionRequest,
    };
    use tower::ServiceExt;

    /// Minimal `RebornServicesApi` fake. Only `stream_events` needs a
    /// real body: the SSE capacity slot is reserved synchronously at
    /// the top of the `stream_events` handler before the facade is
    /// ever touched, so every other method is unreachable for this
    /// test and panics loudly if that ever changes.
    #[derive(Default)]
    struct FakeServices;

    #[async_trait]
    impl RebornServicesApi for FakeServices {
        async fn create_thread(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiCreateThreadRequest,
        ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
            unreachable!("test does not drive create_thread")
        }

        async fn submit_turn(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiSendMessageRequest,
        ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
            unreachable!("test does not drive submit_turn")
        }

        async fn delete_thread(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornDeleteThreadRequest,
        ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
            unreachable!("test does not drive delete_thread")
        }

        async fn get_timeline(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornTimelineRequest,
        ) -> Result<RebornTimelineResponse, RebornServicesError> {
            unreachable!("test does not drive get_timeline")
        }

        async fn stream_events(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornStreamEventsRequest,
        ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
            // Returns instantly with an empty page — no backing
            // projection store is needed because this test never
            // drains the SSE body, only the handshake status.
            Ok(RebornStreamEventsResponse { events: Vec::new() })
        }

        async fn cancel_run(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiCancelRunRequest,
        ) -> Result<RebornCancelRunResponse, RebornServicesError> {
            unreachable!("test does not drive cancel_run")
        }

        async fn resolve_gate(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiResolveGateRequest,
        ) -> Result<RebornResolveGateResponse, RebornServicesError> {
            unreachable!("test does not drive resolve_gate")
        }

        async fn retry_run(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiRetryRunRequest,
        ) -> Result<RebornRetryRunResponse, RebornServicesError> {
            unreachable!("test does not drive retry_run")
        }

        async fn get_run_state(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornGetRunStateRequest,
        ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
            unreachable!("test does not drive get_run_state")
        }

        async fn list_threads(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiListThreadsRequest,
        ) -> Result<RebornListThreadsResponse, RebornServicesError> {
            unreachable!("test does not drive list_threads")
        }

        async fn list_automations(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: WebUiListAutomationsRequest,
        ) -> Result<RebornListAutomationsResponse, RebornServicesError> {
            unreachable!("test does not drive list_automations")
        }

        async fn get_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            unreachable!("test does not drive get_outbound_preferences")
        }

        async fn set_outbound_preferences(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _request: RebornSetOutboundPreferencesRequest,
        ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
            unreachable!("test does not drive set_outbound_preferences")
        }

        async fn list_outbound_delivery_targets(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
            unreachable!("test does not drive list_outbound_delivery_targets")
        }

        async fn list_extensions(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornExtensionListResponse, RebornServicesError> {
            unreachable!("test does not drive list_extensions")
        }

        async fn list_skills(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornSkillListResponse, RebornServicesError> {
            unreachable!("test does not drive list_skills")
        }

        async fn search_skills(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _query: String,
        ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
            unreachable!("test does not drive search_skills")
        }

        async fn install_skill(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _name: String,
            _content: Option<String>,
        ) -> Result<RebornSkillActionResponse, RebornServicesError> {
            unreachable!("test does not drive install_skill")
        }

        async fn read_skill_content(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _name: String,
        ) -> Result<RebornSkillContentResponse, RebornServicesError> {
            unreachable!("test does not drive read_skill_content")
        }

        async fn update_skill(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _name: String,
            _content: String,
        ) -> Result<RebornSkillActionResponse, RebornServicesError> {
            unreachable!("test does not drive update_skill")
        }

        async fn remove_skill(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _name: String,
        ) -> Result<RebornSkillActionResponse, RebornServicesError> {
            unreachable!("test does not drive remove_skill")
        }

        async fn list_extension_registry(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
            unreachable!("test does not drive list_extension_registry")
        }

        async fn install_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _package_ref: LifecyclePackageRef,
        ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
            unreachable!("test does not drive install_extension")
        }

        async fn activate_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _package_ref: LifecyclePackageRef,
        ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
            unreachable!("test does not drive activate_extension")
        }

        async fn remove_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _package_ref: LifecyclePackageRef,
        ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
            unreachable!("test does not drive remove_extension")
        }

        async fn setup_extension(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _package_ref: LifecyclePackageRef,
            _request: WebUiSetupExtensionRequest,
        ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
            unreachable!("test does not drive setup_extension")
        }
    }

    // Rate-limit budget deliberately smaller than the number of
    // SseCapacity rejections fired below: if those refundable 429s were
    // NOT actually refunded, `enforce_rate_limit` itself would start
    // rejecting before this test's final request, which would mask the
    // real assertion.
    let stream_events_route = RouteLimit {
        route_id: "webui_v2.stream_events".into(),
        method: Method::GET,
        segments: parse_pattern("/api/webchat/v2/threads/{thread_id}/events"),
        policy: ResolvedPolicy::Limited {
            scope: RateLimitScope::PerCaller,
            max_requests: 2,
            window: Duration::from_secs(60),
        },
    };
    let shards = (0..SHARD_COUNT)
        .map(|_| Mutex::new(LruCache::new(RATE_LIMIT_PER_SHARD_CAPACITY)))
        .collect::<Vec<_>>();
    let rate_limit_state = RateLimitState {
        routes: Arc::new(vec![stream_events_route]),
        shards: Arc::new(shards),
        next_generation: Arc::new(AtomicU64::new(0)),
    };

    // Real `WebUiV2State` + the real router (real `stream_events`
    // handler, real `SseCapacity`) with a per-caller concurrency cap
    // of 1, wired exactly as `webui_serve::webui_v2_app_with_lifecycle`
    // wires production: `enforce_rate_limit` in front of the v2 route
    // set. Auth is bypassed by stamping the caller `Extension` directly
    // on each request, matching this crate's other handler contract
    // tests — the regression target here is the rate-limit ↔
    // SseCapacity interaction, not the bearer-auth middleware.
    let services: Arc<dyn RebornServicesApi> = Arc::new(FakeServices);
    let app = webui_v2_router(WebUiV2State::new(services, 1)).route_layer(
        middleware::from_fn_with_state(rate_limit_state, enforce_rate_limit),
    );

    let alice = caller("tenant-alpha", "alice");
    let open_request = || {
        let mut request = HttpRequest::builder()
            .method(Method::GET)
            .uri("/api/webchat/v2/threads/thread-x/events")
            .body(Body::empty())
            .expect("request");
        request.extensions_mut().insert(alice.clone());
        request
    };

    // First open succeeds and reserves the caller's only SseCapacity
    // slot. Hold the response alive so the slot stays reserved for the
    // rest of the test — the slot guard lives inside the SSE body.
    let held = app.clone().oneshot(open_request()).await.expect("oneshot");
    assert_eq!(held.status(), StatusCode::OK);

    // Fire more capacity-rejected opens than the rate-limit budget (2)
    // allows. Each must be `SseCapacity`'s own refundable 429 — a JSON
    // body `{"error":"rate_limited","kind":"busy",...}` from
    // `sse_capacity_rejected()` — not the middleware's own plain-text
    // "Rate limit exceeded" 429, which would mean the limiter itself
    // short-circuited before the real handler ran.
    for attempt in 0..5 {
        let rejected = app.clone().oneshot(open_request()).await.expect("oneshot");
        assert_eq!(
            rejected.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "attempt {attempt} must hit the real SseCapacity cap"
        );
        let body = to_bytes(rejected.into_body(), usize::MAX)
            .await
            .expect("read rejected body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(
            json["error"], "rate_limited",
            "attempt {attempt} must be SseCapacity's own 429 body, not the rate limiter's"
        );
        assert_eq!(json["kind"], "busy");
    }

    // Release the held slot; the `SseSlot` guard's Drop runs
    // synchronously, but yield once so any pending wakers settle.
    drop(held);
    tokio::task::yield_now().await;

    // A fresh open must succeed. If the five refundable SseCapacity
    // 429s above had actually drained the (max_requests = 2)
    // rate-limit budget, `enforce_rate_limit` would reject this
    // request itself before it ever reached the handler.
    let recovered = app.clone().oneshot(open_request()).await.expect("oneshot");
    assert_eq!(
        recovered.status(),
        StatusCode::OK,
        "refundable SseCapacity 429s through the real router must not have \
             consumed the caller's rate-limit budget"
    );
}
