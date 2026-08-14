//! Explicit channel-delivery user journeys (`builtin.outbound_deliver`).
//!
//! The tool is the ONLY way a run puts content on a surface other than its
//! own conversation, so every journey here proves the same two independent
//! seams — a green run status proves neither:
//!
//! 1. **Wire recorder** (`captured_network_requests_for_test`): the vendor
//!    API call the destination channel's REAL adapter made, under the
//!    host-injected **bot** credential handle. This is the "did the message
//!    actually leave" seam; without it a tool could report success on a
//!    delivery that never reached the vendor.
//! 2. **Attempt ledger** (`outbound_delivery_stores_for_test` →
//!    `list_delivery_attempts`): the coordinator's persisted, terminal
//!    `OutboundDeliveryAttempt` carrying `OutboundPushKind::ModelDelivery`.
//!    This is the "is it auditable and policy-classed correctly" seam;
//!    without it an explicit delivery would be indistinguishable from an
//!    ordinary final-reply push in the audit trail.
//!
//! Journeys additionally pin the model-visible half — the tool RESULT's
//! `provider_message_refs` must equal the refs the recorded vendor response
//! actually returned (spec §5: never claim what the vendor did not confirm)
//! — and lane-1 separation: the run's own final reply still lands in the
//! originating conversation with NO second external attempt.
//!
//! Beyond the WebUI-origin journey (§13.1), this file also covers a
//! Slack-origin cross-channel delivery to Telegram with the lane-1 echo back
//! to Slack (§13.2), a same-origin delivery denial (§13.3), a partial
//! failure across two deliver calls in one turn (§13.4), and an undeliverable
//! destination that never reaches the tool at all (§13.8). The two
//! Slack-origin journeys admit their turn directly through the generic
//! sink's `InboundSink` seam (`GenericChannelInboundSink::admit`), skipping
//! HTTP transport and signature verification entirely — that transport/auth
//! proof belongs to `reborn_integration_extension_delivery`, exhaustively;
//! this suite only needs a REAL admitted turn under a REAL vendor scope.
//!
//! Complementary suites: `reborn_integration_extension_delivery` owns
//! implicit-source channel ingress (signed HTTP → verified admission) → run →
//! bot-reply (lane 1); `group_triggers` owns trigger-selected delivery through
//! the codec-based `TriggeredRunDeliveryDriver`.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use ironclaw_assistant::{RunDeliveryObserver, RunDeliveryServices};
use ironclaw_composition::RebornRuntime;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelIngress, InboundOutcome, NormalizedInboundMessage, VerifiedInbound,
};
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::preference_target::{
    ActivePreferenceTargetCodecs, PreferenceTargetCodec, PreferenceTargetEncodeRequest,
};
use ironclaw_extension_host::extension_ingress::{
    ChannelInboundSinkConfig, ChannelIngressDrain, GenericChannelInboundSink,
    PostAdmissionObserver, VerifiedEvidenceMint,
};
use ironclaw_extension_host::ingress::{InboundAdmission, InboundAdmissionAck, InboundSink};
use ironclaw_host_api::product_adapter::auth::{AuthRequirement, ProtocolAuthEvidence};
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
use ironclaw_outbound::{
    DeliveryFailureKind, OutboundDeliveryAttempt, OutboundDeliveryStatus, OutboundPushKind,
};
use ironclaw_product_contracts::binding::ProductBindingResolver;
use ironclaw_product_contracts::binding::ResolveBindingRequest;
use ironclaw_product_contracts::inbound::{
    ParsedProductInbound, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    TrustedInboundContext, UserMessagePayload,
};
use ironclaw_product_contracts::surface::ChannelInboundProductSurface;
use ironclaw_turns::{GetRunStateRequest, TurnCoordinator, TurnRunId, TurnScope, TurnStatus};
use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const OUTBOUND_DELIVER: &str = "builtin.outbound_deliver";

const SLACK_BOT_TOKEN: &str = "xoxb-journey-bot-token";
const SLACK_INSTALLATION: &str = "slack-journey-install";
const SLACK_TEAM: &str = "T-JOURNEY";
const SLACK_DM_CHANNEL: &str = "D-JOURNEY";
const SLACK_DM_ACTOR: &str = "U-JOURNEY";
/// The exact opaque catalog id the model names in its tool call. Registered
/// on the caller-owned outbound registry; never invented by the model.
const SLACK_DM_TARGET_ID: &str = "slack:journey-dm";
/// `ts` the profile's Slack vendor router answers `chat.postMessage` with —
/// the ONLY value a truthful `provider_message_refs` may carry.
const SLACK_VENDOR_TS: &str = "1710000200.000001";

const DELIVERED_TEXT: &str = "Here is the summary you asked for on your other channel.";
const WEBUI_ACK: &str = "Sent it to your Slack DM.";

// ── Cross-channel journey (Slack origin -> Telegram destination) ──────────
const CROSS_SLACK_DM_CHANNEL: &str = "D-CROSS-ORIGIN";
const CROSS_SLACK_DM_ACTOR: &str = "U-CROSS-ORIGIN";
const CROSS_SLACK_EVENT_TS: &str = "1710000400.000100";
const CROSS_INBOUND_MESSAGE: &str = "please forward this update to my telegram";
/// The exact opaque catalog id the model names for the Telegram DM target.
const CROSS_TELEGRAM_TARGET_ID: &str = "telegram:journey-dm";
/// A canonical (no leading zero/sign), positive Telegram chat id — positive
/// chat ids are Telegram private chats (`ironclaw_telegram_extension`'s
/// codec doc comment), so this is a personal DM.
const CROSS_TELEGRAM_CHAT_ID: &str = "919191919";
const CROSS_TELEGRAM_BOT_TOKEN: &str = "987654:journey-telegram-bot-token";
const CROSS_DELIVERED_TEXT: &str = "Here is the update on your Telegram DM.";
const CROSS_SLACK_ACK: &str = "Sent it to your Telegram DM.";
/// `result.message_id` the profile's Telegram vendor router answers
/// `sendMessage` with (`harness/profiles/extension.rs::delivery_vendor_router`).
const TELEGRAM_VENDOR_MESSAGE_ID: &str = "4242";

// ── Same-origin denial journey ─────────────────────────────────────────────

// ── Partial-failure journey ────────────────────────────────────────────────
const PARTIAL_OK_DM_CHANNEL: &str = "D-JOURNEY-OK";
const PARTIAL_OK_DM_ACTOR: &str = "U-JOURNEY-OK";
const PARTIAL_OK_TARGET_ID: &str = "slack:journey-dm-ok";
const PARTIAL_OK_CONTENT: &str = "Here is the first update.";
/// Slack conversation id the shared vendor router
/// (`harness/profiles/extension.rs::DELIVERY_VENDOR_PERMANENT_FAILURE_CHANNEL`)
/// maps to a scripted permanent `channel_not_found` rejection instead of the
/// happy-path body. Mirrored, not imported — a separate test binary.
const PARTIAL_FAILING_DM_CHANNEL: &str = "D-JOURNEY-VENDOR-REJECT";
const PARTIAL_FAILING_DM_ACTOR: &str = "U-JOURNEY-REJECT";
const PARTIAL_FAILING_TARGET_ID: &str = "slack:journey-dm-reject";
const PARTIAL_FAILING_CONTENT: &str = "Here is the second update.";
const PARTIAL_FAILURE_ACK: &str = "Sent the first update, but the second one failed to deliver.";

// ── Undeliverable-destination journey ──────────────────────────────────────
const UNDELIVERABLE_REQUEST: &str = "email me the report";
const UNDELIVERABLE_EXPLANATION: &str =
    "I don't see an email or Gmail connection set up yet, so I can't send that there.";

/// The Slack DM binding ref a catalog target resolves to, minted through the
/// vendor's own grammar (`ironclaw_slack_extension`) exactly like the generic
/// outbound-target provider does in production — so the codec-driven
/// resolver behind the coordinator decodes it back to `(team, DM channel)`.
/// Parameterized over the DM channel/actor so callers can mint a target for
/// ANY Slack DM (a second success target, a deliberately failing target, or
/// the origin conversation itself for the same-origin-denial journey).
fn slack_dm_binding_ref_for(
    harness: &RebornIntegrationHarness,
    dm_channel: &str,
    dm_actor: &str,
) -> ironclaw_turns::ReplyTargetBindingRef {
    let installation =
        AdapterInstallationId::new(SLACK_INSTALLATION).expect("slack installation id");
    let agent_id = harness
        .turn_scope
        .agent_id
        .clone()
        .expect("harness turn scope carries an agent id");
    ironclaw_slack_extension::slack_personal_dm_reply_target_binding_ref(
        &installation,
        &agent_id,
        harness.turn_scope.project_id.as_ref(),
        SLACK_TEAM,
        dm_channel,
        dm_actor,
    )
    .expect("slack personal DM binding ref")
}

fn slack_dm_binding_ref(
    harness: &RebornIntegrationHarness,
) -> ironclaw_turns::ReplyTargetBindingRef {
    slack_dm_binding_ref_for(harness, SLACK_DM_CHANNEL, SLACK_DM_ACTOR)
}

/// The Telegram DM binding ref a catalog target resolves to
/// (`tg:<chat_id>:_:_`), minted through the vendor's OWN preference-target
/// codec (`ironclaw_telegram_extension::TelegramPreferenceTargetCodec`) — the
/// same codec `factory.rs` folds into the `CodecChannelTargetResolver` behind
/// the coordinator, so this round-trips exactly like production. Telegram's
/// grammar carries no installation/agent/project identity segment (unlike
/// Slack's), so those fields are structurally required by
/// `PreferenceTargetEncodeRequest` but do not affect the encoded value.
fn telegram_dm_binding_ref(
    harness: &RebornIntegrationHarness,
    chat_id: &str,
) -> ironclaw_turns::ReplyTargetBindingRef {
    let installation =
        AdapterInstallationId::new("telegram-journey").expect("telegram installation id");
    let agent_id = harness
        .turn_scope
        .agent_id
        .clone()
        .expect("harness turn scope carries an agent id");
    let project_id = harness.turn_scope.project_id.clone();
    let conversation = ExternalConversationRef::new(None::<&str>, chat_id, None, None)
        .expect("telegram DM conversation ref");
    let request = PreferenceTargetEncodeRequest {
        installation_id: &installation,
        agent_id: &agent_id,
        project_id: project_id.as_ref(),
        conversation: &conversation,
    };
    ironclaw_telegram_extension::TelegramPreferenceTargetCodec
        .encode_personal_direct_message_target(request, chat_id)
        .expect("telegram personal DM binding ref")
}

