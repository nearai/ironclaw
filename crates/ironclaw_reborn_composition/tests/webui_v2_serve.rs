//! Caller-level tests for the Reborn-owned WebChat v2 HTTP gateway
//! composition (`webui_serve`).
//!
//! These tests drive [`webui_v2_app`] through `tower::ServiceExt::oneshot`
//! so the middleware stack — bearer auth, `?token=` shim for SSE,
//! CORS, body limit, static security headers — is exercised end-to-end
//! against the same axum `Router` `serve_webui_v2` binds at runtime.
//! No TCP listener and no real Reborn runtime are required; the v2
//! facade is mocked so the regression target stays the gateway-layer
//! composition.

use std::sync::{Arc, Mutex};
use std::time::Duration;

// arch-exempt: large_file, root WebUI caller regressions stay with the shared composed-router fixture, plan #5905
use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use ironclaw_host_api::{
    ActivityId, AgentId, NetworkMethod, Outcome, OutcomeRefs, ProductSurfaceCaller,
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind, ProjectId, Resolution,
    ResultPreviewMeta, ResultProgress, ResultRef, SafeSummary, TenantId, TerminateHint, ThreadId,
    ToolVerdict, UserId,
};
use ironclaw_product::{
    EXTENSION_SETUP_SUBMIT_CAPABILITY_ID, EXTENSION_SETUP_VIEW, LifecyclePackageKind,
    LifecyclePackageRef, LifecyclePublicState, ProductCreateThreadRequest,
    ProductListThreadsRequest, ProductResolveGateRequest, ProductSubmitTurnRequest,
    RebornCancelRunResponse, RebornCreateThreadResponse, RebornDeleteThreadRequest,
    RebornListThreadsResponse, RebornSetupExtensionResponse, RebornSubmitTurnResponse,
    RebornTimelineResponse, RebornTraceCreditsResponse, RebornViewQuery,
    THREAD_DELETE_CAPABILITY_ID, THREADS_VIEW, TIMELINE_VIEW, TRACE_CREDITS_VIEW,
};
use ironclaw_reborn_composition::{PublicRouteMount, RebornReadiness, RebornWebuiBundle};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};
use ironclaw_turns::{EventCursor, RunProfileId, RunProfileVersion, TurnRunId, TurnStatus};
use ironclaw_webui::{
    WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig, WebuiServeError, webui_v2_app,
};
use serde_json::json;
use tower::ServiceExt;

const TENANT: &str = "tenant-alpha";
const USER: &str = "user-alpha";
const VALID_TOKEN: &str = "valid-bearer-token";

fn public_test_descriptor(
    route_id: &str,
    route_pattern: &str,
) -> ironclaw_host_api::ingress::IngressRouteDescriptor {
    use ironclaw_host_api::IngressScopeSource;
    use ironclaw_host_api::ingress::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
        IngressJustification, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
        ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
    };
    use std::num::NonZeroU32;

    IngressRouteDescriptor::new(
        route_id,
        NetworkMethod::Get,
        route_pattern,
        IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::LocalGateway,
            auth: IngressAuthPolicy::Public {
                justification: IngressJustification::new("test public", "regression test")
                    .expect("justification"),
            },
            scope_source: IngressScopeSource::PublicRoute,
            body_limit: BodyLimitPolicy::NoBody,
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::PerIp,
                max_requests: NonZeroU32::new(120).expect("120 != 0"),
                window_seconds: NonZeroU32::new(60).expect("60 != 0"),
            },
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::NoEffect,
        })
        .expect("policy"),
    )
    .expect("descriptor")
}

fn compose_with_public_descriptor(
    route_id: &str,
    route_pattern: &str,
) -> Result<axum::Router, WebuiServeError> {
    let bundle = RebornWebuiBundle {
        product_surface: Arc::new(StubServices::default()),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let descriptor = public_test_descriptor(route_id, route_pattern);
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
    .with_public_route_mount(PublicRouteMount::new(axum::Router::new(), vec![descriptor]));

    webui_v2_app(bundle, config)
}

// ─── stubs ────────────────────────────────────────────────────────────

/// `WebuiAuthenticator` accepting only [`VALID_TOKEN`].
struct OnlyValidToken;

#[async_trait]
impl WebuiAuthenticator for OnlyValidToken {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        if token == VALID_TOKEN {
            Some(WebuiAuthentication::operator(
                UserId::new(USER).expect("user id"),
            ))
        } else {
            None
        }
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        true
    }
}

struct MultiUserToken;

#[async_trait]
impl WebuiAuthenticator for MultiUserToken {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        if token == VALID_TOKEN {
            Some(WebuiAuthentication::user(
                UserId::new(USER).expect("user id"),
            ))
        } else {
            None
        }
    }
}

/// `WebuiAuthenticator` resolving [`VALID_TOKEN`] to a fixed,
/// test-supplied user id. The trace-credits tests use it so the
/// authenticated caller's user id equals a unique per-test trace
/// scope — the facade derives the scope from the caller only.
struct FixedUserToken {
    user_id: String,
}

#[async_trait]
impl WebuiAuthenticator for FixedUserToken {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        if token == VALID_TOKEN {
            Some(WebuiAuthentication::operator(
                UserId::new(self.user_id.as_str()).expect("user id"),
            ))
        } else {
            None
        }
    }
}

fn successful_resolution(activity_id: ActivityId) -> Resolution {
    Resolution::Done(Outcome {
        refs: OutcomeRefs {
            result: ResultRef::from_uuid(activity_id.as_uuid()),
            byte_len: 0,
            preview: None,
            preview_meta: ResultPreviewMeta::default(),
            origin: None,
            output_digest: None,
        },
        verdict: ToolVerdict::Success,
        summary: SafeSummary::new("ok").expect("static summary is redaction-safe"),
        progress: ResultProgress::MadeProgress,
        terminate_hint: TerminateHint::Continue,
    })
}

fn trace_credits_response(caller: &ProductSurfaceCaller) -> RebornTraceCreditsResponse {
    let scope = ironclaw_reborn_traces::contribution::trace_scope_key(
        caller.tenant_id.as_str(),
        caller.user_id.as_str(),
    );
    let enrolled =
        ironclaw_reborn_traces::contribution::read_trace_policy_for_scope(Some(scope.as_str()))
            .map(|policy| policy.enabled)
            .unwrap_or(false);
    RebornTraceCreditsResponse {
        enrolled,
        pending_credit: 0.0,
        final_credit: 0.0,
        delayed_credit_delta: 0.0,
        submissions_total: 0,
        submissions_submitted: 0,
        submissions_accepted: 0,
        submissions_revoked: 0,
        submissions_expired: 0,
        credit_events_total: 0,
        last_submission_at: None,
        last_credit_sync_at: None,
        recent_explanations: Vec::new(),
        manual_review_hold_count: 0,
        holds: Vec::new(),
        note: "Local view as of last sync; authoritative ledger is server-side.".to_string(),
    }
}

fn extension_setup_response(package_ref: LifecyclePackageRef) -> RebornSetupExtensionResponse {
    RebornSetupExtensionResponse {
        package_ref,
        phase: LifecyclePublicState::SetupNeeded,
        blockers: Vec::new(),
        payload: None,
        secrets: Vec::new(),
        onboarding: None,
    }
}

