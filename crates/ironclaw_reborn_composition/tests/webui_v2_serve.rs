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

#![cfg(feature = "webui-v2-beta")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use ironclaw_host_api::{AgentId, NetworkMethod, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    LifecyclePackageRef, LifecyclePhase, RebornCancelRunResponse, RebornCreateThreadResponse,
    RebornDeleteThreadRequest, RebornDeleteThreadResponse, RebornExtensionActionResponse,
    RebornExtensionListResponse, RebornExtensionRegistryResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornListAutomationsResponse, RebornListThreadsResponse,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundPreferencesResponse,
    RebornResolveGateResponse, RebornServicesApi, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind, RebornSetOutboundPreferencesRequest, RebornSetupExtensionResponse,
    RebornSkillActionResponse, RebornSkillContentResponse, RebornSkillListResponse,
    RebornSkillSearchResponse, RebornStreamEventsRequest, RebornStreamEventsResponse,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
    WebUiAuthenticatedCaller, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiListAutomationsRequest, WebUiListThreadsRequest, WebUiResolveGateRequest,
    WebUiSendMessageRequest, WebUiSetupExtensionRequest,
};
use ironclaw_reborn_composition::{
    PublicRouteMount, RebornReadiness, RebornWebuiBundle, WebuiAuthentication, WebuiAuthenticator,
    WebuiServeConfig, webui_v2_app,
};
use ironclaw_threads::{SessionThreadRecord, ThreadScope};
use ironclaw_turns::{EventCursor, RunProfileId, RunProfileVersion, TurnRunId, TurnStatus};
use serde_json::json;
use tower::ServiceExt;

const TENANT: &str = "tenant-alpha";
const USER: &str = "user-alpha";
const VALID_TOKEN: &str = "valid-bearer-token";

fn service_unavailable_error(retryable: bool) -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Unavailable,
        kind: RebornServicesErrorKind::ServiceUnavailable,
        status_code: 503,
        retryable,
        field: None,
        validation_code: None,
    }
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

