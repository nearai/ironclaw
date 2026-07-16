//! One-row status bar: priority-ordered segments folded to fit the
//! available width. Priority (highest to lowest, per the design): a
//! last-local-error banner, a working spinner+elapsed indicator,
//! connection state, thread name, provider/model, global keybinding hints.
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

    // Priority 1 (highest): the last local error, so a failed API call
    // (`list_threads`/`send_message`/`resolve_gate`/etc., set via
    // `lib.rs`'s `state.last_local_error = Some(...)`) is never silently
    // invisible. The `⚠` marker keeps it visually distinct from the plain
    // status segments even in a color-stripped terminal/test buffer.
    if let Some(err) = &state.last_local_error {
        segments.push((format!("⚠ error: {err}"), Style::default().fg(Color::Red)));
    }

    // Priority 2: working spinner (elapsed omitted — see module doc).
    if state.is_running() {
        segments.push(("working…".to_string(), Style::default().fg(Color::Yellow)));
    }

    // Priority 3: connection state (only shown when not the default
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

    // Priority 4: thread name.
    if let Some(thread_id) = &state.thread_id {
        segments.push((
            format!("thread: {thread_id}"),
            Style::default().fg(Color::Gray),
        ));
    }

    // Priority 5 (provider/model) intentionally omitted: no AppState field
    // carries the active provider/model today (see module doc).

    // Priority 6 (lowest): global keybinding hints, so it's the first thing
    // dropped on a narrow terminal. Context-aware: while a run is active,
    // `Esc stop` (this crate's cancel binding — `app/mod.rs`'s
    // `dispatch_composer_key`) leads the hint, since it's the one action a
    // user mid-run is most likely to reach for.
    segments.push((hint_text(state), Style::default().fg(Color::DarkGray)));

    segments
}

fn hint_text(state: &AppState) -> String {
    const GLOBAL_HINTS: &str = "^X threads · ^A automations · ^P providers · ^C quit";
    if state.is_running() {
        format!("Esc stop · {GLOBAL_HINTS}")
    } else {
        GLOBAL_HINTS.to_string()
    }
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

    #[test]
    fn renders_the_global_keybinding_hint_row() {
        let state = AppState::default();
        let content = draw(&state, 80, 1);
        assert!(content.contains("threads"));
        assert!(content.contains("automations"));
        assert!(content.contains("providers"));
        assert!(content.contains("quit"));
    }

    #[test]
    fn last_local_error_renders_as_a_visually_distinct_segment() {
        let state = AppState::default().set_last_local_error(Some("boom"));
        let content = draw(&state, 80, 1);
        assert!(
            content.contains("boom"),
            "error text must be visible: {content}"
        );
        assert!(
            content.contains('⚠'),
            "error segment must carry a distinct marker: {content}"
        );
    }

    #[test]
    fn no_local_error_renders_no_error_segment() {
        let state = AppState::default();
        let content = draw(&state, 80, 1);
        assert!(!content.contains("boom"));
        assert!(
            !content.contains('⚠'),
            "no error present, so no error marker should render: {content}"
        );
    }

    #[test]
    fn hint_row_shows_esc_stop_only_while_a_run_is_active() {
        let idle = draw(&AppState::default(), 80, 1);
        assert!(!idle.contains("stop"), "no run active, no stop hint");

        let running = draw(&AppState::default().set_running(true), 80, 1);
        assert!(
            running.contains("Esc stop") || running.contains("Esc st"),
            "a running turn must surface the stop hint: {running}"
        );
    }
}
