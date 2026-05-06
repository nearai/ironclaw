//! Architecture/boundary tests for the ProductAdapter contract.
//!
//! These tests fence the contract boundary so future PRs cannot quietly
//! reach into kernel/runtime internals or invent shortcuts that bypass the
//! workflow facade.

use std::path::Path;

use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeliveryStatus,
    EgressCredentialHandle, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    FakeOutboundDeliverySink, FakeProductWorkflow, FakeProtocolHttpEgress, OutboundDeliverySink,
    ProductAdapterCapabilities, ProductAdapterError, ProductAdapterId, ProductAttachmentDescriptor,
    ProductAttachmentKind, ProductCapabilityFlag, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductOutboundEnvelope, ProductOutboundPayload, ProductRejection,
    ProductRejectionKind, ProductSurfaceKind, ProductTriggerReason, ProductWorkflow,
    ProjectionCursor, ProjectionStream, ProjectionSubscriptionRequest, ProtocolAuthEvidence,
    ProtocolHttpEgress, ProtocolHttpEgressError, REDACTED_PLACEHOLDER, RedactedDebug,
    UserMessagePayload, auth::mark_shared_secret_header_verified, fakes::FakeProjectionStream,
    inbound::ProductInboundPayload as PayloadAlias,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};

// ---------------------------------------------------------------------------
// Cargo manifest boundary check
// ---------------------------------------------------------------------------

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
    "ironclaw_events",
    "ironclaw_reborn_event_store",
    "ironclaw_architecture",
];

#[test]
fn cargo_manifest_does_not_pull_in_forbidden_lower_layers() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).expect("read Cargo.toml");
    for forbidden in FORBIDDEN_DEPENDENCIES {
        let needle = format!("{forbidden} =");
        let needle_bracket = format!("{forbidden}.");
        assert!(
            !manifest.contains(&needle) && !manifest.contains(&needle_bracket),
            "ironclaw_product_adapters must not depend on {forbidden}; \
             ProductAdapter contracts stay above the kernel/runtime layer"
        );
    }
}

#[test]
fn source_does_not_import_runner_transition_apis() {
    // ironclaw_turns::runner contains trusted transition APIs reserved for
    // workers. Adapter contracts must use only the adapter-safe surface
    // (TurnCoordinator types via the workflow facade).
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    for entry in walk_rs_files(&src_root) {
        let body = std::fs::read_to_string(&entry).expect("read source file");
        if body.contains("ironclaw_turns::runner") {
            violations.push(entry.display().to_string());
        }
    }
    assert!(
        violations.is_empty(),
        "files import ironclaw_turns::runner: {violations:?}"
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

// ---------------------------------------------------------------------------
// DTO redaction checks
// ---------------------------------------------------------------------------

#[test]
fn inbound_envelope_debug_does_not_leak_secret_in_redacted_field() {
    use ironclaw_product_adapters::RedactedString;

    // Manufacture an Internal error wrapping a sensitive string and assert
    // its Debug never reveals the inner value.
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
            "attachment descriptor leaked field: {forbidden}"
        );
    }
}

// ---------------------------------------------------------------------------
// Auth evidence sealing
// ---------------------------------------------------------------------------

#[test]
fn verified_auth_evidence_only_constructible_via_host_helpers() {
    // Adapters cannot mint a `Verified` evidence directly. The variant
    // requires a `HostAuthSeal` whose constructor is `pub(crate)`. The only
    // public way to obtain a `Verified` evidence is via the `mark_*_verified`
    // helpers on `crate::auth` — exercise each to pin the contract.
    let signature = ironclaw_product_adapters::auth::mark_request_signature_verified(
        "X-Slack-Signature",
        Some("X-Slack-Request-Timestamp".into()),
        "T01ABCDEF",
    );
    let shared = mark_shared_secret_header_verified(
        "X-Telegram-Bot-Api-Secret-Token",
        "telegram_install_alpha",
    );
    let session =
        ironclaw_product_adapters::auth::mark_session_verified("ironclaw_session", "alice");
    let bearer = ironclaw_product_adapters::auth::mark_bearer_token_verified("alice");

    for evidence in [signature, shared, session, bearer] {
        assert!(matches!(evidence, ProtocolAuthEvidence::Verified { .. }));
        assert!(evidence.is_verified());
        assert!(evidence.claim().is_some());
    }
}

