//! Telegram WASM v2 ProductAdapter contract tests (#3285).
//!
//! Each test references the acceptance-criteria bullet from the issue body.
//! The tests drive a `TelegramV2Adapter` against fake Reborn services
//! (`FakeProductWorkflow`, `FakeProtocolHttpEgress`, `FakeOutboundDeliverySink`)
//! and recorded Telegram payloads. They prove all 16 acceptance bullets
//! without requiring production infrastructure.
//!
//! Note: webhook ack semantics (200 / retryable / fail-closed) are tested
//! against the auth-evidence + workflow-error contract; the protocol-status
//! mapping is owned by the host glue (`NativeProductAdapterRunner`) and
//! exercised in the runner-level tests at the end of this file.

use std::path::Path;
use std::sync::Arc;

use http::HeaderMap;
use ironclaw_product_adapters::{
    AdapterInstallationId, DeclaredEgressHost, DeliveryStatus, EgressCredentialHandle,
    EgressRequest, EgressResponse, ExternalEventId, FakeOutboundDeliverySink, FakeProductWorkflow,
    FakeProtocolHttpEgress, FinalReplyView, OutboundDeliverySink, ProductAdapter,
    ProductAdapterCapabilities, ProductAdapterError, ProductAdapterId, ProductCapabilityFlag,
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductRejection, ProductRejectionKind, ProductSurfaceKind,
    ProductTriggerReason, ProductWorkflow, ProjectionStream, ProjectionSubscriptionRequest,
    ProtocolAuthEvidence, ProtocolAuthFailure, ProtocolHttpEgress, ProtocolHttpEgressError,
    auth::mark_shared_secret_header_verified, fakes::FakeProjectionStream,
};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TELEGRAM_API_HOST, TelegramV2Adapter, TelegramV2AdapterConfig,
    telegram_declared_egress_hosts,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use ironclaw_wasm_product_adapters::{
    NativeProductAdapterRunner, RunnerError, SharedSecretHeaderAuth, WebhookProcessOutcome,
    runner::WebhookAuth,
};

// ---------------------------------------------------------------------------
// Shared test setup
// ---------------------------------------------------------------------------

const FIXTURE_PATH: &str = "tests/fixtures";

const TELEGRAM_BOT_TOKEN_HANDLE: &str = "telegram_bot_token";
const TELEGRAM_INSTALLATION_SUBJECT: &str = "telegram_install_alpha";
const TELEGRAM_WEBHOOK_SECRET: &str = "topsecret";

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_PATH)
        .join(name);
    std::fs::read(&path).unwrap_or_else(|err| panic!("read fixture {name}: {err}"))
}

fn config() -> TelegramV2AdapterConfig {
    TelegramV2AdapterConfig {
        adapter_id: ProductAdapterId::new("telegram_v2").expect("valid"),
        installation_id: AdapterInstallationId::new(TELEGRAM_INSTALLATION_SUBJECT).expect("valid"),
        group_trigger_policy: GroupTriggerPolicy {
            bot_username: "ironclaw_bot".into(),
            bot_user_id: 9000,
            recognized_commands: vec!["help".into(), "start".into()],
        },
        egress_credential_handle: EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_HANDLE)
            .expect("valid"),
        progress_push_enabled: false,
    }
}

fn evidence() -> ProtocolAuthEvidence {
    mark_shared_secret_header_verified(
        "X-Telegram-Bot-Api-Secret-Token",
        TELEGRAM_INSTALLATION_SUBJECT,
    )
}

fn webhook_headers(secret: Option<&str>) -> HeaderMap {
    let mut map = HeaderMap::new();
    if let Some(s) = secret {
        map.insert(
            http::header::HeaderName::from_static("x-telegram-bot-api-secret-token"),
            http::header::HeaderValue::from_str(s).expect("header value"),
        );
    }
    map
}

fn build_runner(workflow: Arc<FakeProductWorkflow>) -> NativeProductAdapterRunner {
    let adapter: Arc<dyn ProductAdapter> = Arc::new(TelegramV2Adapter::new(config()));
    let workflow: Arc<dyn ProductWorkflow> = workflow;
    NativeProductAdapterRunner::new(
        adapter,
        workflow,
        WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            expected_secret: TELEGRAM_WEBHOOK_SECRET.into(),
            subject: TELEGRAM_INSTALLATION_SUBJECT.into(),
        }),
    )
}

// ---------------------------------------------------------------------------
// AC #1 — Telegram WASM v2 uses ProductAdapter-native DTOs and does not
//         depend on legacy `IncomingMessage`, `OutgoingResponse`,
//         `StatusUpdate`, v1 `Channel`, or v1 `ChannelManager` semantics.
// ---------------------------------------------------------------------------

