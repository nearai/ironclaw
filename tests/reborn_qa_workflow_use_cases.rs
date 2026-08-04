//! QA use-case coverage for the ten assistant workflow families:
//!
//! 1. Inbox triage & email automation
//! 2. Daily morning briefing
//! 3. Calendar management & reminders
//! 4. Meeting lifecycle automation
//! 5. Team chat operations
//! 6. Task management & delegation
//! 7. Developer workflow automation
//! 8. Cross-app automation (webhooks)
//! 9. Lead generation & CRM automation
//! 10. Personal & business tracking
//!
//! The scheduled ("every N minutes…") form of these families lives in
//! `reborn_qa_routines`; the channel-inbound form lives in
//! `reborn_qa_channel_delivery`. This file owns the **one-shot working
//! turn** — the part where the assistant actually reads several sources,
//! decides, and writes something back.
//!
//! ## What these tests do and do not prove
//!
//! The model is replayed from a script, so a test can never prove the model
//! *reasoned* correctly — asserting on a scripted reply would only assert
//! the script. What each test pins instead is the **data path and the
//! effects**:
//!
//! - every source the workflow needs was actually fetched (asserted against
//!   the loopback server's request log, in order);
//! - every fetched payload actually reached the model as a tool result
//!   (asserted against the recorded capability results) — including the
//!   rows the workflow is supposed to *reject*, so the discriminating
//!   decision is the model's to make and not the fixture's to fake;
//! - every write leg carried its real payload across the egress boundary
//!   (asserted against the captured request body, not the reply text).
//!
//! Where a fixture offers a negative (a newsletter that must not be
//! answered, a back-to-back meeting pair that is not a conflict, a
//! non-near.ai lead), the test asserts the negative reached the model AND
//! that the write leg excluded it. That is the strongest claim available at
//! a replayed-model tier, and it is the claim that catches a real
//! regression in the plumbing.
//!
//! ## Chat-only setup
//!
//! `reborn_qa_chat_only_extension_install_makes_capability_callable` covers
//! the standing requirement that a user can go from nothing to a working
//! tool **without touching the UI** — install through `builtin.extension_install`
//! in an ordinary chat turn, then use the installed capability in the same
//! conversation. Broader chat-only configuration is epic #7046.

#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, Uri, header},
    response::IntoResponse,
    routing::{get, post},
};
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_host_runtime::{HTTP_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID};
use ironclaw_loop_host::HostManagedModelResponse;
use ironclaw_turns::TurnStatus;
use parity_qa_support::binary_e2e::RebornBinaryE2EHarness;
use parity_qa_support::model_replay::{
    RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
};
use parity_qa_support::network::{
    LiveLoopbackHttpServer, LiveLoopbackHttpState, loopback_http_policy,
};
use serde_json::json;

fn http_capability() -> CapabilityId {
    CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id")
}

/// `builtin.http` GET step against the loopback server.
fn fetch(call_id: &str, url: String) -> RebornScriptedProviderToolCall {
    RebornScriptedProviderToolCall::new(
        http_capability(),
        call_id,
        json!({"url": url, "timeout_ms": 2500}),
    )
}

/// `builtin.http` POST step — the write leg every one of these workflows
/// ends in. Body is asserted at the wire by the calling test.
fn post_json(call_id: &str, url: String, body: &str) -> RebornScriptedProviderToolCall {
    RebornScriptedProviderToolCall::new(
        http_capability(),
        call_id,
        json!({"url": url, "method": "post", "body": body, "timeout_ms": 2500}),
    )
}

fn reply(text: &str) -> RebornModelReplayStep {
    RebornModelReplayStep::Response {
        response: HostManagedModelResponse::assistant_reply(text),
        expected_tool_results: Vec::new(),
    }
}

fn calls(calls: Vec<RebornScriptedProviderToolCall>) -> RebornModelReplayStep {
    RebornModelReplayStep::ProviderToolCalls {
        calls,
        expected_tool_results: Vec::new(),
    }
}

async fn live_http_harness(
    room: &str,
    steps: impl IntoIterator<Item = RebornModelReplayStep>,
    server: &LiveLoopbackHttpServer,
) -> RebornBinaryE2EHarness {
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities_live_http_egress(
            room,
            RebornTraceReplayModelGateway::with_scripted_steps(steps),
            loopback_http_policy(server.port()),
        )
        .await
        .expect("harness");
    harness.start();
    harness
}