fn reborn_services(group: &RebornIntegrationGroup) -> &RebornRuntime {
    group
        .capability_harness()
        .expect("host-runtime capability harness")
        .reborn_services_for_test()
        .expect("composed reborn services")
}

/// Install the REAL bundled Slack package through the production lifecycle
/// tool so the coordinator's snapshot resolver sees an active channel
/// binding (mirrors `extension_delivery.rs::activate_slack`).
async fn activate_slack(group: &RebornIntegrationGroup) {
    let lifecycle = group
        .thread("conv-delivery-journey-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("installed and ready"),
        ])
        .build()
        .await
        .expect("slack lifecycle thread builds");
    lifecycle
        .seed_capability_credential_account(
            "slack",
            "slack delivery journey account",
            &[
                "search:read",
                "channels:history",
                "groups:history",
                "im:history",
                "mpim:history",
                "channels:read",
                "groups:read",
                "im:read",
                "mpim:read",
                "users:read",
                "chat:write",
                "reactions:read",
                "reactions:write",
                "im:write",
            ],
        )
        .await
        .expect("seed slack account");
    lifecycle
        .submit_turn("install slack")
        .await
        .expect("slack install completes");
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await
        .expect("slack install completed readiness and publication");
}

/// Assertion seam 2: the coordinator's persisted ledger for `scope` holds
/// exactly `expected` terminal `Delivered` attempts, every one of them a
/// `ModelDelivery` push, and none stranded mid-lifecycle. Bounded poll —
/// delivery settles inside the tool call, but the store write is async.
/// Mechanically citable Slack DM delivery evidence. The journey-coverage gate
/// (`tests/e2e/scenarios/test_journey_coverage.py`) scrapes this helper's
/// exact literals, so the delivered DM conversation and the unthreaded anchor
/// stay independently verifiable against the wire capture.
fn assert_slack_dm_delivery_evidence(
    posts: &[&ironclaw_network::NetworkHttpRequest],
    delivered_text: &str,
) {
    let expected_conversation_id = "D-JOURNEY";
    let expected_thread_anchor: Option<&serde_json::Value> = None;
    let expected_count = 1;
    let matching = posts.iter().filter(|post| {
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(&post.body) else {
            return false;
        };
        body["channel"] == expected_conversation_id
            && body.get("thread_ts") == expected_thread_anchor
            && body["text"]
                .as_str()
                .is_some_and(|text| text.contains(delivered_text))
    });
    assert_eq!(
        matching.count(),
        expected_count,
        "exactly one unthreaded vendor send must carry the delivered content to the DM \
         conversation; got {:?}",
        posts
            .iter()
            .map(|post| String::from_utf8_lossy(&post.body).into_owned())
            .collect::<Vec<_>>()
    );
}