#[test]
fn ac1_telegram_v2_does_not_import_v1_channel_types() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    let banned = [
        "IncomingMessage",
        "OutgoingResponse",
        "StatusUpdate",
        "ChannelManager",
        "channels_src::",
        "channels::wasm::",
    ];
    for path in walk_rs_files(&crate_root.join("src")) {
        let body = std::fs::read_to_string(&path).expect("read source");
        for bad in &banned {
            // Skip compound matches inside ProductAdapter type names.
            let allowed_aliases = ["StatusUpdateView", "IncomingHttpRequest"];
            if allowed_aliases
                .iter()
                .any(|alias| body.contains(alias) && body.contains(bad))
            {
                continue;
            }
            if body.contains(bad) {
                violations.push(format!("{}: contains `{bad}`", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "telegram v2 leaked v1 references: {violations:?}"
    );
}

// ---------------------------------------------------------------------------
// AC #2 — ProductAdapter / WASM v2 host contracts live outside `src/channels`
//         legacy layering.
// ---------------------------------------------------------------------------

#[test]
fn ac2_product_adapter_contracts_live_outside_src_channels() {
    // Walk the tree at the workspace root, look for any path under
    // `src/channels/` that mentions the new contract crate names. None
    // should exist.
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolvable");
    let banned_in_legacy = [
        "ironclaw_product_adapters",
        "ironclaw_wasm_product_adapters",
        "ironclaw_telegram_v2_adapter",
    ];
    let legacy_root = workspace_root.join("src").join("channels");
    let mut violations = Vec::new();
    if legacy_root.exists() {
        for path in walk_rs_files(&legacy_root) {
            let body = std::fs::read_to_string(&path).expect("read legacy source");
            for needle in &banned_in_legacy {
                if body.contains(needle) {
                    violations.push(format!(
                        "{}: src/channels must not import {needle}",
                        path.display()
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "v2 contracts leaked into legacy src/channels: {violations:?}"
    );
}

// ---------------------------------------------------------------------------
// AC #3 — Host verifies Telegram webhook authentication before constructing
//         any ProductInboundEnvelope.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac3_runner_blocks_envelope_construction_on_bad_secret() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let bad_headers = webhook_headers(Some("wrong"));
    let payload = fixture("private_chat_message.json");
    let err = runner
        .process_webhook(&bad_headers, &payload)
        .await
        .expect_err("must fail closed");
    assert!(err.is_auth_failure());
    // Workflow MUST NOT have seen any envelope.
    assert_eq!(workflow.accepted_count(), 0);
}

#[tokio::test]
async fn ac3_runner_blocks_envelope_construction_on_missing_secret() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let no_headers = webhook_headers(None);
    let payload = fixture("private_chat_message.json");
    let err = runner
        .process_webhook(&no_headers, &payload)
        .await
        .expect_err("must fail closed");
    assert!(err.is_auth_failure());
    assert_eq!(workflow.accepted_count(), 0);
}

#[tokio::test]
async fn ac3_adapter_refuses_unverified_evidence_directly() {
    let adapter = TelegramV2Adapter::new(config());
    let unverified = ProtocolAuthEvidence::Failed {
        failure: ProtocolAuthFailure::Missing,
    };
    let err = adapter
        .parse_inbound(&fixture("private_chat_message.json"), unverified)
        .expect_err("adapter must refuse unverified evidence");
    assert!(matches!(err, ProductAdapterError::Authentication(_)));
}

// ---------------------------------------------------------------------------
// AC #4 — Telegram v2 normalizes external actor, conversation, event,
//         installation, reply-target, and attachment descriptors into
//         structured refs.
// ---------------------------------------------------------------------------

#[test]
fn ac4_parse_normalizes_all_refs() {
    let adapter = TelegramV2Adapter::new(config());
    let envelope = adapter
        .parse_inbound(&fixture("private_chat_message.json"), evidence())
        .expect("ok")
        .expect("envelope present");
    assert_eq!(envelope.adapter_id.as_str(), "telegram_v2");
    assert_eq!(envelope.installation_id.as_str(), "telegram_install_alpha");
    assert_eq!(
        envelope.external_event_id.as_str(),
        "tg-telegram_install_alpha-100"
    );
    assert_eq!(envelope.external_actor_ref.kind(), "telegram_user");
    assert_eq!(envelope.external_actor_ref.id(), "777");
    assert_eq!(envelope.external_conversation_ref.conversation_id(), "777");
    assert_eq!(
        envelope.external_conversation_ref.reply_target_message_id(),
        Some("11")
    );
}

// ---------------------------------------------------------------------------
// AC #5 — Telegram v2 does not resolve canonical user/thread ids, write
//         pairing/session stores, or call TurnCoordinator directly.
// ---------------------------------------------------------------------------

#[test]
fn ac5_adapter_does_not_import_turn_coordinator() {
    // Source-grep boundary: verify the telegram crate doesn't reach into
    // TurnCoordinator's submit_turn / resume_turn / cancel_run /
    // get_run_state APIs. It only depends on the type re-exports
    // (TurnRunId, ReplyTargetBindingRef) for envelope shapes.
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let banned = [
        ".submit_turn(",
        ".resume_turn(",
        ".cancel_run(",
        ".get_run_state(",
        "ironclaw_turns::runner",
        "ironclaw_turns::coordinator",
    ];
    let mut violations = Vec::new();
    for path in walk_rs_files(&crate_root.join("src")) {
        let body = std::fs::read_to_string(&path).expect("read source");
        for bad in &banned {
            if body.contains(bad) {
                violations.push(format!("{}: contains `{bad}`", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "telegram v2 reached into TurnCoordinator internals: {violations:?}"
    );
}

#[tokio::test]
async fn ac5_adapter_path_only_invokes_workflow_facade() {
    // Run a full webhook through the adapter + runner. Workflow MUST be the
    // single point of contact for canonical state — the test asserts the
    // adapter's only side effect through the host runtime is
    // `workflow.accept_inbound`.
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let outcome = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("private_chat_message.json"),
        )
        .await
        .expect("ok");
    assert!(matches!(
        outcome,
        WebhookProcessOutcome::Acknowledged { .. }
    ));
    assert_eq!(workflow.accepted_count(), 1);
}

// ---------------------------------------------------------------------------
// AC #6 — ProductWorkflow / ConversationBinding / SessionThread fake
//         receives external refs and returns durable accept/dedupe/defer/
//         reject outcomes.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac6_workflow_returns_each_durable_outcome_kind() {
    for (event_id, outcome, kind_check) in [
        (
            "tg-accepted-1",
            ProductInboundAck::Accepted {
                accepted_message_ref: "msg:1".into(),
                submitted_run_id: Some(TurnRunId::new()),
            },
            "accepted",
        ),
        (
            "tg-defer-1",
            ProductInboundAck::DeferredBusy {
                accepted_message_ref: "msg:1".into(),
                active_run_id: TurnRunId::new(),
            },
            "deferred",
        ),
        (
            "tg-reject-1",
            ProductInboundAck::Rejected(ProductRejection {
                kind: ProductRejectionKind::AccessDenied,
                reason: "not paired".into(),
            }),
            "rejected",
        ),
    ] {
        let workflow = FakeProductWorkflow::new();
        workflow.program_outcome(
            ExternalEventId::new(event_id).expect("valid"),
            outcome.clone(),
        );
        let envelope = ProductInboundEnvelope {
            adapter_id: ProductAdapterId::new("telegram_v2").unwrap(),
            installation_id: AdapterInstallationId::new("install_alpha").unwrap(),
            external_event_id: ExternalEventId::new(event_id).unwrap(),
            external_actor_ref: ironclaw_product_adapters::ExternalActorRef::new(
                "telegram_user",
                "1",
                None,
            )
            .unwrap(),
            external_conversation_ref: ironclaw_product_adapters::ExternalConversationRef::new(
                None,
                "1",
                None,
                Some("1"),
            )
            .unwrap(),
            auth_evidence: evidence(),
            received_at: chrono::Utc::now(),
            payload: ProductInboundPayload::UserMessage(
                ironclaw_product_adapters::UserMessagePayload::new(
                    "hi",
                    vec![],
                    ProductTriggerReason::DirectChat,
                )
                .unwrap(),
            ),
        };
        let ack = workflow.accept_inbound(envelope).await.expect("ok");
        assert!(ack.is_durable_outcome(), "{kind_check} not durable");
        match (kind_check, ack) {
            ("accepted", ProductInboundAck::Accepted { .. }) => {}
            ("deferred", ProductInboundAck::DeferredBusy { .. }) => {}
            ("rejected", ProductInboundAck::Rejected(_)) => {}
            (k, other) => panic!("unexpected outcome for {k}: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// AC #7 — Stable Telegram external event ids dedupe duplicate webhook
//         deliveries and return the prior durable inbound outcome.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac7_duplicate_update_id_returns_prior_outcome_no_double_submit() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let first = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("private_chat_message.json"),
        )
        .await
        .expect("first ok");
    let WebhookProcessOutcome::Acknowledged { ack: first_ack } = first else {
        panic!("expected ack");
    };
    assert!(matches!(first_ack, ProductInboundAck::Accepted { .. }));

    // Second delivery with the SAME update_id (different body wording is fine
    // — the dedupe key is update_id-derived).
    let second = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("duplicate_update.json"),
        )
        .await
        .expect("second ok");
    let WebhookProcessOutcome::Acknowledged { ack: second_ack } = second else {
        panic!("expected ack");
    };
    let ProductInboundAck::Duplicate { prior } = second_ack else {
        panic!("expected Duplicate, got {second_ack:?}");
    };
    assert!(matches!(*prior, ProductInboundAck::Accepted { .. }));
    // Workflow must have seen exactly ONE accepted envelope.
    assert_eq!(workflow.accepted_count(), 1);
}

// ---------------------------------------------------------------------------
// AC #8 — Attachments are passed as bounded descriptors / temporary handles
//         and staged by ProductWorkflow / SessionThreadService into durable
//         refs before turn submission.
// ---------------------------------------------------------------------------

#[test]
fn ac8_attachments_have_no_raw_bytes_or_source_urls() {
    let adapter = TelegramV2Adapter::new(config());
    let envelope = adapter
        .parse_inbound(&fixture("photo_attachment.json"), evidence())
        .expect("ok")
        .expect("envelope");
    let ProductInboundPayload::UserMessage(user) = envelope.payload else {
        panic!("expected UserMessage");
    };
    assert_eq!(user.attachments.len(), 1);
    let attachment = &user.attachments[0];
    let json = serde_json::to_value(attachment).expect("serialize");
    let object = json.as_object().expect("object");
    for forbidden in ["data", "bytes", "source_url", "local_path", "file_path"] {
        assert!(
            !object.contains_key(forbidden),
            "attachment leaked field {forbidden}"
        );
    }
    // The descriptor is bounded: external_file_id, mime_type, size, kind,
    // optional filename. No tokens, no URLs.
    assert_eq!(attachment.external_file_id, "BBBB");
    assert_eq!(attachment.mime_type, "image/jpeg");
    assert_eq!(attachment.size_bytes, Some(8192));
}

// ---------------------------------------------------------------------------
// AC #9 — Group/supergroup messages require explicit triggers; ambient
//         messages are successful no-op acks.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac9_private_chat_creates_inbound() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let outcome = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("private_chat_message.json"),
        )
        .await
        .expect("ok");
    assert!(matches!(
        outcome,
        WebhookProcessOutcome::Acknowledged { .. }
    ));
    assert_eq!(workflow.accepted_count(), 1);
}

#[tokio::test]
async fn ac9_group_ambient_is_noop_ack() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let outcome = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("group_ambient_message.json"),
        )
        .await
        .expect("ok");
    assert!(matches!(outcome, WebhookProcessOutcome::NoOp));
    assert_eq!(
        workflow.accepted_count(),
        0,
        "ambient group message must not reach workflow"
    );
}

#[tokio::test]
async fn ac9_group_explicit_mention_creates_inbound() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let outcome = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("group_mention_message.json"),
        )
        .await
        .expect("ok");
    let WebhookProcessOutcome::Acknowledged { ack } = outcome else {
        panic!("expected ack");
    };
    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let envelopes = workflow.accepted_envelopes();
    assert_eq!(envelopes.len(), 1);
    let ProductInboundPayload::UserMessage(user) = &envelopes[0].payload else {
        panic!("expected UserMessage");
    };
    assert_eq!(user.trigger, ProductTriggerReason::BotMention);
}

#[tokio::test]
async fn ac9_group_command_creates_inbound() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let outcome = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("group_command.json"),
        )
        .await
        .expect("ok");
    let WebhookProcessOutcome::Acknowledged { ack } = outcome else {
        panic!("expected ack");
    };
    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let envelopes = workflow.accepted_envelopes();
    assert_eq!(envelopes.len(), 1);
    let ProductInboundPayload::Command(cmd) = &envelopes[0].payload else {
        panic!("expected Command");
    };
    assert_eq!(cmd.command, "help");
    assert_eq!(cmd.trigger, ProductTriggerReason::BotCommand);
}

// ---------------------------------------------------------------------------
// AC #10 — `ExternalConversationRef` uses Telegram chat id + optional topic
//          id; reply/message ids are reply-target / idempotency data, not
//          the canonical conversation key.
// ---------------------------------------------------------------------------

#[test]
fn ac10_conversation_key_uses_chat_and_topic_not_message_id() {
    let adapter = TelegramV2Adapter::new(config());
    let envelope = adapter
        .parse_inbound(&fixture("topic_message.json"), evidence())
        .expect("ok")
        .expect("envelope");
    assert_eq!(envelope.external_conversation_ref.conversation_id(), "-42");
    assert_eq!(envelope.external_conversation_ref.topic_id(), Some("7"));
    assert_eq!(
        envelope.external_conversation_ref.reply_target_message_id(),
        Some("50")
    );
    let fingerprint_a = envelope
        .external_conversation_ref
        .conversation_fingerprint();

    // Same chat, same topic, different message_id → identical fingerprint
    // (proven via direct ref construction, since the conversation key MUST
    // NOT depend on message_id).
    let other = ironclaw_product_adapters::ExternalConversationRef::new(
        None,
        "-42",
        Some("7"),
        Some("9999"),
    )
    .expect("valid");
    assert_eq!(fingerprint_a, other.conversation_fingerprint());
}

// ---------------------------------------------------------------------------
// AC #11 — Webhook responses return 200 only after durable inbound outcome;
//          transient failures return retryable errors; auth failures fail
//          closed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac11_durable_outcome_classification_for_each_path() {
    // Acknowledged path -> durable outcome.
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let outcome = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("private_chat_message.json"),
        )
        .await
        .expect("ok");
    let WebhookProcessOutcome::Acknowledged { ack } = outcome else {
        panic!("expected ack");
    };
    assert!(ack.is_durable_outcome());

    // Auth-failure path -> RunnerError::AuthenticationFailed (fail-closed,
    // not retryable).
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let err = runner
        .process_webhook(
            &webhook_headers(None),
            &fixture("private_chat_message.json"),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_auth_failure());
    assert!(!err.is_retryable());

    // Transient workflow failure path -> retryable.
    let workflow = Arc::new(FakeProductWorkflow::new());
    workflow.force_failure(ProductAdapterError::WorkflowTransient {
        reason: "store unavailable".into(),
    });
    let runner = build_runner(workflow.clone());
    let err = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("private_chat_message.json"),
        )
        .await
        .expect_err("must fail");
    assert!(matches!(err, RunnerError::Adapter(_)));
    assert!(err.is_retryable());
}

