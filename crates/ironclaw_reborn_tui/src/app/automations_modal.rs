//! Automations modal: list, pause/resume, inline rename. Mirrors
//! `threads_modal.rs`'s shape. Re-lists after an effect resolves rather than
//! flipping `state` locally (no optimistic update) — the server response is
//! the source of truth for `state`/`is_active`.

use crossterm::event::{KeyCode, KeyEvent};

use super::{ApiCall, AppState, Effect, Modal};
use crate::client::AutomationSummary;

/// Wire `RebornAutomationState` values (raw snake_case strings on
/// `AutomationSummary::state` — see `client/automations.rs`'s doc comment)
/// that count as "currently running" for the purpose of deciding whether
/// `Space` pauses or resumes.
const ACTIVE_STATES: [&str; 2] = ["active", "scheduled"];

/// Wire `RebornAutomationRecentRunStatus` value that marks a run still
/// executing (see `client/automations.rs`'s doc comment on
/// `AutomationRecentRun::status`).
const RUNNING_RUN_STATUS: &str = "running";

/// Default page size for `ApiCall::LoadTimeline` when a thread is opened
/// from this modal — mirrors `threads_modal.rs`'s `DEFAULT_TIMELINE_LIMIT`
/// (no cursor, newest page).
const DEFAULT_TIMELINE_LIMIT: u32 = 50;

/// Picks the run thread `Enter` should open for the selected automation:
/// prefer the currently running run's thread, else fall back to the most
/// recent run that has been accepted (carries a `thread_id`). `recent_runs`
/// is newest-first (mirrors `RebornAutomationInfo::recent_runs`), so the
/// first match in either pass is the one to use. Returns `None` when the
/// automation has never had an accepted run — `Enter` is then a no-op.
fn target_thread_id(automation: &AutomationSummary) -> Option<String> {
    automation
        .recent_runs
        .iter()
        .find(|run| run.status == RUNNING_RUN_STATUS && run.thread_id.is_some())
        .or_else(|| {
            automation
                .recent_runs
                .iter()
                .find(|run| run.thread_id.is_some())
        })
        .and_then(|run| run.thread_id.clone())
}

#[derive(Debug, Clone, Default)]
pub struct AutomationsModalState {
    pub automations: Vec<AutomationSummary>,
    pub selected: usize,
    pub loading: bool,
    /// `Some(draft)` while renaming the selected row inline; the draft name
    /// being typed before `Enter` commits it as `ApiCall::RenameAutomation`.
    pub renaming: Option<String>,
}

/// `Ctrl+A` from the composer: opens the modal and requests the automation
/// list.
pub(crate) fn open(state: &mut AppState) -> Vec<Effect> {
    state.modal = Some(Modal::Automations(AutomationsModalState {
        loading: true,
        ..AutomationsModalState::default()
    }));
    vec![Effect::Api(ApiCall::ListAutomations)]
}

