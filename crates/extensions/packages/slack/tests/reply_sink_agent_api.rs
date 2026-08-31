//! The Slack reply sink against an in-crate fake of Slack's native Agent API
//! (`support/fake_slack_agent_api.rs`): exact request bodies on the happy
//! path, idempotent repeats, char-boundary deltas, rate-limit and ambiguity
//! handling with read-back, the stop button, generation and checkpoint
//! version changes, the no-fallback rule, channel recipient requirements,
//! the conventional terminal post, liveness re-assertion, and terminal
//! attachments.

mod support;

use std::time::Duration;

use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::reply::{
    ReplyActivityState, ReplyAnswerText, ReplyAttention, ReplyAttentionKind, ReplyAudience,
    ReplyContextBytes, ReplyDisplayPreview, ReplyDisplayText, ReplyDocument, ReplyItemId,
    ReplyPhase, ReplyReconcilePoint, ReplyReconcileRequest, ReplyRevision, ReplySink,
    ReplySinkCheckpoint, ReplySinkOutcome, ReplySinkReport, ReplyTarget,
};
use ironclaw_host_api::attachment::WorkspaceFile;
use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
use ironclaw_host_api::path::ScopedPath;
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope};
use ironclaw_slack_extension::{
    SLACK_REPLY_CHECKPOINT_VERSION, SlackChannelAdapter, SlackReplyContext, SlackWebApiMethod,
};
use serde_json::{Value, json};
use support::fake_slack_agent_api::{FakeSlackAgentApi, Fault, StreamState};

const CHANNEL: &str = "C123";
const DM: &str = "D123";
const THREAD: &str = "1710000000.000100";
const USER: &str = "U123";
const TEAM: &str = "T-A";

fn text(value: &str) -> ReplyDisplayText {
    ReplyDisplayText::new(value).expect("display text")
}

