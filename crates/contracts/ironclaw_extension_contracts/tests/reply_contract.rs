//! The progressive-reply vocabulary contract
//! (`docs/internal/design/2026-08-31-progressive-reply-publication.md` §3, §6).
//!
//! What a reply sink may ever see is a bounded, typed desired-state document:
//! these tests pin the bounds (by construction, including through serde), the
//! document's mutator semantics every producer and sink rely on, the cadence
//! each transport admits, and the single-seam binding shape
//! (`ChannelSurfaces.reply` is one `ReplySink`, whatever the transport).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel::{ChannelDescriptor, ReplyTransport};
use ironclaw_extension_contracts::channel_adapter::{ChannelError, ChannelSurfaces};
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::reply::{
    REPLY_ANSWER_MAX_BYTES, REPLY_CONTEXT_MAX_BYTES, REPLY_DISPLAY_PREVIEW_MAX_BYTES,
    REPLY_DISPLAY_TEXT_MAX_BYTES, REPLY_MAX_ACTIVITIES, REPLY_MAX_PROVIDER_REFS,
    REPLY_OUTCOME_REASON_MAX_BYTES, REPLY_REASONING_SEGMENT_MAX_BYTES,
    REPLY_SINK_CHECKPOINT_MAX_BYTES, REPLY_THREAD_ANCHOR_MAX_BYTES, ReplyActivityState,
    ReplyAnswerText, ReplyAttention, ReplyAttentionKind, ReplyAudience, ReplyContextBytes,
    ReplyContractError, ReplyDisplayPreview, ReplyDisplayText, ReplyDocument, ReplyItemId,
    ReplyOutcome, ReplyOutcomeReason, ReplyPhase, ReplyProviderRef, ReplyProviderRefs,
    ReplyReasoningText, ReplyReconcilePoint, ReplyReconcileRequest, ReplyRevision, ReplySink,
    ReplySinkCheckpoint, ReplySinkEvidence, ReplySinkOutcome, ReplySinkReport, ReplyTarget,
    ReplyThreadAnchor,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope};

fn text(value: &str) -> ReplyDisplayText {
    ReplyDisplayText::new(value).expect("display text")
}

fn item(value: &str) -> ReplyItemId {
    ReplyItemId::new(value).expect("item id")
}

fn answer(value: &str) -> ReplyAnswerText {
    ReplyAnswerText::new(value).expect("answer text")
}

fn scope() -> TurnScope {
    TurnScope::new_with_owner(
        TenantId::new("tenant-a").expect("tenant"),
        None,
        None,
        ThreadId::new("thread-1").expect("thread"),
        Some(UserId::new("user-1").expect("user")),
    )
}

fn target(audience: ReplyAudience) -> ReplyTarget {
    ReplyTarget {
        scope: scope(),
        actor: TurnActor::new(UserId::new("user-1").expect("user")),
        run_id: TurnRunId::new(),
        conversation: Some(
            ExternalConversationRef::new(Some("T1"), "C1", Some("1717.1"), Some("1717.2"))
                .expect("conversation"),
        ),
        thread_anchor: Some(ReplyThreadAnchor::new("1717.1").expect("anchor")),
        audience,
    }
}

// ── Bounds: what the seam can and cannot represent ────────────────────────

#[test]
fn display_text_is_byte_bounded_and_rejects_control_characters() {
    assert!(ReplyDisplayText::new("Reading the runbook\nline two\tindented").is_ok());
    assert!(matches!(
        ReplyDisplayText::new("x".repeat(REPLY_DISPLAY_TEXT_MAX_BYTES + 1)),
        Err(ReplyContractError::TextTooLong { .. })
    ));
    assert!(matches!(
        ReplyDisplayText::new("escape\u{1b}[31m"),
        Err(ReplyContractError::ControlCharacter { .. })
    ));
    assert!(matches!(
        ReplyDisplayText::new("   "),
        Err(ReplyContractError::EmptyText { .. })
    ));
}

