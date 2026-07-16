//! Transcript accumulation from server events. Every `WebChatV2Event`
//! variant is handled here (routed from `reduce`'s `AppEvent::Server` arm),
//! including `Gate`/`AuthRequired` which delegate to [`super::gate`] rather
//! than touching the transcript.

use ironclaw_product_workflow::webchat_schema::{WebChatV2Event, WebChatV2EventFrame};
use ironclaw_product_workflow::{
    CapabilityActivityView, CapabilityDisplayPreviewView, ProductProjectionItem,
    ProductProjectionState,
};

use super::{ApiCall, AppState, Effect, TranscriptItem, gate, wire_label};

/// Page size for the timeline refetch fired on every terminal `RunStatus`
/// projection item (see `apply_run_status`). A third independent copy of
/// the same "first page" default `lib.rs`'s `INITIAL_TIMELINE_LIMIT` and
/// `threads_modal.rs`'s `DEFAULT_TIMELINE_LIMIT` already keep — see those
/// modules' own doc comments on why each is its own copy, not a shared
/// constant.
const SETTLED_RUN_TIMELINE_REFETCH_LIMIT: u32 = 50;

/// `ProductProjectionItem::RunStatus.status` values that mean the run is
/// over (mirrors the frontend's `TERMINAL_RUN_STATUSES` in
/// `useChatEvents.ts`).
const TERMINAL_RUN_STATUSES: [&str; 5] = [
    "completed",
    "succeeded",
    "failed",
    "cancelled",
    "recovery_required",
];

/// `ProductProjectionItem::RunStatus.status` values that mean the run is
/// blocked on the user, not actively working (mirrors the frontend's
/// `PROMPT_RUN_STATUSES`). Distinct from "terminal": a prompt status still
/// has an active run, waiting on the gate/auth resolution the `Gate`
/// projection item (or the raw `gate`/`auth_required` frame) supplies.
const PROMPT_RUN_STATUSES: [&str; 4] = [
    "blocked_auth",
    "blocked_approval",
    "blocked_resource",
    "blocked_dependent_run",
];

/// BLOCKER fix (SSE full-history replay on thread switch): drops any
/// run-scoped event whose `turn_run_id` is already in
/// `state.settled_run_ids` — i.e. a run the currently-loaded timeline
/// snapshot already fully represents — before it ever reaches the match
/// below. A cursor-less SSE resubscribe (see `lib.rs`'s `run_event_loop`)
/// replays the thread's ENTIRE event history from origin on every connect
/// and thread switch; without this guard, `Running`/`CapabilityProgress`
/// would toggle `state.running` back on for a long-settled run,
/// `Gate`/`AuthRequired` would resurrect (and briefly flash) an
/// already-resolved gate, and `Cancelled`/`Failed` would re-append a stale
/// status line. `FinalReply` is deliberately excluded here — its own arm
/// below already performs the equivalent check-and-insert (and must still
/// run so a genuinely new run's first arrival gets recorded); dropping it
/// here too would just be redundant. `ProjectionSnapshot`/`ProjectionUpdate`
/// are also excluded: one frame can carry items from several runs at once,
/// so they're filtered per-item by `projection_item_run_id` inside
/// `apply_projection_item` instead. See `AppState::settled_run_ids`'s doc
/// for this filter's coverage boundary (it only protects runs the loaded
/// timeline PAGE captured, not history older than that page).
fn event_run_id(event: &WebChatV2Event) -> Option<String> {
    match event {
        WebChatV2Event::Running { progress } | WebChatV2Event::CapabilityProgress { progress } => {
            Some(progress.turn_run_id.to_string())
        }
        WebChatV2Event::CapabilityActivity { activity } => {
            activity.turn_run_id.map(|id| id.to_string())
        }
        WebChatV2Event::CapabilityDisplayPreview { preview } => {
            preview.turn_run_id.map(|id| id.to_string())
        }
        WebChatV2Event::Gate { prompt } => Some(prompt.turn_run_id.to_string()),
        WebChatV2Event::AuthRequired { prompt } => Some(prompt.turn_run_id.to_string()),
        WebChatV2Event::Cancelled { response } => Some(response.run_id.to_string()),
        WebChatV2Event::Failed { run_state } => Some(run_state.run_id.to_string()),
        WebChatV2Event::Accepted { .. }
        | WebChatV2Event::FinalReply { .. }
        | WebChatV2Event::ProjectionSnapshot { .. }
        | WebChatV2Event::ProjectionUpdate { .. }
        | WebChatV2Event::KeepAlive => None,
    }
}

