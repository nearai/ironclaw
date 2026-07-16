//! Reborn TUI — a thin ratatui client of `ironclaw-reborn serve`'s WebChat v2
//! HTTP + SSE API (`/api/webchat/v2/*`). See
//! `docs/plans/2026-07-15-reborn-tui-service-install-design.md` for the
//! architecture. This crate must never depend on `ironclaw_webui_v2` (the
//! route/handler crate); wire types come from
//! `ironclaw_product_workflow::webchat_schema` — see
//! `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`.
//!
//! `lib.rs` is deliberately thin: rendering lives in `ui/`, application
//! state and business logic in `app/`, HTTP/SSE plumbing in `client/`, and
//! process lifecycle in `spawn/`. This file wires those together into the
//! terminal event loop ([`run_tui`]) and maps each reducer-emitted
//! [`app::ApiCall`] onto the matching [`client::ApiClient`] method
//! ([`execute_effect`]).

#![forbid(unsafe_code)]

pub mod app;
pub mod client;
pub mod spawn;
pub mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Context;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ironclaw_product_workflow::webchat_schema::WebChatV2EventFrame;
use ironclaw_product_workflow::{LlmConfigSnapshot, LlmModelsResult, LlmProbeResult};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::{
    ApiCall, AppEvent, AppState, ConnState, Effect, Modal, ProviderModalState, TranscriptItem,
};
use client::{ApiClient, ClientError, ThreadMessageSummary};

pub use spawn::ProcessInvocation;

/// Startup configuration for [`run_tui`].
#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub base_url: String,
    pub token: String,
    pub spawn: Option<ProcessInvocation>,
}

/// Page size for the timeline fetched on startup and after a thread switch.
/// Mirrors `app::threads_modal`'s own `DEFAULT_TIMELINE_LIMIT` (that
/// constant is private to its module, so this is a separate copy, not a
/// shared one — both independently pick the "first page" default).
const INITIAL_TIMELINE_LIMIT: u32 = 50;

/// Entry point the CLI's `tui` subcommand calls: ensures `serve` is
/// reachable (spawning it if configured to), takes over the terminal, runs
/// the event loop, and always restores the terminal before returning —
/// including when the event loop itself returns an error.
pub async fn run_tui(cfg: TuiConfig) -> anyhow::Result<()> {
    let client = ApiClient::new(cfg.base_url, cfg.token);
    let _serve_handle = spawn::ensure_serve(&client, cfg.spawn.as_ref())
        .await
        .context("could not reach or start the Reborn WebUI service")?;

    let mut terminal = setup_terminal()?;
    install_panic_hook();
    let result = run_event_loop(&mut terminal, &client).await;
    teardown_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stdout()))?)
}

/// Installs a panic hook that restores the terminal (best-effort — errors
/// from the restore calls are swallowed, since a panic hook itself must not
/// panic) before running the previous hook, so a panic mid-event-loop never
/// leaves the user's terminal stuck in raw mode / the alternate screen.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original(info);
    }));
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

/// The terminal event loop: draws one frame, waits for the next of
/// {keypress, SSE frame, 250ms tick}, feeds it through [`app::reduce`], and
/// executes whatever [`Effect`]s came back. Runs until `state.quitting`.
///
/// Startup sequence (per the design's "Initial flow"): list threads, pick
/// the first one (or create one if the account has none yet), rehydrate its
/// timeline, then subscribe to its SSE stream. A later thread switch (via
/// the threads modal's `Enter`, or a successful `ApiCall::CreateThread`)
/// re-subscribes automatically — see the `subscribed_thread_id` check below.
async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    client: &ApiClient,
) -> anyhow::Result<()> {
    let mut state = AppState::default();

    let threads = client
        .list_threads()
        .await
        .context("list_threads during TUI startup")?;
    let initial_thread = match threads.into_iter().next() {
        Some(thread) => thread,
        None => client
            .create_thread()
            .await
            .context("create_thread during TUI startup (account has no threads yet)")?,
    };
    state.thread_id = Some(initial_thread.thread_id.clone());
    if let Ok(page) = client
        .timeline(&initial_thread.thread_id, INITIAL_TIMELINE_LIMIT, None)
        .await
    {
        apply_timeline_page(&mut state, page);
    }

    let mut subscribed_thread_id = initial_thread.thread_id;
    let mut sse = Box::pin(client::events::subscribe(
        client,
        &subscribed_thread_id,
        None,
    ));
    let mut ticks = tokio::time::interval(Duration::from_millis(250));
    let (key_tx, mut key_rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || blocking_crossterm_reader(key_tx));

    // Set by `map_sse_item` when a stream item error reflects the
    // connection as `Reconnecting`; cleared (and paired with a `Connected`
    // event) the next time a frame is actually yielded. See that function's
    // doc for why this can't just be read back off `state.conn` (a modal
    // key press could have raced it via `dispatch_key`'s `Lost` gate — never
    // this path, but there's no reason to couple the two).
    let mut awaiting_reconnect = false;

    loop {
        terminal.draw(|f| ui::render(f, &state))?;

        let events = tokio::select! {
            Some(key) = key_rx.recv() => vec![AppEvent::Key(key)],
            Some(frame) = sse.next() => map_sse_item(frame, &mut awaiting_reconnect),
            _ = ticks.tick() => vec![AppEvent::Tick],
        };

        for event in events {
            for effect in app::reduce(&mut state, event) {
                execute_effect(client, &mut state, effect).await;
            }
        }

        if let Some(thread_id) = state.thread_id.clone()
            && thread_id != subscribed_thread_id
        {
            subscribed_thread_id = thread_id;
            sse = Box::pin(client::events::subscribe(
                client,
                &subscribed_thread_id,
                None,
            ));
        }

        if state.quitting {
            break;
        }
    }
    Ok(())
}

