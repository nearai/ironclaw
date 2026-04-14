//! User input area widget using ratatui-textarea.

use std::sync::Mutex;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{AppState, TuiWidget};

pub struct InputBoxWidget {
    theme: Theme,
    textarea: Mutex<TextArea<'static>>,
}

impl InputBoxWidget {
    pub fn new(theme: Theme) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea
            .set_block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::NONE));
        textarea.set_placeholder_text("Ask anything... (/ for commands, Ctrl-/ for help)");
        textarea.set_placeholder_style(theme.dim_style());
        Self {
            theme,
            textarea: Mutex::new(textarea),
        }
    }

    /// Get the current input text and clear the textarea.
    pub fn take_input(&mut self) -> String {
        let textarea = self.textarea.get_mut().expect("textarea lock poisoned");
        let lines: Vec<String> = textarea.lines().iter().map(|l| l.to_string()).collect();
        let text = lines.join("\n");
        textarea.select_all();
        textarea.cut();
        text
    }

    /// Returns true if the textarea is empty.
    pub fn is_empty(&self) -> bool {
        let textarea = self.textarea.lock().expect("textarea lock poisoned");
        textarea.lines().iter().all(|l| l.is_empty())
    }

    /// Peek at the current text content without consuming it.
    pub fn current_text(&self) -> String {
        let textarea = self.textarea.lock().expect("textarea lock poisoned");
        textarea.lines().join("\n")
    }

    /// Return the current cursor position as (row, column).
    pub fn cursor(&self) -> (usize, usize) {
        let textarea = self.textarea.lock().expect("textarea lock poisoned");
        let c = textarea.cursor();
        (c.0, c.1)
    }

    /// Returns true when the cursor is on the first input line.
    pub fn is_cursor_on_first_line(&self) -> bool {
        self.cursor().0 == 0
    }

    /// Returns true when the cursor is on the last input line.
    pub fn is_cursor_on_last_line(&self) -> bool {
        let (row, _) = self.cursor();
        let textarea = self.textarea.lock().expect("textarea lock poisoned");
        row + 1 >= textarea.lines().len().max(1)
    }

    /// Replace the current text content with `text`.
    pub fn set_text(&mut self, text: &str) {
        let textarea = self.textarea.get_mut().expect("textarea lock poisoned");
        textarea.select_all();
        textarea.cut();
        textarea.insert_str(text);
    }

    /// Insert text at the current cursor position.
    pub fn insert_text(&mut self, text: &str) {
        let textarea = self.textarea.get_mut().expect("textarea lock poisoned");
        textarea.insert_str(text);
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

        // Render prompt character
        let prompt = if state.pending_approval.is_some() {
            "\u{25C6}"
        } else {
            "\u{203A}"
        };

        let prompt_span = Span::styled(
            format!("  {prompt} "),
            self.theme.accent_style().add_modifier(Modifier::BOLD),
        );
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
            let textarea = self.textarea.lock().expect("textarea lock poisoned");
            (&*textarea).render(input_area, buf);
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
        // Let ratatui-textarea handle everything else
        let textarea = self.textarea.get_mut().expect("textarea lock poisoned");
        textarea.input(key);
        true
    }
}
