//! Transcript widget: renders a window of `state.transcript` inside a
//! bordered, wrapping paragraph. Pure function of `&AppState` — windowing is
//! row-budget based after each `TranscriptItem` is expanded into physical
//! lines and soft wrapping is accounted for. `app/mod.rs`'s
//! `PageUp`/`PageDown` reducer still stores an item index because it has no
//! access to render dimensions; this module uses that index as the pinned
//! window's starting item and fills the available rendered rows from there.
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
    let window: Vec<Line<'static>> =
        visible_items(&state.transcript, area, state.transcript_scroll)
            .iter()
            .flat_map(render_item)
            .collect();
    let block = Block::default().borders(Borders::ALL).title("transcript");
    let paragraph = Paragraph::new(window)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Slices `items` down to what the pane can show, per
/// `state.transcript_scroll`, before the selected items are expanded into
/// rendered lines. `area`'s border consumes the top/bottom row, so the usable
/// height is `area.height - 2` (floored at 1 so a degenerate/tiny area still
/// shows something rather than panicking on the slice). Each item's row cost
/// includes both explicit newlines and soft wrapping at the pane's inner
/// width. The stored scroll position remains an item index; only the visible
/// window's end is derived from the rendered row budget.
fn visible_items(items: &[TranscriptItem], area: Rect, scroll: Option<usize>) -> &[TranscriptItem] {
    let len = items.len();
    let row_budget = usize::from(area.height.saturating_sub(2).max(1));
    let inner_width = usize::from(area.width.saturating_sub(2).max(1));

    let (start, end) = match scroll {
        Some(idx) => {
            let start = idx.min(len);
            let mut end = start;
            let mut used_rows = 0_usize;
            while end < len {
                let rows = rendered_rows(&items[end], inner_width);
                if end > start && used_rows.saturating_add(rows) > row_budget {
                    break;
                }
                used_rows = used_rows.saturating_add(rows);
                end += 1;
            }
            (start, end)
        }
        None => {
            let end = len;
            let mut start = end;
            let mut used_rows = 0_usize;
            while start > 0 {
                let rows = rendered_rows(&items[start - 1], inner_width);
                if start < end && used_rows.saturating_add(rows) > row_budget {
                    break;
                }
                used_rows = used_rows.saturating_add(rows);
                start -= 1;
            }
            (start, end)
        }
    };
    &items[start..end]
}

fn rendered_rows(item: &TranscriptItem, inner_width: usize) -> usize {
    render_item(item)
        .iter()
        .map(|line| line.width().max(1).div_ceil(inner_width))
        .sum()
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
    fn long_transcript_renders_only_the_existing_visible_item_slice() {
        // height 10 -> 8 rows. Selecting before final rendering must avoid
        // formatting the other 992 items without changing visible output.
        let state = state_with_items(1_000);
        let area = Rect::new(0, 0, 40, 10);
        let selected = visible_items(&state.transcript, area, None);

        assert_eq!(selected.len(), 8);
        assert!(matches!(
            &selected[0],
            TranscriptItem::System { text } if text == "item-992"
        ));
        assert!(matches!(
            &selected[7],
            TranscriptItem::System { text } if text == "item-999"
        ));

        let mut slice_state = AppState::default();
        slice_state.transcript.extend(selected.iter().cloned());
        assert_eq!(
            draw(&state, 40, 10),
            draw(&slice_state, 40, 10),
            "pre-render item slicing must preserve the existing window output"
        );
    }

    #[test]
    fn default_follow_counts_multiline_rows_before_selecting_the_tail() {
        // height 10 -> 8 usable rows. These seven items would fit an
        // item-count window, but consume nine rendered rows: one explicit
        // newline plus a soft-wrapped 39-cell continuation make the first
        // item three rows tall.
        let mut state = AppState::default();
        state.transcript.push(TranscriptItem::final_text(format!(
            "line one\n{}",
            "x".repeat(39)
        )));
        for i in 1..=6 {
            state.transcript.push(TranscriptItem::System {
                text: format!("item-{i:03}"),
            });
        }

        let content = draw(&state, 40, 10);

        assert!(
            content.contains("item-006"),
            "the newest item must remain visible after multiline history: {content}"
        );
        assert!(
            !content.contains("line one"),
            "the oldest multiline item must scroll off to preserve the tail: {content}"
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
