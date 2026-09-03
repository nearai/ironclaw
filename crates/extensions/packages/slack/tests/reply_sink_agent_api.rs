//! The Slack reply sink against an in-crate fake of Slack's native Agent API
//! (`support/fake_slack_agent_api.rs`): exact request bodies on the happy
//! path, idempotent repeats, char-boundary deltas, rate-limit and ambiguity
//! handling with read-back, the stop button, generation and checkpoint
//! version changes, the no-fallback rule, channel recipient requirements,
//! the terminal stream opened and closed in one reconcile, liveness
//! re-assertion, and terminal attachments.

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
    // status, answer, activity, or attention — only `Preparing`. Nothing is
    // renderable, so Slack gets the session status alone: no stream opens
    // until there is content for it to carry, and no synthetic task is ever
    // invented to make a header render.
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
        "nothing renderable, no stream"
    );
    assert!(refs(&report).is_empty());
    assert_eq!(
        harness.checkpoint_json()["session_status"],
        "processing",
        "the checkpoint persists the session state ahead of any stream"
    );

    // The first real activity opens the one stream, carrying the plan header
    // and its task card. The answer's first characters are not a complete
    // paragraph yet, so they wait: Slack renders every markdown chunk as its
    // own block, and a block is a paragraph, never a token.
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
    assert_eq!(harness.fake.calls()[1..], ["chat.startStream"]);
    let ts = harness.stream_ts();
    assert_eq!(refs(&report), [ts.as_str()]);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream),
        [json!({
            "channel": CHANNEL,
            "thread_ts": THREAD,
            "recipient_user_id": USER,
            "recipient_team_id": TEAM,
            "task_display_mode": "plan",
            "chunks": [
                { "type": "plan_update", "title": "Thinking" },
                { "type": "task_update", "id": "act-0", "title": "Read runbook", "status": "in_progress", "details": "docs/runbook.md" }
            ]
        })]
    );
    assert_eq!(
        harness.checkpoint_json()["stream"]["appended_chars"],
        0,
        "held text is not applied until it is a whole paragraph"
    );
    assert!(
        !report.evidence.read_back_verified,
        "nothing was read back on the happy path"
    );

    // ControlCritical: a gate is raised for a tool call the same model call
    // produced, so the text ahead of it ("Hi") was narration — the
    // projection resets the answer before the block lands. The block goes
    // out alone, then `suspended`; the narration reaches Slack nowhere.
    harness.document.activity_finished(
        item("act-0"),
        ReplyActivityState::Completed,
        Some(preview("Runbook opened")),
        None,
    );
    assert!(harness.document.reset_answer());
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
            "chunks": [
                { "type": "plan_update", "title": "Thinking paused" },
                { "type": "task_update", "id": "act-0", "title": "Read runbook", "status": "complete" },
                {
                    "type": "markdown_text",
                    "text": "\n> **Approval needed:** Approve writing report.md\n> The run wants to write report.md in your workspace.\n"
                }
            ]
        })
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::AgentsSessionsSetStatus)[1],
        json!({ "status": "suspended", "channel_id": CHANNEL, "thread_ts": THREAD })
    );

    // Cleared → the plan and session both return to active processing.
    harness.document.clear_attention();
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );
    assert_eq!(
        harness.fake.calls()[4..],
        ["chat.appendStream", "agents.sessions.setStatus"]
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatAppendStream)[1],
        json!({
            "channel": CHANNEL,
            "ts": ts,
            "chunks": [{ "type": "plan_update", "title": "Thinking" }]
        })
    );
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::AgentsSessionsSetStatus)[2],
        json!({ "status": "processing", "channel_id": CHANNEL, "thread_ts": THREAD })
    );

    // Progress: task cards go out as they change; unfinished text waits.
    harness.document.activity_started(
        item("act-1"),
        text("Read runbook"),
        Some(preview("docs/runbook.md")),
    );
    harness.append("Hello");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(harness.fake.calls()[6..], ["chat.appendStream"]);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatAppendStream)[2],
        json!({
            "channel": CHANNEL,
            "ts": ts,
            "chunks": [
                { "type": "task_update", "id": "act-1", "title": "Read runbook", "status": "in_progress", "details": "docs/runbook.md" }
            ]
        })
    );
    harness.document.activity_finished(
        item("act-1"),
        ReplyActivityState::Completed,
        Some(preview("Runbook opened")),
        None,
    );
    harness.append(" world");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(harness.fake.calls()[7..], ["chat.appendStream"]);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatAppendStream)[3],
        json!({
            "channel": CHANNEL,
            "ts": ts,
            "chunks": [
                { "type": "task_update", "id": "act-1", "title": "Read runbook", "status": "complete" }
            ]
        })
    );

    // Terminal: the canonical answer extends the streamed text → one stop
    // carrying everything still held and `session_status: active`.
    harness
        .document
        .finalize_answer(answer("Hello world!"), Vec::new());
    harness.document.complete();
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    assert_eq!(harness.fake.calls()[8..], ["chat.stopStream"]);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStopStream),
        [json!({
            "channel": CHANNEL,
            "ts": ts,
            "chunks": [
                { "type": "plan_update", "title": "Thinking completed" },
                { "type": "markdown_text", "text": "Hello world!" }
            ],
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
        "\n> **Approval needed:** Approve writing report.md\n> The run wants to write report.md in your workspace.\nHello world!"
    );
    assert_eq!(
        stream.task_updates.len(),
        4,
        "two real tools, two states each — and nothing else"
    );
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
    assert_eq!(checkpoint["stream"]["appended_chars"], 12);
    assert_eq!(checkpoint["generation"], 1);
    assert!(checkpoint["tasks"]["act-1"].is_string());
}

/// Progress text goes out one paragraph at a time — Slack renders every
/// markdown chunk as its own block — and the terminal flushes whatever is
/// still unfinished.
#[tokio::test]
async fn progress_text_streams_by_paragraph_and_the_terminal_flushes_the_rest() {
    let mut harness = Harness::dm();
    harness.append("First paragraph.");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    assert!(
        harness.fake.streams().is_empty(),
        "an unfinished paragraph opens nothing"
    );

    harness.append("\n\nSecond");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    let ts = harness.stream_ts();
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream)[0]["chunks"],
        json!([{ "type": "markdown_text", "text": "First paragraph.\n\n" }]),
        "the stream opens with the first whole paragraph"
    );
    assert_eq!(harness.checkpoint_json()["stream"]["appended_chars"], 18);

    harness.append(" paragraph continues");
    let before = harness.fake.calls().len();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(
        harness.fake.calls().len(),
        before,
        "a growing paragraph costs no provider call until it ends"
    );

    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStopStream),
        [json!({
            "channel": DM,
            "ts": ts,
            "chunks": [{ "type": "markdown_text", "text": "Second paragraph continues" }],
            "session_status": "active"
        })]
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "First paragraph.\n\nSecond paragraph continues"
    );
}

