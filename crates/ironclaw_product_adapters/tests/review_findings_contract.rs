use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, EgressCredentialHandle,
    EgressHeader, EgressMethod, EgressPath, EgressRequest, EgressResponse, ExternalActorRef,
    ExternalConversationRef, ExternalEventId, FakeProjectionStream, FakeProtocolHttpEgress,
    InboundCommandPayload, LinkedThreadActionPayload, ParsedProductInbound, ProductAdapterId,
    ProductAttachmentDescriptor, ProductAttachmentKind, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProductProjectionItem, ProductProjectionState, ProductProjectionSubscribeInput,
    ProductRejection, ProductRejectionDisposition, ProductRejectionKind, ProductSurfaceKind,
    ProductTriggerReason, ProjectionCursor, ProjectionStream, ProjectionSubscriptionPayload,
    ProjectionSubscriptionRequest, ProtocolAuthEvidence, ProtocolHttpEgress,
    ProtocolHttpEgressError, TrustedInboundContext, UserMessagePayload,
};
use ironclaw_turns::{AcceptedMessageRef, ReplyTargetBindingRef, TurnRunId};

fn adapter_id() -> ProductAdapterId {
    ProductAdapterId::new("telegram_v2").expect("valid")
}

fn installation_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install_alpha").expect("valid")
}

fn actor(display_name: Option<&str>) -> ExternalActorRef {
    ExternalActorRef::new("telegram_user", "777", display_name).expect("valid")
}

fn conversation(reply_target: Option<&str>) -> ExternalConversationRef {
    ExternalConversationRef::new(None, "12345", Some("topic-7"), reply_target).expect("valid")
}

fn parsed(event_id: &str, conversation: ExternalConversationRef) -> ParsedProductInbound {
    ParsedProductInbound::new(
        ExternalEventId::new(event_id).expect("valid"),
        actor(Some("Alice")),
        conversation,
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("valid"),
        ),
    )
    .expect("valid parsed event")
}

fn trusted_context() -> TrustedInboundContext {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
        },
        "telegram_install_alpha",
    );
    TrustedInboundContext::from_verified_evidence(
        adapter_id(),
        installation_id(),
        chrono::Utc::now(),
        &evidence,
    )
    .expect("verified context")
}

fn envelope_with_payload(event_id: &str, payload: ProductInboundPayload) -> ProductInboundEnvelope {
    ProductInboundEnvelope::from_trusted_parse(
        trusted_context(),
        ParsedProductInbound::new(
            ExternalEventId::new(event_id).expect("valid"),
            actor(Some("Alice")),
            conversation(Some("message-1")),
            payload,
        )
        .expect("parsed"),
    )
    .expect("envelope")
}

fn target(suffix: &str) -> ProductOutboundTarget {
    ProductOutboundTarget::new(
        ReplyTargetBindingRef::new(format!("reply:{suffix}")).expect("valid"),
        conversation(Some("message-1")),
        Some(actor(None)),
    )
}

#[test]
fn trusted_context_stamps_inbound_envelope_fields() {
    let received_at = chrono::Utc::now();
    let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "alice");
    let context = TrustedInboundContext::from_verified_evidence(
        adapter_id(),
        installation_id(),
        received_at,
        &evidence,
    )
    .expect("verified context");
    let parsed = parsed("update:trusted", conversation(Some("reply-a")));
    let envelope = ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope");

    assert_eq!(envelope.adapter_id().as_str(), "telegram_v2");
    assert_eq!(envelope.installation_id().as_str(), "install_alpha");
    assert_eq!(envelope.received_at(), received_at);
    assert_eq!(envelope.auth_claim().subject(), "alice");
}

#[test]
fn failed_auth_evidence_cannot_create_trusted_context() {
    let evidence =
        ProtocolAuthEvidence::failed(ironclaw_product_adapters::ProtocolAuthFailure::Missing);
    assert!(
        TrustedInboundContext::from_verified_evidence(
            adapter_id(),
            installation_id(),
            chrono::Utc::now(),
            &evidence,
        )
        .is_err()
    );
}