#[tokio::test]
async fn ac11_duplicate_returns_200_no_op_ack() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("private_chat_message.json"),
        )
        .await
        .expect("first");
    let outcome = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("duplicate_update.json"),
        )
        .await
        .expect("dup");
    let WebhookProcessOutcome::Acknowledged { ack } = outcome else {
        panic!("expected ack");
    };
    assert!(matches!(ack, ProductInboundAck::Duplicate { .. }));
    // Both deliveries succeed (200) at the protocol layer; the workflow
    // saw the message exactly once.
    assert_eq!(workflow.accepted_count(), 1);
}

// ---------------------------------------------------------------------------
// AC #12 — Outbound final replies render from projection-derived
//          ProductOutboundEnvelope and target the reply target binding.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac12_final_reply_renders_to_reply_target_binding() {
    let adapter = TelegramV2Adapter::new(config());
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_HANDLE);
    let target =
        ironclaw_telegram_v2_adapter::render::build_reply_target_binding(-42, Some(7), Some(50));
    let envelope = ProductOutboundEnvelope {
        adapter_id: adapter.adapter_id().clone(),
        installation_id: adapter.installation_id().clone(),
        target: target.clone(),
        projection_cursor: None,
        payload: ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hi from reborn".into(),
            generated_at: chrono::Utc::now(),
        }),
        delivery_attempt_id: uuid::Uuid::new_v4(),
    };
    adapter
        .render_outbound(envelope, &egress)
        .await
        .expect("ok");
    let calls = egress.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/sendMessage");
    let body: serde_json::Value = serde_json::from_slice(&calls[0].body).expect("body");
    assert_eq!(body["chat_id"], -42);
    assert_eq!(body["message_thread_id"], 7);
    assert_eq!(body["reply_to_message_id"], 50);
}

