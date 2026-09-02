//! The assistant-owned safe reply projection: loop milestones compose into a
//! bounded, redacted `ReplyDocument`; the terminal revision is built from
//! durable history, never from the ephemeral stream; audience disclosure is
//! applied before anything leaves the projection.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_extension_contracts::reply::{
    REPLY_REASONING_SEGMENT_MAX_BYTES, ReplyActivityState, ReplyAttentionKind, ReplyAudience,
    ReplyDisplayPreview, ReplyItemId, ReplyOutcome, ReplyPhase, ReplyReconcilePoint,
};
use ironclaw_host_api::ids::{
    AgentId, CapabilityId, ExtensionId, InvocationId, TenantId, ThreadId, UserId,
};
use ironclaw_host_api::result_meta::FailureKind;
use ironclaw_host_api::runtime::RuntimeKind;
use ironclaw_host_api::turn::{
    CapabilityActivityId, LoopExitId, LoopGateRef, TurnActor, TurnCheckpointId, TurnId, TurnRunId,
    TurnScope, TurnStatus,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, CapabilityInputRef, LoopCompletionKind, LoopDriverId, LoopDriverNoteKind,
    LoopFailureKind, LoopGateKind, LoopHostMilestone, LoopHostMilestoneKind, LoopHostMilestoneSink,
    LoopSafeSummary,
};
use ironclaw_threads::{AttachmentKind, AttachmentRef};

use super::{
    ReplyProjection, ReplyProjectionEvent, ReplyProjectionMilestoneSink, ReplyProjectionObserver,
    TerminalReplyFacts, disclose_for_audience,
};
use crate::projection::{CapabilityDisplayPreviewResult, CapabilityDisplayPreviewStore};

struct Fixture {
    projection: Arc<ReplyProjection>,
    events: Arc<RecordingObserver>,
    scope: TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
}

#[derive(Default)]
struct RecordingObserver {
    events: Mutex<Vec<(TurnRunId, ReplyProjectionEvent)>>,
}

impl RecordingObserver {
    fn events(&self) -> Vec<ReplyProjectionEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|(_, event)| *event)
            .collect()
    }
}

impl ReplyProjectionObserver for RecordingObserver {
    fn reply_projection_event(
        &self,
        _scope: &TurnScope,
        run_id: TurnRunId,
        event: ReplyProjectionEvent,
    ) {
        self.events.lock().unwrap().push((run_id, event));
    }
}

fn fixture(label: &str) -> Fixture {
    let tenant_id = TenantId::new(format!("{label}-tenant")).unwrap();
    let agent_id = AgentId::new(format!("{label}-agent")).unwrap();
    let thread_id = ThreadId::new(format!("{label}-thread")).unwrap();
    let user_id = UserId::new(format!("{label}-user")).unwrap();
    let projection = Arc::new(ReplyProjection::new());
    let events = Arc::new(RecordingObserver::default());
    projection.add_observer(Arc::clone(&events) as Arc<dyn ReplyProjectionObserver>);
    Fixture {
        projection,
        events,
        scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
        actor: TurnActor::new(user_id),
        run_id: TurnRunId::new(),
    }
}

/// The document's narration as `(phase, text)` pairs.
fn narration(document: &ironclaw_extension_contracts::reply::ReplyDocument) -> Vec<(u64, String)> {
    document
        .narration
        .iter()
        .map(|entry| (entry.phase, entry.text.as_str().to_string()))
        .collect()
}

impl Fixture {
    fn milestone(&self, kind: LoopHostMilestoneKind) -> LoopHostMilestone {
        LoopHostMilestone {
            scope: self.scope.clone(),
            actor: Some(self.actor.clone()),
            turn_id: TurnId::new(),
            run_id: self.run_id,
            loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
            kind,
        }
    }

    fn observe(&self, kind: LoopHostMilestoneKind) {
        self.projection.observe_milestone(&self.milestone(kind));
    }

    fn document(&self) -> ironclaw_extension_contracts::reply::ReplyDocument {
        self.projection
            .snapshot(&self.scope, self.run_id)
            .expect("the run is tracked")
            .document
    }
}

#[test]
fn terminal_facts_applied_twice_leave_the_revision_unchanged() {
    let fixture = fixture("terminal-twice");
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "final answer".to_string(),
    });
    let facts = TerminalReplyFacts {
        actor: Some(fixture.actor.clone()),
        status: TurnStatus::Completed,
        nothing_to_report: false,
        answer: Some("final answer".to_string()),
        attachments: Vec::new(),
        failure_summary: None,
    };
    let first =
        fixture
            .projection
            .apply_terminal_facts(&fixture.scope, fixture.run_id, facts.clone());
    assert!(first.document.is_terminal());

    let second = fixture
        .projection
        .apply_terminal_facts(&fixture.scope, fixture.run_id, facts);
    assert_eq!(
        second.revision, first.revision,
        "idempotent: no second terminal revision"
    );
    assert_eq!(second.document, first.document);
}

