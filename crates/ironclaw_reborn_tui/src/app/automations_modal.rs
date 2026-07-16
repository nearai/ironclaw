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
}
