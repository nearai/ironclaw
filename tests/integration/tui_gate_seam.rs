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

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
#[allow(dead_code)]
#[path = "support/tui_listener.rs"]
mod tui_listener;

use std::sync::Arc;

use futures::StreamExt;
use ironclaw_events::InMemoryDurableEventLog;
use ironclaw_product_workflow::{
    ProductProjectionItem, RebornServices, RebornServicesApi, WebUiGateResolution,
};
use ironclaw_reborn_tui::client::ApiClient;
use ironclaw_turns::{ReplyTargetBindingRef, TurnEventProjectionSource, TurnStatus};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

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
    let gate = loop {
        let frame = events
            .next()
            .await
            .expect("stream stays open until the gate arrives")
            .expect("frame decodes");
        if let ironclaw_product_workflow::webchat_schema::WebChatV2Event::Gate { prompt } =
            frame.event
        {
            break prompt;
        }
    };

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
    const SETTLED_RUN_STATUSES: &[&str] = &[
        "completed",
        "succeeded",
        "failed",
        "cancelled",
        "recovery_required",
        "blocked_approval",
    ];
    let settled_status = loop {
        let frame = events
            .next()
            .await
            .expect("stream stays open until the run settles")
            .expect("frame decodes");
        let projection = match frame.event {
            ironclaw_product_workflow::webchat_schema::WebChatV2Event::ProjectionUpdate {
                state,
            }
            | ironclaw_product_workflow::webchat_schema::WebChatV2Event::ProjectionSnapshot {
                state,
            } => state,
            _ => continue,
        };
        let run_id = gate.turn_run_id.to_string();
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
            break status;
        }
    };
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