#[test]
fn a_failed_model_calls_partial_text_is_discarded_when_the_call_is_retried() {
    let fixture = fixture("model-failed");
    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Wor".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelFailed {
        reason_kind: ironclaw_loop_contracts::AgentLoopHostErrorKind::Invalid,
    });
    // The loop retries the call: a fresh ModelStarted, then the cumulative
    // text of the NEW call. The failed call's fragment is not finished text.
    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "World".to_string(),
    });
    let document = fixture.document();
    assert_eq!(document.answer.text.as_str(), "World");
    assert!(
        document.narration.is_empty(),
        "a failed call's fragment is dropped, not kept as narration"
    );
    assert_eq!(document.answer.phase, 1, "a retry is not a new phase");
}

#[test]
fn milestones_compose_into_a_bounded_safe_reply_document() {
    let fixture = fixture("compose");
    let activity_id = CapabilityActivityId::new();

    fixture.observe(LoopHostMilestoneKind::IterationStarted { iteration: 1 });
    let first = fixture
        .projection
        .snapshot(&fixture.scope, fixture.run_id)
        .expect("tracked from the first milestone");
    assert_eq!(first.revision, 1);
    assert_eq!(first.document.phase, ReplyPhase::Preparing);
    assert_eq!(first.actor.as_ref(), Some(&fixture.actor));

    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    assert_eq!(fixture.document().phase, ReplyPhase::Thinking);

    fixture.observe(LoopHostMilestoneKind::ModelReasoningDelta {
        safe_delta: "Looking at ".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelReasoningDelta {
        safe_delta: "the repo".to_string(),
    });
    let document = fixture.document();
    assert_eq!(document.reasoning.len(), 1, "deltas grow one open segment");
    assert_eq!(document.reasoning[0].as_str(), "Looking at the repo");
    assert!(document.reasoning_open);

    // `ModelTextDelta` carries the cumulative text of the current model call.
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Here is ".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Here is what I found.".to_string(),
    });
    let document = fixture.document();
    assert!(
        !document.reasoning_open,
        "answer text closes the reasoning segment"
    );
    assert_eq!(
        document.phase,
        ReplyPhase::Working,
        "the first answer delta ends the thinking phase"
    );
    assert_eq!(document.answer.text.as_str(), "Here is what I found.");
    assert!(!document.answer.finalized);

    fixture.observe(LoopHostMilestoneKind::CapabilityInvoked {
        activity_id,
        capability_id: CapabilityId::new("acme.search").unwrap(),
    });
    let document = fixture.document();
    assert_eq!(document.phase, ReplyPhase::Working);
    assert_eq!(document.activities.len(), 1);
    assert_eq!(document.activities[0].id.as_str(), activity_id.to_string());
    assert_eq!(document.activities[0].title.as_str(), "acme.search");
    assert_eq!(document.activities[0].state, ReplyActivityState::Started);

    fixture.observe(LoopHostMilestoneKind::CapabilityCompleted {
        activity_id,
        capability_id: CapabilityId::new("acme.search").unwrap(),
        provider: ExtensionId::new("acme").unwrap(),
        runtime: RuntimeKind::Wasm,
        output_bytes: 12,
    });
    let document = fixture.document();
    assert_eq!(document.activities[0].state, ReplyActivityState::Completed);
    let provenance = document.activities[0]
        .provenance
        .as_ref()
        .expect("a finished activity records where it ran");
    assert_eq!(
        provenance.provider.as_ref().map(|p| p.as_str()),
        Some("acme")
    );
    assert_eq!(
        provenance.runtime.as_ref().map(|r| r.as_str()),
        Some("wasm")
    );
    assert_eq!(provenance.output_bytes, Some(12));

    fixture.observe(LoopHostMilestoneKind::DriverNote {
        kind: LoopDriverNoteKind::Planning,
        safe_summary: LoopSafeSummary::new("Checking the tests next").unwrap(),
    });
    assert_eq!(
        fixture.document().status.as_ref().map(|s| s.as_str()),
        Some("Checking the tests next")
    );

    let events_before_gate = fixture.events.events().len();
    fixture.observe(LoopHostMilestoneKind::GateBlocked {
        iteration: 1,
        gate_kind: LoopGateKind::Approval,
    });
    fixture.observe(LoopHostMilestoneKind::Blocked {
        gate_ref: LoopGateRef::new("gate:approval-1").unwrap(),
        checkpoint_id: TurnCheckpointId::new(),
    });
    let document = fixture.document();
    let attention = document.attention.as_ref().expect("parked on the user");
    assert_eq!(attention.kind, ReplyAttentionKind::Approval);
    assert_eq!(
        attention.gate_ref.as_ref().map(|g| g.as_str()),
        Some("gate:approval-1")
    );
    assert_eq!(document.phase, ReplyPhase::WaitingForInput);
    assert!(
        fixture.events.events()[events_before_gate..]
            .iter()
            .any(|event| matches!(
                event,
                ReplyProjectionEvent::Revised(ReplyReconcilePoint::ControlCritical)
            )),
        "an input-required transition is signalled as control-critical"
    );

    // The gate resolves: the loop's next iteration clears the attention.
    fixture.observe(LoopHostMilestoneKind::IterationStarted { iteration: 2 });
    let document = fixture.document();
    assert!(document.attention.is_none());
    assert_eq!(document.phase, ReplyPhase::Working);

    // A loop completion is NOT the terminal revision: the terminal document
    // is built from durable history (transcript + run state), so the
    // milestone only flags that the terminal facts are now worth fetching.
    fixture.observe(LoopHostMilestoneKind::Completed {
        completion_kind: LoopCompletionKind::FinalReply,
        exit_id: LoopExitId::new("exit:test").unwrap(),
    });
    let snapshot = fixture
        .projection
        .snapshot(&fixture.scope, fixture.run_id)
        .unwrap();
    assert!(!snapshot.document.is_terminal());
    assert!(snapshot.terminal_pending);
    let events = fixture.events.events();
    assert_eq!(
        events[0],
        ReplyProjectionEvent::Revised(ReplyReconcilePoint::Opened),
        "the first revision opens the reply"
    );
    assert_eq!(
        events.last().copied(),
        Some(ReplyProjectionEvent::TerminalPending)
    );
    assert!(
        snapshot.revision > 1 && snapshot.revision <= snapshot.document.applied_changes,
        "one revision per observed change set, never more than the changes applied; revision {} applied {}",
        snapshot.revision,
        snapshot.document.applied_changes
    );
}

