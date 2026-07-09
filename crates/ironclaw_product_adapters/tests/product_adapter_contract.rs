//! Architecture/boundary tests for the ProductAdapter contract.

use std::path::Path;
use std::process::Command;

use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeliveryStatus,
    EgressCredentialHandle, EgressMethod, EgressPath, EgressRequest, ExternalActorRef,
    ExternalConversationRef, ExternalEventId, FakeOutboundDeliverySink, FakeProductWorkflow,
    FakeProjectionStream, FakeProtocolHttpEgress, InboundCommandPayload, OutboundDeliverySink,
    ParsedProductInbound, ProductAdapterCapabilities, ProductAdapterError, ProductAdapterId,
    ProductAttachmentDescriptor, ProductAttachmentKind, ProductCapabilityFlag,
    ProductControlActionPayload, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget, ProductProjectionItem,
    ProductProjectionReadInput, ProductProjectionState, ProductProjectionSubject,
    ProductProjectionSubscribeInput, ProductRejection, ProductRejectionKind, ProductSurfaceKind,
    ProductTriggerReason, ProductWorkflow, ProjectionCursor, ProjectionReadRequest,
    ProjectionStream, ProjectionSubscriptionRequest, ProtocolAuthEvidence, ProtocolHttpEgress,
    ProtocolHttpEgressError, REDACTED_PLACEHOLDER, RedactedDebug, RedactedString,
    TrustedInboundContext, UserMessagePayload,
};
use ironclaw_turns::{AcceptedMessageRef, ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

const FORBIDDEN_DEPENDENCIES: &[&str] = &[
    "ironclaw_dispatcher",
    "ironclaw_capabilities",
    "ironclaw_host_runtime",
    "ironclaw_network",
    "ironclaw_secrets",
    "ironclaw_filesystem",
    "ironclaw_wasm",
    "ironclaw_processes",
    "ironclaw_mcp",
    "ironclaw_scripts",
    "ironclaw_runtime_policy",
    "ironclaw_authorization",
    "ironclaw_run_state",
    "ironclaw_approvals",
    "ironclaw_resources",
    "ironclaw_trust",
    "ironclaw_extensions",
    "ironclaw_safety",
    "ironclaw_skills",
    "ironclaw_engine",
    "ironclaw_gateway",
    "ironclaw_tui",
    "ironclaw_memory",
    "ironclaw_memory_native",
    "ironclaw_events",
    "ironclaw_reborn_event_store",
    "ironclaw_architecture",
];

#[test]
fn cargo_manifest_does_not_pull_in_forbidden_lower_layers() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
            manifest_path.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("cargo metadata");
    assert!(output.status.success(), "cargo metadata failed: {output:?}");
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("metadata json");
    let packages = metadata["packages"].as_array().expect("packages");
    let package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_product_adapters")
        .expect("product adapter package");
    let deps = package["dependencies"].as_array().expect("deps");
    for dep in deps {
        let name = dep["name"].as_str().expect("dep name");
        let package_name = dep
            .get("package")
            .and_then(|value| value.as_str())
            .unwrap_or(name);
        assert!(
            !FORBIDDEN_DEPENDENCIES.contains(&name)
                && !FORBIDDEN_DEPENDENCIES.contains(&package_name),
            "ironclaw_product_adapters must not depend on forbidden lower layer {name}/{package_name}"
        );
    }
}

#[test]
fn source_does_not_import_runner_transition_apis() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    for entry in walk_rs_files(&src_root) {
        let body = std::fs::read_to_string(&entry).expect("read source file");
        let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains("ironclaw_turns::runner")
            || compact.contains("ironclaw_turns::{runner")
            || (compact.contains("ironclaw_turns::{") && compact.contains("runner::"))
        {
            violations.push(entry.display().to_string());
        }
    }
    assert!(
        violations.is_empty(),
        "files import runner APIs: {violations:?}"
    );
}

fn walk_rs_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out
}

fn adapter_id() -> ProductAdapterId {
    ProductAdapterId::new("telegram_v2").expect("valid")
}

fn installation_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install_alpha").expect("valid")
}

fn auth_context() -> TrustedInboundContext {
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
    .expect("verified")
}

