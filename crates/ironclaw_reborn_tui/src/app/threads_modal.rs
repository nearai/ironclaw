//! Thread-switch modal: list threads, select, two-step delete.

use crossterm::event::{KeyCode, KeyEvent};

use super::{ApiCall, AppState, Effect, Modal, commit_thread_switch};
use crate::client::ThreadSummary;

/// Default page size for `ApiCall::LoadTimeline` when a thread is selected
/// from this modal (no cursor — the newest page).
const DEFAULT_TIMELINE_LIMIT: u32 = 50;

#[derive(Debug, Clone, Default)]
pub struct ThreadsModalState {
    pub threads: Vec<ThreadSummary>,
    /// Selection index over the *rendered* list, which is the pinned
    /// "+ new" affordance (`ui/modals.rs`) followed by `threads`: `0` is
    /// "+ new", and `1..=threads.len()` map to `threads[selected - 1]`.
    pub selected: usize,
    pub pending_delete_confirm: bool,
    pub loading: bool,
}

/// `Ctrl+X` from the composer: opens the modal and requests the thread list.
pub(crate) fn open(state: &mut AppState) -> Vec<Effect> {
    state.modal = Some(Modal::Threads(ThreadsModalState {
        loading: true,
        ..ThreadsModalState::default()
    }));
    vec![Effect::Api(ApiCall::ListThreads)]
}

