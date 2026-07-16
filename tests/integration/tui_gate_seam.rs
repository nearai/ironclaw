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
//! CURRENTLY RED — verified upstream blocker, not a wiring bug (B2.13's own
//! scope is `tests/integration/tui_gate_seam.rs` +
//! `tests/integration/support/tui_listener.rs` + the root `Cargo.toml`
//! `[[test]]` entry only; fixing it means editing
//! `crates/ironclaw_reborn_tui/src/client/**`, out of this task's ownership).
//! See the comment at the `send_message` call site below for the exact
//! defect and fix.

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
use ironclaw_product_workflow::{RebornServices, RebornServicesApi, WebUiGateResolution};
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

    // submit — real HTTP POST through the TUI's own client, not the harness's
    // `submit_turn`/`submit_turn_until_blocked` shortcut.
    //
    // BLOCKER (verified, not a test-wiring issue): this call currently 400s
    // with `{"field":"client_action_id","validation_code":"missing_field"}`.
    // `ApiClient::send_message` (crates/ironclaw_reborn_tui/src/client/gates.rs)
    // builds `WebUiSendMessageRequest { content: Some(text), ..Default::default() }`
    // and never sets `client_action_id`. Every WebUI v2 mutation body requires
    // it as a mandatory idempotency key — enforced by
    // `parse_client_action_id`/`required_text` in
    // `crates/ironclaw_product_workflow/src/webui_inbound.rs` (~line 587),
    // which every mutation handler calls before dispatch. `resolve_gate`
    // (same file, `resolve_gate_body`) has the identical gap, so this would
    // fail a second time at the resolve step even if submit were patched
    // around. Fix (out of this task's ownership — `crates/ironclaw_reborn_tui/src/**`
    // is off-limits per this task's scope) is two one-line additions in
    // `client/gates.rs`: set `client_action_id: Some(Uuid::new_v4().to_string())`
    // (or similar) on both `WebUiSendMessageRequest` and the
    // `WebUiResolveGateRequest` built by `resolve_gate_body`.
    client
        .send_message(&thread_id, "write the tui gate seam file")
        .await
        .expect("send_message");

    // gate — drive the real SSE `subscribe()` to find the Gate frame.
    // `subscribe()` returns `impl Stream` built over `futures::stream::unfold`
    // of an async block, which is not `Unpin`; `StreamExt::next()` requires
    // `Self: Unpin`, so pin it on the stack with the stable `std::pin::pin!`
    // macro (any real consumer of this client API hits the same requirement).
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

    // completed seam — drain until FinalReply, not a bare status poll.
    let final_text = loop {
        let frame = events
            .next()
            .await
            .expect("stream stays open until the final reply arrives")
            .expect("frame decodes");
        if let ironclaw_product_workflow::webchat_schema::WebChatV2Event::FinalReply { reply } =
            frame.event
        {
            break reply.text;
        }
    };
    assert!(
        final_text.contains("file written after TUI-resolved approval"),
        "unexpected final reply text: {final_text:?}"
    );

    h.wait_for_status(gate.turn_run_id, TurnStatus::Completed)
        .await
        .expect("run completes");
    h.assert_workspace_file_contains("tui_gate_seam.txt", "TUI_GATE_SEAM")
        .await
        .expect("approved write actually re-dispatched and persisted");
}
