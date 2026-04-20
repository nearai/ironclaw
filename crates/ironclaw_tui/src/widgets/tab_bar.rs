//! Tab bar widget: shows styled tabs with icons, active indicator, and notification badges.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{ActiveTab, AppState, TuiWidget};

const TAB_SPECS: [(ActiveTab, &str, &str); 4] = [
    (ActiveTab::Conversation, "\u{25E6}", "Chat"),
    (ActiveTab::Dashboard, "\u{25A0}", "Dashboard"),
    (ActiveTab::Logs, "\u{25B8}", "Logs"),
    (ActiveTab::Settings, "\u{2699}", "Settings"),
];

fn tab_badge_count(tab: ActiveTab, state: &AppState) -> usize {
    match tab {
        ActiveTab::Conversation => state.messages.len(),
        ActiveTab::Dashboard => 0,
        ActiveTab::Logs => state.log_entries.len(),
        ActiveTab::Settings => 0,
    }
}

fn badge_width(badge_count: usize) -> u16 {
    if badge_count == 0 {
        0
    } else if badge_count > 99 {
        4
    } else {
        format!(" {badge_count}").chars().count() as u16
    }
}

pub(crate) fn tab_hit_areas(
    area: Rect,
    state: &AppState,
) -> Vec<(ActiveTab, std::ops::Range<u16>)> {
    if area.height == 0 || area.width < 20 {
        return Vec::new();
    }

    let mut cursor = area.x.saturating_add(1);
    let mut areas = Vec::with_capacity(TAB_SPECS.len());
    for (i, (tab_id, _icon, label)) in TAB_SPECS.iter().enumerate() {
        if i > 0 {
            cursor = cursor.saturating_add(2);
        }
        let mut width = 2 + label.chars().count() as u16;
        if state.active_tab != *tab_id {
            width = width.saturating_add(badge_width(tab_badge_count(*tab_id, state)));
        }
        let start = cursor;
        let end = start.saturating_add(width);
        areas.push((*tab_id, start..end));
        cursor = end;
    }
    areas
}

pub struct TabBarWidget {
    theme: Theme,
}

impl TabBarWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for TabBarWidget {
    fn id(&self) -> &str {
        "tab_bar"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::Tab
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width < 20 {
            return;
        }

        let mut spans: Vec<Span> = Vec::with_capacity(TAB_SPECS.len() * 5 + 2);

        // Left margin
        spans.push(Span::raw(" "));

        // Draw a subtle baseline across the tab bar
        let baseline_style = Style::default().fg(self.theme.border.to_color());

        for (i, (tab_id, icon, label)) in TAB_SPECS.iter().enumerate() {
            let badge_count = tab_badge_count(*tab_id, state);
            let is_active = state.active_tab == *tab_id;

            if i > 0 {
                // Separator between tabs
                spans.push(Span::styled("  ", baseline_style));
            }

            if is_active {
                // Active tab: accent color, bold, with underline indicator
                let active_style = self
                    .theme
                    .accent_style()
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                spans.push(Span::styled(format!("{icon} "), active_style));
                spans.push(Span::styled(*label, active_style));
            } else {
                // Inactive tab: dim
                let inactive_style = self.theme.dim_style();
                spans.push(Span::styled(format!("{icon} "), inactive_style));
                spans.push(Span::styled(*label, inactive_style));
            }

            // Badge for non-zero counts on inactive tabs
            if !is_active && badge_count > 0 {
                let badge_text = if badge_count > 99 {
                    " 99+".to_string()
                } else {
                    format!(" {badge_count}")
                };
                spans.push(Span::styled(
                    badge_text,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::DIM),
                ));
            }
        }

        // Streaming/activity indicator on the right side
        let right_indicator = if state.is_streaming {
            let spinner_frame = state.tick_count % 4;
            let dots = match spinner_frame {
                0 => "\u{2022}  ", // •
                1 => " \u{2022} ",
                2 => "  \u{2022}",
                _ => " \u{2022} ",
            };
            Some(Span::styled(format!("  {dots}"), self.theme.accent_style()))
        } else if !state.active_tools.is_empty() {
            let tool_count = state.active_tools.len();
            Some(Span::styled(
                format!("  \u{26A1}{tool_count}"),
                self.theme.warning_style(),
            ))
        } else {
            None
        };

        if let Some(indicator) = right_indicator {
            // Fill remaining space then append indicator
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let indicator_len = indicator.content.chars().count();
            let fill = (area.width as usize).saturating_sub(used + indicator_len + 1);
            if fill > 0 {
                spans.push(Span::raw(" ".repeat(fill)));
            }
            spans.push(indicator);
        }

        let line = Line::from(spans);
        let paragraph = ratatui::widgets::Paragraph::new(line);
        paragraph.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_bar_renders_without_panic() {
        let theme = Theme::dark();
        let widget = TabBarWidget::new(theme);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &state);
    }

    #[test]
    fn tab_bar_too_narrow_skips_render() {
        let theme = Theme::dark();
        let widget = TabBarWidget::new(theme);
        let state = AppState::default();
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &state);
        // Should not panic and should produce empty buffer
    }

    #[test]
    fn tab_hit_areas_expand_for_badges() {
        let mut log_entries = crate::event::LogRingBuffer::new(10);
        log_entries.push(crate::event::TuiLogEntry {
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "message".to_string(),
            timestamp: "now".to_string(),
        });
        let state = AppState {
            active_tab: ActiveTab::Dashboard,
            messages: vec![
                super::super::ChatMessage {
                    role: super::super::MessageRole::User,
                    content: "msg".to_string(),
                    timestamp: chrono::Utc::now(),
                    cost_summary: None,
                };
                120
            ],
            log_entries,
            ..Default::default()
        };

        let hit_areas = tab_hit_areas(Rect::new(0, 0, 80, 1), &state);
        let (_, logs_range) = hit_areas
            .iter()
            .find(|(tab, _)| *tab == ActiveTab::Logs)
            .expect("logs range");
        let (_, settings_range) = hit_areas
            .iter()
            .find(|(tab, _)| *tab == ActiveTab::Settings)
            .expect("settings range");

        assert!(logs_range.end > logs_range.start);
        assert!(settings_range.start >= logs_range.end);
    }
}
