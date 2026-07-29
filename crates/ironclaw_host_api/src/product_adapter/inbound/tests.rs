use super::*;
use crate::product_adapter::auth::AuthRequirement;
use crate::product_adapter::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentKind,
};

#[test]
fn user_message_payload_round_trips_and_filters_requested_model() {
    let with_model = UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat)
        .unwrap()
        .with_requested_model(Some("gpt-4o".to_string()));
    assert_eq!(with_model.requested_model.as_deref(), Some("gpt-4o"));
    // Round-trips over the wire (custom Deserialize via the wire struct).
    let decoded: UserMessagePayload =
        serde_json::from_str(&serde_json::to_string(&with_model).unwrap()).unwrap();
    assert_eq!(decoded.requested_model.as_deref(), Some("gpt-4o"));

    // Omitted → None, and not serialized when absent.
    let without = UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat).unwrap();
    assert!(without.requested_model.is_none());
    assert!(
        !serde_json::to_string(&without)
            .unwrap()
            .contains("requested_model")
    );

    // An empty requested model is filtered to None.
    assert!(
        UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat)
            .unwrap()
            .with_requested_model(Some(String::new()))
            .requested_model
            .is_none()
    );
}

#[test]
fn user_message_payload_bounds_requested_model_on_every_path() {
    let over_limit = "m".repeat(REQUESTED_MODEL_MAX_BYTES + 1);

    // Explicit validation after the builder rejects an over-long hint.
    let built = UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat)
        .unwrap()
        .with_requested_model(Some(over_limit.clone()));
    assert!(built.validate().is_err());

    // Deserialization must not smuggle an unbounded hint past validation:
    // the wire path attaches `requested_model` after `new`, so it re-validates.
    let wire = serde_json::json!({
        "text": "hi",
        "attachments": [],
        "trigger": "direct_chat",
        "requested_model": over_limit,
    })
    .to_string();
    let decoded: Result<UserMessagePayload, _> = serde_json::from_str(&wire);
    assert!(
        decoded.is_err(),
        "an over-long requested_model must be rejected during deserialization"
    );

    // A hint at the cap is accepted on both paths.
    let at_cap = "m".repeat(REQUESTED_MODEL_MAX_BYTES);
    assert!(
        UserMessagePayload::new("hi", vec![], ProductTriggerReason::DirectChat)
            .unwrap()
            .with_requested_model(Some(at_cap))
            .validate()
            .is_ok()
    );
}

fn sample_context() -> TrustedInboundContext {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
        },
        "telegram_install_alpha",
    );
    TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new("telegram_v2").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        Utc::now(),
        &evidence,
    )
    .expect("verified")
}

fn sample_parsed(payload: ProductInboundPayload) -> ParsedProductInbound {
    ParsedProductInbound::new(
        ExternalEventId::new("update:42").expect("valid"),
        ExternalActorRef::new("telegram_user", "777", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-100"))
            .expect("valid"),
        payload,
    )
    .expect("parsed")
}

fn attachment_descriptor(id: &str) -> ProductAttachmentDescriptor {
    ProductAttachmentDescriptor::new(
        id,
        "application/pdf",
        Some(format!("{id}.pdf")),
        Some(4),
        ProductAttachmentKind::Document,
    )
    .expect("descriptor")
}

fn envelope_with_channel_sources(sources: &[(&str, &str)]) -> ProductInboundEnvelope {
    let descriptors = sources
        .iter()
        .map(|(id, _)| attachment_descriptor(id))
        .collect::<Vec<_>>();
    let refs = descriptors
        .iter()
        .zip(sources)
        .map(|(descriptor, (_, vendor_ref))| ChannelAttachmentRef {
            descriptor: descriptor.clone(),
            vendor_ref: (*vendor_ref).to_string(),
        })
        .collect::<Vec<_>>();
    ProductInboundEnvelope::from_trusted_parse(
        sample_context(),
        sample_parsed(ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "review the attachments",
                descriptors,
                ProductTriggerReason::DirectChat,
            )
            .expect("payload"),
        )),
    )
    .expect("envelope")
    .with_channel_attachment_refs(refs)
    .expect("matching refs")
}