// ---------------------------------------------------------------------------
// FakeProductWorkflow contract behavior
// ---------------------------------------------------------------------------

fn sample_envelope(event_id: &str) -> ProductInboundEnvelope {
    ProductInboundEnvelope {
        adapter_id: ProductAdapterId::new("telegram_v2").expect("valid"),
        installation_id: AdapterInstallationId::new("install_alpha").expect("valid"),
        external_event_id: ExternalEventId::new(event_id).expect("valid"),
        external_actor_ref: ExternalActorRef::new("telegram_user", "777", None).expect("valid"),
        external_conversation_ref: ExternalConversationRef::new(
            None,
            "12345",
            Some("topic-7"),
            Some("msg-100"),
        )
        .expect("valid"),
        auth_evidence: mark_shared_secret_header_verified(
            "X-Telegram-Bot-Api-Secret-Token",
            "telegram_install_alpha",
        ),
        received_at: chrono::Utc::now(),
        payload: ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("valid"),
        ),
    }
}

#[tokio::test]
async fn workflow_default_behavior_accepts_inbound_and_records_envelope() {
    let workflow = FakeProductWorkflow::new();
    let envelope = sample_envelope("update:1");
    let ack = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("accept");
    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(workflow.accepted_count(), 1);
}

#[tokio::test]
async fn workflow_dedupes_duplicate_external_event_id() {
    let workflow = FakeProductWorkflow::new();
    let envelope = sample_envelope("update:42");
    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first call");
    assert!(matches!(first, ProductInboundAck::Accepted { .. }));
    let second = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("dup");
    let ProductInboundAck::Duplicate { prior } = second else {
        panic!("expected Duplicate, got {second:?}");
    };
    assert!(matches!(*prior, ProductInboundAck::Accepted { .. }));
    // Duplicate path must NOT count as a fresh accepted envelope.
    assert_eq!(workflow.accepted_count(), 1);
}

#[tokio::test]
async fn workflow_returns_programmed_outcomes() {
    let workflow = FakeProductWorkflow::new();

    workflow.program_outcome(
        ExternalEventId::new("update:busy").expect("valid"),
        ProductInboundAck::DeferredBusy {
            accepted_message_ref: "msg:busy".into(),
            active_run_id: TurnRunId::new(),
        },
    );
    workflow.program_outcome(
        ExternalEventId::new("update:reject").expect("valid"),
        ProductInboundAck::Rejected(ProductRejection {
            kind: ProductRejectionKind::PolicyDenied,
            reason: "rate limit".into(),
        }),
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

#[tokio::test]
async fn workflow_propagates_transient_failure() {
    let workflow = FakeProductWorkflow::new();
    workflow.force_failure(ProductAdapterError::WorkflowTransient {
        reason: "store unavailable".into(),
    });
    let err = workflow
        .accept_inbound(sample_envelope("update:1"))
        .await
        .expect_err("transient failure");
    assert!(err.is_retryable());
}

// ---------------------------------------------------------------------------
// Egress contract behavior
// ---------------------------------------------------------------------------

#[tokio::test]
async fn egress_to_undeclared_host_fails_closed() {
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    let request = ironclaw_product_adapters::EgressRequest {
        host: DeclaredEgressHost::new("evil.example.com").expect("valid"),
        method: "POST".into(),
        path: "/bot/sendMessage".into(),
        headers: Default::default(),
        body: vec![],
        credential_handle: None,
    };
    let err = egress.send(request).await.expect_err("undeclared host");
    assert!(matches!(
        err,
        ProtocolHttpEgressError::UndeclaredHost { .. }
    ));
}

#[tokio::test]
async fn egress_with_unknown_credential_handle_fails_closed() {
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    let request = ironclaw_product_adapters::EgressRequest {
        host: DeclaredEgressHost::new("api.telegram.org").expect("valid"),
        method: "POST".into(),
        path: "/bot/sendMessage".into(),
        headers: Default::default(),
        body: vec![],
        credential_handle: Some(EgressCredentialHandle::new("ghost_token").expect("valid")),
    };
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
    let request = ironclaw_product_adapters::EgressRequest {
        host: DeclaredEgressHost::new("api.telegram.org").expect("valid"),
        method: "POST".into(),
        path: "/bot/sendMessage".into(),
        headers: Default::default(),
        body: br#"{"text":"hi"}"#.to_vec(),
        credential_handle: Some(EgressCredentialHandle::new("telegram_bot_token").expect("valid")),
    };
    let response = egress.send(request).await.expect("ok");
    assert_eq!(response.status, 200);
    let calls = egress.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].host, "api.telegram.org");
    assert_eq!(calls[0].method, "POST");
}