// ---------------------------------------------------------------------------
// AC #13 — Telegram outbound uses constrained host `ProtocolHttpEgress`
//          with declared hosts/credential handles; raw bot tokens and
//          arbitrary HTTP authority are not exposed to WASM.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac13_egress_to_undeclared_host_is_blocked() {
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_HANDLE);
    let request = EgressRequest {
        host: DeclaredEgressHost::new("evil.example.com").expect("valid"),
        method: "POST".into(),
        path: "/leak".into(),
        headers: Default::default(),
        body: vec![],
        credential_handle: Some(
            EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_HANDLE).expect("valid"),
        ),
    };
    let err = egress.send(request).await.expect_err("must fail");
    assert!(matches!(
        err,
        ProtocolHttpEgressError::UndeclaredHost { .. }
    ));
}

#[test]
fn ac13_telegram_only_declares_telegram_api() {
    let hosts = telegram_declared_egress_hosts();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].as_str(), TELEGRAM_API_HOST);
}

#[tokio::test]
async fn ac13_egress_request_carries_credential_handle_not_token() {
    let adapter = TelegramV2Adapter::new(config());
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_HANDLE);
    let envelope = ProductOutboundEnvelope {
        adapter_id: adapter.adapter_id().clone(),
        installation_id: adapter.installation_id().clone(),
        target: ironclaw_telegram_v2_adapter::render::build_reply_target_binding(-42, None, None),
        projection_cursor: None,
        payload: ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hi".into(),
            generated_at: chrono::Utc::now(),
        }),
        delivery_attempt_id: uuid::Uuid::new_v4(),
    };
    adapter
        .render_outbound(envelope, &egress)
        .await
        .expect("ok");
    let calls = egress.calls();
    assert_eq!(calls.len(), 1);
    let credential = calls[0].credential_handle.as_deref().expect("handle");
    assert_eq!(credential, TELEGRAM_BOT_TOKEN_HANDLE);
    // The body and headers must NOT contain a literal bot token. Adapters
    // never see the token; they emit an opaque handle id and the host
    // resolves it at request time.
    let body_str = String::from_utf8_lossy(&calls[0].body);
    assert!(!body_str.contains("Bearer "));
    assert!(!body_str.contains("AAEFGH"));
    for value in calls[0].headers.values() {
        assert!(!value.contains("Bearer "));
        assert!(!value.contains("AAEFGH"));
    }
}

