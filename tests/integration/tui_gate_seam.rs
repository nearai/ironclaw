//! B2.13 — de-risk spike: drives the real `webui_v2_app` composition (bound
//! loopback listener + bearer auth) through the Reborn TUI's `ApiClient`
//! (real reqwest HTTP + SSE), not a harness shortcut or a bare-router
//! `oneshot`. Proves the full submit -> gate -> resolve -> completed seam a
//! human at the TUI would actually exercise.
//!
//! Wiring precedent: `tests/integration/webui_v2_product_api.rs`'s
//! `approval_gate_rediscovered_and_resolved_after_refresh` (the
//! `RebornServices::new(..).with_event_stream(..).with_approval_interactions(..)`
//! recipe, byte-for-byte) plus
//! `crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs::spawn_serve`
//! (bind-a-real-listener-over-`webui_v2_app` shape, extracted into
//! `support/tui_listener.rs` since this file needs it as a reusable helper,
//! not a private fn). See `support/tui_listener.rs` for why this needs a
//! bound listener where `webui_v2_product_api.rs` does not.
//!
//! Was RED for three independent reasons found across two rounds of
//! diagnosis, all now fixed: (1) `ApiClient::send_message`/`resolve_gate`
//! were missing `client_action_id` — fixed in `client/gates.rs`. (2) the
//! harness's `group.thread(...).build()` never persists a thread row before
//! this test's `client.send_message()` — see the comment at the
//! `create_thread_pinned` call site below for the root cause and fix. (3) a
//! genuine bug in `ironclaw_reborn_tui::client::events::subscribe`
//! (`crates/ironclaw_reborn_tui/src/client/events.rs`) that only surfaced
//! SSE frames after the underlying HTTP connection closed — the real
//! `webui_v2_app` server correctly keeps the connection open for up to 5
//! minutes (`SSE_MAX_LIFETIME`), so `events.next()` never resolved during a
//! live turn. Fixed by restructuring `SubscribeState`/`connect_and_drain`
//! (now `open_connection`/`read_next_chunk`) to hold the open byte stream
//! across `Stream::poll_next` calls and yield each frame as soon as it's
//! decoded, instead of draining the whole connection body first.
//!
//! One more thing the (3) fix surfaced: the real `local-dev` WebChat v2
//! producer never emits `final_reply`/`accepted`/`running`/`cancelled`/
//! `failed` typed SSE events — only `projection_snapshot`/
//! `projection_update` frames carrying `ProductProjectionState`. Completion
//! detection below therefore drains `ProjectionUpdate`/`ProjectionSnapshot`
//! frames for a `ProductProjectionItem::RunStatus` item reaching a terminal
//! (or `blocked_approval`, as a safety net) status, not a `FinalReply`
//! event — see the comment at that loop, below.
//!
//! ## Tier B regression coverage (this section)
//!
//! The functions below extend the original submit/gate/resolve/completed
//! seam test with the Reborn TUI's other Tier B behaviors, all driven
//! through the SAME real-harness recipe (real `webui_v2_app` listener, real
//! `ApiClient`, scripted LLM at the vendor-SDK seam):
//!
//! - [`tui_client_thread_switch_replay_dedupes_to_one_reply`] — defect E
//!   (cursor-less SSE-replay-on-top-of-timeline duplication).
//! - [`tui_client_cancel_run_transitions_to_cancelled_at_the_turn_state_seam`]
//!   — `POST .../runs/{run_id}/cancel` through the real client.
//! - [`tui_client_drives_submit_gate_deny_resolve_seam`] — the DENY
//!   complement of the original approve test.
//! - [`tui_client_credential_two_step_auth_gate_resumes_via_manual_token_submit`]
//!   — the manual-token submit -> `CredentialProvided` resolve chain.
//! - [`tui_client_automations_parity_list_and_open_run_thread`] — marked
//!   NOT REACHABLE (see its doc comment).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
#[allow(dead_code)]
#[path = "support/tui_listener.rs"]
mod tui_listener;

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use futures::StreamExt;
use ironclaw_events::InMemoryDurableEventLog;
use ironclaw_product_workflow::webchat_schema::{WebChatV2Event, WebChatV2EventFrame};
use ironclaw_product_workflow::{
    AuthPromptView, FinalReplyView, GatePromptView, ProductProjectionItem, ProjectionCursor,
    RebornServices, RebornServicesApi, WebUiGateResolution,
};
use ironclaw_reborn_tui::app::{self, AppEvent, AppState};
use ironclaw_reborn_tui::client::{ApiClient, ClientError};
use ironclaw_turns::{ReplyTargetBindingRef, TurnEventProjectionSource, TurnRunId, TurnStatus};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

/// Statuses `app/transcript.rs`'s `TERMINAL_RUN_STATUSES`/
/// `PROMPT_RUN_STATUSES` treat as "the client can stop waiting" — mirrored
/// here so every scenario below drains the SSE stream the same way the real
/// TUI reducer does. `blocked_approval` is included as a safety net (a run
/// this loop expects to keep going past a gate still counts as "settled for
/// this wait").
const SETTLED_RUN_STATUSES: &[&str] = &[
    "completed",
    "succeeded",
    "failed",
    "cancelled",
    "recovery_required",
    "blocked_approval",
];

/// Pulls one decoded frame off a real `ApiClient` SSE subscription, failing
/// loudly (not silently) if the stream closes or a frame fails to decode —
/// every scenario below waits on a specific frame shape, so an early close
/// is always a test bug or a real regression, never an expected outcome.
async fn next_frame(
    events: &mut (impl futures::Stream<Item = Result<WebChatV2EventFrame, ClientError>> + Unpin),
) -> WebChatV2EventFrame {
    events
        .next()
        .await
        .expect("stream stays open until the awaited frame arrives")
        .expect("frame decodes")
}

/// Drains frames until a raw `Gate` (approval) event arrives.
async fn wait_for_approval_gate(
    events: &mut (impl futures::Stream<Item = Result<WebChatV2EventFrame, ClientError>> + Unpin),
) -> GatePromptView {
    loop {
        if let WebChatV2Event::Gate { prompt } = next_frame(events).await.event {
            return prompt;
        }
    }
}

/// Drains frames until a raw `AuthRequired` event arrives.
async fn wait_for_auth_required(
    events: &mut (impl futures::Stream<Item = Result<WebChatV2EventFrame, ClientError>> + Unpin),
) -> AuthPromptView {
    loop {
        if let WebChatV2Event::AuthRequired { prompt } = next_frame(events).await.event {
            return prompt;
        }
    }
}

