//! Thread resume picker modal widget.
//!
//! Centered modal listing past conversations. The user navigates with
//! Up/Down/j/k, selects with Enter, and dismisses with Esc.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::theme::Theme;
use crate::widgets::{AppState, ThreadPickerState};

/// Renders the thread picker modal overlay.
pub struct ThreadPickerWidget {
    theme: Theme,
}

impl ThreadPickerWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Compute a centered modal area sized to fit the thread list.
    pub fn modal_area(terminal: Rect, item_count: usize) -> Rect {
        let width = (terminal.width * 3 / 4)
            .max(50)
            .min(terminal.width.saturating_sub(4));
        // 2 border rows + 1 footer + items (capped at 15)
        let content_rows = item_count.min(15) as u16;
        let height = (content_rows + 4).min(terminal.height.saturating_sub(4));
        let x = (terminal.width.saturating_sub(width)) / 2;
        let y = (terminal.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }

    /// Render the thread picker into the given area.
    pub fn render_picker(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let Some(ref picker) = state.pending_thread_picker else {
            return;
        };
        if area.height < 5 || area.width < 20 {
            return;
        }

        Clear.render(area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.accent_style())
            .title(Span::styled(
                " Resume Conversation ",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 2 || inner.width < 10 {
            return;
        }

        // Reserve last row for footer
        let list_height = inner.height.saturating_sub(1) as usize;
        let total = picker.threads.len();

        // Scroll so selected item is always visible
        let scroll_offset = if picker.selected >= list_height {
            picker.selected - list_height + 1
        } else {
            0
        };

        let visible_range = scroll_offset..total.min(scroll_offset + list_height);

        for (row_idx, thread_idx) in visible_range.enumerate() {
            let thread = &picker.threads[thread_idx];
            let is_selected = thread_idx == picker.selected;
            let y = inner.y + row_idx as u16;

            let marker = if is_selected { "\u{25CF}" } else { "\u{25CB}" };
            let title_text = thread.title.as_deref().unwrap_or("(untitled)");

            // Truncate title to fit
            let meta = format!("  {} msgs  {}", thread.message_count, thread.last_activity);
            let available = inner.width as usize;
            let marker_len = 2; // "● " or "○ "
            let meta_len = meta.len();
            let max_title = available.saturating_sub(marker_len + meta_len + 1);
            let display_title = if title_text.chars().count() > max_title {
                let truncated: String = title_text
                    .chars()
                    .take(max_title.saturating_sub(3))
                    .collect();
                format!("{truncated}...")
            } else {
                title_text.to_string()
            };

            let name_style = if is_selected {
                self.theme.bold_accent_style()
            } else {
                ratatui::style::Style::default().fg(self.theme.fg.to_color())
            };
            let meta_style = self.theme.dim_style();

            let line = Line::from(vec![
                Span::styled(format!("{marker} "), name_style),
                Span::styled(display_title, name_style),
                Span::styled(meta, meta_style),
            ]);

            let line_area = Rect::new(inner.x, y, inner.width, 1);

            // Highlight selected row background
            if is_selected {
                for bx in line_area.x..line_area.x + line_area.width {
                    if let Some(cell) = buf.cell_mut((bx, y)) {
                        cell.set_style(
                            ratatui::style::Style::default().bg(self.theme.border.to_color()),
                        );
                    }
                }
            }

            Paragraph::new(line).render(line_area, buf);
        }

        // Footer
        let footer_y = inner.y + inner.height.saturating_sub(1);
        let footer = Line::from(Span::styled(
            " \u{2191}\u{2193} select  Enter resume  Esc cancel ",
            self.theme.dim_style(),
        ));
        let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
        Paragraph::new(footer).render(footer_area, buf);
    }
}

/// Navigate the thread picker selection up.
pub fn thread_picker_up(picker: &mut ThreadPickerState) {
    if picker.threads.is_empty() {
        return;
    }
    picker.selected = if picker.selected == 0 {
        picker.threads.len() - 1
    } else {
        picker.selected - 1
    };
}

/// Navigate the thread picker selection down.
pub fn thread_picker_down(picker: &mut ThreadPickerState) {
    if picker.threads.is_empty() {
        return;
    }
    picker.selected = (picker.selected + 1) % picker.threads.len();
}

/// Get the thread ID of the currently selected thread.
pub fn thread_picker_selected_id(picker: &ThreadPickerState) -> Option<&str> {
    picker.threads.get(picker.selected).map(|t| t.id.as_str())
}