#[test]
fn previews_and_answers_carry_their_own_bounds() {
    assert!(ReplyDisplayPreview::new("x".repeat(REPLY_DISPLAY_PREVIEW_MAX_BYTES)).is_ok());
    assert!(matches!(
        ReplyDisplayPreview::new("x".repeat(REPLY_DISPLAY_PREVIEW_MAX_BYTES + 1)),
        Err(ReplyContractError::TextTooLong { .. })
    ));
    assert!(ReplyAnswerText::new("x".repeat(REPLY_ANSWER_MAX_BYTES)).is_ok());
    assert!(matches!(
        ReplyAnswerText::new("x".repeat(REPLY_ANSWER_MAX_BYTES + 1)),
        Err(ReplyContractError::TextTooLong { .. })
    ));
    // An answer may legitimately be empty (a run that only produced
    // attachments or failed before any text); display text may not.
    assert!(ReplyAnswerText::new("").is_ok());
}

#[test]
fn item_ids_follow_the_bounded_identifier_grammar() {
    assert!(ReplyItemId::new("activity:0193b7c0-…").is_err());
    assert!(ReplyItemId::new("activity:0193b7c0-1a2b").is_ok());
    assert!(ReplyItemId::new("").is_err());
    assert!(ReplyItemId::new("a".repeat(129)).is_err());
    assert!(ReplyItemId::new("has space").is_err());
}

#[test]
fn edge_fields_are_bounded_by_construction_not_by_optional_validation() {
    // Thread anchors and stored reply context are validated newtypes.
    assert!(ReplyThreadAnchor::new("1717171717.123456").is_ok());
    assert!(matches!(
        ReplyThreadAnchor::new("x".repeat(REPLY_THREAD_ANCHOR_MAX_BYTES + 1)),
        Err(ReplyContractError::TextTooLong { .. })
    ));
    assert!(ReplyContextBytes::new(vec![0u8; REPLY_CONTEXT_MAX_BYTES]).is_ok());
    assert!(matches!(
        ReplyContextBytes::new(vec![0u8; REPLY_CONTEXT_MAX_BYTES + 1]),
        Err(ReplyContractError::ReplyContextTooLarge { .. })
    ));

    // Diagnostic reasons fold into the bound instead of failing: a reason
    // exists to explain a failure, never to gate it.
    let reason = ReplyOutcomeReason::new("x".repeat(REPLY_OUTCOME_REASON_MAX_BYTES + 100));
    assert_eq!(reason.as_str().len(), REPLY_OUTCOME_REASON_MAX_BYTES);
    assert_eq!(
        ReplyOutcomeReason::new("bell\u{7}here").as_str(),
        "bellhere",
        "control characters are stripped, not passed through"
    );
    assert_eq!(ReplyOutcomeReason::new("   ").as_str(), "unspecified");

    // Provider refs: each bounded, the collection capped, over-reporting
    // refused rather than silently dropped.
    assert!(ReplyProviderRef::new("x".repeat(257)).is_err());
    let mut refs = ReplyProviderRefs::default();
    for index in 0..REPLY_MAX_PROVIDER_REFS {
        refs.push(ReplyProviderRef::new(format!("1717.{index}")).expect("ref"))
            .expect("within the bound");
    }
    assert!(matches!(
        refs.push(ReplyProviderRef::new("one too many").expect("ref")),
        Err(ReplyContractError::TooManyItems { .. })
    ));
    let over: Result<ReplyProviderRefs, _> = serde_json::from_str(
        &serde_json::to_string(
            &(0..=REPLY_MAX_PROVIDER_REFS)
                .map(|index| format!("ref-{index}"))
                .collect::<Vec<_>>(),
        )
        .expect("encode"),
    );
    assert!(over.is_err(), "the bound holds through deserialization too");
}