#[test]
fn user_message_text_length_bounded() {
    let oversize = "a".repeat(USER_MESSAGE_TEXT_MAX_BYTES + 1);
    assert!(UserMessagePayload::new(oversize, vec![], ProductTriggerReason::DirectChat).is_err());
}

#[test]
fn user_message_text_length_bounded_through_serde() {
    let empty = serde_json::json!({
        "text": "",
        "attachments": [],
        "trigger": "direct_chat"
    });
    assert!(serde_json::from_value::<UserMessagePayload>(empty).is_ok());

    let at_limit = serde_json::json!({
        "text": "a".repeat(USER_MESSAGE_TEXT_MAX_BYTES),
        "attachments": [],
        "trigger": "direct_chat"
    });
    assert!(serde_json::from_value::<UserMessagePayload>(at_limit).is_ok());

    let forged = serde_json::json!({
        "text": "a".repeat(USER_MESSAGE_TEXT_MAX_BYTES + 1),
        "attachments": [],
        "trigger": "direct_chat"
    });
    assert!(serde_json::from_value::<UserMessagePayload>(forged).is_err());
}

#[test]
fn command_payload_bounds_are_enforced_through_serde() {
    assert!(
        InboundCommandPayload::new(
            "h".repeat(COMMAND_MAX_BYTES + 1),
            "",
            ProductTriggerReason::BotCommand
        )
        .is_err()
    );
    assert!(InboundCommandPayload::new("bad name", "", ProductTriggerReason::BotCommand).is_err());
    assert!(InboundCommandPayload::new("bad/name", "", ProductTriggerReason::BotCommand).is_err());
    let empty_command = serde_json::json!({
        "command": "",
        "arguments": "",
        "trigger": "bot_command"
    });
    assert!(serde_json::from_value::<InboundCommandPayload>(empty_command).is_err());

    let at_limit = serde_json::json!({
        "command": "h".repeat(COMMAND_MAX_BYTES),
        "arguments": "",
        "trigger": "bot_command"
    });
    assert!(serde_json::from_value::<InboundCommandPayload>(at_limit).is_ok());

    let forged = serde_json::json!({
        "command": "h".repeat(COMMAND_MAX_BYTES + 1),
        "arguments": "",
        "trigger": "bot_command"
    });
    assert!(serde_json::from_value::<InboundCommandPayload>(forged).is_err());

    let forged_slash = serde_json::json!({
        "command": "bad/name",
        "arguments": "",
        "trigger": "bot_command"
    });
    assert!(serde_json::from_value::<InboundCommandPayload>(forged_slash).is_err());
}

#[test]
fn channel_inbound_classifier_routes_interactions_and_commands() {
    assert!(matches!(
        classify_channel_inbound_text("`auth deny gate:auth-1`", ProductTriggerReason::DirectChat,),
        Some(ChannelInboundClassification::AuthResolution(_))
    ));
    assert!(matches!(
        classify_channel_inbound_text("approve gate:approval-1", ProductTriggerReason::BotMention,),
        Some(ChannelInboundClassification::ApprovalResolution(_))
    ));
    match classify_channel_inbound_text(
        "/model set-provider openai --model gpt-5",
        ProductTriggerReason::DirectChat,
    ) {
        Some(ChannelInboundClassification::Command(command)) => {
            assert_eq!(command.command, "model");
            assert_eq!(command.arguments, "set-provider openai --model gpt-5");
            assert_eq!(command.trigger, ProductTriggerReason::DirectChat);
        }
        other => panic!("expected command classification, got {other:?}"),
    }
}

#[test]
fn channel_inbound_classifier_preserves_natural_language_and_fails_closed() {
    for text in [
        "hello",
        "approve this design",
        "approve gate:approval-1 but do not run it",
        "deny gate:approval-1 because the scope changed",
        "auth deny",
        "auth deny this",
        "/model@other_bot",
        "/model@other_bot openai/gpt-5",
    ] {
        assert_eq!(
            classify_channel_inbound_text(text, ProductTriggerReason::DirectChat),
            None,
            "{text:?} must remain an ordinary user message"
        );
    }
    for text in [
        "auth deny gate:",
        "approve gate:",
        "auth deny gate:bad\0ref",
        "/bad\\command",
    ] {
        assert_eq!(
            classify_channel_inbound_text(text, ProductTriggerReason::DirectChat),
            Some(ChannelInboundClassification::NoOp),
            "{text:?} is confident reserved syntax and must fail closed"
        );
    }
}