fn preview(value: &str) -> ReplyDisplayPreview {
    ReplyDisplayPreview::new(value).expect("preview")
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

/// One reply's life against the fake: the document accumulates changes, the
/// checkpoint round-trips exactly as the host would round-trip it.
struct Harness {
    fake: FakeSlackAgentApi,
    target: ReplyTarget,
    reply_context: Option<ReplyContextBytes>,
    checkpoint: Option<ReplySinkCheckpoint>,
    generation: u64,
    attachments: Vec<WorkspaceFile>,
    document: ReplyDocument,
    revision: u64,
}

impl Harness {
    fn new(channel: &str, is_dm: bool, with_context: bool) -> Self {
        let run_id = TurnRunId::new();
        let conversation =
            ExternalConversationRef::new(Some(TEAM), channel, Some(THREAD), Some(THREAD))
                .expect("conversation");
        let target = ReplyTarget {
            scope: scope(),
            actor: TurnActor::new(UserId::new("user-1").expect("user")),
            run_id,
            conversation: Some(conversation),
            thread_anchor: None,
            audience: if is_dm {
                ReplyAudience::Private
            } else {
                ReplyAudience::Shared
            },
        };
        let reply_context = with_context.then(|| {
            ReplyContextBytes::new(
                SlackReplyContext {
                    team_id: Some(TEAM.to_string()),
                    channel: channel.to_string(),
                    thread_ts: Some(THREAD.to_string()),
                    user: USER.to_string(),
                    is_dm,
                }
                .to_bytes()
                .expect("context serializes"),
            )
            .expect("context within bound")
        });
        Self {
            fake: FakeSlackAgentApi::new(),
            target,
            reply_context,
            checkpoint: None,
            generation: 1,
            attachments: Vec::new(),
            document: ReplyDocument::default(),
            revision: 0,
        }
    }

    fn channel() -> Self {
        Self::new(CHANNEL, false, true)
    }

    fn dm() -> Self {
        Self::new(DM, true, true)
    }

    fn append(&mut self, text: &str) {
        self.document.append_answer(text);
    }

    async fn reconcile(&mut self, point: ReplyReconcilePoint) -> ReplySinkReport {
        self.revision += 1;
        let request = ReplyReconcileRequest {
            revision: ReplyRevision {
                revision: self.revision,
                document: self.document.clone(),
            },
            point,
            target: self.target.clone(),
            reply_context: self.reply_context.clone(),
            checkpoint: self.checkpoint.clone(),
            extension_generation: self.generation,
            materialized_attachments: if point == ReplyReconcilePoint::Terminal {
                self.attachments.clone()
            } else {
                Vec::new()
            },
        };
        let report = SlackChannelAdapter
            .reconcile(request, &self.fake)
            .await
            .expect("provider outcomes are reports, never errors");
        if let Some(checkpoint) = &report.checkpoint {
            self.checkpoint = Some(checkpoint.clone());
        }
        report
    }

    fn stream_ts(&self) -> String {
        self.fake
            .streams()
            .first()
            .expect("a stream was opened")
            .0
            .clone()
    }

    fn checkpoint_json(&self) -> Value {
        serde_json::from_str(self.checkpoint.as_ref().expect("checkpoint").payload())
            .expect("checkpoint json")
    }

    fn set_checkpoint_json(&mut self, value: Value) {
        self.checkpoint = Some(
            ReplySinkCheckpoint::new(SLACK_REPLY_CHECKPOINT_VERSION, value.to_string())
                .expect("checkpoint within bound"),
        );
    }
}

fn assert_applied(report: &ReplySinkReport) {
    assert!(
        report.outcome.is_applied(),
        "expected Applied, got {:?}",
        report.outcome
    );
}

fn refs(report: &ReplySinkReport) -> Vec<String> {
    report
        .evidence
        .provider_refs
        .iter()
        .map(|reference| reference.as_str().to_string())
        .collect()
}

// ── Happy path ───────────────────────────────────────────────────────────

#[tokio::test]
async fn happy_path_streams_the_reply_through_the_native_agent_surface() {
    let mut harness = Harness::channel();

    // Opened: a status line, no answer yet → session processing + stream.
    harness.document.set_status(text("Thinking…"), None);
    let report = harness.reconcile(ReplyReconcilePoint::Opened).await;
    assert_applied(&report);
    let ts = harness.stream_ts();
    assert_eq!(
        harness.fake.calls(),
        ["agents.sessions.setStatus", "chat.startStream"]
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::AgentsSessionsSetStatus),
        [json!({ "status": "processing", "channel_id": CHANNEL, "thread_ts": THREAD })]
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream),
        [json!({
            "channel": CHANNEL,
            "thread_ts": THREAD,
            "recipient_user_id": USER,
            "recipient_team_id": TEAM,
            "task_display_mode": "timeline",
            "chunks": [{ "type": "markdown_text", "text": "_Thinking…_\n" }]
        })]
    );
    assert_eq!(
        refs(&report),
        [ts.as_str()],
        "the stream ts is the evidence"
    );
    assert!(
        !report.evidence.read_back_verified,
        "nothing was read back on the happy path"
    );

    // ControlCritical: attention before any text → block + suspended.
    harness.document.require_attention(ReplyAttention {
        kind: ReplyAttentionKind::Approval,
        headline: text("Approve writing report.md"),
        body: Some(preview(
            "The run wants to write report.md in your workspace.",
        )),
        action_url: None,
        gate_ref: Some(text("gate:approval-1")),
    });
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );
    assert_eq!(
        harness.fake.calls()[2..],
        ["chat.appendStream", "agents.sessions.setStatus"]
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatAppendStream)[0],
        json!({
            "channel": CHANNEL,
            "ts": ts,
            "chunks": [{
                "type": "markdown_text",
                "text": "\n> **Approval needed:** Approve writing report.md\n> The run wants to write report.md in your workspace.\n"
            }]
        })
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::AgentsSessionsSetStatus)[1],
        json!({ "status": "suspended", "channel_id": CHANNEL, "thread_ts": THREAD })
    );

    // Cleared → processing again, nothing appended.
    harness.document.clear_attention();
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );
    assert_eq!(harness.fake.calls()[4..], ["agents.sessions.setStatus"]);
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::AgentsSessionsSetStatus)[2],
        json!({ "status": "processing", "channel_id": CHANNEL, "thread_ts": THREAD })
    );

    // Progress: an activity starts and the answer begins → one append.
    harness.document.activity_started(
        item("act-1"),
        text("Read runbook"),
        Some(preview("docs/runbook.md")),
    );
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(harness.fake.calls()[5..], ["chat.appendStream"]);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatAppendStream)[1],
        json!({
            "channel": CHANNEL,
            "ts": ts,
            "chunks": [
                { "type": "task_update", "id": "act-1", "title": "Read runbook", "status": "in_progress", "details": "docs/runbook.md" },
                { "type": "markdown_text", "text": "Hello" }
            ]
        })
    );

    // Progress: the activity finishes with output, more text → one append.
    harness.document.activity_finished(
        item("act-1"),
        ReplyActivityState::Completed,
        Some(preview("12 lines")),
        None,
    );
    harness.append(" world");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(harness.fake.calls()[6..], ["chat.appendStream"]);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatAppendStream)[2],
        json!({
            "channel": CHANNEL,
            "ts": ts,
            "chunks": [
                { "type": "task_update", "id": "act-1", "title": "Read runbook", "status": "complete", "details": "docs/runbook.md", "output": "12 lines" },
                { "type": "markdown_text", "text": " world" }
            ]
        })
    );

    // Terminal: the canonical answer extends the streamed text → one stop
    // carrying the remaining delta and `session_status: active`.
    harness
        .document
        .finalize_answer(answer("Hello world!"), Vec::new());
    harness.document.complete();
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    assert_eq!(harness.fake.calls()[7..], ["chat.stopStream"]);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStopStream),
        [json!({
            "channel": CHANNEL,
            "ts": ts,
            "chunks": [{ "type": "markdown_text", "text": "!" }],
            "session_status": "active"
        })]
    );
    assert_eq!(refs(&report), [ts.as_str()]);

    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(
        stream.state,
        StreamState::Stopped {
            session_status: "active".to_string()
        }
    );
    assert_eq!(
        stream.text,
        "_Thinking…_\n\n> **Approval needed:** Approve writing report.md\n> The run wants to write report.md in your workspace.\nHello world!"
    );
    assert_eq!(stream.task_updates.len(), 2);
    assert!(
        harness.fake.posted().is_empty(),
        "the stream path never posts a conventional message"
    );

    let checkpoint = harness.checkpoint_json();
    assert_eq!(checkpoint["terminal"], "applied");
    assert_eq!(checkpoint["session_status"], "active");
    assert_eq!(checkpoint["stream"]["ts"], ts);
    assert_eq!(checkpoint["stream"]["appended_chars"], 12);
    assert_eq!(checkpoint["generation"], 1);
    assert!(checkpoint["tasks"]["act-1"].is_string());
}