fn sample_parsed(event_id: &str) -> ParsedProductInbound {
    ParsedProductInbound::new(
        ExternalEventId::new(event_id).expect("valid"),
        ExternalActorRef::new("telegram_user", "777", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-100"))
            .expect("valid"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("valid"),
        ),
    )
    .expect("parsed")
}

fn sample_envelope(event_id: &str) -> ProductInboundEnvelope {
    ProductInboundEnvelope::from_trusted_parse(auth_context(), sample_parsed(event_id))
        .expect("envelope")
}

fn sample_target() -> ProductOutboundTarget {
    ProductOutboundTarget::new(
        ReplyTargetBindingRef::new("reply:fake-1").expect("valid"),
        ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-100"))
            .expect("valid"),
        Some(ExternalActorRef::new("telegram_user", "777", Option::<String>::None).expect("valid")),
    )
}

#[test]
fn inbound_envelope_debug_does_not_leak_secret_in_redacted_field() {
    let err = ProductAdapterError::Internal {
        detail: RedactedString::new("bot12345:AAEFGH-private-token"),
    };
    assert!(err.debug_does_not_contain("AAEFGH-private-token"));
    let rendered = err.to_string();
    assert!(rendered.contains(REDACTED_PLACEHOLDER) || rendered.contains("redacted"));
}

#[test]
fn attachment_descriptor_serialization_excludes_byte_fields() {
    let descriptor = ProductAttachmentDescriptor::new(
        "file_42",
        "image/jpeg",
        Some("photo.jpg".into()),
        Some(2048),
        ProductAttachmentKind::Image,
    )
    .expect("valid");
    let value = serde_json::to_value(&descriptor).expect("serialize");
    let object = value.as_object().expect("object");
    for forbidden in ["data", "bytes", "source_url", "local_path", "file_path"] {
        assert!(
            !object.contains_key(forbidden),
            "attachment leaked field: {forbidden}"
        );
    }
}

#[test]
fn verified_auth_evidence_only_constructible_for_tests_via_test_support() {
    let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "alice");
    assert!(evidence.is_verified());
    assert_eq!(evidence.claim().expect("claim").subject(), "alice");
    assert!(evidence.claim().expect("claim").tenant_id().is_none());
    let json = serde_json::to_string(&evidence).expect("serialize");
    assert!(serde_json::from_str::<ProtocolAuthEvidence>(&json).is_err());
}

#[test]
fn verified_auth_evidence_can_carry_tenant_scope_for_host_minted_claims() {
    let tenant_id = ironclaw_host_api::TenantId::new("tenant-a").expect("tenant");
    let evidence = ProtocolAuthEvidence::test_verified_for_tenant(
        AuthRequirement::BearerToken,
        "alice",
        tenant_id.clone(),
    );

    let claim = evidence.claim().expect("claim");
    assert_eq!(claim.subject(), "alice");
    assert_eq!(claim.tenant_id(), Some(&tenant_id));
    let json = serde_json::to_value(&evidence).expect("serialize");
    assert_eq!(json["claim"]["tenant_id"], "tenant-a");
    assert!(serde_json::from_value::<ProtocolAuthEvidence>(json).is_err());
}

#[tokio::test]
async fn workflow_default_behavior_accepts_inbound_and_records_envelope() {
    let workflow = FakeProductWorkflow::new();
    let ack = workflow
        .accept_inbound(sample_envelope("update:1"))
        .await
        .expect("accept");
    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(workflow.accepted_count(), 1);
}

#[tokio::test]
async fn workflow_dedupes_duplicate_external_event_id_per_source_binding() {
    let workflow = FakeProductWorkflow::new();
    let first = sample_envelope("update:42");
    let second = sample_envelope("update:42");
    let first_ack = workflow.accept_inbound(first).await.expect("first");
    assert!(matches!(first_ack, ProductInboundAck::Accepted { .. }));
    let second_ack = workflow.accept_inbound(second).await.expect("duplicate");
    assert!(matches!(second_ack, ProductInboundAck::Duplicate { .. }));
    assert_eq!(workflow.accepted_count(), 1);
}

#[tokio::test]
async fn workflow_returns_programmed_outcomes() {
    let workflow = FakeProductWorkflow::new();
    workflow.program_outcome(
        ExternalEventId::new("update:busy").expect("valid"),
        ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:busy").expect("valid"),
            active_run_id: TurnRunId::new(),
        },
    );
    workflow.program_outcome(
        ExternalEventId::new("update:reject").expect("valid"),
        ProductInboundAck::Rejected(ProductRejection::retryable(
            ProductRejectionKind::PolicyDenied,
            "rate limit",
        )),
    );

    let busy_ack = workflow
        .accept_inbound(sample_envelope("update:busy"))
        .await
        .expect("busy");
    assert!(matches!(busy_ack, ProductInboundAck::DeferredBusy { .. }));

    let reject_ack = workflow
        .accept_inbound(sample_envelope("update:reject"))
        .await
        .expect("reject");
    assert!(matches!(reject_ack, ProductInboundAck::Rejected(_)));
}