#[test]
fn sink_checkpoints_are_bounded_through_construction_and_deserialization() {
    let checkpoint = ReplySinkCheckpoint::new(3, "x".repeat(REPLY_SINK_CHECKPOINT_MAX_BYTES))
        .expect("at the bound");
    assert_eq!(checkpoint.version(), 3);
    assert_eq!(checkpoint.payload().len(), REPLY_SINK_CHECKPOINT_MAX_BYTES);
    assert!(matches!(
        ReplySinkCheckpoint::new(1, "x".repeat(REPLY_SINK_CHECKPOINT_MAX_BYTES + 1)),
        Err(ReplyContractError::CheckpointTooLarge { .. })
    ));
    let oversized = serde_json::json!({
        "version": 1,
        "payload": "x".repeat(REPLY_SINK_CHECKPOINT_MAX_BYTES + 1),
    });
    assert!(
        serde_json::from_value::<ReplySinkCheckpoint>(oversized).is_err(),
        "a persisted checkpoint past the bound is refused on read, not trusted"
    );
    let round_trip: ReplySinkCheckpoint =
        serde_json::from_str(&serde_json::to_string(&checkpoint).expect("encode")).expect("decode");
    assert_eq!(round_trip, checkpoint);
}

// ── The document mutators: deterministic desired state ────────────────────

#[test]
fn mutators_fold_the_documented_facets() {
    let mut document = ReplyDocument::default();
    assert_eq!(document.phase, ReplyPhase::Preparing);

    assert!(document.note_phase(ReplyPhase::Thinking));
    assert!(document.close_reasoning(
        ReplyReasoningText::new("Checking live Slack access first.").expect("reasoning")
    ));
    assert!(document.append_answer("Here is "));
    assert!(document.append_answer("what I found."));
    assert!(document.activity_started(
        item("act:1"),
        text("slack.get_conversation_history"),
        Some(ReplyDisplayPreview::new("channel: #private-inference").expect("preview")),
    ));
    assert!(document.set_status(text("Reading the last 50 messages"), None));
    assert!(document.activity_finished(
        item("act:1"),
        ReplyActivityState::Completed,
        Some(ReplyDisplayPreview::new("50 messages").expect("preview")),
        None,
    ));

    assert_eq!(document.phase, ReplyPhase::Working);
    assert_eq!(document.answer.text.as_str(), "Here is what I found.");
    assert!(!document.answer.finalized);
    assert_eq!(document.reasoning.len(), 1);
    assert_eq!(
        document.status.as_ref().map(ReplyDisplayText::as_str),
        Some("Reading the last 50 messages")
    );
    let activity = &document.activities[0];
    assert_eq!(activity.state, ReplyActivityState::Completed);
    assert_eq!(
        activity.detail.as_ref().map(ReplyDisplayPreview::as_str),
        Some("channel: #private-inference")
    );
    assert_eq!(
        activity
            .output_preview
            .as_ref()
            .map(ReplyDisplayPreview::as_str),
        Some("50 messages")
    );
    assert!(!document.is_terminal());
}

#[test]
fn attention_moves_the_phase_and_clears_back_to_working() {
    let mut document = ReplyDocument::default();
    document.note_phase(ReplyPhase::Working);
    assert!(document.require_attention(ReplyAttention {
        kind: ReplyAttentionKind::Approval,
        headline: text("Approve running `git push`?"),
        body: None,
        action_url: None,
        gate_ref: Some(text("gate:approval:1")),
    }));
    assert_eq!(document.phase, ReplyPhase::WaitingForInput);
    assert!(document.attention.is_some());
    // Attention owns the phase while present.
    assert!(!document.note_phase(ReplyPhase::Thinking));
    assert_eq!(document.phase, ReplyPhase::WaitingForInput);
    assert!(document.clear_attention());
    assert_eq!(document.phase, ReplyPhase::Working);
    assert!(document.attention.is_none());
    assert!(!document.clear_attention(), "nothing left to clear");
}

#[test]
fn the_first_terminal_outcome_wins_and_later_mutations_are_ignored() {
    let mut document = ReplyDocument::default();
    assert!(document.append_answer("partial"));
    assert!(document.fail(text("The model provider timed out.")));
    assert!(document.is_terminal());
    assert_eq!(document.phase, ReplyPhase::Failed);
    assert!(matches!(
        document.outcome,
        Some(ReplyOutcome::Failed { .. })
    ));

    assert!(!document.complete());
    assert!(!document.append_answer(" more"));
    assert!(!document.note_phase(ReplyPhase::Working));
    assert_eq!(
        document.phase,
        ReplyPhase::Failed,
        "the first terminal fact is durable"
    );
    assert!(matches!(
        document.outcome,
        Some(ReplyOutcome::Failed { .. })
    ));
    assert_eq!(document.answer.text.as_str(), "partial");
}