pub(crate) fn apply_server_event(state: &mut AppState, frame: WebChatV2EventFrame) -> Vec<Effect> {
    if event_run_id(&frame.event).is_some_and(|run_id| state.settled_run_ids.contains(&run_id)) {
        return Vec::new();
    }
    match frame.event {
        // The ack for a submitted message arrives on this event; nothing
        // durable to show yet (the assistant's reply is what actually lands
        // in the transcript, via `FinalReply` below).
        WebChatV2Event::Accepted { .. } => Vec::new(),
        WebChatV2Event::Running { .. } | WebChatV2Event::CapabilityProgress { .. } => {
            state.running = true;
            Vec::new()
        }
        WebChatV2Event::CapabilityActivity { activity } => {
            upsert_activity(state, activity);
            Vec::new()
        }
        WebChatV2Event::CapabilityDisplayPreview { preview } => {
            upsert_preview(state, preview);
            Vec::new()
        }
        WebChatV2Event::Gate { prompt } => gate::apply_gate_prompt(state, prompt),
        WebChatV2Event::AuthRequired { prompt } => gate::apply_auth_prompt(state, prompt),
        // `HashSet::insert` returns `true` only the first time a given
        // `turn_run_id` is seen: a genuinely new reply appends (and is
        // recorded), while a replayed one — the SSE resubscribe on a thread
        // switch has no cursor, so it re-delivers every past event,
        // including this one, on top of the timeline snapshot `lib.rs`'s
        // `apply_timeline_page` already loaded it from — is a no-op here.
        // `state.running` still clears either way: a stale replay finding
        // the run no longer active is the correct outcome, not a bug. Not
        // gated by `apply_server_event`'s `event_run_id` filter above (see
        // that fn's doc) — this IS the check for `FinalReply` specifically.
        WebChatV2Event::FinalReply { reply } => {
            if state.settled_run_ids.insert(reply.turn_run_id.to_string()) {
                state
                    .transcript
                    .push(TranscriptItem::final_text(reply.text));
            }
            state.running = false;
            Vec::new()
        }
        WebChatV2Event::Cancelled { response } => {
            let run_id = response.run_id.to_string();
            if state
                .pending_gate
                .as_ref()
                .map(|g| g.turn_run_id() == run_id)
                .unwrap_or(false)
            {
                state.pending_gate = None;
            }
            if state.active_run_id.as_deref() == Some(run_id.as_str()) {
                state.running = false;
                state.active_run_id = None;
            }
            state.transcript.push(TranscriptItem::System {
                text: format!("run {}", wire_label(&response.status)),
            });
            Vec::new()
        }
        WebChatV2Event::Failed { run_state } => {
            let status_label = wire_label(&run_state.status);
            let text = match run_state.failure {
                Some(failure) => match failure.detail() {
                    Some(detail) => format!("{}: {detail}", failure.category()),
                    None => failure.category().to_string(),
                },
                None => status_label,
            };
            state.transcript.push(TranscriptItem::Error { text });
            Vec::new()
        }
        // The real `local-dev` producer's primary wire: both snapshot and
        // update carry the same `ProductProjectionState` shape and are
        // applied identically (mirrors the frontend's `useChatEvents.ts`,
        // which routes both event types through the same
        // `applyProjectionItems`).
        WebChatV2Event::ProjectionSnapshot { state: projection }
        | WebChatV2Event::ProjectionUpdate { state: projection } => {
            apply_projection(state, projection)
        }
        WebChatV2Event::KeepAlive => Vec::new(),
    }
}