#[test]
fn terminal_facts_from_durable_history_finalize_the_document() {
    let live = fixture("terminal");
    live.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "partial stream text".to_string(),
    });
    live.observe(LoopHostMilestoneKind::Completed {
        completion_kind: LoopCompletionKind::FinalReply,
        exit_id: LoopExitId::new("exit:test").unwrap(),
    });

    let snapshot = live.projection.apply_terminal_facts(
        &live.scope,
        live.run_id,
        TerminalReplyFacts {
            actor: Some(live.actor.clone()),
            status: TurnStatus::Completed,
            nothing_to_report: false,
            answer: Some("The canonical transcript text.".to_string()),
            attachments: vec![AttachmentRef {
                id: "att-1".to_string(),
                kind: AttachmentKind::from_mime_type("text/csv"),
                mime_type: "text/csv".to_string(),
                filename: Some("report.csv".to_string()),
                size_bytes: Some(42),
                storage_key: Some("/workspace/report.csv".to_string()),
                extracted_text: None,
            }],
            failure_summary: None,
        },
    );
    let document = &snapshot.document;
    assert!(document.is_terminal());
    assert!(!snapshot.terminal_pending);
    assert_eq!(document.outcome, Some(ReplyOutcome::Completed));
    assert_eq!(document.phase, ReplyPhase::Completed);
    assert!(document.answer.finalized);
    assert_eq!(
        document.answer.text.as_str(),
        "The canonical transcript text.",
        "the transcript row replaces the ephemeral stream text"
    );
    assert_eq!(document.attachments.len(), 1);
    assert_eq!(document.attachments[0].filename.as_str(), "report.csv");
    assert_eq!(document.attachments[0].size_bytes, 42);
    assert_eq!(
        live.events.events().last().copied(),
        Some(ReplyProjectionEvent::Revised(ReplyReconcilePoint::Terminal))
    );

    // Recovery on a fresh process: no milestone was ever seen, the durable
    // facts alone build the terminal document.
    let fresh = fixture("terminal-fresh");
    let snapshot = fresh.projection.apply_terminal_facts(
        &fresh.scope,
        fresh.run_id,
        TerminalReplyFacts {
            actor: Some(fresh.actor.clone()),
            status: TurnStatus::Failed,
            nothing_to_report: false,
            answer: None,
            attachments: Vec::new(),
            failure_summary: Some("model gateway failed".to_string()),
        },
    );
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.document.phase, ReplyPhase::Failed);
    assert!(matches!(
        &snapshot.document.outcome,
        Some(ReplyOutcome::Failed { summary }) if summary.as_str() == "model gateway failed"
    ));

    // A cancel is its own outcome; a run that is not terminal yet changes
    // nothing (the facts were fetched too early) and stays pending.
    let cancelled = fixture("terminal-cancelled");
    let snapshot = cancelled.projection.apply_terminal_facts(
        &cancelled.scope,
        cancelled.run_id,
        TerminalReplyFacts {
            actor: None,
            status: TurnStatus::Cancelled,
            nothing_to_report: false,
            answer: None,
            attachments: Vec::new(),
            failure_summary: None,
        },
    );
    assert_eq!(snapshot.document.outcome, Some(ReplyOutcome::Cancelled));
    let running = fixture("terminal-running");
    running.observe(LoopHostMilestoneKind::Completed {
        completion_kind: LoopCompletionKind::FinalReply,
        exit_id: LoopExitId::new("exit:test").unwrap(),
    });
    let before = running
        .projection
        .snapshot(&running.scope, running.run_id)
        .unwrap();
    let snapshot = running.projection.apply_terminal_facts(
        &running.scope,
        running.run_id,
        TerminalReplyFacts {
            actor: None,
            status: TurnStatus::Running,
            nothing_to_report: false,
            answer: None,
            attachments: Vec::new(),
            failure_summary: None,
        },
    );
    assert_eq!(snapshot.revision, before.revision);
    assert!(!snapshot.document.is_terminal());
    assert!(snapshot.terminal_pending);
}