#[tokio::test]
async fn health_route_is_public_for_platform_probes() {
    let bundle = RebornWebuiBundle {
        product_surface: Arc::new(StubServices::default()),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![],
    );
    let app = webui_v2_app(bundle, config).expect("webui v2 app");

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.expect("body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("health json");
    assert_eq!(json["status"], "healthy");
    assert_eq!(json["channel"], "reborn");
}

mod openai_compat_mount_tests {
    use super::*;
    use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
    use ironclaw_reborn_composition::ProtectedRouteMount;
    use ironclaw_reborn_openai_compat::{
        FilesystemOpenAiCompatRefStore, OpenAiChatCompletionProjection,
        OpenAiChatCompletionProjectionReader, OpenAiChatCompletionProjectionRequest,
        OpenAiChatCompletionsWorkflow, OpenAiCompatRouterState, OpenAiResponseId,
        OpenAiResponseObject, OpenAiResponseOutputItem, OpenAiResponseOutputItemStatus,
        OpenAiResponseProjection, OpenAiResponseReadRequest, OpenAiResponseStatus,
        OpenAiResponseWaitRequest, OpenAiResponsesMessageRole, OpenAiResponsesProjectionReader,
        OpenAiResponsesWorkflow, openai_compat_router_with_state, openai_compat_routes,
    };
    use ironclaw_turns::runner::{ClaimRunRequest, CompleteRunRequest, TurnRunTransitionPort};
    use ironclaw_turns::test_support::in_memory_turn_state_store;
    use ironclaw_turns::{
        AcceptedMessageRef, DefaultTurnCoordinator, IdempotencyKey, ReplyTargetBindingRef,
        SourceBindingRef, StaticTurnAdmissionLimitProvider, SubmitTurnRequest,
        TurnAdmissionAxisKind, TurnCoordinator, TurnError, TurnLeaseToken, TurnRunId, TurnRunnerId,
    };

    const AGENT: &str = "agent-alpha";
    const PROJECT: &str = "project-alpha";
    const THREAD: &str = "thread-openai-chat";

    fn in_memory_openai_compat_ref_store() -> Arc<FilesystemOpenAiCompatRefStore> {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        Arc::new(FilesystemOpenAiCompatRefStore::new(filesystem))
    }

    #[tokio::test]
    async fn openai_chat_completions_mount_uses_webui_auth_and_product_surface() {
        let workflow = Arc::new(GatewayOpenAiSurface::default());
        let chat = Arc::new(OpenAiChatCompletionsWorkflow::new(
            workflow.clone(),
            in_memory_openai_compat_ref_store(),
            Arc::new(StaticChatProjectionReader::text(
                "hello through composition",
            )),
        ));
        let mount = ProtectedRouteMount::new(
            openai_compat_router_with_state(OpenAiCompatRouterState::with_chat_completions(chat)),
            openai_compat_routes(),
        );
        let bundle = RebornWebuiBundle {
            product_surface: Arc::new(StubServices::default()),
            product_auth: None,
            readiness: RebornReadiness::disabled(),
        };
        let config = WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OnlyValidToken),
            vec![HeaderValue::from_static("http://localhost:3000")],
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_protected_route_mount(mount);
        let app = webui_v2_app(bundle, config).expect("webui v2 app");

        let unauthenticated = app
            .clone()
            .oneshot(chat_request(None))
            .await
            .expect("oneshot");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(workflow.submit_count(), 0);

        let authenticated = app
            .oneshot(chat_request(Some(VALID_TOKEN)))
            .await
            .expect("oneshot");
        assert_eq!(authenticated.status(), StatusCode::OK);
        let body = to_bytes(authenticated.into_body(), 4096)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            body["choices"][0]["message"]["content"],
            "hello through composition"
        );
        assert_eq!(workflow.submit_count(), 1);
    }

    #[tokio::test]
    async fn openai_chat_timeout_keeps_shared_turn_admission_until_terminal_release() {
        let limits = StaticTurnAdmissionLimitProvider::default()
            .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
        let turn_state =
            Arc::new(in_memory_turn_state_store().with_admission_limit_provider(Arc::new(limits)));
        let turn_coordinator = Arc::new(DefaultTurnCoordinator::new(turn_state.clone()));
        let workflow = Arc::new(AdmissionProductSurface::new(turn_coordinator));
        let chat = Arc::new(
            OpenAiChatCompletionsWorkflow::new(
                workflow,
                in_memory_openai_compat_ref_store(),
                Arc::new(NeverCompletingChatProjectionReader),
            )
            .with_wait_timeout(Duration::from_millis(1)),
        );
        let mount = ProtectedRouteMount::new(
            openai_compat_router_with_state(OpenAiCompatRouterState::with_chat_completions(chat)),
            openai_compat_routes(),
        );
        let bundle = RebornWebuiBundle {
            product_surface: Arc::new(StubServices::default()),
            product_auth: None,
            readiness: RebornReadiness::disabled(),
        };
        let config = WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OnlyValidToken),
            vec![HeaderValue::from_static("http://localhost:3000")],
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_protected_route_mount(mount);
        let app = webui_v2_app(bundle, config).expect("webui v2 app");

        let timed_out = app
            .clone()
            .oneshot(chat_request(Some(VALID_TOKEN)))
            .await
            .expect("timed-out chat response");
        assert_eq!(timed_out.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            turn_state
                .active_admission_reservations()
                .await
                .unwrap()
                .len(),
            1
        );

        let denied = app
            .clone()
            .oneshot(chat_request(Some(VALID_TOKEN)))
            .await
            .expect("admission-denied chat response");
        assert_eq!(denied.status(), StatusCode::TOO_MANY_REQUESTS);
        let denied_body = to_bytes(denied.into_body(), 4096)
            .await
            .expect("denied body");
        let denied_body: serde_json::Value =
            serde_json::from_slice(&denied_body).expect("denied json");
        assert_eq!(denied_body["error"]["code"], "rate_limited");
        assert_eq!(
            turn_state
                .active_admission_reservations()
                .await
                .unwrap()
                .len(),
            1
        );

        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        let claimed = turn_state
            .claim_next_run(ClaimRunRequest {
                runner_id,
                lease_token,
                scope_filter: None,
            })
            .await
            .expect("claim active run")
            .expect("active run should be claimable");
        turn_state
            .complete_run(CompleteRunRequest {
                run_id: claimed.state.run_id,
                runner_id,
                lease_token,
            })
            .await
            .expect("complete active run");
        assert!(
            turn_state
                .active_admission_reservations()
                .await
                .unwrap()
                .is_empty()
        );

        let accepted_after_release = app
            .oneshot(chat_request(Some(VALID_TOKEN)))
            .await
            .expect("chat response after release");
        assert_eq!(
            accepted_after_release.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "the route still times out waiting for projection, but admission accepted a new turn"
        );
        assert_eq!(
            turn_state
                .active_admission_reservations()
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn openai_responses_mount_uses_webui_auth_and_product_surface() {
        let workflow = Arc::new(GatewayOpenAiSurface::default());
        let responses = Arc::new(OpenAiResponsesWorkflow::new(
            workflow.clone(),
            in_memory_openai_compat_ref_store(),
            Arc::new(StaticResponsesProjectionReader::text(
                "hello through responses",
            )),
        ));
        let mount = ProtectedRouteMount::new(
            openai_compat_router_with_state(OpenAiCompatRouterState::with_responses(responses)),
            openai_compat_routes(),
        );
        let bundle = RebornWebuiBundle {
            product_surface: Arc::new(StubServices::default()),
            product_auth: None,
            readiness: RebornReadiness::disabled(),
        };
        let config = WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OnlyValidToken),
            vec![HeaderValue::from_static("http://localhost:3000")],
        )
        .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
        .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
        .with_protected_route_mount(mount);
        let app = webui_v2_app(bundle, config).expect("webui v2 app");

        let unauthenticated = app
            .clone()
            .oneshot(response_request(None))
            .await
            .expect("oneshot");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(workflow.submit_count(), 0);

        let authenticated = app
            .oneshot(response_request(Some(VALID_TOKEN)))
            .await
            .expect("oneshot");
        assert_eq!(authenticated.status(), StatusCode::OK);
        let body = to_bytes(authenticated.into_body(), 4096)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            body["output"][0]["content"][0]["text"],
            "hello through responses"
        );
        assert_eq!(workflow.submit_count(), 1);
    }

    fn chat_request(token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/v1/chat/completions")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder
            .body(Body::from(
                json!({
                    "model": "reborn-test",
                    "messages": [{"role": "user", "content": "hello"}]
                })
                .to_string(),
            ))
            .expect("request")
    }

    fn response_request(token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/responses")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder
            .body(Body::from(
                json!({
                    "model": "reborn-test",
                    "input": "hello"
                })
                .to_string(),
            ))
            .expect("request")
    }

    #[derive(Default)]
    struct GatewayOpenAiSurface {
        submit_count: Mutex<usize>,
    }

    impl GatewayOpenAiSurface {
        fn submit_count(&self) -> usize {
            *self
                .submit_count
                .lock()
                .expect("submit count lock should not be poisoned")
        }
    }

    #[async_trait]
    impl ironclaw_host_api::ProductSurface for GatewayOpenAiSurface {
        async fn invoke(
            &self,
            caller: ProductSurfaceCaller,
            request: ironclaw_host_api::ProductSurfaceInvokeRequest,
        ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
            let output = match request.operation_id.as_str() {
                "thread.create" => {
                    let input: ProductCreateThreadRequest =
                        serde_json::from_value(request.input)
                            .map_err(ProductSurfaceError::internal_from)?;
                    let thread_id = ThreadId::new(
                        input
                            .requested_thread_id
                            .or(input.client_action_id)
                            .unwrap_or_else(|| THREAD.to_string()),
                    )
                    .map_err(ProductSurfaceError::internal_from)?;
                    serde_json::to_value(RebornCreateThreadResponse {
                        thread: test_thread_record(caller, thread_id),
                    })
                    .map_err(ProductSurfaceError::internal_from)?
                }
                "turn.submit" => {
                    let input: ProductSubmitTurnRequest = serde_json::from_value(request.input)
                        .map_err(ProductSurfaceError::internal_from)?;
                    *self
                        .submit_count
                        .lock()
                        .expect("submit count lock should not be poisoned") += 1;
                    let thread_id =
                        ThreadId::new(input.thread_id.unwrap_or_else(|| THREAD.to_string()))
                            .map_err(ProductSurfaceError::internal_from)?;
                    serde_json::to_value(RebornSubmitTurnResponse::Submitted {
                        thread_id,
                        accepted_message_ref: AcceptedMessageRef::new("msg:openai-chat")
                            .map_err(ProductSurfaceError::internal_from)?,
                        turn_id: "turn-openai-chat".to_string(),
                        run_id: TurnRunId::new(),
                        status: TurnStatus::Queued,
                        resolved_run_profile_id: RunProfileId::default_profile()
                            .as_str()
                            .to_string(),
                        resolved_run_profile_version: 1,
                        event_cursor: EventCursor::default(),
                    })
                    .map_err(ProductSurfaceError::internal_from)?
                }
                _ => return Err(ProductSurfaceError::service_unavailable(false)),
            };
            Ok(ironclaw_host_api::ProductSurfaceInvokeResponse { output })
        }

        async fn query(
            &self,
            _caller: ProductSurfaceCaller,
            _request: ironclaw_host_api::ProductSurfaceQueryRequest,
        ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
            Err(ProductSurfaceError::service_unavailable(false))
        }

        async fn stream_events(
            &self,
            _caller: ProductSurfaceCaller,
            _request: ironclaw_host_api::ProductSurfaceStreamRequest,
        ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
            Err(ProductSurfaceError::service_unavailable(false))
        }
    }

    struct AdmissionProductSurface {
        coordinator: Arc<dyn TurnCoordinator>,
    }

    impl AdmissionProductSurface {
        fn new(coordinator: Arc<dyn TurnCoordinator>) -> Self {
            Self { coordinator }
        }
    }

    #[async_trait]
    impl ironclaw_host_api::ProductSurface for AdmissionProductSurface {
        async fn invoke(
            &self,
            caller: ProductSurfaceCaller,
            request: ironclaw_host_api::ProductSurfaceInvokeRequest,
        ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
            let output = match request.operation_id.as_str() {
                "thread.create" => {
                    let input: ProductCreateThreadRequest =
                        serde_json::from_value(request.input)
                            .map_err(ProductSurfaceError::internal_from)?;
                    let thread_id = ThreadId::new(
                        input
                            .requested_thread_id
                            .or(input.client_action_id)
                            .unwrap_or_else(|| THREAD.to_string()),
                    )
                    .map_err(ProductSurfaceError::internal_from)?;
                    serde_json::to_value(RebornCreateThreadResponse {
                        thread: test_thread_record(caller.clone(), thread_id),
                    })
                    .map_err(ProductSurfaceError::internal_from)?
                }
                "turn.submit" => {
                    let input: ProductSubmitTurnRequest = serde_json::from_value(request.input)
                        .map_err(ProductSurfaceError::internal_from)?;
                    let thread_id =
                        ThreadId::new(input.thread_id.clone().ok_or_else(invalid_request_error)?)
                            .map_err(ProductSurfaceError::internal_from)?;
                    let scope = caller.turn_scope(thread_id.clone());
                    let run_id = self
                        .coordinator
                        .prepare_turn(scope.clone())
                        .await
                        .map_err(map_turn_error)?;
                    let accepted_message_ref = AcceptedMessageRef::new(format!(
                        "msg:{}",
                        input.client_action_id.as_deref().unwrap_or("openai-chat")
                    ))
                    .map_err(ProductSurfaceError::internal_from)?;
                    let response = self
                        .coordinator
                        .submit_turn(SubmitTurnRequest {
                            scope,
                            actor: caller.actor(),
                            accepted_message_ref: accepted_message_ref.clone(),
                            source_binding_ref: SourceBindingRef::new("source:openai-chat")
                                .map_err(ProductSurfaceError::internal_from)?,
                            reply_target_binding_ref: ReplyTargetBindingRef::new(
                                "reply:openai-chat",
                            )
                            .map_err(ProductSurfaceError::internal_from)?,
                            requested_run_profile: None,
                            requested_model: input.model,
                            idempotency_key: IdempotencyKey::new(
                                input
                                    .client_action_id
                                    .unwrap_or_else(|| "openai-chat".to_string()),
                            )
                            .map_err(ProductSurfaceError::internal_from)?,
                            received_at: chrono::Utc::now(),
                            requested_run_id: Some(run_id),
                            parent_run_id: None,
                            subagent_depth: 0,
                            spawn_tree_root_run_id: None,
                            product_context: None,
                        })
                        .await;
                    let result = match response {
                        Ok(ironclaw_turns::SubmitTurnResponse::Accepted {
                            turn_id,
                            run_id,
                            status,
                            resolved_run_profile_id,
                            resolved_run_profile_version,
                            event_cursor,
                            accepted_message_ref,
                            ..
                        }) => RebornSubmitTurnResponse::Submitted {
                            thread_id,
                            accepted_message_ref,
                            turn_id: turn_id.to_string(),
                            run_id,
                            status,
                            resolved_run_profile_id: resolved_run_profile_id.as_str().to_string(),
                            resolved_run_profile_version: resolved_run_profile_version.as_u64(),
                            event_cursor,
                        },
                        Err(TurnError::ThreadBusy(busy)) => {
                            RebornSubmitTurnResponse::RejectedBusy {
                                thread_id,
                                accepted_message_ref,
                                active_run_id: Some(busy.active_run_id),
                                status: Some(busy.status),
                                event_cursor: Some(busy.event_cursor),
                                notice: "busy".to_string(),
                            }
                        }
                        Err(error) => return Err(map_turn_error(error)),
                    };
                    serde_json::to_value(result).map_err(ProductSurfaceError::internal_from)?
                }
                _ => return Err(ProductSurfaceError::service_unavailable(false)),
            };
            Ok(ironclaw_host_api::ProductSurfaceInvokeResponse { output })
        }

        async fn query(
            &self,
            _caller: ProductSurfaceCaller,
            _request: ironclaw_host_api::ProductSurfaceQueryRequest,
        ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
            Err(ProductSurfaceError::service_unavailable(false))
        }

        async fn stream_events(
            &self,
            _caller: ProductSurfaceCaller,
            _request: ironclaw_host_api::ProductSurfaceStreamRequest,
        ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
            Err(ProductSurfaceError::service_unavailable(false))
        }
    }

    fn test_thread_record(
        caller: ProductSurfaceCaller,
        thread_id: ThreadId,
    ) -> SessionThreadRecord {
        SessionThreadRecord {
            scope: ThreadScope {
                tenant_id: caller.tenant_id,
                agent_id: caller
                    .agent_id
                    .unwrap_or_else(|| AgentId::new(AGENT).expect("agent")),
                project_id: caller.project_id,
                owner_user_id: Some(caller.user_id.clone()),
                mission_id: None,
            },
            thread_id,
            created_by_actor_id: caller.user_id.as_str().to_string(),
            title: None,
            metadata_json: None,
            goal: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn invalid_request_error() -> ProductSurfaceError {
        ProductSurfaceError {
            code: ProductSurfaceErrorCode::InvalidRequest,
            kind: ProductSurfaceErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: None,
            validation_code: None,
        }
    }

    fn map_turn_error(error: TurnError) -> ProductSurfaceError {
        match error {
            TurnError::AdmissionRejected(_) | TurnError::CapacityExceeded { .. } => {
                ProductSurfaceError {
                    code: ProductSurfaceErrorCode::RateLimited,
                    kind: ProductSurfaceErrorKind::Busy,
                    status_code: 429,
                    retryable: true,
                    field: None,
                    validation_code: None,
                }
            }
            TurnError::ScopeNotFound => ProductSurfaceError {
                code: ProductSurfaceErrorCode::NotFound,
                kind: ProductSurfaceErrorKind::NotFound,
                status_code: 404,
                retryable: false,
                field: None,
                validation_code: None,
            },
            TurnError::Unauthorized => ProductSurfaceError {
                code: ProductSurfaceErrorCode::Forbidden,
                kind: ProductSurfaceErrorKind::ParticipantDenied,
                status_code: 403,
                retryable: false,
                field: None,
                validation_code: None,
            },
            TurnError::InvalidRequest { .. } => invalid_request_error(),
            TurnError::Unavailable { .. } => ProductSurfaceError {
                code: ProductSurfaceErrorCode::Unavailable,
                kind: ProductSurfaceErrorKind::ServiceUnavailable,
                status_code: 503,
                retryable: true,
                field: None,
                validation_code: None,
            },
            _ => ProductSurfaceError {
                code: ProductSurfaceErrorCode::Internal,
                kind: ProductSurfaceErrorKind::Internal,
                status_code: 500,
                retryable: false,
                field: None,
                validation_code: None,
            },
        }
    }

    struct StaticChatProjectionReader {
        projection: OpenAiChatCompletionProjection,
    }

    impl StaticChatProjectionReader {
        fn text(content: &str) -> Self {
            Self {
                projection: OpenAiChatCompletionProjection::text(content),
            }
        }
    }

    #[async_trait]
    impl OpenAiChatCompletionProjectionReader for StaticChatProjectionReader {
        async fn read_chat_completion_projection(
            &self,
            _request: OpenAiChatCompletionProjectionRequest,
        ) -> Result<
            OpenAiChatCompletionProjection,
            ironclaw_reborn_openai_compat::OpenAiCompatHttpError,
        > {
            Ok(self.projection.clone())
        }
    }

    struct NeverCompletingChatProjectionReader;

    #[async_trait]
    impl OpenAiChatCompletionProjectionReader for NeverCompletingChatProjectionReader {
        async fn read_chat_completion_projection(
            &self,
            _request: OpenAiChatCompletionProjectionRequest,
        ) -> Result<
            OpenAiChatCompletionProjection,
            ironclaw_reborn_openai_compat::OpenAiCompatHttpError,
        > {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(OpenAiChatCompletionProjection::text("late"))
        }
    }

    struct StaticResponsesProjectionReader {
        content: String,
    }

    impl StaticResponsesProjectionReader {
        fn text(content: &str) -> Self {
            Self {
                content: content.to_string(),
            }
        }
    }

    #[async_trait]
    impl OpenAiResponsesProjectionReader for StaticResponsesProjectionReader {
        async fn wait_for_response_completion(
            &self,
            request: OpenAiResponseWaitRequest,
        ) -> Result<OpenAiResponseProjection, ironclaw_reborn_openai_compat::OpenAiCompatHttpError>
        {
            Ok(OpenAiResponseProjection::new(response_object(
                request.public_id,
                &self.content,
            )))
        }

        async fn read_response(
            &self,
            request: OpenAiResponseReadRequest,
        ) -> Result<OpenAiResponseObject, ironclaw_reborn_openai_compat::OpenAiCompatHttpError>
        {
            Ok(response_object(request.public_id, &self.content))
        }
    }

    fn response_object(id: OpenAiResponseId, content: &str) -> OpenAiResponseObject {
        OpenAiResponseObject {
            id,
            object: "response".to_string(),
            created_at: 1_777_777_777,
            status: OpenAiResponseStatus::Completed,
            model: "reborn-test".to_string(),
            output: vec![OpenAiResponseOutputItem::Message {
                id: "msg_1".to_string(),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                role: OpenAiResponsesMessageRole::Assistant,
                content: json!([{"type": "output_text", "text": content}]),
            }],
            error: None,
            incomplete_details: None,
            usage: None,
        }
    }
}

#[derive(Default)]
struct StubServices {
    create_thread_calls: Mutex<Vec<ProductSurfaceCaller>>,
    stream_events_calls: Mutex<Vec<ProductSurfaceCaller>>,
    // Records the `gate_ref` value the facade observed on each
    // `resolve_gate` call. Used by the JS-client contract tests to
    // assert axum's path extractor actually percent-decodes the gate
    // segment (e.g. `gate%3Aapproval` → `gate:approval`). The handler
    // overwrites `body.gate_ref` from the matched path param before
    // calling the facade, so this captures whatever the path
    // extractor delivered.
    resolve_gate_refs: Mutex<Vec<Option<String>>>,
}

#[async_trait]
impl ironclaw_host_api::ProductSurface for StubServices {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceInvokeRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceInvokeResponse, ProductSurfaceError> {
        let output = match request.operation_id.as_str() {
            "thread.create" => {
                self.create_thread_calls.lock().expect("lock").push(caller);
                serde_json::to_value(RebornCreateThreadResponse {
                    thread: SessionThreadRecord {
                        thread_id: ThreadId::new("thread.fake").expect("thread"),
                        scope: ThreadScope {
                            tenant_id: TenantId::new(TENANT).expect("tenant"),
                            agent_id: AgentId::new("agent.fake").expect("agent"),
                            project_id: Some(ProjectId::new("project.fake").expect("project")),
                            owner_user_id: Some(UserId::new(USER).expect("user")),
                            mission_id: None,
                        },
                        created_by_actor_id: USER.to_string(),
                        title: None,
                        metadata_json: None,
                        goal: None,
                        created_at: None,
                        updated_at: None,
                    },
                })
                .map_err(ProductSurfaceError::internal_from)?
            }
            "turn.submit" => {
                let input: ProductSubmitTurnRequest = serde_json::from_value(request.input)
                    .map_err(ProductSurfaceError::internal_from)?;
                serde_json::to_value(RebornSubmitTurnResponse::Submitted {
                    thread_id: ThreadId::new(input.thread_id.clone().unwrap_or_default())
                        .expect("thread id"),
                    accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg.fake")
                        .expect("ref"),
                    turn_id: "turn.fake".to_string(),
                    run_id: TurnRunId::new(),
                    status: TurnStatus::Queued,
                    resolved_run_profile_id: RunProfileId::default_profile().as_str().to_string(),
                    resolved_run_profile_version: RunProfileVersion::new(1).as_u64(),
                    event_cursor: EventCursor(1),
                })
                .map_err(ProductSurfaceError::internal_from)?
            }
            "run.cancel" => serde_json::to_value(RebornCancelRunResponse {
                run_id: TurnRunId::new(),
                status: TurnStatus::Cancelled,
                event_cursor: EventCursor(2),
                already_terminal: false,
            })
            .map_err(ProductSurfaceError::internal_from)?,
            "run.retry" => {
                return Err(ProductSurfaceError {
                    code: ProductSurfaceErrorCode::Internal,
                    kind: ProductSurfaceErrorKind::Internal,
                    status_code: 500,
                    retryable: false,
                    field: None,
                    validation_code: None,
                });
            }
            "gate.resolve" => {
                let input: ProductResolveGateRequest = serde_json::from_value(request.input)
                    .map_err(ProductSurfaceError::internal_from)?;
                self.resolve_gate_refs
                    .lock()
                    .expect("lock")
                    .push(input.gate_ref.clone());
                return Err(ProductSurfaceError {
                    code: ProductSurfaceErrorCode::Internal,
                    kind: ProductSurfaceErrorKind::Internal,
                    status_code: 500,
                    retryable: false,
                    field: None,
                    validation_code: None,
                });
            }
            THREAD_DELETE_CAPABILITY_ID => {
                let _input: RebornDeleteThreadRequest = serde_json::from_value(request.input)
                    .map_err(ProductSurfaceError::internal_from)?;
                serde_json::to_value(successful_resolution(request.activity_id))
                    .map_err(ProductSurfaceError::internal_from)?
            }
            EXTENSION_SETUP_SUBMIT_CAPABILITY_ID => {
                serde_json::to_value(successful_resolution(request.activity_id))
                    .map_err(ProductSurfaceError::internal_from)?
            }
            _ => {
                return Err(ProductSurfaceError {
                    code: ProductSurfaceErrorCode::Internal,
                    kind: ProductSurfaceErrorKind::Internal,
                    status_code: 500,
                    retryable: false,
                    field: None,
                    validation_code: None,
                });
            }
        };
        Ok(ironclaw_host_api::ProductSurfaceInvokeResponse { output })
    }

    async fn query(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        let query = RebornViewQuery {
            view_id: request.view_id,
            params: request.input,
            cursor: request.cursor,
        };
        let payload = match query.view_id.as_str() {
            id if id == THREADS_VIEW.id => {
                let mut list_request: ProductListThreadsRequest =
                    serde_json::from_value(query.params)
                        .map_err(ProductSurfaceError::internal_from)?;
                list_request.cursor = query.cursor.or(list_request.cursor);
                let _ = list_request;
                serde_json::to_value(RebornListThreadsResponse {
                    threads: Vec::new(),
                    next_cursor: None,
                })
                .map_err(ProductSurfaceError::internal_from)?
            }
            id if id == TRACE_CREDITS_VIEW.id => {
                serde_json::to_value(trace_credits_response(&caller))
                    .map_err(ProductSurfaceError::internal_from)?
            }
            id if id == TIMELINE_VIEW.id => {
                let thread_id = query.params["thread_id"]
                    .as_str()
                    .ok_or_else(|| ProductSurfaceError::internal_from("missing thread_id"))?
                    .to_string();
                serde_json::to_value(RebornTimelineResponse {
                    thread: SessionThreadRecord {
                        thread_id: ThreadId::new(thread_id).expect("thread id"),
                        scope: ThreadScope {
                            tenant_id: TenantId::new(TENANT).expect("tenant"),
                            agent_id: AgentId::new("agent.fake").expect("agent"),
                            project_id: Some(ProjectId::new("project.fake").expect("project")),
                            owner_user_id: Some(UserId::new(USER).expect("user")),
                            mission_id: None,
                        },
                        created_by_actor_id: USER.to_string(),
                        title: None,
                        metadata_json: None,
                        goal: None,
                        created_at: None,
                        updated_at: None,
                    },
                    messages: Vec::new(),
                    summary_artifacts: Vec::new(),
                    next_cursor: None,
                })
                .map_err(ProductSurfaceError::internal_from)?
            }
            id if id == EXTENSION_SETUP_VIEW.id => {
                let package_id = query.params["package_id"]
                    .as_str()
                    .ok_or_else(|| ProductSurfaceError::internal_from("missing package_id"))?;
                let package_ref =
                    LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
                        .map_err(ProductSurfaceError::internal_from)?;
                serde_json::to_value(extension_setup_response(package_ref))
                    .map_err(ProductSurfaceError::internal_from)?
            }
            _ => {
                return Err(ProductSurfaceError {
                    code: ProductSurfaceErrorCode::Internal,
                    kind: ProductSurfaceErrorKind::Internal,
                    status_code: 500,
                    retryable: false,
                    field: None,
                    validation_code: None,
                });
            }
        };
        Ok(ironclaw_host_api::ProductSurfaceQueryPage {
            items: vec![payload],
            next_cursor: None,
        })
    }

    async fn stream_events(
        &self,
        caller: ProductSurfaceCaller,
        request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        let _ = request;
        self.stream_events_calls.lock().expect("lock").push(caller);
        Ok(ironclaw_host_api::ProductSurfaceStreamResponse {
            events: Vec::new(),
            next_cursor: None,
        })
    }
}

// ─── harness ──────────────────────────────────────────────────────────

const AGENT: &str = "agent-default";
const PROJECT: &str = "project-default";

fn build_app() -> (axum::Router, Arc<StubServices>) {
    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        product_surface: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    // Match the host-installation pattern the CLI's `serve` command
    // uses: stamp trusted default agent_id / project_id onto the auth
    // layer. Without this, every authenticated v2 request would 400
    // on the downstream facade.
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"));
    let app = webui_v2_app(bundle, config).expect("webui v2 app");
    (app, services)
}

fn build_app_with_authenticator(
    authenticator: Arc<dyn WebuiAuthenticator>,
) -> (axum::Router, Arc<StubServices>) {
    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        product_surface: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        authenticator,
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"));
    let app = webui_v2_app(bundle, config).expect("webui v2 app");
    (app, services)
}

async fn read_body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("body bytes");
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn served_static_text(path: &str) -> String {
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("static asset body bytes");
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn served_app_javascript() -> String {
    served_app_vite_asset(".js").await
}

async fn served_app_stylesheet() -> String {
    served_app_vite_asset(".css").await
}

async fn served_app_vite_asset(suffix: &str) -> String {
    let shell = served_static_text("/").await;
    let asset_path = shell_vite_asset_path(&shell, suffix);
    served_static_text(&asset_path).await
}

fn shell_vite_asset_path(shell: &str, suffix: &str) -> String {
    shell
        .split(['"', '\''])
        .find(|part| part.starts_with("/assets/app-") && part.ends_with(suffix))
        .expect("shell should reference requested Vite asset")
        .to_string()
}

fn bundle_segment<'a>(body: &'a str, start: &str, end: &str) -> &'a str {
    let start_idx = body
        .find(start)
        .unwrap_or_else(|| panic!("bundle should contain start marker `{start}`"));
    let after_start = &body[start_idx..];
    let end_rel = after_start
        .find(end)
        .unwrap_or_else(|| panic!("bundle should contain end marker `{end}` after `{start}`"));
    &after_start[..end_rel]
}