// ── Idempotency ──────────────────────────────────────────────────────────

#[tokio::test]
async fn repeated_revisions_and_terminals_make_no_calls() {
    let mut harness = Harness::dm();
    harness.document.note_phase(ReplyPhase::Working);
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    assert_eq!(harness.fake.calls().len(), 2);

    // The same desired state again (a retry, a heartbeat, a lease takeover).
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Heartbeat).await);
    assert_eq!(
        harness.fake.calls().len(),
        2,
        "a reflected revision costs no provider call"
    );

    harness
        .document
        .finalize_answer(answer("Hello"), Vec::new());
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    let ts = harness.stream_ts();
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStopStream),
        [json!({ "channel": DM, "ts": ts, "session_status": "active" })],
        "nothing left to append: the stop carries no chunks"
    );
    let calls = harness.fake.calls().len();

    let repeat = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&repeat);
    assert_eq!(
        harness.fake.calls().len(),
        calls,
        "a repeated terminal is a no-op"
    );
    assert_eq!(harness.fake.stream(&ts).expect("stream").text, "Hello");
}

// ── Char boundaries ──────────────────────────────────────────────────────

#[tokio::test]
async fn text_deltas_are_char_offsets_never_byte_slices() {
    let mut harness = Harness::dm();
    harness.append("hé");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    harness.append("llo 世界");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    harness.append("!");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);

    let ts = harness.stream_ts();
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream)[0]["chunks"],
        json!([{ "type": "markdown_text", "text": "hé" }])
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatAppendStream),
        [
            json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": "llo 世界" }] }),
            json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": "!" }] }),
        ]
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "héllo 世界!"
    );
    assert_eq!(harness.checkpoint_json()["stream"]["appended_chars"], 9);
}