// ---------------------------------------------------------------------------
// Capability gating contract
// ---------------------------------------------------------------------------

#[test]
fn external_channel_default_capabilities_omit_progress_and_gate_push() {
    let caps = ProductAdapterCapabilities::external_channel_default();
    assert!(caps.contains(ProductCapabilityFlag::ExternalFinalReplyPush));
    assert!(caps.contains(ProductCapabilityFlag::DeliveryStatusReporting));
    assert!(!caps.contains(ProductCapabilityFlag::ExternalProgressPush));
    assert!(!caps.contains(ProductCapabilityFlag::ExternalGatePush));
}

// ---------------------------------------------------------------------------
// Projection stream contract behavior
// ---------------------------------------------------------------------------

#[tokio::test]
async fn projection_stream_drains_queued_envelopes() {
    let stream = FakeProjectionStream::new();
    let envelope = ProductOutboundEnvelope {
        adapter_id: ProductAdapterId::new("telegram_v2").expect("valid"),
        installation_id: AdapterInstallationId::new("install_alpha").expect("valid"),
        target: ReplyTargetBindingRef::new("reply:fake-1").expect("valid"),
        projection_cursor: Some(ProjectionCursor::new("cursor:1")),
        payload: ProductOutboundPayload::FinalReply(ironclaw_product_adapters::FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hi".into(),
            generated_at: chrono::Utc::now(),
        }),
        delivery_attempt_id: uuid::Uuid::new_v4(),
    };
    stream.push(envelope.clone());
    let scope = ironclaw_turns::TurnScope::new(
        ironclaw_host_api::TenantId::new("tenant-a").expect("valid"),
        None,
        None,
        ironclaw_host_api::ThreadId::new("thread-1").expect("valid"),
    );
    let actor =
        ironclaw_turns::TurnActor::new(ironclaw_host_api::UserId::new("alice").expect("valid"));
    let drained = stream
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .expect("drain");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].installation_id, envelope.installation_id);
}

// ---------------------------------------------------------------------------
// Delivery sink contract behavior
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delivery_sink_records_success_and_failure_separately() {
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
        attempt_id: uuid::Uuid::new_v4(),
        target: target.clone(),
        run_id: None,
        reason: "telegram 502".into(),
    })
    .await;
    let statuses = sink.statuses();
    assert_eq!(statuses.len(), 2);
    assert!(matches!(statuses[0], DeliveryStatus::Delivered { .. }));
    assert!(matches!(
        statuses[1],
        DeliveryStatus::FailedRetryable { .. }
    ));
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
    // Telegram uses a shared-secret header.
    let requirement = AuthRequirement::SharedSecretHeader {
        header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
    };
    let json = serde_json::to_string(&requirement).expect("serialize");
    assert!(json.contains("shared_secret_header"));
}

#[test]
fn payload_alias_matches_reexport() {
    // Smoke check: the re-export and the alias resolve to the same type.
    fn _coerce(p: PayloadAlias) -> ProductInboundPayload {
        p
    }
}