#[tokio::test]
async fn tool_arguments_are_clean_and_tool_outputs_are_not_published() {
    let mut harness = Harness::channel();
    harness.document.activity_started(
        item("act-json"),
        text("slack.get_conversation_history"),
        Some(preview("{\n  \"conversation\": \"C123\"\n}")),
    );
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness.document.activity_finished(
        item("act-json"),
        ReplyActivityState::Completed,
        Some(preview("{\n  \"messages\": [\"hello\"]\n}")),
        None,
    );
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);

    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(
        stream.task_updates,
        [
            json!({
                "type": "task_update",
                "id": "act-json",
                "title": "slack.get_conversation_history",
                "status": "in_progress",
                "details": "```json\n{\n  \"conversation\": \"C123\"\n}\n```",
            }),
            json!({
                "type": "task_update",
                "id": "act-json",
                "title": "slack.get_conversation_history",
                "status": "complete",
            }),
        ],
        "the completion updates status without repeating arguments or publishing output"
    );
    assert!(
        stream.block_chunks.is_empty(),
        "tool output must not spill into a separate Slack block"
    );
}

#[tokio::test]
async fn long_json_tool_arguments_remain_complete_and_outputs_are_not_published() {
    let mut harness = Harness::channel();
    let arguments = format!("{{\n  \"query\": \"{}\"\n}}", "a".repeat(320));
    let output = format!("{{\n  \"messages\": [\"{}\"]\n}}", "b".repeat(900));

    harness.document.activity_started(
        item("act-long-json"),
        text("slack.get_conversation_history"),
        Some(preview(&arguments)),
    );
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness.document.activity_finished(
        item("act-long-json"),
        ReplyActivityState::Completed,
        Some(preview(&output)),
        None,
    );
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);

    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(
        stream.block_chunks,
        [json!({
            "type": "blocks",
            "blocks": [{
                "type": "rich_text",
                "elements": [
                    {
                        "type": "rich_text_section",
                        "elements": [{
                            "type": "text",
                            "text": "Arguments",
                            "style": { "bold": true },
                        }],
                    },
                    {
                        "type": "rich_text_preformatted",
                        "elements": [{ "type": "text", "text": arguments }],
                        "language": "json",
                    },
                ],
            }],
        })],
        "Slack preserves the complete bounded input without publishing output"
    );
    assert!(
        stream
            .task_updates
            .iter()
            .all(|task| task.get("details").is_none() && task.get("output").is_none()),
        "long inputs belong in rich-text blocks and outputs stay hidden"
    );
}