async fn assert_model_delivery_attempts(
    services: &RebornRuntime,
    scope: &TurnScope,
    expected: usize,
) {
    let (outbound_store, _, _, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("outbound stores");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let attempts = loop {
        let attempts = outbound_store
            .list_delivery_attempts(scope.clone())
            .await
            .expect("list delivery attempts");
        let delivered = attempts
            .iter()
            .filter(|attempt| attempt.status == OutboundDeliveryStatus::Delivered)
            .count();
        let all_terminal = attempts.iter().all(|attempt| {
            !matches!(
                attempt.status,
                OutboundDeliveryStatus::Prepared
                    | OutboundDeliveryStatus::Sending
                    | OutboundDeliveryStatus::Pending
            )
        });
        if delivered >= expected && all_terminal {
            break attempts;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {expected} terminal Delivered attempt(s); got {:?}",
            attempts
                .iter()
                .map(|attempt| (attempt.status, attempt.candidate.kind))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let delivered: Vec<_> = attempts
        .iter()
        .filter(|attempt| attempt.status == OutboundDeliveryStatus::Delivered)
        .collect();
    assert_eq!(
        delivered.len(),
        expected,
        "exactly {expected} delivered attempt(s) expected; got {:?}",
        attempts
            .iter()
            .map(|attempt| (attempt.status, attempt.candidate.kind))
            .collect::<Vec<_>>()
    );
    // The distinguishing audit fact: an explicit model delivery must not be
    // recorded as an ordinary final-reply push.
    for attempt in &delivered {
        assert_eq!(
            attempt.candidate.kind,
            OutboundPushKind::ModelDelivery,
            "explicit deliveries must be audit-distinguishable as ModelDelivery"
        );
    }
}

/// Assertion seam 2 variant: the ledger for `scope` holds NO delivery
/// attempts at all — for journeys where the tool is denied before ever
/// reaching the coordinator (same-origin), never called at all
/// (undeliverable destination / conditional no-op fire), or fanned out to an
/// empty notification set. One-shot: every caller reads this only at a
/// settled point — either after its turn fully completed, or (the
/// blocked-fire caller) after `wait_for_triggered_outcome` proved the
/// notifier settled — and none of those paths writes an attempt row in the
/// first place (a same-origin denial returns before
/// `DeliveryCoordinator::deliver` is ever called), so there is nothing to
/// poll for. Callers on a still-live run MUST settle the notifier first.
async fn assert_no_delivery_attempts(services: &RebornRuntime, scope: &TurnScope) {
    let (outbound_store, _, _, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("outbound stores");
    let attempts = outbound_store
        .list_delivery_attempts(scope.clone())
        .await
        .expect("list delivery attempts");
    assert!(
        attempts.is_empty(),
        "expected zero delivery attempts; got {:?}",
        attempts
            .iter()
            .map(|attempt| (attempt.status, attempt.candidate.kind))
            .collect::<Vec<_>>()
    );
}

/// Assertion seam 2 variant for the cross-channel journey: the SAME scope's
/// ledger legitimately carries BOTH an explicit `ModelDelivery` push (the
/// tool call to Telegram) and the run's own `FinalReply` push (lane-1's
/// automatic origin echo back to Slack, driven by the SAME coordinator/
/// ledger) — attribution must distinguish the two kinds rather than assume
/// every Delivered row is a `ModelDelivery` the way
/// [`assert_model_delivery_attempts`] does for the single-surface journeys.
async fn assert_delivered_attempts_by_kind(
    services: &RebornRuntime,
    scope: &TurnScope,
    expected_model_delivery: usize,
    expected_final_reply: usize,
) {
    let (outbound_store, _, _, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("outbound stores");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let count_by_kind = |attempts: &[OutboundDeliveryAttempt], kind: OutboundPushKind| {
        attempts
            .iter()
            .filter(|attempt| {
                attempt.status == OutboundDeliveryStatus::Delivered
                    && attempt.candidate.kind == kind
            })
            .count()
    };
    let attempts = loop {
        let attempts = outbound_store
            .list_delivery_attempts(scope.clone())
            .await
            .expect("list delivery attempts");
        let all_terminal = attempts.iter().all(|attempt| {
            !matches!(
                attempt.status,
                OutboundDeliveryStatus::Prepared
                    | OutboundDeliveryStatus::Sending
                    | OutboundDeliveryStatus::Pending
            )
        });
        if all_terminal
            && count_by_kind(&attempts, OutboundPushKind::ModelDelivery) >= expected_model_delivery
            && count_by_kind(&attempts, OutboundPushKind::FinalReply) >= expected_final_reply
        {
            break attempts;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {expected_model_delivery} ModelDelivery + {expected_final_reply} FinalReply terminal Delivered attempt(s); got {:?}",
            attempts
                .iter()
                .map(|attempt| (attempt.status, attempt.candidate.kind))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(
        count_by_kind(&attempts, OutboundPushKind::ModelDelivery),
        expected_model_delivery,
        "unexpected ModelDelivery delivered count; got {:?}",
        attempts
            .iter()
            .map(|attempt| (attempt.status, attempt.candidate.kind))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        count_by_kind(&attempts, OutboundPushKind::FinalReply),
        expected_final_reply,
        "unexpected FinalReply delivered count; got {:?}",
        attempts
            .iter()
            .map(|attempt| (attempt.status, attempt.candidate.kind))
            .collect::<Vec<_>>()
    );
    // Deliberately no "total attempts == the two expected kinds" check here:
    // a channel-origin run also posts (and later retracts) an immediate
    // "thinking" placeholder ahead of the real reply, a separate progress-kind
    // push this journey does not own. Per-kind counts above are the load-
    // bearing proof of attribution.
}

/// Assertion seam 2 variant for the partial-failure journey: the ledger
/// holds exactly one terminal Delivered `ModelDelivery` attempt (the
/// succeeding leg) and exactly one terminal Failed `ModelDelivery` attempt
/// carrying `DeliveryFailureKind::Rejected` (the permanently-rejected leg,
/// §OUT-7 — never retried), with no attempt stranded mid-lifecycle.
async fn assert_partial_failure_attempts(services: &RebornRuntime, scope: &TurnScope) {
    let (outbound_store, _, _, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("outbound stores");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let attempts = loop {
        let attempts = outbound_store
            .list_delivery_attempts(scope.clone())
            .await
            .expect("list delivery attempts");
        let all_terminal = attempts.iter().all(|attempt| {
            !matches!(
                attempt.status,
                OutboundDeliveryStatus::Prepared
                    | OutboundDeliveryStatus::Sending
                    | OutboundDeliveryStatus::Pending
            )
        });
        if all_terminal && attempts.len() >= 2 {
            break attempts;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for 2 terminal delivery attempts; got {:?}",
            attempts
                .iter()
                .map(|attempt| (attempt.status, attempt.candidate.kind))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let delivered = attempts
        .iter()
        .filter(|attempt| {
            attempt.status == OutboundDeliveryStatus::Delivered
                && attempt.candidate.kind == OutboundPushKind::ModelDelivery
        })
        .count();
    let failed_rejected = attempts
        .iter()
        .filter(|attempt| {
            attempt.status == OutboundDeliveryStatus::Failed
                && attempt.candidate.kind == OutboundPushKind::ModelDelivery
                && attempt.failure_kind == Some(DeliveryFailureKind::Rejected)
        })
        .count();
    assert_eq!(
        (delivered, failed_rejected, attempts.len()),
        (1, 1, 2),
        "expected exactly one Delivered + one Failed{{Rejected}} ModelDelivery attempt; got {:?}",
        attempts
            .iter()
            .map(|attempt| (attempt.status, attempt.candidate.kind, attempt.failure_kind))
            .collect::<Vec<_>>()
    );
}

// ── Shared plumbing: Slack-origin admission without HTTP/signature transport ──
//
// The cross-channel and same-origin-denial journeys below both need a REAL
// turn whose reply-target binding is a genuine Slack conversation, so lane-1's
// automatic FinalReply echo and the same-origin check both see a real vendor
// binding. This suite is about the delivery TOOL, not ingress transport or
// signature verification — that is `reborn_integration_extension_delivery`'s
// job, exhaustively — so admission goes straight through the generic sink's
// `InboundSink` seam (`GenericChannelInboundSink::admit`), skipping HTTP and
// HMAC signing entirely. Everything from admission onward — the idempotency
// ledger, identity/conversation binding, turn submission, and (via the
// observer) the REAL `RunDeliveryObserver` coordinating the FinalReply back to
// the vendor — is production code.

/// Parse a raw Slack Events API body through the REAL adapter into one
/// normalized message (mirrors the first half of `extension_delivery.rs`'s
/// `preresolve_vendor_turn_scope`, without the HTTP layer).
async fn parse_slack_dm_message(body: &str) -> NormalizedInboundMessage {
    let egress = ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer::new(
        Arc::new(
            |_| ironclaw_extension_contracts::tool_adapter::RestrictedEgressResponse {
                status: 200,
                body: Vec::new(),
            },
        ),
    );
    let outcome = ironclaw_slack_extension::SlackChannelAdapter
        .receive(
            VerifiedInbound {
                extension_id: "slack",
                installation_id: SLACK_INSTALLATION,
                config: &[],
                body: body.as_bytes(),
                headers: &[],
                // Slack's manifest `presentation.can_reply_in_threads` (#7377
                // made the flag load-bearing); a DM body is unaffected either
                // way, but the value must mirror the shipped manifest.
                can_reply_in_threads: true,
            },
            &egress,
        )
        .await
        .expect("the slack DM body must parse through the real adapter");
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("the slack DM body must normalize to messages");
    };
    messages
        .into_iter()
        .next()
        .expect("one normalized slack message")
}

/// Resolve the vendor conversation's REAL `TurnScope` through the SAME
/// binding service the admission below uses, BEFORE admitting — so the
/// scripted gateway can be registered for the run's exact scope up front
/// (mirrors `extension_delivery.rs`'s `preresolve_vendor_turn_scope`).
async fn preresolve_slack_scope(
    binding_service: &Arc<dyn ProductBindingResolver>,
    message: &NormalizedInboundMessage,
    evidence: &ProtocolAuthEvidence,
) -> TurnScope {
    let context = TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new("slack").expect("adapter id"),
        AdapterInstallationId::new(SLACK_INSTALLATION).expect("installation id"),
        Utc::now(),
        evidence,
    )
    .expect("trusted inbound context");
    let payload = ProductInboundPayload::UserMessage(
        UserMessagePayload::new(message.text.clone(), Vec::new(), message.trigger)
            .expect("user message payload"),
    );
    let parsed = ParsedProductInbound::new(
        message.event_id.clone(),
        message.actor.clone(),
        message.conversation.clone(),
        payload,
    )
    .expect("parsed inbound");
    let envelope =
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("inbound envelope");
    let binding = binding_service
        .resolve_binding(
            ResolveBindingRequest::from_envelope(&envelope)
                .expect("verified envelope binding request"),
        )
        .await
        .expect("slack DM conversation binding resolves");
    TurnScope::new_with_owner(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
        Some(binding.actor_user_id.clone()),
    )
}

/// Real generic run-delivery services (mirrors `extension_delivery.rs`'s
/// `delivery_run_services`): the harness's binding/thread/turn services plus
/// the composed runtime's coordinator and outbound stores, so the
/// `RunDeliveryObserver` this journey wires shares the SAME delivery ledger
/// `builtin.outbound_deliver`'s tool dispatch writes to.
fn slack_run_delivery_services(
    harness: &RebornIntegrationHarness,
    services: &RebornRuntime,
) -> RunDeliveryServices {
    let (outbound_store, route_store, communication_preferences, _, delivery_targets) = services
        .outbound_delivery_stores_for_test()
        .expect("composed runtime exposes the coordinator's outbound stores");
    let coordinator = services
        .delivery_coordinator()
        .expect("composition built the delivery coordinator");
    let fallback_notice_scope = TurnScope::new_with_owner(
        harness.binding.tenant_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
        ironclaw_host_api::ids::ThreadId::new("slack-journey-channel-notices")
            .expect("notice thread id"),
        Some(harness.binding.actor_user_id.clone()),
    );
    RunDeliveryServices {
        project_filesystem: Arc::new(ironclaw_assistant::NoProjectFilesystem),
        binding_service: harness
            .binding_service_for_test()
            .expect("group binding service"),
        thread_service: harness
            .thread_service_for_test()
            .expect("group thread service"),
        turn_coordinator: harness.turn_coordinator_for_test(),
        outbound_store,
        route_store,
        communication_preferences,
        delivery_targets,
        coordinator,
        extension_id: "slack".to_string(),
        fallback_notice_scope,
        approval_context: None,
        blocked_auth_prompts: None,
        auth_flow_cancel: None,
    }
}

/// Forwards every ack/error to the REAL `RunDeliveryObserver` (so lane-1's
/// FinalReply is genuinely coordinated back to Slack), while capturing the
/// admitted run id. The run itself executes on the group's shared scheduler,
/// so the caller must still poll for `Completed` after admission settles.
struct CapturingForwardObserver {
    inner: Arc<RunDeliveryObserver>,
    accepted_run_id: Mutex<Option<TurnRunId>>,
}

impl CapturingForwardObserver {
    fn new(inner: Arc<RunDeliveryObserver>) -> Self {
        Self {
            inner,
            accepted_run_id: Mutex::new(None),
        }
    }

    fn accepted_run_id(&self) -> Option<TurnRunId> {
        *self.accepted_run_id.lock().expect("captured run id lock")
    }
}

#[async_trait::async_trait]
impl PostAdmissionObserver for CapturingForwardObserver {
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        if let ProductInboundAck::Accepted {
            submitted_run_id, ..
        } = &ack
        {
            *self.accepted_run_id.lock().expect("captured run id lock") = Some(*submitted_run_id);
        }
        self.inner.observe_ack(envelope, ack).await;
    }

    async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ironclaw_host_api::product_adapter::ProductAdapterError,
    ) {
        self.inner.observe_error(envelope, error).await;
    }
}

/// Parse + pre-resolve in one step: the shape both Slack-origin journeys need
/// BEFORE they can register a scripted gateway for the run they are about to
/// admit (`admit_slack_dm_message`, below, must run strictly after the
/// caller has registered that gateway for the returned scope).
fn slack_agent() -> ironclaw_host_api::ids::AgentId {
    ironclaw_host_api::ids::AgentId::new("slack-journey-agent").expect("agent id")
}

async fn parse_and_preresolve_slack_dm(
    harness: &RebornIntegrationHarness,
    body: &str,
) -> (NormalizedInboundMessage, TurnScope) {
    let message = parse_slack_dm_message(body).await;
    // `test_verified` is the `test-support` seam standing in for the ingress
    // verifier: minting is witness-gated (PROPOSAL §11.2.5) and the harness
    // holds no `VerifiedInboundGrant`.
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::RequestSignature {
            header_name: "X-Slack-Signature".to_string(),
            timestamp_header_name: Some("X-Slack-Request-Timestamp".to_string()),
        },
        SLACK_INSTALLATION,
    );
    let binding_service = harness
        .binding_service_for_test()
        .expect("group binding service");
    let scope = preresolve_slack_scope(&binding_service, &message, &evidence).await;
    (message, scope)
}

/// Admit an ALREADY-PARSED Slack DM message directly through the generic
/// sink (no HTTP, no HMAC — see the module note above), and drain its
/// spawned post-admission observer task. Call only AFTER registering the
/// scripted gateway for the scope `parse_and_preresolve_slack_dm` returned
/// for this same message — the admitted run dispatches to the model
/// immediately.
async fn admit_slack_dm_message(
    harness: &RebornIntegrationHarness,
    observer: Arc<CapturingForwardObserver>,
    message: NormalizedInboundMessage,
) {
    let sink = Arc::new(GenericChannelInboundSink::new(ChannelInboundSinkConfig {
        adapter_id: ProductAdapterId::new("slack").expect("adapter id"),
        evidence: VerifiedEvidenceMint::RequestSignature {
            signature_header: "X-Slack-Signature".to_string(),
            timestamp_header: Some("X-Slack-Request-Timestamp".to_string()),
        },
        surface: harness.product_surface_for_test() as Arc<dyn ChannelInboundProductSurface>,
        observer: Some(observer as Arc<dyn PostAdmissionObserver>),
    }));
    let ack = sink
        .admit(InboundAdmission {
            extension_id: "slack".to_string(),
            installation_id: SLACK_INSTALLATION.to_string(),
            message,
        })
        .await
        .expect("slack DM event is durably admitted");
    assert!(
        matches!(ack, InboundAdmissionAck::Accepted),
        "the slack DM must be admitted; got {ack:?}"
    );
    (sink as Arc<dyn ChannelIngressDrain>).drain().await;
}

/// Poll the shared coordinator until `run_id` in `scope` reaches `expected`,
/// failing fast on any OTHER terminal status (mirrors
/// `extension_delivery.rs`'s `wait_for_run_status_in_scope`).
async fn wait_for_run_status_in_scope(
    coordinator: &Arc<dyn TurnCoordinator>,
    scope: &TurnScope,
    run_id: TurnRunId,
    expected: TurnStatus,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let state = coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .expect("vendor-scoped run state remains readable");
        if state.status == expected {
            return;
        }
        assert!(
            !state.status.is_terminal(),
            "expected {expected:?} but vendor-scoped run reached {:?}; failure={:?}",
            state.status,
            state.failure
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for vendor-scoped run {run_id} to reach {expected:?}; last status={:?}",
            state.status
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Spec §13.1 — "send me X on my other channel" from the WebUI.
///
/// A WebUI-origin run calls `builtin.outbound_deliver` with the catalog id of
/// a Slack DM, then acks in the WebUI thread. Proof at both seams plus the
/// model-visible evidence and lane-1 separation.
#[tokio::test(flavor = "multi_thread")]
async fn webui_send_me_on_slack_delivers_via_bot_with_evidence() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    activate_slack(&group).await;
    let services = reborn_services(&group);
    assert!(
        services.register_static_channel_egress_credentials_for_test(vec![(
            "slack".to_string(),
            "slack_bot_token".to_string(),
            ironclaw_secrets::SecretMaterial::from(SLACK_BOT_TOKEN.to_string()),
        )]),
        "the composed runtime must expose channel-egress credential bridging"
    );

    let harness = group
        .thread("conv-webui-deliver-slack")
        .script([
            RebornScriptedReply::tool_call(
                OUTBOUND_DELIVER,
                json!({"target_id": SLACK_DM_TARGET_ID, "content": DELIVERED_TEXT}),
            ),
            RebornScriptedReply::text(WEBUI_ACK),
        ])
        .build()
        .await
        .expect("webui delivery thread builds");
    group
        .register_source_delivery_target_for_test(
            "slack",
            SLACK_DM_TARGET_ID,
            slack_dm_binding_ref(&harness),
        )
        .expect("slack DM target registers on the caller-owned registry");

    harness
        .submit_turn("send me the summary on my slack dm")
        .await
        .expect("webui delivery turn completes");

    // (c) Model-visible evidence: the refs the vendor actually returned.
    let output = harness
        .tool_result_output(OUTBOUND_DELIVER)
        .await
        .expect("outbound_deliver produced a recorded tool result");
    assert_eq!(
        output["delivered"],
        json!(true),
        "the tool must report the delivery it performed; got {output}"
    );
    assert_eq!(
        output["target_id"],
        json!(SLACK_DM_TARGET_ID),
        "the result must name the exact catalog target; got {output}"
    );
    assert_eq!(
        output["provider_message_refs"],
        json!([SLACK_VENDOR_TS]),
        "provider_message_refs must be exactly the recorded vendor response's refs; got {output}"
    );

    // (a) Wire seam: one chat.postMessage carrying the delivered content to
    // the DM conversation, under the host-injected BOT credential handle.
    let requests = harness.captured_network_requests_for_test();
    let posts: Vec<_> = requests
        .iter()
        .filter(|request| request.url.ends_with("/api/chat.postMessage"))
        .collect();
    assert_slack_dm_delivery_evidence(&posts, DELIVERED_TEXT);
    let authorization = posts[0]
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .expect("host-side credential injection must add the authorization header");
    assert_eq!(
        authorization.1,
        format!("Bearer {SLACK_BOT_TOKEN}"),
        "explicit delivery speaks as the assistant (bot), never as the user"
    );

    // (b) Attempt ledger: one terminal Delivered ModelDelivery attempt.
    assert_model_delivery_attempts(services, &harness.turn_scope, 1).await;

    // (d) Lane 1 is untouched: the final reply is an ack rendered in the
    // WebUI thread, and it produced NO second external attempt (the single
    // postMessage asserted above is the tool's, not a final-reply echo).
    harness
        .assert_reply_contains(WEBUI_ACK)
        .await
        .expect("the run's own final reply lands in the WebUI thread");
}

/// Spec §13.2 — Slack-origin run delivers to Telegram; Slack still gets the
/// lane-1 echo.
///
/// A run admitted from a Slack DM calls `builtin.outbound_deliver` with the
/// catalog id of a Telegram DM, then acks in the SAME Slack conversation.
/// Two independent surfaces, two independent ledger rows: the explicit
/// `ModelDelivery` push to Telegram (the tool) and the automatic
/// `FinalReply` push to Slack (lane 1, the REAL `RunDeliveryObserver` — no
/// second tool attempt to Slack).
#[tokio::test(flavor = "multi_thread")]
async fn slack_origin_delivers_to_telegram_and_acks_in_slack() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    activate_slack(&group).await;
    let services = reborn_services(&group);
    assert!(
        services.register_static_channel_egress_credentials_for_test(vec![
            (
                "slack".to_string(),
                "slack_bot_token".to_string(),
                ironclaw_secrets::SecretMaterial::from(SLACK_BOT_TOKEN.to_string()),
            ),
            (
                "telegram".to_string(),
                "telegram_bot_token".to_string(),
                ironclaw_secrets::SecretMaterial::from(CROSS_TELEGRAM_BOT_TOKEN.to_string()),
            ),
        ]),
        "the composed runtime must expose channel-egress credential bridging"
    );

    let inbound = group
        .thread("conv-cross-channel-inbound")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("cross-channel inbound thread builds");
    group
        .register_source_delivery_target_for_test(
            "telegram",
            CROSS_TELEGRAM_TARGET_ID,
            telegram_dm_binding_ref(&inbound, CROSS_TELEGRAM_CHAT_ID),
        )
        .expect("telegram DM target registers on the caller-owned registry");

    let observer = Arc::new(CapturingForwardObserver::new(Arc::new(
        RunDeliveryObserver::new(slack_run_delivery_services(&inbound, services)),
    )));
    let body = json!({
        "type": "event_callback",
        "event_id": "Ev-cross-channel-1",
        "team_id": SLACK_TEAM,
        "event": {
            "type": "message",
            "user": CROSS_SLACK_DM_ACTOR,
            "channel": CROSS_SLACK_DM_CHANNEL,
            "channel_type": "im",
            "text": CROSS_INBOUND_MESSAGE,
            "ts": CROSS_SLACK_EVENT_TS
        }
    })
    .to_string();
    let (message, vendor_scope) = parse_and_preresolve_slack_dm(&inbound, &body).await;
    group
        .register_scope_script_for_test(
            vendor_scope.clone(),
            "cross-channel-vendor",
            [
                RebornScriptedReply::tool_call(
                    OUTBOUND_DELIVER,
                    json!({"target_id": CROSS_TELEGRAM_TARGET_ID, "content": CROSS_DELIVERED_TEXT}),
                ),
                RebornScriptedReply::text(CROSS_SLACK_ACK),
            ],
        )
        .await
        .expect("cross-channel vendor scope scripts");
    admit_slack_dm_message(&inbound, Arc::clone(&observer), message).await;
    let run_id = observer
        .accepted_run_id()
        .expect("the accepted slack DM must identify its submitted run");
    wait_for_run_status_in_scope(
        &inbound.turn_coordinator_for_test(),
        &vendor_scope,
        run_id,
        TurnStatus::Completed,
    )
    .await;

    // Model-visible evidence: the tool result carries the vendor's message id.
    let output = inbound
        .tool_result_output(OUTBOUND_DELIVER)
        .await
        .expect("outbound_deliver produced a recorded tool result");
    assert_eq!(
        output["target_id"],
        json!(CROSS_TELEGRAM_TARGET_ID),
        "the result must name the exact catalog target; got {output}"
    );
    assert_eq!(
        output["provider_message_refs"],
        json!([TELEGRAM_VENDOR_MESSAGE_ID]),
        "provider_message_refs must be exactly the recorded vendor response's refs; got {output}"
    );

    // Telegram wire: exactly one sendMessage carrying the delivered content.
    let requests = inbound.captured_network_requests_for_test();
    let sends: Vec<_> = requests
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .collect();
    assert_eq!(
        sends.len(),
        1,
        "exactly one telegram send must reach the wire; got {:?}",
        requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
    );
    let sent = String::from_utf8_lossy(&sends[0].body);
    assert!(
        sent.contains(CROSS_DELIVERED_TEXT),
        "the telegram send must carry the delivered content; got {sent}"
    );
    assert!(
        sent.contains(CROSS_TELEGRAM_CHAT_ID),
        "the telegram send must target the registered DM chat id; got {sent}"
    );

    // Slack wire: no SECOND tool attempt to Slack. A channel-origin run also
    // posts (and later retracts) an immediate "thinking" placeholder ahead of
    // the real reply — a separate, already-covered UX feature this journey
    // does not own — so assert on the ack CONTENT rather than the total
    // postMessage count: exactly one post must carry the run's own final
    // reply, and none may carry the tool's delivered content (which belongs
    // on the Telegram wire only).
    let posts: Vec<_> = requests
        .iter()
        .filter(|request| request.url.ends_with("/api/chat.postMessage"))
        .collect();
    let ack_posts: Vec<_> = posts
        .iter()
        .filter(|request| String::from_utf8_lossy(&request.body).contains(CROSS_SLACK_ACK))
        .collect();
    assert_eq!(
        ack_posts.len(),
        1,
        "exactly one slack post must carry the run's own final reply (the lane-1 echo); got {:?}",
        posts
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect::<Vec<_>>()
    );
    let posted = String::from_utf8_lossy(&ack_posts[0].body);
    assert!(
        posted.contains(&format!("\"channel\":\"{CROSS_SLACK_DM_CHANNEL}\"")),
        "the lane-1 echo must target the originating slack DM; got {posted}"
    );
    assert!(
        !posts
            .iter()
            .any(|request| String::from_utf8_lossy(&request.body).contains(CROSS_DELIVERED_TEXT)),
        "the tool's delivered content must never also reach the slack wire (no tool attempt to slack)"
    );

    // Attempt ledger: per-surface attribution — the explicit ModelDelivery
    // to Telegram and the automatic FinalReply to Slack.
    assert_delivered_attempts_by_kind(services, &vendor_scope, 1, 1).await;
}

/// Spec §13.4 — a partial failure across two deliver calls is reported
/// honestly, per call.
///
/// One turn calls `builtin.outbound_deliver` twice (a parallel tool-calls
/// turn): the first target succeeds, the second's scripted vendor API
/// rejects the message permanently. The successful call's result carries
/// provider refs; the failing call surfaces as a Failed tool error naming
/// the sanitized kind — never silently merged, dropped, or mistaken for the
/// other call's outcome.
#[test]
fn partial_failure_reports_per_call_honestly() {
    run_async_test_with_stack(
        "partial-failure-reports-per-call-honestly",
        partial_failure_reports_per_call_honestly_async,
    );
}

async fn partial_failure_reports_per_call_honestly_async() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    activate_slack(&group).await;
    let services = reborn_services(&group);
    assert!(
        services.register_static_channel_egress_credentials_for_test(vec![(
            "slack".to_string(),
            "slack_bot_token".to_string(),
            ironclaw_secrets::SecretMaterial::from(SLACK_BOT_TOKEN.to_string()),
        )]),
        "the composed runtime must expose channel-egress credential bridging"
    );

    let harness = group
        .thread("conv-partial-failure")
        .script([
            RebornScriptedReply::tool_calls([
                (
                    OUTBOUND_DELIVER,
                    json!({"target_id": PARTIAL_OK_TARGET_ID, "content": PARTIAL_OK_CONTENT}),
                ),
                (
                    OUTBOUND_DELIVER,
                    json!({"target_id": PARTIAL_FAILING_TARGET_ID, "content": PARTIAL_FAILING_CONTENT}),
                ),
            ]),
            RebornScriptedReply::text(PARTIAL_FAILURE_ACK),
        ])
        .build()
        .await
        .expect("partial-failure thread builds");
    group
        .register_source_delivery_target_for_test(
            "slack",
            PARTIAL_OK_TARGET_ID,
            slack_dm_binding_ref_for(&harness, PARTIAL_OK_DM_CHANNEL, PARTIAL_OK_DM_ACTOR),
        )
        .expect("succeeding slack DM target registers");
    group
        .register_source_delivery_target_for_test(
            "slack",
            PARTIAL_FAILING_TARGET_ID,
            slack_dm_binding_ref_for(
                &harness,
                PARTIAL_FAILING_DM_CHANNEL,
                PARTIAL_FAILING_DM_ACTOR,
            ),
        )
        .expect("failing slack DM target registers");

    harness
        .submit_turn("send the update to both my slack DMs")
        .await
        .expect("partial-failure turn completes");

    // First (succeeding) call: the recorded tool result carries the
    // vendor's refs — the ONLY completed capability result, since a Failed
    // dispatch is captured on the persisted ToolResultReference path, not
    // the in-process completed-result recorder (see `assert_tool_error`'s
    // doc). So "most recent recorded result" is unambiguously this call's.
    let output = harness
        .tool_result_output(OUTBOUND_DELIVER)
        .await
        .expect("outbound_deliver produced a recorded tool result for the succeeding call");
    assert_eq!(
        output["delivered"],
        json!(true),
        "the succeeding call must report delivery; got {output}"
    );
    assert_eq!(
        output["target_id"],
        json!(PARTIAL_OK_TARGET_ID),
        "the recorded result must be the SUCCEEDING call's, not the failing one's; got {output}"
    );
    assert_eq!(
        output["provider_message_refs"],
        json!([SLACK_VENDOR_TS]),
        "provider_message_refs must be exactly the recorded vendor response's refs; got {output}"
    );

    // Second (failing) call: a Failed tool error naming the sanitized kind
    // and the tool's own honest message.
    harness
        .assert_tool_error(ToolErrorClass::Failed, "operation_failed")
        .await
        .expect("the permanently-rejected leg must surface as a Failed tool error");
    harness
        .assert_tool_error(
            ToolErrorClass::Failed,
            "the delivery attempt failed: rejected",
        )
        .await
        .expect("the failed leg's summary must name the delivery-attempt failure honestly, including the sanitized DeliveryFailureKind (spec §5)");

    // Wire: both targets actually reached the vendor — the failure is a
    // real, terminal vendor rejection, not an internal short-circuit.
    let requests = harness.captured_network_requests_for_test();
    let posts: Vec<_> = requests
        .iter()
        .filter(|request| request.url.ends_with("/api/chat.postMessage"))
        .collect();
    assert_eq!(
        posts.len(),
        2,
        "both deliver calls must reach the vendor wire; got {:?}",
        requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
    );

    // Ledger: one Delivered + one Failed{Rejected} ModelDelivery attempt.
    assert_partial_failure_attempts(services, &harness.turn_scope).await;

    // The scripted final text acknowledges the failed leg.
    harness
        .assert_reply_contains(PARTIAL_FAILURE_ACK)
        .await
        .expect("the run's own final reply lands and acknowledges the failed leg");
}

/// Run the deep parallel-delivery harness on a larger-than-default OS stack.
/// The production decorator chain exercised here can exceed Rust's 2 MiB test
/// thread stack before reaching the assertions; sibling integration suites use
/// this same current-thread runtime wrapper for those deep production paths.
fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}

/// Spec §13.8 — an undeliverable destination is refused without any tool
/// call, never silently routed elsewhere.
///
/// Against a catalog with NO Gmail/email surface, "email me the report"
/// makes zero `builtin.outbound_deliver` calls and zero `builtin.trigger_create`
/// calls, delivers nothing, and the model explains why — pinned at both the
/// trace (tool-invocation) and ledger (delivery-attempt) seams.
#[tokio::test(flavor = "multi_thread")]
async fn undeliverable_destination_is_refused_without_tool_calls() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let services = reborn_services(&group);
    let harness = group
        .thread("conv-undeliverable-destination")
        .script([RebornScriptedReply::text(UNDELIVERABLE_EXPLANATION)])
        .build()
        .await
        .expect("undeliverable-destination thread builds");

    harness
        .submit_turn(UNDELIVERABLE_REQUEST)
        .await
        .expect("undeliverable-destination turn completes");

    harness
        .assert_tool_not_invoked(OUTBOUND_DELIVER)
        .await
        .expect("no outbound_deliver call may be made for an undeliverable destination");
    harness
        .assert_tool_not_invoked("builtin.trigger_create")
        .await
        .expect("no trigger_create call may be made for an undeliverable destination");
    assert_no_delivery_attempts(services, &harness.turn_scope).await;
    harness
        .assert_reply_contains(UNDELIVERABLE_EXPLANATION)
        .await
        .expect("the model's explanatory reply must land");
}

// ── Routine-fire journeys (spec §13.5) ─────────────────────────────────────

const ROUTINE_PROMPT: &str =
    "every morning, check the deploy board and send me the digest on slack";
const ROUTINE_DELIVERED_TEXT: &str = "Deploy board is green across all services.";
const ROUTINE_FIRE_ACK: &str = "Sent the deploy digest to your Slack DM.";
const CONDITIONAL_PROMPT: &str = "ping me on slack only if the deploy board is red";
const CONDITIONAL_FIRE_REPLY: &str = "Deploy board is green, so there is nothing to send.";

/// The background-run notifier over the composed runtime's real coordinator
/// and catalog, driven directly the way the composition's post-submit hook
/// drives it after a settled fire.
fn background_run_notifier(
    harness: &RebornIntegrationHarness,
    services: &RebornRuntime,
) -> ironclaw_assistant::TriggeredRunDeliveryDriver {
    ironclaw_assistant::TriggeredRunDeliveryDriver::with_settings(
        slack_run_delivery_services(harness, services),
        ironclaw_assistant::RunDeliverySettings {
            poll_interval: Duration::from_millis(5),
            max_wait: Duration::from_secs(10),
            max_concurrent_deliveries: std::num::NonZeroUsize::new(4).expect("non-zero"),
            max_pending_deliveries: std::num::NonZeroUsize::new(8).expect("non-zero"),
            first_nudge_after: Duration::from_secs(3600),
            renudge_interval: Duration::from_secs(3600),
        },
        services
            .triggered_run_delivery_store_for_test()
            .expect("composed runtime exposes the triggered-delivery outcome store"),
        Arc::new(vec![
            Arc::new(ironclaw_slack_extension::SlackPreferenceTargetCodec)
                as Arc<dyn PreferenceTargetCodec>,
        ]) as Arc<dyn ActivePreferenceTargetCodecs>,
        slack_agent(),
    )
}

/// The notifier request the composition's post-submit hook builds from a fire.
/// Note what it does NOT carry: there is no per-trigger delivery target field
/// at all any more (spec §8).
fn notifier_request(
    harness: &RebornIntegrationHarness,
    submission: &reborn_support::triggered_submit::TriggeredSubmission,
    prompt: &str,
) -> ironclaw_outbound::TriggeredRunDeliveryRequest {
    ironclaw_outbound::TriggeredRunDeliveryRequest {
        run_id: submission.run_id,
        scope: submission.turn_scope.clone(),
        creator_user_id: harness.binding.actor_user_id.clone(),
        // A personal routine: `TriggerFire::project_id` is the TRIGGER's own
        // project, not the harness binding's capability-scoping project. A
        // project-scoped fire is denied outright by a separate fail-closed
        // rule, pinned at the crate tier.
        project_scoped: false,
        prompt: prompt.to_string(),
    }
}

/// Bounded-poll the notifier's durable outcome for one run.
async fn wait_for_notifier_outcome(
    services: &RebornRuntime,
    run_id: TurnRunId,
) -> ironclaw_outbound::TriggeredRunDeliveryOutcomeKind {
    let store = services
        .triggered_run_delivery_store_for_test()
        .expect("composed runtime exposes the triggered-delivery outcome store");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(record) = store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load triggered delivery outcome")
        {
            return record.outcome;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "background run notifier recorded no outcome for {run_id}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Seed the fire creator's notification-channel set — the surface a background
/// run's gate/auth/failure notices fan out over. Present in both routine
/// journeys so "nothing was delivered" is a discriminating result: a channel
/// WAS configured, and the run still pushed nothing.
async fn seed_notification_channel(
    harness: &RebornIntegrationHarness,
    services: &RebornRuntime,
    target_id: &str,
) {
    let preferences = services
        .standalone_outbound_preferences_for_test()
        .expect("composed runtime exposes outbound preferences");
    preferences
        .write_communication_preference(ironclaw_outbound::WriteCommunicationPreferenceRequest {
            expected_version: None,
            record: ironclaw_outbound::CommunicationPreferenceRecord {
                scope: ironclaw_outbound::DeliveryDefaultScope::personal(
                    harness.binding.tenant_id.clone(),
                    harness.binding.actor_user_id.clone(),
                ),
                legacy_notification_target: None,
                default_modality: None,
                notification_targets: vec![
                    ironclaw_outbound::OutboundDeliveryTargetId::new(target_id)
                        .expect("notification channel id"),
                ],
                updated_at: Utc::now(),
                updated_by: harness.binding.actor_user_id.clone(),
            },
        })
        .await
        .expect("seed the creator's notification channel");
}

/// §13.5 (first half): a routine fire delivers by CALLING the tool — there is
/// no stored per-trigger destination anywhere in the path. Both delivery
/// seams are asserted, and the background-run notifier running alongside
/// contributes NOTHING: the result push is gone (spec §8).
#[tokio::test]
async fn routine_fire_delivers_via_tool_without_stored_target() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    activate_slack(&group).await;
    let services = reborn_services(&group);
    assert!(
        services.register_static_channel_egress_credentials_for_test(vec![(
            "slack".to_string(),
            "slack_bot_token".to_string(),
            ironclaw_secrets::SecretMaterial::from(SLACK_BOT_TOKEN.to_string()),
        )]),
        "the slack bot credential bridge must be registered"
    );

    let harness = group
        .thread("conv-routine-fire-delivers")
        .build()
        .await
        .expect("routine fire thread builds");
    group
        .register_source_delivery_target_for_test(
            "slack",
            SLACK_DM_TARGET_ID,
            slack_dm_binding_ref(&harness),
        )
        .expect("slack DM target registers on the caller-owned registry");
    seed_notification_channel(&harness, services, SLACK_DM_TARGET_ID).await;

    let submission = harness
        .submit_triggered_turn_scripted(
            ROUTINE_PROMPT,
            [
                RebornScriptedReply::tool_call(
                    OUTBOUND_DELIVER,
                    json!({"target_id": SLACK_DM_TARGET_ID, "content": ROUTINE_DELIVERED_TEXT}),
                ),
                RebornScriptedReply::text(ROUTINE_FIRE_ACK),
            ],
        )
        .await
        .expect("routine fire submits through the real trusted-trigger submitter");

    // The notifier watches the fire exactly as the composition hook does.
    let notifier = background_run_notifier(&harness, services);
    notifier
        .on_trigger_submitted(notifier_request(&harness, &submission, ROUTINE_PROMPT))
        .await;

    wait_for_run_status_in_scope(
        &harness.turn_coordinator_for_test(),
        &submission.turn_scope,
        submission.run_id,
        TurnStatus::Completed,
    )
    .await;

    // Seam 1 — the vendor wire: exactly one bot-authenticated send.
    let posts: Vec<_> = harness
        .captured_network_requests_for_test()
        .into_iter()
        .filter(|request| request.url.ends_with("/api/chat.postMessage"))
        .collect();
    assert_eq!(
        posts.len(),
        1,
        "the fire's ONLY external send is its explicit tool call; got {posts:?}"
    );
    let body = String::from_utf8_lossy(&posts[0].body).into_owned();
    assert!(
        body.contains(ROUTINE_DELIVERED_TEXT),
        "the delivered content must reach the vendor: {body}"
    );

    // Seam 2 — the attempt ledger: one terminal `ModelDelivery` attempt, and
    // nothing else. A pushed result would show up here as a second row.
    assert_model_delivery_attempts(services, &submission.turn_scope, 1).await;

    // The notifier deliberately had nothing to say about a completed run.
    assert_eq!(
        wait_for_notifier_outcome(services, submission.run_id).await,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Skipped,
        "a completed background run must record no delivery of its own"
    );
}

/// §13.5 (second half): a conditional routine whose fire decides NOT to
/// deliver puts nothing on any channel. The fire's answer still lives in its
/// own run thread — that is the whole record.
#[tokio::test]
async fn conditional_fire_with_no_delivery_call_produces_zero_attempts() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    activate_slack(&group).await;
    let services = reborn_services(&group);

    let harness = group
        .thread("conv-conditional-fire-no-delivery")
        .build()
        .await
        .expect("conditional fire thread builds");
    group
        .register_source_delivery_target_for_test(
            "slack",
            SLACK_DM_TARGET_ID,
            slack_dm_binding_ref(&harness),
        )
        .expect("slack DM target registers on the caller-owned registry");
    seed_notification_channel(&harness, services, SLACK_DM_TARGET_ID).await;

    let submission = harness
        .submit_triggered_turn_scripted(
            CONDITIONAL_PROMPT,
            [RebornScriptedReply::text(CONDITIONAL_FIRE_REPLY)],
        )
        .await
        .expect("conditional fire submits");

    let notifier = background_run_notifier(&harness, services);
    notifier
        .on_trigger_submitted(notifier_request(&harness, &submission, CONDITIONAL_PROMPT))
        .await;

    wait_for_run_status_in_scope(
        &harness.turn_coordinator_for_test(),
        &submission.turn_scope,
        submission.run_id,
        TurnStatus::Completed,
    )
    .await;
    assert_eq!(
        wait_for_notifier_outcome(services, submission.run_id).await,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Skipped
    );

    assert!(
        harness
            .captured_network_requests_for_test()
            .iter()
            .all(|request| !request.url.ends_with("/api/chat.postMessage")),
        "a fire that makes no delivery call must not reach any vendor send"
    );
    assert_no_delivery_attempts(services, &submission.turn_scope).await;
}

// ── Blocked-fire fan-out journeys (spec §13.6-7) ───────────────────────────

const FANOUT_DM_A_CHANNEL: &str = "D-FANOUT-A";
const FANOUT_DM_A_ACTOR: &str = "U-FANOUT-A";
const FANOUT_DM_A_TARGET_ID: &str = "slack:fanout-dm-a";
const FANOUT_DM_B_CHANNEL: &str = "D-FANOUT-B";
const FANOUT_DM_B_ACTOR: &str = "U-FANOUT-B";
const FANOUT_DM_B_TARGET_ID: &str = "slack:fanout-dm-b";
const GATED_FIRE_PROMPT: &str = "every night, write the deploy report";
const GATED_FIRE_REPLY: &str = "Report written.";

/// Register a Slack DM as BOTH a catalog target and one of the creator's
/// notification channels.
async fn register_notification_channels(
    group: &RebornIntegrationGroup,
    harness: &RebornIntegrationHarness,
    services: &RebornRuntime,
    channels: &[(&str, &str, &str)],
) {
    for (target_id, dm_channel, dm_actor) in channels {
        group
            .register_source_delivery_target_for_test(
                "slack",
                target_id,
                slack_dm_binding_ref_for(harness, dm_channel, dm_actor),
            )
            .expect("slack DM target registers on the caller-owned registry");
    }
    let preferences = services
        .standalone_outbound_preferences_for_test()
        .expect("composed runtime exposes outbound preferences");
    preferences
        .write_communication_preference(ironclaw_outbound::WriteCommunicationPreferenceRequest {
            expected_version: None,
            record: ironclaw_outbound::CommunicationPreferenceRecord {
                scope: ironclaw_outbound::DeliveryDefaultScope::personal(
                    harness.binding.tenant_id.clone(),
                    harness.binding.actor_user_id.clone(),
                ),
                legacy_notification_target: None,
                default_modality: None,
                notification_targets: channels
                    .iter()
                    .map(|(target_id, _, _)| {
                        ironclaw_outbound::OutboundDeliveryTargetId::new(*target_id)
                            .expect("notification channel id")
                    })
                    .collect(),
                updated_at: Utc::now(),
                updated_by: harness.binding.actor_user_id.clone(),
            },
        })
        .await
        .expect("seed the creator's notification channels");
}

/// Gate `builtin.write_file` for this run owner only. `AskEachTime` beats
/// global auto-approve (#4776 precedence), so the delivery/lifecycle verbs
/// keep dispatching gate-free while the scripted write raises a REAL gate.
async fn gate_the_write(group: &RebornIntegrationGroup, harness: &RebornIntegrationHarness) {
    group
        .capability_harness()
        .expect("delivery-with-gated-write always uses HostRuntime")
        .set_ask_each_time_override_for_test(
            &ironclaw_host_api::ids::CapabilityId::new("builtin.write_file")
                .expect("write_file capability id"),
            harness.binding.tenant_id.clone(),
            harness.binding.actor_user_id.clone(),
        )
        .await
        .expect("install the AskEachTime override");
}

/// Every Slack `chat.postMessage` body recorded so far, paired with the
/// channel it targeted.
fn slack_posts_by_channel(harness: &RebornIntegrationHarness) -> Vec<(String, String)> {
    harness
        .captured_network_requests_for_test()
        .into_iter()
        .filter(|request| request.url.ends_with("/api/chat.postMessage"))
        .filter_map(|request| {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null);
            let channel = body["channel"].as_str()?.to_string();
            let text = body["text"].as_str().unwrap_or_default().to_string();
            Some((channel, text))
        })
        .collect()
}

/// Bounded-poll until `expected` approval prompts have reached the Slack wire.
async fn wait_for_gate_prompts(
    harness: &RebornIntegrationHarness,
    expected: usize,
) -> Vec<(String, String)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let prompts: Vec<_> = slack_posts_by_channel(harness)
            .into_iter()
            .filter(|(_, text)| text.contains("Approval needed"))
            .collect();
        if prompts.len() >= expected {
            return prompts;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {expected} gate prompt(s); got {prompts:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// §13.6 — a blocked routine fire reaches EVERY notification channel, and the
/// first approve wins.
///
/// Two Slack DMs are configured as notification channels. The fire parks on a
/// real approval gate; the gate prompt lands in BOTH DMs through the real
/// adapter + host bot credential. Approving from one resumes the run to
/// completion; a second approve for the same gate is refused — gate
/// resolution is one-shot, so whichever channel the user reaches first wins.
#[tokio::test(flavor = "multi_thread")]
async fn blocked_fire_fans_out_and_first_approve_wins() {
    let group = RebornIntegrationGroup::extension_delivery_with_gated_write()
        .await
        .expect("delivery-with-gated-write group builds");
    activate_slack(&group).await;
    let services = reborn_services(&group);
    assert!(
        services.register_static_channel_egress_credentials_for_test(vec![(
            "slack".to_string(),
            "slack_bot_token".to_string(),
            ironclaw_secrets::SecretMaterial::from(SLACK_BOT_TOKEN.to_string()),
        )]),
        "the slack bot credential bridge must be registered"
    );

    let harness = group
        .thread("conv-blocked-fire-fanout")
        .build()
        .await
        .expect("blocked fire thread builds");
    register_notification_channels(
        &group,
        &harness,
        services,
        &[
            (
                FANOUT_DM_A_TARGET_ID,
                FANOUT_DM_A_CHANNEL,
                FANOUT_DM_A_ACTOR,
            ),
            (
                FANOUT_DM_B_TARGET_ID,
                FANOUT_DM_B_CHANNEL,
                FANOUT_DM_B_ACTOR,
            ),
        ],
    )
    .await;
    gate_the_write(&group, &harness).await;

    let submission = harness
        .submit_triggered_turn_scripted(
            GATED_FIRE_PROMPT,
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/fanout-report.txt", "content": "nightly deploy report"}),
                ),
                RebornScriptedReply::text(GATED_FIRE_REPLY),
            ],
        )
        .await
        .expect("gated fire submits");

    let notifier = background_run_notifier(&harness, services);
    notifier
        .on_trigger_submitted(notifier_request(&harness, &submission, GATED_FIRE_PROMPT))
        .await;

    let blocked = harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await
        .expect("the gated fire parks on a real approval gate");
    let gate_ref = blocked
        .gate_ref
        .expect("blocked triggered run carries a gate ref");

    // Fan-out seam: BOTH notification channels received the prompt.
    let prompts = wait_for_gate_prompts(&harness, 2).await;
    assert_eq!(prompts.len(), 2, "one gate prompt per channel: {prompts:?}");
    let mut prompt_channels: Vec<_> = prompts.iter().map(|(channel, _)| channel.clone()).collect();
    prompt_channels.sort();
    assert_eq!(
        prompt_channels,
        vec![
            FANOUT_DM_A_CHANNEL.to_string(),
            FANOUT_DM_B_CHANNEL.to_string()
        ],
        "the gate prompt must reach every configured notification channel"
    );
    assert!(
        prompts
            .iter()
            .all(|(_, text)| text.contains(gate_ref.as_str())),
        "each prompt names the gate it resolves: {prompts:?}"
    );

    // First approve wins: the run resumes and the gated write lands.
    harness
        .approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref)
        .await
        .expect("approving from one notification channel resumes the run");
    harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Completed,
        )
        .await
        .expect("the resumed fire completes");
    harness
        .assert_workspace_file_contains("fanout-report.txt", "nightly deploy report")
        .await
        .expect("the approved write actually executed");

    // Second approve — from the other channel — is refused: gate resolution
    // is one-shot, so the run is already resolved.
    let second = harness
        .approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref)
        .await;
    let error = second.expect_err(
        "a second approve for an already-resolved gate must be refused, not silently re-applied",
    );
    assert!(
        error
            .to_string()
            .contains("approval request is not pending")
            && error.to_string().contains("Approved"),
        "the second approve must report the gate as already resolved, got: {error}"
    );
}