// ── Rate limits ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_rate_limited_append_is_retryable_with_the_provider_hint_and_never_duplicates() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    harness.fake.inject(Fault::RateLimited {
        method: SlackWebApiMethod::ChatAppendStream,
        retry_after: Duration::from_secs(7),
    });
    harness.append(" world");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(
            &report.outcome,
            ReplySinkOutcome::Retryable { retry_after: Some(hint), .. } if *hint == Duration::from_secs(7)
        ),
        "429 + Retry-After must be Retryable with the hint, got {:?}",
        report.outcome
    );
    assert_eq!(
        harness.checkpoint_json()["stream"]["appended_chars"],
        5,
        "a refused append advances nothing"
    );

    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    let ts = harness.stream_ts();
    let appends = harness.fake.bodies(SlackWebApiMethod::ChatAppendStream);
    assert_eq!(appends.len(), 2);
    assert_eq!(
        appends[0], appends[1],
        "the retry re-sends exactly the refused delta"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello world"
    );

    harness.fake.inject(Fault::ServerError {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append("!");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(
            &report.outcome,
            ReplySinkOutcome::Retryable {
                retry_after: None,
                ..
            }
        ),
        "a 5xx is Retryable without a hint, got {:?}",
        report.outcome
    );
}

// ── Ambiguity and read-back ──────────────────────────────────────────────

#[tokio::test]
async fn a_transport_failure_after_an_append_is_ambiguous_and_read_back_decides_the_continuation() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    // The append reached Slack; only the answer was lost.
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    assert!(
        harness.checkpoint_json()["stream"]["pending"].is_object(),
        "the checkpoint remembers the unanswered request"
    );

    harness.append("!");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(
        report.evidence.read_back_verified,
        "the read-back proved the ambiguous append landed"
    );
    let calls = harness.fake.calls();
    assert_eq!(
        calls[calls.len() - 2..],
        ["conversations.replies", "chat.appendStream"],
        "read back BEFORE appending more"
    );
    let read_back = harness
        .fake
        .requests()
        .into_iter()
        .find(|request| request.url.contains("conversations.replies"))
        .expect("read-back request");
    assert!(
        read_back.url.contains(&format!("channel={DM}"))
            && read_back.url.contains(&format!("ts={ts}")),
        "read-back addresses the streaming message: {}",
        read_back.url
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .last(),
        Some(
            &json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": "!" }] })
        ),
        "only the NEW delta is appended; the landed one is not repeated"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello world!"
    );

    // The append never reached Slack.
    harness.fake.inject(Fault::TransportBeforeAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" Bye");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }));

    harness.append(".");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(
        !report.evidence.read_back_verified,
        "a read-back that shows the append missing verifies nothing"
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .last(),
        Some(
            &json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": " Bye." }] })
        ),
        "the lost delta is re-sent together with the new one"
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello world! Bye."
    );
}

// ── The stop button ──────────────────────────────────────────────────────

#[tokio::test]
async fn stopped_by_user_maps_to_stopped_by_user_and_a_cancelled_terminal_settles_the_session() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness.fake.stop_by_user(&ts);
    harness.append(" world");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_eq!(report.outcome, ReplySinkOutcome::StoppedByUser);

    // The host cancels the run; the terminal must transition the session
    // itself ("The session status does not update automatically when the
    // user clicks stop") without failing on the already-ended stream.
    harness.document.cancel();
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    let calls = harness.fake.calls();
    assert_eq!(
        calls[calls.len() - 2..],
        ["chat.stopStream", "agents.sessions.setStatus"]
    );
    assert_eq!(
        harness
            .fake
            .sessions()
            .last()
            .map(|call| call.status.as_str()),
        Some("active")
    );
    assert_eq!(harness.checkpoint_json()["terminal"], "applied");
    assert!(harness.fake.posted().is_empty());

    // A repeated terminal after settling is silent.
    let before = harness.fake.calls().len();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert_eq!(harness.fake.calls().len(), before);
}