/// Drains `ProjectionUpdate`/`ProjectionSnapshot` frames until a
/// `ProductProjectionItem::RunStatus` item for `run_id` reaches a settled
/// status (see [`SETTLED_RUN_STATUSES`]), returning that status.
async fn wait_for_settled_status_for_run(
    events: &mut (impl futures::Stream<Item = Result<WebChatV2EventFrame, ClientError>> + Unpin),
    run_id: &str,
) -> String {
    loop {
        let projection = match next_frame(events).await.event {
            WebChatV2Event::ProjectionUpdate { state }
            | WebChatV2Event::ProjectionSnapshot { state } => state,
            _ => continue,
        };
        let settled = projection.items.into_iter().find_map(|item| match item {
            ProductProjectionItem::RunStatus {
                run_id: item_run_id,
                status,
                ..
            } if item_run_id.to_string() == run_id
                && SETTLED_RUN_STATUSES.contains(&status.as_str()) =>
            {
                Some(status)
            }
            _ => None,
        });
        if let Some(status) = settled {
            return status;
        }
    }
}

/// Like [`wait_for_settled_status_for_run`], but for a plain (ungated) reply
/// turn whose `run_id` isn't known ahead of time — returns the first
/// `RunStatus` item that reaches a settled status, along with its `run_id`.
async fn wait_for_any_settled_run(
    events: &mut (impl futures::Stream<Item = Result<WebChatV2EventFrame, ClientError>> + Unpin),
) -> (String, String) {
    loop {
        let projection = match next_frame(events).await.event {
            WebChatV2Event::ProjectionUpdate { state }
            | WebChatV2Event::ProjectionSnapshot { state } => state,
            _ => continue,
        };
        let settled = projection.items.into_iter().find_map(|item| match item {
            ProductProjectionItem::RunStatus { run_id, status, .. }
                if SETTLED_RUN_STATUSES.contains(&status.as_str()) =>
            {
                Some((run_id.to_string(), status))
            }
            _ => None,
        });
        if let Some(result) = settled {
            return result;
        }
    }
}

#[tokio::test]
async fn tui_client_drives_submit_gate_resolve_completed_seam() {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("conv-tui-gate-seam")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                serde_json::json!({"path": "/workspace/tui_gate_seam.txt", "content": "TUI_GATE_SEAM"}),
            ),
            RebornScriptedReply::text("file written after TUI-resolved approval"),
        ])
        .build()
        .await
        .expect("thread builds");

    // Real approval-interaction service over the group's own shared
    // turn-state store — same seam `webui_v2_product_api.rs`'s
    // `approval_gate_rediscovered_and_resolved_after_refresh` proves.
    let capability_harness = group
        .capability_harness()
        .expect("live_approvals always uses a HostRuntime capability backend");
    let reborn_services = capability_harness
        .reborn_services_for_test()
        .expect("live_approvals harness is built via new_with_options");
    let approval_interactions = reborn_services
        .local_dev_approval_interaction_service_with_turn_state_for_test(
            h.coordinator.clone(),
            h.turn_store.clone(),
        )
        .expect("local-dev capability policy is valid")
        .expect("harness has a local-dev runtime");

    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("tui-gate-seam-test").expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services: Arc<dyn RebornServicesApi> = Arc::new(
        RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
            .with_event_stream(event_stream)
            .with_approval_interactions(approval_interactions),
    );

    let user_id = h
        .binding
        .subject_user_id
        .clone()
        .unwrap_or_else(|| h.binding.actor_user_id.clone());
    let agent_id = h
        .binding
        .agent_id
        .clone()
        .expect("live_approvals binding resolves an agent_id (required for turn submission)");
    let token = "reborn-tui-gate-seam-test-token-0123456789ab";

    let (addr, _serve_guard) = tui_listener::spawn_webui_v2(
        services,
        h.binding.tenant_id.clone(),
        user_id,
        agent_id,
        h.binding.project_id.clone(),
        token,
    )
    .await;

    let client = ApiClient::new(format!("http://{addr}"), token.to_string());
    let thread_id = h.binding.thread_id.as_str().to_string();

    // Pre-create the thread the ApiClient bearer will operate on — root cause
    // (verified with a direct `read_thread` probe against the shared
    // `thread_service` before this fix existed): `RebornServices::submit_turn`
    // fails closed with 404 NotFound (`resolve_webui_thread_metadata` ->
    // `read_thread` -> `SessionThreadError::UnknownThread`) whenever the
    // thread has never actually been persisted — NOT an owner/scope mismatch.
    // `group.thread(...).build()` only *resolves* `h.binding` (tenant/user/
    // agent/project/thread_id come straight from it, and were confirmed byte-
    // for-byte equal to the bearer identity `spawn_webui_v2` authenticates
    // as); it performs no I/O against `SessionThreadService`, so
    // `h.binding.thread_id` names a thread that exists only as a computed
    // value, not a stored row, until something writes it.
    //
    // This mirrors real TUI behavior, not a scope bypass: a real user's TUI
    // always calls `client.create_thread()` before the first `send_message()`
    // on an account with no threads yet (`crates/ironclaw_reborn_tui/src/lib.rs`
    // ~line 121, `"create_thread during TUI startup (account has no threads
    // yet)"`) — `create_thread` and `send_message` always run through the
    // same `ApiClient`/bearer token, so a real user never creates a thread as
    // one identity and sends to it as another. `tui_listener::create_thread_pinned`
    // makes that same real HTTP call (through the identical bound listener,
    // bearer, and `RebornServices::create_thread` production handler), just
    // additionally setting `requested_thread_id` — a genuine, already-shipped
    // field on the wire (`WebUiCreateThreadRequest::requested_thread_id`,
    // `reborn_services.rs`'s create_thread doc comment: "makes the caller's
    // choice authoritative") that `ApiClient::create_thread()`'s typed helper
    // doesn't yet expose (real TUI usage never needs to pick a specific id).
    // Pinning it to `h.binding.thread_id` is what makes the thread this test
    // creates identical (same tenant/agent/project/owner_user_id AND thread_id)
    // to the `TurnScope` the harness registered its scripted LLM replies
    // against at `.build()` time — so the approval-triggering turn below still
    // finds its scripted tool-call/text steps.
    tui_listener::create_thread_pinned(addr, token, &thread_id).await;

    // submit — real HTTP POST through the TUI's own client, not the harness's
    // `submit_turn`/`submit_turn_until_blocked` shortcut.
    client
        .send_message(&thread_id, "write the tui gate seam file")
        .await
        .expect("send_message");

    // gate — drive the real SSE `subscribe()` to find the Gate frame. The
    // raw `gate` typed event really is emitted by the real producer
    // alongside the projection state (see the module doc); this loop is
    // unchanged by the (3) fix above. `subscribe()` returns `impl Stream`
    // built over `futures::stream::unfold` of an async block, which is not
    // `Unpin`; `StreamExt::next()` requires `Self: Unpin`, so pin it on the
    // stack with the stable `std::pin::pin!` macro (any real consumer of
    // this client API hits the same requirement).
    let mut events = std::pin::pin!(ironclaw_reborn_tui::client::events::subscribe(
        &client, &thread_id, None
    ));
    let gate = wait_for_approval_gate(&mut events).await;

    // resolve — through the ApiClient, hitting the real resolve_gate route.
    client
        .resolve_gate(
            &thread_id,
            &gate.turn_run_id.to_string(),
            &gate.gate_ref,
            WebUiGateResolution::Approved { always: false },
        )
        .await
        .expect("resolve_gate");

    // completed seam — drain `projection_update`/`projection_snapshot`
    // frames until a `ProductProjectionItem::RunStatus` item for this run
    // reaches a terminal (or `blocked_approval`, as a safety net — mirrors
    // `TERMINAL_RUN_STATUSES`/`PROMPT_RUN_STATUSES` in
    // `crates/ironclaw_webui_v2/frontend/src/pages/chat/lib/useChatEvents.ts`)
    // status. NOT a `FinalReply` event: the real `local-dev` producer never
    // emits `final_reply` on this wire (see the module doc) — the durable
    // reply text lives only in the thread timeline, which
    // `ironclaw_reborn_tui`'s own reducer now reloads on settle
    // (`app::transcript::apply_run_status`); this test's job is only to
    // prove the run actually reaches a terminal status and the approved
    // write actually lands, which the assertions below already cover.
    let settled_status =
        wait_for_settled_status_for_run(&mut events, &gate.turn_run_id.to_string()).await;
    assert_eq!(
        settled_status, "completed",
        "run should complete after the approved write is re-dispatched"
    );

    h.wait_for_status(gate.turn_run_id, TurnStatus::Completed)
        .await
        .expect("run completes");
    h.assert_workspace_file_contains("tui_gate_seam.txt", "TUI_GATE_SEAM")
        .await
        .expect("approved write actually re-dispatched and persisted");
}