// ─── tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn bearer_happy_path_dispatches_to_facade_with_host_tenant() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"client_action_id": "act-1"}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let calls = services.create_thread_calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1, "facade reached exactly once");
    assert_eq!(calls[0].tenant_id.as_str(), TENANT);
    assert_eq!(calls[0].user_id.as_str(), USER);
    // Regression: caller MUST carry the trusted default agent_id and
    // project_id stamped by `WebuiServeConfig::with_default_agent_id`
    // / `with_default_project_id`. Without those, the downstream
    // facade rejects every mutation/read with 400 InvalidRequest
    // because it cannot build `ThreadScope`.
    assert_eq!(
        calls[0].agent_id.as_ref().map(|a| a.as_str()),
        Some(AGENT),
        "auth middleware must stamp trusted default agent_id onto the caller",
    );
    assert_eq!(
        calls[0].project_id.as_ref().map(|p| p.as_str()),
        Some(PROJECT),
        "auth middleware must stamp trusted default project_id onto the caller",
    );
}

#[tokio::test]
async fn session_endpoint_reports_operator_capability_for_operator_authenticator() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&read_body_string(response).await).expect("session json");
    assert_eq!(body["tenant_id"], TENANT);
    assert_eq!(body["user_id"], USER);
    assert_eq!(body["capabilities"]["operator_webui_config"], true);
}

