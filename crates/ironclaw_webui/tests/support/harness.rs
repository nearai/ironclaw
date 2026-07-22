//! Shared test harness for the `#3615` WebChat v2 security-parity audit
//! suite (`auth_route_contract.rs`, `headers_errors_contract.rs`,
//! `network_limits_contract.rs`).
//!
//! Each of those files used to carry its own byte-identical copy of the
//! ~230-line `StubServices` `RebornServicesApi` impl plus the shared
//! `with_peer` helper and tenant/agent/project constants. They are
//! consolidated here and pulled in via
//! `#[path = "support/harness.rs"] mod harness;` so a new
//! `RebornServicesApi` method only has to be stubbed once. This file is
//! NOT a test binary (it lives under `tests/support/`), and it is
//! deliberately not referenced from `support/mod.rs`, so the OAuth-route
//! tests' `mod support;` does not compile it.
//!
//! `#![allow(dead_code)]` is the standard shared-test-module idiom: not
//! every including binary exercises every helper or reads every
//! `StubServices` field, and that asymmetry is expected, not a smell.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Mutex;

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    RebornCancelRunResponse, RebornCreateThreadResponse, RebornDeleteThreadRequest,
    RebornDeleteThreadResponse, RebornGetRunStateRequest, RebornGetRunStateResponse,
    RebornResolveGateResponse, RebornRetryRunResponse, RebornServicesApi, RebornServicesError,
    RebornStreamEventsRequest, RebornStreamEventsResponse, RebornSubmitTurnResponse,
    RebornTimelineRequest, RebornTimelineResponse, WebUiAuthenticatedCaller, WebUiCancelRunRequest,
    WebUiCreateThreadRequest, WebUiResolveGateRequest, WebUiRetryRunRequest,
    WebUiSendMessageRequest,
};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};

/// Host-installation tenant the audit apps are composed with.
pub const TENANT: &str = "tenant-a";
/// Default agent stamped onto every authenticated caller.
pub const AGENT: &str = "agent-default";
/// Default project stamped onto every authenticated caller.
pub const PROJECT: &str = "project-default";

/// `RebornServicesApi` stub for the audit suite. `create_thread` and
/// `stream_events` record their callers so a test can assert the facade
/// was (or was not) reached and which `UserId` the bearer / `?token=`
/// resolved to; `list_threads` returns an empty page defensively; every
/// other method panics or rejects so an accidental call surfaces loudly
/// rather than masking a routing regression. When `create_thread_panic`
/// is set, `create_thread` panics with that message so the
/// `CatchPanicLayer` boundary can be driven.
#[derive(Default)]
pub struct StubServices {
    pub create_thread_callers: Mutex<Vec<WebUiAuthenticatedCaller>>,
    pub stream_events_callers: Mutex<Vec<WebUiAuthenticatedCaller>>,
    pub create_thread_panic: Option<&'static str>,
}

#[async_trait]
impl RebornServicesApi for StubServices {
    async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        _request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        if let Some(message) = self.create_thread_panic {
            panic!("{message}");
        }
        self.create_thread_callers
            .lock()
            .expect("lock")
            .push(caller);
        Ok(RebornCreateThreadResponse {
            thread: SessionThreadRecord {
                thread_id: ThreadId::new("thread.fake").expect("thread"),
                scope: ThreadScope {
                    tenant_id: TenantId::new(TENANT).expect("tenant"),
                    agent_id: AgentId::new("agent.fake").expect("agent"),
                    project_id: Some(ProjectId::new("project.fake").expect("project")),
                    owner_user_id: Some(UserId::new("alice@example.com").expect("user")),
                    mission_id: None,
                },
                created_by_actor_id: "alice@example.com".to_string(),
                title: None,
                metadata_json: None,
                goal: None,
                created_at: None,
                updated_at: None,
            },
        })
    }

    async fn submit_turn(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        unreachable!("audit suite does not drive submit_turn")
    }

    async fn get_timeline(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        unreachable!("audit suite does not drive get_timeline")
    }

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        // Record the caller so the `?token=` shim test can assert the
        // query token was consumed as the session credential and stamped
        // as that user. Returns an empty event page so a polled SSE
        // stream reaches 200 without panicking; the concurrency slot is
        // acquired at handler entry regardless of stream contents.
        self.stream_events_callers
            .lock()
            .expect("lock")
            .push(caller);
        Ok(RebornStreamEventsResponse { events: Vec::new() })
    }

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        unreachable!("audit suite does not drive get_run_state")
    }

    async fn cancel_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        unreachable!("audit suite does not drive cancel_run")
    }

    async fn retry_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiRetryRunRequest,
    ) -> Result<RebornRetryRunResponse, RebornServicesError> {
        unreachable!("audit suite does not drive retry_run")
    }

    async fn resolve_gate(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        unreachable!("audit suite does not drive resolve_gate")
    }

    async fn delete_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornDeleteThreadRequest,
    ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
        unreachable!("audit suite does not drive delete_thread")
    }
}

/// Tag a request with a specific peer address. The per-IP rate limiter
/// keys on the peer **IP** (port is ignored), so tests that need
/// distinct buckets must vary the IP octets, not just the port. Host
/// composition injects this via `into_make_service_with_connect_info`;
/// the `oneshot` harness has to do it explicitly.
pub fn with_peer_addr(mut req: Request<Body>, addr: SocketAddr) -> Request<Body> {
    req.extensions_mut().insert(ConnectInfo(addr));
    req
}

/// Tag a request with `ConnectInfo` so descriptor-driven middleware
/// (e.g. the PerIp rate limit) can resolve a peer address. Default fixed
/// peer so a test keys every request to the same bucket.
pub fn with_peer(req: Request<Body>) -> Request<Body> {
    with_peer_addr(req, SocketAddr::from(([127, 0, 0, 1], 1234)))
}
