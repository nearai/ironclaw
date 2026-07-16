//! Transcript accumulation from server events. Every `WebChatV2Event`
//! variant is handled here (routed from `reduce`'s `AppEvent::Server` arm),
//! including `Gate`/`AuthRequired` which delegate to [`super::gate`] rather
//! than touching the transcript.

use ironclaw_product_workflow::webchat_schema::{WebChatV2Event, WebChatV2EventFrame};
use ironclaw_product_workflow::{CapabilityActivityView, CapabilityDisplayPreviewView};

use super::{AppState, Effect, TranscriptItem, gate, wire_label};

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
        // Out of MVP transcript scope (noted, not fabricated) — the
        // projection state is rendered by a later increment, not this
        // reducer's transcript.
        WebChatV2Event::ProjectionSnapshot { .. } | WebChatV2Event::ProjectionUpdate { .. } => {
            Vec::new()
        }
        WebChatV2Event::KeepAlive => Vec::new(),
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

    use super::super::test_support::{
        activity_view, final_reply_view, frame, run_state_with_failure,
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
}
