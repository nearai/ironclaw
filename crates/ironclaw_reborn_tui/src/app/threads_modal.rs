//! Thread-switch modal: list threads, select, two-step delete.

use crossterm::event::{KeyCode, KeyEvent};

use super::{ApiCall, AppState, Effect, Modal};
use crate::client::ThreadSummary;

/// Default page size for `ApiCall::LoadTimeline` when a thread is selected
/// from this modal (no cursor — the newest page).
const DEFAULT_TIMELINE_LIMIT: u32 = 50;

#[derive(Debug, Clone, Default)]
pub struct ThreadsModalState {
    pub threads: Vec<ThreadSummary>,
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
            if modal.selected + 1 < modal.threads.len() {
                modal.selected += 1;
            }
            modal.pending_delete_confirm = false;
            state.modal = Some(Modal::Threads(modal));
            Vec::new()
        }
        KeyCode::Enter => {
            let Some(thread) = modal.threads.get(modal.selected) else {
                state.modal = Some(Modal::Threads(modal));
                return Vec::new();
            };
            let thread_id = thread.thread_id.clone();
            state.thread_id = Some(thread_id.clone());
            state.transcript.clear();
            // A freshly selected thread cannot own the previous thread's
            // pending gate; `LoadTimeline` (below) is the recovery if the
            // new thread has its own blocked run.
            state.pending_gate = None;
            state.modal = None;
            vec![Effect::Api(ApiCall::LoadTimeline {
                thread_id,
                limit: DEFAULT_TIMELINE_LIMIT,
                cursor: None,
            })]
        }
        KeyCode::Char('d') => {
            if modal.pending_delete_confirm {
                let Some(thread) = modal.threads.get(modal.selected) else {
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
    use super::super::{ApiCall, AppEvent, AppState, Effect, Modal, reduce};
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
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1", "t-2"], 1)));
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
    fn dd_on_selected_thread_requires_confirm_then_deletes() {
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1"], 0)));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('d'))));
        assert!(matches!(&state.modal, Some(Modal::Threads(m)) if m.pending_delete_confirm));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('d'))));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::DeleteThread { thread_id }) if thread_id == "t-1"
        ));
    }

    #[test]
    fn esc_closes_threads_modal_without_effects() {
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1"], 0)));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(effects.is_empty());
        assert!(state.modal.is_none());
    }
}
