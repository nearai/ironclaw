//! Conversation widget: renders chat messages with basic markdown.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::render::{render_markdown, wrap_text};
use crate::theme::Theme;

use super::{AppState, MessageRole, TuiWidget};

pub struct ConversationWidget {
    theme: Theme,
}

impl ConversationWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for ConversationWidget {
    fn id(&self) -> &str {
        "conversation"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 4 {
            return;
        }

        let usable_width = (area.width as usize).saturating_sub(4);
        let mut all_lines: Vec<Line<'_>> = Vec::new();

        for msg in &state.messages {
            let (prefix, style) = match msg.role {
                MessageRole::User => ("\u{25CF} ", self.theme.accent_style()),
                MessageRole::Assistant => ("", Style::default().fg(self.theme.fg.to_color())),
                MessageRole::System => ("\u{25CB} ", self.theme.dim_style()),
            };

            if msg.role == MessageRole::User {
                // Blank line before user messages (except first)
                if !all_lines.is_empty() {
                    all_lines.push(Line::from(""));
                }
                let user_line = Line::from(vec![
                    Span::styled(prefix.to_string(), self.theme.accent_style()),
                    Span::styled(msg.content.clone(), self.theme.bold_style()),
                ]);
                all_lines.push(user_line);
                all_lines.push(Line::from(""));
            } else if msg.role == MessageRole::Assistant {
                // Separator before assistant response
                let sep = "\u{2500}".repeat(usable_width.min(60));
                all_lines.push(Line::from(Span::styled(
                    format!("  {sep}"),
                    self.theme.dim_style(),
                )));

                let wrapped =
                    render_markdown(&msg.content, usable_width.saturating_sub(2), &self.theme);
                for line in wrapped {
                    let mut padded = vec![Span::raw("  ".to_string())];
                    padded.extend(
                        line.spans
                            .into_iter()
                            .map(|s| Span::styled(s.content.to_string(), s.style)),
                    );
                    all_lines.push(Line::from(padded));
                }
                all_lines.push(Line::from(""));
            } else {
                let wrapped = wrap_text(&msg.content, usable_width, style);
                all_lines.extend(wrapped);
            }
        }

        // Show thinking indicator if active
        if !state.status_text.is_empty() && !state.is_streaming {
            all_lines.push(Line::from(Span::styled(
                format!("  \u{25CB} {}", state.status_text),
                self.theme.dim_style(),
            )));
        }

        // Compute visible window (scroll from bottom)
        let visible_height = area.height as usize;
        let total_lines = all_lines.len();
        let scroll = state.scroll_offset as usize;
        let start = total_lines.saturating_sub(visible_height + scroll);
        let end = total_lines.saturating_sub(scroll).min(total_lines);

        let visible: Vec<Line<'_>> = all_lines
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect();

        let paragraph = ratatui::widgets::Paragraph::new(visible);
        paragraph.render(area, buf);
    }
}

impl ConversationWidget {
    /// Handle scroll up/down. Returns true if scrolling occurred.
    pub fn scroll(&self, state: &mut AppState, delta: i16) {
        if delta < 0 {
            state.scroll_offset = state.scroll_offset.saturating_add(delta.unsigned_abs());
        } else {
            state.scroll_offset = state.scroll_offset.saturating_sub(delta as u16);
        }
    }
}