/// §13.7 — with no notification channels configured, a blocked fire stays in
/// the app: zero external attempts, and the hold is still visible on the run.
#[tokio::test(flavor = "multi_thread")]
async fn empty_notification_set_keeps_blocked_fire_in_app_only() {
    let group = RebornIntegrationGroup::extension_delivery_with_gated_write()
        .await
        .expect("delivery-with-gated-write group builds");
    activate_slack(&group).await;
    let services = reborn_services(&group);

    let harness = group
        .thread("conv-blocked-fire-in-app-only")
        .build()
        .await
        .expect("blocked fire thread builds");
    // A catalog target EXISTS — the creator simply selected none of them, so
    // "nothing was delivered" is a discriminating result rather than a
    // consequence of having nowhere to deliver.
    group
        .register_source_delivery_target_for_test(
            "slack",
            FANOUT_DM_A_TARGET_ID,
            slack_dm_binding_ref_for(&harness, FANOUT_DM_A_CHANNEL, FANOUT_DM_A_ACTOR),
        )
        .expect("slack DM target registers on the caller-owned registry");
    gate_the_write(&group, &harness).await;

    let submission = harness
        .submit_triggered_turn_scripted(
            GATED_FIRE_PROMPT,
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/in-app-only.txt", "content": "nightly deploy report"}),
                ),
                RebornScriptedReply::text(GATED_FIRE_REPLY),
            ],
        )
        .await
        .expect("gated fire submits");

    let notifier = background_run_notifier(&harness, services);
    notifier
        .on_trigger_submitted(notifier_request(&harness, &submission, GATED_FIRE_PROMPT))
        .await;

    harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await
        .expect("the gated fire parks on a real approval gate");

    // The notifier settled with nothing to notify.
    assert_eq!(
        wait_for_notifier_outcome(services, submission.run_id).await,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured
    );
    assert!(
        slack_posts_by_channel(&harness).is_empty(),
        "no notification channel is configured, so nothing may reach a vendor: {:?}",
        slack_posts_by_channel(&harness)
    );
    assert_no_delivery_attempts(services, &submission.turn_scope).await;

    // The hold is still the surface: the run is untouched and still parked on
    // its gate, so the automations panel and in-app gate UI can act on it.
    let still_blocked = harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await
        .expect("the run must still be parked, not cancelled");
    assert!(
        still_blocked.gate_ref.is_some(),
        "the hold keeps its actionable gate ref"
    );
}