#[test]
fn a_failed_run_without_a_safe_summary_gets_the_neutral_failure_copy() {
    let fixture = fixture("failure-copy");
    let snapshot = fixture.projection.apply_terminal_facts(
        &fixture.scope,
        fixture.run_id,
        TerminalReplyFacts {
            actor: None,
            status: TurnStatus::Failed,
            nothing_to_report: false,
            answer: None,
            attachments: Vec::new(),
            failure_summary: Some("boom: api_key=sk-live-1234567890abcdef".to_string()),
        },
    );
    match &snapshot.document.outcome {
        Some(ReplyOutcome::Failed { summary }) => {
            assert!(
                !summary.as_str().contains("sk-live"),
                "a failure summary is redacted before it can reach any channel: {summary:?}"
            );
        }
        other => panic!("expected a failed outcome, got {other:?}"),
    }
}

#[test]
fn reasoning_is_bounded_and_redacted_by_construction() {
    let fixture = fixture("reasoning-bounds");
    let chunk = format!("{} AKIAABCDEFGHIJKLMNOP ", "x".repeat(1000));
    for _ in 0..40 {
        fixture.observe(LoopHostMilestoneKind::ModelReasoningDelta {
            safe_delta: chunk.clone(),
        });
    }
    let document = fixture.document();
    assert!(!document.reasoning.is_empty());
    for segment in &document.reasoning {
        assert!(segment.as_str().len() <= REPLY_REASONING_SEGMENT_MAX_BYTES);
        assert!(
            !segment.as_str().contains("AKIAABCDEFGHIJKLMNOP"),
            "credential-looking tokens never enter the document"
        );
        assert!(segment.as_str().contains("[redacted]"));
    }
    let total: usize = document.reasoning.iter().map(|s| s.as_str().len()).sum();
    assert!(
        total <= 2 * REPLY_REASONING_SEGMENT_MAX_BYTES,
        "the open segment is capped; overflow is dropped, not accumulated: {total}"
    );
}

#[test]
fn a_capability_failure_is_a_row_with_a_sanitized_detail_and_kind() {
    let fixture = fixture("capability-failure");
    let activity_id = CapabilityActivityId::new();
    fixture.observe(LoopHostMilestoneKind::CapabilityFailed {
        activity_id,
        capability_id: CapabilityId::new("acme.write").unwrap(),
        provider: Some(ExtensionId::new("acme").unwrap()),
        runtime: Some(RuntimeKind::Mcp),
        reason_kind: FailureKind::Internal,
        safe_summary: Some(LoopSafeSummary::new("upstream returned 503").unwrap()),
    });
    let document = fixture.document();
    assert_eq!(document.activities.len(), 1);
    let row = &document.activities[0];
    assert!(matches!(
        &row.state,
        ReplyActivityState::Failed { kind } if kind.as_str() == FailureKind::Internal.as_str()
    ));
    assert_eq!(
        row.output_preview.as_ref().map(|p| p.as_str()),
        Some("upstream returned 503")
    );
    assert_eq!(
        row.provenance
            .as_ref()
            .and_then(|p| p.runtime.as_ref())
            .map(|r| r.as_str()),
        Some("mcp")
    );
}

