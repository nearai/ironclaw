//! Settings tab with a selectable runtime configuration table.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::render::truncate;
use crate::theme::Theme;

use super::{AppState, TuiWidget};

pub struct SettingsWidget {
    theme: Theme,
}

impl SettingsWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub(crate) fn visible_rows(area: Rect) -> usize {
        area.height.saturating_sub(8) as usize
    }

    fn line_for_entry(
        &self,
        index: usize,
        selected: bool,
        path_width: usize,
        value_width: usize,
        state: &AppState,
    ) -> Line<'static> {
        let entry = &state.settings.entries[index];
        let marker = if selected { ">" } else { " " };
        let value = if entry.sensitive && !entry.value.is_empty() && entry.value != "null" {
            "********".to_string()
        } else {
            entry.value.clone()
        };
        let changed = entry.value != entry.default_value;
        let source = if changed {
            entry.source.as_str()
        } else {
            "default"
        };

        let style = if selected {
            Style::default()
                .fg(self.theme.fg.to_color())
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.theme.fg.to_color())
        };
        let value_style = if selected {
            style
        } else if changed {
            self.theme.warning_style()
        } else {
            self.theme.dim_style()
        };

        Line::from(vec![
            Span::styled(marker.to_string(), style),
            Span::raw(" "),
            Span::styled(
                format!(
                    "{:<width$}",
                    truncate(&entry.path, path_width),
                    width = path_width
                ),
                style,
            ),
            Span::raw("  "),
            Span::styled(
                format!(
                    "{:<width$}",
                    truncate(&value, value_width),
                    width = value_width
                ),
                value_style,
            ),
            Span::raw("  "),
            Span::styled(source.to_string(), self.theme.dim_style()),
        ])
    }
}

impl TuiWidget for SettingsWidget {
    fn id(&self) -> &str {
        "settings"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let content_width = area.width.saturating_sub(4) as usize;
        let path_width = content_width.clamp(24, 46).min(content_width / 2 + 8);
        let value_width = content_width
            .saturating_sub(path_width)
            .saturating_sub(16)
            .max(16);
        let visible_rows = Self::visible_rows(area);
        let scroll = state.settings.scroll.min(state.settings.entries.len());
        let end = (scroll + visible_rows).min(state.settings.entries.len());

        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(
                "Settings",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!(
                    "{} settings  {} selected",
                    state.settings.entries.len(),
                    state
                        .settings
                        .selected_entry()
                        .map(|entry| entry.path.as_str())
                        .unwrap_or("none")
                ),
                self.theme.dim_style(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Enter/e", self.theme.accent_style()),
            Span::raw(" edit  "),
            Span::styled("r", self.theme.accent_style()),
            Span::raw(" reset override  "),
            Span::styled("Up/Down", self.theme.accent_style()),
            Span::raw(" move  "),
            Span::styled("PgUp/PgDn", self.theme.accent_style()),
            Span::raw(" page"),
        ]));
        lines.push(Line::from(""));

        if state.settings.editing {
            lines.push(Line::from(vec![
                Span::styled("Editing: ", self.theme.accent_style()),
                Span::styled(
                    state
                        .settings
                        .selected_entry()
                        .map(|entry| entry.path.clone())
                        .unwrap_or_default(),
                    self.theme.bold_style(),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("> ", self.theme.accent_style()),
                Span::raw(state.settings.edit_value.clone()),
            ]));
            lines.push(Line::from(Span::styled(
                "Enter saves via /config set. Esc cancels.",
                self.theme.dim_style(),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<width$}", "Path", width = path_width + 2),
                    self.theme.dim_style(),
                ),
                Span::styled(
                    format!("{:<width$}", "Value", width = value_width + 2),
                    self.theme.dim_style(),
                ),
                Span::styled("Source", self.theme.dim_style()),
            ]));
            lines.push(Line::from(""));
            if state.settings.entries.is_empty() {
                lines.push(Line::from("No settings are available."));
            } else {
                for index in scroll..end {
                    lines.push(self.line_for_entry(
                        index,
                        index == state.settings.selected,
                        path_width,
                        value_width,
                        state,
                    ));
                }
            }
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(" Settings ");
        Paragraph::new(lines)
            .block(block)
            .style(Style::default().fg(self.theme.fg.to_color()))
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{SettingEntry, SettingsState};

    fn buffer_text(buf: &Buffer, area: Rect) -> String {
        let mut lines = Vec::new();
        for y in area.y..area.y + area.height {
            let mut line = String::new();
            for x in area.x..area.x + area.width {
                line.push_str(buf[(x, y)].symbol());
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    #[test]
    fn settings_tab_renders_setting_table() {
        let widget = SettingsWidget::new(Theme::dark());
        let mut settings = SettingsState::default();
        settings.set_entries(vec![
            SettingEntry::new("agent.max_tool_iterations", "50"),
            SettingEntry::new("selected_model", "gpt-5.4"),
        ]);
        let state = AppState {
            settings,
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 100, 20);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Settings"));
        assert!(text.contains("agent.max_tool_iterations"));
        assert!(text.contains("selected_model"));
        assert!(text.contains("Enter/e edit"));
    }

    #[test]
    fn settings_tab_masks_sensitive_values() {
        let widget = SettingsWidget::new(Theme::dark());
        let mut entry = SettingEntry::new("channels.gateway_auth_token", "secret-token");
        entry.sensitive = true;
        let mut settings = SettingsState::default();
        settings.set_entries(vec![entry]);
        let state = AppState {
            settings,
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 100, 12);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("********"));
        assert!(!text.contains("secret-token"));
    }
}