#[test]
fn the_canonical_finalized_answer_replaces_progressive_text_even_after_terminal() {
    let mut document = ReplyDocument::default();
    document.append_answer("draft that will be superseded");
    document.complete();
    assert!(document.finalize_answer(answer("canonical transcript text"), Vec::new()));
    assert!(document.answer.finalized);
    assert_eq!(document.answer.text.as_str(), "canonical transcript text");
    assert_eq!(document.phase, ReplyPhase::Completed);
}

#[test]
fn answer_growth_is_capped_and_marked_truncated_instead_of_overflowing() {
    let mut document = ReplyDocument::default();
    document.append_answer(&"a".repeat(REPLY_ANSWER_MAX_BYTES - 2));
    document.append_answer("ééé");
    assert!(document.answer.text.as_str().len() <= REPLY_ANSWER_MAX_BYTES);
    assert!(
        document
            .answer
            .text
            .as_str()
            .is_char_boundary(document.answer.text.as_str().len())
    );
    assert!(document.answer.truncated);
}

#[test]
fn raw_answer_paths_strip_control_characters_by_construction() {
    let mut document = ReplyDocument::default();
    document.append_answer("keep\nlines\tand text\u{1b}[31m but not escapes\u{7}");
    assert_eq!(
        document.answer.text.as_str(),
        "keep\nlines\tand text[31m but not escapes"
    );
    document.rewrite_answer("clean\u{0} rewrite");
    assert_eq!(document.answer.text.as_str(), "clean rewrite");
}

#[test]
fn activity_fan_out_is_bounded_and_unknown_finishes_are_recorded_not_dropped() {
    let mut document = ReplyDocument::default();
    for index in 0..REPLY_MAX_ACTIVITIES + 5 {
        document.activity_started(item(&format!("act:{index}")), text("tool"), None);
    }
    assert_eq!(document.activities.len(), REPLY_MAX_ACTIVITIES);
    assert!(document.activities_truncated);

    let mut sparse = ReplyDocument::default();
    assert!(sparse.activity_finished(
        item("late:1"),
        ReplyActivityState::Failed {
            kind: text("gate_declined"),
        },
        None,
        None,
    ));
    assert_eq!(
        sparse.activities.len(),
        1,
        "a finish for an unseen activity still lands as a row"
    );
    assert!(matches!(
        sparse.activities[0].state,
        ReplyActivityState::Failed { .. }
    ));
}

#[test]
fn the_document_is_wire_stable_snake_case() {
    let mut document = ReplyDocument::default();
    document.activity_started(item("act:1"), text("tool"), None);
    document.activity_finished(
        item("act:1"),
        ReplyActivityState::Failed {
            kind: text("timeout"),
        },
        None,
        None,
    );
    let json = serde_json::to_value(&document).expect("serialize");
    assert_eq!(json["phase"], "working");
    assert_eq!(json["activities"][0]["state"]["failed"]["kind"], "timeout");
    let restored: ReplyDocument = serde_json::from_value(json).expect("document restores");
    assert_eq!(restored, document);
}

// ── Cadence: one seam, two transports ─────────────────────────────────────

#[test]
fn stream_reconciles_at_every_point_and_message_only_at_terminal() {
    for point in [
        ReplyReconcilePoint::Opened,
        ReplyReconcilePoint::Progress,
        ReplyReconcilePoint::ControlCritical,
        ReplyReconcilePoint::Terminal,
        ReplyReconcilePoint::Heartbeat,
    ] {
        assert!(ReplyTransport::Stream.reconciles_at(point), "{point:?}");
        assert_eq!(
            ReplyTransport::Message.reconciles_at(point),
            matches!(point, ReplyReconcilePoint::Terminal),
            "{point:?}"
        );
    }
}

// ── Sink seam shape ───────────────────────────────────────────────────────