#[test]
fn capability_activity_uses_the_existing_safe_display_preview_store() {
    let fixture = fixture("capability-preview");
    let previews = Arc::new(CapabilityDisplayPreviewStore::default());
    assert!(
        fixture
            .projection
            .bind_display_previews(Arc::clone(&previews))
    );

    let activity_id = CapabilityActivityId::new();
    let invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
    let capability_id = CapabilityId::new("slack.get_conversation_history").unwrap();
    let input_ref = CapabilityInputRef::new("input:slack-history").unwrap();
    previews.record_input(
        &fixture.run_id.to_string(),
        &input_ref,
        capability_id.as_str(),
        &serde_json::json!({ "channel": "C123", "limit": 20 }),
    );
    previews.record_running_invocation(invocation_id, &input_ref);

    fixture.observe(LoopHostMilestoneKind::CapabilityInvoked {
        activity_id,
        capability_id: capability_id.clone(),
    });
    let document = fixture.document();
    assert!(
        document.activities[0]
            .detail
            .as_ref()
            .is_some_and(|detail| detail.as_str().contains("C123")),
        "the running row carries the already-sanitized input summary"
    );

    let output = serde_json::json!({ "messages": [{ "text": "hello from Slack" }] });
    previews.record_result(CapabilityDisplayPreviewResult {
        run_id: &fixture.run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability_id,
        result_ref: "result:slack-history",
        output: &output,
        output_bytes: 48,
    });
    fixture.observe(LoopHostMilestoneKind::CapabilityCompleted {
        activity_id,
        capability_id,
        provider: ExtensionId::new("slack").unwrap(),
        runtime: RuntimeKind::FirstParty,
        output_bytes: 48,
    });
    let document = fixture.document();
    assert!(
        document.activities[0]
            .output_preview
            .as_ref()
            .is_some_and(|preview| preview.as_str().contains("hello from Slack")),
        "the completed row carries the already-sanitized output preview"
    );
}

#[test]
fn shared_audience_disclosure_strips_reasoning_and_bearer_links() {
    let fixture = fixture("disclosure");
    fixture.observe(LoopHostMilestoneKind::ModelReasoningDelta {
        safe_delta: "private thinking".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::GateBlocked {
        iteration: 1,
        gate_kind: LoopGateKind::Auth,
    });
    let mut document = fixture.document();
    let attention = document.attention.as_mut().unwrap();
    attention.action_url = Some(
        ironclaw_extension_contracts::reply::ReplyDisplayText::new("https://auth.example/connect")
            .unwrap(),
    );

    let private = disclose_for_audience(&document, ReplyAudience::Private);
    assert_eq!(
        private, document,
        "a private target sees the whole document"
    );

    // An activity's input and output previews are owner-facing (a shell
    // command, a fetched URL, a workspace path): a shared room keeps the
    // row but not the previews.
    let activity = ReplyItemId::new("act:1").unwrap();
    document.activity_started(
        activity.clone(),
        ironclaw_extension_contracts::reply::ReplyDisplayText::new("shell").unwrap(),
        Some(ReplyDisplayPreview::new("command: cat ~/.ssh/config").unwrap()),
    );
    document.activity_finished(
        activity,
        ReplyActivityState::Completed,
        Some(ReplyDisplayPreview::new("Host github.com").unwrap()),
        None,
    );

    let shared = disclose_for_audience(&document, ReplyAudience::Shared);
    assert!(shared.reasoning.is_empty(), "no reasoning in a shared room");
    assert_eq!(shared.activities.len(), 1, "the row itself stays");
    assert!(
        shared.activities[0].detail.is_none() && shared.activities[0].output_preview.is_none(),
        "previews never land in a shared conversation"
    );
    assert!(!shared.reasoning_open);
    assert!(
        shared.attention.as_ref().unwrap().action_url.is_none(),
        "a connect link is bearer material and never lands in a shared conversation"
    );
    assert_eq!(
        shared.attention.as_ref().unwrap().kind,
        ReplyAttentionKind::Auth
    );
    assert_eq!(shared.answer, document.answer);
}

#[derive(Default)]
struct InnerSink {
    seen: Mutex<Vec<String>>,
    fail: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl LoopHostMilestoneSink for InnerSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AgentLoopHostError::new(
                ironclaw_loop_contracts::AgentLoopHostErrorKind::Invalid,
                "durable milestone sink refused the milestone",
            ));
        }
        self.seen
            .lock()
            .unwrap()
            .push(milestone.kind.kind_name().to_string());
        Ok(())
    }
}

