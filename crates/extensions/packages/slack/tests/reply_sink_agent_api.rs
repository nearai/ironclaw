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

#[path = "reply_sink_agent_api/read_back.rs"]
mod read_back;

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

    // Opened: the production first revision (`IterationStarted`) carries no
    // status, answer, activity, or attention — only `Preparing`. The session
    // shows `processing`, the checkpoint persists, and NO stream is opened:
    // Slack would render an empty Agent container.
    assert_eq!(harness.document.phase, ReplyPhase::Preparing);
    assert!(harness.document.status.is_none());
    assert!(harness.document.answer.text.as_str().is_empty());
    assert!(harness.document.activities.is_empty());
    assert!(harness.document.attention.is_none());
    let report = harness.reconcile(ReplyReconcilePoint::Opened).await;
    assert_applied(&report);
    assert_eq!(harness.fake.calls(), ["agents.sessions.setStatus"]);
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::AgentsSessionsSetStatus),
        [json!({ "status": "processing", "channel_id": CHANNEL, "thread_ts": THREAD })]
    );
    assert!(
        harness.fake.streams().is_empty(),
        "an empty first revision never opens a stream"
    );
    assert_eq!(
        harness.checkpoint_json()["session_status"],
        "processing",
        "the checkpoint persists even though no stream exists yet"
    );

    // The first renderable content (an activity plus the answer's first
    // characters) opens exactly one stream, carrying that content in the
    // initial `chat.startStream` request.
    harness.document.activity_started(
        item("act-0"),
        text("Read runbook"),
        Some(preview("docs/runbook.md")),
    );
    harness.append("Hi");
    let report = harness
        .reconcile(ReplyReconcilePoint::ControlCritical)
        .await;
    assert_applied(&report);
    let ts = harness.stream_ts();
    assert_eq!(harness.fake.calls()[1..], ["chat.startStream"]);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream),
        [json!({
            "channel": CHANNEL,
            "thread_ts": THREAD,
            "recipient_user_id": USER,
            "recipient_team_id": TEAM,
            "task_display_mode": "timeline",
            "chunks": [
                { "type": "task_update", "id": "act-0", "title": "Read runbook", "status": "in_progress", "details": "docs/runbook.md" },
                { "type": "markdown_text", "text": "Hi" }
            ]
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
        .finalize_answer(answer("HiHello world!"), Vec::new());
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
        "Hi\n> **Approval needed:** Approve writing report.md\n> The run wants to write report.md in your workspace.\nHello world!"
    );
    assert_eq!(stream.task_updates.len(), 3);
    assert_eq!(
        harness.fake.streams().len(),
        1,
        "one logical reply means exactly one Agent stream"
    );
    assert!(
        harness.fake.posted().is_empty(),
        "the stream path never posts a conventional message"
    );

    let checkpoint = harness.checkpoint_json();
    assert_eq!(checkpoint["terminal"], "applied");
    assert_eq!(checkpoint["session_status"], "active");
    assert_eq!(checkpoint["stream"]["ts"], ts);
    assert_eq!(checkpoint["stream"]["appended_chars"], 14);
    assert_eq!(checkpoint["generation"], 1);
    assert!(checkpoint["tasks"]["act-1"].is_string());
}

/// SYNTHETIC-STATE unit test of chunk planning, not a production path: the
/// production first revision carries no status (see the happy path above).
/// This pins the narrow contract that an explicitly supplied driver status
/// (e.g. "retrying model request") renders as an italic markdown chunk in
/// the stream that opens for it.
#[tokio::test]
async fn an_explicit_driver_status_becomes_an_italic_opening_chunk() {
    let mut harness = Harness::channel();
    harness
        .document
        .set_status(text("retrying model request"), None);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    let bodies = harness.fake.bodies(SlackWebApiMethod::ChatStartStream);
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        bodies[0]["chunks"],
        json!([{ "type": "markdown_text", "text": "_retrying model request_\n" }])
    );
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
async fn a_terminal_without_an_open_stream_opens_and_closes_one_native_stream() {
    // The run failed before any renderable content reached Slack: the
    // terminal content still goes out on the native Agent surface — one
    // stream created and closed in the same reconcile — never as a
    // conventional `chat.postMessage`.
    let mut harness = Harness::dm();
    harness.document.fail(text("The model provider timed out."));
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    assert_eq!(
        harness.fake.calls(),
        ["chat.startStream", "chat.stopStream"]
    );
    let ts = harness.stream_ts();
    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(
        stream.state,
        StreamState::Stopped {
            session_status: "active".to_string()
        }
    );
    assert_eq!(
        stream.text, "**Failed:** The model provider timed out.",
        "the failure summary rides the one native stream"
    );
    assert_eq!(refs(&report), [ts.as_str()]);
    assert!(
        harness.fake.posted().is_empty(),
        "the terminal answer is never posted conventionally"
    );
    assert_eq!(harness.fake.streams().len(), 1);
    assert_eq!(harness.checkpoint_json()["terminal"], "applied");

    // A completed answer that never streamed goes out the same way.
    let mut harness = Harness::dm();
    harness
        .document
        .finalize_answer(answer("Done — **two** files updated."), Vec::new());
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert!(harness.fake.posted().is_empty());
    assert_eq!(harness.fake.streams().len(), 1);
    assert_eq!(
        harness
            .fake
            .stream(&harness.stream_ts())
            .expect("stream")
            .text,
        "Done — **two** files updated.",
        "markdown rides the stream untranslated; Slack renders markdown_text chunks"
    );

    // A cancellation that never streamed leaves a one-line note.
    let mut harness = Harness::dm();
    harness.document.cancel();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert!(harness.fake.posted().is_empty());
    assert_eq!(
        harness
            .fake
            .stream(&harness.stream_ts())
            .expect("stream")
            .text,
        "_Stopped._"
    );

    // A completed run with nothing to report opens nothing at all: there is
    // no content to show, so no stream and no message — only the session
    // settling to `active`.
    let mut harness = Harness::dm();
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert_eq!(harness.fake.calls(), ["agents.sessions.setStatus"]);
    assert!(harness.fake.streams().is_empty());
    assert!(harness.fake.posted().is_empty());
}

/// A terminal whose canonical answer is NOT an extension of the streamed
/// text (a genuine mid-stream rewrite): the stale stream is closed as it
/// stands and the canonical answer goes out on ONE fresh native stream —
/// never as a conventional message beside the stream.
#[tokio::test]
async fn a_terminal_rewrite_re_presents_natively_without_a_conventional_post() {
    let mut harness = Harness::dm();
    harness.append("The old draft answer");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let stale_ts = harness.stream_ts();

    harness
        .document
        .finalize_answer(answer("A different canonical answer."), Vec::new());
    harness.document.complete();
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    assert!(
        harness.fake.posted().is_empty(),
        "a terminal rewrite never posts the answer conventionally"
    );
    let streams = harness.fake.streams();
    assert_eq!(streams.len(), 2, "stale stream + one fresh terminal stream");
    let fresh_ts = streams[1].0.clone();
    assert_ne!(fresh_ts, stale_ts);
    let fresh = harness.fake.stream(&fresh_ts).expect("fresh stream");
    assert_eq!(
        fresh.state,
        StreamState::Stopped {
            session_status: "active".to_string()
        }
    );
    assert_eq!(fresh.text, "A different canonical answer.");
    assert_eq!(harness.checkpoint_json()["terminal"], "applied");
}

/// First renderable content = an attention block (a gate raised before any
/// text): the one stream opens carrying it.
#[tokio::test]
async fn an_attention_block_as_first_content_opens_the_stream_carrying_it() {
    let mut harness = Harness::dm();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    assert!(harness.fake.streams().is_empty());

    harness.document.require_attention(ReplyAttention {
        kind: ReplyAttentionKind::Auth,
        headline: text("Sign in to GitHub"),
        body: None,
        action_url: None,
        gate_ref: Some(text("gate:auth-1")),
    });
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );
    let bodies = harness.fake.bodies(SlackWebApiMethod::ChatStartStream);
    assert_eq!(bodies.len(), 1);
    let chunks = bodies[0]["chunks"].as_array().expect("chunks");
    assert!(
        !chunks.is_empty(),
        "the opening request carries the attention chunk"
    );
    assert!(
        chunks[0]["text"]
            .as_str()
            .expect("text chunk")
            .contains("Sign-in needed"),
    );
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

#[tokio::test]
async fn an_ambiguous_stream_open_never_opens_a_second_stream_or_posts_conventionally() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    // The startStream crosses into transport and the answer is lost: Slack
    // may have created a stream this sink has no handle for, and Slack
    // documents no way to find it.
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatStartStream,
    });
    let report = harness.reconcile(ReplyReconcilePoint::Opened).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    assert_eq!(
        harness.checkpoint_json()["stream_open_ambiguous"],
        Value::Bool(true),
        "the checkpoint remembers the unanswered open"
    );

    // Every later reconcile stays ambiguous and never touches chat.startStream
    // again.
    harness.append(" world");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );

    // The terminal materialization is not posted conventionally either: the
    // ghost stream may already show the chunks the unanswered open carried.
    harness.document.complete();
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    let calls = harness.fake.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.as_str() == "chat.startStream")
            .count(),
        1,
        "exactly one chat.startStream ever went out: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call == "chat.postMessage"),
        "no conventional post beside a possible ghost stream: {calls:?}"
    );
    assert!(harness.fake.posted().is_empty());
}

