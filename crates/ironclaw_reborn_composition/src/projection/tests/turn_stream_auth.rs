use super::*;

#[tokio::test]
async fn webui_event_stream_enriches_auth_prompt_through_projection_stream() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-auth-enriched-thread").unwrap();
    let turn_run = TurnRunId::new();
    let gate_ref = "gate:auth-required";
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new(gate_ref).unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                    activity_id: None,
                    credential_requirements: Vec::new(),
                }),
                sanitized_reason: Some("GitHub authentication required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    )
    .with_auth_challenges(Arc::new(FakeAuthChallengeProvider {
        expected_owner_user_id: user_id.clone(),
        expected_run_id: turn_run,
        expected_gate_ref: gate_ref.to_string(),
    }));

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| matches!(
        event.payload(),
        ProductOutboundPayload::AuthPrompt(prompt)
            if prompt.turn_run_id == turn_run
                && prompt.auth_request_ref == gate_ref
                && prompt.challenge_kind == Some(AuthPromptChallengeKind::OAuthUrl)
                && prompt.provider.as_deref() == Some("github")
                && prompt.authorization_url.as_deref() == Some("https://github.com/login/oauth/authorize")
    )));
    assert!(events.iter().any(|event| matches!(
        event.payload(),
        ProductOutboundPayload::ProjectionUpdate { state }
            if state.items.iter().any(|item| matches!(
                item,
                ProductProjectionItem::Gate {
                    run_id,
                    gate_kind,
                    gate_ref: projected_gate_ref,
                    auth_context: Some(context),
                    ..
                } if *run_id == turn_run
                    && *gate_kind == ProductGateKind::Auth
                    && projected_gate_ref == gate_ref
                    && context.challenge_kind == AuthPromptChallengeKind::OAuthUrl
                    && context.provider.as_deref() == Some("github")
                    && context.authorization_url.as_deref() == Some("https://github.com/login/oauth/authorize")
            ))
    )));
}

#[tokio::test]
async fn webui_event_stream_uses_credential_requirement_for_manual_token_auth_prompt() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-auth-requirement-thread").unwrap();
    let turn_run = TurnRunId::new();
    let gate_ref = "gate:auth-required";
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let credential_requirements = vec![RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
        setup: Default::default(),
        requester_extension: ExtensionId::new("github").unwrap(),
        provider_scopes: Vec::new(),
    }];
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new(gate_ref).unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                    activity_id: None,
                    credential_requirements: credential_requirements.clone(),
                }),
                sanitized_reason: Some("GitHub authentication required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                credential_requirements,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| matches!(
        event.payload(),
        ProductOutboundPayload::AuthPrompt(prompt)
            if prompt.turn_run_id == turn_run
                && prompt.auth_request_ref == gate_ref
                && prompt.challenge_kind == Some(AuthPromptChallengeKind::ManualToken)
                && prompt.provider.as_deref() == Some("github")
                && prompt.account_label.as_deref() == Some("github")
    )));
    assert!(events.iter().any(|event| matches!(
        event.payload(),
        ProductOutboundPayload::ProjectionUpdate { state }
            if state.items.iter().any(|item| matches!(
                item,
                ProductProjectionItem::Gate {
                    run_id,
                    gate_kind,
                    gate_ref: projected_gate_ref,
                    auth_context: Some(context),
                    ..
                } if *run_id == turn_run
                    && *gate_kind == ProductGateKind::Auth
                    && projected_gate_ref == gate_ref
                    && context.challenge_kind == AuthPromptChallengeKind::ManualToken
                    && context.provider.as_deref() == Some("github")
                    && context.account_label.as_deref() == Some("github")
            ))
    )));
}

#[tokio::test]
async fn webui_event_stream_renders_channel_pairing_requirement_as_manual_token_with_connection() {
    // A channel-pairing credential requirement (Slack) blocks on the same
    // auth-gate rail as GitHub, but the projection renders it as a `manual_token`
    // challenge carrying channel-connection context (the render copy + resolve
    // route), so one paste card serves both a stored secret and a pairing code.
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-channel-pairing-thread").unwrap();
    let turn_run = TurnRunId::new();
    let gate_ref = "gate:auth-required";
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let credential_requirements = vec![RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("slack").unwrap(),
        setup: RuntimeCredentialAccountSetup::ChannelPairing {
            channel: "slack".to_string(),
        },
        requester_extension: ExtensionId::new("slack").unwrap(),
        provider_scopes: Vec::new(),
    }];
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new(gate_ref).unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                    activity_id: None,
                    credential_requirements: credential_requirements.clone(),
                }),
                sanitized_reason: Some("Slack connection required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                credential_requirements,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| matches!(
        event.payload(),
        ProductOutboundPayload::AuthPrompt(prompt)
            if prompt.turn_run_id == turn_run
                && prompt.challenge_kind == Some(AuthPromptChallengeKind::ManualToken)
                && prompt.connection.as_ref().is_some_and(|connection| {
                    connection.channel == "slack"
                        && connection.strategy.as_deref() == Some("inbound_proof_code")
                        && connection.input_placeholder.as_deref()
                            == Some("Enter Slack pairing code...")
                        && connection
                            .error_message
                            .as_deref()
                            .is_some_and(|message| message.contains("/pair"))
                })
    )));
    assert!(events.iter().any(|event| matches!(
        event.payload(),
        ProductOutboundPayload::ProjectionUpdate { state }
            if state.items.iter().any(|item| matches!(
                item,
                ProductProjectionItem::Gate {
                    gate_kind,
                    auth_context: Some(context),
                    ..
                } if *gate_kind == ProductGateKind::Auth
                    && context.challenge_kind == AuthPromptChallengeKind::ManualToken
                    && context
                        .connection
                        .as_ref()
                        .is_some_and(|connection| connection.channel == "slack")
            ))
    )));
}

