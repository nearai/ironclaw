//! Transcript widget: renders a window of `state.transcript` inside a
//! bordered, wrapping paragraph. Pure function of `&AppState` — windowing is
//! item-count based (one `TranscriptItem` maps to one *pre-wrap item slot*,
//! itself expanded into one or more `Line`s when the item's body has
//! embedded `\n`s), not a precise post-wrap row count, matching
//! `app/mod.rs`'s `PageUp`/`PageDown` paging (which is likewise item-count
//! based, since the reducer has no access to the render width).
//! `state.transcript_scroll` drives which window of *items* is shown:
//! `None` (follow) always shows the tail, so newly appended items stay
//! visible; `Some(start)` pins the window to an absolute start item index,
//! so it holds position across new content — see `app/mod.rs`'s field doc.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{AppState, TranscriptItem, wire_label};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let item_lines: Vec<Vec<Line<'static>>> = state.transcript.iter().map(render_item).collect();
    let window = visible_window(&item_lines, area, state.transcript_scroll);
    let block = Block::default().borders(Borders::ALL).title("transcript");
    let paragraph = Paragraph::new(window)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Slices `items` (one entry per `TranscriptItem`, each already expanded
/// into its own `Line`s) down to what the pane can show, per
/// `state.transcript_scroll`, then flattens the selected items' lines into
/// the single `Vec<Line>` the `Paragraph` renders. `area`'s border consumes
/// the top/bottom row, so the usable height is `area.height - 2` (floored
/// at 1 so a degenerate/tiny area still shows something rather than
/// panicking on the slice). The item-count-based window size is an
/// approximation of the visible row budget — a multi-line item can still
/// push later items off-screen — matching this module's existing
/// item-count paging contract with `app/mod.rs`.
fn visible_window(
    items: &[Vec<Line<'static>>],
    area: Rect,
    scroll: Option<usize>,
) -> Vec<Line<'static>> {
    let len = items.len();
    let visible = (area.height.saturating_sub(2)).max(1) as usize;
    let start = match scroll {
        Some(idx) => idx.min(len),
        None => len.saturating_sub(visible),
    };
    let end = (start + visible).min(len);
    items[start..end].iter().flatten().cloned().collect()
}

/// Splits `text` on embedded `\n` into one `Line` per physical line, all
/// sharing `style`, so multi-line item bodies (e.g. a numbered list in an
/// LLM reply) render as separate visible rows instead of being squished
/// onto one line. Any individual line still soft-wraps via the
/// `Paragraph`'s `Wrap` setting when it's wider than the pane.
fn split_lines(text: &str, style: Style) -> Vec<Line<'static>> {
    text.split('\n')
        .map(|segment| Line::styled(segment.to_string(), style))
        .collect()
}

/// Same as `split_lines`, but the first physical line is prefixed with a
/// distinctly styled label span (e.g. `"you: "`); the label is not repeated
/// on subsequent physical lines of a multi-line body.
fn split_lines_with_prefix(prefix: &str, prefix_style: Style, text: &str) -> Vec<Line<'static>> {
    let mut segments = text.split('\n');
    let mut lines = Vec::new();
    if let Some(first) = segments.next() {
        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), prefix_style),
            Span::raw(first.to_string()),
        ]));
    }
    for segment in segments {
        lines.push(Line::from(Span::raw(segment.to_string())));
    }
    lines
}

fn render_item(item: &TranscriptItem) -> Vec<Line<'static>> {
    match item {
        TranscriptItem::User { text } => {
            split_lines_with_prefix("you: ", Style::default().fg(Color::White), text)
        }
        TranscriptItem::Assistant { text } => {
            split_lines_with_prefix("assistant: ", Style::default().fg(Color::Cyan), text)
        }
        TranscriptItem::LiveText { body, .. } => {
            split_lines_with_prefix("assistant: ", Style::default().fg(Color::Cyan), body)
        }
        TranscriptItem::Thinking { body, .. } => split_lines(
            &format!("thinking: {body}"),
            Style::default().fg(Color::DarkGray),
        ),
        TranscriptItem::Activity(activity) => {
            let status = wire_label(&activity.status);
            let detail = activity.subtitle.clone().unwrap_or_default();
            split_lines(
                &format!("[{status}] {} {detail}", activity.capability_id),
                Style::default().fg(Color::DarkGray),
            )
        }
        TranscriptItem::Preview(preview) => {
            let summary = preview.output_summary.clone().unwrap_or_default();
            split_lines(
                &format!("{}: {summary}", preview.title),
                Style::default().fg(Color::Magenta),
            )
        }
        TranscriptItem::System { text } => split_lines(text, Style::default().fg(Color::DarkGray)),
        TranscriptItem::Error { text } => {
            split_lines(&format!("error: {text}"), Style::default().fg(Color::Red))
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

    #[test]
    fn multiline_item_body_splits_on_embedded_newlines_into_separate_rows() {
        // Live repro: "list the integers 1..60, one per line" rendered as
        // one squished row ("123456789101112…5960") because embedded `\n`s
        // in the item body were never turned into separate `Line`s.
        let mut state = AppState::default();
        state.transcript.push(TranscriptItem::final_text("a\nb\nc"));

        let content = draw(&state, 40, 10);

        assert!(
            !content.contains("abc"),
            "embedded newlines must not collapse onto one squished line: {content:?}"
        );
        let rows: Vec<&str> = content.lines().collect();
        let row_of = |needle: &str| rows.iter().position(|r| r.contains(needle));
        let row_a = row_of("assistant: a").expect("first segment renders");
        let row_b = row_of("│b").expect("second segment renders on its own row");
        let row_c = row_of("│c").expect("third segment renders on its own row");
        assert_ne!(
            row_a, row_b,
            "each newline-separated segment must be its own visible line: {content:?}"
        );
        assert_ne!(
            row_b, row_c,
            "each newline-separated segment must be its own visible line: {content:?}"
        );
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