// ── Web-push notice journeys (browser channel) ─────────────────────────────

/// Push endpoints on the web-app manifest's declared FCM host. The reserved
/// `gone-subscription-token` suffix mirrors
/// `harness/profiles/extension.rs::WEB_APP_GONE_ENDPOINT_TOKEN`: the
/// profile's vendor router answers it `410 Gone` (every other push POST gets
/// `201 Created`).
const WEB_APP_LIVE_ENDPOINT: &str = "https://fcm.googleapis.com/fcm/send/live-subscription-token";
const WEB_APP_GONE_ENDPOINT: &str = "https://fcm.googleapis.com/fcm/send/gone-subscription-token";
/// The browser channel's catalog target id. Deliberately the pre-rename
/// `web-push` bytes: it is a persisted per-user preference identity (see
/// `ironclaw_web_app::WEB_APP_TARGET_ID`), unlike the `web-app` channel name.
const WEB_APP_TARGET_ID: &str = "web-push";

/// Exact-destination + unthreaded delivery evidence for the journey coverage
/// gate (`tests/e2e/scenarios/test_journey_coverage.py`
/// `_assert_delivery_address_is_citable`). Web push addresses a per-browser
/// endpoint capability URL, not a threaded conversation — the endpoint IS the
/// destination and there is no thread anchor. The gate greps this helper for
/// `expected_conversation_id`/`expected_thread_anchor` gating the count.
fn assert_web_app_delivery_evidence(posts: &[ironclaw_network::NetworkHttpRequest]) {
    // Literal (not the `WEB_APP_LIVE_ENDPOINT` const) because the journey
    // coverage gate greps this body for the exact destination string.
    let expected_conversation_id = "https://fcm.googleapis.com/fcm/send/live-subscription-token";
    let expected_thread_anchor: Option<&str> = None;
    let expected_count = 1;
    let matching = posts.iter().filter(|post| {
        // The endpoint carries no in-URL thread segment; browser push has no
        // threading, so the anchor is unconditionally absent.
        let thread_anchor: Option<&str> = None;
        post.url == expected_conversation_id && thread_anchor == expected_thread_anchor
    });
    assert_eq!(
        matching.count(),
        expected_count,
        "exactly one unthreaded push POST must reach the enrolled endpoint; got {:?}",
        posts
            .iter()
            .map(|post| post.url.clone())
            .collect::<Vec<_>>()
    );
}