// ── Fresh presentations ──────────────────────────────────────────────────

#[tokio::test]
async fn a_generation_change_starts_a_fresh_presentation() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    harness.generation = 2;
    harness.append(" world");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    let calls = harness.fake.calls();
    assert_eq!(
        calls[calls.len() - 2..],
        ["agents.sessions.setStatus", "chat.startStream"],
        "a checkpoint minted under another generation is never appended to"
    );
    let streams = harness.fake.streams();
    assert_eq!(streams.len(), 2);
    assert_eq!(
        streams[1].1.text, "Hello world",
        "the fresh stream carries the whole answer"
    );
    assert_eq!(harness.checkpoint_json()["generation"], 2);
}

#[tokio::test]
async fn an_unknown_checkpoint_version_starts_a_fresh_presentation() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    let payload = harness
        .checkpoint
        .as_ref()
        .expect("checkpoint")
        .payload()
        .to_string();
    harness.checkpoint = Some(
        ReplySinkCheckpoint::new(SLACK_REPLY_CHECKPOINT_VERSION + 1, payload).expect("checkpoint"),
    );
    harness.append(" world");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(harness.fake.streams().len(), 2);
    assert_eq!(
        harness.checkpoint.as_ref().expect("checkpoint").version(),
        SLACK_REPLY_CHECKPOINT_VERSION,
        "the sink writes its own version back"
    );
}

#[tokio::test]
async fn an_answer_rewritten_under_the_stream_is_re_presented_in_full() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let first = harness.stream_ts();

    // The canonical text is not an extension of what was streamed.
    harness
        .document
        .finalize_answer(answer("Goodbye"), Vec::new());
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );
    let calls = harness.fake.calls();
    assert_eq!(
        calls[calls.len() - 2..],
        ["chat.stopStream", "chat.startStream"],
        "close the stale stream, open a fresh one"
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStopStream)[0],
        json!({ "channel": DM, "ts": first, "session_status": "processing" }),
        "re-presenting keeps the session processing"
    );
    let streams = harness.fake.streams();
    assert_eq!(streams.len(), 2);
    assert_eq!(streams[1].1.text, "Goodbye");
}

// ── No conventional fallback ─────────────────────────────────────────────

#[tokio::test]
async fn a_workspace_without_the_agent_feature_fails_clearly_with_no_fallback() {
    let mut harness = Harness::channel();
    harness.fake.inject(Fault::SlackError {
        method: SlackWebApiMethod::AgentsSessionsSetStatus,
        error: "feature_disabled",
    });
    harness.append("Hello");
    let report = harness.reconcile(ReplyReconcilePoint::Opened).await;
    let ReplySinkOutcome::Permanent { reason } = &report.outcome else {
        panic!("expected Permanent, got {:?}", report.outcome);
    };
    assert!(
        reason.as_str().contains("agent_view") && reason.as_str().contains("feature_disabled"),
        "the reason names the missing Slack Agent capability: {reason}"
    );
    assert_eq!(harness.fake.calls(), ["agents.sessions.setStatus"]);
    assert!(
        harness.fake.posted().is_empty(),
        "no chat.postMessage fallback"
    );
    assert!(harness.fake.streams().is_empty());

    // The same rule for a bot token missing the streaming scope.
    let mut harness = Harness::channel();
    harness.fake.inject(Fault::SlackError {
        method: SlackWebApiMethod::ChatStartStream,
        error: "missing_scope",
    });
    harness.append("Hello");
    let report = harness.reconcile(ReplyReconcilePoint::Opened).await;
    let ReplySinkOutcome::Permanent { reason } = &report.outcome else {
        panic!("expected Permanent, got {:?}", report.outcome);
    };
    assert!(
        reason.as_str().contains("chat:write"),
        "the reason names the scope: {reason}"
    );
    assert!(harness.fake.posted().is_empty());
}

