//! The progressive-reply vocabulary contract
//! (`docs/internal/design/2026-08-31-progressive-reply-publication.md` §3, §6).
//!
//! What a reply sink may ever see is a bounded, typed desired-state document:
//! these tests pin the bounds (by construction, including through serde), the
//! reducer semantics every producer and sink rely on, the change classes the
//! publication worker keys coalescing on, the cadence each transport admits,
//! the wire stability of the change vocabulary, and the single-seam binding
//! shape (`ChannelSurfaces.reply` is one `ReplySink`, whatever the transport).

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
    ReplyAnswerText, ReplyAttention, ReplyAttentionKind, ReplyAudience, ReplyChange,
    ReplyChangeClass, ReplyContextBytes, ReplyContractError, ReplyDisplayPreview, ReplyDisplayText,
    ReplyDocument, ReplyId, ReplyItemId, ReplyOutcome, ReplyOutcomeReason, ReplyPhase,
    ReplyProviderRef, ReplyProviderRefs, ReplyReasoningText, ReplyReconcilePoint,
    ReplyReconcileRequest, ReplyRevision, ReplySink, ReplySinkCheckpoint, ReplySinkEvidence,
    ReplySinkOutcome, ReplySinkReport, ReplyTarget, ReplyThreadAnchor,
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
    let run_id = TurnRunId::new();
    assert_eq!(ReplyId::for_run(&run_id).as_str(), run_id.to_string());
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

// ── The reducer: deterministic desired state from changes ─────────────────

#[test]
fn reducer_folds_the_documented_facets() {
    let mut document = ReplyDocument::default();
    assert_eq!(document.phase, ReplyPhase::Preparing);

    document.apply(&ReplyChange::PhaseChanged {
        phase: ReplyPhase::Thinking,
    });
    document.apply(&ReplyChange::ReasoningSummary {
        text: ReplyReasoningText::new("Checking live Slack access first.").expect("reasoning"),
    });
    document.apply(&ReplyChange::AnswerAppended {
        text: answer("Here is "),
    });
    document.apply(&ReplyChange::AnswerAppended {
        text: answer("what I found."),
    });
    document.apply(&ReplyChange::ActivityStarted {
        id: item("act:1"),
        title: text("slack.get_conversation_history"),
        detail: Some(ReplyDisplayPreview::new("channel: #private-inference").expect("preview")),
    });
    document.apply(&ReplyChange::StatusSummary {
        text: text("Reading the last 50 messages"),
        work: None,
    });
    document.apply(&ReplyChange::ActivityFinished {
        id: item("act:1"),
        state: ReplyActivityState::Completed,
        output_preview: Some(ReplyDisplayPreview::new("50 messages").expect("preview")),
        provenance: None,
    });

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
    document.apply(&ReplyChange::PhaseChanged {
        phase: ReplyPhase::Working,
    });
    document.apply(&ReplyChange::AttentionRequired {
        attention: ReplyAttention {
            kind: ReplyAttentionKind::Approval,
            headline: text("Approve running `git push`?"),
            body: None,
            action_url: None,
            gate_ref: Some(text("gate:approval:1")),
        },
    });
    assert_eq!(document.phase, ReplyPhase::WaitingForInput);
    assert!(document.attention.is_some());
    document.apply(&ReplyChange::AttentionCleared);
    assert_eq!(document.phase, ReplyPhase::Working);
    assert!(document.attention.is_none());
}

#[test]
fn the_first_terminal_outcome_wins_and_later_replaceable_changes_are_ignored() {
    let mut document = ReplyDocument::default();
    document.apply(&ReplyChange::AnswerAppended {
        text: answer("partial"),
    });
    document.apply(&ReplyChange::Failed {
        summary: text("The model provider timed out."),
    });
    assert!(document.is_terminal());
    assert_eq!(document.phase, ReplyPhase::Failed);
    assert!(matches!(
        document.outcome,
        Some(ReplyOutcome::Failed { .. })
    ));

    document.apply(&ReplyChange::Completed);
    document.apply(&ReplyChange::AnswerAppended {
        text: answer(" more"),
    });
    document.apply(&ReplyChange::PhaseChanged {
        phase: ReplyPhase::Working,
    });
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
    document.apply(&ReplyChange::AnswerAppended {
        text: answer("draft that will be superseded"),
    });
    document.apply(&ReplyChange::Completed);
    document.apply(&ReplyChange::AnswerFinalized {
        text: answer("canonical transcript text"),
        attachments: Vec::new(),
    });
    assert!(document.answer.finalized);
    assert_eq!(document.answer.text.as_str(), "canonical transcript text");
    assert_eq!(document.phase, ReplyPhase::Completed);
}