#[tokio::test]
async fn session_endpoint_reports_no_operator_capability_for_multi_user_authenticator() {
    let (app, _services) = build_app_with_authenticator(Arc::new(MultiUserToken));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/session")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&read_body_string(response).await).expect("session json");
    assert_eq!(body["tenant_id"], TENANT);
    assert_eq!(body["user_id"], USER);
    assert_eq!(body["capabilities"]["operator_webui_config"], false);
}

#[tokio::test]
async fn missing_bearer_returns_401_before_facade() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

/// Removes a per-test trace scope directory on drop so a failed
/// assertion cannot leak contributor-local state into the shared
/// IronClaw base dir.
struct TraceScopeCleanup(String);

impl Drop for TraceScopeCleanup {
    fn drop(&mut self) {
        let dir = ironclaw_reborn_traces::contribution::trace_contribution_dir_for_scope(Some(
            self.0.as_str(),
        ));
        #[allow(clippy::let_underscore_must_use)] // best-effort per-test scope dir cleanup on drop
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn unique_trace_credits_user() -> String {
    format!("webui-v2-trace-credits-{}", uuid::Uuid::new_v4())
}

#[tokio::test]
async fn trace_credits_bearer_happy_path_returns_unenrolled_zero_state_for_fresh_scope() {
    // Fresh, unique user scope: the facade derives the trace scope from
    // the authenticated caller's user id only, so a uuid-suffixed user
    // guarantees no contributor-local state exists and the response is
    // the unenrolled zero-state — never an error.
    let user_id = unique_trace_credits_user();
    let (app, _services) = build_app_with_authenticator(Arc::new(FixedUserToken {
        user_id: user_id.clone(),
    }));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/traces/credit")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&read_body_string(response).await).expect("trace credits json");
    assert_eq!(body["enrolled"], false);
    assert_eq!(body["submissions_total"], 0);
    assert_eq!(body["submissions_submitted"], 0);
    assert_eq!(body["credit_events_total"], 0);
    assert_eq!(body["pending_credit"], 0.0);
    assert_eq!(body["final_credit"], 0.0);
    assert!(
        body["note"]
            .as_str()
            .expect("note")
            .contains("authoritative ledger is server-side"),
        "response must carry the server-authoritative framing note",
    );
}

#[tokio::test]
async fn trace_credits_missing_bearer_returns_401() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/traces/credit")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn trace_credits_reports_enrolled_for_caller_with_enabled_policy() {
    use ironclaw_reborn_traces::contribution::{
        StandingTraceContributionPolicy, trace_scope_key, write_trace_policy_for_scope,
    };

    // Trace state is keyed by the tenant-scoped composite, so enroll under
    // `trace_scope_key(TENANT, user)` and assert the route reflects enrollment
    // for that caller only.
    let user_id = unique_trace_credits_user();
    let scope = trace_scope_key(TENANT, user_id.as_str());
    let _cleanup = TraceScopeCleanup(scope.clone());
    let policy = StandingTraceContributionPolicy::default().set_enabled(true);
    write_trace_policy_for_scope(Some(scope.as_str()), &policy).expect("write trace policy");

    let (app, _services) = build_app_with_authenticator(Arc::new(FixedUserToken {
        user_id: user_id.clone(),
    }));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/traces/credit")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&read_body_string(response).await).expect("trace credits json");
    assert_eq!(body["enrolled"], true);
    assert_eq!(body["submissions_total"], 0);
}

#[tokio::test]
async fn invalid_bearer_returns_401() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, "Bearer wrong-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn sse_query_token_authenticates_event_stream() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/webchat/v2/threads/thread-x/events?token={VALID_TOKEN}"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream"),
    );
    // The SSE handler runs on the background body task and polls the
    // facade on a 1-second cadence. Pull one frame to drive the
    // generator far enough to record at least the first poll, then
    // drop the body so the long-lived stream does not pin the test.
    let mut body = response.into_body();
    #[allow(clippy::let_underscore_must_use)]
    // frame result intentionally unused; only drives the SSE generator past the first poll
    let _ = tokio::time::timeout(Duration::from_secs(2), body.frame()).await;
    drop(body);
    let calls = services.stream_events_calls.lock().expect("lock").clone();
    assert!(
        !calls.is_empty(),
        "?token= shim authenticated the SSE handler (calls={})",
        calls.len(),
    );
    assert_eq!(calls[0].user_id.as_str(), USER);
    assert_eq!(calls[0].tenant_id.as_str(), TENANT);
}