#[tokio::test]
async fn the_milestone_sink_decorator_records_durably_before_it_composes() {
    let fixture = fixture("decorator");
    let inner = Arc::new(InnerSink::default());
    let sink = ReplyProjectionMilestoneSink::new(
        Arc::clone(&inner) as Arc<dyn LoopHostMilestoneSink>,
        Arc::clone(&fixture.projection),
    );
    sink.publish_loop_milestone(fixture.milestone(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "hello".to_string(),
    }))
    .await
    .unwrap();
    assert_eq!(fixture.document().answer.text.as_str(), "hello");
    assert_eq!(inner.seen.lock().unwrap().len(), 1);

    // The durable record is the truth: when it refuses the milestone, the
    // projection does not move either.
    inner.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let error = sink
        .publish_loop_milestone(fixture.milestone(LoopHostMilestoneKind::ModelTextDelta {
            safe_text: " world".to_string(),
        }))
        .await
        .unwrap_err();
    assert_eq!(
        error.kind,
        ironclaw_loop_contracts::AgentLoopHostErrorKind::Invalid
    );
    assert_eq!(fixture.document().answer.text.as_str(), "hello");
}

#[test]
fn tracked_runs_are_bounded_and_evictable() {
    let fixture = fixture("bounded");
    let projection = ReplyProjection::with_capacity(2);
    let other_runs: Vec<TurnRunId> = (0..3).map(|_| TurnRunId::new()).collect();
    for run_id in &other_runs {
        projection.observe_milestone(&LoopHostMilestone {
            run_id: *run_id,
            ..fixture.milestone(LoopHostMilestoneKind::ModelStarted {
                requested_model_profile_id: None,
            })
        });
    }
    assert!(projection.snapshot(&fixture.scope, other_runs[0]).is_some());
    assert!(projection.snapshot(&fixture.scope, other_runs[1]).is_some());
    assert!(
        projection.snapshot(&fixture.scope, other_runs[2]).is_none(),
        "beyond capacity a run's live facets are not tracked (its terminal reply still is)"
    );
    // Terminal facts always land — the answer must never be lost to a
    // capacity bound — and eviction frees the slot.
    let snapshot = projection.apply_terminal_facts(
        &fixture.scope,
        other_runs[2],
        TerminalReplyFacts {
            actor: None,
            status: TurnStatus::Completed,
            nothing_to_report: false,
            answer: Some("done".to_string()),
            attachments: Vec::new(),
            failure_summary: None,
        },
    );
    assert!(snapshot.document.is_terminal());
    projection.evict(&fixture.scope, other_runs[0]);
    assert!(projection.snapshot(&fixture.scope, other_runs[0]).is_none());
    // Failure milestones flag terminal-pending exactly like completions.
    projection.observe_milestone(&LoopHostMilestone {
        run_id: other_runs[1],
        ..fixture.milestone(LoopHostMilestoneKind::Failed {
            reason_kind: LoopFailureKind::ModelError,
            exit_id: LoopExitId::new("exit:test").unwrap(),
        })
    });
    assert!(
        projection
            .snapshot(&fixture.scope, other_runs[1])
            .unwrap()
            .terminal_pending
    );
}

#[test]
fn a_revision_floor_seeds_numbering_for_a_run_rebuilt_on_another_process() {
    let fixture = fixture("floor");
    fixture
        .projection
        .raise_revision_floor(&fixture.scope, fixture.run_id, 7);
    let snapshot = fixture.projection.apply_terminal_facts(
        &fixture.scope,
        fixture.run_id,
        TerminalReplyFacts {
            actor: Some(fixture.actor.clone()),
            status: TurnStatus::Completed,
            nothing_to_report: false,
            answer: Some("rebuilt".to_string()),
            attachments: Vec::new(),
            failure_summary: None,
        },
    );
    assert_eq!(
        snapshot.revision, 8,
        "the terminal revision numbers above what the store already saw"
    );
    // A floor never lowers a live run's numbering.
    fixture
        .projection
        .raise_revision_floor(&fixture.scope, fixture.run_id, 2);
    assert_eq!(
        fixture
            .projection
            .snapshot(&fixture.scope, fixture.run_id)
            .unwrap()
            .revision,
        8
    );
}