/// Enroll one browser for the harness creator through the REAL WebUI route
/// (`POST /api/webchat/v2/channels/web-app/notifications/enable`, the
/// generic notification-setup surface) over the composed runtime's
/// production product surface — the same path the browser panel drives.
async fn enroll_web_app_browser(
    harness: &RebornIntegrationHarness,
    services: &RebornRuntime,
    endpoint: &str,
) {
    use base64::Engine as _;
    let webui = services
        .product_surface(None)
        .expect("composed runtime builds the production product surface");
    // The enrollment owner must be the fire CREATOR (the notifier resolves
    // notification targets for `creator_user_id` = the binding actor).
    let caller = ironclaw_product_contracts::surface::ProductSurfaceCaller::new(
        harness.binding.tenant_id.clone(),
        harness.binding.actor_user_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
    );
    let point = ironclaw_web_app::generate_vapid_key_material("mailto:browser@example.com")
        .expect("generate a valid P-256 point")
        .public_key_b64url;
    let (status, body) = reborn_support::webui_mount::post_json(
        reborn_support::webui_mount::mount_webui_v2_router(webui, caller),
        "/api/webchat/v2/channels/web-app/notifications/enable",
        json!({
            "payload": {
                "endpoint": endpoint,
                "keys": {
                    "p256dh": point,
                    "auth": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 16]),
                },
                "user_agent": "JourneyBrowser/1.0",
            },
        }),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "enroll response: {body}"
    );
    assert_eq!(body["enabled"], true, "{body}");
    assert_eq!(body["extension_id"], "web-app", "{body}");
    assert_eq!(body["detail"]["registration_count"], 1, "{body}");
}

