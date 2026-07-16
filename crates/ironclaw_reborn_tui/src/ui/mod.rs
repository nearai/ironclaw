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

/// Fixed row budget for the gate zone when a gate/auth prompt is pending
/// (headline + wrapped body + one options line, plus the block's border).
const GATE_ZONE_HEIGHT: u16 = 5;
/// Fixed row budget for the composer (bordered single-line input).
const COMPOSER_HEIGHT: u16 = 3;

/// Renders the whole screen for one frame. Called once per event-loop tick
/// by `lib.rs`'s `run_event_loop`.
pub fn render(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let gate_zone_height = if state.pending_gate.is_some() {
        GATE_ZONE_HEIGHT
    } else {
        0
    };
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(gate_zone_height),
        Constraint::Length(COMPOSER_HEIGHT),
        Constraint::Length(1),
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
    use crate::app::{Modal, ThreadsModalState, TranscriptItem};

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
}
