//! Tab bar widget: shows styled tabs with icons, active indicator, and notification badges.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::theme::Theme;

use super::{ActiveTab, AppState, TuiWidget};

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

        // Compose each tab with icon, label, and optional badge
        let tabs = [
            (
                ActiveTab::Conversation,
                "\u{25E6}", // ◦
                "Chat",
                state.messages.len(),
            ),
            (
                ActiveTab::Dashboard,
                "\u{25A0}", // ■
                "Dashboard",
                0,
            ),
            (
                ActiveTab::Logs,
                "\u{25B8}", // ▸
                "Logs",
                state.log_entries.len(),
            ),
        ];

        let mut spans: Vec<Span> = Vec::with_capacity(tabs.len() * 5 + 2);

        // Left margin
        spans.push(Span::raw(" "));

        // Draw a subtle baseline across the tab bar
        let baseline_style = Style::default().fg(self.theme.border.to_color());

        for (i, (tab_id, icon, label, badge_count)) in tabs.iter().enumerate() {
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
            if !is_active && *badge_count > 0 {
                let badge_text = if *badge_count > 99 {
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
}