#[test]
fn command_payload_is_bounded_and_serde_validated() {
    assert!(InboundCommandPayload::new("help", "", ProductTriggerReason::BotCommand).is_ok());
    assert!(InboundCommandPayload::new("help", "short", ProductTriggerReason::BotCommand).is_ok());
    assert!(
        InboundCommandPayload::new("h".repeat(257), "", ProductTriggerReason::BotCommand).is_err()
    );
    assert!(InboundCommandPayload::new("bad name", "", ProductTriggerReason::BotCommand).is_err());
    assert!(InboundCommandPayload::new("bad/name", "", ProductTriggerReason::BotCommand).is_err());
    assert!(
        InboundCommandPayload::new(
            "help",
            "a".repeat(64 * 1024 + 1),
            ProductTriggerReason::BotCommand
        )
        .is_err()
    );

    let forged = serde_json::json!({
        "command": "h".repeat(257),
        "arguments": "",
        "trigger": "bot_command"
    });
    assert!(serde_json::from_value::<InboundCommandPayload>(forged).is_err());

    let oversized_gate = serde_json::json!({
        "gate_ref": "g".repeat(513),
        "decision": "approve_once"
    });
    assert!(
        serde_json::from_value::<ironclaw_product_adapters::ApprovalResolutionPayload>(
            oversized_gate
        )
        .is_err()
    );
    let newline_gate = serde_json::json!({
        "gate_ref": "gate\nattack",
        "decision": "approve_once"
    });
    assert!(
        serde_json::from_value::<ironclaw_product_adapters::ApprovalResolutionPayload>(
            newline_gate
        )
        .is_err()
    );

    let oversized_auth = serde_json::json!({
        "auth_request_ref": "auth-1",
        "result": {"credential_provided": {"credential_ref": "c".repeat(513)}}
    });
    assert!(
        serde_json::from_value::<ironclaw_product_adapters::AuthResolutionPayload>(oversized_auth)
            .is_err()
    );

    assert!(
        LinkedThreadActionPayload::new("open-thread", None, Some("reply\nattack".into()),).is_err()
    );
}

#[test]
fn attachment_descriptor_normalizes_and_validates_metadata() {
    assert!(
        ProductAttachmentDescriptor::new(
            "file_42",
            "Image/JPEG",
            Some("photo.jpg".into()),
            Some(2048),
            ProductAttachmentKind::Image,
        )
        .is_err()
    );
    assert!(
        ProductAttachmentDescriptor::new(
            "file_42",
            "image/jpeg",
            Some("a".repeat(257)),
            Some(2048),
            ProductAttachmentKind::Image,
        )
        .is_err()
    );
    assert!(
        ProductAttachmentDescriptor::new(
            "file_42",
            "image/jpeg",
            Some("photo.jpg".into()),
            Some(2048),
            ProductAttachmentKind::Audio,
        )
        .is_err()
    );
}

#[test]
fn serde_reuses_validated_identifier_constructors() {
    assert!(serde_json::from_str::<ExternalEventId>("\"bad\\nvalue\"").is_err());
    assert!(serde_json::from_str::<ProductAdapterId>(&format!("\"{}\"", "a".repeat(257))).is_err());

    let forged_actor = serde_json::json!({
        "kind": "telegram_user",
        "id": "777\n888",
        "display_name": null
    });
    assert!(serde_json::from_value::<ExternalActorRef>(forged_actor).is_err());

    let forged_conversation = serde_json::json!({
        "space_id": null,
        "conversation_id": "12345",
        "topic_id": "topic\n7",
        "reply_target_message_id": null
    });
    assert!(serde_json::from_value::<ExternalConversationRef>(forged_conversation).is_err());
}

