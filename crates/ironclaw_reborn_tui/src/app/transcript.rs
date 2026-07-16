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

pub(crate) fn apply_server_event(state: &mut AppState, frame: WebChatV2EventFrame) -> Vec<Effect> {
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
        WebChatV2Event::FinalReply { reply } => {
            state
                .transcript
                .push(TranscriptItem::final_text(reply.text));
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

fn apply_projection_item(state: &mut AppState, item: ProductProjectionItem) -> Vec<Effect> {
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
            let (challenge_kind, authorization_url) = match auth_context {
                Some(ctx) => (Some(wire_label(&ctx.challenge_kind)), ctx.authorization_url),
                None => (None, None),
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
    use ironclaw_product_workflow::CapabilityActivityStatusView;
    use ironclaw_product_workflow::webchat_schema::WebChatV2Event;
    use ironclaw_turns::{EventCursor, TurnRunId, TurnStatus};

    use super::super::test_support::{
        activity_view, final_reply_view, frame, projection_gate, projection_run_status,
        projection_state, projection_text, run_state_with_failure,
    };
    use super::*;

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
}