#[tokio::test]
async fn webui_event_stream_keeps_oauth_requirement_as_oauth_prompt_without_url() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-oauth-fallback-thread").unwrap();
    let turn_run = TurnRunId::new();
    let gate_ref = "gate:auth-required";
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let credential_requirements = vec![RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
        setup: RuntimeCredentialAccountSetup::OAuth {
            scopes: vec!["https://www.googleapis.com/auth/calendar.readonly".to_string()],
        },
        requester_extension: ExtensionId::new("google-calendar").unwrap(),
        provider_scopes: vec!["https://www.googleapis.com/auth/calendar.readonly".to_string()],
    }];
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new(gate_ref).unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                    activity_id: None,
                    credential_requirements: credential_requirements.clone(),
                }),
                sanitized_reason: Some("Google authentication required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                credential_requirements,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events.iter().any(|event| matches!(
            event.payload(),
            ProductOutboundPayload::AuthPrompt(prompt)
                if prompt.turn_run_id == turn_run
                    && prompt.auth_request_ref == gate_ref
                    && prompt.challenge_kind == Some(AuthPromptChallengeKind::OAuthUrl)
                    && prompt.provider.as_deref() == Some("google")
                    && prompt.account_label.is_none()
                    && prompt.authorization_url.is_none()
        )),
        "events: {events:#?}"
    );
}

#[tokio::test]
async fn webui_event_stream_surfaces_auth_challenge_lookup_failure() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-auth-provider-error-thread").unwrap();
    let turn_run = TurnRunId::new();
    let gate_ref = "gate:auth-required";
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new(gate_ref).unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                    activity_id: None,
                    credential_requirements: Vec::new(),
                }),
                sanitized_reason: Some("GitHub authentication required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    )
    .with_auth_challenges(Arc::new(FailingAuthChallengeProvider));

    let error = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .expect_err("auth challenge lookup failure should be surfaced");

    assert!(matches!(
        error,
        ProductAdapterError::WorkflowTransient { .. }
    ));
}

#[tokio::test]
async fn webui_event_stream_creates_google_oauth_prompt_for_runtime_credential_gate() {
    use crate::OAuthClientConfig;
    use crate::auth::{RebornAuthContinuationDispatcher, RebornProductAuthServices};
    use crate::oauth_gate::{GoogleOAuthGateProvider, GoogleOAuthGateProviderRegistry};
    use async_trait::async_trait;
    use ironclaw_auth::{AuthContinuationEvent, InMemoryAuthProductServices};
    use ironclaw_secrets::InMemorySecretStore;

    #[derive(Debug)]
    struct NoopDispatcher;

    #[async_trait]
    impl RebornAuthContinuationDispatcher for NoopDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), ironclaw_auth::AuthProductError> {
            Ok(())
        }
    }

    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-google-auth-thread").unwrap();
    let turn_run = TurnRunId::new();
    let gate_ref = "gate:auth-required";
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let credential_requirements = vec![RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
        setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
            scopes: vec!["https://www.googleapis.com/auth/calendar.readonly".to_string()],
        },
        requester_extension: ExtensionId::new("google-calendar").unwrap(),
        provider_scopes: vec!["https://www.googleapis.com/auth/calendar.readonly".to_string()],
    }];

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let google_gate = Arc::new(GoogleOAuthGateProvider::new(
        OAuthClientConfig::new(
            "google-client.apps.googleusercontent.com",
            "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
            None,
        )
        .unwrap(),
        Arc::new(InMemorySecretStore::new()),
    ));
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher))
            .with_flow_record_source(shared)
            .with_oauth_gate_registry(Arc::new(GoogleOAuthGateProviderRegistry::new(vec![
                google_gate,
            ]))),
    );

    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new(gate_ref).unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                    activity_id: None,
                    credential_requirements: credential_requirements.clone(),
                }),
                sanitized_reason: Some("Google authentication required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                credential_requirements,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    )
    .with_auth_challenges(product_auth);

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| matches!(
        event.payload(),
        ProductOutboundPayload::AuthPrompt(prompt)
            if prompt.turn_run_id == turn_run
                && prompt.auth_request_ref == gate_ref
                && prompt.challenge_kind == Some(AuthPromptChallengeKind::OAuthUrl)
                && prompt.provider.as_deref() == Some("google")
                && prompt.authorization_url.as_deref().is_some_and(|url|
                    url.starts_with("https://accounts.google.com/")
                        && url.contains("https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar.readonly")
                )
                && prompt.account_label.is_none()
    )), "events: {events:#?}");
}