pub(crate) fn dispatch_key(
    state: &mut AppState,
    key: KeyEvent,
    modal: AutomationsModalState,
) -> Vec<Effect> {
    if modal.renaming.is_some() {
        return dispatch_rename_key(state, key, modal);
    }
    let mut modal = modal;
    match key.code {
        KeyCode::Esc => {
            state.modal = None;
            Vec::new()
        }
        KeyCode::Up => {
            modal.selected = modal.selected.saturating_sub(1);
            state.modal = Some(Modal::Automations(modal));
            Vec::new()
        }
        KeyCode::Down => {
            if modal.selected + 1 < modal.automations.len() {
                modal.selected += 1;
            }
            state.modal = Some(Modal::Automations(modal));
            Vec::new()
        }
        // Opens the selected automation's run thread — the unblock path for
        // a held (Approval/Auth) automation: the thread's existing gate zone
        // renders and resolves the pending gate exactly like any other
        // thread's, via `ApiCall::ResolveGate`. No new resolve mechanism.
        // Mirrors `threads_modal.rs`'s Enter-on-a-thread-row handling.
        KeyCode::Enter => {
            let Some(automation) = modal.automations.get(modal.selected) else {
                state.modal = Some(Modal::Automations(modal));
                return Vec::new();
            };
            let Some(thread_id) = target_thread_id(automation) else {
                // No accepted run yet for this automation: graceful no-op.
                state.modal = Some(Modal::Automations(modal));
                return Vec::new();
            };
            state.thread_id = Some(thread_id.clone());
            state.transcript.clear();
            // The newly opened thread cannot own whatever gate was pending
            // on the previously open thread (if any); `LoadTimeline` below
            // is the recovery if this thread has its own blocked run.
            state.pending_gate = None;
            state.modal = None;
            vec![Effect::Api(ApiCall::LoadTimeline {
                thread_id,
                limit: DEFAULT_TIMELINE_LIMIT,
                cursor: None,
            })]
        }
        // No delete key bound here on purpose: `ApiCall` has no
        // `DeleteAutomation` variant. See
        // `automations_modal_does_not_bind_a_delete_key` below.
        KeyCode::Char(' ') => {
            let (id, is_active) = match modal.automations.get(modal.selected) {
                Some(automation) => (
                    automation.automation_id.clone(),
                    ACTIVE_STATES.contains(&automation.state.as_str()),
                ),
                None => (String::new(), false),
            };
            state.modal = Some(Modal::Automations(modal));
            vec![Effect::Api(if is_active {
                ApiCall::PauseAutomation { id }
            } else {
                ApiCall::ResumeAutomation { id }
            })]
        }
        KeyCode::Char('r') => {
            let Some(automation) = modal.automations.get(modal.selected) else {
                state.modal = Some(Modal::Automations(modal));
                return Vec::new();
            };
            modal.renaming = Some(automation.name.clone());
            state.modal = Some(Modal::Automations(modal));
            Vec::new()
        }
        _ => {
            state.modal = Some(Modal::Automations(modal));
            Vec::new()
        }
    }
}