struct CountingSink {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ReplySink for CountingSink {
    async fn reconcile(
        &self,
        request: ReplyReconcileRequest,
        _egress: &dyn RestrictedEgress,
    ) -> Result<ReplySinkReport, ChannelError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let checkpoint = ReplySinkCheckpoint::new(1, format!("rev={}", request.revision.revision))
            .expect("checkpoint");
        Ok(ReplySinkReport::applied(
            Some(checkpoint),
            ReplySinkEvidence::default(),
        ))
    }
}

struct NoEgress;

#[async_trait]
impl RestrictedEgress for NoEgress {
    async fn send(
        &self,
        _request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Err(RestrictedEgressError::PolicyDenied)
    }
}

/// Drive an immediately-ready future without a runtime: the contracts crate
/// carries no async executor, and a reconcile against the counting sink never
/// suspends.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

#[test]
fn a_sink_receives_a_desired_revision_and_returns_evidence_plus_a_checkpoint() {
    let sink = CountingSink {
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let mut document = ReplyDocument::default();
    document.append_answer("hello");
    let request = ReplyReconcileRequest {
        revision: ReplyRevision {
            revision: 3,
            document,
        },
        point: ReplyReconcilePoint::Progress,
        target: target(ReplyAudience::Private),
        reply_context: Some(ReplyContextBytes::new(b"{\"channel\":\"C1\"}".to_vec()).expect("ctx")),
        checkpoint: None,
        extension_generation: 7,
        materialized_attachments: Vec::new(),
    };
    let report = block_on(sink.reconcile(request, &NoEgress)).expect("reconcile");
    assert!(matches!(report.outcome, ReplySinkOutcome::Applied));
    assert_eq!(
        report.checkpoint.as_ref().map(ReplySinkCheckpoint::payload),
        Some("rev=3")
    );
    assert!(!report.evidence.read_back_verified);
}

#[test]
fn retryable_outcomes_carry_a_typed_retry_after_hint() {
    let report = ReplySinkReport {
        outcome: ReplySinkOutcome::Retryable {
            reason: ReplyOutcomeReason::new("ratelimited"),
            retry_after: Some(Duration::from_secs(3)),
        },
        checkpoint: None,
        evidence: ReplySinkEvidence::default(),
    };
    assert!(matches!(
        report.outcome,
        ReplySinkOutcome::Retryable {
            retry_after: Some(hint),
            ..
        } if hint == Duration::from_secs(3)
    ));
    assert!(!report.outcome.is_applied());
    assert_eq!(report.outcome.kind_name(), "retryable");
}

#[test]
fn egress_responses_expose_retry_after_as_the_only_response_header_hint() {
    let response = RestrictedEgressResponse {
        status: 429,
        body: Vec::new(),
        retry_after: Some(Duration::from_secs(2)),
    };
    assert_eq!(response.retry_after, Some(Duration::from_secs(2)));
}

// ── Binding shape: one reply slot, one trait ──────────────────────────────

#[test]
fn the_reply_slot_holds_one_sink_whatever_the_transport() {
    let sink = Arc::new(CountingSink {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let surfaces = ChannelSurfaces::default().with_reply(sink.clone());
    assert!(surfaces.reply.is_some());
    assert!(surfaces.has_outbound());
    assert_eq!(
        format!("{surfaces:?}"),
        "ChannelSurfaces { ingress: false, reply: true, delivery: false }"
    );
    // Binding again replaces rather than accumulates: the slot holds one.
    let replaced = surfaces.with_reply(Arc::new(CountingSink {
        calls: std::sync::atomic::AtomicUsize::new(0),
    }));
    assert!(replaced.reply.is_some());
}

#[test]
fn a_webhook_channel_may_declare_a_stream_reply() {
    let channel: ChannelDescriptor = toml::from_str(
        r#"
id = "messages"
display_name = "Vendor messages"
conversation_model = "continuous"

[reply]
transport = "stream"

[ingress]
route_suffix = "events"
method = "post"

[ingress.verification]
kind = "hmac_sha256"
secret_handle = "vendor_signing_secret"
signature_header = "X-Vendor-Signature"
signature_prefix = "v0="
signature_encoding = "hex"
timestamp_header = "X-Vendor-Request-Timestamp"
max_age_seconds = 300
signed_payload = [
  { literal = "v0:" },
  { header = "X-Vendor-Request-Timestamp" },
  { literal = ":" },
  { body = true },
]
"#,
    )
    .expect("webhook channel with a stream reply deserializes");
    channel
        .validate()
        .expect("streaming is a channel capability, not a session-transport privilege");
    assert_eq!(channel.reply_transport(), Some(ReplyTransport::Stream));
}

#[test]
fn reasoning_appends_grow_the_open_segment_until_a_summary_closes_it() {
    let mut document = ReplyDocument::default();
    assert!(document.append_reasoning(&ReplyReasoningText::new("Looking at ").unwrap()));
    assert!(document.append_reasoning(&ReplyReasoningText::new("the workspace").unwrap()));
    assert_eq!(document.reasoning.len(), 1, "appends grow one open segment");
    assert_eq!(document.reasoning[0].as_str(), "Looking at the workspace");
    assert!(
        document.reasoning_open,
        "the segment is still being produced"
    );
    assert_eq!(
        document.phase,
        ReplyPhase::Thinking,
        "reasoning moves a fresh reply out of Preparing"
    );

    // The boundary summary carries the segment's final text: it replaces
    // the open segment (no duplicate) and closes it.
    assert!(
        document
            .close_reasoning(ReplyReasoningText::new("Looking at the workspace first.").unwrap())
    );
    assert_eq!(document.reasoning.len(), 1);
    assert_eq!(
        document.reasoning[0].as_str(),
        "Looking at the workspace first."
    );
    assert!(!document.reasoning_open);

    // A later append opens a NEW segment rather than reopening the closed one.
    assert!(document.append_reasoning(&ReplyReasoningText::new("Now the tests").unwrap()));
    assert_eq!(document.reasoning.len(), 2);
    assert!(document.reasoning_open);

    // Growth past the per-segment bound is dropped, never split or panicked.
    let big = "é".repeat(REPLY_REASONING_SEGMENT_MAX_BYTES);
    document.append_reasoning(
        &ReplyReasoningText::new(&big[..REPLY_REASONING_SEGMENT_MAX_BYTES / 2]).unwrap(),
    );
    document.append_reasoning(
        &ReplyReasoningText::new(&big[..REPLY_REASONING_SEGMENT_MAX_BYTES / 2]).unwrap(),
    );
    assert!(document.reasoning[1].as_str().len() <= REPLY_REASONING_SEGMENT_MAX_BYTES);
    assert!(
        document.reasoning[1]
            .as_str()
            .is_char_boundary(document.reasoning[1].as_str().len())
    );

    // Terminal documents ignore late reasoning.
    document.complete();
    assert!(!document.append_reasoning(&ReplyReasoningText::new("too late").unwrap()));
    assert_eq!(document.reasoning.len(), 2);
    assert!(!document.reasoning[1].as_str().ends_with("too late"));
}

#[test]
fn a_rewritten_answer_replaces_the_progressive_text_but_never_the_finalized_row() {
    let mut document = ReplyDocument::default();
    document.append_answer("first draft");
    assert!(document.rewrite_answer("second draft"));
    assert_eq!(document.answer.text.as_str(), "second draft");
    assert!(!document.answer.finalized);

    document.finalize_answer(answer("the transcript row"), Vec::new());
    assert!(!document.rewrite_answer("too late"));
    assert_eq!(
        document.answer.text.as_str(),
        "the transcript row",
        "a rewrite never displaces the finalized transcript text"
    );
}

/// A no-op attention clear must not mint a revisionable change: the document
/// would compare unequal to its previous revision and every stream sink
/// would be asked to reconcile a zero-semantic delta.
#[test]
fn a_no_op_attention_clear_changes_nothing() {
    let mut document = ReplyDocument::default();
    let before = document.clone();
    assert!(!document.clear_attention());
    assert_eq!(
        document, before,
        "clearing absent attention must leave the document identical"
    );
}