/// The answer is the text of the current model call, nothing else. The
/// moment the loop does anything after a call other than end the run — here
/// a capability invocation — that call's text was narration ("I’ll research
/// this first."): it moves to the document's narration under its phase and
/// the answer starts empty as the next phase. This is the transcript's own
/// rule (only the final assistant message is the answer), applied
/// progressively.
#[test]
fn a_finished_calls_text_becomes_narration_once_a_capability_follows_it() {
    let fixture = fixture("narration-capability");
    let activity_id = CapabilityActivityId::new();
    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "I’ll research".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "I’ll research this first.".to_string(),
    });
    assert_eq!(
        fixture.document().answer.text.as_str(),
        "I’ll research this first.",
        "the current call's cumulative text is the provisional answer"
    );
    fixture.observe(LoopHostMilestoneKind::ModelCompleted {
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("test-model")
            .unwrap(),
    });
    assert_eq!(
        fixture.document().answer.text.as_str(),
        "I’ll research this first.",
        "a completed call stays the answer until the loop continues"
    );

    fixture.observe(LoopHostMilestoneKind::CapabilityInvoked {
        activity_id,
        capability_id: CapabilityId::new("acme.search").unwrap(),
    });
    let document = fixture.document();
    assert_eq!(
        document.answer.text.as_str(),
        "",
        "the tool call proves the text was narration"
    );
    assert_eq!(
        document.answer.phase, 2,
        "the answer moved on to the next phase"
    );
    assert_eq!(
        narration(&document),
        vec![(1, "I’ll research this first.".to_string())],
        "the narration is kept under the phase it streamed as"
    );
    assert!(document.reasoning.is_empty(), "narration is not reasoning");
    assert_eq!(document.activities.len(), 1);

    // The next call's cumulative text is the answer again; a call that
    // restarts its text (shorter cumulative text) is a rewrite, not an
    // append, and not a new call.
    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Here is".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Here is the final answer.".to_string(),
    });
    assert_eq!(
        fixture.document().answer.text.as_str(),
        "Here is the final answer."
    );
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Actually, here it is.".to_string(),
    });
    let document = fixture.document();
    assert_eq!(document.answer.text.as_str(), "Actually, here it is.");
    assert_eq!(document.answer.phase, 2);
    assert_eq!(document.narration.len(), 1);
}

/// A call that ended without a tool call but did not end the run (the loop
/// nudged the model on, or drained queued follow-up input) is narration too:
/// the next call starting demotes it.
#[test]
fn a_finished_calls_text_becomes_narration_once_another_call_follows_it() {
    let fixture = fixture("narration-next-call");
    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Let me write the file:".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelCompleted {
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("test-model")
            .unwrap(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    let document = fixture.document();
    assert_eq!(document.answer.text.as_str(), "");
    assert_eq!(
        narration(&document),
        vec![(1, "Let me write the file:".to_string())]
    );
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Done.".to_string(),
    });
    let document = fixture.document();
    assert_eq!(document.answer.text.as_str(), "Done.");
    assert_eq!(document.answer.phase, 2);
}

/// Text ahead of a gate is narration by construction: the gate comes from a
/// tool call the same model call produced. It is demoted when the gate
/// blocks, so no surface shows it beside the attention block.
#[test]
fn text_ahead_of_a_gate_is_narration_not_answer() {
    let fixture = fixture("narration-gate");
    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Let me send the summary.".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelCompleted {
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("test-model")
            .unwrap(),
    });
    fixture.observe(LoopHostMilestoneKind::GateBlocked {
        iteration: 1,
        gate_kind: LoopGateKind::Approval,
    });
    let document = fixture.document();
    assert_eq!(document.answer.text.as_str(), "");
    assert_eq!(
        narration(&document),
        vec![(1, "Let me send the summary.".to_string())]
    );
    assert_eq!(
        document.attention.as_ref().map(|attention| attention.kind),
        Some(ReplyAttentionKind::Approval)
    );
    fixture.observe(LoopHostMilestoneKind::Blocked {
        gate_ref: LoopGateRef::new("gate:approval-1").unwrap(),
        checkpoint_id: TurnCheckpointId::new(),
    });
    let document = fixture.document();
    assert_eq!(document.answer.text.as_str(), "");
    assert_eq!(document.narration.len(), 1, "the reset is idempotent");
    assert_eq!(document.answer.phase, 2);
}

/// Narration is bounded like every facet of this display document (the
/// durable transcript keeps every model call in full): at most
/// `REPLY_MAX_NARRATION_ENTRIES` entries, each cut to
/// `REPLY_NARRATION_ENTRY_MAX_BYTES` on a character boundary. The answer
/// still resets and moves to the next phase past the bound.
#[test]
fn narration_is_bounded_like_reasoning() {
    use ironclaw_extension_contracts::reply::{
        REPLY_MAX_NARRATION_ENTRIES, REPLY_NARRATION_ENTRY_MAX_BYTES,
    };
    let fixture = fixture("narration-bounds");
    let long = "é".repeat(REPLY_NARRATION_ENTRY_MAX_BYTES);
    for phase in 1..=(REPLY_MAX_NARRATION_ENTRIES as u64 + 1) {
        fixture.observe(LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None,
        });
        fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
            safe_text: if phase == 1 {
                long.clone()
            } else {
                format!("phase {phase}")
            },
        });
        fixture.observe(LoopHostMilestoneKind::CapabilityInvoked {
            activity_id: CapabilityActivityId::new(),
            capability_id: CapabilityId::new("acme.search").unwrap(),
        });
    }
    let document = fixture.document();
    assert_eq!(document.narration.len(), REPLY_MAX_NARRATION_ENTRIES);
    assert_eq!(
        document.narration[0].text.as_str().len(),
        REPLY_NARRATION_ENTRY_MAX_BYTES,
        "cut to the byte bound on a character boundary"
    );
    assert!(
        document.narration[0]
            .text
            .as_str()
            .chars()
            .all(|c| c == 'é')
    );
    assert_eq!(
        document.narration.last().map(|entry| entry.text.as_str()),
        Some("phase 32"),
        "the entry past the bound is dropped"
    );
    assert_eq!(document.answer.text.as_str(), "");
    assert_eq!(
        document.answer.phase,
        REPLY_MAX_NARRATION_ENTRIES as u64 + 2,
        "the answer still moves on past the bound"
    );
}