#[test]
fn channel_command_classification_converts_to_product_payload() {
    let command =
        InboundCommandPayload::new("model", "openai/gpt-5", ProductTriggerReason::BotCommand)
            .expect("valid command");
    assert!(matches!(
        ProductInboundPayload::from(ChannelInboundClassification::Command(command)),
        ProductInboundPayload::Command(_)
    ));
}

#[test]
fn envelope_is_built_from_trusted_context() {
    let envelope = ProductInboundEnvelope::from_trusted_parse(
        sample_context(),
        sample_parsed(ProductInboundPayload::NoOp),
    )
    .expect("envelope");
    assert_eq!(envelope.adapter_id().as_str(), "telegram_v2");
    assert_eq!(envelope.source_channel().as_str(), "telegram_v2");
    assert_eq!(envelope.payload(), &ProductInboundPayload::NoOp);
}

#[test]
fn channel_attachment_refs_must_match_descriptors_and_stay_transient() {
    let descriptor = crate::product_adapter::external::ProductAttachmentDescriptor::new(
        "file-1",
        "application/pdf",
        Some("report.pdf".to_string()),
        Some(4),
        crate::product_adapter::external::ProductAttachmentKind::Document,
    )
    .expect("descriptor");
    let source = ChannelAttachmentRef {
        descriptor: descriptor.clone(),
        vendor_ref: "opaque-provider-file-reference".to_string(),
    };
    let payload = ProductInboundPayload::UserMessage(
        UserMessagePayload::new(
            "review the report",
            vec![descriptor],
            ProductTriggerReason::DirectChat,
        )
        .expect("payload"),
    );

    // Mismatched refs fail closed before any transfer authority exists.
    let mismatched = ChannelAttachmentRef {
        descriptor: crate::product_adapter::external::ProductAttachmentDescriptor::new(
            "other-file",
            "application/pdf",
            None,
            None,
            crate::product_adapter::external::ProductAttachmentKind::Document,
        )
        .expect("descriptor"),
        vendor_ref: "other".to_string(),
    };
    let envelope =
        ProductInboundEnvelope::from_trusted_parse(sample_context(), sample_parsed(payload))
            .expect("envelope");
    assert!(
        envelope
            .clone()
            .with_channel_attachment_refs(vec![mismatched])
            .is_err(),
        "refs that do not match the payload descriptors are rejected"
    );

    // Matching refs stamp, are readable host-side, and never serialize —
    // the provider transfer reference is transient host state.
    let envelope = envelope
        .with_channel_attachment_refs(vec![source])
        .expect("matching refs stamp");
    assert_eq!(envelope.channel_attachment_refs().len(), 1);
    let serialized = serde_json::to_string(&envelope).expect("envelope serializes");
    assert!(
        !serialized.contains("opaque-provider-file-reference"),
        "provider transfer references must not enter the serialized envelope"
    );
}

#[test]
fn rewritten_user_message_rejects_ambiguous_channel_sources() {
    let envelope =
        envelope_with_channel_sources(&[("duplicate", "vendor-a"), ("duplicate", "vendor-b")]);
    let rewrite = UserMessagePayload::new(
        "keep one",
        vec![attachment_descriptor("duplicate")],
        ProductTriggerReason::DirectChat,
    )
    .expect("rewrite");

    let error = envelope
        .with_rewritten_user_message(rewrite)
        .expect_err("a descriptor matching two vendor refs is ambiguous");
    assert!(matches!(
        error,
        ProductAdapterError::MalformedInboundPayload { .. }
    ));
}

#[test]
fn rewritten_user_message_rejects_reusing_one_channel_source() {
    let envelope = envelope_with_channel_sources(&[("single", "vendor-one")]);
    let descriptor = attachment_descriptor("single");
    let rewrite = UserMessagePayload::new(
        "duplicate the attachment",
        vec![descriptor.clone(), descriptor],
        ProductTriggerReason::DirectChat,
    )
    .expect("rewrite");

    let error = envelope
        .with_rewritten_user_message(rewrite)
        .expect_err("one vendor ref cannot satisfy two rewritten attachments");
    assert!(matches!(
        error,
        ProductAdapterError::MalformedInboundPayload { .. }
    ));
}

