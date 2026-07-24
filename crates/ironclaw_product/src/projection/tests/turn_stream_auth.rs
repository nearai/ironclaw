use super::*;

use crate::{
    AuthChallengeProvider, AuthChallengeView, AuthPromptChallengeKind,
    ChannelConnectionRequirement, ChannelPairingCode, ChannelPairingIssue,
    PairingAuthChallengeView, RebornChannelConnectStrategy,
};
use ironclaw_auth::{AuthProviderId, OAuthAuthorizationUrl};
use ironclaw_host_api::{
    RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, VendorId,
};

struct FakeAuthChallengeProvider {
    expected_owner_user_id: UserId,
    expected_run_id: TurnRunId,
    expected_gate_ref: String,
}

struct FailingAuthChallengeProvider;

struct FakePairingAuthChallengeProvider;

#[async_trait]
impl AuthChallengeProvider for FakeAuthChallengeProvider {
    async fn challenge_for_gate(
        &self,
        _scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
        _credential_requirements: &[RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, ironclaw_auth::AuthProductError> {
        if owner_user_id != &self.expected_owner_user_id
            || run_id != self.expected_run_id
            || gate_ref != self.expected_gate_ref
        {
            return Ok(None);
        }
        Ok(Some(AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: AuthProviderId::new("github".to_string()).unwrap(),
            account_label: None,
            authorization_url: Some(
                OAuthAuthorizationUrl::new("https://github.com/login/oauth/authorize".to_string())
                    .unwrap(),
            ),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
            pairing: None,
        }))
    }
}

#[async_trait]
impl AuthChallengeProvider for FailingAuthChallengeProvider {
    async fn challenge_for_gate(
        &self,
        _scope: &TurnScope,
        _owner_user_id: &UserId,
        _run_id: TurnRunId,
        _gate_ref: &str,
        _credential_requirements: &[RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, ironclaw_auth::AuthProductError> {
        Err(ironclaw_auth::AuthProductError::BackendUnavailable)
    }
}

#[async_trait]
impl AuthChallengeProvider for FakePairingAuthChallengeProvider {
    async fn challenge_for_gate(
        &self,
        _scope: &TurnScope,
        _owner_user_id: &UserId,
        _run_id: TurnRunId,
        _gate_ref: &str,
        _credential_requirements: &[RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, ironclaw_auth::AuthProductError> {
        Ok(Some(AuthChallengeView {
            kind: AuthPromptChallengeKind::Pairing,
            provider: AuthProviderId::new("telegram".to_string()).unwrap(),
            account_label: None,
            authorization_url: None,
            expires_at: None,
            pairing: Some(PairingAuthChallengeView {
                issue: ChannelPairingIssue {
                    code: ChannelPairingCode::new("ABCDEFGH").unwrap(),
                    deep_link: Some("https://t.me/ironclaw_bot?start=ABCDEFGH".to_string()),
                    expires_at: chrono::Utc::now() + chrono::Duration::minutes(15),
                },
                connection: ChannelConnectionRequirement {
                    channel: "telegram".to_string(),
                    display_name: "Telegram".to_string(),
                    strategy: RebornChannelConnectStrategy::WebGeneratedCode,
                    instructions: "Send the generated code to the Telegram bot.".to_string(),
                    input_placeholder: String::new(),
                    submit_label: "Open pairing".to_string(),
                    error_message: "Telegram pairing failed.".to_string(),
                },
            }),
        }))
    }
}

#[tokio::test]
async fn product_event_stream_enriches_auth_prompt_through_projection_stream() {
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
                detail: None,
                retryable: None,
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
        .product_event_stream()
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
async fn product_event_stream_projects_pairing_prompt_without_text_input_placeholder() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-telegram-pairing-thread").unwrap();
    let turn_run = TurnRunId::new();
    let gate_ref = "gate:telegram-pairing";
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let credential_requirements = vec![RuntimeCredentialAuthRequirement {
        provider: VendorId::new("telegram").unwrap(),
        setup: RuntimeCredentialAccountSetup::Pairing,
        requester_extension: ExtensionId::new("telegram").unwrap(),
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
                sanitized_reason: Some("Telegram pairing required".to_string()),
                detail: None,
                retryable: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                gate_ref: Some(GateRef::new(gate_ref).unwrap()),
                credential_requirements,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    )
    .with_auth_challenges(Arc::new(FakePairingAuthChallengeProvider));

    let events = services
        .product_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    let prompt = events
        .iter()
        .find_map(|event| match event.payload() {
            ProductOutboundPayload::AuthPrompt(prompt) => Some(prompt),
            _ => None,
        })
        .expect("pairing auth prompt");
    assert_eq!(prompt.turn_run_id, turn_run);
    assert_eq!(
        prompt.challenge_kind,
        Some(AuthPromptChallengeKind::Pairing)
    );
    assert_eq!(prompt.provider.as_deref(), Some("telegram"));
    assert_eq!(
        prompt
            .connection
            .as_ref()
            .expect("connection context")
            .input_placeholder,
        None
    );
    assert_eq!(
        prompt.pairing.as_ref().expect("pairing context").code,
        "ABCDEFGH"
    );

    let auth_context = events
        .iter()
        .find_map(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => {
                state.items.iter().find_map(|item| match item {
                    ProductProjectionItem::Gate {
                        gate_kind,
                        auth_context,
                        ..
                    } if *gate_kind == ProductGateKind::Auth => auth_context.as_deref(),
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("projected pairing auth context");
    assert_eq!(
        auth_context
            .connection
            .as_ref()
            .expect("projected connection context")
            .input_placeholder,
        None
    );
    assert_eq!(
        auth_context
            .pairing
            .as_ref()
            .expect("projected pairing context")
            .code,
        "ABCDEFGH"
    );
}

#[tokio::test]
async fn product_event_stream_uses_credential_requirement_for_manual_token_auth_prompt() {
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
        provider: VendorId::new("github").unwrap(),
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
                detail: None,
                retryable: None,
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
        .product_event_stream()
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
async fn product_event_stream_keeps_retired_channel_pairing_requirement_generic() {
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
        provider: VendorId::new("slack").unwrap(),
        setup: RuntimeCredentialAccountSetup::Retired,
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
                retryable: None,
                detail: None,
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
        .product_event_stream()
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
                && prompt.provider.as_deref() == Some("slack")
                && prompt.challenge_kind.is_none()
                && prompt.connection.is_none()
    )));
    assert!(events.iter().any(|event| matches!(
        event.payload(),
        ProductOutboundPayload::ProjectionUpdate { state }
            if state.items.iter().any(|item| matches!(
                item,
                ProductProjectionItem::Gate {
                    gate_kind,
                    auth_context,
                    ..
                } if *gate_kind == ProductGateKind::Auth
                    && auth_context.is_none()
            ))
    )));
}

#[tokio::test]
async fn product_event_stream_keeps_oauth_requirement_as_oauth_prompt_without_url() {
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
        provider: VendorId::new("google").unwrap(),
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
                detail: None,
                retryable: None,
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
        .product_event_stream()
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
async fn product_event_stream_surfaces_auth_challenge_lookup_failure() {
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
                detail: None,
                retryable: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    )
    .with_auth_challenges(Arc::new(FailingAuthChallengeProvider));

    let error = services
        .product_event_stream()
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
