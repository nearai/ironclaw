//! Transcript widget: renders `state.transcript` top-to-bottom inside a
//! bordered, wrapping paragraph. Pure function of `&AppState` — no
//! scrolling/paging logic yet (MVP renders everything ratatui's `Paragraph`
//! will fit; a scroll offset is a follow-up, not fabricated here).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{AppState, TranscriptItem, wire_label};

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let lines: Vec<Line> = state.transcript.iter().map(render_item).collect();
    let block = Block::default().borders(Borders::ALL).title("transcript");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
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
}