/// Applies every item in one `projection_snapshot`/`projection_update`
/// frame, in order.
fn apply_projection(state: &mut AppState, projection: ProductProjectionState) -> Vec<Effect> {
    let mut effects = Vec::new();
    for item in projection.items {
        effects.extend(apply_projection_item(state, item));
    }
    effects
}

/// Per-item counterpart of `event_run_id`, for items inside a
/// `ProjectionSnapshot`/`ProjectionUpdate` frame — one frame's `items` list
/// can span several runs at once, so filtering has to happen per item
/// rather than for the whole frame. `Text`/`Thinking`'s `run_id` is
/// `Option` (not every projection item is definitively attributed); `None`
/// is passed through unfiltered by the caller, same as an activity/preview
/// with no `turn_run_id` today.
fn projection_item_run_id(item: &ProductProjectionItem) -> Option<String> {
    match item {
        ProductProjectionItem::RunStatus { run_id, .. } => Some(run_id.to_string()),
        ProductProjectionItem::Text { run_id, .. }
        | ProductProjectionItem::Thinking { run_id, .. } => run_id.map(|id| id.to_string()),
        ProductProjectionItem::CapabilityActivity(activity) => {
            activity.turn_run_id.map(|id| id.to_string())
        }
        ProductProjectionItem::WorkSummary { run_id, .. } => Some(run_id.to_string()),
        ProductProjectionItem::Gate { run_id, .. } => Some(run_id.to_string()),
        ProductProjectionItem::SkillActivation { run_id, .. } => Some(run_id.to_string()),
    }
}

fn apply_projection_item(state: &mut AppState, item: ProductProjectionItem) -> Vec<Effect> {
    if projection_item_run_id(&item).is_some_and(|run_id| state.settled_run_ids.contains(&run_id)) {
        // Same BLOCKER fix as `apply_server_event`'s `event_run_id` guard,
        // for items carried inside a projection frame: a replayed `Text`/
        // `Thinking`/`WorkSummary`/`SkillActivation`/`Gate`/`RunStatus` for
        // an already-settled run must not upsert/append/resurrect, and a
        // replayed terminal `RunStatus` for it must not re-fire another
        // `ApiCall::LoadTimeline`.
        return Vec::new();
    }
    match item {
        ProductProjectionItem::RunStatus { run_id, status, .. } => {
            apply_run_status(state, run_id.to_string(), status)
        }
        ProductProjectionItem::Text { id, body, .. } => {
            upsert_live_text(state, id, body);
            Vec::new()
        }
        ProductProjectionItem::Thinking { id, body, .. } => {
            upsert_thinking(state, id, body);
            Vec::new()
        }
        ProductProjectionItem::Gate {
            run_id,
            gate_kind,
            gate_ref,
            headline,
            body,
            allow_always,
            auth_context,
            ..
        } => {
            // `ProductGateKind`/`AuthPromptContextView` are not re-exported
            // by `ironclaw_product_workflow` (see `gate::apply_projection_gate`'s
            // doc) — reduce them to primitives here, where the compiler
            // still has the concrete wire types in scope from this match's
            // destructure, before crossing into `gate`.
            let is_auth = wire_label(&gate_kind) == "auth";
            let (challenge_kind, provider, account_label, authorization_url) = match auth_context {
                Some(ctx) => (
                    Some(wire_label(&ctx.challenge_kind)),
                    ctx.provider,
                    ctx.account_label,
                    ctx.authorization_url,
                ),
                None => (None, None, None, None),
            };
            gate::apply_projection_gate(
                state,
                gate::ProjectionGateFields {
                    turn_run_id: run_id.to_string(),
                    gate_ref,
                    headline,
                    body: body.unwrap_or_default(),
                    allow_always,
                    is_auth,
                    challenge_kind,
                    provider,
                    account_label,
                    authorization_url,
                },
            )
        }
        // Rendered minimally as an appended `System` line (mirroring the
        // frontend's own scope — neither item type gets bespoke
        // chat-bubble/upsert rendering there either; see the module doc).
        // Unlike `Text`/`Thinking`, the contract does not require an
        // in-place upsert for these two: a `WorkSummary` phase transition
        // reads fine as a sequence of status lines, not a single line that
        // rewrites itself.
        ProductProjectionItem::WorkSummary { phase, body, .. } => {
            state.transcript.push(TranscriptItem::System {
                text: format!("[{}] {body}", wire_label(&phase)),
            });
            Vec::new()
        }
        ProductProjectionItem::SkillActivation { skill_names, .. } => {
            state.transcript.push(TranscriptItem::System {
                text: format!("skills: {}", skill_names.join(", ")),
            });
            Vec::new()
        }
        // Not populated by the current producer for this projection item —
        // the raw `capability_activity` frame (handled by `upsert_activity`
        // above) is the real tool-card path. See the module doc.
        ProductProjectionItem::CapabilityActivity(_) => Vec::new(),
    }
}