#[test]
fn control_action_cancel_run_validates_run_id() {
    assert!(ProductControlActionPayload::cancel_run("").is_err());
    assert!(ProductControlActionPayload::cancel_run("not-a-run-id").is_err());
    assert!(ProductControlActionPayload::cancel_run(&TurnRunId::new().to_string()).is_ok());
}

#[tokio::test]
async fn workflow_fake_records_submit_read_and_subscribe_doors_separately() {
    let workflow = FakeProductWorkflow::new();
    let actor = TurnActor::new(ironclaw_host_api::UserId::new("user:alice").expect("valid"));
    let scope = TurnScope::new(
        ironclaw_host_api::TenantId::new("tenant:alpha").expect("valid"),
        None,
        None,
        ironclaw_host_api::ThreadId::new("thread:alpha").expect("valid"),
    );
    let read_cursor = ProjectionCursor::new("cursor:read").expect("valid");
    let subscribe_cursor = ProjectionCursor::new("cursor:subscribe").expect("valid");
    let read_request = ProjectionReadRequest {
        actor: actor.clone(),
        scope: scope.clone(),
        after_cursor: Some(read_cursor.clone()),
        limit: Some(10),
    };
    let subscribe_request = ProjectionSubscriptionRequest {
        actor: actor.clone(),
        scope: scope.clone(),
        after_cursor: Some(subscribe_cursor.clone()),
    };
    workflow.program_projection_read_resolution(read_request.clone());
    workflow.program_projection_resolution(subscribe_request.clone());

    let read_input = ProductProjectionReadInput::new(
        ProductProjectionSubject::canonical(actor.clone(), scope.clone()),
        None,
        Some(read_cursor),
        Some(10),
    );
    let subscribe_input = ProductProjectionSubscribeInput::new(
        ProductProjectionSubject::canonical(actor, scope),
        None,
        Some(subscribe_cursor),
    );

    let read = workflow
        .read_projection(read_input.clone())
        .await
        .expect("read");
    let subscription = workflow
        .subscribe_projection(subscribe_input.clone())
        .await
        .expect("subscribe");

    assert_eq!(read, read_request);
    assert_eq!(subscription, subscribe_request);
    assert_eq!(workflow.read_inputs(), vec![read_input]);
    assert_eq!(workflow.subscribe_inputs(), vec![subscribe_input]);
    assert_eq!(workflow.accepted_count(), 0);

    let submit = workflow
        .submit_inbound(sample_envelope("update:projection-doors"))
        .await
        .expect("submit");
    assert!(matches!(submit, ProductInboundAck::Accepted { .. }));
    assert_eq!(workflow.accepted_count(), 1);
    assert_eq!(workflow.read_inputs().len(), 1);
    assert_eq!(workflow.subscribe_inputs().len(), 1);
}

#[tokio::test]
async fn workflow_propagates_transient_failure() {
    let workflow = FakeProductWorkflow::new();
    workflow.force_failure(ProductAdapterError::WorkflowTransient {
        reason: RedactedString::new("store unavailable"),
    });
    let err = workflow
        .accept_inbound(sample_envelope("update:1"))
        .await
        .expect_err("transient failure");
    assert!(err.is_retryable());
}

#[tokio::test]
async fn egress_to_undeclared_host_fails_closed() {
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    let request = EgressRequest::new(
        DeclaredEgressHost::new("evil.example.com").expect("valid"),
        EgressMethod::post(),
        EgressPath::new("/bot/sendMessage").expect("valid"),
    );
    let err = egress.send(request).await.expect_err("undeclared host");
    assert!(matches!(
        err,
        ProtocolHttpEgressError::UndeclaredHost { .. }
    ));
}

#[tokio::test]
async fn egress_with_unknown_credential_handle_fails_closed() {
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    let request = EgressRequest::new(
        DeclaredEgressHost::new("api.telegram.org").expect("valid"),
        EgressMethod::post(),
        EgressPath::new("/bot/sendMessage").expect("valid"),
    )
    .with_credential_handle(Some(
        EgressCredentialHandle::new("ghost_token").expect("valid"),
    ));
    let err = egress.send(request).await.expect_err("unknown handle");
    assert!(matches!(
        err,
        ProtocolHttpEgressError::UnknownCredentialHandle { .. }
    ));
}

