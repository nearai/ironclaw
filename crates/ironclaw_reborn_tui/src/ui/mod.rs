//! Ratatui render layer for the Reborn TUI. [`render`] is the single entry
//! point the event loop (`lib.rs`) calls once per frame; every widget below
//! is a pure function of `&AppState` — no I/O, no mutation.
//!
//! Layout A (single column, top to bottom): transcript / an inline gate
//! zone that only takes space while a gate or auth prompt is pending /
//! composer / one-row status bar. A modal (`state.modal`), when open,
//! renders as a `Clear`-backed centered popup on top of everything else.

pub mod gate_zone;
pub mod modals;
pub mod status_bar;
pub mod transcript;

#[cfg(test)]
mod test_support;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::AppState;

/// Upper bound for gate content so a long URL or explanation cannot consume
/// the whole screen.
const MAX_GATE_ZONE_HEIGHT: u16 = 12;
/// A bordered transcript needs three rows to retain one visible content row.
const MIN_TRANSCRIPT_HEIGHT: u16 = 3;
/// Fixed row budget for the composer (bordered single-line input).
const COMPOSER_HEIGHT: u16 = 3;
const STATUS_HEIGHT: u16 = 1;

/// Renders the whole screen for one frame. Called once per event-loop tick
/// by `lib.rs`'s `run_event_loop`.
pub fn render(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let gate_zone_height = state.pending_gate.as_ref().map_or(0, |gate| {
        let available = area.height.saturating_sub(
            MIN_TRANSCRIPT_HEIGHT
                .saturating_add(COMPOSER_HEIGHT)
                .saturating_add(STATUS_HEIGHT),
        );
        gate_zone::desired_height(gate, area.width)
            .min(MAX_GATE_ZONE_HEIGHT)
            .min(available)
    });
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(gate_zone_height),
        Constraint::Length(COMPOSER_HEIGHT),
        Constraint::Length(STATUS_HEIGHT),
    ])
    .split(area);

    transcript::render(frame, chunks[0], state);
    if state.pending_gate.is_some() {
        gate_zone::render(frame, chunks[1], state);
    }
    render_composer(frame, chunks[2], state);
    status_bar::render(frame, chunks[3], state);

    if let Some(modal) = &state.modal {
        let popup = centered_rect(70, 70, area);
        frame.render_widget(Clear, popup);
        modals::render(frame, popup, modal);
    }
}

fn render_composer(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default().borders(Borders::ALL).title("message");
    let paragraph = Paragraph::new(Line::from(state.composer_text.as_str())).block(block);
    frame.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::test_support::buffer_text;
    use super::*;
    use crate::app::{Modal, PendingGate, ThreadsModalState, TranscriptItem};

    fn draw(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, state)).unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn main_screen_renders_transcript_and_status_bar() {
        let mut state = AppState::default().set_thread_id("t-1");
        state
            .transcript
            .push(TranscriptItem::final_text("hello from the assistant"));
        let content = draw(&state, 80, 24);
        assert!(content.contains("hello from the assistant"));
        assert!(content.contains("t-1"));
    }

    #[test]
    fn status_bar_sheds_low_priority_fields_on_narrow_width() {
        let state = AppState::default().set_running(true);
        let content = draw(&state, 30, 10);
        assert!(
            content.contains("…") || content.contains("wor"),
            "spinner survives narrow width"
        );
    }

    #[test]
    fn modal_open_renders_a_centered_popup_on_top() {
        let state =
            AppState::default().set_modal(Some(Modal::Threads(ThreadsModalState::default())));
        let content = draw(&state, 80, 24);
        assert!(
            content.contains("+ new"),
            "threads modal pinned entry renders inside the popup"
        );
    }

    #[test]
    fn composer_text_renders_in_its_own_row() {
        let state = AppState::default().set_composer_text("draft message");
        let content = draw(&state, 80, 24);
        assert!(content.contains("draft message"));
    }

    #[test]
    fn approval_gate_layout_keeps_controls_and_surrounding_panes_visible() {
        let state = AppState::default()
            .set_pending_gate(Some(PendingGate::approval_stub("Allow write_file?")))
            .set_composer_text("draft message");

        let content = draw(&state, 80, 16);

        assert!(content.contains("Allow write_file?"));
        assert!(content.contains("[a] allow"));
        assert!(content.contains("[d] deny"));
        assert!(content.contains("transcript"));
        assert!(content.contains("draft message"));
    }

    #[test]
    fn auth_gate_layout_keeps_url_challenge_and_controls_visible() {
        let state = AppState::default().set_pending_gate(Some(PendingGate::Auth {
            turn_run_id: "run-auth".to_string(),
            gate_ref: "gate-auth".to_string(),
            headline: "Connect Gmail".to_string(),
            body: "Sign in to continue.".to_string(),
            challenge_kind: Some("oauth_url".to_string()),
            authorization_url: Some(
                "https://accounts.example.com/oauth/authorize?client_id=ironclaw&scope=mail"
                    .to_string(),
            ),
            provider: Some("google".to_string()),
            account_label: Some("Gmail".to_string()),
            token_input: None,
        }));

        let content = draw(&state, 80, 18);

        assert!(content.contains("Connect Gmail"));
        assert!(content.contains("kind: oauth_url"));
        assert!(content.contains("[o] open"));
        assert!(content.contains("[t] enter token"));
        assert!(content.contains("message"));
    }

    #[test]
    fn manual_token_layout_keeps_masked_input_and_submit_controls_visible() {
        let state = AppState::default().set_pending_gate(Some(PendingGate::Auth {
            turn_run_id: "run-auth".to_string(),
            gate_ref: "gate-auth".to_string(),
            headline: "Connect GitHub".to_string(),
            body: "Paste a personal access token.".to_string(),
            challenge_kind: Some("manual_token".to_string()),
            authorization_url: None,
            provider: Some("github".to_string()),
            account_label: Some("GitHub PAT".to_string()),
            token_input: Some("secret".to_string()),
        }));

        let content = draw(&state, 60, 14);

        assert!(content.contains("token: ******"));
        assert!(!content.contains("secret"));
        assert!(content.contains("[enter] submit"));
        assert!(content.contains("[esc] cancel"));
        assert!(content.contains("message"));
    }
}
