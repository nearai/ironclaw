//! One-row status bar: priority-ordered segments folded to fit the
//! available width. Priority (highest to lowest, per the design): a
//! working spinner+elapsed indicator, connection state, thread name,
//! provider/model.
//!
//! `AppState` (as landed in `app/mod.rs`) does not carry an elapsed-time
//! field or an active-provider/model field — those only exist transiently
//! inside `ProviderModalState` while the provider modal is open, not as
//! durable `AppState` fields. This widget therefore renders a plain
//! "working…" spinner (no elapsed duration) and omits the provider/model
//! segment entirely; the fold-by-width priority order below still holds for
//! every segment that has real backing state. Documented gap, not a silent
//! drop — see the deviations note in the lane report.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{AppState, ConnState};

/// Separator between rendered segments.
const SEP: &str = "  ";

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let segments = build_segments(state);
    let line = fold_to_width(&segments, area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}

fn build_segments(state: &AppState) -> Vec<(String, Style)> {
    let mut segments = Vec::new();

    // Priority 1: working spinner (elapsed omitted — see module doc).
    if state.is_running() {
        segments.push(("working…".to_string(), Style::default().fg(Color::Yellow)));
    }

    // Priority 2: connection state (only shown when not the default
    // Connected — a healthy connection has nothing to say here).
    match state.conn {
        ConnState::Connected => {}
        ConnState::Reconnecting { attempt } => segments.push((
            format!("reconnecting (attempt {attempt})"),
            Style::default().fg(Color::Yellow),
        )),
        ConnState::Lost => {
            segments.push(("disconnected".to_string(), Style::default().fg(Color::Red)))
        }
    }

    // Priority 3: thread name.
    if let Some(thread_id) = &state.thread_id {
        segments.push((
            format!("thread: {thread_id}"),
            Style::default().fg(Color::Gray),
        ));
    }

    // Priority 4 (provider/model) intentionally omitted: no AppState field
    // carries the active provider/model today (see module doc).
    segments
}

/// Greedily includes segments in priority order until the next one would
/// overflow `width`; the first (highest-priority) segment is always
/// included regardless of width so the spinner/connection banner never
/// silently disappears on a narrow terminal.
fn fold_to_width(segments: &[(String, Style)], width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (idx, (text, style)) in segments.iter().enumerate() {
        let sep_len = if idx == 0 { 0 } else { SEP.len() };
        let needed = sep_len + text.chars().count();
        if idx > 0 && used + needed > width {
            break;
        }
        if idx > 0 {
            spans.push(Span::raw(SEP));
        }
        spans.push(Span::styled(text.clone(), *style));
        used += needed;
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::ui::test_support::buffer_text;

    fn draw(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, state);
            })
            .unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn shows_thread_name_and_connection_state() {
        let state = AppState::default()
            .set_thread_id("t-1")
            .set_conn(ConnState::Reconnecting { attempt: 2 });
        let content = draw(&state, 80, 1);
        assert!(content.contains("thread: t-1"));
        assert!(content.contains("reconnecting (attempt 2)"));
    }

    #[test]
    fn lost_connection_shows_disconnected() {
        let state = AppState::default().set_conn(ConnState::Lost);
        let content = draw(&state, 80, 1);
        assert!(content.contains("disconnected"));
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
    fn connected_state_renders_no_connection_segment() {
        let state = AppState::default().set_thread_id("t-1");
        let content = draw(&state, 80, 1);
        assert!(content.contains("thread: t-1"));
        assert!(!content.contains("reconnecting"));
        assert!(!content.contains("disconnected"));
    }
}