/// The web-app notifier: [`background_run_notifier`]'s services with the
/// browser channel's codec beside Slack's, so the creator's `web-app`
/// notification target decodes.
fn web_app_background_run_notifier(
    harness: &RebornIntegrationHarness,
    services: &RebornRuntime,
) -> ironclaw_assistant::TriggeredRunDeliveryDriver {
    ironclaw_assistant::TriggeredRunDeliveryDriver::with_settings(
        slack_run_delivery_services(harness, services),
        ironclaw_assistant::RunDeliverySettings {
            poll_interval: Duration::from_millis(5),
            max_wait: Duration::from_secs(10),
            max_concurrent_deliveries: std::num::NonZeroUsize::new(4).expect("non-zero"),
            max_pending_deliveries: std::num::NonZeroUsize::new(8).expect("non-zero"),
            first_nudge_after: Duration::from_secs(3600),
            renudge_interval: Duration::from_secs(3600),
        },
        services
            .triggered_run_delivery_store_for_test()
            .expect("composed runtime exposes the triggered-delivery outcome store"),
        Arc::new(vec![
            Arc::new(ironclaw_slack_extension::SlackPreferenceTargetCodec)
                as Arc<dyn PreferenceTargetCodec>,
            Arc::new(ironclaw_web_app_extension::WebAppPreferenceTargetCodec)
                as Arc<dyn PreferenceTargetCodec>,
        ]) as Arc<dyn ActivePreferenceTargetCodecs>,
        slack_agent(),
    )
}