fn dispatch_rename_key(
    state: &mut AppState,
    key: KeyEvent,
    mut modal: AutomationsModalState,
) -> Vec<Effect> {
    let mut draft = modal.renaming.clone().unwrap_or_default();
    match key.code {
        KeyCode::Esc => {
            modal.renaming = None;
            state.modal = Some(Modal::Automations(modal));
            Vec::new()
        }
        KeyCode::Enter => {
            let Some(automation) = modal.automations.get(modal.selected) else {
                modal.renaming = None;
                state.modal = Some(Modal::Automations(modal));
                return Vec::new();
            };
            let id = automation.automation_id.clone();
            modal.renaming = None;
            state.modal = Some(Modal::Automations(modal));
            vec![Effect::Api(ApiCall::RenameAutomation { id, name: draft })]
        }
        KeyCode::Backspace => {
            draft.pop();
            modal.renaming = Some(draft);
            state.modal = Some(Modal::Automations(modal));
            Vec::new()
        }
        KeyCode::Char(c) => {
            draft.push(c);
            modal.renaming = Some(draft);
            state.modal = Some(Modal::Automations(modal));
            Vec::new()
        }
        _ => {
            modal.renaming = Some(draft);
            state.modal = Some(Modal::Automations(modal));
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{automations_modal_with, key};
    use super::super::{ApiCall, AppEvent, AppState, Effect, Modal, reduce};
    use super::*;
    use crate::client::automations::AutomationRecentRun;

    fn recent_run(thread_id: Option<&str>, status: &str) -> AutomationRecentRun {
        AutomationRecentRun {
            thread_id: thread_id.map(str::to_string),
            status: status.to_string(),
        }
    }

    fn automation_with_runs(
        id: &str,
        name: &str,
        state: &str,
        recent_runs: Vec<AutomationRecentRun>,
    ) -> AutomationSummary {
        AutomationSummary {
            automation_id: id.to_string(),
            name: name.to_string(),
            state: state.to_string(),
            next_run_at: None,
            last_run_at: None,
            last_status: None,
            is_active: state == "active",
            active_hold: None,
            recent_runs,
        }
    }

    #[test]
    fn space_on_active_automation_emits_pause() {
        let mut state = AppState::default().set_modal(Some(automations_modal_with(
            &[("a-1", "Daily digest", "active")],
            0,
        )));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char(' '))));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::PauseAutomation { id }) if id == "a-1"
        ));
    }

    #[test]
    fn space_on_paused_automation_emits_resume() {
        let mut state = AppState::default().set_modal(Some(automations_modal_with(
            &[("a-1", "Daily digest", "paused")],
            0,
        )));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char(' '))));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::ResumeAutomation { id }) if id == "a-1"
        ));
    }

    #[test]
    fn automations_modal_does_not_bind_a_delete_key() {
        let mut state = AppState::default().set_modal(Some(automations_modal_with(
            &[("a-1", "Daily digest", "active")],
            0,
        )));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('d'))));
        assert!(effects.is_empty());
        assert!(matches!(state.modal, Some(Modal::Automations(_))));
    }

    #[test]
    fn r_then_enter_emits_rename_automation() {
        let mut state = AppState::default().set_modal(Some(automations_modal_with(
            &[("a-1", "Daily digest", "active")],
            0,
        )));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('r'))));
        assert!(matches!(&state.modal, Some(Modal::Automations(m)) if m.renaming.is_some()));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Backspace)));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('!'))));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::RenameAutomation { id, name })
                if id == "a-1" && name == "Daily diges!"
        ));
        assert!(matches!(&state.modal, Some(Modal::Automations(m)) if m.renaming.is_none()));
    }

    #[test]
    fn enter_on_automation_with_run_thread_opens_it() {
        let automation = automation_with_runs(
            "a-1",
            "Daily digest",
            "active",
            vec![recent_run(Some("thread-1"), "running")],
        );
        let mut state =
            AppState::default().set_modal(Some(Modal::Automations(AutomationsModalState {
                automations: vec![automation],
                selected: 0,
                loading: false,
                renaming: None,
            })));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert_eq!(state.thread_id.as_deref(), Some("thread-1"));
        assert!(
            state.modal.is_none(),
            "opening the automation's thread closes the modal"
        );
        assert!(
            state.transcript.is_empty(),
            "opening a thread clears the previous transcript before reload"
        );
        assert!(state.pending_gate.is_none());
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::LoadTimeline { thread_id, cursor: None, .. })
                if thread_id == "thread-1"
        ));
    }

    #[test]
    fn enter_prefers_the_running_run_over_a_newer_completed_one() {
        // recent_runs is newest-first: the completed run at index 0 is more
        // recent than the still-running one at index 1, but the running
        // run's thread is the one that actually has something to resolve.
        let automation = automation_with_runs(
            "a-1",
            "Daily digest",
            "active",
            vec![
                recent_run(Some("thread-completed"), "ok"),
                recent_run(Some("thread-running"), "running"),
            ],
        );
        let mut state =
            AppState::default().set_modal(Some(Modal::Automations(AutomationsModalState {
                automations: vec![automation],
                selected: 0,
                loading: false,
                renaming: None,
            })));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert_eq!(state.thread_id.as_deref(), Some("thread-running"));
    }

    #[test]
    fn enter_on_automation_without_a_run_thread_is_a_graceful_noop() {
        let automation = automation_with_runs("a-1", "Daily digest", "scheduled", Vec::new());
        let mut state =
            AppState::default().set_modal(Some(Modal::Automations(AutomationsModalState {
                automations: vec![automation],
                selected: 0,
                loading: false,
                renaming: None,
            })));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(effects.is_empty());
        assert!(
            state.thread_id.is_none(),
            "no run thread exists yet, so Enter must not fabricate one"
        );
        assert!(
            matches!(state.modal, Some(Modal::Automations(_))),
            "a no-op Enter must leave the modal open"
        );
    }
}