#[tokio::test]
async fn health_route_is_public_for_platform_probes() {
    let bundle = RebornWebuiBundle {
        api: Arc::new(StubServices::default()),
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

#[cfg(feature = "openai-compat-beta")]
mod openai_compat_mount_tests {
    use super::*;
    use ironclaw_product_adapters::{
        ProductAdapterError, ProductInboundAck, ProductInboundEnvelope, ProductProjectionReadInput,
        ProductProjectionSubject, ProductWorkflow, ProjectionReadRequest, RedactedString,
    };
    use ironclaw_product_workflow::{
        ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
        FakeIdempotencyLedger, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
    };
    use ironclaw_reborn_composition::ProtectedRouteMount;
    use ironclaw_reborn_openai_compat::{
        InMemoryOpenAiCompatRefStore, OpenAiChatCompletionProjection,
        OpenAiChatCompletionProjectionReader, OpenAiChatCompletionProjectionRequest,
        OpenAiChatCompletionsWorkflow, OpenAiCompatRouterState, OpenAiResponseId,
        OpenAiResponseObject, OpenAiResponseOutputItem, OpenAiResponseOutputItemStatus,
        OpenAiResponseProjection, OpenAiResponseReadRequest, OpenAiResponseStatus,
        OpenAiResponseWaitRequest, OpenAiResponsesMessageRole, OpenAiResponsesProjectionReader,
        OpenAiResponsesWorkflow, openai_compat_router_with_state, openai_compat_routes,
    };
    use ironclaw_threads::InMemorySessionThreadService;
    use ironclaw_turns::runner::{ClaimRunRequest, CompleteRunRequest, TurnRunTransitionPort};
    use ironclaw_turns::{
        AcceptedMessageRef, DefaultTurnCoordinator, InMemoryTurnStateStore,
        StaticTurnAdmissionLimitProvider, TurnActor, TurnAdmissionAxisKind, TurnLeaseToken,
        TurnRunId, TurnRunnerId, TurnScope,
    };

    const AGENT: &str = "agent-alpha";
    const PROJECT: &str = "project-alpha";
    const THREAD: &str = "thread-openai-chat";

    #[tokio::test]
    async fn openai_chat_completions_mount_uses_webui_auth_and_product_workflow() {
        let workflow = Arc::new(GatewayOpenAiWorkflow::default());
        let chat = Arc::new(OpenAiChatCompletionsWorkflow::new(
            workflow.clone(),
            Arc::new(InMemoryOpenAiCompatRefStore::new()),
            Arc::new(StaticChatProjectionReader::text(
                "hello through composition",
            )),
        ));
        let mount = ProtectedRouteMount::new(
            openai_compat_router_with_state(OpenAiCompatRouterState::with_chat_completions(chat)),
            openai_compat_routes(),
        );
        let bundle = RebornWebuiBundle {
            api: Arc::new(StubServices::default()),
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
        let turn_state = Arc::new(InMemoryTurnStateStore::with_admission_limit_provider(
            Arc::new(limits),
        ));
        let turn_coordinator = Arc::new(DefaultTurnCoordinator::new(turn_state.clone()));
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let binding = AdmissionTestBindingService;
        let inbound = Arc::new(DefaultInboundTurnService::new(
            binding.clone(),
            thread_service,
            turn_coordinator,
        ));
        let workflow: Arc<dyn ProductWorkflow> = Arc::new(DefaultProductWorkflow::new(
            inbound,
            Arc::new(FakeIdempotencyLedger::new()),
            Arc::new(binding),
        ));
        let chat = Arc::new(
            OpenAiChatCompletionsWorkflow::new(
                workflow,
                Arc::new(InMemoryOpenAiCompatRefStore::new()),
                Arc::new(NeverCompletingChatProjectionReader),
            )
            .with_wait_timeout(Duration::from_millis(1)),
        );
        let mount = ProtectedRouteMount::new(
            openai_compat_router_with_state(OpenAiCompatRouterState::with_chat_completions(chat)),
            openai_compat_routes(),
        );
        let bundle = RebornWebuiBundle {
            api: Arc::new(StubServices::default()),
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
        assert_eq!(turn_state.active_admission_reservations().len(), 1);

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
        assert_eq!(turn_state.active_admission_reservations().len(), 1);

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
        assert!(turn_state.active_admission_reservations().is_empty());

        let accepted_after_release = app
            .oneshot(chat_request(Some(VALID_TOKEN)))
            .await
            .expect("chat response after release");
        assert_eq!(
            accepted_after_release.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "the route still times out waiting for projection, but admission accepted a new turn"
        );
        assert_eq!(turn_state.active_admission_reservations().len(), 1);
    }

    #[tokio::test]
    async fn openai_responses_mount_uses_webui_auth_and_product_workflow() {
        let workflow = Arc::new(GatewayOpenAiWorkflow::default());
        let responses = Arc::new(OpenAiResponsesWorkflow::new(
            workflow.clone(),
            Arc::new(InMemoryOpenAiCompatRefStore::new()),
            Arc::new(StaticResponsesProjectionReader::text(
                "hello through responses",
            )),
        ));
        let mount = ProtectedRouteMount::new(
            openai_compat_router_with_state(OpenAiCompatRouterState::with_responses(responses)),
            openai_compat_routes(),
        );
        let bundle = RebornWebuiBundle {
            api: Arc::new(StubServices::default()),
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
        assert_eq!(workflow.read_count(), 1);
    }

    #[derive(Clone)]
    struct AdmissionTestBindingService;

    #[async_trait]
    impl ConversationBindingService for AdmissionTestBindingService {
        async fn resolve_binding(
            &self,
            request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            self.resolve(request)
        }

        async fn lookup_binding(
            &self,
            request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            self.resolve(request)
        }
    }

    impl AdmissionTestBindingService {
        fn resolve(
            &self,
            request: ResolveBindingRequest,
        ) -> Result<ResolvedBinding, ProductWorkflowError> {
            Ok(ResolvedBinding {
                tenant_id: TenantId::new(TENANT).map_err(binding_error("test OpenAI tenant id"))?,
                actor_user_id: UserId::new(request.external_actor_ref.id())
                    .map_err(binding_error("test OpenAI actor user id"))?,
                subject_user_id: Some(
                    UserId::new(request.external_actor_ref.id())
                        .map_err(binding_error("test OpenAI subject user id"))?,
                ),
                thread_id: ThreadId::new(format!("thread-{}", request.external_event_id.as_str()))
                    .map_err(binding_error("test OpenAI thread id"))?,
                agent_id: Some(AgentId::new(AGENT).map_err(binding_error("test OpenAI agent id"))?),
                project_id: Some(
                    ProjectId::new(PROJECT).map_err(binding_error("test OpenAI project id"))?,
                ),
            })
        }
    }

    fn binding_error(
        field: &'static str,
    ) -> impl FnOnce(ironclaw_host_api::HostApiError) -> ProductWorkflowError + 'static {
        move |reason| ProductWorkflowError::BindingResolutionFailed {
            reason: format!("{field}: {reason}"),
        }
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
    struct GatewayOpenAiWorkflow {
        submit_count: Mutex<usize>,
        read_count: Mutex<usize>,
    }

    impl GatewayOpenAiWorkflow {
        fn submit_count(&self) -> usize {
            *self
                .submit_count
                .lock()
                .expect("submit count lock should not be poisoned")
        }

        fn read_count(&self) -> usize {
            *self
                .read_count
                .lock()
                .expect("read count lock should not be poisoned")
        }
    }

    #[async_trait]
    impl ProductWorkflow for GatewayOpenAiWorkflow {
        async fn submit_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            *self
                .submit_count
                .lock()
                .expect("submit count lock should not be poisoned") += 1;
            Ok(ProductInboundAck::Accepted {
                accepted_message_ref: AcceptedMessageRef::new("msg:openai-chat")
                    .expect("accepted ref"),
                submitted_run_id: TurnRunId::new(),
            })
        }

        async fn read_projection(
            &self,
            request: ProductProjectionReadInput,
        ) -> Result<ProjectionReadRequest, ProductAdapterError> {
            *self
                .read_count
                .lock()
                .expect("read count lock should not be poisoned") += 1;
            let ProductProjectionSubject::AdapterExternalRefs { auth_claim, .. } = request.subject
            else {
                return Err(ProductAdapterError::Internal {
                    detail: RedactedString::new("expected adapter refs projection subject"),
                });
            };
            let user_id = UserId::new(auth_claim.subject()).map_err(|error| {
                ProductAdapterError::Internal {
                    detail: RedactedString::new(format!("invalid user id: {error}")),
                }
            })?;
            Ok(ProjectionReadRequest {
                actor: TurnActor::new(user_id.clone()),
                scope: TurnScope::new_with_owner(
                    TenantId::new(TENANT).expect("tenant"),
                    Some(AgentId::new(AGENT).expect("agent")),
                    Some(ProjectId::new(PROJECT).expect("project")),
                    ThreadId::new(THREAD).expect("thread"),
                    Some(user_id),
                ),
                after_cursor: request.after_cursor,
                limit: request.limit,
            })
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

#[cfg(feature = "slack-v2-host-beta")]
mod slack_personal_binding_pairing_mount_tests {
    use super::*;
    use ironclaw_product_adapters::AdapterInstallationId;
    use ironclaw_reborn_composition::slack_serve::SlackUserId;
    use ironclaw_reborn_composition::{
        IssuedSlackPersonalBindingPairingChallenge, RebornUserIdentityBinding,
        RebornUserIdentityBindingError, RebornUserIdentityBindingStore, SlackInstallationSelector,
        SlackPersonalBindingInstallation, SlackPersonalBindingPairingChallenge,
        SlackPersonalBindingPairingChallengeStore, SlackPersonalBindingPairingCode,
        SlackPersonalBindingPairingError, SlackPersonalBindingPairingNotification,
        SlackPersonalBindingPairingNotifier, SlackPersonalBindingPairingRouteConfig,
        SlackPersonalBindingPairingService, SlackPersonalUserBindingService,
        WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH,
    };

    #[tokio::test]
    async fn pairing_route_mounted_when_config_provided() {
        let binding_store = Arc::new(RecordingBindingStore::default());
        let pairing = SlackPersonalBindingPairingService::new(
            SlackPersonalUserBindingService::new(
                [SlackPersonalBindingInstallation {
                    tenant_id: TenantId::new(TENANT).expect("tenant"),
                    installation_id: installation("install-a"),
                    selector: SlackInstallationSelector::app_team("A-app", "T-team"),
                }],
                binding_store.clone(),
            ),
            Arc::new(StaticChallengeStore),
            Arc::new(NoopNotifier),
        );
        let bundle = RebornWebuiBundle {
            api: Arc::new(StubServices::default()),
            product_auth: None,
            readiness: RebornReadiness::disabled(),
        };
        let config = WebuiServeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            Arc::new(OnlyValidToken),
            vec![HeaderValue::from_static("http://localhost:1234")],
        )
        .with_slack_personal_binding_pairing(SlackPersonalBindingPairingRouteConfig::new(pairing));
        let app = webui_v2_app(bundle, config).expect("webui v2 app");

        let unauthenticated = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"channel":"slack","code":"abc12345"}"#))
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let authenticated = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
                    .header(header::AUTHORIZATION, format!("Bearer {VALID_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"channel":"slack","code":"abc12345"}"#))
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(authenticated.status(), StatusCode::OK);
        assert_eq!(binding_store.bound_user_ids(), vec![USER.to_string()]);
    }

    fn installation(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).expect("installation")
    }

    #[derive(Default)]
    struct RecordingBindingStore {
        bindings: Mutex<Vec<RebornUserIdentityBinding>>,
    }

    impl RecordingBindingStore {
        fn bound_user_ids(&self) -> Vec<String> {
            self.bindings
                .lock()
                .expect("bindings lock should not be poisoned")
                .iter()
                .map(|binding| binding.user_id.to_string())
                .collect()
        }
    }

    #[async_trait]
    impl RebornUserIdentityBindingStore for RecordingBindingStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.bindings
                .lock()
                .expect("bindings lock should not be poisoned")
                .push(binding);
            Ok(())
        }
    }

    struct StaticChallengeStore;

    #[async_trait]
    impl SlackPersonalBindingPairingChallengeStore for StaticChallengeStore {
        async fn issue_challenge(
            &self,
            challenge: SlackPersonalBindingPairingChallenge,
        ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            Ok(IssuedSlackPersonalBindingPairingChallenge {
                code: SlackPersonalBindingPairingCode::new("ABC12345").expect("code"),
                challenge,
            })
        }

        async fn get_challenge(
            &self,
            code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            if code.as_str() != "ABC12345" {
                return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
            }
            Ok(SlackPersonalBindingPairingChallenge {
                installation_id: installation("install-a"),
                slack_user_id: SlackUserId::new("U123"),
            })
        }

        async fn consume_challenge(
            &self,
            code: &SlackPersonalBindingPairingCode,
        ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError>
        {
            self.get_challenge(code).await
        }
    }

    struct NoopNotifier;

    #[async_trait]
    impl SlackPersonalBindingPairingNotifier for NoopNotifier {
        async fn send_pairing_challenge(
            &self,
            _notification: SlackPersonalBindingPairingNotification,
        ) -> Result<(), SlackPersonalBindingPairingError> {
            Ok(())
        }
    }
}

#[derive(Default)]
struct StubServices {
    create_thread_calls: Mutex<Vec<WebUiAuthenticatedCaller>>,
    stream_events_calls: Mutex<Vec<WebUiAuthenticatedCaller>>,
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
impl RebornServicesApi for StubServices {
    async fn create_thread(
        &self,
        caller: WebUiAuthenticatedCaller,
        _request: WebUiCreateThreadRequest,
    ) -> Result<RebornCreateThreadResponse, RebornServicesError> {
        self.create_thread_calls.lock().expect("lock").push(caller);
        Ok(RebornCreateThreadResponse {
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
            },
        })
    }

    async fn submit_turn(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiSendMessageRequest,
    ) -> Result<RebornSubmitTurnResponse, RebornServicesError> {
        Ok(RebornSubmitTurnResponse::Submitted {
            thread_id: ThreadId::new(request.thread_id.clone().unwrap_or_default())
                .expect("thread id"),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("msg.fake").expect("ref"),
            turn_id: "turn.fake".to_string(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile().as_str().to_string(),
            resolved_run_profile_version: RunProfileVersion::new(1).as_u64(),
            event_cursor: EventCursor(1),
        })
    }

    async fn delete_thread(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornDeleteThreadRequest,
    ) -> Result<RebornDeleteThreadResponse, RebornServicesError> {
        Ok(RebornDeleteThreadResponse {
            thread_id: ThreadId::new(request.thread_id).expect("thread id"),
            deleted: true,
        })
    }

    async fn get_timeline(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornTimelineRequest,
    ) -> Result<RebornTimelineResponse, RebornServicesError> {
        Ok(RebornTimelineResponse {
            thread: SessionThreadRecord {
                thread_id: ThreadId::new(request.thread_id.clone()).expect("thread id"),
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
            },
            messages: Vec::new(),
            summary_artifacts: Vec::new(),
            next_cursor: None,
        })
    }

    async fn stream_events(
        &self,
        caller: WebUiAuthenticatedCaller,
        _request: RebornStreamEventsRequest,
    ) -> Result<RebornStreamEventsResponse, RebornServicesError> {
        self.stream_events_calls.lock().expect("lock").push(caller);
        Ok(RebornStreamEventsResponse { events: Vec::new() })
    }

    async fn get_run_state(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornGetRunStateRequest,
    ) -> Result<RebornGetRunStateResponse, RebornServicesError> {
        Err(RebornServicesError {
            code: RebornServicesErrorCode::Internal,
            kind: RebornServicesErrorKind::Internal,
            status_code: 500,
            retryable: false,
            field: None,
            validation_code: None,
        })
    }

    async fn cancel_run(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiCancelRunRequest,
    ) -> Result<RebornCancelRunResponse, RebornServicesError> {
        Ok(RebornCancelRunResponse {
            run_id: TurnRunId::new(),
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(2),
            already_terminal: false,
        })
    }

    async fn resolve_gate(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: WebUiResolveGateRequest,
    ) -> Result<RebornResolveGateResponse, RebornServicesError> {
        self.resolve_gate_refs
            .lock()
            .expect("lock")
            .push(request.gate_ref.clone());
        Err(RebornServicesError {
            code: RebornServicesErrorCode::Internal,
            kind: RebornServicesErrorKind::Internal,
            status_code: 500,
            retryable: false,
            field: None,
            validation_code: None,
        })
    }

    async fn list_threads(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListThreadsRequest,
    ) -> Result<RebornListThreadsResponse, RebornServicesError> {
        Ok(RebornListThreadsResponse {
            threads: Vec::new(),
            next_cursor: None,
        })
    }

    async fn list_automations(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: WebUiListAutomationsRequest,
    ) -> Result<RebornListAutomationsResponse, RebornServicesError> {
        Ok(RebornListAutomationsResponse {
            automations: Vec::new(),
        })
    }

    async fn get_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        Ok(RebornOutboundPreferencesResponse::default())
    }

    async fn set_outbound_preferences(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, RebornServicesError> {
        Err(service_unavailable_error(false))
    }

    async fn list_outbound_delivery_targets(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, RebornServicesError> {
        Err(service_unavailable_error(false))
    }

    async fn list_extensions(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionListResponse, RebornServicesError> {
        Ok(RebornExtensionListResponse {
            extensions: Vec::new(),
        })
    }

    async fn list_skills(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornSkillListResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn search_skills(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _query: String,
    ) -> Result<RebornSkillSearchResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn install_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
        _content: Option<String>,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn read_skill_content(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
    ) -> Result<RebornSkillContentResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn update_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
        _content: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn remove_skill(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _name: String,
    ) -> Result<RebornSkillActionResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn list_extension_registry(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornExtensionRegistryResponse, RebornServicesError> {
        Ok(RebornExtensionRegistryResponse {
            entries: Vec::new(),
        })
    }

    async fn install_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn activate_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn remove_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        _package_ref: LifecyclePackageRef,
    ) -> Result<RebornExtensionActionResponse, RebornServicesError> {
        Err(unused_services_error())
    }

    async fn setup_extension(
        &self,
        _caller: WebUiAuthenticatedCaller,
        package_ref: LifecyclePackageRef,
        _request: WebUiSetupExtensionRequest,
    ) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
        Ok(RebornSetupExtensionResponse {
            package_ref,
            phase: LifecyclePhase::UnsupportedOrLegacy,
            blockers: Vec::new(),
            payload: None,
            secrets: Vec::new(),
            fields: Vec::new(),
            onboarding: None,
        })
    }
}

fn unused_services_error() -> RebornServicesError {
    RebornServicesError {
        code: RebornServicesErrorCode::Internal,
        kind: RebornServicesErrorKind::Internal,
        status_code: 500,
        retryable: false,
        field: None,
        validation_code: None,
    }
}

// ─── harness ──────────────────────────────────────────────────────────

const AGENT: &str = "agent-default";
const PROJECT: &str = "project-default";

fn build_app() -> (axum::Router, Arc<StubServices>) {
    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services.clone(),
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
        api: services.clone(),
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
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body bytes");
    String::from_utf8_lossy(&bytes).into_owned()
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
        StandingTraceContributionPolicy, write_trace_policy_for_scope,
    };

    // The authenticated user id IS the trace scope; write an enabled
    // standing policy for a unique per-test scope and assert the route
    // reflects enrollment for that caller only.
    let user_id = unique_trace_credits_user();
    let _cleanup = TraceScopeCleanup(user_id.clone());
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        ..StandingTraceContributionPolicy::default()
    };
    write_trace_policy_for_scope(Some(user_id.as_str()), &policy).expect("write trace policy");

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
        api: services.clone(),
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
    use ironclaw_reborn_composition::WebuiServeConfig;

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services.clone(),
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let config = WebuiServeConfig::new(
        TenantId::new(TENANT).expect("tenant"),
        Arc::new(OnlyValidToken),
        vec![HeaderValue::from_static("http://localhost:1234")],
    )
    .with_canonical_host("app.example.com");
    let app = ironclaw_reborn_composition::webui_v2_app(bundle, config).expect("app");
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
                .body(Body::from(json!({"action": "begin"}).to_string()))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;
    assert!(
        body.contains("\"phase\":\"unsupported_or_legacy\""),
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
    use ironclaw_reborn_composition::WebuiServeConfig;

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
        api: services.clone(),
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

    for descriptor in ironclaw_webui_v2::webui_v2_routes() {
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

// ─── static SPA mount (`ironclaw_webui_v2_static`) ────────────────────
//
// The composition mounts the embedded SPA bundle under `/v2`. These
// tests drive that mount through the same composed router production
// uses, so a regression that drops the `.nest("/v2", ...)` call (or
// that accidentally routes the SPA through the bearer-auth middleware)
// fails here. Per `.claude/rules/testing.md` ("Test Through the
// Caller") — the standalone router test in `ironclaw_webui_v2_static`
// does not exercise the composition seam, so this layer needs its
// own coverage.

#[tokio::test]
async fn static_root_serves_index_with_substituted_csp_nonce() {
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/")
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
    // v2 JSON routes via `route_layer`, so the static `.nest("/v2", …)`
    // mount escapes it by design.
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn static_js_asset_returns_javascript_content_type() {
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/js/main.js")
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
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/js/pages/chat/components/auth-oauth-card.js")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;

    assert!(
        body.contains("new URL(gate.authorizationUrl).protocol === \"https:\""),
        "OAuth auth card must reject non-HTTPS authorization URLs before opening"
    );
    assert!(
        body.contains("className=\"auth-oauth\""),
        "OAuth auth card must keep the UI-test selector on the authorization control"
    );
    assert!(
        body.contains("href=${hasHttpsAuthorizationUrl ? gate.authorizationUrl : undefined}"),
        "OAuth auth card must expose the HTTPS authorization URL as a link href"
    );
    assert!(
        body.contains("noopener,noreferrer"),
        "OAuth authorization popup must keep opener isolation"
    );
}

#[tokio::test]
async fn static_chat_hook_listens_for_oauth_callback_completion() {
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/js/pages/chat/hooks/useChat.js")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;

    assert!(
        body.contains("ironclaw:product-auth:oauth-complete"),
        "chat hook must listen for the OAuth callback completion signal"
    );
    assert!(
        body.contains("new window.BroadcastChannel(OAUTH_CALLBACK_CHANNEL)"),
        "chat hook must consume same-origin OAuth callback broadcasts"
    );
    assert!(
        body.contains("window.addEventListener(\"storage\", onStorage)"),
        "chat hook must keep a localStorage fallback for browsers without BroadcastChannel"
    );
    assert!(
        body.contains("window.localStorage?.getItem?.(OAUTH_CALLBACK_STORAGE_KEY)"),
        "chat hook must poll localStorage in case the callback write happened before the storage event listener observed it"
    );
    assert!(
        body.contains("oauthCompletionMatchesGate(payload, pendingGate, listeningSince)"),
        "chat hook must match callback completion to the visible OAuth gate when continuation metadata is present"
    );
    assert!(
        body.contains(
            "setPendingGate((current) => (isPendingOAuthGate(current) ? null : current))"
        ),
        "OAuth callback completion must clear only a pending OAuth auth gate"
    );
}

#[tokio::test]
async fn static_chat_events_clear_gate_when_run_resumes() {
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/js/pages/chat/lib/useChatEvents.js")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;

    assert!(
        body.contains("const PROMPT_RUN_STATUSES = new Set"),
        "chat event handler must distinguish active prompts from resumed runs"
    );
    assert!(
        body.contains("clearPendingGateForRun(setPendingGate, runId, promptRunIdRef)"),
        "non-blocked run_status updates must clear stale gates for the resumed run"
    );
    assert!(
        !body.contains(
            "clearPendingGateForRun(\n              setPendingGate,\n              progress.turn_run_id,"
        ),
        "typed running/progress events must not clear blocked auth gates"
    );
    assert!(
        body.contains("clearPendingNonAuthGateForRun(\n              setPendingGate,\n              progress.turn_run_id,\n              promptRunIdRef,"),
        "typed running/progress events should still clear stale non-auth gates"
    );
    assert!(
        body.contains("promptRunIdRef?.current === activeRunId"),
        "projection gates must not be restored after the run has resumed"
    );
    assert!(
        !body.contains("clearPendingAuthGateForForwardProgress"),
        "tool/reasoning/text progress must not hide a still-blocked auth gate"
    );
}

#[tokio::test]
async fn static_css_asset_returns_text_css_content_type() {
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/styles/app.css")
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
    // note below), so this locks the source shape; a behavioral provider
    // test driving `setLang('es')` through an unloaded pack belongs in
    // the deferred JS/e2e scaffold.
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/js/lib/i18n.js")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;

    assert!(
        body.contains("const activeLangRef = React.useRef(lang);"),
        "i18n provider must track the latest requested language in a ref",
    );
    assert!(
        body.contains("activeLangRef.current = next;"),
        "setLang must stamp the requested language before awaiting the pack",
    );
    assert!(
        body.contains("if (!loaded || activeLangRef.current !== next) return;"),
        "a resolved pack load must only commit when the pack is available and still the latest request",
    );
    assert_eq!(
        body.matches("delete pending[lang];").count(),
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
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/styles/app.css")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);
    let body = read_body_string(response).await;

    assert!(
        body.contains("animation: v2-typing-bounce"),
        "typing dots must animate by default",
    );
    assert!(
        body.contains("@media (prefers-reduced-motion: reduce)"),
        "stylesheet must carry a reduced-motion opt-out block",
    );
    assert!(
        body.contains(".v2-typing-dot { animation: none"),
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
                .uri("/v2/missing-asset.bin")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_client_side_route_falls_back_to_spa_shell() {
    // Any `/v2/<no-dot-segment>` path that does not match an asset
    // returns the SPA shell so react-router can render the right
    // view. Without this, a hard refresh on `/v2/chat/<id>` would
    // 404 instead of resuming the chat view.
    let (app, _) = build_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v2/chat/some-thread-id")
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
                    .uri("/v2/")
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
                .uri("/v2/")
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

// ─── Route-shape contract: URLs the SPA's lib/api.js builds ────────────
//
// These tests lock the URL + body shapes the composed router accepts —
// they hand-build requests against the same shapes `static/js/lib/api.js`
// constructs in the browser, so a routing-level regression (path
// segments, body field names) surfaces here rather than as a runtime
// browser failure. They do NOT execute the JS client itself: there is
// no JS test harness in this workspace, so a regression purely inside
// `api.js` (e.g. forgetting `encodeURIComponent` on a gate_ref) would
// pass these tests and only break in the browser. A full JS-level
// caller test belongs in a separate JS test scaffold the workspace
// doesn't currently own.

#[tokio::test]
async fn js_client_send_message_path_shape_reaches_facade() {
    // api.js → `sendMessage({threadId, content, clientActionId})`
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
    // api.js → `cancelRun({threadId, runId, reason, clientActionId})`
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
    // api.js → `resolveGate({threadId, runId, gateRef, resolution, always, clientActionId})`
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
    // `api.js` would otherwise either 404 (slash-bearing refs) or
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
/// `ironclaw_reborn_webui_ingress::webui_v2_auth_router`) must
/// reach its handler WITHOUT going through the bearer-auth
/// middleware, and must still pick up the outer security headers
/// applied to every other response. Regression guard for issue
/// #4116: without the merge in `webui_v2_app`, the SPA's
/// unauthenticated `GET /auth/providers` would 401 before the
/// host's OAuth router ever ran.
#[tokio::test]
async fn public_route_mount_is_merged_without_bearer_auth_and_keeps_descriptor_policy() {
    use axum::extract::ConnectInfo;
    use ironclaw_host_api::ingress::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthPolicy,
        IngressJustification, IngressPolicy, IngressPolicyParts, IngressRouteDescriptor,
        ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
    };
    use ironclaw_host_api::{IngressScopeSource, NetworkMethod};
    use std::net::SocketAddr;
    use std::num::NonZeroU32;

    let services = Arc::new(StubServices::default());
    let bundle = RebornWebuiBundle {
        api: services,
        product_auth: None,
        readiness: RebornReadiness::disabled(),
    };
    let public = axum::Router::new().route(
        "/auth/providers",
        axum::routing::get(|| async { axum::Json(serde_json::json!({ "providers": [] })) }),
    );
    let descriptor = IngressRouteDescriptor::new(
        "webui.sso.providers.test".to_string(),
        NetworkMethod::Get,
        "/auth/providers".to_string(),
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
    .expect("descriptor");

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