#[tokio::test]
async fn webui_event_stream_creates_notion_dcr_oauth_prompt_for_runtime_credential_gate() {
    use crate::auth::{RebornAuthContinuationDispatcher, RebornProductAuthServices};
    use crate::oauth_dcr::{OAuthDcrProvider, OAuthDcrProviderConfig, OAuthDcrProviderRegistry};
    use async_trait::async_trait;
    use ironclaw_auth::{
        AuthContinuationEvent, CredentialAccountLabel, InMemoryAuthProductServices,
    };
    use ironclaw_capabilities::{CapabilityObligationHandler, CapabilityObligationRequest};
    use ironclaw_secrets::InMemorySecretStore;

    #[derive(Debug)]
    struct NoopDispatcher;

    #[async_trait]
    impl RebornAuthContinuationDispatcher for NoopDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), ironclaw_auth::AuthProductError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct NoopObligationHandler;

    #[async_trait]
    impl CapabilityObligationHandler for NoopObligationHandler {
        async fn satisfy(
            &self,
            _request: CapabilityObligationRequest<'_>,
        ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct RouteDcrEgress;

    #[async_trait]
    impl RuntimeHttpEgress for RouteDcrEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
            let body = match request.url.as_str() {
                "https://mcp.notion.com/mcp/.well-known/oauth-protected-resource" => {
                    br#"{"authorization_servers":["https://oauth.notion.com"]}"#.to_vec()
                }
                "https://oauth.notion.com/.well-known/oauth-authorization-server" => {
                    br#"{"authorization_endpoint":"https://oauth.notion.com/authorize","token_endpoint":"https://oauth.notion.com/token","registration_endpoint":"https://oauth.notion.com/register"}"#.to_vec()
                }
                "https://oauth.notion.com/register" => br#"{"client_id":"dcr-client","registration_client_uri":"https://oauth.notion.com/register/dcr-client","registration_access_token":"registration-token"}"#.to_vec(),
                "https://oauth.notion.com/register/dcr-client"
                    if request.method == NetworkMethod::Delete =>
                {
                    br#"{}"#.to_vec()
                }
                other => panic!("unexpected DCR route egress URL: {other}"),
            };
            Ok(RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                request_bytes: request.body.len() as u64,
                response_bytes: body.len() as u64,
                body,
                saved_body: None,
                redaction_applied: false,
            })
        }
    }

    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-notion-auth-thread").unwrap();
    let turn_run = TurnRunId::new();
    let gate_ref = "gate:auth-required";
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let credential_requirements = vec![RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("notion").unwrap(),
        setup: Default::default(),
        requester_extension: ExtensionId::new("notion").unwrap(),
        provider_scopes: Vec::new(),
    }];

    let shared = Arc::new(InMemoryAuthProductServices::new());
    let dcr_provider = Arc::new(
        OAuthDcrProvider::new(
            OAuthDcrProviderConfig {
                spec: crate::notion_oauth::notion_provider_spec(),
                callback_origin: "http://127.0.0.1:3000".to_string(),
                client_name: "Ironclaw".to_string(),
                account_label: CredentialAccountLabel::new("notion").unwrap(),
                scopes: Vec::new(),
            },
            Arc::new(RouteDcrEgress),
            Arc::new(InMemorySecretStore::new()),
            Arc::new(NoopObligationHandler),
        )
        .unwrap(),
    );
    let product_auth = Arc::new(
        RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher))
            .with_flow_record_source(shared)
            .with_dcr_oauth_registry(Arc::new(OAuthDcrProviderRegistry::new(vec![dcr_provider]))),
    );

    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::BlockedAuth,
                kind: TurnEventKind::Blocked,
                blocked_gate: Some(TurnBlockedGateMetadata {
                    gate_ref: GateRef::new(gate_ref).unwrap(),
                    gate_kind: TurnBlockedGateKind::Auth,
                    activity_id: None,
                    credential_requirements: credential_requirements.clone(),
                }),
                sanitized_reason: Some("Notion authentication required".to_string()),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                credential_requirements,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    )
    .with_auth_challenges(product_auth);

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events.iter().any(|event| matches!(
            event.payload(),
            ProductOutboundPayload::AuthPrompt(prompt)
                if prompt.turn_run_id == turn_run
                    && prompt.auth_request_ref == gate_ref
                    && prompt.challenge_kind == Some(AuthPromptChallengeKind::OAuthUrl)
                    && prompt.provider.as_deref() == Some("notion")
                    && prompt.authorization_url.as_deref().is_some_and(|url|
                        url.starts_with("https://oauth.notion.com/authorize")
                            && url.contains("client_id=dcr-client")
                    )
                    && prompt.account_label.is_none()
        )),
        "events: {events:#?}"
    );
}