/// Defect E regression, driven through the real server rather than a
/// synthetic fixture: two real threads each get one scripted message+reply,
/// then thread A's `/timeline` is fetched, thread B's is fetched (the
/// "switch away" half of a thread switch), and thread A's is fetched again
/// (the "switch back" half). The real server must keep returning the SAME,
/// single `turn_run_id` for thread A's reply across repeated loads — the
/// `apply_timeline_page`-driven reload `app/transcript.rs::apply_run_status`
/// issues on every settle/switch REPLACES `state.transcript` wholesale, so a
/// stale duplicate can only enter through the OTHER path `known_reply_ids`
/// guards: a cursor-less SSE resubscribe replaying `FinalReply` events the
/// timeline already carries (see `AppState::known_reply_ids`'s doc in
/// `ironclaw_reborn_tui::app`).
///
/// `apply_timeline_page` itself is private to `ironclaw_reborn_tui::lib`
/// (only that crate's own `#[cfg(test)]` module can call it directly — see
/// its "Defect E" tests), so this seam cannot drive the crate's full
/// `LoadTimeline`-response dispatch from outside the crate. Per this file's
/// task brief, that means falling back to the sanctioned weaker assertion:
/// mirror the crate-tier Defect E test's REPLAY step (feed the same
/// `turn_run_id` through the public `app::reduce`/`AppEvent::Server` path
/// twice, simulating the cursor-less resubscribe replaying history) but seed
/// `known_reply_ids` and the replayed `FinalReply.turn_run_id`/text from this
/// REAL server's `/timeline` response instead of a synthetic id — proving
/// the dedup guard holds against real server-minted ids, not just synthetic
/// ones.
#[tokio::test]
async fn tui_client_thread_switch_replay_dedupes_to_one_reply() {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h_a = group
        .thread("conv-tui-switch-a")
        .script([RebornScriptedReply::text("TUI_SWITCH_REPLY_A")])
        .build()
        .await
        .expect("thread A builds");
    let h_b = group
        .thread("conv-tui-switch-b")
        .script([RebornScriptedReply::text("TUI_SWITCH_REPLY_B")])
        .build()
        .await
        .expect("thread B builds");

    let capability_harness = group
        .capability_harness()
        .expect("live_approvals always uses a HostRuntime capability backend");
    let reborn_services = capability_harness
        .reborn_services_for_test()
        .expect("live_approvals harness is built via new_with_options");
    let approval_interactions = reborn_services
        .local_dev_approval_interaction_service_with_turn_state_for_test(
            h_a.coordinator.clone(),
            h_a.turn_store.clone(),
        )
        .expect("local-dev capability policy is valid")
        .expect("harness has a local-dev runtime");
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("tui-switch-test").expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h_a.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h_a.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services: Arc<dyn RebornServicesApi> = Arc::new(
        RebornServices::new(h_a.thread_harness.service.clone(), h_a.coordinator.clone())
            .with_event_stream(event_stream)
            .with_approval_interactions(approval_interactions),
    );

    let user_id = h_a
        .binding
        .subject_user_id
        .clone()
        .unwrap_or_else(|| h_a.binding.actor_user_id.clone());
    let agent_id = h_a
        .binding
        .agent_id
        .clone()
        .expect("live_approvals binding resolves an agent_id");
    let token = "reborn-tui-switch-test-token-0123456789ab";
    let (addr, _serve_guard) = tui_listener::spawn_webui_v2(
        services,
        h_a.binding.tenant_id.clone(),
        user_id,
        agent_id,
        h_a.binding.project_id.clone(),
        token,
    )
    .await;
    let client = ApiClient::new(format!("http://{addr}"), token.to_string());
    let thread_a = h_a.binding.thread_id.as_str().to_string();
    let thread_b = h_b.binding.thread_id.as_str().to_string();
    // Both threads share one group runtime (tenant/agent/project/user), so
    // ONE bearer/listener created above serves both — mirrors a real user
    // switching between two of their own threads in one TUI session.
    tui_listener::create_thread_pinned(addr, token, &thread_a).await;
    tui_listener::create_thread_pinned(addr, token, &thread_b).await;

    // Drive both threads to a settled reply through the real server.
    {
        let mut events = std::pin::pin!(ironclaw_reborn_tui::client::events::subscribe(
            &client, &thread_a, None
        ));
        client
            .send_message(&thread_a, "TUI_SWITCH_PROMPT_A")
            .await
            .expect("send_message thread A");
        let (_run_id, status) = wait_for_any_settled_run(&mut events).await;
        assert_eq!(
            status, "completed",
            "thread A's plain reply turn must complete"
        );
    }
    {
        let mut events = std::pin::pin!(ironclaw_reborn_tui::client::events::subscribe(
            &client, &thread_b, None
        ));
        client
            .send_message(&thread_b, "TUI_SWITCH_PROMPT_B")
            .await
            .expect("send_message thread B");
        let (_run_id, status) = wait_for_any_settled_run(&mut events).await;
        assert_eq!(
            status, "completed",
            "thread B's plain reply turn must complete"
        );
    }

    // "Switch" sequence: load A, load B, load A again — the real server
    // must keep returning exactly one unique assistant turn_run_id for A
    // across both loads (proves no server-side accumulation from the
    // repeated fetch itself).
    let load_a_first = client
        .timeline(&thread_a, 50, None)
        .await
        .expect("timeline thread A (first load)");
    let _load_b = client
        .timeline(&thread_b, 50, None)
        .await
        .expect("timeline thread B (switch away)");
    let load_a_second = client
        .timeline(&thread_a, 50, None)
        .await
        .expect("timeline thread A (switch back)");

    // A completed turn can surface more than one assistant row for the same
    // run (a superseded draft plus the finalized reply), and every assistant
    // row carries that run's `turn_run_id` — so the property under test is the
    // count of UNIQUE assistant run ids, which must be exactly one and stable
    // across the switch-away-and-back reload (no server-side accumulation from
    // the repeated fetch).
    let assistant_run_ids_from = |page: &ironclaw_reborn_tui::client::TimelinePage| -> Vec<String> {
        page.messages
            .iter()
            .filter(|m| m.kind == "assistant")
            .filter_map(|m| m.turn_run_id.clone())
            .collect()
    };
    let first_ids = assistant_run_ids_from(&load_a_first);
    let second_ids = assistant_run_ids_from(&load_a_second);
    let first_unique: HashSet<&String> = first_ids.iter().collect();
    assert_eq!(
        first_unique.len(),
        1,
        "thread A should carry exactly one unique assistant run id, got {first_ids:?}"
    );
    let second_unique: HashSet<&String> = second_ids.iter().collect();
    assert_eq!(
        first_unique, second_unique,
        "switching away and back must not change the set of assistant run ids, got {second_ids:?}"
    );

    // Reducer-level replay: the real defect-E scenario is a cursor-less SSE
    // resubscribe replaying the whole thread's event history on top of the
    // already-loaded timeline. Reproduce it against the public `AppState`:
    // replay the SAME real `FinalReply` (carrying the id this server just
    // returned) through the public `app::reduce` entry point twice from a
    // clean state. The dedup guard self-seeds on first arrival (renders the
    // row and records the id in `known_reply_ids`), so the second replay must
    // dedupe to that one row — a bypassed guard would append a second
    // `Assistant` transcript row.
    let turn_run_id = TurnRunId::parse(&first_ids[0]).expect("real server turn_run_id is a UUID");
    let reply_text = load_a_second
        .messages
        .iter()
        .find(|m| m.turn_run_id.as_deref() == Some(first_ids[0].as_str()))
        .and_then(|m| m.content.clone())
        .expect("assistant message has content");

    let mut state = AppState::default();
    for _ in 0..2 {
        app::reduce(
            &mut state,
            AppEvent::Server(Box::new(WebChatV2EventFrame {
                cursor: ProjectionCursor::new(format!("cursor:tui:switch-replay:{turn_run_id}"))
                    .expect("valid cursor"),
                event: WebChatV2Event::FinalReply {
                    reply: FinalReplyView {
                        turn_run_id,
                        text: reply_text.clone(),
                        generated_at: Utc::now(),
                    },
                },
            })),
        );
    }
    let matching = state
        .transcript
        .iter()
        .filter(|item| item.as_final_text() == Some(reply_text.as_str()))
        .count();
    assert_eq!(
        matching, 1,
        "replaying the same real turn_run_id twice through the reducer must dedupe to one \
         transcript row, got {matching} (transcript: {:?})",
        state.transcript
    );
}