// ── Recipients ───────────────────────────────────────────────────────────

#[tokio::test]
async fn channel_streaming_requires_the_recipient_ids_from_the_reply_context() {
    // No stored reply context at all: nothing to stream as.
    let mut harness = Harness::new(CHANNEL, false, false);
    harness.append("Hello");
    let report = harness.reconcile(ReplyReconcilePoint::Opened).await;
    let ReplySinkOutcome::Permanent { reason } = &report.outcome else {
        panic!("expected Permanent, got {:?}", report.outcome);
    };
    assert!(reason.as_str().contains("recipient_user_id"), "{reason}");
    assert!(
        harness.fake.calls().is_empty(),
        "refused before any provider call"
    );

    // A context without the team id cannot satisfy recipient_team_id.
    let mut harness = Harness::channel();
    harness.reply_context = Some(
        ReplyContextBytes::new(
            SlackReplyContext {
                team_id: None,
                channel: CHANNEL.to_string(),
                thread_ts: Some(THREAD.to_string()),
                user: USER.to_string(),
                is_dm: false,
            }
            .to_bytes()
            .expect("serializes"),
        )
        .expect("bounded"),
    );
    harness.append("Hello");
    let report = harness.reconcile(ReplyReconcilePoint::Opened).await;
    let ReplySinkOutcome::Permanent { reason } = &report.outcome else {
        panic!("expected Permanent, got {:?}", report.outcome);
    };
    assert!(reason.as_str().contains("recipient_team_id"), "{reason}");
}

#[tokio::test]
async fn a_direct_message_streams_without_a_stored_reply_context() {
    let mut harness = Harness::new(DM, true, false);
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream),
        [json!({
            "channel": DM,
            "thread_ts": THREAD,
            "task_display_mode": "timeline",
            "chunks": [{ "type": "markdown_text", "text": "Hello" }]
        })],
        "recipient ids are optional in a DM and omitted when unknown"
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::AgentsSessionsSetStatus),
        [json!({ "status": "processing", "channel_id": DM, "thread_ts": THREAD })]
    );
}

// ── Terminal without a stream ────────────────────────────────────────────

#[tokio::test]
async fn a_terminal_without_an_open_stream_posts_the_text_and_activates_the_session() {
    // The run failed before its first revision reached Slack.
    let mut harness = Harness::dm();
    harness.document.fail(text("The model provider timed out."));
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    assert_eq!(
        harness.fake.calls(),
        ["chat.postMessage", "agents.sessions.setStatus"]
    );
    let posted = harness.fake.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].channel, DM);
    assert_eq!(posted[0].thread_ts.as_deref(), Some(THREAD));
    assert_eq!(
        posted[0].text, "*Failed:* The model provider timed out.",
        "the failure summary renders through the same mrkdwn path as any message"
    );
    assert_eq!(
        harness.fake.sessions(),
        [support::fake_slack_agent_api::SessionCall {
            channel_id: Some(DM.to_string()),
            thread_ts: Some(THREAD.to_string()),
            status: "active".to_string(),
        }]
    );
    assert_eq!(refs(&report), [posted[0].ts.clone()]);
    assert!(
        harness.fake.streams().is_empty(),
        "no stream is opened at the terminal"
    );
    assert_eq!(harness.checkpoint_json()["terminal"], "applied");

    // A completed answer that never streamed goes out the same way.
    let mut harness = Harness::dm();
    harness
        .document
        .finalize_answer(answer("Done — **two** files updated."), Vec::new());
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert_eq!(harness.fake.posted()[0].text, "Done — *two* files updated.");

    // A cancellation that never streamed leaves a one-line note.
    let mut harness = Harness::dm();
    harness.document.cancel();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert_eq!(harness.fake.posted()[0].text, "_Stopped._");
}

