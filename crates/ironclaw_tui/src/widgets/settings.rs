//! Settings control room with sections, curated themes, and editable runtime settings.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};

use crate::layout::TuiSlot;
use crate::render::truncate;
use crate::theme::{Theme, ThemePresetMeta};

use super::{AppState, SettingEntry, SettingsFocus, SettingsSection, TuiWidget};

pub struct SettingsWidget {
    theme: Theme,
}

impl SettingsWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub(crate) fn visible_rows(area: Rect) -> usize {
        let (_, _, list_area, _) = Self::content_areas(area);
        list_area.height.saturating_sub(4) as usize
    }

    fn content_areas(area: Rect) -> (Rect, Rect, Rect, Rect) {
        let outer = Block::default().borders(Borders::ALL);
        let inner = outer.inner(area);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(20), Constraint::Min(40)])
            .split(inner);
        // Gallery height scales with available space so cards can breathe on
        // taller terminals while staying compact when the viewport is narrow.
        let gallery_height = if cols[1].height >= 30 {
            16
        } else if cols[1].height >= 22 {
            14
        } else {
            12
        };
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(gallery_height), Constraint::Min(12)])
            .split(cols[1]);
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(right[1]);
        (cols[0], right[0], bottom[0], bottom[1])
    }

    fn section_lines(&self, state: &AppState) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(vec![
            Span::styled("Sections", self.theme.bold_accent_style()),
            Span::styled("  Tab focus", self.theme.dim_style()),
        ])];
        lines.push(Line::from(""));
        for section in SettingsSection::all() {
            let selected = state.settings.section == section;
            let focused = state.settings.focus == SettingsFocus::Sections;
            let prefix = if selected { "▶ " } else { "  " };
            let count = state
                .settings
                .entries
                .iter()
                .filter(|entry| super::SettingsState::section_for_path(&entry.path) == section)
                .count();
            let style = if selected && focused {
                self.theme.selected_style()
            } else if selected {
                self.theme.bold_accent_style()
            } else {
                self.theme.dim_style()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{prefix}{}", section.label()), style),
                Span::styled(format!("  {count}"), self.theme.dim_style()),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Tab", self.theme.accent_style()),
            Span::raw(" cycle focus"),
        ]));
        lines.push(Line::from(vec![
            Span::styled("↑/↓", self.theme.accent_style()),
            Span::raw(" move"),
        ]));
        lines
    }

    fn theme_card_block(
        &self,
        area: Rect,
        buf: &mut Buffer,
        preset: ThemePresetMeta,
        selected: bool,
        active: bool,
        focused: bool,
    ) {
        let preview = Theme::from_name(preset.id);
        let marker = if selected && focused { "\u{25B6} " } else { "" };
        let badge = if active { " \u{2713}" } else { "" };
        let title = format!(" {marker}{}{badge} ", preset.name);
        let border_style = if selected && focused {
            self.theme.accent_style().add_modifier(Modifier::BOLD)
        } else if active {
            self.theme.success_style().add_modifier(Modifier::BOLD)
        } else {
            self.theme.chrome_border_style()
        };
        // Card body uses the preview theme's panel color so each card looks
        // like a miniature preview of what the UI would become if applied.
        let card_bg = Style::default()
            .bg(preview.panel_bg.to_color())
            .fg(preview.fg.to_color());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .style(card_bg)
            .title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let dim = Style::default()
            .bg(preview.panel_bg.to_color())
            .fg(preview.dim.to_color());
        let accent = Style::default()
            .bg(preview.panel_bg.to_color())
            .fg(preview.accent.to_color())
            .add_modifier(Modifier::BOLD);
        let success = Style::default()
            .bg(preview.panel_bg.to_color())
            .fg(preview.success.to_color());

        // Full palette swatch: accent, success, warning, error, nav, border.
        let swatch_primary = Line::from(vec![
            Span::styled(
                "\u{2588}\u{2588}",
                Style::default().fg(preview.accent.to_color()),
            ),
            Span::raw(" "),
            Span::styled(
                "\u{2588}\u{2588}",
                Style::default().fg(preview.success.to_color()),
            ),
            Span::raw(" "),
            Span::styled(
                "\u{2588}\u{2588}",
                Style::default().fg(preview.warning.to_color()),
            ),
            Span::raw(" "),
            Span::styled(
                "\u{2588}\u{2588}",
                Style::default().fg(preview.error.to_color()),
            ),
            Span::raw(" "),
            Span::styled(
                "\u{2588}\u{2588}",
                Style::default().fg(preview.nav_active_fg.to_color()),
            ),
            Span::raw(" "),
            Span::styled(
                "\u{2588}\u{2588}",
                Style::default().fg(preview.chrome_border.to_color()),
            ),
        ]);

        // Mini UI preview strip: header bar + caret prompt using the
        // preview theme's header/status/bg colors.
        let preview_strip = Line::from(vec![
            Span::styled(
                " \u{2302} ",
                Style::default()
                    .bg(preview.header_bg.to_color())
                    .fg(preview.header_fg.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " main ",
                Style::default()
                    .bg(preview.status_bg.to_color())
                    .fg(preview.status_fg.to_color()),
            ),
            Span::raw(" "),
            Span::styled("\u{203A} ", accent),
            Span::styled("chat", card_bg),
        ]);

        let tagline = Line::from(Span::styled(
            truncate(preset.tagline, inner.width.saturating_sub(1) as usize),
            dim,
        ));
        let action = if active {
            Line::from(Span::styled("\u{2713} Active theme", success))
        } else if selected && focused {
            Line::from(Span::styled("[Enter] apply", accent))
        } else {
            Line::from(Span::styled("Enter apply", dim))
        };

        let mut lines = vec![tagline];
        if inner.height >= 3 {
            lines.push(swatch_primary);
        }
        if inner.height >= 4 {
            lines.push(preview_strip);
        }
        lines.push(action);
        Paragraph::new(lines).style(card_bg).render(inner, buf);
    }

    fn render_theme_gallery(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(self.theme.chrome_border_style())
            .style(self.theme.panel_style())
            .title(" \u{25C6} Theme gallery ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let header = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(6)])
            .split(inner);
        Paragraph::new(vec![Line::from(vec![
            Span::styled(" Active \u{2022} ", self.theme.dim_style()),
            Span::styled(
                state.settings.active_theme.clone(),
                self.theme.bold_accent_style(),
            ),
            Span::styled(
                "    Tab",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" focus gallery  ", self.theme.dim_style()),
            Span::styled(
                "\u{2190}/\u{2192}",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" browse  ", self.theme.dim_style()),
            Span::styled(
                "Enter",
                self.theme.accent_style().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" apply + save", self.theme.dim_style()),
        ])])
        .style(self.theme.panel_style())
        .render(header[0], buf);

        let presets = Theme::preset_catalog();
        let columns = if header[1].width >= 120 {
            4
        } else if header[1].width >= 84 {
            3
        } else if header[1].width >= 48 {
            2
        } else {
            1
        };
        let rows = presets.len().div_ceil(columns).max(1);
        let row_constraints = vec![Constraint::Percentage((100 / rows) as u16); rows];
        let row_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints)
            .split(header[1]);
        for (row_index, row_area) in row_areas.iter().copied().enumerate() {
            let mut constraints = Vec::new();
            for _ in 0..columns {
                constraints.push(Constraint::Percentage((100 / columns) as u16));
            }
            let cards = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(row_area);
            for (col_index, rect) in cards.iter().copied().enumerate() {
                let index = row_index * columns + col_index;
                if let Some(preset) = presets.get(index) {
                    self.theme_card_block(
                        rect,
                        buf,
                        *preset,
                        state.settings.theme_selected == index,
                        state.settings.active_theme == preset.id,
                        state.settings.focus == SettingsFocus::Themes,
                    );
                }
            }
        }
    }

    fn line_for_entry(
        &self,
        entry: &SettingEntry,
        selected: bool,
        path_width: usize,
        value_width: usize,
    ) -> Line<'static> {
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
            self.theme.selected_style()
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

    fn render_settings_list(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        match state.settings.section {
            SettingsSection::Skills => {
                let focused = state.settings.focus == SettingsFocus::Entries;
                super::browser::render_skill_list(&self.theme, area, buf, state, focused);
                return;
            }
            SettingsSection::Extensions => {
                let focused = state.settings.focus == SettingsFocus::Entries;
                super::browser::render_extension_list(&self.theme, area, buf, state, focused);
                return;
            }
            _ => {}
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.chrome_border_style())
            .style(self.theme.panel_style())
            .title(format!(" {} settings ", state.settings.section.label()));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let filtered = state.settings.filtered_entry_indices();
        let scroll = state.settings.scroll.min(filtered.len());
        let end = (scroll + Self::visible_rows(area)).min(filtered.len());
        let content_width = inner.width.saturating_sub(2) as usize;
        let path_width = content_width.clamp(20, 38).min(content_width / 2 + 6);
        let value_width = content_width
            .saturating_sub(path_width)
            .saturating_sub(14)
            .max(12);

        let mut lines = vec![Line::from(vec![
            Span::styled(
                format!("{:<width$}", "Path", width = path_width + 2),
                self.theme.dim_style(),
            ),
            Span::styled(
                format!("{:<width$}", "Value", width = value_width + 2),
                self.theme.dim_style(),
            ),
            Span::styled("Source", self.theme.dim_style()),
        ])];
        lines.push(Line::from(""));

        if filtered.is_empty() {
            lines.push(Line::from(Span::styled(
                "No settings in this section yet.",
                self.theme.dim_style(),
            )));
        } else {
            for (visible_index, entry_index) in filtered.iter().enumerate().take(end).skip(scroll) {
                let Some(entry) = state.settings.entries.get(*entry_index) else {
                    continue;
                };
                lines.push(self.line_for_entry(
                    entry,
                    visible_index == state.settings.selected,
                    path_width,
                    value_width,
                ));
            }
        }

        Paragraph::new(lines)
            .style(self.theme.panel_style())
            .render(inner, buf);
    }

    fn render_detail_panel(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        match state.settings.section {
            SettingsSection::Skills => {
                super::browser::render_skill_detail(&self.theme, area, buf, state);
                return;
            }
            SettingsSection::Extensions => {
                super::browser::render_extension_detail(&self.theme, area, buf, state);
                return;
            }
            _ => {}
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.chrome_border_style())
            .style(self.theme.panel_alt_style())
            .title(" Detail ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let mut lines = vec![Line::from(vec![
            Span::styled("Focus", self.theme.accent_style()),
            Span::raw("  "),
            Span::styled(
                match state.settings.focus {
                    SettingsFocus::Sections => "Sections",
                    SettingsFocus::Themes => "Themes",
                    SettingsFocus::Entries => "Entries",
                },
                self.theme.bold_style(),
            ),
        ])];
        lines.push(Line::from(""));

        if state.settings.editing {
            lines.push(Line::from(vec![
                Span::styled("Editing", self.theme.accent_style()),
                Span::raw("  "),
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
        } else if let Some(entry) = state.settings.selected_entry() {
            let current_value =
                if entry.sensitive && !entry.value.is_empty() && entry.value != "null" {
                    "********".to_string()
                } else {
                    entry.value.clone()
                };
            let default_value = if entry.sensitive
                && !entry.default_value.is_empty()
                && entry.default_value != "null"
            {
                "********".to_string()
            } else {
                entry.default_value.clone()
            };
            lines.push(Line::from(Span::styled(
                entry.path.clone(),
                self.theme.bold_style(),
            )));
            lines.push(Line::from(vec![
                Span::styled("Current", self.theme.dim_style()),
                Span::raw("  "),
                Span::styled(current_value, self.theme.accent_style()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Default", self.theme.dim_style()),
                Span::raw("  "),
                Span::styled(default_value, self.theme.dim_style()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Source", self.theme.dim_style()),
                Span::raw("  "),
                Span::styled(entry.source.clone(), self.theme.bold_style()),
            ]));
            lines.push(Line::from(""));
            if entry.path == "selected_model" {
                lines.push(Line::from(vec![
                    Span::styled("Enter", self.theme.accent_style()),
                    Span::raw(" open model picker"),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("e", self.theme.accent_style()),
                    Span::raw(" edit raw value"),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("Enter/e", self.theme.accent_style()),
                    Span::raw(" edit"),
                ]));
            }
            lines.push(Line::from(vec![
                Span::styled("r", self.theme.accent_style()),
                Span::raw(" reset override"),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Tab", self.theme.accent_style()),
                Span::raw(" focus sections/themes/entries"),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                "Select a section with settings to inspect details.",
                self.theme.dim_style(),
            )));
        }

        Paragraph::new(lines)
            .style(self.theme.panel_alt_style())
            .render(inner, buf);
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

        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .style(self.theme.panel_style())
            .title(" Settings ");
        outer.render(area, buf);

        let (sections_area, gallery_area, list_area, detail_area) = Self::content_areas(area);
        Paragraph::new(self.section_lines(state))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.theme.chrome_border_style())
                    .style(self.theme.panel_alt_style())
                    .title(" Control "),
            )
            .style(self.theme.panel_alt_style())
            .render(sections_area, buf);
        self.render_theme_gallery(gallery_area, buf, state);
        self.render_settings_list(list_area, buf, state);
        self.render_detail_panel(detail_area, buf, state);
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
        assert!(text.contains("Theme gallery"));
        assert!(text.contains("Detail"));
        assert!(text.contains("Current"));
        assert!(text.contains("Control"));
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

    #[test]
    fn settings_tab_renders_theme_gallery_and_sections() {
        let widget = SettingsWidget::new(Theme::dark());
        let mut settings = SettingsState::default();
        settings.set_entries(vec![SettingEntry::new("selected_model", "gpt-5.4")]);
        let state = AppState {
            settings,
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("Theme gallery"));
        assert!(text.contains("Inference"));
        assert!(text.contains("Agent"));
        assert!(text.contains("Tab focus"));
    }

    #[test]
    fn settings_tab_filters_entries_by_selected_section() {
        let widget = SettingsWidget::new(Theme::dark());
        let mut settings = SettingsState {
            section: crate::widgets::SettingsSection::Agent,
            ..SettingsState::default()
        };
        settings.set_entries(vec![
            SettingEntry::new("agent.max_tool_iterations", "50"),
            SettingEntry::new("selected_model", "gpt-5.4"),
        ]);
        let state = AppState {
            settings,
            ..AppState::default()
        };
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &state);

        let text = buffer_text(&buf, area);
        assert!(text.contains("agent.max_tool_iterations"));
        assert!(!text.contains("selected_model"));
    }
}