/// Drives `state.running` and, on a terminal status, settles the run: clears
/// a matching pending gate and fires the SAME `ApiCall::LoadTimeline` effect
/// startup rehydration and `threads_modal`'s thread-switch already use, so
/// the durable assistant reply (and any tool-input/output previews — never
/// carried by projection state, only by the timeline; see the module doc)
/// lands once the live stream can no longer show them. Mirrors the
/// frontend's `onRunSettled` timeline reload in `useHistory.ts`, fired from
/// `applyProjectionItems`'s terminal-status branch in `useChatEvents.ts`.
fn apply_run_status(state: &mut AppState, run_id: String, status: String) -> Vec<Effect> {
    if TERMINAL_RUN_STATUSES.contains(&status.as_str()) {
        state.running = false;
        state.active_run_id = None;
        if state
            .pending_gate
            .as_ref()
            .is_some_and(|g| g.turn_run_id() == run_id)
        {
            state.pending_gate = None;
        }
        return match state.thread_id.clone() {
            Some(thread_id) => vec![Effect::Api(ApiCall::LoadTimeline {
                thread_id,
                limit: SETTLED_RUN_TIMELINE_REFETCH_LIMIT,
                cursor: None,
            })],
            None => Vec::new(),
        };
    }
    state.running = !PROMPT_RUN_STATUSES.contains(&status.as_str());
    // Tracked whenever the run isn't terminal (including a blocked/prompt
    // status, which is still an active — just paused — run), so a gate
    // that resolves back into the same run keeps a target for `Esc` to
    // cancel. `Esc` itself only fires while `state.running` is also true
    // (see `dispatch_composer_key`), so a blocked run doesn't offer a
    // composer-focus cancel (the gate zone owns `Esc` there instead).
    state.active_run_id = Some(run_id);
    Vec::new()
}

fn upsert_live_text(state: &mut AppState, id: String, body: String) {
    let existing = state.transcript.iter_mut().find(|item| {
        matches!(item, TranscriptItem::LiveText { id: existing_id, .. } if *existing_id == id)
    });
    match existing {
        Some(TranscriptItem::LiveText { body: slot, .. }) => *slot = body,
        _ => state.transcript.push(TranscriptItem::LiveText { id, body }),
    }
}

fn upsert_thinking(state: &mut AppState, id: String, body: String) {
    let existing = state.transcript.iter_mut().find(|item| {
        matches!(item, TranscriptItem::Thinking { id: existing_id, .. } if *existing_id == id)
    });
    match existing {
        Some(TranscriptItem::Thinking { body: slot, .. }) => *slot = body,
        _ => state.transcript.push(TranscriptItem::Thinking { id, body }),
    }
}

fn upsert_activity(state: &mut AppState, activity: CapabilityActivityView) {
    let existing = state.transcript.iter_mut().find(|item| {
        matches!(item, TranscriptItem::Activity(existing) if existing.invocation_id == activity.invocation_id)
    });
    match existing {
        Some(TranscriptItem::Activity(slot)) => *slot = activity,
        _ => state.transcript.push(TranscriptItem::Activity(activity)),
    }
}