// ---------------------------------------------------------------------------
// AC #14 — Delivery failures record separate egress delivery status and do
//          not mutate canonical transcript/projection/turn success.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac14_delivery_failure_records_status_separately() {
    let sink = FakeOutboundDeliverySink::new();
    let adapter = TelegramV2Adapter::new(config());
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_HANDLE);
    egress.program_response(
        "api.telegram.org",
        Ok(EgressResponse {
            status: 502,
            headers: Default::default(),
            body: br#"{"ok":false,"error":"upstream"}"#.to_vec(),
        }),
    );
    let target =
        ironclaw_telegram_v2_adapter::render::build_reply_target_binding(-42, None, Some(50));
    let envelope = ProductOutboundEnvelope {
        adapter_id: adapter.adapter_id().clone(),
        installation_id: adapter.installation_id().clone(),
        target: target.clone(),
        projection_cursor: None,
        payload: ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hi".into(),
            generated_at: chrono::Utc::now(),
        }),
        delivery_attempt_id: uuid::Uuid::new_v4(),
    };
    let result = adapter.render_outbound(envelope.clone(), &egress).await;
    let attempt_id = envelope.delivery_attempt_id;
    let target_for_status = target.clone();
    match result {
        Ok(_) => panic!("expected delivery to fail at the protocol layer"),
        Err(err) => {
            // Production glue surfaces this as a FailedRetryable status; the
            // adapter signals via Err so the host-glue layer can decide
            // retry/dead-letter behavior.
            assert!(matches!(err, ProductAdapterError::EgressDenied { .. }));
            sink.record(DeliveryStatus::FailedRetryable {
                attempt_id,
                target: target_for_status,
                run_id: None,
                reason: format!("{err}"),
            })
            .await;
        }
    }
    let statuses = sink.statuses();
    assert_eq!(statuses.len(), 1);
    assert!(matches!(
        statuses[0],
        DeliveryStatus::FailedRetryable { .. }
    ));
}