#[test]
fn rewritten_user_message_remaps_multiple_channel_sources_in_policy_order() {
    let envelope =
        envelope_with_channel_sources(&[("first", "vendor-first"), ("second", "vendor-second")]);
    let rewrite = UserMessagePayload::new(
        "reverse the attachments",
        vec![
            attachment_descriptor("second"),
            attachment_descriptor("first"),
        ],
        ProductTriggerReason::DirectChat,
    )
    .expect("rewrite");

    let rewritten = envelope
        .with_rewritten_user_message(rewrite)
        .expect("unique sources remap");
    let vendor_refs = rewritten
        .channel_attachment_refs()
        .iter()
        .map(|source| source.vendor_ref.as_str())
        .collect::<Vec<_>>();
    assert_eq!(vendor_refs, ["vendor-second", "vendor-first"]);
}

#[test]
fn trusted_context_can_stamp_explicit_source_channel() {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
        },
        "telegram_install_alpha",
    );
    let context = TrustedInboundContext::from_verified_evidence_with_source_channel(
        ProductAdapterId::new("extension_gateway").expect("valid"),
        ProductSourceChannel::new("vendor_chat").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        Utc::now(),
        &evidence,
    )
    .expect("verified");
    let envelope = ProductInboundEnvelope::from_trusted_parse(
        context,
        sample_parsed(ProductInboundPayload::NoOp),
    )
    .expect("envelope");
    assert_eq!(envelope.adapter_id().as_str(), "extension_gateway");
    assert_eq!(envelope.source_channel().as_str(), "vendor_chat");
}

#[test]
fn rewritten_user_message_rejects_non_user_message_envelope() {
    let envelope = ProductInboundEnvelope::from_trusted_parse(
        sample_context(),
        sample_parsed(ProductInboundPayload::NoOp),
    )
    .expect("envelope");
    let rewrite = UserMessagePayload::new("rewritten", vec![], ProductTriggerReason::DirectChat)
        .expect("valid rewrite");

    let err = envelope
        .with_rewritten_user_message(rewrite)
        .expect_err("non-user-message envelope must not be rewritten");

    assert!(matches!(
        err,
        ProductAdapterError::MalformedInboundPayload { .. }
    ));
}

#[test]
fn failed_auth_cannot_build_context() {
    let evidence =
        ProtocolAuthEvidence::failed(crate::product_adapter::ProtocolAuthFailure::Missing);
    assert!(
        TrustedInboundContext::from_verified_evidence(
            ProductAdapterId::new("telegram_v2").expect("valid"),
            AdapterInstallationId::new("install_alpha").expect("valid"),
            Utc::now(),
            &evidence,
        )
        .is_err()
    );
}

#[test]
fn ack_durable_outcomes_classify_correctly() {
    assert!(
        ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("msg").expect("valid"),
            submitted_run_id: TurnRunId::new(),
        }
        .is_durable_outcome()
    );
    assert!(ProductInboundAck::NoOp.is_durable_outcome());
    assert!(
        ProductInboundAck::CommandResult {
            command: "extension_install".to_string(),
            payload: ProductCommandResultPayload::new(serde_json::json!({
                "phase": "installed",
            })),
        }
        .is_durable_outcome()
    );
    assert!(
        ProductInboundAck::Rejected(ProductRejection::permanent(
            ProductRejectionKind::PolicyDenied,
            "policy denied",
        ))
        .is_durable_outcome()
    );
    assert!(
        !ProductInboundAck::Rejected(ProductRejection::retryable(
            ProductRejectionKind::PolicyDenied,
            "rate limited",
        ))
        .is_durable_outcome()
    );
    assert_eq!(
        ProductInboundAck::Duplicate {
            prior: Box::new(ProductInboundAck::NoOp),
        }
        .retry_disposition(),
        InboundRetryDisposition::ReplayPrior
    );
}