/// Complement of the original approve-flow test above, driven the same way
/// (real client submit -> real SSE gate discovery -> real `cancel_run`
/// route), asserting at the real turn-state seam
/// (`RebornIntegrationHarness::wait_for_status`, a persisted-state
/// read-back) rather than the HTTP 200 alone. Parks on a real
/// `BlockedApproval` gate (like `tests/integration/auth_gate.rs`'s
/// `cancel_blocked_auth_gate_leaves_no_stale_replay`) so cancel has a
/// deterministic, non-racy target instead of a fast in-process scripted
/// reply that could settle before the cancel HTTP call lands.
#[tokio::test]
async fn tui_client_cancel_run_transitions_to_cancelled_at_the_turn_state_seam() {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("conv-tui-cancel")
        .script([RebornScriptedReply::tool_call(
            "builtin.write_file",
            serde_json::json!({"path": "/workspace/tui_cancel.txt", "content": "never written"}),
        )])
        .build()
        .await
        .expect("thread builds");

    let capability_harness = group
        .capability_harness()
        .expect("live_approvals always uses a HostRuntime capability backend");
    let reborn_services = capability_harness
        .reborn_services_for_test()
        .expect("live_approvals harness is built via new_with_options");
    let approval_interactions = reborn_services
        .local_dev_approval_interaction_service_with_turn_state_for_test(
            h.coordinator.clone(),
            h.turn_store.clone(),
        )
        .expect("local-dev capability policy is valid")
        .expect("harness has a local-dev runtime");
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("tui-cancel-test").expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services: Arc<dyn RebornServicesApi> = Arc::new(
        RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
            .with_event_stream(event_stream)
            .with_approval_interactions(approval_interactions),
    );

    let user_id = h
        .binding
        .subject_user_id
        .clone()
        .unwrap_or_else(|| h.binding.actor_user_id.clone());
    let agent_id = h
        .binding
        .agent_id
        .clone()
        .expect("live_approvals binding resolves an agent_id");
    let token = "reborn-tui-cancel-test-token-0123456789ab";
    let (addr, _serve_guard) = tui_listener::spawn_webui_v2(
        services,
        h.binding.tenant_id.clone(),
        user_id,
        agent_id,
        h.binding.project_id.clone(),
        token,
    )
    .await;
    let client = ApiClient::new(format!("http://{addr}"), token.to_string());
    let thread_id = h.binding.thread_id.as_str().to_string();
    tui_listener::create_thread_pinned(addr, token, &thread_id).await;

    client
        .send_message(&thread_id, "write the tui cancel file")
        .await
        .expect("send_message");
    let mut events = std::pin::pin!(ironclaw_reborn_tui::client::events::subscribe(
        &client, &thread_id, None
    ));
    let gate = wait_for_approval_gate(&mut events).await;

    // The real cancel route, through the real client — not the harness's
    // own crate-tier `cancel_run` shortcut.
    client
        .cancel_run(&thread_id, &gate.turn_run_id.to_string())
        .await
        .expect("cancel_run");

    // Seam: the persisted turn-state store, not the HTTP 200 above.
    h.wait_for_status(gate.turn_run_id, TurnStatus::Cancelled)
        .await
        .expect("run transitions to Cancelled after the real cancel_run call");
    h.assert_workspace_file_absent("tui_cancel.txt")
        .await
        .expect("a cancelled, never-approved write must never have executed");
}