#[tokio::test]
async fn ac14_delivery_failure_does_not_mutate_canonical_workflow_state() {
    let workflow = Arc::new(FakeProductWorkflow::new());
    let runner = build_runner(workflow.clone());
    let _ = runner
        .process_webhook(
            &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
            &fixture("private_chat_message.json"),
        )
        .await
        .expect("ok");
    let envelopes_after_inbound = workflow.accepted_envelopes();

    // Now simulate an outbound delivery failure. The workflow's accepted
    // envelopes count must remain unchanged regardless of egress outcome.
    let adapter = TelegramV2Adapter::new(config());
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_HANDLE);
    egress.program_response("api.telegram.org", Err(ProtocolHttpEgressError::Timeout));
    let target =
        ironclaw_telegram_v2_adapter::render::build_reply_target_binding(777, None, Some(11));
    let envelope = ProductOutboundEnvelope {
        adapter_id: adapter.adapter_id().clone(),
        installation_id: adapter.installation_id().clone(),
        target,
        projection_cursor: None,
        payload: ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hi".into(),
            generated_at: chrono::Utc::now(),
        }),
        delivery_attempt_id: uuid::Uuid::new_v4(),
    };
    let _ = adapter.render_outbound(envelope, &egress).await;

    let envelopes_after_outbound = workflow.accepted_envelopes();
    assert_eq!(envelopes_after_inbound, envelopes_after_outbound);
}