#[tokio::test]
async fn model_passes_never_become_plan_rows() {
    let mut harness = Harness::channel();

    // A model pass alone renders nothing: no row, and no stream yet.
    harness.document.note_phase(ReplyPhase::Thinking);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    assert!(harness.fake.streams().is_empty());

    harness.document.note_phase(ReplyPhase::Working);
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    let ts = harness.stream_ts();

    harness.document.activity_finished(
        item("act-0"),
        ReplyActivityState::Completed,
        Some(preview("Found the relevant messages")),
        None,
    );
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);

    harness.document.note_phase(ReplyPhase::Thinking);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);

    harness
        .document
        .finalize_answer(answer("I found the messages you wanted."), Vec::new());
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);

    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(stream.task_display_mode.as_deref(), Some("plan"));
    assert_eq!(
        stream.task_updates,
        [
            json!({ "type": "task_update", "id": "act-0", "title": "Search Slack", "status": "in_progress" }),
            json!({ "type": "task_update", "id": "act-0", "title": "Search Slack", "status": "complete" }),
        ],
        "the only rows are real tool activities"
    );
    assert_eq!(
        stream.plan_updates,
        [
            json!({ "type": "plan_update", "title": "Thinking" }),
            json!({ "type": "plan_update", "title": "Thinking completed" }),
        ]
    );
    assert!(
        stream
            .task_updates
            .last()
            .is_some_and(|chunk| chunk["status"] == "complete"),
        "the terminal message leaves no active spinner"
    );
    assert_eq!(stream.text, "I found the messages you wanted.");
}

#[tokio::test]
async fn attention_and_resume_update_the_plan_without_reopening_a_task() {
    let mut harness = Harness::channel();
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness.document.require_attention(ReplyAttention {
        kind: ReplyAttentionKind::Approval,
        headline: text("Approve sending the summary"),
        body: None,
        action_url: None,
        gate_ref: Some(text("gate:approval-1")),
    });
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );
    harness.document.clear_attention();
    harness.document.note_phase(ReplyPhase::Thinking);
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );

    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(
        stream.plan_updates,
        [
            json!({ "type": "plan_update", "title": "Thinking" }),
            json!({ "type": "plan_update", "title": "Thinking paused" }),
            json!({ "type": "plan_update", "title": "Thinking" }),
        ]
    );
    assert_eq!(
        stream.task_updates,
        [
            json!({ "type": "task_update", "id": "act-0", "title": "Search Slack", "status": "in_progress" })
        ],
        "attention never re-sends or duplicates a task card"
    );
}

#[tokio::test]
async fn failed_and_cancelled_runs_leave_terminal_plan_titles_and_no_spinner() {
    for (cancelled, expected_plan_title, expected_note) in [
        (
            false,
            "Thinking failed",
            "**Failed:** The provider timed out.",
        ),
        (true, "Thinking stopped", "_Stopped._"),
    ] {
        let mut harness = Harness::dm();
        harness
            .document
            .activity_started(item("act-0"), text("Search Slack"), None);
        assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
        let ts = harness.stream_ts();

        if cancelled {
            harness.document.cancel();
        } else {
            harness.document.fail(text("The provider timed out."));
        }
        assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);

        let stream = harness.fake.stream(&ts).expect("stream");
        assert_eq!(
            stream.plan_updates.last(),
            Some(&json!({ "type": "plan_update", "title": expected_plan_title }))
        );
        assert_eq!(
            stream.task_updates,
            [
                json!({ "type": "task_update", "id": "act-0", "title": "Search Slack", "status": "in_progress" }),
                json!({
                    "type": "task_update",
                    "id": "act-0",
                    "title": "Search Slack",
                    "status": "error",
                    "details": "Did not finish before the run ended",
                }),
            ],
            "the run's outcome needs no synthetic row; its unfinished tool is marked, never left spinning"
        );
        assert_eq!(stream.text, expected_note);
        assert_eq!(
            stream.state,
            StreamState::Stopped {
                session_status: "active".to_string()
            }
        );
    }
}