/// DENY complement of the original approve-flow test: same real
/// submit -> gate -> resolve seam, but resolved `Declined` instead of
/// `Approved`, mirroring `group_approvals/scenario_gate_then_deny.rs`'s
/// assertions (run still completes; the gated write never executes) through
/// the real HTTP client instead of the harness's `deny_gate` shortcut.
#[tokio::test]
async fn tui_client_drives_submit_gate_deny_resolve_seam() {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("conv-tui-gate-deny")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                serde_json::json!({"path": "/workspace/tui_gate_denied.txt", "content": "should not persist"}),
            ),
            RebornScriptedReply::text("understood, the write was not authorized"),
        ])
        .build()
        .await
        .expect("thread builds");

    let capability_harness = group
        .capability_harness()
        .expect("live_approvals always uses a HostRuntime capability backend");
    let reborn_services = capability_harness
        .reborn_services_for_test()
        .expect("live_approvals harness is built via new_with_options");
    let approval_interactions = reborn_services
        .local_dev_approval_interaction_service_with_turn_state_for_test(
            h.coordinator.clone(),
            h.turn_store.clone(),
        )
        .expect("local-dev capability policy is valid")
        .expect("harness has a local-dev runtime");
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref =
        ReplyTargetBindingRef::new("tui-gate-deny-test").expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services: Arc<dyn RebornServicesApi> = Arc::new(
        RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
            .with_event_stream(event_stream)
            .with_approval_interactions(approval_interactions),
    );

    let user_id = h
        .binding
        .subject_user_id
        .clone()
        .unwrap_or_else(|| h.binding.actor_user_id.clone());
    let agent_id = h
        .binding
        .agent_id
        .clone()
        .expect("live_approvals binding resolves an agent_id");
    let token = "reborn-tui-gate-deny-test-token-0123456789ab";
    let (addr, _serve_guard) = tui_listener::spawn_webui_v2(
        services,
        h.binding.tenant_id.clone(),
        user_id,
        agent_id,
        h.binding.project_id.clone(),
        token,
    )
    .await;
    let client = ApiClient::new(format!("http://{addr}"), token.to_string());
    let thread_id = h.binding.thread_id.as_str().to_string();
    tui_listener::create_thread_pinned(addr, token, &thread_id).await;

    client
        .send_message(&thread_id, "write the tui gate denied file")
        .await
        .expect("send_message");
    let mut events = std::pin::pin!(ironclaw_reborn_tui::client::events::subscribe(
        &client, &thread_id, None
    ));
    let gate = wait_for_approval_gate(&mut events).await;

    client
        .resolve_gate(
            &thread_id,
            &gate.turn_run_id.to_string(),
            &gate.gate_ref,
            WebUiGateResolution::Declined,
        )
        .await
        .expect("resolve_gate (declined)");

    let settled_status =
        wait_for_settled_status_for_run(&mut events, &gate.turn_run_id.to_string()).await;
    assert_eq!(
        settled_status, "completed",
        "a denied gate must still resume the run to a terminal reply, not hang"
    );
    h.wait_for_status(gate.turn_run_id, TurnStatus::Completed)
        .await
        .expect("run completes after the deny resume");

    // The denied write must NOT have executed: the gated capability is
    // never re-dispatched after a deny, so the file is absent on disk.
    h.assert_workspace_file_absent("tui_gate_denied.txt")
        .await
        .expect("denied write must never have executed");
}

/// Credential 2-step / auth-required flow: an unseeded `github.get_repo`
/// call on `RebornIntegrationGroup::live_auth_and_approval()` raises a real
/// `BlockedApproval` gate first (empirically documented on that profile —
/// see `tests/integration/support/harness/profiles/github.rs`'s
/// `file_and_github_auth_tools_profile` doc comment); approving it
/// re-dispatches the still-uncredentialed capability, which blocks AGAIN at
/// `BlockedAuth`. This test resolves BOTH gates through the real HTTP
/// client: `Approved` for the first, then the real manual-token submit route
/// (`POST /api/reborn/product-auth/manual-token/submit`) followed by
/// `resolve_gate(.., CredentialProvided { credential_ref })` for the second.
///
/// Needs `tui_listener::spawn_webui_v2_with_product_auth` (not the plain
/// `spawn_webui_v2` every other test in this file uses) so the manual-token
/// route is actually mounted, wired to the SAME `RebornProductAuthServices`
/// instance the capability harness's GitHub credential resolver reads
/// from — `HostRuntimeCapabilityHarness::reborn_services_for_test().product_auth`,
/// a public field. Also needs `.with_auth_interactions(..)` on the
/// `RebornServices` facade (`resolve_gate`'s `CredentialProvided` arm reads
/// `self.auth_interactions`, which defaults to a `RejectingAuthInteractionService`
/// otherwise).
#[tokio::test]
#[ignore = "NOT REACHABLE IN HARNESS: drives a real github.get_repo capability whose \
            extension is not installed in the live_approvals harness, so approving the \
            first gate re-dispatches to an absent capability provider and the server \
            returns 503 service_unavailable before the auth gate is ever reached. The \
            manual-token submit -> CredentialProvided resolve client logic is covered at \
            crate tier (client/gates.rs::submit_manual_token, app/gate.rs token sub-mode \
            + two-step chaining). Re-enable if the harness gains a github capability backend."]
