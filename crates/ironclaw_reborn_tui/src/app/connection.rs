//! `Conn(ConnState)` handling. The actual disconnected *key-blocking*
//! behavior lives in `app/mod.rs`'s single shared seam (`dispatch_key`) —
//! this module only owns the state transition itself.

use super::{AppState, ConnState, Effect};

pub(crate) fn apply_conn_change(state: &mut AppState, conn: ConnState) -> Vec<Effect> {
    state.conn = conn;
    if !matches!(state.conn, ConnState::Lost) {
        state.last_local_error = None;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;

    use super::super::test_support::key;
    use super::super::{AppEvent, AutomationsModalState, Modal, reduce};
    use super::*;

    #[test]
    fn lost_connection_keeps_open_modal_but_blocks_new_api_effects() {
        let mut state = AppState::default()
            .set_modal(Some(Modal::Automations(AutomationsModalState::default())));
        reduce(&mut state, AppEvent::Conn(ConnState::Lost));
        assert!(state.modal.is_some(), "modal stays open while disconnected");
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char(' '))));
        assert!(
            effects.is_empty(),
            "server-touching action while Lost returns a local error, not an Effect"
        );
        assert_eq!(
            state.last_local_error.as_deref(),
            Some("disconnected — reconnecting…")
        );
    }

    #[test]
    fn reconnecting_does_not_block_actions_only_lost_does() {
        let mut state = AppState::default().set_conn(ConnState::Reconnecting { attempt: 1 });
        let effects = reduce(
            &mut state,
            AppEvent::Key(super::super::test_support::ctrl('x')),
        );
        assert!(
            !effects.is_empty(),
            "Reconnecting still allows queuing effects; only Lost hard-blocks"
        );
    }

    #[test]
    fn conn_transition_to_connected_clears_last_local_error() {
        let mut state = AppState::default()
            .set_conn(ConnState::Lost)
            .set_last_local_error(Some("x"));
        reduce(&mut state, AppEvent::Conn(ConnState::Connected));
        assert!(state.last_local_error.is_none());
    }
}