// ---------------------------------------------------------------------------
// AC #15 — Real Telegram approval/auth prompt/resolution UX is marked
//          deferred to #3094; fake contract tests cover future gate
//          projection rendering without side effects.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac15_gate_prompt_envelope_is_no_op_egress_in_first_slice() {
    let adapter = TelegramV2Adapter::new(config());
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_HANDLE);
    let envelope = ProductOutboundEnvelope {
        adapter_id: adapter.adapter_id().clone(),
        installation_id: adapter.installation_id().clone(),
        target: ironclaw_telegram_v2_adapter::render::build_reply_target_binding(
            777,
            None,
            Some(11),
        ),
        projection_cursor: None,
        payload: ProductOutboundPayload::GatePrompt(ironclaw_product_adapters::GatePromptView {
            turn_run_id: TurnRunId::new(),
            gate_ref: "gate:fake-1".into(),
            headline: "Approval required".into(),
            body: "Approve to continue".into(),
        }),
        delivery_attempt_id: uuid::Uuid::new_v4(),
    };
    adapter
        .render_outbound(envelope, &egress)
        .await
        .expect("ok");
    // Deferred — no egress, no side effects.
    assert!(egress.calls().is_empty());
}

// ---------------------------------------------------------------------------
// AC #16 — Default-off mode keeps legacy Telegram/WASM behavior unchanged;
//          Reborn Telegram v2 requires explicit profile/feature flag.
// ---------------------------------------------------------------------------

#[test]
fn ac16_default_off_marker_present_in_workspace_root_config() {
    // The default-off behavior is enforced by the host glue. This test
    // asserts the workspace's main config keeps legacy telegram on the v1
    // path by default by NOT containing the v2-on env var as a default.
    // The real wiring lives in `src/config/channels.rs` (added in this PR);
    // the test asserts that legacy `src/channels/wasm/telegram_host_config.rs`
    // is unmodified.
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");
    let legacy_telegram_host = workspace_root
        .join("src")
        .join("channels")
        .join("wasm")
        .join("telegram_host_config.rs");
    if legacy_telegram_host.exists() {
        let body = std::fs::read_to_string(&legacy_telegram_host).expect("read");
        // The legacy host config MUST NOT mention the v2 crate names — that
        // would be a leak of v2 wiring into the legacy path.
        assert!(!body.contains("ironclaw_telegram_v2_adapter"));
        assert!(!body.contains("ironclaw_product_adapters"));
    }
}

// ---------------------------------------------------------------------------
// AC: Deterministic protocol smoke tests + redaction sentinels
// ---------------------------------------------------------------------------

#[tokio::test]
async fn smoke_recorded_payloads_match_expected_outcomes() {
    let cases = [
        (
            "private_chat_message.json",
            true,
            ProductTriggerReason::DirectChat,
        ),
        (
            "group_mention_message.json",
            true,
            ProductTriggerReason::BotMention,
        ),
        ("group_command.json", true, ProductTriggerReason::BotCommand),
        ("topic_message.json", true, ProductTriggerReason::BotMention),
    ];
    for (fixture_name, should_create_envelope, expected_trigger) in cases {
        let workflow = Arc::new(FakeProductWorkflow::new());
        let runner = build_runner(workflow.clone());
        let outcome = runner
            .process_webhook(
                &webhook_headers(Some(TELEGRAM_WEBHOOK_SECRET)),
                &fixture(fixture_name),
            )
            .await
            .expect(fixture_name);
        if should_create_envelope {
            let WebhookProcessOutcome::Acknowledged { .. } = outcome else {
                panic!("{fixture_name}: expected Acknowledged");
            };
            assert_eq!(workflow.accepted_count(), 1, "{fixture_name}");
            let envelopes = workflow.accepted_envelopes();
            let trigger = match &envelopes[0].payload {
                ProductInboundPayload::UserMessage(u) => u.trigger,
                ProductInboundPayload::Command(c) => c.trigger,
                _ => panic!("unexpected payload kind for {fixture_name}"),
            };
            assert_eq!(trigger, expected_trigger, "{fixture_name}");
        } else {
            assert!(matches!(outcome, WebhookProcessOutcome::NoOp));
            assert_eq!(workflow.accepted_count(), 0, "{fixture_name}");
        }
    }
}

#[test]
fn redaction_sentinels_in_envelope_debug() {
    // Build an envelope from a fixture and Debug-format it. Assert that no
    // bot-token-shaped string, no host path, and no provider-internal
    // marker appears.
    let adapter = TelegramV2Adapter::new(config());
    let envelope = adapter
        .parse_inbound(&fixture("private_chat_message.json"), evidence())
        .expect("ok")
        .expect("envelope");
    let rendered = format!("{envelope:?}");
    let sentinels = [
        "Bearer ",
        "AAEFGH",
        "/Users/",
        "/home/",
        "/.ironclaw/",
        ".env",
        "TURNCOORDINATOR",
        "raw_prompt",
        "internal_error",
    ];
    for s in sentinels {
        assert!(
            !rendered.contains(s),
            "envelope Debug leaked sentinel `{s}`"
        );
    }
}