#[tokio::test]
async fn a_terminal_outcome_overrides_stale_attention() {
    let mut harness = Harness::dm();
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness.document.require_attention(ReplyAttention {
        kind: ReplyAttentionKind::Approval,
        headline: text("Approve sending the summary"),
        body: None,
        action_url: None,
        gate_ref: Some(text("gate:approval-1")),
    });
    assert_applied(
        &harness
            .reconcile(ReplyReconcilePoint::ControlCritical)
            .await,
    );

    harness
        .document
        .finalize_answer(answer("Nothing was sent."), Vec::new());
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);

    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(
        stream.plan_updates.last(),
        Some(&json!({ "type": "plan_update", "title": "Thinking completed" })),
        "a terminal stream must never remain visually paused"
    );
}

#[tokio::test]
async fn a_terminal_revision_marks_an_unfinished_tool_as_error() {
    let mut harness = Harness::channel();
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness
        .document
        .finalize_answer(answer("I finished the request."), Vec::new());
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);

    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(
        stream.task_updates.last(),
        Some(&json!({
            "type": "task_update",
            "id": "act-0",
            "title": "Search Slack",
            "status": "error",
            "details": "Did not finish before the run ended",
        })),
        "a terminal reply must never retain an in-progress provider task"
    );
}

/// SYNTHETIC-STATE unit test of chunk planning, not a production path: the
/// production first revision carries no status (see the happy path above).
/// This pins the narrow contract that an explicitly supplied driver status
/// (e.g. "retrying model request") renders as an italic markdown chunk in
/// the stream that opens for it — with no task and therefore no header.
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
    assert_eq!(
        harness.fake.calls(),
        ["agents.sessions.setStatus"],
        "an unfinished paragraph opens nothing"
    );

    // The same desired state again (a retry, a heartbeat, a lease takeover).
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Heartbeat).await);
    assert_eq!(
        harness.fake.calls().len(),
        1,
        "a reflected revision costs no provider call"
    );

    harness
        .document
        .finalize_answer(answer("Hello"), Vec::new());
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    let ts = harness.stream_ts();
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream)[0]["chunks"],
        json!([{ "type": "markdown_text", "text": "Hello" }]),
        "the terminal opens the one stream carrying the whole answer"
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStopStream),
        [json!({ "channel": DM, "ts": ts, "session_status": "active" })],
        "the close carries nothing more; there is no header to close"
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
    harness.append("llo 世界\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    harness.append("!\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);

    let ts = harness.stream_ts();
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream)[0]["chunks"],
        json!([{ "type": "markdown_text", "text": "héllo 世界\n\n" }])
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatAppendStream),
        [
            json!({ "channel": DM, "ts": ts, "chunks": [{ "type": "markdown_text", "text": "!\n\n" }] })
        ]
    );
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").text,
        "héllo 世界\n\n!\n\n"
    );
    assert_eq!(harness.checkpoint_json()["stream"]["appended_chars"], 13);
}

// ── Rate limits ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_rate_limited_append_is_retryable_with_the_provider_hint_and_never_duplicates() {
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    harness.fake.inject(Fault::RateLimited {
        method: SlackWebApiMethod::ChatAppendStream,
        retry_after: Duration::from_secs(7),
    });
    harness.append(" world\n\n");
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
        7,
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
        "Hello\n\n world\n\n"
    );

    harness.fake.inject(Fault::ServerError {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append("!\n\n");
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
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let ts = harness.stream_ts();

    harness.fake.stop_by_user(&ts);
    harness.append(" world\n\n");
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
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    harness.generation = 2;
    harness.append(" world\n\n");
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
        streams[1].1.text, "Hello\n\n world\n\n",
        "the fresh stream carries the whole answer"
    );
    assert_eq!(harness.checkpoint_json()["generation"], 2);
}