#[tokio::test]
async fn sse_without_bearer_or_query_token_returns_401() {
    let (app, services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/events")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .stream_events_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
}

#[tokio::test]
async fn timeline_route_rejects_query_token_shim() {
    // Mutation / read routes must stay bearer-only — only the SSE
    // endpoint accepts `?token=` (browsers' `EventSource` cannot set
    // headers). A query-token leaked via referer must not authenticate
    // a state read.
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/webchat/v2/threads/thread-x/timeline?token={VALID_TOKEN}"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn v2_response_carries_static_security_headers() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
    );
    assert_eq!(
        headers
            .get(header::X_FRAME_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("DENY"),
    );
    assert!(
        headers.contains_key("content-security-policy"),
        "CSP header present on v2 responses",
    );
}

#[tokio::test]
async fn cors_does_not_echo_disallowed_origin_on_preflight() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/webchat/v2/threads")
                .header("origin", "http://evil.example.com")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let echoed = response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());
    assert_ne!(
        echoed,
        Some("http://evil.example.com"),
        "CORS must not echo an attacker-supplied origin",
    );
}

#[tokio::test]
async fn cors_allows_configured_origin() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/webchat/v2/threads")
                .header("origin", "http://localhost:1234")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://localhost:1234"),
    );
}

#[tokio::test]
async fn malformed_user_id_from_authenticator_rejects_with_401() {
    // If a host authenticator returns a user id that doesn't satisfy
    // `UserId`'s grammar at construction time it never reaches the
    // composition. The authenticator's contract only accepts validated
    // `UserId`s inside `WebuiAuthentication`, so the only way to
    // produce a "malformed" id is to return None — which the
    // composition treats as auth failure. This test locks the contract:
    // a `None` decision becomes 401, never 500.
    struct AlwaysReject;
    #[async_trait]
    impl WebuiAuthenticator for AlwaysReject {
        async fn authenticate(&self, _token: &str) -> Option<WebuiAuthentication> {
            None
        }
    }

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        product_surface: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(AlwaysReject),
        vec![HeaderValue::from_static("http://localhost:1234")],
    );
    let app = webui_v2_app(bundle, config).expect("app");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        services
            .create_thread_calls
            .lock()
            .expect("lock")
            .is_empty()
    );
    // body content is opaque to clients — just confirm it's the
    // expected 401 string, not an internal traceback.
    let body = read_body_string(response).await;
    assert!(
        body.contains("Invalid or missing auth token"),
        "401 body should be the generic message, got: {body}",
    );
}

#[tokio::test]
async fn mutation_route_returns_429_after_descriptor_rate_limit_exhausted() {
    // `create_thread`'s descriptor declares 60 requests / 60s
    // per-caller. We send 60 successful POSTs from the same bearer
    // token and then expect the 61st to come back 429 — the rate-limit
    // middleware reads the descriptor at composition time, so this
    // test locks the contract that production-shape policies are
    // enforced (not just unit-test stubs).
    let (app, services) = build_app();
    let body = json!({}).to_string();
    let make_request = || {
        Request::builder()
            .method(Method::POST)
            .uri("/api/webchat/v2/threads")
            .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.clone()))
            .expect("request")
    };

    for i in 0..60 {
        let response = app.clone().oneshot(make_request()).await.expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "request {i} should be within the mutation budget",
        );
    }

    let response = app.clone().oneshot(make_request()).await.expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "61st mutation should exceed the per-caller rate-limit window",
    );
    let body = read_body_string(response).await;
    assert!(
        body.contains("Rate limit exceeded"),
        "429 body should explain the limit, got: {body}",
    );

    // Auth ran but the rate-limit middleware short-circuited, so the
    // facade only saw the 60 successful requests.
    let facade_calls = services.create_thread_calls.lock().expect("lock").len();
    assert_eq!(
        facade_calls, 60,
        "rate-limit must short-circuit BEFORE the v2 handler",
    );
}

#[tokio::test]
async fn oversized_mutation_body_is_rejected_with_413_before_facade() {
    // `create_thread`'s descriptor caps the body at 16 KiB. Send 16 KiB
    // + 1 of JSON and expect 413 from the per-route body limit, with
    // the facade untouched (the limit middleware sits in front of both
    // auth and the v2 handler).
    let (app, services) = build_app();
    let payload = format!(
        "{{\"client_action_id\":\"act-1\",\"padding\":\"{}\"}}",
        "x".repeat(16 * 1024 + 1)
    );
    assert!(
        payload.len() > 16 * 1024,
        "fixture must exceed the create_thread cap; got {} bytes",
        payload.len()
    );
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = read_body_string(response).await;
    assert!(
        body.contains("Request body exceeds the route's body limit."),
        "413 body should explain the cap, got: {body}",
    );
    assert!(
        services
            .create_thread_calls
            .lock()
            .expect("lock")
            .is_empty(),
        "facade must not be reached on an oversized request",
    );
}

#[tokio::test]
async fn mutation_body_within_descriptor_cap_reaches_facade() {
    // Companion to the oversized test: a payload that fits inside the
    // 16 KiB `create_thread` cap should pass through to the facade.
    // Locks the contract that the limit is "above max", not "above
    // some-fraction-of-max".
    let (app, services) = build_app();
    let payload = format!(
        "{{\"client_action_id\":\"act-1\",\"padding\":\"{}\"}}",
        "x".repeat(8 * 1024)
    );
    assert!(payload.len() < 16 * 1024);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        services.create_thread_calls.lock().expect("lock").len(),
        1,
        "facade should be reached for in-budget payload",
    );
}

#[tokio::test]
async fn timeline_route_rejects_nonempty_body_with_413() {
    // `get_timeline`'s descriptor declares `BodyLimitPolicy::NoBody`.
    // A GET with a non-empty body must be rejected upfront — regardless
    // of bearer-token validity — so that the v2 handler never observes
    // a body shape its descriptor said wouldn't arrive.
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/timeline")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("body-not-allowed"))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = read_body_string(response).await;
    assert!(
        body.contains("Request body not allowed for this route."),
        "413 body should name the NoBody policy, got: {body}",
    );
}

/// Spawn the composed v2 `Router` on a kernel-picked loopback port
/// and return the bound `SocketAddr` plus an abort handle. The serve
/// task runs until aborted at test teardown. `axum::serve` is forbidden
/// in `crates/.../src` by the `reborn_product_api_crates_do_not_bind_http_ingress`
/// architecture rule, but the rule scans `src/` only — host-owned tests
/// are the right place to drive a true WS upgrade.
async fn spawn_serve(app: axum::Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move {
        #[allow(clippy::let_underscore_must_use)]
        // background serve task; result observed via the spawned handle
        let _ = axum::serve(listener, app).await;
    });
    (addr, handle)
}

fn ws_upgrade_request(
    addr: std::net::SocketAddr,
    bearer: &str,
    origin: &str,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let url = format!("ws://{addr}/api/webchat/v2/threads/thread-x/ws");
    let mut request = url.into_client_request().expect("ws client request");
    request.headers_mut().insert(
        http::header::AUTHORIZATION,
        format!("Bearer {bearer}").parse().expect("auth header"),
    );
    request
        .headers_mut()
        .insert(http::header::ORIGIN, origin.parse().expect("origin header"));
    request
}

#[tokio::test]
async fn ws_upgrade_with_matching_origin_succeeds_with_101() {
    // Happy path: bind a real listener, open a real WebSocket from a
    // tungstenite client whose Origin matches the bound address. The
    // WS-origin middleware passes, auth passes, axum returns 101
    // Switching Protocols, and the connection upgrades cleanly.
    // Without this coverage a regression in the WS layer ordering
    // (origin check → auth → upgrade) would only be visible through
    // the rejection-path tests, which short-circuit BEFORE the upgrade
    // extractor runs.
    let (app, _services) = build_app();
    let (addr, handle) = spawn_serve(app).await;
    let origin = format!("http://{addr}");
    let request = ws_upgrade_request(addr, VALID_TOKEN, &origin);
    let (ws_stream, response) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .expect("ws connect within 5s")
    .expect("ws upgrade must succeed for matching Origin");
    assert_eq!(
        response.status().as_u16(),
        101,
        "valid bearer + same-origin must yield 101 Switching Protocols",
    );
    drop(ws_stream);
    handle.abort();
}

#[tokio::test]
async fn ws_upgrade_uses_canonical_host_over_client_host_when_configured() {
    // Operators running the v2 listener behind a reverse proxy may
    // receive an attacker-controlled `Host` header. When
    // `canonical_host` is set, the WS-origin middleware compares
    // `Origin` against that operator-trusted value instead of trusting
    // Host. This test binds a real listener, configures canonical_host
    // to a value the listener is NOT actually reachable at, then:
    //   1. A WS upgrade with `Origin: http://127.0.0.1:<port>` (matching
    //      Host, NOT canonical_host) must be rejected.
    //   2. A WS upgrade with `Origin: http://app.example.com` (matching
    //      canonical_host) must succeed.
    use ironclaw_webui::WebuiServeConfig;

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        product_surface: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_canonical_host("app.example.com");
    let app = ironclaw_webui::webui_v2_app(bundle, config).expect("app");
    let (addr, handle) = spawn_serve(app).await;

    // (1) Origin matches Host but NOT canonical_host — fail.
    let host_matching_origin = format!("http://{addr}");
    let attack_request = ws_upgrade_request(addr, VALID_TOKEN, &host_matching_origin);
    let attack = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_tungstenite::connect_async(attack_request),
    )
    .await
    .expect("ws connect attempt within 5s");
    assert!(
        attack.is_err(),
        "canonical_host must override Host: forged Origin must not pass same-origin",
    );

    // (2) Origin matches canonical_host — succeed.
    let canonical_request = ws_upgrade_request(addr, VALID_TOKEN, "http://app.example.com");
    let (ws_stream, response) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_tungstenite::connect_async(canonical_request),
    )
    .await
    .expect("ws connect within 5s")
    .expect("ws upgrade must succeed for canonical_host Origin");
    assert_eq!(
        response.status().as_u16(),
        101,
        "Origin matching canonical_host must yield 101 even when Host disagrees",
    );
    drop(ws_stream);
    handle.abort();
}