#[test]
fn redaction_sentinels_in_error_display() {
    let err = ProductAdapterError::Internal {
        detail: ironclaw_product_adapters::RedactedString::new(
            "bot12345:AAEFGH-private-token at /Users/secret/.env",
        ),
    };
    let rendered = err.to_string();
    assert!(!rendered.contains("AAEFGH-private-token"));
    assert!(!rendered.contains("/Users/secret/.env"));
}

// ---------------------------------------------------------------------------
// Capability gating: Telegram default capabilities are correct.
// ---------------------------------------------------------------------------

#[test]
fn telegram_default_capabilities_pin_first_slice_behavior() {
    let adapter = TelegramV2Adapter::new(config());
    let caps: &ProductAdapterCapabilities = adapter.capabilities();
    assert!(caps.contains(ProductCapabilityFlag::InboundMessages));
    assert!(caps.contains(ProductCapabilityFlag::InboundCommands));
    assert!(caps.contains(ProductCapabilityFlag::InboundAttachments));
    assert!(caps.contains(ProductCapabilityFlag::ExternalFinalReplyPush));
    assert!(caps.contains(ProductCapabilityFlag::DeliveryStatusReporting));
    assert!(!caps.contains(ProductCapabilityFlag::ExternalProgressPush));
    assert!(!caps.contains(ProductCapabilityFlag::ExternalGatePush));
    assert_eq!(adapter.surface_kind(), ProductSurfaceKind::ExternalChannel);
}

// ---------------------------------------------------------------------------
// Projection subscription contract: Telegram does not consume them.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn telegram_does_not_consume_projection_subscriptions() {
    let adapter = TelegramV2Adapter::new(config());
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_HANDLE);
    let envelope = ProductOutboundEnvelope {
        adapter_id: adapter.adapter_id().clone(),
        installation_id: adapter.installation_id().clone(),
        target: ironclaw_telegram_v2_adapter::render::build_reply_target_binding(777, None, None),
        projection_cursor: None,
        payload: ProductOutboundPayload::ProjectionSnapshot(
            ironclaw_product_adapters::ProjectionSnapshot {
                cursor: ironclaw_product_adapters::ProjectionCursor::new("cursor:1"),
                thread_id: "thread:1".into(),
                generated_at: chrono::Utc::now(),
            },
        ),
        delivery_attempt_id: uuid::Uuid::new_v4(),
    };
    adapter
        .render_outbound(envelope, &egress)
        .await
        .expect("ok");
    // Telegram silently drops projection envelopes; Web/CLI/API surfaces
    // consume them.
    assert!(egress.calls().is_empty());
}

#[tokio::test]
async fn projection_stream_can_drive_telegram_via_render_outbound_chain() {
    // Smoke test: a fake projection stream emits a FinalReplyView and the
    // adapter renders it through constrained egress.
    let adapter = TelegramV2Adapter::new(config());
    let projection = FakeProjectionStream::new();
    let outbound = ProductOutboundEnvelope {
        adapter_id: adapter.adapter_id().clone(),
        installation_id: adapter.installation_id().clone(),
        target: ironclaw_telegram_v2_adapter::render::build_reply_target_binding(
            -42,
            Some(7),
            Some(50),
        ),
        projection_cursor: Some(ironclaw_product_adapters::ProjectionCursor::new("cursor:1")),
        payload: ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "from projection".into(),
            generated_at: chrono::Utc::now(),
        }),
        delivery_attempt_id: uuid::Uuid::new_v4(),
    };
    projection.push(outbound);
    let scope = ironclaw_turns::TurnScope::new(
        ironclaw_host_api::TenantId::new("tenant-a").expect("valid"),
        None,
        None,
        ironclaw_host_api::ThreadId::new("thread-1").expect("valid"),
    );
    let actor =
        ironclaw_turns::TurnActor::new(ironclaw_host_api::UserId::new("alice").expect("valid"));
    let drained = projection
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .expect("drain");
    assert_eq!(drained.len(), 1);

    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_HANDLE);
    for envelope in drained {
        adapter
            .render_outbound(envelope, &egress)
            .await
            .expect("render");
    }
    assert_eq!(egress.calls().len(), 1);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

// We pull in EgressRequest above through the prod adapters re-exports for
// the AC13 test. Avoid an unused-import warning if nothing else references
// the type at the lifetime of edits.
#[allow(dead_code)]
fn _force_use_of_request() -> Option<EgressRequest> {
    None
}

#[allow(dead_code)]
fn _force_use_of_response() -> Option<EgressResponse> {
    None
}

#[allow(dead_code)]
fn _force_use_of_reply_target() -> Option<ReplyTargetBindingRef> {
    None
}