#[tokio::test]
async fn an_unknown_checkpoint_version_starts_a_fresh_presentation() {
    let mut harness = Harness::dm();
    harness.append("Hello\n\n");
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
    harness.append(" world\n\n");
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
    harness.append("Hello\n\n");
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
        calls[calls.len() - 3..],
        ["chat.stopStream", "chat.delete", "chat.startStream"],
        "close the stale stream, retract it, open a fresh one"
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStopStream)[0],
        json!({ "channel": DM, "ts": first, "session_status": "processing" }),
        "re-presenting keeps the session processing"
    );
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatDelete),
        [json!({ "channel": DM, "ts": first })],
        "the stale stream's message is retracted, never left beside the fresh one"
    );
    assert_eq!(harness.fake.deleted(), [first]);
    let streams = harness.fake.streams();
    assert_eq!(streams.len(), 1, "only the fresh stream remains");
    assert_eq!(streams[0].1.text, "Goodbye");
    // The fresh stream re-presents everything the stale one showed: here
    // the canonical text, complete, so it goes out whole even before the
    // terminal; with no task there is no header to repeat.
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream)[1]["chunks"],
        json!([{ "type": "markdown_text", "text": "Goodbye" }]),
        "the re-presented stream opens with the full canonical text"
    );
    assert!(streams[0].1.plan_updates.is_empty());
}

// ── Narration ────────────────────────────────────────────────────────────

/// The common shape: a one-line "Let me look." before the tool call. No
/// paragraph boundary, so the paragraph hold never sends it; when the tool
/// call proves it was narration the document resets the answer, and the
/// stream opens with the plan header and task card alone. The narration
/// appears in no request.
#[tokio::test]
async fn a_held_narration_line_never_reaches_slack() {
    let mut harness = Harness::dm();
    harness.append("Let me look.");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    assert!(harness.fake.streams().is_empty(), "held text opens nothing");

    assert!(harness.document.reset_answer());
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    let streams = harness.fake.streams();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].1.text, "");
    assert_eq!(
        streams[0].1.plan_updates,
        [json!({ "type": "plan_update", "title": "Thinking" })]
    );
    assert!(harness.fake.deleted().is_empty(), "nothing to retract");

    harness.append("Here is what I found.\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(
        harness.fake.streams()[0].1.text,
        "Here is what I found.\n\n"
    );
    let bodies = harness
        .fake
        .requests()
        .iter()
        .map(|request| {
            String::from_utf8_lossy(request.body.as_deref().unwrap_or_default()).into_owned()
        })
        .collect::<Vec<_>>();
    assert!(
        bodies.iter().all(|body| !body.contains("Let me look.")),
        "narration never reaches slack: {bodies:?}"
    );
}