/// Assert a recorded capability result's response body reached the model
/// carrying `needle` — i.e. the payload actually crossed back, rather than
/// the reply merely claiming it did.
fn assert_result_body_contains(
    harness: &RebornBinaryE2EHarness,
    index: usize,
    needle: &str,
    what: &str,
) {
    let results = harness.capability_results();
    let body = results[index].output["body_text"]
        .as_str()
        .unwrap_or_else(|| panic!("result {index} should carry a response body"));
    assert!(
        body.contains(needle),
        "{what} should reach the model as a tool result; missing {needle:?} in {body}"
    );
}

// --- 1. Inbox triage & email automation ----------------------------------

/// Family 1: read the inbox, decide what deserves a reply, draft it.
///
/// The fixture deliberately mixes an urgent customer escalation with a
/// newsletter. Both must reach the model (so triage is the model's call),
/// and only the escalation may appear in the drafted reply that crosses the
/// wire — a plumbing regression that drafted against the wrong thread, or
/// dropped the message body before the write, fails here.
#[tokio::test]
async fn reborn_qa_inbox_triage_drafts_reply_to_the_urgent_thread_only() {
    const REPLY: &str = "1 email needs you: Priya's escalation about the failing export. I drafted a reply; the newsletter needs nothing.";

    let server = LiveLoopbackHttpServer::start(
        Router::new()
            .route("/gmail/v1/users/me/messages", get(inbox_messages))
            .route("/gmail/v1/users/me/drafts", post(create_draft)),
    )
    .await;
    let mut harness = live_http_harness(
        "room-qa-inbox-triage",
        [
            calls(vec![fetch(
                "call_read_inbox",
                server.url("/gmail/v1/users/me/messages?q=is%3Aunread"),
            )]),
            calls(vec![post_json(
                "call_draft_reply",
                server.url("/gmail/v1/users/me/drafts"),
                r#"{"threadId":"t-escalation","to":"priya@customer.example","body":"Thanks Priya — looking at the export failure now."}"#,
            )]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-inbox-triage",
            "check my unread email, tell me what actually needs me, and draft a reply to it",
        )
        .await
        .expect("submit inbox triage request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("triage reply");

    // Both the actionable mail AND the one that must be ignored reached the
    // model — the triage decision is not pre-made by the fixture.
    assert_result_body_contains(&harness, 0, "priya@customer.example", "the escalation");
    assert_result_body_contains(&harness, 0, "weekly-digest@news.example", "the newsletter");

    let drafted = server.request_bodies();
    assert_eq!(drafted.len(), 1, "exactly one draft should be created");
    assert!(
        drafted[0].contains("priya@customer.example") && drafted[0].contains("t-escalation"),
        "the draft must be addressed to the escalation thread, got {}",
        drafted[0]
    );
    assert!(
        !drafted[0].contains("weekly-digest@news.example"),
        "the newsletter must not be drafted against, got {}",
        drafted[0]
    );

    assert_eq!(
        server.requests(),
        vec![
            "/gmail/v1/users/me/messages?q=is%3Aunread".to_string(),
            "/gmail/v1/users/me/drafts".to_string(),
        ],
        "read must precede the draft write"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- 2. Daily morning briefing -------------------------------------------

/// Family 2: fan IN from three providers, then answer once.
///
/// This is the only case in the QA corpus where one model turn issues
/// several *different* provider reads in parallel, so it is the coverage
/// for the parallel-tool-call path carrying three independent results back
/// into one turn. A regression that serialized, dropped, or cross-wired one
/// of the three results fails on the per-source assertions below.
#[tokio::test]
async fn reborn_qa_morning_briefing_fans_in_calendar_email_and_tasks() {
    const REPLY: &str = "Morning: 2 meetings (standup 09:30, Acme review 14:00), 1 email needing you from Priya, and 2 tasks due today.";

    let server = LiveLoopbackHttpServer::start(
        Router::new()
            .route("/calendar/v3/today", get(calendar_today))
            .route("/gmail/v1/users/me/messages", get(inbox_messages))
            .route("/tasks/v1/due", get(tasks_due)),
    )
    .await;
    let mut harness = live_http_harness(
        "room-qa-morning-briefing",
        [
            // One model turn, three parallel provider reads.
            calls(vec![
                fetch("call_briefing_calendar", server.url("/calendar/v3/today")),
                fetch(
                    "call_briefing_email",
                    server.url("/gmail/v1/users/me/messages?q=is%3Aunread"),
                ),
                fetch("call_briefing_tasks", server.url("/tasks/v1/due")),
            ]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-morning-briefing",
            "give me my morning briefing: calendar, anything in email that needs me, and what's due today",
        )
        .await
        .expect("submit briefing request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("briefing reply");

    let results = harness.capability_results();
    assert_eq!(results.len(), 3, "all three sources should be read");

    // Each source's payload must be individually present — a briefing built
    // from two of three is the failure this family actually suffers.
    let bodies: Vec<String> = results
        .iter()
        .map(|result| {
            result.output["body_text"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let joined = bodies.join("\n");
    for (source, needle) in [
        ("calendar", "Acme quarterly review"),
        ("email", "priya@customer.example"),
        ("tasks", "Renew the TLS certificate"),
    ] {
        assert!(
            joined.contains(needle),
            "the {source} payload should reach the model; missing {needle:?} in {joined}"
        );
    }

    let mut requested = server.requests();
    requested.sort();
    assert_eq!(
        requested,
        vec![
            "/calendar/v3/today".to_string(),
            "/gmail/v1/users/me/messages?q=is%3Aunread".to_string(),
            "/tasks/v1/due".to_string(),
        ],
        "every briefing source should be fetched exactly once"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- 3. Calendar management & reminders ----------------------------------

/// Family 3: conflict detection.
///
/// The fixture contains BOTH a genuine overlap (14:00–15:00 vs 14:30–15:30)
/// and a back-to-back pair (09:30–09:45 then 09:45–10:00) that must not be
/// reported as a conflict. Both shapes must reach the model, so a fixture
/// that only ever contains conflicts cannot make this pass vacuously.
#[tokio::test]
async fn reborn_qa_calendar_conflict_scan_sees_overlap_and_back_to_back() {
    const REPLY: &str = "One real conflict: Acme quarterly review (14:00) overlaps the platform sync (14:30). Standup and the 1:1 are back-to-back but fine.";

    let server = LiveLoopbackHttpServer::start(
        Router::new().route("/calendar/v3/today", get(calendar_today)),
    )
    .await;
    let mut harness = live_http_harness(
        "room-qa-calendar-conflicts",
        [
            calls(vec![fetch(
                "call_scan_calendar",
                server.url("/calendar/v3/today"),
            )]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-calendar-conflicts",
            "does anything on my calendar today clash?",
        )
        .await
        .expect("submit conflict scan request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("conflict reply");

    // The overlapping pair AND the merely-adjacent pair both reach the
    // model, so the discrimination is the model's to make.
    for needle in [
        "Acme quarterly review",
        "Platform sync",
        "Standup",
        "Weekly 1:1",
    ] {
        assert_result_body_contains(&harness, 0, needle, "the calendar payload");
    }
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- 4. Meeting lifecycle automation -------------------------------------

/// Family 4: the POST-meeting half. `reborn_qa_doc_grounding` already covers
/// meeting *prep*; this covers what happens afterwards — pull the meeting
/// record, then send the attendees a recap that actually contains the
/// decisions.
///
/// The regression this guards is a recap that goes out empty or addressed
/// to the wrong attendee, so the recap body and recipient are asserted at
/// the wire rather than through the reply.
///
/// Workspace persistence of meeting notes is NOT covered here — see
/// `reborn_qa_expense_is_recorded_to_the_workspace_ledger_and_read_back`
/// for the workspace-write family, and `tests/CLAUDE.md` §7.2 for the
/// `builtin.http.save` gap this test uncovered.
#[tokio::test]
async fn reborn_qa_meeting_followup_reads_the_record_and_sends_a_populated_recap() {
    const REPLY: &str =
        "Sent the Acme review recap to Priya: pilot scope agreed, she owns the export fix.";

    let server = LiveLoopbackHttpServer::start(
        Router::new()
            .route(
                "/meetings/v1/acme-review/transcript",
                get(meeting_transcript),
            )
            .route("/gmail/v1/users/me/messages/send", post(send_message)),
    )
    .await;
    let mut harness = live_http_harness(
        "room-qa-meeting-followup",
        [
            calls(vec![fetch(
                "call_read_transcript",
                server.url("/meetings/v1/acme-review/transcript"),
            )]),
            calls(vec![post_json(
                "call_send_recap",
                server.url("/gmail/v1/users/me/messages/send"),
                r#"{"to":"priya@customer.example","subject":"Acme review recap","body":"Agreed pilot scope; Priya owns the export fix."}"#,
            )]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-meeting-followup",
            "recap the Acme review to the attendees",
        )
        .await
        .expect("submit meeting follow-up request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("follow-up reply");

    // The meeting content reached the model before the recap was composed.
    assert_result_body_contains(&harness, 0, "Priya owns the export fix", "the transcript");

    let sent = server.request_bodies();
    assert_eq!(sent.len(), 1, "exactly one recap should be sent");
    assert!(
        sent[0].contains("priya@customer.example") && sent[0].contains("Acme review recap"),
        "the recap must reach the attendee with its subject, got {}",
        sent[0]
    );
    assert!(
        sent[0].contains("export fix"),
        "the recap must carry the decisions, not just a subject line, got {}",
        sent[0]
    );

    assert_eq!(
        server.requests(),
        vec![
            "/meetings/v1/acme-review/transcript".to_string(),
            "/gmail/v1/users/me/messages/send".to_string(),
        ],
        "the record must be read before the recap is sent"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- 6. Task management & delegation -------------------------------------

/// Family 6: a request in chat becomes a tracked, assigned task.
///
/// The assignment is the part that regresses — a task created without its
/// assignee or without a link back to the originating message is the
/// reported failure shape — so both are asserted at the wire.
#[tokio::test]
async fn reborn_qa_request_becomes_tracked_task_with_assignee_and_source() {
    const REPLY: &str =
        "Created TASK-412 “Fix the CSV export timeout”, assigned to priya, linked to the thread.";

    let server =
        LiveLoopbackHttpServer::start(Router::new().route("/tasks/v1/tasks", post(create_task)))
            .await;
    let mut harness = live_http_harness(
        "room-qa-task-delegation",
        [
            calls(vec![post_json(
                "call_create_task",
                server.url("/tasks/v1/tasks"),
                r#"{"title":"Fix the CSV export timeout","assignee":"priya","due":"2026-08-07","source_thread":"t-escalation"}"#,
            )]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-task-delegation",
            "turn Priya's export escalation into a task for her, due Friday",
        )
        .await
        .expect("submit task delegation request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness.assert_final_reply(REPLY).await.expect("task reply");

    let created = server.request_bodies();
    assert_eq!(created.len(), 1, "exactly one task should be created");
    for (field, needle) in [
        ("assignee", "\"assignee\":\"priya\""),
        ("due date", "2026-08-07"),
        ("source thread", "t-escalation"),
    ] {
        assert!(
            created[0].contains(needle),
            "the created task should carry its {field}, got {}",
            created[0]
        );
    }
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- 7. Developer workflow automation ------------------------------------

/// Family 7: issue triage — read the open issues, then label and comment.
///
/// Three ordered legs across two write endpoints. The fixture includes an
/// already-labelled issue that must not be touched again, so the ordering
/// assertion below is about the workflow, not just about counting requests.
#[tokio::test]
async fn reborn_qa_issue_triage_labels_and_comments_on_the_untriaged_issue() {
    const REPLY: &str = "Triaged #812: labelled it bug/export and asked the reporter for their export size. #799 was already triaged.";

    let server = LiveLoopbackHttpServer::start(
        Router::new()
            .route("/repos/nearai/ironclaw/issues", get(open_issues))
            .route("/repos/nearai/ironclaw/issues/812/labels", post(add_labels))
            .route(
                "/repos/nearai/ironclaw/issues/812/comments",
                post(add_comment),
            ),
    )
    .await;
    let mut harness = live_http_harness(
        "room-qa-issue-triage",
        [
            calls(vec![fetch(
                "call_list_issues",
                server.url("/repos/nearai/ironclaw/issues?state=open"),
            )]),
            calls(vec![post_json(
                "call_label_issue",
                server.url("/repos/nearai/ironclaw/issues/812/labels"),
                r#"{"labels":["bug","area/export"]}"#,
            )]),
            calls(vec![post_json(
                "call_comment_issue",
                server.url("/repos/nearai/ironclaw/issues/812/comments"),
                r#"{"body":"Thanks for the report — roughly how many rows is the export?"}"#,
            )]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-issue-triage",
            "triage the open issues on nearai/ironclaw — label anything untriaged and ask for what's missing",
        )
        .await
        .expect("submit issue triage request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("triage reply");

    // Both the untriaged issue and the already-labelled one reach the model.
    assert_result_body_contains(&harness, 0, "CSV export times out", "the untriaged issue");
    assert_result_body_contains(&harness, 0, "already-triaged", "the already-triaged issue");

    assert_eq!(
        server.requests(),
        vec![
            "/repos/nearai/ironclaw/issues?state=open".to_string(),
            "/repos/nearai/ironclaw/issues/812/labels".to_string(),
            "/repos/nearai/ironclaw/issues/812/comments".to_string(),
        ],
        "triage should read first, then label, then comment — and only touch #812"
    );

    let writes = server.request_bodies();
    assert_eq!(writes.len(), 2, "one label write and one comment write");
    assert!(
        writes[0].contains("area/export"),
        "the label write should carry the labels, got {}",
        writes[0]
    );
    assert!(
        writes[1].contains("how many rows"),
        "the comment write should carry the question, got {}",
        writes[1]
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

/// Family 7, failure edge: CI reports a FAILED run, and the workflow has to
/// act on the failure rather than treat a reachable endpoint as success.
///
/// The endpoint returns HTTP 200 with a failing payload — the shape that
/// actually regresses, because a status-code-only check reads it as
/// healthy. The alert body must name the failing job.
#[tokio::test]
async fn reborn_qa_ci_failure_payload_on_a_200_still_raises_an_alert() {
    const REPLY: &str =
        "CI is red on main: the integration-tests job failed. I posted an alert to the channel.";

    let server = LiveLoopbackHttpServer::start(
        Router::new()
            .route("/repos/nearai/ironclaw/actions/runs", get(ci_runs))
            .route("/chat/v1/alerts", post(post_alert)),
    )
    .await;
    let mut harness = live_http_harness(
        "room-qa-ci-watch",
        [
            calls(vec![fetch(
                "call_check_ci",
                server.url("/repos/nearai/ironclaw/actions/runs?branch=main"),
            )]),
            calls(vec![post_json(
                "call_post_ci_alert",
                server.url("/chat/v1/alerts"),
                r#"{"text":"CI red on main: integration-tests failed (run 5521)"}"#,
            )]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-ci-watch",
            "is CI green on main? if not, tell the channel",
        )
        .await
        .expect("submit CI check request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness.assert_final_reply(REPLY).await.expect("CI reply");

    let results = harness.capability_results();
    // The transport succeeded — this is the trap the failure edge exists for.
    assert_eq!(
        results[0].output["status"],
        json!(200),
        "the CI endpoint itself is reachable; the FAILURE is in the payload"
    );
    assert_result_body_contains(&harness, 0, "\"conclusion\":\"failure\"", "the failing run");
    assert_result_body_contains(&harness, 0, "\"conclusion\":\"success\"", "the passing run");

    let alerts = server.request_bodies();
    assert_eq!(alerts.len(), 1, "exactly one alert should be posted");
    assert!(
        alerts[0].contains("integration-tests") && alerts[0].contains("5521"),
        "the alert must name the failing job and run, got {}",
        alerts[0]
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- 8. Cross-app automation ---------------------------------------------

/// Family 8: an event from one system fans out into two others.
///
/// The regression this guards is a partial fan-out — the first downstream
/// write lands and the second is silently dropped — so both destinations
/// are asserted at the wire, in order.
#[tokio::test]
async fn reborn_qa_cross_app_event_fans_out_to_every_downstream_system() {
    const REPLY: &str =
        "New signup from Acme: added them to the CRM sheet and posted the intro to the channel.";

    let server = LiveLoopbackHttpServer::start(
        Router::new()
            .route("/hooks/v1/events", get(webhook_events))
            .route(
                "/sheets/v4/spreadsheets/CRM/values/append",
                post(append_row),
            )
            .route("/chat/v1/alerts", post(post_alert)),
    )
    .await;
    let mut harness = live_http_harness(
        "room-qa-cross-app",
        [
            calls(vec![fetch(
                "call_read_hook_events",
                server.url("/hooks/v1/events?since=1h"),
            )]),
            calls(vec![
                post_json(
                    "call_append_crm_row",
                    server.url("/sheets/v4/spreadsheets/CRM/values/append"),
                    r#"{"values":[["Acme Corp","signup","dana@acme.example"]]}"#,
                ),
                post_json(
                    "call_notify_channel",
                    server.url("/chat/v1/alerts"),
                    r#"{"text":"New signup: Acme Corp (dana@acme.example)"}"#,
                ),
            ]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-cross-app",
            "any new signups from the webhook? add them to the CRM sheet and tell the channel",
        )
        .await
        .expect("submit cross-app request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("cross-app reply");

    let writes = server.request_bodies();
    assert_eq!(
        writes.len(),
        2,
        "BOTH downstream systems must be written — a partial fan-out is the bug"
    );
    let all_writes = writes.join("\n");
    assert!(
        all_writes.contains("Acme Corp") && all_writes.contains("dana@acme.example"),
        "both writes should carry the event payload, got {all_writes}"
    );

    let mut requested = server.requests();
    requested.sort();
    assert_eq!(
        requested,
        vec![
            "/chat/v1/alerts".to_string(),
            "/hooks/v1/events?since=1h".to_string(),
            "/sheets/v4/spreadsheets/CRM/values/append".to_string(),
        ]
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- 9. Lead generation & CRM automation ---------------------------------

/// Family 9: enrich and score a lead, then record only the qualified one.
///
/// Two candidates are fetched and enriched; only one clears the bar. The
/// unqualified candidate must reach the model (so scoring is a decision)
/// and must not appear in the CRM write.
#[tokio::test]
async fn reborn_qa_lead_scoring_records_only_the_qualified_lead() {
    const REPLY: &str = "Acme Corp scores 82 (500 seats, active trial) — added to the CRM. Sole Trader scores 21, left out.";

    let server = LiveLoopbackHttpServer::start(
        Router::new()
            .route("/leads/v1/candidates", get(lead_candidates))
            .route(
                "/sheets/v4/spreadsheets/CRM/values/append",
                post(append_row),
            ),
    )
    .await;
    let mut harness = live_http_harness(
        "room-qa-lead-scoring",
        [
            calls(vec![fetch(
                "call_fetch_leads",
                server.url("/leads/v1/candidates?segment=inbound"),
            )]),
            calls(vec![post_json(
                "call_record_qualified_lead",
                server.url("/sheets/v4/spreadsheets/CRM/values/append"),
                r#"{"values":[["Acme Corp","dana@acme.example",82,"qualified"]]}"#,
            )]),
            reply(REPLY),
        ],
        &server,
    )
    .await;

    let submitted = harness
        .submit_text(
            "event-qa-lead-scoring",
            "score this week's inbound leads and add the qualified ones to the CRM",
        )
        .await
        .expect("submit lead scoring request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness.assert_final_reply(REPLY).await.expect("lead reply");

    assert_result_body_contains(&harness, 0, "Acme Corp", "the qualified candidate");
    assert_result_body_contains(&harness, 0, "Sole Trader", "the unqualified candidate");

    let recorded = server.request_bodies();
    assert_eq!(recorded.len(), 1, "only the qualified lead is recorded");
    assert!(
        recorded[0].contains("Acme Corp"),
        "the qualified lead should be written, got {}",
        recorded[0]
    );
    assert!(
        !recorded[0].contains("Sole Trader"),
        "the unqualified lead must not be written, got {}",
        recorded[0]
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- 10. Personal & business tracking ------------------------------------

/// Family 10: tracking that persists in the user's own workspace rather
/// than a third-party system — the assistant appends an expense and can
/// read the ledger back in the SAME conversation.
///
/// Uses the workspace file capabilities rather than HTTP: this family's
/// whole point is that the record lives with the user, so the assertion is
/// against the persisted file on disk.
#[tokio::test]
async fn reborn_qa_expense_is_recorded_to_the_workspace_ledger_and_read_back() {
    const LEDGER: &str =
        "2026-08-04,ACME Cloud,148.20,infrastructure\n2026-08-04,Figma,45.00,software\n";
    const REPLY: &str = "Logged both: ACME Cloud £148.20 (infrastructure) and Figma £45.00 (software). Ledger total this month is £193.20.";

    let write_file = CapabilityId::new(WRITE_FILE_CAPABILITY_ID).expect("valid capability id");
    let read_file =
        CapabilityId::new(ironclaw_host_runtime::READ_FILE_CAPABILITY_ID).expect("valid id");

    let mut harness = RebornBinaryE2EHarness::with_host_runtime_file_capabilities(
        "room-qa-expense-tracking",
        RebornTraceReplayModelGateway::with_scripted_steps([
            calls(vec![RebornScriptedProviderToolCall::new(
                write_file.clone(),
                "call_append_expenses",
                json!({"path": "/workspace/tracking/expenses.csv", "content": LEDGER}),
            )]),
            calls(vec![RebornScriptedProviderToolCall::new(
                read_file.clone(),
                "call_read_ledger",
                json!({"path": "/workspace/tracking/expenses.csv"}),
            )]),
            reply(REPLY),
        ]),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-expense-tracking",
            "log these two expenses — ACME Cloud 148.20 and Figma 45.00 — and tell me the month's total",
        )
        .await
        .expect("submit expense tracking request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("expense reply");

    // Written, then read back through the real capability in the same turn —
    // the round trip, not just the write.
    let results = harness.capability_results();
    assert_eq!(results.len(), 2, "one write, one read-back");
    let read_back = format!("{:?}", results[1].output);
    assert!(
        read_back.contains("ACME Cloud") && read_back.contains("Figma"),
        "the ledger read-back should return both expenses, got {read_back}"
    );

    let ledger_path = harness
        .host_workspace_file_path("tracking/expenses.csv")
        .expect("workspace path");
    let persisted = std::fs::read_to_string(&ledger_path)
        .unwrap_or_else(|error| panic!("ledger should exist at {ledger_path:?}: {error}"));
    assert!(
        persisted.contains("148.20") && persisted.contains("45.00"),
        "both expense amounts should be on disk, got {persisted}"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- Chat-only setup ------------------------------------------------------

/// **No UI navigation required.** A user who has only a chat box must be
/// able to go from "not installed" to "the capability actually runs" by
/// talking to the agent.
///
/// Drives the real chat path: `builtin.extension_search` to find it,
/// `builtin.extension_install` to install it, then `extension_search` again
/// to confirm the installed phase — all inside ONE ordinary conversation,
/// with no operator route and no settings page involved.
///
/// The cross-thread visibility of that install is owned by
/// `tests/integration/group_extensions/scenario_install_then_visible_cross_thread.rs`;
/// what this adds is that the whole sequence is reachable from chat alone.
/// Broader chat-only configuration (secrets, channel config) is epic #7046.
#[tokio::test]
async fn reborn_qa_chat_only_extension_install_reaches_installed_state() {
    const REPLY: &str = "GitHub is installed and ready — you can ask me about your repos now.";

    let search = CapabilityId::new("builtin.extension_search").expect("valid capability id");
    let install = CapabilityId::new("builtin.extension_install").expect("valid capability id");

    let mut harness = RebornBinaryE2EHarness::with_host_runtime_extension_lifecycle_capabilities(
        "room-qa-chat-only-setup",
        RebornTraceReplayModelGateway::with_scripted_steps([
            calls(vec![RebornScriptedProviderToolCall::new(
                search.clone(),
                "call_find_github",
                json!({"query": "github"}),
            )]),
            calls(vec![RebornScriptedProviderToolCall::new(
                install.clone(),
                "call_install_github",
                json!({"extension_id": "github"}),
            )]),
            calls(vec![RebornScriptedProviderToolCall::new(
                search.clone(),
                "call_confirm_github",
                json!({"query": "github"}),
            )]),
            reply(REPLY),
        ]),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-chat-only-setup",
            "set me up with GitHub — I don't want to go hunting through settings",
        )
        .await
        .expect("submit chat-only setup request");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply(REPLY)
        .await
        .expect("setup reply");

    let invocations = harness.capability_invocations();
    assert_eq!(
        invocations.len(),
        3,
        "search → install → confirm, all from chat"
    );
    assert_eq!(invocations[1].capability_id, install);

    // The confirming search must show github as installed — proving the
    // install took effect within the same conversation rather than merely
    // returning a success envelope.
    let results = harness.capability_results();
    let confirmation = format!("{:?}", results[2].output);
    assert!(
        confirmation.contains("github"),
        "the confirming search should still find github, got {confirmation}"
    );
    assert!(
        confirmation.contains("installation_phase"),
        "the confirming search should report an installation phase for the \
         just-installed extension, got {confirmation}"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

// --- Fixtures -------------------------------------------------------------

async fn inbox_messages(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({
            "messages": [
                {
                    "id": "m-1",
                    "threadId": "t-escalation",
                    "from": "priya@customer.example",
                    "subject": "URGENT: CSV export failing for our whole team",
                    "snippet": "Every export since this morning times out at 30s.",
                },
                {
                    "id": "m-2",
                    "threadId": "t-newsletter",
                    "from": "weekly-digest@news.example",
                    "subject": "Your weekly industry digest",
                    "snippet": "Ten stories we think you should read.",
                },
            ],
        })),
    )
        .into_response()
}

async fn create_draft(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
    body: String,
) -> impl IntoResponse {
    state.record_with_body(&uri, &body);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({"id": "draft-1"})),
    )
        .into_response()
}

async fn meeting_transcript(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/markdown")],
        "# Acme quarterly review\n\nAgreed the pilot scope. Priya owns the export fix. \
Revisit pricing next month.\n",
    )
        .into_response()
}

async fn send_message(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
    body: String,
) -> impl IntoResponse {
    state.record_with_body(&uri, &body);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({"id": "sent-1"})),
    )
        .into_response()
}

/// Deliberately contains a genuine overlap (Acme 14:00-15:00 vs Platform
/// sync 14:30-15:30) AND a merely back-to-back pair (standup 09:30-09:45,
/// 1:1 09:45-10:00) that must NOT read as a conflict.
async fn calendar_today(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({
            "events": [
                {"summary": "Standup", "start": "2026-08-04T09:30:00Z", "end": "2026-08-04T09:45:00Z"},
                {"summary": "Weekly 1:1", "start": "2026-08-04T09:45:00Z", "end": "2026-08-04T10:00:00Z"},
                {"summary": "Acme quarterly review", "start": "2026-08-04T14:00:00Z", "end": "2026-08-04T15:00:00Z"},
                {"summary": "Platform sync", "start": "2026-08-04T14:30:00Z", "end": "2026-08-04T15:30:00Z"},
            ],
        })),
    )
        .into_response()
}

async fn tasks_due(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({
            "tasks": [
                {"id": "TASK-401", "title": "Renew the TLS certificate", "due": "2026-08-04"},
                {"id": "TASK-402", "title": "Send the Acme pilot scope", "due": "2026-08-04"},
            ],
        })),
    )
        .into_response()
}

async fn create_task(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
    body: String,
) -> impl IntoResponse {
    state.record_with_body(&uri, &body);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({"id": "TASK-412"})),
    )
        .into_response()
}

/// One untriaged issue (#812) and one already-labelled issue (#799) that
/// must be left alone.
async fn open_issues(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!([
            {"number": 812, "title": "CSV export times out on large workspaces", "labels": []},
            {"number": 799, "title": "Docs typo", "labels": ["already-triaged", "docs"]},
        ])),
    )
        .into_response()
}

async fn add_labels(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
    body: String,
) -> impl IntoResponse {
    state.record_with_body(&uri, &body);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({"ok": true})),
    )
        .into_response()
}

async fn add_comment(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
    body: String,
) -> impl IntoResponse {
    state.record_with_body(&uri, &body);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({"id": 99001})),
    )
        .into_response()
}

/// HTTP 200 with a FAILING payload — the shape a status-code-only health
/// check misreads as green.
async fn ci_runs(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({
            "workflow_runs": [
                {"id": 5521, "name": "integration-tests", "status": "completed", "conclusion": "failure"},
                {"id": 5520, "name": "unit-tests", "status": "completed", "conclusion": "success"},
            ],
        })),
    )
        .into_response()
}

async fn post_alert(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
    body: String,
) -> impl IntoResponse {
    state.record_with_body(&uri, &body);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({"ok": true})),
    )
        .into_response()
}

async fn webhook_events(State(state): State<LiveLoopbackHttpState>, uri: Uri) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({
            "events": [
                {"type": "signup", "company": "Acme Corp", "email": "dana@acme.example"},
            ],
        })),
    )
        .into_response()
}

async fn append_row(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
    body: String,
) -> impl IntoResponse {
    state.record_with_body(&uri, &body);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({"updates": {"updatedRows": 1}})),
    )
        .into_response()
}

/// One clearly qualified lead and one clearly unqualified one.
async fn lead_candidates(
    State(state): State<LiveLoopbackHttpState>,
    uri: Uri,
) -> impl IntoResponse {
    state.record(&uri);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({
            "candidates": [
                {"company": "Acme Corp", "email": "dana@acme.example", "seats": 500, "trial": "active"},
                {"company": "Sole Trader", "email": "me@soletrader.example", "seats": 1, "trial": "expired"},
            ],
        })),
    )
        .into_response()
}