#[tokio::test]
async fn an_unreadable_read_back_for_a_text_carrying_pending_stays_ambiguous_without_resending() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    // The append reached transport unanswered; the pending carries text.
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

    // Read-back itself is unavailable (a permanent provider answer, not a
    // rate limit): the sink cannot determine whether the text landed, so it
    // must not repeat it — the outcome stays ambiguous and the pending stays
    // on the checkpoint for the host to settle `Unknown`.
    harness.fake.inject(Fault::SlackError {
        method: SlackWebApiMethod::ConversationsReplies,
        error: "missing_scope",
    });
    let appends_before = harness
        .fake
        .bodies(SlackWebApiMethod::ChatAppendStream)
        .len();
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .len(),
        appends_before,
        "nothing was re-sent while the append's fate is unknown"
    );
    assert!(
        harness.checkpoint_json()["stream"]["pending"].is_object(),
        "the pending survives for a later read-back to resolve"
    );

    // Once read-back works again, the ordinary resolution applies (here it
    // proves the append landed) and only the genuinely new delta goes out.
    harness.append("!");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert_applied(&report);
    assert!(report.evidence.read_back_verified);
}

#[tokio::test]
async fn an_ambiguous_attachment_completion_is_never_re_uploaded() {
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

    // `files.completeUploadExternal` reaches Slack but its response is lost:
    // the files may already be shared.
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::FilesCompleteUploadExternal,
    });
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    assert_eq!(
        harness.checkpoint_json()["attachment_upload_ambiguous"],
        Value::Bool(true),
        "the checkpoint remembers the unanswered completion"
    );
    let upload_calls = |calls: &[String]| {
        calls
            .iter()
            .filter(|call| {
                matches!(
                    call.as_str(),
                    "files.getUploadURLExternal" | "upload" | "files.completeUploadExternal"
                )
            })
            .count()
    };
    let uploads_before = upload_calls(&harness.fake.calls());

    // The terminal retry must not touch the upload flow again: an unverified
    // ambiguous completion is never automatically re-uploaded, and the host
    // settles the publication `Unknown` instead of duplicating files.
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );
    assert_eq!(
        upload_calls(&harness.fake.calls()),
        uploads_before,
        "no ticket, byte upload, or completion goes out again: {:?}",
        harness.fake.calls()
    );
}

/// An ambiguous `chat.stopStream` that carried no new answer text leaves
/// read-back nothing to compare — the re-send is the verification: Slack
/// answering `message_not_in_streaming_state` proves a close already landed,
/// so the terminal revision applies instead of failing permanently.
#[tokio::test]
async fn an_ambiguous_no_text_stream_close_is_verified_by_the_resend_answer() {
    let mut harness = Harness::dm();
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    // The close reaches Slack (the stream really stops) but the response is
    // lost. No new answer text rides it, so the pending is text-less.
    harness
        .document
        .finalize_answer(answer("Hello"), Vec::new());
    harness.document.complete();
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatStopStream,
    });
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "got {:?}",
        report.outcome
    );

    // The terminal retry re-sends the close; Slack's
    // `message_not_in_streaming_state` proves the first close landed, and
    // the revision applies — never a permanent failure, never a duplicate.
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    assert_eq!(
        harness
            .fake
            .calls()
            .iter()
            .filter(|call| call.as_str() == "chat.stopStream")
            .count(),
        2,
        "one lost close, one verifying re-send: {:?}",
        harness.fake.calls()
    );
    let ts = harness.stream_ts();
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "Hello",
        "the stream holds exactly the text that was streamed — nothing doubled"
    );
}