/// Maps one item off the SSE stream onto the `AppEvent`(s) the reducer
/// should see, tracking (via `awaiting_reconnect`, threaded in/out rather
/// than owned so a crate-tier test can drive this in isolation — no real
/// terminal or SSE connection needed) whether the previous item left the
/// connection reflected as `Reconnecting`.
///
/// `client/events.rs` owns the actual reconnect/backoff budget; a stream
/// item error here just means "this attempt ended" — reflect it as
/// `Reconnecting` and let the next call (on the next select iteration) pull
/// the next (possibly another `Err`, possibly budget-exhausted) item from
/// the same stream. The bug this fixes: nothing previously cleared
/// `Reconnecting` back to `Connected` once the stream recovered, so a single
/// hiccup left the status bar stuck for the rest of the session even though
/// frames kept arriving — see `app/connection.rs::apply_conn_change`, which
/// already handles the `Connected` transition, it just never had a
/// production caller. A `Connected` event is emitted ahead of the frame's
/// own `Server` event (not merged into it) so it goes through the reducer
/// first and reaches `apply_conn_change` as a real, independent
/// `AppEvent::Conn` transition.
fn map_sse_item(
    item: Result<WebChatV2EventFrame, ClientError>,
    awaiting_reconnect: &mut bool,
) -> Vec<AppEvent> {
    match item {
        Ok(frame) => {
            let mut events = Vec::with_capacity(2);
            if std::mem::take(awaiting_reconnect) {
                events.push(AppEvent::Conn(ConnState::Connected));
            }
            events.push(AppEvent::Server(Box::new(frame)));
            events
        }
        Err(_) => {
            *awaiting_reconnect = true;
            vec![AppEvent::Conn(ConnState::Reconnecting { attempt: 1 })]
        }
    }
}