#[tokio::test]
async fn egress_with_declared_host_and_handle_succeeds() {
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let request = EgressRequest::new(
        DeclaredEgressHost::new("api.telegram.org").expect("valid"),
        EgressMethod::post(),
        EgressPath::new("/bot/sendMessage").expect("valid"),
    )
    .with_body(br#"{"text":"hi"}"#.to_vec())
    .with_credential_handle(Some(
        EgressCredentialHandle::new("telegram_bot_token").expect("valid"),
    ));
    let response = egress.send(request).await.expect("ok");
    assert_eq!(response.status(), 200);
    assert_eq!(egress.calls()[0].method, "POST");
}

#[test]
fn external_channel_default_capabilities_omit_progress_and_gate_push() {
    let caps = ProductAdapterCapabilities::external_channel_default();
    assert!(caps.contains(ProductCapabilityFlag::ExternalFinalReplyPush));
    assert!(caps.contains(ProductCapabilityFlag::DeliveryStatusReporting));
    assert!(!caps.contains(ProductCapabilityFlag::ExternalProgressPush));
    assert!(!caps.contains(ProductCapabilityFlag::ExternalGatePush));
}

#[tokio::test]
async fn projection_stream_drains_queued_envelopes() {
    let stream = FakeProjectionStream::new();
    let envelope = ProductOutboundEnvelope::new(
        adapter_id(),
        installation_id(),
        sample_target(),
        ProjectionCursor::new("cursor:1").expect("valid"),
        ProductOutboundPayload::FinalReply(ironclaw_product_adapters::FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hi".into(),
            generated_at: chrono::Utc::now(),
        }),
    );
    stream.push(envelope.clone());
    let drained = stream
        .drain(sample_subscription(None))
        .await
        .expect("drain");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].installation_id, envelope.installation_id);
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

#[tokio::test]
async fn delivery_sink_dedupes_attempt_id() {
    let sink = FakeOutboundDeliverySink::new();
    let target = ReplyTargetBindingRef::new("reply:fake-1").expect("valid");
    let attempt_id = uuid::Uuid::new_v4();
    sink.record(DeliveryStatus::Delivered {
        attempt_id,
        target: target.clone(),
        run_id: None,
    })
    .await;
    sink.record(DeliveryStatus::FailedRetryable {
        attempt_id,
        target,
        run_id: None,
        reason: RedactedString::new("telegram 502"),
    })
    .await;
    assert_eq!(sink.statuses().len(), 1);
}

#[test]
fn product_surface_kinds_round_trip() {
    for kind in [
        ProductSurfaceKind::ExternalChannel,
        ProductSurfaceKind::Web,
        ProductSurfaceKind::Cli,
        ProductSurfaceKind::SynchronousApi,
    ] {
        let json = serde_json::to_string(&kind).expect("serialize");
        let parsed: ProductSurfaceKind = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(kind, parsed);
    }
}

#[test]
fn auth_requirement_telegram_shape() {
    let requirement = AuthRequirement::SharedSecretHeader {
        header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
    };
    let json = serde_json::to_string(&requirement).expect("serialize");
    assert!(json.contains("shared_secret_header"));
}

#[test]
fn projection_state_is_renderable() {
    let state = ProductProjectionState::new(
        "thread-1",
        vec![ProductProjectionItem::Text {
            id: "message-1".into(),
            run_id: None,
            body: "hello".into(),
        }],
    )
    .expect("state");
    assert_eq!(state.items.len(), 1);
}

#[test]
fn command_payload_bounds_are_public_contract() {
    assert!(InboundCommandPayload::new("help", "", ProductTriggerReason::BotCommand).is_ok());
    assert!(InboundCommandPayload::new("help", "short", ProductTriggerReason::BotCommand).is_ok());
    assert!(
        InboundCommandPayload::new("h".repeat(257), "", ProductTriggerReason::BotCommand).is_err()
    );
    assert!(InboundCommandPayload::new("bad name", "", ProductTriggerReason::BotCommand).is_err());
    assert!(InboundCommandPayload::new("bad/name", "", ProductTriggerReason::BotCommand).is_err());
}

#[test]
fn payload_alias_matches_reexport() {
    fn _coerce(
        p: ironclaw_product_adapters::inbound::ProductInboundPayload,
    ) -> ProductInboundPayload {
        p
    }
}