#[tokio::test]
async fn ws_upgrade_without_origin_is_rejected_with_403() {
    // WebChat v2 declares stream_events_ws as SameOriginRequired.
    // A WS upgrade without the `Origin` header must be rejected at
    // composition time before the v2 router sees the request.
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/ws")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                // Deliberately no Origin header.
                .header("connection", "upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn ws_upgrade_with_disallowed_origin_is_rejected_with_403() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads/thread-x/ws")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::HOST, "127.0.0.1:3000")
                .header(header::ORIGIN, "http://evil.example.com")
                .header("connection", "upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn list_threads_returns_facade_response_with_empty_default() {
    // GET /api/webchat/v2/threads goes through the new list_threads
    // route — descriptor is NoBody + read rate limit. The stub
    // facade returns an empty list which the handler serializes as
    // `{ "threads": [], "next_cursor": null }`.
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/webchat/v2/threads")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    assert!(
        body.contains("\"threads\":[]"),
        "list_threads body should carry the empty thread list, got: {body}",
    );
}

#[tokio::test]
async fn delete_thread_route_returns_facade_ack() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/webchat/v2/threads/thread-x")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    assert!(
        body.contains("\"thread_id\":\"thread-x\""),
        "delete_thread body should carry the deleted thread id, got: {body}",
    );
    assert!(
        body.contains("\"deleted\":true"),
        "delete_thread body should acknowledge deletion, got: {body}",
    );
}

#[tokio::test]
async fn setup_extension_returns_lifecycle_projection_via_facade() {
    let (app, _services) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/extensions/telegram/setup")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "action": "begin",
                        "client_action_id": "action-setup-extension-begin"
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    // #6520 three-state lifecycle: the facade projects setup_needed for an
    // extension awaiting configuration (the "unsupported" literal is retired).
    assert!(
        body.contains("\"phase\":\"setup_needed\""),
        "setup_extension must surface lifecycle phase, got: {body}",
    );
    assert!(
        !body.contains("\"status\""),
        "setup_extension must not surface legacy status aliases, got: {body}",
    );
    assert!(
        body.contains("\"package_ref\":{\"kind\":\"extension\",\"id\":\"telegram\"}"),
        "setup_extension must echo the path-bound package ref, got: {body}",
    );
}

#[tokio::test]
async fn rate_limit_is_independent_per_caller() {
    // Two distinct authenticators / users — alice exhausts her budget
    // but bob's requests still get through.
    use ironclaw_webui::WebuiServeConfig;

    struct UserSwitch;
    #[async_trait]
    impl WebuiAuthenticator for UserSwitch {
        async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
            match token {
                "tok-alice" => Some(WebuiAuthentication::user(
                    UserId::new("alice").expect("user"),
                )),
                "tok-bob" => Some(WebuiAuthentication::user(UserId::new("bob").expect("user"))),
                _ => None,
            }
        }
    }

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        product_surface: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(UserSwitch),
        vec![HeaderValue::from_static("http://localhost:1234")],
    );
    let app = webui_v2_app(bundle, config).expect("app");

    let make_request = |token: &str| -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .uri("/api/webchat/v2/threads")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({}).to_string()))
            .expect("request")
    };

    // Burn alice's full 60-request budget.
    for _ in 0..60 {
        let response = app
            .clone()
            .oneshot(make_request("tok-alice"))
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
    }
    // Next alice request → 429.
    let response = app
        .clone()
        .oneshot(make_request("tok-alice"))
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    // Bob has a fresh window.
    let response = app
        .clone()
        .oneshot(make_request("tok-bob"))
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "bob's per-caller budget must be independent of alice's",
    );
}

/// Every descriptor returned by `webui_v2_routes()` must be reachable on
/// the composed `webui_v2_app` Router. Sends a request with a bogus
/// bearer token to each route and asserts the response is anything *but*
/// 404. A 404 means the descriptor exists but host composition forgot to
/// mount the matching handler — exactly the regression Lane 7 step 1
/// ("Mount WebUI v2 routes in production composition") guards against.
///
/// 401 is the expected status for a mounted route receiving a wrong
/// token; some routes may also legitimately surface 400/405/413/426 (WS
/// upgrade without proper headers) — anything but 404 proves the mount.
#[tokio::test]
async fn every_webui_v2_descriptor_is_mounted_on_composed_app() {
    let (app, _services) = build_app();

    for descriptor in ironclaw_webui::webui_v2::webui_v2_routes() {
        let method = match descriptor.method() {
            NetworkMethod::Get => Method::GET,
            NetworkMethod::Post => Method::POST,
            NetworkMethod::Put => Method::PUT,
            NetworkMethod::Patch => Method::PATCH,
            NetworkMethod::Delete => Method::DELETE,
            NetworkMethod::Head => Method::HEAD,
        };
        let uri = expand_route_pattern(descriptor.route_pattern().as_str());

        let mut builder = Request::builder()
            .method(method.clone())
            .uri(&uri)
            .header(header::AUTHORIZATION, "Bearer not-the-valid-token");
        // POST routes with non-NoBody policies expect a JSON content
        // type; body is empty so it's within every per-route cap.
        if method == Method::POST {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        let request = builder.body(Body::empty()).expect("request");

        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("oneshot must complete");

        assert_ne!(
            response.status(),
            StatusCode::NOT_FOUND,
            "descriptor `{route_id}` ({method} {uri}) returned 404 — host composition did not mount the handler",
            route_id = descriptor.route_id().as_str(),
            method = method,
            uri = uri,
        );
    }
}

#[tokio::test]
async fn operator_routes_are_not_mounted_for_multi_user_authenticator() {
    let (app, _services) = build_app_with_authenticator(Arc::new(MultiUserToken));

    for (method, uri) in [
        (Method::GET, "/api/webchat/v2/llm/providers"),
        (Method::POST, "/api/webchat/v2/llm/providers"),
        (Method::POST, "/api/webchat/v2/llm/providers/openai/delete"),
        (Method::POST, "/api/webchat/v2/llm/active"),
        (Method::POST, "/api/webchat/v2/llm/test-connection"),
        (Method::POST, "/api/webchat/v2/llm/list-models"),
        (Method::POST, "/api/webchat/v2/llm/nearai/login"),
        (Method::POST, "/api/webchat/v2/llm/nearai/wallet"),
        (Method::POST, "/api/webchat/v2/llm/codex/login"),
        (Method::GET, "/api/webchat/v2/operator/setup"),
        (Method::POST, "/api/webchat/v2/operator/setup"),
        (Method::GET, "/api/webchat/v2/operator/config"),
        (
            Method::GET,
            "/api/webchat/v2/operator/config/provider.default",
        ),
        (
            Method::POST,
            "/api/webchat/v2/operator/config/provider.default",
        ),
        (Method::POST, "/api/webchat/v2/operator/config/validate"),
        (Method::GET, "/api/webchat/v2/operator/diagnostics"),
        (Method::GET, "/api/webchat/v2/operator/status"),
        (Method::GET, "/api/webchat/v2/operator/logs"),
        (Method::POST, "/api/webchat/v2/operator/service"),
    ] {
        let mut builder = Request::builder()
            .method(method.clone())
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"));
        if method == Method::POST {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).expect("request"))
            .await
            .expect("oneshot must complete");
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{method} {uri} must not be mounted for non-operator auth"
        );
    }
}

fn expand_route_pattern(pattern: &str) -> String {
    // Stand-in values for the four path params the v2 descriptors use.
    // All must satisfy each handler's path-segment validation.
    pattern
        .replace("{thread_id}", "thread.fake")
        .replace("{run_id}", "11111111-1111-1111-1111-111111111111")
        .replace("{gate_ref}", "gate.fake")
        .replace("{package_id}", "ext-fake")
}

// ─── static SPA mount (`ironclaw_webui`) ────────────────────
//
// The composition mounts the embedded SPA bundle at the gateway root. These
// tests drive that mount through the same composed router production uses, so
// a regression that drops the static router (or accidentally routes the SPA
// through the bearer-auth middleware) fails here. Per
// `.claude/rules/testing.md` ("Test Through the
// Caller") — the standalone router test in `ironclaw_webui`
// does not exercise the composition seam, so this layer needs its
// own coverage.

#[tokio::test]
async fn static_root_serves_index_with_substituted_csp_nonce() {
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string()),
        Some("text/html; charset=utf-8".to_string()),
    );
    let body = read_body_string(response).await;
    assert!(
        body.contains("v2-root"),
        "SPA shell must contain the React mount point",
    );
    assert!(
        !body.contains("__IRONCLAW_CSP_NONCE__"),
        "every CSP-nonce placeholder must be substituted",
    );
}

#[tokio::test]
async fn static_root_does_not_require_bearer_auth() {
    let (app, _) = build_app();
    // No Authorization header at all — anonymous fetch of the SPA shell
    // must succeed. The bearer-auth middleware is only attached to the
    // v2 JSON routes via `route_layer`, so the root static router escapes it
    // by design.
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn legacy_v2_urls_redirect_to_root_without_losing_query_data() {
    let (app, _) = build_app();
    for (source, target) in [
        ("/v2", "/"),
        ("/v2/", "/"),
        (
            "/v2/settings/skills?token=old%2Btoken&tab=installed",
            "/settings/skills?token=old%2Btoken&tab=installed",
        ),
        ("/v2?login_ticket=ticket%2B1", "/?login_ticket=ticket%2B1"),
        ("/v2//evil.example?keep=1", "/evil.example?keep=1"),
        (r"/v2/\evil.example", "/evil.example"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(source)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::TEMPORARY_REDIRECT,
            "GET {source}",
        );
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some(target),
            "GET {source}",
        );
    }
}

#[tokio::test]
async fn root_static_mount_keeps_server_namespaces_fail_closed() {
    let (app, _) = build_app();
    for (method, path) in [
        (Method::GET, "/api/not-a-route"),
        (Method::POST, "/api/not-a-route"),
        (Method::GET, "/auth/not-a-route"),
        (Method::POST, "/auth/not-a-route"),
        (Method::GET, "/v1/not-a-route"),
        (Method::POST, "/v1/not-a-route"),
        (Method::GET, "/webhooks/not-a-route"),
        (Method::POST, "/webhooks/not-a-route"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {path}");
    }
}

#[tokio::test]
async fn root_static_mount_rejects_noncanonical_path_separators() {
    let (app, _) = build_app();
    for path in [
        "//api/not-a-route",
        "///auth/not-a-route",
        "//v1/not-a-route",
        "//webhooks/not-a-route",
        "/%2Fapi/not-a-route",
        r"/\api/not-a-route",
        "/%5Capi/not-a-route",
        r"/api\not-a-route",
        "/api%5Cnot-a-route",
        "//chat",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "malformed path `{path}` must be rejected before SPA fallback",
        );
    }
}