// ── Liveness ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_heartbeat_reasserts_processing_only_once_the_assertion_is_stale() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let before = harness.fake.calls().len();

    assert_applied(&harness.reconcile(ReplyReconcilePoint::Heartbeat).await);
    assert_eq!(
        harness.fake.calls().len(),
        before,
        "a fresh assertion needs no liveness call"
    );

    let mut checkpoint = harness.checkpoint_json();
    checkpoint["status_asserted_at"] =
        json!((chrono::Utc::now() - chrono::Duration::minutes(31)).to_rfc3339());
    harness.set_checkpoint_json(checkpoint);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Heartbeat).await);
    assert_eq!(
        harness.fake.calls()[before..],
        ["agents.sessions.setStatus"]
    );
    assert_eq!(
        harness
            .fake
            .sessions()
            .last()
            .map(|call| call.status.as_str()),
        Some("processing"),
        "processing sessions time out after an hour; re-assert past 30 minutes"
    );

    // Re-asserted: the next heartbeat is quiet again.
    let after = harness.fake.calls().len();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Heartbeat).await);
    assert_eq!(harness.fake.calls().len(), after);
}

// ── Attachments ──────────────────────────────────────────────────────────

#[tokio::test]
async fn terminal_attachments_upload_after_the_stream_closes() {
    let mut harness = Harness::dm();
    harness.attachments = vec![WorkspaceFile {
        path: ScopedPath::new("/workspace/report.txt").expect("path"),
        filename: Some("report.txt".to_string()),
        mime_type: "text/plain".to_string(),
        bytes: b"hello".to_vec(),
    }];
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    harness
        .document
        .finalize_answer(answer("Hello"), Vec::new());
    harness.document.complete();
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    assert_eq!(
        harness.fake.calls()[2..],
        [
            "chat.stopStream",
            "files.getUploadURLExternal",
            "upload",
            "files.completeUploadExternal",
            "files.info",
        ],
        "files go through the external upload flow once the stream is closed"
    );
    let completion = harness
        .fake
        .bodies(SlackWebApiMethod::FilesCompleteUploadExternal)
        .remove(0);
    assert_eq!(completion["channel_id"], DM);
    assert_eq!(completion["thread_ts"], THREAD);
    assert_eq!(completion["files"][0]["title"], "report.txt");
    let ts = harness.stream_ts();
    assert_eq!(refs(&report), [ts, "FAKE1".to_string()]);
    assert_eq!(harness.checkpoint_json()["attachments_delivered"], true);

    // A repeated terminal re-uploads nothing.
    let before = harness.fake.calls().len();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert_eq!(harness.fake.calls().len(), before);
}

// ── Outcome mapping ──────────────────────────────────────────────────────

#[tokio::test]
async fn slack_errors_map_to_the_documented_outcomes() {
    for (error, expected) in [
        ("invalid_auth", "unauthorized"),
        ("not_authed", "unauthorized"),
        ("token_revoked", "unauthorized"),
        ("account_inactive", "unauthorized"),
        ("channel_not_found", "permanent"),
        ("is_archived", "permanent"),
        ("message_not_in_streaming_state", "permanent"),
        ("message_not_owned_by_app", "permanent"),
        ("streaming_mode_mismatch", "permanent"),
        ("service_unavailable", "retryable"),
        ("internal_error", "retryable"),
        ("request_timeout", "retryable"),
        ("rate_limited", "retryable"),
        ("stopped_by_user", "stopped_by_user"),
    ] {
        let mut harness = Harness::dm();
        harness.append("Hello");
        assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
        harness.fake.inject(Fault::SlackError {
            method: SlackWebApiMethod::ChatAppendStream,
            error,
        });
        harness.append(" world");
        let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
        assert_eq!(
            report.outcome.kind_name(),
            expected,
            "{error} → {expected}, got {:?}",
            report.outcome
        );
        match &report.outcome {
            ReplySinkOutcome::Applied | ReplySinkOutcome::StoppedByUser => {}
            ReplySinkOutcome::Retryable { reason, .. }
            | ReplySinkOutcome::Ambiguous { reason }
            | ReplySinkOutcome::Permanent { reason }
            | ReplySinkOutcome::Unauthorized { reason } => {
                assert!(
                    reason.as_str().contains(error)
                        && reason.as_str().contains("chat.appendStream"),
                    "the reason names the method and the Slack error: {reason}"
                );
            }
        }
    }
}