/// Narration that had a complete paragraph before the tool call was already
/// streamed. The reset makes the shown prefix stale: the sink closes that
/// stream, retracts its message with `chat.delete`, and opens a fresh stream
/// carrying the plan header and task card, where the answer then streams.
#[tokio::test]
async fn a_streamed_narration_paragraph_is_retracted_when_the_answer_resets() {
    let mut harness = Harness::dm();
    harness.append("Let me look at two things first.\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let stale = harness.stream_ts();
    assert_eq!(
        harness.fake.stream(&stale).expect("stale stream").text,
        "Let me look at two things first.\n\n"
    );

    assert!(harness.document.reset_answer());
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    let before = harness.fake.calls().len();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(
        harness.fake.calls()[before..],
        ["chat.stopStream", "chat.delete", "chat.startStream"],
        "close the stale stream, retract it, open a fresh one"
    );
    assert_eq!(harness.fake.deleted(), [stale.as_str()]);
    let streams = harness.fake.streams();
    assert_eq!(streams.len(), 1, "only the fresh stream remains");
    let (fresh, stream) = &streams[0];
    assert_ne!(fresh, &stale);
    assert_eq!(stream.text, "", "the fresh stream carries no narration");
    assert_eq!(
        stream.task_updates,
        [
            json!({ "type": "task_update", "id": "act-0", "title": "Search Slack", "status": "in_progress" })
        ]
    );

    harness.append("Here is what I found.\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(
        harness.fake.calls().last().map(String::as_str),
        Some("chat.appendStream")
    );
    assert_eq!(
        harness.fake.stream(fresh).expect("fresh stream").text,
        "Here is what I found.\n\n"
    );
}

/// A retraction whose `chat.delete` never gets an answer is retried, in
/// both shapes: the request never reached Slack (the retry deletes for the
/// first time) and the request was applied before the transport failed
/// (the retry finds the message gone — `message_not_found` on the close and
/// on the delete — which is the state it wanted). Either way the fresh
/// stream opens exactly once.
#[tokio::test]
async fn a_lost_retraction_answer_is_retried_without_a_second_fresh_stream() {
    for fault in [
        Fault::TransportBeforeAccept {
            method: SlackWebApiMethod::ChatDelete,
        },
        Fault::TransportAfterAccept {
            method: SlackWebApiMethod::ChatDelete,
        },
    ] {
        let mut harness = Harness::dm();
        harness.append("Let me look at two things first.\n\n");
        assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
        let stale = harness.stream_ts();

        assert!(harness.document.reset_answer());
        harness
            .document
            .activity_started(item("act-0"), text("Search Slack"), None);
        let applied_before_loss = matches!(fault, Fault::TransportAfterAccept { .. });
        harness.fake.inject(fault);
        let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
        assert!(
            matches!(report.outcome, ReplySinkOutcome::Retryable { .. }),
            "a transport failure on the retraction is retried, got {:?}",
            report.outcome
        );
        assert_eq!(
            harness.fake.deleted().len(),
            usize::from(applied_before_loss),
            "an applied-then-lost delete already removed the message"
        );
        let before = harness.fake.calls().len();
        assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
        assert_eq!(
            harness.fake.calls()[before..],
            ["chat.stopStream", "chat.delete", "chat.startStream"],
            "the retry closes (already closed or gone: tolerated), retracts (or finds it gone), and opens once"
        );
        assert_eq!(harness.fake.deleted(), [stale.as_str()]);
        assert_eq!(harness.fake.streams().len(), 1);
    }
}

/// A workspace can forbid deleting messages (retention policies answer
/// `chat.delete` with a refusal). The answer must still reach the user, so
/// a refused retraction is logged and the fresh stream opens anyway: the
/// stale narration message is the lesser harm, and it is a decision, not
/// an accident. Only a missing message counts as retracted.
#[tokio::test]
async fn a_refused_retraction_leaves_the_stale_message_and_still_delivers_the_answer() {
    let mut harness = Harness::dm();
    harness.append("Let me look at two things first.\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let stale = harness.stream_ts();

    assert!(harness.document.reset_answer());
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    harness.fake.inject(Fault::SlackError {
        method: SlackWebApiMethod::ChatDelete,
        error: "cant_delete_message",
    });
    let before = harness.fake.calls().len();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(
        harness.fake.calls()[before..],
        ["chat.stopStream", "chat.delete", "chat.startStream"],
        "the refusal does not stop the fresh stream from opening"
    );
    assert!(
        harness.fake.deleted().is_empty(),
        "nothing was retracted: the workspace refused"
    );
    let streams = harness.fake.streams();
    assert_eq!(
        streams.len(),
        2,
        "the stale message stays beside the fresh stream"
    );
    let (fresh, fresh_stream) = streams
        .iter()
        .find(|(ts, _)| ts != &stale)
        .expect("a fresh stream");
    assert_eq!(
        fresh_stream.text, "",
        "the fresh stream carries no narration"
    );

    harness.append("Here is what I found.\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(
        harness.fake.stream(fresh).expect("fresh stream").text,
        "Here is what I found.\n\n",
        "the answer streams on the fresh stream regardless"
    );
}

/// A rate-limited `chat.delete` is not a refusal: the retraction is retried
/// with Slack's hint, no fresh stream opens until it lands, and the retry
/// deletes the stale message and opens the fresh stream once.
#[tokio::test]
async fn a_rate_limited_retraction_is_retried_before_the_fresh_stream_opens() {
    let mut harness = Harness::dm();
    harness.append("Let me look at two things first.\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let stale = harness.stream_ts();

    assert!(harness.document.reset_answer());
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    harness.fake.inject(Fault::SlackError {
        method: SlackWebApiMethod::ChatDelete,
        error: "ratelimited",
    });
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Retryable { .. }),
        "a rate limit on the retraction is retried, got {:?}",
        report.outcome
    );
    assert!(harness.fake.deleted().is_empty());
    assert_eq!(
        harness.fake.streams().len(),
        1,
        "no fresh stream opens beside a message that is still there"
    );

    let before = harness.fake.calls().len();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(
        harness.fake.calls()[before..],
        ["chat.stopStream", "chat.delete", "chat.startStream"]
    );
    assert_eq!(harness.fake.deleted(), [stale.as_str()]);
    assert_eq!(
        harness.fake.streams().len(),
        1,
        "only the fresh stream remains"
    );
}

/// The append carrying a narration paragraph crosses transport unanswered,
/// and before the next reconcile the tool call resets the answer. Read-back
/// can no longer verify the pending against a document that dropped that
/// text — and it does not matter whether the append landed: the stream is
/// stale either way. The sink closes it, retracts its message, and opens
/// the fresh stream, instead of appending the next call beneath narration
/// that may be showing.
#[tokio::test]
async fn a_lost_narration_append_followed_by_a_reset_retracts_the_stale_stream() {
    let mut harness = Harness::dm();
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    let stale = harness.stream_ts();

    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append("Let me look at two things first.\n\n");
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Ambiguous { .. }),
        "the lost append is pending, got {:?}",
        report.outcome
    );
    assert_eq!(
        harness.fake.stream(&stale).expect("stale stream").text,
        "Let me look at two things first.\n\n",
        "the append landed provider-side"
    );

    assert!(harness.document.reset_answer());
    harness
        .document
        .activity_finished(item("act-0"), ReplyActivityState::Completed, None, None);
    let before = harness.fake.calls().len();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert!(
        harness.fake.calls()[before..].contains(&"chat.delete".to_string()),
        "the stale stream is retracted: {:?}",
        &harness.fake.calls()[before..]
    );
    assert_eq!(harness.fake.deleted(), [stale.as_str()]);
    let streams = harness.fake.streams();
    assert_eq!(streams.len(), 1, "only the fresh stream remains");
    assert_eq!(
        streams[0].1.text, "",
        "the fresh stream carries no narration"
    );

    harness.append("Here is what I found.\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Progress).await);
    assert_eq!(
        harness.fake.streams()[0].1.text,
        "Here is what I found.\n\n",
        "the answer streams on the fresh stream"
    );
}