#[test]
fn external_identity_hashing_uses_only_stable_keys() {
    let alice = actor(Some("Alice"));
    let renamed = actor(Some("Alice Cooper"));
    assert_eq!(alice, renamed);

    let reply_a = conversation(Some("message-a"));
    let reply_b = conversation(Some("message-b"));
    assert_eq!(reply_a, reply_b);
    assert_eq!(
        reply_a.conversation_fingerprint(),
        reply_b.conversation_fingerprint()
    );

    let ambiguous_a = ExternalConversationRef::new(Some("a;conversation=b"), "c", Some("d"), None)
        .expect("valid");
    let ambiguous_b =
        ExternalConversationRef::new(Some("a"), "b;topic=c", Some("d"), None).expect("valid");
    assert_ne!(
        ambiguous_a.conversation_fingerprint(),
        ambiguous_b.conversation_fingerprint()
    );
}

#[test]
fn egress_request_validates_path_method_and_forbidden_headers() {
    assert!(EgressMethod::new("CONNECT").is_err());
    assert!(EgressPath::new("//169.254.169.254/latest").is_err());
    assert!(EgressPath::new("https://evil.example/path").is_err());
    assert!(EgressHeader::new("Authorization", "Bearer token").is_err());
    assert!(serde_json::from_str::<EgressMethod>("\"CONNECT\"").is_err());
    assert!(
        serde_json::from_value::<EgressHeader>(serde_json::json!({
            "name": "Authorization",
            "value": "Bearer token"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<EgressHeader>(serde_json::json!({
            "name": "X-Test",
            "value": "ok\r\nInjected: yes"
        }))
        .is_err()
    );

    let request = EgressRequest::new(
        DeclaredEgressHost::new("api.telegram.org").expect("valid"),
        EgressMethod::post(),
        EgressPath::new("/bot/sendMessage").expect("valid"),
    )
    .with_header(EgressHeader::new("X-Custom", "one").expect("valid"))
    .with_header(EgressHeader::new("X-Custom", "two").expect("valid"))
    .with_body(br#"{"text":"hi"}"#.to_vec())
    .with_credential_handle(Some(
        EgressCredentialHandle::new("telegram_bot_token").expect("valid"),
    ));
    assert_eq!(request.headers().len(), 2);
}

#[test]
fn egress_response_does_not_expose_raw_headers() {
    let response = EgressResponse::new(200, br#"{"ok":true}"#.to_vec());
    assert_eq!(response.status(), 200);
    assert_eq!(response.body(), br#"{"ok":true}"#);
}

#[tokio::test]
async fn fake_egress_queues_same_host_responses_and_preserves_duplicate_headers() {
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    egress.program_response(
        "api.telegram.org",
        Ok(EgressResponse::new(429, b"rate limited".to_vec())),
    );
    egress.program_response(
        "api.telegram.org",
        Ok(EgressResponse::new(200, b"ok".to_vec())),
    );

    let make_request = || {
        EgressRequest::new(
            DeclaredEgressHost::new("api.telegram.org").expect("valid"),
            EgressMethod::post(),
            EgressPath::new("/bot/sendMessage").expect("valid"),
        )
        .with_header(EgressHeader::new("X-Test", "one").expect("valid"))
        .with_header(EgressHeader::new("X-Test", "two").expect("valid"))
        .with_credential_handle(Some(
            EgressCredentialHandle::new("telegram_bot_token").expect("valid"),
        ))
    };

    assert_eq!(
        egress.send(make_request()).await.expect("first").status(),
        429
    );
    assert_eq!(
        egress.send(make_request()).await.expect("second").status(),
        200
    );
    assert_eq!(egress.calls()[0].headers.len(), 2);
}

#[test]
fn projection_payloads_have_single_cursor_and_renderable_state() {
    let cursor = ProjectionCursor::new("thread:42#cursor:7").expect("valid");
    let state = ProductProjectionState::new(
        "thread-1",
        vec![ProductProjectionItem::Text {
            id: "message-1".into(),
            run_id: None,
            body: "hello".into(),
        }],
    )
    .expect("state");
    let json = serde_json::to_string(&state).expect("serialize");
    assert!(json.contains("text"));
    let parsed: ProductProjectionState = serde_json::from_str(&json).expect("round trip");
    assert_eq!(parsed, state);
    let envelope = ProductOutboundEnvelope::new(
        adapter_id(),
        installation_id(),
        target("projection"),
        cursor.clone(),
        ProductOutboundPayload::ProjectionSnapshot { state },
    );
    assert_eq!(envelope.projection_cursor(), &cursor);
    assert!(matches!(
        envelope.payload(),
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
}

#[tokio::test]
async fn fake_projection_stream_filters_by_subscription_request() {
    let stream = FakeProjectionStream::new();
    let cursor = ProjectionCursor::new("cursor:1").expect("valid");
    let envelope = ProductOutboundEnvelope::new(
        adapter_id(),
        installation_id(),
        target("projection"),
        cursor.clone(),
        ProductOutboundPayload::FinalReply(ironclaw_product_adapters::FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hi".into(),
            generated_at: chrono::Utc::now(),
        }),
    );
    stream.push_for_request(sample_subscription(None), envelope.clone());

    assert!(
        stream
            .drain(sample_subscription(Some(cursor)))
            .await
            .expect("wrong cursor")
            .is_empty()
    );
    assert_eq!(
        stream
            .drain(sample_subscription(None))
            .await
            .expect("matching")
            .len(),
        1
    );
}

#[test]
fn projection_input_uses_trusted_inbound_envelope_context() {
    let request_envelope = envelope_with_payload(
        "update:subscription",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(Some("thread-1".into()), None).expect("valid"),
        ),
    );
    assert_eq!(
        request_envelope.auth_claim().subject(),
        "telegram_install_alpha"
    );
    let input = ProductProjectionSubscribeInput::from_inbound_envelope(&request_envelope)
        .expect("projection input");
    assert_eq!(input.thread_id_hint.as_deref(), Some("thread-1"));
}

fn sample_subscription(after_cursor: Option<ProjectionCursor>) -> ProjectionSubscriptionRequest {
    ProjectionSubscriptionRequest {
        actor: ironclaw_turns::TurnActor::new(
            ironclaw_host_api::UserId::new("alice").expect("valid"),
        ),
        scope: ironclaw_turns::TurnScope::new(
            ironclaw_host_api::TenantId::new("tenant-a").expect("valid"),
            None,
            None,
            ironclaw_host_api::ThreadId::new("thread-1").expect("valid"),
        ),
        after_cursor,
    }
}

#[test]
fn ack_and_rejection_types_are_unambiguous_and_typed() {
    let ack = ProductInboundAck::Accepted {
        accepted_message_ref: AcceptedMessageRef::new("msg:1").expect("valid"),
        submitted_run_id: TurnRunId::new(),
    };
    assert_eq!(
        ack.retry_disposition(),
        ironclaw_product_adapters::InboundRetryDisposition::DoNotRetry
    );

    let rejection = ProductRejection::retryable(ProductRejectionKind::PolicyDenied, "rate limited");
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Retryable
    );
    assert!(!ProductInboundAck::Rejected(rejection).is_durable_outcome());
    assert!(
        ProductInboundAck::Rejected(ProductRejection::permanent(
            ProductRejectionKind::PolicyDenied,
            "policy denied",
        ))
        .is_durable_outcome()
    );
    assert!(ironclaw_product_adapters::fakes::ensure_durable_outcome(
        &ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("msg:2").expect("valid"),
            submitted_run_id: TurnRunId::new(),
        }
    ));
    assert!(!ironclaw_product_adapters::fakes::ensure_noop_outcome(
        &ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("msg:3").expect("valid"),
            submitted_run_id: TurnRunId::new(),
        }
    ));
    assert!(ironclaw_product_adapters::fakes::ensure_noop_outcome(
        &ProductInboundAck::NoOp
    ));
}

#[test]
fn transient_egress_errors_are_retryable_adapter_errors() {
    let timeout: ironclaw_product_adapters::ProductAdapterError =
        ProtocolHttpEgressError::Timeout.into();
    assert!(timeout.is_retryable());
}

#[test]
fn product_surface_contracts_are_explicit() {
    assert!(!ProductSurfaceKind::Web.uses_push_delivery());
    assert!(ProductSurfaceKind::Web.supports_synchronous_response());
    assert!(ProductSurfaceKind::ExternalChannel.uses_push_delivery());
}