#[tokio::test]
async fn root_manifest_and_wallet_popup_are_served_with_owned_contracts() {
    let (app, _) = build_app();
    let manifest_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/assets/site.webmanifest")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(manifest_response.status(), StatusCode::OK);
    assert_eq!(
        manifest_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/manifest+json"),
    );
    let manifest = read_body_string(manifest_response).await;
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest).expect("embedded manifest must be valid JSON");
    assert_eq!(manifest["id"], "/v2/");
    assert_eq!(manifest["start_url"], "/");
    assert_eq!(manifest["scope"], "/");

    let wallet_response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/wallet/connect")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(wallet_response.status(), StatusCode::OK);
    let wallet_csp = wallet_response
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|value| value.to_str().ok())
        .expect("wallet popup CSP");
    assert!(wallet_csp.contains("script-src 'self' 'unsafe-inline' https:"));
    let wallet_body = read_body_string(wallet_response).await;
    assert!(wallet_body.contains("src=\"/wallet-connect.js\""));
}

#[tokio::test]
async fn static_js_asset_returns_javascript_content_type() {
    let (app, _) = build_app();
    let shell = read_body_string(
        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot"),
    )
    .await;
    let app_js_path = shell_vite_asset_path(&shell, ".js");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(app_js_path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    assert!(ct.starts_with("text/javascript"), "got content-type `{ct}`");
}

#[tokio::test]
async fn static_chat_oauth_card_exposes_https_only_authorization_link() {
    let body = served_app_javascript().await;

    assert!(
        body.contains("new URL(e.authorizationUrl).protocol===`https:`"),
        "OAuth auth card must reject non-HTTPS authorization URLs before opening"
    );
    assert!(
        body.contains("className:`auth-oauth`"),
        "OAuth auth card must keep the UI-test selector on the authorization control"
    );
    assert!(
        body.contains("href:") && body.contains("authorizationUrl:void 0"),
        "OAuth auth card must expose the HTTPS authorization URL as a link href"
    );
    assert!(
        body.contains("noopener,noreferrer"),
        "OAuth authorization popup must keep opener isolation"
    );
}

#[tokio::test]
async fn static_chat_hook_listens_for_oauth_callback_completion() {
    let body = served_app_javascript().await;

    // The Vite app bundle must retain the shared OAuth completion transport and
    // the chat hook's gate-matching behavior. Source-level details for the
    // shared transport live in the frontend Vitest suite; this caller-level test
    // protects the served bundle path.
    assert!(
        body.contains("ironclaw-product-auth")
            && body.contains("new window.BroadcastChannel(")
            && body.contains("onmessage="),
        "chat hook must consume same-origin OAuth callback broadcasts"
    );
    assert!(
        body.contains("window.addEventListener(`storage`,"),
        "chat hook must keep a localStorage fallback for browsers without BroadcastChannel"
    );
    assert!(
        body.contains("localStorage?.getItem?.("),
        "chat hook must poll localStorage in case the callback write happened before the storage event listener observed it"
    );
    assert!(
        body.contains("turn_gate_resume"),
        "chat hook must match callback completion to the visible OAuth gate when continuation metadata is present"
    );
    assert!(
        body.contains("auth_required") && body.contains("oauth_url") && body.contains("?null:e"),
        "OAuth callback completion must clear only a pending OAuth auth gate"
    );
    assert!(
        body.contains("ironclaw:product-auth:oauth-complete"),
        "shared OAuth events module must define the callback completion signal in the served bundle"
    );
}

#[tokio::test]
async fn static_chat_events_clear_gate_when_run_resumes() {
    let body = served_app_javascript().await;

    assert!(
        body.contains("blocked_auth")
            && body.contains("blocked_approval")
            && body.contains("blocked_resource")
            && body.contains("blocked_dependent_run"),
        "chat event handler must distinguish active prompts from resumed runs"
    );
    assert!(
        body.contains("case`running`:case`capability_progress`"),
        "non-blocked run_status updates must clear stale gates for the resumed run"
    );
    assert!(
        !body.contains(
            "clearPendingGateForRun(\n              setPendingGate,\n              progress.turn_run_id,"
        ),
        "typed running/progress events must not clear blocked auth gates"
    );
    assert!(
        body.contains("projection_snapshot") && body.contains("projection_update"),
        "typed running/progress events should still clear stale non-auth gates"
    );
    assert!(
        body.contains("awaiting_gate"),
        "projection gates must not be restored after the run has resumed"
    );
}

#[tokio::test]
async fn static_css_asset_returns_text_css_content_type() {
    let (app, _) = build_app();
    let shell = read_body_string(
        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot"),
    )
    .await;
    let app_css_path = shell_vite_asset_path(&shell, ".css");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(app_css_path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    assert!(ct.starts_with("text/css"), "got content-type `{ct}`");
}

#[tokio::test]
async fn static_i18n_module_guards_locale_race_and_clears_failed_pack_cache() {
    // Content-shape regression guard for the i18n loader fixes (PR
    // #4493 review): the single `setLang` transition must (1) discard a
    // slow pack load whose promise resolves after a newer language was
    // requested, and (2) drop the in-flight `pending[lang]` entry once it
    // settles so a transient import failure does not cache a permanent
    // miss. It also locks the follow-up cleanup that removed the
    // `version` counter in favor of committing the loaded pack to state.
    // There is no JS test harness in this workspace (see the route-shape
    // note below), so this locks the served bundle shape; a behavioral provider
    // test driving `setLang('es')` through an unloaded pack belongs in
    // the deferred JS/e2e scaffold.
    let body = served_app_javascript().await;
    let loader_segment = bundle_segment(&body, "ironclaw_language", "createContext({lang:");
    let provider_segment = bundle_segment(&body, "createContext({lang:", "QueryClient");

    assert!(
        provider_segment.contains(".useState(()=>")
            && provider_segment.contains("||null")
            && provider_segment.contains(".useRef("),
        "i18n provider must track the latest requested language in a ref",
    );
    assert!(
        provider_segment.contains(".current="),
        "setLang must stamp the requested language before awaiting the pack",
    );
    assert!(
        loader_segment.contains("Promise.resolve(null)"),
        "a resolved pack load must only commit when the pack is available and still the latest request",
    );
    assert!(
        provider_segment.contains(".current!=="),
        "a resolved pack load must be ignored after a newer language request",
    );
    assert_eq!(
        loader_segment.matches("delete ").count(),
        2,
        "ensurePack must clear pending[lang] on BOTH the success and failure paths so a transient import failure can be retried",
    );
    assert!(
        !body.contains("setVersion"),
        "the version counter must stay removed: async loads re-render by committing the pack to state",
    );
}

#[tokio::test]
async fn static_typing_dot_animation_respects_reduced_motion() {
    // Content-shape regression guard for the typing-indicator animation
    // contract (PR #4493 review): `.v2-typing-dot` is the single
    // intentional animation exception, so it must animate by default and
    // be suppressed under `prefers-reduced-motion: reduce`. A behavioral
    // check that the dot computes to `animation: none` via the emulated
    // media query needs a browser (`getComputedStyle`), which this
    // workspace's Rust/oneshot harness cannot drive; that belongs in the
    // deferred e2e scaffold.
    let body = served_app_stylesheet().await;

    assert!(
        body.contains("animation:1.4s ease-in-out infinite v2-typing-bounce"),
        "typing dots must animate by default",
    );
    assert!(
        body.contains("@media (prefers-reduced-motion:reduce)"),
        "stylesheet must carry a reduced-motion opt-out block",
    );
    assert!(
        body.contains(".v2-typing-dot,.v2-spin{animation:none"),
        "the typing dot must be suppressed under prefers-reduced-motion: reduce",
    );
}

#[tokio::test]
async fn static_unknown_extension_path_returns_404() {
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/missing-asset.bin")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_client_side_route_falls_back_to_spa_shell() {
    // Any root-level no-dot path that does not match an asset
    // returns the SPA shell so react-router can render the right
    // view. Without this, a hard refresh on `/chat/<id>` would
    // 404 instead of resuming the chat view.
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/chat/some-thread-id")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    assert!(body.contains("v2-root"));
}

#[tokio::test]
async fn static_root_emits_a_fresh_nonce_per_request() {
    fn nonce_attribute(body: &str) -> String {
        let marker = "nonce=\"";
        let start = body.find(marker).expect("nonce attribute present");
        let after = &body[start + marker.len()..];
        let end = after.find('"').expect("nonce attribute closed");
        after[..end].to_string()
    }

    let (app, _) = build_app();
    let body_a = read_body_string(
        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot"),
    )
    .await;
    let body_b = read_body_string(
        app.oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot"),
    )
    .await;

    let nonce_a = nonce_attribute(&body_a);
    let nonce_b = nonce_attribute(&body_b);
    assert_ne!(
        nonce_a, nonce_b,
        "CSP nonce must be regenerated for every request",
    );
}

// ─── Route-shape contract: URLs the SPA's lib/api.ts builds ────────────
//
// These tests lock the URL + body shapes the composed router accepts —
// they hand-build requests against the same shapes `frontend/src/lib/api.ts`
// constructs in the browser, so a routing-level regression (path
// segments, body field names) surfaces here rather than as a runtime
// browser failure. They do NOT execute the JS client itself: there is
// no JS test harness in this workspace, so a regression purely inside
// `api.ts` (e.g. forgetting `encodeURIComponent` on a gate_ref) would
// pass these tests and only break in the browser. A full JS-level
// caller test belongs in a separate JS test scaffold the workspace
// doesn't currently own.

#[tokio::test]
async fn js_client_send_message_path_shape_reaches_facade() {
    // api.ts → `sendMessage({threadId, content, clientActionId})`
    // builds `POST /api/webchat/v2/threads/{thread_id}/messages` with
    // body `{client_action_id, content}` (no thread_id in body —
    // it lives in the path).
    let (app, _) = build_app();
    let body = json!({
        "client_action_id": "act-from-js",
        "content": "hello from the SPA",
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads/thread.fake/messages")
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn js_client_cancel_run_path_shape_reaches_facade() {
    // api.ts → `cancelRun({threadId, runId, reason, clientActionId})`
    // builds `POST /api/webchat/v2/threads/{thread_id}/runs/{run_id}/cancel`
    // with body `{client_action_id, reason}`.
    let (app, _) = build_app();
    let run_id = uuid::Uuid::new_v4();
    let body = json!({
        "client_action_id": "act-from-js",
        "reason": "user_requested",
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/webchat/v2/threads/thread.fake/runs/{run_id}/cancel",
                ))
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn js_client_resolve_gate_path_shape_dispatches_to_facade() {
    // api.ts → `resolveGate({threadId, runId, gateRef, resolution, always, clientActionId})`
    // builds `POST /api/webchat/v2/threads/{thread_id}/runs/{run_id}/gates/{gate_ref}/resolve`
    // with body `{client_action_id, resolution, always}`.
    //
    // The stub's `resolve_gate` returns 500 by design; we only care
    // that the path-params parsing succeeded and the facade was
    // reached. A routing-level regression (missing path segment,
    // wrong encoding) would surface as 404, not 500.
    let (app, services) = build_app();
    let run_id = uuid::Uuid::new_v4();
    let gate_ref = "gate-abc";
    let body = json!({
        "client_action_id": "act-from-js",
        "resolution": "approved",
        "always": false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/webchat/v2/threads/thread.fake/runs/{run_id}/gates/{gate_ref}/resolve",
                ))
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    // 500 = facade reached and returned (stub returns Internal); 404
    // would mean the path did not route. Anything else means contract
    // drift.
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "resolve_gate path must reach the stubbed facade (which returns 500)",
    );
    assert_eq!(
        services.resolve_gate_refs.lock().expect("lock").as_slice(),
        &[Some("gate-abc".to_string())],
        "literal gate_ref must reach the facade unchanged",
    );
}

#[tokio::test]
async fn js_client_resolve_gate_path_decodes_percent_encoded_gate_ref() {
    // Real gate refs can carry characters that require percent-encoding
    // in a URL segment (`:` in `gate:approval`, `/` in compound refs).
    // axum's path extractor must decode the segment before the handler
    // assigns it to `body.gate_ref`, so the facade sees the literal
    // ref the JS client built — dropping `encodeURIComponent` in
    // `api.ts` would otherwise either 404 (slash-bearing refs) or
    // silently mismatch (`%3A` left undecoded).
    let (app, services) = build_app();
    let run_id = uuid::Uuid::new_v4();
    // `gate:approval` percent-encoded = `gate%3Aapproval`.
    let encoded_gate_ref = "gate%3Aapproval";
    let body = json!({
        "client_action_id": "act-from-js",
        "resolution": "approved",
        "always": false,
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/webchat/v2/threads/thread.fake/runs/{run_id}/gates/{encoded_gate_ref}/resolve",
                ))
                .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "path-decoded resolve_gate must reach the stubbed facade",
    );
    assert_eq!(
        services.resolve_gate_refs.lock().expect("lock").as_slice(),
        &[Some("gate:approval".to_string())],
        "facade must observe the decoded gate_ref, not the URL-encoded form",
    );
}

/// Locks the [`WebuiServeConfig::with_public_router`] seam: a
/// host-supplied router (today wired by
/// `ironclaw_webui::webui_v2_auth_router`) must
/// reach its handler WITHOUT going through the bearer-auth
/// middleware, and must still pick up the outer security headers
/// applied to every other response. Regression guard for issue
/// #4116: without the merge in `webui_v2_app`, the SPA's
/// unauthenticated `GET /auth/providers` would 401 before the
/// host's OAuth router ever ran.
#[tokio::test]
async fn public_route_mount_is_merged_without_bearer_auth_and_keeps_descriptor_policy() {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        product_surface: services,
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let public = axum::Router::new().route(
        "/auth/providers",
        axum::routing::get(|| async { axum::Json(serde_json::json!({ "providers": [] })) }),
    );
    let descriptor = public_test_descriptor("webui.sso.providers.test", "/auth/providers");

    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
    .with_public_route_mount(PublicRouteMount::new(public, vec![descriptor]));
    let app = webui_v2_app(bundle, config).expect("webui v2 app");

    // No Authorization header — `with_public_route_mount` MUST
    // merge outside the bearer-auth layer.
    // ConnectInfo is required because the descriptor's PerIp rate
    // limit middleware reads the peer address; the production
    // listener injects this via `into_make_service_with_connect_info`,
    // so the oneshot harness simulates it.
    let mut req = Request::builder()
        .method(Method::GET)
        .uri("/auth/providers")
        .body(Body::empty())
        .expect("request");
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));

    let response = app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
        "outer security headers must still wrap the public route mount",
    );
    let body = read_body_string(response).await;
    assert!(body.contains("\"providers\""), "got body {body}");

    // The bearer-protected v2 surface must still 401 without a
    // token, defense in depth that the public merge did not widen
    // auth bypass beyond its mounted paths.
    let protected = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/webchat/v2/threads")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(protected.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn public_route_mount_reserves_its_root_namespace_from_spa_fallback() {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        product_surface: services,
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let public = axum::Router::new().route(
        "/future-host/ping",
        axum::routing::get(|| async { StatusCode::NO_CONTENT }),
    );
    let descriptor = public_test_descriptor("webui.future_host.ping.test", "/future-host/ping");
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
    .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
    .with_public_route_mount(PublicRouteMount::new(public, vec![descriptor]));
    let app = webui_v2_app(bundle, config).expect("webui v2 app");

    let mut exact_request = Request::builder()
        .method(Method::GET)
        .uri("/future-host/ping")
        .body(Body::empty())
        .expect("request");
    exact_request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
    let exact = app.clone().oneshot(exact_request).await.expect("oneshot");
    assert_eq!(exact.status(), StatusCode::NO_CONTENT);

    let unknown = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/future-host/not-a-route")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        unknown.status(),
        StatusCode::NOT_FOUND,
        "unknown paths in a host-owned root namespace must not render the SPA shell",
    );
    assert_ne!(
        unknown
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/html; charset=utf-8"),
    );
}