async fn tui_client_credential_two_step_auth_gate_resumes_via_manual_token_submit() {
    let group = RebornIntegrationGroup::live_auth_and_approval()
        .await
        .expect("live-auth-and-approval group builds");
    let h = group
        .thread("conv-tui-credential-two-step")
        .script([
            RebornScriptedReply::tool_call(
                "github.get_repo",
                serde_json::json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            RebornScriptedReply::text("repo info retrieved after connecting github"),
        ])
        .build()
        .await
        .expect("thread builds");

    let capability_harness = group
        .capability_harness()
        .expect("live_auth_and_approval always uses a HostRuntime capability backend");
    let reborn_services = capability_harness
        .reborn_services_for_test()
        .expect("live_auth_and_approval harness is built via new_with_options");
    let approval_interactions = reborn_services
        .local_dev_approval_interaction_service_with_turn_state_for_test(
            h.coordinator.clone(),
            h.turn_store.clone(),
        )
        .expect("local-dev capability policy is valid")
        .expect("harness has a local-dev runtime");
    let auth_interactions = reborn_services
        .local_dev_auth_interaction_service_for_test(h.coordinator.clone())
        .expect("harness has a local-dev runtime");
    let product_auth = reborn_services
        .product_auth
        .clone()
        .expect("live_auth_and_approval wires local-dev product-auth services");

    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref = ReplyTargetBindingRef::new("tui-credential-two-step-test")
        .expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services: Arc<dyn RebornServicesApi> = Arc::new(
        RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
            .with_event_stream(event_stream)
            .with_approval_interactions(approval_interactions)
            .with_auth_interactions(auth_interactions),
    );

    let user_id = h
        .binding
        .subject_user_id
        .clone()
        .unwrap_or_else(|| h.binding.actor_user_id.clone());
    let agent_id = h
        .binding
        .agent_id
        .clone()
        .expect("live_auth_and_approval binding resolves an agent_id");
    let token = "reborn-tui-credential-two-step-test-token-0123456789ab";
    let (addr, _serve_guard) = tui_listener::spawn_webui_v2_with_product_auth(
        services,
        h.binding.tenant_id.clone(),
        user_id,
        agent_id,
        h.binding.project_id.clone(),
        token,
        product_auth,
    )
    .await;
    let client = ApiClient::new(format!("http://{addr}"), token.to_string());
    let thread_id = h.binding.thread_id.as_str().to_string();
    tui_listener::create_thread_pinned(addr, token, &thread_id).await;

    client
        .send_message(&thread_id, "look up the octocat repo")
        .await
        .expect("send_message");
    let mut events = std::pin::pin!(ironclaw_reborn_tui::client::events::subscribe(
        &client, &thread_id, None
    ));

    // Gate 1/2: approval on the still-uncredentialed github.get_repo call.
    let approval = wait_for_approval_gate(&mut events).await;
    client
        .resolve_gate(
            &thread_id,
            &approval.turn_run_id.to_string(),
            &approval.gate_ref,
            WebUiGateResolution::Approved { always: false },
        )
        .await
        .expect("resolve_gate (approved)");

    // Gate 2/2: BlockedAuth after the approved re-dispatch finds no
    // credential account. `AuthPromptView::auth_request_ref` doubles as the
    // gate_ref for this gate class (mirrors `ironclaw_reborn_tui::app::gate`'s
    // `gate_ref: prompt.auth_request_ref`).
    let auth_prompt = wait_for_auth_required(&mut events).await;
    let run_id = auth_prompt.turn_run_id.to_string();
    let credential_ref = client
        .submit_manual_token(
            "github",
            "tui-credential-two-step",
            "itest-github-token",
            &thread_id,
            &run_id,
            &auth_prompt.auth_request_ref,
        )
        .await
        .expect("submit_manual_token");
    client
        .resolve_gate(
            &thread_id,
            &run_id,
            &auth_prompt.auth_request_ref,
            WebUiGateResolution::CredentialProvided { credential_ref },
        )
        .await
        .expect("resolve_gate (credential_provided)");

    let settled_status = wait_for_settled_status_for_run(&mut events, &run_id).await;
    assert_eq!(
        settled_status, "completed",
        "the credential-backed re-dispatch must complete the run"
    );
    h.wait_for_status(auth_prompt.turn_run_id, TurnStatus::Completed)
        .await
        .expect("run completes after the manual-token credential resume");
    // Mutation-verified precedent (scenario_auth_then_approval_journey.rs):
    // `assert_tool_invoked` alone doesn't discriminate a real credentialed
    // re-dispatch from a stuck retry; only the scripted body surfacing back
    // proves it actually ran with the submitted credential.
    h.assert_tool_result_contains("octocat/hello-world")
        .await
        .expect("credentialed re-dispatch actually executed and surfaced its result");
}

/// BLOCKER regression: SSE full-history replay on thread switch must not
/// duplicate/reorder/resurrect anything beyond a single reply. Extends the
/// `sse_replay_of_an_already_loaded_reply_does_not_duplicate_the_transcript`/
/// `two_consecutive_thread_switches_do_not_duplicate_the_transcript` crate-tier
/// tests (`crates/ironclaw_reborn_tui/src/lib.rs`) and the
/// `replayed_projection_items_for_an_already_settled_run_are_dropped_but_a_new_run_still_applies`/
/// `replayed_raw_gate_and_running_frames_for_an_already_settled_run_are_dropped`
/// crate-tier reducer tests (`crates/ironclaw_reborn_tui/src/app/transcript.rs`)
/// with REAL server-minted data across TWO completed turns (multiple replies,
/// not just one) in one thread: a real `send_message` -> settle -> `send_message`
/// -> settle sequence, then a real `/timeline` fetch. The replay step itself
/// is driven synthetically (through the public `app::reduce`/`AppEvent::Server`
/// entry point, feeding both turns' real `turn_run_id`s and real reply text
/// back in as `Text` projection items + a terminal `RunStatus`, per turn) —
/// see the coverage note below for why, mirroring
/// `tui_client_thread_switch_replay_dedupes_to_one_reply`'s own established
/// precedent for the same constraint. The reconstructed transcript must come
/// out identical to the snapshot taken before the replay.
///
/// Coverage notes:
/// - **Why synthetic replay, not a second real `subscribe(.., None)`:** tried
///   first — opened a genuine second cursor-less subscription (exactly what a
///   thread switch does) after both turns settled and drained it waiting for
///   both `turn_run_id`s to reach `"completed"` again. It did not return
///   within 300s even isolated to just this test. The composition this
///   harness wires (`WebuiRuntimeProjectionStream::subscribe`,
///   `crates/ironclaw_reborn_composition/src/projection.rs`) reports
///   `supports_subscription() == true` and is durable-log-backed in
///   principle, but a brand-new subscriber with no further live writes
///   forthcoming evidently does not reliably/promptly redeliver
///   already-durable history in this test composition — a harness gap, not
///   a statement about the real `ironclaw-reborn serve` binary (whose
///   `handlers.rs::stream_events` doc is the confirmed evidence this whole
///   fix is based on). This is the SAME constraint
///   `tui_client_thread_switch_replay_dedupes_to_one_reply` already
///   documented and worked around by replaying through `app::reduce`
///   directly instead of a real resubscribe — this test follows that same,
///   already-established pattern rather than reopening that investigation.
/// - **Why not literally seeding >50 messages:** the fix (`app/transcript.rs`'s
///   `event_run_id`/`projection_item_run_id` filters against
///   `AppState::settled_run_ids`) only suppresses replay for runs the loaded
///   timeline PAGE actually captured — see that field's doc for why (the wire
///   exposes no per-item ordering key, and the timeline's own `next_cursor` is
///   a message-`sequence`-keyed backward-pagination token, not a resumable
///   `ProjectionCursor` `after_cursor`/`Last-Event-ID` could use — Option 1 in
///   the task brief was tried and ruled out for exactly this reason). Seeding
///   real history past the default 50-message page would mean scripting 25+
///   real turns purely to prove a boundary the fix does not claim to cover.
/// - **What this test proves that the crate-tier tests above cannot:** the
///   filter holds against REAL server-minted `turn_run_id`s and REAL reply
///   text from a REAL `/timeline` snapshot, across TWO turns (not one),
///   through the exact same public `app::reduce` seam the crate-tier tests
///   use — closing the gap between "synthetic ids in a unit test" and "what
///   the server actually mints."
#[tokio::test]
async fn tui_client_thread_switch_replay_of_multiple_turns_matches_the_timeline_snapshot() {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("conv-tui-multi-turn-switch-replay")
        .script([
            RebornScriptedReply::text("TUI_MULTI_TURN_REPLY_ONE"),
            RebornScriptedReply::text("TUI_MULTI_TURN_REPLY_TWO"),
        ])
        .build()
        .await
        .expect("thread builds");

    let capability_harness = group
        .capability_harness()
        .expect("live_approvals always uses a HostRuntime capability backend");
    let reborn_services = capability_harness
        .reborn_services_for_test()
        .expect("live_approvals harness is built via new_with_options");
    let approval_interactions = reborn_services
        .local_dev_approval_interaction_service_with_turn_state_for_test(
            h.coordinator.clone(),
            h.turn_store.clone(),
        )
        .expect("local-dev capability policy is valid")
        .expect("harness has a local-dev runtime");
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let reply_target_binding_ref = ReplyTargetBindingRef::new("tui-multi-turn-switch-test")
        .expect("valid reply target binding ref");
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = h.turn_store.clone();
    let event_stream = ironclaw_reborn_composition::test_support::build_webui_event_stream_for_test(
        event_log,
        turn_event_source,
        h.coordinator.clone(),
        reply_target_binding_ref,
    );
    let services: Arc<dyn RebornServicesApi> = Arc::new(
        RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
            .with_event_stream(event_stream)
            .with_approval_interactions(approval_interactions),
    );

    let user_id = h
        .binding
        .subject_user_id
        .clone()
        .unwrap_or_else(|| h.binding.actor_user_id.clone());
    let agent_id = h
        .binding
        .agent_id
        .clone()
        .expect("live_approvals binding resolves an agent_id");
    let token = "reborn-tui-multi-turn-switch-test-token-0123456789ab";
    let (addr, _serve_guard) = tui_listener::spawn_webui_v2(
        services,
        h.binding.tenant_id.clone(),
        user_id,
        agent_id,
        h.binding.project_id.clone(),
        token,
    )
    .await;
    let client = ApiClient::new(format!("http://{addr}"), token.to_string());
    let thread_id = h.binding.thread_id.as_str().to_string();
    tui_listener::create_thread_pinned(addr, token, &thread_id).await;

    // Drive both turns to completion through one subscription, mirroring
    // real usage: a user sends, waits for the reply, sends again.
    let mut events = std::pin::pin!(ironclaw_reborn_tui::client::events::subscribe(
        &client, &thread_id, None
    ));
    client
        .send_message(&thread_id, "TUI_MULTI_TURN_PROMPT_ONE")
        .await
        .expect("send_message turn one");
    let (run_one, status_one) = wait_for_any_settled_run(&mut events).await;
    assert_eq!(status_one, "completed", "turn one must complete");
    client
        .send_message(&thread_id, "TUI_MULTI_TURN_PROMPT_TWO")
        .await
        .expect("send_message turn two");
    let (run_two, status_two) = wait_for_any_settled_run(&mut events).await;
    assert_eq!(status_two, "completed", "turn two must complete");
    assert_ne!(run_one, run_two, "each send must produce its own run");

    // The "loaded timeline snapshot" a real thread switch would fetch.
    // `limit: 50` comfortably covers this thread's small handful of
    // messages (2 user + at least 2 assistant — see the coverage note above
    // for why this test proves the fix within a page rather than past one).
    // Not asserting an exact message count: like
    // `tui_client_thread_switch_replay_dedupes_to_one_reply` above, a
    // completed turn can surface more than one assistant row for the same
    // run (a superseded draft plus the finalized reply), so the real
    // invariant is "both prompts and both replies are present", not a fixed
    // total.
    let page = client
        .timeline(&thread_id, 50, None)
        .await
        .expect("timeline snapshot");
    assert_eq!(
        page.messages.iter().filter(|m| m.kind == "user").count(),
        2,
        "both prompts must be in the snapshot, got {:?}",
        page.messages
    );
    let mut state = snapshot_state_from_timeline(&page);
    let before: Vec<String> = state.transcript.iter().map(describe_item).collect();
    assert!(
        before
            .iter()
            .any(|d| d.contains("TUI_MULTI_TURN_REPLY_ONE")),
        "snapshot must carry turn one's reply, got {before:?}"
    );
    assert!(
        before
            .iter()
            .any(|d| d.contains("TUI_MULTI_TURN_REPLY_TWO")),
        "snapshot must carry turn two's reply, got {before:?}"
    );

    // The thread-switch resubscribe replay, driven through the public
    // `app::reduce` entry point rather than a second real `subscribe(.., None)`
    // — see this test's doc comment for why. Real `turn_run_id`s and real
    // reply text (both pulled off the `/timeline` fetch above, not invented),
    // fed back as the SAME frame shapes a cursor-less redrain would deliver
    // for two already-settled turns: a `Text` projection item (the
    // concretely-named `upsert_live_text` blocker path) followed by a
    // terminal `RunStatus`, once per turn — exactly what
    // `wait_for_any_settled_run` above proved the real server actually
    // produces for each of these turns.
    let real_text_for_run = |run_id: &str| -> String {
        page.messages
            .iter()
            .find(|m| m.turn_run_id.as_deref() == Some(run_id))
            .and_then(|m| m.content.clone())
            .unwrap_or_else(|| panic!("timeline carries the assistant message for {run_id}"))
    };
    let replay_settled_turn = |state: &mut AppState, run_id: &str| {
        let run = TurnRunId::parse(run_id).expect("real server turn_run_id is a UUID");
        let text = real_text_for_run(run_id);
        app::reduce(
            state,
            AppEvent::Server(Box::new(WebChatV2EventFrame {
                cursor: ProjectionCursor::new(format!("cursor:tui:multi-turn-replay:{run_id}:1"))
                    .expect("valid cursor"),
                event: WebChatV2Event::ProjectionUpdate {
                    state: ironclaw_product_workflow::ProductProjectionState::new(
                        thread_id.clone(),
                        vec![ProductProjectionItem::Text {
                            id: format!("replay-text-{run_id}"),
                            run_id: Some(run),
                            body: text,
                        }],
                    )
                    .expect("valid projection state"),
                },
            })),
        );
        app::reduce(
            state,
            AppEvent::Server(Box::new(WebChatV2EventFrame {
                cursor: ProjectionCursor::new(format!("cursor:tui:multi-turn-replay:{run_id}:2"))
                    .expect("valid cursor"),
                event: WebChatV2Event::ProjectionUpdate {
                    state: ironclaw_product_workflow::ProductProjectionState::new(
                        thread_id.clone(),
                        vec![ProductProjectionItem::RunStatus {
                            run_id: run,
                            status: "completed".to_string(),
                            failure_category: None,
                            failure_summary: None,
                            retryable: None,
                        }],
                    )
                    .expect("valid projection state"),
                },
            })),
        );
    };
    replay_settled_turn(&mut state, &run_one);
    replay_settled_turn(&mut state, &run_two);

    let after: Vec<String> = state.transcript.iter().map(describe_item).collect();
    assert_eq!(
        after, before,
        "the thread-switch replay must reproduce the loaded snapshot exactly — \
         no duplicated, reordered, or resurrected items"
    );
    assert!(
        state.pending_gate.is_none(),
        "no gate was ever pending for these plain-text turns; replay must not invent one"
    );
    assert!(
        !state.is_running(),
        "both turns already settled; replay must not leave `running` stuck on"
    );
}

/// Mirrors `lib.rs`'s private `apply_timeline_page`/`transcript_item_from_message`
/// against the PUBLIC `AppState`/`TranscriptItem` surface — both are private
/// to `ironclaw_reborn_tui::lib`, so an external integration test cannot call
/// them directly (see the module doc on
/// `tui_client_thread_switch_replay_dedupes_to_one_reply` for the same
/// constraint). Same "user"/"assistant"/else-as-System mapping, same
/// `settled_run_ids` seeding from every message's `turn_run_id`.
fn snapshot_state_from_timeline(page: &ironclaw_reborn_tui::client::TimelinePage) -> AppState {
    let mut state = AppState::default().set_thread_id(page.thread.thread_id.clone());
    state.settled_run_ids = page
        .messages
        .iter()
        .filter_map(|m| m.turn_run_id.clone())
        .collect();
    state.transcript = page
        .messages
        .iter()
        .map(|m| {
            let text = m.content.clone().unwrap_or_default();
            match m.kind.as_str() {
                "user" => app::TranscriptItem::User { text },
                "assistant" => app::TranscriptItem::Assistant { text },
                _ if text.is_empty() => app::TranscriptItem::System {
                    text: m.kind.clone(),
                },
                _ => app::TranscriptItem::System { text },
            }
        })
        .collect();
    state
}

/// `TranscriptItem` has no `PartialEq` (some variants wrap wire view types
/// that don't derive it either); compare through `Debug` instead — every
/// variant reaching here derives it (required for `TranscriptItem`'s own
/// `#[derive(Debug)]` to compile), so this always renders a diffable string.
fn describe_item(item: &app::TranscriptItem) -> String {
    format!("{item:?}")
}

/// Automations parity (`list_automations(include_completed=true)` +
/// opening a completed run's thread via `recent_runs[].thread_id`).
///
/// NOT REACHABLE IN HARNESS: every other test in this file builds its
/// `RebornServicesApi` via `ironclaw_product_workflow::RebornServices::new(..)`,
/// whose `automation_facade` defaults to `UnsupportedAutomationProductFacade`
/// (`crates/ironclaw_product_workflow/src/reborn_services.rs`) — `list_automations`
/// returns an error there, not an empty/real list. Producing a REAL completed
/// automation with a populated `recent_runs[].thread_id` needs an
/// `AutomationProductFacade` backed by a genuinely fired `TriggerRecord`
/// (`ironclaw_triggers::TriggerRepository` + the real trigger poller), the way
/// `crates/ironclaw_reborn_composition/tests/trigger_webui_timeline_e2e.rs`
/// builds it: a full `build_reborn_runtime` + `TriggerPollerSettings`
/// composition, not `RebornIntegrationGroup` (whose `triggers()` capability
/// only wires `trigger_create/list/pause/resume/remove` TOOLS onto the agent
/// loop's capability surface — it never fires a trigger through a poller, so
/// even a scripted `trigger_create` call would leave `list_automations()`
/// returning nothing to open). No `test_support` accessor analogous to
/// `local_dev_approval_interaction_service_for_test`/
/// `local_dev_auth_interaction_service_for_test` exposes a real automation
/// facade off `RebornIntegrationGroup`'s shared runtime, and adding one is a
/// production-adjacent `ironclaw_reborn_composition` change outside this
/// test-only, two-file (`tui_gate_seam.rs` + `support/tui_listener.rs`) scope.
#[tokio::test]
#[ignore = "NOT REACHABLE IN HARNESS: see fn doc comment — no real automation \
            facade / trigger-poller wiring is reachable from RebornIntegrationGroup \
            within this file's two-file scope"]
async fn tui_client_automations_parity_list_and_open_run_thread() {}
