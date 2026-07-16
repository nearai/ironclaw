//! Transcript widget: renders a window of `state.transcript` inside a
//! bordered, wrapping paragraph. Pure function of `&AppState` — windowing is
//! item-count based (one `TranscriptItem` maps to one pre-wrap `Line`), not
//! a precise post-wrap row count, matching `app/mod.rs`'s `PageUp`/
//! `PageDown` paging (which is likewise item-count based, since the reducer
//! has no access to the render width). `state.transcript_scroll` drives
//! which window is shown: `None` (follow) always shows the tail, so newly
//! appended items stay visible; `Some(start)` pins the window to an
//! absolute start index, so it holds position across new content — see
//! `app/mod.rs`'s field doc.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{AppState, TranscriptItem, wire_label};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let all_lines: Vec<Line> = state.transcript.iter().map(render_item).collect();
    let window = visible_window(&all_lines, area, state.transcript_scroll);
    let block = Block::default().borders(Borders::ALL).title("transcript");
    let paragraph = Paragraph::new(window.to_vec())
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Slices `lines` down to what the pane can show, per `state.transcript_scroll`.
/// `area`'s border consumes the top/bottom row, so the usable height is
/// `area.height - 2` (floored at 1 so a degenerate/tiny area still shows
/// something rather than panicking on the slice).
fn visible_window<'a>(
    lines: &'a [Line<'static>],
    area: Rect,
    scroll: Option<usize>,
) -> &'a [Line<'static>] {
    let len = lines.len();
    let visible = (area.height.saturating_sub(2)).max(1) as usize;
    let start = match scroll {
        Some(idx) => idx.min(len),
        None => len.saturating_sub(visible),
    };
    let end = (start + visible).min(len);
    &lines[start..end]
}

fn render_item(item: &TranscriptItem) -> Line<'static> {
    match item {
        TranscriptItem::User { text } => Line::from(vec![
            Span::styled("you: ", Style::default().fg(Color::White)),
            Span::raw(text.clone()),
        ]),
        TranscriptItem::Assistant { text } => Line::from(vec![
            Span::styled("assistant: ", Style::default().fg(Color::Cyan)),
            Span::raw(text.clone()),
        ]),
        TranscriptItem::LiveText { body, .. } => Line::from(vec![
            Span::styled("assistant: ", Style::default().fg(Color::Cyan)),
            Span::raw(body.clone()),
        ]),
        TranscriptItem::Thinking { body, .. } => Line::styled(
            format!("thinking: {body}"),
            Style::default().fg(Color::DarkGray),
        ),
        TranscriptItem::Activity(activity) => {
            let status = wire_label(&activity.status);
            let detail = activity.subtitle.clone().unwrap_or_default();
            Line::styled(
                format!("[{status}] {} {detail}", activity.capability_id),
                Style::default().fg(Color::DarkGray),
            )
        }
        TranscriptItem::Preview(preview) => {
            let summary = preview.output_summary.clone().unwrap_or_default();
            Line::styled(
                format!("{}: {summary}", preview.title),
                Style::default().fg(Color::Magenta),
            )
        }
        TranscriptItem::System { text } => {
            Line::styled(text.clone(), Style::default().fg(Color::DarkGray))
        }
        TranscriptItem::Error { text } => {
            Line::styled(format!("error: {text}"), Style::default().fg(Color::Red))
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::ui::test_support::buffer_text;

    #[test]
    fn renders_each_transcript_item_kind_as_a_visible_line() {
        let mut state = AppState::default();
        state.transcript.push(TranscriptItem::User {
            text: "hi there".to_string(),
        });
        state
            .transcript
            .push(TranscriptItem::final_text("hello back"));
        state.transcript.push(TranscriptItem::System {
            text: "run completed".to_string(),
        });
        state.transcript.push(TranscriptItem::Error {
            text: "provider_unavailable".to_string(),
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, &state);
            })
            .unwrap();
        let content = buffer_text(terminal.backend().buffer());

        assert!(content.contains("hi there"));
        assert!(content.contains("hello back"));
        assert!(content.contains("run completed"));
        assert!(content.contains("provider_unavailable"));
    }

    fn state_with_items(count: usize) -> AppState {
        let mut state = AppState::default();
        for i in 0..count {
            state.transcript.push(TranscriptItem::System {
                text: format!("item-{i:03}"),
            });
        }
        state
    }

    fn draw(state: &AppState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), state)).unwrap();
        buffer_text(terminal.backend().buffer())
    }

    #[test]
    fn default_follow_keeps_the_tail_visible_once_content_overflows_the_pane() {
        // height 10 -> 8 usable rows; 20 items overflow that many times over.
        let state = state_with_items(20);
        let content = draw(&state, 40, 10);
        assert!(
            content.contains("item-019"),
            "the newest item must stay visible by default: {content}"
        );
        assert!(
            !content.contains("item-000"),
            "the oldest item must have scrolled off with the default tail window"
        );
    }

    #[test]
    fn page_up_then_new_content_does_not_force_scroll_back_to_the_tail() {
        let mut state = state_with_items(20);
        // Mirrors app/mod.rs's PageUp: 20 - page(10) = pinned index 10.
        state.transcript_scroll = Some(10);
        let before = draw(&state, 40, 10);
        assert!(before.contains("item-010"));
        assert!(!before.contains("item-019"), "scrolled up, tail not shown");

        state.transcript.push(TranscriptItem::System {
            text: "item-020".to_string(),
        });
        let after = draw(&state, 40, 10);
        assert_eq!(
            before, after,
            "appending content while pinned must not change what's rendered"
        );
    }

    #[test]
    fn end_returns_to_follow_and_shows_the_tail_again() {
        let mut state = state_with_items(20);
        state.transcript_scroll = Some(0);
        let scrolled = draw(&state, 40, 10);
        assert!(scrolled.contains("item-000"));

        state.transcript_scroll = None;
        let followed = draw(&state, 40, 10);
        assert!(
            followed.contains("item-019"),
            "None (follow) must render the tail again"
        );
    }

    #[test]
    fn scroll_index_past_the_end_clamps_instead_of_panicking() {
        let mut state = state_with_items(5);
        state.transcript_scroll = Some(9_999);
        // Must not panic on an out-of-range slice.
        let content = draw(&state, 40, 10);
        assert!(!content.contains("item-000"));
    }
}