/// Forwards `crossterm::event::read()` (blocking-only, so it needs its own
/// OS thread) key events to the async event loop. Any non-key event is
/// dropped; a closed channel or a read error ends the thread.
fn blocking_crossterm_reader(tx: tokio::sync::mpsc::UnboundedSender<crossterm::event::KeyEvent>) {
    loop {
        match crossterm::event::read() {
            Ok(crossterm::event::Event::Key(key)) => {
                if tx.send(key).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

/// Executes one reducer-emitted [`Effect`], mutating `state` with the
/// result. `pub(crate)` (not private) so this module's `#[cfg(test)]` block
/// can drive it directly against a local stub server — see the tests below.
pub(crate) async fn execute_effect(client: &ApiClient, state: &mut AppState, effect: Effect) {
    match effect {
        Effect::Quit => state.quitting = true,
        Effect::Api(call) => execute_api_call(client, state, call).await,
    }
}

/// Exhaustive match over every [`ApiCall`] variant onto its
/// [`ApiClient`] method. Every arm degrades an `Err` to
/// `state.last_local_error` rather than propagating — the event loop must
/// never crash on a failed HTTP call (`.claude/CLAUDE.md`: no
/// `.unwrap()`/`.expect()` in production code).
async fn execute_api_call(client: &ApiClient, state: &mut AppState, call: ApiCall) {
    match call {
        ApiCall::ListThreads => match client.list_threads().await {
            Ok(threads) => apply_threads_list(state, threads),
            Err(err) => set_local_error(state, "list threads", err),
        },
        // No current caller emits this (the threads modal's "+ new" row is
        // a render-only affordance — see `ui/modals.rs` — that
        // `app::threads_modal`'s reducer does not yet wire to
        // `ApiCall::CreateThread`; a pre-existing landed-`app/` gap, out of
        // this file's scope). Implemented anyway for match exhaustiveness
        // and because `run_event_loop`'s own startup path can reach it.
        ApiCall::CreateThread => match client.create_thread().await {
            Ok(thread) => {
                state.thread_id = Some(thread.thread_id);
                state.transcript.clear();
                state.pending_gate = None;
                state.modal = None;
            }
            Err(err) => set_local_error(state, "create thread", err),
        },
        ApiCall::DeleteThread { thread_id } => match client.delete_thread(&thread_id).await {
            Ok(()) => {
                if state.thread_id.as_deref() == Some(thread_id.as_str()) {
                    state.thread_id = None;
                    state.transcript.clear();
                }
                refresh_threads_modal(client, state).await;
            }
            Err(err) => set_local_error(state, "delete thread", err),
        },
        ApiCall::LoadTimeline {
            thread_id,
            limit,
            cursor,
        } => match client.timeline(&thread_id, limit, cursor).await {
            Ok(page) => apply_timeline_page(state, page),
            Err(err) => set_local_error(state, "load timeline", err),
        },
        ApiCall::SendMessage { thread_id, text } => {
            // The ack for this message arrives via the SSE stream
            // (`WebChatV2Event::Accepted`), not this response — see
            // `client/gates.rs`'s doc comment on `send_message`. Nothing
            // else to update here on success.
            if let Err(err) = client.send_message(&thread_id, &text).await {
                set_local_error(state, "send message", err);
            }
        }
        ApiCall::ResolveGate {
            thread_id,
            run_id,
            gate_ref,
            resolution,
        } => {
            // `state.pending_gate` deliberately stays untouched here even
            // on success: `app::gate`'s reducer tests pin that the gate
            // only clears when the server confirms via a later SSE event,
            // never optimistically from the resolve call's own response.
            if let Err(err) = client
                .resolve_gate(&thread_id, &run_id, &gate_ref, resolution)
                .await
            {
                set_local_error(state, "resolve gate", err);
            }
        }
        // Step 1 (submit) then, on success, step 2 (resolve) — both awaited
        // in this one arm, the same "call, then chain a follow-up call on
        // success" shape `PauseAutomation`/`RenameAutomation` already use
        // below via `refresh_automations_modal`. No separate event/effect
        // round-trip needed: `execute_api_call` is already the seam where
        // an API result feeds the next step.
        ApiCall::SubmitManualToken {
            thread_id,
            run_id,
            gate_ref,
            provider,
            account_label,
            token,
        } => match client
            .submit_manual_token(
                &provider,
                &account_label,
                &token,
                &thread_id,
                &run_id,
                &gate_ref,
            )
            .await
        {
            Ok(credential_ref) => {
                if let Err(err) = client
                    .resolve_gate(
                        &thread_id,
                        &run_id,
                        &gate_ref,
                        ironclaw_product_workflow::WebUiGateResolution::CredentialProvided {
                            credential_ref,
                        },
                    )
                    .await
                {
                    set_local_error(state, "resolve gate after manual token submit", err);
                }
            }
            Err(err) => set_local_error(state, "submit manual token", err),
        },
        ApiCall::ListAutomations => match client.list_automations().await {
            Ok(automations) => apply_automations_list(state, automations),
            Err(err) => set_local_error(state, "list automations", err),
        },
        ApiCall::PauseAutomation { id } => match client.pause_automation(&id).await {
            Ok(_) => refresh_automations_modal(client, state).await,
            Err(err) => set_local_error(state, "pause automation", err),
        },
        ApiCall::ResumeAutomation { id } => match client.resume_automation(&id).await {
            Ok(_) => refresh_automations_modal(client, state).await,
            Err(err) => set_local_error(state, "resume automation", err),
        },
        ApiCall::RenameAutomation { id, name } => {
            match client.rename_automation(&id, &name).await {
                Ok(_) => refresh_automations_modal(client, state).await,
                Err(err) => set_local_error(state, "rename automation", err),
            }
        }
        ApiCall::LlmProviders => match client.llm_providers().await {
            Ok(snapshot) => apply_llm_providers(state, snapshot),
            Err(err) => set_local_error(state, "list llm providers", err),
        },
        ApiCall::LlmListModels {
            provider_id,
            adapter,
            base_url,
        } => match client
            .llm_list_models(&provider_id, &adapter, base_url.as_deref())
            .await
        {
            Ok(result) => apply_llm_models(state, &provider_id, result),
            Err(err) => set_local_error(state, "list llm models", err),
        },
        ApiCall::LlmSetActive { provider_id, model } => {
            // `ProviderModalState::Confirmed` is already set optimistically
            // by `app::provider_modal`'s reducer (with `test_result: None`)
            // before this effect runs; nothing further to apply on success,
            // the `LlmTestConnection` effect queued alongside this one
            // (same key press, see `provider_modal::dispatch_enter`) is
            // what populates `test_result`.
            if let Err(err) = client.llm_set_active(&provider_id, &model).await {
                set_local_error(state, "set active llm provider", err);
            }
        }
        ApiCall::LlmTestConnection {
            provider_id,
            adapter,
            base_url,
        } => match client
            .llm_test_connection(&provider_id, &adapter, base_url.as_deref())
            .await
        {
            Ok(result) => apply_llm_test_result(state, result),
            Err(err) => set_local_error(state, "test llm connection", err),
        },
        ApiCall::CancelRun { thread_id, run_id } => {
            // `state.running`/`active_run_id` deliberately stay untouched
            // here even on success, same rationale as `ResolveGate` above:
            // they only clear once the server confirms via a later SSE
            // `Cancelled`/terminal `RunStatus` event, never optimistically
            // from this call's own response.
            if let Err(err) = client.cancel_run(&thread_id, &run_id).await {
                set_local_error(state, "cancel run", err);
            }
        }
    }
}

fn set_local_error(state: &mut AppState, action: &str, err: ClientError) {
    state.last_local_error = Some(format!("{action} failed: {err}"));
}

fn apply_threads_list(state: &mut AppState, threads: Vec<client::ThreadSummary>) {
    if let Some(Modal::Threads(modal)) = &mut state.modal {
        modal.selected = modal.selected.min(threads.len().saturating_sub(1));
        modal.threads = threads;
        modal.loading = false;
    }
}

async fn refresh_threads_modal(client: &ApiClient, state: &mut AppState) {
    if !matches!(state.modal, Some(Modal::Threads(_))) {
        return;
    }
    match client.list_threads().await {
        Ok(threads) => apply_threads_list(state, threads),
        Err(err) => set_local_error(state, "refresh threads", err),
    }
}

fn apply_automations_list(state: &mut AppState, automations: Vec<client::AutomationSummary>) {
    if let Some(Modal::Automations(modal)) = &mut state.modal {
        modal.selected = modal.selected.min(automations.len().saturating_sub(1));
        modal.automations = automations;
        modal.loading = false;
    }
}

async fn refresh_automations_modal(client: &ApiClient, state: &mut AppState) {
    if !matches!(state.modal, Some(Modal::Automations(_))) {
        return;
    }
    match client.list_automations().await {
        Ok(automations) => apply_automations_list(state, automations),
        Err(err) => set_local_error(state, "refresh automations", err),
    }
}

fn apply_llm_providers(state: &mut AppState, snapshot: LlmConfigSnapshot) {
    if let Some(Modal::Provider(ProviderModalState::Providers {
        providers,
        selected,
        loading,
    })) = &mut state.modal
    {
        *selected = (*selected).min(snapshot.providers.len().saturating_sub(1));
        *providers = snapshot.providers;
        *loading = false;
    }
}

fn apply_llm_models(state: &mut AppState, provider_id: &str, result: LlmModelsResult) {
    if let Some(Modal::Provider(ProviderModalState::Models {
        provider_id: current_provider_id,
        models,
        selected,
        loading,
        ..
    })) = &mut state.modal
    {
        // Guards against a stale response landing after the user has
        // already moved on to a different provider (Esc back to Providers,
        // then Enter on a different one before this call returned).
        if current_provider_id == provider_id {
            *selected = (*selected).min(result.models.len().saturating_sub(1));
            *models = result.models;
            *loading = false;
        }
    }
}

fn apply_llm_test_result(state: &mut AppState, result: LlmProbeResult) {
    if let Some(Modal::Provider(ProviderModalState::Confirmed { test_result, .. })) =
        &mut state.modal
    {
        *test_result = Some(result);
    }
}

/// Replaces `state.transcript` from a freshly-fetched timeline page, and
/// rebuilds `state.known_reply_ids` (the set of assistant-reply
/// `turn_run_id`s the transcript now represents) alongside it — the one
/// place that mutates both together, called from TUI startup and the
/// `ApiCall::LoadTimeline` arm below. Keeping the two in lockstep is what
/// lets `app/transcript.rs`'s `FinalReply` handling correctly recognize an
/// SSE-replayed reply as already-loaded rather than appending a duplicate —
/// see `AppState::known_reply_ids`'s doc.
fn apply_timeline_page(state: &mut AppState, page: client::TimelinePage) {
    state.known_reply_ids = page
        .messages
        .iter()
        .filter_map(|m| m.turn_run_id.clone())
        .collect();
    state.transcript = page
        .messages
        .iter()
        .map(transcript_item_from_message)
        .collect();
}

/// `ThreadMessageSummary::kind` is the raw wire string (see
/// `client/threads.rs`'s doc comment for the full enumeration); only
/// `user`/`assistant` map to their own `TranscriptItem` variant, everything
/// else (system/summary/checkpoint_reference/tool_result_reference/
/// capability_display_preview) renders as a `System` line — this crate's
/// `TranscriptItem::Activity`/`Preview` variants need the full typed
/// `CapabilityActivityView`/`CapabilityDisplayPreviewView`, which the flat
/// timeline read does not carry (only the live SSE stream does).
fn transcript_item_from_message(message: &ThreadMessageSummary) -> TranscriptItem {
    let text = message.content.clone().unwrap_or_default();
    match message.kind.as_str() {
        "user" => TranscriptItem::User { text },
        "assistant" => TranscriptItem::Assistant { text },
        _ if text.is_empty() => TranscriptItem::System {
            text: message.kind.clone(),
        },
        _ => TranscriptItem::System { text },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::{OriginalUri, State};
    use axum::http::{Method, StatusCode};
    use axum::response::{IntoResponse, Response};
    use tokio::net::TcpListener;

    use chrono::Utc;
    use ironclaw_product_workflow::FinalReplyView;
    use ironclaw_product_workflow::ProjectionCursor;
    use ironclaw_product_workflow::webchat_schema::WebChatV2Event;
    use ironclaw_turns::TurnRunId;

    use super::*;
    use app::{AutomationsModalState, PendingGate, ThreadsModalState};

    /// Builds the `AppEvent::Server` a real SSE-replayed `FinalReply` would
    /// produce, and feeds it straight through the reducer — the exact path
    /// `run_event_loop`'s `map_sse_item`/`Ok(frame)` arm drives in
    /// production, minus the network. Shared by the Defect E dedup tests
    /// below.
    fn replay_final_reply(state: &mut AppState, turn_run_id: TurnRunId, text: &str) {
        app::reduce(
            state,
            AppEvent::Server(Box::new(WebChatV2EventFrame {
                cursor: ProjectionCursor::new(format!("cursor:tui:test:{turn_run_id}"))
                    .expect("valid cursor"),
                event: WebChatV2Event::FinalReply {
                    reply: FinalReplyView {
                        turn_run_id,
                        text: text.to_string(),
                        generated_at: Utc::now(),
                    },
                },
            })),
        );
    }

    /// A minimal, content-agnostic frame: these tests exercise `map_sse_item`'s
    /// connection-bookkeeping (does a `Connected` event get prepended, does
    /// `awaiting_reconnect` clear), not the payload of any particular
    /// `WebChatV2Event` variant.
    fn keepalive_frame() -> WebChatV2EventFrame {
        WebChatV2EventFrame {
            cursor: ProjectionCursor::new("cursor:tui:test:1").expect("valid cursor"),
            event: WebChatV2Event::KeepAlive,
        }
    }

    #[test]
    fn map_sse_item_error_reflects_reconnecting_and_sets_the_awaiting_flag() {
        let mut awaiting_reconnect = false;
        let events = map_sse_item(
            Err(ClientError::StreamParse("boom".to_string())),
            &mut awaiting_reconnect,
        );
        assert!(awaiting_reconnect);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            AppEvent::Conn(ConnState::Reconnecting { attempt: 1 })
        ));
    }

    #[test]
    fn map_sse_item_ok_frame_after_an_error_prepends_connected_and_clears_the_flag() {
        let mut awaiting_reconnect = true;
        let events = map_sse_item(Ok(keepalive_frame()), &mut awaiting_reconnect);
        assert!(
            !awaiting_reconnect,
            "must clear once the stream has recovered"
        );
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AppEvent::Conn(ConnState::Connected)));
        assert!(matches!(events[1], AppEvent::Server(_)));
    }

    #[test]
    fn map_sse_item_ok_frame_without_a_prior_error_emits_only_the_server_event() {
        let mut awaiting_reconnect = false;
        let events = map_sse_item(Ok(keepalive_frame()), &mut awaiting_reconnect);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AppEvent::Server(_)));
    }

    /// Regression for the live-repro bug: one SSE hiccup previously left the
    /// status bar showing "reconnecting" for the rest of the session because
    /// nothing ever fed the reducer a `Connected` transition. Drives
    /// `map_sse_item`'s output through the real `app::reduce` (not just
    /// asserting on the `AppEvent`s themselves) so this also pins that
    /// `app/connection.rs::apply_conn_change` — reachable from a production
    /// caller as of this fix — actually resolves `state.conn` back to
    /// `Connected`.
    #[test]
    fn reconnecting_state_followed_by_a_successful_connect_signal_resolves_back_to_connected() {
        let mut state = AppState::default();
        let mut awaiting_reconnect = false;

        for event in map_sse_item(
            Err(ClientError::StreamParse("boom".to_string())),
            &mut awaiting_reconnect,
        ) {
            app::reduce(&mut state, event);
        }
        assert_eq!(state.conn, ConnState::Reconnecting { attempt: 1 });

        for event in map_sse_item(Ok(keepalive_frame()), &mut awaiting_reconnect) {
            app::reduce(&mut state, event);
        }
        assert_eq!(state.conn, ConnState::Connected);
    }

    /// Minimal local axum stub for `execute_effect` tests, following the
    /// pattern established in `spawn/mod.rs` (self-contained, axum/tower
    /// are already dev-dependencies) rather than the `tests/support`
    /// fixture: `execute_effect` is `pub(crate)`, unreachable from an
    /// external `tests/*.rs` integration-test binary. One scripted JSON
    /// body per `"METHOD /path"` key; a fallback records every hit (and,
    /// since the manual-token two-step chain needs to assert what the
    /// second call's *request* body carried, the body too) so a test can
    /// also assert which route was actually called and with what payload.
    #[derive(Default)]
    struct StubState {
        routes: Mutex<HashMap<String, (u16, serde_json::Value)>>,
        hits: Mutex<Vec<String>>,
        request_bodies: Mutex<HashMap<String, serde_json::Value>>,
    }

    struct StubServer {
        base_url: String,
        state: Arc<StubState>,
        _handle: tokio::task::JoinHandle<()>,
    }

    impl StubServer {
        async fn start() -> Self {
            let state = Arc::new(StubState::default());
            let router = Router::new().fallback(fallback).with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
            let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
            let handle = tokio::spawn(async move {
                axum::serve(listener, router).await.expect("run stub");
            });
            Self {
                base_url,
                state,
                _handle: handle,
            }
        }

        fn ok(&self, method_and_path: &str, body: serde_json::Value) {
            self.state
                .routes
                .lock()
                .expect("routes lock")
                .insert(method_and_path.to_string(), (200, body));
        }

        fn error(&self, method_and_path: &str, status: u16) {
            self.state.routes.lock().expect("routes lock").insert(
                method_and_path.to_string(),
                (status, serde_json::json!({"error": "stubbed"})),
            );
        }

        fn hits(&self) -> Vec<String> {
            self.state.hits.lock().expect("hits lock").clone()
        }

        fn request_body(&self, method_and_path: &str) -> Option<serde_json::Value> {
            self.state
                .request_bodies
                .lock()
                .expect("request bodies lock")
                .get(method_and_path)
                .cloned()
        }

        fn client(&self) -> ApiClient {
            ApiClient::new(self.base_url.clone(), "stub-token".to_string())
        }
    }

    async fn fallback(
        State(state): State<Arc<StubState>>,
        method: Method,
        OriginalUri(uri): OriginalUri,
        body: axum::body::Bytes,
    ) -> Response {
        let key = format!("{method} {}", uri.path());
        state.hits.lock().expect("hits lock").push(key.clone());
        if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) {
            state
                .request_bodies
                .lock()
                .expect("request bodies lock")
                .insert(key.clone(), parsed);
        }
        let routes = state.routes.lock().expect("routes lock");
        match routes.get(&key) {
            Some((status, body)) => (
                StatusCode::from_u16(*status).unwrap_or(StatusCode::OK),
                axum::Json(body.clone()),
            )
                .into_response(),
            None => (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": "unstubbed_route", "path": key})),
            )
                .into_response(),
        }
    }

    #[tokio::test]
    async fn list_threads_populates_open_threads_modal() {
        let stub = StubServer::start().await;
        stub.ok(
            "GET /api/webchat/v2/threads",
            serde_json::json!({"threads": [{"thread_id": "t-1"}, {"thread_id": "t-2"}]}),
        );
        let mut state =
            AppState::default().set_modal(Some(Modal::Threads(ThreadsModalState::default())));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::ListThreads),
        )
        .await;

        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if m.threads.len() == 2));
        assert_eq!(stub.hits(), vec!["GET /api/webchat/v2/threads"]);
    }

    #[tokio::test]
    async fn list_threads_error_degrades_to_local_error_not_panic() {
        let stub = StubServer::start().await;
        stub.error("GET /api/webchat/v2/threads", 500);
        let mut state = AppState::default();

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::ListThreads),
        )
        .await;

        assert!(state.last_local_error.is_some());
    }

    #[tokio::test]
    async fn create_thread_switches_to_the_new_thread_and_closes_modal() {
        let stub = StubServer::start().await;
        stub.ok(
            "POST /api/webchat/v2/threads",
            serde_json::json!({"thread": {"thread_id": "t-new"}}),
        );
        let mut state =
            AppState::default().set_modal(Some(Modal::Threads(ThreadsModalState::default())));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::CreateThread),
        )
        .await;

        assert_eq!(state.thread_id.as_deref(), Some("t-new"));
        assert!(state.modal.is_none());
    }

    #[tokio::test]
    async fn delete_thread_clears_current_thread_when_it_was_deleted() {
        let stub = StubServer::start().await;
        stub.ok("DELETE /api/webchat/v2/threads/t-1", serde_json::json!({}));
        stub.ok(
            "GET /api/webchat/v2/threads",
            serde_json::json!({"threads": []}),
        );
        let mut state = AppState::default().set_thread_id("t-1");
        state.transcript.push(TranscriptItem::final_text("old"));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::DeleteThread {
                thread_id: "t-1".to_string(),
            }),
        )
        .await;

        assert!(state.thread_id.is_none());
        assert!(state.transcript.is_empty());
    }

    #[tokio::test]
    async fn load_timeline_replaces_transcript_from_messages() {
        let stub = StubServer::start().await;
        stub.ok(
            "GET /api/webchat/v2/threads/t-1/timeline",
            serde_json::json!({
                "thread": {"thread_id": "t-1"},
                "messages": [
                    {"message_id": "m-1", "sequence": 1, "kind": "user", "status": "accepted", "content": "hi"},
                    {"message_id": "m-2", "sequence": 2, "kind": "assistant", "status": "finalized", "content": "hello"},
                ],
            }),
        );
        let mut state = AppState::default();
        state.transcript.push(TranscriptItem::System {
            text: "stale".to_string(),
        });

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::LoadTimeline {
                thread_id: "t-1".to_string(),
                limit: 50,
                cursor: None,
            }),
        )
        .await;

        assert_eq!(state.transcript.len(), 2);
        assert_eq!(state.transcript[0].as_final_text(), None);
        assert!(
            !state
                .transcript
                .iter()
                .any(|i| i.as_error_text() == Some("stale"))
        );
    }

    /// Defect E, driven through the real production seam: a `LoadTimeline`
    /// `ApiCall` (as `run_event_loop`'s startup and thread-switch paths both
    /// fire) followed by the SSE stream replaying the same turn's
    /// `FinalReply` — exactly what a cursor-less resubscribe does on every
    /// thread switch (`handlers.rs::stream_events` drains from origin on
    /// first connect) — must not duplicate the already-loaded message, while
    /// a genuinely new reply still appends.
    #[tokio::test]
    async fn sse_replay_of_an_already_loaded_reply_does_not_duplicate_the_transcript() {
        let stub = StubServer::start().await;
        let run_id = TurnRunId::new();
        stub.ok(
            "GET /api/webchat/v2/threads/t-1/timeline",
            serde_json::json!({
                "thread": {"thread_id": "t-1"},
                "messages": [
                    {"message_id": "m-1", "sequence": 1, "kind": "user", "status": "accepted", "content": "hi"},
                    {"message_id": "m-2", "sequence": 2, "kind": "assistant", "status": "finalized", "content": "hello", "turn_run_id": run_id.to_string()},
                ],
            }),
        );
        let mut state = AppState::default().set_thread_id("t-1");

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::LoadTimeline {
                thread_id: "t-1".to_string(),
                limit: 50,
                cursor: None,
            }),
        )
        .await;
        assert_eq!(
            state.transcript.len(),
            2,
            "timeline snapshot loads both rows"
        );

        replay_final_reply(&mut state, run_id, "hello");
        assert_eq!(
            state.transcript.len(),
            2,
            "replaying the already-loaded reply's turn_run_id must not duplicate it"
        );

        replay_final_reply(&mut state, TurnRunId::new(), "a brand new reply");
        assert_eq!(
            state.transcript.len(),
            3,
            "a reply for a genuinely new turn_run_id must still append"
        );
    }

    /// Two thread switches in a row: each `LoadTimeline` wholesale-replaces
    /// the transcript (and `known_reply_ids` alongside it, via
    /// `apply_timeline_page`), so thread A's SSE stream replaying its own
    /// history after the switch to B must not resurrect A's messages, and B's
    /// own replay must not duplicate B's snapshot either.
    #[tokio::test]
    async fn two_consecutive_thread_switches_do_not_duplicate_the_transcript() {
        let stub = StubServer::start().await;
        let run_a = TurnRunId::new();
        let run_b = TurnRunId::new();
        stub.ok(
            "GET /api/webchat/v2/threads/t-a/timeline",
            serde_json::json!({
                "thread": {"thread_id": "t-a"},
                "messages": [
                    {"message_id": "a-1", "sequence": 1, "kind": "user", "status": "accepted", "content": "hi a"},
                    {"message_id": "a-2", "sequence": 2, "kind": "assistant", "status": "finalized", "content": "hello a", "turn_run_id": run_a.to_string()},
                ],
            }),
        );
        stub.ok(
            "GET /api/webchat/v2/threads/t-b/timeline",
            serde_json::json!({
                "thread": {"thread_id": "t-b"},
                "messages": [
                    {"message_id": "b-1", "sequence": 1, "kind": "user", "status": "accepted", "content": "hi b"},
                    {"message_id": "b-2", "sequence": 2, "kind": "assistant", "status": "finalized", "content": "hello b", "turn_run_id": run_b.to_string()},
                ],
            }),
        );
        let mut state = AppState::default().set_thread_id("t-a");

        // Switch 1: load thread A, then A's own SSE stream replays A's history.
        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::LoadTimeline {
                thread_id: "t-a".to_string(),
                limit: 50,
                cursor: None,
            }),
        )
        .await;
        replay_final_reply(&mut state, run_a, "hello a");
        assert_eq!(
            state.transcript.len(),
            2,
            "thread A's own replay must not duplicate its snapshot"
        );

        // Switch 2: load thread B (replaces the transcript wholesale), then
        // B's own SSE stream replays B's history.
        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::LoadTimeline {
                thread_id: "t-b".to_string(),
                limit: 50,
                cursor: None,
            }),
        )
        .await;
        replay_final_reply(&mut state, run_b, "hello b");

        assert_eq!(
            state.transcript.len(),
            2,
            "the second switch's replay must not duplicate, nor leave A's rows behind"
        );
        assert!(
            state
                .transcript
                .iter()
                .any(|i| i.as_final_text() == Some("hello b"))
        );
        assert!(
            !state
                .transcript
                .iter()
                .any(|i| i.as_final_text() == Some("hello a")),
            "thread A's messages must not leak into thread B's transcript"
        );
    }

    #[tokio::test]
    async fn send_message_error_degrades_to_local_error_not_panic() {
        let stub = StubServer::start().await;
        stub.error("POST /api/webchat/v2/threads/t-1/messages", 503);
        let mut state = AppState::default().set_thread_id("t-1");

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::SendMessage {
                thread_id: "t-1".to_string(),
                text: "hi".to_string(),
            }),
        )
        .await;

        assert!(state.last_local_error.is_some());
    }

    #[tokio::test]
    async fn resolve_gate_leaves_pending_gate_untouched_on_success() {
        let stub = StubServer::start().await;
        stub.ok(
            "POST /api/webchat/v2/threads/t-1/runs/run-1/gates/gate-1/resolve",
            serde_json::json!({}),
        );
        let mut state = AppState::default()
            .set_thread_id("t-1")
            .set_pending_gate(Some(PendingGate::approval_stub("Allow write_file?")));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::ResolveGate {
                thread_id: "t-1".to_string(),
                run_id: "run-1".to_string(),
                gate_ref: "gate-1".to_string(),
                resolution: ironclaw_product_workflow::WebUiGateResolution::Approved {
                    always: false,
                },
            }),
        )
        .await;

        assert!(
            state.pending_gate.is_some(),
            "gate clears only on a later server-confirmed event, not the resolve response"
        );
    }

    /// The two-step manual-token flow, driven through the exact same
    /// `execute_effect` seam every other `ApiCall` uses: a single
    /// `SubmitManualToken` effect must hit the submit route first, then
    /// automatically chain a `resolve` call carrying the submit response's
    /// `credential_ref` as `CredentialProvided` — no separate user action
    /// or second effect required.
    #[tokio::test]
    async fn submit_manual_token_chains_into_resolve_gate_with_the_returned_credential_ref() {
        let stub = StubServer::start().await;
        stub.ok(
            "POST /api/reborn/product-auth/manual-token/submit",
            serde_json::json!({
                "credential_ref": "cred-abc-123",
                "status": "active",
                "continuation": "resumed",
            }),
        );
        stub.ok(
            "POST /api/webchat/v2/threads/t-1/runs/run-1/gates/gate-1/resolve",
            serde_json::json!({}),
        );
        let mut state = AppState::default().set_thread_id("t-1");

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::SubmitManualToken {
                thread_id: "t-1".to_string(),
                run_id: "run-1".to_string(),
                gate_ref: "gate-1".to_string(),
                provider: "google".to_string(),
                account_label: "work@example.com".to_string(),
                token: "raw-secret".to_string(),
            }),
        )
        .await;

        assert_eq!(
            stub.hits(),
            vec![
                "POST /api/reborn/product-auth/manual-token/submit".to_string(),
                "POST /api/webchat/v2/threads/t-1/runs/run-1/gates/gate-1/resolve".to_string(),
            ],
            "submit must complete before resolve fires, in that order"
        );
        let submit_body = stub
            .request_body("POST /api/reborn/product-auth/manual-token/submit")
            .expect("submit request body captured");
        assert_eq!(submit_body["token"], "raw-secret");
        assert_eq!(submit_body["provider"], "google");
        let resolve_body = stub
            .request_body("POST /api/webchat/v2/threads/t-1/runs/run-1/gates/gate-1/resolve")
            .expect("resolve request body captured");
        assert_eq!(resolve_body["resolution"], "credential_provided");
        assert_eq!(
            resolve_body["credential_ref"], "cred-abc-123",
            "must be the credential_ref the submit step returned, not the raw token"
        );
        assert!(state.last_local_error.is_none());
    }

    #[tokio::test]
    async fn submit_manual_token_failure_degrades_to_local_error_and_never_calls_resolve() {
        let stub = StubServer::start().await;
        stub.error("POST /api/reborn/product-auth/manual-token/submit", 502);
        let mut state = AppState::default().set_thread_id("t-1");

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::SubmitManualToken {
                thread_id: "t-1".to_string(),
                run_id: "run-1".to_string(),
                gate_ref: "gate-1".to_string(),
                provider: "google".to_string(),
                account_label: "work@example.com".to_string(),
                token: "raw-secret".to_string(),
            }),
        )
        .await;

        assert!(state.last_local_error.is_some());
        assert_eq!(
            stub.hits(),
            vec!["POST /api/reborn/product-auth/manual-token/submit".to_string()],
            "a failed submit must never reach the resolve step"
        );
    }

    #[tokio::test]
    async fn list_automations_populates_open_automations_modal() {
        let stub = StubServer::start().await;
        stub.ok(
            "GET /api/webchat/v2/automations",
            serde_json::json!({"automations": [{"automation_id": "a-1", "name": "Daily digest", "state": "active"}]}),
        );
        let mut state = AppState::default()
            .set_modal(Some(Modal::Automations(AutomationsModalState::default())));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::ListAutomations),
        )
        .await;

        assert!(matches!(&state.modal, Some(Modal::Automations(m)) if m.automations.len() == 1));
    }

    #[tokio::test]
    async fn pause_automation_refreshes_the_open_modal() {
        let stub = StubServer::start().await;
        stub.ok(
            "POST /api/webchat/v2/automations/a-1/pause",
            serde_json::json!({"automation": {"automation_id": "a-1", "name": "Daily digest", "state": "paused"}}),
        );
        stub.ok(
            "GET /api/webchat/v2/automations",
            serde_json::json!({"automations": [{"automation_id": "a-1", "name": "Daily digest", "state": "paused"}]}),
        );
        let mut state = AppState::default()
            .set_modal(Some(Modal::Automations(AutomationsModalState::default())));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::PauseAutomation {
                id: "a-1".to_string(),
            }),
        )
        .await;

        assert!(
            stub.hits()
                .contains(&"GET /api/webchat/v2/automations".to_string())
        );
        assert!(
            matches!(&state.modal, Some(Modal::Automations(m)) if m.automations[0].state == "paused")
        );
    }

    #[tokio::test]
    async fn resume_automation_error_degrades_to_local_error_not_panic() {
        let stub = StubServer::start().await;
        stub.error("POST /api/webchat/v2/automations/a-1/resume", 500);
        let mut state = AppState::default();

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::ResumeAutomation {
                id: "a-1".to_string(),
            }),
        )
        .await;

        assert!(state.last_local_error.is_some());
    }

    #[tokio::test]
    async fn rename_automation_refreshes_the_open_modal() {
        let stub = StubServer::start().await;
        stub.ok(
            "POST /api/webchat/v2/automations/a-1",
            serde_json::json!({"automation": {"automation_id": "a-1", "name": "renamed", "state": "active"}}),
        );
        stub.ok(
            "GET /api/webchat/v2/automations",
            serde_json::json!({"automations": [{"automation_id": "a-1", "name": "renamed", "state": "active"}]}),
        );
        let mut state = AppState::default()
            .set_modal(Some(Modal::Automations(AutomationsModalState::default())));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::RenameAutomation {
                id: "a-1".to_string(),
                name: "renamed".to_string(),
            }),
        )
        .await;

        assert!(
            matches!(&state.modal, Some(Modal::Automations(m)) if m.automations[0].name == "renamed")
        );
    }

    #[tokio::test]
    async fn llm_providers_populates_open_provider_modal() {
        let stub = StubServer::start().await;
        stub.ok(
            "GET /api/webchat/v2/llm/providers",
            serde_json::json!({"providers": [{
                "id": "openai", "description": "OpenAI", "adapter": "open_ai_completions",
                "default_model": "gpt-5", "base_url": null, "builtin": true, "active": false,
                "active_model": null, "api_key_required": true, "accepts_api_key": true,
                "api_key_set": false, "can_list_models": true
            }]}),
        );
        let mut state =
            AppState::default().set_modal(Some(Modal::Provider(ProviderModalState::default())));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::LlmProviders),
        )
        .await;

        assert!(matches!(
            &state.modal,
            Some(Modal::Provider(ProviderModalState::Providers { providers, .. })) if providers.len() == 1
        ));
    }

    #[tokio::test]
    async fn llm_list_models_populates_matching_models_state() {
        let stub = StubServer::start().await;
        stub.ok(
            "POST /api/webchat/v2/llm/list-models",
            serde_json::json!({"ok": true, "models": ["gpt-5", "gpt-5-mini"], "message": "ok"}),
        );
        let mut state =
            AppState::default().set_modal(Some(Modal::Provider(ProviderModalState::Models {
                provider_id: "openai".to_string(),
                adapter: "open_ai_completions".to_string(),
                base_url: None,
                models: Vec::new(),
                selected: 0,
                loading: true,
            })));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::LlmListModels {
                provider_id: "openai".to_string(),
                adapter: "open_ai_completions".to_string(),
                base_url: None,
            }),
        )
        .await;

        assert!(matches!(
            &state.modal,
            Some(Modal::Provider(ProviderModalState::Models { models, .. })) if models.len() == 2
        ));
    }

    #[tokio::test]
    async fn llm_set_active_error_degrades_to_local_error_not_panic() {
        let stub = StubServer::start().await;
        stub.error("POST /api/webchat/v2/llm/active", 500);
        let mut state = AppState::default();

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::LlmSetActive {
                provider_id: "openai".to_string(),
                model: "gpt-5".to_string(),
            }),
        )
        .await;

        assert!(state.last_local_error.is_some());
    }

    #[tokio::test]
    async fn llm_test_connection_populates_confirmed_test_result() {
        let stub = StubServer::start().await;
        stub.ok(
            "POST /api/webchat/v2/llm/test-connection",
            serde_json::json!({"ok": true, "message": "reachable"}),
        );
        let mut state =
            AppState::default().set_modal(Some(Modal::Provider(ProviderModalState::Confirmed {
                provider_id: "openai".to_string(),
                model: "gpt-5".to_string(),
                test_result: None,
            })));

        execute_effect(
            &stub.client(),
            &mut state,
            Effect::Api(ApiCall::LlmTestConnection {
                provider_id: "openai".to_string(),
                adapter: "open_ai_completions".to_string(),
                base_url: None,
            }),
        )
        .await;

        assert!(matches!(
            &state.modal,
            Some(Modal::Provider(ProviderModalState::Confirmed { test_result: Some(r), .. })) if r.ok
        ));
    }

    #[tokio::test]
    async fn quit_effect_sets_quitting_without_touching_the_network() {
        let stub = StubServer::start().await;
        let mut state = AppState::default();

        execute_effect(&stub.client(), &mut state, Effect::Quit).await;

        assert!(state.quitting);
        assert!(stub.hits().is_empty());
    }

    /// Environment-tolerant: sandboxed/headless test runners often have no
    /// real TTY attached, so `enable_raw_mode()` can legitimately return an
    /// `Err` there. The invariant under test is "never panics" — assert
    /// that for both outcomes, not that a real terminal is always present.
    #[test]
    fn terminal_setup_and_teardown_round_trips_without_panic() {
        if let Ok(mut terminal) = setup_terminal() {
            let _ = teardown_terminal(&mut terminal);
        }
    }
}
