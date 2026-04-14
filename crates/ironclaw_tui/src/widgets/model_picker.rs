//! Model picker modal widget.
//!
//! Triggered from the input when the user types `/model`. The modal filters
//! the available model list, supports arrow-key navigation, and submits the
//! selected model back as a `/model <name>` command.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::theme::Theme;
use crate::widgets::AppState;

/// Mutable state for the model picker modal.
#[derive(Debug, Clone, Default)]
pub struct ModelPickerState {
    /// Whether the picker is visible.
    pub visible: bool,
    /// Filter derived from the text after `/model`.
    pub filter: String,
    /// Available models from the active provider.
    pub models: Vec<String>,
    /// Indices into `models` that match `filter`.
    pub filtered: Vec<usize>,
    /// Currently highlighted row.
    pub selected: usize,
}

impl ModelPickerState {
    /// Create state with a known model list.
    pub fn with_models(models: Vec<String>) -> Self {
        let mut state = Self::default();
        state.set_models(models);
        state
    }

    /// Replace the available model list and reset filter state.
    pub fn set_models(&mut self, models: Vec<String>) {
        self.models = models;
        self.close();
    }

    /// Whether the picker has model data to show.
    pub fn has_models(&self) -> bool {
        !self.models.is_empty()
    }

    /// Recompute `filtered` from the current `filter` text.
    pub fn update_filter(&mut self, filter: &str) {
        self.filter = filter.to_string();
        let needle = filter.trim().to_lowercase();
        self.filtered = self
            .models
            .iter()
            .enumerate()
            .filter(|(_, model)| needle.is_empty() || model.to_lowercase().contains(&needle))
            .map(|(idx, _)| idx)
            .collect();

        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }

        if !needle.is_empty()
            && let Some(exact_idx) = self
                .filtered
                .iter()
                .position(|&idx| self.models[idx].eq_ignore_ascii_case(needle.as_str()))
        {
            self.selected = exact_idx;
            return;
        }

        if self.selected >= self.filtered.len() {
            self.selected = 0;
        }
    }

    /// Open the picker with an optional filter.
    pub fn open(&mut self, filter: &str) {
        self.visible = true;
        self.update_filter(filter);
    }

    /// Close the picker and reset the filter.
    pub fn close(&mut self) {
        self.visible = false;
        self.filter.clear();
        self.filtered = (0..self.models.len()).collect();
        self.selected = 0;
    }

    /// Move the selection up.
    pub fn move_up(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.filtered.len() - 1
        } else {
            self.selected - 1
        };
    }

    /// Move the selection down.
    pub fn move_down(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.filtered.len();
    }

    /// Get the currently selected model, if any.
    pub fn selected_model(&self) -> Option<&str> {
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.models.get(idx).map(String::as_str))
    }
}

/// Renders the model picker modal overlay.
pub struct ModelPickerWidget {
    theme: Theme,
}

impl ModelPickerWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Compute a centered modal area sized for the current model list.
    pub fn modal_area(terminal: Rect, item_count: usize) -> Rect {
        let width = (terminal.width * 3 / 4)
            .max(50)
            .min(terminal.width.saturating_sub(4));
        let content_rows = item_count.min(12) as u16;
        let height = (content_rows + 5)
            .max(7)
            .min(terminal.height.saturating_sub(4));
        let x = (terminal.width.saturating_sub(width)) / 2;
        let y = (terminal.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }

    /// Render the picker into the given area.
    pub fn render_picker(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let picker = &state.model_picker;
        if !picker.visible || area.height < 7 || area.width < 24 {
            return;
        }

        Clear.render(area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.accent_style())
            .title(Span::styled(
                " Select model ",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 || inner.width < 10 {
            return;
        }

        let filter_line = Line::from(vec![
            Span::styled(" Filter: ", self.theme.dim_style()),
            Span::styled(
                if picker.filter.is_empty() {
                    "all models".to_string()
                } else {
                    picker.filter.clone()
                },
                self.theme.bold_style(),
            ),
        ]);
        Paragraph::new(filter_line).render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

        let list_top = inner.y + 1;
        let list_height = inner.height.saturating_sub(2) as usize;

        if picker.filtered.is_empty() {
            Paragraph::new(Line::from(Span::styled(
                " No matching models. Press Enter to submit the typed command. ",
                self.theme.dim_style(),
            )))
            .render(Rect::new(inner.x, list_top, inner.width, 1), buf);
        } else {
            let scroll_offset = if picker.selected >= list_height {
                picker.selected - list_height + 1
            } else {
                0
            };
            let visible_range =
                scroll_offset..picker.filtered.len().min(scroll_offset + list_height);

            for (row_idx, filtered_idx) in visible_range.enumerate() {
                let model_idx = picker.filtered[filtered_idx];
                let model = &picker.models[model_idx];
                let is_selected = filtered_idx == picker.selected;
                let is_active = model == &state.model;
                let y = list_top + row_idx as u16;

                let marker = if is_selected { "\u{25CF}" } else { "\u{25CB}" };
                let suffix = if is_active { " (active)" } else { "" };
                let text = format!("{marker} {model}{suffix}");

                let line_area = Rect::new(inner.x, y, inner.width, 1);
                if is_selected {
                    for bx in line_area.x..line_area.x + line_area.width {
                        if let Some(cell) = buf.cell_mut((bx, y)) {
                            cell.set_style(
                                ratatui::style::Style::default().bg(self.theme.border.to_color()),
                            );
                        }
                    }
                }

                let style = if is_selected {
                    self.theme.bold_accent_style()
                } else if is_active {
                    self.theme.accent_style()
                } else {
                    ratatui::style::Style::default().fg(self.theme.fg.to_color())
                };

                Paragraph::new(Line::from(Span::styled(text, style))).render(line_area, buf);
            }
        }

        let footer_y = inner.y + inner.height.saturating_sub(1);
        let footer = Line::from(Span::styled(
            " \u{2191}\u{2193} select  Enter choose  Esc cancel ",
            self.theme.dim_style(),
        ));
        Paragraph::new(footer).render(Rect::new(inner.x, footer_y, inner.width, 1), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::ModelPickerState;

    #[test]
    fn filter_uses_contains_and_exact_match_selection() {
        let mut state = ModelPickerState::with_models(vec![
            "gpt-4o".to_string(),
            "gpt-5".to_string(),
            "claude-sonnet-4-6".to_string(),
        ]);

        state.update_filter("gpt-5");

        assert_eq!(state.filtered.len(), 1);
        assert_eq!(state.selected_model(), Some("gpt-5"));
    }
}