fn upsert_preview(state: &mut AppState, preview: CapabilityDisplayPreviewView) {
    let existing = state.transcript.iter_mut().find(|item| {
        matches!(item, TranscriptItem::Preview(existing) if existing.invocation_id == preview.invocation_id)
    });
    match existing {
        Some(TranscriptItem::Preview(slot)) => *slot = preview,
        _ => state.transcript.push(TranscriptItem::Preview(preview)),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::InvocationId;
    use ironclaw_product_adapters::{
        AuthPromptChallengeKind, AuthPromptContextView, ProductGateKind,
    };
    use ironclaw_product_workflow::webchat_schema::WebChatV2Event;
    use ironclaw_product_workflow::{
        CapabilityActivityStatusView, ProductProjectionItem, ProductWorkSummaryPhase,
    };
    use ironclaw_turns::{EventCursor, TurnRunId, TurnStatus};

    use super::super::test_support::{
        activity_view, final_reply_view, final_reply_view_for_run, frame, key, projection_gate,
        projection_run_status, projection_state, projection_text, run_state_with_failure,
    };
    use super::super::{AppEvent, reduce};
    use super::*;

    /// Pins Defect E's fix at the reducer level: a `FinalReply` carrying a
    /// `turn_run_id` already in `state.settled_run_ids` (as it would be
    /// after a cursor-less SSE resubscribe replays a reply the timeline
    /// snapshot already loaded — see `lib.rs`'s `apply_timeline_page`) must
    /// upsert-skip rather than append a second transcript row, while a
    /// reply for a genuinely unseen run still appends normally.
    #[test]
    fn final_reply_dedupes_by_turn_run_id_but_a_new_run_id_still_appends() {
        let mut state = AppState::default();
        let run_id = TurnRunId::new();

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::FinalReply {
                reply: final_reply_view_for_run(run_id, "hello"),
            }),
        );
        // Simulates the SSE replay: same run_id, same text, arriving again.
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::FinalReply {
                reply: final_reply_view_for_run(run_id, "hello"),
            }),
        );

        assert_eq!(
            state
                .transcript
                .iter()
                .filter(|i| i.as_final_text() == Some("hello"))
                .count(),
            1,
            "a replayed FinalReply for an already-known turn_run_id must not duplicate"
        );

        let other_run = TurnRunId::new();
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::FinalReply {
                reply: final_reply_view_for_run(other_run, "goodbye"),
            }),
        );
        assert!(
            state
                .transcript
                .iter()
                .any(|i| i.as_final_text() == Some("goodbye")),
            "a reply for a genuinely new run_id must still append"
        );
    }

    #[test]
    fn capability_activity_upserts_by_invocation_id() {
        let mut state = AppState::default();
        let inv = InvocationId::new();
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::CapabilityActivity {
                activity: activity_view(inv, CapabilityActivityStatusView::Started, None),
            }),
        );
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::CapabilityActivity {
                activity: activity_view(
                    inv,
                    CapabilityActivityStatusView::Completed,
                    Some("42 rows"),
                ),
            }),
        );
        assert_eq!(
            state.transcript.len(),
            1,
            "same invocation_id must upsert, not append"
        );
        assert_eq!(
            state.transcript[0].as_activity().unwrap().status,
            CapabilityActivityStatusView::Completed
        );
    }

    #[test]
    fn final_reply_appends_assistant_item_and_clears_running_indicator() {
        let mut state = AppState::default().set_running(true);
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::FinalReply {
                reply: final_reply_view("done"),
            }),
        );
        assert!(!state.is_running());
        assert!(
            state
                .transcript
                .iter()
                .any(|i| i.as_final_text() == Some("done"))
        );
    }

    #[test]
    fn failed_event_surfaces_sanitized_category_only() {
        let mut state = AppState::default();
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::Failed {
                run_state: run_state_with_failure("provider_unavailable", Some("HTTP 503")),
            }),
        );
        assert!(
            state
                .transcript
                .iter()
                .any(|i| i.as_error_text() == Some("provider_unavailable: HTTP 503"))
        );
    }

    #[test]
    fn failed_event_without_detail_surfaces_category_only() {
        let mut state = AppState::default();
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::Failed {
                run_state: run_state_with_failure("model_unavailable", None),
            }),
        );
        assert!(
            state
                .transcript
                .iter()
                .any(|i| i.as_error_text() == Some("model_unavailable"))
        );
    }

    #[test]
    fn running_and_capability_progress_set_running_without_transcript_rows() {
        let mut state = AppState::default();
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::Running {
                progress: ironclaw_product_workflow::ProgressUpdateView {
                    turn_run_id: ironclaw_turns::TurnRunId::new(),
                    kind: ironclaw_product_workflow::ProgressKind::Typing,
                    generated_at: chrono::Utc::now(),
                },
            }),
        );
        assert!(state.is_running());
        assert!(state.transcript.is_empty());
    }

    #[test]
    fn projection_text_then_terminal_run_status_renders_reply_and_refetches_timeline() {
        let mut state = AppState::default().set_thread_id("t-1");
        let run_id = TurnRunId::new();

        let effects = apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state(
                    "t-1",
                    vec![
                        projection_text("item-1", run_id, "hello from the live stream"),
                        projection_run_status(run_id, "completed"),
                    ],
                ),
            }),
        );

        assert!(
            state
                .transcript
                .iter()
                .any(|i| i.as_live_text() == Some("hello from the live stream")),
            "projection Text item must render into the transcript"
        );
        assert!(!state.is_running(), "terminal RunStatus clears running");
        assert!(matches!(
            effects.as_slice(),
            [Effect::Api(ApiCall::LoadTimeline { thread_id, .. })] if thread_id == "t-1"
        ));
    }

    #[test]
    fn projection_text_upserts_by_id_instead_of_appending_duplicates() {
        let mut state = AppState::default();
        let run_id = TurnRunId::new();

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state("t-1", vec![projection_text("item-1", run_id, "partial")]),
            }),
        );
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state(
                    "t-1",
                    vec![projection_text(
                        "item-1",
                        run_id,
                        "partial reply, now complete",
                    )],
                ),
            }),
        );

        let live_texts: Vec<_> = state
            .transcript
            .iter()
            .filter_map(|i| i.as_live_text())
            .collect();
        assert_eq!(
            live_texts,
            vec!["partial reply, now complete"],
            "same item id must upsert in place, not append a second row"
        );
    }

    #[test]
    fn projection_gate_item_and_raw_gate_frame_for_the_same_run_dedupe_to_one_pending_gate() {
        let mut state = AppState::default();
        let run_id = TurnRunId::new();

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state("t-1", vec![projection_gate(run_id, "gr-1")]),
            }),
        );
        assert!(state.pending_gate.is_some());

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::Gate {
                prompt: ironclaw_product_workflow::GatePromptView {
                    turn_run_id: run_id,
                    gate_ref: "gr-1".to_string(),
                    invocation_id: None,
                    headline: "Approve action (raw frame)".to_string(),
                    body: "Should not replace the already-pending gate.".to_string(),
                    allow_always: false,
                    approval_context: None,
                },
            }),
        );

        assert_eq!(
            state.pending_gate.as_ref().map(gate::PendingGate::gate_ref),
            Some("gr-1"),
            "still exactly one pending gate, keyed on (run_id, gate_ref)"
        );
        assert!(
            matches!(
                &state.pending_gate,
                Some(gate::PendingGate::Approval { headline, .. }) if headline == "Approve action"
            ),
            "first arrival (the projection item) wins; the later raw frame is a no-op"
        );
    }

    #[test]
    fn projected_auth_context_survives_dedup_into_manual_token_effect() {
        let mut state = AppState::default().set_thread_id("t-1");
        let run_id = TurnRunId::new();
        let gate_ref = "auth-gate-1";
        let auth_gate = |auth_context| ProductProjectionItem::Gate {
            run_id,
            gate_kind: ProductGateKind::Auth,
            gate_ref: gate_ref.to_string(),
            invocation_id: None,
            headline: "Connect GitHub".to_string(),
            body: Some("Paste a personal access token.".to_string()),
            allow_always: false,
            auth_context,
        };

        let context = AuthPromptContextView::new(
            AuthPromptChallengeKind::ManualToken,
            Some("github".to_string()),
            Some("GitHub PAT".to_string()),
            None,
            None,
            None,
        )
        .expect("valid auth projection context");
        reduce(
            &mut state,
            AppEvent::Server(Box::new(frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state("t-1", vec![auth_gate(Some(context))]),
            }))),
        );

        // A replay/update for the same gate without context must not replace
        // the richer first arrival at the shared gate-dedup seam.
        reduce(
            &mut state,
            AppEvent::Server(Box::new(frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state("t-1", vec![auth_gate(None)]),
            }))),
        );

        reduce(
            &mut state,
            AppEvent::Key(key(crossterm::event::KeyCode::Char('t'))),
        );
        for character in "secret".chars() {
            reduce(
                &mut state,
                AppEvent::Key(key(crossterm::event::KeyCode::Char(character))),
            );
        }
        let effects = reduce(
            &mut state,
            AppEvent::Key(key(crossterm::event::KeyCode::Enter)),
        );

        assert!(matches!(
            effects.as_slice(),
            [Effect::Api(ApiCall::SubmitManualToken {
                provider,
                account_label,
                ..
            })] if provider == "github" && account_label == "GitHub PAT"
        ));
    }

    #[test]
    fn non_terminal_run_status_captures_the_active_run_id() {
        let mut state = AppState::default();
        let run_id = TurnRunId::new();
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state("t-1", vec![projection_run_status(run_id, "in_progress")]),
            }),
        );
        assert_eq!(
            state.active_run_id.as_deref(),
            Some(run_id.to_string().as_str())
        );
        assert!(state.is_running());
    }

    #[test]
    fn terminal_run_status_clears_the_active_run_id() {
        let mut state = AppState::default().set_thread_id("t-1");
        let run_id = TurnRunId::new();
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state("t-1", vec![projection_run_status(run_id, "in_progress")]),
            }),
        );
        assert!(state.active_run_id.is_some());

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state("t-1", vec![projection_run_status(run_id, "completed")]),
            }),
        );
        assert_eq!(
            state.active_run_id, None,
            "a terminal RunStatus must clear the cancel target"
        );
    }

    #[test]
    fn cancelled_event_for_the_active_run_clears_running_and_active_run_id() {
        let mut state = AppState::default();
        let run_id = TurnRunId::new();
        state.running = true;
        state.active_run_id = Some(run_id.to_string());

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::Cancelled {
                response: ironclaw_product_workflow::RebornCancelRunResponse {
                    run_id,
                    status: TurnStatus::Cancelled,
                    event_cursor: EventCursor(1),
                    already_terminal: false,
                },
            }),
        );

        assert!(!state.is_running());
        assert_eq!(state.active_run_id, None);
    }

    #[test]
    fn cancelled_event_for_a_different_run_does_not_clear_the_active_run_id() {
        let mut state = AppState::default();
        let active_run_id = TurnRunId::new();
        state.running = true;
        state.active_run_id = Some(active_run_id.to_string());

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::Cancelled {
                response: ironclaw_product_workflow::RebornCancelRunResponse {
                    run_id: TurnRunId::new(),
                    status: TurnStatus::Cancelled,
                    event_cursor: EventCursor(1),
                    already_terminal: false,
                },
            }),
        );

        assert!(
            state.is_running(),
            "a stale/unrelated run must not clear this state"
        );
        assert_eq!(state.active_run_id, Some(active_run_id.to_string()));
    }

    /// BLOCKER fix, generalized beyond `FinalReply`: seeds `settled_run_ids`
    /// with a run (as `lib.rs`'s `apply_timeline_page` would after loading a
    /// timeline page that already captured this run), then replays a FULL
    /// mix of run-scoped projection item types for that SAME run — exactly
    /// what a cursor-less SSE resubscribe would redeliver. None of them may
    /// upsert/append/resurrect: `Text` must not push a stale `LiveText` row,
    /// `WorkSummary`/`SkillActivation` must not push a `System` line, the
    /// projection `Gate` item must not resurrect `pending_gate`, and the
    /// terminal `RunStatus` must not re-fire an `ApiCall::LoadTimeline`
    /// (previously it always would, at extra cost, on every switch replay).
    /// A second, genuinely new run's own item still applies normally,
    /// proving the filter is scoped to the settled run, not projection
    /// items in general.
    #[test]
    fn replayed_projection_items_for_an_already_settled_run_are_dropped_but_a_new_run_still_applies()
     {
        let mut state = AppState::default().set_thread_id("t-1");
        let settled_run = TurnRunId::new();
        state.settled_run_ids.insert(settled_run.to_string());

        let effects = apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state(
                    "t-1",
                    vec![
                        projection_text("old-text-1", settled_run, "stale streamed text"),
                        ProductProjectionItem::WorkSummary {
                            id: "old-work-1".to_string(),
                            run_id: settled_run,
                            phase: ProductWorkSummaryPhase::Planning,
                            body: "stale work summary".to_string(),
                        },
                        ProductProjectionItem::SkillActivation {
                            id: "old-skill-1".to_string(),
                            run_id: settled_run,
                            skill_names: vec!["some-skill".to_string()],
                            feedback: Vec::new(),
                        },
                        projection_gate(settled_run, "stale-gate"),
                        projection_run_status(settled_run, "completed"),
                    ],
                ),
            }),
        );

        assert!(
            state.transcript.is_empty(),
            "no item for an already-settled run may enter the transcript, got {:?}",
            state.transcript
        );
        assert!(
            state.pending_gate.is_none(),
            "a replayed Gate item for an already-settled run must not resurrect pending_gate"
        );
        assert!(!state.is_running());
        assert!(
            effects.is_empty(),
            "a replayed terminal RunStatus for an already-settled run must not re-fire LoadTimeline"
        );

        // A genuinely new (not-yet-settled) run's own item still applies.
        let new_run = TurnRunId::new();
        apply_server_event(
            &mut state,
            frame(WebChatV2Event::ProjectionUpdate {
                state: projection_state(
                    "t-1",
                    vec![projection_text("new-text-1", new_run, "live text")],
                ),
            }),
        );
        assert_eq!(
            state
                .transcript
                .iter()
                .filter_map(|i| i.as_live_text())
                .collect::<Vec<_>>(),
            vec!["live text"],
            "a run not yet in settled_run_ids must still render live"
        );
    }

    /// Same fix, at the top-level frame filter (`event_run_id`, guarding
    /// `apply_server_event` before the match): a replayed raw `Gate` frame
    /// and a replayed `Running` frame for an already-settled run must both
    /// be dropped — the raw-frame counterpart of the projection-item test
    /// above. `Running` matters because, unlike the projection `RunStatus`
    /// item, nothing else would ever clear a stale `state.running = true`
    /// for a run that's already fully settled and will never emit another
    /// terminal status in this replay.
    #[test]
    fn replayed_raw_gate_and_running_frames_for_an_already_settled_run_are_dropped() {
        let mut state = AppState::default();
        let settled_run = TurnRunId::new();
        state.settled_run_ids.insert(settled_run.to_string());

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::Gate {
                prompt: ironclaw_product_workflow::GatePromptView {
                    turn_run_id: settled_run,
                    gate_ref: "stale-raw-gate".to_string(),
                    invocation_id: None,
                    headline: "Approve action (raw frame)".to_string(),
                    body: "stale".to_string(),
                    allow_always: false,
                    approval_context: None,
                },
            }),
        );
        assert!(
            state.pending_gate.is_none(),
            "a replayed raw Gate frame for an already-settled run must not resurrect pending_gate"
        );

        apply_server_event(
            &mut state,
            frame(WebChatV2Event::Running {
                progress: ironclaw_product_workflow::ProgressUpdateView {
                    turn_run_id: settled_run,
                    kind: ironclaw_product_workflow::ProgressKind::Typing,
                    generated_at: chrono::Utc::now(),
                },
            }),
        );
        assert!(
            !state.is_running(),
            "a replayed Running frame for an already-settled run must not flip running back on"
        );
    }
}