pub(crate) fn dispatch_key(
    state: &mut AppState,
    key: KeyEvent,
    mut modal: ThreadsModalState,
) -> Vec<Effect> {
    match key.code {
        KeyCode::Esc => {
            state.modal = None;
            Vec::new()
        }
        KeyCode::Up => {
            modal.selected = modal.selected.saturating_sub(1);
            modal.pending_delete_confirm = false;
            state.modal = Some(Modal::Threads(modal));
            Vec::new()
        }
        KeyCode::Down => {
            // Row 0 is the pinned "+ new" row; rows 1..=threads.len() are
            // the thread list, so the last valid index is threads.len().
            if modal.selected < modal.threads.len() {
                modal.selected += 1;
            }
            modal.pending_delete_confirm = false;
            state.modal = Some(Modal::Threads(modal));
            Vec::new()
        }
        KeyCode::Enter => {
            if modal.selected == 0 {
                // The pinned "+ new" row: close the modal immediately
                // (optimistic, same as picking an existing thread) and let
                // `execute_effect` assign `state.thread_id` once the server
                // confirms creation — the reducer has no id to set yet.
                state.modal = None;
                return vec![Effect::Api(ApiCall::CreateThread)];
            }
            let Some(thread) = modal.threads.get(modal.selected - 1) else {
                state.modal = Some(Modal::Threads(modal));
                return Vec::new();
            };
            let thread_id = thread.thread_id.clone();
            commit_thread_switch(state, thread_id.clone());
            state.modal = None;
            vec![Effect::Api(ApiCall::LoadTimeline {
                thread_id,
                limit: DEFAULT_TIMELINE_LIMIT,
                cursor: None,
            })]
        }
        KeyCode::Char('d') => {
            if modal.selected == 0 {
                // The pinned "+ new" row isn't a real thread; nothing to
                // delete.
                state.modal = Some(Modal::Threads(modal));
                return Vec::new();
            }
            let thread_idx = modal.selected - 1;
            if modal.pending_delete_confirm {
                let Some(thread) = modal.threads.get(thread_idx) else {
                    state.modal = Some(Modal::Threads(modal));
                    return Vec::new();
                };
                let thread_id = thread.thread_id.clone();
                modal.pending_delete_confirm = false;
                state.modal = Some(Modal::Threads(modal));
                vec![Effect::Api(ApiCall::DeleteThread { thread_id })]
            } else {
                modal.pending_delete_confirm = true;
                state.modal = Some(Modal::Threads(modal));
                Vec::new()
            }
        }
        _ => {
            state.modal = Some(Modal::Threads(modal));
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{ctrl, key, threads_modal_with};
    use super::super::{
        ApiCall, AppEvent, AppState, Effect, Modal, PendingGate, TranscriptItem, reduce,
    };
    use super::*;

    #[test]
    fn ctrl_x_opens_threads_modal_and_requests_list() {
        let mut state = AppState::default();
        let effects = reduce(&mut state, AppEvent::Key(ctrl('x')));
        assert!(matches!(state.modal, Some(Modal::Threads(_))));
        assert!(matches!(effects[0], Effect::Api(ApiCall::ListThreads)));
    }

    #[test]
    fn enter_on_selected_thread_swaps_thread_and_loads_timeline() {
        // Row 0 is the pinned "+ new" affordance, so thread rows start at 1:
        // selected 2 -> threads[1] -> "t-2".
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1", "t-2"], 2)));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert_eq!(state.thread_id.as_deref(), Some("t-2"));
        assert!(state.modal.is_none(), "selecting a thread closes the modal");
        assert!(
            state.transcript.is_empty(),
            "swapping threads clears the old transcript before reload"
        );
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::LoadTimeline { thread_id, .. }) if thread_id == "t-2"
        ));
    }

    #[test]
    fn switching_threads_clears_the_previous_threads_live_state() {
        let mut state = AppState::default()
            .set_thread_id("old-thread")
            .set_modal(Some(threads_modal_with(["new-thread"], 1)))
            .set_pending_gate(Some(PendingGate::Approval {
                turn_run_id: "old-run".to_string(),
                gate_ref: "old-gate".to_string(),
                headline: "Approve".to_string(),
                body: String::new(),
                allow_always: false,
            }))
            .set_running(true)
            .set_active_run_id("old-run")
            .set_transcript_scroll(4);
        state.transcript.push(TranscriptItem::System {
            text: "old transcript".to_string(),
        });

        reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));

        assert_eq!(state.thread_id.as_deref(), Some("new-thread"));
        assert!(state.transcript.is_empty());
        assert_eq!(state.transcript_scroll, None);
        assert!(state.pending_gate.is_none());
        assert!(!state.running);
        assert_eq!(state.active_run_id, None);
    }

    #[test]
    fn enter_on_new_row_emits_create_thread() {
        // Default selection (0) is the pinned "+ new" row.
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1"], 0)));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(
            state.modal.is_none(),
            "creating a thread closes the modal immediately"
        );
        assert!(
            state.thread_id.is_none(),
            "the reducer does not fabricate a thread_id; execute_effect assigns it once the server confirms creation"
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::Api(ApiCall::CreateThread)));
    }

    #[test]
    fn up_down_navigate_across_pinned_new_row() {
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1", "t-2"], 1)));

        // Up from the first thread (selected 1) lands on the pinned row (0).
        reduce(&mut state, AppEvent::Key(key(KeyCode::Up)));
        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if m.selected == 0));

        // Up again stays pinned; it never goes negative.
        reduce(&mut state, AppEvent::Key(key(KeyCode::Up)));
        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if m.selected == 0));

        // Down walks back onto t-1 (1), then t-2 (2), then stops (no overflow
        // past the last thread row).
        reduce(&mut state, AppEvent::Key(key(KeyCode::Down)));
        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if m.selected == 1));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Down)));
        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if m.selected == 2));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Down)));
        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if m.selected == 2));
    }

    #[test]
    fn dd_on_selected_thread_requires_confirm_then_deletes() {
        // Selected 1 -> threads[0] -> "t-1" (row 0 is the pinned "+ new" row).
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1"], 1)));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('d'))));
        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if m.pending_delete_confirm));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('d'))));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::DeleteThread { thread_id }) if thread_id == "t-1"
        ));
    }

    #[test]
    fn d_on_pinned_new_row_is_a_no_op() {
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1"], 0)));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('d'))));
        assert!(effects.is_empty());
        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if !m.pending_delete_confirm));
    }

    #[test]
    fn esc_closes_threads_modal_without_effects() {
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1"], 0)));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(effects.is_empty());
        assert!(state.modal.is_none());
    }
}