/// Bounded-poll the wire recorder for push-service POSTs to `endpoint`.
async fn wait_for_push_posts(
    harness: &RebornIntegrationHarness,
    endpoint: &str,
    expected: usize,
) -> Vec<ironclaw_network::NetworkHttpRequest> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let posts: Vec<_> = harness
            .captured_network_requests_for_test()
            .into_iter()
            .filter(|request| request.url.starts_with(endpoint))
            .collect();
        if posts.len() >= expected {
            return posts;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {expected} push POST(s) to {endpoint}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The browser subscriptions currently enrolled for the harness creator,
/// read back through the SAME production status route the panel uses.
async fn web_app_subscription_count(
    harness: &RebornIntegrationHarness,
    services: &RebornRuntime,
) -> u64 {
    let webui = services
        .product_surface(None)
        .expect("composed runtime builds the production product surface");
    let caller = ironclaw_product_contracts::surface::ProductSurfaceCaller::new(
        harness.binding.tenant_id.clone(),
        harness.binding.actor_user_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
    );
    let (status, body) = reborn_support::webui_mount::get_json(
        reborn_support::webui_mount::mount_webui_v2_router(webui, caller),
        "/api/webchat/v2/channels/web-app/notifications",
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "status response: {body}"
    );
    body["detail"]["registration_count"]
        .as_u64()
        .expect("status detail carries registration_count")
}

/// A blocked routine fire's gate notice reaches an enrolled browser as a
/// real Web Push: encrypted `aes128gcm` body, host-injected
/// `Authorization: vapid` (the adapter cannot set that header), protocol
/// headers, one POST to the exact enrolled endpoint — while the run parks on
/// its gate and the notice lands in the durable attempt ledger.
#[tokio::test(flavor = "multi_thread")]
async fn blocked_fire_pushes_web_app_notice_to_enrolled_browser() {
    let group = RebornIntegrationGroup::extension_delivery_with_web_app()
        .await
        .expect("delivery-with-web-app group builds");
    let services = reborn_services(&group);
    let harness = group
        .thread("conv-web-app-notice")
        .build()
        .await
        .expect("web-app notice thread builds");

    enroll_web_app_browser(&harness, services, WEB_APP_LIVE_ENDPOINT).await;
    seed_notification_channel(&harness, services, WEB_APP_TARGET_ID).await;
    gate_the_write(&group, &harness).await;

    let submission = harness
        .submit_triggered_turn_scripted(
            GATED_FIRE_PROMPT,
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/web-app-report.txt", "content": "nightly deploy report"}),
                ),
                RebornScriptedReply::text(GATED_FIRE_REPLY),
            ],
        )
        .await
        .expect("gated fire submits");

    let notifier = web_app_background_run_notifier(&harness, services);
    notifier
        .on_trigger_submitted(notifier_request(&harness, &submission, GATED_FIRE_PROMPT))
        .await;

    let blocked = harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await
        .expect("the gated fire parks on a real approval gate");
    let gate_ref = blocked
        .gate_ref
        .expect("blocked triggered run carries a gate ref");

    // Wire seam: exactly one push POST to the enrolled endpoint, carrying the
    // host-injected VAPID authorization and the RFC 8188/8291 framing.
    let posts = wait_for_push_posts(&harness, WEB_APP_LIVE_ENDPOINT, 1).await;
    // Exact-destination + unthreaded evidence for the journey coverage gate
    // (`tests/e2e/scenarios/test_journey_coverage.py`).
    assert_web_app_delivery_evidence(&posts);
    assert_eq!(posts.len(), 1, "one enrolled browser, one push POST");
    let post = &posts[0];
    let header = |name: &str| {
        post.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    let authorization = header("authorization")
        .expect("host-side VAPID injection must add the authorization header");
    assert!(
        authorization.starts_with("vapid t="),
        "RFC 8292 vapid scheme, host-computed: {authorization}"
    );
    assert!(
        authorization.contains(", k="),
        "the advertised application-server key rides the header: {authorization}"
    );
    assert_eq!(
        header("content-encoding").as_deref(),
        Some("aes128gcm"),
        "RFC 8291 content encoding"
    );
    assert!(header("ttl").is_some(), "push TTL header present");
    assert!(
        post.body.len() >= 87,
        "aes128gcm header + at least one sealed byte; got {}",
        post.body.len()
    );
    assert_eq!(
        post.body[20], 65,
        "RFC 8188 idlen = uncompressed P-256 point"
    );

    // Durable seam: the notice landed in the attempt ledger as a delivered
    // gate prompt, and the run is STILL parked on its gate.
    let (outbound_store, _, _, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("outbound stores");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let delivered_gate_prompts = loop {
        let attempts = outbound_store
            .list_delivery_attempts(submission.turn_scope.clone())
            .await
            .expect("list delivery attempts");
        let delivered = attempts
            .iter()
            .filter(|attempt| {
                attempt.status == OutboundDeliveryStatus::Delivered
                    && attempt.candidate.kind == OutboundPushKind::GateRequired
            })
            .count();
        if delivered >= 1 {
            break delivered;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no delivered gate-prompt attempt recorded; got {:?}",
            attempts
                .iter()
                .map(|attempt| (attempt.status, attempt.candidate.kind))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(delivered_gate_prompts, 1, "one browser notice delivered");
    let still_blocked = harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await
        .expect("the push notice must not resolve the gate");
    assert!(still_blocked.gate_ref.is_some(), "gate still pending");

    // Settle the group: approve and let the fire finish.
    harness
        .approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref)
        .await
        .expect("approving resumes the run");
    harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Completed,
        )
        .await
        .expect("the resumed fire completes");
}

/// A push service answering `410 Gone` prunes the dead subscription: the
/// notice attempt reaches the wire once, then the browser disappears from
/// the caller's enrollment set so future sends stop trying.
#[tokio::test(flavor = "multi_thread")]
async fn gone_push_subscription_is_pruned_after_notice_attempt() {
    let group = RebornIntegrationGroup::extension_delivery_with_web_app()
        .await
        .expect("delivery-with-web-app group builds");
    let services = reborn_services(&group);
    let harness = group
        .thread("conv-web-app-prune")
        .build()
        .await
        .expect("web-app prune thread builds");

    enroll_web_app_browser(&harness, services, WEB_APP_GONE_ENDPOINT).await;
    assert_eq!(web_app_subscription_count(&harness, services).await, 1);
    seed_notification_channel(&harness, services, WEB_APP_TARGET_ID).await;
    gate_the_write(&group, &harness).await;

    let submission = harness
        .submit_triggered_turn_scripted(
            GATED_FIRE_PROMPT,
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/web-app-prune.txt", "content": "nightly deploy report"}),
                ),
                RebornScriptedReply::text(GATED_FIRE_REPLY),
            ],
        )
        .await
        .expect("gated fire submits");

    let notifier = web_app_background_run_notifier(&harness, services);
    notifier
        .on_trigger_submitted(notifier_request(&harness, &submission, GATED_FIRE_PROMPT))
        .await;

    let blocked = harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await
        .expect("the gated fire parks on a real approval gate");

    // The dead endpoint was attempted once, answered 410, and pruned.
    let posts = wait_for_push_posts(&harness, WEB_APP_GONE_ENDPOINT, 1).await;
    assert_eq!(posts.len(), 1, "the dead subscription is attempted once");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if web_app_subscription_count(&harness, services).await == 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the 410 subscription was never pruned"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Settle the group.
    let gate_ref = blocked.gate_ref.expect("gate ref");
    harness
        .approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref)
        .await
        .expect("approving resumes the run");
    harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Completed,
        )
        .await
        .expect("the resumed fire completes");
}