#[test]
fn rejection_kind_user_facing_hint_is_exhaustive_and_sanitized() {
    // Every variant must return a non-empty, static hint with no internal state.
    let cases = [
        (ProductRejectionKind::BindingRequired, "approve gate:"),
        (ProductRejectionKind::AccessDenied, "access"),
        (ProductRejectionKind::UnknownInstallation, "workspace"),
        (ProductRejectionKind::InvalidRequest, "approve"),
        (ProductRejectionKind::PolicyDenied, "policy"),
        (ProductRejectionKind::AmbiguousResolution, "approve gate:"),
    ];
    for (kind, expected_substr) in &cases {
        let hint = kind.user_facing_hint();
        assert!(!hint.is_empty(), "{kind:?} hint must not be empty");
        assert!(
            hint.contains(expected_substr),
            "{kind:?} hint '{hint}' must contain '{expected_substr}'"
        );
    }

    // Hints must be pairwise distinct — two kinds sharing a hint would
    // make the user-facing feedback ambiguous about what went wrong.
    let mut hints: Vec<&str> = cases
        .iter()
        .map(|(kind, _)| kind.user_facing_hint())
        .collect();
    hints.sort_unstable();
    hints.dedup();
    assert_eq!(
        hints.len(),
        cases.len(),
        "every ProductRejectionKind must have a distinct user-facing hint"
    );
}

#[test]
fn rejection_kind_user_facing_auth_hint_overrides_approval_kinds_and_falls_through() {
    // BindingRequired and InvalidRequest must return auth-specific guidance,
    // not the approval-command text from user_facing_hint().
    let binding_hint = ProductRejectionKind::BindingRequired.user_facing_auth_hint();
    assert!(
        binding_hint.contains("auth deny"),
        "BindingRequired auth hint must reference 'auth deny', got: {binding_hint}"
    );
    assert!(
        !binding_hint.contains("approve gate:"),
        "BindingRequired auth hint must not contain approval command, got: {binding_hint}"
    );

    let invalid_hint = ProductRejectionKind::InvalidRequest.user_facing_auth_hint();
    assert!(
        invalid_hint.contains("auth deny"),
        "InvalidRequest auth hint must reference 'auth deny', got: {invalid_hint}"
    );
    assert!(
        !invalid_hint.contains("approve"),
        "InvalidRequest auth hint must not contain approval command, got: {invalid_hint}"
    );

    // AmbiguousResolution must also return auth-specific guidance, not approval text.
    let ambiguous_hint = ProductRejectionKind::AmbiguousResolution.user_facing_auth_hint();
    assert!(
        ambiguous_hint.contains("auth deny"),
        "AmbiguousResolution auth hint must reference 'auth deny', got: {ambiguous_hint}"
    );

    // All other kinds fall through to user_facing_hint().
    for kind in [
        ProductRejectionKind::AccessDenied,
        ProductRejectionKind::UnknownInstallation,
        ProductRejectionKind::PolicyDenied,
    ] {
        assert_eq!(
            kind.user_facing_auth_hint(),
            kind.user_facing_hint(),
            "{kind:?} auth hint must fall through to user_facing_hint()"
        );
    }
}

// BUG 3 regression: StaleGate must have a distinct hint that does NOT say
// "declined by policy" — it means the gate was already resolved.
#[test]
fn stale_gate_hint_is_distinct_from_policy_denied() {
    let stale_hint = ProductRejectionKind::StaleGate.user_facing_hint();
    let policy_hint = ProductRejectionKind::PolicyDenied.user_facing_hint();
    assert_ne!(
        stale_hint, policy_hint,
        "StaleGate hint must differ from PolicyDenied hint"
    );
    assert!(
        !stale_hint.contains("declined by policy"),
        "StaleGate hint must not say 'declined by policy', got: {stale_hint}"
    );
    assert!(
        stale_hint.contains("already approved or denied"),
        "StaleGate hint must mention 'already approved or denied', got: {stale_hint}"
    );
}

#[test]
fn policy_denied_hint_unchanged() {
    // Regression: PolicyDenied string must remain stable — existing usages
    // in other approval flows depend on it.
    assert_eq!(
        ProductRejectionKind::PolicyDenied.user_facing_hint(),
        "That request was declined by policy."
    );
}
