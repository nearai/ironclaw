//! User input area widget using tui-textarea.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use tui_textarea::TextArea;

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{AppState, TuiWidget};

pub struct InputBoxWidget {
    theme: Theme,
    textarea: TextArea<'static>,
}

impl InputBoxWidget {
    pub fn new(theme: Theme) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea
            .set_block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::NONE));
        textarea.set_placeholder_text("Ask anything... (/ for commands, F1 for help)");
        textarea.set_placeholder_style(theme.dim_style());
        Self { theme, textarea }
    }

    /// Get the current input text and clear the textarea.
    pub fn take_input(&mut self) -> String {
        let lines: Vec<String> = self
            .textarea
            .lines()
            .iter()
            .map(|l| l.to_string())
            .collect();
        let text = lines.join("\n");
        // Clear by selecting all and deleting
        self.textarea.select_all();
        self.textarea.cut();
        text
    }

    /// Returns true if the textarea is empty.
    pub fn is_empty(&self) -> bool {
        self.textarea.lines().iter().all(|l| l.is_empty())
    }

    /// Insert a literal newline at the cursor.
    pub fn insert_newline(&mut self) {
        self.textarea.insert_newline();
    }

    /// Move the cursor to the very top of the input (row 0, col 0).
    pub fn move_cursor_to_start(&mut self) {
        self.textarea.move_cursor(tui_textarea::CursorMove::Top);
        self.textarea.move_cursor(tui_textarea::CursorMove::Head);
    }

    /// Peek at the current text content without consuming it.
    pub fn current_text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Return the current cursor position as (row, column).
    pub fn cursor(&self) -> (usize, usize) {
        self.textarea.cursor()
    }

    /// Returns true when the cursor is on the first input line.
    pub fn is_cursor_on_first_line(&self) -> bool {
        self.cursor().0 == 0
    }

    /// Returns true when the cursor is on the last input line.
    pub fn is_cursor_on_last_line(&self) -> bool {
        let (row, _) = self.cursor();
        row + 1 >= self.textarea.lines().len().max(1)
    }

    /// Replace the current text content with `text`.
    pub fn set_text(&mut self, text: &str) {
        self.textarea.select_all();
        self.textarea.cut();
        self.textarea.insert_str(text);
    }

    /// Insert text at the current cursor position.
    ///
    /// Normalizes line endings (`\r\n` and lone `\r` → `\n`) so pastes from
    /// terminals that use CR or CRLF line breaks (macOS Terminal.app, some
    /// clipboards) show up as real newlines instead of a single long line.
    pub fn insert_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.textarea.insert_str(normalized);
    }

    /// Number of logical lines currently in the input.
    pub fn line_count(&self) -> usize {
        self.textarea.lines().len().max(1)
    }
}

impl TuiWidget for InputBoxWidget {
    fn id(&self) -> &str {
        "input_box"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 4 {
            return;
        }

        // If there are pending attachments, render a chip row on the first line
        let (attachment_row_height, input_start_y) = if !state.pending_attachments.is_empty() {
            (1u16, area.y + 1)
        } else {
            (0u16, area.y)
        };

        if attachment_row_height > 0 {
            let chip_area = Rect {
                x: area.x + 4,
                y: area.y,
                width: area.width.saturating_sub(4),
                height: 1,
            };
            let chips: Vec<Span<'_>> = state
                .pending_attachments
                .iter()
                .flat_map(|a| {
                    vec![Span::styled(
                        format!(" [{}] ", a.label),
                        self.theme.accent_style().add_modifier(Modifier::BOLD),
                    )]
                })
                .collect();
            let chip_line = Line::from(chips);
            let chip_paragraph = ratatui::widgets::Paragraph::new(chip_line);
            chip_paragraph.render(chip_area, buf);
        }

        let remaining_area = Rect {
            x: area.x,
            y: input_start_y,
            width: area.width,
            height: area.height.saturating_sub(attachment_row_height),
        };

        // Render prompt character. `shell_mode` swaps the normal
        // `›` arrow for `!` so the user has an unambiguous visual
        // signal that the next submit will dispatch the buffered
        // text as a shell command instead of an LLM message —
        // mirrors the gateway's `!`-prefix badge. Approval still
        // takes precedence because it's modal.
        let prompt = if state.pending_approval.is_some() {
            "\u{25C6}"
        } else if state.shell_mode {
            "!"
        } else {
            "\u{203A}"
        };

        // Pick a colour that differentiates shell mode from a
        // normal submit so the eye catches the mode change even at
        // a glance. Warning-yellow matches the gateway CSS badge.
        let prompt_style = if state.shell_mode {
            self.theme.warning_style().add_modifier(Modifier::BOLD)
        } else {
            self.theme.accent_style().add_modifier(Modifier::BOLD)
        };

        let prompt_span = Span::styled(format!("  {prompt} "), prompt_style);
        let prompt_line = Line::from(prompt_span);
        let prompt_widget = ratatui::widgets::Paragraph::new(prompt_line);

        // Split area: prompt (4 chars) + textarea
        if remaining_area.width > 5 {
            let prompt_area = Rect {
                x: remaining_area.x,
                y: remaining_area.y,
                width: 4,
                height: remaining_area.height,
            };
            let input_area = Rect {
                x: remaining_area.x + 4,
                y: remaining_area.y,
                width: remaining_area.width - 4,
                height: remaining_area.height,
            };

            prompt_widget.render(prompt_area, buf);
            (&self.textarea).render(input_area, buf);
        } else {
            prompt_widget.render(remaining_area, buf);
        }
    }

    fn handle_key(&mut self, key: KeyEvent, _state: &mut AppState) -> bool {
        // Don't handle Enter or Esc here — those are handled by the app
        if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE {
            return false;
        }
        if key.code == KeyCode::Esc {
            return false;
        }
        // Let tui-textarea handle everything else
        self.textarea.input(key);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> InputBoxWidget {
        InputBoxWidget::new(Theme::dark())
    }

    #[test]
    fn insert_text_preserves_unix_newlines() {
        let mut w = widget();
        w.insert_text("hi\nhello\ngoodbye");
        assert_eq!(w.current_text(), "hi\nhello\ngoodbye");
        assert_eq!(w.line_count(), 3);
    }

    #[test]
    fn insert_text_normalizes_crlf_to_lf() {
        let mut w = widget();
        w.insert_text("hi\r\nhello\r\ngoodbye");
        assert_eq!(w.current_text(), "hi\nhello\ngoodbye");
        assert_eq!(w.line_count(), 3);
    }

    #[test]
    fn insert_text_normalizes_lone_cr_to_lf() {
        // macOS Terminal.app bracketed paste delivers CR-only line breaks.
        let mut w = widget();
        w.insert_text("hi\rhello\rgoodbye");
        assert_eq!(w.current_text(), "hi\nhello\ngoodbye");
        assert_eq!(w.line_count(), 3);
    }

    #[test]
    fn insert_newline_splits_line_at_cursor() {
        let mut w = widget();
        w.insert_text("hello world");
        w.insert_newline();
        w.insert_text("next");
        assert_eq!(w.current_text(), "hello world\nnext");
        assert_eq!(w.line_count(), 2);
    }

    #[test]
    fn line_count_defaults_to_one_when_empty() {
        let w = widget();
        assert!(w.is_empty());
        assert_eq!(w.line_count(), 1);
    }
}