/// A call's provider reasoning stays reasoning and its narration stays
/// narration: two facets, never merged.
#[test]
fn narration_is_kept_apart_from_the_calls_reasoning() {
    let fixture = fixture("narration-order");
    fixture.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    fixture.observe(LoopHostMilestoneKind::ModelReasoningDelta {
        safe_delta: "Looking at the repo".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Let me check.".to_string(),
    });
    fixture.observe(LoopHostMilestoneKind::CapabilityInvoked {
        activity_id: CapabilityActivityId::new(),
        capability_id: CapabilityId::new("acme.search").unwrap(),
    });
    let document = fixture.document();
    assert_eq!(
        document
            .reasoning
            .iter()
            .map(|segment| segment.as_str())
            .collect::<Vec<_>>(),
        vec!["Looking at the repo"]
    );
    assert!(!document.reasoning_open);
    assert_eq!(narration(&document), vec![(1, "Let me check.".to_string())]);
    assert_eq!(document.answer.text.as_str(), "");
}

/// The durable transcript finalizes only the run's final assistant message,
/// which is exactly the final call's streamed text: finalization converges
/// IN PLACE (no rewrite), so no stream presentation sees a prefix break at
/// the terminal. The earlier call's text is already narration.
#[test]
fn terminal_facts_matching_the_final_calls_text_finalize_in_place() {
    let live = fixture("terminal-in-place");
    live.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    live.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Let me check the workspace.".to_string(),
    });
    live.observe(LoopHostMilestoneKind::ModelCompleted {
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("scripted")
            .unwrap(),
    });
    live.observe(LoopHostMilestoneKind::ModelStarted {
        requested_model_profile_id: None,
    });
    live.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "The answer is 42.".to_string(),
    });
    live.observe(LoopHostMilestoneKind::ModelCompleted {
        effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("scripted")
            .unwrap(),
    });
    let document = live.document();
    assert_eq!(
        document.answer.text.as_str(),
        "The answer is 42.",
        "the progressive answer is the final call's text alone"
    );
    assert_eq!(document.answer.phase, 2);
    assert_eq!(
        narration(&document),
        vec![(1, "Let me check the workspace.".to_string())]
    );

    let revision_before = live
        .projection
        .snapshot(&live.scope, live.run_id)
        .expect("tracked")
        .revision;
    let snapshot = live.projection.apply_terminal_facts(
        &live.scope,
        live.run_id,
        TerminalReplyFacts {
            actor: Some(live.actor.clone()),
            status: TurnStatus::Completed,
            nothing_to_report: false,
            answer: Some("The answer is 42.".to_string()),
            attachments: Vec::new(),
            failure_summary: None,
        },
    );
    let document = &snapshot.document;
    assert!(document.answer.finalized);
    assert_eq!(document.outcome, Some(ReplyOutcome::Completed));
    assert_eq!(document.answer.text.as_str(), "The answer is 42.");
    assert_eq!(
        document.answer.phase, 2,
        "finalization keeps the final phase"
    );
    assert_eq!(
        document.narration.len(),
        1,
        "finalization leaves the narration alone"
    );
    assert_eq!(snapshot.revision, revision_before + 1);
}

/// The empty-transcript completion (`answer: None` — nothing to report, or a
/// row without content) must not blank streamed text at terminal either:
/// the empty canonical is trivially contained in whatever is shown.
#[test]
fn terminal_facts_without_an_answer_keep_the_streamed_text() {
    let live = fixture("terminal-keep");
    live.observe(LoopHostMilestoneKind::ModelTextDelta {
        safe_text: "Streamed but never persisted.".to_string(),
    });
    let snapshot = live.projection.apply_terminal_facts(
        &live.scope,
        live.run_id,
        TerminalReplyFacts {
            actor: Some(live.actor.clone()),
            status: TurnStatus::Completed,
            nothing_to_report: false,
            answer: None,
            attachments: Vec::new(),
            failure_summary: None,
        },
    );
    assert!(snapshot.document.answer.finalized);
    assert_eq!(
        snapshot.document.answer.text.as_str(),
        "Streamed but never persisted.",
        "an absent canonical answer never erases what the user already saw"
    );
}