/// Only Slack's own refusal is tolerated. A retraction the host's egress
/// policy denies before the network never reached Slack: that is a
/// deployment fault to surface, so the reply fails loudly and no fresh
/// stream opens beside a message that is still there.
#[tokio::test]
async fn an_egress_denied_retraction_fails_the_reply_instead_of_opening_a_fresh_stream() {
    let mut harness = Harness::dm();
    harness.append("Let me look at two things first.\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    assert!(harness.document.reset_answer());
    harness
        .document
        .activity_started(item("act-0"), text("Search Slack"), None);
    harness.fake.inject(Fault::EgressDenied {
        method: SlackWebApiMethod::ChatDelete,
    });
    let report = harness.reconcile(ReplyReconcilePoint::Progress).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Permanent { .. }),
        "an egress denial is not a slack refusal, got {:?}",
        report.outcome
    );
    assert!(harness.fake.deleted().is_empty());
    assert_eq!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatStartStream)
            .len(),
        1,
        "no fresh stream opens beside a message that is still there"
    );
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
    harness.append("Hello\n\n");
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
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    assert_eq!(
        harness.fake.bodies(SlackWebApiMethod::ChatStartStream),
        [json!({
            "channel": DM,
            "thread_ts": THREAD,
            "task_display_mode": "plan",
            "chunks": [{ "type": "markdown_text", "text": "Hello\n\n" }]
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

    // A completed run with nothing to show opens nothing: no stream and no
    // message — only the session settles. Nothing is invented to fill it.
    let mut harness = Harness::dm();
    harness.document.complete();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert!(
        harness
            .fake
            .calls()
            .iter()
            .all(|call| call == "agents.sessions.setStatus"),
        "only the session settles: {:?}",
        harness.fake.calls()
    );
    assert!(harness.fake.streams().is_empty());
    assert!(harness.fake.posted().is_empty());
    assert_eq!(harness.checkpoint_json()["terminal"], "applied");
}

/// The outcome note rides `chat.startStream` when the terminal opens its own
/// stream; a refused `chat.stopStream` after it must not append the note a
/// second time on the retry — the note is checkpointed like every other
/// chunk the stream has already shown.
#[tokio::test]
async fn a_retried_terminal_close_never_repeats_the_outcome_note() {
    let mut harness = Harness::dm();
    harness.document.fail(text("The model provider timed out."));
    harness.fake.inject(Fault::RateLimited {
        method: SlackWebApiMethod::ChatStopStream,
        retry_after: Duration::from_secs(3),
    });
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Retryable { .. }),
        "got {:?}",
        report.outcome
    );
    let ts = harness.stream_ts();
    assert_eq!(
        harness.fake.stream(&ts).expect("stream").state,
        StreamState::Streaming,
        "the refused close left the stream open"
    );

    assert_applied(&harness.reconcile(ReplyReconcilePoint::Terminal).await);
    assert_eq!(
        harness.fake.calls(),
        ["chat.startStream", "chat.stopStream", "chat.stopStream"],
        "one open, one refused close, one retried close"
    );
    let stream = harness.fake.stream(&ts).expect("stream");
    assert_eq!(
        stream.state,
        StreamState::Stopped {
            session_status: "active".to_string()
        }
    );
    assert_eq!(
        stream.text, "**Failed:** The model provider timed out.",
        "the retried close does not repeat the note the open already showed"
    );
    assert_eq!(harness.fake.streams().len(), 1);
    assert_eq!(harness.checkpoint_json()["terminal"], "applied");
}

/// A terminal whose canonical answer is NOT an extension of the streamed
/// text (a genuine mid-stream rewrite): the stale stream is closed as it
/// stands and the canonical answer goes out on ONE fresh native stream —
/// never as a conventional message beside the stream.
#[tokio::test]
async fn a_terminal_rewrite_re_presents_natively_without_a_conventional_post() {
    let mut harness = Harness::dm();
    harness.append("The old draft answer.\n\n");
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
    assert_eq!(
        harness.fake.deleted(),
        [stale_ts.as_str()],
        "the stale stream's message is retracted"
    );
    let streams = harness.fake.streams();
    assert_eq!(streams.len(), 1, "one fresh terminal stream remains");
    let fresh_ts = streams[0].0.clone();
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
/// text): the one stream opens carrying it — nothing opened earlier for an
/// empty document.
#[tokio::test]
async fn an_attention_block_as_first_content_opens_the_stream_carrying_it() {
    let mut harness = Harness::dm();
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
    assert!(harness.fake.streams().is_empty(), "nothing to show yet");

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
    assert_eq!(bodies.len(), 1, "the block opens the one stream");
    let chunks = bodies[0]["chunks"].as_array().expect("chunks");
    assert!(chunks.iter().any(|chunk| {
        chunk["type"] == "markdown_text"
            && chunk["text"]
                .as_str()
                .is_some_and(|text| text.contains("Sign-in needed"))
    }));
    assert!(
        harness
            .fake
            .bodies(SlackWebApiMethod::ChatAppendStream)
            .is_empty(),
        "the stream opens carrying the block; nothing is appended after it"
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

/// The upload ticket (`files.getUploadURLExternal`) and the private byte
/// upload share nothing into the conversation — only
/// `files.completeUploadExternal` does. A lost answer on the ticket is
/// therefore a plain retry, never the fail-closed attachment ambiguity: the
/// retry takes a fresh ticket and the file is shared exactly once.
#[tokio::test]
async fn a_lost_upload_ticket_answer_is_retryable_and_the_retry_shares_the_file_once() {
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

    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::FilesGetUploadUrlExternal,
    });
    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert!(
        matches!(report.outcome, ReplySinkOutcome::Retryable { .. }),
        "a lost ticket answer shares nothing, so it is retryable, got {:?}",
        report.outcome
    );
    let checkpoint = harness.checkpoint_json();
    assert_eq!(
        checkpoint["attachment_upload_ambiguous"],
        Value::Bool(false),
        "the fail-closed latch is reserved for an unanswered completion"
    );
    assert_eq!(checkpoint["terminal"], "stream_closed");
    assert_eq!(
        harness.fake.calls()[2..],
        ["chat.stopStream", "files.getUploadURLExternal"],
        "no bytes went out on a ticket whose answer was lost"
    );

    let report = harness.reconcile(ReplyReconcilePoint::Terminal).await;
    assert_applied(&report);
    assert_eq!(
        harness.fake.calls()[4..],
        [
            "files.getUploadURLExternal",
            "upload",
            "files.completeUploadExternal",
            "files.info",
        ],
        "the retry takes a fresh ticket and completes the upload once"
    );
    assert_eq!(refs(&report), ["FAKE2".to_string()]);
    assert_eq!(harness.checkpoint_json()["attachments_delivered"], true);
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
        harness.append("Hello\n\n");
        assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);
        harness.fake.inject(Fault::SlackError {
            method: SlackWebApiMethod::ChatAppendStream,
            error,
        });
        harness.append(" world\n\n");
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
    harness.append("Hello\n\n");
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
    harness.append(" world\n\n");
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
    harness.append("Hello\n\n");
    assert_applied(&harness.reconcile(ReplyReconcilePoint::Opened).await);

    // The append reached transport unanswered; the pending carries text.
    harness.fake.inject(Fault::TransportAfterAccept {
        method: SlackWebApiMethod::ChatAppendStream,
    });
    harness.append(" world\n\n");
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
    harness.append("!\n\n");
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