#[test]
fn public_route_mount_with_dynamic_root_namespace_fails_composition() {
    let error = match compose_with_public_descriptor(
        "webui.dynamic_namespace.ping.test",
        "/{namespace}/ping",
    ) {
        Ok(_) => panic!("dynamic root namespace must fail composition"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        WebuiServeError::NonCanonicalRootNamespace {
            route_id,
            route_pattern,
            root_namespace,
        } if route_id == "webui.dynamic_namespace.ping.test"
            && route_pattern == "/{namespace}/ping"
            && root_namespace == "{namespace}"
    ));
}

#[test]
fn public_route_mount_with_encoded_root_namespace_fails_composition() {
    let error = match compose_with_public_descriptor(
        "webui.encoded_namespace.ping.test",
        "/%66uture-host/ping",
    ) {
        Ok(_) => panic!("encoded root namespace must fail composition"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        WebuiServeError::NonCanonicalRootNamespace {
            route_id,
            route_pattern,
            root_namespace,
        } if route_id == "webui.encoded_namespace.ping.test"
            && route_pattern == "/%66uture-host/ping"
            && root_namespace == "%66uture-host"
    ));
}

#[test]
fn public_route_mount_with_static_owned_root_namespace_fails_composition() {
    for (root_namespace, route_pattern) in [
        ("v2", "/v2/ping"),
        ("wallet", "/wallet/ping"),
        ("assets", "/assets/ping"),
        ("vendor", "/vendor/ping"),
        ("wallet-connect.js", "/wallet-connect.js/ping"),
    ] {
        let error = match compose_with_public_descriptor(
            "webui.static_namespace_conflict.ping.test",
            route_pattern,
        ) {
            Ok(_) => panic!("static-owned root namespace must fail composition"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            WebuiServeError::StaticRootNamespaceConflict {
                route_id,
                route_pattern: actual_pattern,
                root_namespace: actual_namespace,
            } if route_id == "webui.static_namespace_conflict.ping.test"
                && actual_pattern == route_pattern
                && actual_namespace == root_namespace
        ));
    }
}

// ─── Automations panel UI (fix/reborn-automations-ux) ─────────────────
//
// These lock the served automations SPA source shape so a regression that
// drops one of the panel UX fixes fails here. Behavioral JS coverage needs a
// browser harness this workspace does not own, so — per the existing
// `static_*` precedent — we assert the shipped asset content instead.

#[tokio::test]
async fn static_automations_presenters_label_sub_hourly_schedules() {
    let body = served_app_javascript().await;

    // The cadence labels are now localized: the presenter selects an i18n key
    // for each sub-hourly/hourly branch and the English copy lives in en.js.
    assert!(
        body.contains("automations.schedule.everyMinute"),
        "presenters must label `* * * * *` / `*/1 * * * *` via the everyMinute key"
    );
    assert!(
        body.contains("automations.schedule.everyMinutes"),
        "presenters must label `*/N * * * *` via the everyMinutes key"
    );
    assert!(
        body.contains("automations.schedule.hourlyAt"),
        "presenters must label `M * * * *` via the hourlyAt key"
    );

    // And the English pack must carry the human-readable copy for those keys,
    // so a clean install still reads "Every minute" / "Hourly at :MM".
    let en_body = served_app_javascript().await;
    assert!(
        en_body.contains("automations.schedule.everyMinute\":`Every minute`"),
        "en.js must label `* * * * *` as `Every minute` instead of `Custom schedule`"
    );
    assert!(
        en_body.contains("Every {count} minutes"),
        "en.js must label `*/N * * * *` as `Every N minutes`"
    );
    assert!(
        en_body.contains("Hourly at :"),
        "en.js must label `M * * * *` as an hourly cadence"
    );
}

#[tokio::test]
async fn static_automations_summary_reflows_cards_and_shrinks_next_run() {
    let body = served_app_javascript().await;

    assert!(
        body.contains("lg:grid-cols-3"),
        "summary strip must cap cards per row so detail text stays readable"
    );
    assert!(
        !body.contains("xl:grid-cols-5"),
        "summary strip must not force five cards into one row"
    );
    assert!(
        body.contains("valueClassName"),
        "the NEXT RUN card must pass a smaller value font so the date is not truncated"
    );
}

#[tokio::test]
async fn static_automations_run_row_spaces_action_button_icons() {
    let body = served_app_javascript().await;

    assert!(
        body.contains("name:`chat`,className:`mr-1.5 h-4 w-4`"),
        "the Open run button icon must be spaced away from its label"
    );
    assert!(
        body.contains("name:`file`,className:`mr-1.5 h-4 w-4`"),
        "the Logs button icon must be spaced away from its label"
    );
}

#[tokio::test]
async fn static_automations_delivery_surfaces_save_error_and_gates_slack_hint() {
    let body = served_app_javascript().await;

    assert!(
        body.contains("e.saveError&&!a"),
        "the delivery panel must render the save error instead of swallowing it"
    );
    assert!(
        body.contains("finalReplyTargets.length>0"),
        "the Slack approval footnote must be gated on an external target existing"
    );
}