#[test]
fn answer_growth_is_capped_and_marked_truncated_instead_of_overflowing() {
    let mut document = ReplyDocument::default();
    document.apply(&ReplyChange::AnswerAppended {
        text: answer(&"a".repeat(REPLY_ANSWER_MAX_BYTES - 2)),
    });
    document.apply(&ReplyChange::AnswerAppended {
        text: answer("ééé"),
    });
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
fn activity_fan_out_is_bounded_and_unknown_finishes_are_recorded_not_dropped() {
    let mut document = ReplyDocument::default();
    for index in 0..REPLY_MAX_ACTIVITIES + 5 {
        document.apply(&ReplyChange::ActivityStarted {
            id: item(&format!("act:{index}")),
            title: text("tool"),
            detail: None,
        });
    }
    assert_eq!(document.activities.len(), REPLY_MAX_ACTIVITIES);
    assert!(document.activities_truncated);

    let mut sparse = ReplyDocument::default();
    sparse.apply(&ReplyChange::ActivityFinished {
        id: item("late:1"),
        state: ReplyActivityState::Failed {
            kind: text("gate_declined"),
        },
        output_preview: None,
        provenance: None,
    });
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
fn change_classes_separate_what_a_publisher_may_coalesce_from_what_it_may_not() {
    let critical = [
        ReplyChange::AttentionRequired {
            attention: ReplyAttention {
                kind: ReplyAttentionKind::Auth,
                headline: text("Connect GitHub to continue"),
                body: None,
                action_url: None,
                gate_ref: None,
            },
        },
        ReplyChange::AttentionCleared,
        ReplyChange::AnswerFinalized {
            text: answer("done"),
            attachments: Vec::new(),
        },
    ];
    for change in &critical {
        assert_eq!(
            change.class(),
            ReplyChangeClass::ControlCritical,
            "{}",
            change.kind_name()
        );
        assert!(change.is_control_critical());
        assert!(!change.is_terminal());
    }
    let terminal = [
        ReplyChange::Completed,
        ReplyChange::Failed {
            summary: text("failed"),
        },
        ReplyChange::Cancelled,
    ];
    for change in &terminal {
        assert_eq!(
            change.class(),
            ReplyChangeClass::Terminal,
            "{}",
            change.kind_name()
        );
        assert!(change.is_control_critical());
        assert!(change.is_terminal());
    }
    let replaceable = [
        ReplyChange::AnswerAppended {
            text: answer("token"),
        },
        ReplyChange::ReasoningSummary {
            text: ReplyReasoningText::new("thinking").expect("reasoning"),
        },
        ReplyChange::PhaseChanged {
            phase: ReplyPhase::Thinking,
        },
        ReplyChange::StatusSummary {
            text: text("planning"),
            work: None,
        },
        ReplyChange::ActivityStarted {
            id: item("a"),
            title: text("tool"),
            detail: None,
        },
    ];
    for change in &replaceable {
        assert_eq!(
            change.class(),
            ReplyChangeClass::Replaceable,
            "{}",
            change.kind_name()
        );
        assert!(!change.is_control_critical());
    }
}

#[test]
fn the_change_vocabulary_is_wire_stable_snake_case() {
    let change = ReplyChange::ActivityFinished {
        id: item("act:1"),
        state: ReplyActivityState::Failed {
            kind: text("timeout"),
        },
        output_preview: None,
        provenance: None,
    };
    let json = serde_json::to_value(&change).expect("serialize");
    assert_eq!(json["kind"], "activity_finished");
    assert_eq!(json["state"]["failed"]["kind"], "timeout");
    let round_trip: ReplyChange = serde_json::from_value(json).expect("deserialize");
    assert_eq!(round_trip, change);

    let mut document = ReplyDocument::default();
    document.apply(&change);
    let document_json = serde_json::to_string(&document).expect("document serializes");
    let restored: ReplyDocument = serde_json::from_str(&document_json).expect("document restores");
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
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop_raw_waker() -> RawWaker {
        fn clone(_: *const ()) -> RawWaker {
            noop_raw_waker()
        }
        fn noop(_: *const ()) {}
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    // SAFETY: the vtable functions are no-ops over a null data pointer.
    let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
    let mut context = Context::from_waker(&waker);
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
    document.apply(&ReplyChange::AnswerAppended {
        text: answer("hello"),
    });
    let request = ReplyReconcileRequest {
        revision: ReplyRevision {
            reply_id: ReplyId::for_run(&TurnRunId::new()),
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
    assert_eq!(report.outcome.retry_after(), Some(Duration::from_secs(3)));
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
fn reasoning_appended_grows_the_open_segment_until_a_summary_closes_it() {
    let mut document = ReplyDocument::default();
    document.apply(&ReplyChange::ReasoningAppended {
        text: ReplyReasoningText::new("Looking at ").unwrap(),
    });
    document.apply(&ReplyChange::ReasoningAppended {
        text: ReplyReasoningText::new("the workspace").unwrap(),
    });
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
    document.apply(&ReplyChange::ReasoningSummary {
        text: ReplyReasoningText::new("Looking at the workspace first.").unwrap(),
    });
    assert_eq!(document.reasoning.len(), 1);
    assert_eq!(
        document.reasoning[0].as_str(),
        "Looking at the workspace first."
    );
    assert!(!document.reasoning_open);

    // A later append opens a NEW segment rather than reopening the closed one.
    document.apply(&ReplyChange::ReasoningAppended {
        text: ReplyReasoningText::new("Now the tests").unwrap(),
    });
    assert_eq!(document.reasoning.len(), 2);
    assert!(document.reasoning_open);

    // Growth past the per-segment bound is dropped, never split or panicked.
    let big = "é".repeat(REPLY_REASONING_SEGMENT_MAX_BYTES);
    document.apply(&ReplyChange::ReasoningAppended {
        text: ReplyReasoningText::new(&big[..REPLY_REASONING_SEGMENT_MAX_BYTES / 2]).unwrap(),
    });
    document.apply(&ReplyChange::ReasoningAppended {
        text: ReplyReasoningText::new(&big[..REPLY_REASONING_SEGMENT_MAX_BYTES / 2]).unwrap(),
    });
    assert!(document.reasoning[1].as_str().len() <= REPLY_REASONING_SEGMENT_MAX_BYTES);
    assert!(
        document.reasoning[1]
            .as_str()
            .is_char_boundary(document.reasoning[1].as_str().len())
    );

    // Terminal documents ignore late reasoning.
    document.apply(&ReplyChange::Completed);
    document.apply(&ReplyChange::ReasoningAppended {
        text: ReplyReasoningText::new("too late").unwrap(),
    });
    assert_eq!(document.reasoning.len(), 2);
    assert!(!document.reasoning[1].as_str().ends_with("too late"));
}

#[test]
fn a_rewritten_answer_replaces_the_progressive_text_but_never_the_finalized_row() {
    let mut document = ReplyDocument::default();
    document.apply(&ReplyChange::AnswerAppended {
        text: ReplyAnswerText::new("first draft").unwrap(),
    });
    document.apply(&ReplyChange::AnswerRewritten {
        text: ReplyAnswerText::new("second draft").unwrap(),
    });
    assert_eq!(document.answer.text.as_str(), "second draft");
    assert!(!document.answer.finalized);
    assert_eq!(
        ReplyChange::AnswerRewritten {
            text: ReplyAnswerText::new("x").unwrap()
        }
        .class(),
        ReplyChangeClass::Replaceable
    );

    document.apply(&ReplyChange::AnswerFinalized {
        text: ReplyAnswerText::new("the transcript row").unwrap(),
        attachments: Vec::new(),
    });
    document.apply(&ReplyChange::AnswerRewritten {
        text: ReplyAnswerText::new("too late").unwrap(),
    });
    assert_eq!(
        document.answer.text.as_str(),
        "the transcript row",
        "a rewrite never displaces the finalized transcript text"
    );
}
